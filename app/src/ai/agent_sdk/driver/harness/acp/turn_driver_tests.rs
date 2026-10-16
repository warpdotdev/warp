use std::pin::Pin;
use std::task::{Context, Poll};

use futures::StreamExt as _;
use futures::channel::mpsc;
use futures::executor::block_on;
use futures::future::join;
use futures::io::{AsyncBufReadExt as _, AsyncRead, AsyncWrite, AsyncWriteExt as _, BufReader};
use serde_json::{Value, json};
use warp_multi_agent_api::client_action::Action;
use warp_multi_agent_api::message::Message as MessageKind;
use warp_multi_agent_api::response_event::StreamInit;
use warpui::r#async::executor::Background;

use super::super::connection::AgentRequestHandler;
use super::*;
use crate::ai::blocklist::{QueuedQuery, QueuedQueryOrigin};
use crate::server::server_api::ai::MockAIClient;

const SESSION: &str = "sess_1";

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

    /// Reads the next `session/prompt` request, failing on anything else.
    async fn next_prompt(&mut self) -> Value {
        let message = self.next_message().await;
        assert_eq!(message["method"], "session/prompt", "got {message}");
        message
    }

    /// Reads the next message and checks it is the `session/cancel` notification.
    async fn expect_cancel(&mut self) {
        let message = self.next_message().await;
        assert_eq!(message["method"], "session/cancel", "got {message}");
        assert!(message.get("id").is_none(), "cancel is a notification");
        assert_eq!(message["params"]["sessionId"], SESSION);
    }

    async fn send(&mut self, message: Value) {
        let mut encoded = serde_json::to_string(&message).unwrap();
        encoded.push('\n');
        self.to_client.write_all(encoded.as_bytes()).await.unwrap();
    }

    async fn send_chunk(&mut self, text: &str) {
        self.send(json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": SESSION,
                "update": {
                    "sessionUpdate": "agent_message_chunk",
                    "content": { "type": "text", "text": text },
                },
            },
        }))
        .await;
    }

    async fn respond(&mut self, prompt: &Value, stop_reason: &str) {
        self.send(json!({
            "jsonrpc": "2.0",
            "id": prompt["id"],
            "result": { "stopReason": stop_reason },
        }))
        .await;
    }
}

/// The text of a prompt request, as the agent would concatenate its text blocks.
fn prompt_text(prompt: &Value) -> String {
    prompt["params"]["prompt"]
        .as_array()
        .expect("prompt blocks")
        .iter()
        .filter_map(|block| block["text"].as_str())
        .collect::<Vec<_>>()
        .join("")
}

#[derive(Debug, Clone, PartialEq)]
enum HostEvent {
    BeginTurn {
        stream: ResponseStreamId,
        initiator: Option<ParticipantId>,
    },
    /// User queries and agent output texts applied to `stream`.
    Texts {
        stream: ResponseStreamId,
        texts: Vec<String>,
    },
    Finished {
        stream: ResponseStreamId,
        reason: &'static str,
        settling: bool,
    },
    MarkedCancelled {
        settling: bool,
    },
    InterruptedTerminal,
    ShowQueued(String),
    HideQueued(QueuedQueryId),
    Stopped,
}

/// Records what the turn driver asks of the driver, along with the settling flag at the
/// moments the driver's idle exit would consult it.
struct RecordingHost {
    events: Mutex<Vec<HostEvent>>,
    streams_opened: Mutex<usize>,
    state: Arc<TurnState>,
    queued_rows: Mutex<Vec<(QueuedQueryId, String)>>,
}

impl RecordingHost {
    fn new(state: Arc<TurnState>) -> Self {
        Self {
            events: Mutex::default(),
            streams_opened: Mutex::new(0),
            state,
            queued_rows: Mutex::default(),
        }
    }

    fn events(&self) -> Vec<HostEvent> {
        self.events.lock().clone()
    }

    fn record(&self, event: HostEvent) {
        self.events.lock().push(event);
    }

