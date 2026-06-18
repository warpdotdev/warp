use ai::skills::SkillPathOrigin;
use warp_multi_agent_api::response_event::stream_finished;
use warp_multi_agent_api::{response_event, ResponseEvent};
use warpui::{AppContext, Entity, ModelContext, ModelHandle, SingletonEntity};

use super::controller::response_stream::{ResponseStream, ResponseStreamEvent, ResponseStreamId};
use super::history_model::{AgentSessionOwnerId, BlocklistAIHistoryModel};
use super::RequestInput;
use crate::ai::agent::conversation::{AIConversationId, ConversationStatus};
use crate::ai::agent::{
    api, AIAgentAction, AIAgentOutputStatus, AIIdentifiers, FinishedAIAgentOutput,
    RenderableAIError, RequestCost, TransientNetworkErrorKind,
};
use crate::ai::llms::LLMPreferences;
use crate::ai::AIRequestUsageModel;
use crate::features::FeatureFlag;
use crate::network::NetworkStatus;
use crate::server::server_api::AIApiError;
use crate::workspaces::update_manager::TeamUpdateManager;

/// Surface-specific hooks for the shared agent send/stream engine.
pub(crate) trait AgentConversationEngineDelegate: Entity + Sized {
    /// Returns the skill path origin to use while folding client actions.
    fn skill_path_origin(&self, ctx: &AppContext) -> SkillPathOrigin;

    /// Forwards a raw response event to shared-session viewers, if this surface has any.
    fn forward_response_event_to_shared_session(
        &mut self,
        _event: &ResponseEvent,
        _conversation_id: AIConversationId,
        _ctx: &mut ModelContext<Self>,
    ) {
    }

    /// Sends a cancellation notification to shared-session viewers, if this surface has any.
    fn send_cancellation_to_shared_session_viewers(&mut self, _ctx: &mut ModelContext<Self>) {}

    /// Queues executable actions produced by a completed stream.
    fn queue_client_actions(
        &mut self,
        _actions: Vec<AIAgentAction>,
        _conversation_id: AIConversationId,
        _ctx: &mut ModelContext<Self>,
    ) {
    }

    /// Handles surface-local cancellation side effects.
    fn response_stream_cancelled(
        &mut self,
        _conversation_id: AIConversationId,
        _was_passive_request: bool,
        _ctx: &mut ModelContext<Self>,
    ) {
    }

    /// Handles surface-local cleanup after a naturally completed stream.
    fn response_stream_completed(
        &mut self,
        _stream_id: &ResponseStreamId,
        _conversation_id: AIConversationId,
        _ctx: &mut ModelContext<Self>,
    ) {
    }

    /// Schedules a follow-up resume request after a recoverable post-action stream error.
    fn schedule_auto_resume_after_error(
        &mut self,
        _conversation_id: AIConversationId,
        _ctx: &mut ModelContext<Self>,
    ) {
    }

    /// Returns whether available model metadata should refresh after this stream finishes.
    fn take_should_refresh_available_llms_on_stream_finish(&mut self) -> bool {
        false
    }

    /// Emits any surface-local free-tier refresh event.
    fn free_tier_limit_check_triggered(&mut self, _ctx: &mut ModelContext<Self>) {}

    /// Emits any surface-local stream-finished event.
    fn finished_receiving_output(
        &mut self,
        _stream_id: ResponseStreamId,
        _conversation_id: AIConversationId,
        _ctx: &mut ModelContext<Self>,
    ) {
    }

    /// Refreshes any surface-local AI overage state.
    fn maybe_refresh_ai_overages(&mut self, _ctx: &mut ModelContext<Self>) {}
}

/// Shared engine for sending agent requests and folding streamed responses into history.
pub(crate) struct AgentConversationEngine;

