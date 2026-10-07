use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use futures::future::{pending, ready};
use rmcp::model::{CallToolRequestParams, ErrorData};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::{ServiceError, ServiceExt};
use serde_json::{Value, json};
use tokio::sync::{Notify, oneshot};
use tokio::task::JoinHandle;
use uuid::Uuid;

use super::call_tool_with_deadline_inner;

#[derive(Clone, Copy)]
enum Reply {
    Success,
    SseSuccess,
    ToolError,
    ProtocolError,
    HttpError,
    WrongId,
    EmptySse,
    IncompleteSse,
    ResumableSse,
    StuckSend,
}

struct ServerState {
    reply: Reply,
    calls: AtomicUsize,
    cancellations: AtomicUsize,
    resumes: AtomicUsize,
    request_seen: Notify,
    request_id: Mutex<Option<Value>>,
}

struct TestServer {
    uri: String,
    state: Arc<ServerState>,
    task: JoinHandle<()>,
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn start_server(reply: Reply) -> TestServer {
    let state = Arc::new(ServerState {
        reply,
        calls: AtomicUsize::new(0),
        cancellations: AtomicUsize::new(0),
        resumes: AtomicUsize::new(0),
        request_seen: Notify::new(),
        request_id: Mutex::new(None),
    });
    let router = Router::new()
        .route(
            "/mcp",
            post(handle_post)
                .get(handle_get)
                .delete(|| async { StatusCode::OK }),
        )
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let uri = format!("http://{}/mcp", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    TestServer { uri, state, task }
}

fn result(id: Value, is_error: bool) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {"content": [], "isError": is_error}
    })
}

fn sse(body: String) -> Response {
    ([("content-type", "text/event-stream")], Body::from(body)).into_response()
}

async fn handle_post(
    State(state): State<Arc<ServerState>>,
    Json(request): Json<Value>,
) -> Response {
    match request["method"].as_str().unwrap() {
        "initialize" => {
            let mut response = Json(json!({
                "jsonrpc": "2.0",
                "id": request["id"],
                "result": {
                    "protocolVersion": "2025-03-26",
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name": "deadline-test", "version": "1"}
                }
            }))
            .into_response();
            if matches!(state.reply, Reply::ResumableSse) {
                response
                    .headers_mut()
                    .insert("mcp-session-id", "test-session".parse().unwrap());
            }
            response
        }
        "notifications/initialized" => StatusCode::ACCEPTED.into_response(),
        "notifications/cancelled" => {
            state.cancellations.fetch_add(1, Ordering::SeqCst);
            StatusCode::ACCEPTED.into_response()
        }
        "tools/call" => {
            state.calls.fetch_add(1, Ordering::SeqCst);
            *state.request_id.lock().unwrap() = Some(request["id"].clone());
            state.request_seen.notify_one();
            if request["params"]["name"] == "succeed" {
                return Json(result(request["id"].clone(), false)).into_response();
            }
            match state.reply {
                Reply::Success => Json(result(request["id"].clone(), false)).into_response(),
                Reply::SseSuccess => sse(format!(
                    "data: {}\n\n",
                    result(request["id"].clone(), false)
                )),
                Reply::ToolError => Json(result(request["id"].clone(), true)).into_response(),
                Reply::ProtocolError => Json(json!({
                    "jsonrpc": "2.0",
                    "id": request["id"],
                    "error": {"code": -32602, "message": "test protocol error"}
                }))
                .into_response(),
                Reply::HttpError => StatusCode::BAD_GATEWAY.into_response(),
                Reply::WrongId => Json(result(json!("unrelated"), false)).into_response(),
                Reply::EmptySse => sse(String::new()),
                Reply::IncompleteSse => sse(format!(
                    "data: {}\n\nevent: message\ndata: {{\"jsonrpc\":\"2.0\"",
                    progress(&request),
                )),
                Reply::ResumableSse => sse(format!(
                    "id: resume-1\nretry: 0\ndata: {}\n\n",
                    progress(&request),
                )),
                Reply::StuckSend => pending().await,
            }
        }
        _ => StatusCode::BAD_REQUEST.into_response(),
    }
}

