use self::telemetry::SettingsTelemetryEvent;
use crate::pane_group::focus_state::PaneFocusHandle;
use crate::server::telemetry::MCPServerCollectionPaneEntrypoint;
use crate::settings_view::mcp_servers_page::MCPServersSettingsPage;
use crate::TelemetryEvent;
use crate::{
    ai::execution_profiles::profiles::ClientProfileId,
    appearance::Appearance,
    editor::{
        EditorView, Event as EditorEvent, PropagateAndNoOpNavigationKeys, SingleLineEditorOptions,
        TextColors, TextOptions,
    },
    menu::{self, Menu, MenuItem, MenuItemFields},
    pane_group::{
        pane::view, BackingView, Direction, PaneConfiguration, PaneEvent, SplitPaneState,
    },
    server::server_api::ServerApiProvider,
    settings::{AISettings, BlockVisibilitySettings, SettingsFileError},
    settings_view::mcp_servers_page::MCPServersSettingsPageEvent,
    terminal::{model::blockgrid::BlockGrid, SizeInfo},
    ui_components::icons,
    util::bindings::{keybinding_name_to_display_string, BindingGroup, CustomAction},
    view_components::ToastFlavor,
    workspace::WorkspaceAction,
    GlobalResourceHandlesProvider,
};
use about_page::AboutPageView;
use ai_page::{AISettingsPageAction, AISettingsPageEvent, AISettingsPageView, AISubpage};
use appearance_page::{AppearancePageAction, AppearanceSettingsPageView};
use billing_and_usage_page::{BillingAndUsagePageEvent, BillingAndUsagePageView};
use billing_and_usage_page_v2::BillingAndUsagePageV2View;
use code_page::CodeSubpage;
use code_page::{CodeSettingsPageAction, CodeSettingsPageEvent};
use environments_page::EnvironmentsPageView;
use features_page::{FeaturesPageView, FeaturesSettingsPageEvent};
use itertools::Itertools as _;
use keybindings::KeybindingsView;
use main_page::{MainPageAction, MainSettingsPageEvent, MainSettingsPageView};
use mcp_servers_page::MCPServersSettingsPageView;
use nav::{SettingsNavItem, SettingsUmbrella};
use pathfinder_geometry::vector::Vector2F;
use privacy_page::{PrivacyPageView, PrivacyPageViewEvent};
use referrals_page::{ReferralsPageEvent, ReferralsPageView};
use settings_file_footer::{render_footer, SettingsFooterKind, SettingsFooterMouseStates};
use settings_page::{
    MatchData, SettingsPage, SettingsPageEvent, SettingsPageMeta, SettingsPageViewHandle,
    HEADER_PADDING,
};
use show_blocks_view::{ShowBlocksEvent, ShowBlocksView};
use std::collections::HashMap;
use std::path::PathBuf;
use std::str::FromStr;
use teams_page::{TeamsPageView, TeamsPageViewEvent};
use warp_core::send_telemetry_from_ctx;
use warp_core::{
    channel::ChannelState, context_flag::ContextFlag, features::FeatureFlag,
    settings::ToggleableSetting as _, ui::theme::color::internal_colors,
};
use warp_editor::editor::NavigationKey;
use warpify_page::{WarpifyPageAction, WarpifyPageView};
use warpui::Element;
use warpui::{
    elements::{
        Align, Border, ChildAnchor, ChildView, Clipped, ClippedScrollStateHandle,
        ClippedScrollable, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment,
        DispatchEventResult, Empty, EventHandler, Expanded, Fill, Flex, MainAxisSize,
        OffsetPositioning, ParentAnchor, ParentElement, ParentOffsetBounds, Radius, SavePosition,
        ScrollbarWidth, Shrinkable, Stack, Text,
    },
    fonts::{Properties, Weight},
    id,
    keymap::{ContextPredicate, EnabledPredicate, FixedBinding},
    Action, AppContext, Entity, ModelHandle, SingletonEntity, TypedActionView, UpdateView as _,
    View, ViewContext, ViewHandle,
};

mod about_page;
mod admin_actions;
mod agent_assisted_environment_modal;
mod ai_page;
mod appearance_page;
mod billing_and_usage;
mod billing_and_usage_page;
mod billing_and_usage_page_v2;
mod code_page;
mod custom_inference_modal;
mod delete_environment_confirmation_dialog;
mod directory_color_add_picker;
pub(crate) mod environments_page;
mod execution_profile_view;
mod features;
mod features_page;
pub(crate) mod handoff_environment_creation_modal;
pub mod keybindings;
mod main_page;
pub mod mcp_servers;
pub mod mcp_servers_page;
mod nav;
pub mod pane_manager;
mod platform;
mod platform_page;
mod privacy;
mod privacy_page;
mod referrals_page;
mod remove_custom_endpoint_confirmation_dialog;
mod settings_file_footer;
pub(crate) mod settings_page;
mod show_blocks_view;
mod tab_menu;
mod teams_page;
mod telemetry;
mod transfer_ownership_confirmation_modal;
pub mod update_environment_form;
mod warp_drive_page;
mod warpify_page;

#[cfg(not(target_family = "wasm"))]
pub(crate) use ai_page::cli_agent_settings_widget_id;
pub use billing_and_usage_page::create_discount_badge;
pub use code_page::CodeSettingsPageView;
pub use features_page::FeaturesPageAction;
pub use main_page::handle_experiment_change;
pub use privacy_page::PrivacyPageAction;
pub use settings_page::{
    render_body_item_label, render_info_icon, render_input_list, render_separator, AdditionalInfo,
    InputListItem, LocalOnlyIconState, ToggleState,
};
pub use teams_page::{OpenTeamsSettingsModalArgs, TeamsInviteOption};

/// Original sidebar width used when the settings-file footer is not
/// enabled. Preserved for Preview/Stable until `FeatureFlag::SettingsFile`
/// is promoted.
const SIDEBAR_WIDTH_DEFAULT: f32 = 200.;

/// Wider sidebar used when the settings-file footer is enabled. Sized to
/// match Figma's settings nav rail (223px alert + 12px horizontal padding
/// on each side + 1px right border), giving the error-alert footer enough
/// room to render its "Open file" and "Fix with Oz" buttons side-by-side
/// with the designed 24px indent and 8px internal padding.
const SIDEBAR_WIDTH_WITH_FOOTER: f32 = 248.;

/// Returns the sidebar width, widened only when the settings-file footer
/// is enabled. This keeps the wider layout gated with the footer itself so
/// Preview/Stable users don't see an unexplained 48px width bump before
/// the feature ships.
fn sidebar_width() -> f32 {
    if FeatureFlag::SettingsFile.is_enabled() {
        SIDEBAR_WIDTH_WITH_FOOTER
    } else {
        SIDEBAR_WIDTH_DEFAULT
    }
}

/// Width of the borders for the header and the sidebar.
const SECTION_BORDER_WIDTH: f32 = 1.;

const POSITION_ID: &str = "settings_pane";

pub(super) fn editor_text_colors(appearance: &Appearance) -> TextColors {
    let theme = appearance.theme();
    TextColors {
        default_color: theme.active_ui_text_color(),
        disabled_color: theme.disabled_ui_text_color(),
        hint_color: theme.disabled_ui_text_color(),
    }
}

/// Renders a horizontal row of pill-shaped chips for model labels.
/// Used by custom inference endpoint cards and the remove confirmation dialog.
pub(super) fn render_model_chips(
    labels: impl IntoIterator<Item = String>,
    appearance: &Appearance,
    text_color: warp_core::ui::theme::Fill,
) -> Box<dyn Element> {
    use warpui::ui_components::{
        chip::Chip,
        components::{UiComponent, UiComponentStyles},
    };

    let theme = appearance.theme();
    let chip_border = internal_colors::neutral_4(theme).into();
    let chip_style = UiComponentStyles {
        background: None,
        border_color: Some(chip_border),
        border_width: Some(1.),
        border_radius: Some(CornerRadius::with_all(Radius::Pixels(5.))),
        font_family_id: Some(appearance.ui_font_family()),
        font_size: Some(appearance.ui_font_size()),
        font_color: Some(text_color.into_solid()),
        ..Default::default()
    };

    let mut chips = Flex::row().with_spacing(8.);
    for label in labels {
        chips.add_child(Chip::new(label, chip_style).build().finish());
    }
    chips.finish()
}

#[derive(PartialEq, Eq)]
pub enum SettingsViewEvent {
    Pane(PaneEvent),
    StartResize,
    CheckForUpdate,
    LaunchNetworkLogging,
    OpenWarpDrive,
    SignupAnonymousUser,
    ShowToast {
        message: String,
        flavor: ToastFlavor,
    },
    OpenAIFactCollection,
    OpenMCPServerCollection,
    OpenExecutionProfileEditor(ClientProfileId),
    OpenLspLogs {
        log_path: PathBuf,
    },
    OpenProjectRulesPane {
        rule_paths: Vec<PathBuf>,
    },
}

/// Different navigation sections within the settings view
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum SettingsSection {
    About,
    #[default]
    Account,
    MCPServers,
    BillingAndUsage,
    Appearance,
    Features,
    Keybindings,
    Privacy,
    Referrals,
    SharedBlocks,
    Teams,
    WarpDrive,
    Warpify,
    /// Internal backing-page identifier for AISettingsPageView. Multiple subpages
    /// (WarpAgent, AgentProfiles, Knowledge, ThirdPartyCLIAgents) share this single
    /// backing page, so this variant is needed as the key in `settings_pages`.
    /// External callers should navigate to a specific subpage (e.g. `WarpAgent`) instead.
    AI,
    // ── Agents umbrella subpages ──
    WarpAgent,
    AgentProfiles,
    AgentMCPServers,
    Knowledge,
    ThirdPartyCLIAgents,
    /// Internal backing-page identifier for CodeSettingsPageView. Multiple subpages
    /// (CodeIndexing, EditorAndCodeReview) share this single backing page,
    /// so this variant is needed as the key in `settings_pages`.
    /// External callers should navigate to a specific subpage instead.
    Code,
    // ── Code umbrella subpages ──
    CodeIndexing,
    EditorAndCodeReview,
    // ── Cloud platform umbrella subpages ──
    CloudEnvironments,
    OzCloudAPIKeys,
}

use crate::util::bindings::custom_tag_to_keystroke;
use std::fmt::{self, Display};

impl Display for SettingsSection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SettingsSection::BillingAndUsage => write!(f, "Billing and usage"),
            SettingsSection::Keybindings => write!(f, "Keyboard shortcuts"),
            SettingsSection::SharedBlocks => write!(f, "Shared blocks"),
            SettingsSection::MCPServers => write!(f, "MCP Servers"),
            SettingsSection::WarpDrive => write!(f, "Warp Drive"),
            SettingsSection::WarpAgent => write!(f, "Warp Agent"),
            SettingsSection::AgentProfiles => write!(f, "Profiles"),
            SettingsSection::AgentMCPServers => write!(f, "MCP servers"),
            SettingsSection::Knowledge => write!(f, "Knowledge"),
            SettingsSection::ThirdPartyCLIAgents => write!(f, "Third party CLI agents"),
            SettingsSection::CodeIndexing => write!(f, "Indexing and projects"),
            SettingsSection::EditorAndCodeReview => write!(f, "Editor and Code Review"),
            SettingsSection::CloudEnvironments => write!(f, "Environments"),
            SettingsSection::OzCloudAPIKeys => write!(f, "Oz Cloud API Keys"),
            _ => write!(f, "{self:?}"),
        }
    }
}

impl SettingsSection {
    /// Returns true if this section is a subpage under any umbrella.
    pub fn is_subpage(&self) -> bool {
        self.is_ai_subpage() || self.is_code_subpage() || self.is_cloud_platform_subpage()
    }

    /// Returns true if this section is a subpage under the "Agents" umbrella.
    pub fn is_ai_subpage(&self) -> bool {
        matches!(
            self,
            Self::WarpAgent
                | Self::AgentProfiles
                | Self::AgentMCPServers
                | Self::Knowledge
                | Self::ThirdPartyCLIAgents
        )
    }

    /// Returns true if this section is a subpage under the "Code" umbrella.
    pub fn is_code_subpage(&self) -> bool {
        matches!(self, Self::CodeIndexing | Self::EditorAndCodeReview)
    }

    /// Returns true if this section is a subpage under the "Cloud platform" umbrella.
    pub fn is_cloud_platform_subpage(&self) -> bool {
        matches!(self, Self::CloudEnvironments | Self::OzCloudAPIKeys)
    }

    /// Maps subpage sections back to their parent page section for page lookup.
    /// Non-subpage sections return themselves.
    pub fn parent_page_section(&self) -> Self {
        match self {
            // AgentMCPServers renders the standalone MCPServers page directly.
            Self::AgentMCPServers => Self::MCPServers,
            // All other AI subpages render within the AI page.
            s if s.is_ai_subpage() => Self::AI,
            // Code subpages render within the Code page.
            s if s.is_code_subpage() => Self::Code,
            // CloudEnvironments and OzCloudAPIKeys ARE their own backing pages
            // (1:1 mapping), so they return themselves.
            other => *other,
        }
    }

