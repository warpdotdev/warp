//! The prompt turns of an ACP session: sending `session/prompt`, applying what the agent streams
//! to the native conversation, and delivering follow-ups and interrupts that arrive mid-turn.
//!
//! A follow-up that lands mid-turn is added to the turn in flight when the agent supports the
//! `_session/steering` extension: the agent takes it at its next step without discarding the
//! work under way, as the terminal-driven CLIs do with typed input, and the running
//! `session/prompt` keeps owning the output. Steers go out one at a time, in arrival order;
//! follow-ups the prompt could not carry are steered first. A steer the agent cannot take
//! because its turn is ending waits for the turn and goes out as the next one. Without the
//! extension, or when a steer does not take, the follow-up steers the agent the way Zed does:
//! `session/cancel`, keep applying what the agent still streams to the turn it belongs to until
//! that turn's response arrives, then prompt again. Ready follow-ups go out together in order.
//! If cancellation is ignored, they wait for the original turn to finish; only an explicit
//! interrupt imposes a deadline that interrupts the agent's terminal.
use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::Poll;
use std::time::Duration;

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use event_listener::Event;
use futures::future::FusedFuture;
use futures::stream::{FuturesOrdered, FuturesUnordered};
use futures::{FutureExt, StreamExt, future, pin_mut, select, select_biased};
use parking_lot::Mutex;
use session_sharing_protocol::common::ParticipantId;
use warp_multi_agent_api::ClientAction;
use warp_multi_agent_api::response_event::{StreamFinished, stream_finished};
use warpui::r#async::Timer;

use super::super::SavePoint;
use super::attachments::AttachmentResolver;
use super::connection::{AcpConnection, Arrival, InboundNotification, RpcError};
use super::mapping::{AcpTurnMapper, TurnEvent};
use super::protocol::{
    self, CancelParams, ContentBlock, PromptParams, PromptResponse, SessionUpdate,
    SessionUpdateParams, SteeringIdleBehavior, SteeringMeta, SteeringOptions, SteeringOutcome,
    SteeringParams, SteeringResponse, StopReason,
};
use crate::ai::agent_sdk::setup_observability::{
    AcpActionEvent, OzRunTimelineEvent, SetupClientEventReporter,
};
use crate::ai::attachment_utils::DownloadedAttachment;
use crate::ai::blocklist::{
    ExternalHarnessCancel, ExternalHarnessPrompt, ExternalHarnessTurn, QueuedQueryId,
    ResponseStreamId,
};

/// Follow-up cancellation grace period, and the hard bound for an explicit interrupt.
pub(super) const STEER_CANCEL_DEADLINE: Duration = Duration::from_secs(15);

/// Maximum follow-up cancellation attempts, including the initial attempt.
const MAX_FOLLOW_UP_CANCEL_ATTEMPTS: usize = 3;
const FOLLOW_UP_CANCEL_RETRY_BASE_DELAY: Duration = Duration::from_secs(1);

/// How long a shutdown waits for the agent to answer the `session/cancel` sent on exit before
/// closing the connection. Shorter than [`STEER_CANCEL_DEADLINE`]: the driver's exit ladder
/// starts its own timers (SIGINT after 1s, SIGKILL after a further 14s) only once `exit`
/// returns, so this wait extends every in-turn shutdown by up to its full length. Three
/// seconds is enough for an agent to abort its model call and persist the turn (Pi writes its
/// session JSONL when the cancelled response goes out) without doubling the ladder.
pub(super) const EXIT_CANCEL_DEADLINE: Duration = Duration::from_secs(3);

/// How long a turn whose response has arrived waits for the answer to a `_session/steering`
/// still outstanding, which decides whether that prompt was taken or must be sent as a turn of
/// its own. Past it the prompt is sent again; an answer after that is only noted, since an
/// agent that takes the steer into the re-sent turn has then received the prompt twice.
pub(super) const STEER_SETTLE_DEADLINE: Duration = Duration::from_secs(5);

/// Prefixed to a follow-up that cut the previous turn short, so the agent knows why its work
/// stopped where it did.
pub(super) const STEER_INTERRUPTION_NOTE: &str =
    "[The user interrupted your previous turn with this message.]";

/// Keeps continuous arrivals from indefinitely postponing prompt submission.
pub(super) const MAX_BATCHED_FOLLOW_UPS: usize = 32;

/// The driver-side effects of a turn: the native conversation the agent's updates land in and
/// the terminal the agent runs in. The turn state machine only sees this, so it can be driven
/// against a recording host in tests.
#[async_trait]
pub(super) trait TurnHost: Send + Sync {
    /// Opens a request stream on the conversation, attributed to `initiator` when given.
    async fn begin_turn(&self, initiator: Option<ParticipantId>) -> Result<ExternalHarnessTurn>;
    async fn apply_actions(
        &self,
        stream_id: &ResponseStreamId,
        actions: Vec<ClientAction>,
    ) -> Result<()>;
    async fn finish_turn(
        &self,
        stream_id: ResponseStreamId,
        finished: StreamFinished,
    ) -> Result<()>;
    /// Ends the stream of a turn the agent cancelled; `by` decides what the conversation's
    /// status becomes.
    async fn cancel_turn(&self, stream_id: ResponseStreamId, by: TurnCancellation);
    /// Marks the conversation cancelled when an interrupt arrives with no turn to cut short.
    async fn mark_conversation_cancelled(&self);
    /// Captures the conversation as encoded `ConversationData`, or `None` when there is nothing
    /// to upload.
    async fn capture_snapshot(&self) -> Result<Option<Vec<u8>>>;
    /// Interrupts the agent's block in the terminal.
    async fn interrupt_terminal(&self);
    /// Shows `prompt` as queued on the conversation while it waits for the active turn to be
    /// settled. Returns the row to hide once it is sent.
    async fn show_queued_prompt(&self, prompt: &ExternalHarnessPrompt) -> Option<QueuedQueryId>;
    async fn hide_queued_prompt(&self, query_id: QueuedQueryId);
    /// The turn driver has stopped taking prompts and cancellations; whatever routed them here
    /// must fall back to its native handling.
    async fn turn_driver_stopped(&self);
}

