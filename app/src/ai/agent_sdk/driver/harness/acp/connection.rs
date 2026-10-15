//! JSON-RPC 2.0 with an ACP agent, framed as one JSON object per line.
//!
//! Inbound traffic is dispatched in arrival order: responses resolve their pending request,
//! agent→client requests are handed to a task that answers them through the installed handler,
//! and notifications are queued for the owner to consume. Because the read loop pushes a
//! notification before it reads the next line, every notification that precedes a response on
//! the wire is queued by the time that response resolves. Agent requests never hold up the
//! loop: a `session/request_permission` the handler is still deciding does not delay the
//! `session/update`s or the prompt response behind it, and the owner can answer it itself
//! through [`AcpConnection::respond_pending`] or
//! [`AcpConnection::cancel_pending_permission_requests`].
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use anyhow::{Context as _, Result, anyhow};
use futures::channel::oneshot;
use futures::future::{AbortHandle, Abortable, BoxFuture};
use futures::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, BufWriter};
use futures::lock::Mutex as AsyncMutex;
use parking_lot::Mutex;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use warpui::r#async::executor::{Background, BackgroundTask};

use super::protocol::{METHOD_REQUEST_PERMISSION, PermissionOutcome, RequestPermissionResponse};

const JSON_RPC_VERSION: &str = "2.0";
const METHOD_NOT_FOUND: i64 = -32601;
const INTERNAL_ERROR: i64 = -32603;

/// A JSON-RPC error returned to the agent for a request it made to us.
#[derive(Debug, Deserialize, thiserror::Error)]
#[error("ACP error {code}: {message}")]
pub(super) struct RpcError {
    pub code: i64,
    pub message: String,
}

impl RpcError {
    pub(super) fn method_not_found(method: &str) -> Self {
        Self {
            code: METHOD_NOT_FOUND,
            message: format!("Method {method} not supported"),
        }
    }

    pub(super) fn internal(error: impl std::fmt::Display) -> Self {
        Self {
            code: INTERNAL_ERROR,
            message: error.to_string(),
        }
    }
}

/// Answers requests the agent sends to the client (permissions, file system access). Each
/// request runs on its own background task, so a slow answer only delays that request.
pub(super) type AgentRequestHandler =
    Arc<dyn Fn(&str, Value) -> BoxFuture<'static, Result<Value, RpcError>> + Send + Sync>;

/// A notification received from the agent.
pub(super) struct InboundNotification {
    pub method: String,
    pub params: Value,
}

/// The JSON-RPC id of an agent→client request. Agents may use numbers or strings; the
/// canonical JSON rendering keeps the two spaces apart.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct InboundRequestId(String);

impl InboundRequestId {
    fn new(id: &Value) -> Self {
        Self(id.to_string())
    }
}

/// An agent→client request that has been dispatched to the handler but not answered yet.
#[derive(Clone, Debug)]
pub(super) struct PendingInboundRequest {
    pub id: InboundRequestId,
    pub method: String,
    /// The `sessionId` the request named, if any.
    pub session_id: Option<String>,
    /// For `session/request_permission`, the tool call awaiting the decision.
    pub tool_call_id: Option<String>,
}

struct InboundRequestEntry {
    request: PendingInboundRequest,
    /// The id as the agent sent it, echoed back verbatim on the response.
    wire_id: Value,
    /// Stops the handler when the request is answered out-of-band.
    handler: AbortHandle,
}

type PendingRequests = Arc<AsyncMutex<HashMap<i64, oneshot::Sender<Result<Value>>>>>;
type InboundRequests = Arc<Mutex<HashMap<InboundRequestId, InboundRequestEntry>>>;
type BoxedWriter = Pin<Box<dyn AsyncWrite + Send>>;
/// `None` once the connection has been closed; dropping the writer is what signals EOF to the
/// agent.
type SharedWriter = Arc<AsyncMutex<Option<BufWriter<BoxedWriter>>>>;

pub(super) struct AcpConnection {
    writer: SharedWriter,
    pending: PendingRequests,
    inbound: InboundRequests,
    next_request_id: AtomicI64,
    _reader_task: BackgroundTask,
}

