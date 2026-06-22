use std::sync::Arc;

use ai::agent::action::{AIAgentActionType, FileEdit};
use ai::diff_validation::ParsedDiff;
use chrono::{DateTime, Utc};
use parking_lot::FairMutex;
use warp_core::features::FeatureFlag;
use warpui::r#async::SpawnedFutureHandle;
use warpui::{Entity, EntityId, ModelContext, ModelHandle, SingletonEntity};

use super::super::controller::{BlocklistAIController, BlocklistAIControllerEvent};
use crate::ai::agent::api::generate_multi_agent_output;
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::{
    AIIdentifiers, FileContext, PassiveCodeDiffEntry, PassiveSuggestionTrigger,
    ShellCommandCompletedTrigger,
};
use crate::ai::block_context::BlockContext;
use crate::ai::blocklist::inline_action::code_diff_view::FileDiff;
use crate::ai::blocklist::{
    apply_edits, BlocklistAIHistoryModel, FileReadResult, RequestFileEditsFormatKind,
    SessionContext,
};
use crate::ai::paths::host_native_absolute_path;
use crate::auth::auth_state::AuthStateProvider;
use crate::server::server_api::ServerApiProvider;
use crate::settings::AISettings;
use crate::terminal::event::{BlockType, UserBlockCompleted};
use crate::terminal::model::session::active_session::ActiveSession;
use crate::terminal::model::terminal_model::TerminalModel;
use crate::terminal::model_events::{ModelEvent, ModelEventDispatcher};
use crate::terminal::view::ambient_agent::AmbientAgentViewModel;
use crate::workspaces::user_workspaces::UserWorkspaces;

cfg_if::cfg_if! {
    if #[cfg(feature = "local_fs")] {
        use std::{path::PathBuf, time::Duration};
        use crate::ai::blocklist::{read_local_file_context, BlocklistAIPermissions};
        use warp_terminal::shell::ShellLaunchData;
        use crate::util::link_detection::{detect_file_paths, DetectedLinkType};
        use crate::util::openable_file_type::is_binary_file;
        use ai::agent::FileLocations;
        use warpui::AppContext;
        use warpui::r#async::FutureExt as AsyncFutureExt;
        use itertools::Itertools;
    }
}

pub enum PassiveSuggestionsEvent {
    NewPromptSuggestion {
        prompt: String,
        label: Option<String>,
        request_duration_ms: u64,
        /// The trigger for the suggestion. `None` when the server indicated the
        /// trigger is not relevant to the suggestion.
        trigger: Option<PassiveSuggestionTrigger>,
        conversation_id: Option<AIConversationId>,
        /// The server-assigned request token from the passive suggestion
        /// request. Used to join client-side telemetry with server-side logs.
        server_request_token: Option<String>,
    },
    NewCodeDiffSuggestion {
        diffs: Vec<FileDiff>,
        edit_format_kind: RequestFileEditsFormatKind,
        title: Option<String>,
        /// The original search/replace edits from the LLM response.
        original_edits: Vec<PassiveCodeDiffEntry>,
        /// The conversation ID to continue in when the user clicks "iterate
        /// with agent" or "accept and continue". `None` for ephemeral triggers
        /// (shell command with no prior conversation).
        conversation_id: Option<AIConversationId>,
        request_duration_ms: u64,
        trigger: PassiveSuggestionTrigger,
        /// The server-assigned request token from the passive suggestion
        /// request. Used to join client-side telemetry with server-side logs.
        /// `None` on the legacy code path.
        server_request_token: Option<String>,
    },
}

/// Tracks an out-of-band passive suggestions request.
struct Request {
    /// Conversation ID for the passive suggestion request.
    conversation_id: AIConversationId,
    trigger: PassiveSuggestionTrigger,
    start_ts: DateTime<Utc>,

    /// Handle to the spawned future processing the response stream.
    _stream_handle: SpawnedFutureHandle,
    _cancellation_tx: futures::channel::oneshot::Sender<()>,
}

pub struct PassiveSuggestionsModel {
    ai_controller: ModelHandle<BlocklistAIController>,
    latest_request: Option<Request>,
    pending_file_read_handle: Option<SpawnedFutureHandle>,
    terminal_model: Arc<FairMutex<TerminalModel>>,
    ambient_agent_view_model: Option<ModelHandle<AmbientAgentViewModel>>,

