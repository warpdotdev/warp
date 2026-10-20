//! The prompt turns of an ACP session: sending `session/prompt`, applying what the agent streams
//! to the native conversation, and delivering follow-ups and interrupts that arrive mid-turn.
//!
//! A follow-up that lands mid-turn is added to the turn in flight when the agent supports the
//! `_session/steering` extension: the agent takes it at its next step without discarding the
//! work under way, and the running `session/prompt` keeps owning the output. Steers go out one
//! at a time, in arrival order. Without the extension, or when a steer does not take, the
//! follow-up steers the agent the way Zed does: `session/cancel`, keep applying what the agent
//! still streams to the turn it belongs to until that turn's response arrives, then prompt again
//! with a note that the previous turn was interrupted. Follow-ups that pile up while that one
//! cancel is pending go out together in the next prompt, in order. An interrupt is the same
//! without the prompt. Unlike Zed, the wait for the cancelled response is bounded: an agent that
//! never answers is interrupted in its terminal and the turn is failed.
use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use event_listener::Event;
use futures::future::FusedFuture;
use futures::{FutureExt, future, pin_mut, select, select_biased};
use parking_lot::Mutex;
use session_sharing_protocol::common::ParticipantId;
use warp_multi_agent_api::ClientAction;
use warp_multi_agent_api::response_event::{StreamFinished, stream_finished};
use warpui::r#async::Timer;

use super::super::SavePoint;
use super::attachments::AttachmentResolver;
use super::connection::{AcpConnection, InboundNotification, METHOD_NOT_FOUND, RpcError};
use super::mapping::{AcpTurnMapper, TurnEvent};
use super::protocol::{
    self, CancelParams, ContentBlock, PromptParams, PromptResponse, SessionUpdate,
    SessionUpdateParams, SteeringIdleBehavior, SteeringMeta, SteeringOptions, SteeringOutcome,
    SteeringParams, SteeringResponse, StopReason,
};
use crate::ai::attachment_utils::DownloadedAttachment;
use crate::ai::blocklist::{
    ExternalHarnessCancel, ExternalHarnessPrompt, ExternalHarnessTurn, QueuedQueryId,
    ResponseStreamId,
};

/// Bound on the agent answering `session/cancel` with the cancelled turn's response. Past it the
/// agent is interrupted in its terminal instead, and the turn is failed.
pub(super) const STEER_CANCEL_DEADLINE: Duration = Duration::from_secs(15);

/// How long a shutdown waits for the agent to answer the `session/cancel` sent on exit before
/// closing the connection. Shorter than [`STEER_CANCEL_DEADLINE`]: the driver's exit ladder
/// starts its own timers (SIGINT after 1s, SIGKILL after a further 14s) only once `exit`
/// returns, so this wait extends every in-turn shutdown by up to its full length. Three
/// seconds is enough for an agent to abort its model call and persist the turn (Pi writes its
/// session JSONL when the cancelled response goes out) without doubling the ladder.
pub(super) const EXIT_CANCEL_DEADLINE: Duration = Duration::from_secs(3);

/// How long a turn whose response has arrived waits for the answer to a `_session/steering`
/// still outstanding, which decides whether that prompt was taken or must be sent as a turn of
/// its own. Past it the prompt is sent again.
pub(super) const STEER_SETTLE_DEADLINE: Duration = Duration::from_secs(5);

/// Prefixed to a follow-up that cut the previous turn short, so the agent knows why its work
/// stopped where it did.
pub(super) const STEER_INTERRUPTION_NOTE: &str =
    "[The user interrupted your previous turn with this message.]";

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
    /// cancelled. Returns the row to hide once it is sent.
    async fn show_queued_prompt(&self, prompt: &ExternalHarnessPrompt) -> Option<QueuedQueryId>;
    async fn hide_queued_prompt(&self, query_id: QueuedQueryId);
    /// The turn driver has stopped taking prompts and cancellations; whatever routed them here
    /// must fall back to its native handling.
    async fn turn_driver_stopped(&self);
}