    /// The ordered list of AI subpage sections shown under the Agents umbrella.
    pub fn ai_subpages() -> &'static [Self] {
        &[
            Self::WarpAgent,
            Self::AgentProfiles,
            Self::AgentMCPServers,
            Self::Knowledge,
            Self::ThirdPartyCLIAgents,
        ]
    }

    /// The ordered list of Code subpage sections shown under the Code umbrella.
    pub fn code_subpages() -> &'static [Self] {
        &[Self::CodeIndexing, Self::EditorAndCodeReview]
    }

    /// The ordered list of Cloud platform subpage sections.
    pub fn cloud_platform_subpages() -> &'static [Self] {
        &[Self::CloudEnvironments, Self::OzCloudAPIKeys]
    }
}

impl FromStr for SettingsSection {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "About" => Ok(Self::About),
            "Account" => Ok(Self::Account),
            "AI" => Ok(Self::AI),
            "MCP Servers" => Ok(Self::MCPServers),
            "Billing and usage" => Ok(Self::BillingAndUsage),
            "Appearance" => Ok(Self::Appearance),
            "Code" => Ok(Self::Code),
            "Features" => Ok(Self::Features),
            "Keyboard shortcuts" => Ok(Self::Keybindings),
            "Privacy" => Ok(Self::Privacy),
            "Referrals" => Ok(Self::Referrals),
            "Shared blocks" => Ok(Self::SharedBlocks),
            "Teams" => Ok(Self::Teams),
            "Warpify" => Ok(Self::Warpify),
            "WarpDrive" | "Warp Drive" => Ok(Self::WarpDrive),
            // This page was called "Oz" at one point, keep for backward compatibility.
            "Oz" | "Warp Agent" => Ok(Self::WarpAgent),
            "Profiles" | "AgentProfiles" => Ok(Self::AgentProfiles),
            "MCP servers" | "AgentMCPServers" => Ok(Self::AgentMCPServers),
            "Knowledge" => Ok(Self::Knowledge),
            "Third party CLI agents" | "ThirdPartyCLIAgents" => Ok(Self::ThirdPartyCLIAgents),
            "Indexing and projects" | "CodeIndexing" => Ok(Self::CodeIndexing),
            "Editor and Code Review" | "EditorAndCodeReview" => Ok(Self::EditorAndCodeReview),
            "CloudEnvironments" => Ok(Self::CloudEnvironments),
            "Oz Cloud API Keys" | "OzCloudAPIKeys" => Ok(Self::OzCloudAPIKeys),
            _ => Err(()),
        }
    }
}

pub struct DisplayCount(pub usize);

impl Entity for DisplayCount {
    type Event = ();
}

impl SingletonEntity for DisplayCount {}

impl DisplayCount {
    pub fn num_displays(&self) -> usize {
        self.0
    }

    #[cfg(test)]
    pub fn mock() -> Self {
        Self(1)
    }
}

pub mod flags {
    // The following are context flags to determine if the enable or disable binding is shown.
    pub const COPY_ON_SELECT_CONTEXT_FLAG: &str = "Copy_On_Select";

    pub const LINUX_SELECTION_CLIPBOARD_FLAG: &str = "Linux_Selection_Clipboard";
    pub const RESTORE_SESSION_CONTEXT_FLAG: &str = "Restore_Sessions";
    pub const HONOR_PS1_CONTEXT_FLAG: &str = "Honor_PS1";
    pub const GIT_PROMPT_CONTEXT_FLAG: &str = "Git_Prompt";
    pub const AUTOCOMPLETE_SYMBOLS_CONTEXT_FLAG: &str = "Autocomplete_Symbols";
    pub const QUAKE_MODE_ENABLED_CONTEXT_FLAG: &str = "Quake_Mode_Editor";
    pub const QUAKE_WINDOW_OPEN_FLAG: &str = "Quake_Window_Open";
    pub const EXTRA_META_KEYS_RIGHT_CONTEXT_FLAG: &str = "Extra_Meta_Keys_Right";
    pub const EXTRA_META_KEYS_LEFT_CONTEXT_FLAG: &str = "Extra_Meta_Keys_Left";
    pub const SCROLL_REPORTING_CONTEXT_FLAG: &str = "Scroll_Reporting";
    pub const FOCUS_REPORTING_CONTEXT_FLAG: &str = "Focus_Reporting";
    #[deprecated = "Use `SSH_TMUX_WRAPPER_CONTEXT_FLAG` for new ssh warpification logic"]
    pub const LEGACY_SSH_WRAPPER_CONTEXT_FLAG: &str = "SSH_Wrapper";
    pub const SSH_TMUX_WRAPPER_CONTEXT_FLAG: &str = "SSH_Tmux_Wrapper";
    pub const NOTIFICATIONS_CONTEXT_FLAG: &str = "Notifications_Enabled";
    pub const LINK_TOOLTIP_CONTEXT_FLAG: &str = "Link_Tooltip";
    pub const COMPACT_MODE_CONTEXT_FLAG: &str = "Compact_Mode_Enabled";
    pub const CURSOR_BLINK_CONTEXT_FLAG: &str = "Cursor_Blink_Enabled";
    pub const VIM_MODE_CONTEXT_FLAG: &str = "Vim_Mode_Enabled";
    pub const VIM_UNNAMED_SYSTEM_CLIPBOARD: &str = "Vim_Unnamed_System_Clipboard";
    pub const VIM_SHOW_STATUS_BAR: &str = "Vim_Show_Status_Bar";
    pub const JUMP_TO_BOTTOM_OF_BLOCK_BUTTON_CONTEXT_FLAG: &str =
        "Jump_To_Bottom_Of_Block_Button_Enabled";
    pub const RESPECT_SYSTEM_THEME_CONTEXT_FLAG: &str = "Respect_System_Theme";
    pub const COMPLETIONS_OPEN_WHILE_TYPING_CONTEXT_FLAG: &str = "Completions_Open_While_Typing";
    pub const COMMAND_CORRECTIONS_CONTEXT_FLAG: &str = "Command_Corrections";
    pub const ERROR_UNDERLINING_FLAG: &str = "error_underlining";
    pub const SYNTAX_HIGHLIGHTING_FLAG: &str = "syntax_highlighting";
    pub const SAME_LINE_PROMPT: &str = "Same_Line_Prompt_Enabled";
    pub const TELEMETRY_FLAG: &str = "telemetry";
    pub const SETTINGS_SYNC_FLAG: &str = "settings_sync";
    pub const SAFE_MODE_FLAG: &str = "safe_mode";
    pub const CRASH_REPORTING_FLAG: &str = "crash_reporting";
    pub const DIM_INACTIVE_PANES_FLAG: &str = "Dim_Inactive_Panes";
    pub const QUIT_WARNING_MODAL: &str = "Quit_Warning_Modal";
    pub const BLOCK_DIVIDERS_CONTEXT_FLAG: &str = "Block_Dividers_Enabled";

    pub const LOG_OUT_WARNING_MODAL: &str = "Log_Out_Warning_Modal";
    pub const SMART_SELECT_FLAG: &str = "Smart_Select_Enabled";
    pub const ACTIVATION_HOTKEY_FLAG: &str = "Activation_Hotkey_Enabled";
    pub const TAB_INDICATORS_FLAG: &str = "Tab_Indicators_Enabled";
    pub const SHOW_CODE_REVIEW_BUTTON_FLAG: &str = "Show_Code_Review_Button_Enabled";
    pub const USE_VERTICAL_TABS_FLAG: &str = "Use_Vertical_Tabs";
    pub const SESSION_CONFIG_TAB_CONFIG_CHIP_OPEN: &str = "Session_Config_Tab_Config_Chip_Open";
    pub const FOCUS_PANES_ON_HOVER_CONTEXT_FLAG: &str = "Focus_Panes_On_Hover";
    pub const HIDE_WORKSPACE_DECORATIONS_CONTEXT_FLAG: &str = "Hide_Workspace_Decorations";
    pub const ALIAS_EXPANSION_FLAG: &str = "Alias_Expansion_Enabled";
    pub const MIDDLE_CLICK_PASTE_FLAG: &str = "Middle_Click_Paste_Enabled";
    pub const CODE_AS_DEFAULT_EDITOR: &str = "Code_As_Default_Enabled";
    pub const SYNC_ALL_TABS_FLAG: &str = "Sync_All_Tabs_Enabled";
    pub const SYNC_ALL_PANES_IN_CURRENT_TAB: &str = "Sync_All_Panes_In_Current_Tab";
    pub const USE_AUDIBLE_BELL_CONTEXT_FLAG: &str = "Use_Audible_Terminal_Bell";
    pub const SHOW_INPUT_HINT_TEXT_CONTEXT_FLAG: &str = "Show_Input_Hint_text";
    pub const SHOW_AGENT_TIPS_FLAG: &str = "Show_Agent_Tips";
    pub const SHOW_OZ_UPDATES_IN_ZERO_STATE_FLAG: &str = "Show_Oz_Updates_In_Zero_State";
    pub const USE_AGENT_FOOTER_FLAG: &str = "Use_Agent_Footer";
    pub const THINKING_DISPLAY_SHOW_AND_COLLAPSE: &str = "Thinking_Display_ShowAndCollapse";
    pub const THINKING_DISPLAY_ALWAYS_SHOW: &str = "Thinking_Display_AlwaysShow";
    pub const THINKING_DISPLAY_NEVER_SHOW: &str = "Thinking_Display_NeverShow";
    pub const SHOW_TERMINAL_INPUT_MESSAGE_LINE_FLAG: &str = "Show_Terminal_Input_Message_Line";
    pub const SLASH_COMMANDS_IN_TERMINAL_FLAG: &str = "Slash_Commands_In_Terminal";
    pub const AUTOSUGGESTIONS_ENABLED_FLAG: &str = "Autosuggestions_Enabled";
    pub const AUTOSUGGESTION_KEYBINDING_HINT_FLAG: &str = "Hide_Autosuggestion_Keybinding_Hint";
    pub const PREFER_LOW_POWER_GPU_FLAG: &str = "Prefer_Low_Power_GPU";
    pub const INITIALIZATION_BLOCK_FLAG: &str = "Initialization_Block_Visible";
    pub const IN_BAND_COMMAND_BLOCKS_FLAG: &str = "In_Band_Command_Blocks_Visible";
    pub const RECORDING_MODE_FLAG: &str = "Recording_Mode_Enabled";
    pub const IN_BAND_GENERATORS_FLAG: &str = "In_Band_Generators_Enabled";
    pub const WARP_SAME_LINE_PROMPT_FLAG: &str = "Warp_Same_Line_Prompt_Enabled";
    pub const DEBUG_NETWORK_ONLINE_FLAG: &str = "Network_Status_Online";
    pub const AI_INPUT_AUTODETECTION_FLAG: &str = "AI_Input_Autodetection";
    pub const NLD_IN_TERMINAL_FLAG: &str = "NLD_In_Terminal";
    pub const INTELLIGENT_AUTOSUGGESTIONS_FLAG: &str = "Intelligent_Autosuggestions";
    pub const PROMPT_SUGGESTIONS_FLAG: &str = "Prompt_Suggestions";
    pub const CODE_SUGGESTIONS_FLAG: &str = "Code_Suggestions";
    pub const NATURAL_LANGUAGE_AUTOSUGGESTIONS_FLAG: &str = "Natural_Language_Autosuggestions";
    pub const SHARED_BLOCK_TITLE_GENERATION_FLAG: &str = "Shared_Block_Title_Generation";
    pub const DEBUG_SHOW_MEMORY_STATS_FLAG: &str = "Debug_Memory_Statistics";
    pub const ALLOW_NATIVE_WAYLAND: &str = "Allow_Native_Wayland";
    pub const IS_ANY_AI_ENABLED: &str = "IsAnyAIEnabled";
    pub const IS_ACTIVE_AI_ENABLED: &str = "IsActiveAIEnabled";
    pub const IS_VOICE_INPUT_ENABLED: &str = "IsVoiceInputEnabled";
    pub const IS_BLOCK_AI_SUMMARIES_ENABLED: &str = "IsBlockAISummariesEnabled";
    pub const IS_CODEBASE_INDEXING_ENABLED: &str = "IsCodebaseIndexingEnabled";
    pub const IS_AUTOINDEXING_ENABLED: &str = "IsAutoIndexingEnabled";
    pub const LIGATURE_RENDERING_CONTEXT_FLAG: &str = "Ligature_Rendering_Enabled";
    pub const HAS_SETTINGS_TO_IMPORT_FLAG: &str = "HasSettingsToImport";
    /// The user's setting enabled UDI, but we may show a classic input (e.g. ssh/subshell warpification)
    pub const UNIVERSAL_DEVELOPER_INPUT_ENABLED: &str = "UniversalDeveloperInputEnabled";
    pub const AGENT_MODE_INPUT: &str = "InputAgentMode";
    pub const TERMINAL_MODE_INPUT: &str = "InputTerminalMode";
    pub const WARP_IS_DEFAULT_TERMINAL: &str = "WarpIsDefaultTerminal";
    pub const PASSIVE_CODE_DIFF_KEYBINDINGS_ENABLED: &str = "PassiveCodeDiffKeybindingsEnabled";
    /// When set, ctrl-enter should accept a prompt suggestion rather than insert a newline.
    /// This flag is set by the terminal Input when there's a pending passive code diff.
    pub const CTRL_ENTER_ACCEPTS_PROMPT_SUGGESTION: &str = "CtrlEnterAcceptsPromptSuggestion";
    /// When set, the terminal input owns Page Up / Page Down so the editor's fixed bindings
    /// should not match.
    pub const TERMINAL_INPUT_PAGE_KEYS_HANDLED_BY_INPUT: &str =
        "TerminalInputPageKeysHandledByInput";
    pub const HAS_PENDING_PROMPT_SUGGESTION: &str = "HasPendingPromptSuggestion";
    pub const ACTIVE_AGENT_VIEW: &str = "ActiveAgentView";
    pub const ACTIVE_INLINE_AGENT_VIEW: &str = "ActiveInlineAgentView";
    /// When set, ctrl-enter should be the active binding to enter agent view.
    ///
    /// This is true on linux and windows.
    pub const CTRL_ENTER_ENTERS_AGENT_VIEW: &str = "CtrlEnterEntersAgentView";
    pub const AGENT_VIEW_ENABLED: &str = "FeatureFlag.AgentView";
    pub const LOCKED_INPUT: &str = "LockedInput";
    pub const OPEN_INLINE_CONVERSATION_MENU: &str = "OpenInlineConversationMenu";
    pub const EMPTY_INPUT_BUFFER: &str = "EmptyInputBuffer";
    pub const CLI_AGENT_RICH_INPUT_OPEN: &str = "CLIAgentRichInputOpen";
    pub const CLI_AGENT_FOOTER_ENABLED: &str = "CLIAgentFooterEnabled";
    pub const CLI_AGENT_RICH_INPUT_CHIP_ENABLED: &str = "CLIAgentRichInputChipEnabled";
    pub const ENABLE_WARP_DRIVE: &str = "EnableWarpDrive";
    // Tools panel settings
    pub const SHOW_CONVERSATION_HISTORY: &str = "ShowConversationHistory";
    pub const SHOW_PROJECT_EXPLORER: &str = "ShowProjectExplorer";
    pub const SHOW_GLOBAL_SEARCH: &str = "ShowGlobalSearch";
}

