//! This module contains model, controller, and view logic for Blocklist AI.
mod action_model;
pub mod agent_view;
pub mod block;
pub mod code_block;
mod context_model;
mod controller;
pub(crate) mod orchestration_events;
mod passive_suggestions;
pub(super) use controller::RequestInput;
pub mod history_model;
pub mod inline_action;
mod input_model;
pub mod metadata;
mod permissions;
mod persistence;
pub mod prompt;
pub mod summarization_cancel_dialog;
pub mod usage;

pub(crate) mod codebase_index_speedbump_banner;
pub(super) mod view_util;
pub(crate) use action_model::{
    apply_edits, read_local_file_context, BlocklistAIActionEvent, BlocklistAIActionModel,
    FileReadResult, RequestFileEditsFormatKind, ShellCommandExecutor, ShellCommandExecutorEvent,
    StartAgentExecutor, StartAgentExecutorEvent, StartAgentRequest,
};

#[cfg(any(test, feature = "integration_tests"))]
pub(crate) use block::model::testing::FakeAIBlockModel;
pub(crate) use block::{init, model, AIBlock, AIBlockEvent};

pub(crate) use context_model::{
    block_context_from_terminal_model, AttachmentType, BlocklistAIContextEvent,
    BlocklistAIContextModel, PendingAttachment, PendingFile, PendingQueryState,
};
pub(crate) use controller::{
    response_stream::ResponseStreamId, BlocklistAIController, BlocklistAIControllerEvent,
    ClientIdentifiers, SessionContext, SlashCommandRequest,
};
pub(crate) use history_model::{
    AIQueryHistory, AIQueryHistoryOutputStatus, BlocklistAIHistoryEvent, BlocklistAIHistoryModel,
    FORK_PREFIX, PRE_REWIND_PREFIX,
};
pub(crate) use input_model::{
    BlocklistAIInputEvent, BlocklistAIInputModel, InputConfig, InputType,
};
pub(crate) use passive_suggestions::{
    LegacyPassiveSuggestionsEvent, LegacyPassiveSuggestionsModel, MaaPassiveSuggestionsEvent,
    MaaPassiveSuggestionsModel, PassiveSuggestionsModels,
};
pub(crate) use persistence::PersistedAIInputType;
pub(crate) use persistence::{PersistedAIInput, SerializedBlockListItem};
pub(crate) use view_util::{
    ai_brand_color, ai_indicator_height, get_ai_block_overflow_menu_element_position_id,
    get_attached_blocks_chip_element_position_id, render_ai_agent_mode_icon,
    render_ai_follow_up_icon, ATTACH_AS_AGENT_MODE_CONTEXT_TEXT, CLAUDE_ORANGE,
    NEW_AGENT_PANE_LABEL,
};

pub use crate::ai::blocklist::block::{secret_redaction, AIBlockResponseRating, TextLocation};
pub use block::keyboard_navigable_buttons;
pub use block::toggleable_items;
pub use controller::input_context::{
    BLOCK_CONTEXT_ATTACHMENT_REGEX, DIFF_HUNK_ATTACHMENT_REGEX, DRIVE_OBJECT_ATTACHMENT_REGEX,
};
pub use permissions::BlocklistAIPermissions;
pub use view_util::error_color;
