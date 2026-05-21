use crate::default_terminal::DefaultTerminal;
use crate::gpu_state::{GPUState, GPUStateEvent};
use crate::terminal::input::OPEN_COMPLETIONS_KEYBINDING_NAME;
#[cfg(feature = "local_tty")]
use crate::terminal::session_settings::WorkingDirectoryConfig;

use lazy_static::lazy_static;
use warp_core::context_flag::ContextFlag;
use warpui::platform::GraphicsBackend;
use warpui::rendering::GPUPowerPreference;
use warpui::{elements::DispatchEventResult, platform::Cursor};
#[cfg(target_os = "linux")]
use {
    crate::settings::ForceX11, crate::settings::LinuxAppConfiguration,
    warpui::platform::linux::windowing_system_is_customizable,
};

use super::keybindings::KeyBindingModifyingState;
#[cfg(feature = "local_tty")]
use super::settings_page::render_sub_sub_header;
use super::settings_page::{
    add_setting, build_reset_button, render_body_item_label, render_dropdown_item_label,
    render_local_only_icon, Category, LocalOnlyIconState, MatchData, PageType, SettingsWidget,
    TOGGLE_BUTTON_RIGHT_PADDING,
};
use super::settings_page::{
    render_body_item, render_dropdown_item, AdditionalInfo, SettingsPageMeta,
    SettingsPageViewHandle, ToggleState, CONTENT_FONT_SIZE, HEADER_PADDING,
};
use super::{features, SettingsAction};
use super::{flags, DisplayCount};
use super::{SettingsSection, ToggleSettingActionPair};
use crate::editor::{
    Event as EditorEvent, SingleLineEditorOptions, TextOptions,
    ACCEPT_AUTOSUGGESTION_KEYBINDING_NAME,
};
use crate::search::command_search::settings::{
    CommandSearchSettings, ShowGlobalWorkflowsInUniversalSearch,
};
use crate::server::telemetry::TelemetryEvent;
use crate::settings::ai::AISettings;
use crate::settings::{
    AISettingsChangedEvent, ScrollSettingsChangedEvent, ShowChangelogAfterUpdate,
    UserNativeRedirectPreference,
};
use crate::settings::{
    AliasExpansionEnabled, AliasExpansionSettings, AppEditorSettings, AtContextMenuInTerminalMode,
    AutocompleteSymbols, AutosuggestionKeybindingHint, ChangelogSettings, CloudPreferencesSettings,
    CodeSettings, CommandCorrections, CompletionsOpenWhileTyping, CopyOnSelect, CtrlTabBehavior,
    DefaultSessionMode, EnableSlashCommandsInTerminal, EnableSshWrapper, ErrorUnderliningEnabled,
    ExtraMetaKeys, GPUSettings, GlobalHotkeyMode, InputSettings, InputSettingsChangedEvent,
    LinuxSelectionClipboard, MiddleClickPasteEnabled, MouseScrollMultiplier,
    OutlineCodebaseSymbolsForAtContextMenu, PreferLowPowerGPU, PreferredGraphicsBackend,
    QuakeModeSettings, ScrollSettings, SelectionSettings, ShowAutosuggestionIgnoreButton,
    ShowTerminalInputMessageBar, SshSettings, SyntaxHighlighting, TabBehavior, VimModeEnabled,
    VimStatusBar, VimUnnamedSystemClipboard, DEFAULT_QUAKE_MODE_SIZE_PERCENTAGES,
    QUAKE_WINDOW_AUTOHIDE_SUPPORTED,
};
use crate::terminal::alt_screen_reporting::{
    AltScreenReporting, FocusReportingEnabled, MouseReportingEnabled, ScrollReportingEnabled,
};
use crate::terminal::general_settings::{
    AutoOpenCodeReviewPaneOnFirstAgentChange, GeneralSettings, LinkTooltip, LoginItem,
    QuitOnLastWindowClosed, RestoreSession, ShowWarningBeforeQuitting,
};
use crate::terminal::keys_settings::{
    ActivationHotkeyEnabled, CtrlTabBehaviorSetting, KeysSettings, KeysSettingsChangedEvent,
};
#[cfg(feature = "local_tty")]
use crate::terminal::session_settings::StartupShellOverride;
use crate::terminal::session_settings::{
    Notifications, NotificationsMode, NotificationsSettings, SessionSettings,
    SessionSettingsChangedEvent, ShouldConfirmCloseSession,
};
use crate::terminal::settings::{
    MaximumGridSize, ShowTerminalZeroStateBlock, TerminalSettings, UseAudibleBell,
};
use crate::terminal::{BlockListSettings, SnackbarEnabled};
use crate::undo_close::UndoCloseSettings;
use crate::user_config::{WarpConfig, WarpConfigUpdateEvent};
use crate::util::bindings::{
    keybinding_name_to_display_string, reset_keybinding_to_default, set_custom_keybinding,
};
use crate::view_components::{Dropdown, DropdownItem, FilterableDropdown};
use crate::workspace::tab_settings::{NewTabPlacement, TabSettings};
use crate::workspace::WorkspaceAction;
use crate::{appearance::Appearance, settings::native_preference::NativePreferenceSettings};
use crate::{editor::EditorView, settings::native_preference::UserNativePreference};
use crate::{features::FeatureFlag, terminal::settings::TerminalSettingsChangedEvent};
use crate::{report_if_error, send_telemetry_from_ctx, themes, GlobalResourceHandles};
use crate::{root_view::QuakeModePinPosition, workspace::tab_settings::TabSettingsChangedEvent};
use ::settings::{Setting, ToggleableSetting};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use strum::IntoEnumIterator;
use warp_core::channel::ChannelState;
use warp_core::semantic_selection::{
    SemanticSelection, SemanticSelectionChangedEvent, SmartSelectEnabled,
};
use warpui::elements::{
    Align, Border, ChildView, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Dismiss,
    Element, Empty, EventHandler, Fill, Flex, Hoverable, MainAxisAlignment, MainAxisSize,
    MouseState, MouseStateHandle, ParentElement, Radius, Shrinkable, Text,
};
use warpui::keymap::{ContextPredicate, FixedBinding, Keystroke};
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::ui_components::switch::SwitchStateHandle;
use warpui::{
    Action, AppContext, DisplayIdx, Entity, EventContext, ModelHandle, SingletonEntity, Tracked,
    TypedActionView, View, ViewContext, ViewHandle, WindowId,
};

cfg_if::cfg_if! {
    if #[cfg(target_os = "macos")] {
        static EXTRA_META_KEYS_LEFT_TEXT: &str = "Left Option key is Meta";
        static EXTRA_META_KEYS_RIGHT_TEXT: &str = "Right Option key is Meta";
    } else {
        static EXTRA_META_KEYS_LEFT_TEXT: &str = "Left Alt key is Meta";
        static EXTRA_META_KEYS_RIGHT_TEXT: &str = "Right Alt key is Meta";
    }
}

pub fn init_actions_from_parent_view<T: Action + Clone>(
    app: &mut AppContext,
    context: &ContextPredicate,
    builder: fn(SettingsAction) -> T,
) {
    use warpui::keymap::macros::*;

    // Add all of the toggle settings from the Features Page that you want to show up on the Command Palette here.
    let mut toggle_binding_pairs = vec![
        ToggleSettingActionPair::new(
            "copy on select within the terminal",
            builder(SettingsAction::FeaturesPageToggle(
                FeaturesPageAction::ToggleCopyOnSelect,
            )),
            context,
            flags::COPY_ON_SELECT_CONTEXT_FLAG,
        ),
        ToggleSettingActionPair::new(
            "linux selection clipboard",
            builder(SettingsAction::FeaturesPageToggle(
                FeaturesPageAction::ToggleLinuxClipboardSelection,
            )),
            context,
            flags::LINUX_SELECTION_CLIPBOARD_FLAG,
        )
        .is_supported_on_current_platform(
            SelectionSettings::as_ref(app)
                .linux_selection_clipboard
                .is_supported_on_current_platform(),
        ),
        ToggleSettingActionPair::new(
            "autocomplete quotes, parentheses, and brackets",
            builder(SettingsAction::FeaturesPageToggle(
                FeaturesPageAction::ToggleAutocompleteSymbols,
            )),
            context,
            flags::AUTOCOMPLETE_SYMBOLS_CONTEXT_FLAG,
        )
        .is_supported_on_current_platform(
            AppEditorSettings::as_ref(app)
                .autocomplete_symbols
                .is_supported_on_current_platform(),
        ),
        ToggleSettingActionPair::new(
            "restore windows, tabs, and panes on startup",
            builder(SettingsAction::FeaturesPageToggle(
                FeaturesPageAction::ToggleRestoreSession,
            )),
            context,
            flags::RESTORE_SESSION_CONTEXT_FLAG,
        ),
        ToggleSettingActionPair::new(
            EXTRA_META_KEYS_LEFT_TEXT,
            builder(SettingsAction::FeaturesPageToggle(
                FeaturesPageAction::ToggleLeftMetaKey,
            )),
            context,
            flags::EXTRA_META_KEYS_LEFT_CONTEXT_FLAG,
        )
        .is_supported_on_current_platform(
            KeysSettings::as_ref(app)
                .extra_meta_keys
                .is_supported_on_current_platform(),
        ),
        ToggleSettingActionPair::new(
            EXTRA_META_KEYS_RIGHT_TEXT,
            builder(SettingsAction::FeaturesPageToggle(
                FeaturesPageAction::ToggleRightMetaKey,
            )),
            context,
            flags::EXTRA_META_KEYS_RIGHT_CONTEXT_FLAG,
        )
        .is_supported_on_current_platform(
            KeysSettings::as_ref(app)
                .extra_meta_keys
                .is_supported_on_current_platform(),
        ),
        ToggleSettingActionPair::new(
            "scroll reporting",
            builder(SettingsAction::FeaturesPageToggle(
                FeaturesPageAction::ToggleScrollReporting,
            )),
            context,
            flags::SCROLL_REPORTING_CONTEXT_FLAG,
        )
        .is_supported_on_current_platform(
            AltScreenReporting::as_ref(app)
                .scroll_reporting_enabled
                .is_supported_on_current_platform(),
        ),
        ToggleSettingActionPair::new(
            "completions while typing",
            builder(SettingsAction::FeaturesPageToggle(
                FeaturesPageAction::ToggleCompletionsOpenWhileTyping,
            )),
            context,
            flags::COMPLETIONS_OPEN_WHILE_TYPING_CONTEXT_FLAG,
        )
        .is_supported_on_current_platform(
            InputSettings::as_ref(app)
                .completions_open_while_typing
                .is_supported_on_current_platform(),
        ),
        ToggleSettingActionPair::new(
            "command corrections",
            builder(SettingsAction::FeaturesPageToggle(
                FeaturesPageAction::ToggleCommandCorrections,
            )),
            context,
            flags::COMMAND_CORRECTIONS_CONTEXT_FLAG,
        )
        .is_supported_on_current_platform(
            InputSettings::as_ref(app)
                .command_corrections
                .is_supported_on_current_platform(),
        ),
        ToggleSettingActionPair::new(
            "error underlining",
            builder(SettingsAction::FeaturesPageToggle(
                FeaturesPageAction::ToggleErrorUnderlining,
            )),
            context,
            flags::ERROR_UNDERLINING_FLAG,
        )
        .is_supported_on_current_platform(
            InputSettings::as_ref(app)
                .error_underlining
                .is_supported_on_current_platform(),
        ),
        ToggleSettingActionPair::new(
            "syntax highlighting",
            builder(SettingsAction::FeaturesPageToggle(
                FeaturesPageAction::ToggleSyntaxHighlighting,
            )),
            context,
            flags::SYNTAX_HIGHLIGHTING_FLAG,
        )
        .is_supported_on_current_platform(
            InputSettings::as_ref(app)
                .syntax_highlighting
                .is_supported_on_current_platform(),
        ),
        ToggleSettingActionPair::new(
            "audible terminal bell",
            builder(SettingsAction::FeaturesPageToggle(
                FeaturesPageAction::ToggleUseAudibleBell,
            )),
            context,
            flags::USE_AUDIBLE_BELL_CONTEXT_FLAG,
        )
        .is_supported_on_current_platform(
            TerminalSettings::as_ref(app)
                .use_audible_bell
                .is_supported_on_current_platform(),
        ),
        ToggleSettingActionPair::new(
            "autosuggestions",
            builder(SettingsAction::FeaturesPageToggle(
                FeaturesPageAction::ToggleAutosuggestions,
            )),
            context,
            flags::AUTOSUGGESTIONS_ENABLED_FLAG,
        ),
        ToggleSettingActionPair::new(
            "autosuggestion keybinding hint",
            builder(SettingsAction::FeaturesPageToggle(
                FeaturesPageAction::ToggleAutosuggestionKeybindingHint,
            )),
            context,
            flags::AUTOSUGGESTION_KEYBINDING_HINT_FLAG,
        ),
    ];

    if !FeatureFlag::SSHTmuxWrapper.is_enabled() {
        toggle_binding_pairs.push(ToggleSettingActionPair::new(
            "Warp SSH wrapper",
            builder(SettingsAction::FeaturesPageToggle(
                #[allow(deprecated)]
                FeaturesPageAction::ToggleSshWrapper,
            )),
            context,
            #[allow(deprecated)]
            flags::LEGACY_SSH_WRAPPER_CONTEXT_FLAG,
        ))
    }

    toggle_binding_pairs.push(ToggleSettingActionPair::new(
        "show tooltip on click on links",
        builder(SettingsAction::FeaturesPageToggle(
            FeaturesPageAction::ToggleLinkTooltip,
        )),
        context,
        flags::LINK_TOOLTIP_CONTEXT_FLAG,
    ));

    toggle_binding_pairs.push(
        ToggleSettingActionPair::new(
            "quit warning modal",
            builder(SettingsAction::FeaturesPageToggle(
                FeaturesPageAction::ToggleShowWarningBeforeQuitting,
            )),
            context,
            flags::QUIT_WARNING_MODAL,
        )
        .is_supported_on_current_platform(
            GeneralSettings::as_ref(app)
                .show_warning_before_quitting
                .is_supported_on_current_platform(),
        ),
    );

    toggle_binding_pairs.push(
        ToggleSettingActionPair::new(
            "alias expansion",
            builder(SettingsAction::FeaturesPageToggle(
                FeaturesPageAction::ToggleAliasExpansion,
            )),
            context,
            flags::ALIAS_EXPANSION_FLAG,
        )
        .is_supported_on_current_platform(
            AliasExpansionSettings::as_ref(app)
                .alias_expansion_enabled
                .is_supported_on_current_platform(),
        ),
    );

    toggle_binding_pairs.push(
        ToggleSettingActionPair::new(
            "middle-click paste",
            builder(SettingsAction::FeaturesPageToggle(
                FeaturesPageAction::ToggleMiddleClickPaste,
            )),
            context,
            flags::MIDDLE_CLICK_PASTE_FLAG,
        )
        .is_supported_on_current_platform(
            SelectionSettings::as_ref(app)
                .middle_click_paste_enabled
                .is_supported_on_current_platform(),
        ),
    );

    toggle_binding_pairs.push(
        ToggleSettingActionPair::new(
            "code as default editor",
            builder(SettingsAction::FeaturesPageToggle(
                FeaturesPageAction::ToggleCodeAsDefaultEditor,
            )),
            context,
            flags::CODE_AS_DEFAULT_EDITOR,
        )
        .is_supported_on_current_platform(
            CodeSettings::as_ref(app)
                .code_as_default_editor
                .is_supported_on_current_platform(),
        ),
    );

    toggle_binding_pairs.push(
        ToggleSettingActionPair::new(
            "input hint text",
            builder(SettingsAction::FeaturesPageToggle(
                FeaturesPageAction::ToggleShowInputHintText,
            )),
            context,
            flags::SHOW_INPUT_HINT_TEXT_CONTEXT_FLAG,
        )
        .is_supported_on_current_platform(
            InputSettings::as_ref(app)
                .show_hint_text
                .is_supported_on_current_platform(),
        ),
    );

    toggle_binding_pairs.push(
        ToggleSettingActionPair::new(
            "editing commands with Vim keybindings",
            builder(SettingsAction::FeaturesPageToggle(
                FeaturesPageAction::ToggleVimMode,
            )),
            context,
            flags::VIM_MODE_CONTEXT_FLAG,
        )
        .is_supported_on_current_platform(
            AppEditorSettings::as_ref(app)
                .vim_mode
                .is_supported_on_current_platform(),
        ),
    );

    toggle_binding_pairs.push(
        ToggleSettingActionPair::new(
            "Vim unnamed register as system clipboard",
            builder(SettingsAction::FeaturesPageToggle(
                FeaturesPageAction::ToggleVimUnnamedSystemClipboard,
            )),
            &(context.to_owned() & id!(flags::VIM_MODE_CONTEXT_FLAG)),
            flags::VIM_UNNAMED_SYSTEM_CLIPBOARD,
        )
        .is_supported_on_current_platform(
            AppEditorSettings::as_ref(app)
                .vim_unnamed_system_clipboard
                .is_supported_on_current_platform(),
        ),
    );

    toggle_binding_pairs.push(
        ToggleSettingActionPair::new(
            "Vim status bar",
            builder(SettingsAction::FeaturesPageToggle(
                FeaturesPageAction::ToggleVimStatusBar,
            )),
            &(context.to_owned() & id!(flags::VIM_MODE_CONTEXT_FLAG)),
            flags::VIM_SHOW_STATUS_BAR,
        )
        .is_supported_on_current_platform(
            AppEditorSettings::as_ref(app)
                .vim_status_bar
                .is_supported_on_current_platform(),
        ),
    );

    toggle_binding_pairs.push(
        ToggleSettingActionPair::new(
            "focus reporting",
            builder(SettingsAction::FeaturesPageToggle(
                FeaturesPageAction::ToggleFocusReporting,
            )),
            context,
            flags::FOCUS_REPORTING_CONTEXT_FLAG,
        )
        .is_supported_on_current_platform(
            AltScreenReporting::as_ref(app)
                .focus_reporting_enabled
                .is_supported_on_current_platform(),
        ),
    );

    toggle_binding_pairs.push(ToggleSettingActionPair::new(
        "smart select",
        builder(SettingsAction::FeaturesPageToggle(
            FeaturesPageAction::ToggleSmartSelection,
        )),
        context,
        flags::SMART_SELECT_FLAG,
    ));

    toggle_binding_pairs.push(
        ToggleSettingActionPair::new(
            "terminal input message line",
            builder(SettingsAction::FeaturesPageToggle(
                FeaturesPageAction::ToggleShowTerminalInputMessageLine,
            )),
            context,
            flags::SHOW_TERMINAL_INPUT_MESSAGE_LINE_FLAG,
        )
        .with_enabled(|| FeatureFlag::AgentView.is_enabled()),
    );

    if FeatureFlag::AgentView.is_enabled() && AISettings::as_ref(app).is_any_ai_enabled(app) {
        toggle_binding_pairs.push(
            ToggleSettingActionPair::new(
                "slash commands in terminal mode",
                builder(SettingsAction::FeaturesPageToggle(
                    FeaturesPageAction::ToggleSlashCommandsInTerminalMode,
                )),
                context,
                flags::SLASH_COMMANDS_IN_TERMINAL_FLAG,
            )
            .is_supported_on_current_platform(
                InputSettings::as_ref(app)
                    .enable_slash_commands_in_terminal
                    .is_supported_on_current_platform(),
            ),
        );
    }

    if GPUState::as_ref(app).is_low_power_gpu_available() {
        toggle_binding_pairs.push(
            ToggleSettingActionPair::new(
                "integrated GPU rendering (low power)",
                builder(SettingsAction::FeaturesPageToggle(
                    FeaturesPageAction::TogglePreferLowPowerGPU,
                )),
                context,
                flags::PREFER_LOW_POWER_GPU_FLAG,
            )
            .is_supported_on_current_platform(
                GPUSettings::as_ref(app)
                    .prefer_low_power_gpu
                    .is_supported_on_current_platform(),
            ),
        );
    }

    #[cfg(target_os = "linux")]
    {
        if windowing_system_is_customizable(app) {
            toggle_binding_pairs.push(
                ToggleSettingActionPair::new(
                    "Wayland for window management",
                    builder(SettingsAction::FeaturesPageToggle(
                        FeaturesPageAction::ToggleForceX11,
                    )),
                    context,
                    flags::ALLOW_NATIVE_WAYLAND,
                )
                .is_supported_on_current_platform(cfg!(target_os = "linux")),
            );
        }
    }

    ToggleSettingActionPair::add_toggle_setting_action_pairs_as_bindings(toggle_binding_pairs, app);

    app.register_fixed_bindings([FixedBinding::empty(
        "Configure Global Hotkey",
        WorkspaceAction::ScrollToSettingsWidget {
            page: SettingsSection::Features,
            widget_id: GlobalHotkeyWidget::static_widget_id(),
        },
        id!("Workspace"),
    )]);

    if DefaultTerminal::can_warp_become_default() {
        app.register_fixed_bindings([FixedBinding::empty(
            "Make Warp the default terminal",
            builder(SettingsAction::FeaturesPageToggle(
                FeaturesPageAction::MakeWarpDefaultTerminal,
            )),
            context.to_owned() & !id!(flags::WARP_IS_DEFAULT_TERMINAL),
        )]);
    }
}

#[derive(Clone, Debug)]
pub enum FeaturesPageAction {
    ToggleCopyOnSelect,
    ToggleNotifications,
    ToggleRestoreSession,
    ToggleAutocompleteSymbols,
    ToggleLinuxClipboardSelection,
    ToggleOpenLinksInDesktopApp,
    #[deprecated]
    ToggleSshWrapper,
    ToggleSnackbar,
    ToggleLinkTooltip,
    ToggleCompletionsOpenWhileTyping,
    ToggleCommandCorrections,
    ToggleErrorUnderlining,
    ToggleSyntaxHighlighting,
    ToggleAliasExpansion,
    ToggleMiddleClickPaste,
    ToggleCodeAsDefaultEditor,
    ToggleShowInputHintText,
    ToggleUseAudibleBell,
    ToggleShowTerminalZeroStateBlock,
    TogglePreferLowPowerGPU,
    ToggleVimMode,
    ToggleVimUnnamedSystemClipboard,
    ToggleVimStatusBar,
    ActivationKeybindEditorClicked,
    ActivationKeybindEditorCancel,
    ActivationKeybindEditorSave,
    ActivationKeystrokeDefined(Keystroke),
    QuakeKeystrokeDefined(Keystroke),
    QuakeKeybindEditorClicked,
    QuakeKeybindEditorCancel,
    QuakeKeybindEditorSave,
    QuakeEditorSetPinPosition(QuakeModePinPosition),
    QuakeEditorSetPinScreen(Option<DisplayIdx>),
    QuakeEditorSetWidthPercentage,
    QuakeEditorSetHeightPercentage,
    QuakeEditorResetWidthHeight,
    QuakeEditorTogglePinWindow,
    OpenUrl(String),
    SetExtraMetaKeys(ExtraMetaKeys),
    ToggleLeftMetaKey,
    ToggleRightMetaKey,
    ToggleMouseReporting,
    ToggleGlobalWorkflowsInUniversalSearch,
    ToggleScrollReporting,
    ToggleFocusReporting,
    ToggleLongRunningNotifications,
    SetLongRunningNotificationThreshold,
    /// Legacy. To be combined with `ToggleNeedsAttentionNotifications` when desktop notifs are unflagged.
    TogglePasswordPromptNotifications,
    ToggleAgentTaskCompletedNotifications,
    ToggleNeedsAttentionNotifications,
    ToggleNotificationSound,
    SetNotificationToastDuration,
    ToggleShowWarningBeforeQuitting,
    ToggleLoginItem,
    ToggleQuitOnLastWindowClosed,
    ToggleSmartSelection,
    SetWordCharAllowlist,
    ResetWordCharAllowlist,
    SetGlobalHotkeyMode(GlobalHotkeyMode),
    SetTabBehavior(TabBehavior),
    SetCtrlTabBehavior(CtrlTabBehavior),
    SetPreferredGraphicsBackend(Option<GraphicsBackend>),
    SetNewTabPlacement(NewTabPlacement),
    SetDefaultSessionMode(DefaultSessionMode),
    SetDefaultTabConfig(String),
    SearchForKeybinding(String),
    ToggleAutosuggestions,
    ToggleConfirmCloseSession,
    ToggleShowChangelogAfterUpdate,
    #[cfg(target_os = "linux")]
    ToggleForceX11,
    ToggleAutosuggestionKeybindingHint,
    ToggleShowAutosuggestionIgnoreButton,
    ToggleAtContextMenuInTerminalMode,
    ToggleSlashCommandsInTerminalMode,
    ToggleOutlineCodebaseSymbolsForAtContextMenu,
    ToggleAutoOpenCodeReviewPane,
    ToggleShowTerminalInputMessageLine,
    ToggleAgentInAppNotifications,
    MakeWarpDefaultTerminal,
}

lazy_static! {
    static ref TAB_KEYSTROKE: Keystroke = Keystroke {
        key: "tab".to_string(),
        ..Default::default()
    };
    static ref CTRL_SPACE_KEYSTROKE: Keystroke = Keystroke {
        key: " ".into(),
        ctrl: true,
        ..Default::default()
    };
}

/// Used for styling notification settings
const NOTIFICATION_CHECKBOX_MARGIN_RIGHT: f32 = 5.;
const NOTIFICATION_EDITOR_MARGIN: f32 = 5.;

const NOTIFICATIONS_DOCS_URL: &str = "https://docs.warp.dev/terminal/more-features/notifications";

/// WARNING: this constant was computed manually by determining the pixel width
/// of the quake mode dropdowns based on the number of expanded items in the flex row.
/// This should be adjusted if the flex row is changed in any way!
const QUAKE_DROPDOWN_WIDTH: f32 = 130.;

const MAX_BLOCK_SIZE_INPUT_BOX_WIDTH: f32 = 80.;

const MIN_MAX_GRID_SIZE: usize = 100;

const MOUSE_SCROLL_EDITOR_WIDTH: f32 = 40.;

const MIN_MOUSE_SCROLL_MULTIPLIER: f32 = 1.0;
const MAX_MOUSE_SCROLL_MULTIPLIER: f32 = 20.0;

const TAB_KEYSTROKE_STR: &str = "Tab";

/// Function to get maximum value for max grid size: 10 million for dogfood/dev builds,
/// 1 million for release builds.
///
/// TODO: address the use of f32 in blocklist rendering code that leads to precision errors
/// when the number of lines gets too high.
fn max_max_grid_size() -> usize {
    if ChannelState::enable_debug_features() {
        10_000_000
    } else {
        1_000_000
    }
}

fn block_maximum_rows_description() -> String {
    let max_rows = if ChannelState::enable_debug_features() {
        "10 million"
    } else {
        "1 million"
    };

    format!(
        "Setting the limit above 100k lines may impact performance. Maximum rows supported is {max_rows}."
    )
}

fn to_string(b: bool) -> String {
    format!("{b}")
}