pub fn init_actions_from_parent_view<T: Action + Clone>(
    app: &mut AppContext,
    context: &ContextPredicate,
    builder: fn(SettingsAction) -> T,
) {
    main_page::init_actions_from_parent_view(app, context, builder);
    appearance_page::init_actions_from_parent_view(app, context, builder);
    features_page::init_actions_from_parent_view(app, context, builder);
    warpify_page::init_actions_from_parent_view(app, context, builder);
    privacy_page::init_actions_from_parent_view(app, context, builder);
    ai_page::init_actions_from_parent_view(app, context, builder);
    code_page::init_actions_from_parent_view(app, context, builder);

    if ChannelState::enable_debug_features() || cfg!(windows) {
        ToggleSettingActionPair::add_toggle_setting_action_pairs_as_bindings(
            vec![
                ToggleSettingActionPair::custom(
                    SettingActionPairDescriptions::new(
                        "Show initialization block",
                        "Hide initialization block",
                    ),
                    builder(SettingsAction::Debug(
                        DebugSettingsAction::ToggleInitializationBlock,
                    )),
                    SettingActionPairContexts::new(
                        context.to_owned() & !id!(flags::INITIALIZATION_BLOCK_FLAG),
                        context.to_owned() & id!(flags::INITIALIZATION_BLOCK_FLAG),
                    ),
                    None,
                ),
                ToggleSettingActionPair::custom(
                    SettingActionPairDescriptions::new(
                        "Show in-band command blocks",
                        "Hide in-band command blocks",
                    ),
                    builder(SettingsAction::Debug(
                        DebugSettingsAction::ToggleInBandCommandBlocks,
                    )),
                    SettingActionPairContexts::new(
                        context.to_owned() & !id!(flags::IN_BAND_COMMAND_BLOCKS_FLAG),
                        context.to_owned() & id!(flags::IN_BAND_COMMAND_BLOCKS_FLAG),
                    ),
                    None,
                ),
            ],
            app,
        );
    }

    if FeatureFlag::DebugMode.is_enabled() {
        ToggleSettingActionPair::add_toggle_setting_action_pairs_as_bindings(
            vec![
                ToggleSettingActionPair::new(
                    "recording mode",
                    WorkspaceAction::ToggleRecordingMode,
                    &id!("Workspace"),
                    flags::RECORDING_MODE_FLAG,
                ),
                ToggleSettingActionPair::new(
                    "in-band generators for new sessions",
                    WorkspaceAction::ToggleInBandGenerators,
                    &id!("Workspace"),
                    flags::IN_BAND_GENERATORS_FLAG,
                ),
                ToggleSettingActionPair::new(
                    "debug network status",
                    WorkspaceAction::ToggleDebugNetworkStatus,
                    &id!("Workspace"),
                    flags::DEBUG_NETWORK_ONLINE_FLAG,
                ),
                ToggleSettingActionPair::new(
                    "memory statistics",
                    WorkspaceAction::ToggleShowMemoryStats,
                    &id!("Workspace"),
                    flags::DEBUG_SHOW_MEMORY_STATS_FLAG,
                ),
            ],
            app,
        );
    }

    let context = id!("SettingsViewInTab") & !id!("IMEOpen");
    app.register_fixed_bindings([
        FixedBinding::new("down", SettingsAction::Down, context.clone()),
        FixedBinding::new("up", SettingsAction::Up, context.clone()),
    ]);
}

/// The string the user will see when the action is enabled or disabled.
#[derive(Clone)]
pub struct SettingActionPairDescriptions {
    enable: String,
    disable: String,
}

impl SettingActionPairDescriptions {
    pub fn new(enable: &str, disable: &str) -> Self {
        Self {
            enable: enable.to_owned(),
            disable: disable.to_owned(),
        }
    }
}

/// The context to check to show the enable or disable
/// version of this action pair.
#[derive(Clone)]
pub struct SettingActionPairContexts {
    enable_predicate: ContextPredicate,
    disable_predicate: ContextPredicate,
}

impl SettingActionPairContexts {
    pub fn new(enable_predicate: ContextPredicate, disable_predicate: ContextPredicate) -> Self {
        Self {
            enable_predicate,
            disable_predicate,
        }
    }
}

/// Information needed to create a enable/disable action pair.
/// Note: The action pair doesn't actually need to update settings.
/// We should probably refactor this code to a different module.
#[derive(Clone)]
pub struct ToggleSettingActionPair<T: Action + Clone> {
    /// The user will actually read these strings.
    descriptions: SettingActionPairDescriptions,
    /// The actual action to toggle a setting on/off.
    toggle_action: T,
    /// We use our Context tree to determine where this setting should show up.
    /// Be sure to initialize all context strings you use
    /// in `fn keymap_context` in `impl View for Workspace`.
    contexts: SettingActionPairContexts,
    /// If Some(), custom_action is set as the Custom Trigger for the
    /// the toggle_action.
    /// This makes it possible to bind Mac menu items to the toggle_action.
    custom_action: Option<CustomAction>,
    /// Binding group for the set of actions produced by this pair. If not explicitly set, the
    /// `Settings` [`BindingGroup`] is applied.
    binding_group: BindingGroup,

    /// Predicate that determines if bindings corresponding to this pair are enabled.
    enabled_predicate: Option<EnabledPredicate>,

    /// Whether or not this pairing applies to the current platform (Mac, Linux, Web, etc.)
    supported_on_current_platform: bool,
}

impl<T: Action + Clone> ToggleSettingActionPair<T> {
    /// `description_suffix` will be visible to the user,
    /// e.g. `Enable {description_suffix}` or `Disable {description_suffix}`.
    /// We use contexts to decide if we show the user the enable or disable
    /// version of this action pair.
    /// `context_prefix` is logically ANDed with context_boolean_flag,
    /// like a prerequisite.
    /// `context_prefix` should be `Workspace` to have the action pair to
    /// display in the command palette.
    /// `context_boolean_flag` is will be in the context tree when the action
    /// is in the enabled state,
    /// and absent when the action is in the disabled state.
    pub fn new(
        description_suffix: &str,
        toggle_action: T,
        context_prefix: &ContextPredicate,
        context_boolean_flag: &'static str,
    ) -> Self {
        use warpui::keymap::macros::id;

        ToggleSettingActionPair {
            descriptions: SettingActionPairDescriptions {
                enable: format!("Enable {description_suffix}"),
                disable: format!("Disable {description_suffix}"),
            },
            contexts: SettingActionPairContexts {
                enable_predicate: context_prefix.to_owned() & !id!(context_boolean_flag),
                disable_predicate: context_prefix.to_owned() & id!(context_boolean_flag),
            },
            toggle_action,
            custom_action: None,
            binding_group: BindingGroup::Settings,
            supported_on_current_platform: true,
            enabled_predicate: None,
        }
    }

    pub fn custom(
        descriptions: SettingActionPairDescriptions,
        toggle_action: T,
        contexts: SettingActionPairContexts,
        custom_action: Option<CustomAction>,
    ) -> Self {
        ToggleSettingActionPair {
            toggle_action,
            contexts,
            descriptions,
            custom_action,
            binding_group: BindingGroup::Settings,
            supported_on_current_platform: true,
            enabled_predicate: None,
        }
    }

    pub fn with_group(mut self, group: BindingGroup) -> Self {
        self.binding_group = group;
        self
    }

    pub fn with_enabled(mut self, enabled_predicate: EnabledPredicate) -> Self {
        self.enabled_predicate = Some(enabled_predicate);
        self
    }

    pub fn is_supported_on_current_platform(&self, value: bool) -> Self {
        ToggleSettingActionPair {
            descriptions: self.descriptions.clone(),
            toggle_action: self.toggle_action.clone(),
            contexts: self.contexts.clone(),
            custom_action: self.custom_action,
            binding_group: self.binding_group,
            supported_on_current_platform: value,
            enabled_predicate: None,
        }
    }

    /// Creates enable/disable bindings for a toggle feature, given a list of `ToggleSettingActionPair`'s.
    pub fn add_toggle_setting_action_pairs_as_bindings(
        action_pairs: Vec<ToggleSettingActionPair<T>>,
        app: &mut AppContext,
    ) {
        let (enable_bindings, disable_bindings): (Vec<FixedBinding>, Vec<FixedBinding>) =
            action_pairs
                .into_iter()
                .filter_map(|action_pair| {
                    let ToggleSettingActionPair {
                        toggle_action,
                        contexts,
                        descriptions,
                        custom_action,
                        binding_group,
                        supported_on_current_platform,
                        enabled_predicate,
                    } = action_pair;

                    if !supported_on_current_platform {
                        None
                    } else {
                        match custom_action {
                            Some(custom_action) => {
                                let mut enable_binding = FixedBinding::custom(
                                    custom_action,
                                    toggle_action.clone(),
                                    descriptions.enable,
                                    contexts.enable_predicate,
                                )
                                .with_group(binding_group.as_str());
                                let mut disable_binding = FixedBinding::custom(
                                    custom_action,
                                    toggle_action,
                                    descriptions.disable,
                                    contexts.disable_predicate,
                                )
                                .with_group(binding_group.as_str());

                                if let Some(enabled_predicate) = enabled_predicate {
                                    enable_binding = enable_binding.with_enabled(enabled_predicate);
                                    disable_binding =
                                        disable_binding.with_enabled(enabled_predicate);
                                }

                                Some((enable_binding, disable_binding))
                            }
                            None => {
                                let mut enable_binding = FixedBinding::empty(
                                    descriptions.enable,
                                    toggle_action.clone(),
                                    contexts.enable_predicate,
                                )
                                .with_group(binding_group.as_str());
                                let mut disable_binding = FixedBinding::empty(
                                    descriptions.disable,
                                    toggle_action,
                                    contexts.disable_predicate,
                                )
                                .with_group(binding_group.as_str());

                                if let Some(enabled_predicate) = enabled_predicate {
                                    enable_binding = enable_binding.with_enabled(enabled_predicate);
                                    disable_binding =
                                        disable_binding.with_enabled(enabled_predicate);
                                }

                                Some((enable_binding, disable_binding))
                            }
                        }
                    }
                })
                .unzip();

        app.register_fixed_bindings(enable_bindings);
        app.register_fixed_bindings(disable_bindings);
    }
}

#[derive(Clone, Debug)]
pub enum DebugSettingsAction {
    /// Whether or not the "bootstrap block" or "initialization block" is visible.
    ToggleInitializationBlock,
    /// Whether or not in-band generator commands are visible in the BlockList.
    ToggleInBandCommandBlocks,
}

#[derive(Debug, Clone)]
pub enum SettingsAction {
    SelectAndRefresh(SettingsSection),
    ToggleUmbrella(usize),
    MainPageToggle(MainPageAction),
    AppearancePageToggle(AppearancePageAction),
    FeaturesPageToggle(FeaturesPageAction),
    PrivacyPageToggle(PrivacyPageAction),
    AI(AISettingsPageAction),
    Code(CodeSettingsPageAction),
    WarpDrive(warp_drive_page::WarpDriveSettingsPageAction),
    WarpifyPageToggle(WarpifyPageAction),
    Tab,
    Split(Direction),
    ToggleMaximizePane,
    Close,
    OpenContextMenu(Vector2F),
    FocusSelf,
    Up,
    Down,
    /// For internal, debug-related settings which don't appear in the UI.
    Debug(DebugSettingsAction),
}