/// Where the turn driver reports what it did, so the state machine can be driven against a
/// recording sink in tests.
pub(super) trait TurnTelemetry: Send + Sync {
    /// Reports how an ACP action that began at `started` turned out.
    fn action(
        &self,
        event: AcpActionEvent,
        outcome: &'static str,
        started: DateTime<Utc>,
        is_error: bool,
    );
    /// Reports the agent's first output of the run.
    fn first_output(&self);
}

impl TurnTelemetry for SetupClientEventReporter {
    fn action(
        &self,
        event: AcpActionEvent,
        outcome: &'static str,
        started: DateTime<Utc>,
        is_error: bool,
    ) {
        self.post_action_event(event, outcome, started, is_error);
    }

    fn first_output(&self) {
        self.post_timeline_event_detached(OzRunTimelineEvent::AgentFirstOutput);
    }
}

/// Turn activity shared with the harness runner, which reads it off the turn driver's task.
#[derive(Default)]
pub(super) struct TurnState {
    /// A turn cancellation is settling, or queued follow-ups are about to open a new turn.
    settling: AtomicBool,
    /// A `session/prompt` is outstanding.
    in_turn: AtomicBool,
    changed: Event,
}

impl TurnState {
    pub(super) fn is_settling(&self) -> bool {
        self.settling.load(Ordering::SeqCst)
    }

    pub(super) fn is_in_turn(&self) -> bool {
        self.in_turn.load(Ordering::SeqCst)
    }

    fn set_settling(&self, settling: bool) {
        self.settling.store(settling, Ordering::SeqCst);
    }

    fn set_in_turn(&self, in_turn: bool) {
        self.in_turn.store(in_turn, Ordering::SeqCst);
        self.changed.notify(usize::MAX);
    }

    /// Resolves once no `session/prompt` is outstanding.
    pub(super) async fn wait_until_idle(&self) {
        loop {
            if !self.is_in_turn() {
                return;
            }
            let listener = self.changed.listen();
            if !self.is_in_turn() {
                return;
            }
            listener.await;
        }
    }
}

/// How a prompt turn ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TurnOutcome {
    Stopped(StopReason),
    /// The agent acknowledged a `session/cancel`.
    Cancelled,
}

impl TurnOutcome {
    /// A bounded label for metrics.
    fn metric_label(self) -> &'static str {
        match self {
            Self::Stopped(StopReason::EndTurn) => "stopped_end_turn",
            Self::Stopped(StopReason::MaxTokens) => "stopped_max_tokens",
            Self::Stopped(StopReason::MaxTurnRequests) => "stopped_max_turn_requests",
            Self::Stopped(StopReason::Refusal) => "stopped_refusal",
            Self::Stopped(StopReason::Cancelled) => "stopped_cancelled",
            Self::Stopped(StopReason::Unknown) => "stopped_unknown",
            Self::Cancelled => "cancelled",
        }
    }
}

/// Why held follow-ups will not be sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DropReason {
    HarnessExited,
    TurnOpenFailed,
    AgentDisconnected,
    CancelTimedOut,
}

impl DropReason {
    /// A bounded label for metrics.
    fn metric_label(self) -> &'static str {
        match self {
            Self::HarnessExited => "harness_exited",
            Self::TurnOpenFailed => "turn_open_failed",
            Self::AgentDisconnected => "agent_disconnected",
            Self::CancelTimedOut => "cancel_timed_out",
        }
    }
}

impl std::fmt::Display for DropReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::HarnessExited => "the harness exited before the follow-up could be sent",
            Self::TurnOpenFailed => "the follow-up turn could not be opened",
            Self::AgentDisconnected => "the agent disconnected before the follow-up could be sent",
            Self::CancelTimedOut => "the agent did not answer session/cancel in time",
        })
    }
}

/// What cut a turn short. The distinction is the conversation's status afterwards, which is
/// what the run state and its integrations mirror: a follow-up continues the conversation at
/// once, so the turn boundary is internal and the status stays `InProgress`, as when a native
/// follow-up interrupts a request; an interrupt ends it, as a native stop does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TurnCancellation {
    FollowUp,
    Interrupt,
}

/// An unsent follow-up and its queued conversation row, if already shown.
pub(super) struct QueuedFollowUp {
    prompt: ExternalHarnessPrompt,
    row: Option<QueuedQueryId>,
    /// When the follow-up reached the turn driver, from which its delivery latency counts.
    received: DateTime<Utc>,
}

impl QueuedFollowUp {
    fn new(prompt: ExternalHarnessPrompt, row: Option<QueuedQueryId>) -> Self {
        Self {
            prompt,
            row,
            received: Utc::now(),
        }
    }
}

/// What the agent's answer to a steer means for the prompt.
enum SteerVerdict {
    /// Taken into the running turn; the prompt is consumed.
    Injected,
    /// No turn could take it; it waits, unsent, for the turn to end on its own.
    PromptRequired,
    /// Not taken (`failed`, an unknown outcome, or an error); the turn is cancelled for it.
    Rejected,
    /// The agent started a turn no `session/prompt` owns; it is cancelled and the prompt sent
    /// as a turn.
    DetachedTurn,
}

impl SteerVerdict {
    /// A bounded label for metrics.
    fn metric_label(&self) -> &'static str {
        match self {
            Self::Injected => "injected",
            Self::PromptRequired => "prompt_required",
            Self::Rejected => "rejected",
            Self::DetachedTurn => "detached_turn",
        }
    }

    /// Whether the agent mishandled the steer, as opposed to declining it in order.
    fn is_error(&self) -> bool {
        match self {
            Self::Injected | Self::PromptRequired => false,
            Self::Rejected | Self::DetachedTurn => true,
        }
    }
}

/// The cancellation of the turn in flight, if one has started: what asked for it, when the
/// agent must have answered by, and the re-sends of `session/cancel` still to come.
struct CancelState {
    by: Option<TurnCancellation>,
    deadline: Deadline,
    retries: FuturesOrdered<Deadline>,
    /// When the first `session/cancel` went out; cleared once its outcome has been reported.
    started: Option<DateTime<Utc>>,
    /// How many times `session/cancel` has been sent for this turn.
    sends: u32,
}

impl CancelState {
    fn none() -> Self {
        Self {
            by: None,
            deadline: no_deadline(),
            retries: FuturesOrdered::new(),
            started: None,
            sends: 0,
        }
    }

    /// The outcome of a cancel the agent answered, or `None` if none was pending.
    fn take_acknowledged(&mut self) -> Option<(&'static str, DateTime<Utc>)> {
        let started = self.started.take()?;
        let outcome = if self.sends > 1 { "retried" } else { "acked" };
        Some((outcome, started))
    }
}

