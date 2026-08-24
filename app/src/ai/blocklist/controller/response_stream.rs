use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Local, TimeDelta};
use futures::channel::oneshot;
use uuid::Uuid;
use warp_errors::report_error;
#[cfg(not(target_family = "wasm"))]
use warp_multi_agent_api as maa_api;
use warp_multi_agent_api::response_event;
use warpui::r#async::Timer;
use warpui::{Entity, ModelContext, SingletonEntity};

use crate::ai::agent::api::{self, ConvertToAPITypeError, generate_multi_agent_output};
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::{AIIdentifiers, CancellationReason};
use crate::network::NetworkStatus;
use crate::send_telemetry_from_ctx;
use crate::server::retry_strategies::backoff_after_attempts;
use crate::server::server_api::{AIApiError, ServerApiProvider};

/// Maximum number of recovery attempts spent on one request before the failure is
/// surfaced.
///
/// Retries (the same request re-sent) and resumes (a fresh `ResumeConversation` request)
/// draw from this single budget. Giving resumes their own one-shot allowance, as this code
/// used to, left the effective post-action budget at exactly one attempt — and during a
/// rolling server deploy that one attempt lands inside the same window of transport resets
/// that killed the original request.
const MAX_RECOVERY_ATTEMPTS: usize = 3;

/// Maximum time to wait for a request-time Grok OAuth token refresh before
/// sending with the currently stored token. Bounded so a hung refresh can't
/// stall the request.
#[cfg(not(target_family = "wasm"))]
const GROK_REFRESH_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// How long a request will hold for a request-time GEAP credential mint before
/// giving up and sending anyway.
#[cfg(not(target_family = "wasm"))]
const GEAP_REFRESH_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// The recovery budget for one request and the retries and resumes that recover it,
/// carried forward across each of those attempts.
///
/// A retry keeps the budget inside the same [`ResponseStream`]; a resume hands it to the
/// `ResumeConversation` request the controller sends next. So the two share one counter
/// rather than getting a budget each, and a failure can no longer exhaust recovery in a
/// single attempt.
///
/// The scope is one request, not one agent turn: a turn spans many MAA requests (every
/// tool-result round trip is its own), and each starts with a [`Self::fresh`] budget, as it
/// did before retries and resumes were unified.
///
/// `pub` only to match [`ResponseStream::new`], which takes one; every constructor and
/// accessor is crate-internal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecoveryBudget {
    attempts_used: usize,
    resume_allowed: bool,
}

impl RecoveryBudget {
    /// A full budget, for a request that is not itself recovering another.
    pub(crate) fn fresh() -> Self {
        Self {
            attempts_used: 0,
            resume_allowed: true,
        }
    }

    /// The same budget with resumes disallowed, for requests whose failures must stay
    /// silent and terminal (passive background requests).
    pub(crate) fn without_resume(self) -> Self {
        Self {
            resume_allowed: false,
            ..self
        }
    }

    /// Recovery attempts — retries and resumes — already spent recovering this request.
    pub(crate) fn attempts_used(self) -> usize {
        self.attempts_used
    }

    /// The budget for the next recovery attempt, with that attempt charged against it.
    pub(crate) fn next_attempt(self) -> Self {
        Self {
            attempts_used: self.attempts_used + 1,
            ..self
        }
    }

    fn has_remaining(self) -> bool {
        self.attempts_used < MAX_RECOVERY_ATTEMPTS
    }
}

/// A conversation resume scheduled for a failed request: the budget the resumed request
/// runs with, and how long to wait before sending it.
///
/// The wait is decided here, where the recovery decision is made, rather than recomputed
/// at send time — the schedule is jittered, so recomputing would produce a different
/// duration than the one that was logged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingResume {
    recovery: RecoveryBudget,
    backoff: Duration,
}

impl PendingResume {
    /// The budget the resumed request runs with, already charged for this resume.
    pub(crate) fn recovery(self) -> RecoveryBudget {
        self.recovery
    }

    /// How long to wait before sending the resume.
    pub(crate) fn backoff(self) -> Duration {
        self.backoff
    }

    #[cfg(test)]
    pub(crate) fn new_for_test(recovery: RecoveryBudget, backoff: Duration) -> Self {
        Self { recovery, backoff }
    }
}

/// What to do about a failed or truncated MAA response attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecoveryAction {
    /// Re-send the same request after a backoff.
    Retry,
    /// Re-send the same request once connectivity returns.
    RetryWhenOnline,
    /// Resume the conversation with a fresh request after the stream completes.
    Resume,
    /// Surface the error; the conversation ends in error.
    Fail(FailReason),
}