#[derive(Copy, Clone, Debug)]
enum CycleDirection {
    Up,
    Down,
}

/// A stop in the arrow-key navigation order over the sidebar.
///
/// A collapsed umbrella occupies a single stop rather than being skipped,
/// so arrow-key navigation auto-expands it and selects one of its visible
/// subpages instead of jumping over it. Which subpage is chosen depends
/// on the direction of cycling: navigating Down enters the umbrella at
/// its first visible subpage, while navigating Up enters at its last
/// visible subpage, matching the natural reading order the user was
/// moving through.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum NavStop {
    /// A concrete page, or a subpage of an already-expanded umbrella.
    /// Arrow-key nav lands directly on this section.
    Section(SettingsSection),
    /// A collapsed umbrella. Activating this stop navigates to either
    /// `first_subpage` (when arriving from above via Down) or
    /// `last_subpage` (when arriving from below via Up), which
    /// auto-expands the umbrella via
    /// [`SettingsView::set_and_refresh_current_page_internal`].
    CollapsedUmbrella {
        /// Index into `nav_items`. Used to detect when the currently active
        /// page belongs to this collapsed umbrella (e.g. the user manually
        /// collapsed it while on one of its subpages), so cycling still
        /// moves relative to the umbrella's position in the nav order.
        nav_index: usize,
        first_subpage: SettingsSection,
        last_subpage: SettingsSection,
    },
}

/// Builds the ordered list of arrow-key nav stops from `nav_items`.
///
/// `is_visible` decides which sections are currently shown in the sidebar;
/// callers pass a predicate that ignores the search filter when no search
/// is active and applies it otherwise. Umbrellas with no visible subpages
/// are skipped entirely.
fn build_nav_stops<F>(nav_items: &[SettingsNavItem], is_visible: F) -> Vec<NavStop>
where
    F: Fn(SettingsSection) -> bool,
{
    nav_items
        .iter()
        .enumerate()
        .flat_map(|(nav_index, item)| match item {
            SettingsNavItem::Page(section) => {
                if is_visible(*section) {
                    vec![NavStop::Section(*section)]
                } else {
                    vec![]
                }
            }
            SettingsNavItem::Umbrella(umbrella) => {
                let visible: Vec<SettingsSection> = umbrella
                    .subpages
                    .iter()
                    .copied()
                    .filter(|s| is_visible(*s))
                    .collect();
                if visible.is_empty() {
                    vec![]
                } else if umbrella.expanded {
                    visible.into_iter().map(NavStop::Section).collect()
                } else {
                    let first_subpage = visible[0];
                    let last_subpage = *visible.last().unwrap_or(&first_subpage);
                    vec![NavStop::CollapsedUmbrella {
                        nav_index,
                        first_subpage,
                        last_subpage,
                    }]
                }
            }
        })
        .collect()
}

/// Returns the index in `stops` that corresponds to `section`.
///
/// A collapsed-umbrella stop also matches when `section` is one of the
/// umbrella's subpages — this covers the edge case where the user manually
/// collapsed the umbrella while still on a subpage, so arrow-key cycling
/// continues to move relative to the umbrella's position in the nav order.
fn current_stop_index(
    stops: &[NavStop],
    nav_items: &[SettingsNavItem],
    section: SettingsSection,
) -> Option<usize> {
    stops.iter().position(|stop| match stop {
        NavStop::Section(s) => *s == section,
        NavStop::CollapsedUmbrella { nav_index, .. } => matches!(
            nav_items.get(*nav_index),
            Some(SettingsNavItem::Umbrella(u)) if u.contains(section)
        ),
    })
}

/// Returns the next index after applying `direction`, wrapping around the
/// ends of the list. Caller must ensure `len > 0`.
fn next_stop_index(current: usize, len: usize, direction: CycleDirection) -> usize {
    debug_assert!(len > 0, "next_stop_index requires a non-empty stop list");
    match direction {
        CycleDirection::Up => {
            if current == 0 {
                len - 1
            } else {
                current - 1
            }
        }
        CycleDirection::Down => {
            if current + 1 >= len {
                0
            } else {
                current + 1
            }
        }
    }
}

macro_rules! update_page {
    ($handle:expr, $update:expr, $ctx:expr) => {
        match $handle {
            SettingsPageViewHandle::Main(handle) => $ctx.update_view(handle, $update),
            SettingsPageViewHandle::Appearance(handle) => $ctx.update_view(handle, $update),
            SettingsPageViewHandle::Features(handle) => $ctx.update_view(handle, $update),
            SettingsPageViewHandle::SharedBlocks(handle) => $ctx.update_view(handle, $update),
            SettingsPageViewHandle::Keybindings(handle) => $ctx.update_view(handle, $update),
            SettingsPageViewHandle::Teams(handle) => $ctx.update_view(handle, $update),
            SettingsPageViewHandle::Warpify(handle) => $ctx.update_view(handle, $update),
            SettingsPageViewHandle::OzCloudAPIKeys(handle) => $ctx.update_view(handle, $update),
            SettingsPageViewHandle::Privacy(handle) => $ctx.update_view(handle, $update),
            SettingsPageViewHandle::Referrals(handle) => $ctx.update_view(handle, $update),
            SettingsPageViewHandle::AI(handle) => $ctx.update_view(handle, $update),
            SettingsPageViewHandle::CloudEnvironments(handle) => $ctx.update_view(handle, $update),
            SettingsPageViewHandle::About(handle) => $ctx.update_view(handle, $update),
            SettingsPageViewHandle::Code(handle) => $ctx.update_view(handle, $update),
            SettingsPageViewHandle::BillingAndUsage(handle) => $ctx.update_view(handle, $update),
            SettingsPageViewHandle::BillingAndUsageV2(handle) => $ctx.update_view(handle, $update),
            SettingsPageViewHandle::MCPServers(handle) => $ctx.update_view(handle, $update),
            SettingsPageViewHandle::WarpDrive(handle) => $ctx.update_view(handle, $update),
        }
    };
}

pub struct SettingsView {
    pane_configuration: ModelHandle<PaneConfiguration>,
    focus_handle: Option<PaneFocusHandle>,
    settings_pages: Vec<SettingsPage>,
    pages_filter: Vec<MatchData>,
    current_settings_page: SettingsSection,
    search_editor: ViewHandle<EditorView>,
    clipped_scroll_state: ClippedScrollStateHandle,
    context_menu: ViewHandle<Menu<SettingsAction>>,
    context_menu_state: Option<Vector2F>,
    environments_page_handle: ViewHandle<EnvironmentsPageView>,
    /// Sidebar navigation items (pages + umbrellas).
    nav_items: Vec<SettingsNavItem>,
    /// Handle to the AI settings page, used to switch subpage modes.
    ai_page_handle: ViewHandle<AISettingsPageView>,
    /// Handle to the Code settings page, used to switch subpage modes.
    code_page_handle: ViewHandle<CodeSettingsPageView>,
    /// Per-subpage search match results. Populated during search so that
    /// subpages sharing the same backing page can be filtered independently.
    subpage_filter: HashMap<SettingsSection, MatchData>,
    /// Current settings.toml error, mirrored from `Workspace` via
    /// [`set_settings_error_state`]. Used by the sidebar footer to decide
    /// whether to show the inline error alert.
    settings_file_error: Option<SettingsFileError>,
    /// Whether the workspace-level settings-error banner has been dismissed.
    /// Mirrored from `Workspace` via [`set_settings_error_state`].
    settings_error_banner_dismissed: bool,
    /// Mouse state handles for the nav-rail footer buttons. Constructed once
    /// per `SettingsView` per `WARP.md`'s guidance that inline
    /// `MouseStateHandle::default()` breaks hover/click tracking.
    footer_mouse_states: SettingsFooterMouseStates,
}