enum PromptInput {
    Initial,
    FollowUps { interrupted: bool },
}

enum Wake<T> {
    Notification(Result<InboundNotification, async_channel::RecvError>),
    FollowUp(Result<ExternalHarnessPrompt, async_channel::RecvError>),
    Cancel(Result<ExternalHarnessCancel, async_channel::RecvError>),
    Response(T),
    Steer(SteerAnswer),
    /// The answer to a steer whose turn ended before it came.
    LateSteer(SteerAnswer),
    Deadline,
    Retry,
}

pub(super) type Deadline = Pin<Box<dyn FusedFuture<Output = ()> + Send>>;

fn no_deadline() -> Deadline {
    Box::pin(future::pending().fuse())
}

pub(super) fn cancel_deadline(after: Duration) -> Deadline {
    Box::pin(FutureExt::map(Timer::after(after), |_| ()).fuse())
}

/// The agent's answer to a steer, and where it stood among the agent's messages.
type SteerAnswer = (Arrival, Result<SteeringResponse>);

pub(super) type SteerFuture = Pin<Box<dyn FusedFuture<Output = SteerAnswer> + Send>>;

fn no_steer() -> SteerFuture {
    Box::pin(future::pending().fuse())
}

/// The next answer to a steer that outlived its turn, or never resolves while there is none.
/// The set is only locked inside a poll, so the driver stays `Sync` while holding futures that
/// are not.
async fn next_late_steer(late_steers: &Mutex<FuturesUnordered<SteerFuture>>) -> SteerAnswer {
    future::poll_fn(|cx| match late_steers.lock().poll_next_unpin(cx) {
        Poll::Ready(Some(result)) => Poll::Ready(result),
        Poll::Ready(None) | Poll::Pending => Poll::Pending,
    })
    .await
}

/// Receives from `receiver`, or never resolves once it has been closed and dropped.
async fn recv_or_pending<T>(
    receiver: Option<&async_channel::Receiver<T>>,
) -> Result<T, async_channel::RecvError> {
    match receiver {
        Some(receiver) => receiver.recv().await,
        None => future::pending().await,
    }
}

/// Runs the initial prompt turn, then any follow-up prompts that arrive while the session is
/// open. The session ends when the driver closes the connection (its idle window elapsed or it
/// is shutting down), when the controller unbinds the conversation, or when the agent leaves.
///
/// A prompt turn spans one or more MAA request streams: every time the agent reports results
/// for tool calls announced in the current stream, that stream is finished and a new one is
/// opened, as the MAA server does for its own tool round trips.
pub(super) struct TurnDriver {
    pub connection: Arc<AcpConnection>,
    pub notifications: async_channel::Receiver<InboundNotification>,
    /// Follow-up prompts injected into the conversation (e.g. by shared-session viewers).
    /// `None` once the controller has unbound the conversation.
    pub follow_ups: Option<async_channel::Receiver<ExternalHarnessPrompt>>,
    /// Interrupts of the current turn. `None` once the controller has unbound the conversation.
    pub cancels: Option<async_channel::Receiver<ExternalHarnessCancel>>,
    pub host: Arc<dyn TurnHost>,
    pub telemetry: Arc<dyn TurnTelemetry>,
    /// Whether the initial turn has yet to put anything into the conversation. Cleared when it
    /// does, or when the initial turn ends, so the first output is reported at most once.
    pub first_output_pending: bool,
    pub state: Arc<TurnState>,
    /// The currently open request stream.
    pub turn: ExternalHarnessTurn,
    pub session_id: String,
    /// Text the adapter announced at `session/new` and will also stream once as an agent
    /// message chunk. That chunk is not part of any response, so it is dropped on arrival and
    /// this is cleared.
    pub suppressed_startup_message: Option<String>,
    pub prompt_text: String,
    /// Task attachments downloaded before the run started, sent with the initial prompt.
    pub initial_attachments: Vec<DownloadedAttachment>,
    pub attachments: Arc<AttachmentResolver>,
    pub boundary_snapshot: Arc<Mutex<Option<Vec<u8>>>>,
    pub save_requests: async_channel::Sender<SavePoint>,
    /// Unsent follow-ups, sent together after the current turn settles.
    pub queued: VecDeque<QueuedFollowUp>,
    /// Cancellation grace period; [`STEER_CANCEL_DEADLINE`] outside tests.
    pub cancel_deadline: Duration,
    /// How long an ended turn waits for an outstanding steer's answer; [`STEER_SETTLE_DEADLINE`]
    /// outside tests.
    pub steer_settle_deadline: Duration,
    pub timer_factory: Arc<dyn Fn(Duration) -> Deadline + Send + Sync>,
    /// Whether the agent accepts `_session/steering`, as it announced at `initialize`. Cleared
    /// if the agent then rejects the method.
    pub steering_supported: bool,
    /// The follow-up whose `_session/steering` is outstanding.
    pub steering: Option<QueuedFollowUp>,
    /// Follow-ups to steer, in order, once the outstanding steer has been answered.
    pub awaiting_steer: VecDeque<QueuedFollowUp>,
    /// Steers whose turn ended before the agent answered them. Their prompts went out as turns
    /// once [`Self::steer_settle_deadline`] passed; the answers are read only so a late
    /// `injected` is recognised, and never echoed or sent again.
    pub late_steers: Mutex<FuturesUnordered<SteerFuture>>,
}

