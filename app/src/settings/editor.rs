use std::fmt::{Display, Formatter};

use enum_iterator::{all, Sequence};
use serde::{Deserialize, Serialize};
use settings::macros::define_settings_group;
use settings::{RespectUserSyncSetting, Setting as _, SupportedPlatforms, SyncToCloud};
use warpui::ModelContext;

#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    Eq,
    PartialEq,
    Deserialize,
    Serialize,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(description = "Whether the cursor blinks.", rename_all = "snake_case")]
pub enum CursorBlink {
    #[default]
    Enabled,
    Disabled,
}

impl CursorBlink {
    pub fn other_value(&self) -> Self {
        match self {
            Self::Enabled => Self::Disabled,
            Self::Disabled => Self::Enabled,
        }
    }
}

#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    Eq,
    PartialEq,
    Deserialize,
    Serialize,
    Sequence,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(description = "Visual style of the cursor.", rename_all = "snake_case")]
pub enum CursorDisplayType {
    #[default]
    Bar,
    Block,
    Underline,
}

impl CursorDisplayType {
    pub fn nth(index: usize) -> Option<Self> {
        all::<Self>().nth(index)
    }

    pub fn to_index(&self) -> usize {
        all::<Self>()
            .position(|v| v == *self)
            .expect("Cursor display type not found in Sequence!")
    }
}

impl Display for CursorDisplayType {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let value = match &self {
            CursorDisplayType::Bar => "Bar",
            CursorDisplayType::Block => "Block",
            CursorDisplayType::Underline => "Underline",
        };
        write!(f, "{value}")
    }
}

#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    Eq,
    PartialEq,
    Deserialize,
    Serialize,
    Sequence,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(
    description = "How line numbers are displayed in code editors.",
    rename_all = "snake_case"
)]
pub enum CodeEditorLineNumberMode {
    #[default]
    Absolute,
    Relative,
}

impl CodeEditorLineNumberMode {
    pub fn dropdown_item_label(&self) -> &'static str {
        match self {
            Self::Absolute => "Absolute",
            Self::Relative => "Relative",
        }
    }
}

#[derive(Clone, Copy, Default, Debug, Serialize, Deserialize, PartialEq)]
pub enum TabBehavior {
    #[default]
    Completions,
    Autosuggestions,
    UserDefined,
}

impl TabBehavior {
    pub fn dropdown_item_label(&self) -> &'static str {
        match self {
            TabBehavior::Completions => "Open completions menu",
            TabBehavior::Autosuggestions => "Accept autosuggestion",
            TabBehavior::UserDefined => "User defined",
        }
    }
}

/// This enum is used to enforce options in the dropdown for selecting a separator with the Warp prompt.
/// Note that these separators are added at the END of the Warp prompt (used in the case of same line prompt).
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    Eq,
    PartialEq,
    Deserialize,
    Serialize,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(
    description = "Trailing separator character displayed at the end of the prompt.",
    rename_all = "snake_case"
)]
pub enum WarpPromptSeparator {
    /// No separator for the prompt.
    #[default]
    None,
    /// "%" separator for the prompt. Note this is the default separator used in zsh traditionally.
    PercentSign,
    /// "$" separator for the prompt. Note this is the default separator used in bash traditionally.
    DollarSign,
    /// ">" separator for the prompt. Note this is the default separator used in fish traditionally.
    ChevronSymbol,
}

impl WarpPromptSeparator {
    pub fn dropdown_item_label(&self) -> &'static str {
        match self {
            Self::None => "None",
            Self::PercentSign => "%",
            Self::DollarSign => "$",
            Self::ChevronSymbol => ">",
        }
    }

    pub fn renderable_string(&self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::PercentSign => Some("%"),
            Self::DollarSign => Some("$"),
            Self::ChevronSymbol => Some(">"),
        }
    }
}

define_settings_group!(AppEditorSettings, settings: [
    cursor_blink: CursorBlinkEnabled {
        type: CursorBlink,
        default: CursorBlink::default(),
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        surface: settings::SettingSurfaces::GUI,
        private: false,
        storage_key: "CursorBlink",
        toml_path: "appearance.cursor.cursor_blink",
        description: "Whether the cursor blinks.",
    },
    cursor_display_type: CursorDisplayState {
        type: CursorDisplayType,
        default: CursorDisplayType::default(),
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        surface: settings::SettingSurfaces::GUI,
        private: false,
        storage_key: "CursorDisplayType",
        toml_path: "appearance.cursor.cursor_display_type",
        description: "The visual style of the cursor.",
    },
    vim_mode: VimModeEnabled {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        surface: settings::SettingSurfaces::GUI,
        private: false,
        toml_path: "text_editing.vim_mode_enabled",
        description: "Whether Vim keybindings are enabled.",
    },
    vim_unnamed_system_clipboard: VimUnnamedSystemClipboard {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        surface: settings::SettingSurfaces::GUI,
        private: false,
        toml_path: "text_editing.vim_unnamed_system_clipboard",
        description: "Whether the Vim unnamed register uses the system clipboard.",
    },
    vim_status_bar: VimStatusBar {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        surface: settings::SettingSurfaces::GUI,
        private: false,
        toml_path: "text_editing.vim_status_bar",
        description: "Whether the Vim status bar is displayed.",
    },
    code_editor_line_number_mode: CodeEditorLineNumberModeSetting {
        type: CodeEditorLineNumberMode,
        default: CodeEditorLineNumberMode::default(),
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        surface: settings::SettingSurfaces::GUI,
        private: false,
        toml_path: "text_editing.code_editor_line_number_mode",
        description: "How line numbers are displayed in code editors.",
    },
    autocomplete_symbols: AutocompleteSymbols {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        surface: settings::SettingSurfaces::GUI,
        private: false,
        toml_path: "text_editing.autocomplete_symbols",
        description: "Whether matching symbols like brackets and quotes are auto-completed.",
    },
    enable_autosuggestions: EnableAutosuggestions {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        surface: settings::SettingSurfaces::GUI,
        private: false,
        storage_key: "Autosuggestions",
        toml_path: "terminal.input.autosuggestions.enabled",
        description: "Whether command autosuggestions are shown.",
    },
    autosuggestion_keybinding_hint: AutosuggestionKeybindingHint {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        surface: settings::SettingSurfaces::GUI,
        private: false,
        toml_path: "terminal.input.autosuggestions.keybinding_hint",
        description: "Whether autosuggestion keybinding hints are displayed.",
    },
    show_autosuggestion_ignore_button: ShowAutosuggestionIgnoreButton {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        surface: settings::SettingSurfaces::GUI,
        private: false,
        toml_path: "terminal.input.autosuggestions.show_ignore_button",
        description: "Whether the ignore button is shown for autosuggestions.",
    },
]);

impl AppEditorSettings {
    pub fn toggle_cursor_blink(&mut self, ctx: &mut ModelContext<Self>) {
        self.cursor_blink
            .set_value(self.cursor_blink.other_value(), ctx)
            .expect("failed to serialize CursorBlinkEnabled");
        ctx.notify();
    }

    pub fn vim_mode_enabled(&self) -> bool {
        *self.vim_mode.value()
    }

    pub fn cursor_blink_enabled(&self) -> bool {
        *self.cursor_blink.value() == CursorBlink::Enabled
    }
}