impl AcpConnection {
    /// Starts reading agent messages from `reader` and writing ours to `writer`. Notifications
    /// are delivered on the returned receiver in wire order.
    pub(super) fn new<R, W>(
        reader: R,
        writer: W,
        handler: AgentRequestHandler,
        executor: &Arc<Background>,
    ) -> (Self, async_channel::Receiver<InboundNotification>)
    where
        R: AsyncRead + Send + Unpin + 'static,
        W: AsyncWrite + Send + 'static,
    {
        let writer: SharedWriter = Arc::new(AsyncMutex::new(Some(BufWriter::new(
            Box::pin(writer) as BoxedWriter,
        ))));
        let pending: PendingRequests = Arc::new(AsyncMutex::new(HashMap::new()));
        let inbound: InboundRequests = Arc::default();
        let (notification_tx, notification_rx) = async_channel::unbounded();

        let reader_task = executor.spawn(read_loop(
            BufReader::new(reader),
            writer.clone(),
            pending.clone(),
            inbound.clone(),
            notification_tx,
            handler,
        ));

        (
            Self {
                writer,
                pending,
                inbound,
                next_request_id: AtomicI64::new(1),
                _reader_task: reader_task,
            },
            notification_rx,
        )
    }

    /// Sends a request and waits for the agent's response, deserialized as `R`.
    ///
    /// The returned future is the only signal that the response has arrived, so a caller that
    /// must keep draining notifications while the request is in flight (a `session/prompt`
    /// whose `session/update`s stream before the response) races this future against the
    /// notification receiver rather than awaiting it alone.
    pub(super) async fn request<P: Serialize, R: DeserializeOwned>(
        &self,
        method: &str,
        params: P,
    ) -> Result<R> {
        let id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);
        let message = json!({
            "jsonrpc": JSON_RPC_VERSION,
            "id": id,
            "method": method,
            "params": params,
        });
        if let Err(error) = write_message(&self.writer, &message).await {
            self.pending.lock().await.remove(&id);
            return Err(error.context(format!("Failed to send ACP request {method}")));
        }
        let value = rx
            .await
            .map_err(|_| anyhow!("ACP agent closed before responding to {method}"))??;
        serde_json::from_value(value)
            .with_context(|| format!("Failed to parse ACP response to {method}"))
    }

    pub(super) async fn notify<P: Serialize>(&self, method: &str, params: P) -> Result<()> {
        let message = json!({
            "jsonrpc": JSON_RPC_VERSION,
            "method": method,
            "params": params,
        });
        write_message(&self.writer, &message)
            .await
            .with_context(|| format!("Failed to send ACP notification {method}"))
    }

    /// Agent→client requests the handler has not answered yet, in no particular order.
    // Nothing consults this outside tests yet; `cancel_pending_permission_requests` covers the
    // only out-of-band answer the harness gives today.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) fn pending_inbound_requests(&self) -> Vec<PendingInboundRequest> {
        self.inbound
            .lock()
            .values()
            .map(|entry| entry.request.clone())
            .collect()
    }

    /// Answers an outstanding agent request in place of the handler, whose own answer is then
    /// discarded. Returns `false` when the request was already answered or never existed.
    pub(super) async fn respond_pending(
        &self,
        id: &InboundRequestId,
        response: Result<Value, RpcError>,
    ) -> bool {
        let Some(entry) = self.inbound.lock().remove(id) else {
            return false;
        };
        entry.handler.abort();
        if let Err(error) =
            write_message(&self.writer, &response_message(entry.wire_id, response)).await
        {
            log::warn!(
                "Failed to answer ACP agent request {} out-of-band: {error}",
                entry.request.method
            );
        }
        true
    }

    /// Answers every outstanding `session/request_permission` for `session_id` with
    /// `cancelled`, as the protocol requires once the turn is cancelled, so the agent is not
    /// left waiting on a decision for work that will not proceed. A request that did not name
    /// a session is taken to belong to this one. Returns how many were answered.
    pub(super) async fn cancel_pending_permission_requests(&self, session_id: &str) -> usize {
        let permission_requests: Vec<PendingInboundRequest> = self
            .inbound
            .lock()
            .values()
            .filter(|entry| {
                entry.request.method == METHOD_REQUEST_PERMISSION
                    && entry
                        .request
                        .session_id
                        .as_deref()
                        .is_none_or(|session| session == session_id)
            })
            .map(|entry| entry.request.clone())
            .collect();
        let mut answered = 0;
        for request in &permission_requests {
            let cancelled = serde_json::to_value(RequestPermissionResponse {
                outcome: PermissionOutcome::Cancelled,
            })
            .map_err(RpcError::internal);
            if self.respond_pending(&request.id, cancelled).await {
                log::info!(
                    "Cancelled pending ACP permission request for tool call {:?}",
                    request.tool_call_id
                );
                answered += 1;
            }
        }
        answered
    }

    /// Closes our side of the connection (flushing, then closing the writer so the transport
    /// delivers EOF) so a well-behaved agent exits on its own. Requests still in flight fail
    /// once the agent closes its side in response.
    pub(super) async fn close(&self) {
        let Some(mut writer) = self.writer.lock().await.take() else {
            return;
        };
        if let Err(error) = writer.close().await {
            log::debug!("Closing ACP agent stdin failed: {error}");
        }
    }
}

