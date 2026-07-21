//! Public app APIs used by the `warp_tui` frontend.

pub use ::ai::agent::action::{AskUserQuestionItem, AskUserQuestionOption, AskUserQuestionType};
pub use ::ai::agent::action_result::AskUserQuestionAnswerItem;
pub use ::ai::agent::{
    AskUserQuestionAction, AskUserQuestionEffect, AskUserQuestionPhase, AskUserQuestionSession,
    QuestionDraft,
};
pub use ai::agent::action::{RunAgentsAgentRunConfig, RunAgentsExecutionMode, RunAgentsRequest};
pub use ai::agent::orchestration_config::{OrchestrationConfig, OrchestrationConfigStatus};
pub use repo_metadata::repositories::RepoDetectionSource;
pub use warp_cli::agent::Harness;
use warp_completer::completer::{CompletionContext as _, TopLevelCommandCaseSensitivity};
use warp_completer::signatures::CommandRegistry;
use warpui::SingletonEntity as _;

pub use crate::ai::agent::api::ServerConversationToken;
pub use crate::ai::agent::conversation::{
    AIConversation, AIConversationAutoexecuteMode, AIConversationId, ConversationStatus,
    ConversationUsageTotals, TodoStatus,
};
pub use crate::ai::agent::task::TaskId;
pub use crate::ai::agent::todos::AIAgentTodoList;
pub use crate::ai::agent::{
    AIAgentAction, AIAgentActionId, AIAgentActionResult, AIAgentActionResultType,
    AIAgentActionType, AIAgentContext, AIAgentExchangeId, AIAgentInput, AIAgentOutput,
    AIAgentOutputMessage, AIAgentOutputMessageType, AIAgentPtyWriteMode, AIAgentText,
    AIAgentTextSection, AIAgentTodo, AIAgentTodoId, AgentOutputImage, AgentOutputImageLayout,
    AgentOutputMermaidDiagram, AgentOutputTable, AskUserQuestionResult, CancellationReason,
    FileGlobV2Result, GrepResult, ImageContext, MessageId, ReceivedMessageDisplay,
    RenderableAIError, RequestCommandOutputResult, RunAgentsAgentOutcomeKind, RunAgentsResult,
    SearchCodebaseFailureReason, SearchCodebaseResult, ServerOutputId, Shared, ShellCommandDelay,
    StartAgentExecutionMode, SuggestNewConversationResult, SummarizationType, TodoOperation,
    UserQueryMode,
};
pub use crate::ai::agent_conversations_model::{
    AgentConversationEntry, AgentConversationEntryId, AgentConversationListEntryState,
    AgentConversationListPolicy, AgentConversationsModel, AgentConversationsModelEvent,
    AgentManagementFilters, AgentRunDisplayStatus, HarnessFilter, OwnerFilter,
    query_conversation_entries,
};
pub use crate::ai::ambient_agents::AmbientAgentTaskId;
pub use crate::ai::blocklist::agent_view::{
    AgentViewController, AgentViewDisplayMode, AgentViewEntryOrigin, EnterAgentViewError,
    EphemeralMessageModel,
};
pub use crate::ai::blocklist::block::cli_controller::{
    CLISubagentController, CLISubagentEvent, CLISubagentTarget, LongRunningCommandControlState,
    UserTakeOverReason,
};
pub use crate::ai::blocklist::block::model::{
    AIBlockModel, AIBlockModelHelper, AIBlockModelImpl, AIBlockOutputStatus, AIRequestType,
    OutputStatusUpdateCallback,
};
pub use crate::ai::blocklist::conversation_selection::{
    ConversationSelection, ConversationSelectionEvent, ConversationSelectionHandle,
    PendingQueryState,
};
pub use crate::ai::blocklist::diff_storage::{
    DiffStorage, DiffStorageHelper, FileSnapshot, RegisteredDiffStorage, SaveFuture,
    UpdatedFileState,
};
pub use crate::ai::blocklist::diff_types::{DiffSessionType, FileDiff, changed_lines_from_op};
pub use crate::ai::blocklist::history_model::{
    BlocklistAIHistoryEvent, BlocklistAIHistoryModel, CloudConversationData,
    ConversationStatusUpdate,
};
pub use crate::ai::blocklist::orchestration_event_streamer::{
    register_agent_event_consumer, unregister_agent_event_consumer,
};
pub use crate::ai::blocklist::orchestration_topology::{
    OrchestrationParticipantKind, OrderedOrchestrationDescendant, ResolvedOrchestrationParticipant,
    descendant_conversation_ids_in_spawn_order, descendant_conversations_in_pill_order,
    orchestration_root_conversation_id, orchestrator_agent_id_for_conversation,
    resolve_orchestration_participant,
};
pub use crate::ai::blocklist::view_util::{
    FailedOutputPresentation, failed_output_presentation, format_credits,
    should_show_failed_output_usage_notice,
};
pub use crate::ai::blocklist::{
    AIActionStatus, AskUserQuestionExecutor, AttachmentType, BlocklistAIActionEvent,
    BlocklistAIActionModel, BlocklistAIContextEvent, BlocklistAIContextModel,
    BlocklistAIController, BlocklistAIInputModel, InputConfig, InputModePolicy,
    InputModePolicyHandle, InputType, InputTypeAutoDetectionSource, NewConversationDecision,
    PendingAttachmentSummary, PolicyConfigUpdate, RequestFileEditsExecutor, RunAgentsExecutor,
    RunAgentsExecutorEvent, RunAgentsSpawningSnapshot, ShellCommandExecutor,
    ShellCommandExecutorEvent, StartAgentExecutor, StartAgentExecutorEvent, StartAgentOutcome,
    StartAgentRequest, StartAgentRequestId, block_context_from_terminal_model,
    inherit_child_agent_settings, maybe_build_ai_query_upsert_event,
};
#[cfg(not(target_family = "wasm"))]
pub use crate::ai::blocklist::{
    PreparedLocalOzChildLaunch, apply_child_agent_model_override, prepare_local_oz_child_launch,
};
pub use crate::ai::connected_self_hosted_workers::{
    ConnectedSelfHostedWorkersEvent, ConnectedSelfHostedWorkersModel,
};
#[cfg(feature = "local_fs")]
pub use crate::ai::conversation_export::{
    ConversationFileExport, ConversationFileExportError, export_conversation_markdown,
};
pub use crate::ai::get_relevant_files::controller::GetRelevantFilesController;
pub use crate::ai::harness_availability::{
    AuthSecretEntry, AuthSecretFetchState, HarnessAvailability, HarnessAvailabilityEvent,
    HarnessAvailabilityModel, HarnessModelInfo,
};
pub use crate::ai::llms::{LLMId, LLMInfo, LLMPreferences, LLMPreferencesEvent};
pub use crate::ai::orchestration::{
    AuthSecretSelection, ORCHESTRATION_ENV_NONE_LABEL, ORCHESTRATION_WARP_WORKER_HOST, OptionBadge,
    OptionFooter, OptionRow, OptionSnapshot, OptionSourceStatus, OrchestrationConfigState,
    OrchestrationEditState, accept_disabled_reason_with_auth, api_key_snapshot,
    auth_secret_selection_required, empty_env_recommendation_message, environment_snapshot,
    harness_is_selectable, harness_snapshot, host_snapshot, location_snapshot, model_snapshot,
    persist_environment_selection, persist_host_selection,
    resolve_auth_secret_selection_for_harness, resolve_default_environment_id,
    resolve_default_host_slug, should_show_auth_secret_picker,
};
pub use crate::ai::skills::{SkillManager, SkillReference};
pub use crate::appearance::Appearance;
pub use crate::banner::BannerState;
pub use crate::changelog_model::{
    ChangelogModel, ChangelogRequestType, ChangelogState, Event as ChangelogModelEvent,
};
pub use crate::code::DiffResult;
pub use crate::code_review::git_repo_model::{
    GitRepoModels, GitRepoStatusModel, GitStatusMetadata,
};
pub use crate::completer::SessionContext;
pub use crate::persistence::PersistenceWriter;
pub use crate::search::slash_command_menu::static_commands::commands::{
    self as slash_commands, COMMAND_REGISTRY,
};
pub use crate::search::slash_command_menu::{SlashCommandId, StaticCommand};
pub use crate::settings::AISettingsChangedEvent;
pub use crate::terminal::alt_screen::{should_intercept_mouse, should_intercept_scroll};
pub use crate::terminal::color::{Colors as TerminalColors, List as TerminalColorList};
pub use crate::terminal::conversation_restoration::{
    ConversationBlockRestorationPlan, RestoredConversationExchange,
    prepare_conversation_block_restoration,
};
pub use crate::terminal::event::AfterBlockCompletedEvent;
pub use crate::terminal::input::CommandExecutionSource;
pub use crate::terminal::input::decorations::parse_current_commands_and_tokens;
pub use crate::terminal::input::models::{ModelPickerChoice, query_model_picker_choices};
pub use crate::terminal::input::skills::{
    AcceptSkill, LOCAL_SKILLS_REMOTE_EXECUTION_ERROR_MESSAGE, SelectableSkill,
    query_selectable_skills,
};
pub use crate::terminal::input::slash_command_model::{
    DetectedCommand, DetectedSkillCommand, ParsedSlashCommandInput,
    slash_command_composition_filter,
};
pub use crate::terminal::input::slash_commands::{
    AcceptSlashCommandOrSavedPrompt, InlineItem, SlashCommandDataSource, SlashCommandMixer,
    SlashCommandSelectionBehavior, TuiDataSourceArgs as TuiSlashCommandDataSourceArgs,
    TuiSlashCommand, TuiSlashCommandDataSource, TuiZeroStateDataSource, UpdatedActiveCommands,
    build_slash_command_mixer, record_saved_prompt_accepted, record_static_slash_command_accepted,
    saved_prompt_text_for_id, should_close_slash_command_menu_for_exact_match,
    slash_command_is_submitted_as_prompt, slash_command_is_supported_in_tui, slash_command_query,
    slash_command_selection_behavior,
};
pub use crate::terminal::local_tty::{
    TerminalManager as LocalTtyTerminalManager, TerminalManagerInit, TerminalSurfaceInit,
    TerminalSurfaceResult,
};
pub use crate::terminal::model::block::{
    AgentInteractionMetadata, Block, BlockId, TranscriptScope,
};
pub use crate::terminal::model::blockgrid::BlockGrid;
pub use crate::terminal::model::blocks::{
    BlockHeight, BlockHeightItem, BlockHeightSummary, BlockList, RichContentItem, TotalIndex,
};
pub use crate::terminal::model::escape_sequences::{KeystrokeWithDetails, ToEscapeSequence};
pub use crate::terminal::model::grid::grid_handler::{GridHandler, TermMode};
pub use crate::terminal::model::rich_content::RichContentType;
pub use crate::terminal::model::session::Sessions;
pub use crate::terminal::model::session::active_session::{ActiveSession, ActiveSessionEvent};
pub use crate::terminal::model::terminal_model::BlockIndex;
pub use crate::terminal::model_events::{ModelEvent, ModelEventDispatcher};
pub use crate::terminal::shared_session::IsSharedSessionCreator;
pub use crate::terminal::terminal_manager::BlockSpacing;
pub use crate::terminal::view::blocklist_filter::should_show_task_in_blocklist;
pub use crate::terminal::view::{ExecuteCommandEvent, WAKEUP_THROTTLE_PERIOD};
pub use crate::terminal::{
    BlockPadding, PtyIntent, PtyIntentEvent, ShellLaunchData, SizeInfo, SizeUpdate,
    TerminalManager as TerminalManagerTrait, TerminalModel, TerminalSurface,
};
pub use crate::themes::default_themes::{dark_theme, light_theme};
pub use crate::throttle::throttle;
pub use crate::tui::{
    TuiMcpAction, TuiMcpConfigState, TuiMcpManager, TuiMcpManagerEvent, TuiMcpServerId,
    TuiMcpServerSnapshot, TuiMcpServerStatus, TuiMcpSnapshot, TuiMcpTransport,
};
#[cfg(any(test, feature = "test-util"))]
pub use crate::tui_test_support::{
    queue_tui_permission_action, register_tui_session_view_test_singletons,
};
pub use crate::util::image::{
    MAX_IMAGE_COUNT_FOR_QUERY, MAX_IMAGE_SIZE_BYTES, MIME_SNIFF_BYTES, ProcessImageResult,
    infer_mime_type, is_supported_image_mime_type, process_image_for_agent,
};
pub use crate::util::repo_detection::{RepoDetectionSessionType, detect_possible_git_repo};
pub use crate::util::time_format::format_elapsed_seconds;

