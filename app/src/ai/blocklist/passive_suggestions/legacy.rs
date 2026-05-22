use std::path::PathBuf;
#[cfg(not(target_family = "wasm"))]
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
#[cfg(not(target_family = "wasm"))]
use command::r#async::Command;
use parking_lot::FairMutex;
use serde_json::json;
use warp_core::features::FeatureFlag;
use warpui::r#async::{FutureExt as AsyncFutureExt, SpawnedFutureHandle, Timer};
use warpui::{Entity, EntityId, ModelContext, ModelHandle, SingletonEntity};

use super::static_prompt_suggestions::static_suggested_query;
#[cfg(not(target_family = "wasm"))]
use crate::ai::agent::PassiveSuggestionTrigger;
use crate::ai::agent::{AIAgentExchangeId, CancellationReason};
use crate::ai::blocklist::controller::response_stream::ResponseStreamId;
use crate::ai::blocklist::controller::{BlocklistAIController, BlocklistAIControllerEvent};
use crate::ai::blocklist::{
    read_local_file_context, BlocklistAIHistoryModel, BlocklistAIPermissions,
};
use crate::ai::paths::host_native_absolute_path;
use crate::ai::predict::generate_am_query_suggestions::{
    GenerateAMQuerySuggestionsRequest, GenerateAMQuerySuggestionsResponse, Suggestion,
};
use crate::ai_assistant::execution_context::WarpAiExecutionContext;
use crate::network::NetworkStatus;
use crate::report_error;
use crate::server::server_api::ServerApiProvider;
use crate::server::telemetry::PromptSuggestionFallbackReason;
use crate::settings::AISettings;
use crate::terminal::event::{BlockType, UserBlockCompleted};
use crate::terminal::model::block::BlockId;
use crate::terminal::model::session::active_session::ActiveSession;
use crate::terminal::model::session::SessionType;
use crate::terminal::model::terminal_model::TerminalModel;
use crate::terminal::model_events::{ModelEvent, ModelEventDispatcher};
use crate::terminal::view::{AgentModePromptSuggestion, PromptSuggestion};
use crate::workspaces::user_workspaces::UserWorkspaces;

const NUM_TOP_BLOCK_LINES: usize = 100;
const NUM_BOTTOM_BLOCK_LINES: usize = 200;
const PASSIVE_CODE_DIFF_LONG_FILE_LINE_LIMIT: usize = 2000;
const PASSIVE_CODE_DIFF_LONG_FILE_BYTE_LIMIT: usize = 100_000;
const PASSIVE_CODE_DIFF_TOTAL_LINE_LIMIT: usize = 2500;
const PASSIVE_CODE_DIFF_TOTAL_BYTE_LIMIT: usize = 150_000;
const PASSIVE_CODE_DIFF_FILE_READING_TIMEOUT: Duration = Duration::from_secs(2);
const PASSIVE_CODE_DIFF_AI_QUERY_TIMEOUT: Duration = Duration::from_secs(25);

#[derive(Clone, Debug)]
pub enum PassiveSuggestionsEvent {
    PromptSuggestionsGenerated {
        prompt_suggestion: AgentModePromptSuggestion,
        block_id: BlockId,
        command: String,
        request_duration_ms: u64,
    },
    PassiveCodeDiffRequestStarted {
        prompt_suggestion_id: String,
        code_exchange_id: Option<AIAgentExchangeId>,
        block_id: BlockId,
    },
    PassiveCodeDiffFailed {
        reason: PromptSuggestionFallbackReason,
    },
}

pub struct PassiveSuggestionsModel {
    active_session: ModelHandle<ActiveSession>,
    terminal_model: Arc<FairMutex<TerminalModel>>,
    ai_controller: ModelHandle<BlocklistAIController>,
    terminal_view_id: EntityId,
    prompt_suggestions_future_handle: Option<SpawnedFutureHandle>,
    unit_test_generation_future_handle: Option<SpawnedFutureHandle>,
    code_diff_preflight_future_handle: Option<SpawnedFutureHandle>,
    code_diff_timeout_future_handle: Option<SpawnedFutureHandle>,
    pending_unit_test_stream_id: Option<ResponseStreamId>,
    pending_code_diff_stream_id: Option<ResponseStreamId>,
}