impl FeaturesPageAction {
    fn telemetry_event(&self, ctx: &AppContext) -> TelemetryEvent {
        let workflow_settings = CommandSearchSettings::as_ref(ctx);
        let reporting_settings = AltScreenReporting::as_ref(ctx);
        let selection_settings = SelectionSettings::as_ref(ctx);
        let input_settings = InputSettings::as_ref(ctx);
        let ssh_settings = SshSettings::as_ref(ctx);
        let keys_settings = KeysSettings::as_ref(ctx);
        match self {
            Self::ToggleCopyOnSelect => TelemetryEvent::FeaturesPageAction {
                action: "ToggleCopyOnSelect".to_string(),
                value: to_string(selection_settings.copy_on_select_enabled()),
            },
            Self::ToggleOpenLinksInDesktopApp => TelemetryEvent::FeaturesPageAction {
                action: "ToggleOpenLinksInDesktopApp".to_string(),
                value: to_string(matches!(
                    NativePreferenceSettings::as_ref(ctx)
                        .user_native_redirect_preference
                        .value(),
                    UserNativePreference::Desktop
                )),
            },
            Self::ToggleSnackbar => {
                let settings = BlockListSettings::as_ref(ctx);
                TelemetryEvent::FeaturesPageAction {
                    action: "ToggleSnackbar".to_string(),
                    value: to_string(*settings.snackbar_enabled),
                }
            }
            Self::ToggleGlobalWorkflowsInUniversalSearch => TelemetryEvent::FeaturesPageAction {
                action: "ToggleGlobalWorkflowsInUniversalSearch".to_string(),
                value: to_string(*workflow_settings.show_global_workflows_in_universal_search),
            },
            Self::ToggleNotifications => TelemetryEvent::FeaturesPageAction {
                action: "ToggleNotifications".to_string(),
                value: to_string(matches!(
                    SessionSettings::as_ref(ctx).notifications.mode,
                    NotificationsMode::Enabled
                )),
            },
            Self::ToggleRestoreSession => {
                TelemetryEvent::ToggleRestoreSession(*GeneralSettings::as_ref(ctx).restore_session)
            }
            Self::ToggleAutocompleteSymbols => TelemetryEvent::FeaturesPageAction {
                action: "ToggleAutocompleteSymbols".to_string(),
                value: to_string(*AppEditorSettings::as_ref(ctx).autocomplete_symbols),
            },
            #[allow(deprecated)]
            Self::ToggleSshWrapper => TelemetryEvent::FeaturesPageAction {
                action: "ToggleSshWrapper".to_string(),
                value: to_string(*ssh_settings.enable_legacy_ssh_wrapper.value()),
            },
            Self::SetGlobalHotkeyMode(mode) => TelemetryEvent::FeaturesPageAction {
                action: "SetGlobalHotkeyMode".to_string(),
                value: format!("{mode:?}"),
            },
            Self::ToggleLinkTooltip => TelemetryEvent::FeaturesPageAction {
                action: "ToggleLinkTooltip".to_string(),
                value: to_string(*GeneralSettings::as_ref(ctx).link_tooltip),
            },
            Self::ToggleCompletionsOpenWhileTyping => TelemetryEvent::FeaturesPageAction {
                action: "ToggleCompletionsOpenWhileTyping".to_string(),
                value: to_string(*input_settings.completions_open_while_typing.value()),
            },
            Self::ToggleCommandCorrections => TelemetryEvent::FeaturesPageAction {
                action: "ToggleCommandCorrections".to_string(),
                value: to_string(*input_settings.command_corrections.value()),
            },
            Self::ToggleErrorUnderlining => TelemetryEvent::FeaturesPageAction {
                action: "ToggleErrorUnderlining".to_string(),
                value: to_string(*input_settings.error_underlining.value()),
            },
            Self::ToggleSyntaxHighlighting => TelemetryEvent::FeaturesPageAction {
                action: "ToggleSyntaxHighlighting".to_string(),
                value: to_string(*input_settings.syntax_highlighting.value()),
            },
            Self::ToggleAliasExpansion => {
                let settings = AliasExpansionSettings::as_ref(ctx);
                TelemetryEvent::FeaturesPageAction {
                    action: "ToggleAliasExpansion".to_string(),
                    value: to_string(*settings.alias_expansion_enabled),
                }
            }
            Self::ToggleMiddleClickPaste => {
                let settings = SelectionSettings::as_ref(ctx);
                TelemetryEvent::FeaturesPageAction {
                    action: "ToggleMiddleClickPaste".to_string(),
                    value: to_string(*settings.middle_click_paste_enabled),
                }
            }
            Self::ToggleCodeAsDefaultEditor => TelemetryEvent::FeaturesPageAction {
                action: "ToggleCodeAsDefaultEditor".to_string(),
                value: to_string(*CodeSettings::as_ref(ctx).code_as_default_editor.value()),
            },
            Self::ToggleShowInputHintText => {
                let settings = InputSettings::as_ref(ctx);
                TelemetryEvent::FeaturesPageAction {
                    action: "ToggleShowInputHintText".to_string(),
                    value: to_string(*settings.show_hint_text),
                }
            }
            Self::ToggleShowTerminalInputMessageLine => {
                let settings = InputSettings::as_ref(ctx);
                TelemetryEvent::FeaturesPageAction {
                    action: "ToggleShowTerminalInputMessageLine".to_string(),
                    value: to_string(settings.is_terminal_input_message_bar_enabled()),
                }
            }
            Self::ActivationKeybindEditorClicked => TelemetryEvent::FeaturesPageAction {
                action: "ActivationKeybindEditorClicked".to_string(),
                value: String::new(),
            },
            Self::ActivationKeybindEditorCancel => TelemetryEvent::FeaturesPageAction {
                action: "ActivationKeybindEditorCancel".to_string(),
                value: String::new(),
            },
            Self::ActivationKeybindEditorSave => TelemetryEvent::FeaturesPageAction {
                action: "ActivationKeybindEditorSave".to_string(),
                value: String::new(),
            },
            Self::ActivationKeystrokeDefined(keystroke) => TelemetryEvent::FeaturesPageAction {
                action: "ActivationKeystrokeDefined".to_string(),
                value: keystroke.normalized(),
            },
            Self::QuakeKeybindEditorClicked => TelemetryEvent::FeaturesPageAction {
                action: "QuakeKeybindEditorClicked".to_string(),
                value: String::new(),
            },
            Self::QuakeKeystrokeDefined(keystroke) => TelemetryEvent::FeaturesPageAction {
                action: "QuakeKeystrokeDefined".to_string(),
                value: keystroke.normalized(),
            },
            Self::QuakeKeybindEditorCancel => TelemetryEvent::FeaturesPageAction {
                action: "QuakeKeybindEditorCancel".to_string(),
                value: String::new(),
            },
            Self::QuakeKeybindEditorSave => TelemetryEvent::FeaturesPageAction {
                action: "QuakeKeybindEditorSave".to_string(),
                value: String::new(),
            },
            Self::OpenUrl(url) => TelemetryEvent::FeaturesPageAction {
                action: "OpenUrl".to_string(),
                value: url.clone(),
            },
            Self::SetExtraMetaKeys(extra_metas) => TelemetryEvent::FeaturesPageAction {
                action: "SetExtraMetaKeys".to_string(),
                value: format!("{extra_metas:?}"),
            },
            Self::ToggleLeftMetaKey => TelemetryEvent::FeaturesPageAction {
                action: "ToggleLeftMetaKey".to_string(),
                value: to_string(keys_settings.extra_meta_keys.left_alt),
            },
            Self::ToggleRightMetaKey => TelemetryEvent::FeaturesPageAction {
                action: "ToggleRightMetaKey".to_string(),
                value: to_string(keys_settings.extra_meta_keys.right_alt),
            },
            Self::ToggleMouseReporting => TelemetryEvent::FeaturesPageAction {
                action: "ToggleMouseReporting".to_string(),
                value: to_string(*reporting_settings.mouse_reporting_enabled),
            },
            Self::ToggleScrollReporting => TelemetryEvent::FeaturesPageAction {
                action: "ToggleScrollReporting".to_string(),
                value: to_string(*reporting_settings.scroll_reporting_enabled),
            },
            Self::ToggleFocusReporting => TelemetryEvent::FeaturesPageAction {
                action: "ToggleFocusReporting".to_string(),
                value: to_string(*reporting_settings.focus_reporting_enabled),
            },
            Self::QuakeEditorSetPinPosition(position) => TelemetryEvent::FeaturesPageAction {
                action: "QuakeEditorSetPinPosition".to_string(),
                value: format!("{position:?}"),
            },
            Self::QuakeEditorSetPinScreen(screen) => TelemetryEvent::FeaturesPageAction {
                action: "QuakeEditorSetPinScreen".to_string(),
                value: screen
                    .map(|idx| format!("{idx}"))
                    .unwrap_or_else(|| "Active Screen".into()),
            },
            Self::QuakeEditorResetWidthHeight => TelemetryEvent::FeaturesPageAction {
                action: "QuakeEditorResetWidthHeight".to_string(),
                value: String::new(),
            },
            Self::QuakeEditorSetWidthPercentage | Self::QuakeEditorSetHeightPercentage => {
                TelemetryEvent::FeaturesPageAction {
                    action: "QuakeEditorSetSizePercentage".to_string(),
                    value: format!(
                        "width: {:?}, height: {:?}",
                        KeysSettings::handle(ctx)
                            .as_ref(ctx)
                            .quake_mode_settings
                            .width_percentage(),
                        KeysSettings::handle(ctx)
                            .as_ref(ctx)
                            .quake_mode_settings
                            .height_percentage()
                    ),
                }
            }
            Self::QuakeEditorTogglePinWindow => TelemetryEvent::FeaturesPageAction {
                action: "QuakeEditorTogglePinWindow".to_string(),
                value: to_string(
                    KeysSettings::as_ref(ctx)
                        .quake_mode_settings
                        .hide_window_when_unfocused,
                ),
            },
            Self::ToggleLongRunningNotifications => TelemetryEvent::FeaturesPageAction {
                action: "ToggleLongRunningNotifications".to_string(),
                value: to_string(
                    SessionSettings::as_ref(ctx)
                        .notifications
                        .is_long_running_enabled,
                ),
            },
            Self::SetLongRunningNotificationThreshold => TelemetryEvent::FeaturesPageAction {
                action: "SetLongRunningNotificationThreshold".to_string(),
                value: format!(
                    "{}s",
                    SessionSettings::handle(ctx)
                        .as_ref(ctx)
                        .notifications
                        .long_running_threshold
                        .as_secs_f32()
                ),
            },
            Self::TogglePasswordPromptNotifications => TelemetryEvent::FeaturesPageAction {
                action: "TogglePasswordPromptNotifications".to_string(),
                value: to_string(
                    SessionSettings::as_ref(ctx)
                        .notifications
                        .is_password_prompt_enabled,
                ),
            },
            Self::ToggleAgentTaskCompletedNotifications => TelemetryEvent::FeaturesPageAction {
                action: "ToggleAgentTaskCompletedNotifications".to_string(),
                value: to_string(
                    SessionSettings::as_ref(ctx)
                        .notifications
                        .is_agent_task_completed_enabled,
                ),
            },
            Self::ToggleNeedsAttentionNotifications => TelemetryEvent::FeaturesPageAction {
                action: "ToggleNeedsAttentionNotifications".to_string(),
                value: to_string(
                    SessionSettings::as_ref(ctx)
                        .notifications
                        .is_needs_attention_enabled,
                ),
            },
            Self::ToggleNotificationSound => TelemetryEvent::FeaturesPageAction {
                action: "ToggleNotificationSound".to_string(),
                value: to_string(
                    SessionSettings::as_ref(ctx)
                        .notifications
                        .play_notification_sound,
                ),
            },
            Self::ToggleShowWarningBeforeQuitting => TelemetryEvent::FeaturesPageAction {
                action: "ToggleShowWarningBeforeQuitting".to_string(),
                value: to_string(
                    *GeneralSettings::as_ref(ctx)
                        .show_warning_before_quitting
                        .value(),
                ),
            },
            Self::ToggleLoginItem => TelemetryEvent::FeaturesPageAction {
                action: "ToggleLoginItem".to_string(),
                value: to_string(*GeneralSettings::as_ref(ctx).add_app_as_login_item.value()),
            },
            Self::ToggleQuitOnLastWindowClosed => TelemetryEvent::FeaturesPageAction {
                action: "ToggleQuitOnLastWindowClosed".to_string(),
                value: to_string(
                    *GeneralSettings::as_ref(ctx)
                        .quit_on_last_window_closed
                        .value(),
                ),
            },
            Self::ToggleSmartSelection => TelemetryEvent::FeaturesPageAction {
                action: "ToggleSmartSelection".to_string(),
                value: to_string(SemanticSelection::as_ref(ctx).smart_select_enabled()),
            },
            Self::SetWordCharAllowlist => TelemetryEvent::FeaturesPageAction {
                action: "SetWordCharAllowlist".to_string(),
                value: SemanticSelection::as_ref(ctx).word_char_allowlist_string(),
            },
            Self::ResetWordCharAllowlist => TelemetryEvent::FeaturesPageAction {
                action: "ResetWordCharAllowlist".to_string(),
                value: String::new(),
            },
            Self::ToggleUseAudibleBell => {
                let terminal_settings = TerminalSettings::as_ref(ctx);
                TelemetryEvent::FeaturesPageAction {
                    action: "ToggleUseAudibleBell".to_string(),
                    value: to_string(*terminal_settings.use_audible_bell),
                }
            }
            Self::ToggleVimMode => TelemetryEvent::FeaturesPageAction {
                action: "ToggleVimMode".to_string(),
                value: to_string(*AppEditorSettings::as_ref(ctx).vim_mode.value()),
            },
            Self::ToggleVimUnnamedSystemClipboard => TelemetryEvent::FeaturesPageAction {
                action: "ToggleVimUnnamedSystemClipboard".to_string(),
                value: to_string(
                    *AppEditorSettings::as_ref(ctx)
                        .vim_unnamed_system_clipboard
                        .value(),
                ),
            },
            Self::ToggleVimStatusBar => TelemetryEvent::FeaturesPageAction {
                action: "ToggleVimStatusBar".to_string(),
                value: to_string(*AppEditorSettings::as_ref(ctx).vim_status_bar.value()),
            },
            Self::SetTabBehavior(tab_behavior) => TelemetryEvent::FeaturesPageAction {
                action: "SetTabBehavior".to_string(),
                value: format!("{tab_behavior:?}"),
            },
            Self::SetCtrlTabBehavior(ctrl_tab_behavior) => TelemetryEvent::FeaturesPageAction {
                action: "SetCtrlTabBehavior".to_string(),
                value: format!("{ctrl_tab_behavior:?}"),
            },
            Self::SetNewTabPlacement(new_tab_placement) => TelemetryEvent::FeaturesPageAction {
                action: "SetNewTabPlacement".to_string(),
                value: format!("{new_tab_placement:?}"),
            },
            Self::SetDefaultSessionMode(mode) => TelemetryEvent::FeaturesPageAction {
                action: "SetDefaultSessionMode".to_string(),
                value: format!("{mode:?}"),
            },
            Self::SetDefaultTabConfig(path) => TelemetryEvent::FeaturesPageAction {
                action: "SetDefaultTabConfig".to_string(),
                value: path.clone(),
            },
            Self::SearchForKeybinding(page_name) => TelemetryEvent::FeaturesPageAction {
                action: "SearchForKeybinding".to_string(),
                value: page_name.clone(),
            },
            Self::ToggleAutosuggestions => TelemetryEvent::FeaturesPageAction {
                action: "ToggleAutosuggestions".to_string(),
                value: to_string(*AppEditorSettings::as_ref(ctx).enable_autosuggestions),
            },
            Self::ToggleAutosuggestionKeybindingHint => TelemetryEvent::FeaturesPageAction {
                action: "ToggleHideAutosuggestionKeybindingHint".to_string(),
                value: to_string(
                    *AppEditorSettings::as_ref(ctx)
                        .autosuggestion_keybinding_hint
                        .value(),
                ),
            },
            Self::ToggleShowAutosuggestionIgnoreButton => TelemetryEvent::FeaturesPageAction {
                action: "ToggleShowAutosuggestionIgnoreButton".to_string(),
                value: to_string(
                    *AppEditorSettings::as_ref(ctx)
                        .show_autosuggestion_ignore_button
                        .value(),
                ),
            },
            Self::TogglePreferLowPowerGPU => {
                let gpu_settings = GPUSettings::as_ref(ctx);
                TelemetryEvent::FeaturesPageAction {
                    action: "TogglePreferLowPowerGPU".to_string(),
                    value: to_string(*gpu_settings.prefer_low_power_gpu.value()),
                }
            }
            Self::SetPreferredGraphicsBackend(backend) => TelemetryEvent::FeaturesPageAction {
                action: "SetPreferredGraphicsBackend".to_string(),
                value: format!("{backend:?}"),
            },
            Self::ToggleConfirmCloseSession => TelemetryEvent::FeaturesPageAction {
                action: "ToggleConfirmCloseSession".to_string(),
                value: to_string(*SessionSettings::as_ref(ctx).should_confirm_close_session),
            },
            Self::ToggleShowTerminalZeroStateBlock => TelemetryEvent::FeaturesPageAction {
                action: "ToggleShowTerminalZeroStateBlock".to_string(),
                value: to_string(*TerminalSettings::as_ref(ctx).show_terminal_zero_state_block),
            },
            Self::ToggleShowChangelogAfterUpdate => {
                let changelog_settings = ChangelogSettings::as_ref(ctx);
                TelemetryEvent::FeaturesPageAction {
                    action: "ToggleShowChangelogAfterUpdate".to_string(),
                    value: to_string(*changelog_settings.show_changelog_after_update),
                }
            }
            Self::ToggleLinuxClipboardSelection => {
                let selection_setting =
                    SelectionSettings::as_ref(ctx).linux_selection_clipboard_enabled();
                TelemetryEvent::FeaturesPageAction {
                    action: "ToggleLinuxClipboardSelection".to_string(),
                    value: to_string(selection_setting),
                }
            }
            #[cfg(target_os = "linux")]
            Self::ToggleForceX11 => {
                let setting = *LinuxAppConfiguration::as_ref(ctx).force_x11.value();
                TelemetryEvent::FeaturesPageAction {
                    action: "ToggleForceX11".to_string(),
                    value: to_string(setting),
                }
            }
            Self::ToggleAtContextMenuInTerminalMode => TelemetryEvent::FeaturesPageAction {
                action: "ToggleAtContextMenuInTerminalMode".to_string(),
                value: to_string(
                    *InputSettings::as_ref(ctx)
                        .at_context_menu_in_terminal_mode
                        .value(),
                ),
            },
            Self::ToggleSlashCommandsInTerminalMode => TelemetryEvent::FeaturesPageAction {
                action: "ToggleSlashCommandsInTerminalMode".to_string(),
                value: to_string(
                    *InputSettings::as_ref(ctx)
                        .enable_slash_commands_in_terminal
                        .value(),
                ),
            },
            Self::ToggleOutlineCodebaseSymbolsForAtContextMenu => {
                TelemetryEvent::FeaturesPageAction {
                    action: "ToggleOutlineCodebaseSymbolsForAtContextMenu".to_string(),
                    value: to_string(
                        *InputSettings::as_ref(ctx)
                            .outline_codebase_symbols_for_at_context_menu
                            .value(),
                    ),
                }
            }
            Self::MakeWarpDefaultTerminal => TelemetryEvent::FeaturesPageAction {
                action: "MakeWarpDefaultTerminal".to_string(),
                value: to_string(DefaultTerminal::as_ref(ctx).is_warp_default()),
            },
            Self::ToggleAutoOpenCodeReviewPane => TelemetryEvent::FeaturesPageAction {
                action: "ToggleAutoOpenCodeReviewPane".to_string(),
                value: to_string(
                    *GeneralSettings::as_ref(ctx).auto_open_code_review_pane_on_first_agent_change,
                ),
            },
            Self::SetNotificationToastDuration => TelemetryEvent::FeaturesPageAction {
                action: "SetNotificationToastDuration".to_string(),
                value: format!(
                    "{}s",
                    *SessionSettings::as_ref(ctx).notification_toast_duration_secs
                ),
            },
            Self::ToggleAgentInAppNotifications => TelemetryEvent::FeaturesPageAction {
                action: "ToggleAgentInAppNotifications".to_string(),
                value: to_string(*AISettings::as_ref(ctx).show_agent_notifications),
            },
        }
    }
}

#[derive(Default)]
struct MouseStateHandles {
    local_only_icon_tooltip_states: RefCell<HashMap<String, MouseStateHandle>>,
    tab_behavior_local_only_icon: MouseStateHandle,
    activation_hotkey_keybinding_editor: MouseStateHandle,
    activation_hotkey_save: MouseStateHandle,
    activation_hotkey_cancel: MouseStateHandle,
    quake_mode_keybinding_editor: MouseStateHandle,
    quake_mode_save: MouseStateHandle,
    quake_mode_cancel: MouseStateHandle,
    quake_mode_width_height_reset: MouseStateHandle,
    quake_mode_pin_window_check: MouseStateHandle,
    long_running_notifications_checkbox: MouseStateHandle,
    agent_task_completed_notifications_checkbox: MouseStateHandle,
    agent_needs_attention_notifications_checkbox: MouseStateHandle,
    agent_in_app_notifications_switch: SwitchStateHandle,
    #[cfg(target_os = "macos")]
    notification_sound_checkbox: MouseStateHandle,
    change_keybinding: MouseStateHandle,
    global_hotkey_link: MouseStateHandle,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum KeybindingEditorState {
    /// The editor needs to be clicked first before you can record a keybinding
    Idle,
    /// The editor is active and currently recording a keybinding
    Recording,
}

pub struct FeaturesPageView {
    page: PageType<Self>,

    global_resource_handles: GlobalResourceHandles,

    button_mouse_states: MouseStateHandles,
    ctrl_tab_behavior_dropdown: ViewHandle<Dropdown<FeaturesPageAction>>,

    global_hotkey_dropdown: ViewHandle<Dropdown<FeaturesPageAction>>,
    activation_hotkey_keybinding_editor_state: KeybindingEditorState,
    activation_hotkey_keybinding: KeyBindingModifyingState,
    quake_mode_keybinding_editor_state: KeybindingEditorState,
    quake_mode_keybinding: KeyBindingModifyingState,
    quake_mode_pin_position_dropdown: ViewHandle<Dropdown<FeaturesPageAction>>,
    quake_mode_pin_screen_dropdown: ViewHandle<Dropdown<FeaturesPageAction>>,
    quake_mode_width_editor: ViewHandle<EditorView>,
    quake_mode_height_editor: ViewHandle<EditorView>,

    notifications_long_running_threshold_editor: ViewHandle<EditorView>,
    notification_toast_duration_editor: ViewHandle<EditorView>,

    #[cfg(feature = "local_tty")]
    working_directory_view: ViewHandle<features::WorkingDirectoryView>,
    #[cfg(feature = "local_tty")]
    startup_shell_view: ViewHandle<features::StartupShellView>,
    undo_close_view: ViewHandle<features::UndoCloseView>,

    max_block_size_input_editor: ViewHandle<EditorView>,
    valid_max_block_size: bool,

    mouse_scroll_input_editor: ViewHandle<EditorView>,
    valid_mouse_scroll_multiplier: bool,

    // Whether or not the SSH wrapper value was changed while the page has been
    // open.
    ssh_wrapper_toggled: bool,

    #[cfg(feature = "local_fs")]
    external_editor_view: ViewHandle<features::ExternalEditorView>,
    word_boundary_editor: ViewHandle<EditorView>,

    tab_behavior_dropdown: ViewHandle<Dropdown<FeaturesPageAction>>,
    graphics_backend_dropdown: ViewHandle<Dropdown<FeaturesPageAction>>,
    new_tab_placement_dropdown: ViewHandle<Dropdown<FeaturesPageAction>>,
    default_session_mode_dropdown: ViewHandle<FilterableDropdown<FeaturesPageAction>>,
    tab_behavior: Tracked<TabBehavior>,
    completions_keystroke: Tracked<String>,
    autosuggestions_keystroke: Tracked<String>,

    window_id: WindowId,

    #[cfg(target_os = "linux")]
    force_x11_changed: bool,
    gpu_power_preference_changed: bool,
    graphics_backend_preference_changed: bool,
}

pub enum FeaturesSettingsPageEvent {
    SearchForKeybinding(String),
    FocusModal,
}

impl Entity for FeaturesPageView {
    type Event = FeaturesSettingsPageEvent;
}

impl TypedActionView for FeaturesPageView {
    type Action = FeaturesPageAction;