/// Turn activity shared with the harness runner, which reads it off the turn driver's task.
#[derive(Default)]
pub(super) struct TurnState {
    /// A turn has been cancelled and the status changes that follow are transitional: a
    /// follow-up is about to open a new turn, or the cancellation is still being settled.
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

/// What cut a turn short. The distinction is the conversation's status afterwards, which is
/// what the run state and its integrations mirror: a follow-up continues the conversation at
/// once, so the turn boundary is internal and the status stays `InProgress`, as when a native
/// follow-up interrupts a request; an interrupt ends it, as a native stop does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TurnCancellation {
    FollowUp,
    Interrupt,
}

/// A follow-up taken while a turn was still running, waiting to be steered into it or for it to
/// be cancelled.
pub(super) struct QueuedFollowUp {
    prompt: ExternalHarnessPrompt,
    row: Option<QueuedQueryId>,
}

/// What became of a steer once the agent answered.
enum SteerSettled {
    /// The agent took the prompt into the running turn.
    Injected,
    /// The prompt was not taken and is held for a turn of its own. `detached_turn` means the
    /// agent instead started a turn no `session/prompt` owns, which must be stopped.
    Held { detached_turn: bool },
}

enum Wake<T> {
    Notification(Result<InboundNotification, async_channel::RecvError>),
    FollowUp(Result<ExternalHarnessPrompt, async_channel::RecvError>),
    Cancel(Result<ExternalHarnessCancel, async_channel::RecvError>),
    Response(T),
    Steer(Result<SteeringResponse>),
    Deadline,
}

type Deadline = Pin<Box<dyn FusedFuture<Output = ()> + Send>>;

fn no_deadline() -> Deadline {
    Box::pin(future::pending().fuse())
}

fn cancel_deadline(after: Duration) -> Deadline {
    Box::pin(Timer::after(after).map(|_| ()).fuse())
}

type SteerFuture = Pin<Box<dyn FusedFuture<Output = Result<SteeringResponse>> + Send>>;

fn no_steer() -> SteerFuture {
    Box::pin(future::pending().fuse())
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
    /// Follow-ups taken mid-turn that will go out together in a turn of their own once the
    /// current one has ended.
    pub queued: VecDeque<QueuedFollowUp>,
    /// How long a `session/cancel` may go unanswered; [`STEER_CANCEL_DEADLINE`] outside tests.
    pub cancel_deadline: Duration,
    /// How long an ended turn waits for an outstanding steer's answer; [`STEER_SETTLE_DEADLINE`]
    /// outside tests.
    pub steer_settle_deadline: Duration,
    /// Whether the agent accepts `_session/steering`, as it announced at `initialize`. Cleared
    /// if the agent then rejects the method.
    pub steering_supported: bool,
    /// The follow-up whose `_session/steering` is outstanding.
    pub steering: Option<QueuedFollowUp>,
    /// Follow-ups to steer, in order, once the outstanding steer has been answered.
    pub awaiting_steer: VecDeque<QueuedFollowUp>,
}

