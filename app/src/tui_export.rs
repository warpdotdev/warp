//! Public app APIs used by the `warp_tui` frontend.

pub use ai::agent::action::{RunAgentsAgentRunConfig, RunAgentsExecutionMode, RunAgentsRequest};
pub use ai::agent::orchestration_config::{OrchestrationConfig, OrchestrationConfigStatus};
#[cfg(any(test, feature = "test-util"))]
use ai::api_keys::ApiKeyManager;
#[cfg(any(test, feature = "test-util"))]
use ai::index::full_source_code_embedding::manager::CodebaseIndexManager;
pub use repo_metadata::repositories::RepoDetectionSource;
pub use warp_cli::agent::Harness;
use warp_completer::completer::{CompletionContext as _, TopLevelCommandCaseSensitivity};
use warp_completer::signatures::CommandRegistry;
#[cfg(any(test, feature = "test-util"))]
use warp_core::execution_mode::{AppExecutionMode, ExecutionMode};
use warpui::SingletonEntity as _;

#[cfg(any(test, feature = "test-util"))]
use crate::ai::active_agent_views_model::ActiveAgentViewsModel;
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
    FileGlobV2Result, GrepResult, MessageId, RequestCommandOutputResult, RunAgentsAgentOutcomeKind,
    RunAgentsResult, SearchCodebaseFailureReason, SearchCodebaseResult, ServerOutputId, Shared,
    ShellCommandDelay, StartAgentExecutionMode, SuggestNewConversationResult, SummarizationType,
    TodoOperation, UserQueryMode,
};
pub use crate::ai::agent_conversations_model::{
    query_conversation_entries, AgentConversationEntry, AgentConversationEntryId,
    AgentConversationListEntryState, AgentConversationListPolicy, AgentConversationsModel,
    AgentConversationsModelEvent, AgentManagementFilters, AgentRunDisplayStatus, HarnessFilter,
    OwnerFilter,
};
pub use crate::ai::blocklist::agent_view::{
    AgentViewController, AgentViewDisplayMode, AgentViewEntryOrigin, EnterAgentViewError,
    EphemeralMessageModel,
};
pub use crate::ai::blocklist::block::cli_controller::{
    CLISubagentController, CLISubagentEvent, CLISubagentTarget, LongRunningCommandControlState,
    UserTakeOverReason,
};
pub use crate::ai::blocklist::block::model::{
    AIBlockModel, AIBlockModelImpl, AIBlockOutputStatus, AIRequestType, OutputStatusUpdateCallback,
};
pub use crate::ai::blocklist::conversation_selection::{
    ConversationSelection, ConversationSelectionEvent, ConversationSelectionHandle,
    PendingQueryState,
};
pub use crate::ai::blocklist::diff_storage::{
    DiffStorage, DiffStorageHelper, FileSnapshot, RegisteredDiffStorage, SaveFuture,
    UpdatedFileState,
};
pub use crate::ai::blocklist::diff_types::{changed_lines_from_op, DiffSessionType, FileDiff};
pub use crate::ai::blocklist::history_model::{
    BlocklistAIHistoryEvent, BlocklistAIHistoryModel, CloudConversationData,
    ConversationStatusUpdate,
};
#[cfg(any(test, feature = "test-util"))]
use crate::ai::blocklist::local_agent_task_sync_model::LocalAgentTaskSyncModel;
#[cfg(any(test, feature = "test-util"))]
use crate::ai::blocklist::orchestration_event_streamer::OrchestrationEventStreamer;
#[cfg(any(test, feature = "test-util"))]
use crate::ai::blocklist::orchestration_events::OrchestrationEventService;
pub use crate::ai::blocklist::view_util::format_credits;
pub use crate::ai::blocklist::{
    block_context_from_terminal_model, AIActionStatus, BlocklistAIActionEvent,
    BlocklistAIActionModel, BlocklistAIContextModel, BlocklistAIController, BlocklistAIInputModel,
    InputConfig, InputModePolicy, InputModePolicyHandle, InputType, InputTypeAutoDetectionSource,
    PolicyConfigUpdate, RequestFileEditsExecutor, RunAgentsExecutor, RunAgentsExecutorEvent,
    RunAgentsSpawningSnapshot, ShellCommandExecutor, ShellCommandExecutorEvent,
};
#[cfg(any(test, feature = "test-util"))]
use crate::ai::blocklist::{BlocklistAIPermissions, QueuedQueryModel};
#[cfg(any(test, feature = "test-util"))]
use crate::ai::cloud_agent_settings::CloudAgentSettings;
pub use crate::ai::connected_self_hosted_workers::{
    ConnectedSelfHostedWorkersEvent, ConnectedSelfHostedWorkersModel,
};
#[cfg(feature = "local_fs")]
pub use crate::ai::conversation_export::{
    export_conversation_markdown, ConversationFileExport, ConversationFileExportError,
};
#[cfg(any(test, feature = "test-util"))]
use crate::ai::execution_profiles::profiles::AIExecutionProfilesModel;
pub use crate::ai::get_relevant_files::controller::GetRelevantFilesController;
pub use crate::ai::harness_availability::{
    AuthSecretEntry, AuthSecretFetchState, HarnessAvailability, HarnessAvailabilityEvent,
    HarnessAvailabilityModel, HarnessModelInfo,
};
pub use crate::ai::llms::{LLMId, LLMInfo, LLMPreferences, LLMPreferencesEvent};
#[cfg(any(test, feature = "test-util"))]
use crate::ai::mcp::templatable_manager::TemplatableMCPServerManager;
pub use crate::ai::orchestration::{
    accept_disabled_reason_with_auth, api_key_snapshot, auth_secret_selection_required,
    empty_env_recommendation_message, environment_snapshot, harness_is_selectable,
    harness_snapshot, host_snapshot, location_snapshot, model_snapshot,
    persist_environment_selection, persist_host_selection,
    resolve_auth_secret_selection_for_harness, resolve_default_environment_id,
    resolve_default_host_slug, should_show_auth_secret_picker, AuthSecretSelection, OptionBadge,
    OptionFooter, OptionRow, OptionSnapshot, OptionSourceStatus, OrchestrationConfigState,
    OrchestrationEditState, ORCHESTRATION_ENV_NONE_LABEL, ORCHESTRATION_WARP_WORKER_HOST,
};
pub use crate::ai::skills::{SkillManager, SkillReference};
pub use crate::appearance::Appearance;
#[cfg(any(test, feature = "test-util"))]
use crate::auth::auth_manager::AuthManager;
#[cfg(any(test, feature = "test-util"))]
use crate::auth::AuthStateProvider;
pub use crate::banner::BannerState;
pub use crate::changelog_model::{
    ChangelogModel, ChangelogRequestType, ChangelogState, Event as ChangelogModelEvent,
};
#[cfg(any(test, feature = "test-util"))]
use crate::cloud_object::model::persistence::CloudModel;
pub use crate::code::DiffResult;
pub use crate::code_review::git_repo_model::{
    GitRepoModels, GitRepoStatusModel, GitStatusMetadata,
};
pub use crate::completer::SessionContext;
#[cfg(any(test, feature = "test-util"))]
use crate::network::NetworkStatus;
pub use crate::search::slash_command_menu::static_commands::commands::{
    self as slash_commands, COMMAND_REGISTRY,
};
pub use crate::search::slash_command_menu::{SlashCommandId, StaticCommand};
#[cfg(any(test, feature = "test-util"))]
use crate::server::server_api::ServerApiProvider;
#[cfg(any(test, feature = "test-util"))]
use crate::server::sync_queue::SyncQueue;
#[cfg(any(test, feature = "test-util"))]
use crate::settings::manager::SettingsManager;
pub use crate::settings::AISettingsChangedEvent;
#[cfg(any(test, feature = "test-util"))]
use crate::settings::{init_and_register_user_preferences, AISettings};
pub use crate::terminal::alt_screen::{should_intercept_mouse, should_intercept_scroll};
#[cfg(any(test, feature = "test-util"))]
use crate::terminal::cli_agent_sessions::CLIAgentSessionsModel;
pub use crate::terminal::color::{Colors as TerminalColors, List as TerminalColorList};
pub use crate::terminal::conversation_restoration::{
    prepare_conversation_block_restoration, ConversationBlockRestorationPlan,
    RestoredConversationExchange,
};
pub use crate::terminal::event::AfterBlockCompletedEvent;
pub use crate::terminal::input::decorations::parse_current_commands_and_tokens;
pub use crate::terminal::input::models::{query_model_picker_choices, ModelPickerChoice};
pub use crate::terminal::input::skills::{
    query_selectable_skills, AcceptSkill, SelectableSkill,
    LOCAL_SKILLS_REMOTE_EXECUTION_ERROR_MESSAGE,
};
pub use crate::terminal::input::slash_command_model::{
    slash_command_composition_filter, DetectedCommand, DetectedSkillCommand,
    ParsedSlashCommandInput,
};
pub use crate::terminal::input::slash_commands::{
    build_slash_command_mixer, record_saved_prompt_accepted, record_static_slash_command_accepted,
    saved_prompt_text_for_id, should_close_slash_command_menu_for_exact_match,
    slash_command_is_submitted_as_prompt, slash_command_is_supported_in_tui, slash_command_query,
    slash_command_selection_behavior, AcceptSlashCommandOrSavedPrompt, InlineItem,
    SlashCommandDataSource, SlashCommandMixer, SlashCommandSelectionBehavior,
    TuiDataSourceArgs as TuiSlashCommandDataSourceArgs, TuiSlashCommand, TuiSlashCommandDataSource,
    TuiZeroStateDataSource, UpdatedActiveCommands,
};
pub use crate::terminal::input::CommandExecutionSource;
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
pub use crate::terminal::model::session::active_session::{ActiveSession, ActiveSessionEvent};
pub use crate::terminal::model::session::Sessions;
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
use crate::user_config::WarpConfig;
pub use crate::util::repo_detection::{detect_possible_git_repo, RepoDetectionSessionType};
pub use crate::util::time_format::format_elapsed_seconds;
#[cfg(any(test, feature = "test-util"))]
use crate::workspaces::user_workspaces::UserWorkspaces;
#[cfg(any(test, feature = "test-util"))]
use crate::LaunchMode;

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

