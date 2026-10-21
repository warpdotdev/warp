use std::pin::Pin;
use std::task::{Context, Poll};

use event_listener::Event;
use futures::StreamExt as _;
use futures::channel::{mpsc, oneshot};
use futures::executor::block_on;
use futures::future::join;
use futures::io::{AsyncBufReadExt as _, AsyncRead, AsyncWrite, AsyncWriteExt as _, BufReader};
use serde_json::{Value, json};
use session_sharing_protocol::common::AgentAttachment;
use warp_multi_agent_api::client_action::Action;
use warp_multi_agent_api::message::Message as MessageKind;
use warp_multi_agent_api::response_event::StreamInit;
use warp_multi_agent_api::{QueryAuthor, WarpUser, query_author};
use warpui::r#async::executor::Background;

use super::super::connection::AgentRequestHandler;
use super::*;
use crate::ai::agent::BaseUserQuery;
use crate::ai::blocklist::{QueuedQuery, QueuedQueryOrigin};
use crate::server::server_api::ai::MockAIClient;

const SESSION: &str = "sess_1";

/// One direction of an in-memory pipe.
fn pipe() -> (PipeWriter, PipeReader) {
    let (tx, rx) = mpsc::unbounded();
    (
        PipeWriter(tx, Arc::default()),
        PipeReader {
            rx,
            pending: Vec::new(),
        },
    )
}

#[derive(Default)]
struct ManualTimers {
    timers: Mutex<Vec<ManualTimer>>,
    fired: Mutex<Vec<Duration>>,
    changed: Event,
}

struct ManualTimer {
    after: Duration,
    release: Option<oneshot::Sender<()>>,
}
impl ManualTimers {
    fn factory(self: &Arc<Self>) -> Arc<dyn Fn(Duration) -> Deadline + Send + Sync> {
        let timers = self.clone();
        Arc::new(move |after| {
            let (tx, rx) = oneshot::channel();
            timers.timers.lock().push(ManualTimer {
                after,
                release: Some(tx),
            });
            timers.changed.notify(usize::MAX);
            let timers = timers.clone();
            Box::pin(
                async move {
                    let _ = rx.await;
                    timers.fired.lock().push(after);
                    timers.changed.notify(usize::MAX);
                }
                .fuse(),
            )
        })
    }

    async fn take(&self, after: Duration) -> oneshot::Sender<()> {
        loop {
            let changed = self.changed.listen();
            if let Some(gate) = self.timers.lock().iter_mut().find_map(|timer| {
                if timer.after == after {
                    timer.release.take()
                } else {
                    None
                }
            }) {
                return gate;
            }
            changed.await;
        }
    }

    fn requested(&self) -> Vec<Duration> {
        self.timers.lock().iter().map(|timer| timer.after).collect()
    }

    async fn wait_fired(&self, after: Duration) {
        loop {
            let changed = self.changed.listen();
            if self.fired.lock().contains(&after) {
                return;
            }
            changed.await;
        }
    }
}

type FlushGate = Arc<Mutex<Option<oneshot::Receiver<()>>>>;

struct PipeWriter(mpsc::UnboundedSender<Vec<u8>>, FlushGate);

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

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        let mut gate = self.1.lock();
        if let Some(release) = gate.as_mut() {
            if release.poll_unpin(cx).is_pending() {
                return Poll::Pending;
            }
            *gate = None;
        }
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
    async fn read_barrier(&mut self) {
        self.send(json!({
            "jsonrpc": "2.0", "id": "read-barrier", "method": "test/read_barrier", "params": {}
        }))
        .await;
        let answer = self.next_message().await;
        assert_eq!(answer["id"], "read-barrier", "{answer}");
        assert_eq!(answer["error"]["code"], -32601);
    }
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
    Cancelled {
        stream: ResponseStreamId,
        by: TurnCancellation,
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
    changed: Event,
    begin_gate: Mutex<Option<oneshot::Receiver<()>>>,
    apply_gate: Mutex<Option<oneshot::Receiver<()>>>,
    snapshot_gate: Mutex<Option<oneshot::Receiver<()>>>,
    actions: Mutex<Vec<(ResponseStreamId, Vec<ClientAction>)>>,
}

impl RecordingHost {
    fn new(state: Arc<TurnState>) -> Self {
        Self {
            events: Mutex::default(),
            streams_opened: Mutex::new(0),
            state,
            queued_rows: Mutex::default(),
            changed: Event::new(),
            begin_gate: Mutex::default(),
            apply_gate: Mutex::default(),
            snapshot_gate: Mutex::default(),
            actions: Mutex::default(),
        }
    }