    #[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
    terminal_view_id: EntityId,
    #[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
    active_session: ModelHandle<ActiveSession>,
}

impl PassiveSuggestionsModel {
    pub fn new(
        active_session: ModelHandle<ActiveSession>,
        terminal_model: Arc<FairMutex<TerminalModel>>,
        ai_controller: ModelHandle<BlocklistAIController>,
        model_event_dispatcher: &ModelHandle<ModelEventDispatcher>,
        ambient_agent_view_model: Option<ModelHandle<AmbientAgentViewModel>>,
        terminal_view_id: EntityId,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        ctx.subscribe_to_model(model_event_dispatcher, |me, _, event, ctx| {
            me.handle_model_event(event, ctx);
        });
        ctx.subscribe_to_model(&ai_controller, |me, _, event, ctx| {
            me.handle_controller_event(event, ctx);
        });

        Self {
            active_session,
            ai_controller,
            latest_request: None,
            pending_file_read_handle: None,
            terminal_model,
            ambient_agent_view_model,
            terminal_view_id,
        }
    }

    pub fn abort_pending_requests(&mut self, _ctx: &mut ModelContext<Self>) {
        if let Some(handle) = self.pending_file_read_handle.take() {
            handle.abort();
        }
        // Dropping the [`Request`] aborts the spawned stream handle.
        self.latest_request.take();
    }

    fn is_ambient_agent_session(&self, ctx: &ModelContext<Self>) -> bool {
        self.ambient_agent_view_model
            .as_ref()
            .is_some_and(|model| model.as_ref(ctx).is_ambient_agent())
    }

    /// Sends a MAA request to generate passive suggestions.
    ///
    /// As much as possible, this method avoids mutating the conversation data model;
    /// the response stream is consumed inline to extract relevant tool calls
    /// without touching conversation history. This is intentional to avoid
    /// polluting existing conversations and the history model in general
    /// with passive conversations.
    fn send_request(
        &mut self,
        followup_conversation_id: Option<AIConversationId>,
        trigger: PassiveSuggestionTrigger,
        supported_tools: Vec<warp_multi_agent_api::ToolType>,
        ctx: &mut ModelContext<Self>,
    ) {
        // Capture before the call — `Some` means there's a real conversation
        // the user can continue in; `None` means ephemeral.
        let continuable_conversation_id = followup_conversation_id;
        let Ok((conversation_id, request_params)) =
            self.ai_controller.update(ctx, |controller, ctx| {
                controller.build_passive_suggestions_request_params(
                    followup_conversation_id,
                    trigger.clone(),
                    supported_tools,
                    ctx,
                )
            })
        else {
            return;
        };

        let server_api = ServerApiProvider::as_ref(ctx).get();
        let (cancellation_tx, cancellation_rx) = futures::channel::oneshot::channel();

        let stream_handle = ctx.spawn(
            async move {
                let stream_result =
                    generate_multi_agent_output(server_api, request_params, cancellation_rx).await;
                extract_suggestion_from_stream(stream_result).await
            },
            move |me, result, ctx| {
                let Some(latest_request) = &me.latest_request else {
                    return;
                };
                if latest_request.conversation_id != conversation_id {
                    return;
                }
                if !me.is_suggestion_still_valid(ctx) {
                    return;
                }
                let Some(extracted) = result else {
                    return;
                };

                let request_duration_ms = Utc::now()
                    .signed_duration_since(latest_request.start_ts)
                    .num_milliseconds()
                    .max(0) as u64;
                let trigger = latest_request.trigger.clone();

                let StreamExtractionResult {
                    suggestion: extracted,
                    server_request_token,
                } = extracted;
                match extracted {
                    ExtractedSuggestion::Prompt {
                        prompt,
                        label,
                        is_trigger_irrelevant,
                    } => {
                        if prompt.is_empty() {
                            return;
                        }
                        let trigger = if is_trigger_irrelevant {
                            log::debug!("[passive-suggestions] trigger marked irrelevant, omitting from prompt suggestion event");
                            None
                        } else {
                            Some(trigger)
                        };
                        ctx.emit(PassiveSuggestionsEvent::NewPromptSuggestion {
                            prompt,
                            label,
                            request_duration_ms,
                            trigger,
                            conversation_id: continuable_conversation_id,
                            server_request_token,
                        });
                    }
                    ExtractedSuggestion::CodeDiff { apply_file_diffs } => {
                        let AIAgentActionType::RequestFileEdits { file_edits, title } =
                            AIAgentActionType::from(apply_file_diffs)
                        else {
                            unreachable!()
                        };

                        let edit_format_kind = classify_edit_format(&file_edits);
                        let original_edits = file_edits_to_passive_diffs(&file_edits);

                        let session_context =
                            SessionContext::from_session(me.active_session.as_ref(ctx), ctx);
                        let identifiers = AIIdentifiers::default();
                        let background_executor = ctx.background_executor();
                        let auth_state = AuthStateProvider::as_ref(ctx).get().clone();

                        ctx.spawn(
                            async move {
                                apply_edits(
                                    file_edits,
                                    &session_context,
                                    &identifiers,
                                    background_executor,
                                    auth_state,
                                    true,
                                    |path| async move {
                                        FileReadResult::from(std::fs::read_to_string(path))
                                    },
                                )
                                .await
                            },
                            move |me: &mut Self, applied_diffs: Result<Vec<ai::diff_validation::AIRequestedCodeDiff>, _>, ctx: &mut ModelContext<Self>| {
                                let Ok(applied_diffs) = applied_diffs else {
                                    log::warn!("[passive-code-diff] apply_edits failed");
                                    return;
                                };
                                if applied_diffs.is_empty() {
                                    log::warn!("[passive-code-diff] no diffs generated");
                                    return;
                                }

                                let cwd = me
                                    .active_session
                                    .as_ref(ctx)
                                    .current_working_directory()
                                    .cloned();
                                let shell = me.active_session.as_ref(ctx).shell_launch_data(ctx);

                                let diffs: Vec<FileDiff> = applied_diffs
                                    .into_iter()
                                    .map(|diff: ai::diff_validation::AIRequestedCodeDiff| {
                                        let path = host_native_absolute_path(
                                            diff.file_name.as_str(),
                                            &shell,
                                            &cwd,
                                        );
                                        FileDiff::new(diff.original_content, path, diff.diff_type)
                                    })
                                    .collect();

                                ctx.emit(PassiveSuggestionsEvent::NewCodeDiffSuggestion {
                                    diffs,
                                    edit_format_kind,
                                    title,
                                    original_edits: original_edits.clone(),
                                    conversation_id: continuable_conversation_id,
                                    request_duration_ms,
                                    trigger,
                                    server_request_token: server_request_token.clone(),
                                });
                            },
                        );
                    }
                }
            },
        );

        self.latest_request = Some(Request {
            _stream_handle: stream_handle,
            _cancellation_tx: cancellation_tx,
            conversation_id,
            trigger,
            start_ts: Utc::now(),
        });
    }