/// Registers the minimal singleton set needed to construct, render, and
/// accept the TUI orchestration (`RunAgents`) card against real app models:
/// the settings machinery backing `CloudAgentSettings`/`AISettings`, the
/// auth/server/cloud-object singletons the catalog models read, and the
/// catalog + permission models the card's snapshot builders and accept-path
/// permission checks use. Intended for `warp_tui` tests (via the `test-util`
/// feature) and this crate's own unit tests. Registration order matters:
/// each model subscribes to singletons registered before it.
#[cfg(any(test, feature = "test-util"))]
pub fn register_orchestration_test_singletons(app: &mut warpui::App) {
    // Settings machinery required by CloudAgentSettings/AISettings reads.
    app.add_singleton_model(|ctx| AppExecutionMode::new(ExecutionMode::App, false, ctx));
    app.update(init_and_register_user_preferences);
    app.add_singleton_model(|_| SettingsManager::default());
    app.add_singleton_model(WarpConfig::mock);
    app.update(|ctx| {
        // No-op secure storage backs ApiKeyManager in tests.
        warpui_extras::secure_storage::register_noop("test", ctx);
    });
    app.update(AISettings::register_and_subscribe_to_events);
    CloudAgentSettings::register(app);
    // Secure-storage-backed; LLMPreferences subscribes to it.
    app.add_singleton_model(ApiKeyManager::new);

    // Auth / server / cloud-object singletons the catalog models read.
    app.add_singleton_model(|_| NetworkStatus::new());
    app.add_singleton_model(|_| ServerApiProvider::new_for_test());
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(AuthManager::new_for_test);
    app.add_singleton_model(|ctx| {
        // `UserWorkspaces::default_mock` needs mockall (dev-dependency only),
        // so back the mock with the test ServerApi's clients instead.
        let (team_client, workspace_client) = {
            let provider = ServerApiProvider::as_ref(ctx);
            (provider.get_team_client(), provider.get_workspace_client())
        };
        UserWorkspaces::mock(team_client, workspace_client, vec![], ctx)
    });
    app.add_singleton_model(SyncQueue::mock);
    app.add_singleton_model(CloudModel::mock);
    app.add_singleton_model(|_| crate::appearance::Appearance::mock());

    // Catalog + permission singletons read by the card's construction,
    // snapshot builders, and accept path.
    app.add_singleton_model(|_| TemplatableMCPServerManager::default());
    app.add_singleton_model(LLMPreferences::new);
    app.add_singleton_model(HarnessAvailabilityModel::new);
    app.add_singleton_model(ConnectedSelfHostedWorkersModel::new);
    app.add_singleton_model(BlocklistAIPermissions::new);
    app.add_singleton_model(|ctx| {
        AIExecutionProfilesModel::new(&LaunchMode::new_for_unit_test(), ctx)
    });
    // Plan publication during the accept path reads the document model.
    app.add_singleton_model(|_| {
        crate::ai::document::ai_document_model::AIDocumentModel::new_for_test()
    });
}