fn progress(request: &Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": "notifications/progress",
        "params": {
            "progressToken": request["params"]["_meta"]["progressToken"],
            "progress": 1
        }
    })
}

async fn handle_get(State(state): State<Arc<ServerState>>, headers: HeaderMap) -> Response {
    if headers.get("last-event-id").is_none() {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }
    state.resumes.fetch_add(1, Ordering::SeqCst);
    let id = state.request_id.lock().unwrap().clone().unwrap();
    sse(format!("data: {}\n\n", result(id, false)))
}

#[tokio::test]
async fn deadline_includes_connecting_without_dispatch() {
    let result = call_tool_with_deadline_inner(
        Uuid::nil(),
        Uuid::nil(),
        CallToolRequestParams::new("test"),
        pending(),
        ready(()),
        || ready(()),
    )
    .await;
    assert!(result.unwrap_err().to_string().contains("not dispatched"));
}

#[tokio::test]
async fn connection_error_is_preserved() {
    let result = call_tool_with_deadline_inner(
        Uuid::nil(),
        Uuid::nil(),
        CallToolRequestParams::new("test"),
        ready(Err(ServiceError::McpError(ErrorData::internal_error(
            "test connection error",
            None,
        )))),
        pending(),
        || ready(()),
    )
    .await;
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("test connection error")
    );
}

#[tokio::test]
async fn successful_and_error_results_complete_without_retry() {
    for reply in [
        Reply::Success,
        Reply::SseSuccess,
        Reply::ToolError,
        Reply::ProtocolError,
        Reply::HttpError,
    ] {
        let server = start_server(reply).await;
        let service =
            ().serve(StreamableHttpClientTransport::from_uri(server.uri.clone()))
                .await
                .unwrap();
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            call_tool_with_deadline_inner(
                Uuid::nil(),
                Uuid::nil(),
                CallToolRequestParams::new("test"),
                ready(Ok(service.peer().clone())),
                pending(),
                || ready(()),
            ),
        )
        .await
        .unwrap();
        match reply {
            Reply::Success | Reply::SseSuccess => {
                assert_eq!(result.unwrap().is_error, Some(false))
            }
            Reply::ToolError => assert_eq!(result.unwrap().is_error, Some(true)),
            Reply::ProtocolError => assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("test protocol error")
            ),
            Reply::HttpError => assert!(result.is_err()),
            _ => unreachable!(),
        }
        assert_eq!(server.state.calls.load(Ordering::SeqCst), 1);
        assert_eq!(server.state.cancellations.load(Ordering::SeqCst), 0);
        service.cancel().await.unwrap();
    }
}
#[tokio::test]
async fn wrong_response_id_returns_outcome_unknown_at_deadline() {
    let server = start_server(Reply::WrongId).await;
    let service =
        ().serve(StreamableHttpClientTransport::from_uri(server.uri.clone()))
            .await
            .unwrap();
    let (expire, deadline) = oneshot::channel();
    let task = tokio::spawn(call_tool_with_deadline_inner(
        Uuid::nil(),
        Uuid::nil(),
        CallToolRequestParams::new("test"),
        ready(Ok(service.peer().clone())),
        async { deadline.await.unwrap() },
        pending,
    ));
    server.state.request_seen.notified().await;
    expire.send(()).unwrap();
    let result = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap();
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("outcome is unknown")
    );
    assert_eq!(server.state.calls.load(Ordering::SeqCst), 1);
    assert_eq!(server.state.cancellations.load(Ordering::SeqCst), 1);
    service.cancel().await.unwrap();
}

