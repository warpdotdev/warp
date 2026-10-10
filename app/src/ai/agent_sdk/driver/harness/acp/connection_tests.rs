use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use futures::channel::mpsc;
use futures::executor::block_on;
use futures::io::{AsyncBufReadExt as _, AsyncRead, AsyncWrite, AsyncWriteExt as _, BufReader};
use futures::{FutureExt as _, StreamExt as _, future, join};
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