/// Registers the singleton set needed to construct a full TUI session view's
/// AI stack in tests, on top of
/// [`register_orchestration_test_singletons`]. Registration order matters:
/// each model subscribes to singletons registered before it.
#[cfg(any(test, feature = "test-util"))]
pub fn register_tui_session_test_singletons(app: &mut warpui::App) {
    register_orchestration_test_singletons(app);
    app.add_singleton_model(|_| BlocklistAIHistoryModel::default());
    // QueuedQueryModel subscribes to history events; register after the
    // history model is in place.
    app.add_singleton_model(QueuedQueryModel::new);
    app.add_singleton_model(|_| CLIAgentSessionsModel::new());
    app.add_singleton_model(OrchestrationEventService::new);
    app.add_singleton_model(LocalAgentTaskSyncModel::new);
    app.add_singleton_model(OrchestrationEventStreamer::new);
    app.add_singleton_model(|_| ActiveAgentViewsModel::new());
    app.add_singleton_model(|_| GitRepoModels::new());
    app.add_singleton_model(|ctx| {
        CodebaseIndexManager::new_for_test(ServerApiProvider::as_ref(ctx).get(), ctx)
    });
    app.add_singleton_model(AgentConversationsModel::new);
}

/// [`register_tui_session_test_singletons`] plus the remaining singletons a
/// full `TuiTerminalSessionView` subscribes to.
#[cfg(any(test, feature = "test-util"))]
pub fn register_tui_session_view_test_singletons(app: &mut warpui::App) {
    register_tui_session_test_singletons(app);
    app.add_singleton_model(crate::tui::TuiMcpManager::new_for_test);
    app.add_singleton_model(|ctx| {
        crate::changelog_model::ChangelogModel::new(ServerApiProvider::as_ref(ctx).get())
    });
    app.add_singleton_model(|_| ai::project_context::model::ProjectContextModel::default());
    // The TUI auto-updater (which the session view subscribes to) reads its
    // enablement setting at registration.
    app.update(crate::settings::TuiAutoupdateSettings::register);
    // Settings groups the editor-backed input view and transcript read.
    app.update(crate::settings::CodeSettings::register);
    app.update(crate::settings::FontSettings::register);
    app.update(crate::settings::InputSettings::register);
    app.update(crate::settings::InputModeSettings::register);
    app.update(crate::settings::SelectionSettings::register);
    app.update(crate::settings::ScrollSettings::register);
    app.update(crate::settings::EmacsBindingsSettings::register);
    app.update(crate::terminal::general_settings::GeneralSettings::register);
    // Filesystem-watcher singletons the workflow/skill sources read.
    app.add_singleton_model(|_| repo_metadata::repositories::DetectedRepositories::default());
    app.add_singleton_model(watcher::HomeDirectoryWatcher::new_for_test);
    app.add_singleton_model(repo_metadata::watcher::DirectoryWatcher::new);
    #[cfg(feature = "local_fs")]
    app.add_singleton_model(repo_metadata::RepoMetadataModel::new);
    app.add_singleton_model(
        crate::warp_managed_paths_watcher::WarpManagedPathsWatcher::new_for_testing,
    );
    app.add_singleton_model(crate::workflows::local_workflows::LocalWorkflows::new);
    app.add_singleton_model(crate::ai::skills::SkillManager::new);
}
