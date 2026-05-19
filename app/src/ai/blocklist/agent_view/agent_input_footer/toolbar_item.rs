use serde::{Deserialize, Serialize};
use warpui::SingletonEntity;

use crate::context_chips::{agent_footer_available_chips, available_chips, ContextChipKind};
use crate::features::FeatureFlag;
use crate::settings::AISettings;
use crate::terminal::shared_session::SharedSessionStatus;
use crate::ui_components::icons::Icon;

use super::editor::AgentToolbarEditorMode;

/// Declares which footer(s) a toolbar item is available in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolbarAvailability {
    AgentViewOnly,
    CLIAgentOnly,
    Both,
}

impl ToolbarAvailability {
    pub fn is_available_for_agent_view(self) -> bool {
        matches!(self, Self::AgentViewOnly | Self::Both)
    }

    pub fn is_available_for_cli(self) -> bool {
        matches!(self, Self::CLIAgentOnly | Self::Both)
    }
}

/// A configurable item
///
/// This unifies context-chip data displays with interactive control buttons so
/// they can all be arranged through the same drag-and-drop editor.
#[derive(
    Clone,
    Debug,
    Eq,
    PartialEq,
    Hash,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(
    description = "An item that can appear in the agent toolbar.",
    rename_all = "snake_case"
)]
pub enum AgentToolbarItemKind {
    #[schemars(description = "A prompt context chip.")]
    ContextChip(ContextChipKind),
    // Agent view only
    ModelSelector,
    NLDToggle,
    ContextWindowUsage,

    // CLI agent only
    FileExplorer,
    RichInput,

    // Both
    VoiceInput,
    // Renamed from ImageAttach; alias preserves existing user toolbar configs.
    #[serde(alias = "ImageAttach")]
    FileAttach,
    ShareSession,

    // CLI agent only – opens settings to the Coding Agents section.
    Settings,

    // Agent view only – shows fast-forward (auto-approve) toggle in the footer
    FastForwardToggle,

    // Agent view only – "Hand off to cloud" chip.
    HandoffToCloud,
}

impl AgentToolbarItemKind {
    pub fn available_in(&self) -> ToolbarAvailability {
        match self {
            Self::ContextChip(_) | Self::VoiceInput | Self::FileAttach | Self::ShareSession => {
                ToolbarAvailability::Both
            }
            Self::ModelSelector
            | Self::NLDToggle
            | Self::ContextWindowUsage
            | Self::FastForwardToggle
            | Self::HandoffToCloud => ToolbarAvailability::AgentViewOnly,
            Self::FileExplorer | Self::RichInput | Self::Settings => {
                ToolbarAvailability::CLIAgentOnly
            }
        }
    }

    /// Whether this item should be visible to session viewers.
    /// Items that control host settings or initiate actions on the host's
    /// behalf are hidden from viewers.
    pub fn available_to_session_viewer(
        &self,
        status: &SharedSessionStatus,
        is_cloud_mode: bool,
    ) -> bool {
        match self {
            Self::Settings | Self::ShareSession | Self::FileExplorer => !status.is_viewer(),
            Self::FileAttach => !status.is_viewer() || is_cloud_mode,
            Self::FastForwardToggle => !status.is_viewer() || status.is_executor(),
            // Handoff is host-initiated; viewers cannot hand off another user's conversation.
            Self::HandoffToCloud => !status.is_viewer(),
            Self::ContextChip(_)
            | Self::ModelSelector
            | Self::NLDToggle
            | Self::ContextWindowUsage
            | Self::RichInput
            | Self::VoiceInput => true,
        }
    }