impl RecoveryAction {
    /// Which kind of recovery this is, for the recovery logs. Both retry variants share
    /// one label; the logged wait distinguishes a backed-off retry from a parked one.
    fn log_label(self) -> &'static str {
        match self {
            Self::Retry | Self::RetryWhenOnline => "retry",
            Self::Resume => "resume",
            Self::Fail(_) => "none",
        }
    }
}

/// Why a failed attempt is surfaced instead of recovered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FailReason {
    /// The error is not transient, so a fresh attempt would fail identically.
    NotRecoverable,
    /// The shared retry/resume budget is spent.
    BudgetExhausted,
    /// Only a resume could recover this failure, and this request may not resume.
    ResumeNotAllowed,
}

impl FailReason {
    fn log_label(self) -> &'static str {
        match self {
            Self::NotRecoverable => "not_recoverable",
            Self::BudgetExhausted => "budget_exhausted",
            Self::ResumeNotAllowed => "resume_not_allowed",
        }
    }
}

/// Decides how to recover from a failed response-stream attempt.
///
/// Before any client actions have been received, the request can be re-sent verbatim
/// (after a backoff, or once connectivity returns). After actions have streamed,
/// re-sending is unsafe, so recovery uses a fresh `ResumeConversation` request. Both draw
/// from `recovery`, so the kind of recovery available can change mid-chain without handing
/// the request a second budget.
fn recovery_action(
    has_received_client_actions: bool,
    is_recoverable: bool,
    recovery: RecoveryBudget,
    is_online: bool,
) -> RecoveryAction {
    if !is_recoverable {
        return RecoveryAction::Fail(FailReason::NotRecoverable);
    }
    // Checked ahead of the budget so a request that could never have resumed reports that,
    // rather than whichever constraint happens to bind first: a passive request that spent
    // its budget on pre-action retries and then fails post-action is blocked by both, and
    // the ineligibility is the one worth knowing.
    if has_received_client_actions && !recovery.resume_allowed {
        return RecoveryAction::Fail(FailReason::ResumeNotAllowed);
    }
    if !recovery.has_remaining() {
        return RecoveryAction::Fail(FailReason::BudgetExhausted);
    }
    if !has_received_client_actions {
        return if is_online {
            RecoveryAction::Retry
        } else {
            RecoveryAction::RetryWhenOnline
        };
    }
    RecoveryAction::Resume
}

/// Whether a failed attempt is being recovered or surfaced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecoveryOutcome {
    /// A recovery is in flight: the caller must not emit an error event or complete the
    /// stream for this attempt.
    InFlight,
    /// The failure has been reported and must be surfaced to the conversation.
    Surfaced,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ResponseStreamId(String);

impl ResponseStreamId {
    pub fn for_shared_session(init_event: &response_event::StreamInit) -> Self {
        // Make the stream ID unique per viewing by appending a local UUID
        // This prevents collisions when replaying the same conversation multiple times
        // (either on close-and-reopen or when viewing the same shared session from multiple terminals)
        Self(format!("{}-{}", init_event.request_id, Uuid::new_v4()))
    }

    #[cfg(test)]
    pub fn new_for_test() -> Self {
        Self(Uuid::new_v4().to_string())
    }
}

/// Model wrapping an agent API response stream.
///
/// Emits events when the output corresponding to the stream is updated, typically after receiving
/// each response chunk.
///
/// Handles retries internally - retries are only attempted if no ClientActions events have been
/// received yet, ensuring we don't retry after the AI has started executing actions. Once actions
/// have streamed, recovery falls to the controller's conversation resume; both draw from the one
/// [`RecoveryBudget`] the stream carries.
pub struct ResponseStream {
    id: ResponseStreamId,
    params: api::RequestParams,
    /// The shared retry/resume budget for this request, inherited from the request this one
    /// recovers (if any) and charged for each retry sent from this stream.
    recovery: RecoveryBudget,
    /// In-request retries sent from this stream.
    ///
    /// Deliberately not derived from [`Self::recovery`]: that budget is inherited across a
    /// resume, so it counts attempts made before this request existed and would overstate
    /// the retries this request actually needed.
    retries_sent: usize,
    start_time: DateTime<Local>,
    time_to_latest_event: TimeDelta,
    cancellation_tx: Option<oneshot::Sender<()>>,
    /// Store the original error for telemetry when retries succeed
    original_error: Option<String>,
    /// Track whether we've received any client actions
    /// If true, we cannot retry on subsequent errors since actions may have been executed
    has_received_client_actions: bool,
    /// AI identifiers for telemetry emission
    ai_identifiers: AIIdentifiers,

