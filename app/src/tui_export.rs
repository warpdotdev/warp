//! Public app APIs used by the `warp_tui` frontend.

pub use crate::ai::agent::api::ServerConversationToken;
pub use crate::ai::agent::conversation::{
    AIConversationAutoexecuteMode, AIConversationId, ConversationStatus,
};
pub use crate::ai::agent::task::TaskId;
pub use crate::ai::agent::{
    AIAgentAction, AIAgentActionId, AIAgentActionType, AIAgentExchangeId, AIAgentInput,
    AIAgentOutput, AIAgentOutputMessage, AIAgentOutputMessageType, AIAgentPtyWriteMode,
    AIAgentText, AIAgentTextSection, CancellationReason, MessageId, ServerOutputId, Shared,
    UserQueryMode,
};
pub use crate::ai::blocklist::agent_view::{
    AgentViewDisplayMode, AgentViewEntryOrigin, EnterAgentViewError,
};
pub use crate::ai::blocklist::block::model::{
    AIBlockModel, AIBlockModelImpl, AIBlockOutputStatus, AIRequestType, OutputStatusUpdateCallback,
};
pub use crate::ai::blocklist::conversation_selection::{
    ConversationSelection, ConversationSelectionEvent, ConversationSelectionHandle,
    PendingQueryState,
};
pub use crate::ai::blocklist::history_model::{
    BlocklistAIHistoryEvent, BlocklistAIHistoryModel, CloudConversationData,
    ConversationStatusUpdate,
};
pub use crate::ai::blocklist::{
    BlocklistAIActionModel, BlocklistAIContextModel, BlocklistAIController, BlocklistAIInputModel,
    ShellCommandExecutor, ShellCommandExecutorEvent,
};
pub use crate::ai::get_relevant_files::controller::GetRelevantFilesController;
pub use crate::ai::llms::{LLMId, LLMInfo, LLMPreferences, LLMPreferencesEvent};
pub use crate::appearance::Appearance;
pub use crate::banner::BannerState;
pub use crate::terminal::color::{Colors as TerminalColors, List as TerminalColorList};
pub use crate::terminal::event::AfterBlockCompletedEvent;
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
pub use crate::terminal::model::terminal_model::BlockIndex;
pub use crate::terminal::model_events::ModelEvent;
pub use crate::terminal::shared_session::IsSharedSessionCreator;
pub use crate::terminal::view::blocklist_filter::should_show_task_in_blocklist;
pub use crate::terminal::view::ExecuteCommandEvent;
pub use crate::terminal::{
    PtyIntent, PtyIntentEvent, ShellLaunchData, TerminalManager as TerminalManagerTrait,
    TerminalModel, TerminalSurface,
};
pub use crate::themes::default_themes::dark_theme;