impl PassiveSuggestionsModel {
    pub fn new(
        active_session: ModelHandle<ActiveSession>,
        terminal_model: Arc<FairMutex<TerminalModel>>,
        ai_controller: ModelHandle<BlocklistAIController>,
        model_event_dispatcher: &ModelHandle<ModelEventDispatcher>,
        terminal_view_id: EntityId,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        ctx.subscribe_to_model(model_event_dispatcher, |me, event, ctx| {
            me.handle_model_event(event, ctx);
        });
        ctx.subscribe_to_model(&ai_controller, |me, event, _ctx| {
            me.handle_controller_event(event, _ctx);
        });

        Self {
            active_session,
            terminal_model,
            ai_controller,
            terminal_view_id,
            prompt_suggestions_future_handle: None,
            unit_test_generation_future_handle: None,
            code_diff_preflight_future_handle: None,
            code_diff_timeout_future_handle: None,
            pending_unit_test_stream_id: None,
            pending_code_diff_stream_id: None,
        }
    }

    pub fn is_passive_code_diff_being_generated(&self) -> bool {
        self.pending_code_diff_stream_id.is_some()
    }

    pub fn abort_pending_requests(
        &mut self,
        ctx: &mut ModelContext<Self>,
    ) -> Vec<ResponseStreamId> {
        let mut aborted_stream_ids = Vec::new();
        if let Some(handle) = self.prompt_suggestions_future_handle.take() {
            handle.abort();
        }
        if let Some(handle) = self.unit_test_generation_future_handle.take() {
            handle.abort();
        }
        if let Some(handle) = self.code_diff_preflight_future_handle.take() {
            handle.abort();
        }
        if let Some(handle) = self.code_diff_timeout_future_handle.take() {
            handle.abort();
        }
        if let Some(stream_id) = self.pending_unit_test_stream_id.take() {
            self.ai_controller.update(ctx, |controller, ctx| {
                controller.try_cancel_pending_response_stream(
                    &stream_id,
                    CancellationReason::ManuallyCancelled,
                    ctx,
                );
            });
            aborted_stream_ids.push(stream_id);
        }
        if let Some(stream_id) = self.pending_code_diff_stream_id.take() {
            self.ai_controller.update(ctx, |controller, ctx| {
                controller.try_cancel_pending_response_stream(
                    &stream_id,
                    CancellationReason::ManuallyCancelled,
                    ctx,
                );
            });
            aborted_stream_ids.push(stream_id);
        }
        aborted_stream_ids
    }