    fn handle_action(&mut self, action: &FeaturesPageAction, ctx: &mut ViewContext<Self>) {
        use FeaturesPageAction::*;

        match action {
            SetCtrlTabBehavior(ctrl_tab_behavior) => {
                KeysSettings::handle(ctx).update(ctx, |keys_settings, ctx| {
                    report_if_error!(keys_settings
                        .ctrl_tab_behavior
                        .set_value(*ctrl_tab_behavior, ctx));
                });
            }
            ToggleCopyOnSelect => {
                SelectionSettings::handle(ctx).update(ctx, |selection_settings, ctx| {
                    report_if_error!(selection_settings.copy_on_select.toggle_and_save_value(ctx));
                });
            }
            ToggleSnackbar => {
                BlockListSettings::handle(ctx).update(ctx, |blocklist_settings, ctx| {
                    report_if_error!(blocklist_settings
                        .snackbar_enabled
                        .toggle_and_save_value(ctx));
                });
            }
            ToggleGlobalWorkflowsInUniversalSearch => {
                CommandSearchSettings::handle(ctx).update(ctx, |workflow_settings, ctx| {
                    report_if_error!(workflow_settings
                        .show_global_workflows_in_universal_search
                        .toggle_and_save_value(ctx));
                });
            }
            ToggleCodeAsDefaultEditor => {
                CodeSettings::handle(ctx).update(ctx, |code_settings, ctx| {
                    report_if_error!(code_settings
                        .code_as_default_editor
                        .toggle_and_save_value(ctx));
                })
            }
            ToggleOpenLinksInDesktopApp => {
                NativePreferenceSettings::handle(ctx).update(
                    ctx,
                    |native_preference_settings, ctx| {
                        let new_value = match native_preference_settings
                            .user_native_redirect_preference
                            .value()
                        {
                            UserNativePreference::Desktop => UserNativePreference::Web,
                            UserNativePreference::NotSelected | UserNativePreference::Web => {
                                UserNativePreference::Desktop
                            }
                        };
                        report_if_error!(native_preference_settings
                            .user_native_redirect_preference
                            .set_value(new_value, ctx));
                    },
                );
            }
            ToggleNotifications => {
                ctx.dispatch_typed_action(&WorkspaceAction::ToggleNotifications);
            }
            ToggleRestoreSession => {
                GeneralSettings::handle(ctx).update(ctx, |general_settings, ctx| {
                    report_if_error!(general_settings.restore_session.toggle_and_save_value(ctx))
                })
            }
            ToggleAutocompleteSymbols => {
                AppEditorSettings::handle(ctx).update(ctx, |editor_settings, ctx| {
                    report_if_error!(editor_settings
                        .autocomplete_symbols
                        .toggle_and_save_value(ctx));
                })
            }
            #[allow(deprecated)]
            ToggleSshWrapper => {
                self.ssh_wrapper_toggled = true;
                SshSettings::handle(ctx).update(ctx, |ssh_settings, ctx| {
                    report_if_error!(ssh_settings
                        .enable_legacy_ssh_wrapper
                        .toggle_and_save_value(ctx));
                });
            }
            OpenUrl(url) => {
                ctx.open_url(url.as_str());
            }
            SetGlobalHotkeyMode(mode) => self.set_global_hotkey_mode(mode, ctx),
            ActivationKeybindEditorClicked => {
                ctx.disable_key_bindings_dispatching();
                self.activation_hotkey_keybinding_editor_state = KeybindingEditorState::Recording;
                ctx.notify();
            }
            ActivationKeybindEditorCancel => {
                self.reset_activation_hotkey_keybinding_editor();
                ctx.enable_key_bindings_dispatching();
                ctx.notify();
            }
            ActivationKeybindEditorSave => {
                ctx.enable_key_bindings_dispatching();

                if !self.activation_hotkey_keybinding.is_dirty() {
                    self.activation_hotkey_keybinding_editor_state = KeybindingEditorState::Idle;
                    ctx.notify();
                    return;
                }

                self.disable_activation_hotkey_global_shortcut(ctx);

                self.activation_hotkey_keybinding
                    .current_binding
                    .clone_from(&self.activation_hotkey_keybinding.unsaved_binding);
                self.activation_hotkey_keybinding_editor_state = KeybindingEditorState::Idle;

                self.enable_activation_hotkey_global_shortcut(ctx);

                KeysSettings::handle(ctx).update(ctx, |key_settings, ctx| {
                    key_settings.set_activation_hotkey_keybinding_and_write_to_user_defaults(
                        self.activation_hotkey_keybinding.current_binding.clone(),
                        ctx,
                    )
                });

                ctx.notify();
            }
            ActivationKeystrokeDefined(keystroke) => {
                self.activation_hotkey_keybinding.unsaved_binding = Some(keystroke.clone());
                ctx.notify();
            }
            QuakeKeybindEditorClicked => {
                ctx.disable_key_bindings_dispatching();
                self.quake_mode_keybinding_editor_state = KeybindingEditorState::Recording;
                ctx.notify();
            }
            QuakeKeystrokeDefined(keystroke) => {
                self.quake_mode_keybinding.unsaved_binding = Some(keystroke.clone());
                ctx.notify();
            }
            QuakeKeybindEditorCancel => {
                self.reset_quake_mode_keybinding_editor();
                ctx.enable_key_bindings_dispatching();
                ctx.notify();
            }
            QuakeKeybindEditorSave => {
                ctx.enable_key_bindings_dispatching();

                if !self.quake_mode_keybinding.is_dirty() {
                    self.quake_mode_keybinding_editor_state = KeybindingEditorState::Idle;
                    ctx.notify();
                    return;
                }

                self.disable_quake_mode_global_shortcut(ctx);

                self.quake_mode_keybinding
                    .current_binding
                    .clone_from(&self.quake_mode_keybinding.unsaved_binding);
                self.quake_mode_keybinding_editor_state = KeybindingEditorState::Idle;

                self.enable_quake_mode_global_shortcut(ctx);

                KeysSettings::handle(ctx).update(ctx, |key_settings, ctx| {
                    key_settings.set_quake_mode_keybinding_and_write_to_user_defaults(
                        self.quake_mode_keybinding.current_binding.clone(),
                        ctx,
                    )
                });

                ctx.notify();
            }
            QuakeEditorSetPinPosition(pin_position) => {
                KeysSettings::handle(ctx).update(ctx, |keys_settings, ctx| {
                    keys_settings
                        .set_quake_mode_pin_position_and_write_to_user_defaults(*pin_position, ctx)
                });

                let new_size = KeysSettings::as_ref(ctx)
                    .quake_mode_settings
                    .size_percentages_for_pin_position(pin_position);
                self.quake_mode_width_editor.update(ctx, |editor, ctx| {
                    editor.set_buffer_text(&format!("{}", new_size.width), ctx);
                });
                self.quake_mode_height_editor.update(ctx, |editor, ctx| {
                    editor.set_buffer_text(&format!("{}", new_size.height), ctx);
                });

                ctx.notify();
            }
            QuakeEditorSetPinScreen(pin_screen) => {
                KeysSettings::handle(ctx).update(ctx, |keys_settings, ctx| {
                    keys_settings
                        .set_quake_mode_pin_screen_and_write_to_user_defaults(*pin_screen, ctx)
                });
                ctx.notify();
            }
            QuakeEditorSetHeightPercentage => self.set_height_ratio(ctx),
            QuakeEditorSetWidthPercentage => self.set_width_ratio(ctx),
            QuakeEditorResetWidthHeight => self.reset_size_percentage(ctx),
            QuakeEditorTogglePinWindow => {
                KeysSettings::handle(ctx).update(ctx, |keys_settings, ctx| {
                    keys_settings
                        .toggle_hide_quake_mode_window_when_unfocused_and_write_to_user_defaults(
                            ctx,
                        )
                });
            }
            SetExtraMetaKeys(extra_meta_keys) => {
                KeysSettings::handle(ctx).update(ctx, |keys_settings, ctx| {
                    report_if_error!(keys_settings
                        .extra_meta_keys
                        .set_value(*extra_meta_keys, ctx))
                });
            }
            ToggleLeftMetaKey => {
                let current_meta_keys = *KeysSettings::as_ref(ctx).extra_meta_keys;
                KeysSettings::handle(ctx).update(ctx, |keys_settings, ctx| {
                    report_if_error!(keys_settings
                        .extra_meta_keys
                        .set_value(current_meta_keys.toggle_left_key(), ctx))
                });
            }
            ToggleRightMetaKey => {
                let current_meta_keys = *KeysSettings::as_ref(ctx).extra_meta_keys;
                KeysSettings::handle(ctx).update(ctx, |keys_settings, ctx| {
                    report_if_error!(keys_settings
                        .extra_meta_keys
                        .set_value(current_meta_keys.toggle_right_key(), ctx))
                });
            }
            ToggleMouseReporting => {
                AltScreenReporting::handle(ctx).update(ctx, |reporting, ctx| {
                    reporting
                        .mouse_reporting_enabled
                        .toggle_and_save_value(ctx)
                        .expect("MouseReportingEnabled failed to serialize");
                });
                ctx.notify();
            }
            ToggleScrollReporting => {
                AltScreenReporting::handle(ctx).update(ctx, |reporting, ctx| {
                    reporting
                        .scroll_reporting_enabled
                        .toggle_and_save_value(ctx)
                        .expect("ScrollReportingEnabled failed to serialize");
                });
                ctx.notify();
            }
            ToggleFocusReporting => {
                AltScreenReporting::handle(ctx).update(ctx, |reporting, ctx| {
                    reporting
                        .focus_reporting_enabled
                        .toggle_and_save_value(ctx)
                        .expect("FocusReportingEnabled failed to serialize");
                });
                ctx.notify();
            }
            ToggleLongRunningNotifications => {
                let current_settings = SessionSettings::as_ref(ctx).notifications.value().clone();

                let is_long_running_enabled = !current_settings.is_long_running_enabled;

                SessionSettings::handle(ctx).update(ctx, |settings, ctx| {
                    let new_settings = NotificationsSettings {
                        is_long_running_enabled,
                        ..current_settings
                    };

                    if let Err(e) = settings.notifications.set_value(new_settings, ctx) {
                        log::error!("Failed to persist Notifications setting: {e}");
                    }
                });
                ctx.notify();
            }
            SetLongRunningNotificationThreshold => {
                let user_input = self
                    .notifications_long_running_threshold_editor
                    .as_ref(ctx)
                    .buffer_text(ctx);

                if let Ok(long_running_threshold) = user_input.parse::<f32>() {
                    if long_running_threshold > 0.0 {
                        // TODO: use try_from_secs_32 in the future to avoid previous cmp
                        let current_settings =
                            SessionSettings::as_ref(ctx).notifications.value().clone();
                        SessionSettings::handle(ctx).update(ctx, |settings, ctx| {
                            let new_settings = NotificationsSettings {
                                long_running_threshold: Duration::from_secs_f32(
                                    long_running_threshold,
                                ),
                                ..current_settings
                            };
                            if let Err(e) = settings.notifications.set_value(new_settings, ctx) {
                                log::error!("Error persisting notifications setting: {e}");
                            }
                        });
                    }
                }
            }
            TogglePasswordPromptNotifications => {
                let current_settings = SessionSettings::as_ref(ctx).notifications.value().clone();
                let is_password_prompt_enabled = !current_settings.is_password_prompt_enabled;

                SessionSettings::handle(ctx).update(ctx, |settings, ctx| {
                    let new_settings = NotificationsSettings {
                        is_password_prompt_enabled,
                        ..current_settings
                    };
                    if let Err(e) = settings.notifications.set_value(new_settings, ctx) {
                        log::error!("Error persisting notifications setting: {e}");
                    }
                });
                ctx.notify();
            }
            ToggleAgentTaskCompletedNotifications => {
                let current_settings = SessionSettings::as_ref(ctx).notifications.value().clone();
                let is_agent_task_completed_enabled =
                    !current_settings.is_agent_task_completed_enabled;

                SessionSettings::handle(ctx).update(ctx, |settings, ctx| {
                    let new_settings = NotificationsSettings {
                        is_agent_task_completed_enabled,
                        ..current_settings
                    };
                    if let Err(e) = settings.notifications.set_value(new_settings, ctx) {
                        log::error!("Error persisting notifications setting: {e}");
                    }
                });
                ctx.notify();
            }
            ToggleNeedsAttentionNotifications => {
                let current_settings = SessionSettings::as_ref(ctx).notifications.value().clone();
                let is_agent_needs_attention_enabled = !current_settings.is_needs_attention_enabled;

                SessionSettings::handle(ctx).update(ctx, |settings, ctx| {
                    let new_settings = NotificationsSettings {
                        is_needs_attention_enabled: is_agent_needs_attention_enabled,
                        ..current_settings
                    };
                    if let Err(e) = settings.notifications.set_value(new_settings, ctx) {
                        log::error!("Error persisting notifications setting: {e}");
                    }
                });
                ctx.notify();
            }
            ToggleNotificationSound => {
                let current_settings = SessionSettings::as_ref(ctx).notifications.value().clone();
                let play_notification_sound = !current_settings.play_notification_sound;

                SessionSettings::handle(ctx).update(ctx, |settings, ctx| {
                    let new_settings = NotificationsSettings {
                        play_notification_sound,
                        ..current_settings
                    };
                    if let Err(e) = settings.notifications.set_value(new_settings, ctx) {
                        log::error!("Error persisting notification sound setting: {e}");
                    }
                });
                ctx.notify();
            }
            ToggleAgentInAppNotifications => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(settings.show_agent_notifications.toggle_and_save_value(ctx));
                });
                ctx.notify();
            }
            ToggleCompletionsOpenWhileTyping => {
                InputSettings::handle(ctx).update(ctx, |input_settings, ctx| {
                    report_if_error!(input_settings
                        .completions_open_while_typing
                        .toggle_and_save_value(ctx));
                });
            }
            ToggleCommandCorrections => {
                InputSettings::handle(ctx).update(ctx, |input_settings, ctx| {
                    report_if_error!(input_settings
                        .command_corrections
                        .toggle_and_save_value(ctx));
                });
            }
            ToggleErrorUnderlining => {
                InputSettings::handle(ctx).update(ctx, |input_settings, ctx| {
                    report_if_error!(input_settings.error_underlining.toggle_and_save_value(ctx));
                });
            }
            ToggleSyntaxHighlighting => {
                InputSettings::handle(ctx).update(ctx, |input_settings, ctx| {
                    report_if_error!(input_settings
                        .syntax_highlighting
                        .toggle_and_save_value(ctx));
                });
            }
            ToggleAliasExpansion => {
                AliasExpansionSettings::handle(ctx).update(ctx, |alias_expansion_settings, ctx| {
                    report_if_error!(alias_expansion_settings
                        .alias_expansion_enabled
                        .toggle_and_save_value(ctx));
                });
            }
            ToggleMiddleClickPaste => {
                SelectionSettings::handle(ctx).update(ctx, |selection_settings, ctx| {
                    report_if_error!(selection_settings
                        .middle_click_paste_enabled
                        .toggle_and_save_value(ctx));
                });
            }
            ToggleShowInputHintText => {
                InputSettings::handle(ctx).update(ctx, |input_settings, ctx| {
                    report_if_error!(input_settings.show_hint_text.toggle_and_save_value(ctx));
                });
            }
            ToggleShowTerminalInputMessageLine => {
                InputSettings::handle(ctx).update(ctx, |input_settings, ctx| {
                    report_if_error!(input_settings
                        .show_terminal_input_message_bar
                        .toggle_and_save_value(ctx));
                });
            }
            ToggleLinkTooltip => {
                GeneralSettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(settings.link_tooltip.toggle_and_save_value(ctx));
                });
            }
            ToggleShowWarningBeforeQuitting => {
                GeneralSettings::handle(ctx).update(ctx, |warning_settings, ctx| {
                    report_if_error!(warning_settings
                        .show_warning_before_quitting
                        .toggle_and_save_value(ctx));
                })
            }
            ToggleSmartSelection => {
                SemanticSelection::handle(ctx).update(ctx, |selection, ctx| {
                    report_if_error!(selection.smart_select_enabled.toggle_and_save_value(ctx));
                });
            }
            SetWordCharAllowlist => {
                let word_boundary_allowlist = self
                    .word_boundary_editor
                    .read(ctx, |editor, ctx| editor.buffer_text(ctx));

                SemanticSelection::handle(ctx).update(ctx, |selection, ctx| {
                    report_if_error!(selection
                        .word_char_allowlist
                        .set_value(word_boundary_allowlist, ctx));
                });
            }
            ResetWordCharAllowlist => {
                SemanticSelection::handle(ctx).update(ctx, |selection, ctx| {
                    report_if_error!(selection.word_char_allowlist.set_value_to_default(ctx));
                });
            }
            ToggleUseAudibleBell => {
                TerminalSettings::handle(ctx).update(ctx, |terminal_settings, ctx| {
                    report_if_error!(terminal_settings
                        .use_audible_bell
                        .toggle_and_save_value(ctx));
                })
            }
            ToggleVimMode => AppEditorSettings::handle(ctx).update(ctx, |editor_settings, ctx| {
                editor_settings
                    .vim_mode
                    .toggle_and_save_value(ctx)
                    .expect("failed to serialize VimMode");
                ctx.notify();
            }),
            ToggleVimUnnamedSystemClipboard => {
                AppEditorSettings::handle(ctx).update(ctx, |editor_settings, ctx| {
                    editor_settings
                        .vim_unnamed_system_clipboard
                        .toggle_and_save_value(ctx)
                        .expect("failed to serialize VimUnnamedSystemClipboard");
                    ctx.notify();
                })
            }
            ToggleVimStatusBar => {
                AppEditorSettings::handle(ctx).update(ctx, |editor_settings, ctx| {
                    editor_settings
                        .vim_status_bar
                        .toggle_and_save_value(ctx)
                        .expect("failed to serialize VimStatusBar");
                    ctx.notify();
                })
            }
            SetTabBehavior(tab_behavior) => self.set_tab_behavior_setting(tab_behavior, ctx),
            SetNewTabPlacement(new_tab_placement) => {
                self.set_new_tab_placement(new_tab_placement, ctx)
            }
            SetDefaultSessionMode(mode) => self.set_default_session_mode(mode, ctx),
            SetDefaultTabConfig(path) => {
                AISettings::handle(ctx).update(ctx, |ai_settings, ctx| {
                    report_if_error!(ai_settings
                        .default_session_mode_internal
                        .set_value(DefaultSessionMode::TabConfig, ctx));
                    report_if_error!(ai_settings
                        .default_tab_config_path
                        .set_value(path.clone(), ctx));
                });
            }
            SearchForKeybinding(query) => {
                ctx.emit(FeaturesSettingsPageEvent::SearchForKeybinding(
                    query.clone(),
                ));
                ctx.notify();
            }
            ToggleAutosuggestions => {
                AppEditorSettings::handle(ctx).update(ctx, |editor_settings, ctx| {
                    editor_settings
                        .enable_autosuggestions
                        .toggle_and_save_value(ctx)
                        .expect("failed to serialize EnableAutosuggestions");
                    ctx.notify();
                })
            }
            ToggleAutosuggestionKeybindingHint => {
                AppEditorSettings::handle(ctx).update(ctx, |editor_settings, ctx| {
                    editor_settings
                        .autosuggestion_keybinding_hint
                        .toggle_and_save_value(ctx)
                        .expect("failed to serialize HideAutosuggestionKeybindingHint");
                    ctx.notify();
                })
            }
            ToggleShowAutosuggestionIgnoreButton => {
                AppEditorSettings::handle(ctx).update(ctx, |editor_settings, ctx| {
                    editor_settings
                        .show_autosuggestion_ignore_button
                        .toggle_and_save_value(ctx)
                        .expect("failed to serialize ShowAutosuggestionIgnoreButton");
                    ctx.notify();
                })
            }
            TogglePreferLowPowerGPU => {
                let new_value = GPUSettings::handle(ctx).update(ctx, |gpu_settings, ctx| {
                    report_if_error!(gpu_settings.prefer_low_power_gpu.toggle_and_save_value(ctx));
                    *gpu_settings.prefer_low_power_gpu.value()
                });
                ctx.update_rendering_config(|config| {
                    config.gpu_power_preference = if new_value {
                        GPUPowerPreference::LowPower
                    } else {
                        GPUPowerPreference::default()
                    }
                });
                self.gpu_power_preference_changed = true;
            }
            SetPreferredGraphicsBackend(graphics_backend) => {
                GPUSettings::handle(ctx).update(ctx, |gpu_settings, ctx| {
                    report_if_error!(gpu_settings
                        .preferred_backend
                        .set_value(*graphics_backend, ctx));
                });
                ctx.update_rendering_config(|config| config.backend_preference = *graphics_backend);
                self.graphics_backend_preference_changed = true;
            }
            ToggleConfirmCloseSession => {
                SessionSettings::handle(ctx).update(ctx, |session_settings, ctx| {
                    session_settings
                        .should_confirm_close_session
                        .toggle_and_save_value(ctx)
                        .expect("failed to serialize ShouldConfirmCloseSession");
                    ctx.notify();
                })
            }
            ToggleShowTerminalZeroStateBlock => {
                TerminalSettings::handle(ctx).update(ctx, |terminal_settings, ctx| {
                    report_if_error!(terminal_settings
                        .show_terminal_zero_state_block
                        .toggle_and_save_value(ctx));
                });
            }
            ToggleShowChangelogAfterUpdate => {
                ChangelogSettings::handle(ctx).update(ctx, |changelog_settings, ctx| {
                    report_if_error!(changelog_settings
                        .show_changelog_after_update
                        .toggle_and_save_value(ctx));
                })
            }
            ToggleLinuxClipboardSelection => {
                SelectionSettings::handle(ctx).update(ctx, |selection_settings, ctx| {
                    report_if_error!(selection_settings
                        .linux_selection_clipboard
                        .toggle_and_save_value(ctx));
                });
            }
            #[cfg(target_os = "linux")]
            ToggleForceX11 => {
                LinuxAppConfiguration::handle(ctx).update(ctx, |linux_app_configuration, ctx| {
                    report_if_error!(linux_app_configuration.force_x11.toggle_and_save_value(ctx));
                });
                self.force_x11_changed = true;
                // This is a workaround to make sure the user sees the new text that is added to the description after changing the setting.
                // Without scrolling, the new description text gets cut off.
                self.page.scroll_by(warpui::units::Pixels::new(40.));
                ctx.notify();
            }
            ToggleQuitOnLastWindowClosed => {
                GeneralSettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(settings
                        .quit_on_last_window_closed
                        .toggle_and_save_value(ctx));
                })
            }
            ToggleLoginItem => GeneralSettings::handle(ctx).update(ctx, |settings, ctx| {
                report_if_error!(settings.add_app_as_login_item.toggle_and_save_value(ctx));
            }),
            ToggleAtContextMenuInTerminalMode => {
                InputSettings::handle(ctx).update(ctx, |input_settings, ctx| {
                    report_if_error!(input_settings
                        .at_context_menu_in_terminal_mode
                        .toggle_and_save_value(ctx));
                });
            }
            ToggleSlashCommandsInTerminalMode => {
                InputSettings::handle(ctx).update(ctx, |input_settings, ctx| {
                    report_if_error!(input_settings
                        .enable_slash_commands_in_terminal
                        .toggle_and_save_value(ctx));
                });
            }
            ToggleOutlineCodebaseSymbolsForAtContextMenu => {
                InputSettings::handle(ctx).update(ctx, |input_settings, ctx| {
                    report_if_error!(input_settings
                        .outline_codebase_symbols_for_at_context_menu
                        .toggle_and_save_value(ctx));
                });
            }
            ToggleAutoOpenCodeReviewPane => {
                GeneralSettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(settings
                        .auto_open_code_review_pane_on_first_agent_change
                        .toggle_and_save_value(ctx));
                })
            }
            SetNotificationToastDuration => {
                let user_input = self
                    .notification_toast_duration_editor
                    .as_ref(ctx)
                    .buffer_text(ctx);

                if let Ok(duration_secs) = user_input.parse::<u64>() {
                    if duration_secs > 0 {
                        SessionSettings::handle(ctx).update(ctx, |settings, ctx| {
                            if let Err(e) = settings
                                .notification_toast_duration_secs
                                .set_value(duration_secs, ctx)
                            {
                                log::error!("Error persisting notification toast duration: {e}");
                            }
                        });
                    }
                }
            }
            MakeWarpDefaultTerminal => {
                DefaultTerminal::handle(ctx).update(ctx, |default_terminal, ctx| {
                    default_terminal.make_warp_default(ctx);
                });
            }
        }

        send_telemetry_from_ctx!(action.telemetry_event(ctx), ctx);
    }
}

impl View for FeaturesPageView {
    fn ui_name() -> &'static str {
        "FeaturesPage"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        self.page.render(self, app)
    }
}

impl FeaturesPageView {
    pub fn new(
        global_resource_handles: GlobalResourceHandles,
        ctx: &mut ViewContext<FeaturesPageView>,
    ) -> Self {
        ctx.observe(
            &DisplayCount::handle(ctx),
            Self::on_display_count_model_changed,
        );

        // Listen for model changes on all the settings that are used in this view.
        ctx.subscribe_to_model(&AppEditorSettings::handle(ctx), |_, _, _, ctx| ctx.notify());

        ctx.subscribe_to_model(&SelectionSettings::handle(ctx), |_, _, _, ctx| ctx.notify());

        // TODO(CORE-3029): Remove when we launch the new SSH Warpification.
        ctx.subscribe_to_model(&SshSettings::handle(ctx), |_, _, _, ctx| ctx.notify());
        ctx.subscribe_to_model(&AltScreenReporting::handle(ctx), |_, _, _, ctx| {
            ctx.notify()
        });
        ctx.subscribe_to_model(&BlockListSettings::handle(ctx), |_, _, _, ctx| ctx.notify());
        ctx.subscribe_to_model(&AliasExpansionSettings::handle(ctx), |_, _, _, ctx| {
            ctx.notify()
        });
        ctx.subscribe_to_model(&ChangelogSettings::handle(ctx), |_, _, _, ctx| ctx.notify());
        ctx.subscribe_to_model(&CommandSearchSettings::handle(ctx), |_, _, _, ctx| {
            ctx.notify()
        });
        ctx.subscribe_to_model(&GPUSettings::handle(ctx), |me, _, _, ctx| {
            me.refresh_preferred_graphics_backend_dropdown(ctx);
            ctx.notify();
        });
        ctx.subscribe_to_model(&InputSettings::handle(ctx), |me, _, event, ctx| {
            if matches!(
                event,
                InputSettingsChangedEvent::CompletionsOpenWhileTyping { .. }
            ) {
                me.refresh_tab_behavior_dropdown(ctx);
            }
            ctx.notify();
        });
        ctx.subscribe_to_model(
            &ScrollSettings::handle(ctx),
            |me, scroll_settings, event, ctx| {
                if matches!(
                    event,
                    ScrollSettingsChangedEvent::MouseScrollMultiplier { .. }
                ) {
                    me.mouse_scroll_input_editor.update(ctx, |editor, ctx| {
                        editor.set_buffer_text(
                            format!("{}", *scroll_settings.as_ref(ctx).mouse_scroll_multiplier)
                                .as_str(),
                            ctx,
                        );
                    })
                }
                ctx.notify()
            },
        );
        ctx.subscribe_to_model(&GeneralSettings::handle(ctx), |_, _, _, ctx| ctx.notify());
        ctx.subscribe_to_model(&KeysSettings::handle(ctx), |me, _, _, ctx| {
            me.handle_hotkey_settings_update(ctx);
        });
        ctx.subscribe_to_model(&SessionSettings::handle(ctx), |me, _, event, ctx| {
            match event {
                SessionSettingsChangedEvent::Notifications { .. } => {
                    // Update the value of the notifications threshold input to match the new setting
                    me.notifications_long_running_threshold_editor
                        .update(ctx, |editor, ctx| {
                            editor.set_buffer_text(
                                &format!(
                                    "{}",
                                    SessionSettings::handle(ctx)
                                        .as_ref(ctx)
                                        .notifications
                                        .long_running_threshold
                                        .as_secs_f32()
                                ),
                                ctx,
                            );
                        });
                }
                SessionSettingsChangedEvent::NotificationToastDurationSecs { .. } => {
                    me.notification_toast_duration_editor
                        .update(ctx, |editor, ctx| {
                            editor.set_buffer_text(
                                &format!(
                                    "{}",
                                    *SessionSettings::as_ref(ctx).notification_toast_duration_secs
                                ),
                                ctx,
                            );
                        });
                }
                SessionSettingsChangedEvent::NewSessionShellOverride { .. } => {
                    #[cfg(feature = "local_tty")]
                    {
                        use super::features::startup_shell::NewSessionShellAction;
                        use crate::terminal::session_settings::StartupShell;
                        me.startup_shell_view.update(ctx, |_, ctx| {
                            if matches!(
                                *SessionSettings::as_ref(ctx).startup_shell_override.value(),
                                StartupShell::Custom(_),
                            ) {
                                ctx.dispatch_typed_action(
                                    &NewSessionShellAction::ShowCustomPathInput,
                                );
                            }
                            ctx.notify();
                        });
                    }
                }
                _ => {}
            }
            ctx.notify()
        });
        ctx.subscribe_to_model(
            &TerminalSettings::handle(ctx),
            |me, terminal_settings, event, ctx| {
                if matches!(event, TerminalSettingsChangedEvent::MaximumGridSize { .. }) {
                    me.max_block_size_input_editor.update(ctx, |editor, ctx| {
                        editor.set_buffer_text(
                            &format!("{}", *terminal_settings.as_ref(ctx).maximum_grid_size),
                            ctx,
                        );
                    });
                }
                ctx.notify()
            },
        );

        ctx.subscribe_to_model(&UndoCloseSettings::handle(ctx), |_, _, _, ctx| ctx.notify());

        ctx.subscribe_to_model(&DefaultTerminal::handle(ctx), |_, _, _, ctx| {
            ctx.notify();
        });

        ctx.subscribe_to_model(&AISettings::handle(ctx), |me, _, event, ctx| {
            if matches!(
                event,
                AISettingsChangedEvent::IsAnyAIEnabled { .. }
                    | AISettingsChangedEvent::DefaultSessionMode { .. }
            ) {
                Self::update_default_session_mode_dropdown(
                    me.default_session_mode_dropdown.clone(),
                    ctx,
                );
                ctx.notify();
            }
        });

        let pin_position_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);