    /// The resume to send once the stream finishes, if one was scheduled.
    ///
    /// This is set when a transient network/server failure occurs after client actions
    /// have been received (so an in-request retry is unsafe) and the shared recovery
    /// budget still permits a resume. Per-attempt state: a retry supersedes it.
    pending_resume: Option<PendingResume>,

    /// Whether a `StreamFinished` event was received for the current request. A
    /// stream that completes without one was truncated in transit.
    stream_finished_received: bool,

    /// Whether a terminal error event has already been emitted for the current
    /// request, so stream completion doesn't synthesize a second failure for it.
    error_event_emitted: bool,

    /// Whether a retry is parked waiting for a backoff or for connectivity. While set,
    /// completion of the failed attempt's underlying stream is ignored.
    deferred_retry_pending: bool,

    /// Unique, internal id for the current request.
    ///
    /// This ensures that the model never emits events for a request that was already cancelled (or
    /// retried) and is still receiving lagging events.
    ///
    /// Note this is unique compared to `id`; this is unique across retry requests while the response
    /// stream id remains stable.
    current_request_id: Option<Uuid>,
}

impl ResponseStream {
    /// Emits a synthetic successful response event through the normal controller subscription.
    #[cfg(test)]
    pub fn emit_response_event_for_test(
        &mut self,
        event: warp_multi_agent_api::ResponseEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        ctx.emit(ResponseStreamEvent::ReceivedEvent(Consumable::new(Ok(
            event,
        ))));
    }
    #[cfg(test)]
    pub fn new_for_test(id: ResponseStreamId) -> Self {
        let (cancellation_tx, _rx) = oneshot::channel();
        Self {
            id,
            params: api::RequestParams::new_for_test(),
            recovery: RecoveryBudget::fresh().without_resume(),
            retries_sent: 0,
            start_time: Local::now(),
            time_to_latest_event: TimeDelta::seconds(0),
            cancellation_tx: Some(cancellation_tx),
            original_error: None,
            has_received_client_actions: false,
            ai_identifiers: AIIdentifiers::default(),
            pending_resume: None,
            stream_finished_received: false,
            error_event_emitted: false,
            deferred_retry_pending: false,
            current_request_id: Some(Uuid::new_v4()),
        }
    }

    pub fn new(
        params: api::RequestParams,
        ai_identifiers: AIIdentifiers,
        recovery: RecoveryBudget,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let (cancellation_tx, cancellation_rx) = oneshot::channel();
        let start_time = Local::now();

        let request_id = Uuid::new_v4();
        Self::spawn_request(request_id, params.clone(), cancellation_rx, ctx);
        Self {
            id: ResponseStreamId(Uuid::new_v4().to_string()),
            params,
            start_time,
            time_to_latest_event: TimeDelta::seconds(0),
            cancellation_tx: Some(cancellation_tx),
            recovery,
            retries_sent: 0,
            original_error: None,
            has_received_client_actions: false,
            ai_identifiers,
            pending_resume: None,
            stream_finished_received: false,
            error_event_emitted: false,
            deferred_retry_pending: false,
            current_request_id: Some(request_id),
        }
    }

    pub fn id(&self) -> &ResponseStreamId {
        &self.id
    }

    /// Returns true if we should attempt to resume the conversation after the stream finishes.
    pub fn should_resume_conversation_after_stream_finished(&self) -> bool {
        self.pending_resume.is_some()
    }

    /// The resume to send once the stream finishes, if one was scheduled. It carries this
    /// request's budget with the resume already charged against it, so the resumed request
    /// can't restart recovery from scratch.
    pub(super) fn pending_resume(&self) -> Option<PendingResume> {
        self.pending_resume
    }