impl TurnDriver {
    /// A failed turn is already reflected in the conversation status (the stream finishes with
    /// an internal error), which is what the task status follows; the block's exit code remains
    /// the agent's own.
    pub(super) async fn run(mut self, mapper: &mut AcpTurnMapper) {
        let initial = self
            .attachments
            .prompt_content_for_files(
                self.prompt_text.clone(),
                Vec::new(),
                &self.initial_attachments,
            )
            .await;
        let mut turn_result = self.run_turn(mapper, initial).await;

        log::info!("ACP turn finished; accepting follow-ups until the driver ends the run");
        loop {
            if !self.queued.is_empty() {
                let held: Vec<QueuedFollowUp> = self.queued.drain(..).collect();
                // Held prompts arrive here after a turn they cut short, or after one that ended
                // on its own while they waited to be steered; only the former was an
                // interruption from the agent's point of view.
                let interrupted = matches!(turn_result, Ok(TurnOutcome::Cancelled));
                log::info!(
                    "Running ACP follow-up turn for {} held prompt(s) (interrupted={interrupted})",
                    held.len()
                );
                turn_result = self.run_follow_ups(mapper, held, interrupted).await;
                continue;
            }
            let wake = select! {
                prompt = recv_or_pending(self.follow_ups.as_ref()).fuse() => Wake::FollowUp(prompt),
                cancel = recv_or_pending(self.cancels.as_ref()).fuse() => Wake::Cancel(cancel),
                notification = self.notifications.recv().fuse() => {
                    Wake::<()>::Notification(notification)
                }
            };
            match wake {
                Wake::FollowUp(Ok(prompt)) => {
                    log::info!("Running ACP follow-up turn");
                    let follow_up = QueuedFollowUp { prompt, row: None };
                    turn_result = self.run_follow_ups(mapper, vec![follow_up], false).await;
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
                Wake::Response(()) | Wake::Steer(_) | Wake::Deadline => unreachable!(),
            }
        }
        self.fail_queued("the harness exited before the follow-up could be sent")
            .await;
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
        follow_ups: Vec<QueuedFollowUp>,
        interrupted: bool,
    ) -> Result<TurnOutcome> {
        let mut prompts = Vec::with_capacity(follow_ups.len());
        for QueuedFollowUp { prompt, row } in follow_ups {
            if let Some(row) = row {
                self.host.hide_queued_prompt(row).await;
            }
            prompts.push(prompt);
        }
        let initiator = prompts.first().map(|prompt| prompt.participant_id.clone());
        let next = match self.host.begin_turn(initiator).await {
            Ok(next) => next,
            Err(error) => {
                self.state.set_settling(false);
                return Err(error);
            }
        };
        mapper.start_segment(next.request_id.clone());
        self.turn = next;
        // The new turn is under way; its `InProgress` status cancels any pending idle exit.
        self.state.set_settling(false);
        let echo = prompts
            .iter()
            .map(|prompt| mapper.user_query_action(&prompt.text, prompt.base.as_ref()))
            .collect();
        self.apply_events(mapper, vec![TurnEvent::Actions(echo)])
            .await?;
        let mut content = Vec::new();
        for (index, prompt) in prompts.into_iter().enumerate() {
            let text = if interrupted && index == 0 {
                format!("{STEER_INTERRUPTION_NOTE}\n\n{}", prompt.text)
            } else {
                prompt.text
            };
            content.extend(
                self.attachments
                    .prompt_content(text, prompt.attachments)
                    .await,
            );
        }
        self.run_turn(mapper, content).await
    }

    /// Runs one prompt turn to completion and finishes its last request stream.
    async fn run_turn(
        &mut self,
        mapper: &mut AcpTurnMapper,
        prompt: Vec<ContentBlock>,
    ) -> Result<TurnOutcome> {
        self.state.set_in_turn(true);
        let turn_result = self.prompt(mapper, prompt).await;
        let finish_events = mapper.finish_events();
        if let Err(error) = self.apply_events(mapper, finish_events).await {
            log::warn!("Failed to settle ACP tool calls: {error}");
        }
        // The settling flag must be accurate before the status change that ends the turn is
        // emitted, since that is what gates the driver's idle exit.
        self.state.set_settling(!self.queued.is_empty());
        let stream_id = self.turn.stream_id.clone();
        match &turn_result {
            Ok(TurnOutcome::Cancelled) => {
                let by = if self.queued.is_empty() {
                    TurnCancellation::Interrupt
                } else {
                    TurnCancellation::FollowUp
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
        if turn_result.is_err() {
            self.fail_queued("the harness turn failed before the follow-up could be sent")
                .await;
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

    /// Sends the prompt and applies every update the agent streams before responding. A
    /// follow-up arriving meanwhile is steered into the turn when the agent supports it;
    /// otherwise it, like an interrupt, cancels the turn. A cancelled turn's response is still
    /// awaited (bounded by [`STEER_CANCEL_DEADLINE`]) so late updates land in this turn.
    async fn prompt(
        &mut self,
        mapper: &mut AcpTurnMapper,
        prompt: Vec<ContentBlock>,
    ) -> Result<TurnOutcome> {
        let connection = self.connection.clone();
        let prompt_fut = connection
            .request::<_, PromptResponse>(
                protocol::METHOD_SESSION_PROMPT,
                PromptParams {
                    session_id: self.session_id.clone(),
                    prompt,
                },
            )
            .fuse();
        pin_mut!(prompt_fut);
        let mut cancel_pending = false;
        let mut deadline = no_deadline();
        let mut steer_fut = no_steer();
        loop {
            // Biased so that wire order wins when several arms are ready: a response that
            // arrived before a steer's answer must settle the turn before the answer is read,
            // or a prompt the agent left for us would cancel a turn that has already ended.
            let wake = select_biased! {
                notification = self.notifications.recv().fuse() => Wake::Notification(notification),
                response = prompt_fut => Wake::Response(response),
                follow_up = recv_or_pending(self.follow_ups.as_ref()).fuse() => {
                    Wake::FollowUp(follow_up)
                }
                interrupt = recv_or_pending(self.cancels.as_ref()).fuse() => Wake::Cancel(interrupt),
                result = steer_fut => Wake::Steer(result),
                _ = deadline => Wake::Deadline,
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
                Wake::Response(response) => {
                    while let Ok(notification) = self.notifications.try_recv() {
                        self.handle_notification(mapper, notification).await?;
                    }
                    if self.steering.is_some() {
                        self.settle_steer_after_turn(mapper, steer_fut).await;
                    }
                    return self.settle_response(response, cancel_pending);
                }
                Wake::FollowUp(Ok(prompt)) => {
                    if cancel_pending || !self.steering_supported {
                        self.hold_follow_up(prompt).await;
                        if !cancel_pending {
                            self.start_cancel().await?;
                            cancel_pending = true;
                            deadline = cancel_deadline(self.cancel_deadline);
                        }
                    } else {
                        let row = self.host.show_queued_prompt(&prompt).await;
                        let follow_up = QueuedFollowUp { prompt, row };
                        if self.steering.is_some() {
                            self.awaiting_steer.push_back(follow_up);
                        } else {
                            steer_fut = self.start_steer(&follow_up.prompt);
                            self.steering = Some(follow_up);
                        }
                    }
                }
                Wake::FollowUp(Err(_)) => self.follow_ups = None,
                Wake::Cancel(Ok(ExternalHarnessCancel)) => {
                    if !cancel_pending {
                        self.start_cancel().await?;
                        cancel_pending = true;
                        deadline = cancel_deadline(self.cancel_deadline);
                    }
                }
                Wake::Cancel(Err(_)) => self.cancels = None,
                Wake::Steer(result) => {
                    steer_fut = no_steer();
                    let Some(steer) = self.steering.take() else {
                        continue;
                    };
                    match self.settle_steer(mapper, steer, result).await? {
                        SteerSettled::Injected => {
                            if let Some(next) = self.awaiting_steer.pop_front() {
                                steer_fut = self.start_steer(&next.prompt);
                                self.steering = Some(next);
                            }
                        }
                        SteerSettled::Held { detached_turn } => {
                            self.hold_steers();
                            // A detached turn began after any cancel already sent, so it
                            // needs one of its own.
                            if detached_turn || !cancel_pending {
                                self.start_cancel().await?;
                                cancel_pending = true;
                                deadline = cancel_deadline(self.cancel_deadline);
                            }
                        }
                    }
                }
                Wake::Deadline => return self.cancel_timed_out().await,
            }
        }
        let response = prompt_fut.await;
        self.settle_response(response, cancel_pending)
    }

    /// Takes a follow-up that arrived mid-turn: it is shown as queued and runs once the turn
    /// has been cancelled.
    async fn hold_follow_up(&mut self, prompt: ExternalHarnessPrompt) {
        let row = self.host.show_queued_prompt(&prompt).await;
        self.queued.push_back(QueuedFollowUp { prompt, row });
    }

    /// Moves every follow-up still waiting to be steered to the held prompts, which go out in a
    /// turn of their own.
    fn hold_steers(&mut self) {
        self.queued.extend(self.steering.take());
        self.queued.extend(self.awaiting_steer.drain(..));
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
                connection
                    .request::<_, SteeringResponse>(
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
                    .await
            }
            .fuse(),
        )
    }

    /// Applies the agent's answer to a steer. A prompt the agent took is echoed into the turn;
    /// one it did not take is held for a turn of its own.
    async fn settle_steer(
        &mut self,
        mapper: &mut AcpTurnMapper,
        steer: QueuedFollowUp,
        result: Result<SteeringResponse>,
    ) -> Result<SteerSettled> {
        let outcome = match result {
            Ok(response) => response.outcome,
            Err(error) => {
                if error
                    .downcast_ref::<RpcError>()
                    .is_some_and(|error| error.code == METHOD_NOT_FOUND)
                {
                    log::warn!(
                        "The ACP agent advertised steering but rejects {}; follow-ups will \
                         cancel the turn instead",
                        protocol::METHOD_SESSION_STEERING
                    );
                    self.steering_supported = false;
                } else {
                    log::warn!("ACP steer failed; the follow-up will cancel the turn: {error:#}");
                }
                self.queued.push_back(steer);
                return Ok(SteerSettled::Held {
                    detached_turn: false,
                });
            }
        };
        match outcome {
            SteeringOutcome::Injected => {
                log::info!("ACP steer injected into the running turn");
                if let Some(row) = steer.row {
                    self.host.hide_queued_prompt(row).await;
                }
                let echo = mapper
                    .steered_user_query_action(&steer.prompt.text, steer.prompt.base.as_ref());
                self.apply_events(mapper, vec![TurnEvent::Actions(vec![echo])])
                    .await?;
                Ok(SteerSettled::Injected)
            }
            SteeringOutcome::StartedNewTurn => {
                log::warn!(
                    "ACP steer found no running turn and the agent started one of its own; \
                     stopping it so the prompt can be sent as a turn"
                );
                self.queued.push_back(steer);
                Ok(SteerSettled::Held {
                    detached_turn: true,
                })
            }
            SteeringOutcome::PromptRequired => {
                log::info!("ACP steer found no running turn; the prompt will be sent as a turn");
                self.queued.push_back(steer);
                Ok(SteerSettled::Held {
                    detached_turn: false,
                })
            }
            SteeringOutcome::Failed | SteeringOutcome::Unknown => {
                log::warn!(
                    "ACP steer was not taken ({outcome:?}); the follow-up will cancel the turn"
                );
                self.queued.push_back(steer);
                Ok(SteerSettled::Held {
                    detached_turn: false,
                })
            }
        }
    }

    /// The turn's response arrived while a steer was outstanding. Its answer, awaited for at
    /// most the settle deadline, decides whether that prompt still has to be sent; an
    /// unanswered steer is sent again rather than risk losing it. Follow-ups waiting behind it
    /// have no turn left to join and are held with it.
    async fn settle_steer_after_turn(
        &mut self,
        mapper: &mut AcpTurnMapper,
        steer_fut: SteerFuture,
    ) {
        let settle = Timer::after(self.steer_settle_deadline).fuse();
        pin_mut!(settle, steer_fut);
        let result = select! {
            result = steer_fut => Some(result),
            _ = settle => None,
        };
        let Some(steer) = self.steering.take() else {
            return;
        };
        match result {
            Some(result) => match self.settle_steer(mapper, steer, result).await {
                Ok(SteerSettled::Held {
                    detached_turn: true,
                }) => {
                    if let Err(error) = self.start_cancel().await {
                        log::warn!("Failed to cancel the agent's detached ACP turn: {error:#}");
                    }
                }
                Ok(SteerSettled::Injected | SteerSettled::Held { .. }) => {}
                Err(error) => log::warn!("Failed to settle an ACP steer: {error:#}"),
            },
            None => {
                log::warn!(
                    "The ACP agent did not answer a steer within {:?} of the turn ending; \
                     sending the prompt as a turn",
                    self.steer_settle_deadline
                );
                self.queued.push_back(steer);
            }
        }
        self.hold_steers();
    }

    /// Asks the agent to end the current turn. Permission requests it is waiting on are answered
    /// `cancelled` at once, as the protocol requires, so it is not left deciding for work that
    /// will not proceed.
    async fn start_cancel(&self) -> Result<()> {
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
    async fn cancel_timed_out(&mut self) -> Result<TurnOutcome> {
        let reason = format!(
            "the agent did not answer session/cancel within {:?}",
            self.cancel_deadline
        );
        log::warn!("{reason}; interrupting its terminal");
        self.host.interrupt_terminal().await;
        self.fail_queued(&reason).await;
        Err(anyhow!(reason))
    }

    /// Drops every held follow-up; `reason` is why they will not be sent.
    async fn fail_queued(&mut self, reason: &str) {
        self.state.set_settling(false);
        self.hold_steers();
        if !self.queued.is_empty() {
            log::warn!(
                "Dropping {} held ACP follow-up(s): {reason}",
                self.queued.len()
            );
        }
        while let Some(QueuedFollowUp { row, .. }) = self.queued.pop_front() {
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