impl SettingsView {
    pub fn new(page: Option<SettingsSection>, ctx: &mut ViewContext<Self>) -> Self {
        let pane_configuration = ctx.add_model(|_ctx| PaneConfiguration::new("Settings"));

        let global_resource_handles = GlobalResourceHandlesProvider::as_ref(ctx).get().clone();
        // Main settings page with accounts info
        let main_page_handle = ctx.add_typed_action_view(MainSettingsPageView::new);
        ctx.subscribe_to_view(&main_page_handle, |me, _, event, ctx| {
            me.handle_main_page_event(event, ctx);
        });

        // Appearance & themes page
        let appearance_page_handle = ctx.add_typed_action_view(AppearanceSettingsPageView::new);
        ctx.subscribe_to_view(&appearance_page_handle, |me, _, event, ctx| {
            me.handle_appearance_page_event(event, ctx);
        });

        // Features page
        let features_page_handle = ctx.add_typed_action_view(|ctx| {
            FeaturesPageView::new(global_resource_handles.clone(), ctx)
        });

        ctx.subscribe_to_view(&features_page_handle, |me, _, event, ctx| {
            me.handle_features_page_event(event, ctx);
        });

        // Shared blocks page
        let block_client = ServerApiProvider::as_ref(ctx).get_block_client();
        let show_blocks_view_handle =
            ctx.add_typed_action_view(|ctx| ShowBlocksView::new(block_client, ctx));

        ctx.subscribe_to_view(&show_blocks_view_handle, |_, _, event, ctx| match event {
            ShowBlocksEvent::ShowToast { message, flavor } => {
                ctx.emit(SettingsViewEvent::ShowToast {
                    message: message.clone(),
                    flavor: *flavor,
                })
            }
        });

        // About page
        let about_page_handle = ctx.add_view(AboutPageView::new);

        // AI page
        let ai_page_handle = ctx.add_typed_action_view(AISettingsPageView::new);
        let ai_page_handle_for_nav = ai_page_handle.clone();
        ctx.subscribe_to_view(&ai_page_handle, |me, _, event, ctx| {
            me.handle_ai_page_event(event, ctx);
        });

        // Environments page
        let environments_page_handle = ctx.add_typed_action_view(EnvironmentsPageView::new);
        ctx.subscribe_to_view(&environments_page_handle, |me, _, event, ctx| {
            me.handle_environments_page_event(event, ctx);
        });

        let should_use_billing_and_usage_v2 = FeatureFlag::BillingAndUsagePageV2.is_enabled();
        let billing_and_usage_page: SettingsPage = if should_use_billing_and_usage_v2 {
            let handle = ctx.add_typed_action_view(BillingAndUsagePageV2View::new);
            ctx.subscribe_to_view(&handle, |me, _, event, ctx| {
                me.handle_billing_and_usage_page_event(event, ctx);
            });
            SettingsPage::new(handle)
        } else {
            let handle = ctx.add_typed_action_view(BillingAndUsagePageView::new);
            ctx.subscribe_to_view(&handle, |me, _, event, ctx| {
                me.handle_billing_and_usage_page_event(event, ctx);
            });
            SettingsPage::new(handle)
        };

        // Keybindings page
        let keybindings_handle = ctx.add_typed_action_view(KeybindingsView::new);

        // Code page
        let code_page_handle = ctx.add_typed_action_view(CodeSettingsPageView::new);
        let code_page_handle_for_nav = code_page_handle.clone();
        ctx.subscribe_to_view(&code_page_handle, |me, _, event, ctx| {
            me.handle_code_page_event(event, ctx);
        });

        // Teams page, adding unconditionally, as `should_render` later on decides whether it
        // should be shown to the user or not
        let teams_page_handle = ctx.add_typed_action_view(TeamsPageView::new);
        ctx.subscribe_to_view(&teams_page_handle, |_, _, event, ctx| match event {
            TeamsPageViewEvent::TeamsChanged => ctx.notify(),
            TeamsPageViewEvent::OpenWarpDrive => ctx.emit(SettingsViewEvent::OpenWarpDrive),
            TeamsPageViewEvent::ShowToast { message, flavor } => {
                ctx.emit(SettingsViewEvent::ShowToast {
                    message: message.clone(),
                    flavor: *flavor,
                })
            }
        });

        let warpify_page_handle = ctx.add_typed_action_view(WarpifyPageView::new);
        ctx.subscribe_to_view(&warpify_page_handle, |me, _, event, ctx| {
            me.handle_warpify_page_event(event, ctx);
        });

        // Render the privacy page only if telemetry opt-out is enabled.
        let privacy_page_handle = ctx.add_typed_action_view(PrivacyPageView::new);
        ctx.subscribe_to_view(&privacy_page_handle, |me, _, event, ctx| {
            me.handle_privacy_page_event(event, ctx);
        });

        let referrals_client = ServerApiProvider::as_ref(ctx).get_referrals_client();
        let referrals_page_handle =
            ctx.add_typed_action_view(|ctx| ReferralsPageView::new(referrals_client, ctx));
        ctx.subscribe_to_view(&referrals_page_handle, |me, _, event, ctx| {
            me.handle_referrals_page_event(event, ctx);
        });

        // Warp Drive page
        let warp_drive_page_handle =
            ctx.add_typed_action_view(warp_drive_page::WarpDriveSettingsPageView::new);
        ctx.subscribe_to_view(&warp_drive_page_handle, |me, _, event, ctx| {
            me.handle_warp_drive_page_event(event, ctx);
        });

        let platform_page_handle = ctx.add_typed_action_view(platform_page::PlatformPageView::new);
        ctx.subscribe_to_view(&platform_page_handle, |me, _, event, ctx| {
            me.handle_platform_page_event(event, ctx);
        });

        // MCP Servers page
        let mcp_servers_page_handle = ctx.add_typed_action_view(MCPServersSettingsPageView::new);
        ctx.subscribe_to_view(&mcp_servers_page_handle, |me, _, event, ctx| {
            me.handle_mcp_servers_page_event(event, ctx);
        });

        let font_family = Appearance::as_ref(ctx).ui_font_family();
        let search_editor = ctx.add_typed_action_view(|ctx| {
            let options = SingleLineEditorOptions {
                text: TextOptions {
                    font_family_override: Some(font_family),
                    ..Default::default()
                },
                // We want "up" and "down" to cycle settings pages.
                propagate_and_no_op_vertical_navigation_keys:
                    PropagateAndNoOpNavigationKeys::Always,
                ..Default::default()
            };
            let mut editor = EditorView::single_line(options, ctx);
            editor.set_placeholder_text("Search", ctx);
            editor
        });

        ctx.subscribe_to_view(&search_editor, Self::handle_search_editor_event);

        let context_menu = ctx.add_typed_action_view(|_| {
            Menu::new()
                .prevent_interaction_with_other_elements()
                .with_drop_shadow()
        });
        ctx.subscribe_to_view(&context_menu, move |me, _, event, ctx| {
            me.handle_menu_event(event, ctx);
        });

        let mut settings_pages = vec![
            SettingsPage::new(main_page_handle),
            SettingsPage::new(ai_page_handle),
            billing_and_usage_page,
            SettingsPage::new(code_page_handle),
            SettingsPage::new(teams_page_handle),
            SettingsPage::new(appearance_page_handle),
            SettingsPage::new(features_page_handle),
            SettingsPage::new(keybindings_handle),
            SettingsPage::new(platform_page_handle),
            SettingsPage::new(warpify_page_handle),
            SettingsPage::new(referrals_page_handle),
            SettingsPage::new(show_blocks_view_handle),
            SettingsPage::new(warp_drive_page_handle),
        ];

        settings_pages.extend(vec![
            SettingsPage::new(mcp_servers_page_handle),
            SettingsPage::new(environments_page_handle.clone()),
            SettingsPage::new(privacy_page_handle),
            SettingsPage::new(about_page_handle),
        ]);

        // Build sidebar nav items. AI page is presented as an "Agents" umbrella
        // with subpages; the actual AI SettingsPage is hidden from direct sidebar listing.
        let mut nav_items = vec![
            SettingsNavItem::Page(SettingsSection::Account),
            SettingsNavItem::Umbrella(SettingsUmbrella::new(
                "Agents",
                SettingsSection::ai_subpages().to_vec(),
            )),
            SettingsNavItem::Page(SettingsSection::BillingAndUsage),
            SettingsNavItem::Umbrella(SettingsUmbrella::new(
                "Code",
                vec![
                    SettingsSection::CodeIndexing,
                    SettingsSection::EditorAndCodeReview,
                ],
            )),
            SettingsNavItem::Umbrella(SettingsUmbrella::new(
                "Cloud platform",
                vec![
                    SettingsSection::CloudEnvironments,
                    SettingsSection::OzCloudAPIKeys,
                ],
            )),
            SettingsNavItem::Page(SettingsSection::Teams),
            SettingsNavItem::Page(SettingsSection::Appearance),
            SettingsNavItem::Page(SettingsSection::Features),
            SettingsNavItem::Page(SettingsSection::Keybindings),
            SettingsNavItem::Page(SettingsSection::Warpify),
            SettingsNavItem::Page(SettingsSection::Referrals),
            SettingsNavItem::Page(SettingsSection::SharedBlocks),
            SettingsNavItem::Page(SettingsSection::WarpDrive),
            SettingsNavItem::Page(SettingsSection::Privacy),
            SettingsNavItem::Page(SettingsSection::About),
        ];

        // Resolve the initial page: map internal backing-page sections to their default subpage.
        let initial_page = match page {
            Some(SettingsSection::AI) => SettingsSection::WarpAgent,
            Some(SettingsSection::Code) => SettingsSection::CodeIndexing,
            Some(section) if section.is_subpage() => section,
            other => other.unwrap_or_default(),
        };

        // Auto-expand the umbrella if the initial page is one of its subpages.
        if initial_page.is_subpage() {
            for item in &mut nav_items {
                if let SettingsNavItem::Umbrella(umbrella) = item {
                    if umbrella.contains(initial_page) {
                        umbrella.expanded = true;
                    }
                }
            }
        }

        Self {
            pages_filter: settings_pages
                .iter()
                .map(|_| MatchData::Uncounted(true))
                .collect(),
            settings_pages,
            current_settings_page: initial_page,
            pane_configuration,
            focus_handle: None,
            search_editor,
            clipped_scroll_state: Default::default(),
            context_menu,
            context_menu_state: Default::default(),
            environments_page_handle,
            nav_items,
            ai_page_handle: ai_page_handle_for_nav,
            code_page_handle: code_page_handle_for_nav,
            subpage_filter: HashMap::new(),
            settings_file_error: None,
            settings_error_banner_dismissed: false,
            footer_mouse_states: SettingsFooterMouseStates::default(),
        }
    }

    /// Pushes the current settings-file error state from `Workspace` into this
    /// view. Called by `Workspace` once at construction time and then again
    /// whenever the error state or banner dismissal changes. Triggers a
    /// re-render when anything actually changed.
    pub fn set_settings_error_state(
        &mut self,
        error: Option<SettingsFileError>,
        banner_dismissed: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        let error_changed = self.settings_file_error != error;
        let dismissed_changed = self.settings_error_banner_dismissed != banner_dismissed;
        if !error_changed && !dismissed_changed {
            return;
        }
        self.settings_file_error = error;
        self.settings_error_banner_dismissed = banner_dismissed;
        ctx.notify();
    }

    pub fn pane_configuration(&self) -> ModelHandle<PaneConfiguration> {
        self.pane_configuration.clone()
    }

    pub fn focus(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.focus(&self.search_editor);
        ctx.emit(SettingsViewEvent::Pane(PaneEvent::FocusSelf));
    }