            let top = DropdownItem::new(
                "Pin to top",
                FeaturesPageAction::QuakeEditorSetPinPosition(QuakeModePinPosition::Top),
            );

            let bottom = DropdownItem::new(
                "Pin to bottom",
                FeaturesPageAction::QuakeEditorSetPinPosition(QuakeModePinPosition::Bottom),
            );

            let left = DropdownItem::new(
                "Pin to left",
                FeaturesPageAction::QuakeEditorSetPinPosition(QuakeModePinPosition::Left),
            );

            let right = DropdownItem::new(
                "Pin to right",
                FeaturesPageAction::QuakeEditorSetPinPosition(QuakeModePinPosition::Right),
            );

            // Note that the index here has to correspond to the ordering of the items.
            let selected_index = match KeysSettings::as_ref(ctx)
                .quake_mode_settings
                .active_pin_position
            {
                QuakeModePinPosition::Top => 0,
                QuakeModePinPosition::Bottom => 1,
                QuakeModePinPosition::Left => 2,
                QuakeModePinPosition::Right => 3,
            };
            dropdown.add_items(vec![top, bottom, left, right], ctx);

            dropdown.set_selected_by_index(selected_index, ctx);
            dropdown
        });

        let pin_screen_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);
            init_display_count_dropdown(
                &KeysSettings::as_ref(ctx)
                    .quake_mode_settings
                    .value()
                    .clone(),
                DisplayCount::as_ref(ctx).num_displays(),
                &mut dropdown,
                ctx,
            );
            dropdown
        });

        let new_tab_placement_dropdown = ctx.add_typed_action_view(Dropdown::new);

        Self::update_new_tab_placement_dropdown(new_tab_placement_dropdown.clone(), ctx);

        ctx.subscribe_to_model(&TabSettings::handle(ctx), |me, _, event, ctx| {
            if matches!(event, TabSettingsChangedEvent::NewTabPlacement { .. }) {
                Self::update_new_tab_placement_dropdown(me.new_tab_placement_dropdown.clone(), ctx);
            }
            ctx.notify();
        });

        let default_session_mode_dropdown = ctx.add_typed_action_view(FilterableDropdown::new);
        Self::update_default_session_mode_dropdown(default_session_mode_dropdown.clone(), ctx);

        ctx.subscribe_to_model(&WarpConfig::handle(ctx), |me, _, event, ctx| {
            if matches!(event, WarpConfigUpdateEvent::TabConfigs) {
                Self::update_default_session_mode_dropdown(
                    me.default_session_mode_dropdown.clone(),
                    ctx,
                );
                ctx.notify();
            }
        });

        #[cfg(feature = "local_fs")]
        let external_editor_view = ctx.add_typed_action_view(features::ExternalEditorView::new);

        let global_hotkey_mode =
            KeysSettings::handle(ctx).read(ctx, |settings, ctx| settings.global_hotkey_mode(ctx));
        let global_hotkey_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);
            init_global_hotkey_dropdown(global_hotkey_mode, &mut dropdown, ctx);
            dropdown
        });

        // The state for this dropdown is initialized by `refresh_tab_behavior_state`.
        let tab_behavior_dropdown = ctx.add_typed_action_view(Dropdown::new);

        let ctrl_tab_behavior_dropdown = ctx.add_typed_action_view(Dropdown::new);
        Self::update_ctrl_tab_behavior_dropdown(ctrl_tab_behavior_dropdown.clone(), ctx);

        ctx.subscribe_to_model(&KeysSettings::handle(ctx), |me, _, event, ctx| {
            if matches!(
                event,
                KeysSettingsChangedEvent::CtrlTabBehaviorSetting { .. }
            ) {
                Self::update_ctrl_tab_behavior_dropdown(me.ctrl_tab_behavior_dropdown.clone(), ctx);
            }
            ctx.notify();
        });

        let graphics_backend_dropdown = ctx.add_typed_action_view(Dropdown::new);

        #[cfg(feature = "local_tty")]
        let working_directory_view = ctx.add_typed_action_view(features::WorkingDirectoryView::new);

        #[cfg(feature = "local_tty")]
        let startup_shell_view = ctx.add_typed_action_view(features::StartupShellView::new);

        let undo_close_view = ctx.add_typed_action_view(features::UndoCloseView::new);

        let appearance_handle = Appearance::handle(ctx);

        let width_and_height_editor_options = SingleLineEditorOptions {
            text: TextOptions::ui_font_size(appearance_handle.as_ref(ctx)),
            ..Default::default()
        };

        let width_editor = {
            ctx.add_typed_action_view(|ctx| {
                EditorView::single_line(width_and_height_editor_options.clone(), ctx)
            })
        };
        width_editor.update(ctx, |editor, ctx| {
            editor.set_buffer_text(
                &format!(
                    "{}",
                    &KeysSettings::handle(ctx)
                        .as_ref(ctx)
                        .quake_mode_settings
                        .value()
                        .width_percentage()
                ),
                ctx,
            );
        });
        ctx.subscribe_to_view(&width_editor, move |me, _, event, ctx| {
            me.handle_width_editor_event(event, ctx);
        });

        let word_boundary_editor = ctx.add_typed_action_view(|ctx| {
            EditorView::single_line(width_and_height_editor_options.clone(), ctx)
        });

        word_boundary_editor.update(ctx, |editor, ctx| {
            let word_char_allowlist = SemanticSelection::as_ref(ctx).word_char_allowlist_string();
            editor.set_buffer_text(&word_char_allowlist, ctx);
        });

        ctx.subscribe_to_model(&SemanticSelection::handle(ctx), |me, _, event, ctx| {
            if matches!(
                event,
                SemanticSelectionChangedEvent::WordCharAllowlist { .. }
            ) {
                me.word_boundary_editor.update(ctx, |editor, ctx| {
                    let word_char_allowlist =
                        SemanticSelection::as_ref(ctx).word_char_allowlist_string();
                    editor.set_buffer_text(&word_char_allowlist, ctx);
                });
            }
            ctx.notify();
        });

        let height_editor = {
            ctx.add_typed_action_view(|ctx| {
                EditorView::single_line(width_and_height_editor_options.clone(), ctx)
            })
        };
        height_editor.update(ctx, |editor, ctx| {
            editor.set_buffer_text(
                &format!(
                    "{}",
                    &KeysSettings::handle(ctx)
                        .as_ref(ctx)
                        .quake_mode_settings
                        .value()
                        .height_percentage()
                ),
                ctx,
            );
        });
        ctx.subscribe_to_view(&height_editor, move |me, _, event, ctx| {
            me.handle_height_editor_event(event, ctx);
        });

        // Editor for configuring max block size
        let block_size_editor = {
            ctx.add_typed_action_view(|ctx| {
                EditorView::single_line(width_and_height_editor_options.clone(), ctx)
            })
        };
        block_size_editor.update(ctx, |editor, ctx| {
            editor.set_buffer_text(
                &format!(
                    "{}",
                    *TerminalSettings::handle(ctx)
                        .as_ref(ctx)
                        .maximum_grid_size
                        .value()
                ),
                ctx,
            );
        });
        ctx.subscribe_to_view(&block_size_editor, move |me, _, event, ctx| {
            me.handle_block_size_editor_event(event, ctx);
        });

        let mouse_scroll_input_editor = ctx.add_typed_action_view(|ctx| {
            EditorView::single_line(width_and_height_editor_options.clone(), ctx)
        });
        mouse_scroll_input_editor.update(ctx, |editor, ctx| {
            editor.set_buffer_text(
                format!(
                    "{}",
                    ScrollSettings::handle(ctx)
                        .as_ref(ctx)
                        .mouse_scroll_multiplier
                        .value()
                )
                .as_str(),
                ctx,
            );
        });
        ctx.subscribe_to_view(&mouse_scroll_input_editor, |me, _, event, ctx| {
            me.handle_mouse_scroll_input_editor_event(event, ctx);
        });

        let notifications_long_running_threshold_editor = {
            ctx.add_typed_action_view(|ctx| {
                let options = SingleLineEditorOptions {
                    text: TextOptions {
                        font_size_override: Some(appearance_handle.as_ref(ctx).ui_font_size() - 2.),
                        ..Default::default()
                    },
                    ..Default::default()
                };

                EditorView::single_line(options, ctx)
            })
        };
        notifications_long_running_threshold_editor.update(ctx, |editor, ctx| {
            editor.set_buffer_text(
                &format!(
                    "{}",
                    SessionSettings::handle(ctx)
                        .as_ref(ctx)
                        .notifications
                        .long_running_threshold
                        .as_secs_f32()
                ),
                ctx,
            );
        });

        let notification_toast_duration_editor = ctx.add_typed_action_view(|ctx| {
            let options = SingleLineEditorOptions {
                text: TextOptions {
                    font_size_override: Some(appearance_handle.as_ref(ctx).ui_font_size() - 2.),
                    ..Default::default()
                },
                ..Default::default()
            };
            EditorView::single_line(options, ctx)
        });
        notification_toast_duration_editor.update(ctx, |editor, ctx| {
            editor.set_buffer_text(
                &format!(
                    "{}",
                    *SessionSettings::as_ref(ctx).notification_toast_duration_secs
                ),
                ctx,
            );
        });

        ctx.subscribe_to_model(&GPUState::handle(ctx), |me, _, event, ctx| {
            if matches!(event, GPUStateEvent::LowPowerGPUAvailable) {
                me.page = Self::build_page(ctx);
                ctx.notify();
            }
        });

        let mut features_page_view = FeaturesPageView {
            page: Self::build_page(ctx),
            global_resource_handles,
            button_mouse_states: Default::default(),
            activation_hotkey_keybinding_editor_state: KeybindingEditorState::Idle,
            completions_keystroke: Default::default(),
            autosuggestions_keystroke: Default::default(),
            activation_hotkey_keybinding: KeyBindingModifyingState::new(
                KeysSettings::as_ref(ctx)
                    .activation_hotkey_keybinding
                    .value()
                    .clone(),
            ),
            quake_mode_keybinding_editor_state: KeybindingEditorState::Idle,
            quake_mode_keybinding: KeyBindingModifyingState::new(
                KeysSettings::as_ref(ctx)
                    .quake_mode_settings
                    .value()
                    .keybinding
                    .clone(),
            ),
            quake_mode_pin_position_dropdown: pin_position_dropdown,
            quake_mode_pin_screen_dropdown: pin_screen_dropdown,
            quake_mode_width_editor: width_editor,
            quake_mode_height_editor: height_editor,

            notifications_long_running_threshold_editor,
            notification_toast_duration_editor,

            #[cfg(feature = "local_tty")]
            working_directory_view,
            #[cfg(feature = "local_tty")]
            startup_shell_view,
            undo_close_view,

            max_block_size_input_editor: block_size_editor,
            valid_max_block_size: true,

            ssh_wrapper_toggled: false,

            #[cfg(feature = "local_fs")]
            external_editor_view,
            word_boundary_editor,
            global_hotkey_dropdown,

            tab_behavior_dropdown,
            ctrl_tab_behavior_dropdown,
            graphics_backend_dropdown,
            new_tab_placement_dropdown,
            default_session_mode_dropdown,
            tab_behavior: Default::default(),

            window_id: ctx.window_id(),

            mouse_scroll_input_editor,
            valid_mouse_scroll_multiplier: true,

            #[cfg(target_os = "linux")]
            force_x11_changed: false,
            gpu_power_preference_changed: false,
            graphics_backend_preference_changed: false,
        };

        features_page_view.refresh_tab_behavior_state(ctx);
        features_page_view.refresh_tab_behavior_dropdown(ctx);
        features_page_view.refresh_preferred_graphics_backend_dropdown(ctx);
        features_page_view
    }

    fn build_page(ctx: &mut ViewContext<Self>) -> PageType<Self> {
        let mut general_widgets: Vec<Box<dyn SettingsWidget<View = Self>>> =
            vec![Box::new(DefaultSessionModeWidget::default())];

        let native_preference_settings = NativePreferenceSettings::as_ref(ctx);
        if native_preference_settings
            .user_native_redirect_preference
            .is_supported_on_current_platform()
        {
            general_widgets.push(Box::new(NativeRedirectWidget::default()));
        }

        let general_settings = &GeneralSettings::as_ref(ctx);
        if general_settings
            .restore_session
            .is_supported_on_current_platform()
        {
            general_widgets.push(Box::new(SessionRestorationWidget::default()))
        }

        general_widgets.push(Box::new(SnackbarHeaderWidget::default()));
        general_widgets.push(Box::new(LinkTooltipWidget::default()));

        #[cfg(feature = "local_fs")]
        {
            if !FeatureFlag::OpenWarpNewSettingsModes.is_enabled() {
                let external_editor_settings =
                    crate::util::file::external_editor::EditorSettings::as_ref(ctx);
                if external_editor_settings
                    .open_file_editor
                    .is_supported_on_current_platform()
                {
                    general_widgets.push(Box::new(ExternalEditorWidget::default()));
                }
            }
        }

        if general_settings
            .show_warning_before_quitting
            .is_supported_on_current_platform()
        {
            general_widgets.push(Box::new(QuitWarningModalWidget::default()));
        }

        if general_settings
            .quit_on_last_window_closed
            .is_supported_on_current_platform()
        {
            general_widgets.push(Box::new(QuitWhenAllWindowsClosedWidget::default()));
        }

        if general_settings
            .add_app_as_login_item
            .is_supported_on_current_platform()
        {
            general_widgets.push(Box::new(LoginItemWidget::default()));
        }

        let changelog_settings = ChangelogSettings::as_ref(ctx);
        if changelog_settings
            .show_changelog_after_update
            .is_supported_on_current_platform()
        {
            general_widgets.push(Box::new(ShowChangelogWidget::default()));
        }

        let scroll_settings = ScrollSettings::as_ref(ctx);
        if scroll_settings
            .mouse_scroll_multiplier
            .is_supported_on_current_platform()
        {
            general_widgets.push(Box::new(MouseScrollMultiplierWidget::default()));
        }

        if FeatureFlag::AutoOpenCodeReviewPane.is_enabled()
            && !FeatureFlag::OpenWarpNewSettingsModes.is_enabled()
        {
            general_widgets.push(Box::new(AutoOpenCodeReviewPaneWidget::default()));
        }

        if DefaultTerminal::can_warp_become_default() {
            general_widgets.push(Box::new(DefaultTerminalWidget::default()));
        }

        let app_editor_settings = AppEditorSettings::as_ref(ctx);

        let notifications_widgets: Vec<Box<dyn SettingsWidget<View = Self>>> =
            vec![Box::new(DesktopNotificationsWidget::default())];

        let mut session_widgets: Vec<Box<dyn SettingsWidget<View = Self>>> = vec![];

        session_widgets.push(Box::new(BlockLimitWidget::default()));

        if !FeatureFlag::SSHTmuxWrapper.is_enabled()
            && SshSettings::as_ref(ctx)
                .enable_legacy_ssh_wrapper
                .is_supported_on_current_platform()
        {
            session_widgets.push(Box::new(SSHWrapperWidget::default()));
        }

        let session_settings = SessionSettings::as_ref(ctx);

        #[cfg(feature = "local_tty")]
        {
            if session_settings
                .startup_shell_override
                .is_supported_on_current_platform()
            {
                session_widgets.push(Box::new(StartupShellWidget::default()));
            }
            if session_settings
                .working_directory_config
                .is_supported_on_current_platform()
            {
                session_widgets.push(Box::new(WorkingDirectoryWidget::default()));
            }
        }

        let undo_close_settings = UndoCloseSettings::as_ref(ctx);
        if undo_close_settings
            .enabled
            .is_supported_on_current_platform()
        {
            session_widgets.push(Box::new(UndoCloseWidget::default()));
        }

        if FeatureFlag::CreatingSharedSessions.is_enabled()
            && ContextFlag::CreateSharedSession.is_enabled()
            && session_settings
                .should_confirm_close_session
                .is_supported_on_current_platform()
        {
            session_widgets.push(Box::new(ConfirmCloseSharedSessionWidget::default()));
        }

        let mut keys_widgets: Vec<Box<dyn SettingsWidget<View = Self>>> = vec![];
        let keys_settings = KeysSettings::as_ref(ctx);
        if keys_settings
            .extra_meta_keys
            .is_supported_on_current_platform()
        {
            keys_widgets.push(Box::new(ExtraMetaKeysWidget::default()))
        }

        if keys_settings
            .ctrl_tab_behavior
            .is_supported_on_current_platform()
        {
            keys_widgets.push(Box::new(CtrlTabBehaviorWidget::default()));
        }

        if keys_settings
            .activation_hotkey_enabled
            .is_supported_on_current_platform()
        {
            keys_widgets.push(Box::new(GlobalHotkeyWidget::default()));
        }

        let mut text_editing_widgets: Vec<Box<dyn SettingsWidget<View = Self>>> =
            vec![Box::new(AutocompleteSymbolsWidget::default())];

        if app_editor_settings
            .vim_mode
            .is_supported_on_current_platform()
        {
            text_editing_widgets.push(Box::new(VimModeWidget::default()));
        }

        let mut editor_widgets: Vec<Box<dyn SettingsWidget<View = Self>>> = vec![];

        let input_settings = InputSettings::as_ref(ctx);
        if input_settings
            .error_underlining
            .is_supported_on_current_platform()
        {
            editor_widgets.push(Box::new(ErrorUnderliningWidget::default()))
        }
        if input_settings
            .syntax_highlighting
            .is_supported_on_current_platform()
        {
            editor_widgets.push(Box::new(SyntaxHighlightingWidget::default()))
        }
        if input_settings
            .completions_open_while_typing
            .is_supported_on_current_platform()
        {
            editor_widgets.push(Box::new(CompletionsMenuWhileTypingWidget::default()));
        }
        if input_settings
            .command_corrections
            .is_supported_on_current_platform()
        {
            editor_widgets.push(Box::new(CommandCorrectionsWidget::default()));
        }

        let alias_expansion_settings = AliasExpansionSettings::as_ref(ctx);
        if alias_expansion_settings
            .alias_expansion_enabled
            .is_supported_on_current_platform()
        {
            editor_widgets.push(Box::new(AliasExpansionWidget::default()));
        }

        let selection_settings = SelectionSettings::as_ref(ctx);
        if selection_settings
            .middle_click_paste_enabled
            .is_supported_on_current_platform()
        {
            editor_widgets.push(Box::new(MiddleClickPasteWidget::default()));
        }

        editor_widgets.push(Box::new(AutosuggestionKeybindingHintWidget::default()));

        if FeatureFlag::AllowIgnoringInputSuggestions.is_enabled() {
            editor_widgets.push(Box::new(AutosuggestionIgnoreButtonWidget::default()));
        }

        if input_settings
            .at_context_menu_in_terminal_mode
            .is_supported_on_current_platform()
        {
            editor_widgets.push(Box::new(AtContextMenuInTerminalModeWidget::default()));
        }

        if FeatureFlag::AgentView.is_enabled()
            && input_settings
                .enable_slash_commands_in_terminal
                .is_supported_on_current_platform()
        {
            editor_widgets.push(Box::new(SlashCommandsInTerminalModeWidget::default()));
        }

        if input_settings
            .outline_codebase_symbols_for_at_context_menu
            .is_supported_on_current_platform()
            && FeatureFlag::AIContextMenuCode.is_enabled()
        {
            editor_widgets.push(Box::new(
                OutlineCodebaseSymbolsForAtContextMenuWidget::default(),
            ));
        }

        if FeatureFlag::AgentView.is_enabled() {
            editor_widgets.push(Box::new(ShowTerminalInputMessageLineWidget::default()));
        }

        editor_widgets.push(Box::new(TabKeyBehaviorWidget::default()));

        let mut terminal_widgets: Vec<Box<dyn SettingsWidget<View = Self>>> = vec![];

        let reporting_settings = AltScreenReporting::as_ref(ctx);
        if reporting_settings
            .mouse_reporting_enabled
            .is_supported_on_current_platform()
        {
            terminal_widgets.push(Box::new(MouseReportingWidget::default()));
        }
        if reporting_settings
            .scroll_reporting_enabled
            .is_supported_on_current_platform()
        {
            terminal_widgets.push(Box::new(ScrollReportingWidget::default()));
        }
        if reporting_settings
            .focus_reporting_enabled
            .is_supported_on_current_platform()
        {
            terminal_widgets.push(Box::new(FocusReportingWidget::default()));
        }

        let terminal_settings = TerminalSettings::as_ref(ctx);
        if terminal_settings
            .use_audible_bell
            .is_supported_on_current_platform()
        {
            terminal_widgets.push(Box::new(AudibleBellWidget::default()));
        }

        if FeatureFlag::AgentView.is_enabled() {
            terminal_widgets.push(Box::new(ShowTerminalZeroStateBlockWidget::default()));
        }

        terminal_widgets.push(Box::new(SmartSelectWidget::default()));
        terminal_widgets.push(Box::new(CopyOnSelectWidget::default()));
        terminal_widgets.push(Box::new(NewTabPlacementWidget::default()));

        let mut system_widgets: Vec<Box<dyn SettingsWidget<View = Self>>> = vec![];
        let selection_settings = SelectionSettings::as_ref(ctx);
        if selection_settings
            .linux_selection_clipboard
            .is_supported_on_current_platform()
        {
            system_widgets.push(Box::new(LinuxSelectionClipboardWidget::default()));
        }

        let gpu_settings = GPUSettings::as_ref(ctx);
        if gpu_settings
            .prefer_low_power_gpu
            .is_supported_on_current_platform()
            && GPUState::as_ref(ctx).is_low_power_gpu_available()
        {
            system_widgets.push(Box::new(GPUWidget::default()));
        }

        if gpu_settings
            .preferred_backend
            .is_supported_on_current_platform()
        {
            system_widgets.push(Box::new(GraphicsBackendWidget::default()));
        }

        #[cfg(target_os = "linux")]
        {
            if windowing_system_is_customizable(ctx) {
                system_widgets.push(Box::new(WindowSystemWidget::default()));
            }
        }

        let categories = vec![
            Category::new("General", general_widgets),
            Category::new("Session", session_widgets),
            Category::new("Keys", keys_widgets),
            Category::new("Text Editing", text_editing_widgets),
            Category::new("Terminal Input", editor_widgets),
            Category::new("Terminal", terminal_widgets),
            Category::new("Notifications", notifications_widgets),
            Category::new(
                "Workflows",
                vec![Box::new(WorkflowsInCommandSearch::default())],
            ),
            Category::new("System", system_widgets),
        ];

        PageType::new_categorized(categories, None)
    }

    fn update_ctrl_tab_behavior_dropdown(
        dropdown: ViewHandle<Dropdown<FeaturesPageAction>>,
        ctx: &mut ViewContext<Self>,
    ) {
        dropdown.update(ctx, |dropdown, ctx| {
            let values = vec![
                CtrlTabBehavior::ActivatePrevNextTab,
                CtrlTabBehavior::CycleMostRecentSession,
            ];

            let current_value = *KeysSettings::as_ref(ctx).ctrl_tab_behavior;

            let selected_index = values
                .iter()
                .position(|val| *val == current_value)
                .unwrap_or_else(|| {
                    log::error!(
                        "Could not find current Ctrl-Tab behavior value in dropdown option list"
                    );
                    0
                });

            dropdown.set_items(
                values
                    .into_iter()
                    .map(|val| {
                        DropdownItem::new(
                            val.as_dropdown_label(),
                            FeaturesPageAction::SetCtrlTabBehavior(val),
                        )
                    })
                    .collect(),
                ctx,
            );
            dropdown.set_selected_by_index(selected_index, ctx);
        });
    }

    fn update_new_tab_placement_dropdown(
        dropdown: ViewHandle<Dropdown<FeaturesPageAction>>,
        ctx: &mut ViewContext<Self>,
    ) {
        dropdown.update(ctx, |dropdown, ctx| {
            let values = vec![
                NewTabPlacement::AfterCurrentTab,
                NewTabPlacement::AfterAllTabs,
            ];
            let current_value = TabSettings::as_ref(ctx).new_tab_placement;

            let selected_index = values
                .iter()
                .position(|val| *val == current_value)
                .unwrap_or_else(|| {
                    log::error!(
                        "Could not find current NewTabPlacement value in dropdown option list"
                    );
                    0
                });

            dropdown.set_items(
                values
                    .into_iter()
                    .map(|val| {
                        DropdownItem::new(
                            Self::new_tab_placement_dropdown_item_label(val),
                            FeaturesPageAction::SetNewTabPlacement(val),
                        )
                    })
                    .collect(),
                ctx,
            );
            dropdown.set_selected_by_index(selected_index, ctx);
        });
    }

    fn handle_hotkey_settings_update(&mut self, ctx: &mut ViewContext<Self>) {
        let global_hotkey_mode =
            KeysSettings::handle(ctx).read(ctx, |settings, ctx| settings.global_hotkey_mode(ctx));
        // Update the keybinding editor if it is already open.
        match global_hotkey_mode {
            GlobalHotkeyMode::Disabled => (),
            GlobalHotkeyMode::QuakeMode => {
                self.quake_mode_keybinding
                    .current_binding
                    .clone_from(&KeysSettings::as_ref(ctx).quake_mode_settings.keybinding);
                self.reset_quake_mode_keybinding_editor();
            }
            GlobalHotkeyMode::ActivationHotkey => {
                self.activation_hotkey_keybinding
                    .current_binding
                    .clone_from(&KeysSettings::as_ref(ctx).activation_hotkey_keybinding);
                self.reset_activation_hotkey_keybinding_editor();
            }
        }
        // the selected item needs to update if the settings gets changed via the command palette
        self.global_hotkey_dropdown.update(ctx, |dropdown, ctx| {
            dropdown.set_selected_by_name(global_hotkey_mode.as_dropdown_label(), ctx);
            ctx.notify();
        });
        ctx.notify();
    }

    fn on_display_count_model_changed(
        &mut self,
        display_count: ModelHandle<DisplayCount>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.quake_mode_pin_screen_dropdown
            .update(ctx, |dropdown, ctx| {
                init_display_count_dropdown(
                    &KeysSettings::as_ref(ctx)
                        .quake_mode_settings
                        .value()
                        .clone(),
                    display_count.as_ref(ctx).num_displays(),
                    dropdown,
                    ctx,
                );
                ctx.notify();
            });
    }

    pub fn handle_height_editor_event(&mut self, event: &EditorEvent, ctx: &mut ViewContext<Self>) {
        match event {
            EditorEvent::Blurred | EditorEvent::Enter => self.set_height_ratio(ctx),
            EditorEvent::Escape => ctx.emit(FeaturesSettingsPageEvent::FocusModal),
            _ => {}
        }
    }

    pub fn handle_width_editor_event(&mut self, event: &EditorEvent, ctx: &mut ViewContext<Self>) {
        match event {
            EditorEvent::Blurred | EditorEvent::Enter => self.set_width_ratio(ctx),
            EditorEvent::Escape => ctx.emit(FeaturesSettingsPageEvent::FocusModal),
            _ => {}
        }
    }

    pub fn handle_block_size_editor_event(
        &mut self,
        event: &EditorEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            EditorEvent::Edited(_) => {
                let buffer_text = self
                    .max_block_size_input_editor
                    .as_ref(ctx)
                    .buffer_text(ctx);

                if let Ok(input) = buffer_text.parse::<usize>() {
                    if (MIN_MAX_GRID_SIZE..=max_max_grid_size()).contains(&input) {
                        self.valid_max_block_size = true;
                        ctx.notify();
                        return;
                    }
                }
                self.valid_max_block_size = false;
                ctx.notify();
            }
            EditorEvent::Enter | EditorEvent::Blurred => self.set_max_block_size(ctx),
            EditorEvent::Escape => ctx.emit(FeaturesSettingsPageEvent::FocusModal),
            _ => {}
        }
    }

    fn set_max_block_size(&mut self, ctx: &mut ViewContext<Self>) {
        let buffer_text = self
            .max_block_size_input_editor
            .as_ref(ctx)
            .buffer_text(ctx);

        if let Ok(input) = buffer_text.parse::<usize>() {
            // Block sizes need to fall within the range limit
            let new_size = input.clamp(MIN_MAX_GRID_SIZE, max_max_grid_size());

            if new_size != input {
                self.max_block_size_input_editor.update(ctx, |editor, ctx| {
                    editor.set_buffer_text(&format!("{new_size}"), ctx);
                });
            };

            // If the input is not a new value, do nothing
            let current_size = *TerminalSettings::as_ref(ctx).maximum_grid_size.value();
            if current_size == new_size {
                return;
            }

            TerminalSettings::handle(ctx).update(ctx, |settings, ctx| {
                report_if_error!(settings.maximum_grid_size.set_value(new_size, ctx));
            });
        } else {
            // Any invalid input should reset the input back to the last known value
            self.max_block_size_input_editor.update(ctx, |editor, ctx| {
                editor.set_buffer_text(
                    &format!(
                        "{}",
                        *TerminalSettings::handle(ctx)
                            .as_ref(ctx)
                            .maximum_grid_size
                            .value()
                    ),
                    ctx,
                );
            });
        }
    }

    fn handle_mouse_scroll_input_editor_event(
        &mut self,
        event: &EditorEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            EditorEvent::Edited(_) => {
                let buffer_text = self.mouse_scroll_input_editor.as_ref(ctx).buffer_text(ctx);

                if buffer_text.parse::<f32>().is_ok_and(|parsed_multiplier| {
                    (MIN_MOUSE_SCROLL_MULTIPLIER..=MAX_MOUSE_SCROLL_MULTIPLIER)
                        .contains(&parsed_multiplier)
                }) {
                    self.valid_mouse_scroll_multiplier = true;
                    ctx.notify();
                } else {
                    self.valid_mouse_scroll_multiplier = false;
                    ctx.notify();
                }
            }
            EditorEvent::Enter | EditorEvent::Blurred => self.set_mouse_scroll_multiplier(ctx),
            EditorEvent::Escape => ctx.emit(FeaturesSettingsPageEvent::FocusModal),
            _ => {}
        }
    }

    fn set_mouse_scroll_multiplier(&mut self, ctx: &mut ViewContext<Self>) {
        let user_input = self.mouse_scroll_input_editor.as_ref(ctx).buffer_text(ctx);
        let scroll_settings = ScrollSettings::handle(ctx);

        if let Ok(new_multiplier) = user_input.parse::<f32>() {
            let constrained_multiplier =
                new_multiplier.clamp(MIN_MOUSE_SCROLL_MULTIPLIER, MAX_MOUSE_SCROLL_MULTIPLIER);

            if constrained_multiplier != new_multiplier {
                self.mouse_scroll_input_editor.update(ctx, |editor, ctx| {
                    editor.set_buffer_text(format!("{constrained_multiplier}").as_str(), ctx);
                });
            }

            if *scroll_settings.as_ref(ctx).mouse_scroll_multiplier.value()
                == constrained_multiplier
            {
                return;
            }

            scroll_settings.update(ctx, |settings, ctx| {
                let _ = settings
                    .mouse_scroll_multiplier
                    .set_value(constrained_multiplier, ctx);
            });
        } else {
            // Fall back to the current setting value.
            self.mouse_scroll_input_editor.update(ctx, |editor, ctx| {
                editor.set_buffer_text(
                    format!(
                        "{}",
                        scroll_settings.as_ref(ctx).mouse_scroll_multiplier.value()
                    )
                    .as_str(),
                    ctx,
                );
            })
        }
    }

    fn set_height_ratio(&mut self, ctx: &mut ViewContext<Self>) {
        let user_input = self.quake_mode_height_editor.as_ref(ctx).buffer_text(ctx);

        if let Ok(num) = user_input.parse() {
            KeysSettings::handle(ctx).update(ctx, |keys_settings, ctx| {
                keys_settings.set_quake_mode_width_or_height_and_write_to_user_defaults(
                    None,
                    Some(num),
                    ctx,
                )
            })
        }
    }

    fn set_width_ratio(&mut self, ctx: &mut ViewContext<Self>) {
        let user_input = self.quake_mode_width_editor.as_ref(ctx).buffer_text(ctx);

        if let Ok(num) = user_input.parse::<usize>() {
            KeysSettings::handle(ctx).update(ctx, |keys_settings, ctx| {
                keys_settings.set_quake_mode_width_or_height_and_write_to_user_defaults(
                    Some(num as u8),
                    None,
                    ctx,
                );
            })
        }
    }

    fn reset_size_percentage(&mut self, ctx: &mut ViewContext<Self>) {
        let default_size_percentage = DEFAULT_QUAKE_MODE_SIZE_PERCENTAGES
            .get(
                &KeysSettings::as_ref(ctx)
                    .quake_mode_settings
                    .value()
                    .active_pin_position,
            )
            .expect("Pin position should exist in default size percentages");

        self.quake_mode_height_editor.update(ctx, |editor, ctx| {
            editor.set_buffer_text(&format!("{}", default_size_percentage.height), ctx);
        });

        self.quake_mode_width_editor.update(ctx, |editor, ctx| {
            editor.set_buffer_text(&format!("{}", default_size_percentage.width), ctx);
        });

        KeysSettings::handle(ctx).update(ctx, |keys_settings, ctx| {
            keys_settings.set_quake_mode_width_or_height_and_write_to_user_defaults(
                Some(default_size_percentage.width),  /*width percentages*/
                Some(default_size_percentage.height), /*height percentages*/
                ctx,
            );
        });

        ctx.notify();
    }

    fn reset_quake_mode_keybinding_editor(&mut self) {
        self.quake_mode_keybinding
            .unsaved_binding
            .clone_from(&self.quake_mode_keybinding.current_binding);
        self.quake_mode_keybinding_editor_state = KeybindingEditorState::Idle;
    }

    fn enable_quake_mode_global_shortcut(&self, app: &mut AppContext) {
        if let Some(shortcut) = &self.quake_mode_keybinding.current_binding {
            app.register_global_shortcut(
                shortcut.clone(),
                "root_view:toggle_quake_mode_window",
                self.global_resource_handles.clone(),
            );
        }
    }

    fn disable_quake_mode_global_shortcut(&self, app: &mut AppContext) {
        if let Some(shortcut) = &self.quake_mode_keybinding.current_binding {
            app.unregister_global_shortcut(shortcut);
        }
    }

    fn reset_activation_hotkey_keybinding_editor(&mut self) {
        self.activation_hotkey_keybinding
            .unsaved_binding
            .clone_from(&self.activation_hotkey_keybinding.current_binding);
        self.activation_hotkey_keybinding_editor_state = KeybindingEditorState::Idle;
    }

    fn enable_activation_hotkey_global_shortcut(&self, app: &mut AppContext) {
        if let Some(shortcut) = &self.activation_hotkey_keybinding.current_binding {
            app.register_global_shortcut(
                shortcut.clone(),
                "root_view:show_or_hide_non_quake_mode_windows",
                (),
            );
        }
    }

    fn disable_activation_hotkey_global_shortcut(&self, app: &mut AppContext) {
        if let Some(shortcut) = &self.activation_hotkey_keybinding.current_binding {
            app.unregister_global_shortcut(shortcut);
        }
    }

    /// A lot of state needs to get updated here:
    ///   1. Settings
    ///   2. AppContext::disabled_key_bindings_windows
    ///   3. AppContext::global_shortcuts
    ///   4. self.quake_mode_keybinding and self.activation_hotkey_keybinding
    ///   5. quake_mode_keybinding_editor_state and activation_hotkey_keybinding_editor_state
    fn set_global_hotkey_mode(&mut self, mode: &GlobalHotkeyMode, ctx: &mut ViewContext<Self>) {
        ctx.enable_key_bindings_dispatching();

        self.reset_quake_mode_keybinding_editor();
        self.reset_activation_hotkey_keybinding_editor();
        ctx.notify();

        KeysSettings::handle(ctx).update(ctx, |keys_settings, ctx| {
            keys_settings.set_global_hotkey_mode_and_write_to_user_defaults(mode, ctx);
        });

        self.disable_quake_mode_global_shortcut(ctx);
        self.disable_activation_hotkey_global_shortcut(ctx);

        match mode {
            GlobalHotkeyMode::Disabled => {}
            GlobalHotkeyMode::QuakeMode => {
                self.enable_quake_mode_global_shortcut(ctx);
            }
            GlobalHotkeyMode::ActivationHotkey => {
                self.enable_activation_hotkey_global_shortcut(ctx);
            }
        }
    }

    pub(super) fn refresh_preferred_graphics_backend_dropdown(
        &mut self,
        ctx: &mut ViewContext<Self>,
    ) {
        self.graphics_backend_dropdown.update(ctx, |dropdown, ctx| {
            if let Some(window) = ctx.windows().platform_window(ctx.window_id()) {
                let mut items = vec![DropdownItem::new(
                    "Default",
                    FeaturesPageAction::SetPreferredGraphicsBackend(None),
                )];
                items.extend(window.supported_backends().into_iter().map(|backend| {
                    DropdownItem::new(
                        backend.to_label(),
                        FeaturesPageAction::SetPreferredGraphicsBackend(Some(backend)),
                    )
                }));
                dropdown.set_items(items, ctx);
            }
            let gpu_settings = GPUSettings::as_ref(ctx);
            dropdown.set_selected_by_name(
                gpu_settings
                    .preferred_backend
                    .map(|backend| backend.to_label())
                    .unwrap_or("Default"),
                ctx,
            );
        });
    }

    /// Updates the state of the tab behavior dropdown based on the current tab behavior.
    fn refresh_tab_behavior_dropdown(&mut self, ctx: &mut ViewContext<Self>) {
        self.tab_behavior_dropdown.update(ctx, |dropdown, ctx| {
            let mut items = vec![
                DropdownItem::new(
                    TabBehavior::Completions.dropdown_item_label(),
                    FeaturesPageAction::SetTabBehavior(TabBehavior::Completions),
                ),
                DropdownItem::new(
                    TabBehavior::Autosuggestions.dropdown_item_label(),
                    FeaturesPageAction::SetTabBehavior(TabBehavior::Autosuggestions),
                ),
            ];
            // We only show the "User defined" label if the user has manually edited their
            // Tab keybinding to neither completions nor autosuggestions. It's not a
            // selectable option from the dropdown.
            if matches!(*self.tab_behavior, TabBehavior::UserDefined) {
                items.push(DropdownItem::new(
                    TabBehavior::UserDefined.dropdown_item_label(),
                    FeaturesPageAction::SetTabBehavior(TabBehavior::UserDefined),
                ));
            }
            dropdown.set_items(items, ctx);
            dropdown.set_selected_by_name(self.tab_behavior.dropdown_item_label(), ctx);
        });
    }

    /// Retrieves the current completions/autosuggestions keybindings, determines the
    /// current tab behavior, and updates the state on the view.
    ///
    /// Note that this method does not update the state of the tab behavior dropdown.
    /// To update that, use `refresh_tab_behavior_dropdown`.
    fn refresh_tab_behavior_state(&mut self, ctx: &mut ViewContext<Self>) {
        // Get the current keybindings for opening the completions menu and inserting
        // autosuggestions.
        *self.completions_keystroke =
            keybinding_name_to_display_string(OPEN_COMPLETIONS_KEYBINDING_NAME, ctx)
                .unwrap_or_default();
        *self.autosuggestions_keystroke =
            keybinding_name_to_display_string(ACCEPT_AUTOSUGGESTION_KEYBINDING_NAME, ctx)
                .unwrap_or_default();

        // Determine the current tab behavior based on the user's keybindings.
        *self.tab_behavior = if self.completions_keystroke.to_uppercase() == TAB_KEYSTROKE_STR {
            TabBehavior::Completions
        } else if self.autosuggestions_keystroke.to_uppercase() == TAB_KEYSTROKE_STR {
            TabBehavior::Autosuggestions
        } else {
            TabBehavior::UserDefined
        };
    }

    /// Sets the keybindings for opening the completions menu and inserting autosuggestions
    /// when the user selects an option from the tab key behavior dropdown.
    fn set_tab_behavior_setting(
        &mut self,
        tab_behavior: &TabBehavior,
        ctx: &mut ViewContext<Self>,
    ) {
        match tab_behavior {
            TabBehavior::Completions => {
                // Set the binding for opening completions to Tab. If accepting autosuggestions
                // was previously bound to Tab, unbind it. We unbind because autosuggestions
                // are accepted via right arrow by default, so we don't need another binding.
                if *self.autosuggestions_keystroke == TAB_KEYSTROKE_STR {
                    reset_keybinding_to_default(ACCEPT_AUTOSUGGESTION_KEYBINDING_NAME, ctx);
                }
                set_custom_keybinding(OPEN_COMPLETIONS_KEYBINDING_NAME, &TAB_KEYSTROKE, ctx);
            }
            TabBehavior::Autosuggestions => {
                // Set the binding for accepting autosuggestions to Tab. If opening completions
                // was previously bound to Tab, set it to ctrl-space.
                if *self.completions_keystroke == TAB_KEYSTROKE_STR {
                    set_custom_keybinding(
                        OPEN_COMPLETIONS_KEYBINDING_NAME,
                        &CTRL_SPACE_KEYSTROKE,
                        ctx,
                    );
                }
                set_custom_keybinding(ACCEPT_AUTOSUGGESTION_KEYBINDING_NAME, &TAB_KEYSTROKE, ctx);
            }
            // No-op if the user selects "User defined" from the dropdown.
            TabBehavior::UserDefined => {}
        }

        self.refresh_tab_behavior_state(ctx);
    }

    fn new_tab_placement_dropdown_item_label(val: NewTabPlacement) -> &'static str {
        match val {
            NewTabPlacement::AfterAllTabs => "After all tabs",
            NewTabPlacement::AfterCurrentTab => "After current tab",
        }
    }

    fn set_new_tab_placement(&mut self, value: &NewTabPlacement, ctx: &mut ViewContext<Self>) {
        let _ = TabSettings::handle(ctx).update(ctx, |tab_settings, ctx| {
            tab_settings.new_tab_placement.set_value(*value, ctx)
        });
    }

    fn update_default_session_mode_dropdown(
        dropdown: ViewHandle<FilterableDropdown<FeaturesPageAction>>,
        ctx: &mut ViewContext<Self>,
    ) {
        dropdown.update(
            ctx,
            |dropdown: &mut FilterableDropdown<FeaturesPageAction>, ctx| {
                let is_ai_enabled = AISettings::as_ref(ctx).is_any_ai_enabled(ctx);

                if is_ai_enabled {
                    dropdown.set_enabled(ctx);
                } else {
                    dropdown.set_disabled(ctx);
                }

                let ai_settings = AISettings::as_ref(ctx);
                let current_mode = ai_settings.default_session_mode(ctx);
                let current_tab_config_path = ai_settings.default_tab_config_path().to_string();

                // Build items: built-in modes (skip TabConfig since configs are listed individually,
                // and skip DockerSandbox when its feature flag is disabled).
                let docker_sandbox_enabled = FeatureFlag::LocalDockerSandbox.is_enabled();
                let mut items: Vec<DropdownItem<FeaturesPageAction>> = DefaultSessionMode::iter()
                    .filter(|val| *val != DefaultSessionMode::TabConfig)
                    .filter(|val| {
                        *val != DefaultSessionMode::DockerSandbox || docker_sandbox_enabled
                    })
                    .map(|val| {
                        DropdownItem::new(
                            val.display_name(),
                            FeaturesPageAction::SetDefaultSessionMode(val),
                        )
                    })
                    .collect();

                // Append each loaded tab config
                let tab_configs = WarpConfig::as_ref(ctx).tab_configs().to_vec();
                for config in &tab_configs {
                    if let Some(path) = &config.source_path {
                        items.push(DropdownItem::new(
                            config.name.clone(),
                            FeaturesPageAction::SetDefaultTabConfig(
                                path.to_string_lossy().into_owned(),
                            ),
                        ));
                    }
                }

                dropdown.set_items(items, ctx);

                // Select the currently active item.
                let selected_name = match current_mode {
                    DefaultSessionMode::TabConfig => tab_configs
                        .iter()
                        .find(|c| {
                            c.source_path
                                .as_ref()
                                .is_some_and(|p| p.to_string_lossy() == current_tab_config_path)
                        })
                        .map(|c| c.name.clone())
                        .unwrap_or_else(|| DefaultSessionMode::Terminal.display_name().to_string()),
                    other => other.display_name().to_string(),
                };
                dropdown.set_selected_by_name(&selected_name, ctx);
            },
        );
    }

    fn set_default_session_mode(
        &mut self,
        value: &DefaultSessionMode,
        ctx: &mut ViewContext<Self>,
    ) {
        AISettings::handle(ctx).update(ctx, |ai_settings, ctx| {
            report_if_error!(ai_settings
                .default_session_mode_internal
                .set_value(*value, ctx));
        });
    }

    /// This function renders the component that allows the user to record a keybinding for the
    /// global hotkey. The default is to display the current keybinding, called the "summary".
    /// Once clicked, it tells the view to disable keybinding dispatching in order to record a
    /// keybinding. From there it can be saved or cancelled.
    #[allow(clippy::too_many_arguments)]
    fn render_keybinding_editor_row<T: Fn(&mut EventContext, Keystroke) + 'static>(
        &self,
        outer_button_mouse_state: Arc<Mutex<MouseState>>,
        cancel_button_mouse_state: Arc<Mutex<MouseState>>,
        save_button_mouse_state: Arc<Mutex<MouseState>>,
        keybinding_editor_state: KeybindingEditorState,
        keybinding: &KeyBindingModifyingState,
        editor_clicked_action: FeaturesPageAction,
        cancel_action: FeaturesPageAction,
        save_action: FeaturesPageAction,
        record_keystroke: T,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        Hoverable::new(outer_button_mouse_state, |state| {
            let background: Option<Fill> = if state.is_hovered() {
                Some(appearance.theme().accent().with_opacity(40).into())
            } else {
                None
            };

            let container =
                Container::new(if keybinding_editor_state == KeybindingEditorState::Idle {
                    self.render_summary(
                        keybinding,
                        keybinding_editor_state,
                        record_keystroke,
                        appearance,
                    )
                } else {
                    self.render_clicked(
                        cancel_button_mouse_state,
                        save_button_mouse_state,
                        keybinding,
                        keybinding_editor_state,
                        cancel_action,
                        save_action,
                        record_keystroke,
                        appearance,
                    )
                })
                .with_padding_left(10.)
                .with_padding_right(10.)
                .with_padding_top(10.)
                .with_margin_right(4.)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)));

            if let Some(background) = background {
                container.with_background(background).finish()
            } else {
                container.finish()
            }
        })
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(editor_clicked_action.clone());
        })
        .finish()
    }

    fn render_dropdown(
        &self,
        drop_down: &ViewHandle<Dropdown<FeaturesPageAction>>,
        width: f32,
    ) -> Box<dyn Element> {
        ConstrainedBox::new(ChildView::new(drop_down).finish())
            .with_width(width)
            .finish()
    }

    fn render_quake_width_height_editor(
        &self,
        quake_mode_settings: &QuakeModeSettings,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let editor_style = UiComponentStyles {
            width: Some(40.),
            padding: Some(Coords::uniform(5.)),
            background: Some(theme.surface_2().into()),
            ..Default::default()
        };
        Flex::column()
            .with_child(
                Container::new(
                    Flex::row()
                        .with_child(
                            Container::new(
                                Text::new_inline(
                                    "Width %",
                                    appearance.ui_font_family(),
                                    appearance.ui_font_size(),
                                )
                                .with_color(theme.active_ui_text_color().into())
                                .finish(),
                            )
                            .with_padding_right(8.5)
                            .finish(),
                        )
                        .with_child(
                            Dismiss::new(
                                appearance
                                    .ui_builder()
                                    .text_input(self.quake_mode_width_editor.clone())
                                    .with_style(editor_style)
                                    .build()
                                    .finish(),
                            )
                            .on_dismiss(|ctx, _app| {
                                ctx.dispatch_typed_action(
                                    FeaturesPageAction::QuakeEditorSetWidthPercentage,
                                )
                            })
                            .finish(),
                        )
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .finish(),
                )
                .with_padding_top(5.)
                .finish(),
            )
            .with_child(
                Container::new(
                    Flex::row()
                        .with_child(
                            Container::new(
                                Text::new_inline(
                                    "Height %",
                                    appearance.ui_font_family(),
                                    appearance.ui_font_size(),
                                )
                                .with_color(theme.active_ui_text_color().into())
                                .finish(),
                            )
                            .with_padding_right(5.)
                            .finish(),
                        )
                        .with_child(
                            Dismiss::new(
                                appearance
                                    .ui_builder()
                                    .text_input(self.quake_mode_height_editor.clone())
                                    .with_style(editor_style)
                                    .build()
                                    .finish(),
                            )
                            .on_dismiss(|ctx, _app| {
                                ctx.dispatch_typed_action(
                                    FeaturesPageAction::QuakeEditorSetHeightPercentage,
                                )
                            })
                            .finish(),
                        )
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .finish(),
                )
                .with_padding_top(5.)
                .finish(),
            )
            .with_child({
                let button = build_reset_button(
                    appearance,
                    self.button_mouse_states
                        .quake_mode_width_height_reset
                        .clone(),
                    quake_mode_settings.size_changed_from_default(),
                );

                button
                    .build()
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(FeaturesPageAction::QuakeEditorResetWidthHeight);
                    })
                    .finish()
            })
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .finish()
    }

    fn render_quake_mode_pin_window_toggle_row(
        &self,
        quake_mode_settings: &QuakeModeSettings,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        Container::new(
            Flex::row()
                .with_child(
                    appearance
                        .ui_builder()
                        .checkbox(
                            self.button_mouse_states.quake_mode_pin_window_check.clone(),
                            None,
                        )
                        .check(quake_mode_settings.hide_window_when_unfocused)
                        .build()
                        .on_click(move |ctx, _, _| {
                            ctx.dispatch_typed_action(
                                FeaturesPageAction::QuakeEditorTogglePinWindow,
                            )
                        })
                        .finish(),
                )
                .with_child(
                    appearance
                        .ui_builder()
                        .span("Autohides on loss of keyboard focus")
                        .build()
                        .with_margin_left(5.)
                        .finish(),
                )
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .finish(),
        )
        .with_margin_bottom(2.)
        .finish()
    }

    fn render_quake_mode_position_row(
        &self,
        quake_mode_settings: &QuakeModeSettings,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        Flex::row()
            .with_child(
                Container::new(
                    self.render_dropdown(
                        &self.quake_mode_pin_position_dropdown,
                        QUAKE_DROPDOWN_WIDTH,
                    ),
                )
                .with_padding_right(30.)
                .finish(),
            )
            .with_child(
                Container::new(
                    self.render_dropdown(
                        &self.quake_mode_pin_screen_dropdown,
                        QUAKE_DROPDOWN_WIDTH,
                    ),
                )
                .with_padding_right(30.)
                .finish(),
            )
            .with_child(self.render_quake_width_height_editor(quake_mode_settings, appearance))
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .finish()
    }

    fn render_long_running_notifications_setting(
        &self,
        notification_settings: &Notifications,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let font_size = appearance.ui_font_size() - 2.;
        let font_color = if notification_settings.is_long_running_enabled {
            theme.active_ui_text_color()
        } else {
            theme.nonactive_ui_text_color()
        };

        let editor_style = UiComponentStyles {
            width: Some(appearance.ui_font_size() * 3.),
            height: Some(appearance.ui_font_size() * 2.),
            padding: Some(Coords::uniform(5.)),
            background: Some(theme.surface_2().into()),
            ..Default::default()
        };

        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                Container::new(
                    Align::new(
                        appearance
                            .ui_builder()
                            .checkbox(
                                self.button_mouse_states
                                    .long_running_notifications_checkbox
                                    .clone(),
                                None,
                            )
                            .check(notification_settings.is_long_running_enabled)
                            .build()
                            .on_click(move |ctx, _, _| {
                                ctx.dispatch_typed_action(
                                    FeaturesPageAction::ToggleLongRunningNotifications,
                                );
                            })
                            .finish(),
                    )
                    .top_left()
                    .finish(),
                )
                .with_margin_right(NOTIFICATION_CHECKBOX_MARGIN_RIGHT)
                .finish(),
            )
            .with_child(
                Container::new(
                    Align::new(
                        Text::new_inline(
                            "When a command takes longer than",
                            appearance.ui_font_family(),
                            font_size,
                        )
                        .with_color(font_color.into())
                        .finish(),
                    )
                    .finish(),
                )
                .finish(),
            )
            .with_child(
                Container::new(
                    Dismiss::new(
                        appearance
                            .ui_builder()
                            .text_input(self.notifications_long_running_threshold_editor.clone())
                            .with_style(editor_style)
                            .build()
                            .finish(),
                    )
                    .on_dismiss(|ctx, _app| {
                        ctx.dispatch_typed_action(
                            FeaturesPageAction::SetLongRunningNotificationThreshold,
                        )
                    })
                    .finish(),
                )
                .with_margin_right(NOTIFICATION_EDITOR_MARGIN)
                .with_margin_left(NOTIFICATION_EDITOR_MARGIN)
                .finish(),
            )
            .with_child(
                Container::new(
                    Align::new(
                        Text::new_inline(
                            "seconds to complete",
                            appearance.ui_font_family(),
                            font_size,
                        )
                        .with_color(font_color.into())
                        .finish(),
                    )
                    .finish(),
                )
                .finish(),
            )
            .finish()
    }

    fn render_notification_toggle(
        &self,
        is_enabled: bool,
        text: &str,
        toggle_action: FeaturesPageAction,
        mouse_state: Arc<Mutex<MouseState>>,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let text = text.to_string();
        let font_size = appearance.ui_font_size() - 2.;
        let font_color = if is_enabled {
            appearance.theme().active_ui_text_color()
        } else {
            appearance.theme().nonactive_ui_text_color()
        };
        Container::new(
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(
                    Container::new(
                        Align::new(
                            appearance
                                .ui_builder()
                                .checkbox(mouse_state.clone(), None)
                                .check(is_enabled)
                                .build()
                                .on_click(move |ctx, _, _| {
                                    let toggle_action = toggle_action.clone();
                                    ctx.dispatch_typed_action(toggle_action);
                                })
                                .finish(),
                        )
                        .top_left()
                        .finish(),
                    )
                    .with_margin_right(NOTIFICATION_CHECKBOX_MARGIN_RIGHT)
                    .finish(),
                )
                .with_child(
                    Container::new(
                        Align::new(
                            Text::new_inline(text, appearance.ui_font_family(), font_size)
                                .with_color(font_color.into())
                                .finish(),
                        )
                        .finish(),
                    )
                    .finish(),
                )
                .finish(),
        )
        .finish()
    }

    fn render_summary<T: Fn(&mut EventContext, Keystroke) + 'static>(
        &self,
        keybinding: &KeyBindingModifyingState,
        keybinding_editor_state: KeybindingEditorState,
        record_keystroke: T,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let element = Container::new(
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(
                    Shrinkable::new(
                        2.,
                        Align::new(
                            Text::new_inline("Keybinding", appearance.ui_font_family(), 13.)
                                .with_color(appearance.theme().active_ui_text_color().into())
                                .finish(),
                        )
                        .left()
                        .finish(),
                    )
                    .finish(),
                )
                .with_child(
                    Shrinkable::new(
                        1.,
                        Align::new(if let Some(keybinding) = &keybinding.unsaved_binding {
                            appearance
                                .ui_builder()
                                .keyboard_shortcut(keybinding)
                                .build()
                                .finish()
                        } else {
                            appearance
                                .ui_builder()
                                .paragraph("Click to set global hotkey".to_string())
                                .build()
                                .finish()
                        })
                        .finish(),
                    )
                    .finish(),
                )
                .finish(),
        )
        .with_padding_bottom(10.)
        .finish();

        match keybinding_editor_state {
            KeybindingEditorState::Recording => {
                EventHandler::new(element)
                    .on_keydown(move |ctx, _, keystroke| {
                        let mut keystroke = keystroke.clone();

                        // For global hotkey we don't support meta keys.
                        if keystroke.meta {
                            keystroke.alt = true;
                            keystroke.meta = false;
                        }

                        record_keystroke(ctx, keystroke);
                        DispatchEventResult::StopPropagation
                    })
                    .finish()
            }
            _ => element,
        }
    }

    /// This renders the keybinding editor once it's clicked, and is listening to record a new
    /// keybinding.
    #[allow(clippy::too_many_arguments)]
    fn render_clicked<T: Fn(&mut EventContext, Keystroke) + 'static>(
        &self,
        cancel_button_mouse_state: Arc<Mutex<MouseState>>,
        save_button_mouse_state: Arc<Mutex<MouseState>>,
        keybinding: &KeyBindingModifyingState,
        keybinding_editor_state: KeybindingEditorState,
        cancel_action: FeaturesPageAction,
        save_action: FeaturesPageAction,
        record_keystroke: T,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let cancel_button = appearance
            .ui_builder()
            .button(ButtonVariant::Text, cancel_button_mouse_state)
            .with_style(UiComponentStyles {
                padding: Some(Coords::default().right(10.)),
                ..Default::default()
            })
            .with_text_label("Cancel".to_string())
            .build()
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(cancel_action.clone());
            })
            .finish();

        let save_button = Shrinkable::new(
            1.,
            appearance
                .ui_builder()
                .button(ButtonVariant::Text, save_button_mouse_state)
                .with_text_label("Save".to_string())
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(save_action.clone());
                })
                .finish(),
        )
        .finish();

        Container::new(
            Flex::column()
                .with_child(self.render_summary(
                    keybinding,
                    keybinding_editor_state,
                    record_keystroke,
                    appearance,
                ))
                .with_child(
                    Flex::row()
                        .with_child(
                            Shrinkable::new(
                                2.,
                                Align::new(
                                    Text::new_inline(
                                        "Press new keyboard shortcut",
                                        appearance.ui_font_family(),
                                        13.,
                                    )
                                    .with_color(appearance.theme().active_ui_text_color().into())
                                    .finish(),
                                )
                                .left()
                                .finish(),
                            )
                            .finish(),
                        )
                        .with_child(
                            Shrinkable::new(
                                1.,
                                Align::new(
                                    Flex::row()
                                        .with_child(cancel_button)
                                        .with_child(save_button)
                                        .finish(),
                                )
                                .left()
                                .finish(),
                            )
                            .finish(),
                        )
                        .finish(),
                )
                .finish(),
        )
        .with_padding_bottom(10.)
        .finish()
    }

    fn render_change_keybinding_button(
        &self,
        keybinding_name: &str,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let keybinding_name = keybinding_name.to_string();
        Hoverable::new(
            self.button_mouse_states.change_keybinding.clone(),
            |state| {
                let button_color = appearance.theme().accent().into_solid();

                let mut border = Border::bottom(1.);
                if state.is_hovered() {
                    border = border.with_border_color(button_color);
                }

                Container::new(
                    Text::new_inline("Change keybinding", appearance.ui_font_family(), 12.)
                        .with_color(button_color)
                        .finish(),
                )
                .with_border(border)
                .finish()
            },
        )
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(FeaturesPageAction::SearchForKeybinding(
                keybinding_name.clone(),
            ));
        })
        .with_cursor(Cursor::PointingHand)
        .finish()
    }

    fn render_setting_subgroup_item(
        &self,
        appearance: &Appearance,
        local_only_icon_state: LocalOnlyIconState,
        switch: Box<dyn Element>,
        label_text: String,
    ) -> Box<dyn Element> {
        Container::new(
            Flex::row()
                .with_child(
                    Shrinkable::new(
                        1.0,
                        Container::new(
                            Align::new(render_body_item_label::<FeaturesPageAction>(
                                label_text,
                                None,
                                None,
                                local_only_icon_state,
                                ToggleState::Enabled,
                                appearance,
                            ))
                            .left()
                            .finish(),
                        )
                        .with_padding_top(5.)
                        .finish(),
                    )
                    .finish(),
                )
                .with_child(
                    Container::new(switch)
                        .with_padding_right(TOGGLE_BUTTON_RIGHT_PADDING)
                        .finish(),
                )
                .finish(),
        )
        .with_padding_top(4.)
        .with_padding_bottom(4.)
        .finish()
    }
}