    fn events(&self) -> Vec<HostEvent> {
        self.events.lock().clone()
    }

    fn record(&self, event: HostEvent) {
        self.events.lock().push(event);
        self.changed.notify(usize::MAX);
    }

    async fn wait_for(&self, predicate: impl Fn(&[HostEvent]) -> bool) {
        loop {
            let changed = self.changed.listen();
            if predicate(&self.events()) {
                return;
            }
            changed.await;
        }
    }

    fn gate_next_begin(&self) -> oneshot::Sender<()> {
        let (tx, rx) = oneshot::channel();
        assert!(self.begin_gate.lock().replace(rx).is_none());
        tx
    }

    fn gate_next_apply(&self) -> oneshot::Sender<()> {
        let (tx, rx) = oneshot::channel();
        assert!(self.apply_gate.lock().replace(rx).is_none());
        tx
    }

    fn gate_next_snapshot(&self) -> oneshot::Sender<()> {
        let (tx, rx) = oneshot::channel();
        assert!(self.snapshot_gate.lock().replace(rx).is_none());
        tx
    }

    async fn wait_for_finished(&self, count: usize) {
        self.wait_for(|events| {
            events
                .iter()
                .filter(|event| matches!(event, HostEvent::Finished { .. }))
                .count()
                == count
        })
        .await;
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
        let gate = self.begin_gate.lock().take();
        if let Some(gate) = gate {
            gate.await.unwrap();
        }
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
        self.actions
            .lock()
            .push((stream_id.clone(), actions.clone()));
        let texts = message_texts(&actions);
        if !texts.is_empty() {
            self.record(HostEvent::Texts {
                stream: stream_id.clone(),
                texts,
            });
        }
        let gate = self.apply_gate.lock().take();
        if let Some(gate) = gate {
            gate.await.unwrap();
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

    async fn cancel_turn(&self, stream_id: ResponseStreamId, by: TurnCancellation) {
        self.record(HostEvent::Cancelled {
            stream: stream_id,
            by,
            settling: self.state.is_settling(),
        });
    }

    async fn mark_conversation_cancelled(&self) {
        self.record(HostEvent::MarkedCancelled {
            settling: self.state.is_settling(),
        });
    }

    async fn capture_snapshot(&self) -> Result<Option<Vec<u8>>> {
        let gate = self.snapshot_gate.lock().take();
        if let Some(gate) = gate {
            gate.await.unwrap();
        }
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
    flush_gate: FlushGate,
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
    let flush_gate = client_writer.1.clone();
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
        timer_factory: Arc::new(|_| no_deadline()),
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
        flush_gate,
    };
    (driver, mapper, fixture)
}

fn start_default() -> (TurnDriver, AcpTurnMapper, Fixture) {
    start(rejecting_handler(), STEER_CANCEL_DEADLINE)
}

fn start_manual() -> (TurnDriver, AcpTurnMapper, Fixture, Arc<ManualTimers>) {
    let (mut driver, mapper, fixture) = start_default();
    let timers = Arc::new(ManualTimers::default());
    driver.timer_factory = timers.factory();
    (driver, mapper, fixture, timers)
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
    let first_cancelled = events
        .iter()
        .position(|event| matches!(event, HostEvent::Cancelled { stream, .. } if stream == first))
        .unwrap();
    let second_began = events
        .iter()
        .position(|event| matches!(event, HostEvent::BeginTurn { stream, .. } if stream == second))
        .unwrap();
    assert!(first_cancelled < second_began);
    assert_eq!(
        events[first_cancelled],
        HostEvent::Cancelled {
            stream: first.clone(),
            by: TurnCancellation::FollowUp,
            settling: true,
        },
        "the cancelled turn settles while the driver's idle exit is held off"
    );
    assert!(
        !events.iter().any(|event| matches!(
            event,
            HostEvent::Finished { stream, .. } if stream == first
        )),
        "the steered turn ends through the cancellation, not as a completed stream: {events:?}"
    );
    assert!(
        !events.iter().any(|event| matches!(
            event,
            HostEvent::MarkedCancelled { .. }
                | HostEvent::Cancelled {
                    by: TurnCancellation::Interrupt,
                    ..
                }
        )),
        "a steer never surfaces a cancelled conversation status: {events:?}"
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
            HostEvent::ShowQueued(_)
                | HostEvent::Cancelled { .. }
                | HostEvent::MarkedCancelled { .. }
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
            .wait_for(|events| matches!(events.last(), Some(HostEvent::Finished { .. })))
            .await;
        (author_a, fixture.unbind())
    };
    let ((author_a, (host, state)), ()) = block_on(join(script, driver.run(&mut mapper)));

    let streams = host.streams();
    assert_eq!(streams.len(), 2);
    assert!(
        host.events().contains(&HostEvent::Cancelled {
            stream: streams[0].clone(),
            by: TurnCancellation::FollowUp,
            settling: true,
        }),
        "{:?}",
        host.events()
    );
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
fn exhausted_soft_cancels_retain_fifo_rows_and_drain_updates_until_a_natural_final() {
    let (driver, mut mapper, mut fixture, timers) = start_manual();
    let script = async {
        let first = fixture.agent.next_prompt().await;
        let author = fixture.send_follow_up("B").await;
        fixture.agent.expect_cancel().await;
        let grace = timers.take(STEER_CANCEL_DEADLINE).await;
        timers.take(Duration::from_secs(1)).await.send(()).unwrap();
        fixture.agent.expect_cancel().await;
        fixture.send_follow_up("C").await;
        fixture
            .host
            .wait_for(|events| events.contains(&HostEvent::ShowQueued("C".to_owned())))
            .await;
        timers.take(Duration::from_secs(3)).await.send(()).unwrap();
        fixture.agent.expect_cancel().await;
        fixture.agent.send_chunk("A after retries").await;
        fixture.host.wait_for(|events| events.iter().any(
            |event| matches!(event, HostEvent::Texts { texts, .. } if texts.contains(&"A after retries".to_owned()))
        )).await;
        assert_eq!(timers.requested().len(), MAX_FOLLOW_UP_CANCEL_ATTEMPTS);
        grace.send(()).unwrap();
        timers.wait_fired(STEER_CANCEL_DEADLINE).await;
        fixture.agent.send_chunk("A after grace").await;
        fixture.host.wait_for(|events| events.iter().any(
            |event| matches!(event, HostEvent::Texts { texts, .. } if texts.contains(&"A after grace".to_owned()))
        )).await;
        assert_eq!(fixture.host.streams().len(), 1);
        assert!(fixture.state.is_settling());
        assert!(fixture.state.is_in_turn());
        assert_eq!(
            fixture
                .host
                .queued_rows
                .lock()
                .iter()
                .map(|(_, text)| text.clone())
                .collect::<Vec<_>>(),
            vec!["B", "C"]
        );
        assert!(!fixture.host.events().iter().any(|event| matches!(
            event,
            HostEvent::InterruptedTerminal
                | HostEvent::Finished { .. }
                | HostEvent::Cancelled { .. }
        )));
        let mut requested = timers.requested();
        requested.sort();
        assert_eq!(
            requested,
            vec![
                Duration::from_secs(1),
                Duration::from_secs(3),
                STEER_CANCEL_DEADLINE
            ]
        );

        fixture.agent.respond(&first, "end_turn").await;
        let second = fixture.agent.next_prompt().await;
        assert_eq!(
            second["params"]["prompt"],
            json!([
                {"type": "text", "text": "B"}, {"type": "text", "text": "C"}
            ])
        );
        assert!(fixture.host.queued_rows.lock().is_empty());
        fixture.agent.respond(&second, "end_turn").await;
        fixture.host.wait_for_finished(2).await;
        (author, fixture.unbind())
    };
    let ((author, (host, state)), ()) = block_on(join(script, driver.run(&mut mapper)));
    let streams = host.streams();
    assert_eq!(streams.len(), 2, "{:?}", host.events());
    assert_eq!(
        host.texts_on(&streams[0]),
        vec!["A after retries", "A after grace"]
    );
    assert_eq!(host.texts_on(&streams[1]), vec!["B", "C"]);
    assert!(host.events().contains(&HostEvent::BeginTurn {
        stream: streams[1].clone(),
        initiator: Some(author),
    }));
    assert!(!host.events().iter().any(|event| matches!(
        event,
        HostEvent::InterruptedTerminal | HostEvent::Cancelled { .. }
    )));
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
        fixture.agent.read_barrier().await;
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
            .wait_for(|events| matches!(events.last(), Some(HostEvent::Finished { .. })))
            .await;
        fixture.unbind()
    };
    let ((host, _), ()) = block_on(join(script, driver.run(&mut mapper)));
    assert_eq!(host.streams().len(), 2, "{:?}", host.events());
}

#[test]
fn follow_ups_after_expired_grace_do_not_rearm_cancellation_or_start_an_overlapping_prompt() {
    let (driver, mut mapper, mut fixture, timers) = start_manual();
    let script = async {
        let first = fixture.agent.next_prompt().await;
        fixture.send_follow_up("B").await;
        fixture.agent.expect_cancel().await;
        let retry_one = timers.take(Duration::from_secs(1)).await;
        let retry_three = timers.take(Duration::from_secs(3)).await;
        timers.take(STEER_CANCEL_DEADLINE).await.send(()).unwrap();
        timers.wait_fired(STEER_CANCEL_DEADLINE).await;
        assert!(retry_one.send(()).is_err());
        assert!(retry_three.send(()).is_err());
        fixture.send_follow_up("C after grace").await;
        fixture
            .host
            .wait_for(|events| events.contains(&HostEvent::ShowQueued("C after grace".to_owned())))
            .await;
        assert_eq!(fixture.host.streams().len(), 1);
        assert_eq!(fixture.host.queued_rows.lock().len(), 2);
        assert_eq!(timers.requested().len(), 3);
        assert!(fixture.state.is_in_turn());
        assert!(fixture.state.is_settling());
        fixture.agent.read_barrier().await;
        fixture.agent.respond(&first, "end_turn").await;
        let second = fixture.agent.next_prompt().await;
        assert_eq!(
            second["params"]["prompt"],
            json!([
                {"type": "text", "text": "B"}, {"type": "text", "text": "C after grace"}
            ])
        );
        fixture.agent.respond(&second, "end_turn").await;
        fixture.host.wait_for_finished(2).await;
        fixture.unbind()
    };
    let ((host, _), ()) = block_on(join(script, driver.run(&mut mapper)));
    assert_eq!(host.streams().len(), 2, "{:?}", host.events());
    assert!(host.queued_rows.lock().is_empty());
}

#[test]
fn more_than_two_ready_batches_are_submitted_fifo_without_empty_prompts_or_duplicate_echoes() {
    let (mut driver, mut mapper, mut fixture) = start_default();
    let expected = (0..MAX_BATCHED_FOLLOW_UPS * 2 + 3)
        .map(|index| format!("follow-up-{index}"))
        .collect::<Vec<_>>();
    let authors = block_on(async {
        let mut authors = Vec::new();
        for (index, text) in expected.iter().enumerate() {
            let author = if index < 5 {
                let participant_id = ParticipantId::new();
                driver.queued.push_back(QueuedFollowUp {
                    prompt: ExternalHarnessPrompt {
                        text: text.clone(),
                        attachments: Vec::new(),
                        participant_id: participant_id.clone(),
                        base: None,
                    },
                    row: None,
                });
                participant_id
            } else {
                fixture.send_follow_up(text).await
            };
            authors.push(author);
        }
        authors
    });
    let script = async {
        let mut received = Vec::new();
        let mut prompts = 0;
        while received.len() < expected.len() {
            let prompt = fixture.agent.next_prompt().await;
            let mut texts = prompt["params"]["prompt"]
                .as_array()
                .unwrap()
                .iter()
                .map(|block| {
                    assert_eq!(block["type"], "text", "{block}");
                    block["text"].as_str().unwrap().to_owned()
                })
                .collect::<Vec<_>>();
            if prompts == 0 {
                assert_eq!(texts.remove(0), "initial prompt");
            }
            let remaining = expected.len() - received.len();
            assert_eq!(texts.len(), remaining.min(MAX_BATCHED_FOLLOW_UPS));
            assert!(texts.iter().all(|text| !text.is_empty()));
            received.extend(texts);
            fixture.agent.read_barrier().await;
            fixture.agent.respond(&prompt, "end_turn").await;
            prompts += 1;
            fixture.host.wait_for_finished(prompts).await;
        }
        assert_eq!(received, expected);
        fixture.unbind()
    };
    let ((host, state), ()) = block_on(join(script, driver.run(&mut mapper)));
    let streams = host.streams();
    assert_eq!(
        streams.len(),
        expected.len().div_ceil(MAX_BATCHED_FOLLOW_UPS)
    );
    let echoes = streams
        .iter()
        .flat_map(|stream| host.texts_on(stream))
        .collect::<Vec<_>>();
    assert_eq!(echoes, expected);
    for (index, stream) in streams.iter().enumerate().skip(1) {
        assert!(host.events().contains(&HostEvent::BeginTurn {
            stream: stream.clone(),
            initiator: Some(authors[index * MAX_BATCHED_FOLLOW_UPS].clone()),
        }));
    }
    assert!(host.queued_rows.lock().is_empty());
    assert!(!state.is_settling());
}

#[test]
fn stop_during_unsent_preparation_is_local_and_retains_later_fifo_input() {
    for opening_turn in [true, false] {
        let (driver, mut mapper, mut fixture) = start_default();
        let script = async {
            let finished_before = if opening_turn {
                0
            } else {
                let opening = fixture.agent.next_prompt().await;
                fixture.agent.respond(&opening, "end_turn").await;
                fixture.host.wait_for_finished(1).await;
                1
            };
            let release_apply = fixture.host.gate_next_apply();
            let release_boundary = fixture.host.gate_next_snapshot();
            fixture.send_follow_up("B being prepared").await;
            fixture.send_follow_up("C not yet prepared").await;
            fixture.host.wait_for(|events| events.iter().any(
                |event| matches!(event, HostEvent::Texts { texts, .. } if texts.contains(&"B being prepared".to_owned()))
            )).await;
            fixture.send_interrupt().await;
            fixture
                .host
                .wait_for(|events| {
                    events
                        .iter()
                        .any(|event| matches!(event, HostEvent::Cancelled { .. }))
                })
                .await;
            assert!(
                release_apply.send(()).is_err(),
                "Stop must drop the unsent preparation"
            );
            assert!(
                fixture.agent.next_message().now_or_never().is_none(),
                "an unsent prompt must not write either session/prompt or session/cancel"
            );
            assert!(
                !fixture
                    .host
                    .events()
                    .contains(&HostEvent::InterruptedTerminal)
            );
            release_boundary.send(()).unwrap();
            let next = fixture.agent.next_prompt().await;
            assert_eq!(
                prompt_text(&next),
                format!("{STEER_INTERRUPTION_NOTE}\n\nC not yet prepared")
            );
            fixture.agent.read_barrier().await;
            fixture.agent.respond(&next, "end_turn").await;
            fixture.host.wait_for_finished(finished_before + 1).await;
            fixture.unbind()
        };
        let ((host, state), ()) = block_on(join(script, driver.run(&mut mapper)));
        let streams = host.streams();
        let unsent = usize::from(!opening_turn);
        assert_eq!(streams.len(), unsent + 2, "{:?}", host.events());
        assert_eq!(host.texts_on(&streams[unsent]), vec!["B being prepared"]);
        assert_eq!(
            host.texts_on(&streams[unsent + 1]),
            vec!["C not yet prepared"]
        );
        assert!(host.queued_rows.lock().is_empty());
        assert!(!state.is_settling());
    }
}

#[test]
fn a_follow_up_cannot_begin_cancellation_until_the_prompt_flush_completes() {
    let (driver, mut mapper, mut fixture) = start_default();
    let (release, blocked) = oneshot::channel();
    *fixture.flush_gate.lock() = Some(blocked);
    let (host, state) = block_on(async {
        let run = driver.run(&mut mapper);
        futures::pin_mut!(run);
        assert!(futures::poll!(run.as_mut()).is_pending());
        let first = fixture.agent.next_prompt().await;
        fixture.send_follow_up("B after write, before flush").await;
        assert!(futures::poll!(run.as_mut()).is_pending());
        assert!(fixture.host.queued_rows.lock().is_empty());
        assert!(!fixture.state.is_settling());
        release.send(()).unwrap();

        let script = async {
            fixture.agent.expect_cancel().await;
            fixture.agent.respond(&first, "cancelled").await;
            let second = fixture.agent.next_prompt().await;
            assert_eq!(
                prompt_text(&second),
                format!("{STEER_INTERRUPTION_NOTE}\n\nB after write, before flush")
            );
            fixture.agent.respond(&second, "end_turn").await;
            fixture.host.wait_for_finished(1).await;
            fixture.unbind()
        };
        let (result, ()) = join(script, run).await;
        result
    });
    assert_eq!(host.streams().len(), 2, "{:?}", host.events());
    assert!(!state.is_settling());
}

#[test]
fn a_cancel_lost_before_registration_is_retried_and_old_timers_cannot_cancel_the_next_prompt() {
    let (driver, mut mapper, mut fixture, timers) = start_manual();
    let script = async {
        let first = fixture.agent.next_prompt().await;
        fixture.send_follow_up("B").await;
        // The adapter has received bytes but has not registered this prompt yet.
        fixture.agent.expect_cancel().await;
        let retry = timers.take(Duration::from_secs(1)).await;
        let late_retry = timers.take(Duration::from_secs(3)).await;
        let late_grace = timers.take(STEER_CANCEL_DEADLINE).await;
        // Register the prompt before releasing the next cancellation attempt.
        retry.send(()).unwrap();
        fixture.agent.expect_cancel().await;
        fixture.agent.respond(&first, "cancelled").await;
        let second = fixture.agent.next_prompt().await;
        assert_eq!(
            prompt_text(&second),
            format!("{STEER_INTERRUPTION_NOTE}\n\nB")
        );
        assert!(
            late_retry.send(()).is_err(),
            "old retry must be dropped with A"
        );
        assert!(
            late_grace.send(()).is_err(),
            "old grace must be dropped with A"
        );
        fixture.agent.read_barrier().await;
        fixture.agent.respond(&second, "end_turn").await;
        fixture.host.wait_for_finished(1).await;
        fixture.unbind()
    };
    let ((host, state), ()) = block_on(join(script, driver.run(&mut mapper)));
    assert_eq!(host.streams().len(), 2, "{:?}", host.events());
    assert_eq!(host.texts_on(&host.streams()[1]), vec!["B"]);
    assert!(!state.is_settling());
}

#[test]
fn an_ignored_explicit_stop_uses_the_bounded_terminal_path() {
    let (driver, mut mapper, mut fixture, timers) = start_manual();
    let script = async {
        let _first = fixture.agent.next_prompt().await;
        fixture.send_interrupt().await;
        fixture.agent.expect_cancel().await;
        timers.take(STEER_CANCEL_DEADLINE).await.send(()).unwrap();
        fixture.host.wait_for_finished(1).await;
        fixture.unbind()
    };
    let ((host, state), ()) = block_on(join(script, driver.run(&mut mapper)));
    assert!(host.events().contains(&HostEvent::InterruptedTerminal));
    assert!(host.events().iter().any(|event| matches!(
        event,
        HostEvent::Finished {
            reason: "internal_error",
            settling: false,
            ..
        }
    )));
    assert_eq!(host.streams().len(), 1);
    assert_eq!(timers.requested(), vec![STEER_CANCEL_DEADLINE]);
    assert!(!state.is_settling());
}

#[test]
fn explicit_stop_upgrades_soft_cancellation_before_and_after_grace_expiry() {
    for expire_soft_grace in [false, true] {
        let (driver, mut mapper, mut fixture, timers) = start_manual();
        let script = async {
            let _first = fixture.agent.next_prompt().await;
            fixture.send_follow_up("B pending").await;
            fixture.agent.expect_cancel().await;
            let soft_grace = timers.take(STEER_CANCEL_DEADLINE).await;
            let retry_one = timers.take(Duration::from_secs(1)).await;
            let retry_three = timers.take(Duration::from_secs(3)).await;
            if expire_soft_grace {
                soft_grace.send(()).unwrap();
                timers.wait_fired(STEER_CANCEL_DEADLINE).await;
                assert!(fixture.state.is_settling());
                assert_eq!(fixture.host.queued_rows.lock().len(), 1);
            }
            fixture.send_interrupt().await;
            fixture.agent.expect_cancel().await;
            assert!(retry_one.send(()).is_err());
            assert!(retry_three.send(()).is_err());
            timers.take(STEER_CANCEL_DEADLINE).await.send(()).unwrap();
            fixture.host.wait_for_finished(1).await;
            fixture.unbind()
        };
        let ((host, state), ()) = block_on(join(script, driver.run(&mut mapper)));
        assert!(
            host.events().contains(&HostEvent::InterruptedTerminal),
            "{expire_soft_grace}"
        );
        assert!(
            host.events().iter().any(|event| matches!(
                event,
                HostEvent::Finished {
                    reason: "internal_error",
                    settling: false,
                    ..
                }
            )),
            "{:?}",
            host.events()
        );
        assert_eq!(host.streams().len(), 1);
        assert!(host.queued_rows.lock().is_empty());
        assert!(!state.is_settling());
    }
}

#[test]
fn transport_eof_during_soft_cancellation_remains_an_error_and_cleans_pending_rows() {
    let (driver, mut mapper, mut fixture, _timers) = start_manual();
    let script = async {
        let _first = fixture.agent.next_prompt().await;
        fixture.send_follow_up("B").await;
        fixture.agent.expect_cancel().await;
        fixture.agent.to_client.close().await.unwrap();
        fixture.host.wait_for_finished(1).await;
        fixture.unbind()
    };
    let ((host, state), ()) = block_on(join(script, driver.run(&mut mapper)));
    assert!(
        host.events().iter().any(|event| matches!(
            event,
            HostEvent::Finished {
                reason: "internal_error",
                ..
            }
        )),
        "{:?}",
        host.events()
    );
    assert!(!host.events().iter().any(|event| matches!(
        event,
        HostEvent::Cancelled { .. } | HostEvent::InterruptedTerminal
    )));
    assert_eq!(host.streams().len(), 1);
    assert!(host.queued_rows.lock().is_empty());
    assert!(!state.is_settling());
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
            .wait_for(|events| matches!(events.last(), Some(HostEvent::Cancelled { .. })))
            .await;
        fixture.unbind()
    };
    let ((host, state), ()) = block_on(join(script, driver.run(&mut mapper)));

    let events = host.events();
    let stream = host.streams()[0].clone();
    assert_eq!(
        &events[events.len() - 2..],
        &[
            // No follow-up is coming: the `Cancelled` status is the run settling, and the
            // driver's idle exit must act on it.
            HostEvent::Cancelled {
                stream,
                by: TurnCancellation::Interrupt,
                settling: false,
            },
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
            .wait_for(|events| matches!(events.last(), Some(HostEvent::Cancelled { .. })))
            .await;
        fixture.unbind()
    };
    let ((host, _), ()) = block_on(join(script, driver.run(&mut mapper)));

    assert!(
        host.events().iter().any(|event| matches!(
            event,
            HostEvent::Cancelled {
                by: TurnCancellation::Interrupt,
                ..
            }
        )),
        "{:?}",
        host.events()
    );
    assert!(
        !host
            .events()
            .iter()
            .any(|event| matches!(event, HostEvent::Finished { .. })),
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

#[test]
fn opening_batch_preserves_attachments_and_authorship_across_preparation() {
    let (driver, mut mapper, mut fixture) = start_default();
    let author_b = ParticipantId::new();
    let base = BaseUserQuery::from_proto(warp_multi_agent_api::request::input::UserQuery {
        author: Some(QueryAuthor {
            principal: Some(query_author::Principal::User(WarpUser {
                uid: "author-B".to_owned(),
                ..Default::default()
            })),
            ..Default::default()
        }),
        ..Default::default()
    });
    let expected_author = base.to_proto().author;
    fixture
        .follow_ups
        .try_send(ExternalHarnessPrompt {
            text: "B".to_owned(),
            attachments: vec![AgentAttachment::PlainText {
                content: "B context".to_owned(),
            }],
            participant_id: author_b,
            base: Some(base),
        })
        .unwrap();
    block_on(fixture.send_follow_up("C"));
    let release = fixture.host.gate_next_apply();
    let script = async {
        fixture.host.wait_for(|events| events.iter().any(
            |event| matches!(event, HostEvent::Texts { texts, .. } if texts.contains(&"B".to_owned()))
        )).await;
        fixture.send_follow_up("D during preparation").await;
        release.send(()).unwrap();
        let opening = fixture.agent.next_prompt().await;
        assert_eq!(
            opening["params"]["prompt"],
            json!([
                {"type": "text", "text": "initial prompt"},
                {"type": "text", "text": "B"},
                {"type": "text", "text": "<attached_text>\nB context\n</attached_text>"},
                {"type": "text", "text": "C"},
                {"type": "text", "text": "D during preparation"}
            ])
        );
        fixture.agent.read_barrier().await;
        fixture.agent.respond(&opening, "end_turn").await;
        fixture.host.wait_for_finished(1).await;
        fixture.unbind()
    };
    let ((host, state), ()) = block_on(join(script, driver.run(&mut mapper)));
    assert_eq!(host.streams().len(), 1, "{:?}", host.events());
    assert_eq!(
        host.texts_on(&host.streams()[0]),
        vec!["B", "C", "D during preparation"]
    );
    let actions = host.actions.lock();
    let echoes = actions
        .iter()
        .flat_map(|(_, actions)| actions)
        .filter_map(|action| match &action.action {
            Some(Action::AddMessagesToTask(add)) => Some(&add.messages),
            _ => None,
        })
        .flatten()
        .filter_map(|message| match &message.message {
            Some(MessageKind::UserQuery(query)) => Some(query),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(echoes.len(), 3);
    assert_eq!(echoes[0].author, expected_author);
    assert!(!state.is_settling());
}

#[test]
fn idle_batch_preserves_fifo_and_first_author_across_async_preparation() {
    let (driver, mut mapper, mut fixture) = start_default();
    let script = async {
        let first = fixture.agent.next_prompt().await;
        fixture.agent.respond(&first, "end_turn").await;
        fixture.host.wait_for_finished(1).await;
        let release = fixture.host.gate_next_begin();
        let author = fixture.send_follow_up("B").await;
        fixture.send_follow_up("C ready").await;
        fixture
            .host
            .wait_for(|events| {
                events
                    .iter()
                    .filter(|event| matches!(event, HostEvent::BeginTurn { .. }))
                    .count()
                    == 2
            })
            .await;
        fixture.send_follow_up("D during preparation").await;
        release.send(()).unwrap();
        let second = fixture.agent.next_prompt().await;
        assert_eq!(
            second["params"]["prompt"],
            json!([
                {"type": "text", "text": "B"},
                {"type": "text", "text": "C ready"},
                {"type": "text", "text": "D during preparation"}
            ])
        );
        fixture.agent.read_barrier().await;
        fixture.agent.respond(&second, "end_turn").await;
        fixture.host.wait_for_finished(2).await;
        (author, fixture.unbind())
    };
    let ((author, (host, _)), ()) = block_on(join(script, driver.run(&mut mapper)));
    let streams = host.streams();
    assert_eq!(streams.len(), 2, "{:?}", host.events());
    assert_eq!(
        host.texts_on(&streams[1]),
        vec!["B", "C ready", "D during preparation"]
    );
    assert!(host.events().contains(&HostEvent::BeginTurn {
        stream: streams[1].clone(),
        initiator: Some(author),
    }));
    assert!(
        !host
            .events()
            .iter()
            .any(|event| matches!(event, HostEvent::Cancelled { .. }))
    );
}

#[test]
fn a_ready_final_beats_retry_and_grace_and_drains_ready_input_into_the_held_batch() {
    let (driver, mut mapper, mut fixture, timers) = start_manual();
    let script = async {
        let first = fixture.agent.next_prompt().await;
        fixture.send_follow_up("B held").await;
        fixture.agent.expect_cancel().await;
        let retry_one = timers.take(Duration::from_secs(1)).await;
        let retry_three = timers.take(Duration::from_secs(3)).await;
        let grace = timers.take(STEER_CANCEL_DEADLINE).await;
        let release = fixture.host.gate_next_apply();
        let release_boundary = fixture.host.gate_next_snapshot();
        fixture.agent.send_chunk("A final chunk").await;
        fixture.host.wait_for(|events| events.iter().any(
            |event| matches!(event, HostEvent::Texts { texts, .. } if texts.contains(&"A final chunk".to_owned()))
        )).await;
        fixture.send_follow_up("C ready").await;
        fixture.agent.respond(&first, "end_turn").await;
        fixture.agent.read_barrier().await;
        retry_one.send(()).unwrap();
        retry_three.send(()).unwrap();
        grace.send(()).unwrap();
        release.send(()).unwrap();
        fixture.host.wait_for_finished(1).await;
        fixture.send_follow_up("D after final").await;
        release_boundary.send(()).unwrap();
        let second = fixture.agent.next_prompt().await;
        assert_eq!(
            second["params"]["prompt"],
            json!([
                {"type": "text", "text": "B held"},
                {"type": "text", "text": "C ready"},
                {"type": "text", "text": "D after final"}
            ])
        );
        fixture.agent.respond(&second, "end_turn").await;
        fixture.host.wait_for_finished(2).await;
        fixture.unbind()
    };
    let ((host, _), ()) = block_on(join(script, driver.run(&mut mapper)));
    let streams = host.streams();
    assert_eq!(streams.len(), 2, "{:?}", host.events());
    assert_eq!(host.texts_on(&streams[0]), vec!["A final chunk"]);
    assert_eq!(
        host.texts_on(&streams[1]),
        vec!["B held", "C ready", "D after final"]
    );
    assert!(!host.events().iter().any(|event| matches!(
        event,
        HostEvent::InterruptedTerminal | HostEvent::Cancelled { .. }
    )));
}