    /// Polls until `predicate` holds for the recorded events; the agent script uses this to
    /// act only after the driver has applied what it sent.
    async fn wait_for(&self, predicate: impl Fn(&[HostEvent]) -> bool) {
        for _ in 0..400 {
            if predicate(&self.events()) {
                return;
            }
            Timer::after(Duration::from_millis(5)).await;
        }
        panic!(
            "host never recorded the expected event: {:?}",
            self.events()
        );
    }

    fn streams(&self) -> Vec<ResponseStreamId> {
        self.events()
            .into_iter()
            .filter_map(|event| match event {
                HostEvent::BeginTurn { stream, .. } => Some(stream),
                _ => None,
            })
            .collect()
    }

    /// Every text applied to `stream`, in order.
    fn texts_on(&self, stream: &ResponseStreamId) -> Vec<String> {
        self.events()
            .into_iter()
            .filter_map(|event| match event {
                HostEvent::Texts {
                    stream: on, texts, ..
                } if &on == stream => Some(texts),
                _ => None,
            })
            .flatten()
            .collect()
    }
}

fn message_texts(actions: &[ClientAction]) -> Vec<String> {
    let mut texts = Vec::new();
    for action in actions {
        match &action.action {
            Some(Action::AddMessagesToTask(add)) => {
                for message in &add.messages {
                    match &message.message {
                        Some(MessageKind::UserQuery(query)) => texts.push(query.query.clone()),
                        Some(MessageKind::AgentOutput(output)) => texts.push(output.text.clone()),
                        _ => {}
                    }
                }
            }
            Some(Action::AppendToMessageContent(append)) => {
                if let Some(MessageKind::AgentOutput(output)) = append
                    .message
                    .as_ref()
                    .and_then(|message| message.message.as_ref())
                {
                    texts.push(output.text.clone());
                }
            }
            _ => {}
        }
    }
    texts
}

#[async_trait]
impl TurnHost for RecordingHost {
    async fn begin_turn(&self, initiator: Option<ParticipantId>) -> Result<ExternalHarnessTurn> {
        let n = {
            let mut opened = self.streams_opened.lock();
            *opened += 1;
            *opened
        };
        let request_id = format!("req-{n}");
        let stream_id = ResponseStreamId::for_shared_session(&StreamInit {
            conversation_id: "conversation".to_owned(),
            request_id: request_id.clone(),
            run_id: String::new(),
        });
        self.record(HostEvent::BeginTurn {
            stream: stream_id.clone(),
            initiator,
        });
        Ok(ExternalHarnessTurn {
            stream_id,
            request_id,
        })
    }

    async fn apply_actions(
        &self,
        stream_id: &ResponseStreamId,
        actions: Vec<ClientAction>,
    ) -> Result<()> {
        let texts = message_texts(&actions);
        if !texts.is_empty() {
            self.record(HostEvent::Texts {
                stream: stream_id.clone(),
                texts,
            });
        }
        Ok(())
    }

    async fn finish_turn(
        &self,
        stream_id: ResponseStreamId,
        finished: StreamFinished,
    ) -> Result<()> {
        let reason = match finished.reason {
            Some(stream_finished::Reason::Done(_)) => "done",
            Some(stream_finished::Reason::InternalError(_)) => "internal_error",
            _ => "other",
        };
        self.record(HostEvent::Finished {
            stream: stream_id,
            reason,
            settling: self.state.is_settling(),
        });
        Ok(())
    }

    async fn mark_conversation_cancelled(&self) {
        self.record(HostEvent::MarkedCancelled {
            settling: self.state.is_settling(),
        });
    }

    async fn capture_snapshot(&self) -> Result<Option<Vec<u8>>> {
        Ok(None)
    }

    async fn interrupt_terminal(&self) {
        self.record(HostEvent::InterruptedTerminal);
    }

    async fn show_queued_prompt(&self, prompt: &ExternalHarnessPrompt) -> Option<QueuedQueryId> {
        let row = QueuedQuery::new(prompt.text.clone(), QueuedQueryOrigin::QueueSlashCommand).id();
        self.queued_rows.lock().push((row, prompt.text.clone()));
        self.record(HostEvent::ShowQueued(prompt.text.clone()));
        Some(row)
    }

    async fn hide_queued_prompt(&self, query_id: QueuedQueryId) {
        self.queued_rows.lock().retain(|(row, _)| *row != query_id);
        self.record(HostEvent::HideQueued(query_id));
    }