    pub fn display_label(&self) -> &'static str {
        match self {
            Self::ContextChip(_) => "Context Chip",
            Self::ModelSelector => "Model Selector",
            Self::NLDToggle => "Autodetection",
            Self::VoiceInput => "Voice Input",
            Self::FileAttach => "Attach File",
            Self::ContextWindowUsage => "Context Usage",
            Self::FileExplorer => "File Explorer",
            Self::RichInput => "Rich Input",
            Self::ShareSession => "/remote-control",
            Self::Settings => "Settings",
            Self::FastForwardToggle => "Fast Forward",
            Self::HandoffToCloud => "Hand off to cloud",
        }
    }

    pub fn icon(&self) -> Option<Icon> {
        match self {
            Self::ContextChip(kind) => kind.udi_icon(),
            Self::ModelSelector => Some(Icon::Oz),
            Self::NLDToggle => Some(Icon::NLD),
            Self::VoiceInput => Some(Icon::Microphone),
            Self::FileAttach => Some(Icon::Plus),
            Self::ContextWindowUsage => Some(Icon::ConversationContext0),
            Self::FileExplorer => Some(Icon::FileCopy),
            Self::RichInput => Some(Icon::TextInput),
            Self::ShareSession => Some(Icon::Phone01),
            Self::Settings => Some(Icon::Settings),
            Self::FastForwardToggle => Some(Icon::FastForward),
            // The bundled `upload-cloud-01.svg` (cloud-with-upward-arrow) is the
            // closest fit among the existing icons for V0; design may swap it later.
            Self::HandoffToCloud => Some(Icon::UploadCloud),
        }
    }

    /// Whether this item should remain visible during `&` handoff-compose mode.
    /// Only items relevant to composing a cloud run are shown.
    pub(super) fn is_available_during_handoff_compose(&self) -> bool {
        match self {
            Self::ContextChip(ContextChipKind::ShellGitBranch) => true,
            Self::ModelSelector | Self::VoiceInput | Self::FileAttach => true,
            Self::ContextChip(_)
            | Self::NLDToggle
            | Self::ContextWindowUsage
            | Self::FastForwardToggle
            | Self::HandoffToCloud
            | Self::ShareSession
            | Self::FileExplorer
            | Self::RichInput
            | Self::Settings => false,
        }
    }

    /// Whether this item should be included in the toolbar given the current app state.
    /// Feature-flag checks live in `all_available()` / `default_*()`. This method
    /// handles runtime conditions that depend on user settings or workspace state.
    pub fn is_available(&self, app: &warpui::AppContext) -> bool {
        match self {
            Self::HandoffToCloud => AISettings::as_ref(app).is_cloud_handoff_enabled(app),
            _ => true,
        }
    }

    pub fn is_context_chip(&self) -> bool {
        matches!(self, Self::ContextChip(_))
    }

    pub fn context_chip_kind(&self) -> Option<&ContextChipKind> {
        match self {
            Self::ContextChip(kind) => Some(kind),
            _ => None,
        }
    }

    /// Default left-side items for the agent view footer.
    pub fn default_left() -> Vec<Self> {
        let mut items = vec![
            Self::ContextChip(ContextChipKind::Ssh),
            Self::ContextChip(ContextChipKind::WorkingDirectory),
            Self::ContextChip(ContextChipKind::ShellGitBranch),
            Self::ContextChip(ContextChipKind::GitDiffStats),
        ];
        if FeatureFlag::GithubPrPromptChip.is_enabled() {
            items.push(Self::ContextChip(ContextChipKind::GithubPullRequest));
        }
        items.push(Self::NLDToggle);
        items
    }

    /// Default right-side items for the agent view footer.
    pub fn default_right() -> Vec<Self> {
        let mut items = vec![
            Self::ContextChip(ContextChipKind::AgentPlanAndTodoList),
            Self::ContextWindowUsage,
            Self::ModelSelector,
        ];
        if FeatureFlag::CreatingSharedSessions.is_enabled()
            && FeatureFlag::HOARemoteControl.is_enabled()
        {
            items.push(Self::ShareSession);
        }
        if FeatureFlag::OzHandoff.is_enabled()
            && FeatureFlag::HandoffLocalCloud.is_enabled()
            && cfg!(all(feature = "local_fs", not(target_family = "wasm")))
        {
            items.push(Self::HandoffToCloud);
        }
        items.push(Self::VoiceInput);
        items.push(Self::FileAttach);
        items
    }

    /// All items available for the agent view footer configurator.
    pub fn all_available() -> Vec<Self> {
        let mut items: Vec<Self> = agent_footer_available_chips()
            .into_iter()
            .map(Self::ContextChip)
            .collect();
        items.extend([
            Self::ModelSelector,
            Self::NLDToggle,
            Self::VoiceInput,
            Self::FileAttach,
            Self::ContextWindowUsage,
        ]);
        if FeatureFlag::FastForwardAutoexecuteButton.is_enabled() {
            items.push(Self::FastForwardToggle);
        }
        if FeatureFlag::CreatingSharedSessions.is_enabled()
            && FeatureFlag::HOARemoteControl.is_enabled()
        {
            items.push(Self::ShareSession);
        }
        if FeatureFlag::OzHandoff.is_enabled()
            && FeatureFlag::HandoffLocalCloud.is_enabled()
            && cfg!(all(feature = "local_fs", not(target_family = "wasm")))
        {
            items.push(Self::HandoffToCloud);
        }
        items
    }

    /// Default left-side items for the CLI agent footer.
    pub fn cli_default_left() -> Vec<Self> {
        let mut items = vec![
            Self::FileAttach,
            Self::VoiceInput,
            Self::ContextChip(ContextChipKind::GitDiffStats),
        ];
        if FeatureFlag::CreatingSharedSessions.is_enabled()
            && FeatureFlag::HOARemoteControl.is_enabled()
        {
            items.push(Self::ShareSession);
        }
        items.push(Self::FileExplorer);
        if FeatureFlag::CLIAgentRichInput.is_enabled() {
            items.push(Self::RichInput);
        }
        items
    }

    /// Default right-side items for the CLI agent footer.
    pub fn cli_default_right() -> Vec<Self> {
        vec![
            Self::ContextChip(ContextChipKind::WorkingDirectory),
            Self::ContextChip(ContextChipKind::ShellGitBranch),
            Self::Settings,
        ]
    }

    /// All items available for the CLI agent footer configurator.
    pub fn all_available_for_cli_input() -> Vec<Self> {
        let mut items: Vec<Self> = available_chips()
            .into_iter()
            .map(Self::ContextChip)
            .collect();
        items.extend([
            Self::FileExplorer,
            Self::RichInput,
            Self::FileAttach,
            Self::VoiceInput,
            Self::Settings,
        ]);
        if FeatureFlag::CreatingSharedSessions.is_enabled()
            && FeatureFlag::HOARemoteControl.is_enabled()
        {
            items.push(Self::ShareSession);
        }
        items
    }

    /// Returns the appropriate defaults and available items for a given editor mode.
    pub fn defaults_for_mode(mode: AgentToolbarEditorMode) -> (Vec<Self>, Vec<Self>, Vec<Self>) {
        match mode {
            AgentToolbarEditorMode::AgentView => (
                Self::default_left(),
                Self::default_right(),
                Self::all_available(),
            ),
            AgentToolbarEditorMode::CLIAgent => (
                Self::cli_default_left(),
                Self::cli_default_right(),
                Self::all_available_for_cli_input(),
            ),
        }
    }
}

impl From<ContextChipKind> for AgentToolbarItemKind {
    fn from(kind: ContextChipKind) -> Self {
        Self::ContextChip(kind)
    }
}