impl SettingsPageMeta for FeaturesPageView {
    fn section() -> SettingsSection {
        SettingsSection::Features
    }

    fn should_render(&self, _ctx: &AppContext) -> bool {
        true
    }

    fn on_page_selected(&mut self, _: bool, ctx: &mut ViewContext<Self>) {
        // On MacOS, we rely on [`warpui::platform::AppCallbacks::on_screen_changed`] to update and
        // notify on the [`DisplayCount`] model. However, no mechanism exists on Linux to trigger
        // that callback. As a workaround, we check for updates here where quake mode is
        // configured.
        #[cfg(target_os = "linux")]
        DisplayCount::handle(ctx).update(ctx, |display_count, ctx| {
            display_count.0 = ctx.windows().display_count();
            ctx.notify();
        });

        // Make sure we're not already showing the hint text for the SSH wrapper
        // toggle when the user switches to the page.
        self.ssh_wrapper_toggled = false;

        // Fetch the latest tab behavior state in case the user changed their keybindings
        // since we last loaded this page.
        self.refresh_tab_behavior_state(ctx);
        self.refresh_tab_behavior_dropdown(ctx);
        self.refresh_preferred_graphics_backend_dropdown(ctx);
    }

    fn update_filter(&mut self, query: &str, ctx: &mut ViewContext<Self>) -> MatchData {
        self.page.update_filter(query, ctx)
    }