    /// Whether the request that just failed was the turn's own request or an automatic
    /// resume of it. Logged so `attempt=1/3` on a resume can't be misread as the first
    /// failure of the original request.
    fn failed_request_label(&self) -> &'static str {
        let is_auto_resume = self
            .params
            .metadata
            .as_ref()
            .is_some_and(|metadata| metadata.is_auto_resume_after_error);
        if is_auto_resume { "resume" } else { "original" }
    }

    /// Helper function to emit AgentModeError telemetry for error that is retryable (not user visible).
    fn emit_retryable_agent_mode_error_telemetry(
        &self,
        error: String,
        ctx: &mut ModelContext<Self>,
    ) {
        send_telemetry_from_ctx!(
            crate::TelemetryEvent::AgentModeError {
                identifiers: self.ai_identifiers.clone(),
                error,
                is_user_visible: false,
                will_attempt_to_resume: false,
            },
            ctx
        );
    }

    fn retry(&mut self, ctx: &mut ModelContext<Self>) {
        self.recovery = self.recovery.next_attempt();
        self.retries_sent += 1;
        // Reset per-attempt state for the new attempt.
        self.has_received_client_actions = false;
        self.stream_finished_received = false;
        self.error_event_emitted = false;
        self.deferred_retry_pending = false;
        // A retry supersedes any resume this stream had scheduled. Unreachable today (the
        // eventsource closes on its first error, so a `Resume` decision is never followed by
        // another error on the same stream), but that depends on a transport detail several
        // crates away, and the retry backoff widens the window it holds in.
        self.pending_resume = None;

        let (cancellation_tx, cancellation_rx) = oneshot::channel();
        if let Some(old_cancellation_tx) = self.cancellation_tx.take() {
            let _ = old_cancellation_tx.send(());
        }
        self.cancellation_tx = Some(cancellation_tx);

        let request_id = Uuid::new_v4();
        self.current_request_id = Some(request_id);
        Self::spawn_request(request_id, self.params.clone(), cancellation_rx, ctx);
    }

    /// Decides how to recover from `error` and starts the recovery, or reports the failure
    /// so the caller can surface it.
    fn begin_recovery(
        &mut self,
        error: &Arc<AIApiError>,
        ctx: &mut ModelContext<Self>,
    ) -> RecoveryOutcome {
        let is_online = NetworkStatus::as_ref(ctx).is_online();
        let action = recovery_action(
            self.has_received_client_actions,
            error.is_recoverable(),
            self.recovery,
            is_online,
        );
        match action {
            RecoveryAction::Retry => {
                let delay = backoff_after_attempts(self.recovery.attempts_used() + 1);
                self.log_recovery(action, &format!("{delay:?}"), error);
                // Only emit error telemetry here if we're recovering in-request. Final
                // errors that aren't being retried are emitted elsewhere.
                self.emit_retryable_agent_mode_error_telemetry(format!("{error:?}"), ctx);
                self.defer_retry_after_backoff(delay, ctx);
                RecoveryOutcome::InFlight
            }
            RecoveryAction::RetryWhenOnline => {
                self.log_recovery(action, "connectivity", error);
                self.emit_retryable_agent_mode_error_telemetry(format!("{error:?}"), ctx);
                self.defer_retry_until_online(ctx);
                RecoveryOutcome::InFlight
            }
            RecoveryAction::Resume => {
                // The controller sends the resume once this stream finishes, after the same
                // backoff a retry would take. The failure is still surfaced, but as a
                // non-terminal `TransientError`, so the UI suppresses the banner.
                let delay = backoff_after_attempts(self.recovery.attempts_used() + 1);
                self.pending_resume = Some(PendingResume {
                    recovery: self.recovery.next_attempt(),
                    backoff: delay,
                });
                self.log_recovery(action, &format!("after_stream_finished+{delay:?}"), error);
                self.error_event_emitted = true;
                self.report_request_failure(error, is_online, self.recovery.attempts_used() + 1);
                RecoveryOutcome::Surfaced
            }
            RecoveryAction::Fail(reason) => {
                log::warn!(
                    "MultiAgent request failed; not recovering: recovery={} reason={} \
                     attempt={}/{MAX_RECOVERY_ATTEMPTS} failed_request={} - Error: {error:?}",
                    action.log_label(),
                    reason.log_label(),
                    self.recovery.attempts_used(),
                    self.failed_request_label(),
                );
                self.error_event_emitted = true;
                self.report_request_failure(error, is_online, self.recovery.attempts_used());
                RecoveryOutcome::Surfaced
            }
        }
    }

    /// Logs a recovery decision.
    ///
    /// Retries and resumes log the same fields in the same shape, with the attempt number
    /// read against the one shared budget, so a single line says which kind of recovery ran
    /// and where in the budget it sits.
    fn log_recovery(&self, action: RecoveryAction, wait: &str, error: &Arc<AIApiError>) {
        log::warn!(
            "MultiAgent request failed; recovering: recovery={} \
             attempt={}/{MAX_RECOVERY_ATTEMPTS} wait={wait} failed_request={} - Error: {error:?}",
            action.log_label(),
            self.recovery.attempts_used() + 1,
            self.failed_request_label(),
        );
    }

    /// Sends the request for `request_id`. When the request's model is served by
    /// the connected Grok subscription or may route to Gemini Enterprise, and
    /// that credential is already past hard expiry, this first blocks on a
    /// single shared refresh (owned by `ApiKeyManager`, so only one runs at a
    /// time) before sending. Requests with valid credentials, and requests for
    /// other providers, are sent directly.
    fn spawn_request(
        request_id: Uuid,
        params: api::RequestParams,
        cancellation_rx: oneshot::Receiver<()>,
        ctx: &mut ModelContext<Self>,
    ) {
        // The Grok subscription and its OAuth refresh are native-only.
        #[cfg(not(target_family = "wasm"))]
        {
            use ::ai::api_keys::{ApiKeyManager, GeapRefreshOutcome, GrokRefreshOutcome};
            use warpui::r#async::FutureExt as _;

            use crate::ai::llms::{LLMModelHost, LLMPreferences, LLMProvider};
            use crate::workspaces::user_workspaces::UserWorkspaces;

            // Only touch the Grok token for requests that actually use the Grok
            // subscription. The subscription is the only client-side source of
            // xAI auth (there's no BYO xAI key), so a base model whose provider
            // is xAI is exactly a subscription request.
            let uses_grok_subscription = LLMPreferences::as_ref(ctx)
                .get_llm_info(&params.model)
                .is_some_and(|info| info.provider == LLMProvider::Xai);
            if uses_grok_subscription {
                let byo_allowed = UserWorkspaces::as_ref(ctx).is_byo_api_key_enabled(ctx);
                // Reserve + start the shared refresh on `ApiKeyManager`'s context;
                // the in-flight guard is released there even if this stream is
                // dropped mid-refresh. `None` means the token is already usable.
                let refresh_rx = ApiKeyManager::handle(ctx).update(ctx, |manager, ctx| {
                    manager.begin_expired_grok_refresh(byo_allowed, ctx)
                });
                if let Some(refresh_rx) = refresh_rx {
                    let _ = ctx.spawn(
                        async move {
                            // Block on the shared refresh, bounded so a hung
                            // refresh can't stall the request forever.
                            refresh_rx.with_timeout(GROK_REFRESH_REQUEST_TIMEOUT).await
                        },
                        move |me, result, ctx| {
                            // Cancelled or superseded while refreshing — drop this attempt.
                            if me.current_request_id != Some(request_id) {
                                return;
                            }
                            if matches!(result, Ok(Ok(GrokRefreshOutcome::Refreshed))) {
                                // Send with the freshly refreshed token.
                                if let Some(access_token) = ApiKeyManager::as_ref(ctx)
                                    .grok_tokens()
                                    .and_then(|tokens| tokens.access_token_for_request())
                                    .map(str::to_owned)
                                    && let Some(keys) = me.params.api_keys.as_mut()
                                {
                                    keys.grok_oauth_access_token = access_token;
                                }
                                Self::spawn_generate(
                                    request_id,
                                    me.params.clone(),
                                    cancellation_rx,
                                    ctx,
                                );
                            } else {
                                // The refresh failed or timed out: don't send with
                                // the dead token — surface a terminal error asking
                                // the user to reconnect their subscription.
                                me.surface_grok_refresh_failure(request_id, ctx);
                            }
                        },
                    );
                    return;
                }
            }

            let uses_geap = LLMPreferences::as_ref(ctx)
                .get_llm_info(&params.model)
                .is_some_and(|info| {
                    info.host_configs
                        .get(&LLMModelHost::GeminiEnterprise)
                        .is_some_and(|host| host.enabled)
                });
            if uses_geap
                && let Some(binding) =
                    crate::ai::geap_credentials::current_geap_policy(ctx).mint_binding()
            {
                let refresh_binding = binding.clone();
                let refresh_rx = ApiKeyManager::handle(ctx).update(ctx, |manager, ctx| {
                    manager.begin_expired_geap_refresh(&binding, ctx, |manager, waiter, ctx| {
                        crate::ai::geap_credentials::start_geap_refresh_for_waiter(
                            manager, waiter, ctx,
                        );
                    })
                });
                if let Some(refresh_rx) = refresh_rx {
                    let _ = ctx.spawn(
                        async move { refresh_rx.with_timeout(GEAP_REFRESH_REQUEST_TIMEOUT).await },
                        move |me, result, ctx| {
                            // Cancelled or superseded while waiting — drop this attempt.
                            if me.current_request_id != Some(request_id) {
                                return;
                            }
                            // `RequestParams` snapshotted the credentials before
                            // the wait, so re-read just the GEAP credential and
                            // leave every other key alone.
                            //
                            // Unlike the Grok branch above, a mint failure, a
                            // timeout, or a dropped sender is never surfaced as a
                            // terminal error — the request goes out with the
                            // snapshot untouched, and it is the job of the server
                            // to respond with an error if the GEAP credentials are bad.
                            if matches!(result, Ok(Ok(GeapRefreshOutcome::Refreshed)))
                                && let Some(credentials) = ApiKeyManager::as_ref(ctx)
                                    .geap_credentials_for_request(&refresh_binding)
                            {
                                apply_geap_refresh_to_params(&mut me.params, Some(credentials));
                            }
                            Self::spawn_generate(
                                request_id,
                                me.params.clone(),
                                cancellation_rx,
                                ctx,
                            );
                        },
                    );
                    return;
                }
            }
        }

        Self::spawn_generate(request_id, params, cancellation_rx, ctx);
    }

    /// Emits a terminal, user-visible error for a failed request-time Grok token
    /// refresh instead of sending the request with an expired token. Mirrors the
    /// terminal-error emission in [`Self::handle_response_stream_result`].
    #[cfg(not(target_family = "wasm"))]
    fn surface_grok_refresh_failure(&mut self, request_id: Uuid, ctx: &mut ModelContext<Self>) {
        let error = Arc::new(AIApiError::GrokSubscriptionTokenRefreshFailed);
        self.error_event_emitted = true;
        self.report_request_failure(
            &error,
            NetworkStatus::as_ref(ctx).is_online(),
            self.recovery.attempts_used(),
        );
        ctx.emit(ResponseStreamEvent::ReceivedEvent(Consumable::new(Err(
            error,
        ))));
        self.on_response_stream_complete(request_id, ctx);
    }

    /// Spawns the actual multi-agent request send for `request_id`.
    fn spawn_generate(
        request_id: Uuid,
        params: api::RequestParams,
        cancellation_rx: oneshot::Receiver<()>,
        ctx: &mut ModelContext<Self>,
    ) {
        let server_api = ServerApiProvider::as_ref(ctx).get();
        let _ = ctx.spawn(
            async move { generate_multi_agent_output(server_api, params, cancellation_rx).await },
            move |me, stream, ctx| {
                me.handle_response_stream_result(request_id, stream, ctx);
            },
        );
    }

    /// Cancels the stream. The conversation_id is preserved in the emitted event for async handling.
    pub(super) fn cancel(
        &mut self,
        reason: CancellationReason,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        self.current_request_id = None;
        let Some(cancellation_tx) = self.cancellation_tx.take() else {
            return;
        };
        let _ = cancellation_tx.send(());
        ctx.emit(ResponseStreamEvent::AfterStreamFinished {
            cancellation: Some(StreamCancellation {
                reason,
                conversation_id,
            }),
        });
    }

    fn handle_response_stream_result(
        &mut self,
        request_id: Uuid,
        stream_result: Result<api::ResponseStream, ConvertToAPITypeError>,
        ctx: &mut ModelContext<Self>,
    ) {
        match stream_result {
            Ok(stream) => {
                ctx.spawn_stream_local(
                    stream,
                    move |me, event, ctx| {
                        me.handle_response_stream_event(request_id, event, ctx);
                    },
                    move |me, ctx| {
                        me.on_response_stream_complete(request_id, ctx);
                    },
                );
            }
            Err(e) => {
                if self.current_request_id.is_none_or(|id| id != request_id) {
                    return;
                }
                // A request-conversion failure is a deterministic client-side error and
                // no stream was ever created: retrying would fail identically, and
                // letting completion synthesize `UnexpectedEof` would misreport it as
                // a transient network failure. Surface the original error and finish
                // terminally. (HTTP send failures don't take this path — they arrive as
                // in-stream error events.)
                let error = Arc::new(AIApiError::Other(
                    anyhow::Error::new(e).context("Failed to send request to multi-agent API"),
                ));
                self.error_event_emitted = true;
                self.report_request_failure(
                    &error,
                    NetworkStatus::as_ref(ctx).is_online(),
                    self.recovery.attempts_used(),
                );
                ctx.emit(ResponseStreamEvent::ReceivedEvent(Consumable::new(Err(
                    error,
                ))));
                self.on_response_stream_complete(request_id, ctx);
            }
        }
    }

    fn handle_response_stream_event(
        &mut self,
        request_id: Uuid,
        event: api::Event,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.current_request_id.is_none_or(|id| id != request_id) {
            return;
        }
        self.time_to_latest_event = Local::now().signed_duration_since(self.start_time);

        match &event {
            Ok(response_event) => {
                if let Some(event_type) = &response_event.r#type {
                    match event_type {
                        warp_multi_agent_api::response_event::Type::Init(init_event) => {
                            // Capture server_output_id from StreamInit event
                            self.ai_identifiers.server_output_id =
                                Some(crate::ai::agent::ServerOutputId::new(
                                    init_event.request_id.clone(),
                                ));
                        }
                        warp_multi_agent_api::response_event::Type::ClientActions(_) => {
                            // Mark that we've received client actions
                            self.has_received_client_actions = true;
                        }
                        warp_multi_agent_api::response_event::Type::Finished(finished_event) => {
                            self.stream_finished_received = true;
                            // Emit retry success telemetry on successful completion
                            if matches!(
                                finished_event.reason,
                                Some(warp_multi_agent_api::response_event::stream_finished::Reason::Done(_)) | None
                            ) {
                                // Emit retry success telemetry if this was a successful completion after retries
                                if self.retries_sent > 0
                                    && let Some(original_error) = &self.original_error {
                                        send_telemetry_from_ctx!(
                                            crate::TelemetryEvent::AgentModeRequestRetrySucceeded {
                                                identifiers: self.ai_identifiers.clone(),
                                                retry_count: self.retries_sent,
                                                original_error: original_error.clone(),
                                            },
                                            ctx
                                        );
                                    }
                            }
                        }
                    }
                }
                ctx.emit(ResponseStreamEvent::ReceivedEvent(Consumable::new(event)));
            }
            Err(e) => {
                // Store original error if this is the first error
                if self.original_error.is_none() {
                    self.original_error = Some(format!("{e:?}"));
                }

                if matches!(self.begin_recovery(e, ctx), RecoveryOutcome::InFlight) {
                    // Don't emit the error event, we're recovering in-request.
                    return;
                }

                ctx.emit(ResponseStreamEvent::ReceivedEvent(Consumable::new(event)));
            }
        }
    }

    fn on_response_stream_complete(&mut self, request_id: Uuid, ctx: &mut ModelContext<Self>) {
        if self.current_request_id.is_none_or(|id| id != request_id) {
            return;
        }
        // A retry is parked waiting for a backoff or for connectivity; the request is
        // logically still active, so don't complete the stream for the failed attempt.
        if self.deferred_retry_pending {
            return;
        }

        // The server always sends a StreamFinished event before ending the response,
        // but a transport cut between chunks surfaces as a clean EOF. Synthesize the
        // failure and recover like any transient error.
        if !self.stream_finished_received && !self.error_event_emitted {
            log::warn!(
                "generate_multi_agent_output stream ended without emitting StreamFinished event."
            );
            let unexpected_eof = Arc::new(AIApiError::UnexpectedEof);
            if matches!(
                self.begin_recovery(&unexpected_eof, ctx),
                RecoveryOutcome::InFlight
            ) {
                return;
            }
            ctx.emit(ResponseStreamEvent::ReceivedEvent(Consumable::new(Err(
                unexpected_eof,
            ))));
        }

        ctx.emit(ResponseStreamEvent::AfterStreamFinished { cancellation: None });
        self.cancellation_tx = None;
    }

    /// Reports a non-retried request failure to crash reporting with classification
    /// tags.
    ///
    /// `recovery_attempt` is the attempt this failure sits at, counted the same way the
    /// recovery log line counts it: the attempt a scheduled resume is about to make, or the
    /// attempts already spent when the failure is terminal. Passing it in rather than
    /// deriving it here keeps the two surfaces from disagreeing by one for one failure.
    fn report_request_failure(
        &self,
        error: &Arc<AIApiError>,
        is_online: bool,
        recovery_attempt: usize,
    ) {
        #[cfg(feature = "crash_reporting")]
        sentry::with_scope(
            |scope| {
                scope.set_tag(
                    "has_received_client_actions",
                    self.has_received_client_actions,
                );
                scope.set_tag("error", format!("{error:?}"));
                scope.set_tag("is_recoverable", error.is_recoverable());
                scope.set_tag(
                    "will_attempt_resume",
                    self.should_resume_conversation_after_stream_finished(),
                );
                scope.set_tag("is_online", is_online);
                scope.set_tag("failed_request", self.failed_request_label());
            },
            || {
                report_error!(
                    error.as_ref(),
                    extra: {
                        "has_received_client_actions" => self.has_received_client_actions,
                        "is_recoverable" => error.is_recoverable(),
                        "will_attempt_resume" => self.should_resume_conversation_after_stream_finished(),
                        "is_online" => is_online,
                        "failed_request" => self.failed_request_label(),
                        "recovery_attempt" => recovery_attempt,
                        "max_recovery_attempts" => MAX_RECOVERY_ATTEMPTS,
                        "error_debug" => %format!("{error:?}"),
                    }
                );
            },
        );
        #[cfg(not(feature = "crash_reporting"))]
        {
            report_error!(
                error.as_ref(),
                extra: {
                    "has_received_client_actions" => self.has_received_client_actions,
                    "is_recoverable" => error.is_recoverable(),
                    "will_attempt_resume" => self.should_resume_conversation_after_stream_finished(),
                    "is_online" => is_online,
                    "failed_request" => self.failed_request_label(),
                    "recovery_attempt" => recovery_attempt,
                    "max_recovery_attempts" => MAX_RECOVERY_ATTEMPTS,
                    "error_debug" => %format!("{error:?}"),
                }
            );
        }
    }

    /// Parks a retry until connectivity returns; cancellation invalidates the parked
    /// retry through `current_request_id`.
    fn defer_retry_until_online(&mut self, ctx: &mut ModelContext<Self>) {
        self.deferred_retry_pending = true;
        ctx.emit(ResponseStreamEvent::WaitingForNetwork { waiting: true });
        let request_id_at_defer = self.current_request_id;
        let wait_for_online = NetworkStatus::as_ref(ctx).wait_until_online();
        let _ = ctx.spawn(wait_for_online, move |me, _, ctx| {
            // Cancelled or superseded while waiting — drop the parked retry.
            if request_id_at_defer.is_none() || me.current_request_id != request_id_at_defer {
                return;
            }
            ctx.emit(ResponseStreamEvent::WaitingForNetwork { waiting: false });
            me.retry(ctx);
        });
    }

    /// Parks a retry behind the shared recovery backoff, so a re-send doesn't land in the
    /// same window of failures that killed the previous attempt.
    ///
    /// No `WaitingForNetwork` event is emitted: the failure hasn't been surfaced, the
    /// conversation is still in progress, and the wait is bounded to a couple of seconds.
    fn defer_retry_after_backoff(&mut self, delay: Duration, ctx: &mut ModelContext<Self>) {
        self.deferred_retry_pending = true;
        let request_id_at_defer = self.current_request_id;
        let _ = ctx.spawn(
            async move { Timer::after(delay).await },
            move |me, _, ctx| {
                // Cancelled or superseded while backing off — drop the parked retry.
                if request_id_at_defer.is_none() || me.current_request_id != request_id_at_defer {
                    return;
                }
                me.retry(ctx);
            },
        );
    }
}