    async fn turn_driver_stopped(&self) {
        self.record(HostEvent::Stopped);
    }
}

struct Fixture {
    agent: FakeAgent,
    follow_ups: async_channel::Sender<ExternalHarnessPrompt>,
    cancels: async_channel::Sender<ExternalHarnessCancel>,
    host: Arc<RecordingHost>,
    state: Arc<TurnState>,
    connection: Arc<AcpConnection>,
}

impl Fixture {
    async fn send_follow_up(&self, text: &str) -> ParticipantId {
        let participant_id = ParticipantId::new();
        self.follow_ups
            .send(ExternalHarnessPrompt {
                text: text.to_owned(),
                attachments: Vec::new(),
                participant_id: participant_id.clone(),
                base: None,
            })
            .await
            .unwrap();
        participant_id
    }

    async fn send_interrupt(&self) {
        self.cancels.send(ExternalHarnessCancel).await.unwrap();
    }

    /// Closes the controller's sinks, which ends the turn driver once it is between turns.
    fn unbind(self) -> (Arc<RecordingHost>, Arc<TurnState>) {
        let Fixture { host, state, .. } = self;
        (host, state)
    }
}

fn rejecting_handler() -> AgentRequestHandler {
    Arc::new(|method, _| futures::future::ready(Err(RpcError::method_not_found(method))).boxed())
}

/// A handler that never answers permission requests, so they stay pending until the turn
/// driver answers them itself.
fn parking_permission_handler() -> AgentRequestHandler {
    Arc::new(|method, _| match method {
        "session/request_permission" => futures::future::pending().boxed(),
        other => futures::future::ready(Err(RpcError::method_not_found(other))).boxed(),
    })
}

fn start(
    handler: AgentRequestHandler,
    cancel_deadline: Duration,
) -> (TurnDriver, AcpTurnMapper, Fixture) {
    let (client_writer, agent_reader) = pipe();
    let (agent_writer, client_reader) = pipe();
    let executor = Arc::new(Background::new(1, |_| "acp-turn-driver-test".to_string()));
    let (connection, notifications) =
        AcpConnection::new(client_reader, client_writer, handler, &executor);
    let connection = Arc::new(connection);
    let state = Arc::new(TurnState::default());
    let host = Arc::new(RecordingHost::new(state.clone()));
    let (follow_up_tx, follow_up_rx) = async_channel::unbounded();
    let (cancel_tx, cancel_rx) = async_channel::unbounded();
    let (save_tx, _save_rx) = async_channel::unbounded();
    let turn = block_on(host.begin_turn(None)).unwrap();
    let mapper = AcpTurnMapper::new("task-1".to_owned(), turn.request_id.clone());
    let driver = TurnDriver {
        connection: connection.clone(),
        notifications,
        follow_ups: Some(follow_up_rx),
        cancels: Some(cancel_rx),
        host: host.clone(),
        state: state.clone(),
        turn,
        session_id: SESSION.to_owned(),
        suppressed_startup_message: None,
        prompt_text: "initial prompt".to_owned(),
        initial_attachments: Vec::new(),
        attachments: Arc::new(AttachmentResolver::new(
            Arc::new(MockAIClient::new()),
            Arc::new(http_client::Client::new()),
            None,
            std::env::temp_dir().join("acp-turn-driver-test"),
        )),
        boundary_snapshot: Arc::default(),
        save_requests: save_tx,
        queued: VecDeque::new(),
        cancel_deadline,
    };
    let fixture = Fixture {
        agent: FakeAgent {
            from_client: BufReader::new(agent_reader),
            to_client: agent_writer,
            _executor: executor,
        },
        follow_ups: follow_up_tx,
        cancels: cancel_tx,
        host,
        state,
        connection,
    };
    (driver, mapper, fixture)
}

fn start_default() -> (TurnDriver, AcpTurnMapper, Fixture) {
    start(rejecting_handler(), STEER_CANCEL_DEADLINE)
}