    fn scroll_to_widget(&mut self, widget_id: &'static str) {
        self.page.scroll_to_widget(widget_id)
    }

    fn clear_highlighted_widget(&mut self) {
        self.page.clear_highlighted_widget();
    }
}

impl From<ViewHandle<FeaturesPageView>> for SettingsPageViewHandle {
    fn from(view_handle: ViewHandle<FeaturesPageView>) -> Self {
        SettingsPageViewHandle::Features(view_handle)
    }
}

pub(super) fn render_group(
    children: impl IntoIterator<Item = Box<dyn Element>>,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let bar = Container::new(
        ConstrainedBox::new(Empty::new().finish())
            .with_width(4.)
            .finish(),
    )
    .with_background(appearance.theme().outline())
    .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)))
    .with_margin_right(8.)
    .with_margin_left(8.)
    .finish();

    Container::new(
        Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(bar)
            .with_child(
                Shrinkable::new(1., Flex::column().with_children(children).finish()).finish(),
            )
            .finish(),
    )
    .with_margin_top(-4.)
    .with_margin_bottom(HEADER_PADDING)
    .finish()
}

fn init_global_hotkey_dropdown(
    hotkey_mode: GlobalHotkeyMode,
    dropdown: &mut Dropdown<FeaturesPageAction>,
    ctx: &mut ViewContext<Dropdown<FeaturesPageAction>>,
) {
    let items = vec![
        DropdownItem::new(
            GlobalHotkeyMode::Disabled.as_dropdown_label(),
            FeaturesPageAction::SetGlobalHotkeyMode(GlobalHotkeyMode::Disabled),
        ),
        DropdownItem::new(
            GlobalHotkeyMode::QuakeMode.as_dropdown_label(),
            FeaturesPageAction::SetGlobalHotkeyMode(GlobalHotkeyMode::QuakeMode),
        ),
        DropdownItem::new(
            GlobalHotkeyMode::ActivationHotkey.as_dropdown_label(),
            FeaturesPageAction::SetGlobalHotkeyMode(GlobalHotkeyMode::ActivationHotkey),
        ),
    ];

    dropdown.set_items(items, ctx);
    dropdown.set_selected_by_name(hotkey_mode.as_dropdown_label(), ctx);
}

fn init_display_count_dropdown(
    quake_mode_settings: &QuakeModeSettings,
    display_count: usize,
    dropdown: &mut Dropdown<FeaturesPageAction>,
    ctx: &mut ViewContext<Dropdown<FeaturesPageAction>>,
) {
    let no_preference = DropdownItem::new(
        "Active Screen",
        //|| {
        FeaturesPageAction::QuakeEditorSetPinScreen(None), //}
    );
    let mut items = vec![no_preference];
    items.push(DropdownItem::new(
        format!("{}", DisplayIdx::Primary),
        //move ||
        FeaturesPageAction::QuakeEditorSetPinScreen(Some(DisplayIdx::Primary)),
    ));

    (1..display_count).for_each(|idx| {
        items.push(DropdownItem::new(
            format!("{}", DisplayIdx::External(idx - 1)),
            //move || {
            FeaturesPageAction::QuakeEditorSetPinScreen(Some(DisplayIdx::External(idx - 1))), //},
        ));
    });

    dropdown.set_items(items, ctx);
    match quake_mode_settings.pin_screen {
        Some(idx) if idx.is_valid_given_display_count(display_count) => {
            dropdown.set_selected_by_name(format!("{idx}"), ctx)
        }
        _ => dropdown.set_selected_by_name("Active Screen", ctx),
    };
}

#[derive(Default)]
struct NativeRedirectWidget {
    additional_info_link: MouseStateHandle,
    switch_state: SwitchStateHandle,
}

impl SettingsWidget for NativeRedirectWidget {
    type View = FeaturesPageView;

    fn search_terms(&self) -> &str {
        "link open desktop native redirect url intent deep link deeplink"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let ui_builder = appearance.ui_builder();
        render_body_item::<FeaturesPageAction>(
            "Open links in desktop app".into(),
            Some(AdditionalInfo {
                mouse_state: self.additional_info_link.clone(),
                on_click_action: None,
                secondary_text: None,
                tooltip_override_text: Some(
                    "Automatically open links in desktop app whenever possible.".into(),
                ),
            }),
            LocalOnlyIconState::for_setting(
                UserNativeRedirectPreference::storage_key(),
                UserNativeRedirectPreference::sync_to_cloud(),
                &mut view
                    .button_mouse_states
                    .local_only_icon_tooltip_states
                    .borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            ui_builder
                .switch(self.switch_state.clone())
                .check(matches!(
                    NativePreferenceSettings::as_ref(app)
                        .user_native_redirect_preference
                        .value(),
                    UserNativePreference::Desktop
                ))
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(FeaturesPageAction::ToggleOpenLinksInDesktopApp);
                })
                .finish(),
            None,
        )
    }
}

#[derive(Default)]
struct SessionRestorationWidget {
    switch_state: SwitchStateHandle,
    additional_info_link: MouseStateHandle,
    docs_link: MouseStateHandle,
}

impl SettingsWidget for SessionRestorationWidget {
    type View = FeaturesPageView;

    fn search_terms(&self) -> &str {
        "restore session window tab pane startup"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let ui_builder = appearance.ui_builder();

        let switch = ui_builder
            .switch(self.switch_state.clone())
            .check(*GeneralSettings::as_ref(app).restore_session)
            .build()
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(FeaturesPageAction::ToggleRestoreSession);
            })
            .finish();

        let labeled_switch = render_body_item::<FeaturesPageAction>(
            "Restore windows, tabs, and panes on startup".into(),
            Some(AdditionalInfo {
                mouse_state: self.additional_info_link.clone(),
                on_click_action: Some(FeaturesPageAction::OpenUrl(
                    "https://docs.warp.dev/terminal/sessions/session-restoration".into(),
                )),
                secondary_text: None,
                tooltip_override_text: None,
            }),
            LocalOnlyIconState::for_setting(
                RestoreSession::storage_key(),
                RestoreSession::sync_to_cloud(),
                &mut view
                    .button_mouse_states
                    .local_only_icon_tooltip_states
                    .borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            switch,
            None,
        );

        if app.is_wayland() {
            let message = Text::new_inline(
                "Window positions won't be restored on Wayland. ",
                appearance.ui_font_family(),
                CONTENT_FONT_SIZE,
            )
            .with_color(appearance.theme().disabled_ui_text_color().into())
            .finish();

            let link = ui_builder
                .link(
                    "See docs.".to_owned(),
                    Some("https://docs.warp.dev/terminal/sessions/session-restoration".to_owned()),
                    None,
                    self.docs_link.clone(),
                )
                .soft_wrap(false)
                .build()
                .finish();

            Flex::column()
                .with_children([
                    labeled_switch,
                    Container::new(
                        Flex::row()
                            .with_children([message, link])
                            .with_main_axis_alignment(MainAxisAlignment::End)
                            .finish(),
                    )
                    .with_padding_bottom(HEADER_PADDING)
                    .finish(),
                ])
                .with_main_axis_size(MainAxisSize::Min)
                .finish()
        } else {
            labeled_switch
        }
    }
}

#[derive(Default)]
struct SnackbarHeaderWidget {
    switch_state: SwitchStateHandle,
    additional_info_link: MouseStateHandle,
}

impl SettingsWidget for SnackbarHeaderWidget {
    type View = FeaturesPageView;

    fn search_terms(&self) -> &str {
        "sticky command block header snackbar"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let ui_builder = appearance.ui_builder();
        render_body_item::<FeaturesPageAction>(
            "Show sticky command header".into(),
            Some(AdditionalInfo {
                mouse_state: self.additional_info_link.clone(),
                on_click_action: Some(FeaturesPageAction::OpenUrl(
                    "https://docs.warp.dev/terminal/blocks/sticky-command-header".into(),
                )),
                secondary_text: None,
                tooltip_override_text: None,
            }),
            LocalOnlyIconState::for_setting(
                SnackbarEnabled::storage_key(),
                SnackbarEnabled::sync_to_cloud(),
                &mut view
                    .button_mouse_states
                    .local_only_icon_tooltip_states
                    .borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            ui_builder
                .switch(self.switch_state.clone())
                .check(*BlockListSettings::as_ref(app).snackbar_enabled)
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(FeaturesPageAction::ToggleSnackbar)
                })
                .finish(),
            None,
        )
    }
}

#[derive(Default)]
struct LinkTooltipWidget {
    switch_state: SwitchStateHandle,
}

impl SettingsWidget for LinkTooltipWidget {
    type View = FeaturesPageView;

    fn search_terms(&self) -> &str {
        "link tooltip click open"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let ui_builder = appearance.ui_builder();
        render_body_item::<FeaturesPageAction>(
            "Show tooltip on click on links".into(),
            None,
            LocalOnlyIconState::for_setting(
                LinkTooltip::storage_key(),
                LinkTooltip::sync_to_cloud(),
                &mut view
                    .button_mouse_states
                    .local_only_icon_tooltip_states
                    .borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            ui_builder
                .switch(self.switch_state.clone())
                .check(*GeneralSettings::as_ref(app).link_tooltip)
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(FeaturesPageAction::ToggleLinkTooltip);
                })
                .finish(),
            None,
        )
    }
}

#[cfg(feature = "local_fs")]
#[derive(Default)]
struct ExternalEditorWidget {}

#[cfg(feature = "local_fs")]
impl SettingsWidget for ExternalEditorWidget {
    type View = FeaturesPageView;

    fn search_terms(&self) -> &str {
        "editor open files markdown AI conversations layout pane tab"
    }

    fn render(
        &self,
        view: &Self::View,
        _appearance: &Appearance,
        _app: &AppContext,
    ) -> Box<dyn Element> {
        ChildView::new(&view.external_editor_view).finish()
    }
}

#[derive(Default)]
struct QuitWarningModalWidget {
    switch_state: SwitchStateHandle,
}

impl SettingsWidget for QuitWarningModalWidget {
    type View = FeaturesPageView;

    fn search_terms(&self) -> &str {
        "warning popup modal dialog quit logout log out close"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let general_settings = GeneralSettings::as_ref(app);
        let ui_builder = appearance.ui_builder();
        render_body_item::<FeaturesPageAction>(
            "Show warning before quitting/logging out".into(),
            None,
            LocalOnlyIconState::for_setting(
                ShowWarningBeforeQuitting::storage_key(),
                ShowWarningBeforeQuitting::sync_to_cloud(),
                &mut view
                    .button_mouse_states
                    .local_only_icon_tooltip_states
                    .borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            ui_builder
                .switch(self.switch_state.clone())
                .check(*general_settings.show_warning_before_quitting)
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(FeaturesPageAction::ToggleShowWarningBeforeQuitting);
                })
                .finish(),
            None,
        )
    }
}

#[derive(Default)]
struct LoginItemWidget {
    switch_state: SwitchStateHandle,
}

impl SettingsWidget for LoginItemWidget {
    type View = FeaturesPageView;

    fn search_terms(&self) -> &str {
        "login item startup start mac windows app restart automatic"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let general_settings = GeneralSettings::as_ref(app);
        let ui_builder = appearance.ui_builder();
        #[cfg(target_os = "macos")]
        let label = "Start Warp at login (requires macOS 13+)";
        #[cfg(not(target_os = "macos"))]
        let label = "Start Warp at login";
        render_body_item::<FeaturesPageAction>(
            label.into(),
            None,
            LocalOnlyIconState::for_setting(
                LoginItem::storage_key(),
                LoginItem::sync_to_cloud(),
                &mut view
                    .button_mouse_states
                    .local_only_icon_tooltip_states
                    .borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            ui_builder
                .switch(self.switch_state.clone())
                .check(*general_settings.add_app_as_login_item)
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(FeaturesPageAction::ToggleLoginItem);
                })
                .finish(),
            None,
        )
    }
}

#[derive(Default)]
struct QuitWhenAllWindowsClosedWidget {
    switch_state: SwitchStateHandle,
}

impl SettingsWidget for QuitWhenAllWindowsClosedWidget {
    type View = FeaturesPageView;

    fn search_terms(&self) -> &str {
        "quit all windows closed"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let general_settings = GeneralSettings::as_ref(app);
        let ui_builder = appearance.ui_builder();
        render_body_item::<FeaturesPageAction>(
            "Quit when all windows are closed".into(),
            None,
            LocalOnlyIconState::for_setting(
                QuitOnLastWindowClosed::storage_key(),
                QuitOnLastWindowClosed::sync_to_cloud(),
                &mut view
                    .button_mouse_states
                    .local_only_icon_tooltip_states
                    .borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            ui_builder
                .switch(self.switch_state.clone())
                .check(*general_settings.quit_on_last_window_closed)
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(FeaturesPageAction::ToggleQuitOnLastWindowClosed);
                })
                .finish(),
            None,
        )
    }
}

#[derive(Default)]
struct ShowChangelogWidget {
    switch_state: SwitchStateHandle,
}

impl SettingsWidget for ShowChangelogWidget {
    type View = FeaturesPageView;

    fn search_terms(&self) -> &str {
        "changelog updates"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let changelog_settings = ChangelogSettings::as_ref(app);
        let ui_builder = appearance.ui_builder();
        render_body_item::<FeaturesPageAction>(
            "Show changelog toast after updates".into(),
            None,
            LocalOnlyIconState::for_setting(
                ShowChangelogAfterUpdate::storage_key(),
                ShowChangelogAfterUpdate::sync_to_cloud(),
                &mut view
                    .button_mouse_states
                    .local_only_icon_tooltip_states
                    .borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            ui_builder
                .switch(self.switch_state.clone())
                .check(*changelog_settings.show_changelog_after_update)
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(FeaturesPageAction::ToggleShowChangelogAfterUpdate);
                })
                .finish(),
            None,
        )
    }
}

#[derive(Default)]
struct MouseScrollMultiplierWidget {
    additional_info_link: MouseStateHandle,
}

impl SettingsWidget for MouseScrollMultiplierWidget {
    type View = FeaturesPageView;

    fn search_terms(&self) -> &str {
        "mouse scroll wheel multiplier lines"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let border_color = match view.valid_mouse_scroll_multiplier {
            false => Some(themes::theme::Fill::error().into()),
            true => None,
        };
        let input_field = appearance
            .ui_builder()
            .text_input(view.mouse_scroll_input_editor.clone())
            .with_style(UiComponentStyles {
                width: Some(MOUSE_SCROLL_EDITOR_WIDTH),
                padding: Some(Coords {
                    top: 4.,
                    bottom: 4.,
                    left: 6.,
                    right: 6.,
                }),
                background: Some(appearance.theme().surface_2().into()),
                border_color,
                ..Default::default()
            })
            .build()
            .finish();
        let input_column = Flex::column()
            .with_children([
                input_field,
                if view.valid_mouse_scroll_multiplier {
                    Empty::new().finish()
                } else {
                    appearance
                        .ui_builder()
                        .wrappable_text("Allowed Values: 1-20", true)
                        .with_style(UiComponentStyles {
                            font_color: Some(themes::theme::Fill::error().into_solid()),
                            ..Default::default()
                        })
                        .build()
                        .finish()
                },
            ])
            .with_cross_axis_alignment(CrossAxisAlignment::End)
            .finish();

        render_body_item::<FeaturesPageAction>(
            "Lines scrolled by mouse wheel interval".into(),
            Some(AdditionalInfo {
                mouse_state: self.additional_info_link.clone(),
                on_click_action: None,
                secondary_text: None,
                tooltip_override_text: Some(
                    "Supports floating point values between 1 and 20.".to_string(),
                ),
            }),
            LocalOnlyIconState::for_setting(
                MouseScrollMultiplier::storage_key(),
                MouseScrollMultiplier::sync_to_cloud(),
                &mut view
                    .button_mouse_states
                    .local_only_icon_tooltip_states
                    .borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            input_column,
            None,
        )
    }
}

#[derive(Default)]
struct AutoOpenCodeReviewPaneWidget {
    switch_state: SwitchStateHandle,
}

impl SettingsWidget for AutoOpenCodeReviewPaneWidget {
    type View = FeaturesPageView;

    fn search_terms(&self) -> &str {
        "oz auto open code review pane panel agent mode change first time accepted diff view conversation"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let general_settings = GeneralSettings::as_ref(app);
        let ui_builder = appearance.ui_builder();
        render_body_item::<FeaturesPageAction>(
            "Auto open code review panel".into(),
            None,
            LocalOnlyIconState::for_setting(
                AutoOpenCodeReviewPaneOnFirstAgentChange::storage_key(),
                AutoOpenCodeReviewPaneOnFirstAgentChange::sync_to_cloud(),
                &mut view
                    .button_mouse_states
                    .local_only_icon_tooltip_states
                    .borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            ui_builder
                .switch(self.switch_state.clone())
                .check(*general_settings.auto_open_code_review_pane_on_first_agent_change)
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(FeaturesPageAction::ToggleAutoOpenCodeReviewPane);
                })
                .finish(),
            Some("When this setting is on, the code review panel will open on the first accepted diff of a conversation".into()),
        )
    }
}

#[derive(Default)]
struct DefaultTerminalWidget {
    link_state: MouseStateHandle,
}

impl SettingsWidget for DefaultTerminalWidget {
    type View = FeaturesPageView;

    fn search_terms(&self) -> &str {
        "warp default terminal application"
    }

    fn render(
        &self,
        _view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let ui_builder = appearance.ui_builder();
        let default_terminal = DefaultTerminal::as_ref(app);
        if default_terminal.is_warp_default() {
            ui_builder
                .wrappable_text("Warp is the default terminal", true)
                .with_style(UiComponentStyles {
                    font_color: Some(appearance.theme().disabled_ui_text_color().into()),
                    margin: Some(Coords::default().bottom(16.)),
                    ..Default::default()
                })
                .build()
                .finish()
        } else {
            ui_builder
                .link(
                    "Make Warp the default terminal".to_string(),
                    None,
                    Some(Box::new(|ctx| {
                        ctx.dispatch_typed_action(FeaturesPageAction::MakeWarpDefaultTerminal);
                    })),
                    self.link_state.clone(),
                )
                .build()
                .with_margin_bottom(16.)
                .finish()
        }
    }
}

#[derive(Default)]
struct BlockLimitWidget {}

impl SettingsWidget for BlockLimitWidget {
    type View = FeaturesPageView;

    fn search_terms(&self) -> &str {
        "max block size lines maximum limit memory"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let border_color: Option<Fill> = match view.valid_max_block_size {
            false => Some(themes::theme::Fill::error().into()),
            true => Default::default(),
        };
        let input_field = appearance
            .ui_builder()
            .text_input(view.max_block_size_input_editor.clone())
            .with_style(UiComponentStyles {
                width: Some(MAX_BLOCK_SIZE_INPUT_BOX_WIDTH),
                padding: Some(Coords {
                    top: 4.,
                    bottom: 4.,
                    left: 6.,
                    right: 6.,
                }),
                background: Some(appearance.theme().surface_2().into()),
                border_color,
                ..Default::default()
            })
            .build()
            .finish();

        render_body_item::<FeaturesPageAction>(
            "Maximum rows in a block".into(),
            None,
            LocalOnlyIconState::for_setting(
                MaximumGridSize::storage_key(),
                MaximumGridSize::sync_to_cloud(),
                &mut view
                    .button_mouse_states
                    .local_only_icon_tooltip_states
                    .borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            input_field,
            Some(block_maximum_rows_description()),
        )
    }
}

#[derive(Default)]
struct SSHWrapperWidget {
    additional_info_link: MouseStateHandle,
    switch_state: SwitchStateHandle,
}

impl SettingsWidget for SSHWrapperWidget {
    type View = FeaturesPageView;

