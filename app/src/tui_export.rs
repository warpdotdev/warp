//! Public app APIs used by the `warp_tui` frontend.

pub use repo_metadata::repositories::RepoDetectionSource;

pub use crate::ai::agent::api::ServerConversationToken;
pub use crate::ai::agent::conversation::{
    AIConversationAutoexecuteMode, AIConversationId, ConversationStatus, ConversationUsageTotals,
};
pub use crate::ai::agent::task::TaskId;
pub use crate::ai::agent::{
    AIAgentAction, AIAgentActionId, AIAgentActionResult, AIAgentActionResultType,
    AIAgentActionType, AIAgentExchangeId, AIAgentInput, AIAgentOutput, AIAgentOutputMessage,
    AIAgentOutputMessageType, AIAgentPtyWriteMode, AIAgentText, AIAgentTextSection,
    AskUserQuestionResult, CancellationReason, FileGlobV2Result, GrepResult, MessageId,
    RequestCommandOutputResult, RunAgentsAgentOutcomeKind, RunAgentsResult,
    SearchCodebaseFailureReason, SearchCodebaseResult, ServerOutputId, Shared,
    StartAgentExecutionMode, SuggestNewConversationResult, UserQueryMode,
};
pub use crate::ai::blocklist::agent_view::{
    AgentViewController, AgentViewDisplayMode, AgentViewEntryOrigin, EnterAgentViewError,
    EphemeralMessageModel,
};
pub use crate::ai::blocklist::block::cli_controller::CLISubagentController;
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
pub use crate::ai::blocklist::view_util::format_credits;
pub use crate::ai::blocklist::{
    AIActionStatus, BlocklistAIActionEvent, BlocklistAIActionModel, BlocklistAIContextModel,
    BlocklistAIController, BlocklistAIInputModel, InputConfig, InputModePolicy,
    InputModePolicyHandle, InputType, InputTypeAutoDetectionSource, PolicyConfigUpdate,
    RequestFileEditsExecutor, ShellCommandExecutor, ShellCommandExecutorEvent,
};
pub use crate::ai::get_relevant_files::controller::GetRelevantFilesController;
pub use crate::ai::llms::{LLMId, LLMInfo, LLMPreferences, LLMPreferencesEvent};
pub use crate::ai::skills::SkillManager;
pub use crate::appearance::Appearance;
pub use crate::banner::BannerState;
pub use crate::changelog_model::{
    ChangelogModel, ChangelogRequestType, ChangelogState, Event as ChangelogModelEvent,
};
pub use crate::code::DiffResult;
pub use crate::code_review::git_repo_model::{
    GitRepoModels, GitRepoStatusModel, GitStatusMetadata,
};
pub use crate::settings::AISettingsChangedEvent;
pub use crate::terminal::color::{Colors as TerminalColors, List as TerminalColorList};
pub use crate::terminal::event::AfterBlockCompletedEvent;
pub use crate::terminal::input::slash_command_model::slash_command_composition_filter;
pub use crate::terminal::input::slash_commands::{
    build_slash_command_mixer, slash_command_query, AcceptSlashCommandOrSavedPrompt, InlineItem,
    SlashCommandDataSource, SlashCommandMixer, TuiDataSourceArgs as TuiSlashCommandDataSourceArgs,
    TuiSlashCommandDataSource, TuiZeroStateDataSource, UpdatedActiveCommands,
};
pub use crate::terminal::input::CommandExecutionSource;
pub use crate::terminal::local_tty::{
    TerminalManager as LocalTtyTerminalManager, TerminalManagerInit, TerminalSurfaceInit,
    TerminalSurfaceResult,
};
pub use crate::terminal::model::block::{AgentInteractionMetadata, Block, BlockId};
pub use crate::terminal::model::blockgrid::BlockGrid;
pub use crate::terminal::model::blocks::{
    BlockHeight, BlockHeightItem, BlockHeightSummary, BlockList, RichContentItem, TotalIndex,
};
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
    BlockPadding, PtyIntent, PtyIntentEvent, ShellLaunchData,
    TerminalManager as TerminalManagerTrait, TerminalModel, TerminalSurface,
};
pub use crate::themes::default_themes::{dark_theme, light_theme};
pub use crate::throttle::throttle;
pub use crate::util::repo_detection::{detect_possible_git_repo, RepoDetectionSessionType};
pub use crate::util::time_format::format_elapsed_seconds;