#[test]
fn a_follow_up_mid_turn_cancels_the_turn_then_prompts_with_an_interruption_note() {
    let (driver, mut mapper, mut fixture) = start_default();
    let script = async {
        let first = fixture.agent.next_prompt().await;
        assert_eq!(prompt_text(&first), "initial prompt");
        fixture.agent.send_chunk("working").await;
        fixture
            .host
            .wait_for(|events| {
                events
                    .iter()
                    .any(|event| matches!(event, HostEvent::Texts { texts, .. } if texts.contains(&"working".to_owned())))
            })
            .await;

        let participant = fixture.send_follow_up("steer").await;
        fixture.agent.expect_cancel().await;
        // What the agent streams before answering the cancel still belongs to the first turn.
        fixture.agent.send_chunk(" late").await;
        fixture.agent.respond(&first, "cancelled").await;

        let second = fixture.agent.next_prompt().await;
        assert_eq!(
            prompt_text(&second),
            format!("{STEER_INTERRUPTION_NOTE}\n\nsteer")
        );
        fixture.agent.respond(&second, "end_turn").await;
        fixture
            .host
            .wait_for(|events| matches!(events.last(), Some(HostEvent::Finished { .. })))
            .await;
        (participant, fixture.unbind())
    };
    let ((participant, (host, state)), ()) = block_on(join(script, driver.run(&mut mapper)));

    let streams = host.streams();
    assert_eq!(streams.len(), 2, "{:?}", host.events());
    let (first, second) = (&streams[0], &streams[1]);
    assert_eq!(host.texts_on(first), vec!["working", " late"]);
    assert_eq!(
        host.texts_on(second),
        vec!["steer"],
        "the echo shows the user's text only"
    );
    let events = host.events();
    let first_finished = events
        .iter()
        .position(|event| matches!(event, HostEvent::Finished { stream, .. } if stream == first))
        .unwrap();
    let second_began = events
        .iter()
        .position(|event| matches!(event, HostEvent::BeginTurn { stream, .. } if stream == second))
        .unwrap();
    assert!(first_finished < second_began);
    assert_eq!(
        events[first_finished],
        HostEvent::Finished {
            stream: first.clone(),
            reason: "done",
            settling: true,
        },
        "the cancelled turn settles while the driver's idle exit is held off"
    );
    assert_eq!(
        events[first_finished + 1],
        HostEvent::MarkedCancelled { settling: true }
    );
    assert_eq!(
        events[second_began],
        HostEvent::BeginTurn {
            stream: second.clone(),
            initiator: Some(participant),
        }
    );
    assert!(
        events
            .iter()
            .any(|event| event == &HostEvent::ShowQueued("steer".to_owned())),
        "the follow-up is shown as queued while the cancel is pending"
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, HostEvent::HideQueued(_))),
        "and hidden once it is sent"
    );
    assert!(host.queued_rows.lock().is_empty());
    assert_eq!(
        &events[events.len() - 2..],
        &[
            HostEvent::Finished {
                stream: second.clone(),
                reason: "done",
                settling: false,
            },
            HostEvent::Stopped,
        ]
    );
    assert!(!state.is_settling());
}

#[test]
fn a_follow_up_between_turns_is_sent_without_a_cancel() {
    let (driver, mut mapper, mut fixture) = start_default();
    let script = async {
        let first = fixture.agent.next_prompt().await;
        fixture.agent.respond(&first, "end_turn").await;
        fixture
            .host
            .wait_for(|events| matches!(events.last(), Some(HostEvent::Finished { .. })))
            .await;

        fixture.send_follow_up("next").await;
        let second = fixture.agent.next_prompt().await;
        assert_eq!(
            prompt_text(&second),
            "next",
            "no interruption note between turns"
        );
        fixture.agent.respond(&second, "end_turn").await;
        fixture
            .host
            .wait_for(|events| {
                events
                    .iter()
                    .filter(|event| matches!(event, HostEvent::Finished { .. }))
                    .count()
                    == 2
            })
            .await;
        fixture.unbind()
    };
    let ((host, _), ()) = block_on(join(script, driver.run(&mut mapper)));

    assert_eq!(
        host.events().last(),
        Some(&HostEvent::Stopped),
        "the host learns the driver stopped so cancellations fall back to the native path"
    );
    assert!(
        !host.events().iter().any(|event| matches!(
            event,
            HostEvent::ShowQueued(_) | HostEvent::MarkedCancelled { .. }
        )),
        "{:?}",
        host.events()
    );
}

