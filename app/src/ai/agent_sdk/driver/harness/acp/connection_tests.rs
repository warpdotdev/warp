use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use futures::channel::{mpsc, oneshot};
use futures::executor::block_on;
use futures::io::{AsyncBufReadExt as _, AsyncRead, AsyncWrite, AsyncWriteExt as _, BufReader};
use futures::{FutureExt as _, StreamExt as _, future, join};
use parking_lot::Mutex;
use serde_json::{Value, json};
use warpui::r#async::executor::Background;

use super::{AcpConnection, AgentRequestHandler, InboundNotification, RpcError};

/// One direction of an in-memory pipe.
fn pipe() -> (PipeWriter, PipeReader) {
    let (tx, rx) = mpsc::unbounded();
    (
        PipeWriter(tx),
        PipeReader {
            rx,
            pending: Vec::new(),
        },
    )
}

struct PipeWriter(mpsc::UnboundedSender<Vec<u8>>);

impl AsyncWrite for PipeWriter {
    fn poll_write(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        self.0
            .unbounded_send(buf.to_vec())
            .map_err(|_| std::io::Error::from(std::io::ErrorKind::BrokenPipe))?;
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        self.0.close_channel();
        Poll::Ready(Ok(()))
    }
}

struct PipeReader {
    rx: mpsc::UnboundedReceiver<Vec<u8>>,
    pending: Vec<u8>,
}

impl AsyncRead for PipeReader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<std::io::Result<usize>> {
        if self.pending.is_empty() {
            match self.rx.poll_next_unpin(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(None) => return Poll::Ready(Ok(0)),
                Poll::Ready(Some(bytes)) => self.pending = bytes,
            }
        }
        let n = buf.len().min(self.pending.len());
        buf[..n].copy_from_slice(&self.pending[..n]);
        self.pending.drain(..n);
        Poll::Ready(Ok(n))
    }
}

/// The agent's end of the wire: reads what the client wrote and writes raw lines back.
struct FakeAgent {
    from_client: BufReader<PipeReader>,
    to_client: PipeWriter,
    /// Runs the connection's read loop; dropping it would cancel the loop.
    _executor: Arc<Background>,
}

impl FakeAgent {
    async fn next_message(&mut self) -> Value {
        let mut line = String::new();
        self.from_client.read_line(&mut line).await.unwrap();
        serde_json::from_str(&line).expect("client must write one JSON object per line")
    }

    async fn send(&mut self, message: Value) {
        let mut encoded = serde_json::to_string(&message).unwrap();
        encoded.push('\n');
        self.to_client.write_all(encoded.as_bytes()).await.unwrap();
    }
}

fn connect(
    handler: AgentRequestHandler,
) -> (
    AcpConnection,
    async_channel::Receiver<InboundNotification>,
    FakeAgent,
) {
    let (client_writer, agent_reader) = pipe();
    let (agent_writer, client_reader) = pipe();
    let executor = Arc::new(Background::new(1, |_| "acp-connection-test".to_string()));
    let (connection, notifications) =
        AcpConnection::new(client_reader, client_writer, handler, &executor);
    (
        connection,
        notifications,
        FakeAgent {
            from_client: BufReader::new(agent_reader),
            to_client: agent_writer,
            _executor: executor,
        },
    )
}

fn rejecting_handler() -> AgentRequestHandler {
    Arc::new(|method, _| future::ready(Err(RpcError::method_not_found(method))).boxed())
}

/// A handler whose permission answers wait until the test releases them; every other method
/// is rejected. Each `session/request_permission` parks a sender in `gates`, in arrival order.
fn gated_permission_handler() -> (AgentRequestHandler, Arc<Mutex<Vec<oneshot::Sender<Value>>>>) {
    let gates: Arc<Mutex<Vec<oneshot::Sender<Value>>>> = Arc::default();
    let handler_gates = gates.clone();
    let handler: AgentRequestHandler = Arc::new(move |method, _| match method {
        "session/request_permission" => {
            let (tx, rx) = oneshot::channel();
            handler_gates.lock().push(tx);
            async move {
                rx.await
                    .map_err(|_| RpcError::internal("permission gate dropped"))
            }
            .boxed()
        }
        other => future::ready(Err(RpcError::method_not_found(other))).boxed(),
    });
    (handler, gates)
}

fn permission_request(id: Value, tool_call_id: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "session/request_permission",
        "params": {
            "sessionId": "sess_1",
            "toolCall": {"toolCallId": tool_call_id, "title": "Run"},
            "options": [{"optionId": "allow", "name": "Allow", "kind": "allow_once"}],
        },
    })
}