    /// Returns true if the current suggestion context is still valid.
    fn is_suggestion_still_valid(&self, ctx: &ModelContext<Self>) -> bool {
        let Some(latest_request) = &self.latest_request else {
            return false;
        };
        match &latest_request.trigger {
            PassiveSuggestionTrigger::AgentResponseCompleted { exchange_id } => {
                // If there's been a non-passive exchange since the one that triggered
                // this prompt suggestion, it isn't valid anymore.
                BlocklistAIHistoryModel::as_ref(ctx)
                    .conversation(&latest_request.conversation_id)
                    .and_then(|c| c.last_non_passive_exchange())
                    .map(|e| &e.id)
                    == Some(exchange_id)
            }
            PassiveSuggestionTrigger::ShellCommandCompleted(trigger) => {
                // If the latest block has changed, this isn't a valid suggestion anymore.
                self.terminal_model
                    .lock()
                    .block_list()
                    .last_non_hidden_block()
                    .map(|b| b.id())
                    == Some(&trigger.executed_shell_command.id)
            }
            PassiveSuggestionTrigger::CommandRun | PassiveSuggestionTrigger::FilesChanged => false,
        }
    }

    fn handle_model_event(&mut self, event: &ModelEvent, ctx: &mut ModelContext<Self>) {
        match event {
            ModelEvent::AfterBlockStarted { .. } => {
                self.abort_pending_requests(ctx);
            }
            ModelEvent::AfterBlockCompleted(after_block_completed_event) => {
                if !FeatureFlag::PromptSuggestionsViaMAA.is_enabled() {
                    self.abort_pending_requests(ctx);
                    return;
                }
                if let BlockType::User(block_completed) = &after_block_completed_event.block_type {
                    if !block_completed.was_part_of_agent_interaction {
                        self.handle_user_block_completed(block_completed, ctx);
                    }
                }
            }
            _ => {}
        }
    }