#[test]
fn follow_ups_that_arrive_during_one_cancel_go_out_together_in_order() {
    let (driver, mut mapper, mut fixture) = start_default();
    let script = async {
        let first = fixture.agent.next_prompt().await;
        let author_a = fixture.send_follow_up("a").await;
        fixture.agent.expect_cancel().await;
        fixture.send_follow_up("b").await;
        fixture
            .host
            .wait_for(|events| events.contains(&HostEvent::ShowQueued("b".to_owned())))
            .await;
        fixture.agent.respond(&first, "cancelled").await;

        // One cancel, then both prompts in one `session/prompt`, in arrival order.
        let second = fixture.agent.next_prompt().await;
        let blocks = second["params"]["prompt"].as_array().unwrap().clone();
        assert_eq!(blocks.len(), 2, "{second}");
        assert_eq!(
            blocks[0]["text"],
            format!("{STEER_INTERRUPTION_NOTE}\n\na"),
            "only the first prompt carries the interruption note"
        );
        assert_eq!(blocks[1]["text"], "b");
        fixture.agent.respond(&second, "end_turn").await;
        fixture
            .host
            .wait_for(|events| {
                events
                    .iter()
                    .filter(|event| matches!(event, HostEvent::Finished { .. }))
                    .count()
                    == 2
            })
            .await;
        (author_a, fixture.unbind())
    };
    let ((author_a, (host, state)), ()) = block_on(join(script, driver.run(&mut mapper)));

    let streams = host.streams();
    assert_eq!(streams.len(), 2);
    assert_eq!(
        host.texts_on(&streams[1]),
        vec!["a", "b"],
        "each prompt is echoed"
    );
    assert!(host.events().contains(&HostEvent::BeginTurn {
        stream: streams[1].clone(),
        initiator: Some(author_a),
    }));
    assert!(host.queued_rows.lock().is_empty());
    assert!(!state.is_settling());
}

#[test]
fn an_unanswered_cancel_interrupts_the_terminal_and_fails_the_follow_up() {
    let (driver, mut mapper, mut fixture) = start(rejecting_handler(), Duration::from_millis(50));
    let script = async {
        let _first = fixture.agent.next_prompt().await;
        fixture.send_follow_up("steer").await;
        fixture.agent.expect_cancel().await;
        // The agent never answers.
        fixture
            .host
            .wait_for(|events| matches!(events.last(), Some(HostEvent::Finished { .. })))
            .await;
        fixture.unbind()
    };
    let ((host, state), ()) = block_on(join(script, driver.run(&mut mapper)));

    let events = host.events();
    assert!(
        events.contains(&HostEvent::InterruptedTerminal),
        "{events:?}"
    );
    assert!(
        matches!(
            events[events.len() - 2],
            HostEvent::Finished {
                reason: "internal_error",
                settling: false,
                ..
            }
        ),
        "{events:?}"
    );
    assert!(
        host.streams().len() == 1,
        "the follow-up never opens a turn"
    );
    assert!(host.queued_rows.lock().is_empty());
    assert!(!state.is_settling());
}