/// Polls until `connection` reports `count` outstanding requests, since dispatch happens on
/// the read loop's own thread.
async fn wait_for_pending(connection: &AcpConnection, count: usize) {
    for _ in 0..200 {
        if connection.pending_inbound_requests().len() == count {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!(
        "expected {count} pending inbound requests, found {:?}",
        connection.pending_inbound_requests()
    );
}

#[test]
fn responses_resolve_their_own_request_regardless_of_arrival_order() {
    let (connection, _notifications, mut agent) = connect(rejecting_handler());
    block_on(async {
        let first = connection.request::<_, Value>("first", json!({}));
        let second = connection.request::<_, Value>("second", json!({}));
        let agent = async {
            let a = agent.next_message().await;
            let b = agent.next_message().await;
            assert_eq!(a["method"], "first");
            assert_eq!(b["method"], "second");
            assert_eq!(a["jsonrpc"], "2.0");
            agent
                .send(json!({"jsonrpc": "2.0", "id": b["id"], "result": {"which": "second"}}))
                .await;
            agent
                .send(json!({"jsonrpc": "2.0", "id": a["id"], "result": {"which": "first"}}))
                .await;
        };
        let (first, second, ()) = join!(first, second, agent);
        assert_eq!(first.unwrap()["which"], "first");
        assert_eq!(second.unwrap()["which"], "second");
    });
}

#[test]
fn notifications_preceding_a_response_are_queued_before_it_resolves() {
    let (connection, notifications, mut agent) = connect(rejecting_handler());
    block_on(async {
        let request = connection.request::<_, Value>("session/prompt", json!({}));
        let agent = async {
            let message = agent.next_message().await;
            agent
                .send(json!({"jsonrpc": "2.0", "method": "session/update", "params": {"n": 1}}))
                .await;
            agent
                .send(json!({"jsonrpc": "2.0", "method": "session/update", "params": {"n": 2}}))
                .await;
            agent
                .send(json!({"jsonrpc": "2.0", "id": message["id"], "result": null}))
                .await;
        };
        let (response, ()) = join!(request, agent);
        assert_eq!(response.unwrap(), Value::Null);
        let first = notifications.try_recv().unwrap();
        let second = notifications.try_recv().unwrap();
        assert_eq!(first.method, "session/update");
        assert_eq!(first.params["n"], 1);
        assert_eq!(second.params["n"], 2);
        assert!(notifications.try_recv().is_err());
    });
}

#[test]
fn authentication_error_code_survives_request_context() {
    let (connection, _notifications, mut agent) = connect(rejecting_handler());
    block_on(async {
        let request = connection.request::<_, Value>("session/new", json!({}));
        let agent = async {
            let message = agent.next_message().await;
            agent
                .send(json!({
                    "jsonrpc": "2.0",
                    "id": message["id"],
                    "error": {"code": -32000, "message": "Authentication required"},
                }))
                .await;
        };
        let (response, ()) = join!(request, agent);
        let error = response.unwrap_err().context("session/new failed");
        assert_eq!(error.downcast_ref::<RpcError>().unwrap().code, -32000);
    });
}

#[test]
fn error_responses_surface_as_request_errors() {
    let (connection, _notifications, mut agent) = connect(rejecting_handler());
    block_on(async {
        let request = connection.request::<_, Value>("initialize", json!({}));
        let agent = async {
            let message = agent.next_message().await;
            agent
                .send(json!({
                    "jsonrpc": "2.0",
                    "id": message["id"],
                    "error": {"code": -32600, "message": "bad request"},
                }))
                .await;
        };
        let (response, ()) = join!(request, agent);
        let error = response.unwrap_err().to_string();
        assert!(error.contains("bad request"), "{error}");
    });
}

#[test]
fn agent_requests_are_answered_through_the_handler() {
    let handler: AgentRequestHandler = Arc::new(|method, params| {
        let answer = match method {
            "fs/read_text_file" => Ok(json!({"content": format!("read {}", params["path"])})),
            other => Err(RpcError::method_not_found(other)),
        };
        future::ready(answer).boxed()
    });
    let (_connection, _notifications, mut agent) = connect(handler);
    block_on(async {
        agent
            .send(json!({
                "jsonrpc": "2.0",
                "id": "agent-1",
                "method": "fs/read_text_file",
                "params": {"path": "/tmp/x"},
            }))
            .await;
        let answer = agent.next_message().await;
        assert_eq!(answer["id"], "agent-1");
        assert_eq!(answer["result"]["content"], "read \"/tmp/x\"");

        agent
            .send(json!({"jsonrpc": "2.0", "id": 7, "method": "terminal/create", "params": {}}))
            .await;
        let answer = agent.next_message().await;
        assert_eq!(answer["id"], 7);
        assert_eq!(answer["error"]["code"], -32601);
    });
}

#[test]
fn unanswered_agent_request_does_not_block_notifications_or_responses() {
    let (handler, gates) = gated_permission_handler();
    let (connection, notifications, mut agent) = connect(handler);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    runtime.block_on(async {
        let prompt = connection.request::<_, Value>("session/prompt", json!({}));
        let agent_turn = async {
            let prompt_message = agent.next_message().await;
            agent.send(permission_request(json!(1), "call-1")).await;
            wait_for_pending(&connection, 1).await;
            agent
                .send(json!({"jsonrpc": "2.0", "method": "session/update", "params": {"n": 1}}))
                .await;
            agent
                .send(json!({"jsonrpc": "2.0", "id": prompt_message["id"], "result": {"stopReason": "end_turn"}}))
                .await;
        };
        let (response, ()) = join!(prompt, agent_turn);
        assert_eq!(response.unwrap()["stopReason"], "end_turn");
        assert_eq!(notifications.try_recv().unwrap().params["n"], 1);

        let pending = connection.pending_inbound_requests();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].method, "session/request_permission");
        assert_eq!(pending[0].session_id.as_deref(), Some("sess_1"));
        assert_eq!(pending[0].tool_call_id.as_deref(), Some("call-1"));

        let gate = gates.lock().pop().unwrap();
        gate.send(json!({"outcome": {"outcome": "selected", "optionId": "allow"}}))
            .unwrap();
        let answer = agent.next_message().await;
        assert_eq!(answer["id"], 1);
        assert_eq!(answer["result"]["outcome"]["optionId"], "allow");
        wait_for_pending(&connection, 0).await;
    });
}

