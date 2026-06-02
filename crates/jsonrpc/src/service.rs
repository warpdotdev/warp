use std::collections::HashMap;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Result};
use futures::channel::oneshot;
use futures::lock::Mutex as AsyncMutex;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use serde_json::Value;
use warpui_core::r#async::executor::Background;

use crate::transport::Transport;

pub const JSON_RPC_VERSION: &str = "2.0";

// Technically, this could be either a string or an integer, but since the client always
// sets the ID, we're just going to use integers.
pub type RequestId = i32;

fn is_null_value<T: Serialize>(value: &T) -> bool {
    matches!(serde_json::to_value(value), Ok(Value::Null))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request<T> {
    pub jsonrpc: &'static str,
    pub id: RequestId,
    pub method: String,
    #[serde(skip_serializing_if = "is_null_value")]
    pub params: T,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification<T> {
    pub jsonrpc: &'static str,
    pub method: String,
    #[serde(skip_serializing_if = "is_null_value")]
    pub params: T,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnyNotification<'a> {
    pub jsonrpc: String,
    pub method: &'a str,
    #[serde(borrow)]
    pub params: Option<&'a RawValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnyRequest<'a> {
    pub jsonrpc: String,
    pub method: &'a str,
    #[serde(borrow)]
    pub params: Option<&'a RawValue>,
    pub id: RequestId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnyResponse<'a> {
    pub jsonrpc: String,
    #[serde(borrow)]
    pub result: Option<&'a RawValue>,
    pub error: Option<&'a RawValue>,
    pub id: RequestId,
}

pub struct ServerNotificationEvent {
    pub method: String,
    pub params: Value,
}

type Subscription = async_channel::Sender<ServerNotificationEvent>;

type ServerRequestHandler = Arc<dyn Fn(String, Value, RequestId) -> Result<()> + Send + Sync>;

pub struct JsonRpcService {
    transport: Arc<dyn Transport>,
    request_id_counter: AtomicI32,
    pending_requests: Arc<AsyncMutex<HashMap<RequestId, oneshot::Sender<Result<Value>>>>>,
    notification_subscriptions: Arc<AsyncMutex<HashMap<String, Subscription>>>,
    server_request_handler: Arc<Mutex<Option<ServerRequestHandler>>>,
    executor: Arc<Background>,
}

impl JsonRpcService {
    pub fn new(
        transport: Box<dyn Transport>,
        executor: Arc<Background>,
        // Take an explicit error code to report to the server for unhandled
        // server -> client requests.
        request_error_code: i64,
    ) -> Self {
        let transport: Arc<dyn Transport> = transport.into();
        let pending_requests = Arc::new(AsyncMutex::new(HashMap::new()));
        let notification_subscriptions = Arc::new(AsyncMutex::new(HashMap::new()));
        let server_request_handler = Arc::new(Mutex::new(None));

        let transport_clone = transport.clone();
        let pending_requests_clone = pending_requests.clone();
        let notification_subscriptions_clone = notification_subscriptions.clone();
        let server_request_handler_clone = server_request_handler.clone();

        executor
            .spawn(async move {
                if let Err(e) = Self::read_loop(
                    transport_clone,
                    pending_requests_clone,
                    notification_subscriptions_clone,
                    server_request_handler_clone,
                    request_error_code,
                )
                .await
                {
                    log::error!("JSON-RPC read loop error: {e}");
                }
            })
            .detach();

        Self {
            transport,
            request_id_counter: AtomicI32::new(1),
            pending_requests,
            notification_subscriptions,
            server_request_handler,
            executor,
        }
    }

    /// Installs a best-effort handler for server -> client requests.
    pub fn set_server_request_handler(
        &self,
        handler: impl Fn(String, Value, RequestId) -> Result<()> + Send + Sync + 'static,
    ) {
        let mut guard = self
            .server_request_handler
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        *guard = Some(Arc::new(handler));
    }

    /// Background loop that reads complete messages from the transport and dispatches them.
    async fn read_loop(
        transport: Arc<dyn Transport>,
        pending_requests: Arc<AsyncMutex<HashMap<RequestId, oneshot::Sender<Result<Value>>>>>,
        notification_subscriptions: Arc<AsyncMutex<HashMap<String, Subscription>>>,
        server_request_handler: Arc<Mutex<Option<ServerRequestHandler>>>,
        request_error_code: i64,
    ) -> Result<()> {
        loop {
            let message = transport.read().await?;
            if message.is_empty() {
                log::debug!("JSON-RPC transport closed");
                break;
            }

            log::trace!("JSON-RPC: received message: {message}");
            if let Err(e) = Self::handle_message(
                &transport,
                &message,
                &pending_requests,
                &notification_subscriptions,
                &server_request_handler,
                request_error_code,
            )
            .await
            {
                log::warn!("Failed to dispatch message: {e}");
            }
        }

        Ok(())
    }

    /// Dispatches a complete JSON-RPC message to the appropriate handler.
    async fn handle_message(
        transport: &Arc<dyn Transport>,
        message: &str,
        pending_requests: &AsyncMutex<HashMap<RequestId, oneshot::Sender<Result<Value>>>>,
        notification_subscriptions: &AsyncMutex<HashMap<String, Subscription>>,
        server_request_handler: &Mutex<Option<ServerRequestHandler>>,
        request_error_code: i64,
    ) -> Result<()> {
        if let Ok(request) = serde_json::from_str::<AnyRequest>(message) {
            let should_ack = matches!(
                request.method,
                "window/workDoneProgress/create"
                    | "client/registerCapability"
                    | "client/unregisterCapability"
            );

            // Handle specific server -> client requests that we can safely acknowledge.
            // Some LSP servers crash if we return an error.
            let response = if should_ack {
                log::debug!("Acknowledging {} request", request.method);
                serde_json::json!({
                    "jsonrpc": JSON_RPC_VERSION,
                    "id": request.id,
                    "result": null
                })
            } else {
                // For other requests, return an error
                log::debug!(
                    "Returning error for unhandled server request: {}",
                    request.method
                );
                serde_json::json!({
                    "jsonrpc": JSON_RPC_VERSION,
                    "id": request.id,
                    "error": {
                        "code": request_error_code,
                        "message": format!("Method {} not implemented", request.method),
                    }
                })
            };
            let content = serde_json::to_string(&response)?;
            transport.write(&content).await?;

            if should_ack {
                let handler = server_request_handler
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone();

                if let Some(handler) = handler {
                    let params = if let Some(params) = request.params {
                        match serde_json::from_str(params.get()) {
                            Ok(value) => value,
                            Err(e) => {
                                log::warn!(
                                    "Failed to parse params for {} request: {e}",
                                    request.method
                                );
                                Value::Null
                            }
                        }
                    } else {
                        Value::Null
                    };

                    if let Err(e) = handler(request.method.to_string(), params, request.id) {
                        log::warn!("Server request handler error for {}: {e}", request.method);
                    }
                }
            }

            return Ok(());
        }

        if let Ok(response) = serde_json::from_str::<AnyResponse>(message) {
            let mut pending = pending_requests.lock().await;
            if let Some(tx) = pending.remove(&response.id) {
                let result = if let Some(result) = response.result {
                    serde_json::from_str(result.get())
                        .map_err(|e| anyhow!("Failed to parse JSON-RPC response result: {e}"))
                } else if let Some(error) = response.error {
                    Err(anyhow!("JSON-RPC error: {}", error.get()))
                } else {
                    Ok(Value::Null)
                };
                let _ = tx.send(result);
            }
            return Ok(());
        }

        if let Ok(notification) = serde_json::from_str::<AnyNotification>(message) {
            let params = if let Some(params) = notification.params {
                serde_json::from_str(params.get())?
            } else {
                Value::Null
            };
            Self::handle_notification(notification.method, params, notification_subscriptions)
                .await;
            return Ok(());
        }

        log::warn!("Received unsupported JSON-RPC message - likely a server-to-client request.");

        Ok(())
    }

    async fn handle_notification(
        method: &str,
        params: Value,
        notification_subscriptions: &AsyncMutex<HashMap<String, Subscription>>,
    ) {
        let subs = notification_subscriptions.lock().await;
        if let Some(subscription) = subs.get(method) {
            if let Err(e) = subscription.try_send(ServerNotificationEvent {
                method: method.to_string(),
                params,
            }) {
                log::error!("Failed to send notification: {e}");
            }
        }
    }

    /// Returns the next available request ID
    pub fn next_id(&self) -> RequestId {
        self.request_id_counter.fetch_add(1, Ordering::Relaxed)
    }

    /// Subscribe to notifications with the given key, if the key is already subscribed,
    /// this will overwrite.
    pub async fn subscribe(&self, key: String, on_notification: Subscription) {
        let mut subs = self.notification_subscriptions.lock().await;
        subs.insert(key, on_notification);
    }

    /// Send a JSON-RPC request and wait for the response.
    pub async fn send_request(
        &self,
        request_id: RequestId,
        method: String,
        params: Value,
    ) -> Result<Value> {
        log::trace!("Sending request {request_id}: {method}: {params}");

        let request = Request {
            jsonrpc: JSON_RPC_VERSION,
            id: request_id,
            method,
            params,
        };

        let (tx, rx) = oneshot::channel();
        self.pending_requests.lock().await.insert(request_id, tx);

        let content = serde_json::to_string(&request)?;

        self.transport.write(&content).await?;

        let response_result = rx.await.map_err(|_| anyhow!("Response channel closed"))?;

        self.pending_requests.lock().await.remove(&request_id);

        response_result
    }

    /// Fire-and-forget notification
    pub fn send_notification(&self, method: String, params: Value) -> Result<()> {
        let notification = Notification {
            jsonrpc: JSON_RPC_VERSION,
            method,
            params,
        };

        let content = serde_json::to_string(&notification)?;
        let transport = self.transport.clone();
        let future = async move {
            if let Err(e) = transport.write(&content).await {
                log::error!("Failed to send notification: {e}");
            };
        };

        self.executor.spawn(future).detach();
        Ok(())
    }

    pub async fn shutdown(&self, timeout: std::time::Duration) -> Result<()> {
        self.transport.shutdown(timeout).await
    }
}

#[cfg(test)]
#[path = "service_tests.rs"]
mod tests;