impl TurnDriver {
    /// A failed turn is already reflected in the conversation status (the stream finishes with
    /// an internal error), which is what the task status follows; the block's exit code remains
    /// the agent's own.
    pub(super) async fn run(mut self, mapper: &mut AcpTurnMapper) {
        let mut turn_result = self.run_turn(mapper, PromptInput::Initial).await;
        self.first_output_pending = false;

        log::info!("ACP turn finished; accepting follow-ups until the driver ends the run");
        loop {
            if !self.pending_follow_ups().is_empty() {
                let interrupted = matches!(turn_result, Ok(TurnOutcome::Cancelled));
                turn_result = self.run_follow_ups(mapper, interrupted).await;
                continue;
            }
            let wake = select! {
                prompt = recv_or_pending(self.follow_ups.as_ref()).fuse() => Wake::FollowUp(prompt),
                cancel = recv_or_pending(self.cancels.as_ref()).fuse() => Wake::Cancel(cancel),
                notification = self.notifications.recv().fuse() => {
                    Wake::<()>::Notification(notification)
                }
                result = next_late_steer(&self.late_steers).fuse() => Wake::LateSteer(result),
            };
            match wake {
                Wake::FollowUp(Ok(prompt)) => {
                    log::info!("Running ACP follow-up turn");
                    self.queued.push_back(QueuedFollowUp::new(prompt, None));
                    turn_result = self.run_follow_ups(mapper, false).await;
                }
                // The controller unbound the conversation; nothing more can be routed here.
                Wake::FollowUp(Err(_)) => break,
                Wake::Cancel(Ok(ExternalHarnessCancel)) => {
                    log::info!("ACP interrupt received between turns; nothing to cancel");
                    self.host.mark_conversation_cancelled().await;
                }
                Wake::Cancel(Err(_)) => self.cancels = None,
                Wake::Notification(Ok(notification)) => log::debug!(
                    "Ignoring ACP notification {} received between turns",
                    notification.method
                ),
                // The agent closed its side, typically because the driver closed ours.
                Wake::Notification(Err(_)) => break,
                Wake::LateSteer((_, result)) => self.note_late_steer(result),
                Wake::Response(()) | Wake::Steer(_) | Wake::Deadline | Wake::Retry => {
                    unreachable!()
                }
            }
        }
        self.fail_queued(DropReason::HarnessExited).await;
        self.host.turn_driver_stopped().await;
        self.connection.close().await;
        match turn_result {
            Ok(outcome) => log::info!("ACP turns finished (last outcome {outcome:?})"),
            Err(error) => log::warn!("ACP turns finished with an error: {error:#}"),
        }
    }

    /// Opens a new request stream for one or more follow-up prompts, echoes each, and runs the
    /// turn with all of them in order. The echo shows the text only; attachments reach the agent
    /// but are not yet rendered on the user-query entry. `interrupted` means the prompts cut the
    /// previous turn short, which the agent is told. The stream is attributed to the first
    /// prompt's author.
    async fn run_follow_ups(
        &mut self,
        mapper: &mut AcpTurnMapper,
        interrupted: bool,
    ) -> Result<TurnOutcome> {
        let initiator = self
            .pending_follow_ups()
            .front()
            .map(|follow_up| follow_up.prompt.participant_id.clone());
        let next = match self.host.begin_turn(initiator).await {
            Ok(next) => next,
            Err(error) => {
                self.fail_queued(DropReason::TurnOpenFailed).await;
                return Err(error);
            }
        };
        mapper.start_segment(next.request_id.clone());
        self.turn = next;
        // The new turn is under way; its `InProgress` status cancels any pending idle exit.
        self.state.set_settling(false);
        self.run_turn(mapper, PromptInput::FollowUps { interrupted })
            .await
    }

    fn pending_follow_ups(&mut self) -> &mut VecDeque<QueuedFollowUp> {
        if let Some(receiver) = &self.follow_ups {
            for _ in 0..receiver.len() {
                match receiver.try_recv() {
                    Ok(prompt) => self.queued.push_back(QueuedFollowUp::new(prompt, None)),
                    Err(_) => break,
                }
            }
        }
        &mut self.queued
    }

    fn next_batch_follow_up(&mut self, prepared: usize) -> Option<QueuedFollowUp> {
        // Collect before checking the cap so ready overflow waits for the next batch.
        let pending = self.pending_follow_ups();
        if prepared < MAX_BATCHED_FOLLOW_UPS {
            pending.pop_front()
        } else {
            None
        }
    }

    async fn prepare_prompt(
        &mut self,
        mapper: &mut AcpTurnMapper,
        input: PromptInput,
    ) -> Result<Vec<ContentBlock>> {
        let (mut content, mut interrupted) = match input {
            PromptInput::Initial => (
                self.attachments
                    .prompt_content_for_files(
                        self.prompt_text.clone(),
                        Vec::new(),
                        &self.initial_attachments,
                    )
                    .await,
                false,
            ),
            PromptInput::FollowUps { interrupted } => (Vec::new(), interrupted),
        };
        let mut prepared = 0;
        while let Some(QueuedFollowUp { prompt, row, .. }) = self.next_batch_follow_up(prepared) {
            prepared += 1;
            if let Some(row) = row {
                self.host.hide_queued_prompt(row).await;
            }
            let echo = mapper.user_query_action(&prompt.text, prompt.base.as_ref());
            self.apply_events(mapper, vec![TurnEvent::Actions(vec![echo])])
                .await?;
            let text = if interrupted {
                format!("{STEER_INTERRUPTION_NOTE}\n\n{}", prompt.text)
            } else {
                prompt.text
            };
            interrupted = false;
            content.extend(
                self.attachments
                    .prompt_content(text, prompt.attachments)
                    .await,
            );
        }
        Ok(content)
    }

    /// Runs one prompt turn to completion and finishes its last request stream.
    async fn run_turn(
        &mut self,
        mapper: &mut AcpTurnMapper,
        input: PromptInput,
    ) -> Result<TurnOutcome> {
        let started = Utc::now();
        self.state.set_in_turn(true);
        let cancels = self.cancels.clone();
        let prepared = {
            let prepare = self.prepare_prompt(mapper, input).fuse();
            let interrupt = async {
                if recv_or_pending(cancels.as_ref()).await.is_err() {
                    future::pending::<()>().await;
                }
            }
            .fuse();
            pin_mut!(prepare, interrupt);
            select_biased! {
                _ = interrupt => Ok(None),
                result = prepare => result.map(Some),
            }
        };
        let turn_result = match prepared {
            Ok(Some(prompt)) => self.prompt(mapper, prompt).await,
            Ok(None) => Ok(TurnOutcome::Cancelled),
            Err(error) => Err(error),
        };
        let (outcome, is_error) = match &turn_result {
            Ok(outcome) => (outcome.metric_label(), false),
            Err(_) => ("error", true),
        };
        self.telemetry
            .action(AcpActionEvent::Turn, outcome, started, is_error);
        // A turn that failed mid-prompt can leave steers unanswered; they are held like the rest.
        self.hold_steers();
        let finish_events = mapper.finish_events();
        if let Err(error) = self.apply_events(mapper, finish_events).await {
            log::warn!("Failed to settle ACP tool calls: {error}");
        }
        // The settling flag must be accurate before the status change that ends the turn is
        // emitted, since that is what gates the driver's idle exit.
        let has_follow_ups = !self.pending_follow_ups().is_empty();
        self.state.set_settling(has_follow_ups);
        let stream_id = self.turn.stream_id.clone();
        match &turn_result {
            Ok(TurnOutcome::Cancelled) => {
                let by = if has_follow_ups {
                    TurnCancellation::FollowUp
                } else {
                    TurnCancellation::Interrupt
                };
                self.host.cancel_turn(stream_id, by).await;
            }
            Ok(TurnOutcome::Stopped(stop_reason)) => {
                self.finish_turn(stream_id, stream_finished_for(*stop_reason))
                    .await;
            }
            Err(error) => {
                log::error!("ACP turn failed: {error:#}");
                let finished = StreamFinished {
                    reason: Some(stream_finished::Reason::InternalError(
                        stream_finished::InternalError {
                            message: format!("{error:#}"),
                        },
                    )),
                    ..Default::default()
                };
                self.finish_turn(stream_id, finished).await;
            }
        }
        self.request_save().await;
        self.state.set_in_turn(false);
        // Follow-ups held through a failed turn go out as the next turn while the agent is still
        // connected; once it has gone they cannot be delivered.
        if turn_result.is_err() && self.notifications.is_closed() {
            self.fail_queued(DropReason::AgentDisconnected).await;
        }
        turn_result
    }

