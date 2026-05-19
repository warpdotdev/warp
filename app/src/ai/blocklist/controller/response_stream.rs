use std::{cell::RefCell, rc::Rc};

use anyhow::anyhow;
use chrono::{DateTime, Local, TimeDelta};
use futures::channel::oneshot;
use uuid::Uuid;
use warp_multi_agent_api::response_event;
use warpui::{Entity, ModelContext, SingletonEntity};

use crate::{
    ai::agent::{
        api::{self, generate_multi_agent_output, ConvertToAPITypeError},
        conversation::AIConversationId,
        AIIdentifiers, CancellationReason,
    },
    network::NetworkStatus,
    report_error, send_telemetry_from_ctx,
    server::server_api::ServerApiProvider,
};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ResponseStreamId(String);

impl ResponseStreamId {
    pub fn for_shared_session(init_event: &response_event::StreamInit) -> Self {
        // Make the stream ID unique per viewing by appending a local UUID
        // This prevents collisions when replaying the same conversation multiple times
        // (either on close-and-reopen or when viewing the same shared session from multiple terminals)
        Self(format!("{}-{}", init_event.request_id, Uuid::new_v4()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
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
    /// This is set when we receive a retryable error after client actions have been received
    /// and `can_attempt_resume_on_error` is true.
    should_resume_conversation_after_stream_finished: bool,

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
    pub fn new(
        params: api::RequestParams,
        ai_identifiers: AIIdentifiers,
        can_attempt_resume_on_error: bool,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let server_api = ServerApiProvider::as_ref(ctx).get();
        let (cancellation_tx, cancellation_rx) = oneshot::channel();
        let start_time = Local::now();

        let request_id = Uuid::new_v4();
        let params_clone = params.clone();
        let _ =
            ctx.spawn(
                async move {
                    generate_multi_agent_output(server_api, params_clone, cancellation_rx).await
                },
                move |me, stream, ctx| {
                    me.handle_response_stream_result(request_id, stream, ctx);
                },
            );
        Self {
            id: ResponseStreamId(Uuid::new_v4().to_string()),
            params: params.clone(),
            start_time,
            time_to_latest_event: TimeDelta::seconds(0),
            cancellation_tx: Some(cancellation_tx),
            retry_count: 0,
            original_error: None,
            has_received_client_actions: false,
            ai_identifiers,
            can_attempt_resume_on_error,
            should_resume_conversation_after_stream_finished: false,
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
        self.has_received_client_actions = false; // Reset for the new attempt

        let (cancellation_tx, cancellation_rx) = oneshot::channel();
        if let Some(old_cancellation_tx) = self.cancellation_tx.take() {
            let _ = old_cancellation_tx.send(());
        }
        self.cancellation_tx = Some(cancellation_tx);

        let request_id = Uuid::new_v4();
        self.current_request_id = Some(request_id);
        let params = self.params.clone();
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
                log::error!("Failed to send request to multi-agent API: {e:?}");
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

                // Only retry if:
                // 1. We haven't received any client actions yet (this is the first event or only init events)
                // 2. The error is retryable
                // 3. We haven't exceeded max retries
                // 4. We're online
                const MAX_RETRIES: usize = 3;
                let network_status = NetworkStatus::as_ref(ctx);
                let is_online = network_status.is_online();
                let is_retryable = e.is_retryable();

                let should_retry = !self.has_received_client_actions
                    && is_retryable
                    && self.retry_count < MAX_RETRIES
                    && is_online;

                if should_retry {
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
                    // TODO: emit a separate event if controller needs to know about failures that are being retried
                    return;
                }

                // If we can't retry (because client actions were received) but the error is
                // retryable and we're allowed to attempt a resume, signal that the controller
                // should resume the conversation after the stream completes.
                let should_attempt_resume = self.has_received_client_actions
                    && is_retryable
                    && self.can_attempt_resume_on_error;
                if should_attempt_resume {
                    self.should_resume_conversation_after_stream_finished = true;
                }

                #[cfg(feature = "crash_reporting")]
                sentry::with_scope(
                    |scope| {
                        scope.set_tag(
                            "has_received_client_actions",
                            self.has_received_client_actions,
                        );
                        scope.set_tag("error", format!("{e:?}"));
                        scope.set_tag("is_retryable", e.is_retryable());
                        scope.set_tag("is_online", is_online);
                        scope.set_tag("retry_count", self.retry_count);
                    },
                    || {
                        report_error!(anyhow!(e.clone()).context(format!(
                            "MultiAgent request failed after {} retries",
                            self.retry_count
                        )));
                    },
                );
                #[cfg(not(feature = "crash_reporting"))]
                {
                    report_error!(anyhow!(e.clone()).context(format!(
                        "MultiAgent request failed after {} retries",
                        self.retry_count
                    )));
                }

                ctx.emit(ResponseStreamEvent::ReceivedEvent(Consumable::new(event)));
            }
        }
    }

    fn on_response_stream_complete(&mut self, request_id: Uuid, ctx: &mut ModelContext<Self>) {
        if self.current_request_id.is_none_or(|id| id != request_id) {
            return;
        }
        ctx.emit(ResponseStreamEvent::AfterStreamFinished { cancellation: None });
        self.cancellation_tx = None;
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
    AfterStreamFinished {
        /// Some for cancellation (with context), None for natural completion (uses dynamic lookup).
        cancellation: Option<StreamCancellation>,
    },
}

impl Entity for ResponseStream {
    type Event = ResponseStreamEvent;
}