#[test]
fn a_pending_permission_request_is_answered_cancelled_when_a_follow_up_steers() {
    let (driver, mut mapper, mut fixture) =
        start(parking_permission_handler(), STEER_CANCEL_DEADLINE);
    let connection = fixture.connection.clone();
    let script = async {
        let first = fixture.agent.next_prompt().await;
        fixture
            .agent
            .send(json!({
                "jsonrpc": "2.0",
                "id": "perm-1",
                "method": "session/request_permission",
                "params": {
                    "sessionId": SESSION,
                    "toolCall": {"toolCallId": "call-1", "title": "Run"},
                    "options": [{"optionId": "allow", "name": "Allow", "kind": "allow_once"}],
                },
            }))
            .await;
        for _ in 0..400 {
            if !connection.pending_inbound_requests().is_empty() {
                break;
            }
            Timer::after(Duration::from_millis(5)).await;
        }
        assert_eq!(connection.pending_inbound_requests().len(), 1);

        fixture.send_follow_up("steer").await;
        fixture.agent.expect_cancel().await;
        let answer = fixture.agent.next_message().await;
        assert_eq!(answer["id"], "perm-1", "got {answer}");
        assert_eq!(answer["result"]["outcome"]["outcome"], "cancelled");
        fixture.agent.respond(&first, "cancelled").await;
        let second = fixture.agent.next_prompt().await;
        fixture.agent.respond(&second, "end_turn").await;
        fixture
            .host
            .wait_for(|events| {
                events
                    .iter()
                    .filter(|event| matches!(event, HostEvent::Finished { .. }))
                    .count()
                    == 2
            })
            .await;
        fixture.unbind()
    };
    let ((_, _), ()) = block_on(join(script, driver.run(&mut mapper)));
}

#[test]
fn an_interrupt_mid_turn_cancels_the_turn_and_marks_the_conversation_cancelled() {
    let (driver, mut mapper, mut fixture) = start_default();
    let script = async {
        let first = fixture.agent.next_prompt().await;
        fixture.send_interrupt().await;
        fixture.agent.expect_cancel().await;
        fixture.agent.respond(&first, "cancelled").await;
        fixture
            .host
            .wait_for(|events| matches!(events.last(), Some(HostEvent::MarkedCancelled { .. })))
            .await;
        fixture.unbind()
    };
    let ((host, state), ()) = block_on(join(script, driver.run(&mut mapper)));

    let events = host.events();
    let stream = host.streams()[0].clone();
    assert_eq!(
        &events[events.len() - 3..],
        &[
            HostEvent::Finished {
                stream,
                reason: "done",
                settling: true,
            },
            // No follow-up is coming: the `Cancelled` status is the run settling, and the
            // driver's idle exit must act on it.
            HostEvent::MarkedCancelled { settling: false },
            HostEvent::Stopped,
        ],
        "{events:?}"
    );
    assert_eq!(host.streams().len(), 1);
    assert!(!state.is_settling());
}

#[test]
fn an_agent_error_while_a_cancel_is_pending_counts_as_the_cancellation() {
    let (driver, mut mapper, mut fixture) = start_default();
    let script = async {
        let first = fixture.agent.next_prompt().await;
        fixture.send_interrupt().await;
        fixture.agent.expect_cancel().await;
        fixture
            .agent
            .send(json!({
                "jsonrpc": "2.0",
                "id": first["id"],
                "error": { "code": -32603, "message": "aborted" },
            }))
            .await;
        fixture
            .host
            .wait_for(|events| matches!(events.last(), Some(HostEvent::MarkedCancelled { .. })))
            .await;
        fixture.unbind()
    };
    let ((host, _), ()) = block_on(join(script, driver.run(&mut mapper)));

    assert!(
        host.events()
            .iter()
            .any(|event| matches!(event, HostEvent::Finished { reason: "done", .. })),
        "{:?}",
        host.events()
    );
}

#[test]
fn an_interrupt_between_turns_is_applied_at_once() {
    let (driver, mut mapper, mut fixture) = start_default();
    let script = async {
        let first = fixture.agent.next_prompt().await;
        fixture.agent.respond(&first, "end_turn").await;
        fixture
            .host
            .wait_for(|events| matches!(events.last(), Some(HostEvent::Finished { .. })))
            .await;
        fixture.send_interrupt().await;
        fixture
            .host
            .wait_for(|events| matches!(events.last(), Some(HostEvent::MarkedCancelled { .. })))
            .await;
        fixture.unbind()
    };
    let ((_, _), ()) = block_on(join(script, driver.run(&mut mapper)));
}

#[test]
fn turn_state_reports_idle_once_the_prompt_resolves() {
    let state = Arc::new(TurnState::default());
    state.set_in_turn(true);
    let mut wait = Box::pin(state.wait_until_idle());
    block_on(async {
        assert!(futures::poll!(wait.as_mut()).is_pending());
    });
    state.set_in_turn(false);
    block_on(wait);
    assert!(!state.is_in_turn());
}