    fn filtered_pages<'a>(
        &'a self,
        app: &'a AppContext,
    ) -> impl Iterator<Item = (&'a SettingsPage, MatchData)> {
        self.settings_pages
            .iter()
            .zip(self.pages_filter.iter())
            .filter_map(move |(page, match_data)| {
                (self.should_render_page(page, app) && match_data.is_truthy())
                    .then_some((page, *match_data))
            })
    }

    fn handle_search_editor_event(
        &mut self,
        editor: ViewHandle<EditorView>,
        event: &EditorEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            EditorEvent::Edited(_) => {
                let search_query = editor.as_ref(ctx).buffer_text(ctx);
                let is_search_active = !search_query.is_empty();

                if is_search_active {
                    // Save umbrella expanded state before search modifies it.
                    for item in &mut self.nav_items {
                        if let SettingsNavItem::Umbrella(umbrella) = item {
                            if umbrella.pre_search_expanded.is_none() {
                                umbrella.pre_search_expanded = Some(umbrella.expanded);
                            }
                        }
                    }

                    // Run per-subpage filtering for pages with multiple subpages.
                    // For each AI subpage, temporarily switch to that subpage's
                    // widget set and run the filter to get a subpage-specific result.
                    self.subpage_filter.clear();
                    for &subpage_section in SettingsSection::ai_subpages() {
                        if subpage_section == SettingsSection::AgentMCPServers {
                            // AgentMCPServers has its own backing page; handled below.
                            continue;
                        }
                        if let Some(subpage) = AISubpage::from_section(subpage_section) {
                            self.ai_page_handle.update(ctx, |view, ctx| {
                                view.set_active_subpage(Some(subpage), ctx);
                            });
                            let match_data = self
                                .ai_page_handle
                                .update(ctx, |view, ctx| view.update_filter(&search_query, ctx));
                            self.subpage_filter.insert(subpage_section, match_data);
                        }
                    }
                    // Do the same for Code subpages.
                    for &subpage_section in SettingsSection::code_subpages() {
                        if let Some(subpage) = CodeSubpage::from_section(subpage_section) {
                            self.code_page_handle.update(ctx, |view, ctx| {
                                view.set_active_subpage(Some(subpage), ctx);
                            });
                            let match_data = self
                                .code_page_handle
                                .update(ctx, |view, ctx| view.update_filter(&search_query, ctx));
                            self.subpage_filter.insert(subpage_section, match_data);
                        }
                    }
                } else {
                    // Search cleared: restore umbrella expanded state.
                    for item in &mut self.nav_items {
                        if let SettingsNavItem::Umbrella(umbrella) = item {
                            if let Some(saved) = umbrella.pre_search_expanded.take() {
                                umbrella.expanded = saved;
                            }
                        }
                    }
                    self.subpage_filter.clear();
                }

                // Run the standard page-level filter (needed for non-subpage pages
                // and for subpages with their own backing page like AgentMCPServers).
                // Switch AI/Code to all-widgets mode so standalone backing page
                // filter is correct for pages_filter.
                if is_search_active {
                    self.ai_page_handle.update(ctx, |view, ctx| {
                        view.set_active_subpage(None, ctx);
                    });
                    self.code_page_handle.update(ctx, |view, ctx| {
                        view.set_active_subpage(None, ctx);
                    });
                }

                for (i, page) in self.settings_pages.iter().enumerate() {
                    self.pages_filter[i] = update_page!(
                        &page.view_handle,
                        |view, ctx| {
                            let match_data = view.update_filter(&search_query, ctx);
                            ctx.notify();
                            match_data
                        },
                        ctx
                    );
                }

                // Restore the active subpage after filtering.
                if is_search_active {
                    let current = self.current_settings_page;
                    if current.is_ai_subpage() && current != SettingsSection::AgentMCPServers {
                        if let Some(subpage) = AISubpage::from_section(current) {
                            self.ai_page_handle.update(ctx, |view, ctx| {
                                view.set_active_subpage(Some(subpage), ctx);
                            });
                        }
                    }
                    if current.is_code_subpage() {
                        if let Some(subpage) = CodeSubpage::from_section(current) {
                            self.code_page_handle.update(ctx, |view, ctx| {
                                view.set_active_subpage(Some(subpage), ctx);
                            });
                        }
                    }
                }

                // Auto-expand umbrellas that have matching subpages during search.
                if is_search_active {
                    for item in &mut self.nav_items {
                        if let SettingsNavItem::Umbrella(umbrella) = item {
                            let has_match = umbrella.subpages.iter().any(|subpage_section| {
                                self.subpage_filter
                                    .get(subpage_section)
                                    .map(|md| md.is_truthy())
                                    .unwrap_or_else(|| {
                                        // Subpages with their own backing page
                                        // (e.g. AgentMCPServers, CloudEnvironments)
                                        // fall back to pages_filter.
                                        let backing = subpage_section.parent_page_section();
                                        self.settings_pages
                                            .iter()
                                            .zip(self.pages_filter.iter())
                                            .any(|(p, md)| p.section == backing && md.is_truthy())
                                    })
                            });
                            if has_match {
                                umbrella.expanded = true;
                            }
                        }
                    }
                }

                // Auto-select: if the current subpage/page is no longer visible,
                // jump to the first visible subpage or page.
                let current_still_visible = if is_search_active {
                    // For subpages with per-subpage filter, check the subpage itself.
                    if let Some(md) = self.subpage_filter.get(&self.current_settings_page) {
                        md.is_truthy()
                    } else {
                        // Fall back to backing page filter.
                        let current_backing = self.current_settings_page.parent_page_section();
                        self.filtered_pages(ctx)
                            .any(|(page, _)| page.section == current_backing)
                    }
                } else {
                    let current_backing = self.current_settings_page.parent_page_section();
                    self.filtered_pages(ctx)
                        .any(|(page, _)| page.section == current_backing)
                };

                if !current_still_visible {
                    // Find the first visible section: check subpages first, then pages.
                    let first_visible = if is_search_active {
                        self.nav_items
                            .iter()
                            .flat_map(|item| match item {
                                SettingsNavItem::Page(section) => vec![*section],
                                SettingsNavItem::Umbrella(umbrella) => umbrella.subpages.clone(),
                            })
                            .find(|section| {
                                if let Some(md) = self.subpage_filter.get(section) {
                                    md.is_truthy()
                                } else {
                                    let backing = section.parent_page_section();
                                    self.settings_pages
                                        .iter()
                                        .zip(self.pages_filter.iter())
                                        .any(|(p, md)| p.section == backing && md.is_truthy())
                                }
                            })
                    } else {
                        self.filtered_pages(ctx)
                            .next()
                            .map(|(page, _)| page.section)
                    };

                    if let Some(new_section) = first_visible {
                        self.set_and_refresh_current_page_internal(
                            new_section,
                            false, /* should_clear_query */
                            false, /* allow_steal_focus */
                            ctx,
                        );
                    }
                }
                ctx.notify();
            }
            EditorEvent::Navigate(NavigationKey::Down) => self.key_down(ctx),
            EditorEvent::Navigate(NavigationKey::Up) => self.key_up(ctx),
            EditorEvent::Escape => ctx.focus_self(),
            _ => {}
        }
    }

    fn context_menu_items(&self, ctx: &mut ViewContext<Self>) -> Vec<MenuItem<SettingsAction>> {
        let mut items = vec![];

        if ContextFlag::CreateNewSession.is_enabled() {
            items.extend(vec![
                MenuItemFields::new("Split pane right")
                    .with_on_select_action(SettingsAction::Split(Direction::Right))
                    .with_key_shortcut_label(keybinding_name_to_display_string(
                        "pane_group:add_right",
                        ctx,
                    ))
                    .into_item(),
                MenuItemFields::new("Split pane left")
                    .with_on_select_action(SettingsAction::Split(Direction::Left))
                    .with_key_shortcut_label(keybinding_name_to_display_string(
                        "pane_group:add_left",
                        ctx,
                    ))
                    .into_item(),
                MenuItemFields::new("Split pane down")
                    .with_on_select_action(SettingsAction::Split(Direction::Down))
                    .with_key_shortcut_label(keybinding_name_to_display_string(
                        "pane_group:add_down",
                        ctx,
                    ))
                    .into_item(),
                MenuItemFields::new("Split pane up")
                    .with_on_select_action(SettingsAction::Split(Direction::Up))
                    .with_key_shortcut_label(keybinding_name_to_display_string(
                        "pane_group:add_up",
                        ctx,
                    ))
                    .into_item(),
            ]);
        }

        let split_pane_state = self
            .focus_handle
            .as_ref()
            .map(|h| h.split_pane_state(ctx))
            .unwrap_or(SplitPaneState::NotInSplitPane);

        if split_pane_state.is_in_split_pane() {
            let is_maximized = split_pane_state.is_maximized();
            items.push(
                MenuItemFields::toggle_pane_action(is_maximized)
                    .with_on_select_action(SettingsAction::ToggleMaximizePane)
                    .with_key_shortcut_label(keybinding_name_to_display_string(
                        "pane_group:toggle_maximize_pane",
                        ctx,
                    ))
                    .into_item(),
            );

            items.push(
                MenuItemFields::new("Close pane")
                    .with_on_select_action(SettingsAction::Close)
                    .with_key_shortcut_label(
                        custom_tag_to_keystroke(CustomAction::CloseCurrentSession.into())
                            .map(|keystroke| keystroke.displayed()),
                    )
                    .into_item(),
            );
        }

        items
    }

    fn handle_menu_event(&mut self, event: &menu::Event, ctx: &mut ViewContext<Self>) {
        if let menu::Event::Close { .. } = event {
            self.context_menu_state.take();
        }
        ctx.notify();
    }

    fn clear_search_query(&mut self, ctx: &mut ViewContext<Self>) {
        self.search_editor.update(ctx, |editor, ctx| {
            editor.clear_buffer(ctx);
        });
        self.pages_filter = self
            .settings_pages
            .iter()
            .map(|_| MatchData::Uncounted(true))
            .collect();
    }

    fn handle_main_page_event(
        &mut self,
        event: &MainSettingsPageEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            MainSettingsPageEvent::CheckForUpdate => ctx.emit(SettingsViewEvent::CheckForUpdate),
            MainSettingsPageEvent::SignupAnonymousUser => {
                ctx.emit(SettingsViewEvent::SignupAnonymousUser)
            }
            _ => (),
        }
    }

    fn handle_billing_and_usage_page_event(
        &mut self,
        event: &BillingAndUsagePageEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            BillingAndUsagePageEvent::SignupAnonymousUser => {
                ctx.emit(SettingsViewEvent::SignupAnonymousUser)
            }
            BillingAndUsagePageEvent::ShowToast { message, flavor } => {
                ctx.emit(SettingsViewEvent::ShowToast {
                    message: message.clone(),
                    flavor: *flavor,
                })
            }
            BillingAndUsagePageEvent::ShowModal => ctx.notify(),
            BillingAndUsagePageEvent::HideModal => ctx.notify(),
        }
    }

    fn handle_appearance_page_event(
        &mut self,
        event: &SettingsPageEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            SettingsPageEvent::FocusModal => ctx.focus(&self.search_editor),
            SettingsPageEvent::Pane(_)
            | SettingsPageEvent::EnvironmentSetupModeSelectorToggled { .. }
            | SettingsPageEvent::AgentAssistedEnvironmentModalToggled { .. } => {
                // These events are not handled in standalone settings - only used
                // when the view is hosted inside a pane.
            }
        }
    }

    fn handle_environments_page_event(
        &mut self,
        event: &SettingsPageEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            SettingsPageEvent::FocusModal => ctx.focus(&self.search_editor),
            SettingsPageEvent::EnvironmentSetupModeSelectorToggled { .. }
            | SettingsPageEvent::AgentAssistedEnvironmentModalToggled { .. } => {
                // Re-render so the modal overlay is shown/hidden.
                ctx.notify();
            }
            SettingsPageEvent::Pane(_) => {
                // Not applicable in standalone settings view.
            }
        }
    }

    fn handle_features_page_event(
        &mut self,
        event: &FeaturesSettingsPageEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            FeaturesSettingsPageEvent::SearchForKeybinding(query) => {
                self.search_for_keybinding(query, ctx);
            }
            FeaturesSettingsPageEvent::FocusModal => ctx.focus(&self.search_editor),
        }
    }

    fn handle_warpify_page_event(
        &mut self,
        event: &SettingsPageEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            SettingsPageEvent::FocusModal => ctx.focus(&self.search_editor),
            SettingsPageEvent::Pane(_)
            | SettingsPageEvent::EnvironmentSetupModeSelectorToggled { .. }
            | SettingsPageEvent::AgentAssistedEnvironmentModalToggled { .. } => {
                // These events are not handled in standalone settings - only used
                // when the view is hosted inside a pane.
            }
        }
    }

    fn handle_privacy_page_event(
        &mut self,
        event: &PrivacyPageViewEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            PrivacyPageViewEvent::LaunchNetworkLogging => {
                ctx.emit(SettingsViewEvent::LaunchNetworkLogging);
            }
            PrivacyPageViewEvent::ShowAddRegexModal => {
                // Modal rendering is handled in get_modal_content_for_page
                ctx.notify();
            }
            PrivacyPageViewEvent::HideAddRegexModal => {
                // Modal rendering is handled in get_modal_content_for_page
                ctx.notify();
            }
        }
    }

    fn handle_platform_page_event(
        &mut self,
        event: &platform_page::PlatformPageViewEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            platform_page::PlatformPageViewEvent::ShowCreateApiKeyModal => {
                // Modal rendering is handled in get_modal_content_for_page
                ctx.notify();
            }
            platform_page::PlatformPageViewEvent::HideCreateApiKeyModal => {
                // Modal rendering is handled in get_modal_content_for_page
                ctx.notify();
            }
        }
    }

    fn handle_mcp_servers_page_event(
        &mut self,
        event: &MCPServersSettingsPageEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            MCPServersSettingsPageEvent::ShowModal => {
                // Modal rendering is handled in get_modal_content_for_page
                ctx.notify();
            }
            MCPServersSettingsPageEvent::HideModal => {
                // Modal rendering is handled in get_modal_content_for_page
                ctx.notify();
            }
        }
    }

    pub fn search_for_keybinding(&mut self, keybinding_name: &str, ctx: &mut ViewContext<Self>) {
        self.set_and_refresh_current_page(SettingsSection::Keybindings, ctx);

        if let Some(settings_page) = self.current_settings_page() {
            if let SettingsPageViewHandle::Keybindings(view_handle) = &settings_page.view_handle {
                view_handle.update(ctx, |view, ctx| {
                    view.search_for_binding(keybinding_name, ctx);
                })
            }
        }
    }

    fn handle_referrals_page_event(
        &mut self,
        event: &ReferralsPageEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            ReferralsPageEvent::SignupAnonymousUser => {
                ctx.emit(SettingsViewEvent::SignupAnonymousUser)
            }
            ReferralsPageEvent::FocusModal => ctx.focus(&self.search_editor),
            ReferralsPageEvent::ShowToast { message, flavor } => {
                ctx.emit(SettingsViewEvent::ShowToast {
                    message: message.clone(),
                    flavor: *flavor,
                })
            }
        }
    }

    fn handle_warp_drive_page_event(
        &mut self,
        event: &warp_drive_page::WarpDriveSettingsPageEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            warp_drive_page::WarpDriveSettingsPageEvent::SignUp => {
                ctx.emit(SettingsViewEvent::SignupAnonymousUser)
            }
        }
    }

    fn handle_ai_page_event(&mut self, event: &AISettingsPageEvent, ctx: &mut ViewContext<Self>) {
        match event {
            AISettingsPageEvent::FocusModal => ctx.focus(&self.search_editor),
            AISettingsPageEvent::OpenAIFactCollection => {
                ctx.emit(SettingsViewEvent::OpenAIFactCollection)
            }
            AISettingsPageEvent::OpenMCPServerCollection => {
                ctx.emit(SettingsViewEvent::OpenMCPServerCollection)
            }
            AISettingsPageEvent::OpenExecutionProfileEditor(profile_id) => {
                ctx.emit(SettingsViewEvent::OpenExecutionProfileEditor(*profile_id));
            }
            AISettingsPageEvent::SignupAnonymousUser => {
                ctx.emit(SettingsViewEvent::SignupAnonymousUser)
            }
            AISettingsPageEvent::ShowModal | AISettingsPageEvent::HideModal => {
                // Modal rendering is handled in get_modal_content_for_page
                ctx.notify();
            }
        }
    }

    fn handle_code_page_event(
        &mut self,
        event: &CodeSettingsPageEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            CodeSettingsPageEvent::SignupAnonymousUser => {
                ctx.emit(SettingsViewEvent::SignupAnonymousUser)
            }
            CodeSettingsPageEvent::OpenLspLogs { log_path } => {
                ctx.emit(SettingsViewEvent::OpenLspLogs {
                    log_path: log_path.clone(),
                });
            }
            CodeSettingsPageEvent::OpenProjectRules { rule_paths } => {
                ctx.emit(SettingsViewEvent::OpenProjectRulesPane {
                    rule_paths: rule_paths.clone(),
                });
            }
        }
    }

    pub fn current_settings_section(&self) -> SettingsSection {
        self.current_settings_page
    }

    fn current_settings_page(&self) -> Option<&SettingsPage> {
        // For AI subpages, the backing SettingsPage has section == AI.
        let lookup_section = self.current_settings_page.parent_page_section();
        self.settings_pages
            .iter()
            .find(|page| page.section == lookup_section)
    }

    fn settings_page(&self, section: SettingsSection) -> Option<&SettingsPage> {
        let settings_page = self
            .settings_pages
            .iter()
            .find(|page| page.section == section);
        if settings_page.is_none() {
            log::warn!("settings section {section:?} not found");
        }
        settings_page
    }

    pub fn set_and_refresh_current_page_internal(
        &mut self,
        section: SettingsSection,
        should_clear_query: bool,
        allow_steal_focus: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        // Map internal backing-page sections to their default subpage.
        // External callers should use subpage variants directly.
        let section = match section {
            SettingsSection::AI => SettingsSection::WarpAgent,
            SettingsSection::Code => SettingsSection::CodeIndexing,
            other => other,
        };

        // For AI subpages, the backing page is the AI page. Check it exists.
        let page_section = section.parent_page_section();
        if self.settings_page(page_section).is_none() {
            return;
        }
        let previous_section = self.current_settings_page;

        ctx.enable_key_bindings_dispatching();

        if let Some(current_page) = self.current_settings_page() {
            update_page!(
                &current_page.view_handle,
                |view, ctx| {
                    view.clear_highlighted_widget();
                    ctx.notify();
                },
                ctx
            );
        }

        if should_clear_query {
            self.clear_search_query(ctx);
        }
        self.current_settings_page = section;
        if previous_section != section && section == SettingsSection::CloudEnvironments {
            send_telemetry_from_ctx!(SettingsTelemetryEvent::EnvironmentsPageOpened, ctx);
        }

        // When navigating to a subpage, update the backing page's active subpage mode
        // and auto-expand the umbrella containing it.
        if section.is_subpage() {
            // AI subpages: update the AI page's subpage mode.
            if section.is_ai_subpage() && section != SettingsSection::AgentMCPServers {
                let subpage = AISubpage::from_section(section);
                self.ai_page_handle.update(ctx, |view, ctx| {
                    view.set_active_subpage(subpage, ctx);
                });
            }
            // Code subpages: update the Code page's subpage mode.
            if section.is_code_subpage() {
                let subpage = CodeSubpage::from_section(section);
                self.code_page_handle.update(ctx, |view, ctx| {
                    view.set_active_subpage(subpage, ctx);
                });
            }
            // Cloud platform subpages render their backing pages directly
            // (no subpage mode switch needed — the full page is shown).

            // Auto-expand the umbrella containing this subpage.
            for item in &mut self.nav_items {
                if let SettingsNavItem::Umbrella(umbrella) = item {
                    if umbrella.contains(section) {
                        umbrella.expanded = true;
                    }
                }
            }
        }

        #[cfg(feature = "crash_reporting")]
        {
            crate::crash_reporting::set_tag("warp.settings_page", section.to_string());
        }

        if let Some(settings_page) = self.current_settings_page() {
            update_page!(
                &settings_page.view_handle,
                |view, ctx| {
                    view.on_page_selected(allow_steal_focus, ctx);
                },
                ctx
            );
        }
        ctx.notify();
    }

    pub fn set_and_refresh_current_page(
        &mut self,
        section: SettingsSection,
        ctx: &mut ViewContext<Self>,
    ) {
        self.set_and_refresh_current_page_internal(section, true, true, ctx);
    }

    pub fn set_search_query(&mut self, query: &str, ctx: &mut ViewContext<Self>) {
        self.search_editor.update(ctx, |editor, ctx| {
            editor.set_buffer_text(query, ctx);
        });
    }

    fn should_render_page(&self, settings_page: &SettingsPage, app: &AppContext) -> bool {
        match &settings_page.view_handle {
            SettingsPageViewHandle::Main(v) => v.as_ref(app).should_render(app),
            SettingsPageViewHandle::Teams(v) => v.as_ref(app).should_render(app),
            SettingsPageViewHandle::SharedBlocks(v) => v.as_ref(app).should_render(app),
            SettingsPageViewHandle::Keybindings(v) => v.as_ref(app).should_render(app),
            SettingsPageViewHandle::Features(v) => v.as_ref(app).should_render(app),
            SettingsPageViewHandle::Appearance(v) => v.as_ref(app).should_render(app),
            SettingsPageViewHandle::BillingAndUsage(v) => v.as_ref(app).should_render(app),
            SettingsPageViewHandle::BillingAndUsageV2(v) => v.as_ref(app).should_render(app),
            SettingsPageViewHandle::About(v) => v.as_ref(app).should_render(app),
            SettingsPageViewHandle::OzCloudAPIKeys(v) => v.as_ref(app).should_render(app),
            SettingsPageViewHandle::Privacy(v) => v.as_ref(app).should_render(app),
            SettingsPageViewHandle::Warpify(v) => v.as_ref(app).should_render(app),
            SettingsPageViewHandle::Referrals(v) => v.as_ref(app).should_render(app),
            SettingsPageViewHandle::AI(v) => v.as_ref(app).should_render(app),
            SettingsPageViewHandle::CloudEnvironments(v) => v.as_ref(app).should_render(app),
            SettingsPageViewHandle::MCPServers(v) => v.as_ref(app).should_render(app),
            SettingsPageViewHandle::Code(v) => v.as_ref(app).should_render(app),
            SettingsPageViewHandle::WarpDrive(v) => v.as_ref(app).should_render(app),
        }
    }

    /// Open the invite section of the teams page, optionally with an email to invite.
    pub fn open_teams_page_email_invite(
        &mut self,
        email: Option<&String>,
        ctx: &mut ViewContext<Self>,
    ) {
        if let Some(team_page) = self.settings_page(SettingsSection::Teams) {
            if let SettingsPageViewHandle::Teams(view) = &team_page.view_handle {
                view.update(ctx, |view, ctx| {
                    view.open_team_members(email, ctx);
                })
            }
        }
    }

    /// Open the MCP servers page, optionally to list page or edit page.
    /// If `autoinstall_gallery_title` is provided, triggers auto-install of the specified gallery MCP.
    pub fn open_mcp_servers_page(
        &mut self,
        page: MCPServersSettingsPage,
        autoinstall_gallery_title: Option<&str>,
        ctx: &mut ViewContext<Self>,
    ) {
        // Navigate to the AgentMCPServers subpage (under the Agents umbrella).
        self.set_and_refresh_current_page(SettingsSection::AgentMCPServers, ctx);
        if let Some(mcp_page) = self.settings_page(SettingsSection::MCPServers) {
            if let SettingsPageViewHandle::MCPServers(view) = &mcp_page.view_handle {
                view.update(ctx, |view, ctx| {
                    view.update_page(page, ctx);
                    if let Some(title) = autoinstall_gallery_title {
                        view.autoinstall_from_gallery(title, ctx);
                    }
                })
            }
        }
    }

    /// Updates the PS1 prompt that is shown on the Appearance page.
    pub fn set_ps1_info(
        &mut self,
        ps1_grid_info: Option<(BlockGrid, SizeInfo)>,
        ctx: &mut ViewContext<Self>,
    ) {
        if let Some(appearance_page) = self.settings_page(SettingsSection::Appearance) {
            if let SettingsPageViewHandle::Appearance(view) = &appearance_page.view_handle {
                view.update(ctx, |view, ctx| {
                    view.set_ps1_info(ps1_grid_info, ctx);
                })
            }
        }
    }

    pub fn get_ps1_info(&self, app: &AppContext) -> Option<(BlockGrid, SizeInfo)> {
        self.settings_page(SettingsSection::Appearance)
            .and_then(|appearance_page| {
                if let SettingsPageViewHandle::Appearance(view) = &appearance_page.view_handle {
                    view.read(app, |view, _| view.get_ps1_info().map(ToOwned::to_owned))
                } else {
                    None
                }
            })
    }

    pub fn refresh_preferred_graphics_backend_dropdown(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(features_page) = self.settings_page(SettingsSection::Features) {
            if let SettingsPageViewHandle::Features(view) = &features_page.view_handle {
                view.update(ctx, |view, ctx| {
                    view.refresh_preferred_graphics_backend_dropdown(ctx);
                });
            }
        }
    }

    fn key_up(&mut self, ctx: &mut ViewContext<Self>) {
        self.cycle_pages(CycleDirection::Up, ctx)
    }

    fn key_down(&mut self, ctx: &mut ViewContext<Self>) {
        self.cycle_pages(CycleDirection::Down, ctx)
    }

    /// Predicate for whether `section` is currently visible in the sidebar
    /// under the active search filter. Mirrors the inline filtering used
    /// when rendering sidebar items so arrow-key navigation stays in sync
    /// with what the user can actually see.
    fn section_passes_search_filter(&self, section: SettingsSection) -> bool {
        if let Some(md) = self.subpage_filter.get(&section) {
            md.is_truthy()
        } else {
            let backing = section.parent_page_section();
            self.settings_pages
                .iter()
                .zip(self.pages_filter.iter())
                .any(|(p, md)| p.section == backing && md.is_truthy())
        }
    }

    fn cycle_pages(&mut self, direction: CycleDirection, ctx: &mut ViewContext<Self>) {
        let is_search_active = !self.search_editor.as_ref(ctx).buffer_text(ctx).is_empty();

        // Build nav stops from the current sidebar state. A collapsed umbrella
        // is represented as a single stop (rather than being skipped) so that
        // arrow-key navigation auto-expands it and selects its first visible
        // subpage instead of silently jumping over it.
        let stops = build_nav_stops(&self.nav_items, |section| {
            !is_search_active || self.section_passes_search_filter(section)
        });

        if stops.is_empty() {
            return;
        }

        let next_index =
            match current_stop_index(&stops, &self.nav_items, self.current_settings_page) {
                Some(idx) => next_stop_index(idx, stops.len(), direction),
                // Current page isn't in the visible nav order (e.g. it was
                // just filtered out); jump to the first visible stop.
                None => 0,
            };

        // Selecting a subpage auto-expands its umbrella in
        // set_and_refresh_current_page_internal, which is exactly the behavior
        // we want when landing on a `CollapsedUmbrella` stop. We pick the
        // entry subpage based on `direction` so that Up into a collapsed
        // umbrella lands on its last visible subpage (matching the reading
        // order the user was moving through) and Down lands on the first.
        let target_section = match stops[next_index] {
            NavStop::Section(section) => section,
            NavStop::CollapsedUmbrella {
                first_subpage,
                last_subpage,
                ..
            } => match direction {
                CycleDirection::Up => last_subpage,
                CycleDirection::Down => first_subpage,
            },
        };

        self.set_and_refresh_current_page_internal(target_section, false, false, ctx);
    }

    fn input_tab(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(current_page) = self.current_settings_page() {
            match &current_page.view_handle {
                SettingsPageViewHandle::Keybindings(view_handle) => {
                    view_handle.update(ctx, |view, ctx| view.on_tab_pressed(ctx));
                }
                SettingsPageViewHandle::Teams(view_handle) => {
                    view_handle.update(ctx, |view, ctx| view.on_tab_pressed(ctx));
                }
                _ => (),
            };
        }
    }

    pub fn scroll_to_settings_widget(
        &mut self,
        page: SettingsSection,
        widget_id: &'static str,
        ctx: &mut ViewContext<Self>,
    ) {
        self.set_and_refresh_current_page_internal(page, true, true, ctx);
        if let Some(current_page) = self.current_settings_page() {
            update_page!(
                &current_page.view_handle,
                |view, _| {
                    view.scroll_to_widget(widget_id);
                },
                ctx
            )
        }
    }

    fn debug_settings_action(&mut self, action: &DebugSettingsAction, ctx: &mut ViewContext<Self>) {
        match action {
            DebugSettingsAction::ToggleInitializationBlock => {
                BlockVisibilitySettings::handle(ctx).update(
                    ctx,
                    |block_visibility_settings, ctx| {
                        let _ = block_visibility_settings
                            .should_show_bootstrap_block
                            .toggle_and_save_value(ctx);
                    },
                );
            }
            DebugSettingsAction::ToggleInBandCommandBlocks => {
                BlockVisibilitySettings::handle(ctx).update(
                    ctx,
                    |block_visibility_settings, ctx| {
                        let _ = block_visibility_settings
                            .should_show_in_band_command_blocks
                            .toggle_and_save_value(ctx);
                    },
                );
            }
        }
    }

    fn get_modal_content_for_page(
        &self,
        page_handle: &SettingsPageViewHandle,
        app: &AppContext,
    ) -> Option<Box<dyn Element>> {
        match page_handle {
            SettingsPageViewHandle::BillingAndUsage(view) => {
                view.read(app, |view, _| view.get_modal_content())
            }
            SettingsPageViewHandle::BillingAndUsageV2(view) => {
                view.read(app, |view, _| view.get_modal_content())
            }
            SettingsPageViewHandle::Privacy(view) => {
                view.read(app, |view, _| view.get_modal_content())
            }
            SettingsPageViewHandle::OzCloudAPIKeys(view) => {
                view.read(app, |view, _| view.get_modal_content())
            }
            SettingsPageViewHandle::MCPServers(view) => {
                view.read(app, |view, _| view.get_modal_content(app))
            }
            SettingsPageViewHandle::AI(view) => {
                view.read(app, |view, _| view.get_modal_content(app))
            }
            _ => None,
        }
    }

    fn render_search_editor(&self, appearance: &Appearance) -> Box<dyn Element> {
        Container::new(
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(
                    Container::new(
                        ConstrainedBox::new(
                            icons::Icon::SearchSmall
                                .to_warpui_icon(appearance.theme().active_ui_text_color())
                                .finish(),
                        )
                        .with_width(16.)
                        .with_height(16.)
                        .finish(),
                    )
                    .with_uniform_margin(4.)
                    .with_margin_right(12.)
                    .finish(),
                )
                .with_child(
                    Shrinkable::new(
                        1.,
                        Clipped::new(ChildView::new(&self.search_editor).finish()).finish(),
                    )
                    .finish(),
                )
                .finish(),
        )
        .with_margin_left(16.)
        .with_margin_right(16.)
        .with_margin_bottom(8.)
        .finish()
    }

    fn render_search_zero_state(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        Container::new(
            Align::new(
                Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_children([
                        Text::new(
                            "No settings match your search.",
                            appearance.ui_font_family(),
                            appearance.ui_font_size(),
                        )
                        .with_style(Properties::default().weight(Weight::Medium))
                        .with_color(theme.sub_text_color(theme.background()).into_solid())
                        .finish(),
                        Text::new(
                            "You may want to try using different keywords or checking for any possible typos.",
                            appearance.ui_font_family(),
                            appearance.ui_font_size(),
                        )
                        .with_color(theme.disabled_ui_text_color().into_solid())
                        .finish(),
                    ])
                    .finish(),
            )
            .finish(),
        )
            .with_uniform_margin(16.)
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
        .with_background(internal_colors::fg_overlay_1(appearance.theme()))
        .finish()
    }
}

