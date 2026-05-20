// Session-sharing specific logic for BlocklistAIController.
// This module extends BlocklistAIController with methods used when viewing a shared session
// and defines state used only for session sharing.
use std::collections::HashMap;

use itertools::Itertools;
use session_sharing_protocol::common::{AgentAttachment, ParticipantId, ServerConversationToken};
use warp_core::features::FeatureFlag;
use warp_multi_agent_api::response_event::{stream_finished, ClientActions};
use warp_multi_agent_api::{client_action::Action, message::Message};

use super::response_stream::ResponseStreamId;
use super::{BlocklistAIController, RequestInput};
use crate::ai::agent::conversation::{AIConversationId, ConversationStatus};
use crate::ai::agent::{AIAgentActionId, AIAgentAttachment, EntrypointType};
use crate::ai::attachment_utils::{
    build_file_attachment_map, download_file, sanitize_filename, DownloadedAttachment,
};
use crate::ai::blocklist::agent_view::AgentViewEntryOrigin;
use crate::ai::blocklist::history_model::BlocklistAIHistoryModel;
use crate::server::server_api::ServerApiProvider;
use crate::terminal::model::block::BlockId;
use warpui::{AppContext, ModelContext, SingletonEntity};

#[derive(Default)]
pub(super) struct SharedSessionState {
    // The current active request id for the shared session (used if subsequent events do not provide a request id)
    current_response_id: Option<ResponseStreamId>,
    /// `true` if shared response streams should be ignored, used to skip processing MAA response streams that
    /// originate from session sharing replay when joining a new shared session for a follow-up on an existing
    /// ambient agent conversation.
    should_suppress_replayed_response_for_existing_conversation: bool,
    /// Per-stream latch derived from the gate above once we decide the current replay should be
    /// ignored.
    should_skip_current_replayed_response: bool,
    // The participant who initiated the current response stream
    current_response_initiator: Option<ParticipantId>,
    // The sharer's participant ID (set when session sharing starts)
    sharer_participant_id: Option<ParticipantId>,
}

impl BlocklistAIController {
    /// Controls whether replayed events for an already-restored shared-session conversation should
    /// be ignored to avoid duplicating content appended from follow-up scrollback.
    pub fn set_should_suppress_existing_agent_conversation_replay(&mut self, value: bool) {
        self.shared_session_state
            .should_suppress_replayed_response_for_existing_conversation = value;
    }

    /// Returns the current conversation ID for the active shared session stream.
    /// Returns None if there's no active shared session conversation.
    pub(crate) fn get_current_shared_session_conversation_id(
        &self,
        app: &AppContext,
    ) -> Option<AIConversationId> {
        self.shared_session_state
            .current_response_id
            .as_ref()
            .and_then(|response_id| {
                BlocklistAIHistoryModel::as_ref(app).conversation_for_response_stream(response_id)
            })
    }

    /// Handle a shared cancel control action and cancel the provided conversation
    /// (if it exists and is live).
    pub fn handle_shared_session_cancel_action(
        &mut self,
        server_conversation_token: ServerConversationToken,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(conversation_id) = self.find_existing_conversation_by_server_token(
            &server_conversation_token.to_string(),
            ctx,
        ) else {
            return;
        };

        if BlocklistAIHistoryModel::as_ref(ctx).is_conversation_live(conversation_id) {
            self.cancel_conversation_progress(
                conversation_id,
                super::CancellationReason::ManuallyCancelled,
                ctx,
            );
        }
    }