impl AgentConversationEngine {
    /// Creates and subscribes to a response stream, then records the new request in history.
    pub(crate) fn send_request<E>(
        owner_id: AgentSessionOwnerId,
        request_input: RequestInput,
        request_params: api::RequestParams,
        conversation_data: api::ConversationData,
        can_attempt_resume_on_error: bool,
        ctx: &mut ModelContext<E>,
    ) -> (ModelHandle<ResponseStream>, ResponseStreamId)
    where
        E: AgentConversationEngineDelegate,
    {
        let response_stream = ctx.add_model(|ctx| {
            let ai_identifiers = AIIdentifiers {
                server_output_id: None,
                server_conversation_id: conversation_data
                    .server_conversation_token
                    .clone()
                    .map(Into::into),
                client_conversation_id: Some(conversation_data.id),
                client_exchange_id: None,
                model_id: Some(request_params.model.clone()),
            };
            ResponseStream::new(
                request_params.clone(),
                ai_identifiers,
                can_attempt_resume_on_error,
                ctx,
            )
        });
        Self::register_request_with_stream(
            owner_id,
            request_input,
            conversation_data,
            response_stream,
            ctx,
        )
    }

    /// Sends a request using a no-network response stream model for tests.
    #[cfg(test)]
    pub(crate) fn send_request_for_test<E>(
        owner_id: AgentSessionOwnerId,
        request_input: RequestInput,
        request_params: api::RequestParams,
        conversation_data: api::ConversationData,
        ctx: &mut ModelContext<E>,
    ) -> (ModelHandle<ResponseStream>, ResponseStreamId)
    where
        E: AgentConversationEngineDelegate,
    {
        let response_stream = ctx.add_model(|_| {
            let ai_identifiers = AIIdentifiers {
                server_output_id: None,
                server_conversation_id: conversation_data
                    .server_conversation_token
                    .clone()
                    .map(Into::into),
                client_conversation_id: Some(conversation_data.id),
                client_exchange_id: None,
                model_id: Some(request_params.model.clone()),
            };
            ResponseStream::new_for_test(request_params, ai_identifiers)
        });
        Self::register_request_with_stream(
            owner_id,
            request_input,
            conversation_data,
            response_stream,
            ctx,
        )
    }

    /// Registers a response stream and records the request in history.
    fn register_request_with_stream<E>(
        owner_id: AgentSessionOwnerId,
        request_input: RequestInput,
        conversation_data: api::ConversationData,
        response_stream: ModelHandle<ResponseStream>,
        ctx: &mut ModelContext<E>,
    ) -> (ModelHandle<ResponseStream>, ResponseStreamId)
    where
        E: AgentConversationEngineDelegate,
    {
        let response_stream_id = response_stream.as_ref(ctx).id().clone();
        let response_stream_clone = response_stream.clone();
        let input_contains_user_query = request_input
            .all_inputs()
            .any(|input| input.is_user_query());
        ctx.subscribe_to_model(&response_stream, move |me, event, ctx| {
            Self::handle_response_stream_event(
                me,
                owner_id,
                input_contains_user_query,
                event,
                &response_stream_clone,
                ctx,
            );
        });

        BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, ctx| match history_model
            .update_conversation_for_new_request_input(
                request_input,
                response_stream_id.clone(),
                owner_id.entity_id(),
                ctx,
            ) {
            Ok(_) => {
                history_model.update_conversation_status(
                    owner_id.entity_id(),
                    conversation_data.id,
                    ConversationStatus::InProgress,
                    ctx,
                );
            }
            Err(e) => {
                log::warn!("Failed to push new exchange to AI conversation: {e:?}");
            }
        });