impl Entity for SettingsView {
    type Event = SettingsViewEvent;
}

impl View for SettingsView {
    fn ui_name() -> &'static str {
        "SettingsViewInTab"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let settings_pages = self.filtered_pages(app).collect_vec();
        let appearance = Appearance::as_ref(app);

        // For AI subpages, the backing SettingsPage has a different section
        // (e.g. Oz -> AI, AgentMCPServers -> MCPServers).
        let content_page_section = self.current_settings_page.parent_page_section();
        let (page, current_page_handle) = if settings_pages.is_empty() {
            (self.render_search_zero_state(appearance), None)
        } else {
            match settings_pages
                .iter()
                .find(|(page, _)| page.section == content_page_section)
            {
                None => (Empty::new().finish(), None),
                Some((page, _)) => (page.view_handle.child_view(), Some(&page.view_handle)),
            }
        };

        let theme = appearance.theme();

        let mut buttons = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(self.render_search_editor(appearance));

        // Render sidebar using nav_items (pages + umbrellas).
        for (nav_index, nav_item) in self.nav_items.iter().enumerate() {
            match nav_item {
                SettingsNavItem::Page(section) => {
                    let section = *section;
                    // Find the page in settings_pages for render/visibility check.
                    if let Some((page, match_data)) =
                        settings_pages.iter().find(|(p, _)| p.section == section)
                    {
                        let page_active = section == self.current_settings_page;
                        buttons.add_child(
                            page.render_page_button(appearance, *match_data, page_active)
                                .on_click(move |ctx, _, _| {
                                    ctx.dispatch_typed_action(SettingsAction::SelectAndRefresh(
                                        section,
                                    ));
                                })
                                .finish(),
                        );
                    }
                }
                SettingsNavItem::Umbrella(umbrella) => {
                    // Check which subpages are visible. Use per-subpage filter
                    // if available (search active), otherwise fall back to backing page.
                    let is_subpage_visible = |section: &SettingsSection| -> bool {
                        if let Some(md) = self.subpage_filter.get(section) {
                            return md.is_truthy();
                        }
                        let backing = section.parent_page_section();
                        settings_pages.iter().any(|(p, _)| p.section == backing)
                    };

                    let any_subpage_visible = umbrella.subpages.iter().any(is_subpage_visible);

                    // Only show the umbrella if at least one subpage is visible.
                    if !any_subpage_visible {
                        continue;
                    }

                    // Render umbrella header row. The whole row is a single
                    // Hoverable so hover styling + pointing-hand cursor apply
                    // across the full clickable area, not just the text.
                    buttons.add_child(
                        umbrella
                            .render_umbrella_row(appearance)
                            .on_click(move |ctx, _, _| {
                                ctx.dispatch_typed_action(SettingsAction::ToggleUmbrella(
                                    nav_index,
                                ));
                            })
                            .finish(),
                    );
                    // Render subpage items when expanded.
                    if umbrella.expanded {
                        for (sub_idx, subpage_section) in umbrella.subpages.iter().enumerate() {
                            let subpage_section = *subpage_section;
                            // Use per-subpage filter if available, otherwise backing page.
                            let match_data = self
                                .subpage_filter
                                .get(&subpage_section)
                                .copied()
                                .unwrap_or_else(|| {
                                    let backing = subpage_section.parent_page_section();
                                    settings_pages
                                        .iter()
                                        .find(|(p, _)| p.section == backing)
                                        .map(|(_, md)| *md)
                                        .unwrap_or(MatchData::Uncounted(false))
                                });

                            if !match_data.is_truthy() {
                                continue;
                            }

                            let is_active = subpage_section == self.current_settings_page;
                            if let Some(hoverable) = umbrella
                                .render_subpage_button(sub_idx, appearance, match_data, is_active)
                            {
                                buttons.add_child(
                                    hoverable
                                        .on_click(move |ctx, _, _| {
                                            ctx.dispatch_typed_action(
                                                SettingsAction::SelectAndRefresh(subpage_section),
                                            );
                                        })
                                        .finish(),
                                );
                            }
                        }
                    }
                }
            }
        }
        // Footer: "Open settings file" button, or an inline error alert if
        // the workspace-level banner was dismissed. Rendered below the
        // scrollable nav list but inside the same sidebar column so it
        // shares the right-border and SIDEBAR_WIDTH constraint.
        let footer_kind = SettingsFooterKind::choose(
            FeatureFlag::SettingsFile.is_enabled(),
            self.settings_file_error.is_some(),
            self.settings_error_banner_dismissed,
        );
        let footer = render_footer(
            footer_kind,
            appearance,
            self.settings_file_error.as_ref(),
            AISettings::as_ref(app).is_any_ai_enabled(app),
            &self.footer_mouse_states,
        );