    /// Apply agent session events to the current conversation state.
    pub fn handle_shared_session_response_event(
        &mut self,
        resp: warp_multi_agent_api::ResponseEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(kind) = resp.r#type else {
            return;
        };
        match kind {
            warp_multi_agent_api::response_event::Type::Init(init) => {
                self.on_shared_init(init, ctx)
            }
            warp_multi_agent_api::response_event::Type::ClientActions(actions) => {
                self.on_shared_client_actions(actions, ctx)
            }
            warp_multi_agent_api::response_event::Type::Finished(finished) => {
                self.on_shared_finished(finished, ctx);
            }
        }
    }

    fn on_shared_init(
        &mut self,
        init_event: warp_multi_agent_api::response_event::StreamInit,
        ctx: &mut ModelContext<Self>,
    ) {
        let stream_id = ResponseStreamId::for_shared_session(&init_event);
        self.shared_session_state.current_response_id = None;
        self.shared_session_state
            .should_skip_current_replayed_response = false;
        let terminal_view_id = self.terminal_view_id;
        let history = BlocklistAIHistoryModel::handle(ctx);

        // If the server conversation already exists locally (matched by server_conversation_token), reuse it.
        // Otherwise, if we're currently in an empty agent view conversation, reuse that
        // local conversation ID and bind the incoming server token to it.
        // This preserves block visibility for terminal blocks created in the given agent view.
        let existing_conversation_id =
            self.find_existing_conversation_by_server_token(&init_event.conversation_id, ctx);
        let conversation_id = existing_conversation_id
            .or_else(|| {
                let selected_conversation_id = self
                    .context_model
                    .as_ref(ctx)
                    .selected_conversation_id(ctx)?;

                // If the current agent view's conversation is completely empty,
                // we should just associate it with the incoming request/token.
                let should_reuse_selected_conversation = history
                    .as_ref(ctx)
                    .conversation(&selected_conversation_id)
                    .is_some_and(|conversation| {
                        conversation.exchange_count() == 0
                            && conversation.server_conversation_token().is_none()
                    });
                if !should_reuse_selected_conversation {
                    return None;
                }

                history.update(ctx, |history, ctx| {
                    history.set_server_conversation_token_for_conversation(
                        selected_conversation_id,
                        init_event.conversation_id.clone(),
                    );
                    history.set_viewing_shared_session_for_conversation(
                        selected_conversation_id,
                        true,
                    );
                    ctx.notify();
                });

                Some(selected_conversation_id)
            })
            .unwrap_or_else(|| {
                history.update(ctx, |h, ctx| {
                    h.start_new_conversation(terminal_view_id, false, true, false, ctx)
                })
            });
        if self.should_skip_replayed_response_for_existing_conversation(
            existing_conversation_id,
            &init_event.request_id,
            ctx,
        ) {
            self.shared_session_state.current_response_id = Some(stream_id);
            self.shared_session_state
                .should_skip_current_replayed_response = true;
            return;
        }

        self.shared_session_state.current_response_id = Some(stream_id.clone());
        if existing_conversation_id.is_some() {
            history.update(ctx, |history, ctx| {
                history.set_viewing_shared_session_for_conversation(conversation_id, true);
                ctx.notify();
            });
        }

        let Some(conversation) = history.as_ref(ctx).conversation(&conversation_id) else {
            log::error!(
                "Tried to initialize shared session stream for non-existent conversation  {conversation_id:?}"
            );
            return;
        };
        let task_id = conversation.get_root_task_id().clone();

        // Ensure the action executor is in view-only mode for shared-session viewers.
        self.action_model.update(ctx, |action_model, _ctx| {
            action_model.set_view_only(true);
        });

        // Eagerly create an exchange for this request (with empty inputs) and initialize output.
        history.update(ctx, |history_model, ctx| {
            let _ = history_model.update_conversation_for_new_request_input(
                RequestInput::for_task(
                    vec![],
                    task_id,
                    &self.active_session,
                    self.get_current_response_initiator(),
                    conversation_id,
                    self.terminal_view_id,
                    ctx,
                ),
                stream_id.clone(),
                self.terminal_view_id,
                ctx,
            );

            history_model.initialize_output_for_response_stream(
                &stream_id,
                conversation_id,
                self.terminal_view_id,
                init_event.clone(),
                ctx,
            );

            // Mark conversation as in progress and active/selected
            history_model.update_conversation_status(
                self.terminal_view_id,
                conversation_id,
                ConversationStatus::InProgress,
                ctx,
            );
            history_model.set_active_conversation_id(conversation_id, self.terminal_view_id, ctx);
        });
        self.context_model.update(ctx, |context_model, ctx| {
            context_model.set_pending_query_state_for_existing_conversation(
                conversation_id,
                AgentViewEntryOrigin::SharedSessionSelection,
                ctx,
            );
        });
    }

    /// Returns whether replayed events for an already-populated shared-session conversation should
    /// be ignored to avoid duplicating content that was restored from scrollback.
    fn should_skip_replayed_response_for_existing_conversation(
        &self,
        existing_conversation_id: Option<AIConversationId>,
        init_request_id: &str,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        let Some(conversation_id) = existing_conversation_id else {
            log::info!(
                "should_skip_replayed_response: no existing conversation id, not skipping \
                 (request_id={init_request_id})"
            );
            return false;
        };
        let is_receiving_replay = self
            .terminal_model
            .lock()
            .is_receiving_agent_conversation_replay();
        let suppress_enabled = self
            .shared_session_state
            .should_suppress_replayed_response_for_existing_conversation;
        if !is_receiving_replay || !suppress_enabled {
            log::info!(
                "should_skip_replayed_response: not skipping \
                 (request_id={init_request_id}, conversation_id={conversation_id:?}, \
                 is_receiving_replay={is_receiving_replay}, suppress_enabled={suppress_enabled})"
            );
            return false;
        }

        // Only skip the replayed response when our local task already has the given request_id.
        // New exchanges (e.g. the user's first post-handoff prompt) carry unseen request_ids and must flow through normally.
        let history = BlocklistAIHistoryModel::as_ref(ctx);
        let found = history.conversation(&conversation_id).is_some_and(|conv| {
            conv.all_tasks()
                .any(|task| task.messages().any(|msg| msg.request_id == init_request_id))
        });
        if !found {
            log::info!(
                "should_skip_replayed_response: not skipping, request_id not found in local \
                 conversation (request_id={init_request_id}, conversation_id={conversation_id:?})"
            );
        }
        found
    }

    fn on_shared_client_actions(
        &mut self,
        actions: warp_multi_agent_api::response_event::ClientActions,
        ctx: &mut ModelContext<Self>,
    ) {
        if self
            .shared_session_state
            .should_skip_current_replayed_response
        {
            return;
        }
        let Some(stream_id) = self.shared_session_state.current_response_id.clone() else {
            log::warn!("Received shared session client actions with no active response stream id.");
            return;
        };

        let Some(conversation_id) =
            BlocklistAIHistoryModel::as_ref(ctx).conversation_for_response_stream(&stream_id)
        else {
            log::warn!(
                "No conversation ID for shared session response stream with id={stream_id:?}"
            );
            return;
        };

        self.update_directory_context_from_client_actions(&actions, ctx);
        let history_model = BlocklistAIHistoryModel::handle(ctx);
        history_model.update(ctx, |history_model, ctx| {
            if let Err(e) = history_model.apply_client_actions(
                &stream_id,
                actions.actions,
                conversation_id,
                self.terminal_view_id,
                ctx,
            ) {
                log::error!(
                    "Failed to apply client actions to conversation for shared session: {e:?}"
                );
            }
        });
        let Some(conversation) = history_model.as_ref(ctx).conversation(&conversation_id) else {
            log::error!("Failed to find conversation with id: {conversation_id:?}");
            return;
        };

        let new_action_results_to_apply = conversation
            .new_exchange_ids_for_response(&stream_id)
            .filter_map(|exchange_id| conversation.exchange_with_id(exchange_id))
            .flat_map(|exchange| {
                exchange
                    .input
                    .iter()
                    .filter_map(|i| i.action_result().cloned())
            })
            .collect_vec();

        // Apply finished results to unfinished actions.
        for result in new_action_results_to_apply.into_iter() {
            if self
                .action_model
                .as_ref(ctx)
                .get_action_result(&result.id)
                .is_none()
            {
                self.action_model.update(ctx, |action_model, ctx| {
                    action_model.apply_finished_action_result(conversation_id, result, ctx);
                });
            }
        }
    }

    /// Update the context model's working directory context using the most recent message context.
    fn update_directory_context_from_client_actions(
        &mut self,
        actions: &ClientActions,
        ctx: &mut ModelContext<Self>,
    ) {
        for client_action in &actions.actions {
            if let Some(Action::AddMessagesToTask(add)) = &client_action.action {
                for message in &add.messages {
                    if let Some(inner) = &message.message {
                        let ctx_opt = match inner {
                            Message::UserQuery(uq) => uq.context.as_ref(),
                            Message::SystemQuery(sq) => sq.context.as_ref(),
                            Message::ToolCallResult(tcr) => tcr.context.as_ref(),
                            _ => None,
                        };

                        if let Some(input_ctx) = ctx_opt {
                            if let Some(dir) = &input_ctx.directory {
                                self.context_model.update(ctx, |context_model, ctx| {
                                    context_model.update_directory_context(
                                        if dir.pwd.is_empty() {
                                            None
                                        } else {
                                            Some(dir.pwd.clone())
                                        },
                                        if dir.home.is_empty() {
                                            None
                                        } else {
                                            Some(dir.home.clone())
                                        },
                                        ctx,
                                    );
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    fn on_shared_finished(
        &mut self,
        finished: warp_multi_agent_api::response_event::StreamFinished,
        ctx: &mut ModelContext<Self>,
    ) {
        if self
            .shared_session_state
            .should_skip_current_replayed_response
        {
            self.shared_session_state.current_response_id.take();
            self.shared_session_state
                .should_skip_current_replayed_response = false;
            return;
        }
        let Some(stream_id) = self.shared_session_state.current_response_id.take() else {
            log::warn!("Shared Finished missing request_id");
            return;
        };
        let Some(conversation_id) =
            BlocklistAIHistoryModel::as_ref(ctx).conversation_for_response_stream(&stream_id)
        else {
            log::warn!(
                "No conversation ID for shared session response stream with id={stream_id:?}"
            );
            return;
        };

        let history_model = BlocklistAIHistoryModel::handle(ctx);
        let Some(conversation) = history_model.as_ref(ctx).conversation(&conversation_id) else {
            log::error!("Failed to find conversation with id: {conversation_id:?}");
            return;
        };

        // Queue actions for viewer UI in view-only mode
        let mut actions_to_queue = vec![];
        let mut did_exchange_contain_user_query = false;

        for new_exchange_id in conversation.new_exchange_ids_for_response(&stream_id) {
            let Some(exchange) = conversation.exchange_with_id(new_exchange_id) else {
                continue;
            };
            did_exchange_contain_user_query |=
                exchange.input.iter().any(|input| input.is_user_query());

            if let Some(output) = exchange.output_status.output() {
                actions_to_queue.extend(output.get().actions().cloned().collect_vec().into_iter());
            }
        }

        if !actions_to_queue.is_empty() {
            self.action_model.update(ctx, |action_model, ctx| {
                action_model.queue_actions(actions_to_queue, conversation_id, ctx);
            });
        }

        self.handle_response_stream_finished(
            &stream_id,
            finished,
            conversation_id,
            did_exchange_contain_user_query,
            ctx,
        );
    }

    /// Finds an existing client conversation whose server_conversation_token matches `server_token`.
    /// Searches only live conversations for this terminal view. Returns None if no match is found.
    pub fn find_existing_conversation_by_server_token(
        &self,
        server_token: &str,
        ctx: &mut ModelContext<Self>,
    ) -> Option<AIConversationId> {
        let history = BlocklistAIHistoryModel::handle(ctx);
        history
            .as_ref(ctx)
            .all_live_conversations_for_terminal_view(self.terminal_view_id)
            .find_map(|conv| {
                conv.server_conversation_token()
                    .and_then(|t| (t.as_str() == server_token).then_some(conv.id()))
            })
    }

    /// Sends a synthetic cancellation event to viewers when the sharer cancels a conversation.
    /// This ensures viewers see the conversation as cancelled and update their UI accordingly.
    pub(super) fn send_cancellation_to_viewers(&mut self, ctx: &mut ModelContext<Self>) {
        if !self
            .terminal_model
            .lock()
            .shared_session_status()
            .is_sharer()
        {
            return;
        }

        // Get the current conversation and build usage metadata from it.
        let conversation_id = self.get_current_shared_session_conversation_id(ctx);
        let usage_metadata = conversation_id.and_then(|conv_id| {
            BlocklistAIHistoryModel::as_ref(ctx)
                .conversation(&conv_id)
                .map(|conversation| stream_finished::ConversationUsageMetadata {
                    context_window_usage: conversation.context_window_usage(),
                    credits_spent: conversation.credits_spent(),
                    summarized: conversation.was_summarized(),
                    #[allow(deprecated)]
                    token_usage: conversation
                        .token_usage()
                        .iter()
                        .map(|u| u.to_proto_combined())
                        .collect(),
                    tool_usage_metadata: Some(conversation.tool_usage_metadata().into()),
                    warp_token_usage: conversation
                        .token_usage()
                        .iter()
                        .filter_map(|u| u.to_proto_warp_usage())
                        .collect(),
                    byok_token_usage: conversation
                        .token_usage()
                        .iter()
                        .filter_map(|u| u.to_proto_byok_usage())
                        .collect(),
                })
        });

        // Create a synthetic StreamFinished event to notify viewers of the cancellation.
        // We use "Done" reason rather than a specific cancellation reason because
        // the proto doesn't have explicit variants for UserCommandExecuted or ManuallyCancelled.
        // TODO: we should probably add representations for said variants in the proto for this usecase.
        let finished_event = warp_multi_agent_api::ResponseEvent {
            r#type: Some(warp_multi_agent_api::response_event::Type::Finished(
                warp_multi_agent_api::response_event::StreamFinished {
                    reason: Some(stream_finished::Reason::Done(stream_finished::Done {})),
                    conversation_usage_metadata: usage_metadata,
                    token_usage: vec![],
                    should_refresh_model_config: false,
                    request_cost: None,
                },
            )),
        };

        // Send the cancellation event to viewers.
        // If no initiator is tracked, fall back to the sharer's participant ID.
        let forked_from_token = conversation_id.and_then(|conv_id| {
            BlocklistAIHistoryModel::as_ref(ctx)
                .conversation(&conv_id)
                .and_then(|conv| {
                    conv.forked_from_server_conversation_token()
                        .map(|t| t.as_str().to_string())
                })
        });
        self.terminal_model
            .lock()
            .send_agent_response_for_shared_session(
                &finished_event,
                self.get_current_response_initiator()
                    .or_else(|| self.get_sharer_participant_id()),
                forked_from_token,
            );
    }

    /// Marks an action as remotely executing when a viewer receives a CommandExecutionStarted event.
    /// This allows the viewer's UI to show the action as running rather than queued.
    pub fn mark_action_as_remotely_executing_in_shared_session(
        &mut self,
        action_id: &AIAgentActionId,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        self.action_model.update(ctx, |action_model, ctx| {
            action_model.mark_action_as_remotely_executing(action_id, conversation_id, ctx);
        });
    }

    /// Sets the participant ID for the current response.
    /// This should be called when initiating a query to track who sent it.
    pub fn set_current_response_initiator(&mut self, participant_id: ParticipantId) {
        self.shared_session_state.current_response_initiator = Some(participant_id);
    }

    /// Gets the participant ID for the current response.
    pub(super) fn get_current_response_initiator(&self) -> Option<ParticipantId> {
        self.shared_session_state.current_response_initiator.clone()
    }

    /// Sets the sharer's participant ID. Should be called when a shared session is created.
    pub fn set_sharer_participant_id(&mut self, participant_id: ParticipantId) {
        self.shared_session_state.sharer_participant_id = Some(participant_id);
    }

    /// Gets the sharer's participant ID.
    pub(super) fn get_sharer_participant_id(&self) -> Option<ParticipantId> {
        self.shared_session_state.sharer_participant_id.clone()
    }

    /// Links a forked conversation's new token to an existing conversation.
    /// This is called on the viewer side when receiving a response for a forked conversation
    /// so that new responses are added to the correct conversation.
    pub fn link_forked_conversation_token(
        &mut self,
        forked_from_token: &str,
        event: &warp_multi_agent_api::ResponseEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        // Extract the new server conversation id from the StreamInit event
        let new_conversation_id = match &event.r#type {
            Some(warp_multi_agent_api::response_event::Type::Init(init)) => {
                init.conversation_id.as_str()
            }
            // Only StreamInit events have conversation_id.
            _ => return,
        };

        // Find the conversation with the forked_from token
        if let Some(conversation_id) =
            self.find_existing_conversation_by_server_token(forked_from_token, ctx)
        {
            // Update the conversation's server_conversation_token to the new one
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                history.set_server_conversation_token_for_conversation(
                    conversation_id,
                    new_conversation_id.to_string(),
                );
                ctx.notify();
            });
        }
    }

    /// Execute an agent prompt on behalf of the viewer.
    pub fn execute_agent_prompt_for_shared_session(
        &mut self,
        prompt: String,
        server_conversation_token: Option<ServerConversationToken>,
        attachments: Vec<AgentAttachment>,
        participant_id: ParticipantId,
        ctx: &mut ModelContext<Self>,
    ) {
        // Map server token to sharer's local conversation ID
        let conversation_id = server_conversation_token
            .and_then(|id| self.find_existing_conversation_by_server_token(&id.to_string(), ctx))
            .and_then(
                |id| match BlocklistAIHistoryModel::as_ref(ctx).conversation(&id) {
                    Some(c) => Some(c),
                    None => {
                        log::error!(
                            "Tried to execute prompt for non-existent conversation: {id:?}",
                        );
                        None
                    }
                },
            )
            .map(|conversation| conversation.id());

        // Process attachments and set them in the context model
        let mut block_ids = Vec::new();
        let mut selected_text_parts = Vec::new();
        let mut file_downloads: Vec<(String, String)> = Vec::new();
        for attachment in attachments {
            match attachment {
                AgentAttachment::BlockReference { block_id } => {
                    // Convert protocol BlockId to app BlockId
                    block_ids.push(BlockId::from(block_id.to_string()));
                }
                AgentAttachment::PlainText { content } => {
                    selected_text_parts.push(content);
                }
                AgentAttachment::FileReference {
                    attachment_id,
                    file_name,
                } => {
                    file_downloads.push((attachment_id, file_name));
                }
            }
        }

        // Set block and text attachments in the context model.
        self.context_model.update(ctx, |context_model, ctx| {
            // Set block IDs if any were provided
            if !block_ids.is_empty() {
                context_model.set_pending_context_block_ids(block_ids, false, ctx);
            }

            // Set selected text if any was provided
            if !selected_text_parts.is_empty() {
                let combined_text = selected_text_parts.join("\n");
                context_model.set_pending_context_selected_text(Some(combined_text), false, ctx);
            }
        });

        // If there are no file downloads (or the feature is disabled), send the query immediately.
        if file_downloads.is_empty() || !FeatureFlag::CloudModeImageContext.is_enabled() {
            self.send_shared_session_query(
                prompt,
                conversation_id,
                participant_id,
                HashMap::new(),
                ctx,
            );
            return;
        }

        // We have file downloads — ensure both the download dir and task ID are available.
        let Some(attachment_dir) = self.attachments_download_dir.clone() else {
            log::error!(
                "No attachments_download_dir set on controller, cannot process file attachments"
            );
            self.send_shared_session_query(
                prompt,
                conversation_id,
                participant_id,
                HashMap::new(),
                ctx,
            );
            return;
        };
        let Some(task_id) = self.ambient_agent_task_id else {
            log::error!("No task_id available to download attachments");
            self.send_shared_session_query(
                prompt,
                conversation_id,
                participant_id,
                HashMap::new(),
                ctx,
            );
            return;
        };

        let ai_client = ServerApiProvider::as_ref(ctx).get_ai_client();
        let server_api = ServerApiProvider::as_ref(ctx).get();
        let attachment_ids: Vec<String> = file_downloads.iter().map(|(id, _)| id.clone()).collect();

        // Fetch presigned download URLs from the server, download files to disk,
        // then build the attachment map from only the successfully downloaded files.
        ctx.spawn(
            async move {
                let download_urls = match ai_client
                    .download_task_attachments(&task_id, &attachment_ids)
                    .await
                {
                    Ok(resp) => resp
                        .attachments
                        .into_iter()
                        .map(|att| (att.attachment_id, att.download_url))
                        .collect::<std::collections::HashMap<_, _>>(),
                    Err(e) => {
                        log::error!("Failed to get download URLs for task {task_id}: {e}");
                        return vec![];
                    }
                };

                if let Err(e) = async_fs::create_dir_all(&attachment_dir).await {
                    log::error!("Failed to create attachments directory: {e}");
                    return vec![];
                }

                let mut downloaded = Vec::new();
                for (attachment_id, file_name) in &file_downloads {
                    let Some(url) = download_urls.get(attachment_id) else {
                        log::warn!("No download URL for attachment {attachment_id}");
                        continue;
                    };
                    let safe_name = sanitize_filename(file_name).to_string();
                    let dest = attachment_dir.join(format!("{attachment_id}_{safe_name}"));

                    match download_file(server_api.http_client(), url, &dest).await {
                        Ok(_) => {
                            downloaded.push(DownloadedAttachment {
                                file_id: attachment_id.clone(),
                                file_name: safe_name,
                                file_path: dest.to_string_lossy().into_owned(),
                            });
                        }
                        Err(e) => {
                            log::error!("Failed to download {safe_name}: {e}");
                        }
                    }
                }
                downloaded
            },
            move |controller, downloaded, ctx| {
                let file_attachments = build_file_attachment_map(&downloaded);
                controller.send_shared_session_query(
                    prompt,
                    conversation_id,
                    participant_id,
                    file_attachments,
                    ctx,
                );
            },
        );
    }

    /// Helper to send a shared-session query, used both for immediate sends
    /// (no file attachments) and deferred sends (after file downloads complete).
    fn send_shared_session_query(
        &mut self,
        prompt: String,
        conversation_id: Option<AIConversationId>,
        participant_id: ParticipantId,
        file_attachments: HashMap<String, AIAgentAttachment>,
        ctx: &mut ModelContext<Self>,
    ) {
        if let Some(conversation_id) = conversation_id {
            if FeatureFlag::AgentView.is_enabled() {
                // Enter agent view for this conversation so the sharer's UI state is correct
                // and updates are sent to the viewer.
                self.context_model.update(ctx, |context_model, ctx| {
                    context_model.set_pending_query_state_for_existing_conversation(
                        conversation_id,
                        AgentViewEntryOrigin::SharedSessionSelection,
                        ctx,
                    );
                });
            }
            self.send_user_query_in_conversation_with_attachments(
                prompt,
                conversation_id,
                Some(participant_id),
                file_attachments,
                ctx,
            );
        } else {
            if FeatureFlag::AgentView.is_enabled() {
                // If we're already in an empty agent view conversation, reuse it
                // (so that any command blocks remain visible). Otherwise create a new one for the given prompt.
                let history = BlocklistAIHistoryModel::handle(ctx);
                let origin = AgentViewEntryOrigin::SharedSessionSelection;

                let Some(conversation_id) = self
                    .context_model
                    .as_ref(ctx)
                    .selected_conversation_id(ctx)
                    .filter(|conversation_id| {
                        history
                            .as_ref(ctx)
                            .conversation(conversation_id)
                            .is_some_and(|conversation| {
                                conversation.exchange_count() == 0
                                    && conversation.server_conversation_token().is_none()
                            })
                    })
                    .or_else(|| {
                        self.context_model.update(ctx, |context_model, ctx| {
                            context_model
                                .try_enter_agent_view_for_new_conversation(origin, ctx)
                                .ok()
                        })
                    })
                else {
                    log::error!("Failed to get conversation id for shared session prompt");
                    return;
                };

                self.send_user_query_in_conversation_with_attachments(
                    prompt,
                    conversation_id,
                    Some(participant_id),
                    file_attachments,
                    ctx,
                );
                return;
            }

            self.send_user_query_in_new_conversation(
                prompt,
                None,
                EntrypointType::SharedSession,
                Some(participant_id),
                ctx,
            );
        }
    }
}