    fn search_terms(&self) -> &str {
        "ssh wrapper"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let ui_builder = appearance.ui_builder();
        render_body_item::<FeaturesPageAction>(
            "Warp SSH Wrapper".into(),
            Some(AdditionalInfo {
                mouse_state: self.additional_info_link.clone(),
                on_click_action: Some(FeaturesPageAction::OpenUrl(
                    "https://docs.warp.dev/terminal/warpify/ssh-legacy#implementation".into(),
                )),
                secondary_text: if view.ssh_wrapper_toggled {
                    Some("This change will take effect in new sessions".to_string())
                } else {
                    None
                },
                tooltip_override_text: None,
            }),
            LocalOnlyIconState::for_setting(
                EnableSshWrapper::storage_key(),
                EnableSshWrapper::sync_to_cloud(),
                &mut view
                    .button_mouse_states
                    .local_only_icon_tooltip_states
                    .borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            ui_builder
                .switch(self.switch_state.clone())
                .check(*SshSettings::as_ref(app).enable_legacy_ssh_wrapper.value())
                .build()
                .on_click(move |ctx, _, _| {
                    #[allow(deprecated)]
                    ctx.dispatch_typed_action(FeaturesPageAction::ToggleSshWrapper);
                })
                .finish(),
            None,
        )
    }
}

#[derive(Default)]
struct DesktopNotificationsWidget {
    additional_info_link: MouseStateHandle,
    switch_state: SwitchStateHandle,
}

impl SettingsWidget for DesktopNotificationsWidget {
    type View = FeaturesPageView;

    fn search_terms(&self) -> &str {
        "desktop notifications"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let session_settings = SessionSettings::as_ref(app);
        let ui_builder = appearance.ui_builder();
        let mut column = Flex::column();
        column.add_child(render_body_item::<FeaturesPageAction>(
            "Receive desktop notifications from Warp".into(),
            Some(AdditionalInfo {
                mouse_state: self.additional_info_link.clone(),
                on_click_action: Some(FeaturesPageAction::OpenUrl(NOTIFICATIONS_DOCS_URL.into())),
                secondary_text: None,
                tooltip_override_text: None,
            }),
            LocalOnlyIconState::for_setting(
                Notifications::storage_key(),
                Notifications::sync_to_cloud(),
                &mut view
                    .button_mouse_states
                    .local_only_icon_tooltip_states
                    .borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            ui_builder
                .switch(self.switch_state.clone())
                .check(matches!(
                    session_settings.notifications.mode,
                    NotificationsMode::Enabled
                ))
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(FeaturesPageAction::ToggleNotifications);
                })
                .finish(),
            None,
        ));

        if matches!(
            session_settings.notifications.mode,
            NotificationsMode::Enabled
        ) {
            let toggles = vec![
                view.render_notification_toggle(
                    session_settings
                        .notifications
                        .is_agent_task_completed_enabled,
                    "Notify when an agent completes a task",
                    FeaturesPageAction::ToggleAgentTaskCompletedNotifications,
                    view.button_mouse_states
                        .agent_task_completed_notifications_checkbox
                        .clone(),
                    appearance,
                ),
                view.render_long_running_notifications_setting(
                    &session_settings.notifications,
                    appearance,
                ),
                view.render_notification_toggle(
                    session_settings.notifications.is_needs_attention_enabled,
                    "Notify when a command or agent needs your attention to continue",
                    FeaturesPageAction::ToggleNeedsAttentionNotifications,
                    view.button_mouse_states
                        .agent_needs_attention_notifications_checkbox
                        .clone(),
                    appearance,
                ),
                // Add notification sound toggle only on macOS
                #[cfg(target_os = "macos")]
                {
                    view.render_notification_toggle(
                        session_settings.notifications.play_notification_sound,
                        "Play notification sounds",
                        FeaturesPageAction::ToggleNotificationSound,
                        view.button_mouse_states.notification_sound_checkbox.clone(),
                        appearance,
                    )
                },
            ];

            column.add_child(render_group(toggles, appearance));
        }

        if FeatureFlag::HOANotifications.is_enabled() {
            let ai_settings = AISettings::as_ref(app);
            let show_agent_notifications = *ai_settings.show_agent_notifications;
            column.add_child(render_body_item::<FeaturesPageAction>(
                "Show in-app agent notifications".into(),
                None,
                LocalOnlyIconState::Hidden,
                ToggleState::Enabled,
                appearance,
                ui_builder
                    .switch(
                        view.button_mouse_states
                            .agent_in_app_notifications_switch
                            .clone(),
                    )
                    .check(show_agent_notifications)
                    .build()
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(
                            FeaturesPageAction::ToggleAgentInAppNotifications,
                        );
                    })
                    .finish(),
                None,
            ));

            if show_agent_notifications {
                let theme = appearance.theme();
                let font_size = appearance.ui_font_size() - 2.;
                let font_color = theme.active_ui_text_color();

                let editor_style = UiComponentStyles {
                    width: Some(appearance.ui_font_size() * 3.),
                    height: Some(appearance.ui_font_size() * 2.),
                    padding: Some(Coords::uniform(5.)),
                    background: Some(theme.surface_2().into()),
                    ..Default::default()
                };

                let toast_duration_row = Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_child(
                        Text::new_inline(
                            "Toast notifications stay visible for",
                            appearance.ui_font_family(),
                            font_size,
                        )
                        .with_color(font_color.into())
                        .finish(),
                    )
                    .with_child(
                        Container::new(
                            Dismiss::new(
                                appearance
                                    .ui_builder()
                                    .text_input(view.notification_toast_duration_editor.clone())
                                    .with_style(editor_style)
                                    .build()
                                    .finish(),
                            )
                            .on_dismiss(|ctx, _app| {
                                ctx.dispatch_typed_action(
                                    FeaturesPageAction::SetNotificationToastDuration,
                                )
                            })
                            .finish(),
                        )
                        .with_margin_right(NOTIFICATION_EDITOR_MARGIN)
                        .with_margin_left(NOTIFICATION_EDITOR_MARGIN)
                        .finish(),
                    )
                    .with_child(
                        Text::new_inline("seconds", appearance.ui_font_family(), font_size)
                            .with_color(font_color.into())
                            .finish(),
                    )
                    .finish();

                column.add_child(render_group(vec![toast_duration_row], appearance));
            }
        }

        column.finish()
    }
}

#[cfg(feature = "local_tty")]
#[derive(Default)]
struct StartupShellWidget {}

#[cfg(feature = "local_tty")]
impl SettingsWidget for StartupShellWidget {
    type View = FeaturesPageView;

    fn search_terms(&self) -> &str {
        "startup shell session"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        Flex::column()
            .with_children([
                render_sub_sub_header(
                    appearance,
                    "Default shell for new sessions".to_string(),
                    Some(LocalOnlyIconState::for_setting(
                        StartupShellOverride::storage_key(),
                        StartupShellOverride::sync_to_cloud(),
                        &mut view
                            .button_mouse_states
                            .local_only_icon_tooltip_states
                            .borrow_mut(),
                        app,
                    )),
                ),
                ChildView::new(&view.startup_shell_view).finish(),
            ])
            .finish()
    }
}

#[cfg(feature = "local_tty")]
#[derive(Default)]
struct WorkingDirectoryWidget {}

#[cfg(feature = "local_tty")]
impl SettingsWidget for WorkingDirectoryWidget {
    type View = FeaturesPageView;

    fn search_terms(&self) -> &str {
        "starting working directory pwd session"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        Flex::column()
            .with_children([
                render_sub_sub_header(
                    appearance,
                    "Working directory for new sessions".to_string(),
                    Some(LocalOnlyIconState::for_setting(
                        WorkingDirectoryConfig::storage_key(),
                        WorkingDirectoryConfig::sync_to_cloud(),
                        &mut view
                            .button_mouse_states
                            .local_only_icon_tooltip_states
                            .borrow_mut(),
                        app,
                    )),
                ),
                ChildView::new(&view.working_directory_view).finish(),
            ])
            .finish()
    }
}

#[derive(Default)]
struct UndoCloseWidget {}

impl SettingsWidget for UndoCloseWidget {
    type View = FeaturesPageView;

    fn search_terms(&self) -> &str {
        "reopen restore recover closed tab session"
    }

    fn render(
        &self,
        view: &Self::View,
        _appearance: &Appearance,
        _app: &AppContext,
    ) -> Box<dyn Element> {
        ChildView::new(&view.undo_close_view).finish()
    }
}

#[derive(Default)]
struct ConfirmCloseSharedSessionWidget {
    switch_state: SwitchStateHandle,
}

impl SettingsWidget for ConfirmCloseSharedSessionWidget {
    type View = FeaturesPageView;

    fn search_terms(&self) -> &str {
        "warning popup modal dialog shared session close"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let ui_builder = appearance.ui_builder();
        let session_settings = SessionSettings::as_ref(app);
        render_body_item::<FeaturesPageAction>(
            "Confirm before closing shared session".into(),
            None,
            LocalOnlyIconState::for_setting(
                ShouldConfirmCloseSession::storage_key(),
                ShouldConfirmCloseSession::sync_to_cloud(),
                &mut view
                    .button_mouse_states
                    .local_only_icon_tooltip_states
                    .borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            ui_builder
                .switch(self.switch_state.clone())
                .check(*session_settings.should_confirm_close_session)
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(FeaturesPageAction::ToggleConfirmCloseSession);
                })
                .finish(),
            None,
        )
    }
}

#[derive(Default)]
struct ExtraMetaKeysWidget {
    left_switch_state: SwitchStateHandle,
    right_switch_state: SwitchStateHandle,
}

impl SettingsWidget for ExtraMetaKeysWidget {
    type View = FeaturesPageView;

    fn search_terms(&self) -> &str {
        "extra meta key alt option"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let ui_builder = appearance.ui_builder();
        let key_settings = KeysSettings::as_ref(app);
        let mut tooltip_states = view
            .button_mouse_states
            .local_only_icon_tooltip_states
            .borrow_mut();
        Flex::column()
            .with_child(render_body_item::<FeaturesPageAction>(
                EXTRA_META_KEYS_LEFT_TEXT.into(),
                None,
                LocalOnlyIconState::for_setting(
                    crate::terminal::keys_settings::ExtraMetaKeys::storage_key(),
                    crate::terminal::keys_settings::ExtraMetaKeys::sync_to_cloud(),
                    &mut tooltip_states,
                    app,
                ),
                ToggleState::Enabled,
                appearance,
                ui_builder
                    .switch(self.left_switch_state.clone())
                    .check(key_settings.extra_meta_keys.left_alt)
                    .build()
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(FeaturesPageAction::ToggleLeftMetaKey);
                    })
                    .finish(),
                None,
            ))
            .with_child(render_body_item::<FeaturesPageAction>(
                EXTRA_META_KEYS_RIGHT_TEXT.into(),
                None,
                LocalOnlyIconState::for_setting(
                    crate::terminal::keys_settings::ExtraMetaKeys::storage_key(),
                    crate::terminal::keys_settings::ExtraMetaKeys::sync_to_cloud(),
                    &mut tooltip_states,
                    app,
                ),
                ToggleState::Enabled,
                appearance,
                ui_builder
                    .switch(self.right_switch_state.clone())
                    .check(key_settings.extra_meta_keys.right_alt)
                    .build()
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(FeaturesPageAction::ToggleRightMetaKey);
                    })
                    .finish(),
                None,
            ))
            .finish()
    }
}

#[derive(Default)]
struct GlobalHotkeyWidget {}

impl SettingsWidget for GlobalHotkeyWidget {
    type View = FeaturesPageView;

    fn search_terms(&self) -> &str {
        "global hotkey quake mode keybinding quick terminal"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let mut column = Flex::column();
        let ui_builder = appearance.ui_builder();
        if app.is_wayland() {
            column.add_child(render_body_item::<FeaturesPageAction>(
                "Global hotkey:".to_owned(),
                None,
                // Fine not to show local only icon state for this, as it's not a supported setting.
                LocalOnlyIconState::Hidden,
                ToggleState::Disabled,
                appearance,
                Flex::row()
                    .with_children([
                        ui_builder
                            .span("Not supported on Wayland. ")
                            .build()
                            .finish(),
                        ui_builder
                            .link(
                                "See docs.".to_owned(),
                                Some(
                                    "https://docs.warp.dev/terminal/windows/global-hotkey"
                                        .to_owned(),
                                ),
                                None,
                                view.button_mouse_states.global_hotkey_link.clone(),
                            )
                            .soft_wrap(false)
                            .build()
                            .finish(),
                    ])
                    .finish(),
                None,
            ))
        } else {
            add_setting(
                &mut column,
                &KeysSettings::as_ref(app).activation_hotkey_enabled,
                || {
                    render_dropdown_item(
                        appearance,
                        "Global hotkey:",
                        None,
                        None,
                        LocalOnlyIconState::for_setting(
                            ActivationHotkeyEnabled::storage_key(),
                            ActivationHotkeyEnabled::sync_to_cloud(),
                            &mut view
                                .button_mouse_states
                                .local_only_icon_tooltip_states
                                .borrow_mut(),
                            app,
                        ),
                        None,
                        &view.global_hotkey_dropdown,
                    )
                },
            );
        }

        let global_hotkey_mode =
            KeysSettings::handle(app).read(app, |settings, ctx| settings.global_hotkey_mode(ctx));
        match global_hotkey_mode {
            GlobalHotkeyMode::QuakeMode => {
                column.add_child(render_group(
                    [
                        view.render_keybinding_editor_row(
                            view.button_mouse_states
                                .quake_mode_keybinding_editor
                                .clone(),
                            view.button_mouse_states.quake_mode_cancel.clone(),
                            view.button_mouse_states.quake_mode_save.clone(),
                            view.quake_mode_keybinding_editor_state,
                            &view.quake_mode_keybinding,
                            FeaturesPageAction::QuakeKeybindEditorClicked,
                            FeaturesPageAction::QuakeKeybindEditorCancel,
                            FeaturesPageAction::QuakeKeybindEditorSave,
                            |ctx, keystroke| {
                                ctx.dispatch_typed_action(
                                    FeaturesPageAction::QuakeKeystrokeDefined(keystroke),
                                )
                            },
                            appearance,
                        ),
                        view.render_quake_mode_position_row(
                            KeysSettings::as_ref(app).quake_mode_settings.value(),
                            appearance,
                        ),
                        // This feature is only supported on MacOS.
                        if QUAKE_WINDOW_AUTOHIDE_SUPPORTED {
                            view.render_quake_mode_pin_window_toggle_row(
                                KeysSettings::as_ref(app).quake_mode_settings.value(),
                                appearance,
                            )
                        } else {
                            Empty::new().finish()
                        },
                    ],
                    appearance,
                ));
            }
            GlobalHotkeyMode::ActivationHotkey => column.add_child(render_group(
                [view.render_keybinding_editor_row(
                    view.button_mouse_states
                        .activation_hotkey_keybinding_editor
                        .clone(),
                    view.button_mouse_states.activation_hotkey_cancel.clone(),
                    view.button_mouse_states.activation_hotkey_save.clone(),
                    view.activation_hotkey_keybinding_editor_state,
                    &view.activation_hotkey_keybinding,
                    FeaturesPageAction::ActivationKeybindEditorClicked,
                    FeaturesPageAction::ActivationKeybindEditorCancel,
                    FeaturesPageAction::ActivationKeybindEditorSave,
                    |ctx, keystroke| {
                        ctx.dispatch_typed_action(FeaturesPageAction::ActivationKeystrokeDefined(
                            keystroke,
                        ))
                    },
                    appearance,
                )],
                appearance,
            )),
            GlobalHotkeyMode::Disabled => {}
        }
        column.finish()
    }
}

#[derive(Default)]
struct AutocompleteSymbolsWidget {
    switch_state: SwitchStateHandle,
}

impl SettingsWidget for AutocompleteSymbolsWidget {
    type View = FeaturesPageView;

    fn search_terms(&self) -> &str {
        "autocomplete autoclose symbol bracket quote parentheses braces"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let ui_builder = appearance.ui_builder();
        render_body_item::<FeaturesPageAction>(
            "Autocomplete quotes, parentheses, and brackets".into(),
            None,
            LocalOnlyIconState::for_setting(
                AutocompleteSymbols::storage_key(),
                AutocompleteSymbols::sync_to_cloud(),
                &mut view
                    .button_mouse_states
                    .local_only_icon_tooltip_states
                    .borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            ui_builder
                .switch(self.switch_state.clone())
                .check(*AppEditorSettings::as_ref(app).autocomplete_symbols)
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(FeaturesPageAction::ToggleAutocompleteSymbols);
                })
                .finish(),
            None,
        )
    }
}

#[derive(Default)]
struct ErrorUnderliningWidget {
    switch_state: SwitchStateHandle,
}

impl SettingsWidget for ErrorUnderliningWidget {
    type View = FeaturesPageView;

    fn search_terms(&self) -> &str {
        "error underline editor"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let ui_builder = appearance.ui_builder();
        render_body_item::<FeaturesPageAction>(
            "Error underlining for commands".into(),
            None,
            LocalOnlyIconState::for_setting(
                ErrorUnderliningEnabled::storage_key(),
                ErrorUnderliningEnabled::sync_to_cloud(),
                &mut view
                    .button_mouse_states
                    .local_only_icon_tooltip_states
                    .borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            ui_builder
                .switch(self.switch_state.clone())
                .check(*InputSettings::as_ref(app).error_underlining.value())
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(FeaturesPageAction::ToggleErrorUnderlining);
                })
                .finish(),
            None,
        )
    }
}

#[derive(Default)]
struct SyntaxHighlightingWidget {
    switch_state: SwitchStateHandle,
}

impl SettingsWidget for SyntaxHighlightingWidget {
    type View = FeaturesPageView;

    fn search_terms(&self) -> &str {
        "syntax highlighting editor"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let ui_builder = appearance.ui_builder();
        render_body_item::<FeaturesPageAction>(
            "Syntax highlighting for commands".into(),
            None,
            LocalOnlyIconState::for_setting(
                SyntaxHighlighting::storage_key(),
                SyntaxHighlighting::sync_to_cloud(),
                &mut view
                    .button_mouse_states
                    .local_only_icon_tooltip_states
                    .borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            ui_builder
                .switch(self.switch_state.clone())
                .check(*InputSettings::as_ref(app).syntax_highlighting.value())
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(FeaturesPageAction::ToggleSyntaxHighlighting);
                })
                .finish(),
            None,
        )
    }
}

#[derive(Default)]
struct CompletionsMenuWhileTypingWidget {
    switch_state: SwitchStateHandle,
}

impl SettingsWidget for CompletionsMenuWhileTypingWidget {
    type View = FeaturesPageView;

    fn search_terms(&self) -> &str {
        "completions menu type typing"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let ui_builder = appearance.ui_builder();
        render_body_item::<FeaturesPageAction>(
            "Open completions menu as you type".into(),
            None,
            LocalOnlyIconState::for_setting(
                CompletionsOpenWhileTyping::storage_key(),
                CompletionsOpenWhileTyping::sync_to_cloud(),
                &mut view
                    .button_mouse_states
                    .local_only_icon_tooltip_states
                    .borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            ui_builder
                .switch(self.switch_state.clone())
                .check(
                    *InputSettings::as_ref(app)
                        .completions_open_while_typing
                        .value(),
                )
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(FeaturesPageAction::ToggleCompletionsOpenWhileTyping);
                })
                .finish(),
            None,
        )
    }
}

#[derive(Default)]
struct CommandCorrectionsWidget {
    switch_state: SwitchStateHandle,
}

impl SettingsWidget for CommandCorrectionsWidget {
    type View = FeaturesPageView;

    fn search_terms(&self) -> &str {
        "suggest command corrections"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let ui_builder = appearance.ui_builder();
        render_body_item::<FeaturesPageAction>(
            "Suggest corrected commands".into(),
            None,
            LocalOnlyIconState::for_setting(
                CommandCorrections::storage_key(),
                CommandCorrections::sync_to_cloud(),
                &mut view
                    .button_mouse_states
                    .local_only_icon_tooltip_states
                    .borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            ui_builder
                .switch(self.switch_state.clone())
                .check(*InputSettings::as_ref(app).command_corrections.value())
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(FeaturesPageAction::ToggleCommandCorrections);
                })
                .finish(),
            None,
        )
    }
}

#[derive(Default)]
struct AliasExpansionWidget {
    switch_state: SwitchStateHandle,
}

impl SettingsWidget for AliasExpansionWidget {
    type View = FeaturesPageView;

    fn search_terms(&self) -> &str {
        "expand alias expansion"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let alias_expansion_settings = AliasExpansionSettings::as_ref(app);
        let ui_builder = appearance.ui_builder();
        render_body_item::<FeaturesPageAction>(
            "Expand aliases as you type".into(),
            None,
            LocalOnlyIconState::for_setting(
                AliasExpansionEnabled::storage_key(),
                AliasExpansionEnabled::sync_to_cloud(),
                &mut view
                    .button_mouse_states
                    .local_only_icon_tooltip_states
                    .borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            ui_builder
                .switch(self.switch_state.clone())
                .check(*alias_expansion_settings.alias_expansion_enabled)
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(FeaturesPageAction::ToggleAliasExpansion);
                })
                .finish(),
            None,
        )
    }
}

#[derive(Default)]
struct MiddleClickPasteWidget {
    switch_state: SwitchStateHandle,
}

impl SettingsWidget for MiddleClickPasteWidget {
    type View = FeaturesPageView;

    fn search_terms(&self) -> &str {
        "middle click paste clipboard"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let ui_builder = appearance.ui_builder();
        let selection_settings = SelectionSettings::as_ref(app);
        render_body_item::<FeaturesPageAction>(
            "Middle-click to paste".into(),
            None,
            LocalOnlyIconState::for_setting(
                MiddleClickPasteEnabled::storage_key(),
                MiddleClickPasteEnabled::sync_to_cloud(),
                &mut view
                    .button_mouse_states
                    .local_only_icon_tooltip_states
                    .borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            ui_builder
                .switch(self.switch_state.clone())
                .check(*selection_settings.middle_click_paste_enabled)
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(FeaturesPageAction::ToggleMiddleClickPaste);
                })
                .finish(),
            None,
        )
    }
}

#[derive(Default)]
struct VimModeWidget {
    enabled_switch_state: SwitchStateHandle,
    clipboard_switch_state: SwitchStateHandle,
    status_bar_switch_state: SwitchStateHandle,
}

impl SettingsWidget for VimModeWidget {
    type View = FeaturesPageView;

    fn search_terms(&self) -> &str {
        "vim mode keybindings"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let ui_builder = appearance.ui_builder();
        let mut column = Flex::column();

        let app_editor_settings = AppEditorSettings::as_ref(app);
        let vim_mode_enabled = *app_editor_settings.vim_mode.value();
        column.add_child(render_body_item::<FeaturesPageAction>(
            "Edit code and commands with Vim keybindings".into(),
            None,
            LocalOnlyIconState::for_setting(
                VimModeEnabled::storage_key(),
                VimModeEnabled::sync_to_cloud(),
                &mut view
                    .button_mouse_states
                    .local_only_icon_tooltip_states
                    .borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            ui_builder
                .switch(self.enabled_switch_state.clone())
                .check(vim_mode_enabled)
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(FeaturesPageAction::ToggleVimMode);
                })
                .finish(),
            None,
        ));

        if vim_mode_enabled {
            let unnamed_system_clipboard =
                *app_editor_settings.vim_unnamed_system_clipboard.value();

            let clipboard_switch = ui_builder
                .switch(self.clipboard_switch_state.clone())
                .check(unnamed_system_clipboard)
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(FeaturesPageAction::ToggleVimUnnamedSystemClipboard);
                })
                .finish();
            let clipboard_setting = view.render_setting_subgroup_item(
                appearance,
                LocalOnlyIconState::for_setting(
                    VimUnnamedSystemClipboard::storage_key(),
                    VimUnnamedSystemClipboard::sync_to_cloud(),
                    &mut view
                        .button_mouse_states
                        .local_only_icon_tooltip_states
                        .borrow_mut(),
                    app,
                ),
                clipboard_switch,
                "Set unnamed register as system clipboard".into(),
            );

            let vim_status_bar = *app_editor_settings.vim_status_bar.value();
            let status_bar_switch = ui_builder
                .switch(self.status_bar_switch_state.clone())
                .check(vim_status_bar)
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(FeaturesPageAction::ToggleVimStatusBar);
                })
                .finish();
            let status_bar_setting = view.render_setting_subgroup_item(
                appearance,
                LocalOnlyIconState::for_setting(
                    VimStatusBar::storage_key(),
                    VimStatusBar::sync_to_cloud(),
                    &mut view
                        .button_mouse_states
                        .local_only_icon_tooltip_states
                        .borrow_mut(),
                    app,
                ),
                status_bar_switch,
                "Show Vim status bar".into(),
            );

            column.add_child(render_group(
                [clipboard_setting, status_bar_setting],
                appearance,
            ));
        }

        column.finish()
    }
}

#[derive(Default)]
struct AtContextMenuInTerminalModeWidget {
    switch_state: SwitchStateHandle,
}

impl SettingsWidget for AtContextMenuInTerminalModeWidget {
    type View = FeaturesPageView;

    fn search_terms(&self) -> &str {
        "@ at sign context menu terminal mode AI assistant"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let ui_builder = appearance.ui_builder();
        render_body_item::<FeaturesPageAction>(
            "Enable '@' context menu in terminal mode".into(),
            None,
            LocalOnlyIconState::for_setting(
                AtContextMenuInTerminalMode::storage_key(),
                AtContextMenuInTerminalMode::sync_to_cloud(),
                &mut view
                    .button_mouse_states
                    .local_only_icon_tooltip_states
                    .borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            ui_builder
                .switch(self.switch_state.clone())
                .check(
                    *InputSettings::as_ref(app)
                        .at_context_menu_in_terminal_mode
                        .value(),
                )
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(
                        FeaturesPageAction::ToggleAtContextMenuInTerminalMode,
                    );
                })
                .finish(),
            None,
        )
    }
}

#[derive(Default)]
struct SlashCommandsInTerminalModeWidget {
    switch_state: SwitchStateHandle,
}

impl SettingsWidget for SlashCommandsInTerminalModeWidget {
    type View = FeaturesPageView;

    fn search_terms(&self) -> &str {
        "slash commands terminal mode input menu"
    }

    fn should_render(&self, app: &AppContext) -> bool {
        AISettings::as_ref(app).is_any_ai_enabled(app)
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let ui_builder = appearance.ui_builder();
        render_body_item::<FeaturesPageAction>(
            "Enable slash commands in terminal mode".into(),
            None,
            LocalOnlyIconState::for_setting(
                EnableSlashCommandsInTerminal::storage_key(),
                EnableSlashCommandsInTerminal::sync_to_cloud(),
                &mut view
                    .button_mouse_states
                    .local_only_icon_tooltip_states
                    .borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            ui_builder
                .switch(self.switch_state.clone())
                .check(
                    *InputSettings::as_ref(app)
                        .enable_slash_commands_in_terminal
                        .value(),
                )
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(
                        FeaturesPageAction::ToggleSlashCommandsInTerminalMode,
                    );
                })
                .finish(),
            None,
        )
    }
}

#[derive(Default)]
struct OutlineCodebaseSymbolsForAtContextMenuWidget {
    switch_state: SwitchStateHandle,
}

impl SettingsWidget for OutlineCodebaseSymbolsForAtContextMenuWidget {
    type View = FeaturesPageView;