    async fn finish_turn(&self, stream_id: ResponseStreamId, finished: StreamFinished) {
        if self.host.finish_turn(stream_id, finished).await.is_err() {
            log::warn!("Agent driver dropped before the ACP turn could be finished");
        }
    }

    /// Captures the conversation at the request-stream boundary just closed and asks the driver
    /// to upload it. The capture happens here, before another stream can open, so the upload
    /// is of a conversation whose every request has finished.
    async fn request_save(&self) {
        match self.host.capture_snapshot().await {
            Ok(Some(snapshot)) => *self.boundary_snapshot.lock() = Some(snapshot),
            Ok(None) => return,
            Err(error) => {
                log::warn!("Skipping ACP save: {error:#}");
                return;
            }
        }
        if self.save_requests.try_send(SavePoint::PostTurn).is_err() {
            log::debug!("ACP save request dropped; the runner is gone");
        }
    }

    fn follow_up_cancel_retries(&self) -> FuturesOrdered<Deadline> {
        let mut after = Duration::ZERO;
        let mut delay = FOLLOW_UP_CANCEL_RETRY_BASE_DELAY;
        (1..MAX_FOLLOW_UP_CANCEL_ATTEMPTS)
            .map(|_| {
                after = after.saturating_add(delay);
                delay = delay.saturating_mul(2);
                (self.timer_factory)(after)
            })
            .collect()
    }

    /// Sends the prompt and applies updates until its response arrives. A follow-up arriving
    /// meanwhile is steered into the turn when the agent supports it; otherwise it, like an
    /// interrupt, cancels the turn.
    async fn prompt(
        &mut self,
        mapper: &mut AcpTurnMapper,
        prompt: Vec<ContentBlock>,
    ) -> Result<TurnOutcome> {
        let connection = self.connection.clone();
        // Flushing orders our cancel after the prompt, but ACP has no registration acknowledgement.
        let prompt_fut = connection
            .send_sequenced_request::<_, PromptResponse>(
                protocol::METHOD_SESSION_PROMPT,
                PromptParams {
                    session_id: self.session_id.clone(),
                    prompt,
                },
            )
            .await?
            .fuse();
        pin_mut!(prompt_fut);
        let mut cancel = CancelState::none();
        let mut steer_fut = self.steer_held_overflow().await;
        loop {
            let cancels = self.cancels.as_ref();
            let follow_ups = self.follow_ups.as_ref();
            let notifications = &self.notifications;
            let late_steers = &self.late_steers;
            let mut deadline = &mut cancel.deadline;
            let retries = &mut cancel.retries;
            let wake = select_biased! {
                response = prompt_fut => Wake::Response(response),
                interrupt = recv_or_pending(cancels).fuse() => Wake::Cancel(interrupt),
                // After the response, so a turn that has ended is settled before a steer's answer
                // is read: a prompt the agent left for the client must not cancel it.
                result = steer_fut => Wake::Steer(result),
                _ = deadline => Wake::Deadline,
                _ = async {
                    if retries.next().await.is_none() {
                        future::pending::<()>().await;
                    }
                }.fuse() => Wake::Retry,
                result = next_late_steer(late_steers).fuse() => Wake::LateSteer(result),
                input = async {
                    select! {
                        follow_up = recv_or_pending(follow_ups).fuse() => {
                            Wake::FollowUp(follow_up)
                        }
                        notification = notifications.recv().fuse() => {
                            Wake::Notification(notification)
                        }
                    }
                }.fuse() => input,
            };
            match wake {
                Wake::Notification(Ok(notification)) => {
                    self.handle_notification(mapper, notification).await?;
                }
                // The agent closed its stdout; the pending prompt resolves with the error.
                Wake::Notification(Err(_)) => {
                    self.hold_steers();
                    break;
                }
                Wake::Response((arrival, response)) => {
                    while let Ok(notification) = self.notifications.try_recv() {
                        self.handle_notification(mapper, notification).await?;
                    }
                    if let Some((outcome, started)) = cancel.take_acknowledged() {
                        self.telemetry
                            .action(AcpActionEvent::Cancel, outcome, started, false);
                    }
                    if self.steering.is_some() {
                        self.settle_steer_after_turn(mapper, steer_fut, arrival)
                            .await?;
                    }
                    return self.settle_response(response, cancel.by.is_some());
                }
                // Steers only start while nothing is held, so they never overtake an older
                // follow-up.
                Wake::FollowUp(Ok(prompt))
                    if cancel.by.is_none() && self.steering_supported && self.queued.is_empty() =>
                {
                    let row = self.host.show_queued_prompt(&prompt).await;
                    let follow_up = QueuedFollowUp::new(prompt, row);
                    if self.steering.is_some() {
                        self.awaiting_steer.push_back(follow_up);
                    } else {
                        steer_fut = self.start_steer(&follow_up.prompt);
                        self.steering = Some(follow_up);
                    }
                }
                Wake::FollowUp(Ok(prompt)) => {
                    let received = self.hold_follow_up(prompt).await;
                    if cancel.by.is_none() {
                        self.telemetry.action(
                            AcpActionEvent::Steer,
                            "cancel_fallback",
                            received,
                            false,
                        );
                        self.begin_follow_up_cancel(&mut cancel).await?;
                    }
                }
                Wake::FollowUp(Err(_)) => self.follow_ups = None,
                Wake::Cancel(Ok(ExternalHarnessCancel)) => {
                    if cancel.by != Some(TurnCancellation::Interrupt) {
                        self.begin_interrupt(&mut cancel).await?;
                    }
                }
                Wake::Cancel(Err(_)) => self.cancels = None,
                Wake::Steer((_, result)) => {
                    steer_fut = no_steer();
                    let Some(steer) = self.steering.take() else {
                        continue;
                    };
                    let verdict = self.classify_steer(result);
                    self.report_steer(&verdict, steer.received);
                    match verdict {
                        SteerVerdict::Injected => {
                            self.steer_taken(mapper, steer).await?;
                            // Nothing joins a turn that is being cancelled: what is still
                            // waiting goes out as the next turn instead.
                            if cancel.by.is_none() {
                                steer_fut = self.start_next_steer();
                            } else {
                                self.hold_steers();
                            }
                        }
                        // The turn is ending on its own (or the agent is waiting on the client
                        // for it); the prompt goes out once it has, with no cancel.
                        SteerVerdict::PromptRequired => self.hold_steer(steer),
                        SteerVerdict::Rejected => {
                            self.hold_steer(steer);
                            if cancel.by.is_none() {
                                self.begin_follow_up_cancel(&mut cancel).await?;
                            }
                        }
                        SteerVerdict::DetachedTurn => {
                            self.hold_steer(steer);
                            if cancel.by.is_none() {
                                self.begin_follow_up_cancel(&mut cancel).await?;
                            } else {
                                // The detached turn began after the cancel already sent.
                                self.start_cancel(&mut cancel).await?;
                            }
                        }
                    }
                }
                Wake::LateSteer((_, result)) => self.note_late_steer(result),
                Wake::Deadline => match cancel.by {
                    Some(TurnCancellation::Interrupt) => {
                        return self.cancel_timed_out(cancel.started.take()).await;
                    }
                    Some(TurnCancellation::FollowUp) => {
                        log::info!(
                            "ACP follow-up cancellation grace expired; waiting for the turn to finish"
                        );
                        if let Some(started) = cancel.started.take() {
                            self.telemetry.action(
                                AcpActionEvent::Cancel,
                                "timed_out",
                                started,
                                true,
                            );
                        }
                        cancel.deadline = no_deadline();
                        cancel.retries = FuturesOrdered::new();
                    }
                    None => unreachable!(),
                },
                Wake::Retry => self.start_cancel(&mut cancel).await?,
            }
        }
        let (_, response) = prompt_fut.await;
        self.settle_response(response, cancel.by.is_some())
    }