#[test]
fn cancelling_answers_pending_permission_requests_and_discards_the_handlers_answer() {
    let (handler, gates) = gated_permission_handler();
    let (connection, _notifications, mut agent) = connect(handler);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    runtime.block_on(async {
        agent.send(permission_request(json!(1), "call-1")).await;
        agent.send(permission_request(json!("p-2"), "call-2")).await;
        agent
            .send(json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "session/request_permission",
                "params": {"sessionId": "other-session", "toolCall": {"toolCallId": "call-3"}, "options": []},
            }))
            .await;
        wait_for_pending(&connection, 3).await;

        assert_eq!(connection.cancel_pending_permission_requests("sess_1").await, 2);

        let mut answered_ids = vec![agent.next_message().await, agent.next_message().await]
            .into_iter()
            .map(|answer| {
                assert_eq!(answer["result"]["outcome"]["outcome"], "cancelled");
                answer["id"].to_string()
            })
            .collect::<Vec<_>>();
        answered_ids.sort();
        assert_eq!(answered_ids, vec!["\"p-2\"".to_owned(), "1".to_owned()]);

        let pending = connection.pending_inbound_requests();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].session_id.as_deref(), Some("other-session"));

        // The handler's late answers for the cancelled requests must not reach the agent; the
        // one for the other session still does.
        for gate in gates.lock().drain(..) {
            let _ = gate.send(json!({"outcome": {"outcome": "selected", "optionId": "allow"}}));
        }
        let answer = agent.next_message().await;
        assert_eq!(answer["id"], 3);
        assert_eq!(answer["result"]["outcome"]["optionId"], "allow");
        wait_for_pending(&connection, 0).await;
        assert_eq!(connection.cancel_pending_permission_requests("sess_1").await, 0);
    });
}

#[test]
fn agent_eof_fails_pending_requests_and_closes_the_notification_stream() {
    let (connection, notifications, agent) = connect(rejecting_handler());
    let _keep_read_loop_alive = agent._executor.clone();
    block_on(async {
        let request = connection.request::<_, Value>("session/prompt", json!({}));
        let agent = async move {
            let mut agent = agent;
            agent.next_message().await;
            drop(agent);
        };
        let (response, ()) = join!(request, agent);
        let error = response.unwrap_err().to_string();
        assert!(error.contains("closed"), "{error}");
        assert!(notifications.recv().await.is_err());
    });
}

#[test]
fn requests_after_close_fail_immediately() {
    let (connection, _notifications, _agent) = connect(rejecting_handler());
    block_on(async {
        connection.close().await;
        let error = connection
            .request::<_, Value>("session/prompt", json!({}))
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("session/prompt"), "{error}");
    });
}