    fn search_terms(&self) -> &str {
        "outline codebase symbols context menu code indexing"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let ui_builder = appearance.ui_builder();
        render_body_item::<FeaturesPageAction>(
            "Outline codebase symbols for '@' context menu".into(),
            None,
            LocalOnlyIconState::for_setting(
                OutlineCodebaseSymbolsForAtContextMenu::storage_key(),
                OutlineCodebaseSymbolsForAtContextMenu::sync_to_cloud(),
                &mut view
                    .button_mouse_states
                    .local_only_icon_tooltip_states
                    .borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            ui_builder
                .switch(self.switch_state.clone())
                .check(
                    *InputSettings::as_ref(app)
                        .outline_codebase_symbols_for_at_context_menu
                        .value(),
                )
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(
                        FeaturesPageAction::ToggleOutlineCodebaseSymbolsForAtContextMenu,
                    );
                })
                .finish(),
            None,
        )
    }
}

#[derive(Default)]
struct ShowTerminalInputMessageLineWidget {
    switch_state: SwitchStateHandle,
}

impl SettingsWidget for ShowTerminalInputMessageLineWidget {
    type View = FeaturesPageView;

    fn search_terms(&self) -> &str {
        "terminal input message line bar agent"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let ui_builder = appearance.ui_builder();
        render_body_item::<FeaturesPageAction>(
            "Show terminal input message line".into(),
            None,
            LocalOnlyIconState::for_setting(
                ShowTerminalInputMessageBar::storage_key(),
                ShowTerminalInputMessageBar::sync_to_cloud(),
                &mut view
                    .button_mouse_states
                    .local_only_icon_tooltip_states
                    .borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            ui_builder
                .switch(self.switch_state.clone())
                .check(InputSettings::as_ref(app).is_terminal_input_message_bar_enabled())
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(
                        FeaturesPageAction::ToggleShowTerminalInputMessageLine,
                    );
                })
                .finish(),
            None,
        )
    }
}

#[derive(Default)]
struct AutosuggestionKeybindingHintWidget {
    enabled_switch_state: SwitchStateHandle,
}

impl SettingsWidget for AutosuggestionKeybindingHintWidget {
    type View = FeaturesPageView;

    fn search_terms(&self) -> &str {
        "autosuggestion keybinding hint"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let ui_builder = appearance.ui_builder();
        let mut column = Flex::column();

        let app_editor_settings = AppEditorSettings::as_ref(app);
        let autosuggestion_keybinding_hint =
            *app_editor_settings.autosuggestion_keybinding_hint.value();
        column.add_child(render_body_item::<FeaturesPageAction>(
            "Show autosuggestion keybinding hint".into(),
            None,
            LocalOnlyIconState::for_setting(
                AutosuggestionKeybindingHint::storage_key(),
                AutosuggestionKeybindingHint::sync_to_cloud(),
                &mut view
                    .button_mouse_states
                    .local_only_icon_tooltip_states
                    .borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            ui_builder
                .switch(self.enabled_switch_state.clone())
                .check(autosuggestion_keybinding_hint)
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(
                        FeaturesPageAction::ToggleAutosuggestionKeybindingHint,
                    );
                })
                .finish(),
            None,
        ));

        column.finish()
    }
}

#[derive(Default)]
struct AutosuggestionIgnoreButtonWidget {
    enabled_switch_state: SwitchStateHandle,
}

impl SettingsWidget for AutosuggestionIgnoreButtonWidget {
    type View = FeaturesPageView;

    fn search_terms(&self) -> &str {
        "autosuggestion ignore button hide"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let ui_builder = appearance.ui_builder();
        let mut column = Flex::column();

        let app_editor_settings = AppEditorSettings::as_ref(app);
        let show_autosuggestion_ignore_button = *app_editor_settings
            .show_autosuggestion_ignore_button
            .value();
        column.add_child(render_body_item::<FeaturesPageAction>(
            "Show autosuggestion ignore button".into(),
            None,
            LocalOnlyIconState::for_setting(
                ShowAutosuggestionIgnoreButton::storage_key(),
                ShowAutosuggestionIgnoreButton::sync_to_cloud(),
                &mut view
                    .button_mouse_states
                    .local_only_icon_tooltip_states
                    .borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            ui_builder
                .switch(self.enabled_switch_state.clone())
                .check(show_autosuggestion_ignore_button)
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(
                        FeaturesPageAction::ToggleShowAutosuggestionIgnoreButton,
                    );
                })
                .finish(),
            None,
        ));

        column.finish()
    }
}

#[derive(Default)]
struct TabKeyBehaviorWidget {}

impl TabKeyBehaviorWidget {
    fn render_tab_behavior_setting_secondary_row(
        &self,
        view: &FeaturesPageView,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let input_settings = InputSettings::as_ref(app);
        let other_keybinding_info = match *view.tab_behavior {
            TabBehavior::Completions if view.autosuggestions_keystroke.is_empty() => {
                // If the "Accept autosuggestions" keybinding is unbound, the
                // user can always still accept with right arrow.
                Some("→ accepts autosuggestions.".into())
            }
            TabBehavior::Completions => Some(format!(
                "{} accepts autosuggestions.",
                *view.autosuggestions_keystroke
            )),
            TabBehavior::Autosuggestions
                if *input_settings.completions_open_while_typing.value() =>
            {
                if view.completions_keystroke.is_empty() {
                    Some("Completions open as you type.".into())
                } else {
                    Some(format!(
                        "Completions open as you type (or {}).",
                        *view.completions_keystroke
                    ))
                }
            }
            TabBehavior::Autosuggestions if view.completions_keystroke.is_empty() => {
                Some("Opening the completion menu is unbound.".into())
            }
            TabBehavior::Autosuggestions => Some(format!(
                "{} opens completion menu.",
                *view.completions_keystroke
            )),
            TabBehavior::UserDefined => None,
        };
        let other_keybinding_name = match *view.tab_behavior {
            TabBehavior::Completions => Some("Accept Autosuggestion"),
            TabBehavior::Autosuggestions => Some("Open Completions Menu"),
            TabBehavior::UserDefined => None,
        };

        if let (Some(other_keybinding_info), Some(other_keybinding_name)) =
            (other_keybinding_info, other_keybinding_name)
        {
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(Shrinkable::new(1.0, Empty::new().finish()).finish())
                .with_child(
                    appearance
                        .ui_builder()
                        .span(other_keybinding_info)
                        .with_style(UiComponentStyles {
                            font_color: Some(appearance.theme().nonactive_ui_text_color().into()),
                            ..Default::default()
                        })
                        .build()
                        .finish(),
                )
                .with_child(
                    Container::new(
                        view.render_change_keybinding_button(other_keybinding_name, appearance),
                    )
                    .with_margin_left(4.)
                    .finish(),
                )
                .finish()
        } else {
            Empty::new().finish()
        }
    }
}

impl SettingsWidget for TabKeyBehaviorWidget {
    type View = FeaturesPageView;

    fn search_terms(&self) -> &str {
        "completions autosuggestions tab keybinding arrow accept"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let mut tab_key_span = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(
                appearance
                    .ui_builder()
                    .span("Tab key behavior")
                    .with_style(UiComponentStyles {
                        font_size: Some(CONTENT_FONT_SIZE + 1.),
                        ..Default::default()
                    })
                    .build()
                    .finish(),
            );
        if *CloudPreferencesSettings::as_ref(app).settings_sync_enabled {
            tab_key_span.add_child(render_local_only_icon(
                appearance,
                view.button_mouse_states
                    .tab_behavior_local_only_icon
                    .clone(),
                None,
            ));
        }

        let main_row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                Shrinkable::new(1.0, Align::new(tab_key_span.finish()).left().finish()).finish(),
            )
            .with_child(ChildView::new(&view.tab_behavior_dropdown).finish())
            .finish();

        Container::new(
            Flex::column()
                .with_child(main_row)
                .with_child(self.render_tab_behavior_setting_secondary_row(view, appearance, app))
                .finish(),
        )
        .with_margin_bottom(10.)
        .finish()
    }
}

#[derive(Default)]
struct CtrlTabBehaviorWidget {}

impl SettingsWidget for CtrlTabBehaviorWidget {
    type View = FeaturesPageView;

    fn search_terms(&self) -> &str {
        "ctrl tab behavior pane switch session recent"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let mut column = Flex::column();
        add_setting(
            &mut column,
            &KeysSettings::as_ref(app).ctrl_tab_behavior,
            || {
                render_dropdown_item(
                    appearance,
                    "Ctrl+Tab behavior:",
                    None,
                    None,
                    LocalOnlyIconState::for_setting(
                        CtrlTabBehaviorSetting::storage_key(),
                        CtrlTabBehaviorSetting::sync_to_cloud(),
                        &mut view
                            .button_mouse_states
                            .local_only_icon_tooltip_states
                            .borrow_mut(),
                        app,
                    ),
                    None,
                    &view.ctrl_tab_behavior_dropdown,
                )
            },
        );
        column.finish()
    }
}

#[derive(Default)]
struct MouseReportingWidget {
    additional_info_link: MouseStateHandle,
    switch_state: SwitchStateHandle,
}

impl SettingsWidget for MouseReportingWidget {
    type View = FeaturesPageView;

    fn search_terms(&self) -> &str {
        "mouse reporting"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let reporting_settings = AltScreenReporting::as_ref(app);
        let ui_builder = appearance.ui_builder();
        render_body_item::<FeaturesPageAction>(
            "Enable Mouse Reporting".into(),
            Some(AdditionalInfo {
                mouse_state: self.additional_info_link.clone(),
                on_click_action: Some(FeaturesPageAction::OpenUrl(
                    "https://docs.warp.dev/terminal/more-features/full-screen-apps#mouse-and-scroll-reporting"
                        .into(),
                )),
                secondary_text: None,
                tooltip_override_text: None,
            }),
            LocalOnlyIconState::for_setting(
                MouseReportingEnabled::storage_key(),
                MouseReportingEnabled::sync_to_cloud(),
                &mut view
                    .button_mouse_states
                    .local_only_icon_tooltip_states
                    .borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            ui_builder
                .switch(self.switch_state.clone())
                .check(*reporting_settings.mouse_reporting_enabled.value())
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(FeaturesPageAction::ToggleMouseReporting)
                })
                .finish(),
            None,
        )
    }
}

#[derive(Default)]
struct ScrollReportingWidget {
    switch_state: SwitchStateHandle,
}

impl SettingsWidget for ScrollReportingWidget {
    type View = FeaturesPageView;

    fn search_terms(&self) -> &str {
        "scroll reporting"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let reporting_settings = AltScreenReporting::as_ref(app);
        let ui_builder = appearance.ui_builder();
        render_body_item::<FeaturesPageAction>(
            "Enable Scroll Reporting".into(),
            None,
            LocalOnlyIconState::for_setting(
                ScrollReportingEnabled::storage_key(),
                ScrollReportingEnabled::sync_to_cloud(),
                &mut view
                    .button_mouse_states
                    .local_only_icon_tooltip_states
                    .borrow_mut(),
                app,
            ),
            if *reporting_settings.mouse_reporting_enabled.value() {
                ToggleState::Enabled
            } else {
                ToggleState::Disabled
            },
            appearance,
            {
                let switch = ui_builder
                    .switch(self.switch_state.clone())
                    .check(*reporting_settings.scroll_reporting_enabled.value());
                if *reporting_settings.mouse_reporting_enabled.value() {
                    switch
                        .build()
                        .on_click(move |ctx, _, _| {
                            ctx.dispatch_typed_action(FeaturesPageAction::ToggleScrollReporting)
                        })
                        .finish()
                } else {
                    switch.disable().build().finish()
                }
            },
            None,
        )
    }
}

#[derive(Default)]
struct FocusReportingWidget {
    switch_state: SwitchStateHandle,
}

impl SettingsWidget for FocusReportingWidget {
    type View = FeaturesPageView;

    fn search_terms(&self) -> &str {
        "focus reporting"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let reporting_settings = AltScreenReporting::as_ref(app);
        let ui_builder = appearance.ui_builder();
        render_body_item::<FeaturesPageAction>(
            "Enable Focus Reporting".into(),
            None,
            LocalOnlyIconState::for_setting(
                FocusReportingEnabled::storage_key(),
                FocusReportingEnabled::sync_to_cloud(),
                &mut view
                    .button_mouse_states
                    .local_only_icon_tooltip_states
                    .borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            ui_builder
                .switch(self.switch_state.clone())
                .check(*reporting_settings.focus_reporting_enabled.value())
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(FeaturesPageAction::ToggleFocusReporting)
                })
                .finish(),
            None,
        )
    }
}

#[derive(Default)]
struct AudibleBellWidget {
    switch_state: SwitchStateHandle,
}

impl SettingsWidget for AudibleBellWidget {
    type View = FeaturesPageView;

    fn search_terms(&self) -> &str {
        "audible bell"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let ui_builder = appearance.ui_builder();
        let terminal_settings = TerminalSettings::as_ref(app);
        render_body_item::<FeaturesPageAction>(
            "Use Audible Bell".into(),
            None,
            LocalOnlyIconState::for_setting(
                UseAudibleBell::storage_key(),
                UseAudibleBell::sync_to_cloud(),
                &mut view
                    .button_mouse_states
                    .local_only_icon_tooltip_states
                    .borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            ui_builder
                .switch(self.switch_state.clone())
                .check(*terminal_settings.use_audible_bell)
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(FeaturesPageAction::ToggleUseAudibleBell)
                })
                .finish(),
            None,
        )
    }
}

#[derive(Default)]
struct SmartSelectWidget {
    additional_info_link: MouseStateHandle,
    switch_state: SwitchStateHandle,
    word_char_allowlist_reset_state: MouseStateHandle,
}

impl SmartSelectWidget {
    fn render_word_char_config(
        &self,
        view: &FeaturesPageView,
        appearance: &Appearance,
        non_default: bool,
    ) -> Box<dyn Element> {
        let ui_builder = appearance.ui_builder();
        Flex::column()
            .with_child(
                ui_builder
                    .label("Characters considered part of a word".to_string())
                    .with_style(UiComponentStyles {
                        margin: Some(Coords {
                            top: 10.0,
                            bottom: 5.0,
                            ..Default::default()
                        }),
                        ..Default::default()
                    })
                    .build()
                    .finish(),
            )
            .with_child(
                Dismiss::new(
                    ui_builder
                        .text_input(view.word_boundary_editor.clone())
                        .with_style(UiComponentStyles {
                            width: Some(240.0),
                            ..Default::default()
                        })
                        .build()
                        .finish(),
                )
                .on_dismiss(|ctx, _app| {
                    ctx.dispatch_typed_action(FeaturesPageAction::SetWordCharAllowlist)
                })
                .finish(),
            )
            .with_child(
                build_reset_button(
                    appearance,
                    self.word_char_allowlist_reset_state.clone(),
                    non_default,
                )
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(FeaturesPageAction::ResetWordCharAllowlist);
                })
                .finish(),
            )
            .finish()
    }
}

impl SettingsWidget for SmartSelectWidget {
    type View = FeaturesPageView;

    fn search_terms(&self) -> &str {
        "word smart select semantic separator"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let ui_builder = appearance.ui_builder();
        let selection = SemanticSelection::as_ref(app);
        let mut column = Flex::column();
        column.add_child(render_body_item::<FeaturesPageAction>(
            "Double-click smart selection".into(),
            Some(AdditionalInfo {
                mouse_state: self.additional_info_link.clone(),
                on_click_action: Some(FeaturesPageAction::OpenUrl(
                    "https://docs.warp.dev/terminal/more-features/text-selection".into(),
                )),
                secondary_text: None,
                tooltip_override_text: None,
            }),
            LocalOnlyIconState::for_setting(
                SmartSelectEnabled::storage_key(),
                SmartSelectEnabled::sync_to_cloud(),
                &mut view
                    .button_mouse_states
                    .local_only_icon_tooltip_states
                    .borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            ui_builder
                .switch(self.switch_state.clone())
                .check(selection.smart_select_enabled())
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(FeaturesPageAction::ToggleSmartSelection)
                })
                .finish(),
            None,
        ));

        if !selection.smart_select_enabled() {
            column.add_child(render_group(
                [self.render_word_char_config(
                    view,
                    appearance,
                    selection.word_char_allowlist_changed_from_default(),
                )],
                appearance,
            ));
        }

        column.finish()
    }
}

#[derive(Default)]
struct CopyOnSelectWidget {
    switch_state: SwitchStateHandle,
}

#[derive(Default)]
struct ShowTerminalZeroStateBlockWidget {
    switch_state: SwitchStateHandle,
}

impl SettingsWidget for ShowTerminalZeroStateBlockWidget {
    type View = FeaturesPageView;

    fn search_terms(&self) -> &str {
        "zero state new conversation terminal block welcome output first"
    }

    fn should_render(&self, app: &AppContext) -> bool {
        AISettings::as_ref(app).is_any_ai_enabled(app)
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let ui_builder = appearance.ui_builder();
        let terminal_settings = TerminalSettings::as_ref(app);
        render_body_item::<FeaturesPageAction>(
            "Show help block in new sessions".into(),
            None,
            LocalOnlyIconState::for_setting(
                ShowTerminalZeroStateBlock::storage_key(),
                ShowTerminalZeroStateBlock::sync_to_cloud(),
                &mut view
                    .button_mouse_states
                    .local_only_icon_tooltip_states
                    .borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            ui_builder
                .switch(self.switch_state.clone())
                .check(*terminal_settings.show_terminal_zero_state_block)
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(FeaturesPageAction::ToggleShowTerminalZeroStateBlock)
                })
                .finish(),
            None,
        )
    }
}

impl SettingsWidget for CopyOnSelectWidget {
    type View = FeaturesPageView;

    fn search_terms(&self) -> &str {
        "copy on select"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let ui_builder = appearance.ui_builder();
        let copy_on_select_enabled = SelectionSettings::as_ref(app).copy_on_select_enabled();
        render_body_item::<FeaturesPageAction>(
            "Copy on select".into(),
            None,
            LocalOnlyIconState::for_setting(
                CopyOnSelect::storage_key(),
                CopyOnSelect::sync_to_cloud(),
                &mut view
                    .button_mouse_states
                    .local_only_icon_tooltip_states
                    .borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            ui_builder
                .switch(self.switch_state.clone())
                .check(copy_on_select_enabled)
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(FeaturesPageAction::ToggleCopyOnSelect);
                })
                .finish(),
            None,
        )
    }
}

#[derive(Default)]
struct NewTabPlacementWidget {}

impl SettingsWidget for NewTabPlacementWidget {
    type View = FeaturesPageView;

    fn search_terms(&self) -> &str {
        "new tab placement"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        render_dropdown_item(
            appearance,
            "New tab placement",
            None,
            None,
            LocalOnlyIconState::for_setting(
                NewTabPlacement::storage_key(),
                NewTabPlacement::sync_to_cloud(),
                &mut view
                    .button_mouse_states
                    .local_only_icon_tooltip_states
                    .borrow_mut(),
                app,
            ),
            None,
            &view.new_tab_placement_dropdown,
        )
    }
}

#[derive(Default)]
struct DefaultSessionModeWidget {}

impl SettingsWidget for DefaultSessionModeWidget {
    type View = FeaturesPageView;

    fn search_terms(&self) -> &str {
        "default session mode agent terminal new pane tab open config"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let label = render_dropdown_item_label(
            "Default mode for new sessions".to_string(),
            None,
            LocalOnlyIconState::for_setting(
                DefaultSessionMode::storage_key(),
                DefaultSessionMode::sync_to_cloud(),
                &mut view
                    .button_mouse_states
                    .local_only_icon_tooltip_states
                    .borrow_mut(),
                app,
            ),
            None,
            appearance,
        );

        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                Shrinkable::new(
                    1.0,
                    Container::new(Align::new(label).left().finish())
                        .with_margin_bottom(4.)
                        .with_padding_right(16.)
                        .finish(),
                )
                .finish(),
            )
            .with_child(ChildView::new(&view.default_session_mode_dropdown).finish())
            .finish()
    }
}

#[derive(Default)]
struct WorkflowsInCommandSearch {
    additional_info_link: MouseStateHandle,
    switch_state: SwitchStateHandle,
}

impl SettingsWidget for WorkflowsInCommandSearch {
    type View = FeaturesPageView;

    fn search_terms(&self) -> &str {
        "global workflows command search"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let ui_builder = appearance.ui_builder();
        let workflow_settings = CommandSearchSettings::as_ref(app);
        render_body_item::<FeaturesPageAction>(
            "Show Global Workflows in Command Search (ctrl-r)".into(),
            Some(AdditionalInfo {
                mouse_state: self.additional_info_link.clone(),
                on_click_action: Some(FeaturesPageAction::OpenUrl(
                    "https://docs.warp.dev/terminal/entry/yaml-workflows".into(),
                )),
                secondary_text: None,
                tooltip_override_text: None,
            }),
            LocalOnlyIconState::for_setting(
                ShowGlobalWorkflowsInUniversalSearch::storage_key(),
                ShowGlobalWorkflowsInUniversalSearch::sync_to_cloud(),
                &mut view
                    .button_mouse_states
                    .local_only_icon_tooltip_states
                    .borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            ui_builder
                .switch(self.switch_state.clone())
                .check(*workflow_settings.show_global_workflows_in_universal_search)
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(
                        FeaturesPageAction::ToggleGlobalWorkflowsInUniversalSearch,
                    )
                })
                .finish(),
            None,
        )
    }
}

#[derive(Default)]
struct LinuxSelectionClipboardWidget {
    additional_info_link: MouseStateHandle,
    switch_state: SwitchStateHandle,
}

impl SettingsWidget for LinuxSelectionClipboardWidget {
    type View = FeaturesPageView;

    fn search_terms(&self) -> &str {
        "linux selection clipboard middle click"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        render_body_item::<FeaturesPageAction>(
            "Honor linux selection clipboard".into(),
            Some(AdditionalInfo {
                mouse_state: self.additional_info_link.clone(),
                on_click_action: None,
                secondary_text: None,
                tooltip_override_text: Some(
                    "Whether the Linux primary clipboard should be supported.".into(),
                ),
            }),
            LocalOnlyIconState::for_setting(
                LinuxSelectionClipboard::storage_key(),
                LinuxSelectionClipboard::sync_to_cloud(),
                &mut view
                    .button_mouse_states
                    .local_only_icon_tooltip_states
                    .borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            appearance
                .ui_builder()
                .switch(self.switch_state.clone())
                .check(SelectionSettings::as_ref(app).linux_selection_clipboard_enabled())
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(FeaturesPageAction::ToggleLinuxClipboardSelection);
                })
                .finish(),
            None,
        )
    }
}

#[derive(Default)]
struct GPUWidget {
    switch_state: SwitchStateHandle,
}

impl SettingsWidget for GPUWidget {
    type View = FeaturesPageView;

    fn search_terms(&self) -> &str {
        "render integrated discrete gpu graphics"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let gpu_settings = GPUSettings::as_ref(app);
        let mut col = Flex::column().with_child(render_body_item::<FeaturesPageAction>(
            "Prefer rendering new windows with integrated GPU (low power)".into(),
            None,
            LocalOnlyIconState::for_setting(
                PreferLowPowerGPU::storage_key(),
                PreferLowPowerGPU::sync_to_cloud(),
                &mut view
                    .button_mouse_states
                    .local_only_icon_tooltip_states
                    .borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            appearance
                .ui_builder()
                .switch(self.switch_state.clone())
                .check(*gpu_settings.prefer_low_power_gpu)
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(FeaturesPageAction::TogglePreferLowPowerGPU)
                })
                .finish(),
            None,
        ));
        if view.gpu_power_preference_changed {
            let theme = appearance.theme();
            col.add_child(
                Container::new(
                    appearance
                        .ui_builder()
                        .wrappable_text("Changes will apply to new windows.", true)
                        .with_style(UiComponentStyles {
                            font_color: Some(theme.sub_text_color(theme.background()).into_solid()),
                            ..Default::default()
                        })
                        .build()
                        .finish(),
                )
                .with_margin_bottom(10.)
                .finish(),
            );
        }
        col.finish()
    }
}

#[cfg(target_os = "linux")]
#[derive(Default)]
struct WindowSystemWidget {
    additional_info_link: MouseStateHandle,
    switch_state: SwitchStateHandle,
}

#[cfg(target_os = "linux")]
impl SettingsWidget for WindowSystemWidget {
    type View = FeaturesPageView;

    fn search_terms(&self) -> &str {
        "wayland x11 window system compositor"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let mut children = Flex::column();
        let force_x11 = *LinuxAppConfiguration::as_ref(app).force_x11.value();
        children.add_child(render_body_item::<FeaturesPageAction>(
            "Use Wayland for window management".into(),
            Some(AdditionalInfo {
                mouse_state: self.additional_info_link.clone(),
                on_click_action: None,
                secondary_text: None,
                tooltip_override_text: Some("Enables the use of Wayland".to_string()),
            }),
            LocalOnlyIconState::for_setting(
                ForceX11::storage_key(),
                ForceX11::sync_to_cloud(),
                &mut view
                    .button_mouse_states
                    .local_only_icon_tooltip_states
                    .borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            appearance
                .ui_builder()
                .switch(self.switch_state.clone())
                .check(!force_x11)
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(FeaturesPageAction::ToggleForceX11)
                })
                .finish(),
            None,
        ));

        let mut secondary_text =
            "Enabling this setting disables global hotkey support. When disabled, text \
                    may be blurry if your Wayland compositor is using fraction scaling (ex: 125%)."
                .to_string();
        if view.force_x11_changed {
            secondary_text.push_str("\n\nRestart Warp for changes to take effect.");
        }
        let warp_theme = appearance.theme();
        children.add_child(
            appearance
                .ui_builder()
                .wrappable_text(secondary_text, true)
                .with_style(UiComponentStyles {
                    font_color: Some(
                        warp_theme
                            .sub_text_color(warp_theme.background())
                            .into_solid(),
                    ),
                    ..Default::default()
                })
                .build()
                .finish(),
        );
        children.finish()
    }
}

#[derive(Default)]
struct GraphicsBackendWidget {}

impl SettingsWidget for GraphicsBackendWidget {
    type View = FeaturesPageView;

    fn search_terms(&self) -> &str {
        "gpu graphics backend vulkan dx12 directx12 opengl driver"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let dropdown = render_dropdown_item(
            appearance,
            "Preferred graphics backend",
            None,
            None,
            LocalOnlyIconState::for_setting(
                PreferredGraphicsBackend::storage_key(),
                PreferredGraphicsBackend::sync_to_cloud(),
                &mut view
                    .button_mouse_states
                    .local_only_icon_tooltip_states
                    .borrow_mut(),
                app,
            ),
            None,
            &view.graphics_backend_dropdown,
        );
        let mut col = Flex::column().with_child(dropdown);
        if let Some(window) = app.windows().platform_window(view.window_id) {
            let backend = window.graphics_backend();
            col.add_child(
                appearance
                    .ui_builder()
                    .wrappable_text(format!("Current backend: {}", backend.to_label()), true)
                    .with_style(UiComponentStyles {
                        font_color: Some(theme.sub_text_color(theme.background()).into_solid()),
                        ..Default::default()
                    })
                    .build()
                    .finish(),
            );
        }
        if view.graphics_backend_preference_changed {
            col.add_child(
                Container::new(
                    appearance
                        .ui_builder()
                        .wrappable_text("Changes will apply to new windows.", true)
                        .with_style(UiComponentStyles {
                            font_color: Some(theme.sub_text_color(theme.background()).into_solid()),
                            ..Default::default()
                        })
                        .build()
                        .finish(),
                )
                .with_margin_bottom(10.)
                .finish(),
            );
        }
        col.finish()
    }
}