    /// Cancels the turn for a follow-up. `session/cancel` is re-sent on a backoff within the
    /// grace period; past it the turn is left to finish on its own.
    async fn begin_follow_up_cancel(&self, cancel: &mut CancelState) -> Result<()> {
        cancel.by = Some(TurnCancellation::FollowUp);
        cancel.deadline = (self.timer_factory)(self.cancel_deadline);
        cancel.retries = self.follow_up_cancel_retries();
        self.start_cancel(cancel).await
    }

    /// Cancels the turn for an interrupt, which takes over from a follow-up cancellation: past
    /// the deadline the agent's terminal is interrupted.
    async fn begin_interrupt(&self, cancel: &mut CancelState) -> Result<()> {
        cancel.by = Some(TurnCancellation::Interrupt);
        cancel.deadline = (self.timer_factory)(self.cancel_deadline);
        cancel.retries = FuturesOrdered::new();
        self.start_cancel(cancel).await
    }

    /// Shows a follow-up as queued until the active prompt settles. Returns when it arrived.
    async fn hold_follow_up(&mut self, prompt: ExternalHarnessPrompt) -> DateTime<Utc> {
        let row = self.host.show_queued_prompt(&prompt).await;
        let follow_up = QueuedFollowUp::new(prompt, row);
        let received = follow_up.received;
        self.queued.push_back(follow_up);
        received
    }

    fn report_steer(&self, verdict: &SteerVerdict, received: DateTime<Utc>) {
        self.telemetry.action(
            AcpActionEvent::Steer,
            verdict.metric_label(),
            received,
            verdict.is_error(),
        );
    }

    /// Moves every follow-up still waiting to be steered to the held prompts, which go out in a
    /// turn of their own. Steers only start while nothing is held, so whatever is held already
    /// arrived after them and stays behind them.
    fn hold_steers(&mut self) {
        let later = std::mem::take(&mut self.queued);
        self.queued.extend(self.steering.take());
        self.queued.append(&mut self.awaiting_steer);
        self.queued.extend(later);
    }

    /// Holds `steer`, whose answer left it unsent, ahead of everything waiting behind it.
    fn hold_steer(&mut self, steer: QueuedFollowUp) {
        self.steering = Some(steer);
        self.hold_steers();
    }

    /// Follow-ups the prompt could not carry (more were ready than one prompt takes) join the
    /// turn by steering, in order, rather than wait a whole turn or cancel it. Returns the first
    /// steer's answer, or a future that never resolves when there is nothing to steer.
    async fn steer_held_overflow(&mut self) -> SteerFuture {
        if !self.steering_supported || self.queued.is_empty() {
            return no_steer();
        }
        let mut overflow = std::mem::take(&mut self.queued);
        for follow_up in &mut overflow {
            if follow_up.row.is_none() {
                follow_up.row = self.host.show_queued_prompt(&follow_up.prompt).await;
            }
        }
        self.awaiting_steer.append(&mut overflow);
        self.start_next_steer()
    }

    /// Steers the next follow-up waiting for it, if any.
    fn start_next_steer(&mut self) -> SteerFuture {
        match self.awaiting_steer.pop_front() {
            Some(next) => {
                let steer_fut = self.start_steer(&next.prompt);
                self.steering = Some(next);
                steer_fut
            }
            None => no_steer(),
        }
    }

