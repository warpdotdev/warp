use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use anyhow::anyhow;
use chrono::{DateTime, Local, TimeDelta};
use futures::channel::oneshot;
use uuid::Uuid;
use warp_multi_agent_api::response_event;
use warpui::{Entity, ModelContext, SingletonEntity};

use crate::ai::agent::api::{self, generate_multi_agent_output, ConvertToAPITypeError};
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::{AIIdentifiers, CancellationReason};
use crate::network::NetworkStatus;
use crate::server::server_api::{AIApiError, ServerApiProvider};
use crate::{report_error, send_telemetry_from_ctx};

/// Maximum number of times a single MAA request is re-sent before the failure is
/// surfaced.
const MAX_RETRIES: usize = 3;

/// Maximum time to wait for a request-time Grok OAuth token refresh before
/// sending with the currently stored token. Bounded so a hung refresh can't
/// stall the request.
#[cfg(not(target_family = "wasm"))]
const GROK_REFRESH_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// What to do about a failed or truncated MAA response attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecoveryAction {
    /// Re-send the same request immediately.
    RetryNow,
    /// Re-send the same request once connectivity returns.
    RetryWhenOnline,
    /// Resume the conversation with a fresh request after the stream completes.
    Resume,
    /// Surface the error; the conversation ends in error.
    Fail,
}