    fn handle_controller_event(
        &mut self,
        event: &BlocklistAIControllerEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        match event {
            BlocklistAIControllerEvent::SentRequest { .. } => {
                // Once a non-passive request is sent, cancel any pending passive requests.
                self.abort_pending_requests(ctx);
            }
            BlocklistAIControllerEvent::FinishedReceivingOutput {
                conversation_id, ..
            } => {
                if !FeatureFlag::PromptSuggestionsViaMAA.is_enabled() {
                    self.abort_pending_requests(ctx);
                    return;
                }
                self.handle_finished_stream(*conversation_id, ctx);
            }
            _ => {}
        }
    }

    fn handle_finished_stream(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        self.abort_pending_requests(ctx);

        if !is_prompt_suggestions_enabled(ctx) {
            return;
        }

        // Suppress passive suggestions in cloud mode sessions.
        if self.is_ambient_agent_session(ctx) {
            return;
        }

        let history_model = BlocklistAIHistoryModel::as_ref(ctx);
        let Some(conversation) = history_model.conversation(&conversation_id) else {
            return;
        };
        let Some(latest_exchange) = conversation.latest_exchange() else {
            return;
        };
        let latest_exchange_id = latest_exchange.id;

        let status = conversation.status();
        if status.is_done() {
            self.send_request(
                Some(conversation_id),
                PassiveSuggestionTrigger::AgentResponseCompleted {
                    exchange_id: latest_exchange_id,
                },
                vec![warp_multi_agent_api::ToolType::SuggestPrompt],
                ctx,
            );
        }
    }

    fn send_shell_command_completed_request(
        &mut self,
        conversation_id: Option<AIConversationId>,
        block_context: Box<BlockContext>,
        relevant_files: Vec<FileContext>,
        supported_tools: Vec<warp_multi_agent_api::ToolType>,
        ctx: &mut ModelContext<Self>,
    ) {
        let trigger =
            PassiveSuggestionTrigger::ShellCommandCompleted(ShellCommandCompletedTrigger {
                executed_shell_command: block_context,
                relevant_files,
            });
        self.send_request(conversation_id, trigger, supported_tools, ctx);
    }