    /// Asks the agent to add `prompt` to the turn in flight. The request runs to completion on
    /// its own so the turn's updates keep flowing meanwhile.
    fn start_steer(&self, prompt: &ExternalHarnessPrompt) -> SteerFuture {
        log::info!("Steering the current ACP turn with a follow-up");
        let connection = self.connection.clone();
        let attachments = self.attachments.clone();
        let session_id = self.session_id.clone();
        let text = prompt.text.clone();
        let prompt_attachments = prompt.attachments.clone();
        Box::pin(
            async move {
                let prompt = attachments.prompt_content(text, prompt_attachments).await;
                let sent = connection
                    .send_sequenced_request::<_, SteeringResponse>(
                        protocol::METHOD_SESSION_STEERING,
                        SteeringParams {
                            session_id,
                            prompt,
                            meta: SteeringMeta {
                                steering: SteeringOptions {
                                    idle_behavior: SteeringIdleBehavior::PromptRequired,
                                },
                            },
                        },
                    )
                    .await;
                match sent {
                    Ok(answer) => answer.await,
                    Err(error) => (Arrival::NEVER, Err(error)),
                }
            }
            .fuse(),
        )
    }

    /// Reads the agent's answer to a steer. An error the agent returned, as opposed to the
    /// connection failing, means it rejects the method or this session despite advertising it
    /// and would again, so steering is turned off for the rest of the session.
    fn classify_steer(&mut self, result: Result<SteeringResponse>) -> SteerVerdict {
        let outcome = match result {
            Ok(response) => response.outcome,
            Err(error) => {
                match error.downcast_ref::<RpcError>() {
                    Some(rejection) => {
                        log::warn!(
                            "The ACP agent advertised steering but rejected {} ({}: {}); \
                             follow-ups will cancel the turn instead",
                            protocol::METHOD_SESSION_STEERING,
                            rejection.code,
                            rejection.message
                        );
                        self.steering_supported = false;
                    }
                    None => {
                        log::warn!(
                            "ACP steer failed; the follow-up will cancel the turn: {error:#}"
                        )
                    }
                }
                return SteerVerdict::Rejected;
            }
        };
        match outcome {
            SteeringOutcome::Injected => {
                log::info!("ACP steer injected into the running turn");
                SteerVerdict::Injected
            }
            SteeringOutcome::StartedNewTurn => {
                log::warn!(
                    "ACP steer found no running turn and the agent started one of its own; \
                     stopping it so the prompt can be sent as a turn"
                );
                SteerVerdict::DetachedTurn
            }
            SteeringOutcome::PromptRequired => {
                log::info!(
                    "ACP steer found no turn to join; the prompt will be sent as a turn once \
                     this one ends"
                );
                SteerVerdict::PromptRequired
            }
            SteeringOutcome::Failed | SteeringOutcome::Unknown => {
                log::warn!(
                    "ACP steer was not taken ({outcome:?}); the follow-up will cancel the turn"
                );
                SteerVerdict::Rejected
            }
        }
    }

    /// The agent took `steer` into the turn. Its queued row goes, what the agent streamed before
    /// answering lands ahead of the echo, and the prompt is echoed where it joined. `steer` is
    /// consumed whatever the host makes of that: a failure here must not send it again.
    async fn steer_taken(
        &mut self,
        mapper: &mut AcpTurnMapper,
        steer: QueuedFollowUp,
    ) -> Result<()> {
        if let Some(row) = steer.row {
            self.host.hide_queued_prompt(row).await;
        }
        while let Ok(notification) = self.notifications.try_recv() {
            self.handle_notification(mapper, notification).await?;
        }
        let echo = mapper.steered_user_query_action(&steer.prompt.text, steer.prompt.base.as_ref());
        self.apply_events(mapper, vec![TurnEvent::Actions(vec![echo])])
            .await
    }

    /// The turn's response arrived while a steer was outstanding. Its answer, awaited for at
    /// most [`Self::steer_settle_deadline`], is read against the response: one the agent wrote
    /// first came while the turn was still running and means what it would have mid-turn, while
    /// one written after the response can no longer mean the prompt was replied to, so whatever
    /// it says the prompt is held and sent as a turn. If none comes in time the prompt is sent
    /// as a turn too; the late answer is still read, so a steer the agent took after all is
    /// recognised and not echoed. Follow-ups waiting behind the steer are held with it.
    async fn settle_steer_after_turn(
        &mut self,
        mapper: &mut AcpTurnMapper,
        mut steer_fut: SteerFuture,
        response_arrival: Arrival,
    ) -> Result<()> {
        let mut settle = (self.timer_factory)(self.steer_settle_deadline);
        let answer = select! {
            answer = steer_fut => Some(answer),
            _ = settle => None,
        };
        let Some(steer) = self.steering.take() else {
            return Ok(());
        };
        match answer {
            None => {
                log::warn!(
                    "The ACP agent did not answer a steer within {:?} of the turn ending; \
                     sending the prompt as a turn",
                    self.steer_settle_deadline
                );
                self.telemetry
                    .action(AcpActionEvent::Steer, "timed_out", steer.received, true);
                self.late_steers.lock().push(steer_fut);
            }
            Some((arrival, result)) => {
                let in_turn = arrival < response_arrival;
                let verdict = self.classify_steer(result);
                self.report_steer(&verdict, steer.received);
                match verdict {
                    SteerVerdict::Injected if in_turn => {
                        self.steer_taken(mapper, steer).await?;
                        self.hold_steers();
                        return Ok(());
                    }
                    SteerVerdict::Injected => log::warn!(
                        "The ACP agent took a steer after the turn had ended, so nothing \
                         replied to it; sending the prompt as a turn"
                    ),
                    SteerVerdict::PromptRequired | SteerVerdict::Rejected => {}
                    SteerVerdict::DetachedTurn => {
                        if let Err(error) = self.send_cancel().await {
                            log::warn!("Failed to cancel the agent's detached ACP turn: {error:#}");
                        }
                    }
                }
            }
        }
        self.hold_steer(steer);
        Ok(())
    }

    /// A steer that outlived its turn was answered. Its prompt went out as a turn when the wait
    /// for this ran out, so the answer only tells whether the agent ended up with it twice.
    fn note_late_steer(&self, result: Result<SteeringResponse>) {
        match result {
            Ok(response) if response.outcome == SteeringOutcome::Injected => log::warn!(
                "The ACP agent took a steer long after its turn ended; the prompt, already sent \
                 as a turn, reached it twice"
            ),
            Ok(response) => log::info!(
                "A late ACP steer answered {:?}; its prompt was already sent as a turn",
                response.outcome
            ),
            Err(error) => log::debug!("A late ACP steer failed: {error:#}"),
        }
    }