#[tokio::test]
async fn ended_sse_without_result_fails_without_waiting_for_deadline() {
    for reply in [Reply::EmptySse, Reply::IncompleteSse] {
        let server = start_server(reply).await;
        let service =
            ().serve(StreamableHttpClientTransport::from_uri(server.uri.clone()))
                .await
                .unwrap();
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            call_tool_with_deadline_inner(
                Uuid::nil(),
                Uuid::nil(),
                CallToolRequestParams::new("test"),
                ready(Ok(service.peer().clone())),
                pending(),
                pending,
            ),
        )
        .await
        .expect("a terminated response must not leave the tool call pending");
        assert!(matches!(result, Err(ServiceError::TransportClosed)));
        assert_eq!(server.state.calls.load(Ordering::SeqCst), 1);
        assert_eq!(server.state.cancellations.load(Ordering::SeqCst), 0);
        assert!(!service.is_transport_closed());
        service.cancel().await.unwrap();
    }
}

#[tokio::test]
async fn timed_out_request_does_not_close_the_connection_or_replay_the_tool() {
    let server = start_server(Reply::WrongId).await;
    let service =
        ().serve(StreamableHttpClientTransport::from_uri(server.uri.clone()))
            .await
            .unwrap();
    let peer = service.peer().clone();
    let (expire, deadline) = oneshot::channel();
    let task = tokio::spawn(call_tool_with_deadline_inner(
        Uuid::nil(),
        Uuid::nil(),
        CallToolRequestParams::new("test"),
        ready(Ok(peer.clone())),
        async { deadline.await.unwrap() },
        pending,
    ));
    server.state.request_seen.notified().await;
    let succeeding = call_tool_with_deadline_inner(
        Uuid::nil(),
        Uuid::nil(),
        CallToolRequestParams::new("succeed"),
        ready(Ok(peer)),
        pending(),
        pending,
    );
    assert!(
        tokio::time::timeout(Duration::from_secs(5), succeeding)
            .await
            .unwrap()
            .is_ok()
    );
    expire.send(()).unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    assert_eq!(server.state.calls.load(Ordering::SeqCst), 2);
    assert_eq!(server.state.cancellations.load(Ordering::SeqCst), 1);
    assert!(!service.is_transport_closed());
    service.cancel().await.unwrap();
}

#[tokio::test]
async fn stalled_send_cannot_block_deadline_or_cancellation() {
    let server = start_server(Reply::StuckSend).await;
    let service =
        ().serve(StreamableHttpClientTransport::from_uri(server.uri.clone()))
            .await
            .unwrap();
    let peer = service.peer().clone();
    let (expire, deadline) = oneshot::channel();
    let task = tokio::spawn(call_tool_with_deadline_inner(
        Uuid::nil(),
        Uuid::nil(),
        CallToolRequestParams::new("test"),
        ready(Ok(peer)),
        async { deadline.await.unwrap() },
        || ready(()),
    ));
    server.state.request_seen.notified().await;
    expire.send(()).unwrap();
    let result = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap();
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("outcome is unknown")
    );
    assert_eq!(server.state.calls.load(Ordering::SeqCst), 1);
    assert_eq!(server.state.cancellations.load(Ordering::SeqCst), 0);
    service.cancellation_token().cancel();
}

#[tokio::test]
async fn resumable_sse_keeps_waiting_for_the_matching_result() {
    let server = start_server(Reply::ResumableSse).await;
    let service =
        ().serve(StreamableHttpClientTransport::from_uri(server.uri.clone()))
            .await
            .unwrap();
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        call_tool_with_deadline_inner(
            Uuid::nil(),
            Uuid::nil(),
            CallToolRequestParams::new("test"),
            ready(Ok(service.peer().clone())),
            pending(),
            || ready(()),
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(result.is_error, Some(false));
    assert_eq!(server.state.calls.load(Ordering::SeqCst), 1);
    assert_eq!(server.state.resumes.load(Ordering::SeqCst), 1);
    service.cancel().await.unwrap();
}