    fn handle_user_block_completed(
        &mut self,
        block_completed: &UserBlockCompleted,
        ctx: &mut ModelContext<Self>,
    ) {
        self.abort_pending_requests(ctx);
        // Suppress passive suggestions in cloud mode sessions.
        if self.is_ambient_agent_session(ctx) {
            return;
        }

        // Startup commands run while bootstrapping an Oz cloud environment, so we skip
        // passive prompt suggestion generation for them to avoid unnecessary requests.
        let is_oz_environment_startup_command = FeatureFlag::CloudModeSetupV2.is_enabled()
            && self
                .terminal_model
                .lock()
                .block_list()
                .block_at(block_completed.index)
                .is_some_and(|block| block.is_oz_environment_startup_command());
        if is_oz_environment_startup_command {
            return;
        }
        let is_prompt_suggestions_enabled = is_prompt_suggestions_enabled(ctx);
        let is_passive_code_diffs_enabled = is_passive_code_diffs_enabled(ctx);
        if !is_prompt_suggestions_enabled && !is_passive_code_diffs_enabled {
            return;
        }

        let mut supported_tools = Vec::new();
        if is_prompt_suggestions_enabled {
            supported_tools.push(warp_multi_agent_api::ToolType::SuggestPrompt);
        }

        let block_context = BlockContext::from_completed_block(block_completed);
        let (conversation_id, block_context) = {
            let model = self.terminal_model.lock();
            let Some(block) = model.block_list().block_at(block_completed.index) else {
                return;
            };

            let conversation_id = block.agent_view_visibility().agent_view_conversation_id();
            (conversation_id, block_context)
        };

        // If passive code diffs are enabled, check for any files that were read.
        #[cfg(feature = "local_fs")]
        if is_passive_code_diffs_enabled {
            if let Some(current_working_directory) = block_completed.serialized_block.pwd.clone() {
                let block_contents =
                    format!("{}\n{}", &block_context.command, &block_context.output);
                let shell = self.active_session.as_ref(ctx).shell_launch_data(ctx);
                let shell_for_detection = shell.clone();
                let current_working_directory_for_detection = current_working_directory.clone();
                let terminal_view_id = self.terminal_view_id;

                self.pending_file_read_handle = Some(ctx.spawn(
                    async move {
                        match tokio::task::spawn_blocking(move || {
                            detect_relevant_file_paths_for_block(
                                &block_contents,
                                &current_working_directory_for_detection,
                                shell_for_detection.as_ref(),
                            )
                        })
                        .await
                        {
                            Ok(paths) => paths,
                            Err(err) => {
                                log::warn!(
                                    "[passive-suggestions] failed to detect relevant file paths: {err}"
                                );
                                vec![]
                            }
                        }
                    },
                    move |me, candidate_paths, ctx| {
                        let Some(file_locations) = get_allowed_file_locations_for_paths(
                            candidate_paths,
                            conversation_id.as_ref(),
                            terminal_view_id,
                            ctx,
                        ) else {
                            me.pending_file_read_handle = None;
                            me.send_shell_command_completed_request(
                                conversation_id,
                                block_context,
                                vec![],
                                supported_tools,
                                ctx,
                            );
                            return;
                        };

                        me.pending_file_read_handle =
                            Some(ctx.spawn(read_files(file_locations, current_working_directory, shell), move |me, relevant_files, ctx| {
                                me.pending_file_read_handle = None;
                                supported_tools.push(warp_multi_agent_api::ToolType::ApplyFileDiffs);
                                me.send_shell_command_completed_request(
                                    conversation_id,
                                    block_context,
                                    relevant_files,
                                    supported_tools,
                                    ctx,
                                );
                            }));
                    },
                ));
                return;
            }
        }

        if !supported_tools.is_empty() {
            self.send_shell_command_completed_request(
                conversation_id,
                block_context,
                vec![],
                supported_tools,
                ctx,
            );
        }
    }
}

impl Entity for PassiveSuggestionsModel {
    type Event = PassiveSuggestionsEvent;
}

/// Result of extracting a suggestion from an out-of-band response stream.
struct StreamExtractionResult {
    suggestion: ExtractedSuggestion,
    /// The server-assigned request token from the `StreamInit` event,
    /// used to correlate client telemetry with server-side logs.
    server_request_token: Option<String>,
}

enum ExtractedSuggestion {
    Prompt {
        prompt: String,
        label: Option<String>,
        is_trigger_irrelevant: bool,
    },
    CodeDiff {
        apply_file_diffs: warp_multi_agent_api::message::tool_call::ApplyFileDiffs,
    },
}

/// Consumes the entire response stream, coalescing incremental message
/// updates (via field masks) into final messages, then inspects them
/// for `SuggestPrompt` or `ApplyFileDiffs` tool calls.
async fn extract_suggestion_from_stream(
    stream_result: Result<
        crate::ai::agent::api::ResponseStream,
        ai::agent::convert::ConvertToAPITypeError,
    >,
) -> Option<StreamExtractionResult> {
    use futures_util::StreamExt;
    use warp_multi_agent_api as api;

    use crate::ai::agent::task::helper::MessageExt;

    let Ok(mut stream) = stream_result else {
        return None;
    };

    // Drain the stream, collecting all client actions and the server token.
    let mut client_actions: Vec<api::ClientAction> = Vec::new();
    let mut server_request_token: Option<String> = None;
    while let Some(event) = stream.next().await {
        let Ok(response_event) = event else {
            continue;
        };
        match response_event.r#type {
            Some(api::response_event::Type::Init(init)) => {
                if !init.request_id.is_empty() {
                    server_request_token = Some(init.request_id);
                }
            }
            Some(api::response_event::Type::ClientActions(actions)) => {
                client_actions.extend(actions.actions);
            }
            _ => {}
        }
    }

    // Coalesce incremental message updates into final messages.
    let messages = coalesce_messages_from_client_actions(&client_actions);

    // Scan final messages for suggestion tool calls.
    for message in &messages {
        let Some(tool_call) = message.tool_call() else {
            continue;
        };
        let Some(tool) = tool_call.tool.as_ref() else {
            continue;
        };

        match tool {
            api::message::tool_call::Tool::SuggestPrompt(suggest_prompt) => {
                let Some(display_mode) = suggest_prompt.display_mode.as_ref() else {
                    continue;
                };
                if let api::message::tool_call::suggest_prompt::DisplayMode::PromptChip(chip) =
                    display_mode
                {
                    let label = if chip.label.is_empty() {
                        None
                    } else {
                        Some(chip.label.clone())
                    };
                    return Some(StreamExtractionResult {
                        suggestion: ExtractedSuggestion::Prompt {
                            prompt: chip.prompt.clone(),
                            label,
                            is_trigger_irrelevant: suggest_prompt.is_trigger_irrelevant,
                        },
                        server_request_token,
                    });
                }
            }
            api::message::tool_call::Tool::ApplyFileDiffs(apply_file_diffs) => {
                return Some(StreamExtractionResult {
                    suggestion: ExtractedSuggestion::CodeDiff {
                        apply_file_diffs: apply_file_diffs.clone(),
                    },
                    server_request_token,
                });
            }
            _ => {}
        }
    }
    None
}