fn response_message(id: Value, response: Result<Value, RpcError>) -> Value {
    match response {
        Ok(result) => json!({
            "jsonrpc": JSON_RPC_VERSION,
            "id": id,
            "result": result,
        }),
        Err(error) => json!({
            "jsonrpc": JSON_RPC_VERSION,
            "id": id,
            "error": { "code": error.code, "message": error.message },
        }),
    }
}

/// Registers an agent→client request and answers it from a task of its own. Whichever side
/// removes the entry from `inbound` (the handler finishing or an out-of-band answer) is the one
/// that writes the response, so the agent never sees two.
fn dispatch_inbound_request(
    wire_id: Value,
    method: String,
    params: Value,
    writer: &SharedWriter,
    inbound: &InboundRequests,
    handler: &AgentRequestHandler,
) {
    let id = InboundRequestId::new(&wire_id);
    let session_id = params
        .get("sessionId")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let tool_call_id = params
        .pointer("/toolCall/toolCallId")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let (abort_handle, abort_registration) = AbortHandle::new_pair();
    let answer = Abortable::new(handler(&method, params), abort_registration);
    inbound.lock().insert(
        id.clone(),
        InboundRequestEntry {
            request: PendingInboundRequest {
                id: id.clone(),
                method: method.clone(),
                session_id,
                tool_call_id,
            },
            wire_id,
            handler: abort_handle,
        },
    );
    let writer = writer.clone();
    let inbound = inbound.clone();
    // The read loop runs on the background runtime, so the request's task joins it there.
    tokio::spawn(async move {
        let Ok(response) = answer.await else {
            return;
        };
        let Some(entry) = inbound.lock().remove(&id) else {
            return;
        };
        if let Err(error) = write_message(&writer, &response_message(entry.wire_id, response)).await
        {
            log::warn!("Failed to answer ACP agent request {method}: {error}");
        }
    });
}

async fn write_message(writer: &SharedWriter, message: &Value) -> Result<()> {
    let mut encoded = serde_json::to_string(message)?;
    encoded.push('\n');
    let mut guard = writer.lock().await;
    let writer = guard
        .as_mut()
        .ok_or_else(|| anyhow!("ACP connection is closed"))?;
    writer.write_all(encoded.as_bytes()).await?;
    writer.flush().await?;
    Ok(())
}

async fn read_loop<R: AsyncRead + Unpin>(
    mut reader: BufReader<R>,
    writer: SharedWriter,
    pending: PendingRequests,
    inbound: InboundRequests,
    notifications: async_channel::Sender<InboundNotification>,
    handler: AgentRequestHandler,
) {
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) => break,
            Ok(_) => {}
            Err(error) => {
                log::warn!("Failed to read from ACP agent: {error}");
                break;
            }
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let message: Value = match serde_json::from_str(trimmed) {
            Ok(message) => message,
            Err(error) => {
                log::warn!("Ignoring non-JSON line from ACP agent: {error}");
                continue;
            }
        };
        let id = message.get("id").filter(|id| !id.is_null()).cloned();
        let method = message
            .get("method")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        match (id, method) {
            (Some(id), Some(method)) => {
                dispatch_inbound_request(id, method, params, &writer, &inbound, &handler);
            }
            (Some(id), None) => {
                let Some(id) = id.as_i64() else {
                    log::warn!("ACP response with non-numeric id {id}");
                    continue;
                };
                let error = message.get("error").filter(|error| !error.is_null());
                let result = match (message.get("result"), error) {
                    (_, Some(error)) => {
                        Err(match serde_json::from_value::<RpcError>(error.clone()) {
                            Ok(error) => anyhow!(error),
                            Err(_) => anyhow!("Malformed ACP error response"),
                        })
                    }
                    (Some(result), None) => Ok(result.clone()),
                    (None, None) => Ok(Value::Null),
                };
                if let Some(tx) = pending.lock().await.remove(&id) {
                    let _ = tx.send(result);
                }
            }
            (None, Some(method)) => {
                if notifications
                    .send(InboundNotification { method, params })
                    .await
                    .is_err()
                {
                    break;
                }
            }
            (None, None) => log::warn!("Ignoring ACP message with neither id nor method"),
        }
    }
    for (_, tx) in pending.lock().await.drain() {
        let _ = tx.send(Err(anyhow!("ACP agent closed its stdout")));
    }
    // Nothing is listening for answers any more; let the handlers stop where they are.
    for (_, entry) in inbound.lock().drain() {
        entry.handler.abort();
    }
}

#[cfg(test)]
#[path = "connection_tests.rs"]
mod tests;