    /// Asks the agent to end the turn in flight, counting the send on `cancel` so the answer
    /// can be reported against it.
    async fn start_cancel(&self, cancel: &mut CancelState) -> Result<()> {
        cancel.started.get_or_insert_with(Utc::now);
        cancel.sends += 1;
        self.send_cancel().await
    }

    /// Asks the agent to end the current turn. Permission requests it is waiting on are answered
    /// `cancelled` at once, as the protocol requires, so it is not left deciding for work that
    /// will not proceed.
    async fn send_cancel(&self) -> Result<()> {
        self.state.set_settling(true);
        log::info!("Cancelling the current ACP turn");
        self.connection
            .notify(
                protocol::METHOD_SESSION_CANCEL,
                CancelParams {
                    session_id: self.session_id.clone(),
                },
            )
            .await?;
        let answered = self
            .connection
            .cancel_pending_permission_requests(&self.session_id)
            .await;
        if answered > 0 {
            log::info!("Answered {answered} pending ACP permission request(s) with cancelled");
        }
        Ok(())
    }

    /// Maps the prompt response to the turn's outcome. While a cancel is pending an agent error
    /// is taken as the cancellation (some agents report an aborted turn that way rather than
    /// with a `cancelled` stop reason).
    fn settle_response(
        &self,
        response: Result<PromptResponse>,
        cancel_pending: bool,
    ) -> Result<TurnOutcome> {
        match response {
            Ok(response) if response.stop_reason == StopReason::Cancelled => {
                Ok(TurnOutcome::Cancelled)
            }
            Ok(response) => Ok(TurnOutcome::Stopped(response.stop_reason)),
            Err(error) if cancel_pending && error.downcast_ref::<RpcError>().is_some() => {
                log::info!(
                    "Treating the agent's error while cancelling as the cancellation: {error}"
                );
                Ok(TurnOutcome::Cancelled)
            }
            Err(error) => Err(error),
        }
    }

    /// The agent did not answer `session/cancel` in time: its block is interrupted (the
    /// driver's exit ladder force-kills it if that is not enough) and the turn is failed.
    /// `cancel_started` is when the first `session/cancel` went out.
    async fn cancel_timed_out(
        &mut self,
        cancel_started: Option<DateTime<Utc>>,
    ) -> Result<TurnOutcome> {
        let reason = format!(
            "the agent did not answer session/cancel within {:?}",
            self.cancel_deadline
        );
        log::warn!("{reason}; interrupting its terminal");
        if let Some(started) = cancel_started {
            self.telemetry
                .action(AcpActionEvent::Cancel, "timed_out", started, true);
        }
        self.host.interrupt_terminal().await;
        self.fail_queued(DropReason::CancelTimedOut).await;
        Err(anyhow!(reason))
    }

    /// Drops every held follow-up; `reason` is why they will not be sent.
    async fn fail_queued(&mut self, reason: DropReason) {
        self.state.set_settling(false);
        self.hold_steers();
        if !self.queued.is_empty() {
            log::warn!(
                "Dropping {} held ACP follow-up(s): {reason}",
                self.queued.len()
            );
        }
        while let Some(QueuedFollowUp { row, received, .. }) = self.queued.pop_front() {
            self.telemetry.action(
                AcpActionEvent::FollowupsDropped,
                reason.metric_label(),
                received,
                true,
            );
            if let Some(row) = row {
                self.host.hide_queued_prompt(row).await;
            }
        }
    }

    async fn handle_notification(
        &mut self,
        mapper: &mut AcpTurnMapper,
        notification: InboundNotification,
    ) -> Result<()> {
        if notification.method != protocol::METHOD_SESSION_UPDATE {
            log::debug!("Ignoring ACP notification {}", notification.method);
            return Ok(());
        }
        let params: SessionUpdateParams = match serde_json::from_value(notification.params) {
            Ok(params) => params,
            Err(error) => {
                log::warn!("Ignoring unparseable ACP session/update: {error}");
                return Ok(());
            }
        };
        if let SessionUpdate::AgentMessageChunk { content, .. } = &params.update
            && self.suppressed_startup_message.as_deref() == Some(content.as_text().trim())
        {
            self.suppressed_startup_message = None;
            return Ok(());
        }
        let events = mapper.map_update(params.update);
        if self.first_output_pending
            && events
                .iter()
                .any(|event| matches!(event, TurnEvent::Actions(actions) if !actions.is_empty()))
        {
            self.first_output_pending = false;
            self.telemetry.first_output();
        }
        self.apply_events(mapper, events).await
    }

    async fn apply_events(
        &mut self,
        mapper: &mut AcpTurnMapper,
        events: Vec<TurnEvent>,
    ) -> Result<()> {
        for event in events {
            match event {
                TurnEvent::Actions(actions) => {
                    self.host
                        .apply_actions(&self.turn.stream_id, actions)
                        .await?;
                }
                TurnEvent::SegmentBoundary => {
                    self.host
                        .finish_turn(
                            self.turn.stream_id.clone(),
                            stream_finished_for(StopReason::EndTurn),
                        )
                        .await?;
                    self.request_save().await;
                    let next = self.host.begin_turn(None).await?;
                    mapper.start_segment(next.request_id.clone());
                    self.turn = next;
                }
            }
        }
        Ok(())
    }
}

pub(super) fn stream_finished_for(stop_reason: StopReason) -> StreamFinished {
    let reason = match stop_reason {
        StopReason::EndTurn | StopReason::Cancelled => {
            stream_finished::Reason::Done(stream_finished::Done {})
        }
        StopReason::MaxTokens => {
            stream_finished::Reason::MaxTokenLimit(stream_finished::ReachedMaxTokenLimit {})
        }
        StopReason::MaxTurnRequests | StopReason::Refusal | StopReason::Unknown => {
            stream_finished::Reason::InternalError(stream_finished::InternalError {
                message: format!("ACP agent stopped: {stop_reason:?}"),
            })
        }
    };
    StreamFinished {
        reason: Some(reason),
        ..Default::default()
    }
}

#[cfg(test)]
#[path = "turn_driver_tests.rs"]
mod tests;