/// Coalesces a sequence of client actions into final message state by applying
/// field-mask updates and appends incrementally.
fn coalesce_messages_from_client_actions(
    client_actions: &[warp_multi_agent_api::ClientAction],
) -> Vec<warp_multi_agent_api::Message> {
    use std::collections::HashMap;

    use field_mask::FieldMaskOperation;
    use warp_multi_agent_api as api;
    use warp_multi_agent_api::client_action::Action;

    let mut messages_by_id: HashMap<String, api::Message> = HashMap::new();
    let mut message_order: Vec<String> = Vec::new();

    for action in client_actions {
        match &action.action {
            Some(Action::CreateTask(create_task)) => {
                if let Some(task) = &create_task.task {
                    for message in &task.messages {
                        let id = message.id.clone();
                        if !messages_by_id.contains_key(&id) {
                            message_order.push(id.clone());
                        }
                        messages_by_id.insert(id, message.clone());
                    }
                }
            }
            Some(Action::AddMessagesToTask(add)) => {
                for message in &add.messages {
                    let id = message.id.clone();
                    if !messages_by_id.contains_key(&id) {
                        message_order.push(id.clone());
                    }
                    messages_by_id.insert(id, message.clone());
                }
            }
            Some(Action::UpdateTaskMessage(update)) => {
                if let Some(new_message) = &update.message {
                    let id = new_message.id.clone();
                    let mask = update.mask.clone().unwrap_or_default();
                    if let Some(existing) = messages_by_id.get_mut(&id) {
                        if let Ok(merged) = FieldMaskOperation::update(
                            &api::MESSAGE_DESCRIPTOR,
                            existing,
                            new_message,
                            mask,
                        )
                        .apply()
                        {
                            *existing = merged;
                        }
                    } else {
                        message_order.push(id.clone());
                        messages_by_id.insert(id, new_message.clone());
                    }
                }
            }
            Some(Action::AppendToMessageContent(append)) => {
                if let Some(new_message) = &append.message {
                    let id = new_message.id.clone();
                    let mask = append.mask.clone().unwrap_or_default();
                    if let Some(existing) = messages_by_id.get_mut(&id) {
                        if let Ok(merged) = FieldMaskOperation::append(
                            &api::MESSAGE_DESCRIPTOR,
                            existing,
                            new_message,
                            mask,
                        )
                        .apply()
                        {
                            *existing = merged;
                        }
                    } else {
                        message_order.push(id.clone());
                        messages_by_id.insert(id, new_message.clone());
                    }
                }
            }
            _ => {}
        }
    }

    message_order
        .into_iter()
        .filter_map(|id| messages_by_id.remove(&id))
        .collect()
}