    fn handle_model_event(&mut self, event: &ModelEvent, ctx: &mut ModelContext<Self>) {
        match event {
            ModelEvent::AfterBlockStarted { .. } => {
                self.abort_pending_requests(ctx);
            }
            ModelEvent::AfterBlockCompleted(after_block_completed_event) => {
                if FeatureFlag::PromptSuggestionsViaMAA.is_enabled() {
                    self.abort_pending_requests(ctx);
                    return;
                }
                let BlockType::User(block_completed) = &after_block_completed_event.block_type
                else {
                    return;
                };
                self.handle_user_block_completed(block_completed, ctx);
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
            BlocklistAIControllerEvent::FinishedReceivingOutput { stream_id, .. } => {
                if self
                    .pending_unit_test_stream_id
                    .as_ref()
                    .is_some_and(|pending| pending == stream_id)
                {
                    self.pending_unit_test_stream_id = None;
                }
                if self
                    .pending_code_diff_stream_id
                    .as_ref()
                    .is_some_and(|pending| pending == stream_id)
                {
                    self.pending_code_diff_stream_id = None;
                    if let Some(handle) = self.code_diff_timeout_future_handle.take() {
                        handle.abort();
                    }
                }
            }
            BlocklistAIControllerEvent::SentRequest { stream_id, .. } => {
                if self.pending_unit_test_stream_id.as_ref() == Some(stream_id)
                    || self.pending_code_diff_stream_id.as_ref() == Some(stream_id)
                    || self.pending_code_diff_stream_id.as_ref() == Some(stream_id)
                {
                    return;
                }
                // If a new request is sent that wasn't for passive suggestions, abort any passive suggestions requests.
                self.abort_pending_requests(ctx);
            }
            _ => {}
        }
    }

    fn handle_user_block_completed(
        &mut self,
        block_completed: &UserBlockCompleted,
        ctx: &mut ModelContext<Self>,
    ) {
        if block_completed.was_part_of_agent_interaction {
            return;
        }

        self.abort_pending_requests(ctx);

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

        if should_generate_unit_test_suggestion(block_completed, ctx) {
            self.generate_unit_test_suggestion(block_completed.clone(), ctx);
        } else if should_generate_prompt_suggestions(block_completed, ctx) {
            self.generate_prompt_suggestions(block_completed.clone(), ctx);
        }
    }

    fn generate_prompt_suggestions(
        &mut self,
        block_completed: UserBlockCompleted,
        ctx: &mut ModelContext<Self>,
    ) {
        let block_id = block_completed.serialized_block.id.clone();
        let command = block_completed.command.clone();
        let start_ts_ms = Utc::now().timestamp_millis();

        if let Some(suggestion) = fetch_static_prompt_suggestion(&block_completed) {
            ctx.emit(PassiveSuggestionsEvent::PromptSuggestionsGenerated {
                prompt_suggestion: suggestion.clone(),
                block_id: block_id.clone(),
                command,
                request_duration_ms: 0,
            });
            self.maybe_generate_passive_code_diff(suggestion, block_id, ctx);
            return;
        }

        let Some(execution_context) = self
            .active_session
            .as_ref(ctx)
            .ai_execution_environment(ctx)
        else {
            return;
        };
        let Some(request) = build_prompt_suggestions_request(
            &block_completed,
            execution_context,
            &self.terminal_model,
        ) else {
            return;
        };

        let server_api = ServerApiProvider::handle(ctx).as_ref(ctx).get();
        let request_future =
            async move { server_api.generate_am_query_suggestions(&request).await };

        self.prompt_suggestions_future_handle =
            Some(ctx.spawn(request_future, move |me, result, ctx| {
                me.prompt_suggestions_future_handle = None;
                let end_ts_ms = Utc::now().timestamp_millis();
                let request_duration_ms = end_ts_ms.saturating_sub(start_ts_ms) as u64;
                let prompt_suggestion = match result {
                    Ok(response) => map_prompt_suggestions_response(response),
                    Err(err) => {
                        report_error!(
                            anyhow::Error::new(err).context("Failed to fetch prompt suggestions")
                        );
                        AgentModePromptSuggestion::Error
                    }
                };

                ctx.emit(PassiveSuggestionsEvent::PromptSuggestionsGenerated {
                    prompt_suggestion: prompt_suggestion.clone(),
                    block_id: block_id.clone(),
                    command,
                    request_duration_ms,
                });
                me.maybe_generate_passive_code_diff(prompt_suggestion, block_id, ctx);
            }));
    }

    fn generate_unit_test_suggestion(
        &mut self,
        block_completed: UserBlockCompleted,
        ctx: &mut ModelContext<Self>,
    ) {
        #[cfg(target_family = "wasm")]
        {
            let (_, _) = (block_completed, ctx);
        }

        #[cfg(not(target_family = "wasm"))]
        {
            let Some(current_dir) = block_completed
                .serialized_block
                .pwd
                .as_ref()
                .map(PathBuf::from)
            else {
                return;
            };

            self.unit_test_generation_future_handle = Some(ctx.spawn(
                async move {
                    let output = Command::new("git")
                        .args(["show", "HEAD"])
                        .current_dir(current_dir)
                        .stdout(Stdio::piped())
                        .output()
                        .await;
                    if let Ok(output) = output {
                        return String::from_utf8_lossy(&output.stdout).to_string();
                    }
                    String::new()
                },
                |me, diff_output: String, ctx| {
                    me.unit_test_generation_future_handle = None;
                    if diff_output.is_empty() {
                        return;
                    }
                    let diff_json = json!({ "diffs": diff_output });
                    let request = me.ai_controller.update(ctx, |controller, ctx| {
                        controller.send_unit_test_suggestions_request(
                            diff_json.to_string(),
                            PassiveSuggestionTrigger::CommandRun,
                            ctx,
                        )
                    });
                    if let Ok((_, stream_id)) = request {
                        me.pending_unit_test_stream_id = Some(stream_id.clone());
                    }
                },
            ));
        }
    }

    fn maybe_generate_passive_code_diff(
        &mut self,
        prompt_suggestion: AgentModePromptSuggestion,
        block_id: BlockId,
        ctx: &mut ModelContext<Self>,
    ) {
        if !passive_code_diffs_enabled(ctx) {
            return;
        }
        let AgentModePromptSuggestion::Success(query) = prompt_suggestion else {
            return;
        };
        let Some(files) = query.coding_query_context.clone() else {
            return;
        };

        let current_working_directory = self
            .active_session
            .as_ref(ctx)
            .current_working_directory()
            .cloned();
        let shell = self.active_session.as_ref(ctx).shell_launch_data(ctx);

        let can_read_file = BlocklistAIPermissions::as_ref(ctx)
            .can_read_files(
                None,
                files
                    .iter()
                    .map(|file| {
                        PathBuf::from(host_native_absolute_path(
                            &file.name,
                            &shell,
                            &current_working_directory,
                        ))
                    })
                    .collect(),
                Some(self.terminal_view_id),
                ctx,
            )
            .is_allowed();
        let should_skip_for_remote = self
            .active_session
            .as_ref(ctx)
            .session_type(ctx)
            .map(|session_type| matches!(session_type, SessionType::WarpifiedRemote { .. }))
            .unwrap_or(true);
        if !can_read_file || should_skip_for_remote {
            let reason = if !can_read_file {
                PromptSuggestionFallbackReason::NoReadFilesPermission
            } else {
                PromptSuggestionFallbackReason::SSHRemoteSession
            };
            ctx.emit(PassiveSuggestionsEvent::PassiveCodeDiffFailed { reason });
            return;
        }

        let prompt_suggestion_id = query.id;
        let query_text = query.prompt;
        self.code_diff_preflight_future_handle = Some(ctx.spawn(
            async move {
                let file_future =
                    read_local_file_context(&files, current_working_directory, shell, None, None);
                let Ok(result) = file_future.with_timeout(PASSIVE_CODE_DIFF_FILE_READING_TIMEOUT).await
                else {
                    return Err(anyhow::anyhow!("File reading timed out"));
                };
                result
            },
            move |me, content, ctx| {
                me.code_diff_preflight_future_handle = None;

                let content = match content {
                    Ok(content) => {
                        if !content.missing_files.is_empty() {
                            log::warn!(
                                "Missing files when retrieving file content for suggested code diffs: {:?}",
                                content.missing_files
                            );
                            ctx.emit(PassiveSuggestionsEvent::PassiveCodeDiffFailed {
                                reason: PromptSuggestionFallbackReason::MissingFile,
                            });
                            return;
                        }
                        content
                    }
                    Err(err) => {
                        log::warn!("Failed to retrieve file content for suggested code diffs: {err}");
                        ctx.emit(PassiveSuggestionsEvent::PassiveCodeDiffFailed {
                            reason: PromptSuggestionFallbackReason::FailedToRetrieveFile,
                        });
                        return;
                    }
                };

                let mut total_lines = 0;
                let mut total_bytes = 0;
                let has_large_file = content.file_contexts.iter().any(|file_context| {
                    let file_content = &file_context.content;
                    let line_count = file_content.line_count();
                    let byte_count = file_content.len();

                    total_lines += line_count;
                    total_bytes += byte_count;

                    line_count >= PASSIVE_CODE_DIFF_LONG_FILE_LINE_LIMIT
                        || byte_count >= PASSIVE_CODE_DIFF_LONG_FILE_BYTE_LIMIT
                });
                if has_large_file
                    || total_lines >= PASSIVE_CODE_DIFF_TOTAL_LINE_LIMIT
                    || total_bytes >= PASSIVE_CODE_DIFF_TOTAL_BYTE_LIMIT
                {
                    let reason =
                        if has_large_file && total_lines >= PASSIVE_CODE_DIFF_LONG_FILE_LINE_LIMIT {
                            PromptSuggestionFallbackReason::FileTooManyLines
                        } else {
                            PromptSuggestionFallbackReason::FileTooManyBytes
                        };
                    ctx.emit(PassiveSuggestionsEvent::PassiveCodeDiffFailed {
                        reason,
                    });
                    return;
                }

                let result = me.ai_controller.update(ctx, |controller, ctx| {
                    controller.send_passive_code_diff_request(
                        query_text,
                        &block_id,
                        content.file_contexts,
                        ctx,
                    )
                });

                match result {
                    Ok((conversation_id, stream_id)) => {
                        me.pending_code_diff_stream_id = Some(stream_id.clone());
                        let code_exchange_id = BlocklistAIHistoryModel::as_ref(ctx)
                            .conversation(&conversation_id)
                            .and_then(|conversation| conversation.root_task_exchanges().next())
                            .map(|exchange| exchange.id);
                        ctx.emit(PassiveSuggestionsEvent::PassiveCodeDiffRequestStarted {
                            prompt_suggestion_id: prompt_suggestion_id.clone(),
                            code_exchange_id,
                            block_id: block_id.clone(),
                        });
                        me.start_code_diff_timeout(stream_id, ctx);
                    }
                    Err(_) => {
                        ctx.emit(PassiveSuggestionsEvent::PassiveCodeDiffFailed {
                            reason: PromptSuggestionFallbackReason::FailedToSendAIRequest,
                        });
                    }
                }
            },
        ));
    }

    fn start_code_diff_timeout(
        &mut self,
        stream_id: ResponseStreamId,
        ctx: &mut ModelContext<Self>,
    ) {
        if let Some(handle) = self.code_diff_timeout_future_handle.take() {
            handle.abort();
        }
        self.code_diff_timeout_future_handle = Some(ctx.spawn(
            async move {
                Timer::after(PASSIVE_CODE_DIFF_AI_QUERY_TIMEOUT).await;
                stream_id
            },
            |me, timed_out_stream_id, ctx| {
                me.code_diff_timeout_future_handle = None;
                if me
                    .pending_code_diff_stream_id
                    .as_ref()
                    .is_some_and(|pending| pending == &timed_out_stream_id)
                {
                    log::warn!(
                        "Passive code diff AI request timed out, cancelling stream {timed_out_stream_id:?}"
                    );
                    me.ai_controller.update(ctx, |controller, ctx| {
                        controller.try_cancel_pending_response_stream(
                            &timed_out_stream_id,
                            CancellationReason::ManuallyCancelled,
                            ctx,
                        );
                    });
                    me.pending_code_diff_stream_id = None;
                    ctx.emit(PassiveSuggestionsEvent::PassiveCodeDiffFailed {
                        reason: PromptSuggestionFallbackReason::AIQueryTimeout,
                    });
                }
            },
        ));
    }
}

impl Entity for PassiveSuggestionsModel {
    type Event = PassiveSuggestionsEvent;
}

fn should_generate_prompt_suggestions(
    block_completed: &UserBlockCompleted,
    ctx: &ModelContext<PassiveSuggestionsModel>,
) -> bool {
    if block_completed.command.trim().is_empty() {
        return false;
    }
    if !NetworkStatus::as_ref(ctx).is_online() {
        return false;
    }

    AISettings::as_ref(ctx).is_prompt_suggestions_enabled(ctx)
        && UserWorkspaces::as_ref(ctx).is_prompt_suggestions_toggleable()
}

fn should_generate_unit_test_suggestion(
    block_completed: &UserBlockCompleted,
    ctx: &ModelContext<PassiveSuggestionsModel>,
) -> bool {
    let enabled = AISettings::as_ref(ctx).is_code_suggestions_enabled(ctx)
        && UserWorkspaces::as_ref(ctx).is_code_suggestions_toggleable();

    enabled
        && block_completed.command.starts_with("git")
        && block_completed.command.contains("commit")
        && block_completed.serialized_block.exit_code.was_successful()
}

fn passive_code_diffs_enabled(ctx: &ModelContext<PassiveSuggestionsModel>) -> bool {
    let ai_settings = AISettings::as_ref(ctx);
    let is_prompt_suggestions_enabled = ai_settings.is_prompt_suggestions_enabled(ctx);
    let is_code_suggestions_enabled = ai_settings.is_code_suggestions_enabled(ctx);
    let is_toggleable = UserWorkspaces::as_ref(ctx).is_code_suggestions_toggleable();
    is_prompt_suggestions_enabled && is_code_suggestions_enabled && is_toggleable
}

fn fetch_static_prompt_suggestion(block: &UserBlockCompleted) -> Option<AgentModePromptSuggestion> {
    if !block.serialized_block.exit_code.was_successful() {
        return None;
    }
    static_suggested_query(&block.command).map(AgentModePromptSuggestion::Success)
}

fn build_prompt_suggestions_request(
    block: &UserBlockCompleted,
    execution_context: WarpAiExecutionContext,
    terminal_model: &Arc<FairMutex<TerminalModel>>,
) -> Option<GenerateAMQuerySuggestionsRequest> {
    let exit_code = block.serialized_block.exit_code;
    let working_dir = block.serialized_block.pwd.as_ref();
    let (processed_input, processed_output) = {
        let model = terminal_model.lock();
        let terminal_width = model.block_list().size().columns();
        let Some(current_block) = model.block_list().block_with_id(&block.serialized_block.id)
        else {
            log::error!(
                "Failed to fetch prompt suggestions, could not find block with ID: {:?}",
                block.serialized_block.id
            );
            return None;
        };
        current_block.get_block_content_summary(
            terminal_width,
            NUM_TOP_BLOCK_LINES,
            NUM_BOTTOM_BLOCK_LINES,
        )
    };

    let json_message = json!({
        "command": processed_input,
        "output": processed_output,
        "exit_code": exit_code,
        "pwd": working_dir,
    });
    Some(GenerateAMQuerySuggestionsRequest {
        context_messages: vec![json_message.to_string()],
        system_context: execution_context.to_json_string(),
        exit_code: exit_code.value(),
    })
}

fn map_prompt_suggestions_response(
    response: GenerateAMQuerySuggestionsResponse,
) -> AgentModePromptSuggestion {
    let is_valid_code_delegation = response.is_valid_code_delegation();
    let Some(suggestion) = response.suggestion else {
        return AgentModePromptSuggestion::None;
    };

    match suggestion {
        Suggestion::Coding(coding_query) if is_valid_code_delegation => {
            AgentModePromptSuggestion::Success(PromptSuggestion {
                id: response.id,
                label: None,
                prompt: coding_query.query,
                coding_query_context: Some(
                    coding_query
                        .files
                        .into_iter()
                        .map(Into::into)
                        .collect::<Vec<_>>(),
                ),
                static_prompt_suggestion_name: None,
                should_start_new_conversation: true,
            })
        }
        Suggestion::Simple(simple_query) => AgentModePromptSuggestion::Success(PromptSuggestion {
            id: response.id,
            label: None,
            prompt: simple_query.query,
            coding_query_context: None,
            static_prompt_suggestion_name: None,
            should_start_new_conversation: true,
        }),
        _ => AgentModePromptSuggestion::None,
    }
}