/// Applies the result of a request-time GEAP mint to the request snapshot.
///
/// A successful mint swaps in the fresh credential.
#[cfg(not(target_family = "wasm"))]
fn apply_geap_refresh_to_params(
    params: &mut api::RequestParams,
    fresh_credentials: Option<maa_api::request::settings::api_keys::GoogleCloudCredentials>,
) {
    if let Some(credentials) = fresh_credentials
        && let Some(keys) = params.api_keys.as_mut()
    {
        keys.google_cloud_credentials = Some(credentials);
    }
}

#[derive(Debug)]
pub struct Consumable<T> {
    value: Rc<RefCell<Option<T>>>,
}

impl<T> Consumable<T> {
    fn new(value: T) -> Self {
        Consumable {
            value: Rc::new(RefCell::new(Some(value))),
        }
    }

    pub(super) fn consume(&self) -> Option<T> {
        self.value.borrow_mut().take()
    }
}

impl<T> Clone for Consumable<T> {
    fn clone(&self) -> Self {
        Consumable {
            value: Rc::clone(&self.value),
        }
    }
}

/// Cancellation context preserved for async event handling.
/// Includes conversation_id because truncation can remove exchange mappings before the event is processed.
#[derive(Debug, Clone)]
pub struct StreamCancellation {
    pub reason: CancellationReason,
    pub conversation_id: AIConversationId,
}

#[derive(Debug, Clone)]
pub enum ResponseStreamEvent {
    ReceivedEvent(Consumable<api::Event>),
    /// A retry is parked until connectivity returns (`waiting: true`) or has just
    /// fired (`waiting: false`). The controller mirrors this on the conversation
    /// status (`TransientError` ↔ `InProgress`).
    ///
    /// Only emitted from `defer_retry_until_online`, i.e. always after a recoverable
    /// request failure while offline — never speculatively before an attempt. Consumers
    /// can therefore treat `waiting: true` as a transient-error (reconnecting) state.
    WaitingForNetwork {
        waiting: bool,
    },
    AfterStreamFinished {
        /// Some for cancellation (with context), None for natural completion (uses dynamic lookup).
        cancellation: Option<StreamCancellation>,
    },
}

impl Entity for ResponseStream {
    type Event = ResponseStreamEvent;
}

#[cfg(test)]
#[path = "response_stream_tests.rs"]
mod tests;