/// Builds the live-shell completion context used to parse TUI input for NLD.
pub fn tui_completion_session_context(
    active_session: &ActiveSession,
    current_working_directory: String,
    app: &warpui::AppContext,
) -> Option<SessionContext> {
    let session = active_session.session(app)?;
    let current_working_directory =
        session.convert_directory_to_typed_path_buf(current_working_directory);
    Some(SessionContext::new(
        session,
        CommandRegistry::global_instance(),
        current_working_directory,
        app,
    ))
}

/// Returns whether `command` exactly matches a top-level command available in
/// the TUI's live shell completion context.
pub fn tui_completion_context_has_exact_command(
    completion_context: &SessionContext,
    command: &str,
) -> bool {
    let case_sensitivity = completion_context.command_case_sensitivity();
    let is_live_shell_command =
        completion_context
            .top_level_commands()
            .any(|candidate| match case_sensitivity {
                TopLevelCommandCaseSensitivity::CaseSensitive => candidate == command,
                TopLevelCommandCaseSensitivity::CaseInsensitive => {
                    candidate.eq_ignore_ascii_case(command)
                }
            });
    if is_live_shell_command {
        return true;
    }

    #[cfg(feature = "completions_v2")]
    {
        completion_context
            .command_registry()
            .get_signature(command)
            .is_some()
    }
    #[cfg(not(feature = "completions_v2"))]
    {
        completion_context
            .command_registry()
            .signature_from_line(command, case_sensitivity)
            .is_some()
    }
}

/// Returns whether cloud conversation metadata failed to load.
pub fn agent_conversations_cloud_metadata_load_failed(app: &warpui::AppContext) -> bool {
    crate::ai::agent_conversations_model::AgentConversationsModel::as_ref(app)
        .cloud_conversation_metadata_load_failed()
}