        (response_stream, response_stream_id)
    }

    /// Folds a response-stream model event into history through the delegate hooks.
    fn handle_response_stream_event<E>(
        delegate: &mut E,
        owner_id: AgentSessionOwnerId,
        did_input_contain_user_query: bool,
        event: &ResponseStreamEvent,
        response_stream: &ModelHandle<ResponseStream>,
        ctx: &mut ModelContext<E>,
    ) where
        E: AgentConversationEngineDelegate,
    {
        let stream_id = response_stream.as_ref(ctx).id().clone();

        match event {
            ResponseStreamEvent::ReceivedEvent(event) => {
                let Some(conversation_id) = BlocklistAIHistoryModel::as_ref(ctx)
                    .conversation_for_response_stream(&stream_id)
                else {
                    log::warn!("Could not find conversation for response stream: {stream_id:?}");
                    return;
                };
                let Some(event) = event.consume() else {
                    debug_assert!(
                        false,
                        "This model should only have a single subscriber that takes ownership over the event."
                    );
                    return;
                };
                let recovery_pending = response_stream
                    .as_ref(ctx)
                    .should_resume_conversation_after_stream_finished();
                Self::fold_received_event(
                    delegate,
                    owner_id,
                    did_input_contain_user_query,
                    &stream_id,
                    conversation_id,
                    event,
                    recovery_pending,
                    ctx,
                );
            }
            ResponseStreamEvent::WaitingForNetwork { waiting } => {
                let Some(conversation_id) = BlocklistAIHistoryModel::as_ref(ctx)
                    .conversation_for_response_stream(&stream_id)
                else {
                    log::warn!("Could not find conversation for response stream: {stream_id:?}");
                    return;
                };
                let status = if *waiting {
                    ConversationStatus::TransientError
                } else {
                    ConversationStatus::InProgress
                };
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, ctx| {
                    history_model.update_conversation_status(
                        owner_id.entity_id(),
                        conversation_id,
                        status,
                        ctx,
                    );
                });
            }
            ResponseStreamEvent::AfterStreamFinished { cancellation } => {
                Self::fold_after_stream_finished(
                    delegate,
                    owner_id,
                    &stream_id,
                    cancellation.as_ref(),
                    response_stream,
                    ctx,
                );
            }
        }
    }

    /// Test hook for exercising response-event folding without spawning a live response stream.
    #[cfg(test)]
    pub(crate) fn fold_received_event_for_test<E>(
        delegate: &mut E,
        owner_id: AgentSessionOwnerId,
        did_input_contain_user_query: bool,
        stream_id: &ResponseStreamId,
        conversation_id: AIConversationId,
        event: api::Event,
        recovery_pending: bool,
        ctx: &mut ModelContext<E>,
    ) where
        E: AgentConversationEngineDelegate,
    {
        Self::fold_received_event(
            delegate,
            owner_id,
            did_input_contain_user_query,
            stream_id,
            conversation_id,
            event,
            recovery_pending,
            ctx,
        );
    }

    /// Test hook for exercising stream finalization without waiting for a real stream.
    #[cfg(test)]
    pub(crate) fn fold_after_stream_finished_for_test<E>(
        delegate: &mut E,
        owner_id: AgentSessionOwnerId,
        stream_id: &ResponseStreamId,
        response_stream: &ModelHandle<ResponseStream>,
        ctx: &mut ModelContext<E>,
    ) where
        E: AgentConversationEngineDelegate,
    {
        Self::fold_after_stream_finished(delegate, owner_id, stream_id, None, response_stream, ctx);
    }

    /// Folds a received API event or API error into history.
    fn fold_received_event<E>(
        delegate: &mut E,
        owner_id: AgentSessionOwnerId,
        did_input_contain_user_query: bool,
        stream_id: &ResponseStreamId,
        conversation_id: AIConversationId,
        event: api::Event,
        recovery_pending: bool,
        ctx: &mut ModelContext<E>,
    ) where
        E: AgentConversationEngineDelegate,
    {
        let history_model = BlocklistAIHistoryModel::handle(ctx);
        match event {
            Ok(event) => {
                delegate.forward_response_event_to_shared_session(&event, conversation_id, ctx);
                let Some(event) = event.r#type else {
                    return;
                };
                match event {
                    response_event::Type::Init(init_event) => {
                        history_model.update(ctx, |history_model, ctx| {
                            history_model.initialize_output_for_response_stream(
                                stream_id,
                                conversation_id,
                                owner_id.entity_id(),
                                init_event,
                                ctx,
                            );

                            if let Some(conversation) =
                                history_model.conversation_mut(&conversation_id)
                            {
                                conversation.clear_forked_from_server_conversation_token();
                            }
                        });
                    }
                    response_event::Type::Finished(finished_event) => {
                        let should_refresh_model_config =
                            finished_event.should_refresh_model_config;
                        Self::fold_response_stream_finished(
                            owner_id,
                            stream_id,
                            finished_event,
                            conversation_id,
                            did_input_contain_user_query,
                            ctx,
                        );
                        if should_refresh_model_config {
                            delegate.free_tier_limit_check_triggered(ctx);
                        }
                    }
                    response_event::Type::ClientActions(actions) => {
                        let client_actions = actions.actions;
                        let skill_path_origin = delegate.skill_path_origin(ctx);
                        let apply_result = history_model.update(ctx, |history_model, ctx| {
                            history_model.apply_client_actions(
                                stream_id,
                                client_actions,
                                conversation_id,
                                owner_id.entity_id(),
                                &skill_path_origin,
                                ctx,
                            )
                        });
                        if let Err(e) = apply_result {
                            log::error!("Failed to apply client actions to conversation: {e:?}");
                        }
                    }
                }
            }
            Err(e) => {
                if matches!(e.as_ref(), AIApiError::QuotaLimit { .. }) {
                    TeamUpdateManager::handle(ctx).update(ctx, |team_update_manager, ctx| {
                        std::mem::drop(team_update_manager.refresh_workspace_metadata(ctx));
                    });
                    AIRequestUsageModel::handle(ctx).update(ctx, |model, ctx| {
                        model.enable_buy_credits_banner(ctx);
                    });
                }

                let mut renderable_error: RenderableAIError = (&e).into();
                if let RenderableAIError::Other {
                    will_attempt_resume,
                    waiting_for_network,
                    ..
                }
                | RenderableAIError::TransientNetworkError {
                    will_attempt_resume,
                    waiting_for_network,
                    ..
                } = &mut renderable_error
                {
                    *will_attempt_resume |= recovery_pending;
                    if recovery_pending {
                        let network_status = NetworkStatus::as_ref(ctx);
                        *waiting_for_network = !network_status.is_online();
                    }
                }

                history_model.update(ctx, |history_model, ctx| {
                    history_model.mark_response_stream_completed_with_error(
                        renderable_error,
                        recovery_pending,
                        stream_id,
                        conversation_id,
                        owner_id.entity_id(),
                        ctx,
                    );
                });
            }
        }
    }

    /// Finalizes stream state after natural completion or cancellation.
    fn fold_after_stream_finished<E>(
        delegate: &mut E,
        owner_id: AgentSessionOwnerId,
        stream_id: &ResponseStreamId,
        cancellation: Option<&super::controller::response_stream::StreamCancellation>,
        response_stream: &ModelHandle<ResponseStream>,
        ctx: &mut ModelContext<E>,
    ) where
        E: AgentConversationEngineDelegate,
    {
        let conversation_id = match cancellation {
            Some(stream_cancellation) => stream_cancellation.conversation_id,
            None => {
                let Some(id) = BlocklistAIHistoryModel::as_ref(ctx)
                    .conversation_for_response_stream(stream_id)
                else {
                    log::warn!("Could not find conversation for response stream: {stream_id:?}");
                    return;
                };
                id
            }
        };

        let history_model = BlocklistAIHistoryModel::handle(ctx);
        let Some(conversation) = history_model.as_ref(ctx).conversation(&conversation_id) else {
            log::warn!("Conversation not found.");
            return;
        };
        let new_exchange_ids = conversation
            .new_exchange_ids_for_response(stream_id)
            .collect::<Vec<_>>();
        let mut was_passive_request = false;
        let mut is_any_exchange_unfinished = false;
        let mut actions_to_queue = vec![];

        for new_exchange_id in new_exchange_ids {
            let Some(exchange) = conversation.exchange_with_id(new_exchange_id) else {
                log::warn!("Exchange not found.");
                return;
            };
            was_passive_request |= exchange.has_passive_request();
            is_any_exchange_unfinished |= !exchange.output_status.is_finished();

            if let AIAgentOutputStatus::Finished {
                finished_output: FinishedAIAgentOutput::Success { output },
                ..
            } = &exchange.output_status
            {
                actions_to_queue.extend(output.get().actions().cloned());
            }
        }

        if let Some(stream_cancellation) = cancellation {
            if FeatureFlag::AgentSharedSessions.is_enabled()
                && !stream_cancellation
                    .reason
                    .should_preserve_in_progress_status()
            {
                delegate.send_cancellation_to_shared_session_viewers(ctx);
            }

            history_model.update(ctx, |history_model, ctx| {
                history_model.mark_response_stream_cancelled(
                    stream_id,
                    conversation_id,
                    owner_id.entity_id(),
                    stream_cancellation.reason,
                    ctx,
                );
            });

            if !was_passive_request
                && !stream_cancellation
                    .reason
                    .should_preserve_in_progress_status()
            {
                delegate.response_stream_cancelled(conversation_id, was_passive_request, ctx);
            }
        } else if is_any_exchange_unfinished {
            log::warn!("Response stream completed with an unfinished exchange and no error event.");

            history_model.update(ctx, |history_model, ctx| {
                history_model.mark_response_stream_completed_with_error(
                    RenderableAIError::transient_network_error(
                        false,
                        false,
                        TransientNetworkErrorKind::UnfinishedExchange,
                    ),
                    /*recovery_pending*/ false,
                    stream_id,
                    conversation_id,
                    owner_id.entity_id(),
                    ctx,
                );
            });
        } else if !actions_to_queue.is_empty() {
            delegate.queue_client_actions(actions_to_queue, conversation_id, ctx);
        }

        if cancellation.is_none() {
            delegate.response_stream_completed(stream_id, conversation_id, ctx);
        }

        if response_stream
            .as_ref(ctx)
            .should_resume_conversation_after_stream_finished()
        {
            delegate.schedule_auto_resume_after_error(conversation_id, ctx);
        }

        history_model.update(ctx, |history_model, _| {
            if let Some(conversation) = history_model.conversation_mut(&conversation_id) {
                conversation.cleanup_completed_response_stream(stream_id);
            }
        });
        ctx.unsubscribe_from_model(response_stream);

        if delegate.take_should_refresh_available_llms_on_stream_finish() {
            LLMPreferences::handle(ctx).update(ctx, |llm_preferences, ctx| {
                llm_preferences.refresh_authed_models(ctx);
            });
        }
        delegate.finished_receiving_output(stream_id.clone(), conversation_id, ctx);
        AIRequestUsageModel::handle(ctx).update(ctx, |request_usage_model, ctx| {
            request_usage_model.refresh_request_usage_async(ctx);
        });

        delegate.maybe_refresh_ai_overages(ctx);
    }

    /// Folds a stream-finished event into conversation history.
    pub(crate) fn fold_response_stream_finished<E>(
        owner_id: AgentSessionOwnerId,
        stream_id: &ResponseStreamId,
        mut finished_event: response_event::StreamFinished,
        conversation_id: AIConversationId,
        did_request_contain_user_query: bool,
        ctx: &mut ModelContext<E>,
    ) where
        E: Entity,
    {
        let history_model = BlocklistAIHistoryModel::handle(ctx);
        history_model.update(ctx, |history_model, ctx| {
            history_model.update_conversation_cost_and_usage_for_request(
                conversation_id,
                finished_event.request_cost.map(|cost| {
                    RequestCost::new(f64::from(cost.exact) + f64::from(cost.platform_credits))
                }),
                finished_event.token_usage,
                finished_event.conversation_usage_metadata.take(),
                did_request_contain_user_query,
                ctx,
            );
        });
        let should_refresh_model_config = finished_event.should_refresh_model_config;

        let owner_entity_id = owner_id.entity_id();
        match finished_event.reason {
            Some(stream_finished::Reason::Done(_)) | None => {
                history_model.update(ctx, |history_model, ctx| {
                    history_model.mark_response_stream_completed_successfully(
                        stream_id,
                        conversation_id,
                        owner_entity_id,
                        ctx,
                    );
                });
            }
            Some(stream_finished::Reason::Other(_)) => {
                let error_message =
                    "Response stream finished unexpectedly (with finish reason `Other`).";
                history_model.update(ctx, |history_model, ctx| {
                    history_model.mark_response_stream_completed_with_error(
                        RenderableAIError::Other {
                            error_message: error_message.to_owned(),
                            will_attempt_resume: false,
                            waiting_for_network: false,
                            is_user_error: false,
                        },
                        /*recovery_pending*/ false,
                        stream_id,
                        conversation_id,
                        owner_entity_id,
                        ctx,
                    );
                });
            }
            Some(stream_finished::Reason::ContextWindowExceeded(_)) => {
                let error_message = "Input exceeded context window limit.";
                history_model.update(ctx, |history_model, ctx| {
                    history_model.mark_response_stream_completed_with_error(
                        RenderableAIError::ContextWindowExceeded(error_message.to_owned()),
                        /*recovery_pending*/ false,
                        stream_id,
                        conversation_id,
                        owner_entity_id,
                        ctx,
                    );
                });
            }
            Some(stream_finished::Reason::QuotaLimit(_)) => {
                history_model.update(ctx, |history_model, ctx| {
                    history_model.mark_response_stream_completed_with_error(
                        RenderableAIError::QuotaLimit {
                            user_display_message: None,
                        },
                        /*recovery_pending*/ false,
                        stream_id,
                        conversation_id,
                        owner_entity_id,
                        ctx,
                    );
                });
            }
            Some(stream_finished::Reason::LlmUnavailable(_)) => {
                let error_message = "The LLM is currently unavailable.";
                history_model.update(ctx, |history_model, ctx| {
                    history_model.mark_response_stream_completed_with_error(
                        RenderableAIError::Other {
                            error_message: error_message.to_owned(),
                            will_attempt_resume: false,
                            waiting_for_network: false,
                            is_user_error: false,
                        },
                        /*recovery_pending*/ false,
                        stream_id,
                        conversation_id,
                        owner_entity_id,
                        ctx,
                    );
                });
            }
            Some(stream_finished::Reason::InvalidApiKey(details)) => {
                use warp_multi_agent_api::LlmProvider;
                let is_aws_bedrock = details
                    .provider
                    .try_into()
                    .ok()
                    .is_some_and(|p: LlmProvider| p == LlmProvider::AwsBedrock);

                let error = if is_aws_bedrock {
                    RenderableAIError::AwsBedrockCredentialsExpiredOrInvalid {
                        model_name: details.model_name,
                    }
                } else {
                    let provider = details.provider.try_into().ok().and_then(|p| match p {
                        LlmProvider::Google => Some("Google"),
                        LlmProvider::Anthropic => Some("Anthropic"),
                        LlmProvider::Openai => Some("OpenAI"),
                        LlmProvider::Xai => Some("xAI"),
                        LlmProvider::Openrouter => Some("OpenRouter"),
                        LlmProvider::AwsBedrock | LlmProvider::Unknown => None,
                    });
                    RenderableAIError::InvalidApiKey {
                        provider: provider.unwrap_or("Unknown").to_string(),
                        model_name: details.model_name,
                    }
                };

                history_model.update(ctx, |history_model, ctx| {
                    history_model.mark_response_stream_completed_with_error(
                        error,
                        /*recovery_pending*/ false,
                        stream_id,
                        conversation_id,
                        owner_entity_id,
                        ctx,
                    );
                });
            }
            Some(stream_finished::Reason::InternalError(stream_finished::InternalError {
                message,
            })) => {
                let error_message =
                    format!("Response stream finished unexpectedly with internal error: {message}");
                history_model.update(ctx, |history_model, ctx| {
                    history_model.mark_response_stream_completed_with_error(
                        RenderableAIError::Other {
                            error_message,
                            will_attempt_resume: false,
                            waiting_for_network: false,
                            is_user_error: false,
                        },
                        /*recovery_pending*/ false,
                        stream_id,
                        conversation_id,
                        owner_entity_id,
                        ctx,
                    );
                });
            }
            Some(stream_finished::Reason::MaxTokenLimit(_)) => {
                let error_message = "Input exceeded context window limit.";
                history_model.update(ctx, |history_model, ctx| {
                    history_model.mark_response_stream_completed_with_error(
                        RenderableAIError::ContextWindowExceeded(error_message.to_owned()),
                        /*recovery_pending*/ false,
                        stream_id,
                        conversation_id,
                        owner_entity_id,
                        ctx,
                    );
                });
            }
        }

        if should_refresh_model_config {
            LLMPreferences::handle(ctx).update(ctx, |llm_preferences, ctx| {
                llm_preferences.refresh_authed_models(ctx);
            });
        }
    }
}

#[cfg(test)]
#[path = "agent_conversation_engine_tests.rs"]
mod tests;
