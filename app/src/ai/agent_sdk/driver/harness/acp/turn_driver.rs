//! The prompt turns of an ACP session: sending `session/prompt`, applying what the agent streams
//! to the native conversation, and cutting a turn short when a follow-up or an interrupt arrives.
//!
//! ACP cannot add to a turn in flight, so a follow-up that lands mid-turn steers the agent the
//! way Zed does: `session/cancel`, keep applying what the agent still streams to the turn it
//! belongs to until that turn's response arrives, then prompt again. Ready follow-ups go out
//! together in order. If cancellation is ignored, they wait for the original turn to finish;
//! only an explicit interrupt imposes a deadline that interrupts the agent's terminal.
use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use event_listener::Event;
use futures::future::FusedFuture;
use futures::stream::FuturesOrdered;
use futures::{FutureExt, StreamExt, future, pin_mut, select, select_biased};
use parking_lot::Mutex;
use session_sharing_protocol::common::ParticipantId;
use warp_multi_agent_api::ClientAction;
use warp_multi_agent_api::response_event::{StreamFinished, stream_finished};
use warpui::r#async::Timer;

use super::super::SavePoint;
use super::attachments::AttachmentResolver;
use super::connection::{AcpConnection, InboundNotification, RpcError};
use super::mapping::{AcpTurnMapper, TurnEvent};
use super::protocol::{
    self, CancelParams, ContentBlock, PromptParams, PromptResponse, SessionUpdate,
    SessionUpdateParams, StopReason,
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
    /// Unsent follow-ups, sent together after the current turn settles.
    pub queued: VecDeque<QueuedFollowUp>,
    /// Cancellation grace period; [`STEER_CANCEL_DEADLINE`] outside tests.
    pub cancel_deadline: Duration,
    pub timer_factory: Arc<dyn Fn(Duration) -> Deadline + Send + Sync>,
}

impl TurnDriver {
    /// A failed turn is already reflected in the conversation status (the stream finishes with
    /// an internal error), which is what the task status follows; the block's exit code remains
    /// the agent's own.
    pub(super) async fn run(mut self, mapper: &mut AcpTurnMapper) {
        let mut turn_result = self.run_turn(mapper, PromptInput::Initial).await;

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
            };
            match wake {
                Wake::FollowUp(Ok(prompt)) => {
                    log::info!("Running ACP follow-up turn");
                    self.queued.push_back(QueuedFollowUp { prompt, row: None });
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
                Wake::Response(()) | Wake::Deadline | Wake::Retry => unreachable!(),
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
        interrupted: bool,
    ) -> Result<TurnOutcome> {
        let initiator = self
            .pending_follow_ups()
            .front()
            .map(|follow_up| follow_up.prompt.participant_id.clone());
        let next = match self.host.begin_turn(initiator).await {
            Ok(next) => next,
            Err(error) => {
                self.fail_queued("the follow-up turn could not be opened")
                    .await;
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
                    Ok(prompt) => self.queued.push_back(QueuedFollowUp { prompt, row: None }),
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
        while let Some(QueuedFollowUp { prompt, row }) = self.next_batch_follow_up(prepared) {
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

    /// Sends the prompt and applies updates until its response arrives.
    async fn prompt(
        &mut self,
        mapper: &mut AcpTurnMapper,
        prompt: Vec<ContentBlock>,
    ) -> Result<TurnOutcome> {
        let connection = self.connection.clone();
        // Flushing orders our cancel after the prompt, but ACP has no registration acknowledgement.
        let prompt_fut = connection
            .send_request::<_, PromptResponse>(
                protocol::METHOD_SESSION_PROMPT,
                PromptParams {
                    session_id: self.session_id.clone(),
                    prompt,
                },
            )
            .await?
            .fuse();
        pin_mut!(prompt_fut);
        let mut cancellation = None;
        let mut deadline = no_deadline();
        let mut retries = FuturesOrdered::new();
        loop {
            let wake = select_biased! {
                response = prompt_fut => Wake::Response(response),
                interrupt = recv_or_pending(self.cancels.as_ref()).fuse() => Wake::Cancel(interrupt),
                _ = deadline => Wake::Deadline,
                _ = async {
                    if retries.next().await.is_none() {
                        future::pending::<()>().await;
                    }
                }.fuse() => Wake::Retry,
                input = async {
                    select! {
                        follow_up = recv_or_pending(self.follow_ups.as_ref()).fuse() => {
                            Wake::FollowUp(follow_up)
                        }
                        notification = self.notifications.recv().fuse() => {
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
                Wake::Notification(Err(_)) => break,
                Wake::Response(response) => {
                    while let Ok(notification) = self.notifications.try_recv() {
                        self.handle_notification(mapper, notification).await?;
                    }
                    return self.settle_response(response, cancellation.is_some());
                }
                Wake::FollowUp(Ok(prompt)) => {
                    self.hold_follow_up(prompt).await;
                    if cancellation.is_none() {
                        cancellation = Some(TurnCancellation::FollowUp);
                        deadline = (self.timer_factory)(self.cancel_deadline);
                        retries = self.follow_up_cancel_retries();
                        self.start_cancel().await?;
                    }
                }
                Wake::FollowUp(Err(_)) => self.follow_ups = None,
                Wake::Cancel(Ok(ExternalHarnessCancel)) => {
                    if cancellation != Some(TurnCancellation::Interrupt) {
                        cancellation = Some(TurnCancellation::Interrupt);
                        deadline = (self.timer_factory)(self.cancel_deadline);
                        retries = FuturesOrdered::new();
                        self.start_cancel().await?;
                    }
                }
                Wake::Cancel(Err(_)) => self.cancels = None,
                Wake::Deadline => match cancellation {
                    Some(TurnCancellation::Interrupt) => return self.cancel_timed_out().await,
                    Some(TurnCancellation::FollowUp) => {
                        log::info!(
                            "ACP follow-up cancellation grace expired; waiting for the turn to finish"
                        );
                        deadline = no_deadline();
                        retries = FuturesOrdered::new();
                    }
                    None => unreachable!(),
                },
                Wake::Retry => self.start_cancel().await?,
            }
        }
        let response = prompt_fut.await;
        self.settle_response(response, cancellation.is_some())
    }

    /// Shows a follow-up as queued until the active prompt settles.
    async fn hold_follow_up(&mut self, prompt: ExternalHarnessPrompt) {
        let row = self.host.show_queued_prompt(&prompt).await;
        self.queued.push_back(QueuedFollowUp { prompt, row });
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
