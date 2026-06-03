//! Generic inline menu view for rendering search results with selection and navigation.
mod message_bar;
mod message_provider;
mod model;
pub(crate) mod positioning;
pub mod styles;
mod view;

pub use message_bar::{InlineMenuMessageArgs, InlineMenuMessageBarArgs};
pub use message_provider::{default_navigation_message_items, InlineMenuMessageProvider};
pub use model::{InlineMenuModel, InlineMenuModelEvent, InlineMenuTabConfig};
pub use positioning::InlineMenuPositioner;
use serde::{Deserialize, Serialize};
pub use view::{
    DetailsRenderConfig, InlineMenuAction, InlineMenuClickBehavior, InlineMenuEvent,
    InlineMenuHeaderConfig, InlineMenuRowAction, InlineMenuView, QueryResultRendererExt,
};

use super::{InputSuggestionsMode, UserQueryMenuAction};

/// Identifies a specific inline menu type.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(
    description = "Identifies a specific inline menu.",
    rename_all = "snake_case"
)]
pub enum InlineMenuType {
    SlashCommands,
    ModelSelector,
    ConversationMenu,
    ProfileSelector,
    PromptsMenu,
    SkillMenu,
    UserQueryMenu,
    RewindMenu,
    InlineHistoryMenu,
    IndexedReposMenu,
    PlanMenu,
}

impl InlineMenuType {
    fn display_label(&self) -> String {
        match self {
            InlineMenuType::SlashCommands => "/Commands".to_owned(),
            InlineMenuType::ModelSelector => "/Model".to_owned(),
            InlineMenuType::ConversationMenu => "/Conversations".to_owned(),
            InlineMenuType::ProfileSelector => "/Profiles".to_owned(),
            InlineMenuType::PromptsMenu => "/Prompts".to_owned(),
            InlineMenuType::SkillMenu => "/Skills".to_owned(),
            InlineMenuType::UserQueryMenu => "/Fork".to_owned(),
            InlineMenuType::RewindMenu => "/Rewind".to_owned(),
            InlineMenuType::InlineHistoryMenu => i18n::t("terminal.input.inline_menu.history"),
            InlineMenuType::IndexedReposMenu => "/Repos".to_owned(),
            InlineMenuType::PlanMenu => "/Plans".to_owned(),
        }
    }

    pub(crate) fn from_suggestions_mode(mode: &InputSuggestionsMode) -> Option<Self> {
        match mode {
            InputSuggestionsMode::SlashCommands => Some(InlineMenuType::SlashCommands),
            InputSuggestionsMode::ModelSelector => Some(InlineMenuType::ModelSelector),
            InputSuggestionsMode::ConversationMenu => Some(InlineMenuType::ConversationMenu),
            InputSuggestionsMode::ProfileSelector => Some(InlineMenuType::ProfileSelector),
            InputSuggestionsMode::PromptsMenu => Some(InlineMenuType::PromptsMenu),
            InputSuggestionsMode::SkillMenu => Some(InlineMenuType::SkillMenu),
            InputSuggestionsMode::UserQueryMenu {
                action: UserQueryMenuAction::ForkFrom,
                ..
            } => Some(InlineMenuType::UserQueryMenu),
            InputSuggestionsMode::UserQueryMenu {
                action: UserQueryMenuAction::Rewind,
                ..
            } => Some(InlineMenuType::RewindMenu),
            InputSuggestionsMode::InlineHistoryMenu { .. } => {
                Some(InlineMenuType::InlineHistoryMenu)
            }
            InputSuggestionsMode::IndexedReposMenu => Some(InlineMenuType::IndexedReposMenu),
            InputSuggestionsMode::PlanMenu { .. } => Some(InlineMenuType::PlanMenu),
            InputSuggestionsMode::Closed
            | InputSuggestionsMode::HistoryUp { .. }
            | InputSuggestionsMode::CompletionSuggestions { .. }
            | InputSuggestionsMode::StaticWorkflowEnumSuggestions { .. }
            | InputSuggestionsMode::DynamicWorkflowEnumSuggestions { .. }
            | InputSuggestionsMode::AIContextMenu { .. } => None,
        }
    }
}