/// Decides how to recover from a failed response-stream attempt.
///
/// Before any client actions have been received, the request can be re-sent verbatim
/// (immediately, or once connectivity returns). After actions have streamed,
/// re-sending is unsafe, so recovery uses a fresh `ResumeConversation` request.
fn recovery_action(
    has_received_client_actions: bool,
    is_recoverable: bool,
    has_retry_budget: bool,
    can_attempt_resume_on_error: bool,
    is_online: bool,
) -> RecoveryAction {
    if !has_received_client_actions && is_recoverable && has_retry_budget {
        if is_online {
            RecoveryAction::RetryNow
        } else {
            RecoveryAction::RetryWhenOnline
        }
    } else if has_received_client_actions && is_recoverable && can_attempt_resume_on_error {
        RecoveryAction::Resume
    } else {
        RecoveryAction::Fail
    }
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
/// received yet, ensuring we don't retry after the AI has started executing actions.
pub struct ResponseStream {
    id: ResponseStreamId,
    params: api::RequestParams,
    retry_count: usize,
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

    /// Whether this request can attempt to resume the conversation on error.
    /// This is true for all requests except those that are themselves the result of a resume
    /// triggered by a previous error.
    can_attempt_resume_on_error: bool,

    /// Whether we should attempt to resume the conversation after the stream finishes.
    ///
    /// This is set when a transient network/server failure occurs after client actions
    /// have been received (so an in-request retry is unsafe) and
    /// `can_attempt_resume_on_error` is true.
    should_resume_conversation_after_stream_finished: bool,

    /// Whether a `StreamFinished` event was received for the current request. A
    /// stream that completes without one was truncated in transit.
    stream_finished_received: bool,

    /// Whether a terminal error event has already been emitted for the current
    /// request, so stream completion doesn't synthesize a second failure for it.
    error_event_emitted: bool,

    /// Whether a retry is parked waiting for connectivity. While set, completion of
    /// the failed attempt's underlying stream is ignored.
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
            retry_count: 0,
            start_time: Local::now(),
            time_to_latest_event: TimeDelta::seconds(0),
            cancellation_tx: Some(cancellation_tx),
            original_error: None,
            has_received_client_actions: false,
            ai_identifiers: AIIdentifiers::default(),
            can_attempt_resume_on_error: false,
            should_resume_conversation_after_stream_finished: false,
            stream_finished_received: false,
            error_event_emitted: false,
            deferred_retry_pending: false,
            current_request_id: Some(Uuid::new_v4()),
        }
    }

    pub fn new(
        params: api::RequestParams,
        ai_identifiers: AIIdentifiers,
        can_attempt_resume_on_error: bool,
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
            retry_count: 0,
            original_error: None,
            has_received_client_actions: false,
            ai_identifiers,
            can_attempt_resume_on_error,
            should_resume_conversation_after_stream_finished: false,
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
        self.should_resume_conversation_after_stream_finished
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
        self.retry_count += 1;
        // Reset per-attempt state for the new attempt.
        self.has_received_client_actions = false;
        self.stream_finished_received = false;
        self.error_event_emitted = false;
        self.deferred_retry_pending = false;

        let (cancellation_tx, cancellation_rx) = oneshot::channel();
        if let Some(old_cancellation_tx) = self.cancellation_tx.take() {
            let _ = old_cancellation_tx.send(());
        }
        self.cancellation_tx = Some(cancellation_tx);

        let request_id = Uuid::new_v4();
        self.current_request_id = Some(request_id);
        Self::spawn_request(request_id, self.params.clone(), cancellation_rx, ctx);
    }

    /// Sends the request for `request_id`. When the request's model is served by
    /// the connected Grok subscription and that subscription's OAuth token is
    /// already past hard expiry, this first blocks on a single shared refresh
    /// (owned by `ApiKeyManager`, so only one runs at a time) before sending.
    /// The wait is bounded by [`GROK_REFRESH_REQUEST_TIMEOUT`]. If the refresh
    /// fails or times out, the request is NOT sent with the dead token; a
    /// terminal, user-visible error is surfaced instead. Requests that don't use
    /// the Grok subscription (and tokens that are still valid) are sent directly.
    fn spawn_request(
        request_id: Uuid,
        params: api::RequestParams,
        cancellation_rx: oneshot::Receiver<()>,
        ctx: &mut ModelContext<Self>,
    ) {
        // The Grok subscription and its OAuth refresh are native-only.
        #[cfg(not(target_family = "wasm"))]
        {
            use ::ai::api_keys::{ApiKeyManager, GrokRefreshOutcome};
            use warpui::r#async::FutureExt as _;

            use crate::ai::llms::{LLMPreferences, LLMProvider};
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
                                {
                                    if let Some(keys) = me.params.api_keys.as_mut() {
                                        keys.grok_oauth_access_token = access_token;
                                    }
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
        self.report_request_failure(&error, NetworkStatus::as_ref(ctx).is_online());
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
                report_error!(
                    anyhow::anyhow!("{e:?}").context("Failed to send request to multi-agent API")
                );
                if self.current_request_id.is_none_or(|id| id != request_id) {
                    return;
                }
                // A request-conversion failure is a deterministic client-side error and
                // no stream was ever created: retrying would fail identically, and
                // letting completion synthesize `UnexpectedEof` would misreport it as
                // a transient network failure. Surface the original error and finish
                // terminally. (HTTP send failures don't take this path — they arrive as
                // in-stream error events.)
                let error = Arc::new(AIApiError::Other(anyhow!(e)));
                self.error_event_emitted = true;
                self.report_request_failure(&error, NetworkStatus::as_ref(ctx).is_online());
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
                                if self.retry_count > 0 {
                                    if let Some(original_error) = &self.original_error {
                                        send_telemetry_from_ctx!(
                                            crate::TelemetryEvent::AgentModeRequestRetrySucceeded {
                                                identifiers: self.ai_identifiers.clone(),
                                                retry_count: self.retry_count,
                                                original_error: original_error.clone(),
                                            },
                                            ctx
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
                ctx.emit(ResponseStreamEvent::ReceivedEvent(Consumable::new(event)));
            }
            Err(e) => {
                // Store original error if this is the first error
                if self.retry_count == 0 {
                    self.original_error = Some(format!("{e:?}"));
                }

                let is_online = NetworkStatus::as_ref(ctx).is_online();
                match recovery_action(
                    self.has_received_client_actions,
                    e.is_recoverable(),
                    self.retry_count < MAX_RETRIES,
                    self.can_attempt_resume_on_error,
                    is_online,
                ) {
                    RecoveryAction::RetryNow => {
                        log::warn!(
                            "MultiAgent request failed, retrying (attempt {}/{}) - Error: {e:?}",
                            self.retry_count + 1,
                            MAX_RETRIES
                        );
                        // Only emit error telemetry here if we're retrying.
                        // Final errors that aren't being retried are emitted elsewhere.
                        self.emit_retryable_agent_mode_error_telemetry(format!("{e:?}"), ctx);
                        self.retry(ctx);
                        // Don't emit the error event, we're retrying
                        return;
                    }
                    RecoveryAction::RetryWhenOnline => {
                        log::warn!(
                            "MultiAgent request failed while offline; retrying (attempt {}/{}) once connectivity returns - Error: {e:?}",
                            self.retry_count + 1,
                            MAX_RETRIES
                        );
                        self.emit_retryable_agent_mode_error_telemetry(format!("{e:?}"), ctx);
                        self.defer_retry_until_online(ctx);
                        return;
                    }
                    RecoveryAction::Resume => {
                        // Recoverable failure after client actions: we'll resume the
                        // conversation once the stream finishes rather than surface the
                        // error, so the UI suppresses the banner. Log it so the
                        // auto-recovery isn't completely silent.
                        log::warn!(
                            "MultiAgent request failed after client actions; resuming conversation after stream finishes - Error: {e:?}"
                        );
                        // The resume spawn itself waits for connectivity.
                        self.should_resume_conversation_after_stream_finished = true;
                    }
                    RecoveryAction::Fail => {}
                }
                self.error_event_emitted = true;

                self.report_request_failure(e, is_online);

                ctx.emit(ResponseStreamEvent::ReceivedEvent(Consumable::new(event)));
            }
        }
    }

    fn on_response_stream_complete(&mut self, request_id: Uuid, ctx: &mut ModelContext<Self>) {
        if self.current_request_id.is_none_or(|id| id != request_id) {
            return;
        }
        // A retry is parked waiting for connectivity; the request is logically still
        // active, so don't complete the stream for the failed attempt.
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
            let is_online = NetworkStatus::as_ref(ctx).is_online();
            match recovery_action(
                self.has_received_client_actions,
                unexpected_eof.is_recoverable(),
                self.retry_count < MAX_RETRIES,
                self.can_attempt_resume_on_error,
                is_online,
            ) {
                RecoveryAction::RetryNow => {
                    log::warn!(
                        "MultiAgent request failed, retrying (attempt {}/{}) - Error: {unexpected_eof:?}",
                        self.retry_count + 1,
                        MAX_RETRIES
                    );
                    self.emit_retryable_agent_mode_error_telemetry(
                        format!("{unexpected_eof:?}"),
                        ctx,
                    );
                    self.retry(ctx);
                    return;
                }
                RecoveryAction::RetryWhenOnline => {
                    log::warn!(
                        "MultiAgent request failed while offline; retrying (attempt {}/{}) once connectivity returns - Error: {unexpected_eof:?}",
                        self.retry_count + 1,
                        MAX_RETRIES
                    );
                    self.emit_retryable_agent_mode_error_telemetry(
                        format!("{unexpected_eof:?}"),
                        ctx,
                    );
                    self.defer_retry_until_online(ctx);
                    return;
                }
                RecoveryAction::Resume => {
                    // Recoverable truncation after client actions: we'll resume the
                    // conversation once the stream finishes rather than surface the
                    // error, so the UI suppresses the banner. Log it so the
                    // auto-recovery isn't completely silent.
                    log::warn!(
                        "MultiAgent request truncated after client actions; resuming conversation after stream finishes - Error: {unexpected_eof:?}"
                    );
                    self.should_resume_conversation_after_stream_finished = true;
                    self.error_event_emitted = true;
                    self.report_request_failure(&unexpected_eof, is_online);
                    ctx.emit(ResponseStreamEvent::ReceivedEvent(Consumable::new(Err(
                        unexpected_eof,
                    ))));
                }
                RecoveryAction::Fail => {
                    self.error_event_emitted = true;
                    self.report_request_failure(&unexpected_eof, is_online);
                    ctx.emit(ResponseStreamEvent::ReceivedEvent(Consumable::new(Err(
                        unexpected_eof,
                    ))));
                }
            }
        }

        ctx.emit(ResponseStreamEvent::AfterStreamFinished { cancellation: None });
        self.cancellation_tx = None;
    }

    /// Reports a non-retried request failure to crash reporting with classification
    /// tags.
    #[cfg_attr(not(feature = "crash_reporting"), expect(unused_variables))]
    fn report_request_failure(&self, error: &Arc<AIApiError>, is_online: bool) {
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
                    self.should_resume_conversation_after_stream_finished,
                );
                scope.set_tag("is_online", is_online);
                scope.set_tag("retry_count", self.retry_count);
            },
            || {
                report_error!(anyhow!(error.clone()).context(format!(
                    "MultiAgent request failed after {} retries",
                    self.retry_count
                )));
            },
        );
        #[cfg(not(feature = "crash_reporting"))]
        {
            report_error!(anyhow!(error.clone()).context(format!(
                "MultiAgent request failed after {} retries",
                self.retry_count
            )));
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
