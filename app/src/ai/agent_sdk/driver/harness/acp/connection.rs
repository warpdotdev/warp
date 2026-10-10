//! JSON-RPC 2.0 with an ACP agent, framed as one JSON object per line.
//!
//! Inbound traffic is dispatched in arrival order: responses resolve their pending request,
//! agent→client requests are answered synchronously through the installed handler, and
//! notifications are queued for the owner to consume. Because the read loop pushes a
//! notification before it reads the next line, every notification that precedes a response on
//! the wire is queued by the time that response resolves.
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use anyhow::{Context as _, Result, anyhow};
use futures::channel::oneshot;
use futures::future::BoxFuture;
use futures::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, BufWriter};
use futures::lock::Mutex as AsyncMutex;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use warpui::r#async::executor::{Background, BackgroundTask};

const JSON_RPC_VERSION: &str = "2.0";
const METHOD_NOT_FOUND: i64 = -32601;
const INTERNAL_ERROR: i64 = -32603;

/// A JSON-RPC error returned to the agent for a request it made to us.
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

/// Answers requests the agent sends to the client (permissions, file system access). Runs on
/// the connection's background read loop, so blocking work must be offloaded.
pub(super) type AgentRequestHandler =
    Arc<dyn Fn(&str, Value) -> BoxFuture<'static, Result<Value, RpcError>> + Send + Sync>;

/// A notification received from the agent.
pub(super) struct InboundNotification {
    pub method: String,
    pub params: Value,
}

type PendingRequests = Arc<AsyncMutex<HashMap<i64, oneshot::Sender<Result<Value>>>>>;
type BoxedWriter = Pin<Box<dyn AsyncWrite + Send>>;
/// `None` once the connection has been closed; dropping the writer is what signals EOF to the
/// agent.
type SharedWriter = Arc<AsyncMutex<Option<BufWriter<BoxedWriter>>>>;

pub(super) struct AcpConnection {
    writer: SharedWriter,
    pending: PendingRequests,
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
        let (notification_tx, notification_rx) = async_channel::unbounded();

        let reader_task = executor.spawn(read_loop(
            BufReader::new(reader),
            writer.clone(),
            pending.clone(),
            notification_tx,
            handler,
        ));

        (
            Self {
                writer,
                pending,
                next_request_id: AtomicI64::new(1),
                _reader_task: reader_task,
            },
            notification_rx,
        )
    }

    /// Sends a request and waits for the agent's response, deserialized as `R`.
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
                let response = match handler(&method, params).await {
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
                };
                if let Err(error) = write_message(&writer, &response).await {
                    log::warn!("Failed to answer ACP agent request {method}: {error}");
                }
            }
            (Some(id), None) => {
                let Some(id) = id.as_i64() else {
                    log::warn!("ACP response with non-numeric id {id}");
                    continue;
                };
                let error = message.get("error").filter(|error| !error.is_null());
                let result = match (message.get("result"), error) {
                    (_, Some(error)) => Err(anyhow!("ACP error response: {error}")),
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
}

#[cfg(test)]
#[path = "connection_tests.rs"]
mod tests;