        let scrollable = Container::new(
            ClippedScrollable::vertical(
                self.clipped_scroll_state.clone(),
                buttons.finish(),
                ScrollbarWidth::Auto,
                theme.nonactive_ui_detail().into(),
                theme.active_ui_detail().into(),
                Fill::None,
            )
            .finish(),
        )
        .with_padding_top(HEADER_PADDING)
        .finish();

        let sidebar = ConstrainedBox::new(
            Container::new(
                Flex::column()
                    .with_child(Expanded::new(1., scrollable).finish())
                    .with_child(footer)
                    .finish(),
            )
            .with_border(Border::right(SECTION_BORDER_WIDTH).with_border_fill(theme.outline()))
            .finish(),
        )
        .with_width(sidebar_width())
        .finish();

        let row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Max)
            .with_child(Shrinkable::new(1., sidebar).finish())
            .with_child(Shrinkable::new(1., page).finish())
            .finish();

        let mut stack = Stack::new().with_child(
            EventHandler::new(
                EventHandler::new(row)
                    .with_always_handle()
                    .on_left_mouse_down(|ctx, _app, _pos| {
                        ctx.dispatch_typed_action(SettingsAction::FocusSelf);
                        DispatchEventResult::PropagateToParent
                    })
                    .finish(),
            )
            .on_right_mouse_down(|event, _app, position| {
                let Some(parent_bounds) = event.element_position_by_id(POSITION_ID) else {
                    return DispatchEventResult::PropagateToParent;
                };
                let offset = position - parent_bounds.origin();
                event.dispatch_typed_action(SettingsAction::OpenContextMenu(offset));
                DispatchEventResult::StopPropagation
            })
            .finish(),
        );

        if let Some(position) = &self.context_menu_state {
            stack.add_positioned_overlay_child(
                ChildView::new(&self.context_menu).finish(),
                OffsetPositioning::offset_from_parent(
                    *position,
                    ParentOffsetBounds::WindowByPosition,
                    ParentAnchor::TopLeft,
                    ChildAnchor::TopLeft,
                ),
            );
        }

        if let Some(modal_content) =
            current_page_handle.and_then(|handle| self.get_modal_content_for_page(handle, app))
        {
            stack.add_positioned_overlay_child(
                modal_content,
                OffsetPositioning::offset_from_parent(
                    pathfinder_geometry::vector::vec2f(0., 0.),
                    ParentOffsetBounds::WindowByPosition,
                    ParentAnchor::Center,
                    ChildAnchor::Center,
                ),
            );
        }

        // Render environment setup mode selector overlay when open.
        if let Some(selector_handle) = self
            .environments_page_handle
            .as_ref(app)
            .environment_setup_mode_selector_handle()
        {
            stack.add_child(ChildView::new(selector_handle).finish());
        }

        // Render agent-assisted environment modal overlay when open.
        if let Some(modal_handle) = self
            .environments_page_handle
            .as_ref(app)
            .agent_assisted_environment_modal_handle(app)
        {
            stack.add_child(ChildView::new(modal_handle).finish());
        }

        SavePosition::new(stack.finish(), POSITION_ID).finish()
    }
}

impl TypedActionView for SettingsView {
    type Action = SettingsAction;

    fn handle_action(&mut self, action: &SettingsAction, ctx: &mut ViewContext<Self>) {
        match action {
            SettingsAction::SelectAndRefresh(section) => {
                self.set_and_refresh_current_page_internal(*section, false, true, ctx);

                if *section == SettingsSection::MCPServers {
                    send_telemetry_from_ctx!(
                        TelemetryEvent::MCPServerCollectionPaneOpened {
                            entrypoint: MCPServerCollectionPaneEntrypoint::MCPSettingsTab,
                        },
                        ctx
                    );
                }
            }
            SettingsAction::ToggleUmbrella(nav_index) => {
                if let Some(SettingsNavItem::Umbrella(umbrella)) =
                    self.nav_items.get_mut(*nav_index)
                {
                    umbrella.toggle();
                    ctx.notify();
                }
            }
            SettingsAction::MainPageToggle(main_page_action) => {
                if let Some(main_page) = self.settings_page(SettingsSection::Account) {
                    if let SettingsPageViewHandle::Main(view) = &main_page.view_handle {
                        view.update(ctx, |view, ctx| {
                            view.handle_action(main_page_action, ctx);
                        })
                    }
                }
            }
            SettingsAction::AppearancePageToggle(appearance_action) => {
                if let Some(appearance_page) = self.settings_page(SettingsSection::Appearance) {
                    if let SettingsPageViewHandle::Appearance(view) = &appearance_page.view_handle {
                        view.update(ctx, |view, ctx| {
                            view.handle_action(appearance_action, ctx);
                        })
                    }
                }
            }
            SettingsAction::FeaturesPageToggle(feature_action) => {
                if let Some(features_page) = self.settings_page(SettingsSection::Features) {
                    if let SettingsPageViewHandle::Features(view) = &features_page.view_handle {
                        view.update(ctx, |view, ctx| {
                            view.handle_action(feature_action, ctx);
                        })
                    }
                }
            }
            SettingsAction::PrivacyPageToggle(privacy_action) => {
                if let Some(privacy_page) = self.settings_page(SettingsSection::Privacy) {
                    if let SettingsPageViewHandle::Privacy(view) = &privacy_page.view_handle {
                        view.update(ctx, |view, ctx| {
                            view.handle_action(privacy_action, ctx);
                        })
                    }
                }
            }
            SettingsAction::AI(ai_action) => {
                if let Some(ai_page) = self.settings_page(SettingsSection::AI) {
                    if let SettingsPageViewHandle::AI(view) = &ai_page.view_handle {
                        view.update(ctx, |view, ctx| {
                            view.handle_action(ai_action, ctx);
                        })
                    }
                }
            }
            SettingsAction::Code(code_action) => {
                if let Some(code_page) = self.settings_page(SettingsSection::Code) {
                    if let SettingsPageViewHandle::Code(view) = &code_page.view_handle {
                        view.update(ctx, |view, ctx| {
                            view.handle_action(code_action, ctx);
                        })
                    }
                }
            }
            SettingsAction::WarpDrive(warp_drive_action) => {
                if let Some(warp_drive_page) = self.settings_page(SettingsSection::WarpDrive) {
                    if let SettingsPageViewHandle::WarpDrive(view) = &warp_drive_page.view_handle {
                        view.update(ctx, |view, ctx| {
                            view.handle_action(warp_drive_action, ctx);
                        })
                    }
                }
            }
            SettingsAction::WarpifyPageToggle(warpify_action) => {
                if let Some(warpify_page) = self.settings_page(SettingsSection::Warpify) {
                    if let SettingsPageViewHandle::Warpify(view) = &warpify_page.view_handle {
                        view.update(ctx, |view, ctx| {
                            view.handle_action(warpify_action, ctx);
                        })
                    }
                }
            }
            SettingsAction::Tab => self.input_tab(ctx),
            SettingsAction::Split(direction) => {
                let event = match direction {
                    Direction::Left => PaneEvent::SplitLeft(None),
                    Direction::Right => PaneEvent::SplitRight(None),
                    Direction::Up => PaneEvent::SplitUp(None),
                    Direction::Down => PaneEvent::SplitDown(None),
                };
                ctx.emit(SettingsViewEvent::Pane(event));
            }
            SettingsAction::ToggleMaximizePane => {
                ctx.emit(SettingsViewEvent::Pane(PaneEvent::ToggleMaximized))
            }
            SettingsAction::Close => ctx.emit(SettingsViewEvent::Pane(PaneEvent::Close)),
            SettingsAction::OpenContextMenu(position) => {
                self.context_menu_state = Some(*position);
                let menu_items = self.context_menu_items(ctx);
                self.context_menu.update(ctx, move |menu, ctx| {
                    menu.set_items(menu_items, ctx);
                    ctx.notify();
                });
                ctx.notify();
            }
            SettingsAction::FocusSelf => ctx.emit(SettingsViewEvent::Pane(PaneEvent::FocusSelf)),
            SettingsAction::Up => self.key_up(ctx),
            SettingsAction::Down => self.key_down(ctx),
            SettingsAction::Debug(action) => self.debug_settings_action(action, ctx),
        }
    }
}

impl BackingView for SettingsView {
    type PaneHeaderOverflowMenuAction = SettingsAction;
    type CustomAction = ();
    type AssociatedData = ();

    fn handle_pane_header_overflow_menu_action(
        &mut self,
        action: &Self::PaneHeaderOverflowMenuAction,
        ctx: &mut warpui::ViewContext<Self>,
    ) {
        self.handle_action(action, ctx)
    }

    fn close(&mut self, ctx: &mut warpui::ViewContext<Self>) {
        ctx.emit(SettingsViewEvent::Pane(PaneEvent::Close));
    }

    fn focus_contents(&mut self, ctx: &mut warpui::ViewContext<Self>) {
        ctx.focus(&self.search_editor)
    }

    fn render_header_content(
        &self,
        _ctx: &view::HeaderRenderContext<'_>,
        _app: &AppContext,
    ) -> view::HeaderContent {
        view::HeaderContent::simple("Settings")
    }

    fn set_focus_handle(&mut self, focus_handle: PaneFocusHandle, _ctx: &mut ViewContext<Self>) {
        self.focus_handle = Some(focus_handle);
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