/// Converts a slice of [`FileEdit`]s into [`PassiveCodeDiffEntry`] values,
/// preserving the original search/replace content from the LLM response.
fn file_edits_to_passive_diffs(file_edits: &[FileEdit]) -> Vec<PassiveCodeDiffEntry> {
    let mut entries = Vec::new();
    for edit in file_edits {
        let file_path = edit.file().unwrap_or_default().to_string();
        match edit {
            FileEdit::Edit(ParsedDiff::StrReplaceEdit {
                search, replace, ..
            }) => {
                entries.push(PassiveCodeDiffEntry {
                    file_path,
                    search: search.clone().unwrap_or_default(),
                    replace: replace.clone().unwrap_or_default(),
                });
            }
            FileEdit::Edit(ParsedDiff::V4AEdit { hunks, .. }) => {
                for hunk in hunks {
                    entries.push(PassiveCodeDiffEntry {
                        file_path: file_path.clone(),
                        search: hunk.old.clone(),
                        replace: hunk.new.clone(),
                    });
                }
            }
            FileEdit::Create { content, .. } => {
                entries.push(PassiveCodeDiffEntry {
                    file_path,
                    search: String::new(),
                    replace: content.clone().unwrap_or_default(),
                });
            }
            FileEdit::Delete { .. } => {
                entries.push(PassiveCodeDiffEntry {
                    file_path,
                    search: String::new(),
                    replace: String::new(),
                });
            }
        }
    }
    entries
}

fn classify_edit_format(file_edits: &[FileEdit]) -> RequestFileEditsFormatKind {
    let has_str_replace = file_edits
        .iter()
        .any(|e| matches!(e, FileEdit::Edit(ParsedDiff::StrReplaceEdit { .. })));
    let has_v4a = file_edits
        .iter()
        .any(|e| matches!(e, FileEdit::Edit(ParsedDiff::V4AEdit { .. })));
    match (has_str_replace, has_v4a) {
        (true, false) => RequestFileEditsFormatKind::StrReplace,
        (false, true) => RequestFileEditsFormatKind::V4A,
        (true, true) => RequestFileEditsFormatKind::Mixed,
        (false, false) => RequestFileEditsFormatKind::Unknown,
    }
}

fn is_passive_code_diffs_enabled(ctx: &ModelContext<PassiveSuggestionsModel>) -> bool {
    AISettings::as_ref(ctx).is_code_suggestions_enabled(ctx)
        && UserWorkspaces::as_ref(ctx).is_code_suggestions_toggleable()
}

fn is_prompt_suggestions_enabled(ctx: &ModelContext<PassiveSuggestionsModel>) -> bool {
    AISettings::as_ref(ctx).is_prompt_suggestions_enabled(ctx)
        && UserWorkspaces::as_ref(ctx).is_prompt_suggestions_toggleable()
}

#[cfg(feature = "local_fs")]
fn detect_relevant_file_paths_for_block(
    block_contents: &str,
    current_working_directory: &str,
    shell: Option<&ShellLaunchData>,
) -> Vec<PathBuf> {
    // TODO (suraj): use line num hint to limit the line range to read.
    detect_file_paths(current_working_directory, block_contents, shell)
        .into_values()
        .filter_map(|link| match link {
            DetectedLinkType::FilePath { absolute_path, .. } => Some(absolute_path),
            DetectedLinkType::Url(_) => None,
        })
        .filter(|path| path.is_file())
        .filter(|path| !is_binary_file(path))
        .unique()
        .collect()
}

#[cfg(feature = "local_fs")]
fn get_allowed_file_locations_for_paths(
    paths: Vec<PathBuf>,
    conversation_id: Option<&AIConversationId>,
    terminal_view_id: EntityId,
    ctx: &AppContext,
) -> Option<Vec<FileLocations>> {
    if paths.is_empty() {
        return None;
    }

    if !BlocklistAIPermissions::as_ref(ctx)
        .can_read_files(conversation_id, paths.clone(), Some(terminal_view_id), ctx)
        .is_allowed()
    {
        return None;
    }

    Some(
        paths
            .into_iter()
            .map(|path| FileLocations {
                name: path.to_string_lossy().to_string(),
                lines: vec![],
            })
            .collect_vec(),
    )
}

#[cfg(feature = "local_fs")]
async fn read_files(
    file_locations: Vec<FileLocations>,
    current_working_directory: String,
    shell: Option<ShellLaunchData>,
) -> Vec<FileContext> {
    let file_future = read_local_file_context(
        &file_locations,
        Some(current_working_directory),
        shell,
        None,
        // TODO (suraj): do something smarter than a single, fixed limit.
        Some(500000),
    );
    let Ok(result) = file_future.with_timeout(Duration::from_secs(2)).await else {
        return vec![];
    };

    match result {
        Ok(result) => result.file_contexts,
        Err(err) => {
            log::warn!("Failed to retrieve file content for suggest prompt relevant files: {err}");
            vec![]
        }
    }
}
