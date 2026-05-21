pub(crate) mod codex_modal;
#[cfg(enable_crash_recovery)]
mod crash_recovery;
pub mod global_search;
pub(crate) mod left_panel;
pub(crate) mod right_panel;
mod startup_directory;
#[cfg(test)]
#[path = "view_test.rs"]
mod tests;
mod vertical_tabs;
#[cfg(target_family = "wasm")]
mod wasm_view;

use self::vertical_tabs::{
    render_detail_sidecar, render_settings_popup, VerticalTabsPanelState,
    VERTICAL_TABS_SETTINGS_BUTTON_POSITION_ID,
};
use crate::ai::active_agent_views_model::ActiveAgentViewsModel;
use crate::ai::blocklist::agent_view::agent_input_footer::editor::AgentToolbarEditorMode;
use crate::ai::blocklist::agent_view::AgentViewEntryOrigin;
use crate::ai::conversation_utils;
use crate::ai::document::ai_document_model::{AIDocumentId, AIDocumentModel};
use crate::ai::llms::LLMPreferences;
use crate::ai::persisted_workspace::PersistedWorkspace;
use crate::ai::{
    agent::{conversation::AIConversationId, EntrypointType},
    blocklist::{inline_action::code_diff_view::CodeDiffView, SlashCommandRequest},
    facts::view::AIFactPage,
};
use crate::app_state::{
    LeafContents, LeafSnapshot, LeftPanelDisplayedTab, LeftPanelSnapshot, NotebookPaneSnapshot,
    PaneNodeSnapshot, PaneUuid, RightPanelSnapshot, SettingsPaneSnapshot, TabSnapshot,
    WindowSnapshot,
};
use crate::code_review::diff_state::DiffStateModel;
#[cfg(feature = "local_fs")]
use crate::code_review::GlobalCodeReviewModel;
use crate::coding_panel_enablement_state::CodingPanelEnablementState;
use crate::default_terminal::DefaultTerminal;
use crate::notification::NotificationContext;
use crate::pane_group::pane::ActionOrigin;
use crate::projects::ProjectManagementModel;
use crate::settings_view::mcp_servers_page::MCPServersSettingsPage;
use crate::terminal::model::terminal_model::ConversationTranscriptViewerStatus;
use crate::terminal::session_settings::SessionSettings;
use crate::terminal::view::inline_banner::ZeroStatePromptSuggestionType;
use crate::terminal::view::load_ai_conversation::{RestorationDirState, RestoredAIConversation};
use crate::terminal::view::ConversationRestorationInNewPaneType;
#[cfg(feature = "local_fs")]
use crate::util::file::external_editor::settings::OpenConversationPreference;
use crate::workspace::toast_stack::ToastStack;
use crate::workspace::view::global_search::view::GlobalSearchEntryFocus;
use crate::workspace::view::left_panel::{
    LeftPanelAction, LeftPanelEvent, LeftPanelView, ToolPanelView,
};
use crate::workspace::view::right_panel::{RightPanelEvent, RightPanelView};

use crate::ui_components::window_focus_dimming::WindowFocusDimming;
#[cfg(feature = "local_fs")]
use crate::util::file::external_editor::Editor;
#[cfg(feature = "local_fs")]
use crate::util::file::external_editor::EditorSettings;
use crate::util::openable_file_type::FileTarget;
#[cfg(feature = "local_fs")]
use crate::util::openable_file_type::{resolve_file_target_with_editor_choice, EditorLayout};

use crate::ai::blocklist::history_model::LocalConversationData;
use crate::ai::blocklist::FORK_PREFIX;
#[cfg(not(target_family = "wasm"))]
use crate::terminal::cli_agent_sessions::plugin_manager::{plugin_manager_for, PluginModalKind};
use crate::terminal::cli_agent_sessions::{CLIAgentSessionsModel, CLIAgentSessionsModelEvent};
use crate::workspace::header_toolbar_editor::{HeaderToolbarEditorEvent, HeaderToolbarEditorModal};
use crate::workspace::header_toolbar_item::HeaderToolbarItemKind;
use crate::workspace::tab_settings::TabCloseButtonPosition;
use crate::workspace::view::codex_modal::{CodexModal, CodexModalEvent};
use crate::workspace::{ForkFromExchange, ForkedConversationDestination};
use crate::BlocklistAIHistoryModel;
use ai::index::full_source_code_embedding::manager::CodebaseIndexManager;
use serde_json;
use warpui::notification::NotificationSendError;

use super::lightbox_view::{LightboxParams, LightboxView, LightboxViewEvent};
use super::util;
use super::WorkspaceRegistry;
use crate::ai::execution_profiles::editor::ExecutionProfileEditorManager;
use crate::ai::execution_profiles::profiles::{AIExecutionProfilesModel, ClientProfileId};
#[cfg(feature = "local_fs")]
use crate::code::editor_management::CodeManager;
use crate::code::editor_management::CodeSource;
use crate::launch_configs::launch_config::WindowTemplate;
use crate::pane_group::{
    CodeReviewPanelArg, Direction as PaneGroupDirection, ExecutionProfileEditorPane, PaneGroup,
    PaneId, TerminalPaneId,
};
use crate::quit_warning::UnsavedStateSummary;
use crate::search::command_palette::view::NavigationMode;
use crate::search::slash_command_menu::static_commands::commands;
use crate::settings::{
    AISettings, AISettingsChangedEvent, CodeSettings, CodeSettingsChangedEvent, CtrlTabBehavior,
    DefaultSessionMode, InputModeSettings,
};
use crate::settings_view::pane_manager::SettingsPaneManager;
use crate::settings_view::{SettingsSection, SettingsView, SettingsViewEvent};
#[cfg(all(target_os = "windows", feature = "local_tty"))]
use crate::shell_indicator::ShellIndicatorType;
use crate::terminal::available_shells::AvailableShell;
#[cfg(target_os = "windows")]
use crate::terminal::available_shells::AvailableShells;
use crate::terminal::block_list_viewport::InputMode;
use crate::terminal::ligature_settings::should_use_ligature_rendering;
use crate::ui_components::avatar::{Avatar, AvatarContent};

use crate::workflows::workflow::Workflow;
#[cfg(feature = "local_fs")]
use repo_metadata::RemoteRepositoryIdentifier;
#[cfg(target_family = "wasm")]
use url::Url;

#[cfg(target_family = "wasm")]
use crate::wasm_nux_dialog::WasmNUXDialog;

use crate::appearance::{Appearance, AppearanceManager};
use crate::banner::BannerState;
use crate::channel::Channel;
use crate::context_chips::ChipRuntimeCapabilities;
use crate::menu::{
    Event as MenuEvent, Menu, MenuItem, MenuItemFields, MenuSelectionSource,
    DEFAULT_WIDTH as MENU_DEFAULT_WIDTH,
};
use crate::modal::{Modal, ModalEvent, ModalViewState};
use crate::network::{NetworkStatus, NetworkStatusEvent};
#[cfg(feature = "local_fs")]
use crate::pane_group::FilePane;
use crate::pane_group::{
    self, AnyPaneContent, CodeDiffPane, CodePane, Direction, NewTerminalOptions, PanesLayout,
    TabBarHoverIndex,
};
use crate::terminal::keys_settings::KeysSettings;

use crate::ai::blocklist::agent_view::editor::{AgentToolbarEditorEvent, AgentToolbarEditorModal};
use crate::prompt::editor_modal::{
    EditorModal as PromptEditorModal, EditorModalEvent as PromptEditorModalEvent,
};
use crate::report_if_error;
use crate::resource_center::{
    mark_feature_used_and_write_to_user_defaults, skip_tips_and_write_to_user_defaults,
    ResourceCenterEvent, ResourceCenterPage, ResourceCenterView, Tip, TipAction, TipsCompleted,
};
use crate::root_view::{NewWorkspaceSource, OpenLaunchConfigArg};
use crate::search::command_search::searcher::{
    AcceptedHistoryItem, AcceptedWorkflow, CommandSearchItemAction,
};
use crate::search::command_search::view::{CommandSearchEvent, CommandSearchView};
use crate::session_management::{SessionNavigationData, SessionSource};
use crate::settings::{
    active_theme_kind, respect_system_theme, AccessibilitySettings, AliasExpansionSettings,
    AppEditorSettings, BlockVisibilitySettings, CursorBlink, DebugSettings, FontSettings,
    GPUSettings, InputSettings, MonospaceFontSize, PaneSettings, SelectionSettings, SshSettings,
    ThemeSettings,
};
use crate::settings_view::flags;
use crate::settings_view::keybindings::{KeybindingChangedEvent, KeybindingChangedNotifier};
use crate::terminal::alt_screen_reporting::AltScreenReporting;
use crate::terminal::general_settings::GeneralSettings;
use crate::terminal::input::{Input, MenuPositioning};
#[cfg(feature = "local_tty")]
use crate::terminal::local_tty::docker_sandbox::resolve_sbx_path_from_user_shell;
use crate::terminal::model::blockgrid::BlockGrid;
#[cfg(feature = "local_fs")]
use crate::terminal::model::session::Session;
use crate::terminal::model::session::SessionId;
use crate::terminal::resizable_data::{
    ModalSizes, ModalType, ResizableData, DEFAULT_LEFT_PANEL_WIDTH, DEFAULT_RIGHT_PANEL_WIDTH,
};
use crate::terminal::safe_mode_settings::SafeModeSettings;
use crate::terminal::session_settings::{
    NewSessionSource, NotificationsMode, NotificationsSettings, SessionSettingsChangedEvent,
    WorkingDirectoryMode,
};
use crate::terminal::settings::{SpacingMode, TerminalSettings};
use crate::terminal::shell::ShellType;
#[cfg(feature = "local_tty")]
use crate::terminal::view::docker_sandbox::DEFAULT_DOCKER_SANDBOX_BASE_IMAGE;
use crate::terminal::{self, SizeInfo, TerminalView};
#[cfg(target_os = "macos")]
use crate::workspace::cli_install;
#[cfg(all(target_os = "windows", feature = "local_tty"))]
use crate::workspace::metadata::AddTabWithShellSource;
use crate::workspace::metadata::LaunchConfigUiLocation;
use crate::workspaces::user_workspaces::UserWorkspaces;
use ::settings::{Setting, ToggleableSetting};
use warp_core::features::FeatureFlag;

use crate::search::{self, QueryFilter};
use crate::terminal::view::{
    SyncEvent, SyncInputType, TerminalAction, NOTIFICATIONS_TROUBLESHOOT_URL,
};
use crate::terminal::{BlockListSettings, TerminalModel};
use crate::themes::theme::{AnsiColorIdentifier, RespectSystemTheme, ThemeKind};
use crate::themes::theme_chooser::{ThemeChooser, ThemeChooserEvent, ThemeChooserMode};
use crate::themes::theme_creator_modal::{ThemeCreatorModal, ThemeCreatorModalEvent};
use crate::themes::theme_deletion_modal::{ThemeDeletionModal, ThemeDeletionModalEvent};
use crate::tips::{TipsEvent, TipsView};
#[cfg(target_family = "wasm")]
use crate::ui_components::blended_colors;
use crate::ui_components::buttons::{combo_inner_button, icon_button_with_color};
use crate::undo_close::UndoCloseStack;
#[cfg(feature = "local_fs")]
use crate::user_config::{
    ensure_default_worktree_config, find_unused_tab_config_path, find_unused_toml_path,
    find_unused_worktree_config_path, materialize_default_worktree_config, sanitize_toml_base_name,
    tab_configs_dir,
};
use crate::user_config::{WarpConfig, WarpConfigUpdateEvent};
use crate::util::bindings::{keybinding_name_to_display_string, keybinding_name_to_keystroke};
use crate::util::links;
use crate::util::traffic_lights::{traffic_light_data, TrafficLightMouseStates, TrafficLightSide};
use crate::util::truncation::truncate_from_end;
#[cfg(target_family = "wasm")]
use crate::view_components::action_button::ActionButton;
use crate::view_components::callout_bubble::{
    render_callout_bubble, CalloutArrowDirection, CalloutArrowPosition, CalloutBubbleConfig,
};
use crate::view_components::{DismissibleToast, DismissibleToastStack, ToastLink};
use crate::window_settings::{WindowSettings, WindowSettingsChangedEvent, ZoomLevel};
use crate::workflows::{AIWorkflowOrigin, WorkflowSelectionSource, WorkflowSource, WorkflowType};
use crate::workspace::action::CommandSearchOptions;
use crate::workspace::sync_inputs::SyncedInputState;
use crate::workspace::toast_stack::{
    ToastStack as WorkspaceToastStack, ToastStackEvent as WorkspaceToastStackEvent,
};
use crate::GlobalResourceHandles;

use itertools::Itertools;
use parking_lot::FairMutex;
use pathfinder_geometry::rect::RectF;
use repo_metadata::repositories::DetectedRepositories;
use std::collections::{HashMap, HashSet};
#[cfg(feature = "local_fs")]
use std::convert::TryFrom;
use std::time::Duration;
use warp_core::context_flag::ContextFlag;
use warp_core::semantic_selection::SemanticSelection;
use warp_util::path::{user_friendly_path, LineAndColumnArg};
use warpui::fonts::Weight;
use warpui::modals::{AlertDialogWithCallbacks, AppModalCallback};
use warpui::windowing::{StateEvent, WindowManager};

use warpui::clipboard::ClipboardContent;
#[cfg(target_family = "wasm")]
use warpui::elements::Percentage;
use warpui::elements::{
    CacheOption, DispatchEventResult, DropTarget, EventHandler, Image, MouseInBehavior, Rect,
};
use warpui::ui_components::button::Button;
use warpui::{elements::MouseStateHandle, fonts::Properties};

use crate::channel::ChannelState;

use crate::ai::blocklist::{BlocklistAIHistoryEvent, PendingQueryState, SerializedBlockListItem};
use crate::editor::{
    EditorView, Event as EditorEvent, PropagateAndNoOpNavigationKeys, SingleLineEditorOptions,
    TextOptions,
};
use crate::persistence::ModelEvent;

use super::action::{
    InitContent, RestoreConversationLayout, TabContextMenuAnchor,
    VerticalTabsPaneContextMenuTarget, WorkspaceAction,
};
use super::delete_conversation_confirmation_dialog::{
    DeleteConversationConfirmationDialog, DeleteConversationConfirmationEvent,
    DeleteConversationDialogSource,
};
use super::native_modal::{NativeModal, NativeModalEvent};
use super::rewind_confirmation_dialog::{
    RewindConfirmationDialog, RewindConfirmationEvent, RewindDialogSource,
};
use super::{ActiveSession, TabBarDropTargetData, TabBarLocation};

use super::tab_settings::{
    HeaderToolbarChipSelection, NewTabPlacement, TabSettings, TabSettingsChangedEvent,
    VerticalTabsDisplayGranularity, WorkspaceDecorationVisibility,
};
use super::util::{
    PaneViewLocator, TabMovement, TerminalSessionFallbackBehavior, WelcomeTipsViewState,
    WorkspaceMouseStates, WorkspaceState,
};
use crate::launch_configs::save_modal::{LaunchConfigModalEvent, LaunchConfigSaveModal};
use crate::tab_configs::action_sidecar::SidecarItemKind;
use crate::tab_configs::remove_confirmation_dialog::{
    RemoveTabConfigConfirmationDialog, RemoveTabConfigConfirmationEvent,
};
use crate::tab_configs::session_config_modal::{SessionConfigModal, SessionConfigModalEvent};
use crate::tab_configs::{
    NewWorktreeModal, NewWorktreeModalEvent, TabConfigParamsModal, TabConfigParamsModalEvent,
};

use crate::code::editor::{add_color, remove_color};
use crate::palette::PaletteMode;
use crate::search::command_palette::view::{Event as CommandPaletteEvent, View as CommandPalette};
use crate::tab::{
    tab_position_id, NewSessionMenuItem, PaneNameMenuTarget, SelectedTabColor, TabBarState,
    TabComponent, TabData, TAB_BAR_BORDER_HEIGHT,
};
use crate::terminal::view::ssh_file_upload::FileUploadId;
use crate::ui_components::icons;
use crate::workspace::metadata::PaletteSource;
use lazy_static::lazy_static;
use pathfinder_color::ColorU;
use std::fmt::Write;
use std::path::Path;
use std::path::PathBuf;
use std::sync::mpsc;
use std::{cmp::Ordering, sync::Arc};
use warp_core::ui::theme::{color::internal_colors, phenomenon::PhenomenonStyle, Fill};
use warp_core::ui::{color::coloru_with_opacity, Icon};
use warp_editor::editor::NavigationKey;
use warpui::keymap::Context;
use warpui::notification::{RequestPermissionsOutcome, UserNotification};
use warpui::platform::{
    Cursor, FilePickerConfiguration, FullscreenState, SystemTheme, TerminationMode,
};
use warpui::text_layout::ClipConfig;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::{
    accessibility::{
        AccessibilityContent, AccessibilityVerbosity, ActionAccessibilityContent, WarpA11yRole,
    },
    elements::{
        Align, Border, ChildAnchor, ChildView, Clipped, ConstrainedBox, Container, CornerRadius,
        CrossAxisAlignment, Dismiss, Element, Empty, Expanded, Fill as ElementFill, Flex,
        Highlight, Hoverable, Icon as WarpUiIcon, MainAxisAlignment, MainAxisSize,
        OffsetPositioning, ParentAnchor, ParentElement, ParentOffsetBounds,
        PositionedElementAnchor, PositionedElementOffsetBounds, Radius, SavePosition, Shrinkable,
        Stack, Text,
    },
    geometry::vector::{vec2f, Vector2F},
    AppContext, Entity, TypedActionView, UpdateView, View, ViewContext, ViewHandle,
};
use warpui::{
    EntityId, FocusContext, ModelHandle, SingletonEntity, UpdateModel, ViewAsRef, WeakViewHandle,
    WindowId,
};

/// The padding that should be applied to the workspace as a whole.
pub const WORKSPACE_PADDING: f32 = 1.0;

/// The minimum font size at which terminal text will be rendered.
const MIN_FONT_SIZE: f32 = 5.0;

/// The maximum font size at which terminal text will be rendered.
const MAX_FONT_SIZE: f32 = 25.0;

/// The increment for increasing/decreasing the font size.
const FONT_SIZE_INCREMENT: f32 = 1.0;

pub const TAB_BAR_HEIGHT: f32 = 34.;
/// Height for all panel headers (tab bar, resource center, theme chooser, etc.).
/// This ensures consistent header heights across all UI panels.
pub const PANEL_HEADER_HEIGHT: f32 = TAB_BAR_HEIGHT;
/// The hover area height for states where the tab bar is revealed on hover.
const TAB_BAR_HOVER_HEIGHT: f32 = 12.;
const TAB_BAR_PADDING_LEFT: f32 = 4.;
const TAB_BAR_PADDING_RIGHT: f32 = 8.;
const TITLE_BAR_SEARCH_BAR_MAX_WIDTH: f32 = 320.;
const TITLE_BAR_SEARCH_BAR_SLOT_PADDING: f32 = 8.;

// The total height taken up by the tab bar, including its bottom border.
pub const TOTAL_TAB_BAR_HEIGHT: f32 = TAB_BAR_HEIGHT + TAB_BAR_BORDER_HEIGHT;

const TAB_BAR_ICON_PADDING: f32 = 4.;

const TAB_BAR_OVERFLOW_MENU_WIDTH: f32 = 300.;

#[cfg(not(target_family = "wasm"))]
const RESOURCE_CENTER_WIDTH: f32 = 361.;

// Ratio of terminal : theme chooser when theme chooser is active
const THEME_CHOOSER_RATIO: f32 = 3.5;

/// Save position for the tab bar.
const TAB_BAR_POSITION_ID: &str = "workspace_view:tab_bar";

/// Save position for the vertical tabs panel.
const VERTICAL_TABS_PANEL_POSITION_ID: &str = "workspace_view:vertical_tabs_panel";

/// The main content area in a workspace. This is directly below the tab bar.
const TAB_CONTENT_POSITION_ID: &str = "workspace_view:tab_content";

const WELCOME_TIPS_POSITION_ID: &str = "welcome_tips_pill";
const ELLIPSE_SVG_PATH: &str = "bundled/svg/ellipse.svg";

const TOGGLE_RESOURCE_CENTER_KEYBINDING_NAME: &str = "workspace:toggle_resource_center";

/// Shared position ID for the new-session sidecar overlay. Used for both the
/// `SavePosition` wrapper and the safe-zone rect lookup.
const NEW_SESSION_SIDECAR_POSITION_ID: &str = "new_session_sidecar";
const NEW_SESSION_SIDECAR_WIDTH: f32 = 300.;
const NEW_SESSION_SIDECAR_SEARCH_BOX_HEIGHT: f32 = 32.;
const NEW_SESSION_SIDECAR_SEARCH_BOX_HORIZONTAL_PADDING: f32 = 12.;
const NEW_SESSION_SIDECAR_SEARCH_BOX_VERTICAL_PADDING: f32 = 6.;
const NEW_SESSION_SIDECAR_FOOTER_HORIZONTAL_PADDING: f32 = 16.;
const NEW_SESSION_SIDECAR_FOOTER_VERTICAL_PADDING: f32 = 8.;
const SESSION_CONFIG_TAB_CONFIG_CHIP_TEXT: &str = "Access your tab configs here.";
const SESSION_CONFIG_TAB_CONFIG_CHIP_WIDTH: f32 = 206.;
const SHOW_SETTINGS_KEYBINDING_NAME: &str = "workspace:show_settings";
pub const TOGGLE_COMMAND_PALETTE_KEYBINDING_NAME: &str = "workspace:toggle_command_palette";

const USER_AVATAR_BUTTON_POSITION_ID: &str = "workspace:user_avatar_button";

// these won't have to be public after we deprecate the code mode v1 project explorer which is defined in terminal
pub(crate) const TOGGLE_PROJECT_EXPLORER_BINDING_NAME: &str = "workspace:toggle_project_explorer";
pub(crate) const TOGGLE_RIGHT_PANEL_BINDING_NAME: &str = "workspace:toggle_right_panel";
pub(crate) const TOGGLE_VERTICAL_TABS_PANEL_BINDING_NAME: &str =
    "workspace:toggle_vertical_tabs_panel";
pub(crate) const OPEN_GLOBAL_SEARCH_BINDING_NAME: &str = "workspace:open_global_search";
pub(crate) const NEW_TAB_BINDING_NAME: &str = "workspace:new_tab";
pub(crate) const NEW_TERMINAL_TAB_BINDING_NAME: &str = "workspace:new_terminal_tab";
pub(crate) const NEW_AGENT_TAB_BINDING_NAME: &str = "workspace:new_agent_tab";
pub(crate) const TOGGLE_TAB_CONFIGS_MENU_BINDING_NAME: &str = "workspace:toggle_tab_configs_menu";

// Editable left panel toolbelt keybindings.
pub(crate) const LEFT_PANEL_PROJECT_EXPLORER_BINDING_NAME: &str =
    "workspace:left_panel_project_explorer";
pub(crate) const LEFT_PANEL_GLOBAL_SEARCH_BINDING_NAME: &str = "workspace:left_panel_global_search";
const KEYBINDINGS_TO_CACHE: [&str; 3] = [
    TOGGLE_RESOURCE_CENTER_KEYBINDING_NAME,
    SHOW_SETTINGS_KEYBINDING_NAME,
    TOGGLE_COMMAND_PALETTE_KEYBINDING_NAME,
];

#[cfg(target_family = "wasm")]
const MOBILE_OVERLAY_PANEL_WIDTH_RATIO: f32 = 0.9;
#[cfg(target_family = "wasm")]
const MOBILE_OVERLAY_SCRIM_ALPHA: u8 = 128;

pub const NEW_TAB_BUTTON_POSITION_ID: &str = "new_tab_button";
pub const NEW_SESSION_MENU_BUTTON_POSITION_ID: &str = "new_session_menu_button";

// The max length of the title of a fork toast (after which we truncate it).
const MAX_FORK_TOAST_TITLE_LENGTH: usize = 100;

// The max length of the window title (matching conversation title truncation).
const MAX_WINDOW_TITLE_LENGTH: usize = 80;

lazy_static! {
    static ref PANEL_CORNER_RADIUS: CornerRadius = CornerRadius::with_all(Radius::Pixels(8.));
    static ref PANEL_HEADER_CORNER_RADIUS: CornerRadius =
        CornerRadius::with_top(Radius::Pixels(8.));
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TabConfigsMenuOpenSource {
    KeyboardShortcut,
    Pointer,
}

/// This enumerates the different kinds of banners we show to the user.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum WorkspaceBanner {
    /// to display when recovering from a crash that may have been due to use
    /// of Wayland
    #[cfg(target_os = "linux")]
    WaylandCrashRecovery,
    /// to display when settings.toml has errors (parse failure or invalid values)
    InvalidSettings,
}

impl WorkspaceBanner {
    /// We want some banners to have a close button and not others, e.g. if they are running a very
    /// outdated version and we want to nag them to update.
    fn is_dismissible(&self) -> bool {
        match self {
            #[cfg(target_os = "linux")]
            Self::WaylandCrashRecovery => true,
            Self::InvalidSettings => true,
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
enum SessionCycleDirection {
    Next,
    Previous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PanePanelDirection {
    Prev,
    Next,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FocusRegion {
    LeftPanel,
    PaneGroup,
    RightPanel,
    Other,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PanelPosition {
    Left,
    Right,
}

pub struct TabPaneGroupIdentifiers {
    pub tab_idx: usize,
    pub pane_group_id: EntityId,
    pub terminal_ids: Vec<EntityId>,
}

/// Categorization of how the tab bar should be rendered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum ShowTabBar {
    /// Show the tab bar stacked on top of the pane group area.
    #[default]
    Stacked,
    /// Hide the tab bar.
    Hidden,
}

impl ShowTabBar {
    fn has_tab_bar(self) -> bool {
        matches!(self, ShowTabBar::Stacked)
    }
}

/// The type of content being displayed when the simplified WASM tab bar is shown.
/// Used to determine which elements to render (e.g., icon, info button).
#[cfg(target_family = "wasm")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SimplifiedWasmTabBarContent {
    /// Viewing a local conversation transcript.
    ConversationTranscript,
}

type RemoteUploadId = (TerminalPaneId, FileUploadId);
type WorkspaceMenuHandles = (
    ViewHandle<Menu<WorkspaceAction>>,
    ViewHandle<Menu<WorkspaceAction>>,
    ViewHandle<Menu<NewSessionSidecarSelection>>,
);

#[derive(Clone, Debug, PartialEq, Eq)]
enum NewSessionSidecarSelection {
    OpenWorktreeRepo { repo_path: String },
}

#[derive(Debug, Default)]
struct FileUploadSessions {
    /// Maps a local session pane handling a file upload
    /// to the remote session pane through which the upload was initiated.
    local_to_remote_map: HashMap<TerminalPaneId, TerminalPaneId>,
    /// Maps a local pane to the ID of the file upload it is responsible for.
    local_to_upload_id_map: HashMap<TerminalPaneId, RemoteUploadId>,
    upload_id_to_local_map: HashMap<RemoteUploadId, TerminalPaneId>,
}

/// Controls the color palette used for a workspace banner.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum BannerSeverity {
    /// Warning banners use an ansi-blended yellow background.
    Warning,
    /// Error banners use an ansi-blended red background.
    Error,
}

/// Visual style for an individual banner action button.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum BannerButtonVariant {
    /// No fill, no border, just text (and optional icon). Used for the primary
    /// action in warning banners.
    Naked,
    /// Border-only, no fill (e.g. "Open file").
    Outlined,
}

struct WorkspaceBannerButtonDetails {
    text: String,
    action: WorkspaceAction,
    variant: BannerButtonVariant,
    /// Optional leading icon shown before the label.
    icon: Option<Icon>,
    /// If set, renders an adjacent "More info" pill that dispatches this action.
    more_info_button_action: Option<WorkspaceAction>,
}

struct WorkspaceBannerFields {
    banner_type: WorkspaceBanner,
    severity: BannerSeverity,
    /// Optional bold heading rendered inline before the description.
    heading: Option<String>,
    /// Main description text (regular weight).
    description: String,
    secondary_button: Option<WorkspaceBannerButtonDetails>,
    button: Option<WorkspaceBannerButtonDetails>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DefaultSessionModeBehavior {
    /// Respect the user's default-session-mode setting and auto-enter agent view when applicable.
    Apply,
    /// Skip default-session-mode auto-entry because the caller is explicitly specifying the mode for the new session.
    Ignore,
}

#[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
struct CodeReviewPaneContext {
    repo_path: Option<PathBuf>,
    diff_state_model: ModelHandle<DiffStateModel>,
    terminal_view: WeakViewHandle<TerminalView>,
}

/// Parameters for updating the right panel's 'state.
#[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
struct RightPanelUpdateParams<'a> {
    pane_group: &'a ViewHandle<PaneGroup>,
    target_open_state: bool,
    review_pane_context: Option<&'a CodeReviewPaneContext>,
}

/// Context saved when the session config modal triggers `open_tab_config` and
/// the tab config has params (worktree). The params modal opens asynchronously,
/// so we store what we need to finish the tab replacement when it completes.
struct PendingSessionConfigReplacement {
    old_pane_group_id: EntityId,
}
pub struct TransferredTab {
    pub pane_group: ViewHandle<PaneGroup>,
    pub color: Option<AnsiColorIdentifier>,
    pub custom_title: Option<String>,
    pub left_panel_open: bool,
    pub vertical_tabs_panel_open: bool,
    pub right_panel_open: bool,
    pub is_right_panel_maximized: bool,
}

pub struct Workspace {
    window_id: WindowId,
    tabs: Vec<TabData>,
    active_tab_index: usize,
    hovered_tab_index: Option<TabBarHoverIndex>,
    tab_bar_hover_state: MouseStateHandle,
    tab_fixed_width: Option<f32>,
    traffic_light_mouse_states: TrafficLightMouseStates,
    tab_rename_editor: ViewHandle<EditorView>,
    pane_rename_editor: ViewHandle<EditorView>,
    vertical_tabs_search_input: ViewHandle<EditorView>,
    tips_completed: ModelHandle<TipsCompleted>,
    user_default_shell_unsupported_banner_model_handle: ModelHandle<BannerState>,
    tab_bar_overflow_menu: ViewHandle<Menu<WorkspaceAction>>,
    show_tab_bar_overflow_menu: bool,
    tab_right_click_menu: ViewHandle<Menu<WorkspaceAction>>,
    show_tab_right_click_menu: Option<(usize, TabContextMenuAnchor)>,
    // TODO(CORE-2300): this used to be add_tab_dropdown_menu.
    // Because we are rolling out the change behind a feature flag,
    // keep this comment here until the feature flag is removed.
    // Otherwise people might be confused as to why there is a right click
    // menu in the "new_session_dropdown_menu"
    // Same applies to "show_new_session_dropdown_menu"
    new_session_dropdown_menu: ViewHandle<Menu<WorkspaceAction>>,
    show_new_session_dropdown_menu: Option<Vector2F>,
    palette: ViewHandle<CommandPalette>,
    ctrl_tab_palette: ViewHandle<CommandPalette>,
    mouse_states: WorkspaceMouseStates,
    settings_pane: ViewHandle<SettingsView>,
    theme_chooser_view: ViewHandle<ThemeChooser>,
    previous_theme: Option<ThemeKind>,
    current_workspace_state: WorkspaceState,
    previous_workspace_state: Option<WorkspaceState>,
    welcome_tips_view_state: WelcomeTipsViewState,
    welcome_tips_view: ViewHandle<TipsView>,
    model_event_sender: Option<mpsc::SyncSender<ModelEvent>>,
    launch_config_save_modal: ModalViewState<LaunchConfigSaveModal>,
    tab_config_params_modal: ModalViewState<Modal<TabConfigParamsModal>>,
    session_config_modal: ModalViewState<Modal<SessionConfigModal>>,
    pending_session_config_replacement: Option<PendingSessionConfigReplacement>,
    pending_session_config_tab_config_chip: bool,
    show_session_config_tab_config_chip: bool,
    new_worktree_modal: ModalViewState<Modal<NewWorktreeModal>>,
    rewind_confirmation_dialog: ViewHandle<RewindConfirmationDialog>,
    delete_conversation_confirmation_dialog: ViewHandle<DeleteConversationConfirmationDialog>,
    resource_center_view: ViewHandle<ResourceCenterView>,
    command_search_view: ViewHandle<CommandSearchView>,
    settings_file_error: Option<crate::settings::SettingsFileError>,
    settings_error_banner_dismissed: bool,
    prompt_editor_modal: ViewHandle<PromptEditorModal>,
    agent_toolbar_editor_modal: ViewHandle<AgentToolbarEditorModal>,
    header_toolbar_editor_modal: ViewHandle<HeaderToolbarEditorModal>,
    header_toolbar_context_menu: ViewHandle<Menu<WorkspaceAction>>,
    show_header_toolbar_context_menu: Option<Vector2F>,
    theme_creator_modal: ViewHandle<ThemeCreatorModal>,
    theme_deletion_modal: ViewHandle<ThemeDeletionModal>,
    codex_modal: ViewHandle<CodexModal>,
    toast_stack: ViewHandle<DismissibleToastStack<WorkspaceAction>>,
    update_toast_stack: ViewHandle<DismissibleToastStack<WorkspaceAction>>,
    /// We need to render some dynamic keybindings for our tooltips. These cannot be looked up in the
    /// render method, so look them up when the view is constructed and cache them here. Note that they
    /// need to be kept in sync as the keybindings change.
    cached_keybindings: HashMap<String, Option<String>>,
    is_user_menu_open: bool,
    tab_bar_pinned_by_popup: bool,
    user_menu: ViewHandle<Menu<WorkspaceAction>>,
    native_modal: ViewHandle<NativeModal>,

    // When user's open WEB for the first time, we ask them to select a preference of
    // always opening in web or opening in native app.
    #[cfg(target_family = "wasm")]
    show_wasm_nux_dialog: bool,
    #[cfg(target_family = "wasm")]
    wasm_nux_dialog: ViewHandle<WasmNUXDialog>,
    #[cfg(target_family = "wasm")]
    open_in_warp_button: ViewHandle<ActionButton>,

    file_upload_sessions: FileUploadSessions,
    left_panel_open: bool,
    vertical_tabs_panel_open: bool,
    vertical_tabs_panel: VerticalTabsPanelState,
    left_panel_view: ViewHandle<LeftPanelView>,
    left_panel_views: Vec<ToolPanelView>,
    right_panel_view: ViewHandle<RightPanelView>,
    working_directories_model: ModelHandle<pane_group::WorkingDirectoriesModel>,
    lightbox_view: Option<ViewHandle<LightboxView>>,
    /// When true, this workspace was created to receive a transferred PaneGroup.
    /// The placeholder tab will be replaced when adopt_transferred_pane_group is called.
    pending_pane_group_transfer: bool,
    is_drag_preview_workspace: bool,
    /// Sidecar menu for submenu-parent items (Terminal, New worktree config) in the
    /// new-session dropdown. Shown as a positioned overlay next to the hovered
    /// parent item, following the model picker sidecar pattern.
    new_session_sidecar_menu: ViewHandle<Menu<NewSessionSidecarSelection>>,
    show_new_session_sidecar: bool,
    worktree_sidecar_active: bool,
    worktree_sidecar_search_editor: ViewHandle<EditorView>,
    worktree_sidecar_search_query: String,
    new_session_sidecar_add_repo_mouse_state: MouseStateHandle,
    tab_config_action_sidecar_item: Option<SidecarItemKind>,
    tab_config_action_sidecar_mouse_states: crate::tab_configs::action_sidecar::SidecarMouseStates,
    remove_tab_config_confirmation_dialog: ViewHandle<RemoveTabConfigConfirmationDialog>,
}

impl Workspace {
    pub fn is_drag_preview_workspace(&self) -> bool {
        self.is_drag_preview_workspace
    }

    fn tab_rename_editor_font_size(ctx: &AppContext, appearance: &Appearance) -> f32 {
        if FeatureFlag::VerticalTabs.is_enabled() && *TabSettings::as_ref(ctx).use_vertical_tabs {
            match *TabSettings::as_ref(ctx)
                .vertical_tabs_display_granularity
                .value()
            {
                VerticalTabsDisplayGranularity::Panes => 10.,
                VerticalTabsDisplayGranularity::Tabs => 12.,
            }
        } else {
            appearance.ui_font_size()
        }
    }

    /// Clears the worktree sidecar state and hides the sidecar.
    fn clear_worktree_sidecar_state(&mut self, ctx: &mut ViewContext<Self>) {
        self.show_new_session_sidecar = false;
        self.worktree_sidecar_active = false;
        self.worktree_sidecar_search_query.clear();
        self.worktree_sidecar_search_editor
            .update(ctx, |editor, ctx| {
                editor.clear_buffer(ctx);
            });
        self.new_session_sidecar_menu.update(ctx, |menu, view_ctx| {
            menu.clear_pinned_header_builder();
            menu.clear_pinned_footer_builder();
            menu.set_content_padding_overrides(None, None);
            menu.reset_selection(view_ctx);
        });
    }

    fn close_new_session_dropdown_menu(&mut self, ctx: &mut ViewContext<Self>) {
        self.show_new_session_dropdown_menu = None;
        self.tab_config_action_sidecar_item = None;
        self.clear_worktree_sidecar_state(ctx);
        self.new_session_dropdown_menu.update(ctx, |menu, _| {
            menu.set_safe_zone_target(None);
            menu.set_submenu_being_shown_for_item_index(None);
        });
        ctx.notify();
    }

    fn select_first_worktree_sidecar_repo(&mut self, ctx: &mut ViewContext<Self>) {
        self.new_session_sidecar_menu.update(ctx, |menu, view_ctx| {
            if menu.items_len() > 1 {
                menu.set_selected_by_index(1, view_ctx);
            } else {
                menu.reset_selection(view_ctx);
            }
        });
    }

    fn reset_worktree_sidecar_repo_selection(&mut self, ctx: &mut ViewContext<Self>) {
        self.new_session_sidecar_menu.update(ctx, |menu, view_ctx| {
            menu.reset_selection(view_ctx);
        });
    }

    fn navigate_worktree_sidecar_selection(
        &mut self,
        select_next: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        self.new_session_sidecar_menu.update(ctx, |menu, view_ctx| {
            let items_len = menu.items_len();
            if items_len <= 1 {
                return;
            }

            match menu.selected_index() {
                Some(_) if select_next => menu.select_next(view_ctx),
                Some(_) => menu.select_previous(view_ctx),
                None if select_next => menu.set_selected_by_index(1, view_ctx),
                None => menu.set_selected_by_index(items_len.saturating_sub(1), view_ctx),
            }
        });
    }

    fn confirm_worktree_sidecar_selection(&mut self, ctx: &mut ViewContext<Self>) {
        let selected_selection = self.new_session_sidecar_menu.update(ctx, |menu, view_ctx| {
            if menu.items_len() <= 1 {
                return None;
            }

            if menu.selected_index().is_none() {
                menu.set_selected_by_index(1, view_ctx);
            }

            menu.selected_item().and_then(|item| match item {
                MenuItem::Item(fields) => fields.on_select_action().cloned(),
                _ => None,
            })
        });

        if let Some(selection) = selected_selection {
            self.execute_new_session_sidecar_selection(selection, ctx);
            self.close_new_session_dropdown_menu(ctx);
        }
    }

    fn sync_new_session_sidecar_selection_to_hover(&mut self, ctx: &mut ViewContext<Self>) {
        self.new_session_sidecar_menu.update(ctx, |menu, view_ctx| {
            let Some(hovered_index) = menu.hovered_index() else {
                return;
            };
            let hovered_item_has_action = menu
                .items()
                .get(hovered_index)
                .and_then(MenuItem::item_on_select_action)
                .is_some();

            if hovered_item_has_action && menu.selected_index() != Some(hovered_index) {
                menu.set_selected_by_index(hovered_index, view_ctx);
            }
        });
    }

    fn build_worktree_sidecar_search_input(ctx: &mut ViewContext<Self>) -> ViewHandle<EditorView> {
        let editor = ctx.add_typed_action_view(|ctx| {
            let appearance = Appearance::as_ref(ctx);
            let mut editor = EditorView::single_line(
                SingleLineEditorOptions {
                    text: TextOptions::ui_text(Some(appearance.ui_font_size()), appearance),
                    select_all_on_focus: true,
                    clear_selections_on_blur: true,
                    propagate_and_no_op_vertical_navigation_keys:
                        PropagateAndNoOpNavigationKeys::Always,
                    ..Default::default()
                },
                ctx,
            );
            editor.set_placeholder_text("Search repos", ctx);
            editor
        });
        ctx.subscribe_to_view(&editor, |me, editor_view, event, ctx| match event {
            EditorEvent::Edited(_) => {
                me.worktree_sidecar_search_query = editor_view.as_ref(ctx).buffer_text(ctx);
                me.refresh_worktree_sidecar_if_active(ctx);
                ctx.notify();
            }
            EditorEvent::Escape => {
                me.close_new_session_dropdown_menu(ctx);
            }
            EditorEvent::Navigate(NavigationKey::Up) => {
                me.navigate_worktree_sidecar_selection(false, ctx);
            }
            EditorEvent::Navigate(NavigationKey::Down) => {
                me.navigate_worktree_sidecar_selection(true, ctx);
            }
            EditorEvent::Enter => {
                me.confirm_worktree_sidecar_selection(ctx);
            }
            _ => {}
        });
        editor
    }

    fn vertical_tabs_search_input(ctx: &mut ViewContext<Self>) -> ViewHandle<EditorView> {
        let editor = ctx.add_typed_action_view(|ctx| {
            let appearance = Appearance::as_ref(ctx);
            let options = SingleLineEditorOptions {
                text: TextOptions::ui_text(Some(12.), appearance),
                ..Default::default()
            };
            EditorView::single_line(options, ctx)
        });
        editor.update(ctx, |editor, ctx| {
            editor.set_placeholder_text("Search tabs...", ctx);
        });
        ctx.subscribe_to_view(&editor, |me, editor_view, event, ctx| match event {
            EditorEvent::Edited(_) => {
                me.vertical_tabs_panel.search_query = editor_view.as_ref(ctx).buffer_text(ctx);
                ctx.notify();
            }
            EditorEvent::Escape => {
                me.vertical_tabs_panel.search_query.clear();
                me.focus_active_tab(ctx);
            }
            _ => {}
        });
        editor
    }

    fn tab_rename_editor(ctx: &mut ViewContext<Self>) -> ViewHandle<EditorView> {
        let editor = {
            ctx.add_typed_action_view(|ctx| {
                let appearance = Appearance::as_ref(ctx);
                let options = SingleLineEditorOptions {
                    text: TextOptions::ui_text(
                        Some(Self::tab_rename_editor_font_size(ctx, appearance)),
                        appearance,
                    ),
                    ..Default::default()
                };
                EditorView::single_line(options, ctx)
            })
        };
        ctx.subscribe_to_view(&editor, move |me, _, event, ctx| {
            me.handle_tab_rename_editor_event(event, ctx);
        });
        editor
    }

    fn pane_rename_editor(ctx: &mut ViewContext<Self>) -> ViewHandle<EditorView> {
        let editor = ctx.add_typed_action_view(|ctx| {
            let appearance = Appearance::as_ref(ctx);
            let options = SingleLineEditorOptions {
                text: TextOptions::ui_text(Some(12.), appearance),
                ..Default::default()
            };
            EditorView::single_line(options, ctx)
        });
        ctx.subscribe_to_view(&editor, move |me, _, event, ctx| {
            me.handle_pane_rename_editor_event(event, ctx);
        });
        editor
    }

    pub fn handle_tab_rename_editor_event(
        &mut self,
        event: &EditorEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.current_workspace_state.is_tab_being_renamed() {
            match event {
                EditorEvent::Blurred | EditorEvent::Enter => {
                    self.finish_tab_rename(ctx);
                }
                EditorEvent::Escape => {
                    self.cancel_tab_rename(ctx);
                }
                _ => {}
            }
        }
    }

    pub fn handle_pane_rename_editor_event(
        &mut self,
        event: &EditorEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.current_workspace_state.is_any_pane_being_renamed() {
            match event {
                EditorEvent::Blurred | EditorEvent::Enter => {
                    self.finish_pane_rename(ctx);
                }
                EditorEvent::Escape => {
                    self.cancel_pane_rename(ctx);
                }
                _ => {}
            }
        }
    }

    fn finish_tab_rename(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(tab_index) = self.current_workspace_state.tab_being_renamed() {
            self.current_workspace_state.clear_tab_being_renamed();
            let title = self.tab_rename_editor.as_ref(ctx).buffer_text(ctx);
            let tab = &self.tabs[tab_index];
            tab.pane_group.update(ctx, |view, ctx| {
                // Only update the title if it was actually changed. Otherwise, lets assume
                // user's intend was to cancel the operation.
                if view.display_title(ctx) != title {
                    view.set_title(&title, ctx);
                }
            });
            self.clear_tab_name_editor(ctx);
            self.update_window_title(ctx);
            ctx.notify();
        }
    }

    fn finish_pane_rename(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(locator) = self.current_workspace_state.pane_being_renamed() else {
            return;
        };

        self.current_workspace_state.clear_pane_being_renamed();
        let title = self.pane_rename_editor.as_ref(ctx).buffer_text(ctx);
        self.set_custom_pane_name(locator, title, ctx);
        self.clear_pane_name_editor(ctx);
        self.focus_pane(locator, ctx);
        ctx.dispatch_global_action("workspace:save_app", ());
        ctx.notify();
    }

    fn cancel_tab_rename(&mut self, ctx: &mut ViewContext<Self>) {
        if self.current_workspace_state.is_tab_being_renamed() {
            self.current_workspace_state.clear_tab_being_renamed();
            self.clear_tab_name_editor(ctx);
            self.focus_active_tab(ctx);
            ctx.notify();
        }
    }

    fn cancel_pane_rename(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(locator) = self.current_workspace_state.pane_being_renamed() {
            self.current_workspace_state.clear_pane_being_renamed();
            self.clear_pane_name_editor(ctx);
            self.focus_pane(locator, ctx);
            ctx.notify();
        }
    }

    fn build_prompt_editor_modal(ctx: &mut ViewContext<Self>) -> ViewHandle<PromptEditorModal> {
        let modal = ctx.add_typed_action_view(PromptEditorModal::new);
        ctx.subscribe_to_view(&modal, |me, _, event, ctx| {
            me.handle_prompt_editor_modal_event(event, ctx);
        });
        modal
    }

    fn build_agent_toolbar_editor_modal(
        ctx: &mut ViewContext<Self>,
    ) -> ViewHandle<AgentToolbarEditorModal> {
        let modal = ctx.add_typed_action_view(AgentToolbarEditorModal::new);
        ctx.subscribe_to_view(&modal, |me, _, event, ctx| {
            me.handle_agent_toolbar_editor_modal_event(event, ctx);
        });
        modal
    }

    fn build_welcome_tips(
        tips_completed: ModelHandle<TipsCompleted>,
        ctx: &mut ViewContext<Self>,
    ) -> (ViewHandle<TipsView>, WelcomeTipsViewState) {
        let welcome_tips_view = ctx.add_typed_action_view(|ctx| {
            TipsView::new(tips_completed.clone(), WELCOME_TIPS_POSITION_ID.into(), ctx)
        });

        ctx.subscribe_to_view(&welcome_tips_view, move |me, _, event, ctx| {
            me.handle_welcome_tips_event(event, ctx);
        });
        let show_welcome_tips = !tips_completed.as_ref(ctx).skipped_or_completed;
        let welcome_tips_view_state = if show_welcome_tips {
            WelcomeTipsViewState::Available {
                is_popup_open: false,
            }
        } else {
            WelcomeTipsViewState::Unavailable
        };
        (welcome_tips_view, welcome_tips_view_state)
    }

    fn build_resource_center_view(
        ctx: &mut ViewContext<Self>,
        tips_completed: ModelHandle<TipsCompleted>,
    ) -> ViewHandle<ResourceCenterView> {
        let resource_center_view =
            ctx.add_typed_action_view(|ctx| ResourceCenterView::new(ctx, tips_completed.clone()));

        ctx.subscribe_to_view(&resource_center_view, |me, _, event, ctx| {
            me.handle_resource_center_event(event, ctx);
        });

        resource_center_view
    }

    fn build_settings_views(
        _global_resource_handles: GlobalResourceHandles,
        tips_completed: ModelHandle<TipsCompleted>,
        ctx: &mut ViewContext<Self>,
    ) -> (ViewHandle<SettingsView>, ViewHandle<ThemeChooser>) {
        let theme_chooser_view =
            ctx.add_typed_action_view(move |ctx| ThemeChooser::new(ctx, tips_completed));

        ctx.subscribe_to_view(&theme_chooser_view, |me, _, event, ctx| {
            me.handle_theme_chooser_event(event, ctx);
        });

        let settings_pane = ctx.add_typed_action_view(move |ctx| SettingsView::new(None, ctx));
        ctx.subscribe_to_view(&settings_pane, move |me, _, event, ctx| {
            me.handle_settings_pane_event(event, ctx);
        });

        let window_id = ctx.window_id();
        SettingsPaneManager::handle(ctx).update(ctx, |manager, _| {
            manager.register_view(window_id, settings_pane.clone());
        });

        (settings_pane, theme_chooser_view)
    }

    fn build_theme_creator_modal(ctx: &mut ViewContext<Self>) -> ViewHandle<ThemeCreatorModal> {
        let theme_creator_modal = ctx.add_typed_action_view(ThemeCreatorModal::new);
        ctx.subscribe_to_view(&theme_creator_modal, move |me, _, event, ctx| {
            me.handle_theme_creator_modal_event(event, ctx);
        });

        theme_creator_modal
    }

    fn build_theme_deletion_modal(ctx: &mut ViewContext<Self>) -> ViewHandle<ThemeDeletionModal> {
        let theme_deletion_modal = ctx.add_typed_action_view(ThemeDeletionModal::new);
        ctx.subscribe_to_view(&theme_deletion_modal, move |me, _, event, ctx| {
            me.handle_theme_deletion_modal_event(event, ctx);
        });

        theme_deletion_modal
    }

    fn build_rewind_confirmation_dialog(
        ctx: &mut ViewContext<Self>,
    ) -> ViewHandle<RewindConfirmationDialog> {
        let rewind_confirmation_dialog =
            ctx.add_typed_action_view(|_| RewindConfirmationDialog::new());
        ctx.subscribe_to_view(&rewind_confirmation_dialog, move |me, _, event, ctx| {
            me.handle_rewind_confirmation_dialog_event(event, ctx);
        });

        rewind_confirmation_dialog
    }

    fn build_delete_conversation_confirmation_dialog(
        ctx: &mut ViewContext<Self>,
    ) -> ViewHandle<DeleteConversationConfirmationDialog> {
        let delete_conversation_confirmation_dialog =
            ctx.add_typed_action_view(DeleteConversationConfirmationDialog::new);
        ctx.subscribe_to_view(
            &delete_conversation_confirmation_dialog,
            move |me, _, event, ctx| {
                me.handle_delete_conversation_confirmation_dialog_event(event, ctx);
            },
        );

        delete_conversation_confirmation_dialog
    }

    fn build_native_modal_view(ctx: &mut ViewContext<Self>) -> ViewHandle<NativeModal> {
        let native_modal = ctx.add_typed_action_view(NativeModal::new);
        ctx.subscribe_to_view(&native_modal, move |me, _, event, ctx| {
            me.handle_native_modal_event(event, ctx);
        });
        native_modal
    }

    fn build_tab_bar_overflow_menu(
        ctx: &mut ViewContext<Self>,
    ) -> ViewHandle<Menu<WorkspaceAction>> {
        let tab_bar_overflow_menu = ctx.add_typed_action_view(|_| {
            Menu::new()
                .with_width(TAB_BAR_OVERFLOW_MENU_WIDTH)
                .with_drop_shadow()
        });
        ctx.subscribe_to_view(&tab_bar_overflow_menu, move |me, _, event, ctx| {
            me.handle_tab_bar_overflow_menu_event(event, ctx);
        });
        tab_bar_overflow_menu
    }

    fn build_menus(ctx: &mut ViewContext<Self>) -> WorkspaceMenuHandles {
        let tab_right_click_menu = ctx.add_typed_action_view(|_| Menu::new());
        ctx.subscribe_to_view(&tab_right_click_menu, move |me, _, event, ctx| {
            me.handle_tab_right_click_menu_event(event, ctx);
        });

        // Currently setting the width to 300 px as a middle ground that looks
        // ok when the shells show the path to the executables, and when they
        // don't. Going forward we may want to enhance the menu to allow for a
        // `max_width` and `min_width` instead, so we can allow the menu to
        // grow as needed.
        const NEW_SESSION_MENU_WIDTH: f32 = 300.;
        let new_session_menu = ctx.add_typed_action_view(|ctx| {
            if FeatureFlag::ShellSelector.is_enabled() {
                let theme = Appearance::as_ref(ctx).theme();
                Menu::new()
                    .with_width(NEW_SESSION_MENU_WIDTH)
                    .with_border(Border::all(1.).with_border_color(theme.outline().into()))
                    .with_drop_shadow()
                    .with_safe_triangle()
                    .with_ignore_hover_when_covered()
                    .prevent_interaction_with_other_elements()
            } else {
                Menu::new()
                    .with_safe_triangle()
                    .with_ignore_hover_when_covered()
            }
        });
        ctx.subscribe_to_view(&new_session_menu, move |me, _, event, ctx| {
            me.handle_new_session_menu_event(event, ctx);
        });

        let new_session_sidecar = ctx.add_typed_action_view(|_ctx| {
            let mut menu = Menu::new()
                .without_item_action_dispatch()
                .with_width(NEW_SESSION_SIDECAR_WIDTH)
                .with_menu_variant(crate::menu::MenuVariant::scrollable());
            menu.set_height(400.);
            menu
        });
        ctx.subscribe_to_view(&new_session_sidecar, move |me, _, event, ctx| {
            me.handle_new_session_sidecar_event(event, ctx);
        });

        (tab_right_click_menu, new_session_menu, new_session_sidecar)
    }

    fn build_launch_config_save_modal(
        ctx: &mut ViewContext<Self>,
    ) -> ModalViewState<LaunchConfigSaveModal> {
        let launch_config_save_modal = ctx.add_typed_action_view(LaunchConfigSaveModal::new);
        ctx.subscribe_to_view(&launch_config_save_modal, move |me, _, event, ctx| {
            me.handle_launch_config_save_modal_event(event, ctx);
        });

        ModalViewState::new(launch_config_save_modal)
    }

    fn build_tab_config_params_modal(
        ctx: &mut ViewContext<Self>,
    ) -> ModalViewState<Modal<TabConfigParamsModal>> {
        let body = ctx.add_typed_action_view(TabConfigParamsModal::new);
        // Subscribe to body events before moving `body` into the Modal closure.
        ctx.subscribe_to_view(&body, |me, _, event, ctx| {
            me.handle_tab_config_params_modal_body_event(event, ctx);
        });
        let modal = ctx.add_typed_action_view(|ctx| {
            Modal::new(None, body, ctx)
                .with_modal_style(UiComponentStyles {
                    width: Some(460.),
                    height: Some(480.),
                    ..Default::default()
                })
                .with_body_style(UiComponentStyles {
                    padding: Some(Coords::uniform(0.)),
                    height: Some(480.),
                    background: Some(ElementFill::None),
                    ..Default::default()
                })
                .with_dismiss_on_click()
        });
        ctx.subscribe_to_view(&modal, |me, _, event, ctx| {
            me.handle_tab_config_params_modal_event(event, ctx);
        });
        ModalViewState::new(modal)
    }

    fn build_session_config_modal(
        ctx: &mut ViewContext<Self>,
    ) -> ModalViewState<Modal<SessionConfigModal>> {
        let body = ctx.add_typed_action_view(SessionConfigModal::new);
        ctx.subscribe_to_view(&body, |me, _, event, ctx| {
            me.handle_session_config_modal_event(event, ctx);
        });
        let modal = ctx.add_typed_action_view(|ctx| {
            Modal::new(None, body, ctx)
                .close_modal_button_disabled()
                .with_modal_style(UiComponentStyles {
                    width: Some(424.),
                    ..Default::default()
                })
                .with_background_opacity(0)
                .with_body_style(UiComponentStyles {
                    padding: Some(Coords::uniform(0.)),
                    ..Default::default()
                })
                .with_header_style(UiComponentStyles {
                    height: Some(0.),
                    padding: Some(Coords::uniform(0.)),
                    ..Default::default()
                })
        });
        ctx.subscribe_to_view(&modal, |me, _, event, ctx| {
            if matches!(event, ModalEvent::Close) {
                me.close_session_config_modal(ctx);
            }
        });
        ModalViewState::new(modal)
    }

    fn build_new_worktree_modal(
        ctx: &mut ViewContext<Self>,
    ) -> ModalViewState<Modal<NewWorktreeModal>> {
        let body = ctx.add_typed_action_view(NewWorktreeModal::new);
        ctx.subscribe_to_view(&body, |me, _, event, ctx| {
            me.handle_new_worktree_modal_body_event(event, ctx);
        });
        let modal = ctx.add_typed_action_view(|ctx| {
            // We intentionally pass `None` for the title so the Modal renders
            // no built-in header — the body view renders its own header to
            // match the Figma mock exactly (bold title + X close + ESC badge).
            Modal::new(None, body, ctx)
                .with_modal_style(UiComponentStyles {
                    width: Some(460.),
                    height: Some(480.),
                    ..Default::default()
                })
                .with_body_style(UiComponentStyles {
                    padding: Some(Coords {
                        top: 0.,
                        bottom: 0.,
                        left: 0.,
                        right: 0.,
                    }),
                    height: Some(480.),
                    background: Some(ElementFill::None),
                    ..Default::default()
                })
        });
        ctx.subscribe_to_view(&modal, |me, _, event, ctx| {
            me.handle_new_worktree_modal_event(event, ctx);
        });
        ModalViewState::new(modal)
    }

    fn build_remove_tab_config_confirmation_dialog(
        ctx: &mut ViewContext<Self>,
    ) -> ViewHandle<RemoveTabConfigConfirmationDialog> {
        let dialog = ctx.add_typed_action_view(RemoveTabConfigConfirmationDialog::new);
        ctx.subscribe_to_view(&dialog, |me, _, event, ctx| {
            me.handle_remove_tab_config_confirmation_event(event, ctx);
        });
        dialog
    }

    #[cfg(feature = "local_fs")]
    fn handle_remove_tab_config_confirmation_event(
        &mut self,
        event: &RemoveTabConfigConfirmationEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            RemoveTabConfigConfirmationEvent::Confirm { path } => {
                // If the removed config was the default, revert to Terminal.
                let ai_settings = AISettings::as_ref(ctx);
                let is_removed_default = ai_settings.default_session_mode(ctx)
                    == DefaultSessionMode::TabConfig
                    && ai_settings.default_tab_config_path() == path.to_string_lossy();
                if is_removed_default {
                    AISettings::handle(ctx).update(ctx, |settings, ctx| {
                        report_if_error!(settings
                            .default_session_mode_internal
                            .set_value(DefaultSessionMode::Terminal, ctx));
                        report_if_error!(settings
                            .default_tab_config_path
                            .set_value(String::new(), ctx));
                    });
                }
                if let Err(e) = std::fs::remove_file(path) {
                    log::warn!("Failed to remove tab config file: {e:?}");
                    self.toast_stack.update(ctx, |toast_stack, ctx| {
                        toast_stack.add_ephemeral_toast(
                            DismissibleToast::error(format!("Failed to remove tab config: {e}")),
                            ctx,
                        );
                    });
                } else {
                    WarpConfig::handle(ctx).update(ctx, |warp_config, ctx| {
                        warp_config.remove_tab_config_by_path(path, ctx);
                    });
                }
                self.current_workspace_state
                    .is_remove_tab_config_dialog_open = false;
                ctx.notify();
            }
            RemoveTabConfigConfirmationEvent::Cancel => {
                self.current_workspace_state
                    .is_remove_tab_config_dialog_open = false;
                ctx.notify();
            }
        }
    }

    #[cfg(not(feature = "local_fs"))]
    fn handle_remove_tab_config_confirmation_event(
        &mut self,
        _event: &RemoveTabConfigConfirmationEvent,
        _ctx: &mut ViewContext<Self>,
    ) {
        log::error!("Cannot delete a tab config from the web");
    }

    fn handle_session_config_modal_event(
        &mut self,
        event: &SessionConfigModalEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            SessionConfigModalEvent::Completed(selection) => {
                self.close_session_config_modal(ctx);
                self.handle_session_config_completed(selection, ctx);

                // Show the chip only when no params modal followed.
                if !self.current_workspace_state.is_tab_config_params_modal_open {
                    self.promote_session_config_tab_config_chip(ctx);
                }
            }
            SessionConfigModalEvent::Dismissed => {
                // No tab config was created, so don't show the chip.
                self.pending_session_config_tab_config_chip = false;
                self.close_session_config_modal(ctx);
            }
        }
    }

    #[cfg(feature = "local_fs")]
    fn handle_session_config_completed(
        &mut self,
        selection: &crate::tab_configs::session_config::SessionConfigSelection,
        ctx: &mut ViewContext<Self>,
    ) {
        use crate::tab_configs::session_config::{build_tab_config, write_tab_config};

        // Build a TabConfig.
        let config = build_tab_config(
            &selection.session_type,
            &selection.directory,
            selection.enable_worktree,
            selection.autogenerate_worktree_branch_name,
        );

        let old_pane_group_id = self.active_tab_pane_group().id();
        let has_params = !config.params.is_empty();

        // Save and open the tab config. The user's `default_session_mode`
        // is intentionally left untouched: creating a tab config should not
        // change the global default for new tabs.
        // Agent view entry for the built-in agent is handled by PaneMode::Agent in the tab config,
        // so no manual enter_agent_view call is needed.
        let dir = crate::user_config::tab_configs_dir();
        if let Err(e) = write_tab_config(&config, &dir, "startup_config") {
            log::warn!("Failed to write startup tab config: {e:?}");
        }

        if has_params {
            // When the config has params (worktree), open_tab_config shows the
            // params modal instead of creating the tab immediately.
            // Store the replacement context so we can finish when the modal completes.
            self.pending_session_config_replacement =
                Some(PendingSessionConfigReplacement { old_pane_group_id });
            self.open_tab_config(config, ctx);
        } else {
            let worktree_branch_name = self.maybe_generate_worktree_name(&config);
            let param_values = config.default_param_values();
            self.open_tab_config_with_params(
                config,
                param_values,
                worktree_branch_name.as_deref(),
                ctx,
            );
            self.remove_tab_by_pane_group_id(old_pane_group_id, ctx);
        }
    }

    #[cfg(not(feature = "local_fs"))]
    fn handle_session_config_completed(
        &mut self,
        _selection: &crate::tab_configs::session_config::SessionConfigSelection,
        _ctx: &mut ViewContext<Self>,
    ) {
    }

    pub(crate) fn show_session_config_modal(&mut self, ctx: &mut ViewContext<Self>) {
        // Configure the modal to hide the built-in agent when AI is disabled.
        let show_agent = AISettings::as_ref(ctx).is_any_ai_enabled(ctx);
        self.session_config_modal.view.update(ctx, |modal, ctx| {
            modal.body().update(ctx, |body, ctx| {
                body.configure(show_agent);
                ctx.notify();
            });
        });

        self.session_config_modal.open();
        self.current_workspace_state.is_session_config_modal_open = true;
        self.pending_session_config_tab_config_chip = false;
        self.show_session_config_tab_config_chip = false;
        ctx.focus(&self.session_config_modal.view);
        ctx.notify();
    }

    fn close_session_config_modal(&mut self, ctx: &mut ViewContext<Self>) {
        self.session_config_modal.close();
        self.current_workspace_state.is_session_config_modal_open = false;
        // Don't promote pending → show here. The caller is responsible for
        // calling `promote_session_config_tab_config_chip` once all
        // intermediate modals (e.g. params modal) have closed.
        self.focus_active_tab(ctx);
        ctx.notify();
    }

    /// Promotes the pending tab-config chip to visible. This must be called
    /// only after **all** intermediate modals (session config modal, params
    /// modal) are closed. The chip is non-blocking: the user can still
    /// interact with the terminal and must click the chip's close button or
    /// press Escape/Enter to dismiss it.
    fn promote_session_config_tab_config_chip(&mut self, ctx: &mut ViewContext<Self>) {
        if self.pending_session_config_tab_config_chip {
            self.show_session_config_tab_config_chip = true;
            self.pending_session_config_tab_config_chip = false;
            ctx.notify();
        }
    }

    fn should_show_session_config_tab_config_chip(&self) -> bool {
        self.show_session_config_tab_config_chip
            && !self.current_workspace_state.is_session_config_modal_open
            && !self.current_workspace_state.is_tab_config_params_modal_open
    }

    fn dismiss_session_config_tab_config_chip(&mut self, ctx: &mut ViewContext<Self>) {
        self.pending_session_config_tab_config_chip = false;
        self.show_session_config_tab_config_chip = false;
        ctx.notify();
    }

    fn render_session_config_tab_config_chip(
        &self,
        use_vertical: bool,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let close_button = Hoverable::new(
            self.mouse_states
                .session_config_tab_config_chip_close
                .clone(),
            |hover_state| {
                let icon = ConstrainedBox::new(
                    icons::Icon::X
                        .to_warpui_icon(Fill::Solid(PhenomenonStyle::modal_close_button_text()))
                        .finish(),
                )
                .with_width(16.)
                .with_height(16.)
                .finish();

                let mut button = Container::new(icon)
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)));
                if hover_state.is_hovered() {
                    button =
                        button.with_background_color(PhenomenonStyle::modal_close_button_hover());
                }
                button.finish()
            },
        )
        .with_cursor(Cursor::PointingHand)
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action(WorkspaceAction::DismissSessionConfigTabConfigChip);
        })
        .finish();

        let text = Text::new_inline(
            SESSION_CONFIG_TAB_CONFIG_CHIP_TEXT.to_string(),
            appearance.ui_font_family(),
            12.,
        )
        .with_color(PhenomenonStyle::body_text())
        .with_selectable(false)
        .finish();

        let content = Flex::row()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(4.)
            .with_child(text)
            .with_child(close_button)
            .finish();
        let chip_content = Container::new(content)
            .with_padding_left(16.)
            .with_padding_right(12.)
            .with_padding_top(12.)
            .with_padding_bottom(12.)
            .finish();

        let (arrow_direction, arrow_position) = if use_vertical {
            (CalloutArrowDirection::Left, CalloutArrowPosition::Center)
        } else {
            (CalloutArrowDirection::Up, CalloutArrowPosition::Center)
        };

        render_callout_bubble(
            chip_content,
            &CalloutBubbleConfig {
                width: SESSION_CONFIG_TAB_CONFIG_CHIP_WIDTH,
                arrow_direction,
                arrow_position,
            },
            appearance,
        )
    }
    fn subscribe_to_workspace_toast_stack(
        toast_stack: ViewHandle<DismissibleToastStack<WorkspaceAction>>,
        ctx: &mut ViewContext<Self>,
    ) {
        let workspace_toast_stack = WorkspaceToastStack::handle(ctx);
        ctx.subscribe_to_model(
            &workspace_toast_stack,
            move |_me, _, event, ctx| match event {
                WorkspaceToastStackEvent::AddEphemeralToast { window_id, toast }
                    if *window_id == ctx.window_id() =>
                {
                    toast_stack.update(ctx, |toast_stack, ctx| {
                        toast_stack.add_ephemeral_toast(toast.clone(), ctx)
                    });
                }
                WorkspaceToastStackEvent::AddPersistentToast { window_id, toast }
                    if *window_id == ctx.window_id() =>
                {
                    toast_stack.update(ctx, |toast_stack, ctx| {
                        toast_stack.add_persistent_toast(toast.clone(), ctx)
                    });
                }
                WorkspaceToastStackEvent::RemoveToast {
                    window_id,
                    identifier,
                } if *window_id == ctx.window_id() => {
                    toast_stack.update(ctx, |toast_stack, ctx| {
                        toast_stack.dismiss_older_toasts(identifier, ctx)
                    });
                }
                _ => {}
            },
        );
    }

    /// Subscribes to `WarpConfigUpdateEvent::TabConfigErrors` and shows a persistent
    /// error toast for each tab config file that failed to parse.  Uses `object_id`
    /// keyed by file path so that re-saving the same file auto-dismisses the stale
    /// toast.
    fn subscribe_to_tab_config_errors(
        toast_stack: ViewHandle<DismissibleToastStack<WorkspaceAction>>,
        ctx: &mut ViewContext<Self>,
    ) {
        ctx.subscribe_to_model(&WarpConfig::handle(ctx), move |_me, _, event, ctx| {
            match event {
                WarpConfigUpdateEvent::TabConfigs => {
                    // On every tab config reload, dismiss error toasts for
                    // files that now parse successfully.  The model has already
                    // been updated with the current error set before this event
                    // fires, so we just need to clear stale toasts.
                    //
                    // `TabConfigErrors` is only emitted when errors exist, so
                    // when all files are fixed we only get `TabConfigs` — this
                    // branch handles that case by prefix-dismissing all
                    // tab-config-error toasts and letting `TabConfigErrors`
                    // re-add any that still apply.
                    toast_stack.update(ctx, |toast_stack, ctx| {
                        toast_stack.dismiss_toasts_by_prefix("tab_config_error:", ctx);
                    });
                }
                WarpConfigUpdateEvent::TabConfigErrors(errors) => {
                    let home_dir = dirs::home_dir();
                    for error in errors {
                        let object_id = format!("tab_config_error:{}", error.file_path.display());
                        let raw_path = error.file_path.display().to_string();
                        let friendly_path = user_friendly_path(
                            &raw_path,
                            home_dir.as_ref().and_then(|h| h.to_str()),
                        );
                        let message = format!(
                            "Failed to load tab config {friendly_path}: {}",
                            error.error_message
                        );
                        let path = error.file_path.clone();
                        let toast = DismissibleToast::error(message)
                            .with_object_id(object_id.clone())
                            .with_link(
                                ToastLink::new("Open file".to_string()).with_onclick_action(
                                    WorkspaceAction::OpenTabConfigErrorFile {
                                        path,
                                        toast_object_id: object_id,
                                    },
                                ),
                            );
                        toast_stack.update(ctx, |toast_stack, ctx| {
                            toast_stack.add_persistent_toast(toast, ctx);
                        });
                    }
                }
                _ => {}
            }
        });
    }

    /// Subscribes to `WarpConfigUpdateEvent::SettingsErrors` and
    /// `SettingsErrorsCleared` to update the workspace settings-error banner
    /// and mirror the state into the settings pane for its nav-rail footer.
    fn subscribe_to_settings_errors(ctx: &mut ViewContext<Self>) {
        ctx.subscribe_to_model(&WarpConfig::handle(ctx), |me, _, event, ctx| match event {
            WarpConfigUpdateEvent::SettingsErrors(error) => {
                me.settings_file_error = Some(error.clone());
                me.sync_settings_error_state_into_settings_pane(ctx);
                ctx.notify();
            }
            WarpConfigUpdateEvent::SettingsErrorsCleared => {
                me.settings_file_error = None;
                me.sync_settings_error_state_into_settings_pane(ctx);
                ctx.notify();
            }
            _ => {}
        });
    }

    /// Pushes the current settings-file error + banner-dismissal state into
    /// the settings pane so its nav-rail footer ("Open settings file" button
    /// or inline error alert) stays in sync with the workspace banner.
    fn sync_settings_error_state_into_settings_pane(&mut self, ctx: &mut ViewContext<Self>) {
        let error = self.settings_file_error.clone();
        let dismissed = self.settings_error_banner_dismissed;
        self.settings_pane.update(ctx, |view, ctx| {
            view.set_settings_error_state(error, dismissed, ctx);
        });
    }

    pub fn dismiss_older_toasts(&mut self, object_id: &str, ctx: &mut ViewContext<Self>) {
        self.toast_stack.update(ctx, |toast_stack, ctx| {
            toast_stack.dismiss_older_toasts(object_id, ctx);
        });
    }

    fn on_tips_model_changed(
        &mut self,
        _: ModelHandle<TipsCompleted>,
        ctx: &mut ViewContext<Self>,
    ) {
        ctx.notify();
    }

    pub fn new(
        global_resource_handles: GlobalResourceHandles,
        workspace_setting: NewWorkspaceSource,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let GlobalResourceHandles {
            model_event_sender,
            tips_completed,
            user_default_shell_unsupported_banner_model_handle,
            settings_file_error,
        } = global_resource_handles.clone();

        // Inserting a (window, ModalSizes) pair to the ResizableData singleton. A restored window
        // reads the sizes from the window snapshot. A new window initializes with all default sizes.
        let resizable_data = ResizableData::handle(ctx);
        let window_id = ctx.window_id();
        let has_horizontal_split = workspace_setting.has_horizontal_split();

        let (left_panel_size, right_panel_size) =
            compute_default_panel_widths(ctx, window_id, has_horizontal_split);
        let new_resizable_modal_sizes = match workspace_setting.clone() {
            NewWorkspaceSource::Restored {
                window_snapshot, ..
            } => ModalSizes::from_restored(&window_snapshot, left_panel_size, right_panel_size),
            _ => ModalSizes::default_with_panel_defaults(left_panel_size, right_panel_size),
        };
        resizable_data.update(ctx, |model, _| {
            model.insert(window_id, new_resizable_modal_sizes)
        });

        terminal::platform::init().expect("Terminal platform initialized");

        let tab_bar_overflow_menu = Self::build_tab_bar_overflow_menu(ctx);
        let (tab_right_click_menu, new_session_dropdown_menu, new_session_sidecar_menu) =
            Self::build_menus(ctx);

        // Subscribe to network changes
        ctx.subscribe_to_model(
            &NetworkStatus::handle(ctx),
            Self::handle_network_status_event,
        );

        let palette =
            ctx.add_typed_action_view(|ctx| CommandPalette::new(NavigationMode::Normal, ctx));
        ctx.subscribe_to_view(&palette, |me, _, event, ctx| {
            me.handle_palette_event(event, ctx);
        });

        let ctrl_tab_palette =
            ctx.add_typed_action_view(|ctx| CommandPalette::new(NavigationMode::CtrlTab, ctx));
        ctx.subscribe_to_view(&ctrl_tab_palette, |me, _, event, ctx| {
            me.handle_palette_event(event, ctx);
        });

        // Handle local theme updates while the picker is open.
        ctx.subscribe_to_model(&ThemeSettings::handle(ctx), |me, _, _, ctx| {
            if me.is_theme_chooser_open() {
                me.theme_chooser_view.update(ctx, |view, ctx| {
                    view.handle_theme_change(ctx);
                });
            }
        });

        let bindings_notifier = KeybindingChangedNotifier::handle(ctx);
        ctx.subscribe_to_model(&bindings_notifier, |me, _, event, ctx| {
            me.handle_keybinding_changed(event, ctx);
        });

        let state_handle = WindowManager::handle(ctx);
        ctx.subscribe_to_model(&state_handle, |me, _, event, ctx| {
            me.handle_window_state_change(event, ctx);
        });

        let (welcome_tips_view, welcome_tips_view_state) =
            Self::build_welcome_tips(tips_completed.clone(), ctx);
        let (settings_pane, theme_chooser_view) =
            Self::build_settings_views(global_resource_handles, tips_completed.clone(), ctx);

        let resource_center_view = Self::build_resource_center_view(ctx, tips_completed.clone());

        let codex_modal = ctx.add_typed_action_view(CodexModal::new);
        ctx.subscribe_to_view(&codex_modal, |me, _, event, ctx| {
            me.handle_codex_modal_event(event, ctx);
        });

        let theme_creator_modal = Self::build_theme_creator_modal(ctx);

        let theme_deletion_modal = Self::build_theme_deletion_modal(ctx);

        let launch_config_save_modal = Self::build_launch_config_save_modal(ctx);

        let tab_config_params_modal = Self::build_tab_config_params_modal(ctx);
        let new_worktree_modal = Self::build_new_worktree_modal(ctx);

        let session_config_modal = Self::build_session_config_modal(ctx);

        let rewind_confirmation_dialog = Self::build_rewind_confirmation_dialog(ctx);
        let delete_conversation_confirmation_dialog =
            Self::build_delete_conversation_confirmation_dialog(ctx);
        let command_search_view = ctx.add_typed_action_view(CommandSearchView::new);
        ctx.subscribe_to_view(&command_search_view, |me, _, event, ctx| {
            me.handle_command_search_event(event, ctx);
        });

        let working_directories_model =
            ctx.add_model(|_| pane_group::WorkingDirectoriesModel::new());

        let left_panel_views = Self::compute_left_panel_views(ctx);

        let left_panel_view = ctx.add_typed_action_view(|ctx| {
            LeftPanelView::new(
                working_directories_model.clone(),
                left_panel_views.clone(),
                ctx,
            )
        });

        ctx.subscribe_to_view(&left_panel_view, |me, _, event, ctx| {
            me.handle_left_panel_event(event, ctx);
        });

        let right_panel_view = ctx.add_typed_action_view(|ctx| {
            RightPanelView::new(working_directories_model.clone(), ctx)
        });
        ctx.subscribe_to_view(&right_panel_view, |me, _, event, ctx| {
            me.handle_right_panel_event(event.clone(), ctx);
        });

        ctx.observe(&tips_completed, Workspace::on_tips_model_changed);

        ctx.subscribe_to_model(
            &BlocklistAIHistoryModel::handle(ctx),
            Self::handle_history_model_event,
        );
        ctx.subscribe_to_model(&CLIAgentSessionsModel::handle(ctx), |me, _, event, ctx| {
            me.handle_cli_agent_sessions_event(event, ctx);
        });

        ctx.subscribe_to_model(
            &SessionSettings::handle(ctx),
            Self::handle_session_settings_event,
        );

        ctx.subscribe_to_model(&WindowSettings::handle(ctx), |me, _handle, event, ctx| {
            me.handle_window_settings_changed_event(event, ctx);
        });

        let tab_settings_handle = TabSettings::handle(ctx);
        ctx.subscribe_to_model(&tab_settings_handle, |me, _, event, ctx| {
            me.handle_tab_settings_change(event, ctx)
        });

        ctx.subscribe_to_model(&CodeSettings::handle(ctx), |me, _, event, ctx| {
            if matches!(
                event,
                CodeSettingsChangedEvent::ShowProjectExplorer { .. }
                    | CodeSettingsChangedEvent::ShowGlobalSearch { .. }
            ) {
                me.update_left_panel_available_views(ctx);
                ctx.notify();
            }
        });

        let toast_stack =
            ctx.add_typed_action_view(|_| DismissibleToastStack::new(Duration::from_secs(4)));

        let update_toast_stack =
            ctx.add_typed_action_view(|_| DismissibleToastStack::new(Duration::from_secs(4)));

        #[cfg(target_family = "wasm")]
        let wasm_nux_dialog = Self::build_wasm_nux_dialog(ctx);

        #[cfg(target_family = "wasm")]
        let open_in_warp_button = Self::build_open_in_warp_button(ctx);

        let cached_keybindings = KEYBINDINGS_TO_CACHE
            .iter()
            .map(|name| {
                (
                    String::from(*name),
                    keybinding_name_to_display_string(name, ctx),
                )
            })
            .collect();

        let prompt_editor_modal = Self::build_prompt_editor_modal(ctx);
        let agent_toolbar_editor_modal = Self::build_agent_toolbar_editor_modal(ctx);

        Self::subscribe_to_workspace_toast_stack(toast_stack.clone(), ctx);
        Self::subscribe_to_tab_config_errors(toast_stack.clone(), ctx);
        Self::subscribe_to_settings_errors(ctx);
        let user_menu = ctx.add_typed_action_view(|_| {
            Menu::new()
                .with_drop_shadow()
                .prevent_interaction_with_other_elements()
        });
        ctx.subscribe_to_view(&user_menu, |me, _, event, ctx| {
            if let MenuEvent::Close { .. } = event {
                me.is_user_menu_open = false;
                ctx.notify();
            }
        });

        let native_modal = Self::build_native_modal_view(ctx);

        ctx.subscribe_to_model(&AISettings::handle(ctx), |me, _, event, ctx| match event {
            AISettingsChangedEvent::IsAnyAIEnabled { .. } => {
                me.update_left_panel_available_views(ctx);
                ctx.notify();
            }
            AISettingsChangedEvent::IsActiveAIEnabled { .. }
            | AISettingsChangedEvent::ThinkingDisplayMode { .. } => {
                ctx.notify();
            }
            AISettingsChangedEvent::ShowAgentNotifications { .. } => ctx.notify(),
            _ => (),
        });

        let mut ws = Self {
            tabs: Vec::new(),
            active_tab_index: 0,
            hovered_tab_index: None,
            tab_bar_hover_state: Default::default(),
            traffic_light_mouse_states: Default::default(),
            tab_rename_editor: Self::tab_rename_editor(ctx),
            pane_rename_editor: Self::pane_rename_editor(ctx),
            vertical_tabs_search_input: Self::vertical_tabs_search_input(ctx),
            tips_completed,
            user_default_shell_unsupported_banner_model_handle,
            tab_bar_overflow_menu,
            show_tab_bar_overflow_menu: false,
            tab_right_click_menu,
            show_tab_right_click_menu: None,
            new_session_dropdown_menu,
            show_new_session_dropdown_menu: None,
            welcome_tips_view_state,
            welcome_tips_view,
            palette,
            ctrl_tab_palette,
            mouse_states: Default::default(),
            previous_theme: None,
            settings_pane,
            theme_chooser_view,
            current_workspace_state: Default::default(),
            previous_workspace_state: None,
            model_event_sender,
            launch_config_save_modal,
            tab_config_params_modal,
            session_config_modal,
            pending_session_config_replacement: None,
            pending_session_config_tab_config_chip: false,
            show_session_config_tab_config_chip: false,
            new_worktree_modal,
            rewind_confirmation_dialog,
            delete_conversation_confirmation_dialog,
            resource_center_view,
            command_search_view,
            settings_file_error,
            settings_error_banner_dismissed: false,
            theme_creator_modal,
            theme_deletion_modal,
            window_id: ctx.window_id(),
            toast_stack,
            update_toast_stack,
            cached_keybindings,
            prompt_editor_modal,
            agent_toolbar_editor_modal,
            header_toolbar_editor_modal: Self::build_header_toolbar_editor_modal(ctx),
            header_toolbar_context_menu: Self::build_header_toolbar_context_menu(ctx),
            show_header_toolbar_context_menu: None,
            is_user_menu_open: false,
            tab_bar_pinned_by_popup: false,
            user_menu,
            native_modal,
            file_upload_sessions: Default::default(),
            left_panel_open: false,
            vertical_tabs_panel_open: false,
            vertical_tabs_panel: Default::default(),
            left_panel_view,
            left_panel_views,
            right_panel_view,
            working_directories_model,

            #[cfg(target_family = "wasm")]
            show_wasm_nux_dialog: WasmNUXDialog::should_display(ctx),
            #[cfg(target_family = "wasm")]
            wasm_nux_dialog,
            #[cfg(target_family = "wasm")]
            open_in_warp_button,
            tab_fixed_width: None,
            codex_modal,
            lightbox_view: None,
            pending_pane_group_transfer: false,
            is_drag_preview_workspace: false,
            new_session_sidecar_menu,
            show_new_session_sidecar: false,
            worktree_sidecar_active: false,
            worktree_sidecar_search_editor: Self::build_worktree_sidecar_search_input(ctx),
            worktree_sidecar_search_query: String::new(),
            new_session_sidecar_add_repo_mouse_state: Default::default(),
            tab_config_action_sidecar_item: None,
            tab_config_action_sidecar_mouse_states: Default::default(),
            remove_tab_config_confirmation_dialog:
                Self::build_remove_tab_config_confirmation_dialog(ctx),
        };

        ws.configure_new_workspace(workspace_setting, ctx);
        ws.sync_panel_positions_from_config(ctx);
        ws.sync_window_button_visibility(ctx);
        ws.update_titlebar_height(ctx);
        // Seed the settings pane with the initial settings-file error (if
        // any) read from `GlobalResourceHandles`. Subsequent updates are
        // pushed by `subscribe_to_settings_errors` and `dismiss_workspace_banner`.
        ws.sync_settings_error_state_into_settings_pane(ctx);

        let weak_handle = ctx.handle();
        WorkspaceRegistry::handle(ctx).update(ctx, |registry, _| {
            registry.register(window_id, weak_handle);
        });

        ws
    }

    #[cfg(any(test, feature = "integration_tests"))]
    pub fn command_palette_view(&self) -> ViewHandle<crate::search::command_palette::View> {
        self.palette.clone()
    }

    fn handle_task_status_reset(&mut self, pane_group_id: EntityId, ctx: &mut ViewContext<Self>) {
        // Re-render the workspace so the tab indicator picks up the new state.
        let has_tab = self
            .tabs
            .iter()
            .any(|tab| tab.pane_group.id() == pane_group_id);
        if has_tab {
            ctx.notify();
        }
    }

    /// Handles updating the tab status when an agent task status changes.
    fn handle_history_model_event(
        &mut self,
        _: ModelHandle<BlocklistAIHistoryModel>,
        event: &BlocklistAIHistoryEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        if matches!(
            event,
            BlocklistAIHistoryEvent::UpdatedConversationStatus { .. }
        ) {
            ctx.notify();
        }

        if self.agent_conversation_event_affects_vertical_tabs(event, ctx) {
            ctx.notify();
        }
    }

    fn workspace_contains_terminal_view(
        &self,
        terminal_view_id: EntityId,
        ctx: &AppContext,
    ) -> bool {
        self.tabs.iter().any(|tab| {
            tab.pane_group
                .as_ref(ctx)
                .contains_terminal_view(terminal_view_id, ctx)
        })
    }

    fn agent_conversation_event_affects_vertical_tabs(
        &self,
        event: &BlocklistAIHistoryEvent,
        ctx: &AppContext,
    ) -> bool {
        matches!(
            event,
            BlocklistAIHistoryEvent::StartedNewConversation { .. }
                | BlocklistAIHistoryEvent::AppendedExchange { .. }
                | BlocklistAIHistoryEvent::UpdatedStreamingExchange { .. }
                | BlocklistAIHistoryEvent::SetActiveConversation { .. }
                | BlocklistAIHistoryEvent::ClearedActiveConversation { .. }
                | BlocklistAIHistoryEvent::ClearedConversationsInTerminalView { .. }
                | BlocklistAIHistoryEvent::SplitConversation { .. }
                | BlocklistAIHistoryEvent::RestoredConversations { .. }
                | BlocklistAIHistoryEvent::UpdatedConversationMetadata { .. }
        ) && event.terminal_view_id().is_some_and(|terminal_view_id| {
            self.workspace_contains_terminal_view(terminal_view_id, ctx)
        })
    }

    fn handle_cli_agent_sessions_event(
        &mut self,
        event: &CLIAgentSessionsModelEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        if matches!(
            event,
            CLIAgentSessionsModelEvent::Started { .. }
                | CLIAgentSessionsModelEvent::StatusChanged { .. }
                | CLIAgentSessionsModelEvent::Ended { .. }
                | CLIAgentSessionsModelEvent::SessionUpdated { .. }
        ) && self.workspace_contains_terminal_view(event.terminal_view_id(), ctx)
        {
            ctx.notify();
        }
    }

    /// Handle session settings changes.
    fn handle_session_settings_event(
        &mut self,
        session_settings: ModelHandle<SessionSettings>,
        event: &SessionSettingsChangedEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        if let SessionSettingsChangedEvent::HonorPS1 { .. } = event {
            let honor_ps1 = *session_settings.as_ref(ctx).honor_ps1;
            for tab in &self.tabs {
                // Each tab has a pane group.
                tab.pane_group.update(ctx, |pane_group, ctx| {
                    pane_group.send_prompt_change_bindkey_to_all_sessions(honor_ps1, ctx);
                });
            }
        }

        // When Notifications settings change, request system notification permissions if needed.
        if let SessionSettingsChangedEvent::Notifications { .. } = event {
            self.request_notification_permissions_if_needed(ctx);
        }
    }

    /// Handle a change to the tab settings.
    fn handle_tab_settings_change(
        &mut self,
        event: &TabSettingsChangedEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            TabSettingsChangedEvent::WorkspaceDecorationVisibility { .. } => {
                self.sync_window_button_visibility(ctx);
                ctx.notify();
            }
            TabSettingsChangedEvent::ShowIndicatorsButton { .. }
            | TabSettingsChangedEvent::NewTabPlacement { .. }
            | TabSettingsChangedEvent::TabCloseButtonPosition { .. }
            | TabSettingsChangedEvent::PreserveActiveTabColor { .. } => {
                self.sync_window_button_visibility(ctx);
                ctx.notify();
            }
            TabSettingsChangedEvent::UseVerticalTabs { .. } => {
                let vertical_tabs_enabled = *TabSettings::as_ref(ctx).use_vertical_tabs;
                self.vertical_tabs_panel_open = vertical_tabs_enabled;

                if vertical_tabs_enabled {
                    Self::ensure_tabs_panel_in_config(ctx);
                }

                let appearance = Appearance::as_ref(ctx);
                let font_family = appearance.ui_font_family();
                let font_size = Self::tab_rename_editor_font_size(ctx, appearance);
                self.tab_rename_editor.update(ctx, |editor, ctx| {
                    editor.set_font_family(font_family, ctx);
                    editor.set_font_size(font_size, ctx);
                });
                if !vertical_tabs_enabled {
                    self.close_vertical_tabs_settings_popup();
                }
                self.sync_panel_positions_from_config(ctx);
                self.sync_window_button_visibility(ctx);
                ctx.notify();
            }
            TabSettingsChangedEvent::ShowCodeReviewButton { .. } => {
                // Close the right panel if it's open and the setting was just disabled.
                if !*TabSettings::as_ref(ctx).show_code_review_button {
                    let pane_group = self.active_tab_pane_group().clone();
                    if pane_group.as_ref(ctx).right_panel_open {
                        self.close_right_panel(&pane_group, ctx);
                    }
                }
                ctx.notify();
            }
            TabSettingsChangedEvent::ShowCodeReviewDiffStats { .. } => {
                ctx.notify();
            }
            TabSettingsChangedEvent::DirectoryTabColors { .. } => {
                if FeatureFlag::DirectoryTabColors.is_enabled() {
                    for tab in &mut self.tabs {
                        Self::sync_codebase_tab_color(tab, ctx);
                    }
                }
                ctx.notify();
            }
            TabSettingsChangedEvent::VerticalTabsViewMode { .. }
            | TabSettingsChangedEvent::VerticalTabsTabItemMode { .. }
            | TabSettingsChangedEvent::VerticalTabsPrimaryInfo { .. }
            | TabSettingsChangedEvent::VerticalTabsCompactSubtitle { .. }
            | TabSettingsChangedEvent::UseLatestUserPromptAsConversationTitleInTabNames {
                ..
            }
            | TabSettingsChangedEvent::VerticalTabsShowPrLink { .. }
            | TabSettingsChangedEvent::VerticalTabsShowDiffStats { .. } => {
                ctx.notify();
            }
            TabSettingsChangedEvent::VerticalTabsShowDetailsOnHover { .. } => {
                if !*TabSettings::as_ref(ctx).vertical_tabs_show_details_on_hover {
                    self.vertical_tabs_panel.clear_detail_sidecar();
                }
                ctx.notify();
            }
            TabSettingsChangedEvent::VerticalTabsDisplayGranularity { .. } => {
                let appearance = Appearance::as_ref(ctx);
                let font_size = Self::tab_rename_editor_font_size(ctx, appearance);
                self.tab_rename_editor.update(ctx, |editor, ctx| {
                    editor.set_font_size(font_size, ctx);
                });
                ctx.notify();
            }
            TabSettingsChangedEvent::HeaderToolbarChipSelection { .. } => {
                self.sync_panel_positions_from_config(ctx);
                ctx.notify();
            }
        }
    }

    /// Opens a launch config window into the workspace.
    pub fn open_launch_config_window(
        &mut self,
        window: WindowTemplate,
        ctx: &mut ViewContext<Self>,
    ) {
        let start_index = self.tabs.len();

        window
            .tabs
            .iter()
            .enumerate()
            .for_each(|(tab_index, tab_template)| {
                self.add_tab_with_pane_layout(
                    PanesLayout::Template(tab_template.layout.clone()),
                    Arc::new(HashMap::new()),
                    tab_template.title.clone(),
                    ctx,
                );
                self.tabs[start_index + tab_index].selected_color = tab_template
                    .color
                    .map_or(SelectedTabColor::Unset, SelectedTabColor::Color);
            });

        if !window.tabs.is_empty() {
            // Focus the active tab from the launch config.

            let mut index = start_index + window.active_tab_index.unwrap_or_default();

            if index >= self.tab_count() {
                index = start_index;
            }

            self.activate_tab_internal(index, ctx);
        }
    }

    fn configure_new_workspace(
        &mut self,
        workspace_setting: NewWorkspaceSource,
        ctx: &mut ViewContext<Self>,
    ) {
        self.vertical_tabs_panel_open =
            Self::initial_vertical_tabs_panel_open(&workspace_setting, ctx);
        match workspace_setting {
            NewWorkspaceSource::Empty {
                previous_active_window,
                shell,
            } => {
                self.configure_empty_workspace(previous_active_window, shell, ctx);
            }
            NewWorkspaceSource::Restored {
                window_snapshot,
                block_lists,
            } => {
                let active_tab_index = window_snapshot.active_tab_index;
                let restored_left_panel_open = window_snapshot.left_panel_open;

                window_snapshot
                    .tabs
                    .iter()
                    .enumerate()
                    .for_each(|(tab_index, saved_tab)| {
                        let custom_title = saved_tab.custom_title.clone();
                        self.add_tab_with_pane_layout(
                            PanesLayout::Snapshot(Box::new(saved_tab.root.clone())),
                            block_lists.clone(),
                            custom_title,
                            ctx,
                        );
                        self.tabs[tab_index].default_directory_color =
                            saved_tab.default_directory_color;
                        self.tabs[tab_index].selected_color = saved_tab.selected_color;

                        let pane_group = self.tabs[tab_index].pane_group.clone();

                        if let Some(left_panel_snapshot) = &saved_tab.left_panel {
                            self.restore_left_panel_for_tab(&pane_group, left_panel_snapshot, ctx);
                        }

                        if let Some(right_panel_snapshot) = &saved_tab.right_panel {
                            self.restore_right_panel_for_tab(
                                &pane_group,
                                right_panel_snapshot,
                                ctx,
                            );
                        }
                    });

                if self.tab_count() == 0 {
                    if self.should_open_get_started_tab(ctx) {
                        self.open_get_started_tab(ctx);
                        return;
                    }
                    // If we still haven't created any tabs after attempting to restore, create a new tab
                    // with sensible defaults.
                    self.add_new_session_tab_with_default_mode(
                        NewSessionSource::Window,
                        None,  /* previous_active_window */
                        None,  /* chosen_shell */
                        None,  /* ai_conversation */
                        false, /* hide_homepage */
                        ctx,
                    );
                } else if self.left_panel_visibility_across_tabs_enabled(ctx) {
                    self.left_panel_open = restored_left_panel_open;
                }

                self.activate_tab_internal(active_tab_index, ctx);
            }
            NewWorkspaceSource::FromTemplate { window_template } => {
                self.open_launch_config_window(window_template, ctx);
            }
            NewWorkspaceSource::Session { options } => {
                self.add_tab_with_pane_layout(
                    PanesLayout::SingleTerminal(options),
                    Arc::new(HashMap::new()),
                    None,
                    ctx,
                );
            }
            NewWorkspaceSource::AgentSession {
                options,
                initial_query,
            } => {
                self.add_tab_with_pane_layout(
                    PanesLayout::SingleTerminal(options),
                    Arc::new(HashMap::new()),
                    None,
                    ctx,
                );
                // Enter agent mode with the environment creation query.
                self.active_tab_pane_group().update(ctx, |pane_group, ctx| {
                    pane_group.start_agent_mode_in_new_pane(initial_query.as_deref(), None, ctx);
                });
            }
            NewWorkspaceSource::NotebookFromFilePath { file_path } => {
                self.add_tab_for_file_notebook(file_path, ctx);
            }
            #[cfg(feature = "local_fs")]
            NewWorkspaceSource::TransferredTab {
                tab_color,
                custom_title,
                left_panel_open,
                right_panel_open,
                is_right_panel_maximized,
                for_drag_preview,
                ..
            } => {
                self.is_drag_preview_workspace = for_drag_preview;
                self.add_tab_with_pane_layout(
                    Default::default(),
                    Arc::new(HashMap::new()),
                    custom_title,
                    ctx,
                );
                if let (Some(color), Some(tab)) = (tab_color, self.tabs.last_mut()) {
                    tab.selected_color = SelectedTabColor::Color(color);
                }
                if self.left_panel_visibility_across_tabs_enabled(ctx) {
                    self.left_panel_open = left_panel_open;
                }
                if right_panel_open {
                    self.right_panel_view.update(ctx, |rp, ctx| {
                        rp.set_maximized(is_right_panel_maximized, ctx);
                    });
                    self.setup_code_review_panel(None, ctx);
                }
                self.pending_pane_group_transfer = true;
            }
            #[cfg(not(feature = "local_fs"))]
            NewWorkspaceSource::TransferredTab {
                tab_color,
                custom_title,
                left_panel_open,
                for_drag_preview,
                ..
            } => {
                self.is_drag_preview_workspace = for_drag_preview;
                self.add_tab_with_pane_layout(
                    Default::default(),
                    Arc::new(HashMap::new()),
                    custom_title,
                    ctx,
                );
                if let (Some(color), Some(tab)) = (tab_color, self.tabs.last_mut()) {
                    tab.selected_color = SelectedTabColor::Color(color);
                }
                if self.left_panel_visibility_across_tabs_enabled(ctx) {
                    self.left_panel_open = left_panel_open;
                }
                self.pending_pane_group_transfer = true;
            }
        };

        debug_assert!(
            self.tab_count() > 0,
            "Workspace should have at least one tab upon configuration"
        );

        if self.left_panel_visibility_across_tabs_enabled(ctx) {
            self.reconcile_left_panel_open_for_active_tab(ctx);
        }

        let active_pane_group = self.active_tab_pane_group().clone();
        let working_directories_model = self.working_directories_model.clone();
        self.left_panel_view.update(ctx, |left_panel, ctx| {
            left_panel.set_active_pane_group(active_pane_group, &working_directories_model, ctx);
        });
    }

    fn initial_vertical_tabs_panel_open(
        workspace_setting: &NewWorkspaceSource,
        ctx: &AppContext,
    ) -> bool {
        let should_default_open =
            FeatureFlag::VerticalTabs.is_enabled() && *TabSettings::as_ref(ctx).use_vertical_tabs;

        match workspace_setting {
            NewWorkspaceSource::Restored {
                window_snapshot, ..
            } => window_snapshot.vertical_tabs_panel_open,
            NewWorkspaceSource::TransferredTab {
                vertical_tabs_panel_open,
                ..
            } => *vertical_tabs_panel_open,
            NewWorkspaceSource::Empty { .. }
            | NewWorkspaceSource::FromTemplate { .. }
            | NewWorkspaceSource::Session { .. }
            | NewWorkspaceSource::AgentSession { .. }
            | NewWorkspaceSource::NotebookFromFilePath { .. } => should_default_open,
        }
    }

    fn restore_left_panel_for_tab(
        &mut self,
        pane_group: &ViewHandle<PaneGroup>,
        left_panel_snapshot: &LeftPanelSnapshot,
        ctx: &mut ViewContext<Self>,
    ) {
        pane_group.update(ctx, |pg, ctx| {
            pg.set_left_panel_open(true, ctx);
        });

        let resizable = ResizableData::handle(ctx);
        if let Some(modal_sizes) = resizable.as_ref(ctx).get_all_handles(self.window_id) {
            if let Ok(mut handle) = modal_sizes.left_panel_width.lock() {
                handle.set_size(left_panel_snapshot.width as f32);
            }
        }

        self.left_panel_view.update(ctx, |lp, ctx| {
            // Restore which panel tab was active
            let active_view = match left_panel_snapshot.left_panel_displayed_tab {
                LeftPanelDisplayedTab::FileTree => ToolPanelView::ProjectExplorer,
                LeftPanelDisplayedTab::GlobalSearch => ToolPanelView::GlobalSearch {
                    entry_focus: GlobalSearchEntryFocus::Results,
                },
                LeftPanelDisplayedTab::ConversationListView => ToolPanelView::ProjectExplorer,
            };
            lp.restore_active_view_from_snapshot(active_view, ctx);
            lp.set_active_pane_group(pane_group.clone(), &self.working_directories_model, ctx);
        });

        ctx.notify();
    }

    fn restore_right_panel_for_tab(
        &mut self,
        pane_group: &ViewHandle<PaneGroup>,
        right_panel_snapshot: &RightPanelSnapshot,
        ctx: &mut ViewContext<Self>,
    ) {
        pane_group.update(ctx, |pg, _| {
            pg.right_panel_open = true;
            pg.is_right_panel_maximized = right_panel_snapshot.is_maximized;
        });

        let resizable = ResizableData::handle(ctx);
        if let Some(modal_sizes) = resizable.as_ref(ctx).get_all_handles(self.window_id) {
            if let Ok(mut handle) = modal_sizes.right_panel_width.lock() {
                handle.set_size(right_panel_snapshot.width as f32);
            }
        }

        self.right_panel_view.update(ctx, |rp, ctx| {
            rp.set_active_pane_group(pane_group.clone(), &self.working_directories_model, ctx);
            rp.set_maximized(right_panel_snapshot.is_maximized, ctx);
        });

        ctx.notify();
    }

    // Configure an empty workspace. The behavior here is platform-specific.
    fn configure_empty_workspace(
        &mut self,
        previous_active_window: Option<WindowId>,
        shell: Option<AvailableShell>,
        ctx: &mut ViewContext<Self>,
    ) {
        let show_warp_home = !ContextFlag::CreateNewSession.is_enabled();
        if !show_warp_home {
            if self.should_open_get_started_tab(ctx) {
                self.open_get_started_tab(ctx);
            } else if FeatureFlag::WelcomeTab.is_enabled() {
                self.add_welcome_tab(ctx);
            } else {
                self.add_new_session_tab_with_default_mode(
                    NewSessionSource::Window,
                    previous_active_window,
                    shell,
                    None,  /* ai_conversation */
                    false, /* hide_homepage */
                    ctx,
                );
            }
        } else {
            let home_pane = super::home::create_home_pane(ctx);
            self.add_tab_from_existing_pane(home_pane, 0, ctx);
        };
    }

    pub fn is_conversation_transcript_viewer_focused(&self, app: &AppContext) -> bool {
        self.active_tab_pane_group()
            .as_ref(app)
            .active_session_view(app)
            .is_some_and(|view| {
                view.as_ref(app)
                    .model
                    .lock()
                    .is_conversation_transcript_viewer()
            })
    }

    /// Returns the type of simplified WASM tab bar content to display, if any.
    /// Used to determine whether to show the simplified tab bar layout on WASM.
    #[cfg(target_family = "wasm")]
    fn get_simplified_wasm_tab_bar_content(
        &self,
        ctx: &AppContext,
    ) -> Option<SimplifiedWasmTabBarContent> {
        let pane_group = self.active_tab_pane_group().as_ref(ctx);

        // Check if focused pane is a terminal with special state
        if let Some(terminal_view) = pane_group.focused_session_view(ctx) {
            let model = terminal_view.as_ref(ctx).model.lock();

            // Conversation transcript viewer takes priority
            if model.is_conversation_transcript_viewer() {
                return Some(SimplifiedWasmTabBarContent::ConversationTranscript);
            }
        }

        None
    }

    /// Add and focus a new terminal pane in AI mode in a new tab.
    fn add_terminal_tab_in_ai_mode(
        &mut self,
        zero_state_prompt_suggestion_type: Option<ZeroStatePromptSuggestionType>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.add_new_session_tab_internal_with_default_session_mode_behavior(
            NewSessionSource::Tab,
            Some(ctx.window_id()),
            None,
            None,
            false,
            DefaultSessionModeBehavior::Ignore,
            ctx,
        );
        self.active_tab_pane_group().update(ctx, |pane_group, ctx| {
            pane_group.start_agent_mode_in_new_pane(None, zero_state_prompt_suggestion_type, ctx);
        });
    }

    /// Add and focus a new terminal pane in AI mode. Add the terminal pane to the right of
    /// all other panes, as a split on the root node.
    fn add_terminal_pane_in_ai_mode(
        &mut self,
        zero_state_prompt_suggestion_type: Option<ZeroStatePromptSuggestionType>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.active_tab_pane_group().update(ctx, |pane_group, ctx| {
            pane_group.add_terminal_pane_in_agent_mode(
                None,
                zero_state_prompt_suggestion_type,
                ctx,
            );
        });
    }

    /// Add a new terminal tab and enter the agent view with a new conversation.
    fn add_terminal_tab_with_new_agent_view(&mut self, ctx: &mut ViewContext<Self>) {
        let was_left_panel_open = self.active_tab_pane_group().as_ref(ctx).left_panel_open;
        self.add_new_session_tab_internal_with_default_session_mode_behavior(
            NewSessionSource::Tab,
            Some(ctx.window_id()),
            None,
            None,
            false,
            DefaultSessionModeBehavior::Ignore,
            ctx,
        );
        self.active_tab_pane_group().update(ctx, |pane_group, ctx| {
            if was_left_panel_open {
                pane_group.set_left_panel_open(true, ctx);
            }
            if let Some(terminal_view) = pane_group.active_session_view(ctx) {
                terminal_view.update(ctx, |view, ctx| {
                    view.enter_agent_view_for_new_conversation(
                        None,
                        AgentViewEntryOrigin::ConversationListView,
                        ctx,
                    );
                });
            }
        });
    }

    fn current_focus_region(&self, ctx: &mut ViewContext<Self>) -> FocusRegion {
        let app = ctx;
        if self.active_tab_pane_group().is_self_or_child_focused(app) {
            return FocusRegion::PaneGroup;
        }

        if self.left_panel_view.is_self_or_child_focused(app) {
            return FocusRegion::LeftPanel;
        }
        if self.right_panel_view.is_self_or_child_focused(app) {
            return FocusRegion::RightPanel;
        }

        if self.resource_center_view.is_self_or_child_focused(app) {
            return FocusRegion::RightPanel;
        }

        FocusRegion::Other
    }

    fn has_left_region(&self, app: &AppContext) -> bool {
        self.active_tab_pane_group().as_ref(app).left_panel_open
    }

    fn has_right_region(&self, app: &AppContext) -> bool {
        let group = self.active_tab_pane_group().as_ref(app);
        group.right_panel_open || self.current_workspace_state.is_right_panel_open()
    }

    fn focus_next_pane_in_group(&mut self, ctx: &mut ViewContext<Self>) -> bool {
        let handle = self.active_tab_pane_group().clone();
        handle.update(ctx, |pane_group, ctx| pane_group.try_navigate_next(ctx))
    }

    fn focus_prev_pane_in_group(&mut self, ctx: &mut ViewContext<Self>) -> bool {
        let handle = self.active_tab_pane_group().clone();
        handle.update(ctx, |pane_group, ctx| pane_group.try_navigate_prev(ctx))
    }

    fn focus_first_visible_pane_in_group(&mut self, ctx: &mut ViewContext<Self>) -> bool {
        let handle = self.active_tab_pane_group().clone();
        handle.update(ctx, |pane_group, ctx| pane_group.focus_first_pane(ctx))
    }

    fn focus_last_visible_pane_in_group(&mut self, ctx: &mut ViewContext<Self>) -> bool {
        let handle = self.active_tab_pane_group().clone();
        handle.update(ctx, |pane_group, ctx| pane_group.focus_last_pane(ctx))
    }

    fn focus_left_region_entry(&mut self, ctx: &mut ViewContext<Self>) {
        if self.has_left_region(ctx) {
            self.left_panel_view.update(ctx, |left_panel, ctx| {
                left_panel.focus_active_view_on_entry(ctx);
            });
        }
    }

    fn focus_right_region_entry(&mut self, ctx: &mut ViewContext<Self>) {
        let group = self.active_tab_pane_group().as_ref(ctx);
        if group.right_panel_open {
            ctx.focus(&self.right_panel_view);
            return;
        }
        if self.current_workspace_state.is_resource_center_open {
            ctx.focus(&self.resource_center_view);
        }
    }

    fn navigate_pane_or_panel(
        &mut self,
        direction: PanePanelDirection,
        ctx: &mut ViewContext<Self>,
    ) {
        let current_region = self.current_focus_region(ctx);
        let has_left_panel = self.has_left_region(ctx);
        let has_right_panel = self.has_right_region(ctx);

        let target_region = self.compute_target_focus_region(
            current_region,
            direction,
            has_left_panel,
            has_right_panel,
            ctx,
        );

        self.set_pane_dimming_for_region(target_region, ctx);

        ctx.notify();
    }

    fn compute_target_focus_region(
        &mut self,
        region: FocusRegion,
        direction: PanePanelDirection,
        has_left_panel: bool,
        has_right_panel: bool,
        ctx: &mut ViewContext<Self>,
    ) -> FocusRegion {
        match (region, direction) {
            // NEXT: Left panel to first pane
            (FocusRegion::LeftPanel, PanePanelDirection::Next) => {
                // Always attempt to focus the first pane in the group and ensure the pane group
                // regains application focus.
                self.focus_first_visible_pane_in_group(ctx);
                self.focus_active_tab(ctx);
                FocusRegion::PaneGroup
            }
            // NEXT: Right panel to left panel if open, else first pane
            (FocusRegion::RightPanel, PanePanelDirection::Next) => {
                if has_left_panel {
                    self.focus_left_region_entry(ctx);
                    FocusRegion::LeftPanel
                } else {
                    self.focus_first_visible_pane_in_group(ctx);
                    FocusRegion::PaneGroup
                }
            }
            // NEXT: Pane group to next pane, or at end to right panel, left panel, first pane
            // Included Other here for cases like the command palette action "Activate next Pane"
            (FocusRegion::PaneGroup, PanePanelDirection::Next)
            | (FocusRegion::Other, PanePanelDirection::Next) => {
                let moved = self.focus_next_pane_in_group(ctx);
                if moved {
                    FocusRegion::PaneGroup
                } else if has_right_panel {
                    self.focus_right_region_entry(ctx);
                    FocusRegion::RightPanel
                } else if has_left_panel {
                    self.focus_left_region_entry(ctx);
                    FocusRegion::LeftPanel
                } else {
                    // No panels, wrap within panes.
                    self.focus_first_visible_pane_in_group(ctx);
                    FocusRegion::PaneGroup
                }
            }

            // PREV: Right panel to last pane
            (FocusRegion::RightPanel, PanePanelDirection::Prev) => {
                // Always attempt to focus the last pane in the group and ensure the pane group
                // regains application focus.
                self.focus_last_visible_pane_in_group(ctx);
                self.focus_active_tab(ctx);
                FocusRegion::PaneGroup
            }
            // PREV: Left panel to right panel if open, else last pane
            (FocusRegion::LeftPanel, PanePanelDirection::Prev) => {
                if has_right_panel {
                    self.focus_right_region_entry(ctx);
                    FocusRegion::RightPanel
                } else {
                    self.focus_last_visible_pane_in_group(ctx);
                    FocusRegion::PaneGroup
                }
            }
            // PREV: Pane group to prev pane, or at beginning to left panel to right panel to last pane
            // Included Other here for cases like the command palette action "Activate next Pane"
            (FocusRegion::PaneGroup, PanePanelDirection::Prev)
            | (FocusRegion::Other, PanePanelDirection::Prev) => {
                let did_move = self.focus_prev_pane_in_group(ctx);
                if did_move {
                    FocusRegion::PaneGroup
                } else if has_left_panel {
                    self.focus_left_region_entry(ctx);
                    FocusRegion::LeftPanel
                } else if has_right_panel {
                    self.focus_right_region_entry(ctx);
                    FocusRegion::RightPanel
                } else {
                    // No panels, wrap within panes.
                    self.focus_last_visible_pane_in_group(ctx);
                    FocusRegion::PaneGroup
                }
            }
        }
    }

    fn update_pane_dimming_for_current_focus_region(&mut self, ctx: &mut ViewContext<Self>) {
        let current_region = self.current_focus_region(ctx);
        self.set_pane_dimming_for_region(current_region, ctx);
    }

    fn set_pane_dimming_for_region(&mut self, region: FocusRegion, ctx: &mut ViewContext<Self>) {
        let dim_even_if_focused =
            matches!(region, FocusRegion::LeftPanel | FocusRegion::RightPanel);
        let handle = self.active_tab_pane_group().clone();
        handle.update(ctx, |pane_group, ctx| {
            pane_group.set_dim_even_if_focused_for_all_panes(dim_even_if_focused, ctx);
        });
    }

    /// This function shifts focus to the panel on the left.
    /// The current focusable panels are: theme chooser and resource center.
    fn focus_left_panel(&mut self, ctx: &mut ViewContext<Self>) {
        // Starts from terminal
        if self.active_tab_pane_group().is_self_or_child_focused(ctx) {
            if self.is_theme_chooser_open() {
                ctx.focus(&self.theme_chooser_view);
            } else if self.current_workspace_state.is_resource_center_open {
                ctx.focus(&self.resource_center_view);
            }
        }
        // Starts from a right panel: resource center.
        else if self.resource_center_view.is_self_or_child_focused(ctx) {
            self.focus_active_tab(ctx);
        }
        // Starts from a left panel: theme chooser
        else if self.theme_chooser_view.is_self_or_child_focused(ctx) {
            if self.current_workspace_state.is_right_panel_open() {
                if self.current_workspace_state.is_resource_center_open {
                    ctx.focus(&self.resource_center_view);
                }
            } else {
                self.focus_active_tab(ctx);
            }
        }

        self.update_pane_dimming_for_current_focus_region(ctx);

        ctx.notify();
    }

    /// This function shifts focus to the panel on the right.
    fn focus_right_panel(&mut self, ctx: &mut ViewContext<Self>) {
        // Starts from terminal
        if self.active_tab_pane_group().is_self_or_child_focused(ctx) {
            if self.current_workspace_state.is_resource_center_open {
                ctx.focus(&self.resource_center_view);
            } else if self.is_theme_chooser_open() {
                ctx.focus(&self.theme_chooser_view);
            }
        }
        // Starts from a left panel: theme chooser
        else if self.theme_chooser_view.is_self_or_child_focused(ctx) {
            self.focus_active_tab(ctx);
        }
        // Starts from a right panel: resource center.
        else if self.resource_center_view.is_self_or_child_focused(ctx) {
            if self.current_workspace_state.is_left_panel_open() {
                if self.is_theme_chooser_open() {
                    ctx.focus(&self.theme_chooser_view);
                }
            } else {
                self.focus_active_tab(ctx);
            }
        }

        self.update_pane_dimming_for_current_focus_region(ctx);

        ctx.notify();
    }

    pub fn active_tab_index(&self) -> usize {
        self.active_tab_index
    }

    pub fn is_overflow_menu_showing(&self) -> bool {
        self.show_tab_bar_overflow_menu
    }

    pub fn is_resource_center_showing(&self) -> bool {
        self.current_workspace_state.is_resource_center_open
    }

    #[cfg(feature = "integration_tests")]
    pub fn is_command_search_open(&self) -> bool {
        self.current_workspace_state.is_command_search_open
    }

    /// Retrieves the Pane Group view for the passed tab index.
    pub fn get_pane_group_view(&self, index: usize) -> Option<&ViewHandle<PaneGroup>> {
        self.tabs.get(index).map(|s| &s.pane_group)
    }

    /// Retrieves the Pane Group view for the passed tab index. Unlike the other
    /// method, this does not check for out of bounds.
    pub fn get_pane_group_view_unchecked(&self, index: usize) -> &ViewHandle<PaneGroup> {
        &self.tabs[index].pane_group
    }

    pub fn tab_count(&self) -> usize {
        self.tabs.len()
    }

    pub fn tab_views(&self) -> impl Iterator<Item = &ViewHandle<PaneGroup>> {
        self.tabs.iter().map(|s| &s.pane_group)
    }

    /// Get the tab color for a given tab index.
    pub fn get_tab_color(&self, index: usize) -> Option<AnsiColorIdentifier> {
        self.tabs.get(index).and_then(|tab| tab.color())
    }

    /// Get information needed for transferring a tab to another window.
    /// Returns None if the index is invalid or if this is the last tab.
    pub fn get_tab_transfer_info(&self, index: usize, ctx: &AppContext) -> Option<TransferredTab> {
        if self.tabs.len() <= 1 {
            return None;
        }
        let tab = self.tabs.get(index)?;
        let pane_group = tab.pane_group.clone();
        let color = tab.color();
        let custom_title = pane_group.read(ctx, |pg, ctx| pg.custom_title(ctx));
        let left_panel_open = pane_group.read(ctx, |pg, _| pg.left_panel_open);
        let vertical_tabs_panel_open = self.vertical_tabs_panel_open;
        let right_panel_open = pane_group.read(ctx, |pg, _| pg.right_panel_open);
        let is_right_panel_maximized = pane_group.read(ctx, |pg, _| pg.is_right_panel_maximized);

        Some(TransferredTab {
            pane_group,
            color,
            custom_title,
            left_panel_open,
            vertical_tabs_panel_open,
            right_panel_open,
            is_right_panel_maximized,
        })
    }

    /// Gets all sessions in the current workspace.
    pub fn workspace_sessions<'a>(
        &'a self,
        window_id: WindowId,
        app: &'a AppContext,
    ) -> impl Iterator<Item = SessionNavigationData> + 'a {
        self.tabs.iter().flat_map(move |tab| {
            // Each tab has a pane group
            let pane_group_id = tab.pane_group.id();
            let view = tab.pane_group.as_ref(app);

            view.pane_sessions(pane_group_id, window_id, app)
        })
    }

    /// Returns the PaneGroup view handle for the currently active tab.
    pub fn active_tab_pane_group(&self) -> &ViewHandle<PaneGroup> {
        self.get_pane_group_view(self.active_tab_index)
            .expect("Active tab index entry should exist")
    }

    /// Attempts to get selected text from the focused pane.
    /// Returns None if there is no selection, multiple selections, or an empty selection.
    /// Supports code, notebook, AI document, and terminal panes.
    fn get_selected_text_from_focused_view(&self, ctx: &AppContext) -> Option<String> {
        self.active_tab_pane_group()
            .as_ref(ctx)
            .selected_text_from_focused_pane(ctx)
    }

    /// This is meant to be dispatched directly by actions.
    pub fn activate_tab(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        self.activate_tab_internal(index, ctx);
        ctx.notify();
    }

    /// This function is meant to be used by other actions to perform the logic to update the
    /// view's state. It's not meant to be invoked directly by an action.
    pub fn activate_tab_internal(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        if index < self.tab_count() {
            // If the command palette is open when the tab is switched using a keybinding,
            // we want to close the palette so that we don't get into a state where the palette
            // is open but doesn't have focus.
            if self.is_palette_open() {
                self.close_palette(false, None, ctx);
            }

            self.set_active_tab_index(index, ctx);
            self.focus_active_tab(ctx);
            self.update_window_title(ctx);
        }
    }

    fn left_panel_visibility_across_tabs_enabled(&self, ctx: &AppContext) -> bool {
        *WindowSettings::as_ref(ctx)
            .left_panel_visibility_across_tabs
            .value()
    }

    /// Reconciles the active tab's tools panel open/closed state to match the window-scoped desired state
    /// (syncing left panel open/closed state across tabs).
    fn reconcile_left_panel_open_for_active_tab(&mut self, ctx: &mut ViewContext<Self>) {
        let pane_group = self.active_tab_pane_group().clone();
        let pane_group_supports_tools_panel = pane_group.read(ctx, |pane_group, _| {
            Self::should_enable_file_tree_and_global_search_for_pane_group(pane_group)
        });

        if !pane_group_supports_tools_panel {
            return;
        }

        let desired_open = self.left_panel_open;
        pane_group.update(ctx, |pane_group, ctx| {
            pane_group.set_left_panel_open(desired_open, ctx);
        });
    }

    /// Notifies the agent views model and notifications model that a terminal view gained focus.
    fn notify_terminal_focus_change(
        &self,
        focused_terminal_view_id: Option<EntityId>,
        ctx: &mut ViewContext<Self>,
    ) {
        let window_id = ctx.window_id();
        ActiveAgentViewsModel::handle(ctx).update(ctx, |model, ctx| {
            model.handle_pane_focus_change(window_id, focused_terminal_view_id, ctx);
        });
        if let Some(terminal_view_id) = focused_terminal_view_id {
            let is_active_window = ctx.windows().active_window() == Some(ctx.window_id());
            if is_active_window {
                let _ = terminal_view_id;
            }
        }
    }

    /// Change the active tab index. This must be used instead of setting `self.active_tab_index`
    /// directly, as it updates related state.
    fn set_active_tab_index(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        let index = if index >= self.tab_count() {
            log::warn!(
                "Attempted to set active tab index {index} but only {} tabs exist, clamping",
                self.tab_count()
            );
            self.tab_count().saturating_sub(1)
        } else {
            index
        };

        self.active_tab_index = index;

        if self.vertical_tabs_panel_open
            && FeatureFlag::VerticalTabs.is_enabled()
            && *TabSettings::as_ref(ctx).use_vertical_tabs
        {
            self.vertical_tabs_panel.scroll_to_tab(index);
        }

        if self.left_panel_visibility_across_tabs_enabled(ctx) {
            self.reconcile_left_panel_open_for_active_tab(ctx);
        }

        let left_active_pane_group = self.active_tab_pane_group().clone();
        let right_active_pane_group = self.active_tab_pane_group().clone();
        let working_directories_model = self.working_directories_model.clone();

        self.left_panel_view.update(ctx, |left_panel, ctx| {
            left_panel.set_active_pane_group(
                left_active_pane_group,
                &working_directories_model,
                ctx,
            );
        });
        self.right_panel_view.update(ctx, |right_pane, ctx| {
            right_pane.set_active_pane_group(
                right_active_pane_group,
                &working_directories_model,
                ctx,
            );
        });

        let pane_group = self.active_tab_pane_group();
        let focused_terminal_view_id = self
            .active_tab_pane_group()
            .as_ref(ctx)
            .terminal_view_from_pane_id(pane_group.as_ref(ctx).focused_pane_id(ctx), ctx)
            .map(|tv| tv.id());
        self.notify_terminal_focus_change(focused_terminal_view_id, ctx);

        self.update_active_session(ctx);
    }

    fn update_window_title(&self, ctx: &mut ViewContext<Self>) {
        let Some(tab) = self.tabs.get(self.active_tab_index) else {
            log::warn!(
                "Tried to update window title but active tab index ({}) was out of range 0..{}",
                self.active_tab_index,
                self.tabs.len()
            );
            return;
        };
        let tab_title = tab.pane_group.as_ref(ctx).display_title(ctx);

        let window_title = truncate_from_end(&tab_title, MAX_WINDOW_TITLE_LENGTH);

        let window_id = ctx.window_id();
        ctx.windows().set_window_title(window_id, &window_title);
    }

    fn rename_tab_internal(&mut self, index: usize, title: &str, ctx: &mut ViewContext<Self>) {
        // Focusing on the clicked tab
        if index >= self.tab_count() {
            return;
        }
        self.set_active_tab_index(index, ctx);

        self.current_workspace_state.set_tab_being_renamed(index);

        // Clear the tab name editor to handle the case when another tab is already being renamed
        self.clear_tab_name_editor(ctx);
        let font_size = Self::tab_rename_editor_font_size(ctx, Appearance::as_ref(ctx));

        self.tab_rename_editor.update(ctx, move |editor, ctx| {
            editor.set_font_size(font_size, ctx);
            editor.insert_selected_text(title, ctx);
        });

        ctx.focus(&self.tab_rename_editor);
        ctx.notify();
    }

    pub fn rename_tab(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        let tab = &self.tabs[index];
        let title = tab.pane_group.as_ref(ctx).display_title(ctx);

        self.rename_tab_internal(index, &title, ctx);
    }

    fn set_active_tab_name(&mut self, title: &str, ctx: &mut ViewContext<Self>) {
        let Some(pane_group) = self
            .tabs
            .get(self.active_tab_index)
            .map(|tab| tab.pane_group.clone())
        else {
            log::warn!(
                "Tried to set active tab name but active tab index ({}) was out of range 0..{}",
                self.active_tab_index,
                self.tabs.len()
            );
            return;
        };

        if self.current_workspace_state.is_tab_being_renamed() {
            self.current_workspace_state.clear_tab_being_renamed();
            self.clear_tab_name_editor(ctx);
        }

        let title = title.trim();
        if title.is_empty() {
            ctx.notify();
            return;
        }
        pane_group.update(ctx, |pane_group, ctx| {
            if pane_group.display_title(ctx) != title {
                pane_group.set_title(title, ctx);
            }
        });
        ctx.notify();
    }

    pub fn toggle_tab_color(
        &mut self,
        index: usize,
        color: AnsiColorIdentifier,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.tabs.get(index).is_none() {
            log::warn!(
                "Not toggling tab color: index was {index} but len is {}",
                self.tabs.len()
            );
            return;
        }
        let is_same = self.tabs[index].color() == Some(color);
        self.tabs[index].selected_color = if is_same {
            if FeatureFlag::DirectoryTabColors.is_enabled() {
                SelectedTabColor::Cleared
            } else {
                SelectedTabColor::Unset
            }
        } else {
            SelectedTabColor::Color(color)
        };
        ctx.notify();
    }

    /// Syncs the tab color for the given tab based on the active terminal's CWD.
    /// If the CWD is within a directory that has a configured color, applies it.
    /// If the CWD moves outside all configured directories, the directory color is cleared.
    fn sync_codebase_tab_color(tab: &mut TabData, ctx: &mut ViewContext<Self>) {
        let cwd = tab
            .pane_group
            .as_ref(ctx)
            .active_session_view(ctx)
            .and_then(|tv| tv.as_ref(ctx).pwd_if_local(ctx));

        let Some(cwd) = cwd else {
            return;
        };

        let cwd_path = Path::new(&cwd);
        let color = TabSettings::as_ref(ctx)
            .directory_tab_colors
            .value()
            .color_for_directory(cwd_path)
            .and_then(|c| c.ansi_color());

        tab.default_directory_color = color;
        ctx.notify();
    }

    fn clear_tab_name_editor(&mut self, ctx: &mut ViewContext<Self>) {
        self.tab_rename_editor.update(ctx, move |editor, ctx| {
            editor.clear_buffer_and_reset_undo_stack(ctx);
        });
    }

    fn clear_pane_name_editor(&mut self, ctx: &mut ViewContext<Self>) {
        self.pane_rename_editor.update(ctx, move |editor, ctx| {
            editor.clear_buffer_and_reset_undo_stack(ctx);
        });
    }

    pub fn clear_tab_name(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        let tab = &self.tabs[index];
        tab.pane_group.update(ctx, |view, ctx| {
            view.clear_title(ctx);
        });
        self.update_window_title(ctx);
        ctx.notify();
    }

    fn set_custom_pane_name(
        &mut self,
        locator: PaneViewLocator,
        title: String,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(pane_group_view) = self.get_pane_group_view_with_id(locator.pane_group_id) else {
            log::warn!("Tried to rename pane in a missing pane group");
            return;
        };
        pane_group_view.update(ctx, |pane_group, ctx| {
            let Some(pane) = pane_group.pane_by_id(locator.pane_id) else {
                log::warn!("Tried to rename a missing pane");
                return;
            };
            pane.pane_configuration().update(ctx, |configuration, ctx| {
                configuration.set_custom_vertical_tabs_title(title, ctx);
            });
            ctx.emit(pane_group::Event::AppStateChanged);
        });
    }

    pub fn clear_pane_name(&mut self, locator: PaneViewLocator, ctx: &mut ViewContext<Self>) {
        let Some(pane_group_view) = self.get_pane_group_view_with_id(locator.pane_group_id) else {
            log::warn!("Tried to clear pane name in a missing pane group");
            return;
        };
        pane_group_view.update(ctx, |pane_group, ctx| {
            let Some(pane) = pane_group.pane_by_id(locator.pane_id) else {
                log::warn!("Tried to clear a missing pane name");
                return;
            };
            pane.pane_configuration().update(ctx, |configuration, ctx| {
                configuration.clear_custom_vertical_tabs_title(ctx);
            });
            ctx.emit(pane_group::Event::AppStateChanged);
        });
        ctx.dispatch_global_action("workspace:save_app", ());
        ctx.notify();
    }

    pub fn rename_pane(&mut self, locator: PaneViewLocator, ctx: &mut ViewContext<Self>) {
        let Some((index, tab)) = self
            .tabs
            .iter()
            .enumerate()
            .find(|(_, tab_data)| tab_data.pane_group.id() == locator.pane_group_id)
        else {
            log::warn!("Tried to rename pane in a missing tab");
            return;
        };

        let Some(title) = tab
            .pane_group
            .as_ref(ctx)
            .pane_by_id(locator.pane_id)
            .map(|pane| {
                let configuration = pane.pane_configuration();
                let configuration = configuration.as_ref(ctx);
                configuration
                    .custom_vertical_tabs_title()
                    .map(str::to_owned)
                    .unwrap_or_else(|| {
                        let title = configuration.title().trim();
                        if title.is_empty() {
                            "Untitled pane".to_string()
                        } else {
                            title.to_string()
                        }
                    })
            })
        else {
            log::warn!("Tried to rename a missing pane");
            return;
        };

        tab.pane_group.update(ctx, |pane_group, ctx| {
            pane_group.focus_pane_by_id(locator.pane_id, ctx);
        });
        self.set_active_tab_index(index, ctx);
        self.current_workspace_state.set_pane_being_renamed(locator);
        self.clear_pane_name_editor(ctx);
        self.pane_rename_editor.update(ctx, move |editor, ctx| {
            editor.insert_selected_text(&title, ctx);
        });
        ctx.focus(&self.pane_rename_editor);
        ctx.notify();
    }

    pub fn list_tab_pane_groups(&self, app: &AppContext) -> Vec<TabPaneGroupIdentifiers> {
        self.tabs
            .iter()
            .enumerate()
            .map(|(tab_idx, tab)| {
                let pane_group_id = tab.pane_group.id();
                let pane_group = tab.pane_group.as_ref(app);

                let pane_ids = pane_group.terminal_pane_ids();
                let terminal_ids = pane_ids
                    .into_iter()
                    .filter_map(|pane_id| {
                        let terminal_view = pane_group.terminal_view_from_pane_id(pane_id, app)?;
                        Some(terminal_view.id())
                    })
                    .collect::<Vec<_>>();

                TabPaneGroupIdentifiers {
                    tab_idx,
                    pane_group_id,
                    terminal_ids,
                }
            })
            .collect::<Vec<_>>()
    }

    /// Focuses the given pane within the pane group.
    pub fn focus_pane(&mut self, pane_view_locator: PaneViewLocator, ctx: &mut ViewContext<Self>) {
        if let Some((index, tab)) = self
            .tabs
            .iter()
            .enumerate()
            .find(|(_, tab_data)| tab_data.pane_group.id() == pane_view_locator.pane_group_id)
        {
            // Update the pane group to focus the active pane,
            // and then focus the pane group (tab). The order is important
            // because if we otherwise focus the tab first and another pane
            // was focused in the mean time, that pane will be the one that will
            // remain focused (as opposed to the pane with pane_id) since its
            // input would remain focused.
            tab.pane_group.update(ctx, |view, ctx| {
                view.focus_pane_by_id(pane_view_locator.pane_id, ctx);
            });
            self.activate_tab_internal(index, ctx);
            ctx.notify();
        }
    }

    /// Searches this workspace's tabs for the given terminal view and focuses it.
    /// Returns true if the terminal view was found and focused.
    fn focus_terminal_view_locally(
        &mut self,
        terminal_view_id: EntityId,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        for tab in self.tabs.iter() {
            let pane_group_handle = &tab.pane_group;
            let pane_group = pane_group_handle.as_ref(ctx);
            if let Some(pane_id) = pane_group.find_pane_id_for_terminal_view(terminal_view_id, ctx)
            {
                self.focus_pane(
                    PaneViewLocator {
                        pane_group_id: pane_group_handle.id(),
                        pane_id,
                    },
                    ctx,
                );
                return true;
            }
        }
        false
    }

    /// Searches other windows for the given terminal view and focuses it there.
    /// (Uses the same cross-window dispatch pattern as open_notebook/open_workflow.)
    fn focus_terminal_view_in_other_window(
        &self,
        terminal_view_id: EntityId,
        ctx: &mut ViewContext<Self>,
    ) {
        let current_window = ctx.window_id();
        let result = WorkspaceRegistry::as_ref(ctx)
            .all_workspaces(ctx)
            .iter()
            .filter(|(win_id, _)| *win_id != current_window)
            .find_map(|(win_id, workspace)| {
                workspace.as_ref(ctx).tab_views().find_map(|pane_group| {
                    let pane_id = pane_group
                        .as_ref(ctx)
                        .find_pane_id_for_terminal_view(terminal_view_id, ctx)?;
                    Some((
                        *win_id,
                        PaneViewLocator {
                            pane_group_id: pane_group.id(),
                            pane_id,
                        },
                    ))
                })
            });

        if let Some((window_id, locator)) = result {
            ctx.windows().show_window_and_focus_app(window_id);
            if let Some(root_view_id) = ctx.root_view_id(window_id) {
                ctx.dispatch_action_for_view(
                    window_id,
                    root_view_id,
                    "root_view:handle_pane_navigation_event",
                    &locator,
                );
            }
        }
    }

    /// Shows the notification error in the specific pane.
    pub fn show_notification_error(
        &mut self,
        notification_error: NotificationSendError,
        pane_group_id: EntityId,
        pane_id: PaneId,
        ctx: &mut ViewContext<Self>,
    ) {
        if let Some(tab) = self
            .tabs
            .iter()
            .find(|tab_data| tab_data.pane_group.id() == pane_group_id)
        {
            tab.pane_group.update(ctx, |view, ctx| {
                view.show_notification_error(notification_error, pane_id, ctx);
            });

            ctx.notify();
        }
    }

    fn handle_prompt_editor_modal_event(
        &mut self,
        event: &PromptEditorModalEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            PromptEditorModalEvent::Close => {
                self.current_workspace_state.is_prompt_editor_open = false;
                self.focus_active_tab(ctx);
                ctx.notify();
            }
        }
    }

    fn handle_agent_toolbar_editor_modal_event(
        &mut self,
        event: &AgentToolbarEditorEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            AgentToolbarEditorEvent::Close => {
                self.current_workspace_state.is_agent_toolbar_editor_open = false;
                self.focus_active_tab(ctx);
                ctx.notify();
            }
        }
    }

    fn build_header_toolbar_editor_modal(
        ctx: &mut ViewContext<Self>,
    ) -> ViewHandle<HeaderToolbarEditorModal> {
        let modal = ctx.add_typed_action_view(HeaderToolbarEditorModal::new);
        ctx.subscribe_to_view(&modal, |me, _, event, ctx| {
            me.handle_header_toolbar_editor_modal_event(event, ctx);
        });
        modal
    }

    fn handle_header_toolbar_editor_modal_event(
        &mut self,
        event: &HeaderToolbarEditorEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            HeaderToolbarEditorEvent::Close => {
                self.current_workspace_state.is_header_toolbar_editor_open = false;
                self.focus_active_tab(ctx);
                ctx.notify();
            }
        }
    }

    fn ensure_tabs_panel_in_config(ctx: &mut ViewContext<Self>) {
        let config = TabSettings::as_ref(ctx)
            .header_toolbar_chip_selection
            .clone();
        let left = config.left_items();
        let right = config.right_items();
        let already_present = left.contains(&HeaderToolbarItemKind::TabsPanel)
            || right.contains(&HeaderToolbarItemKind::TabsPanel);
        if already_present {
            return;
        }

        let mut new_left = left;
        new_left.insert(0, HeaderToolbarItemKind::TabsPanel);
        let selection = HeaderToolbarChipSelection::Custom {
            left: new_left,
            right,
        };
        TabSettings::handle(ctx).update(ctx, |settings, ctx| {
            report_if_error!(settings
                .header_toolbar_chip_selection
                .set_value(selection, ctx));
        });
    }

    fn sync_panel_positions_from_config(&mut self, ctx: &mut ViewContext<Self>) {
        let config = TabSettings::as_ref(ctx)
            .header_toolbar_chip_selection
            .clone();
        let left_items = config.left_items();
        let tools_position = if left_items.contains(&HeaderToolbarItemKind::ToolsPanel) {
            PanelPosition::Left
        } else {
            PanelPosition::Right
        };
        let code_review_position = if left_items.contains(&HeaderToolbarItemKind::CodeReview) {
            PanelPosition::Left
        } else {
            PanelPosition::Right
        };
        self.left_panel_view.update(ctx, |view, ctx| {
            view.set_panel_position(tools_position, ctx);
        });
        self.right_panel_view.update(ctx, |view, ctx| {
            view.set_panel_position(code_review_position, ctx);
        });
    }

    fn build_header_toolbar_context_menu(
        ctx: &mut ViewContext<Self>,
    ) -> ViewHandle<Menu<WorkspaceAction>> {
        let menu = ctx.add_typed_action_view(|_| Menu::new().with_drop_shadow());
        ctx.subscribe_to_view(&menu, |me, _, event, ctx| {
            if let MenuEvent::Close { .. } = event {
                me.show_header_toolbar_context_menu = None;
                ctx.notify();
            }
        });
        menu
    }

    fn show_header_toolbar_context_menu(
        &mut self,
        position: Vector2F,
        ctx: &mut ViewContext<Self>,
    ) {
        if !FeatureFlag::ConfigurableToolbar.is_enabled() {
            return;
        }
        let items = vec![MenuItemFields::new("Re-arrange toolbar items")
            .with_on_select_action(WorkspaceAction::OpenHeaderToolbarEditor)
            .into_item()];
        self.header_toolbar_context_menu
            .update(ctx, |menu, ctx| menu.set_items(items, ctx));
        self.show_header_toolbar_context_menu = Some(position);
        ctx.focus(&self.header_toolbar_context_menu);
        ctx.notify();
    }

    fn open_header_toolbar_editor(&mut self, ctx: &mut ViewContext<Self>) {
        if !FeatureFlag::ConfigurableToolbar.is_enabled() {
            return;
        }
        self.header_toolbar_editor_modal
            .update(ctx, |modal, ctx| modal.open(ctx));
        self.close_all_overlays(ctx);
        self.current_workspace_state.is_header_toolbar_editor_open = true;
        ctx.focus(&self.header_toolbar_editor_modal);
    }

    #[cfg(feature = "local_fs")]
    fn get_active_session(&self, ctx: &mut ViewContext<Self>) -> Option<Arc<Session>> {
        let pane_group = self.active_tab_pane_group();
        pane_group
            .as_ref(ctx)
            .active_session_id(ctx)
            .and_then(|session_id| {
                pane_group
                    .as_ref(ctx)
                    .terminal_view_from_pane_id(session_id, ctx)
            })
            .and_then(|tv| {
                let tv_ref = tv.as_ref(ctx);
                let session_id = tv_ref.active_block_session_id()?;
                tv_ref.sessions_model().as_ref(ctx).get(session_id)
            })
    }

    #[cfg(not(feature = "local_fs"))]
    pub fn open_file_with_target(
        &mut self,
        _path: PathBuf,
        _target: FileTarget,
        _line_col: Option<LineAndColumnArg>,
        _code_source: CodeSource,
        _ctx: &mut ViewContext<Self>,
    ) {
    }

    #[cfg(feature = "local_fs")]
    pub fn open_file_with_target(
        &mut self,
        path: PathBuf,
        target: FileTarget,
        line_col: Option<LineAndColumnArg>,
        code_source: CodeSource,
        ctx: &mut ViewContext<Self>,
    ) {
        // Handle directories for CodeEditor(NewTab) target by opening a new terminal tab
        if path.is_dir() && matches!(target, FileTarget::CodeEditor(EditorLayout::NewTab)) {
            self.add_tab_with_pane_layout(
                PanesLayout::SingleTerminal(Box::new(NewTerminalOptions {
                    initial_directory: Some(path.clone()),
                    hide_homepage: true,
                    ..Default::default()
                })),
                Arc::new(HashMap::new()),
                None,
                ctx,
            );
            return;
        }

        match target {
            FileTarget::MarkdownViewer(layout) => {
                let session = self.get_active_session(ctx);

                self.open_file_notebook(path.clone(), session, layout, ctx);
            }
            FileTarget::EnvEditor => {
                let editor_value: Option<String> = self
                    .get_active_session(ctx)
                    .and_then(|session| session.editor().map(|s| s.to_string()));

                if let Some(ref editor_env) = editor_value {
                    if let Ok(editor) = Editor::try_from(editor_env.as_str()) {
                        crate::util::file::open_file_path_with_editor(
                            line_col,
                            path.clone(),
                            Some(editor),
                            ctx,
                        );
                        return;
                    }

                    // If we have an editor string but it's not a known Editor, we try to run it in a new pane
                    let new_pane_id =
                        self.active_tab_pane_group().update(ctx, |pane_group, ctx| {
                            pane_group.add_terminal_pane(
                                Direction::Right,
                                None, /*chosen_shell*/
                                ctx,
                            )
                        });

                    if let Some(terminal_view_handle) = self
                        .active_tab_pane_group()
                        .as_ref(ctx)
                        .terminal_view_from_pane_id(new_pane_id, ctx)
                    {
                        let editor_ref = Some(editor_env.as_str());
                        let path_clone = path.clone();
                        terminal_view_handle.update(ctx, |terminal, ctx| {
                            let editor_command =
                                crate::util::file::external_editor::generate_editor_command(
                                    &path_clone,
                                    line_col,
                                    editor_ref,
                                );
                            terminal.set_pending_command(&editor_command, ctx);
                        });
                        return;
                    } else {
                        log::error!(
                            "Could not get terminal view handle for new pane when attempting to open file with $EDITOR."
                        );
                    }
                }

                crate::util::file::open_file_path_in_external_editor(line_col, path.clone(), ctx);
            }
            FileTarget::CodeEditor(layout) => {
                let open_as_preview = false;
                self.open_code(code_source, layout, line_col, open_as_preview, &[], ctx);
            }
            FileTarget::ExternalEditor(editor) => {
                crate::util::file::open_file_path_with_editor(
                    line_col,
                    path.clone(),
                    Some(editor),
                    ctx,
                );
            }
            FileTarget::SystemDefault => {
                crate::util::file::open_file_path_with_editor(line_col, path.clone(), None, ctx);
            }
            FileTarget::SystemGeneric => {
                ctx.open_file_path(&path);
            }
        }
    }

    fn handle_left_panel_event(&mut self, event: &LeftPanelEvent, ctx: &mut ViewContext<Self>) {
        match event {
            LeftPanelEvent::FileTree(pane_group_event) => {
                let pane_group = self.active_tab_pane_group().clone();
                self.handle_file_tree_event(pane_group, pane_group_event, ctx);
            }
            LeftPanelEvent::OpenFileWithTarget {
                path,
                target,
                line_col,
            } => {
                self.open_file_with_target(
                    path.clone(),
                    target.clone(),
                    *line_col,
                    CodeSource::FileTree { path: path.clone() },
                    ctx,
                );
            }
        }
    }

    fn handle_right_panel_event(&mut self, event: RightPanelEvent, ctx: &mut ViewContext<Self>) {
        #[cfg(feature = "local_fs")]
        match event {
            RightPanelEvent::ToggleMaximize => {
                self.toggle_right_panel_maximized(ctx);
            }
            RightPanelEvent::OpenFileWithTarget {
                path,
                target,
                line_col,
            } => {
                // Exit maximized mode so the opened file is visible.
                if self
                    .active_tab_pane_group()
                    .as_ref(ctx)
                    .is_right_panel_maximized
                {
                    self.toggle_right_panel_maximized(ctx);
                }

                self.open_file_with_target(
                    path.clone(),
                    target,
                    line_col,
                    CodeSource::Link {
                        path,
                        range_start: None,
                        range_end: None,
                    },
                    ctx,
                );
            }
            RightPanelEvent::OpenFileInNewTab {
                path,
                line_and_column,
            } => {
                self.add_tab_for_code_file(path, line_and_column, ctx);
            }
            #[cfg(not(target_family = "wasm"))]
            RightPanelEvent::OpenLspLogs { log_path } => {
                self.open_lsp_logs(&log_path, ctx);
            }
        }
        #[cfg(not(feature = "local_fs"))]
        let _ = (event, ctx);
    }

    fn view_privacy_policy(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.open_url(links::PRIVACY_POLICY_URL);
    }

    fn view_user_docs(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.open_url(links::USER_DOCS_URL);
    }

    fn send_feedback(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.open_url(&links::feedback_form_url());
    }

    #[cfg(not(target_family = "wasm"))]
    fn view_logs(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.spawn(
            async { tokio::task::spawn_blocking(warp_logging::create_log_bundle_zip).await },
            |me, result, ctx| match result {
                Ok(Ok(path)) => {
                    ctx.open_file_path_in_explorer(&path);
                }
                Ok(Err(err)) => {
                    let error_message = format!("Failed to create log bundle: {err}");
                    log::error!("{error_message}");
                    me.toast_stack.update(ctx, |toast_stack, ctx| {
                        let toast = DismissibleToast::error(error_message);
                        toast_stack.add_persistent_toast(toast, ctx);
                    });
                }
                Err(err) => {
                    let error_message = format!("Failed to create log bundle: {err}");
                    log::error!("{error_message}");
                    me.toast_stack.update(ctx, |toast_stack, ctx| {
                        let toast = DismissibleToast::error(error_message);
                        toast_stack.add_persistent_toast(toast, ctx);
                    });
                }
            },
        );
    }

    fn copy_version(&mut self, version: &str, ctx: &mut ViewContext<Self>) {
        ctx.clipboard()
            .write(ClipboardContent::plain_text(version.to_string()));
    }

    /// Builds the unified new-session menu items
    /// tab bar chevron and the vertical tab bar `+` button.
    ///
    /// Order: Agent -> Terminal (sidecar) -> [tab configs] -> separator -> New worktree config (sidecar) -> New tab config.
    fn unified_new_session_menu_items(
        &self,
        ctx: &mut ViewContext<Self>,
    ) -> Vec<MenuItem<WorkspaceAction>> {
        let mut menu_items = vec![];

        let is_any_ai_enabled = AISettings::as_ref(ctx).is_any_ai_enabled(ctx);
        let ai_settings = AISettings::as_ref(ctx);
        let effective_default = ai_settings.default_session_mode(ctx);
        let default_tab_config_path = ai_settings.default_tab_config_path().to_string();
        let shortcut_label = keybinding_name_to_display_string(NEW_TAB_BINDING_NAME, ctx);

        // 1. Agent (if AI enabled)
        if is_any_ai_enabled {
            let mut agent_item = MenuItemFields::new("Agent")
                .with_on_select_action(WorkspaceAction::AddAgentTab)
                .with_icon(icons::Icon::LayoutAlt01);
            if effective_default == DefaultSessionMode::Agent {
                agent_item = agent_item.with_key_shortcut_label(shortcut_label.clone());
            }
            menu_items.push(agent_item.into_item());
        }

        // 2. Terminal (+ individual shells on Windows)
        {
            // On Windows, list the default terminal and each available shell as
            // individual top-level items (no submenu) so each gets a sidecar.
            #[cfg(target_os = "windows")]
            {
                let is_terminal_default = effective_default == DefaultSessionMode::Terminal;
                let mut terminal_item = MenuItemFields::new("Terminal")
                    .with_on_select_action(WorkspaceAction::AddTerminalTab {
                        hide_homepage: false,
                    })
                    .with_icon(icons::Icon::LayoutAlt01);
                if is_terminal_default {
                    terminal_item = terminal_item.with_key_shortcut_label(shortcut_label.clone());
                }
                menu_items.push(terminal_item.into_item());

                #[cfg(feature = "local_tty")]
                if FeatureFlag::ShellSelector.is_enabled() {
                    AvailableShells::handle(ctx).read(ctx, |model, _| {
                        for shell in model.get_available_shells() {
                            let shell_name = model.display_name_for_shell(shell);
                            let icon = shell
                                .get_valid_shell_path_and_type()
                                .and_then(|shell_launch_data| {
                                    ShellIndicatorType::try_from(&shell_launch_data).ok()
                                })
                                .map(|shell_indicator_type| shell_indicator_type.to_icon())
                                .unwrap_or(icons::Icon::Terminal);
                            let item = MenuItemFields::new(shell_name)
                                .with_on_select_action(WorkspaceAction::AddTabWithShell {
                                    shell: shell.clone(),
                                    source: AddTabWithShellSource::ShellSelectorMenu,
                                })
                                .with_icon(icon);
                            menu_items.push(item.into_item());
                        }
                    });
                }
            }

            // On other platforms, Terminal is a regular item.
            #[cfg(not(target_os = "windows"))]
            {
                let mut terminal_item = MenuItemFields::new("Terminal")
                    .with_on_select_action(WorkspaceAction::AddTerminalTab {
                        hide_homepage: false,
                    })
                    .with_icon(icons::Icon::LayoutAlt01);
                if effective_default == DefaultSessionMode::Terminal {
                    terminal_item = terminal_item.with_key_shortcut_label(shortcut_label.clone());
                }
                menu_items.push(terminal_item.into_item());
            }
        }

        // 3. Local Docker Sandbox
        if FeatureFlag::LocalDockerSandbox.is_enabled() {
            let mut docker_item = MenuItemFields::new("Local Docker Sandbox")
                .with_on_select_action(WorkspaceAction::AddDockerSandboxTab)
                .with_icon(icons::Icon::Docker);
            if effective_default == DefaultSessionMode::DockerSandbox {
                docker_item = docker_item.with_key_shortcut_label(shortcut_label.clone());
            }
            menu_items.push(docker_item.into_item());
        }

        // 4. User tab configs
        if FeatureFlag::TabConfigs.is_enabled() {
            let tab_configs = WarpConfig::as_ref(ctx).tab_configs().to_vec();

            // Count occurrences of each config name so we can disambiguate
            // duplicates in the menu (e.g. "My Tab Config", "My Tab Config (1)").
            let mut name_totals: HashMap<String, usize> = HashMap::new();
            for config in &tab_configs {
                *name_totals.entry(config.name.clone()).or_default() += 1;
            }
            let mut name_seen: HashMap<String, usize> = HashMap::new();

            for tab_config in tab_configs {
                let is_worktree = tab_config.is_worktree();
                let icon = if is_worktree {
                    icons::Icon::Dataflow02
                } else {
                    icons::Icon::LayoutAlt01
                };
                let is_default_config = effective_default == DefaultSessionMode::TabConfig
                    && tab_config
                        .source_path
                        .as_ref()
                        .is_some_and(|p| p.to_string_lossy() == default_tab_config_path);

                let display_name = if name_totals.get(&tab_config.name).copied().unwrap_or(0) > 1 {
                    let seen = name_seen.entry(tab_config.name.clone()).or_default();
                    *seen += 1;
                    if *seen == 1 {
                        tab_config.name.clone()
                    } else {
                        format!("{} ({})", tab_config.name, *seen - 1)
                    }
                } else {
                    tab_config.name.clone()
                };

                let mut item = MenuItemFields::new(display_name)
                    .with_on_select_action(WorkspaceAction::SelectTabConfig(tab_config))
                    .with_icon(icon);
                if is_default_config {
                    item = item.with_key_shortcut_label(shortcut_label.clone());
                }
                menu_items.push(item.into_item());
            }
        }

        // 5. Separator + worktree config entry + new tab config
        if FeatureFlag::TabConfigs.is_enabled() {
            menu_items.push(MenuItem::Separator);
            menu_items.push(
                MenuItemFields::new_submenu("New worktree config")
                    .with_icon(icons::Icon::Dataflow02)
                    .into_item(),
            );

            // 6. New tab config — V0: opens the TOML template.
            menu_items.push(
                MenuItemFields::new("New tab config")
                    .with_on_select_action(WorkspaceAction::SelectNewSessionMenuItem(
                        NewSessionMenuItem::CreateNewTabConfig,
                    ))
                    .with_icon(icons::Icon::Plus)
                    .into_item(),
            );
        }

        menu_items
    }

    fn open_tab_configs_menu(
        &mut self,
        position: Vector2F,
        is_vertical_tabs: bool,
        open_source: TabConfigsMenuOpenSource,
        ctx: &mut ViewContext<Self>,
    ) {
        let menu_items = self.unified_new_session_menu_items(ctx);
        ctx.update_view(&self.new_session_dropdown_menu, |context_menu, view_ctx| {
            if is_vertical_tabs {
                // Match the Figma mock width (OptionMenuItem component is 268px).
                context_menu.set_width(268.);
            } else {
                context_menu.set_width(MENU_DEFAULT_WIDTH);
            }
            context_menu.set_items(menu_items, view_ctx);
            match open_source {
                TabConfigsMenuOpenSource::KeyboardShortcut => {
                    context_menu.set_selected_by_index(0, view_ctx);
                }
                TabConfigsMenuOpenSource::Pointer => {
                    context_menu.reset_selection(view_ctx);
                }
            }
        });
        self.show_new_session_dropdown_menu = Some(position);
        ctx.focus(&self.new_session_dropdown_menu);
        ctx.notify();
    }

    pub fn open_new_session_dropdown_menu(
        &mut self,
        position: Vector2F,
        ctx: &mut ViewContext<Self>,
    ) {
        self.open_tab_configs_menu(position, false, TabConfigsMenuOpenSource::Pointer, ctx);
    }

    fn toggle_tab_configs_menu(&mut self, ctx: &mut ViewContext<Self>) {
        let use_vertical_tabs =
            FeatureFlag::VerticalTabs.is_enabled() && *TabSettings::as_ref(ctx).use_vertical_tabs;
        if self.show_new_session_dropdown_menu.is_some() {
            self.close_new_session_dropdown_menu(ctx);
            return;
        }

        if use_vertical_tabs {
            if !self.vertical_tabs_panel_open {
                self.vertical_tabs_panel_open = true;
                self.sync_window_button_visibility(ctx);
            }
            self.open_tab_configs_menu(
                Vector2F::zero(),
                true,
                TabConfigsMenuOpenSource::KeyboardShortcut,
                ctx,
            );
            return;
        }

        let position = ctx
            .element_position_by_id_at_last_frame(self.window_id, NEW_TAB_BUTTON_POSITION_ID)
            .map(|position| position.lower_left())
            .unwrap_or_else(Vector2F::zero);
        self.open_tab_configs_menu(
            position,
            false,
            TabConfigsMenuOpenSource::KeyboardShortcut,
            ctx,
        );
    }

    pub fn toggle_new_session_dropdown_menu(
        &mut self,
        position: Vector2F,
        is_vertical_tabs: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.show_new_session_dropdown_menu.is_some() {
            self.close_new_session_dropdown_menu(ctx);
            return;
        }

        self.open_tab_configs_menu(
            position,
            is_vertical_tabs,
            TabConfigsMenuOpenSource::Pointer,
            ctx,
        );
    }

    fn open_launch_config_from_menu(
        &mut self,
        new_session_menu_item: NewSessionMenuItem,
        ctx: &mut ViewContext<Self>,
    ) {
        match new_session_menu_item {
            NewSessionMenuItem::OpenLaunchConfig(launch_config) => ctx.dispatch_global_action(
                "root_view:open_launch_config",
                OpenLaunchConfigArg {
                    launch_config,
                    ui_location: LaunchConfigUiLocation::TabMenu,
                    open_in_active_window: false,
                },
            ),
            NewSessionMenuItem::OpenLaunchConfigDocs => {
                ctx.open_url("https://docs.warp.dev/terminal/sessions/launch-configurations")
            }
            #[cfg(feature = "local_fs")]
            NewSessionMenuItem::CreateNewTabConfig => {
                self.create_and_open_new_tab_config(ctx);
            }
            #[cfg(not(feature = "local_fs"))]
            NewSessionMenuItem::CreateNewTabConfig => {}
        }
    }

    /// Opens a tab config after the user has filled in (or confirmed) param values.
    fn open_tab_config_with_params(
        &mut self,
        tab_config: crate::tab_configs::TabConfig,
        param_values: HashMap<String, String>,
        worktree_branch_name: Option<&str>,
        ctx: &mut ViewContext<Self>,
    ) {
        let tab_color = tab_config.color;
        let (rendered_title, pane_template) =
            crate::tab_configs::render_tab_config(&tab_config, &param_values, worktree_branch_name);
        self.add_tab_with_pane_layout(
            PanesLayout::Template(pane_template),
            Arc::new(HashMap::new()),
            rendered_title,
            ctx,
        );
        if let Some(tab) = self.tabs.get_mut(self.active_tab_index) {
            // Apply tab color if specified, matching the launch config pattern.
            if let Some(color) = tab_color {
                tab.selected_color = SelectedTabColor::Color(color);
            }
        }
    }

    /// Opens a tab config, showing the param-fill modal when the config has parameters,
    /// or opening the tab directly when there are no parameters.
    fn open_tab_config(
        &mut self,
        tab_config: crate::tab_configs::TabConfig,
        ctx: &mut ViewContext<Self>,
    ) {
        if tab_config.params.is_empty() {
            let worktree_branch_name = self.maybe_generate_worktree_name(&tab_config);
            let param_values = tab_config.default_param_values();
            self.open_tab_config_with_params(
                tab_config,
                param_values,
                worktree_branch_name.as_deref(),
                ctx,
            );
        } else {
            // Pass the active terminal's cwd to seed the branch picker's git lookup.
            let cwd = self
                .active_session_view(ctx)
                .and_then(|view| view.as_ref(ctx).pwd())
                .map(PathBuf::from);

            let modal_title = format!("Open: {}", tab_config.name);
            self.tab_config_params_modal.view.update(ctx, |modal, ctx| {
                modal.body().update(ctx, |body, ctx| {
                    body.set_title(modal_title);
                    body.on_open(tab_config, cwd, ctx);
                });
            });
            self.tab_config_params_modal.open();
            self.current_workspace_state.is_tab_config_params_modal_open = true;
            ctx.notify();
        }
    }

    /// Writes the default tab config template to an unused path in `~/.warp/tab_configs/`
    /// and opens it respecting the user's configured editor setting.
    #[cfg(feature = "local_fs")]
    fn create_and_open_new_tab_config(&mut self, ctx: &mut ViewContext<Self>) {
        let dir = tab_configs_dir();
        if let Err(e) = std::fs::create_dir_all(&dir) {
            log::warn!("Failed to create tab_configs dir: {e:?}");
            return;
        }
        let path = find_unused_tab_config_path(&dir);
        const TEMPLATE: &str =
            include_str!("../../resources/tab_configs/new_tab_config_template.toml");
        if let Err(e) = std::fs::write(&path, TEMPLATE) {
            log::warn!("Failed to write new tab config template: {e:?}");
            return;
        }
        let settings = EditorSettings::as_ref(ctx);
        let target = resolve_file_target_with_editor_choice(
            &path,
            *settings.open_code_panels_file_editor,
            *settings.prefer_markdown_viewer,
            *settings.open_file_layout,
            None,
        );
        self.open_file_with_target(
            path.clone(),
            target,
            None,
            CodeSource::Link {
                path,
                range_start: None,
                range_end: None,
            },
            ctx,
        );
    }

    /// Snapshots the given tab's pane layout and writes it as a new tab config
    /// TOML to `~/.warp/tab_configs/`, then opens the file in the user's editor.
    #[cfg(feature = "local_fs")]
    fn save_current_tab_as_new_config(&mut self, tab_index: usize, ctx: &mut ViewContext<Self>) {
        use crate::tab_configs::session_config::{tab_config_from_pane_snapshot, write_tab_config};

        let tab = &self.tabs[tab_index];
        let snapshot = tab.pane_group.as_ref(ctx).snapshot(ctx);
        let custom_title = tab.pane_group.as_ref(ctx).custom_title(ctx);
        let color = tab.color();
        let config = tab_config_from_pane_snapshot(&snapshot, custom_title, color);

        let dir = tab_configs_dir();
        match write_tab_config(&config, &dir, "my_tab_config") {
            Ok(path) => {
                let settings = EditorSettings::as_ref(ctx);
                let target = resolve_file_target_with_editor_choice(
                    &path,
                    *settings.open_code_panels_file_editor,
                    *settings.prefer_markdown_viewer,
                    *settings.open_file_layout,
                    None,
                );
                self.open_file_with_target(
                    path.clone(),
                    target,
                    None,
                    CodeSource::Link {
                        path,
                        range_start: None,
                        range_end: None,
                    },
                    ctx,
                );
            }
            Err(e) => log::warn!("Failed to save tab config: {e:?}"),
        }
    }

    #[cfg(not(feature = "local_fs"))]
    fn save_current_tab_as_new_config(&mut self, _tab_index: usize, _ctx: &mut ViewContext<Self>) {}

    pub fn toggle_tab_right_click_menu(
        &mut self,
        tab_index: usize,
        anchor: TabContextMenuAnchor,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.show_tab_right_click_menu.is_some() {
            self.show_tab_right_click_menu = None;
            ctx.notify();
            return;
        }

        let tab = &self.tabs[tab_index];
        let menu_items = tab.menu_items(tab_index, self.tabs.len(), ctx);
        ctx.update_view(&self.tab_right_click_menu, |context_menu, view_ctx| {
            context_menu.set_items(menu_items, view_ctx);
        });
        self.show_tab_right_click_menu = Some((tab_index, anchor));
        ctx.focus(&self.tab_right_click_menu);
        ctx.notify();
    }

    pub fn toggle_vertical_tabs_pane_context_menu(
        &mut self,
        tab_index: usize,
        target: VerticalTabsPaneContextMenuTarget,
        position: Vector2F,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.show_tab_right_click_menu.is_some() {
            self.show_tab_right_click_menu = None;
            ctx.notify();
            return;
        }

        let Some(tab) = self.tabs.get(tab_index) else {
            log::warn!("Tried to open pane context menu for a missing tab");
            return;
        };
        let pane = target.locator();
        if tab.pane_group.id() != pane.pane_group_id {
            log::warn!("Tried to open pane context menu for a pane in another tab");
            return;
        }

        let pane_name_target = match target {
            VerticalTabsPaneContextMenuTarget::ClickedPane(locator) => PaneNameMenuTarget {
                locator,
                rename_label: "Rename pane",
                reset_label: "Reset pane name",
            },
            VerticalTabsPaneContextMenuTarget::ActivePane(locator) => PaneNameMenuTarget {
                locator,
                rename_label: "Rename active pane",
                reset_label: "Reset active pane name",
            },
        };
        let menu_items = tab.menu_items_with_pane_name_target(
            tab_index,
            self.tabs.len(),
            Some(pane_name_target),
            ctx,
        );

        ctx.update_view(&self.tab_right_click_menu, |context_menu, view_ctx| {
            context_menu.set_items(menu_items, view_ctx);
        });
        self.show_tab_right_click_menu = Some((tab_index, TabContextMenuAnchor::Pointer(position)));
        ctx.focus(&self.tab_right_click_menu);
        ctx.notify();
    }

    /// The tab bar overflow menu is retained for local toolbar compatibility.
    pub fn toggle_tab_bar_overflow_menu(&mut self, ctx: &mut ViewContext<Self>) {
        if self.show_tab_bar_overflow_menu {
            self.close_tab_bar_overflow_menu(ctx);
            return;
        }

        let menu_items = vec![];

        ctx.update_view(&self.tab_bar_overflow_menu, |context_menu, view_ctx| {
            context_menu.set_items(menu_items, view_ctx);
        });
        self.show_tab_bar_overflow_menu = true;
        ctx.focus(&self.tab_bar_overflow_menu);
        ctx.notify();
    }

    fn read_from_active_terminal_view<T>(
        &self,
        ctx: &AppContext,
        accessor: impl FnOnce(&TerminalView) -> T,
    ) -> Option<T> {
        self.get_pane_group_view(self.active_tab_index)
            .and_then(|view| {
                view.read(ctx, |pane_group, ctx| {
                    pane_group
                        .active_session_view(ctx)
                        .map(|terminal_view_handle| {
                            terminal_view_handle.read(ctx, |terminal, _| accessor(terminal))
                        })
                })
            })
    }

    pub fn active_terminal_id(&self, app: &AppContext) -> Option<EntityId> {
        self.read_from_active_terminal_view(app, |terminal| terminal.id())
    }

    /// Retrieves the entity id of the active current active input. This is needed
    /// by the Welcome Tip View in order to know where to dispatch the actions
    /// directly from the tip menu.
    fn active_input_id(&self, app: &AppContext) -> Option<EntityId> {
        self.read_from_active_terminal_view(app, |terminal| terminal.input().id())
    }

    /// Gets the ID of the active terminal session, if any.
    pub fn active_session_id(&self, ctx: &ViewContext<Self>) -> Option<SessionId> {
        self.get_pane_group_view(self.active_tab_index)
            .and_then(|view| {
                view.read(ctx, |pane_group, ctx| {
                    pane_group
                        .active_session_view(ctx)
                        .and_then(|terminal_view_handle| {
                            terminal_view_handle
                                .read(ctx, |terminal, _| terminal.active_block_session_id())
                        })
                })
            })
    }

    fn should_open_get_started_tab(&self, ctx: &mut ViewContext<Self>) -> bool {
        if !FeatureFlag::GetStartedTab.is_enabled() {
            return false;
        }

        let _ = ctx;
        false
    }

    fn open_get_started_tab(&mut self, ctx: &mut ViewContext<Self>) {
        self.add_get_started_tab(ctx);
    }

    fn open_settings_pane(
        &mut self,
        page: Option<SettingsSection>,
        search_query: Option<&str>,
        ctx: &mut ViewContext<Self>,
    ) {
        // Ensure there is only one settings pane per window
        let settings_pane_manager = SettingsPaneManager::handle(ctx);
        if let Some(locator) = settings_pane_manager.as_ref(ctx).find_pane(ctx.window_id()) {
            // Update to new page if specified
            if let Some(page) = page {
                self.settings_pane.update(ctx, |settings_pane, ctx| {
                    settings_pane.set_and_refresh_current_page(page, ctx);
                    if let Some(search_query) = search_query {
                        settings_pane.set_search_query(search_query, ctx);
                    }
                });
            }
            // Navigate to and focus existing pane
            self.focus_pane(locator, ctx);
            return;
        }

        let ps1_grid_info = self.active_session_ps1_grid_info(ctx);
        // Open new tab and update current page
        self.settings_pane.update(ctx, move |settings_pane, ctx| {
            // TODO: This check shouldn't be necessary, but `active_session_ps1_grid_info` returns
            // None when the active tab has no running terminal sessions, e.g. if it contains only
            // notebooks/workflow panes.
            if ps1_grid_info.is_some() {
                settings_pane.set_ps1_info(ps1_grid_info, ctx);
            }
        });

        let panes_layout = PanesLayout::Snapshot(Box::new(PaneNodeSnapshot::Leaf(LeafSnapshot {
            is_focused: true,
            custom_vertical_tabs_title: None,
            contents: LeafContents::Settings(SettingsPaneSnapshot::Local {
                current_page: page.unwrap_or_default(),
                search_query: search_query.map(|s| s.to_owned()),
            }),
        })));
        self.add_tab_with_pane_layout(
            panes_layout,
            Arc::new(HashMap::new()),
            Some("Settings".to_owned()),
            ctx,
        );
    }

    /// Open a file from the given session as a notebook pane.
    #[cfg(feature = "local_fs")]
    fn open_file_notebook(
        &mut self,
        path: PathBuf,
        session: Option<Arc<Session>>,
        layout: EditorLayout,
        ctx: &mut ViewContext<Self>,
    ) {
        let pane = FilePane::new(
            Some(path),
            session,
            #[cfg(feature = "local_fs")]
            None,
            ctx,
        );

        match layout {
            EditorLayout::NewTab => {
                let new_tab_placement_setting = TabSettings::as_ref(ctx).new_tab_placement;
                let new_idx = match new_tab_placement_setting {
                    NewTabPlacement::AfterAllTabs => self.tab_count(),
                    // Add tab after current tab
                    NewTabPlacement::AfterCurrentTab => self.active_tab_index + 1,
                };
                self.add_tab_from_existing_pane(Box::new(pane), new_idx, ctx);
            }
            EditorLayout::SplitPane => {
                self.active_tab_pane_group().update(ctx, |pane_group, ctx| {
                    pane_group.add_pane_with_direction(
                        Direction::Right,
                        pane,
                        true, /* focus_new_pane */
                        ctx,
                    );
                });
            }
        }
    }

    fn attach_path_as_context(&mut self, path: PathBuf, ctx: &mut ViewContext<Self>) {
        let Some(view) = self.active_session_view(ctx) else {
            log::warn!("No active terminal view session when trying to attach path as context");
            return;
        };

        view.update(ctx, |terminal_view, ctx| {
            terminal_view.attach_path_as_context(&path, ctx);
        });
    }

    fn cd_to_directory(&mut self, path: PathBuf, ctx: &mut ViewContext<Self>) {
        let Some(input_handle) = self.get_active_input_view_handle(ctx) else {
            log::warn!("No active input view when trying to cd to directory");
            return;
        };

        let Some(path_str) = path.to_str() else {
            log::warn!("Could not convert path to string for cd command");
            return;
        };

        let cd_command = format!("cd {}", shell_words::quote(path_str));
        input_handle.update(ctx, |input_view, ctx| {
            input_view.replace_buffer_content(&cd_command, ctx);
        });
    }

    fn open_directory_in_new_tab(&mut self, path: PathBuf, ctx: &mut ViewContext<Self>) {
        let options = NewTerminalOptions::default().with_initial_directory(path);
        self.add_tab_with_pane_layout(
            PanesLayout::SingleTerminal(Box::new(options)),
            Arc::new(HashMap::new()),
            None,
            ctx,
        );
    }

    #[cfg(feature = "local_fs")]
    fn open_code(
        &mut self,
        source: CodeSource,
        layout: EditorLayout,
        line_col: Option<LineAndColumnArg>,
        preview: bool,
        additional_paths: &[PathBuf],
        ctx: &mut ViewContext<Self>,
    ) {
        let grouping_on = FeatureFlag::TabbedEditorView.is_enabled()
            && *EditorSettings::as_ref(ctx)
                .prefer_tabbed_editor_view
                .value();

        if grouping_on {
            let code_view = self
                .active_tab_pane_group()
                .as_ref(ctx)
                .code_panes(ctx)
                .find(|(pane_id, _)| {
                    !self
                        .active_tab_pane_group()
                        .as_ref(ctx)
                        .is_pane_hidden_for_close(*pane_id)
                });
            // If the tabbed editor view is enabled and there is an existing CodeView, we should group the newly opened file into this view.
            if let (Some(path), Some((pane_id, code_view))) = (source.path(), code_view) {
                code_view.update(ctx, |code_view, ctx| {
                    if preview {
                        code_view.open_in_preview_or_promote_and_jump(path, line_col, ctx);
                    } else {
                        code_view.open_or_focus_existing(Some(path), line_col, ctx);
                    }
                    for extra in additional_paths {
                        code_view.open_or_focus_existing(Some(extra.clone()), None, ctx);
                    }
                });
                // Only focus the pane for non-preview opens
                if !preview {
                    self.active_tab_pane_group().update(ctx, |pane_group, ctx| {
                        pane_group.focus_pane(pane_id, true, ctx);
                    });
                }
                return;
            }
        } else {
            // When grouping is off, avoid opening duplicate code panes for the same file in the
            // current pane group. Instead, focus the existing pane and jump.
            if let Some(path) = source.path() {
                let pane_group_id = self.active_tab_pane_group().id();
                let existing_locator = CodeManager::handle(ctx).read(ctx, |manager, _| {
                    manager.get_locator_for_path_in_tab(pane_group_id, path.as_path())
                });

                if let Some(locator) = existing_locator {
                    self.active_tab_pane_group().update(ctx, |pane_group, ctx| {
                        pane_group.focus_pane_by_id(locator.pane_id, ctx);

                        if let Some(code_view) =
                            pane_group.code_view_from_pane_id(locator.pane_id, ctx)
                        {
                            code_view.update(ctx, |code_view, ctx| {
                                if preview {
                                    code_view.open_in_preview_or_promote_and_jump(
                                        path.clone(),
                                        line_col,
                                        ctx,
                                    );
                                } else {
                                    code_view.open_or_focus_existing(
                                        Some(path.clone()),
                                        line_col,
                                        ctx,
                                    );
                                }

                                for extra in additional_paths {
                                    code_view.open_or_focus_existing(
                                        Some(extra.clone()),
                                        None,
                                        ctx,
                                    );
                                }
                            });
                        }
                    });

                    return;
                }
            }
        }

        let pane = if preview {
            CodePane::new_preview(source, ctx)
        } else {
            CodePane::new(source, line_col, ctx)
        };

        match layout {
            EditorLayout::NewTab => {
                let new_tab_placement_setting = TabSettings::as_ref(ctx).new_tab_placement;
                let new_idx = match new_tab_placement_setting {
                    NewTabPlacement::AfterAllTabs => self.tab_count(),
                    // Add tab after current tab
                    NewTabPlacement::AfterCurrentTab => self.active_tab_index + 1,
                };
                self.add_tab_from_existing_pane(Box::new(pane), new_idx, ctx);
            }
            EditorLayout::SplitPane => {
                self.active_tab_pane_group().update(ctx, |pane_group, ctx| {
                    pane_group.add_pane_with_direction(
                        Direction::Right,
                        pane,
                        !preview, /* focus_new_pane */
                        ctx,
                    );
                });
            }
        }

        // Open any additional paths as tabs in the code view we just created.
        if !additional_paths.is_empty() {
            let code_view_handle = self
                .active_tab_pane_group()
                .as_ref(ctx)
                .code_panes(ctx)
                .find(|(pane_id, _)| {
                    !self
                        .active_tab_pane_group()
                        .as_ref(ctx)
                        .is_pane_hidden_for_close(*pane_id)
                })
                .map(|(_, view)| view);
            if let Some(code_view) = code_view_handle {
                code_view.update(ctx, |code_view, ctx| {
                    for path in additional_paths {
                        code_view.open_or_focus_existing(Some(path.clone()), None, ctx);
                    }
                });
            }
        }
    }

    /// Open a code diff view by temporarily replacing the current pane or in a new tab.
    fn open_code_diff(&mut self, view: ViewHandle<CodeDiffView>, ctx: &mut ViewContext<Self>) {
        let focused_pane_id = self
            .active_tab_pane_group()
            .as_ref(ctx)
            .focused_pane_id(ctx);
        view.update(ctx, |view, _| {
            view.set_original_pane_id(Some(focused_pane_id));
        });

        // Check if the ExpandEditToPane feature flag is enabled
        if FeatureFlag::ExpandEditToPane.is_enabled() {
            // Try to temporarily replace the current pane with the diff view
            let new_pane = CodeDiffPane::from_view(view.clone(), ctx);
            self.active_tab_pane_group().update(ctx, |pane_group, ctx| {
                if !pane_group.replace_pane(focused_pane_id, new_pane, true, ctx) {
                    // If replacement failed, remove the pane we just added and fall back
                    //pane_group.close_pane(new_pane_id, ctx);
                    log::warn!("Failed to temporarily replace pane, falling back to new tab");
                }
            });
        } else {
            // Feature flag disabled: use the original behavior of opening in a new tab
            let new_pane = CodeDiffPane::from_view(view, ctx);
            let new_tab_placement_setting = TabSettings::as_ref(ctx).new_tab_placement;

            let new_idx = match new_tab_placement_setting {
                NewTabPlacement::AfterAllTabs => self.tab_count(),
                NewTabPlacement::AfterCurrentTab => self.active_tab_index + 1,
            };
            self.add_tab_from_existing_pane(Box::new(new_pane), new_idx, ctx);
        }
    }

    /// Open the AI Fact Collection pane in a split pane (default direction is left).
    pub fn open_ai_fact_collection_pane(
        &mut self,
        direction: Option<Direction>,
        page: Option<AIFactPage>,
        ctx: &mut ViewContext<Self>,
    ) {
        let _ = (direction, page, ctx);
    }

    /// Open the Execution Profile Editor pane
    pub fn open_execution_profile_editor_pane(
        &mut self,
        direction: Option<Direction>,
        profile_id: ClientProfileId,
        ctx: &mut ViewContext<Self>,
    ) {
        let manager = ExecutionProfileEditorManager::handle(ctx);

        if let Some(locator) = manager.as_ref(ctx).find_pane(ctx.window_id(), profile_id) {
            self.focus_pane(locator, ctx);
            return;
        }

        let pane = ExecutionProfileEditorPane::new(profile_id, ctx);
        let direction = direction.unwrap_or(Direction::Right);
        self.active_tab_pane_group().update(ctx, |pane_group, ctx| {
            pane_group
                .add_pane_with_direction(direction, pane, true /* focus_new_pane */, ctx);
        });
    }

    pub(super) fn active_session_view(
        &self,
        ctx: &mut ViewContext<Self>,
    ) -> Option<ViewHandle<TerminalView>> {
        self.active_tab_pane_group()
            .read(ctx, |pane_group, ctx| pane_group.active_session_view(ctx))
    }

    pub fn toggle_welcome_tips_visiblity(&mut self, ctx: &mut ViewContext<Self>) {
        self.welcome_tips_view_state.toggle_popup();
        if self.welcome_tips_view_state.is_popup_open() {
            let input_id = self.active_input_id(ctx);
            self.welcome_tips_view.update(ctx, |tips_view, ctx| {
                tips_view.set_action_target(ctx.window_id(), input_id, ctx)
            });
        }
        ctx.focus(&self.welcome_tips_view);
        ctx.notify();
    }

    pub fn close_tab_bar_overflow_menu(&mut self, ctx: &mut ViewContext<Self>) {
        self.show_tab_bar_overflow_menu = false;
        ctx.notify();
    }

    /// Find an active session and pre-fill the input editor the Warp executable with the
    /// [`warp_cli::Command::DumpDebugInfo`] subcommand.
    fn dump_debug_info(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(exec) = std::env::current_exe()
            .ok()
            .map(|path| path.to_string_lossy().into_owned())
        {
            let command = format!("{exec} {}", warp_cli::dump_debug_info_flag());
            // Get the active session for this tab if it exists.
            let mut active_session_handle = self
                .active_tab_pane_group()
                .read(ctx, |pane_group_view, ctx| {
                    pane_group_view.active_session_view(ctx)
                });
            // A tab may not have any active session, say if it only contains notebook(s). If
            // that's the case, create a new tab.
            if active_session_handle.is_none() {
                self.add_new_session_tab_with_default_mode(
                    NewSessionSource::Tab,
                    None,
                    None,
                    None,
                    false,
                    ctx,
                );
            }
            active_session_handle = self
                .active_tab_pane_group()
                .read(ctx, |pane_group_view, ctx| {
                    pane_group_view.active_session_view(ctx)
                });
            if let Some(terminal_view_handle) = active_session_handle {
                terminal_view_handle.update(ctx, |terminal_view, ctx| {
                    terminal_view.set_pending_command(&command, ctx);
                });
            }
        }
    }

    /// Install the Warper CLI by creating a symlink in /usr/local/bin
    #[cfg(target_os = "macos")]
    fn install_cli(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.spawn(async { cli_install::install_cli() }, |view, result, ctx| {
            match result {
                Ok(_) => {
                    let command_name = ChannelState::channel().cli_command_name();
                    let message = format!("Successfully installed the Warper CLI. You can now run '{command_name}' from the command line.");
                    view.toast_stack.update(ctx, |toast_stack, ctx| {
                        let toast = DismissibleToast::success(message.to_string())
                            .with_link(
                                ToastLink::new("Learn more".to_string()).with_href(
                                    "https://docs.warp.dev/reference/cli".to_string(),
                                ),
                            );
                        toast_stack.add_ephemeral_toast(toast, ctx);
                    });
                }
                Err(error) => {
                    let error_message = format!("Failed to install Warper CLI command: {error}");
                    log::error!("{error_message}");
                    view.toast_stack.update(ctx, |toast_stack, ctx| {
                        let toast = DismissibleToast::error(error_message);
                        toast_stack.add_persistent_toast(toast, ctx);
                    });
                }
            }
        });
    }

    /// Uninstall the Warper CLI by removing the symlink from /usr/local/bin
    #[cfg(target_os = "macos")]
    fn uninstall_cli(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.spawn(
            async { cli_install::uninstall_cli() },
            |view, result, ctx| match result {
                Ok(_) => {
                    let message = "Successfully uninstalled the Warper CLI command.";
                    view.toast_stack.update(ctx, |toast_stack, ctx| {
                        let toast = DismissibleToast::success(message.to_string());
                        toast_stack.add_ephemeral_toast(toast, ctx);
                    });
                }
                Err(error) => {
                    let error_message = format!("Failed to uninstall Warper CLI command: {error}");
                    log::error!("{error_message}");
                    view.toast_stack.update(ctx, |toast_stack, ctx| {
                        let toast = DismissibleToast::error(error_message);
                        toast_stack.add_persistent_toast(toast, ctx);
                    });
                }
            },
        );
    }

    fn undo_revert_in_code_review_pane(
        &mut self,
        window_id: WindowId,
        view_id: EntityId,
        ctx: &mut ViewContext<Self>,
    ) {
        GlobalCodeReviewModel::handle(ctx).update(ctx, |global_code_review_model, ctx| {
            global_code_review_model.undo_revert_in_code_review_pane(window_id, view_id, ctx);
        });
    }

    fn toggle_recording_mode(&self, ctx: &mut ViewContext<Self>) {
        DebugSettings::handle(ctx).update(ctx, |debug_settings, settings_ctx| {
            report_if_error!(debug_settings
                .recording_mode
                .toggle_and_save_value(settings_ctx));
        });
    }

    fn toggle_in_band_generators(&self, ctx: &mut ViewContext<Self>) {
        DebugSettings::handle(ctx).update(ctx, |debug_settings, settings_ctx| {
            report_if_error!(debug_settings
                .are_in_band_generators_for_all_sessions_enabled
                .toggle_and_save_value(settings_ctx));
        });
    }

    fn toggle_debug_network_status(&self, ctx: &mut ViewContext<Self>) {
        NetworkStatus::handle(ctx).update(ctx, |network_status, network_ctx| {
            let is_reachable = network_status.is_online();
            let new_is_reachable = !is_reachable;
            if new_is_reachable {
                log::info!("Manually toggled network status to be reachable");
            } else {
                log::info!("Manually toggled network status to be not reachable");
            }
            network_status.reachability_changed(new_is_reachable, network_ctx);
        });
    }

    fn toggle_show_memory_stats(&self, ctx: &mut ViewContext<Self>) {
        DebugSettings::handle(ctx).update(ctx, |debug_settings, ctx| {
            report_if_error!(debug_settings.show_memory_stats.toggle_and_save_value(ctx));
        })
    }

    fn open_resource_center_main_page(&mut self, ctx: &mut ViewContext<Self>) {
        // Set current page to Main
        self.resource_center_view
            .update(ctx, |resource_center_view, ctx| {
                resource_center_view.set_current_page(ResourceCenterPage::Main, ctx)
            });

        // Open side panel
        self.current_workspace_state.is_resource_center_open = true;
    }

    pub fn toggle_resource_center(&mut self, ctx: &mut ViewContext<Self>) {
        if !self.current_workspace_state.is_resource_center_open {
            self.focus_active_tab(ctx);
        }

        if !self.current_workspace_state.is_resource_center_open {
            self.open_resource_center_main_page(ctx);
        } else {
            // Close side panel
            self.current_workspace_state.is_resource_center_open = false;
        }

        self.update_resource_center_action_target(ctx);
        ctx.notify();
    }

    fn open_left_panel(&mut self, ctx: &mut ViewContext<Self>) {
        self.left_panel_open = true;

        let active_pane_group = self.active_tab_pane_group().clone();
        active_pane_group.update(ctx, |pane_group, ctx| {
            pane_group.set_left_panel_open(true, ctx);
        });

        ctx.notify();
    }

    fn close_left_panel(&mut self, ctx: &mut ViewContext<Self>) {
        self.left_panel_open = false;

        let active_pane_group = self.active_tab_pane_group().clone();
        active_pane_group.update(ctx, |pane_group, ctx| {
            pane_group.set_left_panel_open(false, ctx);
        });

        ctx.notify();
    }

    fn toggle_vertical_tabs_panel(&mut self, ctx: &mut ViewContext<Self>) {
        self.vertical_tabs_panel_open = !self.vertical_tabs_panel_open;
        if !self.vertical_tabs_panel_open {
            self.close_vertical_tabs_settings_popup();
            self.vertical_tabs_panel.clear_detail_sidecar();
        }
        self.sync_window_button_visibility(ctx);
        ctx.notify();
    }

    fn close_vertical_tabs_settings_popup(&mut self) {
        self.vertical_tabs_panel.show_settings_popup = false;
    }

    fn toggle_left_panel(&mut self, ctx: &mut ViewContext<Self>) {
        let active_pane_group = self.active_tab_pane_group().clone();

        let was_open = active_pane_group.read(ctx, |pane_group, _| pane_group.left_panel_open);
        let new_state = !was_open;

        if new_state {
            self.open_left_panel(ctx);
        } else {
            self.close_left_panel(ctx);
        }

        // If we are opening the panel, set width based on the most recent tab's width if available,
        // otherwise compute default width from current window size. Also auto-expand the project
        // explorer if it's the active left panel view.
        if new_state {
            let window_id = ctx.window_id();
            let resizable_data = ResizableData::handle(ctx);
            if let Some(handle) = resizable_data
                .as_ref(ctx)
                .get_handle(window_id, ModalType::LeftPanelWidth)
            {
                if let Ok(mut state) = handle.lock() {
                    // Get the current width from ResizableData - this reflects the most recent tab's width
                    let current_width = state.size();

                    // Only recompute default if the current width is at the default value
                    // This preserves the width from the most recent tab
                    if current_width == DEFAULT_LEFT_PANEL_WIDTH {
                        let has_horizontal_split = active_pane_group
                            .read(ctx, |pane_group, _| pane_group.has_horizontal_split());
                        let (left_width, _right_width) =
                            compute_default_panel_widths(ctx, window_id, has_horizontal_split);
                        state.set_size(left_width);
                    }
                    // If current_width is not the default, it means we have a width from a previous tab,
                    // so we don't need to do anything - the width is already preserved
                }
            }

            // Auto-expand the file tree when the left panel is opened and the project explorer is
            // the active view.
            let file_tree_active = self
                .left_panel_view
                .read(ctx, |lp, _| lp.is_file_tree_active());
            if file_tree_active {
                self.left_panel_view.update(ctx, |left_panel, ctx| {
                    left_panel.auto_expand_active_file_tree_to_most_recent_directory(ctx);
                });
            }
        }

        if !new_state {
            self.focus_active_tab(ctx);
        }

        ctx.notify();
    }

    #[cfg(feature = "local_fs")]
    fn setup_code_review_panel(
        &mut self,
        context: Option<&CodeReviewPaneContext>,
        ctx: &mut ViewContext<Self>,
    ) {
        if !*TabSettings::as_ref(ctx).show_code_review_button {
            return;
        }

        // If context is provided, use it directly. Otherwise, derive from active pane group.
        let context_data: Option<(
            Option<PathBuf>,
            ModelHandle<DiffStateModel>,
            WeakViewHandle<TerminalView>,
        )> = if let Some(context) = context {
            Some((
                context.repo_path.clone(),
                context.diff_state_model.clone(),
                context.terminal_view.clone(),
            ))
        } else {
            let active_pane_group = self.active_tab_pane_group().clone();
            // Read repo_path and terminal_view from the pane group (immutable context).
            let read_result = active_pane_group.read(ctx, |pane_group, ctx| {
                pane_group.active_session_view(ctx).map(|terminal_view| {
                    let repo_path = terminal_view.as_ref(ctx).current_repo_path().cloned();
                    (repo_path, terminal_view.downgrade())
                })
            });
            // Resolve DiffStateModel outside the read closure (needs mutable context).
            read_result.and_then(
                |(repo_path, terminal_view): (Option<PathBuf>, WeakViewHandle<TerminalView>)| {
                    let diff_state_model = repo_path.as_ref().and_then(|rp: &PathBuf| {
                        self.working_directories_model.update(ctx, |model, ctx| {
                            model.get_or_create_diff_state_model(rp.clone(), ctx)
                        })
                    })?;
                    Some((repo_path, diff_state_model, terminal_view))
                },
            )
        };

        if let Some((repo, diff_state_model, terminal_view)) = context_data {
            self.right_panel_view.update(ctx, |right_pane_view, ctx| {
                right_pane_view.open_code_review(
                    repo.clone(),
                    diff_state_model,
                    terminal_view,
                    ctx,
                );
            });
        } else {
            self.right_panel_view.update(ctx, |right_panel_view, ctx| {
                right_panel_view.close_code_review(ctx);
            })
        }
    }

    fn open_code_review_panel_from_arg(
        &mut self,
        panel_context: &CodeReviewPanelArg,
        pane_group: ViewHandle<PaneGroup>,
        ctx: &mut ViewContext<Self>,
    ) {
        // Skip the full panel setup when the panel is already open for the target repo.
        let panel_already_showing_repo = pane_group.as_ref(ctx).right_panel_open
            && panel_context
                .repo_path
                .as_ref()
                .is_some_and(|target_repo_path| {
                    self.right_panel_view.as_ref(ctx).selected_repo_path() == Some(target_repo_path)
                });
        if panel_already_showing_repo {
            return;
        }

        let repo_path = panel_context.repo_path.clone();
        let diff_state_model = repo_path.as_ref().and_then(|rp| {
            self.working_directories_model.update(ctx, |model, ctx| {
                model.get_or_create_diff_state_model(rp.clone(), ctx)
            })
        });
        let Some(diff_state_model) = diff_state_model else {
            return;
        };
        let context = CodeReviewPaneContext {
            repo_path,
            diff_state_model,
            terminal_view: panel_context.terminal_view.clone(),
        };

        self.open_right_panel(&context, &pane_group, ctx);

        let active_conversation_id = panel_context
            .terminal_view
            .upgrade(ctx)
            .and_then(|tv| BlocklistAIHistoryModel::as_ref(ctx).active_conversation_id(tv.id()));

        if let Some(conversation_id) = active_conversation_id {
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, _| {
                history_model.set_has_code_review_opened_to_true(conversation_id);
            });
        }
    }

    fn update_right_panel_open_state(
        &mut self,
        #[cfg_attr(target_family = "wasm", allow(unused_variables))]
        panel_update_params: RightPanelUpdateParams,
        ctx: &mut ViewContext<Self>,
    ) {
        let should_open = panel_update_params.target_open_state;
        let should_close = !panel_update_params.target_open_state;

        let new_is_maximized = panel_update_params.pane_group.update(ctx, |pane_group, _| {
            pane_group.right_panel_open = should_open;
            pane_group.is_right_panel_maximized
        });

        self.right_panel_view.update(ctx, |view, ctx| {
            view.set_maximized(new_is_maximized, ctx);
            if should_close {
                view.close_code_review(ctx);
            }
        });

        if should_open {
            #[cfg(feature = "local_fs")]
            {
                let window_id = ctx.window_id();
                let resizable_data = ResizableData::handle(ctx);
                if let Some(handle) = resizable_data
                    .as_ref(ctx)
                    .get_handle(window_id, ModalType::RightPanelWidth)
                {
                    if let Ok(mut state) = handle.lock() {
                        // Get the current width from ResizableData - this reflects the most recent tab's width
                        let current_width = state.size();

                        // Only recompute default if the current width is at the default value
                        // This preserves the width from the most recent tab
                        if current_width == DEFAULT_RIGHT_PANEL_WIDTH {
                            let has_horizontal_split = panel_update_params
                                .pane_group
                                .read(ctx, |pane_group, _| pane_group.has_horizontal_split());
                            let (_left_width, right_width) =
                                compute_default_panel_widths(ctx, window_id, has_horizontal_split);
                            state.set_size(right_width);
                        }
                        // If current_width is not the default, it means we have a width from a previous tab,
                        // so we don't need to do anything - the width is already preserved
                    }
                }
                self.setup_code_review_panel(panel_update_params.review_pane_context, ctx);
            }
        } else {
            self.focus_active_tab(ctx);
        }

        ctx.notify();
    }

    fn toggle_right_panel(
        &mut self,
        pane_group_handle: &ViewHandle<PaneGroup>,
        ctx: &mut ViewContext<Self>,
    ) {
        let target_open_state =
            pane_group_handle.read(ctx, |pane_group, _| !pane_group.right_panel_open);

        // Read repo_path and terminal_view from pane group (immutable context).
        let read_result = pane_group_handle.read(ctx, |pane_group, ctx| {
            pane_group.active_session_view(ctx).map(|terminal_view| {
                let repo_path = terminal_view.as_ref(ctx).current_repo_path().cloned();
                (repo_path, terminal_view.downgrade())
            })
        });
        // Resolve DiffStateModel outside the read closure (needs mutable context).
        let context = read_result.and_then(
            |(repo_path, terminal_view): (Option<PathBuf>, WeakViewHandle<TerminalView>)| {
                let diff_state_model = repo_path.as_ref().and_then(|rp: &PathBuf| {
                    self.working_directories_model.update(ctx, |model, ctx| {
                        model.get_or_create_diff_state_model(rp.clone(), ctx)
                    })
                })?;
                Some(CodeReviewPaneContext {
                    repo_path,
                    diff_state_model,
                    terminal_view,
                })
            },
        );

        self.update_right_panel_open_state(
            RightPanelUpdateParams {
                pane_group: pane_group_handle,
                target_open_state,
                review_pane_context: context.as_ref(),
            },
            ctx,
        );
    }

    #[cfg(feature = "local_fs")]
    fn open_right_panel(
        &mut self,
        context: &CodeReviewPaneContext,
        pane_group_handle: &ViewHandle<PaneGroup>,
        ctx: &mut ViewContext<Self>,
    ) {
        if pane_group_handle.as_ref(ctx).right_panel_open {
            if let Some(repo_path) = &context.repo_path {
                self.right_panel_view.update(ctx, |right_panel, ctx| {
                    right_panel.update_selected_repo(repo_path.clone(), ctx);
                });
            }
            return;
        }

        self.update_right_panel_open_state(
            RightPanelUpdateParams {
                pane_group: pane_group_handle,
                target_open_state: true,
                review_pane_context: Some(context),
            },
            ctx,
        );
        if let Some(repo_path) = &context.repo_path {
            self.right_panel_view.update(ctx, |right_panel, ctx| {
                right_panel.update_selected_repo(repo_path.clone(), ctx);
            });
        }
    }

    #[cfg(not(feature = "local_fs"))]
    fn open_right_panel(
        &mut self,
        _context: &CodeReviewPaneContext,
        _pane_group_handle: &ViewHandle<PaneGroup>,
        _ctx: &mut ViewContext<Self>,
    ) {
    }

    pub fn close_right_panel(
        &mut self,
        pane_group_handle: &ViewHandle<PaneGroup>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.update_right_panel_open_state(
            RightPanelUpdateParams {
                pane_group: pane_group_handle,
                target_open_state: false,
                review_pane_context: None,
            },
            ctx,
        );
    }

    #[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
    fn toggle_right_panel_maximized(&mut self, ctx: &mut ViewContext<Self>) {
        let pane_group = self.active_tab_pane_group().clone();
        let is_maximized = pane_group.update(ctx, |pane_group, _| {
            pane_group.is_right_panel_maximized = !pane_group.is_right_panel_maximized;
            pane_group.is_right_panel_maximized
        });

        self.right_panel_view.update(ctx, |view, ctx| {
            view.set_maximized(is_maximized, ctx);
            if is_maximized {
                view.focus_active_code_review_view(ctx);
            }
        });
        if !is_maximized {
            self.focus_active_tab(ctx);
        }
        ctx.notify();
    }

    fn user_menu_items(&self, _app: &AppContext) -> Vec<MenuItem<WorkspaceAction>> {
        let mut items: Vec<MenuItem<WorkspaceAction>> = vec![
            MenuItemFields::new("Settings")
                .with_on_select_action(WorkspaceAction::ShowSettings)
                .into_item(),
            MenuItemFields::new("Keyboard shortcuts")
                .with_on_select_action(WorkspaceAction::ToggleKeybindingsPage)
                .into_item(),
            MenuItem::Separator,
            MenuItemFields::new("Documentation")
                .with_on_select_action(WorkspaceAction::ViewUserDocs)
                .into_item(),
        ];

        #[cfg(not(target_family = "wasm"))]
        items.push(
            MenuItemFields::new("View Warper logs")
                .with_on_select_action(WorkspaceAction::ViewLogs)
                .into_item(),
        );

        items
    }

    fn selected_new_session_sidecar_selection(
        &self,
        ctx: &AppContext,
    ) -> Option<NewSessionSidecarSelection> {
        self.new_session_sidecar_menu.read(ctx, |menu, _| {
            menu.selected_item().and_then(|item| match item {
                MenuItem::Item(fields) => fields.on_select_action().cloned(),
                _ => None,
            })
        })
    }

    fn execute_new_session_sidecar_selection(
        &mut self,
        selection: NewSessionSidecarSelection,
        ctx: &mut ViewContext<Self>,
    ) {
        match selection {
            NewSessionSidecarSelection::OpenWorktreeRepo { repo_path } => {
                self.open_worktree_in_repo(repo_path, ctx);
            }
        }
    }

    fn toggle_user_menu(&mut self, ctx: &mut ViewContext<Self>) {
        self.is_user_menu_open = !self.is_user_menu_open;
        if self.is_user_menu_open {
            let items = self.user_menu_items(ctx);
            self.user_menu.update(ctx, |menu, ctx| {
                menu.set_items(items, ctx);
            });
        }
        ctx.focus(&self.user_menu);
        ctx.notify();
    }

    pub fn toggle_keybindings_page(&mut self, ctx: &mut ViewContext<Self>) {
        let current_page = self
            .resource_center_view
            .read(ctx, |resource_center_view, _ctx| {
                resource_center_view.get_current_page()
            });

        if !self.current_workspace_state.is_resource_center_open {
            // Set current page to Keybindings
            self.resource_center_view
                .update(ctx, |resource_center_view, ctx| {
                    resource_center_view.set_current_page(ResourceCenterPage::Keybindings, ctx)
                });

            // Open side panel
            self.current_workspace_state.is_resource_center_open = true;
        } else if current_page != ResourceCenterPage::Keybindings
            && self.current_workspace_state.is_resource_center_open
        {
            // Navigate to keybindings page
            self.resource_center_view
                .update(ctx, |resource_center_view, ctx| {
                    resource_center_view.set_current_page(ResourceCenterPage::Keybindings, ctx)
                });
        } else {
            // Close side panel
            self.current_workspace_state.is_resource_center_open = false;
            self.focus_active_tab(ctx);
        }

        ctx.notify();
    }

    fn update_resource_center_action_target(&mut self, ctx: &mut ViewContext<Self>) {
        if self.current_workspace_state.is_resource_center_open {
            let input_id = self.active_input_id(ctx);
            self.resource_center_view
                .update(ctx, |resource_center_view, ctx| {
                    resource_center_view.set_action_target(ctx.window_id(), input_id, ctx)
                });
        }
    }

    fn handle_tab_right_click_menu_event(
        &mut self,
        event: &MenuEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        if let MenuEvent::Close { via_select_item: _ } = event {
            self.show_tab_right_click_menu = None;
            ctx.notify();
        }
    }

    fn handle_new_session_menu_event(&mut self, event: &MenuEvent, ctx: &mut ViewContext<Self>) {
        match event {
            MenuEvent::Close { .. } => {
                self.close_new_session_dropdown_menu(ctx);
            }
            MenuEvent::ItemHovered => {
                self.update_new_session_sidecar(ctx);
            }
            MenuEvent::ItemSelected => {
                self.update_new_session_sidecar(ctx);
            }
        }
    }

    fn handle_new_session_sidecar_event(&mut self, event: &MenuEvent, ctx: &mut ViewContext<Self>) {
        match event {
            MenuEvent::Close { via_select_item } => {
                let selection = if *via_select_item {
                    self.selected_new_session_sidecar_selection(ctx)
                } else {
                    None
                };
                log::info!(
                    "New-session sidecar closed: worktree_active={}, via_select_item={via_select_item}",
                    self.worktree_sidecar_active
                );
                if let Some(selection) = selection {
                    self.execute_new_session_sidecar_selection(selection, ctx);
                }
                if *via_select_item {
                    // Item clicked in sidecar — also close the main menu.
                    self.show_new_session_dropdown_menu = None;
                }
                self.clear_worktree_sidecar_state(ctx);
                self.new_session_dropdown_menu.update(ctx, |menu, _| {
                    menu.set_safe_zone_target(None);
                    menu.set_submenu_being_shown_for_item_index(None);
                });
                ctx.notify();
            }
            MenuEvent::ItemSelected => {}
            MenuEvent::ItemHovered => {
                self.sync_new_session_sidecar_selection_to_hover(ctx);
            }
        }
    }

    fn should_include_worktree_sidecar_repo(repo_path: &Path, ctx: &AppContext) -> bool {
        // This performs one repo-metadata lookup per persisted workspace while the
        // sidecar items are rebuilt. That's acceptable for now given the expected
        // repo counts here, and it keeps linked-worktree filtering scoped to the
        // only UI that currently needs it.
        let Some(repository) =
            DetectedRepositories::as_ref(ctx).get_watched_repo_for_path(repo_path, ctx)
        else {
            return true;
        };
        // Linked worktrees (and submodules) have an external gitdir; exclude
        // them so only primary repository checkouts appear in the list.

        repository.as_ref(ctx).external_git_directory().is_none()
    }

    fn build_worktree_sidecar_items(
        &self,
        ctx: &AppContext,
    ) -> Vec<MenuItem<NewSessionSidecarSelection>> {
        let search_editor = self.worktree_sidecar_search_editor.clone();
        let search_item = MenuItemFields::new_with_custom_label(
            Arc::new(move |_, _, appearance, _| {
                let theme = appearance.theme();
                let search_icon = ConstrainedBox::new(
                    icons::Icon::SearchSmall
                        .to_warpui_icon(theme.sub_text_color(theme.surface_2()))
                        .finish(),
                )
                .with_width(16.)
                .with_height(16.)
                .finish();
                let search_row = Flex::row()
                    .with_child(Container::new(search_icon).with_margin_right(8.).finish())
                    .with_child(
                        Shrinkable::new(1., ChildView::new(&search_editor).finish()).finish(),
                    )
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_main_axis_size(MainAxisSize::Max)
                    .finish();

                ConstrainedBox::new(
                    Container::new(search_row)
                        .with_padding_left(NEW_SESSION_SIDECAR_SEARCH_BOX_HORIZONTAL_PADDING)
                        .with_padding_right(NEW_SESSION_SIDECAR_SEARCH_BOX_HORIZONTAL_PADDING)
                        .with_padding_top(NEW_SESSION_SIDECAR_SEARCH_BOX_VERTICAL_PADDING)
                        .with_padding_bottom(NEW_SESSION_SIDECAR_SEARCH_BOX_VERTICAL_PADDING)
                        .with_border(Border::all(1.).with_border_fill(theme.surface_3()))
                        .with_corner_radius(CornerRadius::with_top(Radius::Pixels(4.)))
                        .finish(),
                )
                .with_height(NEW_SESSION_SIDECAR_SEARCH_BOX_HEIGHT)
                .finish()
            }),
            Some("Search repos".to_string()),
        )
        .with_no_interaction_on_hover()
        .no_highlight_on_hover()
        .with_padding_override(0., 0.)
        .into_item();
        let query = self.worktree_sidecar_search_query.trim().to_lowercase();
        let mut items = vec![search_item];
        items.extend(
            PersistedWorkspace::as_ref(ctx)
                .workspaces()
                .filter(|ws| ws.path.exists())
                .filter(|ws| Self::should_include_worktree_sidecar_repo(&ws.path, ctx))
                .filter(|ws| {
                    if query.is_empty() {
                        true
                    } else {
                        ws.path
                            .to_string_lossy()
                            .to_lowercase()
                            .contains(query.as_str())
                    }
                })
                .map(|ws| {
                    let path_str = ws.path.to_string_lossy().into_owned();
                    MenuItemFields::new(path_str.clone())
                        .with_on_select_action(NewSessionSidecarSelection::OpenWorktreeRepo {
                            repo_path: path_str,
                        })
                        .with_icon(icons::Icon::Folder)
                        .into_item()
                })
                .collect::<Vec<_>>(),
        );
        items
    }

    fn configure_worktree_new_session_sidecar(
        &mut self,
        hovered_index: usize,
        auto_select_first_repo: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        let items = self.build_worktree_sidecar_items(ctx);
        let repo_count = items.len().saturating_sub(1);
        log::info!(
            "Configuring worktree sidecar: hovered_index={hovered_index}, query={:?}, repo_count={repo_count}",
            self.worktree_sidecar_search_query
        );
        let add_repo_mouse_state = self.new_session_sidecar_add_repo_mouse_state.clone();

        self.new_session_sidecar_menu.update(ctx, |menu, view_ctx| {
            menu.set_items(items, view_ctx);
            menu.clear_pinned_header_builder();
            menu.set_content_padding_overrides(Some(0.), None);
            menu.set_pinned_footer_builder(move |app| {
                let appearance = Appearance::as_ref(app);
                let theme = appearance.theme();
                let font_family = appearance.ui_font_family();
                let font_size = appearance.ui_font_size();
                let border_fill = theme.outline();
                let mouse_state = add_repo_mouse_state.clone();
                Hoverable::new(mouse_state, move |state| {
                    let bg = if state.is_hovered() {
                        theme.accent_button_color()
                    } else {
                        theme.surface_2()
                    };
                    let text_color = theme.main_text_color(bg);
                    ConstrainedBox::new(
                        Container::new(
                            Flex::row()
                                .with_main_axis_size(MainAxisSize::Max)
                                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                                .with_child(
                                    Text::new_inline(" + Add new repo", font_family, font_size)
                                        .with_color(text_color.into())
                                        .finish(),
                                )
                                .finish(),
                        )
                        .with_padding_left(NEW_SESSION_SIDECAR_FOOTER_HORIZONTAL_PADDING)
                        .with_padding_right(NEW_SESSION_SIDECAR_FOOTER_HORIZONTAL_PADDING)
                        .with_padding_top(NEW_SESSION_SIDECAR_FOOTER_VERTICAL_PADDING)
                        .with_padding_bottom(NEW_SESSION_SIDECAR_FOOTER_VERTICAL_PADDING)
                        .with_background(bg)
                        .with_border(Border::top(1.).with_border_fill(border_fill))
                        .with_corner_radius(CornerRadius::with_bottom(Radius::Pixels(5.)))
                        .finish(),
                    )
                    .with_width(NEW_SESSION_SIDECAR_WIDTH)
                    .finish()
                })
                .with_cursor(Cursor::PointingHand)
                .on_click(|ctx: &mut warpui::elements::EventContext, _, _| {
                    ctx.dispatch_typed_action(WorkspaceAction::OpenWorktreeAddRepoPicker);
                    ctx.dispatch_typed_action(crate::menu::MenuAction::Close(true));
                })
                .finish()
            });
        });
        if auto_select_first_repo {
            self.select_first_worktree_sidecar_repo(ctx);
        } else {
            self.reset_worktree_sidecar_repo_selection(ctx);
        }

        self.worktree_sidecar_active = true;
        self.show_new_session_sidecar = true;
        let sidecar_rect = ctx
            .element_position_by_id_at_last_frame(self.window_id, NEW_SESSION_SIDECAR_POSITION_ID);
        log::info!(
            "Worktree sidecar safe-zone target from previous frame available: {}",
            sidecar_rect.is_some()
        );
        self.new_session_dropdown_menu.update(ctx, |menu, _| {
            menu.set_safe_zone_target(sidecar_rect);
            menu.set_submenu_being_shown_for_item_index(Some(hovered_index));
        });
        ctx.focus(&self.worktree_sidecar_search_editor);
    }

    fn configure_action_sidecar_for_hovered_item(
        &mut self,
        label: &str,
        hovered_index: usize,
        ctx: &mut ViewContext<Self>,
    ) {
        // Determine the SidecarItemKind from the hovered menu item's label and action.
        let hovered_action = self.new_session_dropdown_menu.read(ctx, |menu, _| {
            menu.items().get(hovered_index).and_then(|item| match item {
                MenuItem::Item(fields) => fields.on_select_action().cloned(),
                _ => None,
            })
        });

        let item_kind = match &hovered_action {
            Some(WorkspaceAction::SelectTabConfig(config)) => SidecarItemKind::UserTabConfig {
                config: config.clone(),
            },
            Some(WorkspaceAction::AddAgentTab) => SidecarItemKind::BuiltIn {
                name: label.to_string(),
                default_mode: DefaultSessionMode::Agent,
                shell: None,
            },
            Some(WorkspaceAction::AddTerminalTab { .. }) => SidecarItemKind::BuiltIn {
                name: label.to_string(),
                default_mode: DefaultSessionMode::Terminal,
                shell: None,
            },
            Some(WorkspaceAction::AddTabWithShell { shell, .. }) => SidecarItemKind::BuiltIn {
                name: label.to_string(),
                default_mode: DefaultSessionMode::Terminal,
                shell: Some(shell.clone()),
            },
            Some(WorkspaceAction::AddDockerSandboxTab) => SidecarItemKind::BuiltIn {
                name: label.to_string(),
                default_mode: DefaultSessionMode::DockerSandbox,
                shell: None,
            },
            _ => {
                // Hovered item has no associated sidecar. Clear any stale
                // sidecar state left over from a previously-hovered item so
                // the menu doesn't keep rendering that item as the
                // submenu-parent highlight.
                self.tab_config_action_sidecar_item = None;
                self.new_session_dropdown_menu.update(ctx, |menu, _| {
                    menu.set_safe_zone_target(None);
                    menu.set_submenu_being_shown_for_item_index(None);
                });
                return;
            }
        };

        self.tab_config_action_sidecar_item = Some(item_kind);

        let sidecar_rect = ctx
            .element_position_by_id_at_last_frame(self.window_id, NEW_SESSION_SIDECAR_POSITION_ID);
        self.new_session_dropdown_menu.update(ctx, |menu, _| {
            menu.set_safe_zone_target(sidecar_rect);
            menu.set_submenu_being_shown_for_item_index(Some(hovered_index));
        });
    }

    /// Returns `true` when a sidecar of the given width should render on the left
    /// of the menu (because it would overflow the window on the right).
    fn should_render_sidecar_left(
        &self,
        anchor_label: &str,
        sidecar_width: f32,
        app: &AppContext,
    ) -> bool {
        let Some(window) = app.windows().platform_window(self.window_id) else {
            return false;
        };
        let Some(anchor_rect) =
            app.element_position_by_id_at_last_frame(self.window_id, anchor_label)
        else {
            return false;
        };

        let gap = 4.0;

        let would_overflow_right = anchor_rect.max_x() + gap + sidecar_width >= window.size().x();
        let would_overflow_left = anchor_rect.min_x() - gap - sidecar_width < 0.0;

        match (would_overflow_left, would_overflow_right) {
            (true, false) => false, // Only right fits
            (false, true) => true,  // Only left fits
            _ => false,             // Default to right
        }
    }

    fn refresh_worktree_sidecar_if_active(&mut self, ctx: &mut ViewContext<Self>) {
        if !self.worktree_sidecar_active {
            return;
        }
        let Some(hovered_index) = self
            .new_session_dropdown_menu
            .read(ctx, |menu, _| menu.hovered_index())
        else {
            return;
        };
        self.configure_worktree_new_session_sidecar(hovered_index, true, ctx);
    }

    /// Updates the sidecar menu based on which item is hovered in the main
    /// new-session dropdown. If the hovered item is a submenu parent (Terminal
    /// or New worktree config), populates the sidecar with the appropriate items.
    fn update_new_session_sidecar(&mut self, ctx: &mut ViewContext<Self>) {
        // Use hovered_index (not selected_index) as the source of truth.
        // hovered_row_index accurately tracks the mouse position and survives
        // reset_selection (which only clears selected_row/item indices).
        // selected_index can get stuck on a submenu parent when
        // UnhoverSubmenuParent resets the selection.
        let hovered_index = self
            .new_session_dropdown_menu
            .read(ctx, |menu, _| menu.hovered_index());

        // If hovered is None the mouse has left the menu (possibly onto the
        // sidecar) or is on a non-hoverable element. Keep current state.
        let Some(hovered_index) = hovered_index else {
            return;
        };

        // Check what the hovered item is by reading its label.
        let hovered_label = self.new_session_dropdown_menu.read(ctx, |menu, _| {
            menu.items().get(hovered_index).and_then(|item| match item {
                MenuItem::Item(fields) => Some(fields.label().to_string()),
                _ => None,
            })
        });

        // Separator or non-labeled item — hide sidecar.
        let Some(label) = hovered_label else {
            if self.show_new_session_sidecar {
                self.show_new_session_sidecar = false;
                self.new_session_dropdown_menu.update(ctx, |menu, _| {
                    menu.set_safe_zone_target(None);
                    menu.set_submenu_being_shown_for_item_index(None);
                });
                ctx.notify();
            }
            return;
        };

        match label.as_str() {
            "New worktree config" => {
                self.tab_config_action_sidecar_item = None;
                let auto_select_first_repo = self.new_session_dropdown_menu.read(ctx, |menu, _| {
                    menu.last_selection_source() != Some(MenuSelectionSource::Pointer)
                });
                self.configure_worktree_new_session_sidecar(
                    hovered_index,
                    auto_select_first_repo,
                    ctx,
                );
            }
            // Items that don't get any sidecar.
            "New tab config" => {
                self.tab_config_action_sidecar_item = None;
                if self.show_new_session_sidecar {
                    self.show_new_session_sidecar = false;
                    self.worktree_sidecar_active = false;
                    self.new_session_dropdown_menu.update(ctx, |menu, _| {
                        menu.set_safe_zone_target(None);
                        menu.set_submenu_being_shown_for_item_index(None);
                    });
                }
            }
            // All other actionable items get the action sidecar.
            _ => {
                self.show_new_session_sidecar = false;
                self.worktree_sidecar_active = false;
                self.configure_action_sidecar_for_hovered_item(&label, hovered_index, ctx);
            }
        }

        ctx.notify();
    }

    fn handle_tab_bar_overflow_menu_event(
        &mut self,
        event: &MenuEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        if let MenuEvent::Close { via_select_item: _ } = event {
            self.close_tab_bar_overflow_menu(ctx)
        }
    }

    fn handle_launch_config_save_modal_event(
        &mut self,
        event: &LaunchConfigModalEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            LaunchConfigModalEvent::Close => {
                self.current_workspace_state
                    .is_launch_config_save_modal_open = false;
                self.launch_config_save_modal.close();
                ctx.notify();
            }
            LaunchConfigModalEvent::SuccessfullySavedConfig(launch_config) => {
                ctx.update_model(&WarpConfig::handle(ctx), move |warp_config, ctx| {
                    warp_config.append_launch_config(launch_config, ctx);
                });
                ctx.notify();
            }
            #[cfg(feature = "local_fs")]
            LaunchConfigModalEvent::OpenFileWithTarget {
                path,
                target,
                line_col,
            } => {
                self.open_file_with_target(
                    path.clone(),
                    target.clone(),
                    *line_col,
                    CodeSource::Link {
                        path: path.clone(),
                        range_start: None,
                        range_end: None,
                    },
                    ctx,
                );
            }
        }
    }

    fn handle_tab_config_params_modal_event(
        &mut self,
        event: &ModalEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            ModalEvent::Close => {
                self.cancel_tab_config_params_modal(ctx);
            }
        }
    }

    /// Cleans up pending state and closes the tab-config params modal without
    /// creating a tab config. Used when the modal is dismissed or cancelled.
    fn cancel_tab_config_params_modal(&mut self, ctx: &mut ViewContext<Self>) {
        self.pending_session_config_replacement = None;
        self.pending_session_config_tab_config_chip = false;
        self.close_tab_config_params_modal(ctx);
    }

    fn handle_tab_config_params_modal_body_event(
        &mut self,
        event: &TabConfigParamsModalEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            TabConfigParamsModalEvent::Submit { config, params } => {
                let should_track_existing_config_open =
                    self.pending_session_config_replacement.is_none();
                let worktree_name = self.maybe_generate_worktree_name(config);
                self.open_tab_config_with_params(
                    config.as_ref().clone(),
                    params.clone(),
                    worktree_name.as_deref(),
                    ctx,
                );
                let _ = should_track_existing_config_open;
                self.close_tab_config_params_modal(ctx);
                self.complete_pending_session_config_replacement(ctx);

                // Params modal is now closed; show the chip if it was pending.
                self.promote_session_config_tab_config_chip(ctx);
            }
            TabConfigParamsModalEvent::Close => {
                self.cancel_tab_config_params_modal(ctx);
            }
            TabConfigParamsModalEvent::PickNewRepo { param_index } => {
                ctx.dispatch_typed_action_deferred(WorkspaceAction::OpenTabConfigRepoPicker {
                    param_index: *param_index,
                });
            }
        }
    }

    /// Finishes the tab replacement that was deferred while the params modal
    /// was open (worktree flow from the session config modal).
    fn complete_pending_session_config_replacement(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(pending) = self.pending_session_config_replacement.take() else {
            return;
        };

        self.remove_tab_by_pane_group_id(pending.old_pane_group_id, ctx);
    }

    /// Removes the tab whose pane group matches `pane_group_id`, if it exists
    /// and there is more than one tab.
    fn remove_tab_by_pane_group_id(
        &mut self,
        pane_group_id: EntityId,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.tabs.len() <= 1 {
            return;
        }
        if let Some(index) = self
            .tabs
            .iter()
            .position(|tab| tab.pane_group.id() == pane_group_id)
        {
            self.remove_tab(index, false, true, ctx);
        }
    }

    /// Opens a native folder picker and, when the user selects a folder, upserts it
    /// into `PersistedWorkspace` and notifies the modal's repo picker at `param_index`.
    fn open_repo_picker_for_tab_config_modal(
        &mut self,
        param_index: usize,
        ctx: &mut ViewContext<Self>,
    ) {
        let modal_view = self.tab_config_params_modal.view.clone();
        ctx.open_file_picker(
            move |result, ctx| {
                let Ok(paths) = result else { return };
                let Some(path) = paths.into_iter().next() else {
                    return;
                };
                // Register the chosen directory as a workspace so it appears in
                // PersistedWorkspace (which is the data source for the repo picker
                // and also triggers codebase indexing / project rules scanning).
                let path_buf: PathBuf = path.clone().into();
                PersistedWorkspace::handle(ctx).update(ctx, |persisted, ctx| {
                    persisted.user_added_workspace(path_buf.clone(), ctx);
                });
                // Refresh the repo picker and pre-select the new path.
                modal_view.update(ctx, |modal, ctx| {
                    modal.body().update(ctx, |body, ctx| {
                        body.on_new_repo_selected(path_buf, param_index, ctx);
                    });
                });
            },
            warpui::platform::FilePickerConfiguration::new().folders_only(),
        );
    }

    fn close_tab_config_params_modal(&mut self, ctx: &mut ViewContext<Self>) {
        self.current_workspace_state.is_tab_config_params_modal_open = false;
        self.tab_config_params_modal.close();
        self.tab_config_params_modal.view.update(ctx, |modal, ctx| {
            modal.body().update(ctx, |body, ctx| {
                body.on_close(ctx);
            });
        });
        ctx.notify();
    }

    fn handle_new_worktree_modal_event(&mut self, event: &ModalEvent, ctx: &mut ViewContext<Self>) {
        match event {
            ModalEvent::Close => self.close_new_worktree_modal(ctx),
        }
    }

    fn handle_new_worktree_modal_body_event(
        &mut self,
        event: &NewWorktreeModalEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            NewWorktreeModalEvent::Close => self.close_new_worktree_modal(ctx),
            NewWorktreeModalEvent::Submit {
                repo,
                branch,
                worktree_branch_name,
            } => {
                self.handle_new_worktree_submit(repo, branch, worktree_branch_name.as_deref(), ctx);
                self.close_new_worktree_modal(ctx);
            }
            NewWorktreeModalEvent::PickNewRepo => {
                ctx.dispatch_typed_action_deferred(WorkspaceAction::OpenNewWorktreeRepoPicker);
            }
        }
    }

    fn close_new_worktree_modal(&mut self, ctx: &mut ViewContext<Self>) {
        self.current_workspace_state.is_new_worktree_modal_open = false;
        self.new_worktree_modal.close();
        self.new_worktree_modal.view.update(ctx, |modal, ctx| {
            modal.body().update(ctx, |body, ctx| {
                body.on_close(ctx);
            });
        });
        ctx.notify();
    }

    /// Checks whether the tab config references the special-cased
    /// `autogenerated_branch_name` template var. If so, fetches existing
    /// branches and generates a unique themed name.
    fn maybe_generate_worktree_name(
        &self,
        config: &crate::tab_configs::TabConfig,
    ) -> Option<String> {
        if !config.uses_autogenerated_branch_name() {
            return None;
        }
        let pane = config
            .panes
            .iter()
            .find(|pane| pane.directory.is_some())
            .or_else(|| config.panes.first())?;

        let repo_path = pane.directory.as_deref().map(Path::new);
        let branches = repo_path
            .map(crate::util::git::list_local_branches_sync)
            .unwrap_or_default();
        let branch_refs: HashSet<&str> = branches.iter().map(|s| s.as_str()).collect();
        Some(warp_util::worktree_names::generate_worktree_branch_name(
            &branch_refs,
        ))
    }

    /// Generates a worktree tab config TOML, writes it to `~/.warp/tab_configs/`,
    /// and opens the resulting config as a new tab.
    ///
    /// When `worktree_branch_name` is `None` (autogenerate), the TOML stores
    /// commands with `{autogenerated_branch_name}` template variables that get
    /// substituted with a fresh name on every open.
    /// When `Some(name)` (manual naming), the commands are baked in and a
    /// `worktree_branch_name` param is added so re-opens show the params modal.
    #[cfg(feature = "local_fs")]
    fn handle_new_worktree_submit(
        &mut self,
        repo: &str,
        base_branch: &str,
        worktree_branch_name: Option<&str>,
        ctx: &mut ViewContext<Self>,
    ) {
        let repo_display_name = Path::new(repo)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| repo.to_string());
        let config_name = match worktree_branch_name {
            Some(name) if !name.is_empty() => {
                format!("New worktree: {repo_display_name}, {name}")
            }
            _ if !base_branch.is_empty() => {
                format!("New worktree: {repo_display_name}, {base_branch}")
            }
            _ => format!("New worktree: {repo_display_name}"),
        };

        let filename_hint = if let Some(name) = worktree_branch_name {
            name.to_string()
        } else {
            let branches = crate::util::git::list_local_branches_sync(Path::new(repo));
            let branch_refs: HashSet<&str> = branches.iter().map(|s| s.as_str()).collect();
            warp_util::worktree_names::generate_worktree_branch_name(&branch_refs)
        };

        let toml_content = crate::tab_configs::build_worktree_config_toml(
            &config_name,
            repo,
            base_branch,
            worktree_branch_name,
        );

        let dir = tab_configs_dir();
        if let Err(e) = std::fs::create_dir_all(&dir) {
            log::warn!("Failed to create tab_configs dir: {e:?}");
            return;
        }

        let path = find_unused_worktree_config_path(&dir, &filename_hint);
        if let Err(e) = std::fs::write(&path, &toml_content) {
            log::warn!("Failed to write worktree tab config: {e:?}");
            return;
        }

        match toml::from_str::<crate::tab_configs::TabConfig>(&toml_content) {
            Ok(tab_config) => {
                if let Some(name) = worktree_branch_name {
                    // First open with manual name — bypass the params modal.
                    let mut param_values = HashMap::new();
                    param_values.insert("worktree_branch_name".to_string(), name.to_string());
                    self.open_tab_config_with_params(tab_config, param_values, None, ctx);
                } else {
                    // Autogenerate — open with the name we just generated.
                    let param_values = tab_config.default_param_values();
                    self.open_tab_config_with_params(
                        tab_config,
                        param_values,
                        Some(&filename_hint),
                        ctx,
                    );
                }
            }
            Err(e) => {
                log::warn!("Failed to parse generated worktree config: {e:?}");
            }
        }
    }

    #[cfg(not(feature = "local_fs"))]
    fn handle_new_worktree_submit(
        &mut self,
        _repo: &str,
        _base_branch: &str,
        _worktree_branch_name: Option<&str>,
        _ctx: &mut ViewContext<Self>,
    ) {
    }

    fn open_repo_picker_for_new_worktree_modal(&mut self, ctx: &mut ViewContext<Self>) {
        let modal_view = self.new_worktree_modal.view.clone();
        ctx.open_file_picker(
            move |result, ctx| {
                let Ok(paths) = result else { return };
                let Some(path) = paths.into_iter().next() else {
                    return;
                };
                let path_buf: PathBuf = path.clone().into();
                PersistedWorkspace::handle(ctx).update(ctx, |persisted, ctx| {
                    persisted.user_added_workspace(path_buf.clone(), ctx);
                });
                modal_view.update(ctx, |modal, ctx| {
                    modal.body().update(ctx, |body, ctx| {
                        body.on_new_repo_selected(path_buf, ctx);
                    });
                });
            },
            warpui::platform::FilePickerConfiguration::new().folders_only(),
        );
    }

    /// Opens a worktree in the given repo using the default worktree tab config,
    /// saving the materialized config to `~/.warp/tab_configs/` first.
    /// The branch name is auto-generated.
    #[cfg(feature = "local_fs")]
    fn open_worktree_in_repo(&mut self, repo_path: String, ctx: &mut ViewContext<Self>) {
        log::info!("open_worktree_in_repo requested: repo_path={repo_path:?}");
        let config_path = ensure_default_worktree_config();
        log::info!("Reading default worktree config from {config_path:?}");
        let template_toml = match std::fs::read_to_string(&config_path) {
            Ok(s) => s,
            Err(e) => {
                log::warn!("Failed to read default worktree config from {config_path:?}: {e:?}");
                return;
            }
        };
        let branches = crate::util::git::list_local_branches_sync(Path::new(&repo_path));
        let branch_refs: HashSet<&str> = branches.iter().map(|s| s.as_str()).collect();
        let branch_name = warp_util::worktree_names::generate_worktree_branch_name(&branch_refs);
        let repo_display_name = Path::new(&repo_path)
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| repo_path.clone());
        let config_name = format!("Worktree: {repo_display_name}");
        // Use the user's default session mode to decide pane type.
        let pane_type = if AISettings::as_ref(ctx).is_any_ai_enabled(ctx)
            && AISettings::as_ref(ctx).default_session_mode(ctx) == DefaultSessionMode::Agent
        {
            "agent"
        } else {
            "terminal"
        };
        log::info!(
            "Materializing default worktree config: repo_path={repo_path:?}, branch_name={branch_name:?}, pane_type={pane_type}"
        );

        let (toml_content, tab_config) = match materialize_default_worktree_config(
            &template_toml,
            &config_name,
            &repo_path,
            pane_type,
        ) {
            Ok(materialized) => materialized,
            Err(e) => {
                log::warn!(
                    "Failed to materialize default worktree config from {config_path:?}: {e}"
                );
                return;
            }
        };

        let dir = tab_configs_dir();
        if let Err(e) = std::fs::create_dir_all(&dir) {
            log::warn!("Failed to create tab_configs dir: {e:?}");
            return;
        }

        let saved_config_path =
            find_unused_toml_path(&dir, &sanitize_toml_base_name(&repo_display_name));
        if let Err(e) = std::fs::write(&saved_config_path, &toml_content) {
            log::warn!("Failed to write worktree tab config to {saved_config_path:?}: {e:?}");
            return;
        }

        log::info!(
            "Saved default worktree config to {saved_config_path:?}: config_name={:?}",
            tab_config.name
        );

        let param_values = tab_config.default_param_values();
        log::info!("Opening tab from saved worktree config");
        self.open_tab_config_with_params(tab_config, param_values, Some(&branch_name), ctx);
    }

    #[cfg(not(feature = "local_fs"))]
    fn open_worktree_in_repo(&mut self, _repo_path: String, _ctx: &mut ViewContext<Self>) {}

    /// Opens a native folder picker to add a new repo to PersistedWorkspace,
    /// triggered from the "+ Add new repo..." item in the New worktree config submenu.
    fn open_folder_picker_for_worktree_submenu(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.open_file_picker(
            move |result, ctx| {
                let Ok(paths) = result else { return };
                let Some(path) = paths.into_iter().next() else {
                    return;
                };
                let path_buf: PathBuf = path.into();
                PersistedWorkspace::handle(ctx).update(ctx, |persisted, ctx| {
                    persisted.user_added_workspace(path_buf, ctx);
                });
            },
            warpui::platform::FilePickerConfiguration::new().folders_only(),
        );
    }

    fn handle_welcome_tips_event(&mut self, event: &TipsEvent, ctx: &mut ViewContext<Self>) {
        match event {
            TipsEvent::Close => {
                self.welcome_tips_view_state.close_popup();
                ctx.notify();
            }
            TipsEvent::TipsDismissed => {
                self.tips_completed.update(ctx, |tips_completed, ctx| {
                    skip_tips_and_write_to_user_defaults(tips_completed, ctx);
                    ctx.notify();
                });
                self.welcome_tips_view_state = WelcomeTipsViewState::Unavailable;
                ctx.notify();
            }
        }
    }

    fn is_input_box_visible(&self, app: &AppContext) -> bool {
        if let (Some(terminal_model), Some(terminal_view)) = (
            self.get_active_session_terminal_model(app),
            self.active_tab_pane_group()
                .as_ref(app)
                .active_session_view(app),
        ) {
            terminal_view.read(app, |view, ctx| {
                view.is_input_box_visible(&terminal_model.lock(), ctx)
            })
        } else {
            false
        }
    }

    fn handle_theme_creator_modal_event(
        &mut self,
        event: &ThemeCreatorModalEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            ThemeCreatorModalEvent::Close => {
                self.current_workspace_state.is_theme_creator_modal_open = false;
                ctx.notify();
            }
            ThemeCreatorModalEvent::SetCustomTheme { theme } => {
                self.theme_chooser_view
                    .update(ctx, |theme_chooser_view, ctx| {
                        theme_chooser_view.reload_and_set_custom_theme(theme.clone(), ctx);
                    });
            }
            ThemeCreatorModalEvent::ShowErrorToast { message } => {
                self.toast_stack.update(ctx, |view, ctx| {
                    let new_toast = DismissibleToast::error(message.clone());
                    view.add_ephemeral_toast(new_toast, ctx);
                });
            }
        }
    }

    fn handle_theme_deletion_modal_event(
        &mut self,
        event: &ThemeDeletionModalEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            ThemeDeletionModalEvent::Close => {
                self.current_workspace_state.is_theme_deletion_modal_open = false;
                ctx.notify();
            }
            ThemeDeletionModalEvent::ShowErrorToast { message } => {
                self.toast_stack.update(ctx, |view, ctx| {
                    let new_toast = DismissibleToast::error(message.clone());
                    view.add_ephemeral_toast(new_toast, ctx);
                });
            }
            ThemeDeletionModalEvent::DeleteCurrentTheme => {
                self.theme_chooser_view
                    .update(ctx, |theme_chooser_view, ctx| {
                        // Reset theme to Dark if we are deleting the current theme
                        theme_chooser_view.select_and_save_theme(&ThemeKind::Dark, ctx);
                    });
            }
        }
    }

    /// Returns the pane group with the matching EntityId, or None if it doesn't exist.
    fn get_pane_group_view_with_id(&self, id: EntityId) -> Option<&ViewHandle<PaneGroup>> {
        self.tab_views().find(|view| view.id() == id)
    }

    // The workspace manages the close confirmation dialog, so it may need to close a pane after the user confirms in the dialog.
    // The flow is:
    // - User closes pane in pane group, which emits event to workspace
    // - Workspace shows confirmation dialog, and calls back into pane group to close pane here if user confirms
    fn handle_rewind_confirmation_dialog_event(
        &mut self,
        event: &RewindConfirmationEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            RewindConfirmationEvent::Cancel => {
                self.current_workspace_state
                    .is_rewind_confirmation_dialog_open = false;
                self.focus_active_tab(ctx);
                ctx.notify();
            }
            RewindConfirmationEvent::Confirm { rewind_source } => {
                self.current_workspace_state
                    .is_rewind_confirmation_dialog_open = false;
                self.handle_action(
                    &WorkspaceAction::ExecuteRewindAIConversation {
                        ai_block_view_id: rewind_source.ai_block_view_id,
                        exchange_id: rewind_source.exchange_id,
                        conversation_id: rewind_source.conversation_id,
                    },
                    ctx,
                );
                self.focus_active_tab(ctx);
                ctx.notify();
            }
        }
    }

    fn handle_delete_conversation_confirmation_dialog_event(
        &mut self,
        event: &DeleteConversationConfirmationEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            DeleteConversationConfirmationEvent::Cancel => {
                self.current_workspace_state
                    .is_delete_conversation_confirmation_dialog_open = false;
                ctx.focus(&self.left_panel_view);
                ctx.notify();
            }
            DeleteConversationConfirmationEvent::Confirm { source } => {
                self.current_workspace_state
                    .is_delete_conversation_confirmation_dialog_open = false;
                self.handle_action(
                    &WorkspaceAction::ExecuteDeleteConversation {
                        conversation_id: source.conversation_id,
                        terminal_view_id: source.terminal_view_id,
                    },
                    ctx,
                );
                ctx.focus(&self.left_panel_view);
                ctx.notify();
            }
        }
    }

    pub fn handle_network_status_event(
        &mut self,
        _handle: ModelHandle<NetworkStatus>,
        _event: &NetworkStatusEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        ctx.notify();
    }

    pub fn toggle_block_snackbar(&mut self, ctx: &mut ViewContext<Self>) {
        BlockListSettings::handle(ctx).update(ctx, |blocklist_settings, ctx| {
            report_if_error!(blocklist_settings
                .snackbar_enabled
                .toggle_and_save_value(ctx));
        });
    }

    pub fn toggle_error_underlining(&mut self, ctx: &mut ViewContext<Self>) {
        InputSettings::handle(ctx).update(ctx, |input_settings, ctx| {
            report_if_error!(input_settings.error_underlining.toggle_and_save_value(ctx));
        });
    }

    pub fn toggle_syntax_highlighting(&mut self, ctx: &mut ViewContext<Self>) {
        InputSettings::handle(ctx).update(ctx, |input_settings, ctx| {
            report_if_error!(input_settings
                .syntax_highlighting
                .toggle_and_save_value(ctx));
        });
    }

    pub fn change_cursor(&mut self, cursor_shape: Cursor, ctx: &mut ViewContext<Self>) {
        ctx.set_cursor_shape(cursor_shape);
        ctx.notify();
    }

    pub fn set_a11y_verbosity(
        &mut self,
        verbosity: AccessibilityVerbosity,
        ctx: &mut ViewContext<Self>,
    ) {
        AccessibilitySettings::handle(ctx).update(ctx, |accessibility_settings, ctx| {
            report_if_error!(accessibility_settings
                .a11y_verbosity
                .set_value(verbosity, ctx));
        });
    }

    pub fn snapshot(
        &self,
        window_id: WindowId,
        quake_mode: bool,
        app: &AppContext,
    ) -> WindowSnapshot {
        let window_bounds = app.window_bounds(&window_id);
        let window_fullscreen_state = app
            .windows()
            .platform_window(window_id)
            .map(|window| window.fullscreen_state())
            .unwrap_or_default();
        let active_tab_index = self.active_tab_index();
        let tabs = self
            .tab_views()
            .enumerate()
            .map(|(tab_index, pane_group_view)| {
                let resizable_data = ResizableData::handle(app);
                let modal_sizes = resizable_data.as_ref(app).get_all_handles(window_id);

                let left_panel_width = modal_sizes.map(|ms| {
                    ms.left_panel_width
                        .lock()
                        .expect("should be able to lock left panel handle")
                        .size()
                });

                let right_panel_width = modal_sizes.map(|ms| {
                    ms.right_panel_width
                        .lock()
                        .expect("should be able to lock right panel handle")
                        .size()
                });

                let pane_group = pane_group_view.as_ref(app);
                let root = pane_group.snapshot(app);
                let left_panel =
                    self.compute_left_panel_snapshot(pane_group_view, left_panel_width, app);
                let right_panel =
                    self.compute_right_panel_snapshot(pane_group_view, right_panel_width, app);
                TabSnapshot {
                    root,
                    custom_title: pane_group.custom_title(app),
                    default_directory_color: self
                        .tabs
                        .get(tab_index)
                        .and_then(|tab| tab.default_directory_color),
                    selected_color: self
                        .tabs
                        .get(tab_index)
                        .map_or(SelectedTabColor::Unset, |tab| tab.selected_color),
                    left_panel,
                    right_panel,
                }
            })
            .collect();

        let resizable_data = ResizableData::handle(app);
        let modal_sizes = resizable_data.as_ref(app).get_all_handles(window_id);

        // Reads the current width of the universal search modal, to store with the window snapshot
        let universal_search_width = modal_sizes.map(|ms| {
            ms.universal_search_width
                .lock()
                .expect("should be able to lock universal search resizable state handle")
                .size()
        });

        let voltron_width = modal_sizes.map(|ms| {
            ms.voltron_width
                .lock()
                .expect("should be able to lock voltron resizable state handle")
                .size()
        });

        let left_panel_width = modal_sizes.map(|ms| {
            ms.left_panel_width
                .lock()
                .map(|guard| guard.size())
                .unwrap_or(DEFAULT_LEFT_PANEL_WIDTH)
        });

        let right_panel_width = modal_sizes.map(|ms| {
            ms.right_panel_width
                .lock()
                .map(|guard| guard.size())
                .unwrap_or(DEFAULT_RIGHT_PANEL_WIDTH)
        });

        WindowSnapshot {
            tabs,
            active_tab_index,
            bounds: window_bounds,
            fullscreen_state: window_fullscreen_state,
            quake_mode,
            universal_search_width,
            voltron_width,
            left_panel_open: self.left_panel_open,
            vertical_tabs_panel_open: self.vertical_tabs_panel_open,
            left_panel_width,
            right_panel_width,
        }
    }

    fn compute_left_panel_snapshot(
        &self,
        pane_group: &ViewHandle<PaneGroup>,
        left_panel_width: Option<f32>,
        app: &AppContext,
    ) -> Option<LeftPanelSnapshot> {
        let pane_group_ref = pane_group.as_ref(app);
        if !pane_group_ref.left_panel_open {
            return None;
        }

        let pane_group_id = pane_group.id();

        self.left_panel_view.read(app, |lp, _| {
            Some(LeftPanelSnapshot {
                left_panel_displayed_tab: lp.active_view().into(),
                pane_group_id: pane_group_id.to_string(),
                width: left_panel_width.unwrap_or(DEFAULT_LEFT_PANEL_WIDTH) as usize,
            })
        })
    }

    fn compute_right_panel_snapshot(
        &self,
        pane_group: &ViewHandle<PaneGroup>,
        right_panel_width: Option<f32>,
        app: &AppContext,
    ) -> Option<RightPanelSnapshot> {
        let pane_group_ref = pane_group.as_ref(app);
        if !pane_group_ref.right_panel_open {
            return None;
        }

        let pane_group_id = pane_group.id();
        let is_maximized = pane_group_ref.is_right_panel_maximized;

        Some(RightPanelSnapshot {
            pane_group_id: pane_group_id.to_string(),
            width: right_panel_width.unwrap_or(DEFAULT_RIGHT_PANEL_WIDTH) as usize,
            is_maximized,
        })
    }

    pub fn open_launch_config_save_modal(&mut self, ctx: &mut ViewContext<Self>) {
        self.close_palette(true, None, ctx); // close palettes if any are open
        self.launch_config_save_modal.open();
        self.current_workspace_state
            .is_launch_config_save_modal_open = true;

        self.launch_config_save_modal.view.update(ctx, |view, ctx| {
            view.set_snapshot_source(ctx);
            view.reset_editor(ctx); // placeholder and clear editor
            ctx.notify();
        });

        self.tips_completed.update(ctx, |tips_completed, ctx| {
            mark_feature_used_and_write_to_user_defaults(
                Tip::Action(TipAction::SaveNewLaunchConfig),
                tips_completed,
                ctx,
            );
            ctx.notify();
        });

        ctx.focus(&self.launch_config_save_modal.view);
        ctx.notify();
    }

    pub fn cycle_prev_session(&mut self, ctx: &mut ViewContext<Self>) {
        self.cycle_session(SessionCycleDirection::Previous, ctx);
    }

    pub fn cycle_next_session(&mut self, ctx: &mut ViewContext<Self>) {
        self.cycle_session(SessionCycleDirection::Next, ctx);
    }

    fn cycle_session(&mut self, direction: SessionCycleDirection, ctx: &mut ViewContext<Self>) {
        let keys_settings = KeysSettings::as_ref(ctx);
        match *keys_settings.ctrl_tab_behavior {
            CtrlTabBehavior::ActivatePrevNextTab => match direction {
                SessionCycleDirection::Next => {
                    self.activate_next_tab(ctx);
                }
                SessionCycleDirection::Previous => {
                    self.activate_prev_tab(ctx);
                }
            },
            CtrlTabBehavior::CycleMostRecentSession => {
                self.current_workspace_state.is_palette_open = false;
                if !self.current_workspace_state.is_ctrl_tab_palette_open {
                    self.open_palette_action(
                        PaletteMode::Navigation,
                        PaletteSource::CtrlTab {
                            shift_pressed_initially: matches!(
                                direction,
                                SessionCycleDirection::Previous
                            ),
                        },
                        None,
                        ctx,
                    );
                }
                self.ctrl_tab_palette
                    .update(ctx, |palette, ctx| match direction {
                        SessionCycleDirection::Next => {
                            palette.select_next_item(ctx);
                        }
                        SessionCycleDirection::Previous => {
                            palette.select_prev_item(ctx);
                        }
                    });
                ctx.notify();
            }
        }
    }

    pub fn activate_prev_tab(&mut self, ctx: &mut ViewContext<Self>) {
        let index = if self.vertical_tabs_panel.search_query.is_empty() {
            if self.active_tab_index > 0 {
                self.active_tab_index - 1
            } else {
                self.tabs.len() - 1
            }
        } else {
            let matching = self.vertical_tabs_panel.matching_tab_indices(
                &self.tabs,
                self.active_tab_index,
                ctx,
            );
            matching
                .iter()
                .rev()
                .find(|&&i| i < self.active_tab_index)
                .or_else(|| matching.last())
                .copied()
                .unwrap_or(self.active_tab_index)
        };
        self.activate_tab(index, ctx);
    }

    pub fn activate_next_tab(&mut self, ctx: &mut ViewContext<Self>) {
        let index = if self.vertical_tabs_panel.search_query.is_empty() {
            if self.active_tab_index + 1 < self.tabs.len() {
                self.active_tab_index + 1
            } else {
                0
            }
        } else {
            let matching = self.vertical_tabs_panel.matching_tab_indices(
                &self.tabs,
                self.active_tab_index,
                ctx,
            );
            matching
                .iter()
                .find(|&&i| i > self.active_tab_index)
                .or_else(|| matching.first())
                .copied()
                .unwrap_or(self.active_tab_index)
        };
        self.activate_tab(index, ctx);
    }

    pub fn activate_last_tab(&mut self, ctx: &mut ViewContext<Self>) {
        if self.tabs.len() > 1 {
            let target_index = self.tabs.len() - 1;
            self.activate_tab(target_index, ctx);
        }
    }

    fn remove_tab(
        &mut self,
        index: usize,
        add_to_undo_stack: bool,
        detach_panes_for_close: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(tab_data) = self.tabs.get(index) else {
            debug_assert!(false, "Tried to remove a tab with an invalid index");
            return;
        };

        // If the vertical-tabs detail sidecar is anchored to this tab's pane group, clear it.
        // Otherwise it will try to position itself against a pane row that is about to disappear
        // (either because the tab is being removed from `self.tabs`, or because we're about to
        // close the window for the last tab).
        self.vertical_tabs_panel
            .clear_detail_sidecar_if_for_pane_group(tab_data.pane_group.id());

        // If this is the last tab, close the window instead of actually removing
        // the tab.
        if self.tabs.len() == 1 {
            if ContextFlag::CloseWindow.is_enabled() {
                ctx.close_window();
            }
            return;
        }

        if detach_panes_for_close {
            let working_directories_model = self.working_directories_model.clone();
            tab_data.pane_group.update(ctx, |pane_group, ctx| {
                pane_group.for_all_terminal_panes(
                    |terminal_view, ctx| {
                        if terminal_view
                            .model
                            .lock()
                            .block_list()
                            .active_block()
                            .is_active_and_long_running()
                        {
                            terminal_view.shutdown_pty(ctx);
                        }
                    },
                    ctx,
                );

                pane_group.detach_panes_for_close(&working_directories_model, ctx);
            });
        }

        let tab_data = self.tabs.remove(index);

        if add_to_undo_stack {
            let handle = ctx.handle();
            UndoCloseStack::handle(ctx).update(ctx, |stack, ctx| {
                log::info!("storing data for closed tab");
                stack.handle_tab_closed(handle, index, tab_data, ctx);
            });
        }

        match index.cmp(&self.active_tab_index) {
            Ordering::Equal => {
                // If there's a previous tab, activate it. Otherwise, keep the active
                // tab at index 0.
                self.activate_tab_internal(index.saturating_sub(1), ctx);
            }
            Ordering::Less => {
                // If we are closing a tab before the active tab we need to adjust
                // the active tab index.
                self.active_tab_index -= 1;
            }
            _ => {}
        }

        ctx.dispatch_global_action("workspace:save_app", ());
        ctx.notify();
    }

    pub fn remove_tab_without_undo(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        self.remove_tab(index, false, false, ctx);
    }
    /// Adopts a transferred PaneGroup into the placeholder tab created during window transfer.
    /// This replaces the placeholder tab's PaneGroup with the actual transferred one.
    pub fn adopt_transferred_pane_group(
        &mut self,
        new_pane_group: ViewHandle<PaneGroup>,
        ctx: &mut ViewContext<Self>,
    ) {
        if !self.pending_pane_group_transfer {
            debug_assert!(
                false,
                "adopt_transferred_pane_group called without pending transfer"
            );
            return;
        }

        if self.tabs.is_empty() {
            debug_assert!(false, "adopt_transferred_pane_group called with no tabs");
            return;
        }
        let Some(placeholder_tab) = self.tabs.last_mut() else {
            debug_assert!(
                false,
                "adopt_transferred_pane_group missing placeholder tab"
            );
            return;
        };

        let placeholder_pane_group =
            std::mem::replace(&mut placeholder_tab.pane_group, new_pane_group);
        let working_directories_model = self.working_directories_model.clone();
        placeholder_pane_group.update(ctx, |pg, ctx| {
            pg.detach_panes_for_close(&working_directories_model, ctx);
        });
        self.pending_pane_group_transfer = false;

        ctx.dispatch_global_action("workspace:save_app", ());
        ctx.notify();
    }

    /// Checks whether the provided tab indices need quit-warning confirmation before closing.
    /// If none of them need confirmation, we close all the provided tabs.
    /// Returns true iff all of the tabs were closed.
    fn close_tabs(
        &mut self,
        tab_indices: impl Iterator<Item = usize>,
        skip_confirmation: bool,
        add_to_undo_stack: bool,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        let tab_indices_vec = tab_indices.collect_vec();
        if !skip_confirmation {
            let tabs = tab_indices_vec
                .iter()
                .filter_map(|i| self.get_pane_group_view(*i))
                .map(|tab| tab.downgrade())
                .collect_vec();
            let summary = UnsavedStateSummary::for_tabs(tabs, ctx);

            if summary.should_display_warning(ctx) {
                // The quit-warning dialog uses app-scoped callbacks (ironically, because that's
                // what Self::show_native_modal expects). That means we need a handle to the
                // current workspace here.
                let confirm_self = ctx.handle();
                let navigate_self = ctx.handle();
                let confirm_tabs = tab_indices_vec.clone();
                let dialog = summary
                    .dialog()
                    .on_confirm(move |ctx| {
                        if let Some(workspace) = confirm_self.upgrade(ctx) {
                            workspace.update(ctx, |workspace, ctx| {
                                workspace.close_tabs(
                                    confirm_tabs.into_iter(),
                                    true,
                                    add_to_undo_stack,
                                    ctx,
                                );
                            });
                        }
                    })
                    .on_cancel(|_ctx| { /* No action needed besides dismissing the dialog. */ })
                    .on_show_processes(move |ctx| {
                        if let Some(workspace) = navigate_self.upgrade(ctx) {
                            workspace.update(ctx, |workspace, ctx| {
                                // TODO(ben): Ideally, this would filter to the relevant tabs.
                                workspace.open_palette_action(
                                    PaletteMode::Navigation,
                                    PaletteSource::QuitModal,
                                    Some("running"),
                                    ctx,
                                );
                            })
                        }
                    })
                    .build();

                if cfg!(all(not(target_family = "wasm"), target_os = "macos")) {
                    AppContext::show_native_platform_modal(ctx, dialog);
                    return false;
                } else if cfg!(all(
                    not(target_family = "wasm"),
                    any(target_os = "linux", windows)
                )) {
                    self.show_native_modal(dialog, ctx);
                    return false;
                }
            }
        }

        // If we are renaming a tab, cancel that.  Closing tabs causes the renamed tab index
        // to fall out of sync.  This can cause inconsistencies.
        self.cancel_tab_rename(ctx);

        // Remove the tabs in reverse order to avoid indexing OOB.
        for i in tab_indices_vec.into_iter().sorted().rev() {
            self.remove_tab(i, add_to_undo_stack, true, ctx);
        }
        true
    }

    /// Opens a confirmation dialog if necessary, or closes immediately if not.
    /// Always closes immediately if skip_confirmation is true.
    fn close_tab(
        &mut self,
        index: usize,
        skip_confirmation: bool,
        add_to_undo_stack: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        let is_last_tab = self.tabs.len() == 1;
        if !ContextFlag::CloseWindow.is_enabled() && is_last_tab {
            return;
        }

        let tabs_closed = self.close_tabs(
            vec![index].into_iter(),
            skip_confirmation || is_last_tab, // If this is the last tab, the confirmation dialog will be handled by the window close.
            add_to_undo_stack,
            ctx,
        );

        // Telemetry whenever tabs actually closed, not when confirmation dialog comes up.
        if tabs_closed {
            ctx.dispatch_global_action("workspace:save_app", ());
        }
    }

    /// Opens a confirmation dialog if necessary, or closes immediately if not.
    /// Always closes immediately if skip_confirmation is true.
    pub fn close_other_tabs(
        &mut self,
        index: usize,
        skip_confirmation: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        // Figure out what indices we want to delete for the "other tabs" case.
        let indices_to_remove = (0..self.tabs.len()).filter(|i| *i != index);

        let tabs_closed = self.close_tabs(indices_to_remove, skip_confirmation, true, ctx);

        // Telemetry whenever tabs actually closed, not when confirmation dialog comes up.
        if tabs_closed {}
    }

    /// Opens a confirmation dialog if necessary, or closes immediately if not.
    /// Always closes immediately if skip_confirmation is true.
    pub fn close_tabs_direction(
        &mut self,
        index: usize,
        direction: TabMovement,
        skip_confirmation: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        let indices_to_remove = match direction {
            TabMovement::Left => 0..index,
            TabMovement::Right => (index + 1)..self.tabs.len(),
        };
        let tabs_closed = self.close_tabs(indices_to_remove, skip_confirmation, true, ctx);

        // Telemetry whenever tabs actually closed, not when confirmation dialog comes up.
        if tabs_closed {
            match direction {
                TabMovement::Right if self.active_tab_index > index => {}
                _ => (),
            }
        }
    }

    /// Closes all tabs that have code panes with the specified file path open.
    /// This is used when a file is renamed or deleted in the file tree
    #[cfg(feature = "local_fs")]
    fn close_tabs_with_file_path(&mut self, old_path: &Path, ctx: &mut ViewContext<Self>) {
        // Find all code panes across all tabs that have this file open
        for tab_data in &self.tabs {
            // Check if this tab has any code panes with the old file path open
            tab_data.pane_group.update(ctx, |pane_group, ctx| {
                // Collect code panes first to avoid borrowing issues
                let code_panes: Vec<_> = pane_group.code_panes(ctx).collect();
                for (_, code_pane) in code_panes {
                    code_pane.update(ctx, |code_view, ctx| {
                        code_view.close_tabs_with_path(old_path, ctx);
                    });
                }
            });
        }

        ctx.notify();
    }

    /// Renames all open code tabs that point to `old_path` to now point to `new_path`,
    /// updating their contents in-place rather than closing them.
    #[cfg(feature = "local_fs")]
    fn rename_tabs_with_file_path(
        &mut self,
        old_path: &Path,
        new_path: &Path,
        ctx: &mut ViewContext<Self>,
    ) {
        for tab_data in &self.tabs {
            tab_data.pane_group.update(ctx, |pane_group, ctx| {
                // Collect code panes first to avoid borrowing issues
                let code_panes: Vec<_> = pane_group.code_panes(ctx).collect();
                for (_, code_pane) in code_panes {
                    code_pane.update(ctx, |code_view, ctx| {
                        code_view.rename_tabs_with_path(old_path, new_path, ctx);
                    });
                }
            });
        }
        ctx.notify();
    }

    /// Update this workspace when it is reopened after being closed.
    pub fn handle_reopen(&mut self, ctx: &mut ViewContext<Self>) {
        self.sync_window_button_visibility(ctx);
        for pane_group in self.tab_views() {
            pane_group.update(ctx, |pane_group, ctx| {
                pane_group.reattach_panes(ctx);
            })
        }
        self.update_active_session(ctx);
    }

    pub fn restore_closed_tab(
        &mut self,
        tab_index: usize,
        tab_data: TabData,
        ctx: &mut ViewContext<Self>,
    ) {
        // When restoring a closed tab, we have to reattach its panes so that they know they're
        // user-accessible again.
        tab_data.pane_group.update(ctx, |pane_group, ctx| {
            pane_group.reattach_panes(ctx);
        });

        self.tabs.insert(tab_index, tab_data);
        self.activate_tab(tab_index, ctx);

        ctx.notify();
    }

    pub fn add_terminal_tab(&mut self, hide_homepage: bool, ctx: &mut ViewContext<Self>) {
        self.add_new_session_tab_with_default_mode(
            NewSessionSource::Tab,
            Some(ctx.window_id()),
            None,
            None,
            hide_homepage,
            ctx,
        );
        ctx.notify();
    }

    fn add_welcome_tab(&mut self, ctx: &mut ViewContext<Self>) {
        let startup_directory = self.get_new_tab_startup_directory(
            NewSessionSource::Tab,
            Some(ctx.window_id()),
            None,
            ctx,
        );
        self.add_tab_with_pane_layout(
            PanesLayout::Snapshot(Box::new(PaneNodeSnapshot::Leaf(LeafSnapshot {
                is_focused: true,
                custom_vertical_tabs_title: None,
                contents: LeafContents::Welcome { startup_directory },
            }))),
            Arc::new(HashMap::new()),
            None,
            ctx,
        );
        ctx.notify();
    }

    fn add_get_started_tab(&mut self, ctx: &mut ViewContext<Self>) {
        self.add_tab_with_pane_layout(
            PanesLayout::Snapshot(Box::new(PaneNodeSnapshot::Leaf(LeafSnapshot {
                is_focused: true,
                custom_vertical_tabs_title: None,
                contents: LeafContents::GetStarted,
            }))),
            Arc::new(HashMap::new()),
            None,
            ctx,
        );
        ctx.notify();
    }

    fn add_docker_sandbox_tab(&mut self, ctx: &mut ViewContext<Self>) {
        if !FeatureFlag::LocalDockerSandbox.is_enabled() {
            log::warn!("Local docker sandbox feature flag is disabled");
            return;
        }
        // Docker sandboxes are inherently local — sbx resolution and the
        // `AvailableShell::new_docker_sandbox_shell` constructor both require
        // `local_tty`. Other builds log and bail.
        #[cfg(feature = "local_tty")]
        {
            // Resolve sbx via the user's interactive shell PATH (same mechanism
            // MCP servers use) so we find it when installed via homebrew on Apple
            // Silicon, `~/.local/bin`, `nvm`-style paths, etc. This is async
            // because capturing the interactive PATH requires spawning the user's
            // login shell.
            let window_id = ctx.window_id();
            let sbx_future = resolve_sbx_path_from_user_shell(ctx);
            ctx.spawn(sbx_future, move |me, sbx_path, ctx| {
                let Some(sbx_path) = sbx_path else {
                    log::error!("sbx binary not found; cannot create Docker sandbox");
                    return;
                };
                let shell = AvailableShell::new_docker_sandbox_shell(
                    sbx_path,
                    DEFAULT_DOCKER_SANDBOX_BASE_IMAGE.map(str::to_owned),
                );
                me.add_new_session_tab_internal_with_default_session_mode_behavior(
                    NewSessionSource::Tab,
                    Some(window_id),
                    Some(shell),
                    None,
                    true, /* hide_homepage */
                    DefaultSessionModeBehavior::Ignore,
                    ctx,
                );
                ctx.notify();
            });
        }
        #[cfg(not(feature = "local_tty"))]
        {
            let _ = ctx;
            log::warn!("Docker sandbox requires the `local_tty` feature; ignoring request");
        }
    }

    // Adds a tab with a specific shell, only meant to be dispatched directly by actions.
    fn add_tab_with_shell(&mut self, shell: AvailableShell, ctx: &mut ViewContext<Self>) {
        self.add_new_session_tab_with_default_mode(
            NewSessionSource::Tab,
            Some(ctx.window_id()),
            Some(shell),
            None,
            false,
            ctx,
        );
        ctx.notify();
    }

    fn add_new_session_tab_with_default_mode(
        &mut self,
        new_session_source: NewSessionSource,
        previous_session_window_id: Option<WindowId>,
        chosen_shell: Option<AvailableShell>,
        conversation_restoration: Option<ConversationRestorationInNewPaneType>,
        hide_homepage: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        self.add_new_session_tab_internal_with_default_session_mode_behavior(
            new_session_source,
            previous_session_window_id,
            chosen_shell,
            conversation_restoration,
            hide_homepage,
            DefaultSessionModeBehavior::Apply,
            ctx,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn add_new_session_tab_internal_with_default_session_mode_behavior(
        &mut self,
        new_session_source: NewSessionSource,
        previous_session_window_id: Option<WindowId>,
        chosen_shell: Option<AvailableShell>,
        conversation_restoration: Option<ConversationRestorationInNewPaneType>,
        hide_homepage: bool,
        default_session_mode_behavior: DefaultSessionModeBehavior,
        ctx: &mut ViewContext<Self>,
    ) {
        // Check if we should default to agent mode (only for new sessions, not restorations)
        let should_enter_agent_view = matches!(
            default_session_mode_behavior,
            DefaultSessionModeBehavior::Apply
        ) && conversation_restoration.is_none()
            && AISettings::as_ref(ctx).default_session_mode(ctx) == DefaultSessionMode::Agent;
        #[cfg(feature = "local_tty")]
        let is_docker_sandbox = chosen_shell
            .as_ref()
            .is_some_and(AvailableShell::is_docker_sandbox);
        #[cfg(not(feature = "local_tty"))]
        let is_docker_sandbox = {
            let _ = chosen_shell.as_ref();
            false
        };

        // If restoring a conversation, use its initial working directory if it exists
        let startup_directory_from_conversation = conversation_restoration
            .as_ref()
            .and_then(|restoration| restoration.initial_working_directory())
            .map(PathBuf::from)
            .filter(|path| path.is_dir());

        let startup_directory = startup_directory_from_conversation.or_else(|| {
            self.get_new_tab_startup_directory(
                new_session_source,
                previous_session_window_id,
                chosen_shell.as_ref(),
                ctx,
            )
        });

        self.add_tab_with_pane_layout(
            PanesLayout::SingleTerminal(Box::new(NewTerminalOptions {
                shell: chosen_shell,
                initial_directory: startup_directory,
                conversation_restoration,
                hide_homepage,
                ..Default::default()
            })),
            Arc::new(HashMap::new()),
            None, /*custom_tab_title*/
            ctx,
        );

        #[cfg(all(feature = "local_tty", not(target_family = "wasm")))]
        if is_docker_sandbox {
            if let Some(terminal_view) = self
                .active_tab_pane_group()
                .as_ref(ctx)
                .active_session_view(ctx)
            {
                TerminalView::initialize_docker_sandbox_environment(&terminal_view, ctx);
            } else {
                log::warn!("Could not find docker sandbox terminal view after creating new tab");
            }
        }
        #[cfg(not(all(feature = "local_tty", not(target_family = "wasm"))))]
        let _ = is_docker_sandbox;
        // If the default session mode is Agent and AI is enabled, enter agent view
        if should_enter_agent_view {
            self.enter_agent_view_on_active_tab(ctx);
        }
    }

    /// Enters agent view with a new conversation on the active tab's terminal.
    ///
    /// Used after adding a new tab when the session mode should default to agent view.
    fn enter_agent_view_on_active_tab(&self, ctx: &mut ViewContext<Self>) {
        self.active_tab_pane_group().update(ctx, |pane_group, ctx| {
            if let Some(terminal_view) = pane_group.active_session_view(ctx) {
                terminal_view.update(ctx, |view, ctx| {
                    view.enter_agent_view_for_new_conversation(
                        None,
                        AgentViewEntryOrigin::DefaultSessionMode,
                        ctx,
                    );
                });
            }
        });
    }

    pub fn add_tab_with_pane_layout(
        &mut self,
        panes_layout: PanesLayout,
        block_lists: Arc<HashMap<PaneUuid, Vec<SerializedBlockListItem>>>,
        custom_tab_title: Option<String>,
        ctx: &mut ViewContext<Self>,
    ) {
        // Remember whether the left panel was open on the current active pane group
        // before creating a new active pane group.
        let left_panel_was_open = if self.tabs.is_empty() {
            false
        } else {
            self.active_tab_pane_group().as_ref(ctx).left_panel_open
        };

        // Capture the active tab's colors before creating the new tab.
        let active_tab = self.tabs.get(self.active_tab_index);
        let active_tab_selected_color = active_tab.map(|tab| tab.selected_color);
        let active_tab_default_color = active_tab.and_then(|tab| tab.default_directory_color);

        let is_new_terminal = matches!(panes_layout, PanesLayout::SingleTerminal(_));
        let is_restoration = matches!(panes_layout, PanesLayout::Snapshot(_));
        let new_pane_group = ctx.add_typed_action_view(|ctx| {
            let mut pane_group = PaneGroup::new_with_panes_layout(
                self.tips_completed.clone(),
                self.user_default_shell_unsupported_banner_model_handle
                    .clone(),
                panes_layout,
                block_lists,
                self.model_event_sender.clone(),
                ctx,
            );
            if let Some(title) = custom_tab_title {
                pane_group.set_title(&title, ctx);
            }
            pane_group
        });

        ctx.subscribe_to_view(&new_pane_group, move |me, pane_group, event, ctx| {
            me.handle_file_tree_event(pane_group, event, ctx)
        });

        let new_tab_placement_setting = TabSettings::as_ref(ctx).new_tab_placement;

        match new_tab_placement_setting {
            NewTabPlacement::AfterAllTabs => {
                self.tabs.push(TabData::new(new_pane_group));
                self.activate_tab_internal(self.tab_count() - 1, ctx);
            }
            // Add tab after current tab
            _ => {
                if self.tab_count() == 0 {
                    self.tabs.push(TabData::new(new_pane_group));
                    self.activate_tab_internal(self.tab_count() - 1, ctx);
                } else {
                    self.tabs
                        .insert(self.active_tab_index + 1, TabData::new(new_pane_group));
                    self.activate_tab_internal(self.active_tab_index + 1, ctx);
                }
            }
        }

        if !is_restoration {
            if *TabSettings::as_ref(ctx).preserve_active_tab_color.value() {
                if let Some(SelectedTabColor::Color(color)) = active_tab_selected_color {
                    self.tabs[self.active_tab_index].selected_color =
                        SelectedTabColor::Color(color);
                }
            }

            // preserve the current tab's default directory color when the new tab inherits the working directory
            // (otherwise the new tab's color flashes from no-color to default color during bootstrapping).
            if FeatureFlag::DirectoryTabColors.is_enabled() && is_new_terminal {
                let wd_config = &SessionSettings::as_ref(ctx).working_directory_config;
                let inherits_cwd = wd_config.config_for_source(NewSessionSource::Tab).mode
                    == WorkingDirectoryMode::PreviousDir
                    || wd_config.config_for_source(NewSessionSource::Window).mode
                        == WorkingDirectoryMode::PreviousDir;
                if inherits_cwd {
                    if let Some(color) = active_tab_default_color {
                        self.tabs[self.active_tab_index].default_directory_color = Some(color);
                    }
                }
            }
        }

        // If the previous tab's left panel was open, maintain that state with the new tab
        // (unless we're restoring the tab from a persisted snapshot).
        if !is_restoration && left_panel_was_open {
            self.active_tab_pane_group().update(ctx, |pg, ctx| {
                pg.set_left_panel_open(true, ctx);
            });
        }
    }

    pub fn add_tab_from_existing_pane(
        &mut self,
        pane: Box<dyn AnyPaneContent>,
        new_idx: usize,
        ctx: &mut ViewContext<Self>,
    ) {
        let new_pane_group = ctx.add_typed_action_view(|ctx| {
            PaneGroup::new_from_existing_pane(
                pane,
                self.tips_completed.clone(),
                self.user_default_shell_unsupported_banner_model_handle
                    .clone(),
                self.model_event_sender.clone(),
                ctx,
            )
        });
        ctx.subscribe_to_view(&new_pane_group, move |me, pane_group, event, ctx| {
            me.handle_file_tree_event(pane_group, event, ctx)
        });

        if self.tab_count() == 0 {
            self.tabs.push(TabData::new(new_pane_group));
            self.activate_tab_internal(self.tab_count() - 1, ctx);
        } else {
            self.tabs.insert(new_idx, TabData::new(new_pane_group));
            self.activate_tab_internal(new_idx, ctx);
        }
    }

    /// Add a tab with a file notebook pane open.
    pub fn add_tab_for_file_notebook(
        &mut self,
        file_path: Option<PathBuf>,
        ctx: &mut ViewContext<Self>,
    ) {
        let panes_layout = PanesLayout::Snapshot(Box::new(PaneNodeSnapshot::Leaf(LeafSnapshot {
            is_focused: true,
            custom_vertical_tabs_title: None,
            contents: LeafContents::Notebook(NotebookPaneSnapshot::LocalFileNotebook {
                path: file_path,
            }),
        })));
        self.add_tab_with_pane_layout(panes_layout, Arc::new(HashMap::new()), None, ctx);
    }

    /// Add a tab with a code pane open for the specified file.
    pub fn add_tab_for_code_file(
        &mut self,
        file_path: PathBuf,
        line_and_column: Option<LineAndColumnArg>,
        ctx: &mut ViewContext<Self>,
    ) {
        let source = CodeSource::Link {
            path: file_path,
            range_start: None,
            range_end: None,
        };
        let pane = CodePane::new(source, line_and_column, ctx);

        let new_tab_placement_setting = TabSettings::as_ref(ctx).new_tab_placement;
        let new_idx = match new_tab_placement_setting {
            NewTabPlacement::AfterAllTabs => self.tab_count(),
            NewTabPlacement::AfterCurrentTab => self.active_tab_index + 1,
        };
        self.add_tab_from_existing_pane(Box::new(pane), new_idx, ctx);
    }

    pub fn add_tab_for_new_code_file(&mut self, ctx: &mut ViewContext<Self>) {
        #[cfg(feature = "local_fs")]
        {
            let default_directory = self
                .active_session_view(ctx)
                .and_then(|view| view.as_ref(ctx).pwd())
                .map(PathBuf::from)
                .or_else(dirs::home_dir);
            let source = CodeSource::New { default_directory };

            let layout = *EditorSettings::as_ref(ctx).open_file_layout.value();

            // Check if we can add the new file to an existing code pane (when using split pane
            // layout).
            if layout == EditorLayout::SplitPane
                && FeatureFlag::TabbedEditorView.is_enabled()
                && *EditorSettings::as_ref(ctx)
                    .prefer_tabbed_editor_view
                    .value()
            {
                let code_view = self
                    .active_tab_pane_group()
                    .as_ref(ctx)
                    .code_panes(ctx)
                    .find(|(pane_id, _)| {
                        !self
                            .active_tab_pane_group()
                            .as_ref(ctx)
                            .is_pane_hidden_for_close(*pane_id)
                    });

                if let Some((pane_id, code_view)) = code_view {
                    code_view.update(ctx, |code_view, ctx| {
                        code_view.open_or_focus_existing(None, None, ctx);
                    });
                    self.active_tab_pane_group().update(ctx, |pane_group, ctx| {
                        pane_group.focus_pane(pane_id, true, ctx);
                    });
                    return;
                }
            }

            let pane = CodePane::new(source, None, ctx);

            match layout {
                EditorLayout::NewTab => {
                    let new_tab_placement_setting = TabSettings::as_ref(ctx).new_tab_placement;
                    let new_idx = match new_tab_placement_setting {
                        NewTabPlacement::AfterAllTabs => self.tab_count(),
                        NewTabPlacement::AfterCurrentTab => self.active_tab_index + 1,
                    };
                    self.add_tab_from_existing_pane(Box::new(pane), new_idx, ctx);
                }
                EditorLayout::SplitPane => {
                    self.active_tab_pane_group().update(ctx, |pane_group, ctx| {
                        pane_group.add_pane_with_direction(
                            Direction::Right,
                            pane,
                            true, /* focus_new_pane */
                            ctx,
                        );
                    });
                }
            }
        }

        #[cfg(not(feature = "local_fs"))]
        {
            let _ = ctx;
            // Code file functionality is not available without local_fs feature
            log::warn!("NewCodeFile action called but local_fs feature is not enabled");
        }
    }

    fn open_repository(&mut self, path: Option<&str>, ctx: &mut ViewContext<Self>) {
        match path {
            Some(path) => self.handle_open_repository(path, ctx),
            None => ctx.open_file_picker(
                |result, ctx| match result {
                    Ok(paths) => {
                        let Some(path) = paths.into_iter().next() else {
                            return;
                        };

                        if let Some(handle) = ctx.handle().upgrade(ctx) {
                            handle.update(ctx, |workspace, ctx| {
                                workspace.handle_open_repository(&path, ctx);
                            });
                        }
                    }
                    Err(err) => {
                        let window_id = ctx.window_id();
                        ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                            toast_stack.add_ephemeral_toast(
                                DismissibleToast::error(format!("{err}")),
                                window_id,
                                ctx,
                            );
                        });
                    }
                },
                FilePickerConfiguration::new().folders_only(),
            ),
        }
    }

    fn handle_open_repository(&mut self, path: &str, ctx: &mut ViewContext<Self>) {
        let path_buf = PathBuf::from(path);
        ProjectManagementModel::handle(ctx).update(ctx, |projects, ctx| {
            projects.upsert_project(path_buf.clone(), ctx);
        });
        self.add_tab_with_pane_layout(
            PanesLayout::SingleTerminal(Box::new(NewTerminalOptions {
                initial_directory: Some(path_buf.clone()),
                hide_homepage: true,
                ..Default::default()
            })),
            Arc::new(HashMap::new()),
            None,
            ctx,
        );
        self.active_tab_pane_group().update(ctx, |tab, ctx| {
            if let Some(active_terminal) = tab.active_session_view(ctx) {
                active_terminal.update(ctx, |terminal, _| {
                    terminal.maybe_set_pending_repo_init_path(path_buf);
                });
            }
        });
    }

    /// Navigate to an existing AI conversation, focusing on its terminal view, if it's open anywhere.
    /// If the conversation is not in an open pane, restore it based on the provided layout override
    /// or the user's setting.
    fn restore_or_navigate_to_conversation(
        &mut self,
        conversation_id: AIConversationId,
        window_id: Option<WindowId>,
        pane_view_locator: Option<PaneViewLocator>,
        terminal_view_id: Option<EntityId>,
        mut restore_layout: Option<RestoreConversationLayout>,
        ctx: &mut ViewContext<Self>,
    ) {
        // If we have all required navigation data, try to navigate to the existing pane
        if let (Some(pane_view_locator), Some(window_id), Some(terminal_view_id)) =
            (pane_view_locator, window_id, terminal_view_id)
        {
            // The pane group will be in the undo stack if its parent view (either the tab or the split pane)
            // has been recently closed, and this closure can still be undone.
            let is_pane_in_undo_stack = UndoCloseStack::as_ref(ctx)
                .is_pane_group_tab_in_stack(pane_view_locator.pane_group_id)
                || ctx
                    .view_with_id::<PaneGroup>(window_id, pane_view_locator.pane_group_id)
                    .is_some_and(|pg| {
                        pg.as_ref(ctx)
                            .is_pane_hidden_for_close(pane_view_locator.pane_id)
                    });
            // Check if there's an active long-running command in the target terminal and target conversation.
            let has_blocking_long_running_command = ctx
                .view_with_id::<TerminalView>(window_id, terminal_view_id)
                .map(|view| {
                    let model = view.as_ref(ctx).model.lock();
                    let is_long_running = model
                        .block_list()
                        .active_block()
                        .is_active_and_long_running();
                    let selected_conversation_id = view
                        .as_ref(ctx)
                        .ai_context_model()
                        .as_ref(ctx)
                        .selected_conversation_id(ctx);

                    is_long_running && selected_conversation_id != Some(conversation_id)
                })
                .unwrap_or(false);
            // If the pane group is in the undo stack, we want to make sure its tab/pane parent
            // is not re-openable (as this would cause a duplicate of this conversation to be created).
            if is_pane_in_undo_stack {
                // Don't open in this pane, fall back to user setting
                restore_layout = None;

                // TODO(harry): this does not detect correctly when the conversation is part of a window that has been closed.
                UndoCloseStack::handle(ctx).update(ctx, |stack, ctx| {
                    stack.discard_pane_group_parent(pane_view_locator.pane_group_id, ctx);
                });
            } else if has_blocking_long_running_command {
                // Don't open in this pane and use the existing restore layout.
            } else {
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, ctx| {
                    history_model.set_active_conversation_id(
                        conversation_id,
                        terminal_view_id,
                        ctx,
                    );
                });

                Self::set_pending_query_state_for_terminal_view(
                    terminal_view_id,
                    PendingQueryState::Existing { conversation_id },
                    ctx,
                );

                // If the conversation is in the current window, focus the pane directly.
                // Otherwise, dispatch an action to the appropriate window.
                if window_id == ctx.window_id() {
                    ctx.windows().show_window_and_focus_app(window_id);
                    self.focus_pane(pane_view_locator, ctx);
                } else if let Some(root_view_id) = ctx.root_view_id(window_id) {
                    ctx.dispatch_action_for_view(
                        window_id,
                        root_view_id,
                        "root_view:handle_pane_navigation_event",
                        &pane_view_locator,
                    );
                }
                return;
            }
        }

        // Determine effective layout: use provided layout or fall back to setting
        #[cfg(feature = "local_fs")]
        let layout_from_setting =
            match *EditorSettings::as_ref(ctx).open_conversation_layout_preference {
                OpenConversationPreference::NewTab => RestoreConversationLayout::NewTab,
                OpenConversationPreference::SplitPane => RestoreConversationLayout::SplitPane,
            };
        #[cfg(not(feature = "local_fs"))]
        let layout_from_setting = RestoreConversationLayout::NewTab;

        let effective_layout = restore_layout.unwrap_or(layout_from_setting);
        // Handle based on effective layout
        match effective_layout {
            RestoreConversationLayout::ActivePane => {
                self.restore_conversation_in_active_pane(conversation_id, ctx);
            }
            RestoreConversationLayout::SplitPane => {
                self.restore_conversation_in_split_pane(conversation_id, ctx);
            }
            RestoreConversationLayout::NewTab => {
                self.restore_conversation_in_new_tab(conversation_id, ctx);
            }
        }
    }

    /// Restores a conversation into the active terminal pane.
    /// Shows a full-screen loading state while fetching, then restores the conversation into the existing terminal.
    /// Falls back to new tab if we cannot restore into the active pane (has a long-running command or is invalid).
    fn restore_conversation_in_active_pane(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ViewContext<Self>,
    ) {
        let terminal_view_for_active_pane = self.active_session_view(ctx).filter(|_| {
            self.get_active_session_terminal_model(ctx).is_some()
                && FeatureFlag::AgentView.is_enabled()
        });

        // If we can't restore in the active pane, fall back to restoring in new tab.
        let Some(terminal_view) = &terminal_view_for_active_pane else {
            self.restore_conversation_in_new_tab(conversation_id, ctx);
            return;
        };
        // Check if there's an active long-running command in the active terminal.
        // If not, set the active terminal view to loading since we're going to restore in the active pane.
        // We do these operations together to hold the model lock across them.
        let active_pane_has_long_running_command =
            terminal_view.update(ctx, |terminal_view, ctx| {
                let mut model_lock = terminal_view.model.lock();
                if model_lock
                    .block_list()
                    .active_block()
                    .is_active_and_long_running()
                {
                    return true;
                }
                // Active pane does not have a long running command. We're going to restore in the active pane
                // so set the loading status atomically.
                model_lock.set_conversation_transcript_viewer_status(Some(
                    ConversationTranscriptViewerStatus::Loading,
                ));
                ctx.notify();
                false
            });
        // If there's an active long-running command in the active terminal, fall back to restoring in new tab.
        if active_pane_has_long_running_command {
            self.restore_conversation_in_new_tab(conversation_id, ctx);
            return;
        }

        let history_model = BlocklistAIHistoryModel::handle(ctx);
        let future = history_model
            .as_ref(ctx)
            .load_conversation_data(conversation_id, ctx);
        let terminal_view_for_closure = terminal_view.clone();
        let window_id = ctx.window_id();
        ctx.spawn(future, move |_workspace, conversation, ctx| {
            let Some(conversation) = conversation else {
                log::warn!("Failed to load conversation {conversation_id}");
                // Unset the loading status
                terminal_view_for_closure.update(ctx, |terminal_view, ctx| {
                    terminal_view
                        .model
                        .lock()
                        .set_conversation_transcript_viewer_status(None);
                    ctx.notify();
                });
                WorkspaceToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                    let toast = DismissibleToast::error("Failed to load conversation.".to_owned());
                    toast_stack.add_ephemeral_toast(toast, window_id, ctx);
                });
                return;
            };

            terminal_view_for_closure.update(ctx, |terminal_view, ctx| {
                // Unset the loading status
                terminal_view
                    .model
                    .lock()
                    .set_conversation_transcript_viewer_status(None);
                terminal_view.restore_conversation_and_directory_context(
                    conversation,
                    FeatureFlag::AgentView.is_enabled(),
                    |terminal_view, ctx| {
                        terminal_view.redetermine_global_focus(ctx);
                    },
                    ctx,
                );
            });
        });
    }

    /// Restores a conversation in a new split pane.
    /// Creates a loading pane immediately, then replaces it with the real terminal with the conversation once data loads.
    /// We have to do this instead of loading the data into the same terminal pane to avoid problems with
    /// restoring conversations while the shell is bootstrapping.
    fn restore_conversation_in_split_pane(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ViewContext<Self>,
    ) {
        let window_id = ctx.window_id();
        let tab_pane_group = self.active_tab_pane_group().clone();
        let pane_group_id = tab_pane_group.id();
        let loading_pane_id = tab_pane_group.update(ctx, |pane_group, ctx| {
            let base_pane_id = pane_group.focused_pane_id(ctx);
            pane_group.add_loading_conversation_pane(
                PaneGroupDirection::Right,
                Some(base_pane_id),
                ctx,
            )
        });

        let history_model = BlocklistAIHistoryModel::handle(ctx);
        let future = history_model
            .as_ref(ctx)
            .load_conversation_data(conversation_id, ctx);
        ctx.spawn(future, move |_workspace, conversation, ctx| {
            let Some(conversation) = conversation else {
                log::warn!("Failed to load conversation {conversation_id}");
                WorkspaceToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                    let toast = DismissibleToast::error("Failed to load conversation.".to_owned());
                    toast_stack.add_ephemeral_toast(toast, window_id, ctx);
                });
                // Close the loading pane
                if let Some(pane_group) = ctx.view_with_id::<PaneGroup>(window_id, pane_group_id) {
                    pane_group.update(ctx, |pane_group, ctx| {
                        pane_group.close_pane(loading_pane_id, ctx);
                    });
                }
                return;
            };

            // Replace the loading pane with real terminal
            if let Some(pane_group) = ctx.view_with_id::<PaneGroup>(window_id, pane_group_id) {
                pane_group.update(ctx, |pane_group, ctx| {
                    pane_group.replace_loading_pane_with_terminal(
                        loading_pane_id,
                        conversation,
                        ctx,
                    );
                });
            }
        });
    }

    /// Restores a conversation in a new tab.
    /// Creates a new tab with a loading pane immediately, then replaces it with the real terminal with the conversation once data loads.
    /// We have to do this instead of loading the data into the same terminal pane to avoid problems with
    /// restoring conversations while the shell is bootstrapping.
    fn restore_conversation_in_new_tab(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ViewContext<Self>,
    ) {
        let window_id = ctx.window_id();

        // Create a new tab with loading pane
        let new_pane_group = ctx.add_typed_action_view(|ctx| {
            PaneGroup::new_for_conversation_transcript_viewer_loading(
                self.tips_completed.clone(),
                self.user_default_shell_unsupported_banner_model_handle
                    .clone(),
                self.model_event_sender.clone(),
                ctx,
            )
        });

        ctx.subscribe_to_view(&new_pane_group, move |me, pane_group, event, ctx| {
            me.handle_file_tree_event(pane_group, event, ctx)
        });

        self.tabs.push(TabData::new(new_pane_group.clone()));
        let new_tab_index = self.tab_count() - 1;
        self.activate_tab_internal(new_tab_index, ctx);

        // Get both IDs from the NEW tab's pane group
        let pane_group_id = new_pane_group.id();
        let loading_pane_id = new_pane_group.as_ref(ctx).focused_pane_id(ctx);

        let history_model = BlocklistAIHistoryModel::handle(ctx);
        let future = history_model
            .as_ref(ctx)
            .load_conversation_data(conversation_id, ctx);

        ctx.spawn(future, move |workspace, conversation, ctx| {
            let Some(conversation) = conversation else {
                log::warn!("Failed to load conversation {conversation_id}");
                WorkspaceToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                    let toast = DismissibleToast::error("Failed to load conversation.".to_owned());
                    toast_stack.add_ephemeral_toast(toast, window_id, ctx);
                });
                // Close the loading tab
                if let Some(tab_index) = workspace
                    .tabs
                    .iter()
                    .position(|tab| tab.pane_group.id() == pane_group_id)
                {
                    workspace.close_tab(tab_index, true, false, ctx);
                }
                return;
            };

            // Find the tab with this pane_group_id and replace its loading pane
            if let Some(tab_pane_group) = workspace
                .tabs
                .iter()
                .find(|tab| tab.pane_group.id() == pane_group_id)
                .map(|tab| tab.pane_group.clone())
            {
                tab_pane_group.update(ctx, |pane_group, ctx| {
                    pane_group.replace_loading_pane_with_terminal(
                        loading_pane_id,
                        conversation,
                        ctx,
                    );
                });
            }
        });
    }

    fn set_pending_query_state_for_terminal_view(
        terminal_view_id: EntityId,
        pending_query_state: PendingQueryState,
        ctx: &mut AppContext,
    ) {
        let terminal_view = ctx.window_ids().find_map(|window_id| {
            ctx.views_of_type::<TerminalView>(window_id)
                .and_then(|terminal_views| {
                    terminal_views.iter().find_map(|terminal_view| {
                        if terminal_view.as_ref(ctx).view_id() == terminal_view_id {
                            Some(terminal_view.clone())
                        } else {
                            None
                        }
                    })
                })
        });

        if let Some(terminal_view) = terminal_view {
            terminal_view.update(ctx, |terminal_view, ctx| {
                terminal_view.set_pending_query_state(pending_query_state, ctx);
                ctx.notify();
            });
        } else {
            log::warn!(
                "Failed to find terminal view with id {terminal_view_id} to set pending query state"
            );
        }
    }

    /// Fork an existing AI conversation.
    /// Optionally summarizes the conversation after forking and/or sends an initial prompt.
    #[allow(clippy::too_many_arguments)]
    fn fork_ai_conversation(
        &mut self,
        conversation_id: AIConversationId,
        fork_from_exchange: Option<ForkFromExchange>,
        summarize_after_fork: bool,
        summarization_prompt: Option<String>,
        initial_prompt: Option<String>,
        destination: ForkedConversationDestination,
        ctx: &mut ViewContext<Self>,
    ) {
        let history_model = BlocklistAIHistoryModel::handle(ctx);
        let window_id = ctx.window_id();

        let source_terminal_view_id = history_model
            .as_ref(ctx)
            .all_live_conversations()
            .into_iter()
            .find(|(_, convo)| convo.id() == conversation_id)
            .map(|(terminal_view_id, _)| terminal_view_id);

        // An empty prompt should not be provided as a query for the new forked conversation.
        let initial_prompt = initial_prompt.and_then(|prompt| {
            if prompt.trim().is_empty() {
                None
            } else {
                Some(prompt)
            }
        });

        // Load the conversation data asynchronously
        let future = history_model
            .as_ref(ctx)
            .load_conversation_data(conversation_id, ctx);

        ctx.spawn(future, move |workspace, source_conversation, ctx| {
            let Some(LocalConversationData::AI(source_conversation)) = source_conversation else {
                log::error!("Failed to load local conversation {conversation_id} for forking.");
                WorkspaceToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                    let toast = DismissibleToast::error(
                        "Failed to load conversation for forking.".to_owned(),
                    );
                    toast_stack.add_ephemeral_toast(toast, window_id, ctx);
                });
                return;
            };

            let history_model = BlocklistAIHistoryModel::handle(ctx);
            let fork_result = history_model.update(ctx, |history_model, ctx| {
                if let Some(fork_from) = fork_from_exchange {
                    history_model.fork_conversation_at_exchange(
                        &source_conversation,
                        fork_from.exchange_id,
                        fork_from.fork_from_exact_exchange,
                        FORK_PREFIX,
                        ctx,
                    )
                } else {
                    history_model.fork_conversation(&source_conversation, FORK_PREFIX, ctx)
                }
            });

            let forked_conversation = match fork_result {
                Ok(forked_conversation) => forked_conversation,
                Err(e) => {
                    log::error!("Conversation forking failed. {e}.");
                    WorkspaceToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                        let toast =
                            DismissibleToast::error("Conversation forking failed.".to_owned());
                        toast_stack.add_ephemeral_toast(toast, window_id, ctx);
                    });
                    return;
                }
            };

            // Handle forking into the current pane
            if destination.is_current_pane() {
                if let Some(terminal_view) = workspace.active_session_view(ctx) {
                    let forked_conversation_id = forked_conversation.id();
                    terminal_view.update(ctx, move |terminal_view, ctx| {
                        terminal_view.restore_conversation_after_view_creation(
                            RestoredAIConversation::new(forked_conversation.clone()),
                            true,
                            ctx,
                        );
                        terminal_view
                            .maybe_show_restore_context_hint(RestorationDirState::Unchanged, ctx);

                        terminal_view.redetermine_global_focus(ctx);
                    });

                    Self::handle_forked_conversation_prompts(
                        terminal_view,
                        summarize_after_fork,
                        summarization_prompt,
                        initial_prompt,
                        forked_conversation_id,
                        ctx,
                    );

                    Self::show_fork_toast(conversation_id, window_id, ctx);
                    return;
                }
                // If no active session view, fall through to create a new pane
                log::warn!("CurrentPane fork requested with no active session view");
            }

            // Respect the explicit destination: SplitPane opens a split pane, NewTab opens a
            // new tab. `open_conversation_layout_preference` is only consulted as a fallback by
            // `restore_or_navigate_to_conversation`; fork callers always pass an explicit
            // destination, so overriding SplitPane with NewTab here would silently defeat the
            // user's choice (e.g. `/fork` with Enter explicitly picks SplitPane).
            let should_open_in_new_tab = destination.is_new_tab();

            if should_open_in_new_tab {
                let forked_conversation_id = forked_conversation.id();
                workspace.add_new_session_tab_with_default_mode(
                    NewSessionSource::Tab,
                    Some(window_id),
                    None,
                    Some(ConversationRestorationInNewPaneType::Forked {
                        conversation: forked_conversation,
                    }),
                    false,
                    ctx,
                );

                // Handle sending summarize and/or initial prompt to the forked conversation
                if let Some(terminal_view) = workspace
                    .active_tab_pane_group()
                    .as_ref(ctx)
                    .active_session_view(ctx)
                {
                    // Copy model selection and execution profile from source to new terminal view
                    if let Some(source_id) = source_terminal_view_id {
                        Self::copy_model_and_profile_to_terminal_view(
                            source_id,
                            terminal_view.id(),
                            ctx,
                        );
                    }

                    Self::handle_forked_conversation_prompts(
                        terminal_view,
                        summarize_after_fork,
                        summarization_prompt,
                        initial_prompt,
                        forked_conversation_id,
                        ctx,
                    );
                }

                Self::show_fork_toast(conversation_id, window_id, ctx);
                return;
            }

            let active_pane_group = workspace.active_tab_pane_group();
            let active_pane_group_id = active_pane_group.id();
            let created_pane_id: PaneId = active_pane_group.update(ctx, |pane_group, ctx| {
                let active_pane_id = pane_group.focused_pane_id(ctx);

                let new_pane_id = pane_group.add_session(
                    PaneGroupDirection::Right,
                    Some(active_pane_id),
                    active_pane_id.as_terminal_pane_id(),
                    None, /* chosen_shell */
                    Some(ConversationRestorationInNewPaneType::Forked {
                        conversation: forked_conversation.clone(),
                    }),
                    ctx,
                );

                new_pane_id.into()
            });

            // Handle sending summarize and/or initial prompt to the forked conversation
            let forked_conversation_id = forked_conversation.id();
            let tab_pane_group_handle = active_pane_group.clone();
            if let Some(terminal_view) = tab_pane_group_handle.as_ref(ctx).focused_session_view(ctx)
            {
                // Copy model selection and execution profile from source to new terminal view
                if let Some(source_id) = source_terminal_view_id {
                    Self::copy_model_and_profile_to_terminal_view(
                        source_id,
                        terminal_view.id(),
                        ctx,
                    );
                }

                Self::handle_forked_conversation_prompts(
                    terminal_view,
                    summarize_after_fork,
                    summarization_prompt,
                    initial_prompt,
                    forked_conversation_id,
                    ctx,
                );
            }
            // After splitting, focus the newly created pane
            let locator = PaneViewLocator {
                pane_group_id: active_pane_group_id,
                pane_id: created_pane_id,
            };
            workspace.focus_pane(locator, ctx);

            Self::show_fork_toast(conversation_id, window_id, ctx);
        });
    }

    /// Handle sending summarize and/or initial prompt to a forked conversation.
    fn handle_forked_conversation_prompts(
        terminal_view: ViewHandle<TerminalView>,
        summarize_after_fork: bool,
        summarization_prompt: Option<String>,
        initial_prompt: Option<String>,
        forked_conversation_id: AIConversationId,
        ctx: &mut ViewContext<Self>,
    ) {
        if !summarize_after_fork && initial_prompt.is_none() {
            return;
        }

        terminal_view.update(ctx, |terminal_view, terminal_view_ctx| {
            if summarize_after_fork {
                terminal_view
                    .ai_controller()
                    .update(terminal_view_ctx, |controller, ctx| {
                        controller.send_slash_command_request(
                            SlashCommandRequest::Summarize {
                                prompt: summarization_prompt,
                            },
                            ctx,
                        );
                    });

                if let Some(prompt) = initial_prompt {
                    terminal_view.send_user_query_after_next_conversation_finished(
                        prompt,
                        /* show_close_button */ true,
                        /* show_send_now_button */ false,
                        terminal_view_ctx,
                    );
                }
            } else if let Some(prompt) = initial_prompt {
                terminal_view
                    .ai_controller()
                    .update(terminal_view_ctx, |controller, ctx| {
                        controller.send_user_query_in_conversation_no_lrc_subagent(
                            prompt,
                            forked_conversation_id,
                            ctx,
                        );
                    });
            }
        });
    }

    /// Copy the model selection and execution profile from the source terminal view to a new terminal view.
    fn copy_model_and_profile_to_terminal_view(
        source_terminal_view_id: EntityId,
        new_terminal_view_id: EntityId,
        ctx: &mut ViewContext<Self>,
    ) {
        // Copy the LLM preference from source to new terminal view
        let source_llm_id = LLMPreferences::as_ref(ctx)
            .get_active_base_model(ctx, Some(source_terminal_view_id))
            .id
            .clone();
        LLMPreferences::handle(ctx).update(ctx, |prefs, ctx| {
            prefs.update_preferred_agent_mode_llm(&source_llm_id, new_terminal_view_id, ctx);
        });

        // Copy the execution profile from source to new terminal view
        let source_profile_id = *AIExecutionProfilesModel::as_ref(ctx)
            .active_profile(Some(source_terminal_view_id), ctx)
            .id();
        AIExecutionProfilesModel::handle(ctx).update(ctx, |profiles, ctx| {
            profiles.set_active_profile(new_terminal_view_id, source_profile_id, ctx);
        });
    }

    /// Show a toast notification for a forked conversation.
    fn show_fork_toast(
        conversation_id: AIConversationId,
        window_id: WindowId,
        ctx: &mut ViewContext<Self>,
    ) {
        let history_model = BlocklistAIHistoryModel::handle(ctx);
        let source_title = history_model
            .as_ref(ctx)
            .conversation(&conversation_id)
            .and_then(|c| c.title())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "Conversation".to_string());

        let title = if source_title.chars().count() > MAX_FORK_TOAST_TITLE_LENGTH {
            let truncated: String = source_title
                .chars()
                .take(MAX_FORK_TOAST_TITLE_LENGTH)
                .collect();
            format!("{truncated}...")
        } else {
            source_title
        };

        WorkspaceToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
            let toast = DismissibleToast::default(format!("Forked \"{title}\""));
            toast_stack.add_ephemeral_toast(toast, window_id, ctx);
        });
    }

    fn summarize_active_ai_conversation(
        &mut self,
        prompt: Option<String>,
        initial_prompt: Option<String>,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(terminal_view) = self
            .active_tab_pane_group()
            .as_ref(ctx)
            .active_session_view(ctx)
        else {
            return;
        };

        terminal_view.update(ctx, |terminal, ctx| {
            terminal.ai_controller().update(ctx, |controller, ctx| {
                controller
                    .send_slash_command_request(SlashCommandRequest::Summarize { prompt }, ctx);
            });

            if let Some(prompt) = initial_prompt {
                terminal.send_user_query_after_next_conversation_finished(
                    prompt, /* show_close_button */ true,
                    /* show_send_now_button */ false, ctx,
                );
            }
        });
    }

    /// Handle a tab being dragged
    ///
    /// Will determine if the dragged tab needs to be swapped with another tab in the list and
    /// perform the swap, making sure to maintain the active tab if necessary
    fn on_tab_drag(&mut self, current_index: usize, position: RectF, ctx: &mut ViewContext<Self>) {
        let new_index = if FeatureFlag::VerticalTabs.is_enabled()
            && *TabSettings::as_ref(ctx).use_vertical_tabs
        {
            self.calculate_updated_tab_index_vertical(current_index, position, ctx)
        } else {
            self.calculate_updated_tab_index(current_index, position, ctx)
        };

        if new_index != current_index {
            self.tabs.swap(new_index, current_index);

            // Update the active tab index if it was impacted by the swap
            if current_index == self.active_tab_index {
                self.set_active_tab_index(new_index, ctx);
            } else if new_index == self.active_tab_index {
                self.set_active_tab_index(current_index, ctx);
            }

            ctx.notify();
        }
    }

    /// Determines the appropriate index for a tab that is being dragged, based on its current
    /// index and drag position
    ///
    /// We check if the midpoint of the dragged tab has crossed into the boundary of either
    /// surrounding tab. For the tab immediately to the left, this means checking against the
    /// rightmost boundary, while for the tab immediately to the right, we check against the
    /// leftmost boundary.
    ///
    /// If the midpoint is not in either location, then we return the current index, as the tab has
    /// not moved out of its position
    fn calculate_updated_tab_index(
        &self,
        current_index: usize,
        drag_position: RectF,
        ctx: &mut ViewContext<Self>,
    ) -> usize {
        let midpoint_drag_x = (drag_position.min_x() + drag_position.max_x()) / 2.;

        let maybe_left_tab = if current_index > 0 {
            ctx.element_position_by_id(tab_position_id(current_index - 1))
        } else {
            None
        };
        if let Some(tab_position) = maybe_left_tab {
            if midpoint_drag_x < tab_position.max_x() {
                return current_index - 1;
            }
        }

        let maybe_right_tab = if current_index < self.tabs.len() - 1 {
            ctx.element_position_by_id(tab_position_id(current_index + 1))
        } else {
            None
        };
        if let Some(tab_position) = maybe_right_tab {
            if midpoint_drag_x > tab_position.min_x() {
                return current_index + 1;
            }
        }

        current_index
    }

    /// Y-axis variant of `calculate_updated_tab_index` for vertical tab layout.
    ///
    /// Uses midpoint-of-neighbor thresholds rather than edge thresholds to prevent
    /// oscillation when groups have different heights.
    fn calculate_updated_tab_index_vertical(
        &self,
        current_index: usize,
        drag_position: RectF,
        ctx: &mut ViewContext<Self>,
    ) -> usize {
        let midpoint_drag_y = (drag_position.min_y() + drag_position.max_y()) / 2.;

        let maybe_above_tab = if current_index > 0 {
            ctx.element_position_by_id(tab_position_id(current_index - 1))
        } else {
            None
        };
        if let Some(tab_position) = maybe_above_tab {
            let neighbor_midpoint_y = (tab_position.min_y() + tab_position.max_y()) / 2.;
            if midpoint_drag_y < neighbor_midpoint_y {
                return current_index - 1;
            }
        }

        let maybe_below_tab = if current_index < self.tabs.len() - 1 {
            ctx.element_position_by_id(tab_position_id(current_index + 1))
        } else {
            None
        };
        if let Some(tab_position) = maybe_below_tab {
            let neighbor_midpoint_y = (tab_position.min_y() + tab_position.max_y()) / 2.;
            if midpoint_drag_y > neighbor_midpoint_y {
                return current_index + 1;
            }
        }

        current_index
    }

    // Move tab, given tab index, left or right
    fn move_tab(&mut self, index: usize, direction: TabMovement, ctx: &mut ViewContext<Self>) {
        let tabs_len = self.tabs.len();
        let new_index = match direction {
            TabMovement::Left if index > 0 => index - 1,
            TabMovement::Right if index < tabs_len - 1 => index + 1,
            _ => return,
        };
        // Don't need to worry about negative numbers because that case is covered above
        self.tabs.swap(index, new_index);

        if index == self.active_tab_index {
            self.set_active_tab_index(new_index, ctx);
        } else {
            // Don't want to change the active tab for the user due to an adjacent
            // tab being moved left/right.
            if new_index == self.active_tab_index {
                self.set_active_tab_index(index, ctx);
            }
        }

        ctx.notify();
    }

    /// How to render the tab bar.
    fn tab_bar_mode(&self, app: &AppContext) -> ShowTabBar {
        if self.should_show_session_config_tab_config_chip() {
            return ShowTabBar::Stacked;
        }

        if !FeatureFlag::FullScreenZenMode.is_enabled() {
            return ShowTabBar::default();
        }

        let is_fullscreen = app
            .windows()
            .platform_window(self.window_id)
            .is_some_and(|window| window.fullscreen_state() == FullscreenState::Fullscreen);

        let is_hovered = self
            .tab_bar_hover_state
            .lock()
            .is_ok_and(|state| state.is_hovered())
            || self.traffic_light_mouse_states.are_traffic_lights_hovered();

        // Check if any of the menus/popups rendered relative to the tab bar are open.
        let is_vertical_tabs_active = FeatureFlag::VerticalTabs.is_enabled()
            && *TabSettings::as_ref(app).use_vertical_tabs
            && self.vertical_tabs_panel_open;
        let is_tab_menu_open = self.show_tab_bar_overflow_menu
            || (self.show_tab_right_click_menu.is_some() && !is_vertical_tabs_active)
            || (self.show_new_session_dropdown_menu.is_some() && !is_vertical_tabs_active)
            || self.is_user_menu_open
            || self.tab_bar_pinned_by_popup;

        // Check if any panes are being dragged (potentially into a new tab).
        let is_pane_being_dragged = self
            .active_tab_pane_group()
            .as_ref(app)
            .any_pane_being_dragged(app);

        let workspace_decoration_visibility = TabSettings::as_ref(app)
            .workspace_decoration_visibility
            .value();

        let hovered_visibility = if is_pane_being_dragged || is_hovered || is_tab_menu_open {
            ShowTabBar::Stacked
        } else {
            ShowTabBar::Hidden
        };

        match workspace_decoration_visibility {
            WorkspaceDecorationVisibility::OnHover => hovered_visibility,
            // If the tab bar is hidden when fullscreen, show/hide on hover.
            WorkspaceDecorationVisibility::HideFullscreen if is_fullscreen => hovered_visibility,
            // If the user always wants a tab bar OR the window isn't fullscreen, make it
            // persistently stacked above the content area.
            _ => ShowTabBar::Stacked,
        }
    }

    #[cfg(target_os = "macos")]
    pub fn sync_window_button_visibility(&self, ctx: &mut ViewContext<Self>) {
        use warpui::platform::mac::WindowExt;
        let show = if FeatureFlag::FullScreenZenMode.is_enabled()
            && TabSettings::as_ref(ctx)
                .workspace_decoration_visibility
                .value()
                == &WorkspaceDecorationVisibility::OnHover
        {
            self.tab_bar_mode(ctx).has_tab_bar()
        } else {
            TabSettings::as_ref(ctx)
                .workspace_decoration_visibility
                .show_window_decorations()
        };
        if let Some(platform_window) = ctx.windows().platform_window(ctx.window_id()) {
            platform_window.as_ref().set_window_buttons(show);
        }
    }

    #[cfg(not(target_os = "macos"))]
    pub fn sync_window_button_visibility(&self, _: &mut ViewContext<Self>) {
        // Only macOS uses native window buttons.
    }

    /// Updates the titlebar height to match the scaled tab bar height.
    pub fn update_titlebar_height(&self, ctx: &mut ViewContext<Self>) {
        let zoom_factor = WindowSettings::as_ref(ctx).zoom_level.as_zoom_factor();
        let scaled_tab_bar_height = (TOTAL_TAB_BAR_HEIGHT * zoom_factor) as f64;

        if let Some(platform_window) = ctx.windows().platform_window(ctx.window_id()) {
            platform_window
                .as_ref()
                .set_titlebar_height(scaled_tab_bar_height);
        }
    }

    fn request_notification_permissions_if_needed(&mut self, ctx: &mut ViewContext<Self>) {
        // Request permissions any time notifications are currently enabled.
        let current_mode = SessionSettings::as_ref(ctx).notifications.value().mode;

        if current_mode == NotificationsMode::Enabled {
            ctx.request_desktop_notification_permissions(move |view, outcome, ctx| {
                match &outcome {
                    RequestPermissionsOutcome::Accepted => (),
                    RequestPermissionsOutcome::PermissionsDenied => {
                        // Show a helpful toast if the user denied permissions.
                        let url = NOTIFICATIONS_TROUBLESHOOT_URL.to_string();
                        view.toast_stack.update(ctx, |toast_stack, ctx| {
                            let toast = DismissibleToast::error(
                                "Warper doesn't have permission to send desktop notifications.".to_string(),
                            )
                            .with_link(ToastLink::new("Troubleshoot notifications".to_string()).with_href(url));
                            toast_stack.add_persistent_toast(toast, ctx);
                        });
                    }
                    RequestPermissionsOutcome::OtherError { error_message } => {
                        log::error!(
                            "Unknown error when requesting notification permissions. error_msg: {error_message}"
                        );
                    }
                }
                            });
        }
    }

    fn toggle_notifications(&mut self, ctx: &mut ViewContext<Self>) {
        let current_settings = SessionSettings::as_ref(ctx).notifications.value().clone();
        let previous_mode = current_settings.mode;
        let new_mode = match previous_mode {
            NotificationsMode::Unset | NotificationsMode::Dismissed => NotificationsMode::Enabled,
            NotificationsMode::Enabled => NotificationsMode::Disabled,
            NotificationsMode::Disabled => NotificationsMode::Enabled,
        };

        SessionSettings::handle(ctx).update(ctx, |settings, ctx| {
            let new_notifications = NotificationsSettings {
                mode: new_mode,
                ..current_settings
            };
            if let Err(e) = settings.notifications.set_value(new_notifications, ctx) {
                log::error!("Error persisting notifications setting: {e}");
            }
        });
    }

    fn open_command_palette(&mut self, ctx: &mut ViewContext<Self>) {
        self.tips_completed.update(ctx, |tips_completed, ctx| {
            mark_feature_used_and_write_to_user_defaults(
                Tip::Action(TipAction::CommandPalette),
                tips_completed,
                ctx,
            );
            ctx.notify();
        });

        self.palette.update(ctx, |view, ctx| {
            view.reset(ctx);
        });
    }

    fn open_files_palette(&mut self, ctx: &mut ViewContext<Self>) {
        self.tips_completed.update(ctx, |tips_completed, ctx| {
            mark_feature_used_and_write_to_user_defaults(
                Tip::Action(TipAction::CommandPalette),
                tips_completed,
                ctx,
            );
            ctx.notify();
        });

        self.palette.update(ctx, |view, ctx| {
            view.reset(ctx);
            // Reset mixer with correct file data source before setting filter
            let mixer = view.search_bar.as_ref(ctx).mixer().clone();
            view.data_source_store.update(ctx, |store, ctx| {
                store.reset_search_mixer(mixer, ctx);
            });
            view.set_active_query_filter(QueryFilter::Files, ctx);
        });
    }
    fn set_command_palette_binding_source(
        &mut self,
        source: PaletteSource,
        ctx: &mut ViewContext<Self>,
    ) {
        let window_id = ctx.window_id();
        // Safety: Unwrap is okay here because we just retrieved the window_id from the context
        // so we know it exists
        let view_id = ctx
            .focused_view_id(window_id)
            .expect("Just retrieved the window_id from the context.");

        let active_palette_handle = if matches!(source, PaletteSource::CtrlTab { .. }) {
            &self.ctrl_tab_palette
        } else {
            &self.palette
        };
        active_palette_handle.update(ctx, |view, ctx| {
            view.set_binding_source(window_id, view_id, ctx);
            ctx.notify();
        });
    }

    fn open_navigation_palette(&mut self, ctx: &mut ViewContext<Self>) {
        self.palette.update(ctx, |view, ctx| {
            view.reset(ctx);
            view.set_active_query_filter(QueryFilter::Sessions, ctx);
            view.set_initial_selection_offset(0, ctx);
        });
        ctx.notify();
    }

    fn open_recent_repos_and_convos_palette(&mut self, ctx: &mut ViewContext<Self>) {
        self.palette.update(ctx, |view, ctx| {
            view.reset(ctx);
            view.set_fixed_query_filters(
                "Search recent repos and conversations".to_string(),
                vec![QueryFilter::HistoricalConversations, QueryFilter::Repos],
                ctx,
            );
        });
    }

    fn open_conversations_palette(&mut self, ctx: &mut ViewContext<Self>) {
        self.palette.update(ctx, |view, ctx| {
            view.reset(ctx);
            view.set_active_query_filter(QueryFilter::Conversations, ctx);
            view.set_initial_selection_offset(0, ctx);
        });
        ctx.notify();
    }

    fn open_ctrl_tab_palette(
        &mut self,
        shift_pressed_initially: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        let offset = if shift_pressed_initially { -1 } else { 1 };
        self.ctrl_tab_palette.update(ctx, |view, ctx| {
            view.reset(ctx);
            view.set_active_query_filter(QueryFilter::Sessions, ctx);
            view.set_initial_selection_offset(offset, ctx);
        });
        ctx.notify();
    }

    fn set_navigation_palette_session_source(
        &mut self,
        source: PaletteSource,
        ctx: &mut ViewContext<Self>,
    ) {
        let active_pane_id = self
            .active_tab_pane_group()
            .as_ref(ctx)
            .focused_pane_id(ctx);
        let active_tab_id = self
            .tabs
            .get(self.active_tab_index)
            .map(|tab| tab.pane_group.id());
        let active_window_id = ctx.window_id();

        let active_palette_handle = if matches!(source, PaletteSource::CtrlTab { .. }) {
            &self.ctrl_tab_palette
        } else {
            &self.palette
        };
        active_palette_handle.update(ctx, |view, ctx| {
            // Set the session source when the active_tab_id is Some.
            if let Some(active_tab_id) = active_tab_id {
                view.set_session_source(
                    SessionSource::Set {
                        active_pane_id,
                        active_tab_id,
                        active_window_id,
                    },
                    ctx,
                );
                ctx.notify();
            }
            ctx.notify();
        });
    }

    fn set_palette_sources(&mut self, source: PaletteSource, ctx: &mut ViewContext<Self>) {
        self.set_command_palette_binding_source(source, ctx);
        self.set_navigation_palette_session_source(source, ctx);
    }

    fn open_launch_config_palette(&mut self, ctx: &mut ViewContext<Self>) {
        self.palette.update(ctx, |view, ctx| {
            view.reset(ctx);
            view.set_active_query_filter(QueryFilter::LaunchConfigurations, ctx);
        });
    }

    fn close_palette(
        &mut self,
        focus_active_tab: bool,
        accepted_action_type: Option<&'static str>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.current_workspace_state.is_palette_open = false;
        self.current_workspace_state.is_ctrl_tab_palette_open = false;
        self.tab_bar_pinned_by_popup = false;
        self.sync_window_button_visibility(ctx);
        if focus_active_tab
            // If the user did not do any action on the command palette (eg. closed via shortcut or clicking away)
            // we always force the focus back onto the terminal input
            // Otherwise we check if any other views are open before moving focus back to terminal input
            && (accepted_action_type.is_none()
                || !self
                    .current_workspace_state
                    .is_any_non_terminal_view_open())
        {
            self.focus_active_tab(ctx);
        }
        ctx.notify();
    }

    /// Close all overlays in this workspace and the active pane group.
    fn close_all_overlays(&mut self, ctx: &mut ViewContext<Self>) {
        self.current_workspace_state.close_all_modals();
        self.close_tab_bar_overflow_menu(ctx);
        self.close_all_chip_menus(ctx);

        self.active_tab_pane_group()
            .update(ctx, |pane_group, ctx| pane_group.close_overlays(ctx));
    }

    /// Close all chip menus across all inputs to prevent overlapping with modals.
    /// This is a defensive measure to ensure chip menus don't stay open when focus-stealing modals appear.
    fn close_all_chip_menus(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(active_input_handle) = self.get_active_input_view_handle(ctx) {
            active_input_handle.update(ctx, |input, ctx| {
                input.prompt_render_helper.prompt_view().update(
                    ctx,
                    |prompt_display, prompt_ctx| {
                        prompt_display.close_all_chip_menus(prompt_ctx);
                    },
                );
            });
        }
    }

    fn open_palette(
        &mut self,
        mode: PaletteMode,
        source: PaletteSource,
        ctx: &mut ViewContext<Self>,
    ) {
        self.close_all_overlays(ctx);

        if matches!(source, PaletteSource::TitleBarSearchBar) {
            self.tab_bar_pinned_by_popup = true;
        }
        if matches!(source, PaletteSource::CtrlTab { .. }) {
            self.current_workspace_state.is_ctrl_tab_palette_open = true;
        } else {
            self.current_workspace_state.is_palette_open = true;
        }
        match mode {
            PaletteMode::Command => self.open_command_palette(ctx),
            PaletteMode::Navigation => match source {
                PaletteSource::CtrlTab {
                    shift_pressed_initially,
                } => self.open_ctrl_tab_palette(shift_pressed_initially, ctx),
                _ => self.open_navigation_palette(ctx),
            },
            PaletteMode::LaunchConfig => self.open_launch_config_palette(ctx),
            PaletteMode::Files => self.open_files_palette(ctx),
            PaletteMode::Conversations => self.open_conversations_palette(ctx),
            PaletteMode::ConversationsAndRepos => self.open_recent_repos_and_convos_palette(ctx),
        }

        ctx.focus(&self.palette);

        ctx.notify();
    }

    /// Implements the WorkspaceAction::OpenPalette. This method makes sure the palette is open and
    /// has up-to-date sources. Use this if you don't want toggle semantics.
    fn open_palette_action(
        &mut self,
        palette_mode: PaletteMode,
        source: PaletteSource,
        with_content: Option<&str>,
        ctx: &mut ViewContext<Self>,
    ) {
        // ensure the palette sources are up-to-date, e.g. maybe there is already a navigation
        // palette open and then new sessions were opened after that
        self.set_palette_sources(source, ctx);
        self.open_palette(palette_mode, source, ctx);
        if let Some(text) = with_content {
            self.palette.update(ctx, |palette, ctx| {
                palette.insert_query_text(text, ctx);
            });
        }
    }

    pub fn is_palette_mode_enabled(&self, palette_mode: PaletteMode, app: &AppContext) -> bool {
        self.palette.as_ref(app).is_mode_enabled(palette_mode, app)
    }

    /// Toggle the open / closed state of the palette (so that hitting shortcut a second time
    /// will close the palette)
    fn toggle_palette(
        &mut self,
        palette_mode: PaletteMode,
        source: PaletteSource,
        ctx: &mut ViewContext<Self>,
    ) {
        // If the invite modal is open, don't show the palette since it won't be visible anyway
        if !self.current_workspace_state.is_any_non_palette_modal_open() {
            let is_palette_mode_already_open =
                self.palette.as_ref(ctx).is_mode_enabled(palette_mode, ctx)
                    && ((matches!(source, PaletteSource::CtrlTab { .. })
                        && self.current_workspace_state.is_ctrl_tab_palette_open)
                        || self.current_workspace_state.is_palette_open);
            if is_palette_mode_already_open {
                self.close_palette(true, None, ctx);
            } else {
                self.set_palette_sources(source, ctx);
                self.open_palette(palette_mode, source, ctx);
            }
        }
    }

    fn handle_palette_event(&mut self, event: &CommandPaletteEvent, ctx: &mut ViewContext<Self>) {
        match event {
            CommandPaletteEvent::Close {
                accepted_action_type,
            } => self.close_palette(true, *accepted_action_type, ctx),
            CommandPaletteEvent::ExecuteWorkflow { .. }
            | CommandPaletteEvent::InvokeEnvironmentVariables { .. } => {}
            CommandPaletteEvent::OpenNotebook { .. } => {}
            #[allow(unused_variables)]
            CommandPaletteEvent::OpenFile {
                path,
                line_and_column_arg,
            } => {
                #[cfg(feature = "local_fs")]
                self.open_code(
                    CodeSource::Link {
                        path: path.clone().into(),
                        range_start: None,
                        range_end: None,
                    },
                    *EditorSettings::as_ref(ctx).open_file_layout.value(),
                    *line_and_column_arg,
                    false, // preview
                    &[],
                    ctx,
                );
            }
            CommandPaletteEvent::OpenDirectory { path } => {
                let active_terminal_view = self
                    .active_tab_pane_group()
                    .as_ref(ctx)
                    .active_session_view(ctx);

                if let Some(terminal_view) = active_terminal_view {
                    terminal_view.update(ctx, |terminal_view, ctx| {
                        terminal_view.open_repo_folder(path.to_string(), false, ctx);
                    });
                }
            }
        }
    }

    pub fn is_theme_creator_modal_open(&self) -> bool {
        self.current_workspace_state.is_theme_creator_modal_open
    }

    pub fn is_theme_deletion_modal_open(&self) -> bool {
        self.current_workspace_state.is_theme_deletion_modal_open
    }

    pub fn is_palette_open(&self) -> bool {
        self.current_workspace_state.is_palette_open
            || self.current_workspace_state.is_ctrl_tab_palette_open
    }

    pub fn is_left_panel_open(&self, ctx: &AppContext) -> bool {
        self.active_tab_pane_group().as_ref(ctx).left_panel_open
    }

    fn handle_settings_pane_event(
        &mut self,
        event: &SettingsViewEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            SettingsViewEvent::Pane(_) | SettingsViewEvent::StartResize => {}
            SettingsViewEvent::ShowToast { message, flavor } => {
                self.toast_stack.update(ctx, |toast_stack, ctx| {
                    toast_stack
                        .add_ephemeral_toast(DismissibleToast::new(message.clone(), *flavor), ctx);
                });
            }
            SettingsViewEvent::OpenAIFactCollection => {
                self.open_ai_fact_collection_pane(Some(Direction::Right), None, ctx);
            }
            SettingsViewEvent::OpenMCPServerCollection => {
                self.show_settings_with_section(Some(SettingsSection::MCPServers), ctx);
            }
            SettingsViewEvent::OpenExecutionProfileEditor(profile_id) => {
                self.open_execution_profile_editor_pane(None, *profile_id, ctx);
            }
            SettingsViewEvent::OpenLspLogs { log_path } => {
                self.open_lsp_logs(log_path, ctx);
            }
            SettingsViewEvent::OpenProjectRulesPane { rule_paths } => {
                #[cfg(feature = "local_fs")]
                if let Some((first, rest)) = rule_paths.split_first() {
                    self.open_code(
                        CodeSource::ProjectRules {
                            path: first.clone(),
                        },
                        EditorLayout::SplitPane,
                        None,
                        false,
                        rest,
                        ctx,
                    );
                }
                #[cfg(not(feature = "local_fs"))]
                let _ = rule_paths;
            }
        }
    }

    fn refresh_working_directories_for_pane_group(
        &mut self,
        pane_group: &ViewHandle<PaneGroup>,
        ctx: &mut ViewContext<Self>,
    ) {
        let pane_group_id = pane_group.id();
        let terminal_cwds: Vec<(EntityId, String)> = pane_group
            .as_ref(ctx)
            .terminal_view_working_directories(ctx)
            .filter_map(|(id, cwd)| cwd.map(|c| (id, c)))
            .collect();
        let code_local_paths: Vec<(EntityId, String)> = pane_group
            .as_ref(ctx)
            .code_view_local_paths(ctx)
            .filter_map(|(id, cwd)| cwd.map(|c| (id, c)))
            .collect();
        let code_diff_local_paths: Vec<(EntityId, String)> = pane_group
            .as_ref(ctx)
            .code_diff_view_local_paths(ctx)
            .filter_map(|(id, cwd)| cwd.map(|c| (id, c)))
            .collect();
        let notebook_local_paths: Vec<(EntityId, String)> = pane_group
            .as_ref(ctx)
            .file_notebook_local_paths(ctx)
            .filter_map(|(id, cwd)| cwd.map(|c| (id, c)))
            .collect();
        let local_paths: Vec<(EntityId, String)> = code_local_paths
            .into_iter()
            .chain(notebook_local_paths)
            .chain(code_diff_local_paths)
            .collect();

        // Get the focused terminal ID to prioritize it in the repo_to_terminal map
        let focused_terminal_id = pane_group
            .as_ref(ctx)
            .active_session_view(ctx)
            .map(|terminal_view| terminal_view.id());

        self.working_directories_model.update(ctx, |model, ctx| {
            model.refresh_working_directories_for_pane_group(
                pane_group_id,
                terminal_cwds,
                local_paths,
                focused_terminal_id,
                ctx,
            );
        });
    }

    fn handle_file_tree_event(
        &mut self,
        pane_group: ViewHandle<PaneGroup>,
        event: &pane_group::Event,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            pane_group::Event::AppStateChanged => {
                ctx.dispatch_global_action("workspace:save_app", ());
                self.refresh_working_directories_for_pane_group(&pane_group, ctx);
                self.update_resource_center_action_target(ctx);
                self.update_active_session(ctx);

                if FeatureFlag::DirectoryTabColors.is_enabled() {
                    if let Some(tab) = self
                        .tabs
                        .iter_mut()
                        .find(|t| t.pane_group.id() == pane_group.id())
                    {
                        Self::sync_codebase_tab_color(tab, ctx);
                    }
                }
            }
            pane_group::Event::ActiveSessionChanged => {
                self.update_active_session(ctx);
                // ctx.notify();
            }
            pane_group::Event::Escape => {
                if self.current_workspace_state.is_resource_center_open {
                    self.current_workspace_state.is_resource_center_open = false;
                    ctx.notify()
                }
            }
            pane_group::Event::Exited { add_to_undo_stack } => {
                let tab = self.tabs.iter().position(|t| {
                    t.pane_group.id() == pane_group.id()
                        && t.pane_group.window_id(ctx) == pane_group.window_id(ctx)
                });

                if let Some(tab_index) = tab {
                    self.close_tab(tab_index, true, *add_to_undo_stack, ctx);
                }
            }
            pane_group::Event::PaneTitleUpdated => {
                self.update_window_title(ctx);
                ctx.notify();
            }
            pane_group::Event::ShowCommandSearch(options) => {
                self.show_command_search(options.filter, &options.init_content, ctx);
            }
            pane_group::Event::SendNotification {
                notification,
                pane_id,
            } => {
                // Right now, all notifications are block-specific, but in the future,
                // we might want to serialize a block-agnostic notification context.
                let window_id = ctx.window_id();
                let pane_group_id = pane_group.id();
                let pane_id = *pane_id;
                let notification_data = NotificationContext::BlockOrigin {
                    window_id,
                    pane_group_id,
                    pane_id,
                };

                if let Ok(notification_data_str) = serde_json::to_string(&notification_data) {
                    // Read the notification sound setting from SessionSettings
                    let play_sound = SessionSettings::as_ref(ctx)
                        .notifications
                        .play_notification_sound;

                    ctx.send_desktop_notification(
                        UserNotification::new_with_sound(
                            notification.title.to_string(),
                            notification.body.to_string(),
                            Some(notification_data_str),
                            play_sound,
                        ),
                        move |workspace, notification_error, ctx| {
                            // Log unknown permission errors locally.
                            if let NotificationSendError::Other { error_message } =
                                &notification_error
                            {
                                log::error!(
                                    "Unknown error when sending notification. error_msg: {error_message}"
                                );
                            }

                            // Surface error to user
                            workspace.show_notification_error(
                                notification_error,
                                pane_group_id,
                                pane_id,
                                ctx,
                            );
                        },
                    )
                }
            }
            pane_group::Event::OpenSettings(section) => {
                self.show_settings_with_section(Some(*section), ctx);
            }
            #[cfg(not(target_family = "wasm"))]
            pane_group::Event::OpenPluginInstructionsPane(agent, kind) => {
                self.open_plugin_instructions_pane(*agent, *kind, ctx);
            }
            pane_group::Event::SyncInput(input_type) => {
                self.process_sync_event_for_all_synced_pane_groups(input_type, ctx);
            }
            pane_group::Event::TerminalViewStateChanged => ctx.notify(),
            pane_group::Event::InvalidatedActiveConversation => {
                self.handle_task_status_reset(pane_group.id(), ctx);
            }
            pane_group::Event::ExecuteCommand(execute_event) => {
                // Clear the task status indicator as soon as the user runs a command. If a command is
                // run as part of the task, leave the task marked as in-progress.
                if !execute_event.source.is_ai_command() {
                    self.handle_task_status_reset(pane_group.id(), ctx);
                }
            }
            pane_group::Event::OpenWorkflowModalWithCommand(_)
            | pane_group::Event::OpenLocalWorkflowForEdit(_)
            | pane_group::Event::OpenWorkflowModalWithTemporary(_) => {}
            pane_group::Event::OpenAIFactCollection { sync_id } => {
                let _ = sync_id;
                self.open_ai_fact_collection_pane(None, Some(AIFactPage::Rules), ctx);
            }
            pane_group::Event::OpenPromptEditor => {
                self.open_prompt_editor(ctx);
            }
            pane_group::Event::OpenAgentToolbarEditor => {
                self.open_agent_toolbar_editor(AgentToolbarEditorMode::AgentView, ctx);
            }
            pane_group::Event::OpenCLIAgentToolbarEditor => {
                self.open_agent_toolbar_editor(AgentToolbarEditorMode::CLIAgent, ctx);
            }
            pane_group::Event::OpenMCPSettingsPage { page } => {
                // Open the MCP servers settings page to the list page
                self.open_mcp_servers_page(page.unwrap_or_default(), ctx);
            }
            pane_group::Event::OpenAddRulePane => {
                self.open_ai_fact_collection_pane(None, Some(AIFactPage::Rules), ctx);
            }
            #[cfg_attr(not(feature = "local_fs"), allow(unused_variables))]
            pane_group::Event::OpenFileInWarp { path, session } => {
                #[cfg(feature = "local_fs")]
                {
                    let layout = *EditorSettings::as_ref(ctx).open_file_layout.value();
                    self.open_file_notebook(path.clone(), Some(session.clone()), layout, ctx);
                }
            }
            #[cfg(feature = "local_fs")]
            pane_group::Event::OpenCodeInWarp {
                source,
                layout,
                line_col,
            } => {
                self.open_code(source.clone(), *layout, *line_col, false, &[], ctx);
            }
            #[cfg(feature = "local_fs")]
            pane_group::Event::PreviewCodeInWarp { source } => {
                self.open_code(
                    source.clone(),
                    EditorLayout::SplitPane, // preview always uses split pane
                    None,                    // no line/column for preview
                    true,                    // preview
                    &[],
                    ctx,
                );
            }
            pane_group::Event::OpenCodeDiff { view } => {
                self.open_code_diff(view.clone(), ctx);
            }
            pane_group::Event::AttachPathAsContext { path } => {
                self.attach_path_as_context(path.clone(), ctx);
            }
            pane_group::Event::AttachPlanAsContext { ai_document_id } => {
                self.attach_plan_as_context(*ai_document_id, ctx);
            }
            pane_group::Event::CDToDirectory { path } => {
                self.cd_to_directory(path.clone(), ctx);
            }
            pane_group::Event::OpenDirectoryInNewTab { path } => {
                self.open_directory_in_new_tab(path.clone(), ctx);
            }
            pane_group::Event::RunTabConfigSkill { path } => {
                self.run_tab_config_skill(path, ctx);
            }
            pane_group::Event::OpenCodeReviewPane(arg) => {
                self.open_code_review_panel_from_arg(arg, pane_group.clone(), ctx);
            }
            pane_group::Event::ToggleCodeReviewPane(arg) => {
                self.toggle_right_panel(&pane_group, ctx);
                let active_conversation_id = arg.terminal_view.upgrade(ctx).and_then(|tv| {
                    BlocklistAIHistoryModel::as_ref(ctx).active_conversation_id(tv.id())
                });
                if let Some(conversation_id) = active_conversation_id {
                    BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, _| {
                        history_model.set_has_code_review_opened_to_true(conversation_id);
                    });
                }
            }
            pane_group::Event::RunWorkflow {
                workflow,
                workflow_source,
                workflow_selection_source,
                argument_override,
            } => {
                self.run_workflow_in_active_input(
                    workflow,
                    workflow_source.clone(),
                    *workflow_selection_source,
                    argument_override.clone(),
                    TerminalSessionFallbackBehavior::default(),
                    ctx,
                );
            }
            pane_group::Event::InvokeEnvVarCollection { .. } => {}
            pane_group::Event::MaximizePaneToggled => {
                ctx.notify();
            }
            pane_group::Event::FocusPaneGroup => {
                for (index, tab) in self.tabs.iter().enumerate() {
                    if tab.pane_group.id() == pane_group.id() {
                        self.activate_tab(index, ctx);
                        break;
                    }
                }
            }
            pane_group::Event::FocusPane { pane_to_focus } => {
                let Some(tab_index_to_focus) = self
                    .tabs
                    .iter()
                    .position(|tab| tab.pane_group.as_ref(ctx).has_pane_id(*pane_to_focus))
                else {
                    log::warn!("Could not find tab to focus pane");
                    return;
                };

                self.activate_tab(tab_index_to_focus, ctx);

                // TODO(CODE-266): This should focus the correct pane in the tab,
                // but for some reason application focus is not being moved to
                // the correct pane.
                if let Some(tab) = self.tabs.get_mut(tab_index_to_focus) {
                    tab.pane_group.update(ctx, |pane_group, ctx| {
                        if let Some(pane) = pane_group.pane_by_id(*pane_to_focus) {
                            pane.focus(ctx);
                        }
                    });
                }
            }
            pane_group::Event::FocusPaneInWorkspace { locator } => {
                // Focus an existing pane by its locator (used when avoiding duplicate file panes during undo close pane)
                self.focus_pane(*locator, ctx);
            }
            // If focused pane contains an object, then set selected state in WD to that object
            pane_group::Event::PaneFocused => {
                self.current_workspace_state.close_all_modals();

                // Re-evaluate which region is focused and update pane dimming accordingly.
                self.update_pane_dimming_for_current_focus_region(ctx);

                if self
                    .active_tab_pane_group()
                    .as_ref(ctx)
                    .active_session_view(ctx)
                    .is_some()
                {
                    #[cfg(feature = "local_fs")]
                    if self.active_tab_pane_group().as_ref(ctx).right_panel_open {
                        self.setup_code_review_panel(None, ctx);
                    }
                }

                let focused_terminal_view_id = {
                    let pane_group = self.active_tab_pane_group().as_ref(ctx);
                    pane_group
                        .terminal_view_from_pane_id(pane_group.focused_pane_id(ctx), ctx)
                        .map(|tv| tv.id())
                };
                self.notify_terminal_focus_change(focused_terminal_view_id, ctx);
            }
            pane_group::Event::RepoChanged => {
                self.refresh_working_directories_for_pane_group(&pane_group, ctx);
                #[cfg(feature = "local_fs")]
                if self.active_tab_pane_group().as_ref(ctx).right_panel_open {
                    self.setup_code_review_panel(None, ctx);
                }

                if FeatureFlag::DirectoryTabColors.is_enabled() {
                    if let Some(tab) = self
                        .tabs
                        .iter_mut()
                        .find(|t| t.pane_group.id() == pane_group.id())
                    {
                        Self::sync_codebase_tab_color(tab, ctx);
                    }
                }
            }
            #[cfg(feature = "local_fs")]
            pane_group::Event::RemoteRepoNavigated {
                host_id,
                indexed_path,
            } => {
                use warp_util::standardized_path::StandardizedPath;

                if let Ok(std_path) = StandardizedPath::try_new(indexed_path) {
                    let remote_id = RemoteRepositoryIdentifier::new(host_id.clone(), std_path);
                    let pane_group_id = pane_group.id();
                    if let Some(file_tree_view) = self
                        .working_directories_model
                        .as_ref(ctx)
                        .get_file_tree_view(pane_group_id)
                    {
                        file_tree_view.update(ctx, |view, ctx| {
                            view.set_remote_root_directories(std::slice::from_ref(&remote_id), ctx);
                        });
                    }
                }
            }
            #[cfg(not(feature = "local_fs"))]
            pane_group::Event::RemoteRepoNavigated { .. } => {}
            pane_group::Event::DroppedOnTabBar { origin, pane_id } => {
                if let Some(hovered_tab_index) = self.hovered_tab_index {
                    match hovered_tab_index {
                        TabBarHoverIndex::BeforeTab(workspace_tab_index) => {
                            // If an editor tab is dropped into a new position in the workspace tab group,
                            // create a new pane and insert it into the group.
                            let pane = if let ActionOrigin::EditorTab(editor_tab_index) = origin {
                                pane_group.update(ctx, |pane_group, ctx| {
                                    pane_group.remove_editor_tab_for_move(
                                        *pane_id,
                                        *editor_tab_index,
                                        ctx,
                                    )
                                })
                            } else {
                                // Otherwise, move the existing pane's contents into the workspace tab group.
                                pane_group.update(ctx, |pane_group, ctx| {
                                    pane_group.remove_pane_for_move(pane_id, ctx)
                                })
                            };

                            if let Some(pane) = pane {
                                self.add_tab_from_existing_pane(pane, workspace_tab_index, ctx);

                                // If the setting is enabled, preserve the color of the original pane's
                                // tab for the newly created tab.
                                if *TabSettings::as_ref(ctx).preserve_active_tab_color.value() {
                                    if let Some(source_tab) = self
                                        .tabs
                                        .iter()
                                        .find(|t| t.pane_group.id() == pane_group.id())
                                    {
                                        let selected = source_tab.selected_color;
                                        let default = source_tab.default_directory_color;
                                        self.tabs[self.active_tab_index].selected_color = selected;
                                        self.tabs[self.active_tab_index].default_directory_color =
                                            default;
                                    }
                                }
                            }
                        }
                        #[cfg_attr(target_family = "wasm", allow(unused_variables))]
                        TabBarHoverIndex::OverTab(workspace_tab_index) => {
                            #[cfg(not(target_family = "wasm"))]
                            {
                                let prefers_tabbed_editor_view = FeatureFlag::TabbedEditorView
                                    .is_enabled()
                                    && *EditorSettings::as_ref(ctx)
                                        .prefer_tabbed_editor_view
                                        .value();

                                let target_pane_group =
                                    self.get_pane_group_view(workspace_tab_index);
                                let target_code_view = target_pane_group.and_then(|pane_group| {
                                    pane_group
                                        .as_ref(ctx)
                                        .code_panes(ctx)
                                        .next()
                                        .map(|(_, view)| view)
                                });

                                if let ActionOrigin::EditorTab(editor_tab_index) = origin {
                                    // If the target pane group has an existing editor view, we want to open the dragged file as a tab in it.
                                    if let Some(target_code_view) = target_code_view {
                                        // If an editor tab is dropped onto its originating workspace tab, ensure that tab becomes the active editor within that workspace tab.
                                        if self.active_tab_index() == workspace_tab_index {
                                            target_code_view.update(ctx, |view, ctx| {
                                                view.set_active_tab_index(*editor_tab_index, ctx);
                                            });
                                            return;
                                        }

                                        let moved_file_path =
                                            pane_group.update(ctx, |pane_group, ctx| {
                                                pane_group.code_pane_by_id(*pane_id).and_then(
                                                    |pane| {
                                                        pane.file_view(ctx).update(
                                                            ctx,
                                                            |file_view, ctx| {
                                                                let moved_file_path = file_view
                                                                    .tab_at(*editor_tab_index)
                                                                    .and_then(|t| t.path());

                                                                file_view.remove_tab_for_move(
                                                                    *editor_tab_index,
                                                                    ctx,
                                                                );

                                                                moved_file_path
                                                            },
                                                        )
                                                    },
                                                )
                                            });

                                        // After removing the file from the origin's editor, we want to open it in the target's editor.
                                        if let Some(path) = moved_file_path {
                                            target_code_view.update(ctx, |view, ctx| {
                                                view.open_or_focus_existing(Some(path), None, ctx);
                                            });
                                        }
                                        return;
                                    } else if let Some(target_pane_group) = target_pane_group {
                                        // Otherwise, we want to open the dragged file in a new editor pane in the hovered tab's pane group.
                                        pane_group.update(ctx, |pane_group, ctx| {
                                            pane_group
                                                .code_pane_by_id(*pane_id)
                                                .and_then(|pane| {
                                                    pane.file_view(ctx).update(
                                                        ctx,
                                                        |file_view, ctx| {
                                                            file_view.remove_tab_for_move(
                                                                *editor_tab_index,
                                                                ctx,
                                                            )
                                                        },
                                                    )
                                                })
                                                .map(|new_pane| {
                                                    target_pane_group.update(
                                                        ctx,
                                                        |pane_group, ctx| {
                                                            pane_group.add_pane_with_direction(
                                                                Direction::Right,
                                                                new_pane,
                                                                true, /* focus_new_pane */
                                                                ctx,
                                                            );
                                                        },
                                                    );
                                                })
                                        });
                                    }

                                    self.set_active_tab_index(workspace_tab_index, ctx);
                                    return;
                                } else if prefers_tabbed_editor_view
                                    && pane_id.is_code_pane()
                                    && target_code_view.is_some()
                                    && self.active_tab_index() != workspace_tab_index
                                {
                                    // If a CodePane is dropped onto a tab with another CodePane and grouping is enabled, we want to merge them.
                                    if let Some(target_code_view) = target_code_view.as_ref() {
                                        pane_group.update(ctx, |pane_group, ctx| {
                                            if let Some(pane) = pane_group.code_pane_by_id(*pane_id)
                                            {
                                                pane.file_view(ctx).update(
                                                    ctx,
                                                    |source_file_view, ctx| {
                                                        target_code_view.update(
                                                            ctx,
                                                            |target_code_view, ctx| {
                                                                target_code_view.merge_tabs(
                                                                    source_file_view,
                                                                    ctx,
                                                                );
                                                            },
                                                        );
                                                    },
                                                );
                                                pane_group.remove_pane_for_move(pane_id, ctx);
                                            }
                                        });
                                    }
                                    self.set_active_tab_index(workspace_tab_index, ctx);
                                    return;
                                }
                            }

                            #[allow(unreachable_code)]
                            // Otherwise, we want to perform a left split on the root.
                            pane_group.update(ctx, |pane_group, ctx| {
                                pane_group.move_pane_with_root_split(
                                    *pane_id,
                                    Direction::Left,
                                    ctx,
                                );
                            });
                        }
                    }
                }
            }
            pane_group::Event::SwitchTabFocusAndMovePane {
                tab_idx,
                pane_id,
                hidden_pane_preview_direction,
            } => {
                #[cfg(feature = "local_fs")]
                let prefers_tabbed_editor_view = FeatureFlag::TabbedEditorView.is_enabled()
                    && *EditorSettings::as_ref(ctx)
                        .prefer_tabbed_editor_view
                        .value();

                #[cfg(not(feature = "local_fs"))]
                let prefers_tabbed_editor_view = false;

                // If a code pane is being dragged over a workspace tab with an existing code pane,
                // we don't allow it to be placed freely. Instead, it should be merged into the existing
                // code pane in the target tab (handled above in the OverTab case).
                let should_not_move_pane = prefers_tabbed_editor_view
                    && pane_id.is_code_pane()
                    && self
                        .get_pane_group_view(*tab_idx)
                        .map(|target_pane_group| {
                            target_pane_group.read(ctx, |pane_group, _| pane_group.has_code_panes())
                        })
                        .unwrap_or(false);

                // If we are already on this tab, then we should just make sure the pane is hidden.
                if self.active_tab_index() == *tab_idx || should_not_move_pane {
                    pane_group.update(ctx, |pane_group, ctx| {
                        pane_group.hide_pane_for_move(*pane_id, ctx)
                    });
                    return;
                };

                if let Some(pane) = pane_group.update(ctx, |pane_group, ctx| {
                    pane_group.remove_pane_for_move(pane_id, ctx)
                }) {
                    self.set_active_tab_index(*tab_idx, ctx);
                    self.active_tab_pane_group().update(ctx, |pane_group, ctx| {
                        pane_group.add_pane_as_hidden(pane, *hidden_pane_preview_direction, ctx)
                    });
                }
            }
            pane_group::Event::UpdateHoveredTabIndex { tab_hover_index } => {
                self.hovered_tab_index = Some(*tab_hover_index);
                ctx.notify();
            }
            pane_group::Event::ClearHoveredTabIndex => self.hovered_tab_index = None,
            pane_group::Event::OpenPalette {
                mode,
                source,
                query,
            } => self.open_palette_action(*mode, *source, query.as_deref(), ctx),
            pane_group::Event::FileUploadCommand {
                upload_id,
                command: _,
                remote_pane_id,
                local_pane_id,
            } => {
                self.file_upload_sessions
                    .local_to_remote_map
                    .insert(*local_pane_id, *remote_pane_id);
                self.file_upload_sessions
                    .local_to_upload_id_map
                    .insert(*local_pane_id, (*remote_pane_id, *upload_id));
                self.file_upload_sessions
                    .upload_id_to_local_map
                    .insert((*remote_pane_id, *upload_id), *local_pane_id);
            }
            pane_group::Event::FileUploadPasswordPending { local_pane_id } => {
                if let Some(remote_pane_id) = self
                    .file_upload_sessions
                    .local_to_remote_map
                    .get(local_pane_id)
                {
                    let (_, upload_id) = self
                        .file_upload_sessions
                        .local_to_upload_id_map
                        .get(local_pane_id)
                        .expect("Local session should map to upload ID");

                    let terminal_view =
                        self.active_tab_pane_group().read(ctx, |pane_group, ctx| {
                            pane_group
                                .terminal_view_from_pane_id(*remote_pane_id, ctx)
                                .expect("PaneGroup should find remote pane ID")
                        });
                    terminal_view.update(ctx, |terminal_view, ctx| {
                        terminal_view
                            .ssh_file_upload()
                            .update(ctx, |file_upload, ctx| {
                                file_upload.prompt_for_file_upload_password(*upload_id, ctx);
                            });
                    });
                }
            }
            pane_group::Event::FileUploadFinished {
                local_pane_id,
                exit_code,
            } => {
                if let Some(remote_pane_id) = self
                    .file_upload_sessions
                    .local_to_remote_map
                    .get(local_pane_id)
                {
                    let (_, upload_id) = self
                        .file_upload_sessions
                        .local_to_upload_id_map
                        .get(local_pane_id)
                        .expect("Local session should map to upload ID");

                    let terminal_view =
                        self.active_tab_pane_group().read(ctx, |pane_group, ctx| {
                            pane_group
                                .terminal_view_from_pane_id(*remote_pane_id, ctx)
                                .expect("PaneGroup should find remote pane ID")
                        });

                    terminal_view.update(ctx, |terminal_view, ctx| {
                        terminal_view
                            .ssh_file_upload()
                            .update(ctx, |file_upload, ctx| {
                                file_upload.file_upload_finished(*upload_id, exit_code, ctx);
                            });
                    });
                }
            }
            pane_group::Event::OpenFileUploadSession {
                remote_pane_id,
                upload_id,
            } => {
                // Find the local pane handling the upload.
                let local_pane_id = *self
                    .file_upload_sessions
                    .upload_id_to_local_map
                    .get(&(*remote_pane_id, *upload_id))
                    .expect("Upload ID should map to a local session");

                // Toggle the visibility of the local pane.
                let local_pane_open =
                    self.active_tab_pane_group().update(ctx, |pane_group, ctx| {
                        pane_group.toggle_pane_visibility_for_job(local_pane_id.into(), ctx)
                    });

                // Inform the remote pane of the state of the local pane.
                let terminal_view = self.active_tab_pane_group().read(ctx, |pane_group, ctx| {
                    pane_group
                        .terminal_view_from_pane_id(*remote_pane_id, ctx)
                        .expect("PaneGroup should find remote pane ID")
                });
                terminal_view.update(ctx, |terminal_view, ctx| {
                    terminal_view
                        .ssh_file_upload()
                        .update(ctx, |file_upload, ctx| {
                            file_upload.local_session_state_changed(
                                *upload_id,
                                local_pane_open,
                                ctx,
                            );
                        });
                });
            }
            pane_group::Event::TerminateFileUploadSession {
                remote_pane_id,
                upload_id,
            } => {
                // Find the local pane handling the upload.
                let local_pane_id = *self
                    .file_upload_sessions
                    .upload_id_to_local_map
                    .get(&(*remote_pane_id, *upload_id))
                    .expect("Upload ID should map to a local session");

                // Close the local pane.
                self.active_tab_pane_group().update(ctx, |pane_group, ctx| {
                    pane_group.close_pane(local_pane_id.into(), ctx);
                });
            }
            pane_group::Event::ShowToast {
                message,
                flavor,
                pane_id,
            } => {
                let pane_group_id = pane_group.id();
                self.toast_stack.update(ctx, |toast_stack, ctx| {
                    let mut toast = DismissibleToast::new(message.clone(), *flavor);
                    if let Some(pane_id) = pane_id {
                        let locator = PaneViewLocator {
                            pane_group_id,
                            pane_id: *pane_id,
                        };
                        toast = toast.with_on_body_click(move |ctx| {
                            ctx.dispatch_typed_action(&WorkspaceAction::FocusPane(locator));
                        });
                    }
                    toast_stack.add_ephemeral_toast(toast, ctx);
                });
            }
            pane_group::Event::OpenThemeChooser => {
                self.show_theme_chooser_for_custom_theme(ctx);
            }
            pane_group::Event::OpenConversationHistory => {
                self.open_palette_action(
                    PaletteMode::Conversations,
                    PaletteSource::ConversationManager,
                    None,
                    ctx,
                );
            }
            pane_group::Event::OpenAddPromptPane { .. } => {}
            pane_group::Event::OpenFilesPalette { source } => {
                self.open_palette_action(PaletteMode::Files, *source, None, ctx);
            }
            pane_group::Event::ToggleLeftPanel {
                target_view: _,
                force_open,
            } => {
                let is_target_active = self
                    .left_panel_view
                    .read(ctx, |left_panel, _| left_panel.is_file_tree_active());

                if self.active_tab_pane_group().as_ref(ctx).left_panel_open && is_target_active {
                    // No-op if we are forcing the target to open when it is already active.
                    if !*force_open {
                        self.toggle_left_panel(ctx);
                    }
                } else {
                    if !self.active_tab_pane_group().as_ref(ctx).left_panel_open {
                        self.toggle_left_panel(ctx);
                    }
                    self.left_panel_view.update(ctx, |left_panel, ctx| {
                        left_panel.handle_action_with_force_open(
                            &LeftPanelAction::ProjectExplorer,
                            *force_open,
                            ctx,
                        );
                    });
                }
            }
            #[cfg(feature = "local_fs")]
            pane_group::Event::OpenFileWithTarget {
                path,
                target,
                line_col,
            } => {
                self.open_file_with_target(
                    path.clone(),
                    target.clone(),
                    *line_col,
                    CodeSource::Link {
                        path: path.clone(),
                        range_start: None,
                        range_end: None,
                    },
                    ctx,
                );
            }
            #[cfg(feature = "local_fs")]
            pane_group::Event::FileRenamed { old_path, new_path } => {
                self.rename_tabs_with_file_path(old_path, new_path, ctx);
            }
            #[cfg(feature = "local_fs")]
            pane_group::Event::FileDeleted { path } => {
                self.close_tabs_with_file_path(path, ctx);
            }
            pane_group::Event::OpenAgentProfileEditor { profile_id } => {
                self.open_execution_profile_editor_pane(None, *profile_id, ctx);
            }
            pane_group::Event::OpenLspLogs { log_path } => {
                self.open_lsp_logs(log_path, ctx);
            }
            pane_group::Event::LeftPanelToggled { is_open } => {
                // Only handle visibility changes from the active pane group.
                if pane_group.id() == self.active_tab_pane_group().id() {
                    self.left_panel_open = *is_open;
                    self.left_panel_view.update(ctx, |left_panel, ctx| {
                        left_panel.on_left_panel_visibility_changed(*is_open, ctx);
                    });
                }
            }
            pane_group::Event::InsertCodeReviewComments {
                repo_path,
                comments,
                diff_mode,
                open_code_review,
            } => {
                if let Some(open_code_review) = open_code_review {
                    self.open_code_review_panel_from_arg(open_code_review, pane_group.clone(), ctx);
                }

                self.working_directories_model
                    .update(ctx, |working_directories, ctx| {
                        working_directories.insert_code_review_comments(
                            pane_group.id(),
                            repo_path.as_path(),
                            comments,
                            diff_mode,
                            ctx,
                        )
                    });
            }
            pane_group::Event::OpenCodeReviewPaneAndScrollToComment {
                open_code_review,
                comment,
                diff_mode,
            } => {
                self.open_code_review_panel_from_arg(open_code_review, pane_group.clone(), ctx);

                let Some(repo_path) = &open_code_review.repo_path else {
                    return;
                };
                self.working_directories_model
                    .update(ctx, |working_directories, ctx| {
                        working_directories.upsert_flattened_code_review_comments(
                            repo_path,
                            vec![comment.clone()],
                            ctx,
                        );
                    });

                let Some(code_review_view) = self
                    .working_directories_model
                    .as_ref(ctx)
                    .get_code_review_view(pane_group.id(), repo_path)
                else {
                    return;
                };
                code_review_view.update(ctx, |code_review, ctx| {
                    code_review.navigate_to_imported_comment(comment.id, diff_mode.clone(), ctx);
                });
            }
            pane_group::Event::ImportAllCodeReviewComments {
                comments,
                diff_mode,
                open_code_review,
            } => {
                self.open_code_review_panel_from_arg(open_code_review, pane_group.clone(), ctx);

                let Some(repo_path) = &open_code_review.repo_path else {
                    return;
                };
                self.working_directories_model
                    .update(ctx, |working_directories, ctx| {
                        working_directories.upsert_flattened_code_review_comments(
                            repo_path,
                            comments.clone(),
                            ctx,
                        );
                    });

                if let Some(code_review_view) = self
                    .working_directories_model
                    .as_ref(ctx)
                    .get_code_review_view(pane_group.id(), repo_path.as_path())
                {
                    code_review_view.update(ctx, |code_review_view, ctx| {
                        code_review_view.set_diff_base(diff_mode.clone(), ctx);
                        code_review_view.expand_comment_list(ctx);
                    });
                }
            }
        }
    }

    fn handle_theme_chooser_event(
        &mut self,
        event: &ThemeChooserEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            ThemeChooserEvent::Click => self.focus_theme_chooser(ctx),
            ThemeChooserEvent::Close(mode) => {
                self.save_theme_chooser(mode, ctx);
                self.restore_previous_workspace_state(ctx);
            }
            ThemeChooserEvent::OpenThemeCreatorModal => {
                self.open_theme_creator_modal(ctx);
            }
            ThemeChooserEvent::OpenThemeDeletionModal(theme_kind) => {
                self.open_theme_deletion_modal(theme_kind.clone(), ctx);
            }
        };
    }

    fn handle_resource_center_event(
        &mut self,
        event: &ResourceCenterEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            ResourceCenterEvent::Close => {
                self.current_workspace_state.is_resource_center_open = false;
                ctx.notify();
            }
            ResourceCenterEvent::Escape => {
                // Calls terminal view focus to determine where focus should be
                if let Some(pane_group_handle) = self.get_pane_group_view(self.active_tab_index) {
                    pane_group_handle.update(ctx, |pane_group, ctx| {
                        if let Some(terminal_view_handle) = pane_group.active_session_view(ctx) {
                            terminal_view_handle.update(ctx, |terminal, ctx| {
                                terminal.redetermine_global_focus(ctx);
                            });
                        }
                    });
                }
            }
        };
    }

    fn show_command_search(
        &mut self,
        query_filter: Option<search::QueryFilter>,
        init_content: &InitContent,
        ctx: &mut ViewContext<Self>,
    ) {
        // Close all overlays including chip menus before opening command search
        self.close_all_overlays(ctx);

        if let Some(session_id) = self.active_session_id(ctx) {
            let active_input_handle = self.get_active_input_view_handle(ctx);

            let initial_query = match init_content {
                InitContent::FromInputBuffer => {
                    if let Some(input_handle) = &active_input_handle {
                        input_handle.read(ctx, |input, ctx| input.buffer_text(ctx))
                    } else {
                        "".to_owned()
                    }
                }
                InitContent::Custom(query) => query.to_owned(),
            };

            let session_context = active_input_handle.as_ref().and_then(|input_handle| {
                input_handle.read(ctx, |input, ctx| input.completion_session_context(ctx))
            });

            let menu_positioning = active_input_handle
                .as_ref()
                .map_or_else(MenuPositioning::default, |input_handle| {
                    input_handle.read(ctx, |input, ctx| input.menu_positioning(ctx))
                });

            // Make sure we close any already-open input suggestions panel.
            if let Some(input_handle) = &active_input_handle {
                input_handle.update(ctx, |input, ctx| {
                    input.close_input_suggestions(false, ctx);
                });
            };

            self.current_workspace_state.is_command_search_open = true;
            self.command_search_view.update(ctx, |view, ctx| {
                view.reset_state(
                    session_id,
                    session_context,
                    initial_query,
                    query_filter,
                    menu_positioning,
                    ctx,
                );
            });

            let tip = match query_filter {
                Some(search::QueryFilter::History) => Tip::Action(TipAction::HistorySearch),
                _ => Tip::Action(TipAction::CommandSearch),
            };

            self.tips_completed.update(ctx, |tips_completed, ctx| {
                mark_feature_used_and_write_to_user_defaults(tip, tips_completed, ctx);
                ctx.notify();
            });

            ctx.notify();
            ctx.focus(&self.command_search_view);
        } else {
            log::error!("Command search keybinding triggered but no session is active!");
        }
    }

    fn get_active_input_view_handle(&self, app: &AppContext) -> Option<ViewHandle<Input>> {
        app.view(self.active_tab_pane_group())
            .active_session_view(app)
            .map(|terminal_view_handle| app.view(&terminal_view_handle).input().clone())
    }

    fn get_active_session_terminal_model(
        &self,
        app: &AppContext,
    ) -> Option<Arc<FairMutex<TerminalModel>>> {
        self.active_tab_pane_group()
            .as_ref(app)
            .active_session_terminal_model(app)
    }

    /// Replace the active terminal input's buffer with `contents`. Adds to the
    /// undo stack.
    pub fn set_active_terminal_input_contents_and_focus_app(
        &mut self,
        contents: &str,
        ctx: &mut ViewContext<Self>,
    ) {
        let window_id = ctx.window_id();

        if let Some(active_input_view_handle) = self.get_active_input_view_handle(ctx) {
            active_input_view_handle.update(ctx, |input_view, input_ctx| {
                input_view.replace_buffer_content(contents, input_ctx);
            });

            ctx.windows().show_window_and_focus_app(window_id);

            ctx.notify();
        } else {
            log::error!("workspace::view::fill_input(): no active input view handle to fill");
        }
    }

    /// Insert the given command that should open a subshell. And set a flag that we should
    /// automatically bootstrap AKA "warpify" that subshell if we support it. No-op if there is
    /// no active terminal session.
    pub fn insert_subshell_command_and_bootstrap_if_supported(
        &mut self,
        command: &str,
        shell: Option<ShellType>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.active_tab_pane_group()
            .update(ctx, |pane_group_view, pane_group_ctx| {
                pane_group_view
                    .active_session_view(pane_group_ctx)
                    .map(|terminal_view_handle| {
                        terminal_view_handle.update(
                            pane_group_ctx,
                            |terminal_view, terminal_view_ctx| {
                                terminal_view.insert_subshell_command_and_bootstrap_if_supported(
                                    command,
                                    shell,
                                    terminal_view_ctx,
                                );
                            },
                        )
                    })
            });
    }

    /// Update the active session model state.
    fn update_active_session(&mut self, ctx: &mut ViewContext<Self>) {
        let pane_group_handle = self.active_tab_pane_group();
        let file_tree_and_global_search_are_enabled = {
            #[cfg(feature = "local_fs")]
            {
                Self::should_enable_file_tree_and_global_search_for_pane_group(
                    self.active_tab_pane_group().as_ref(ctx),
                )
            }

            #[cfg(not(feature = "local_fs"))]
            {
                false
            }
        };

        // Update working directories for the current pane group
        let pane_group_handle = pane_group_handle.clone();
        self.refresh_working_directories_for_pane_group(&pane_group_handle, ctx);

        if let Some(terminal_handle) = pane_group_handle.as_ref(ctx).active_session_view(ctx) {
            #[cfg_attr(not(feature = "local_fs"), allow(unused_variables))]
            let (session, path_if_local, is_local, is_wsl_session) =
                terminal_handle.read(ctx, |terminal, ctx| {
                    let active_session_id = terminal.active_block_session_id();
                    let session = active_session_id
                        .and_then(|id| terminal.sessions_model().as_ref(ctx).get(id));
                    let path_if_local = terminal.active_session_path_if_local(ctx);
                    let is_local = terminal.active_session_is_local(ctx);
                    let is_wsl_session = session.as_ref().map(|s| s.is_wsl()).unwrap_or(false);
                    (session, path_if_local, is_local, is_wsl_session)
                });

            let window_id = ctx.window_id();
            let working_directory_clone = path_if_local.clone();
            let path_if_local_clone = path_if_local.clone();
            ActiveSession::handle(ctx).update(ctx, |active_session, ctx| {
                active_session.set_session_state(
                    window_id,
                    session,
                    path_if_local_clone.clone(),
                    Some(terminal_handle.id()),
                    ctx,
                );
            });

            CodebaseIndexManager::handle(ctx).update(ctx, |manager, _ctx| {
                if let Some(working_directory) = working_directory_clone {
                    manager.handle_active_session_changed(working_directory.as_path());
                }
            });

            let is_remote = matches!(is_local, Some(false));
            let is_unsupported_session = is_wsl_session;

            let enablement = CodingPanelEnablementState::from_session_env(
                file_tree_and_global_search_are_enabled,
                is_remote,
                is_unsupported_session,
            );

            self.left_panel_view.update(ctx, |left_panel, ctx| {
                left_panel.update_coding_panel_enablement(enablement, ctx);
            });

            #[cfg(feature = "local_fs")]
            {
                self.right_panel_view.update(ctx, |right_panel, ctx| {
                    right_panel.update_session_env(is_remote, is_wsl_session, ctx);
                });

                if self.active_tab_pane_group().as_ref(ctx).right_panel_open {
                    self.setup_code_review_panel(None, ctx);
                }
            }
        } else {
            let enablement = CodingPanelEnablementState::from_session_env(
                file_tree_and_global_search_are_enabled,
                false,
                false,
            );

            self.left_panel_view.update(ctx, |left_panel, ctx| {
                left_panel.update_coding_panel_enablement(enablement, ctx);
            });

            #[cfg(feature = "local_fs")]
            {
                self.right_panel_view.update(ctx, |right_panel, ctx| {
                    right_panel.update_session_env(false, false, ctx);
                });
            }
        }
    }

    fn attach_plan_as_context(&mut self, id: AIDocumentId, ctx: &mut ViewContext<Self>) {
        let Some(view) = self.active_session_view(ctx) else {
            let window_id = ctx.window_id();
            WorkspaceToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                let toast = DismissibleToast::default(
                    "No terminal pane open. Open a new pane to attach as context.".to_owned(),
                );
                toast_stack.add_ephemeral_toast(toast, window_id, ctx);
            });
            return;
        };

        // Check if the plan's conversation is already selected in the target terminal before
        // attaching as context. This is to stop users from reattaching plans to conversations that already
        // have them in context.
        if let Some(conversation_id) =
            AIDocumentModel::as_ref(ctx).get_conversation_id_for_document_id(&id)
        {
            if view
                .as_ref(ctx)
                .is_conversation_selected(&conversation_id, ctx)
            {
                let window_id = ctx.window_id();
                WorkspaceToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                    let toast =
                        DismissibleToast::default("This plan is already in context.".to_owned());
                    toast_stack.add_ephemeral_toast(toast, window_id, ctx);
                });
                return;
            }
        }

        view.update(ctx, |session, ctx| {
            session.attach_plan_as_context(id, ctx);
        });
    }

    /// Focus and return the active terminal input. If there is no active terminal input (either
    /// because a command is running or because there are no terminal panes), this may create a new
    /// terminal pane according to the [`UnavailableTerminalBehavior`].
    fn focus_terminal_input(
        &mut self,
        fallback_behavior: TerminalSessionFallbackBehavior,
        ctx: &mut ViewContext<Self>,
    ) -> Option<ViewHandle<TerminalView>> {
        let active_pane_group = self.active_tab_pane_group();

        // If there's an active terminal session and it's not busy, return it.
        // If there is no terminal session open, add a terminal pane to the right and return the new terminal view handle.
        let terminal_view_handle = active_pane_group
            .as_ref(ctx)
            .active_session_view(ctx)
            .unwrap_or_else(|| {
                let active_pane_group = self.active_tab_pane_group();
                active_pane_group.update(ctx, |pane_group, ctx| {
                    pane_group.add_terminal_pane(Direction::Right, None /*chosen_shell*/, ctx);
                });
                active_pane_group
                    .as_ref(ctx)
                    .active_session_view(ctx)
                    .unwrap()
            });

        let is_env_var_block = terminal_view_handle.read(ctx, |terminal_view, ctx| {
            terminal_view.has_active_env_var_block(ctx)
        });

        if self.is_input_box_visible(ctx) {
            active_pane_group.update(ctx, |pane_group, ctx| pane_group.focus_active_session(ctx));
            return Some(terminal_view_handle);
        } else if is_env_var_block {
            terminal_view_handle.update(ctx, |terminal_view, ctx| {
                terminal_view.cancel_env_var_block(ctx);
            });
            active_pane_group.update(ctx, |pane_group, ctx| pane_group.focus_active_session(ctx));
            return Some(terminal_view_handle);
        } else if fallback_behavior != TerminalSessionFallbackBehavior::OpenIfNeeded {
            // The active terminal exists but is busy, and the fallback behavior is
            // RequireExisting or OpenIfNone. In those cases, show a toast and no-op.
            self.toast_stack.update(ctx, |toast_stack, ctx| {
                let toast = DismissibleToast::error(
                    "A command in this session is still running.".to_string(),
                );
                toast_stack.add_ephemeral_toast(toast, ctx);
            });
            return None;
        }

        // There's no available session and we were asked not to create one.
        if fallback_behavior == TerminalSessionFallbackBehavior::RequireExisting {
            return None;
        }

        // Either:
        // * There's no active session
        // * The active session is busy but the fallback behavior is OpenIfNeeded
        // In this case, open a new terminal pane to the right.

        if !ContextFlag::CreateNewSession.is_enabled() {
            self.toast_stack.update(ctx, |toast_stack, ctx| {
                let toast =
                    DismissibleToast::error("Cannot open a new terminal session".to_string());
                toast_stack.add_ephemeral_toast(toast, ctx);
            });
            return None;
        }

        active_pane_group.as_ref(ctx).active_session_view(ctx)
    }

    /// Opens the LSP log file in a new terminal pane using `tail -f`.
    fn open_lsp_logs(&mut self, log_path: &PathBuf, ctx: &mut ViewContext<Self>) {
        use crate::workflows::local_workflows::tail_command_for_shell;

        let active_pane_group = self.active_tab_pane_group();

        // Add a terminal pane to the right
        active_pane_group.update(ctx, |pane_group, ctx| {
            pane_group.add_terminal_pane(PaneGroupDirection::Right, None, ctx);
        });

        let Some(terminal_view_handle) = active_pane_group.as_ref(ctx).active_session_view(ctx)
        else {
            log::error!("Could not get terminal view handle when attempting to open LSP logs.");
            return;
        };

        terminal_view_handle.update(ctx, |terminal, ctx| {
            let shell_family = terminal.shell_family(ctx);
            let tail_command = tail_command_for_shell(shell_family, log_path);
            terminal.set_pending_command(&tail_command, ctx);
        });
    }

    fn run_tab_config_skill(&mut self, path: &Path, ctx: &mut ViewContext<Self>) {
        if !AISettings::as_ref(ctx).is_any_ai_enabled(ctx) {
            return;
        }

        let Some(terminal_view_handle) =
            self.focus_terminal_input(TerminalSessionFallbackBehavior::OpenIfNeeded, ctx)
        else {
            return;
        };

        let prefix = CLIAgentSessionsModel::as_ref(ctx)
            .session(terminal_view_handle.id())
            .map(|session| session.agent.skill_command_prefix())
            .unwrap_or("/");
        let prompt = format!("{prefix}update-tab-config Update {} to...", path.display());

        terminal_view_handle.update(ctx, |terminal_view, ctx| {
            terminal_view.input().update(ctx, |input, ctx| {
                input.clear_buffer_and_reset_undo_stack(ctx);
                input.set_input_mode_agent(true, ctx);
                input.ensure_agent_mode_for_ai_features(true, ctx);
                input.replace_buffer_content(&prompt, ctx);
                input.focus_input_box(ctx);
            });
        });
    }

    /// Runs a workflow in whichever terminal input is currently active.
    /// No-ops if the active session is long-running.
    fn run_workflow_in_active_input(
        &mut self,
        workflow: &WorkflowType,
        workflow_source: WorkflowSource,
        workflow_selection_source: WorkflowSelectionSource,
        argument_override: Option<HashMap<String, String>>,
        fallback_behavior: TerminalSessionFallbackBehavior,
        ctx: &mut ViewContext<Self>,
    ) {
        if let Some(terminal_view_handle) = self.focus_terminal_input(fallback_behavior, ctx) {
            let terminal_input =
                terminal_view_handle.read(ctx, |terminal_view, _| terminal_view.input().clone());
            terminal_input.update(ctx, |input, ctx| {
                input.show_workflows_info_box_on_workflow_selection(
                    workflow.clone(),
                    workflow_source,
                    workflow_selection_source,
                    argument_override,
                    ctx,
                );
                ctx.notify();
            });
        }
    }

    /// Inserts given command into active Input Editor, optionally replacing the current buffer. No-ops if
    /// there is no active terminal pane open, with an input box active.
    fn insert_in_input(
        &mut self,
        content: &str,
        replace_buffer: bool,
        should_submit: bool,
        ensure_agent_mode: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        let active_input_handle = self.get_active_input_view_handle(ctx);

        if let Some(active_input_handle) = active_input_handle {
            active_input_handle.update(ctx, |input, ctx| {
                if replace_buffer {
                    input.replace_buffer_content(content, ctx);
                } else {
                    input.append_to_buffer(content, ctx);
                }

                if ensure_agent_mode {
                    input.ensure_agent_mode_for_ai_features(true, ctx);
                }

                if should_submit {
                    input.input_enter(ctx);
                }
                ctx.notify();
            });
        }
    }

    fn handle_command_search_event(
        &mut self,
        event: &CommandSearchEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        use CommandSearchEvent::*;
        let Some(active_input_handle) = self.get_active_input_view_handle(ctx) else {
            return;
        };
        match event {
            Close {
                query: query_when_closed,
                filter: filter_when_closed,
            } => {
                self.current_workspace_state.is_command_search_open = false;

                active_input_handle.update(ctx, |input, ctx| {
                    input.handle_command_search_closed(query_when_closed, filter_when_closed, ctx);
                    ctx.notify();
                });

                ctx.notify();
            }
            Blur => {
                self.current_workspace_state.is_command_search_open = false;
                ctx.notify();
            }
            ItemSelected { query: _, payload } => {
                use CommandSearchItemAction::*;
                match payload.as_ref() {
                    AcceptHistory(AcceptedHistoryItem {
                        command,
                        linked_workflow_data,
                    }) => {
                        // Switch to shell input mode so the history command is
                        // treated as a shell command, not an agent prompt.
                        active_input_handle.update(ctx, |input, ctx| {
                            input.set_input_mode_terminal(false, ctx);
                            input.replace_buffer_content(command.as_str(), ctx);
                            input.focus_input_box(ctx);
                        });

                        if let Some(linked_workflow_data) = linked_workflow_data {
                            active_input_handle.update(ctx, |input, ctx| {
                                if let Some((workflow_type, workflow_source)) =
                                    linked_workflow_data.linked_workflow(ctx)
                                {
                                    input.show_workflow_info_box_for_history_command(
                                        command.as_str(),
                                        workflow_type,
                                        workflow_source,
                                        WorkflowSelectionSource::UniversalSearch,
                                        ctx,
                                    );
                                }
                                ctx.notify();
                            });
                        }
                    }
                    ExecuteHistory(command) => {
                        active_input_handle.update(ctx, |input, ctx| {
                            input.try_execute_command(command.as_str(), ctx);
                            ctx.notify();
                        });
                    }
                    AcceptWorkflow(accepted) => {
                        let (workflow, workflow_source) = match accepted {
                            AcceptedWorkflow::Local {
                                workflow, source, ..
                            } => ((**workflow).clone(), source.clone()),
                        };
                        active_input_handle.update(ctx, |input, ctx| {
                            input.show_workflows_info_box_on_workflow_selection(
                                workflow,
                                workflow_source,
                                WorkflowSelectionSource::UniversalSearch,
                                None,
                                ctx,
                            );
                            ctx.notify();
                        });
                    }
                    AcceptAIQuery(ai_query) => {
                        let active_terminal_view = self.active_session_view(ctx).expect("There must be an active terminal view if the user selected a command search result");

                        active_terminal_view.update(ctx, |terminal_view, ctx| {
                            terminal_view.set_ai_input_mode_with_query(Some(ai_query), ctx);
                        });
                    }
                    RunAIQuery(ai_query) => {
                        let active_terminal_view = self.active_session_view(ctx).expect("There must be an active terminal view if the user selected a command search result");

                        active_terminal_view.update(ctx, |terminal_view, ctx| {
                            terminal_view.set_ai_input_mode_with_query(Some(ai_query), ctx);
                        });

                        active_input_handle.update(ctx, |input, ctx| input.input_enter(ctx));
                    }
                }
            }
            Resize => {
                // A resize of universal search should write the app snapshot to sqlite.
                ctx.dispatch_global_action("workspace:save_app", ());
            }
        }
    }

    fn handle_window_settings_changed_event(
        &mut self,
        event: &WindowSettingsChangedEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            WindowSettingsChangedEvent::BackgroundOpacity { .. }
            | WindowSettingsChangedEvent::TerminalBackgroundImagePath { .. } => {
                ctx.notify();
            }
            WindowSettingsChangedEvent::LeftPanelVisibilityAcrossTabs { .. } => {
                if self.left_panel_visibility_across_tabs_enabled(ctx) {
                    self.left_panel_open = self
                        .active_tab_pane_group()
                        .read(ctx, |pane_group, _| pane_group.left_panel_open);
                }
            }
            WindowSettingsChangedEvent::ZoomLevel { .. } => {
                self.update_titlebar_height(ctx);
            }
            _ => {}
        }
    }

    fn restore_previous_workspace_state(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(previous_state) = self.previous_workspace_state.take() {
            self.current_workspace_state = previous_state;

            // Assumption: at most one of the states will be active.
            // If none are, then we focus the terminal view instead.
            if self.current_workspace_state.is_palette_open {
                self.open_command_palette(ctx);
            } else if self.current_workspace_state.is_theme_chooser_open {
                self.focus_theme_chooser(ctx);
            } else if self.current_workspace_state.is_resource_center_open {
                ctx.focus(&self.resource_center_view);
            } else if self
                .current_workspace_state
                .is_rewind_confirmation_dialog_open
            {
                ctx.focus(&self.rewind_confirmation_dialog);
            } else if self.current_workspace_state.is_native_quit_modal_open {
                ctx.focus(&self.native_modal);
            } else {
                ctx.focus_self();
            }

            self.cancel_tab_rename(ctx);
        }
    }

    fn should_keep_theme(system_theme: SystemTheme, ctx: &mut ViewContext<Self>) -> bool {
        if system_theme == ctx.system_theme() {
            let respect_system_theme = respect_system_theme(ThemeSettings::as_ref(ctx));
            if let RespectSystemTheme::On { .. } = respect_system_theme {
                return true;
            }
        }
        false
    }

    fn save_theme_chooser(&mut self, mode: &ThemeChooserMode, ctx: &mut ViewContext<Self>) {
        let keep_theme = match mode {
            ThemeChooserMode::SystemAgnostic => true,
            ThemeChooserMode::SystemLight => Workspace::should_keep_theme(SystemTheme::Light, ctx),
            ThemeChooserMode::SystemDark => Workspace::should_keep_theme(SystemTheme::Dark, ctx),
        };
        if keep_theme {
            self.keep_theme(ctx);
        } else {
            self.revert_theme(ctx);
        }
    }

    fn revert_theme(&mut self, ctx: &mut ViewContext<Self>) {
        AppearanceManager::handle(ctx).update(ctx, |appearance_manager, ctx| {
            appearance_manager.clear_transient_theme(ctx);
        });
        self.current_workspace_state.is_theme_chooser_open = false;
        self.previous_theme = None;
        ctx.notify();
    }

    fn keep_theme(&mut self, ctx: &mut ViewContext<Self>) {
        self.current_workspace_state.is_theme_chooser_open = false;
        self.previous_theme = None;
        ctx.notify();
    }

    fn active_session_ps1_grid_info(&self, app: &AppContext) -> Option<(BlockGrid, SizeInfo)> {
        self.get_active_session_terminal_model(app)
            .and_then(|model| {
                let lock = model.lock();
                lock.prompt_grid()
                    .cloned()
                    .zip(Some(*lock.block_list().size()))
            })
            .or_else(|| {
                (0..self.tabs.len()).find_map(|i| {
                    self.get_pane_group_view(i)?
                        .as_ref(app)
                        .active_session_terminal_model(app)
                        .and_then(|model| {
                            let lock = model.lock();
                            lock.prompt_grid()
                                .cloned()
                                .zip(Some(*lock.block_list().size()))
                        })
                })
            })
    }

    pub fn show_rewind_confirmation_dialog(
        &mut self,
        source: RewindDialogSource,
        ctx: &mut ViewContext<Self>,
    ) {
        self.rewind_confirmation_dialog.update(ctx, |view, _| {
            view.set_rewind_source(source);
        });
        self.current_workspace_state
            .is_rewind_confirmation_dialog_open = true;
        ctx.focus(&self.rewind_confirmation_dialog);
        ctx.notify();
    }

    pub fn show_delete_conversation_confirmation_dialog(
        &mut self,
        source: DeleteConversationDialogSource,
        ctx: &mut ViewContext<Self>,
    ) {
        self.delete_conversation_confirmation_dialog
            .update(ctx, |view, _| {
                view.set_source(source);
            });
        self.current_workspace_state
            .is_delete_conversation_confirmation_dialog_open = true;
        ctx.focus(&self.delete_conversation_confirmation_dialog);
        ctx.notify();
    }

    pub fn show_native_modal(
        &mut self,
        dialog: AlertDialogWithCallbacks<AppModalCallback>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.native_modal.update(ctx, |view, ctx| {
            view.set_alert_dialog(dialog);
            ctx.notify();
        });
        self.current_workspace_state.is_native_quit_modal_open = true;
        ctx.focus(&self.native_modal);
        ctx.notify();
    }

    fn handle_native_modal_event(&mut self, event: &NativeModalEvent, ctx: &mut ViewContext<Self>) {
        match event {
            NativeModalEvent::Close => {
                self.current_workspace_state.is_native_quit_modal_open = false;
                ctx.notify();
            }
        }
    }

    /// Mock pressing a button on the native quit modal. This function has an unusual signature so
    /// that the workspace view is not borrowed while the button press is handled.
    #[cfg(any(test, feature = "integration_tests"))]
    pub fn press_native_modal_button(
        handle: &ViewHandle<Self>,
        button_index: usize,
        app: &mut AppContext,
    ) {
        use super::native_modal::NativeModalAction;
        let modal_handle = handle.as_ref(app).native_modal.clone();
        modal_handle.update(app, |modal, ctx| {
            modal.handle_action(&NativeModalAction::TriggerButtonCallback(button_index), ctx);
        });
    }

    #[cfg(any(test, feature = "integration_tests"))]
    pub fn is_native_quit_modal_open(&self, ctx: &AppContext) -> bool {
        self.current_workspace_state.is_native_quit_modal_open
            && self.native_modal.as_ref(ctx).has_alert_dialog()
    }

    fn show_settings(&mut self, ctx: &mut ViewContext<Self>) {
        self.show_settings_with_section(None, ctx);
    }

    fn show_settings_with_section(
        &mut self,
        section: Option<SettingsSection>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.close_all_overlays(ctx);
        self.open_settings_pane(section, None, ctx);
    }

    fn show_settings_with_search(
        &mut self,
        search_query: &str,
        section: Option<SettingsSection>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.close_all_overlays(ctx);
        self.open_settings_pane(section, Some(search_query), ctx);
    }

    /// Opens the MCP servers settings page.
    pub fn open_mcp_servers_page(
        &mut self,
        page: MCPServersSettingsPage,
        ctx: &mut ViewContext<Self>,
    ) {
        self.show_settings_with_section(Some(SettingsSection::MCPServers), ctx);

        self.settings_pane.update(ctx, |view, ctx| {
            view.open_mcp_servers_page(page, ctx);
        });
    }

    /// Shows the theme chooser so the user can change the active theme.
    pub fn show_theme_chooser_for_active_theme(&mut self, ctx: &mut ViewContext<Self>) {
        self.show_theme_chooser(Some(ThemeChooserMode::for_active_theme(ctx)), ctx)
    }

    pub fn show_theme_chooser_for_custom_theme(&mut self, ctx: &mut ViewContext<Self>) {
        self.show_theme_chooser(None, ctx)
    }

    /// Shows the theme chooser so the user can change a specific theme.
    pub fn show_theme_chooser(
        &mut self,
        theme_chooser_mode: Option<ThemeChooserMode>,
        ctx: &mut ViewContext<Self>,
    ) {
        let current_theme = active_theme_kind(ThemeSettings::as_ref(ctx), ctx);

        self.close_tab_bar_overflow_menu(ctx);

        self.current_workspace_state.close_all_left_panels();

        // When showing the theme chooser, let's close the command palette
        // in case it was used to open the theme chooser.
        self.current_workspace_state.is_palette_open = false;
        self.current_workspace_state.is_ctrl_tab_palette_open = false;
        self.previous_workspace_state = Some(self.current_workspace_state);
        self.current_workspace_state.is_theme_chooser_open = true;

        self.previous_theme = Some(current_theme);

        self.theme_chooser_view.update(ctx, |view, ctx| {
            view.record_open_theme();
            if let Some(theme_chooser_mode) = theme_chooser_mode {
                view.select_theme(theme_chooser_mode.into_theme_kind(ctx), ctx);
                view.set_mode(theme_chooser_mode);
            } else {
                view.reload_and_set_latest_theme(ctx);
            }
        });

        self.focus_theme_chooser(ctx);
    }

    pub fn show_keyboard_settings(
        &mut self,
        keybinding_name: Option<&str>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.show_settings_with_section(Some(SettingsSection::Keybindings), ctx);
        if let Some(keybinding_name) = keybinding_name {
            self.settings_pane.update(ctx, |settings_pane, ctx| {
                settings_pane.search_for_keybinding(keybinding_name, ctx);
            });
        }
    }

    pub fn is_theme_chooser_open(&self) -> bool {
        self.current_workspace_state.is_theme_chooser_open
    }

    /// Returns whether the workspace is currently showing a settings file
    /// error banner (i.e. settings_file_error is set and not dismissed).
    #[cfg(feature = "integration_tests")]
    pub fn has_settings_file_error_banner(&self) -> bool {
        self.settings_file_error.is_some() && !self.settings_error_banner_dismissed
    }

    fn increase_font_size(&mut self, ctx: &mut ViewContext<Self>) {
        self.adjust_terminal_font_size(FONT_SIZE_INCREMENT, ctx);
    }

    fn decrease_font_size(&mut self, ctx: &mut ViewContext<Self>) {
        self.adjust_terminal_font_size(-FONT_SIZE_INCREMENT, ctx);
    }

    fn reset_font_size(&mut self, ctx: &mut ViewContext<Self>) {
        self.set_terminal_font_size(MonospaceFontSize::default_value(), ctx);
    }

    fn increase_zoom(&mut self, ctx: &mut ViewContext<Self>) {
        self.adjust_zoom(true /* increase */, ctx);
    }

    fn decrease_zoom(&mut self, ctx: &mut ViewContext<Self>) {
        self.adjust_zoom(false /* increase */, ctx);
    }

    fn reset_zoom(&mut self, ctx: &mut ViewContext<Self>) {
        WindowSettings::handle(ctx).update(ctx, |window_settings, ctx| {
            report_if_error!(window_settings
                .zoom_level
                .set_value(ZoomLevel::default_value(), ctx));
        });
    }

    fn adjust_zoom(&mut self, increase: bool, ctx: &mut ViewContext<Self>) {
        let current_zoom = *WindowSettings::as_ref(ctx).zoom_level.value();
        let Some(current_index) = crate::window_settings::ZoomLevel::VALUES
            .iter()
            .position(|zoom| *zoom == current_zoom)
        else {
            return;
        };

        let next_index = if increase {
            (current_index + 1).min(crate::window_settings::ZoomLevel::VALUES.len() - 1)
        } else {
            current_index.saturating_sub(1)
        };

        WindowSettings::handle(ctx).update(ctx, |window_settings, ctx| {
            report_if_error!(window_settings
                .zoom_level
                .set_value(crate::window_settings::ZoomLevel::VALUES[next_index], ctx));
        });
    }

    fn adjust_terminal_font_size(&mut self, font_size_delta: f32, ctx: &mut ViewContext<Self>) {
        let appearance = Appearance::as_ref(ctx);
        let new_font_size = (appearance.monospace_font_size() + font_size_delta)
            .clamp(MIN_FONT_SIZE, MAX_FONT_SIZE);
        self.set_terminal_font_size(new_font_size, ctx);
    }

    fn set_terminal_font_size(&mut self, new_font_size: f32, ctx: &mut ViewContext<Self>) {
        FontSettings::handle(ctx).update(ctx, |font_settings, ctx| {
            report_if_error!(font_settings
                .monospace_font_size
                .set_value(new_font_size, ctx));
        });
    }

    fn toggle_mouse_reporting(&mut self, ctx: &mut ViewContext<Self>) {
        let prev_mouse_reporting_enabled =
            AltScreenReporting::handle(ctx).update(ctx, |reporting, ctx| {
                let prev_mouse_reporting_enabled = *reporting.mouse_reporting_enabled.value();
                reporting
                    .mouse_reporting_enabled
                    .set_value(!prev_mouse_reporting_enabled, ctx)
                    .expect("MouseReportingEnabled failed to serialize");
                prev_mouse_reporting_enabled
            });

        let verb = if prev_mouse_reporting_enabled {
            "disabled"
        } else {
            "enabled"
        };
        let mut message = format!("You {verb} mouse reporting.");
        if let Some(keystroke) =
            keybinding_name_to_keystroke("workspace:toggle_mouse_reporting", ctx)
        {
            let _ = write!(message, " Press {} to undo.", keystroke.displayed());
        }

        self.toast_stack.update(ctx, |view, ctx| {
            let new_toast = DismissibleToast::default(message);
            view.add_ephemeral_toast(new_toast, ctx);
        });
    }

    fn toggle_scroll_reporting(&mut self, ctx: &mut ViewContext<Self>) {
        AltScreenReporting::handle(ctx).update(ctx, |reporting, ctx| {
            reporting
                .scroll_reporting_enabled
                .toggle_and_save_value(ctx)
                .expect("ScrollReportingEnabled failed to serialize");
        });
    }

    fn toggle_focus_reporting(&mut self, ctx: &mut ViewContext<Self>) {
        AltScreenReporting::handle(ctx).update(ctx, |reporting, ctx| {
            reporting
                .focus_reporting_enabled
                .toggle_and_save_value(ctx)
                .expect("FocusReportingEnabled failed to serialize");
        });
    }

    /// This listens for changes to keybindings and keeps the cached versions up-to-date in our
    /// tooltips.
    fn handle_keybinding_changed(
        &mut self,
        event: &KeybindingChangedEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match &event {
            KeybindingChangedEvent::BindingChanged {
                binding_name,
                new_trigger: new_trigger_option,
            } => self
                .cached_keybindings
                .entry(binding_name.to_owned())
                .and_modify(|keystroke| {
                    *keystroke = new_trigger_option.as_ref().map(|key| key.displayed())
                }),
        };
        ctx.notify()
    }

    fn handle_window_state_change(&mut self, event: &StateEvent, ctx: &mut ViewContext<Self>) {
        match &event {
            StateEvent::ValueChanged { current, previous } => {
                // Re-render if fullscreen state for active window has changed.
                if current.is_active_window_fullscreen != previous.is_active_window_fullscreen {
                    ctx.notify();
                } else if WindowManager::did_window_change_focus(self.window_id, current, previous)
                {
                    // Re-render if this window's focus state has changed.
                    ctx.notify();
                } else if current.stage != previous.stage {
                    // Re-render if the app's focus state has changed (Active/Inactive)
                    // This ensures dimming updates properly when the app gains/loses focus
                    ctx.notify();
                }
            }
        };
    }

    fn handle_codex_modal_event(&mut self, event: &CodexModalEvent, ctx: &mut ViewContext<Self>) {
        use crate::ai::blocklist::agent_view::AgentViewEntryOrigin;
        use crate::AIExecutionProfilesModel;

        match event {
            CodexModalEvent::Close => {
                self.current_workspace_state.is_codex_modal_open = false;
                self.focus_active_tab(ctx);
                ctx.notify();
            }
            CodexModalEvent::UseCodex => {
                // Add a new terminal tab
                self.add_new_session_tab_internal_with_default_session_mode_behavior(
                    NewSessionSource::Tab,
                    Some(ctx.window_id()),
                    None,
                    None,
                    false,
                    DefaultSessionModeBehavior::Ignore,
                    ctx,
                );
                ctx.notify();

                // Get the active terminal view
                let Some(terminal_view) = self
                    .active_tab_pane_group()
                    .as_ref(ctx)
                    .active_session_view(ctx)
                else {
                    log::error!("No active terminal view after adding tab for Codex session");
                    return;
                };

                let Some(codex_model_id) = LLMPreferences::as_ref(ctx)
                    .get_preferred_codex_model()
                    .map(|info| info.id.clone())
                else {
                    log::error!("No preferred codex model found");
                    return;
                };

                // Set codex as the model for the default profile and make the default profile active.
                AIExecutionProfilesModel::handle(ctx).update(ctx, |profiles, ctx| {
                    let default_profile_id = profiles.default_profile_id();
                    profiles.set_base_model(default_profile_id, Some(codex_model_id), ctx);
                    profiles.set_active_profile(terminal_view.id(), default_profile_id, ctx);
                });

                // Enter agent view and submit the initial prompt
                let initial_prompt = "Hello, Agent Mode x Codex!".to_string();
                terminal_view.update(ctx, |terminal_view, ctx| {
                    terminal_view.enter_agent_view_for_new_conversation(
                        Some(initial_prompt),
                        AgentViewEntryOrigin::CodexModal,
                        ctx,
                    );
                });

                self.current_workspace_state.is_codex_modal_open = false;
                ctx.notify();
            }
        }
    }

    #[cfg(not(target_family = "wasm"))]
    fn open_plugin_instructions_pane(
        &mut self,
        agent: crate::terminal::CLIAgent,
        kind: PluginModalKind,
        ctx: &mut ViewContext<Self>,
    ) {
        use crate::terminal::model::rich_content::RichContentType;
        use crate::terminal::view::plugin_instructions_block::{
            PluginInstructionsBlock, PluginInstructionsBlockEvent,
        };
        use crate::terminal::view::rich_content::{
            RichContentInsertionPosition, RichContentMetadata,
        };

        let Some(manager) = plugin_manager_for(agent) else {
            return;
        };

        let instructions = match kind {
            PluginModalKind::Install => manager.install_instructions(),
            PluginModalKind::Update => manager.update_instructions(),
        };

        // Read session metadata from the originating terminal before creating the instructions pane.
        let active_view = self
            .active_tab_pane_group()
            .as_ref(ctx)
            .active_session_view(ctx);

        let is_remote_session = active_view
            .as_ref()
            .and_then(|view| view.as_ref(ctx).active_session_is_local(ctx))
            .is_some_and(|is_local| !is_local);

        let custom_command_prefix = active_view.and_then(|view| {
            CLIAgentSessionsModel::as_ref(ctx)
                .session(view.id())
                .and_then(|s| s.custom_command_prefix.clone())
        });

        self.active_tab_pane_group().update(ctx, |pane_group, ctx| {
            let pane_id = pane_group.add_terminal_pane_ignoring_default_session_mode(
                pane_group::Direction::Right,
                None,
                ctx,
            );

            if let Some(terminal_view) = pane_group.terminal_view_from_pane_id(pane_id, ctx) {
                terminal_view.update(ctx, |view, ctx| {
                    let custom_command_prefix = custom_command_prefix.clone();
                    let block = ctx.add_typed_action_view(|ctx| {
                        PluginInstructionsBlock::new(
                            instructions,
                            agent,
                            custom_command_prefix,
                            is_remote_session,
                            ctx,
                        )
                    });
                    ctx.subscribe_to_view(&block, |view, block, event, ctx| match event {
                        PluginInstructionsBlockEvent::Close => {
                            view.remove_plugin_instructions_block(block.clone(), ctx);
                        }
                    });
                    view.insert_rich_content(
                        Some(RichContentType::PluginInstructionsBlock),
                        block,
                        Some(RichContentMetadata::PluginInstructionsBlock),
                        RichContentInsertionPosition::Append {
                            insert_below_long_running_block: false,
                        },
                        ctx,
                    );
                });
            }
        });
    }

    /// Opens the Codex modal.
    pub fn open_codex_modal(&mut self, ctx: &mut ViewContext<Self>) {
        self.current_workspace_state.is_codex_modal_open = true;
        ctx.focus(&self.codex_modal);
        ctx.notify();
    }

    /// Opens a new tab and enters agent view with a prompt from a Linear deeplink.
    pub fn open_linear_issue_work(
        &mut self,
        args: &crate::linear::LinearIssueWork,
        ctx: &mut ViewContext<Self>,
    ) {
        self.add_new_session_tab_internal_with_default_session_mode_behavior(
            NewSessionSource::Tab,
            Some(ctx.window_id()),
            None,  // Chosen shell
            None,  // Conversation restoration
            false, // Hide the agent view homepage
            DefaultSessionModeBehavior::Ignore,
            ctx,
        );

        let Some(terminal_view) = self
            .active_tab_pane_group()
            .as_ref(ctx)
            .active_session_view(ctx)
        else {
            log::error!("No active terminal view after adding tab for Linear issue work");
            return;
        };

        let prompt = args.prompt.clone();
        terminal_view.update(ctx, |terminal_view, ctx| {
            terminal_view.enter_agent_view_for_new_conversation(
                prompt,
                AgentViewEntryOrigin::LinearDeepLink,
                ctx,
            );
        });

        if let Some(conversation_id) = terminal_view
            .as_ref(ctx)
            .agent_view_controller()
            .as_ref(ctx)
            .agent_view_state()
            .active_conversation_id()
        {
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, _ctx| {
                if let Some(conversation) = history.conversation_mut(&conversation_id) {
                    conversation.set_fallback_display_title("Linear Issue".to_string());
                }
            });
        }
    }

    fn focus_active_tab(&mut self, ctx: &mut ViewContext<Self>) {
        self.active_tab_pane_group().update(ctx, |tab, ctx| {
            tab.focus(ctx);
        })
    }

    fn focus_theme_chooser(&self, ctx: &mut ViewContext<Self>) {
        ctx.focus(&self.theme_chooser_view);
        ctx.notify();
    }

    fn open_prompt_editor(&mut self, ctx: &mut ViewContext<Self>) {
        // Try to get a prompt preview from an active session. Otherwise, read it from the settings
        // view.
        let ps1_grid_info = self.active_session_ps1_grid_info(ctx).or_else(|| {
            self.settings_pane
                .read(ctx, |settings, app| settings.get_ps1_info(app))
        });
        let chip_runtime_capabilities = self
            .active_tab_pane_group()
            .as_ref(ctx)
            .active_session_view(ctx)
            .and_then(|terminal_view| {
                terminal_view.read(ctx, |terminal, ctx| {
                    let required_executables = crate::context_chips::available_chips()
                        .into_iter()
                        .filter_map(|kind| kind.to_chip())
                        .flat_map(|chip| chip.runtime_policy().required_executables().to_vec())
                        .collect::<std::collections::HashSet<_>>();
                    terminal
                        .active_block_session_id()
                        .and_then(|id| terminal.sessions_model().as_ref(ctx).get(id))
                        .as_deref()
                        .map(|session| {
                            ChipRuntimeCapabilities::from_session_with_external_command_queries(
                                session,
                                required_executables.iter().map(String::as_str),
                                false,
                            )
                        })
                })
            })
            .unwrap_or_default();
        self.prompt_editor_modal.update(ctx, |prompt_editor, ctx| {
            prompt_editor.open(ps1_grid_info, chip_runtime_capabilities, ctx);
        });
        self.close_all_overlays(ctx);
        self.current_workspace_state.is_prompt_editor_open = true;
        ctx.focus(&self.prompt_editor_modal);
    }

    fn open_agent_toolbar_editor(
        &mut self,
        mode: AgentToolbarEditorMode,
        ctx: &mut ViewContext<Self>,
    ) {
        if !FeatureFlag::AgentToolbarEditor.is_enabled() {
            return;
        }
        self.agent_toolbar_editor_modal
            .update(ctx, |modal, ctx| modal.open(mode, ctx));
        self.close_all_overlays(ctx);
        self.current_workspace_state.is_agent_toolbar_editor_open = true;
        ctx.focus(&self.agent_toolbar_editor_modal);
    }

    fn open_theme_creator_modal(&mut self, ctx: &mut ViewContext<Self>) {
        self.current_workspace_state.is_theme_creator_modal_open = true;
        ctx.focus(&self.theme_creator_modal);
        ctx.notify();
    }

    fn open_theme_deletion_modal(&mut self, theme_kind: ThemeKind, ctx: &mut ViewContext<Self>) {
        self.current_workspace_state.is_theme_deletion_modal_open = true;
        self.theme_deletion_modal
            .update(ctx, |theme_deletion_modal, ctx| {
                theme_deletion_modal.set_theme_kind(theme_kind, ctx);
            });
        ctx.focus(&self.theme_deletion_modal);
        ctx.notify();
    }

    fn render_tab_in_tab_bar(
        &self,
        tab_index: usize,
        tab_bar_state: TabBarState,
        ctx: &AppContext,
    ) -> Box<dyn Element> {
        let tab = &self.tabs[tab_index];
        let close_button_position = if FeatureFlag::TabCloseButtonOnLeft.is_enabled() {
            TabSettings::as_ref(ctx).close_button_position
        } else {
            TabCloseButtonPosition::default()
        };

        let is_drag_target = self
            .hovered_tab_index
            .as_ref()
            .is_some_and(|hovered_index| match hovered_index {
                TabBarHoverIndex::OverTab(idx) => *idx == tab_index,
                TabBarHoverIndex::BeforeTab(_) => false,
            });

        TabComponent::new(
            tab_index,
            tab_bar_state,
            tab,
            self.tab_rename_editor.clone(),
            close_button_position,
            is_drag_target,
            ctx,
        )
        .build()
        .finish()
    }

    fn render_left_toggle_button(
        &self,
        appearance: &Appearance,
        ctx: &AppContext,
    ) -> Box<dyn Element> {
        let vertical_tabs_active =
            FeatureFlag::VerticalTabs.is_enabled() && *TabSettings::as_ref(ctx).use_vertical_tabs;

        let (is_active, tooltip_text, action, keybinding_name, save_position_id) =
            if vertical_tabs_active {
                (
                    self.vertical_tabs_panel_open,
                    "Tabs panel",
                    WorkspaceAction::ToggleVerticalTabsPanel,
                    "workspace:toggle_vertical_tabs_panel",
                    "workspace:toggle_vertical_tabs_panel",
                )
            } else {
                let tooltip = if self.left_panel_views.len() <= 1 {
                    match self
                        .left_panel_views
                        .first()
                        .copied()
                        .unwrap_or(ToolPanelView::ProjectExplorer)
                    {
                        ToolPanelView::ProjectExplorer => "Project explorer",
                        ToolPanelView::GlobalSearch { .. } => "Global search",
                    }
                } else {
                    "Tools panel"
                };
                (
                    self.active_tab_pane_group().as_ref(ctx).left_panel_open,
                    tooltip,
                    WorkspaceAction::ToggleLeftPanel,
                    "workspace:toggle_left_panel",
                    "workspace:toggle_left_panel",
                )
            };

        SavePosition::new(
            Container::new(
                Align::new(
                    self.render_tab_bar_icon_button(
                        appearance,
                        icons::Icon::Menu,
                        &self.mouse_states.left_panel_icon,
                        action,
                        tooltip_text.to_string(),
                        keybinding_name_to_display_string(keybinding_name, ctx),
                        is_active,
                        false,
                    )
                    .finish(),
                )
                .finish(),
            )
            .finish(),
            save_position_id,
        )
        .finish()
    }

    fn render_tools_panel_button(
        &self,
        appearance: &Appearance,
        ctx: &AppContext,
    ) -> Box<dyn Element> {
        let is_active = self.active_tab_pane_group().as_ref(ctx).left_panel_open;

        let tooltip_text = if self.left_panel_views.len() <= 1 {
            match self
                .left_panel_views
                .first()
                .copied()
                .unwrap_or(ToolPanelView::ProjectExplorer)
            {
                ToolPanelView::ProjectExplorer => "Project explorer",
                ToolPanelView::GlobalSearch { .. } => "Global search",
            }
        } else {
            "Tools panel"
        };

        SavePosition::new(
            Container::new(
                Align::new(
                    self.render_tab_bar_icon_button(
                        appearance,
                        icons::Icon::Tool2,
                        &self.mouse_states.tools_panel_icon,
                        WorkspaceAction::ToggleLeftPanel,
                        tooltip_text.to_string(),
                        keybinding_name_to_display_string("workspace:toggle_left_panel", ctx),
                        is_active,
                        false,
                    )
                    .finish(),
                )
                .finish(),
            )
            .finish(),
            "workspace:toggle_left_panel",
        )
        .finish()
    }

    fn should_enable_file_tree_and_global_search_for_pane_group(pane_group: &PaneGroup) -> bool {
        pane_group
            .pane_ids()
            .filter(|id| !pane_group.is_pane_hidden_for_close(*id))
            .any(|id| {
                id.is_terminal_pane()
                    || id.is_file_pane()
                    || id.is_code_pane()
                    || id.is_code_diff_pane()
            })
    }

    fn render_right_panel_button(
        &self,
        appearance: &Appearance,
        ctx: &AppContext,
    ) -> Box<dyn Element> {
        let is_active = self.active_tab_pane_group().as_ref(ctx).right_panel_open;
        let is_enabled = Self::should_enable_file_tree_and_global_search_for_pane_group(
            self.active_tab_pane_group().as_ref(ctx),
        );
        let disable = !is_enabled;

        let theme = appearance.theme();
        let font_color = if disable {
            theme.disabled_text_color(theme.background())
        } else if is_active {
            theme.main_text_color(theme.background())
        } else {
            theme.sub_text_color(theme.background())
        };

        // Build the button content: Diff icon + optional diff stats
        let icon = ConstrainedBox::new(icons::Icon::Diff.to_warpui_icon(font_color).finish())
            .with_width(16.)
            .with_height(16.)
            .finish();

        let show_diff_stats = *TabSettings::as_ref(ctx).show_code_review_diff_stats;

        let line_changes = if show_diff_stats {
            self.active_tab_pane_group()
                .as_ref(ctx)
                .active_session_view(ctx)
                .and_then(|tv| tv.as_ref(ctx).current_diff_line_changes(ctx))
                .filter(|lc| {
                    // Only show the stat badge when there are actual line-level changes
                    // (files_changed alone, e.g. mode-only changes, is not surfaced here).
                    lc.lines_added > 0 || lc.lines_removed > 0
                })
        } else {
            None
        };

        let has_stats = line_changes.is_some();

        let mut row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
        row.add_child(icon);

        if let Some(lc) = line_changes {
            let stat = |value: u32, prefix: &str, color: ColorU| -> Box<dyn Element> {
                Container::new(
                    Text::new_inline(format!("{prefix}{value}"), appearance.ui_font_family(), 12.)
                        .with_color(color)
                        .with_style(Properties::default().weight(Weight::Semibold))
                        .finish(),
                )
                .with_margin_left(4.)
                .finish()
            };
            row.add_child(stat(lc.lines_added, "+", add_color(appearance)));
            row.add_child(stat(lc.lines_removed, "-", remove_color(appearance)));
        }

        let label = row.finish();

        // The diff icon SVG has intrinsic horizontal whitespace in its 14px viewBox: its visible
        // paths start around x=3 and end around x=11. When stats are shown, equal container padding
        // makes the gap between the button edge and the visible icon look wider than the gap after
        // the text. Locally compensate for that artwork padding without changing the shared icon.
        let (header_padding_left, header_padding_right) =
            if has_stats { (5., 8.) } else { (4., 4.) };
        let default_styles = UiComponentStyles {
            font_color: Some(font_color.into()),
            font_size: Some(12.),
            font_weight: Some(Weight::Medium),
            font_family_id: Some(appearance.ui_font_family()),
            height: Some(24.),
            border_radius: Some(CornerRadius::with_all(Radius::Pixels(4.))),
            border_width: Some(0.),
            padding: Some(Coords {
                top: 0.,
                bottom: 0.,
                left: header_padding_left,
                right: header_padding_right,
            }),
            ..Default::default()
        };

        let hover_styles = UiComponentStyles {
            background: Some(theme.surface_2().into()),
            ..default_styles
        };

        let clicked_styles = UiComponentStyles {
            background: Some(theme.background().into()),
            ..default_styles
        };

        let mut button = Button::new(
            self.mouse_states.right_panel_icon.clone(),
            default_styles,
            Some(hover_styles),
            Some(clicked_styles),
            None,
        )
        .with_custom_label(label);

        if is_active {
            button = button.active().with_active_styles(UiComponentStyles {
                background: Some(internal_colors::fg_overlay_3(theme).into()),
                ..UiComponentStyles::default()
            });
        }

        let hoverable = if disable {
            button.build().disable()
        } else {
            button
                .with_tooltip(self.render_tab_bar_icon_button_tooltip(
                    appearance,
                    "Code review panel".to_string(),
                    keybinding_name_to_display_string("workspace:toggle_right_panel", ctx),
                ))
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(WorkspaceAction::ToggleRightPanel);
                })
        };

        SavePosition::new(
            Container::new(Align::new(hoverable.finish()).finish()).finish(),
            "workspace:right_panel_button",
        )
        .finish()
    }

    /// Renders an invisible rect for detecting hovers over the tab bar.
    fn render_tab_bar_hover_area(&self) -> Box<dyn Element> {
        self.render_tab_bar_hoverable(
            ConstrainedBox::new(Empty::new().finish())
                .with_height(TAB_BAR_HOVER_HEIGHT)
                .finish(),
        )
    }

    /// Renders the provided content wrapped in the tab bar hover behavior.
    fn render_tab_bar_hoverable(&self, content: Box<dyn Element>) -> Box<dyn Element> {
        Hoverable::new(self.tab_bar_hover_state.clone(), |_| content)
            .with_hover_out_delay(Duration::from_millis(500))
            .on_hover(|_is_hovered, ctx, _app, _position| {
                ctx.dispatch_typed_action(WorkspaceAction::SyncTrafficLights);
            })
            .finish()
    }

    fn render_tab_hover_indicator(&self, appearance: &Appearance) -> Box<dyn Element> {
        ConstrainedBox::new(
            Rect::new()
                .with_background(appearance.theme().accent())
                .finish(),
        )
        .with_height(32.)
        .with_width(4.)
        .finish()
    }

    fn render_title_bar_search_bar(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let text_color = theme.sub_text_color(theme.background());

        Hoverable::new(
            self.mouse_states.title_bar_search_bar.clone(),
            |mouse_state| {
                let row = Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(10.)
                    .with_child(
                        ConstrainedBox::new(
                            icons::Icon::Search.to_warpui_icon(text_color).finish(),
                        )
                        .with_width(16.)
                        .with_height(16.)
                        .finish(),
                    )
                    .with_child(
                        Shrinkable::new(
                            1.,
                            Text::new_inline(
                                "Search sessions, agents, files...",
                                appearance.ui_font_family(),
                                14.,
                            )
                            .with_color(text_color.into())
                            .with_clip(ClipConfig::ellipsis())
                            .finish(),
                        )
                        .finish(),
                    )
                    .finish();

                ConstrainedBox::new(
                    Container::new(row)
                        .with_background(if mouse_state.is_hovered() {
                            internal_colors::fg_overlay_2(theme)
                        } else {
                            internal_colors::fg_overlay_1(theme)
                        })
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                        .with_padding_left(16.)
                        .with_padding_right(16.)
                        .with_padding_top(4.)
                        .with_padding_bottom(4.)
                        .finish(),
                )
                .with_width(TITLE_BAR_SEARCH_BAR_MAX_WIDTH)
                .finish()
            },
        )
        .with_cursor(Cursor::PointingHand)
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action(WorkspaceAction::OpenPalette {
                mode: PaletteMode::Command,
                source: PaletteSource::TitleBarSearchBar,
                query: None,
            });
        })
        .finish()
    }

    fn render_tab_bar_contents(
        &self,
        hover_fixed_width: Option<f32>,
        appearance: &Appearance,
        ctx: &AppContext,
    ) -> Box<dyn Element> {
        let mut tab_bar = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
        let is_web_anonymous_user = false;

        // Simplified mode for legacy restored non-terminal views on WASM.
        #[cfg(target_family = "wasm")]
        if let Some(_content_type) = self.get_simplified_wasm_tab_bar_content(ctx) {
            // Use MainAxisAlignment::SpaceBetween and expand to fill width
            tab_bar = tab_bar
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_main_axis_size(MainAxisSize::Max);
            let bg_color = blended_colors::neutral_1(appearance.theme());

            // Left: Warper logo - clickable to link to the Warper repository.
            let warp_logo = Hoverable::new(self.mouse_states.warp_logo.clone(), |_state| {
                ConstrainedBox::new(
                    warp_core::ui::Icon::Warp
                        .to_warpui_icon(appearance.theme().foreground())
                        .finish(),
                )
                .with_height(24.)
                .with_width(24.)
                .finish()
            })
            .on_click(|ctx, _, _| {
                ctx.dispatch_typed_action(WorkspaceAction::OpenLink(
                    "https://github.com/ruslanvakhitov/warper".to_owned(),
                ));
            })
            .with_cursor(Cursor::PointingHand)
            .finish();
            tab_bar.add_child(warp_logo);

            // Right: "Open in Warper" button
            let mut right_row = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_main_axis_size(MainAxisSize::Min);

            // Hide "Open in Warper" button on mobile devices
            if !warpui::platform::wasm::is_mobile_device() {
                right_row.add_child(ChildView::new(&self.open_in_warp_button).finish());
            }
            tab_bar.add_child(right_row.finish());

            return Container::new(tab_bar.finish())
                .with_background_color(bg_color)
                .with_border(
                    Border::bottom(1.0)
                        .with_border_fill(blended_colors::neutral_2(appearance.theme())),
                )
                .with_padding_left(24.)
                .with_padding_right(24.)
                .with_padding_top(4.)
                .with_padding_bottom(4.)
                .finish();
        }

        // Check if vertical tabs mode is active
        let vertical_tabs_active =
            FeatureFlag::VerticalTabs.is_enabled() && *TabSettings::as_ref(ctx).use_vertical_tabs;

        // Render config-driven left-side toolbar buttons (both horizontal and vertical tabs)
        let knowledge_center_closed = true;
        let config = TabSettings::as_ref(ctx)
            .header_toolbar_chip_selection
            .clone();
        if knowledge_center_closed && !self.is_theme_chooser_open() {
            let left_toolbar_buttons = config
                .left_items()
                .into_iter()
                .filter_map(|item| self.render_header_toolbar_button(&item, appearance, ctx))
                .collect::<Vec<_>>();
            let left_toolbar_button_count = left_toolbar_buttons.len();
            for (index, button) in left_toolbar_buttons.into_iter().enumerate() {
                let is_last_left_toolbar_button = index + 1 == left_toolbar_button_count;
                if !vertical_tabs_active && is_last_left_toolbar_button {
                    tab_bar.add_child(Container::new(button).with_margin_right(8.).finish());
                } else {
                    tab_bar.add_child(button);
                }
            }
        }

        if vertical_tabs_active {
            let mut right_controls = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_main_axis_size(MainAxisSize::Min);

            self.add_configurable_right_side_tab_bar_controls(
                &mut right_controls,
                &config,
                is_web_anonymous_user,
                appearance,
                ctx,
            );

            let left_padding = self.compute_tab_bar_left_padding(ctx);

            let tab_bar = Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(tab_bar.finish())
                .with_child(
                    Shrinkable::new(
                        1.,
                        Clipped::new(
                            Container::new(
                                Align::new(self.render_title_bar_search_bar(appearance)).finish(),
                            )
                            .with_padding_left(TITLE_BAR_SEARCH_BAR_SLOT_PADDING)
                            .with_padding_right(TITLE_BAR_SEARCH_BAR_SLOT_PADDING)
                            .finish(),
                        )
                        .finish(),
                    )
                    .finish(),
                )
                .with_child(right_controls.finish())
                .finish();

            return EventHandler::new(
                Container::new(tab_bar)
                    .with_padding_left(left_padding)
                    .with_padding_right(TAB_BAR_PADDING_RIGHT)
                    .finish(),
            )
            .on_right_mouse_down(|ctx, _, position| {
                ctx.dispatch_typed_action(WorkspaceAction::ShowHeaderToolbarContextMenu {
                    position,
                });
                DispatchEventResult::StopPropagation
            })
            .finish();
        } else {
            // Copy from our saved tab_bar_state to ensure all tabs get rendered with the same state
            let active_tab_index = Some(self.active_tab_index);

            let tab_bar_state = TabBarState {
                tab_count: self.tabs.len(),
                active_tab_index,
                is_any_tab_renaming: self.current_workspace_state.is_tab_being_renamed(),
                is_any_tab_dragging: self.current_workspace_state.is_tab_being_dragged,
                hover_fixed_width,
            };

            for i in 0..self.tabs.len() {
                // If we are hovered between two tabs, show the drop hover indicator
                if self.hovered_tab_index.as_ref().is_some_and(
                    |hovered_index| match hovered_index {
                        TabBarHoverIndex::BeforeTab(idx) => i == *idx,
                        TabBarHoverIndex::OverTab(_) => false,
                    },
                ) {
                    tab_bar.add_child(self.render_tab_hover_indicator(appearance));
                }
                tab_bar.add_child(self.render_tab_in_tab_bar(i, tab_bar_state, ctx));
            }

            // Fencepost problem - add the indicator at the end if needed
            if self
                .hovered_tab_index
                .as_ref()
                .is_some_and(|hovered_index| match hovered_index {
                    TabBarHoverIndex::BeforeTab(idx) => self.tabs.len() == *idx,
                    TabBarHoverIndex::OverTab(_) => false,
                })
            {
                tab_bar.add_child(self.render_tab_hover_indicator(appearance));
            }

            if ContextFlag::CreateNewSession.is_enabled() {
                tab_bar.add_child(self.render_new_session_button(ctx));
            }
        }

        // Placeholder to make sure the flex row expands across the entire width of the app.
        tab_bar.add_child(Shrinkable::new(0.5, Empty::new().finish()).finish());

        self.add_configurable_right_side_tab_bar_controls(
            &mut tab_bar,
            &config,
            is_web_anonymous_user,
            appearance,
            ctx,
        );

        let left_padding = self.compute_tab_bar_left_padding(ctx);

        EventHandler::new(
            Container::new(tab_bar.finish())
                .with_padding_left(left_padding)
                .with_padding_right(TAB_BAR_PADDING_RIGHT)
                .finish(),
        )
        .on_right_mouse_down(|ctx, _, position| {
            ctx.dispatch_typed_action(WorkspaceAction::ShowHeaderToolbarContextMenu { position });
            DispatchEventResult::StopPropagation
        })
        .finish()
    }

    /// Renders a single header toolbar button for the given item kind.
    /// Returns `None` if the item is not currently available.
    /// The button is wrapped with a right-click handler that opens the
    /// toolbar configurator.
    fn render_header_toolbar_button(
        &self,
        item: &HeaderToolbarItemKind,
        appearance: &Appearance,
        ctx: &AppContext,
    ) -> Option<Box<dyn Element>> {
        if !item.is_available(ctx) {
            return None;
        }
        let vertical_tabs_active =
            FeatureFlag::VerticalTabs.is_enabled() && *TabSettings::as_ref(ctx).use_vertical_tabs;
        let inner = match item {
            HeaderToolbarItemKind::TabsPanel => self.render_left_toggle_button(appearance, ctx),
            HeaderToolbarItemKind::ToolsPanel => {
                if self.left_panel_views.is_empty() {
                    return None;
                }
                if vertical_tabs_active {
                    self.render_tools_panel_button(appearance, ctx)
                } else {
                    self.render_left_toggle_button(appearance, ctx)
                }
            }
            HeaderToolbarItemKind::CodeReview => self.render_right_panel_button(appearance, ctx),
        };
        Some(
            Container::new(
                EventHandler::new(inner)
                    .on_right_mouse_down(|ctx, _, position| {
                        ctx.dispatch_typed_action(WorkspaceAction::ShowHeaderToolbarContextMenu {
                            position,
                        });
                        DispatchEventResult::StopPropagation
                    })
                    .finish(),
            )
            .with_margin_left(TAB_BAR_ICON_PADDING)
            .finish(),
        )
    }

    /// Adds the configurable right-side toolbar items plus the fixed controls
    /// (update pill, offline indicator, avatar, etc.) that are not configurable.
    fn add_configurable_right_side_tab_bar_controls(
        &self,
        target: &mut Flex,
        config: &crate::workspace::tab_settings::HeaderToolbarChipSelection,
        _is_web_anonymous_user: bool,
        appearance: &Appearance,
        ctx: &AppContext,
    ) {
        if let Some(update_pill) = self.render_tab_overflow_menu(ctx, appearance) {
            target.add_child(
                Container::new(update_pill)
                    .with_margin_left(TAB_BAR_PADDING_LEFT)
                    .finish(),
            );
        }

        let is_online = NetworkStatus::as_ref(ctx).is_online();

        if !is_online {
            target.add_child(
                Container::new(self.render_offline_button(appearance))
                    .with_margin_right(4.)
                    .finish(),
            );
        }

        for item in config.right_items() {
            if let Some(button) = self.render_header_toolbar_button(&item, appearance, ctx) {
                target.add_child(button);
            }
        }

        if FeatureFlag::AvatarInTabBar.is_enabled() {
            target.add_child(
                Container::new(self.render_avatar_button(appearance, ctx))
                    .with_margin_left(TAB_BAR_PADDING_LEFT)
                    .finish(),
            );
        } else {
            let resource_center_closed = !self.current_workspace_state.is_resource_center_open;
            if resource_center_closed && ContextFlag::WarpEssentials.is_enabled() {
                target.add_child(
                    Container::new(self.render_resource_center_button(appearance, ctx))
                        .with_margin_left(TAB_BAR_PADDING_LEFT)
                        .finish(),
                );
            }

            target.add_child(
                Container::new(self.render_settings_button(appearance))
                    .with_margin_left(TAB_BAR_PADDING_LEFT)
                    .finish(),
            );
        }

        let zoom_factor = WindowSettings::as_ref(ctx).zoom_level.as_zoom_factor();
        let traffic_light_data = traffic_light_data(ctx, self.window_id);
        if let Some(traffic_light_data) = traffic_light_data.as_ref() {
            let vertical_tabs_active = FeatureFlag::VerticalTabs.is_enabled()
                && *TabSettings::as_ref(ctx).use_vertical_tabs;
            let right_panel_open = self.current_workspace_state.is_right_panel_open();
            let should_reserve_right_traffic_light_space =
                vertical_tabs_active || !right_panel_open;

            if traffic_light_data.side == TrafficLightSide::Right
                && should_reserve_right_traffic_light_space
            {
                target.add_child(
                    ConstrainedBox::new(Empty::new().finish())
                        .with_width(traffic_light_data.width(zoom_factor))
                        .finish(),
                );
            }
        }
    }

    fn compute_tab_bar_left_padding(&self, ctx: &AppContext) -> f32 {
        let zoom_factor = WindowSettings::as_ref(ctx).zoom_level.as_zoom_factor();
        let traffic_light_data = traffic_light_data(ctx, self.window_id);
        let is_window_fullscreen = ctx
            .windows()
            .platform_window(self.window_id)
            .map(|window| window.fullscreen_state() == FullscreenState::Fullscreen)
            .unwrap_or(false);
        if self.current_workspace_state.is_left_panel_open() {
            0.
        } else if is_window_fullscreen && cfg!(target_os = "macos") {
            // Full-screen mode on MacOS does not need as much padding (traffic lights are hidden).
            TAB_BAR_PADDING_LEFT
        } else {
            traffic_light_data
                .as_ref()
                .filter(|data| data.side == TrafficLightSide::Left)
                .map(|data| data.width(zoom_factor))
                .unwrap_or(0.)
                + 16.
        }
    }

    /// Renders the tab bar contents, wrapped in hover and drag-drop behaviors.
    fn render_tab_bar(
        &self,
        tab_fixed_width: Option<f32>,
        appearance: &Appearance,
        ctx: &AppContext,
    ) -> Box<dyn Element> {
        let bar_contents = ConstrainedBox::new(
            // We can wrap the whole tab bar in the a drop target with the `AfterTabIndex` drop target data since the API for accepting a drop target with nested
            // drop target elements will default to the inner ones (in this case the tabs or the button before the tabs)
            DropTarget::new(
                self.render_tab_bar_contents(tab_fixed_width, appearance, ctx),
                TabBarDropTargetData {
                    tab_bar_location: TabBarLocation::AfterTabIndex(self.tabs.len()),
                },
            )
            .finish(),
        )
        .with_height(TAB_BAR_HEIGHT)
        .finish();

        let tab_bar_border =
            Border::bottom(TAB_BAR_BORDER_HEIGHT).with_border_fill(appearance.theme().outline());

        let mut tab_bar_container = Container::new(
            EventHandler::new(Clipped::new(self.render_tab_bar_hoverable(bar_contents)).finish())
                .on_back_mouse_down(move |ctx, _app, _position| {
                    ctx.dispatch_typed_action(WorkspaceAction::ActivatePrevTab);
                    DispatchEventResult::StopPropagation
                })
                .on_forward_mouse_down(move |ctx, _app, _position| {
                    ctx.dispatch_typed_action(WorkspaceAction::ActivateNextTab);
                    DispatchEventResult::StopPropagation
                })
                .finish(),
        )
        .with_border(tab_bar_border);
        if FeatureFlag::NewTabStyling.is_enabled() {
            tab_bar_container = tab_bar_container
                .with_background(internal_colors::fg_overlay_1(appearance.theme()));
        }
        let tab_bar_element = tab_bar_container.finish();

        let dimming_color = appearance.theme().background().into();
        SavePosition::new(
            WindowFocusDimming::apply_panel_header_dimming(
                tab_bar_element,
                self.mouse_states.header_dimming.clone(),
                TAB_BAR_HEIGHT,
                dimming_color,
                self.window_id,
                ctx,
            ),
            TAB_BAR_POSITION_ID,
        )
        .finish()
    }

    // Render traffic lights, if appropriate for the current platform.
    fn maybe_render_traffic_lights(&self, stack: &mut Stack, app: &AppContext) {
        let Some(traffic_light_data) = traffic_light_data(app, self.window_id) else {
            return;
        };

        let appearance = Appearance::as_ref(app);
        let fullscreen_state = app
            .windows()
            .platform_window(self.window_id)
            .map(|window| window.fullscreen_state())
            .unwrap_or_default();
        stack.add_positioned_child(
            traffic_light_data.render(
                fullscreen_state,
                &self.traffic_light_mouse_states,
                appearance.theme(),
                app,
            ),
            OffsetPositioning::offset_from_parent(
                Vector2F::zero(),
                ParentOffsetBounds::WindowByPosition,
                ParentAnchor::TopRight,
                ChildAnchor::TopRight,
            ),
        );
    }

    fn render_new_session_button(&self, ctx: &AppContext) -> Box<dyn Element> {
        const CORNER_RADIUS: Radius = Radius::Pixels(4.);
        const BUTTON_HEIGHT: f32 = 24.;
        const SIDE_MENU_WIDTH: f32 = 16.;
        const BUTTON_WIDTH: f32 = 24. + SIDE_MENU_WIDTH;
        const BUTTON_LEFT_MARGIN: f32 = 4.;

        let new_tab_tool_tip_label_text = "New Tab".to_string();
        let new_tab_tool_tip_sublabel_text =
            keybinding_name_to_display_string(NEW_TAB_BINDING_NAME, ctx);
        let tab_configs_tool_tip_label_text = "Tab configs".to_string();
        let tab_configs_tool_tip_sublabel_text =
            keybinding_name_to_display_string(TOGGLE_TAB_CONFIGS_MENU_BINDING_NAME, ctx);
        let appearance = Appearance::as_ref(ctx);

        if !FeatureFlag::ShellSelector.is_enabled() {
            // Legacy new tab button, which shows the menu on right click.
            let new_tab_button = self
                .render_tab_bar_icon_button(
                    appearance,
                    icons::Icon::Plus,
                    &self.mouse_states.new_tab_button.clone(),
                    WorkspaceAction::AddDefaultTab,
                    new_tab_tool_tip_label_text,
                    new_tab_tool_tip_sublabel_text,
                    false,
                    false,
                )
                .on_right_click(move |ctx, _, position| {
                    ctx.dispatch_typed_action(WorkspaceAction::ToggleNewSessionMenu {
                        position,
                        is_vertical_tabs: false,
                    });
                })
                .finish();
            return Container::new(
                SavePosition::new(
                    Align::new(new_tab_button).finish(),
                    NEW_TAB_BUTTON_POSITION_ID,
                )
                .finish(),
            )
            .with_margin_left(BUTTON_LEFT_MARGIN)
            .finish();
        }

        let theme = appearance.theme();

        Hoverable::new(self.mouse_states.new_tab.clone(), |state| {
            let window_id = self.window_id;
            let is_active = self.show_new_session_dropdown_menu.is_some();

            let new_tab_button = combo_inner_button(
                appearance,
                icons::Icon::Plus,
                false,
                self.mouse_states.new_tab_button.clone(),
            )
            .with_style(
                UiComponentStyles::default()
                    .set_border_radius(CornerRadius::with_left(CORNER_RADIUS)),
            )
            .with_tooltip(self.render_tab_bar_icon_button_tooltip(
                appearance,
                new_tab_tool_tip_label_text.clone(),
                new_tab_tool_tip_sublabel_text.clone(),
            ))
            .build()
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(WorkspaceAction::AddDefaultTab);
            })
            .finish();

            let new_session_menu_button = combo_inner_button(
                appearance,
                icons::Icon::ChevronDown,
                is_active,
                self.mouse_states.new_tab_menu.clone(),
            )
            .with_style(
                UiComponentStyles::default()
                    .set_border_radius(CornerRadius::with_right(CORNER_RADIUS))
                    .set_width(SIDE_MENU_WIDTH),
            )
            .with_active_styles(
                UiComponentStyles::default()
                    .set_background(internal_colors::fg_overlay_3(theme).into()),
            )
            .with_tooltip(self.render_tab_bar_icon_button_tooltip(
                appearance,
                tab_configs_tool_tip_label_text.clone(),
                tab_configs_tool_tip_sublabel_text.clone(),
            ))
            .build()
            .on_click(move |ctx, app, _| {
                // We are positioning the menu to the lower-left corner of the new tab button.
                // This gives the impression that both individual buttons are one big button.
                if let Some(position) =
                    app.element_position_by_id_at_last_frame(window_id, NEW_TAB_BUTTON_POSITION_ID)
                {
                    ctx.dispatch_typed_action(WorkspaceAction::ToggleNewSessionMenu {
                        position: position.lower_left(),
                        is_vertical_tabs: false,
                    });
                }
            })
            .finish();

            let row = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(
                    SavePosition::new(
                        Align::new(new_tab_button).finish(),
                        NEW_TAB_BUTTON_POSITION_ID,
                    )
                    .finish(),
                )
                .with_child(
                    SavePosition::new(
                        Align::new(new_session_menu_button).finish(),
                        NEW_SESSION_MENU_BUTTON_POSITION_ID,
                    )
                    .finish(),
                )
                .finish();

            let mut ret = Container::new(
                ConstrainedBox::new(row)
                    .with_height(BUTTON_HEIGHT)
                    .with_width(BUTTON_WIDTH)
                    .finish(),
            )
            .with_corner_radius(CornerRadius::with_all(CORNER_RADIUS))
            .with_margin_left(BUTTON_LEFT_MARGIN);

            if state.is_hovered() {
                ret = ret.with_background(internal_colors::neutral_1(theme));
            }
            ret.finish()
        })
        .finish()
    }

    fn render_avatar_button(&self, appearance: &Appearance, _ctx: &AppContext) -> Box<dyn Element> {
        let avatar = Avatar::new(
            AvatarContent::Icon(icons::Icon::Gear),
            UiComponentStyles {
                width: Some(20.),
                height: Some(20.),
                border_radius: Some(CornerRadius::with_all(Radius::Percentage(50.))),
                font_family_id: Some(appearance.ui_font_family()),
                font_weight: Some(Weight::Bold),
                background: Some(appearance.theme().accent().into()),
                font_size: Some(12.),
                font_color: Some(ColorU::black()),
                ..Default::default()
            },
        );

        let button = Hoverable::new(self.mouse_states.avatar_icon.clone(), |state| {
            let mut stack = Stack::new();
            let mut container = Container::new(avatar.build().finish())
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                .with_uniform_padding(2.);

            if state.is_mouse_over_element() && !state.is_clicked() {
                container = container.with_background(appearance.theme().surface_2());
            }
            stack.add_child(container.finish());
            stack.finish()
        })
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(WorkspaceAction::ToggleUserMenu);
        })
        .with_cursor(Cursor::PointingHand)
        .finish();

        SavePosition::new(Align::new(button).finish(), USER_AVATAR_BUTTON_POSITION_ID).finish()
    }

    fn render_resource_center_button(
        &self,
        appearance: &Appearance,
        ctx: &AppContext,
    ) -> Box<dyn Element> {
        // only show the unread indicator if the tips are NOT completed
        let should_show_unread_indicator = !self.tips_completed.as_ref(ctx).skipped_or_completed;
        let mut button = self
            .render_tab_bar_icon_button(
                appearance,
                icons::Icon::Lightbulb,
                &self.mouse_states.resource_center_icon,
                WorkspaceAction::ToggleResourceCenter,
                "Warper Essentials".to_string(),
                self.cached_keybindings[TOGGLE_RESOURCE_CENTER_KEYBINDING_NAME].clone(),
                false,
                false,
            )
            .finish();

        if should_show_unread_indicator {
            const INDICATOR_DIAMETER: f32 = 6.;
            let indicator = Container::new(
                ConstrainedBox::new(
                    WarpUiIcon::new(ELLIPSE_SVG_PATH, appearance.theme().accent()).finish(),
                )
                .with_height(INDICATOR_DIAMETER)
                .with_width(INDICATOR_DIAMETER)
                .finish(),
            )
            .finish();
            let mut stack = Stack::new();
            stack.add_child(button);
            stack.add_positioned_child(
                indicator,
                OffsetPositioning::offset_from_parent(
                    Vector2F::zero(),
                    ParentOffsetBounds::WindowByPosition,
                    ParentAnchor::TopRight,
                    ChildAnchor::TopRight,
                ),
            );
            button = stack.finish();
        }

        Align::new(button).finish()
    }

    fn render_settings_button(&self, appearance: &Appearance) -> Box<dyn Element> {
        Align::new(
            self.render_tab_bar_icon_button(
                appearance,
                icons::Icon::Gear,
                &self.mouse_states.settings_icon,
                WorkspaceAction::ShowSettings,
                "Settings".to_string(),
                self.cached_keybindings[SHOW_SETTINGS_KEYBINDING_NAME].clone(),
                false,
                false,
            )
            .finish(),
        )
        .finish()
    }

    fn render_offline_button(&self, appearance: &Appearance) -> Box<dyn Element> {
        let ui_builder = appearance.ui_builder().clone();

        let tool_tip_label_text = "Some features may be unavailable offline".to_string();
        let icon = ConstrainedBox::new(
            Container::new(
                icons::Icon::CloudOffline
                    .to_warpui_icon(appearance.theme().foreground())
                    .finish(),
            )
            .with_uniform_padding(3.)
            .finish(),
        )
        .with_width(icons::ICON_DIMENSIONS)
        .with_height(icons::ICON_DIMENSIONS)
        .finish();

        let hoverable = Hoverable::new(self.mouse_states.offline_icon.clone(), |state| {
            let mut stack = Stack::new().with_child(icon);
            if state.is_hovered() {
                let tool_tip = ui_builder.tool_tip(tool_tip_label_text);
                stack.add_positioned_overlay_child(
                    tool_tip.build().finish(),
                    OffsetPositioning::offset_from_parent(
                        vec2f(0., 4.),
                        ParentOffsetBounds::WindowByPosition,
                        ParentAnchor::BottomMiddle,
                        ChildAnchor::TopMiddle,
                    ),
                );
            }
            stack.finish()
        });

        Align::new(hoverable.finish()).finish()
    }

    fn render_tab_bar_icon_button_tooltip(
        &self,
        appearance: &Appearance,
        tool_tip_label_text: String,
        tool_tip_sublabel_text: Option<String>,
    ) -> Box<dyn FnOnce() -> Box<dyn Element>> {
        let ui_builder = appearance.ui_builder().clone();

        Box::new(move || {
            if let Some(tool_tip_sublabel_text) = tool_tip_sublabel_text {
                ui_builder
                    .tool_tip_with_sublabel(tool_tip_label_text, tool_tip_sublabel_text)
                    .build()
                    .finish()
            } else {
                ui_builder.tool_tip(tool_tip_label_text).build().finish()
            }
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn render_tab_bar_icon_button(
        &self,
        appearance: &Appearance,
        icon_type: icons::Icon,
        mouse_state_handle: &MouseStateHandle,
        action: WorkspaceAction,
        tool_tip_label_text: String,
        tool_tip_sublabel_text: Option<String>,
        is_active: bool,
        disable: bool,
    ) -> Hoverable {
        let theme = appearance.theme();
        let icon_color = if is_active {
            theme.main_text_color(theme.background())
        } else {
            theme.sub_text_color(theme.background())
        };
        let mut button = icon_button_with_color(
            appearance,
            icon_type,
            is_active,
            mouse_state_handle.clone(),
            icon_color,
        );
        button = button
            .with_hovered_styles(UiComponentStyles {
                font_color: Some(icon_color.into()),
                background: Some(theme.surface_2().into()),
                ..UiComponentStyles::default()
            })
            .with_clicked_styles(UiComponentStyles {
                font_color: Some(icon_color.into()),
                background: Some(theme.background().into()),
                ..UiComponentStyles::default()
            });

        if is_active {
            button = button.with_active_styles(UiComponentStyles {
                background: Some(internal_colors::fg_overlay_3(theme).into()),
                ..UiComponentStyles::default()
            });
        }

        if disable {
            button = button.with_style(UiComponentStyles {
                font_color: Some(theme.disabled_text_color(theme.background()).into()),
                ..UiComponentStyles::default()
            });
            button.build().disable()
        } else {
            button
                .with_tooltip(self.render_tab_bar_icon_button_tooltip(
                    appearance,
                    tool_tip_label_text,
                    tool_tip_sublabel_text,
                ))
                .build()
                .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
        }
    }

    fn render_tab_overflow_menu(
        &self,
        _app: &AppContext,
        _appearance: &Appearance,
    ) -> Option<Box<dyn Element>> {
        None
    }

    fn render_banner_and_active_tab(
        &self,
        app: &AppContext,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let active_tab_data = &self.tabs[self.active_tab_index];

        let active_content = ChildView::new(&active_tab_data.pane_group).finish();

        let terminal_content = match self.maybe_render_workspace_banner(app, appearance) {
            Some(banner_element) => Flex::column()
                .with_child(banner_element)
                .with_child(Shrinkable::new(1., active_content).finish())
                .finish(),
            None => active_content,
        };

        let vertical_tabs_active =
            FeatureFlag::VerticalTabs.is_enabled() && *TabSettings::as_ref(app).use_vertical_tabs;
        let pane_group = self.active_tab_pane_group().as_ref(app);
        let is_right_open = pane_group.right_panel_open;
        let is_right_maximized = is_right_open && pane_group.is_right_panel_maximized;

        let mut main_content = Flex::row();

        // In horizontal tabs mode, config-driven panels render inside this row
        // so they share the same background/corner-radius wrapper from render_main_panel.
        // In vertical tabs mode, panels are rendered in render_panels instead.
        if !vertical_tabs_active {
            let config = TabSettings::as_ref(app)
                .header_toolbar_chip_selection
                .clone();
            let mut prev_panel_added = false;
            for item in config.left_items() {
                Self::add_panel_with_separator(
                    &mut main_content,
                    &mut prev_panel_added,
                    self.render_config_panel(&item, pane_group, &config, app),
                    app,
                );
            }

            if !is_right_maximized {
                if prev_panel_added {
                    main_content.add_child(Self::render_panel_separator(app));
                }
                main_content =
                    main_content.with_child(Shrinkable::new(1.0, terminal_content).finish());
                prev_panel_added = true;
            }

            for item in config.right_items() {
                Self::add_panel_with_separator(
                    &mut main_content,
                    &mut prev_panel_added,
                    self.render_config_panel(&item, pane_group, &config, app),
                    app,
                );
            }

            if is_right_maximized {
                Self::add_panel_with_separator(
                    &mut main_content,
                    &mut prev_panel_added,
                    self.render_config_panel_maximized(pane_group, &config, app),
                    app,
                );
            }
        } else if !is_right_maximized {
            main_content = main_content.with_child(Shrinkable::new(1.0, terminal_content).finish());
        }

        let clickable_element = EventHandler::new(main_content.finish())
            .on_back_mouse_down(|ctx, _app, _position| {
                ctx.dispatch_typed_action(WorkspaceAction::ActivatePrevTab);
                DispatchEventResult::StopPropagation
            })
            .on_forward_mouse_down(|ctx, _app, _position| {
                ctx.dispatch_typed_action(WorkspaceAction::ActivateNextTab);
                DispatchEventResult::StopPropagation
            })
            .finish();

        Shrinkable::new(
            THEME_CHOOSER_RATIO,
            SavePosition::new(clickable_element, TAB_CONTENT_POSITION_ID).finish(),
        )
        .finish()
    }

    fn render_theme_chooser(&self) -> Box<dyn Element> {
        let theme_chooser = ChildView::new(&self.theme_chooser_view).finish();
        ConstrainedBox::new(theme_chooser)
            .with_max_width(240.0)
            .finish()
    }

    #[cfg(not(target_family = "wasm"))]
    fn render_resource_center(&self) -> Box<dyn Element> {
        ConstrainedBox::new(ChildView::new(&self.resource_center_view).finish())
            .with_width(RESOURCE_CENTER_WIDTH)
            .finish()
    }

    // Allow let and return because of the conditional linux compilation (otherwise we get a clippy
    // warning on mac)
    #[allow(clippy::let_and_return)]
    fn banner_fields(&self, app: &AppContext) -> Option<WorkspaceBannerFields> {
        let banner_fields = self.render_settings_error_banner(app);

        #[cfg(enable_crash_recovery)]
        let banner_fields = banner_fields.or_else(|| crash_recovery::banner_metadata(app));

        banner_fields
    }

    fn render_settings_error_banner(&self, app: &AppContext) -> Option<WorkspaceBannerFields> {
        if self.settings_error_banner_dismissed {
            return None;
        }
        let error = self.settings_file_error.as_ref()?;
        let (heading, description) = error.heading_and_description();
        let secondary_button =
            AISettings::as_ref(app)
                .is_any_ai_enabled(app)
                .then(|| WorkspaceBannerButtonDetails {
                    text: "Fix with AI".to_owned(),
                    action: WorkspaceAction::FixSettingsWithAgent {
                        error_description: error.to_string(),
                    },
                    variant: BannerButtonVariant::Naked,
                    icon: None,
                    more_info_button_action: None,
                });
        Some(WorkspaceBannerFields {
            banner_type: WorkspaceBanner::InvalidSettings,
            severity: BannerSeverity::Warning,
            heading: Some(heading),
            description,
            secondary_button,
            button: Some(WorkspaceBannerButtonDetails {
                text: "Open file".to_owned(),
                action: WorkspaceAction::OpenSettingsFile,
                variant: BannerButtonVariant::Outlined,
                icon: None,
                more_info_button_action: None,
            }),
        })
    }

    fn maybe_render_workspace_banner(
        &self,
        app: &AppContext,
        appearance: &Appearance,
    ) -> Option<Box<dyn Element>> {
        self.banner_fields(app)
            .map(|fields| self.render_workspace_banner(fields, appearance))
    }

    fn render_workspace_banner(
        &self,
        fields: WorkspaceBannerFields,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let bg_color = match fields.severity {
            BannerSeverity::Warning => theme.ansi_fg_yellow(),
            BannerSeverity::Error => theme.ansi_fg_red(),
        };
        let text_color = theme.main_text_color(Fill::Solid(bg_color)).into_solid();

        // Left side: alert icon + bold heading + regular description, all inline.
        let icon =
            ConstrainedBox::new(Icon::AlertCircle.to_warpui_icon(text_color.into()).finish())
                .with_width(16.)
                .with_height(16.)
                .finish();

        let ui_font_family = appearance.ui_font_family();
        const BANNER_FONT_SIZE: f32 = 12.;

        // Combine heading and description into a single `Text` so it can
        // elide with a trailing ellipsis when there isn't enough room for the
        // buttons. The heading portion is highlighted with Semibold weight.
        // See `ConversationSearchItem::render_item` for the same pattern.
        let heading_char_count = fields
            .heading
            .as_ref()
            .map(|heading| heading.chars().count())
            .unwrap_or(0);
        let combined_text = match fields.heading {
            Some(heading) => format!("{heading} {}", fields.description),
            None => fields.description,
        };
        let mut text = Text::new_inline(combined_text, ui_font_family, BANNER_FONT_SIZE)
            .with_color(text_color)
            .with_clip(ClipConfig::ellipsis());
        if heading_char_count > 0 {
            text = text.with_single_highlight(
                Highlight::new().with_properties(Properties::default().weight(Weight::Semibold)),
                (0..heading_char_count).collect(),
            );
        }

        let mut banner = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(Container::new(icon).with_margin_right(8.).finish())
            // `Expanded` (not `Shrinkable`) so the text fills the remaining
            // row width and pushes the action buttons to the right even when
            // the text is short. Truncation still applies when the text would
            // otherwise overflow.
            .with_child(Expanded::new(1., text.finish()).finish());

        if let Some(secondary_button) = fields.secondary_button {
            banner.add_child(
                Container::new(self.render_banner_action_button(
                    secondary_button,
                    self.mouse_states.banner_secondary_button.clone(),
                    text_color,
                    appearance,
                ))
                .with_margin_left(4.)
                .finish(),
            );
        }

        if let Some(button) = fields.button {
            let more_info_button_action = button.more_info_button_action.clone();
            banner.add_child(
                Container::new(self.render_banner_action_button(
                    button,
                    self.mouse_states.banner_button.clone(),
                    text_color,
                    appearance,
                ))
                .with_margin_left(4.)
                .finish(),
            );

            if let Some(more_info_button_action) = more_info_button_action {
                let more_info_details = WorkspaceBannerButtonDetails {
                    text: "More info".to_owned(),
                    action: more_info_button_action,
                    variant: BannerButtonVariant::Outlined,
                    icon: None,
                    more_info_button_action: None,
                };
                banner.add_child(
                    Container::new(self.render_banner_action_button(
                        more_info_details,
                        self.mouse_states.more_info_banner_button.clone(),
                        text_color,
                        appearance,
                    ))
                    .with_margin_left(4.)
                    .finish(),
                );
            }
        }

        if fields.banner_type.is_dismissible() {
            let dismiss_target = fields.banner_type;
            banner.add_child(
                Container::new(
                    Hoverable::new(
                        self.mouse_states.dismiss_banner_button.clone(),
                        move |state| {
                            let mut container = Container::new(
                                ConstrainedBox::new(
                                    // Plain x-close glyph (`Icon::X` →
                                    // `x-close.svg`), matching the Figma
                                    // design. `Icon::XCircle` wraps the x in
                                    // a circle which is not what we want.
                                    Icon::X.to_warpui_icon(text_color.into()).finish(),
                                )
                                .with_width(16.)
                                .with_height(16.)
                                .finish(),
                            )
                            .with_uniform_padding(2.)
                            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)));
                            if state.is_hovered() {
                                container = container
                                    .with_background_color(coloru_with_opacity(text_color, 20));
                            }
                            container.finish()
                        },
                    )
                    .with_cursor(Cursor::PointingHand)
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(WorkspaceAction::DismissWorkspaceBanner(
                            dismiss_target,
                        ));
                    })
                    .finish(),
                )
                .with_margin_left(4.)
                .finish(),
            );
        }

        ConstrainedBox::new(
            Container::new(banner.finish())
                .with_background_color(bg_color)
                .with_uniform_padding(8.)
                .finish(),
        )
        .finish()
    }

    /// Renders a single banner action button using the Figma-spec'd Naked or
    /// Secondary variants: no fill by default, optional 1px border, text and
    /// icon tinted with the banner's contrast-safe text color.
    fn render_banner_action_button(
        &self,
        details: WorkspaceBannerButtonDetails,
        mouse_state: MouseStateHandle,
        text_color: ColorU,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let WorkspaceBannerButtonDetails {
            text,
            action,
            variant,
            icon,
            ..
        } = details;
        let ui_font_family = appearance.ui_font_family();
        Hoverable::new(mouse_state, move |state| {
            let mut row = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_main_axis_alignment(MainAxisAlignment::Center)
                .with_main_axis_size(MainAxisSize::Min);
            if let Some(icon) = icon {
                row.add_child(
                    Container::new(
                        ConstrainedBox::new(icon.to_warpui_icon(text_color.into()).finish())
                            .with_width(14.)
                            .with_height(14.)
                            .finish(),
                    )
                    .with_margin_right(4.)
                    .finish(),
                );
            }
            row.add_child(
                Text::new_inline(text.clone(), ui_font_family, 12.)
                    .with_color(text_color)
                    .with_style(Properties {
                        weight: Weight::Semibold,
                        ..Default::default()
                    })
                    .finish(),
            );

            let mut container = Container::new(row.finish())
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                .with_horizontal_padding(8.);
            if matches!(variant, BannerButtonVariant::Outlined) {
                container = container.with_border(Border::all(1.).with_border_color(text_color));
            }
            if state.is_hovered() {
                container = container.with_background_color(coloru_with_opacity(text_color, 20));
            }

            ConstrainedBox::new(container.finish())
                .with_height(24.)
                .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
        .finish()
    }

    fn dismiss_workspace_banner(
        &mut self,
        ctx: &mut ViewContext<Self>,
        banner_type: &WorkspaceBanner,
    ) {
        match banner_type {
            #[cfg(all(enable_crash_recovery, target_os = "linux"))]
            WorkspaceBanner::WaylandCrashRecovery => {
                crash_recovery::dismiss_workspace_banner(ctx);
            }
            WorkspaceBanner::InvalidSettings => {
                self.settings_error_banner_dismissed = true;
                self.sync_settings_error_state_into_settings_pane(ctx);
            }
        }
        ctx.notify();
    }

    fn render_panel(
        &self,
        app: &AppContext,
        contents: Box<dyn Element>,
        side: &PanelPosition,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);

        let mut col = Flex::column().with_main_axis_size(MainAxisSize::Max);
        let mut contents = contents;

        let traffic_light_data = traffic_light_data(app, self.window_id);
        let vertical_tabs_active =
            FeatureFlag::VerticalTabs.is_enabled() && *TabSettings::as_ref(app).use_vertical_tabs;
        // Add a spacer for the traffic light buttons on Windows/Linux.
        if traffic_light_data.is_some_and(|data| data.side == TrafficLightSide::Right)
            && *side == PanelPosition::Right
            && !vertical_tabs_active
        {
            col.add_child(
                ConstrainedBox::new(Empty::new().finish())
                    .with_height(TAB_BAR_HEIGHT)
                    .finish(),
            );
            contents = Container::new(contents)
                .with_border(Border::top(1.).with_border_fill(appearance.theme().surface_2()))
                .finish();
        }
        col.add_child(Shrinkable::new(1.0, contents).finish());

        self.wrap_in_panel_surface(appearance, side, col.finish(), *PANEL_CORNER_RADIUS)
    }

    fn wrap_in_panel_surface(
        &self,
        appearance: &Appearance,
        side: &PanelPosition,
        contents: Box<dyn Element>,
        corner_radius: CornerRadius,
    ) -> Box<dyn Element> {
        let mut container = Container::new(contents)
            .with_background(appearance.theme().surface_1().with_opacity(90))
            .with_corner_radius(corner_radius);

        match side {
            PanelPosition::Left => container = container.with_margin_right(2.0),
            PanelPosition::Right => container = container.with_margin_left(2.0),
        };

        container.finish()
    }

    fn render_main_panel(
        &self,
        app: &AppContext,
        terminal_view: Box<dyn Element>,
    ) -> Box<dyn Element> {
        if FeatureFlag::VerticalTabs.is_enabled() && *TabSettings::as_ref(app).use_vertical_tabs {
            Shrinkable::new(1.0, terminal_view).finish()
        } else {
            let main_content = Container::new(terminal_view)
                .with_background(util::get_terminal_background_fill(self.window_id, app))
                .with_corner_radius(*PANEL_CORNER_RADIUS)
                .finish();

            Shrinkable::new(1.0, main_content).finish()
        }
    }

    fn render_panel_separator(app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        ConstrainedBox::new(
            Rect::new()
                .with_background_color(appearance.theme().outline().into_solid())
                .finish(),
        )
        .with_width(1.0)
        .finish()
    }

    fn add_panel_with_separator(
        panels_view: &mut Flex,
        prev_panel_added: &mut bool,
        panel: Option<Box<dyn Element>>,
        app: &AppContext,
    ) {
        if let Some(panel) = panel {
            if *prev_panel_added {
                panels_view.add_child(Self::render_panel_separator(app));
            }
            panels_view.add_child(panel);
            *prev_panel_added = true;
        }
    }

    fn render_panels(
        &self,
        app: &AppContext,
        terminal_view: Box<dyn Element>,
        hide_vertical_tabs: bool,
    ) -> Box<dyn Element> {
        let mut panels_view = Flex::row();
        let mut prev_panel_added = false;

        // Config-driven vertical-tabs-era panels (left side).
        // Hidden for simplified WASM views (notebooks, transcripts, etc.)
        // where these panels are unnecessary.
        let vertical_tabs_active = !hide_vertical_tabs
            && FeatureFlag::VerticalTabs.is_enabled()
            && *TabSettings::as_ref(app).use_vertical_tabs;

        // In vertical tabs mode, config-driven panels are rendered here.
        // In horizontal tabs mode, they're rendered inside render_banner_and_active_tab
        // so they share the same background/corner-radius wrapper.
        if vertical_tabs_active {
            let config = TabSettings::as_ref(app)
                .header_toolbar_chip_selection
                .clone();
            let pane_group = self.active_tab_pane_group().as_ref(app);

            for item in config.left_items() {
                Self::add_panel_with_separator(
                    &mut panels_view,
                    &mut prev_panel_added,
                    self.render_config_panel(&item, pane_group, &config, app),
                    app,
                );
            }
        }

        // Theme chooser (workspace-level, not configurable).
        // Uses wrap_in_panel_surface which adds margin for its own visual separation,
        // so we add a separator before it only if a config panel is to its left, then
        // reset the flag so no separator is added between the theme chooser and the terminal.
        if self.current_workspace_state.is_theme_chooser_open {
            if prev_panel_added {
                panels_view.add_child(Self::render_panel_separator(app));
            }
            panels_view.add_child(self.render_panel(
                app,
                self.render_theme_chooser(),
                &PanelPosition::Left,
            ));
            prev_panel_added = false;
        }

        if prev_panel_added {
            panels_view.add_child(Self::render_panel_separator(app));
        }
        panels_view = panels_view.with_child(self.render_main_panel(app, terminal_view));
        prev_panel_added = true;

        if vertical_tabs_active {
            let config = TabSettings::as_ref(app)
                .header_toolbar_chip_selection
                .clone();
            let pane_group = self.active_tab_pane_group().as_ref(app);

            for item in config.right_items() {
                Self::add_panel_with_separator(
                    &mut panels_view,
                    &mut prev_panel_added,
                    self.render_config_panel(&item, pane_group, &config, app),
                    app,
                );
            }

            if pane_group.right_panel_open && pane_group.is_right_panel_maximized {
                Self::add_panel_with_separator(
                    &mut panels_view,
                    &mut prev_panel_added,
                    self.render_config_panel_maximized(pane_group, &config, app),
                    app,
                );
            }
        }

        // Resource center is a workspace-level panel, not configurable.
        #[cfg(not(target_family = "wasm"))]
        if self.current_workspace_state.is_right_panel_open() {
            let right_panel_content = if self.current_workspace_state.is_resource_center_open {
                Some(self.render_panel(app, self.render_resource_center(), &PanelPosition::Right))
            } else {
                log::warn!(
                    "is_right_panel_open() returned true, but the resource center is not open"
                );
                None
            };

            if let Some(right_panel_content) = right_panel_content {
                panels_view = panels_view.with_child(right_panel_content);
            }
        }

        panels_view.finish()
    }

    fn tabs_panel_side(config: &HeaderToolbarChipSelection) -> PanelPosition {
        if config
            .left_items()
            .contains(&HeaderToolbarItemKind::TabsPanel)
        {
            PanelPosition::Left
        } else {
            PanelPosition::Right
        }
    }

    /// Renders a configurable panel for the given toolbar item, if it is open.
    /// Returns `None` if the panel should not be rendered (item not available,
    /// panel not open, or item is not a panel type).
    fn render_config_panel(
        &self,
        item: &HeaderToolbarItemKind,
        pane_group: &PaneGroup,
        config: &HeaderToolbarChipSelection,
        app: &AppContext,
    ) -> Option<Box<dyn Element>> {
        if !item.is_available(app) || !item.is_panel() {
            return None;
        }
        match item {
            HeaderToolbarItemKind::TabsPanel => {
                if !self.vertical_tabs_panel_open {
                    return None;
                }
                Some(
                    SavePosition::new(
                        self.render_vertical_tabs_panel(Self::tabs_panel_side(config), app),
                        VERTICAL_TABS_PANEL_POSITION_ID,
                    )
                    .finish(),
                )
            }
            HeaderToolbarItemKind::ToolsPanel => {
                if !pane_group.left_panel_open || warpui::platform::is_mobile_device() {
                    return None;
                }
                Some(ChildView::new(&self.left_panel_view).finish())
            }
            HeaderToolbarItemKind::CodeReview => {
                if !pane_group.right_panel_open {
                    return None;
                }
                if pane_group.is_right_panel_maximized {
                    return None;
                }
                Some(ChildView::new(&self.right_panel_view).finish())
            }
        }
    }

    /// Renders the maximized code review panel if it is configured and maximized.
    fn render_config_panel_maximized(
        &self,
        pane_group: &PaneGroup,
        _config: &HeaderToolbarChipSelection,
        app: &AppContext,
    ) -> Option<Box<dyn Element>> {
        if !pane_group.right_panel_open || !pane_group.is_right_panel_maximized {
            return None;
        }
        if !HeaderToolbarItemKind::CodeReview.is_available(app) {
            return None;
        }
        Some(Shrinkable::new(1.0, ChildView::new(&self.right_panel_view).finish()).finish())
    }

    /// Offset positioning for global toasts.
    // TODO: update positioning based on input mode.
    fn global_toast_positioning(&self) -> OffsetPositioning {
        OffsetPositioning::offset_from_save_position_element(
            TAB_CONTENT_POSITION_ID,
            vec2f(0., 16.),
            PositionedElementOffsetBounds::WindowByPosition,
            PositionedElementAnchor::TopMiddle,
            ChildAnchor::TopMiddle,
        )
    }

    /// Offset positioning for the update toast.
    fn update_toast_positioning(
        &self,
        input_position_id: String,
        app: &AppContext,
    ) -> OffsetPositioning {
        let input_mode = InputModeSettings::as_ref(app).input_mode.value();

        match input_mode {
            InputMode::PinnedToBottom => OffsetPositioning::offset_from_save_position_element(
                input_position_id,
                vec2f(-16., -16.),
                PositionedElementOffsetBounds::WindowByPosition,
                PositionedElementAnchor::TopRight,
                ChildAnchor::BottomRight,
            ),
            InputMode::PinnedToTop => OffsetPositioning::offset_from_save_position_element(
                input_position_id,
                vec2f(-16., 16.),
                PositionedElementOffsetBounds::WindowByPosition,
                PositionedElementAnchor::BottomRight,
                ChildAnchor::TopRight,
            ),
            InputMode::Waterfall => OffsetPositioning::offset_from_parent(
                vec2f(-16., -16.),
                ParentOffsetBounds::WindowByPosition,
                ParentAnchor::BottomRight,
                ChildAnchor::BottomRight,
            ),
        }
    }

    fn add_toggle_setting_context_flags(&self, app: &AppContext, context: &mut Context) {
        let editor_settings = AppEditorSettings::as_ref(app);
        let semantic_selection_settings = SemanticSelection::as_ref(app);
        let selection_settings = SelectionSettings::as_ref(app);
        let session_settings = SessionSettings::as_ref(app);
        let safe_mode_settings: &SafeModeSettings = SafeModeSettings::as_ref(app);
        let block_list_settings = BlockListSettings::as_ref(app);
        let tab_settings = TabSettings::as_ref(app);
        let alias_expansion_settings = AliasExpansionSettings::as_ref(app);
        let code_settings = CodeSettings::as_ref(app);
        let input_settings = InputSettings::as_ref(app);
        let reporting_setings = AltScreenReporting::as_ref(app);
        let general_settings = GeneralSettings::as_ref(app);
        let theme_settings = ThemeSettings::as_ref(app);
        let ssh_settings = SshSettings::as_ref(app);
        let terminal_settings = TerminalSettings::as_ref(app);
        let pane_settings = PaneSettings::as_ref(app);
        let keys_settings = KeysSettings::as_ref(app);

        let is_compact_mode =
            matches!(terminal_settings.spacing_mode.value(), SpacingMode::Compact);
        if is_compact_mode {
            context.set.insert(flags::COMPACT_MODE_CONTEXT_FLAG);
        }

        let respect_system_theme = respect_system_theme(theme_settings);
        if matches!(respect_system_theme, RespectSystemTheme::On(_)) {
            context.set.insert(flags::RESPECT_SYSTEM_THEME_CONTEXT_FLAG);
        }

        if SelectionSettings::as_ref(app).copy_on_select_enabled() {
            context.set.insert(flags::COPY_ON_SELECT_CONTEXT_FLAG);
        }

        if SelectionSettings::as_ref(app).linux_selection_clipboard_enabled() {
            context.set.insert(flags::LINUX_SELECTION_CLIPBOARD_FLAG);
        }

        if *editor_settings.autocomplete_symbols {
            context.set.insert(flags::AUTOCOMPLETE_SYMBOLS_CONTEXT_FLAG);
        }

        if *general_settings.restore_session {
            context.set.insert(flags::RESTORE_SESSION_CONTEXT_FLAG);
        }

        if *session_settings.honor_ps1 {
            context.set.insert(flags::HONOR_PS1_CONTEXT_FLAG);
        }

        if session_settings
            .saved_prompt
            .value()
            .same_line_prompt_enabled()
        {
            context.set.insert(flags::WARP_SAME_LINE_PROMPT_FLAG);
        }

        if *ssh_settings.enable_legacy_ssh_wrapper.value() {
            #[allow(deprecated)]
            context.set.insert(flags::LEGACY_SSH_WRAPPER_CONTEXT_FLAG);
        }

        if keys_settings.extra_meta_keys.left_alt {
            context.set.insert(flags::EXTRA_META_KEYS_LEFT_CONTEXT_FLAG);
        }

        if keys_settings.extra_meta_keys.right_alt {
            context
                .set
                .insert(flags::EXTRA_META_KEYS_RIGHT_CONTEXT_FLAG);
        }

        if *reporting_setings.scroll_reporting_enabled.value() {
            context.set.insert(flags::SCROLL_REPORTING_CONTEXT_FLAG);
        }

        if *reporting_setings.focus_reporting_enabled.value() {
            context.set.insert(flags::FOCUS_REPORTING_CONTEXT_FLAG);
        }

        if *KeysSettings::as_ref(app).quake_mode_enabled {
            context.set.insert(flags::QUAKE_MODE_ENABLED_CONTEXT_FLAG);
        }

        if matches!(
            SessionSettings::as_ref(app).notifications.mode,
            NotificationsMode::Enabled
        ) {
            context.set.insert(flags::NOTIFICATIONS_CONTEXT_FLAG);
        }

        if *general_settings.link_tooltip {
            context.set.insert(flags::LINK_TOOLTIP_CONTEXT_FLAG);
        }

        if *input_settings.completions_open_while_typing.value() {
            context
                .set
                .insert(flags::COMPLETIONS_OPEN_WHILE_TYPING_CONTEXT_FLAG);
        }

        if *input_settings.command_corrections.value() {
            context.set.insert(flags::COMMAND_CORRECTIONS_CONTEXT_FLAG);
        }

        if *input_settings.error_underlining.value() {
            context.set.insert(flags::ERROR_UNDERLINING_FLAG);
        }

        if *input_settings.syntax_highlighting.value() {
            context.set.insert(flags::SYNTAX_HIGHLIGHTING_FLAG);
        }

        if *block_list_settings
            .show_jump_to_bottom_of_block_button
            .value()
        {
            context
                .set
                .insert(flags::JUMP_TO_BOTTOM_OF_BLOCK_BUTTON_CONTEXT_FLAG);
        }

        if *block_list_settings.show_block_dividers.value() {
            context.set.insert(flags::BLOCK_DIVIDERS_CONTEXT_FLAG);
        }

        if *safe_mode_settings.safe_mode_enabled.value() {
            context.set.insert(flags::SAFE_MODE_FLAG);
        }

        if editor_settings.cursor_blink.value() == &CursorBlink::Enabled {
            context.set.insert(flags::CURSOR_BLINK_CONTEXT_FLAG);
        }

        if *editor_settings.vim_mode.value() {
            context.set.insert(flags::VIM_MODE_CONTEXT_FLAG);
            if *editor_settings.vim_unnamed_system_clipboard.value() {
                context.set.insert(flags::VIM_UNNAMED_SYSTEM_CLIPBOARD);
            }
            if *editor_settings.vim_status_bar.value() {
                context.set.insert(flags::VIM_SHOW_STATUS_BAR);
            }
        }

        if *pane_settings.should_dim_inactive_panes {
            context.set.insert(flags::DIM_INACTIVE_PANES_FLAG);
        }

        if *pane_settings.focus_panes_on_hover {
            context.set.insert(flags::FOCUS_PANES_ON_HOVER_CONTEXT_FLAG);
        }

        if *general_settings.show_warning_before_quitting.value() {
            context.set.insert(flags::QUIT_WARNING_MODAL);
        }

        if semantic_selection_settings.smart_select_enabled() {
            context.set.insert(flags::SMART_SELECT_FLAG);
        }

        if *KeysSettings::as_ref(app).activation_hotkey_enabled.value() {
            context.set.insert(flags::ACTIVATION_HOTKEY_FLAG);
        }

        if *tab_settings.show_indicators.value() {
            context.set.insert(flags::TAB_INDICATORS_FLAG);
        }
        if *tab_settings.show_code_review_button.value() {
            context.set.insert(flags::SHOW_CODE_REVIEW_BUTTON_FLAG);
        }
        if *tab_settings.use_vertical_tabs.value() {
            context.set.insert(flags::USE_VERTICAL_TABS_FLAG);
        }
        if self.should_show_session_config_tab_config_chip() {
            context
                .set
                .insert(flags::SESSION_CONFIG_TAB_CONFIG_CHIP_OPEN);
        }

        if tab_settings
            .workspace_decoration_visibility
            .value()
            .hides_decorations_by_default()
        {
            context
                .set
                .insert(flags::HIDE_WORKSPACE_DECORATIONS_CONTEXT_FLAG);
        }

        if *alias_expansion_settings.alias_expansion_enabled.value() {
            context.set.insert(flags::ALIAS_EXPANSION_FLAG);
        }

        if *selection_settings.middle_click_paste_enabled.value() {
            context.set.insert(flags::MIDDLE_CLICK_PASTE_FLAG);
        }

        if *code_settings.code_as_default_editor.value() {
            context.set.insert(flags::CODE_AS_DEFAULT_EDITOR);
        }

        if *code_settings.codebase_context_enabled.value() {
            context.set.insert(flags::IS_CODEBASE_INDEXING_ENABLED);
        }

        if *code_settings.auto_indexing_enabled.value() {
            context.set.insert(flags::IS_AUTOINDEXING_ENABLED);
        }

        if *input_settings.show_hint_text.value() {
            context.set.insert(flags::SHOW_INPUT_HINT_TEXT_CONTEXT_FLAG);
        }

        if *input_settings.show_agent_tips.value() {
            context.set.insert(flags::SHOW_AGENT_TIPS_FLAG);
        }
        if *editor_settings.enable_autosuggestions {
            context.set.insert(flags::AUTOSUGGESTIONS_ENABLED_FLAG);
        }

        if *editor_settings.autosuggestion_keybinding_hint.value() {
            context
                .set
                .insert(flags::AUTOSUGGESTION_KEYBINDING_HINT_FLAG);
        }

        #[cfg(target_os = "linux")]
        {
            let force_x11 = *crate::settings::LinuxAppConfiguration::as_ref(app)
                .force_x11
                .value();

            if !force_x11 {
                context.set.insert(flags::ALLOW_NATIVE_WAYLAND);
            }
        }

        let terminal_settings = TerminalSettings::as_ref(app);
        if *terminal_settings.use_audible_bell {
            context.set.insert(flags::USE_AUDIBLE_BELL_CONTEXT_FLAG);
        }

        let gpu_settings = GPUSettings::as_ref(app);
        if *gpu_settings.prefer_low_power_gpu {
            context.set.insert(flags::PREFER_LOW_POWER_GPU_FLAG);
        }

        let ai_settings = AISettings::as_ref(app);
        if ai_settings.is_ai_autodetection_enabled(app) {
            context.set.insert(flags::AI_INPUT_AUTODETECTION_FLAG);
        }
        if ai_settings.is_nld_in_terminal_enabled(app) {
            context.set.insert(flags::NLD_IN_TERMINAL_FLAG);
        }
        if ai_settings.is_intelligent_autosuggestions_enabled(app) {
            context.set.insert(flags::INTELLIGENT_AUTOSUGGESTIONS_FLAG);
        }
        if ai_settings.is_prompt_suggestions_enabled(app) {
            context.set.insert(flags::PROMPT_SUGGESTIONS_FLAG);
        }
        if ai_settings.is_code_suggestions_enabled(app) {
            context.set.insert(flags::CODE_SUGGESTIONS_FLAG);
        }
        if ai_settings.is_natural_language_autosuggestions_enabled(app) {
            context
                .set
                .insert(flags::NATURAL_LANGUAGE_AUTOSUGGESTIONS_FLAG);
        }

        if ai_settings.is_shared_block_title_generation_enabled(app) {
            context
                .set
                .insert(flags::SHARED_BLOCK_TITLE_GENERATION_FLAG);
        }

        if *ai_settings
            .should_render_use_agent_footer_for_user_commands
            .value()
        {
            context.set.insert(flags::USE_AGENT_FOOTER_FLAG);
        }

        match ai_settings.thinking_display_mode {
            crate::settings::ThinkingDisplayMode::ShowAndCollapse => {
                context
                    .set
                    .insert(flags::THINKING_DISPLAY_SHOW_AND_COLLAPSE);
            }
            crate::settings::ThinkingDisplayMode::AlwaysShow => {
                context.set.insert(flags::THINKING_DISPLAY_ALWAYS_SHOW);
            }
            crate::settings::ThinkingDisplayMode::NeverShow => {
                context.set.insert(flags::THINKING_DISPLAY_NEVER_SHOW);
            }
        }

        if input_settings.is_terminal_input_message_bar_enabled() {
            context
                .set
                .insert(flags::SHOW_TERMINAL_INPUT_MESSAGE_LINE_FLAG);
        }

        if *input_settings.enable_slash_commands_in_terminal.value() {
            context.set.insert(flags::SLASH_COMMANDS_IN_TERMINAL_FLAG);
        }

        if ChannelState::enable_debug_features() {
            let block_visibility_settings = BlockVisibilitySettings::as_ref(app);
            if *block_visibility_settings
                .should_show_bootstrap_block
                .value()
            {
                context.set.insert(flags::INITIALIZATION_BLOCK_FLAG);
            }
            if *block_visibility_settings
                .should_show_in_band_command_blocks
                .value()
            {
                context.set.insert(flags::IN_BAND_COMMAND_BLOCKS_FLAG);
            }
        }

        if should_use_ligature_rendering(app) {
            context.set.insert(flags::LIGATURE_RENDERING_CONTEXT_FLAG);
        }
    }

    /// Send SyncEvent to all synced pane groups.
    fn process_sync_event_for_all_synced_pane_groups(
        &mut self,
        event: &SyncEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        for tab in self.tab_views() {
            // We have to get the latest SyncInputStatus each iteration because
            // tab.update below could potentially change it.
            let synced_pane_group_ids = SyncedInputState::as_ref(ctx);

            if synced_pane_group_ids.should_sync_this_pane_group(tab.id(), ctx.window_id()) {
                tab.update(ctx, |pane_group, ctx| {
                    pane_group.send_sync_event_to_panes(event, ctx);
                });
            }
        }

        self.update_pane_dimming_for_current_focus_region(ctx);
    }

    /// Sends SyncEvent to all synced terminal views.
    /// The purpose of the event could be match the active terminal input,
    /// expand the terminal input box, or collapse the terminal input box.
    fn process_updated_sync_state(&self, ctx: &mut ViewContext<Self>) {
        // If there is an active terminal, return a sync event that all
        // other synced terminals should apply to match it.
        // If there is no active terminal (like when all Warp windows are
        // minimized), return an event to start syncing.
        let sync_event = self
            .active_tab_pane_group()
            .as_ref(ctx)
            .active_session_view(ctx)
            .map_or(
                SyncEvent {
                    source_view_id: ctx.view_id(),
                    data: SyncInputType::StartSyncing,
                },
                |terminal_view_handle| {
                    terminal_view_handle
                        .as_ref(ctx)
                        .create_sync_event_based_on_terminal_state(ctx)
                },
            );

        let stop_syncing_event = SyncEvent {
            source_view_id: ctx.view_id(),
            data: terminal::view::SyncInputType::StopSyncing,
        };

        for tab in self.tab_views() {
            // We have to get the latest SyncInputStatus each iteration because
            // tab.update below could potentially change it.
            let synced_pane_group_ids = SyncedInputState::as_ref(ctx);

            if synced_pane_group_ids.should_sync_this_pane_group(tab.id(), ctx.window_id()) {
                tab.update(ctx, |pane_group, pane_group_ctx| {
                    pane_group.send_sync_event_to_panes(&sync_event, pane_group_ctx);
                });
            } else {
                // Note: we're sending StopSyncing to tabs that could already
                // know they're not syncing. We can optimize this later.
                tab.update(ctx, |pane_group, pane_group_ctx| {
                    pane_group.send_sync_event_to_panes(&stop_syncing_event, pane_group_ctx);
                });
            }
        }

        // Update tab indicators based on the new sync state.
        ctx.notify();
    }

    fn all_pane_group_ids(&self) -> impl Iterator<Item = EntityId> + '_ {
        self.tab_views().map(|tab| tab.id())
    }

    fn open_left_panel_view(&mut self, action: &LeftPanelAction, ctx: &mut ViewContext<Self>) {
        if !self.active_tab_pane_group().as_ref(ctx).left_panel_open {
            self.toggle_left_panel(ctx);
        }

        if self.active_tab_pane_group().as_ref(ctx).left_panel_open {
            self.left_panel_view.update(ctx, |left_panel, ctx| {
                left_panel.handle_action_with_force_open(action, false, ctx);
                left_panel.focus_active_view_on_entry(ctx);
            });
        }
    }

    fn toggle_left_panel_view(
        &mut self,
        action: &LeftPanelAction,
        is_showing_target_view: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        let is_left_panel_open = self.active_tab_pane_group().as_ref(ctx).left_panel_open;

        if is_left_panel_open && is_showing_target_view {
            // If we're showing the target view for this action,
            // toggle the left panel closed.
            self.toggle_left_panel(ctx);
        } else {
            self.open_left_panel_view(action, ctx);
        }
    }

    /// Computes the list of available left panel views based on current AI settings and feature flags.
    fn compute_left_panel_views(ctx: &AppContext) -> Vec<ToolPanelView> {
        let mut views = vec![];
        if cfg!(feature = "local_fs") && *CodeSettings::as_ref(ctx).show_project_explorer.value() {
            views.push(ToolPanelView::ProjectExplorer);
        }
        if cfg!(feature = "local_fs")
            && FeatureFlag::GlobalSearch.is_enabled()
            && *CodeSettings::as_ref(ctx).show_global_search.value()
        {
            views.push(ToolPanelView::GlobalSearch {
                entry_focus: GlobalSearchEntryFocus::Results,
            });
        }
        views
    }

    /// Recomputes the available left panel views based on current AI settings and feature flags,
    /// then updates both the workspace's left_panel_views and the LeftPanelView's toolbelt buttons.
    fn update_left_panel_available_views(&mut self, ctx: &mut ViewContext<Self>) {
        let views = Self::compute_left_panel_views(ctx);
        self.left_panel_views = views.clone();
        self.left_panel_view.update(ctx, |left_panel, ctx| {
            left_panel.update_available_views(views, ctx);
        });
    }

    /// Opens a given URL in the desktop Warp app if installed, or redirects to download page.
    #[cfg(target_family = "wasm")]
    fn open_link_on_desktop(&mut self, url: &Url, ctx: &mut ViewContext<Self>) {
        use crate::settings::app_installation_detection::{
            UserAppInstallDetectionSettings, UserAppInstallStatus,
        };

        // Check if the desktop app is installed
        let is_app_installed = *UserAppInstallDetectionSettings::as_ref(ctx)
            .user_app_installation_detected
            .value()
            == UserAppInstallStatus::Detected;

        if !is_app_installed {
            // App not installed. Warper has no hosted download endpoint, so fail closed.
            log::info!("Desktop app install was not detected; skipping hosted download redirect");
            // In webapp code we cannot distinguish between
            // the localhost:9277/install_detection endpoint not running (not installed) vs
            // the browser blocking Local Network Access which results in CORS error;
            // the browser intentionally obscures the error root cause for privacy reasons.
            // Many users' browser settings will block Local Network Access so this will end up redirecting to download page,
            // even if they have the app installed.
            let toast_message =
                "Desktop app was not detected. Enable Local Network Access in your browser."
                    .to_string();
            self.toast_stack.update(ctx, |toast_stack, ctx| {
                toast_stack.add_persistent_toast(DismissibleToast::default(toast_message), ctx)
            });
            // Still try to open the url on desktop below
        }

        // Open the URL on desktop. This does nothing if the app isn't installed.
        crate::uri::web_intent_parser::open_url_on_desktop(url);
    }
}

impl Entity for Workspace {
    type Event = ();
}

impl TypedActionView for Workspace {
    type Action = WorkspaceAction;

    fn action_accessibility_contents(
        &mut self,
        action: &WorkspaceAction,
        _: &mut ViewContext<Self>,
    ) -> ActionAccessibilityContent {
        match action {
            WorkspaceAction::SetA11yVerbosityLevel(verbosity) => {
                ActionAccessibilityContent::Custom(AccessibilityContent::new_without_help(
                    format!("{verbosity:?} accessibility announcements set"),
                    WarpA11yRole::UserAction,
                ))
            }
            _ => ActionAccessibilityContent::from_debug(),
        }
    }

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        use WorkspaceAction::*;
        let window_id = ctx.window_id();

        match action {
            ActivateTab(index) => self.activate_tab(*index, ctx),
            ActivateTabByNumber(num) => self.activate_tab(num.saturating_sub(1), ctx),
            ActivatePrevTab => self.activate_prev_tab(ctx),
            OpenLaunchConfigSaveModal => self.open_launch_config_save_modal(ctx),
            ActivateNextTab => self.activate_next_tab(ctx),
            ActivateLastTab => self.activate_last_tab(ctx),
            CyclePrevSession => self.cycle_prev_session(ctx),
            CycleNextSession => self.cycle_next_session(ctx),
            MoveActiveTabLeft => self.move_tab(self.active_tab_index, TabMovement::Left, ctx),
            MoveActiveTabRight => self.move_tab(self.active_tab_index, TabMovement::Right, ctx),
            MoveTabLeft(index) => self.move_tab(*index, TabMovement::Left, ctx),
            MoveTabRight(index) => self.move_tab(*index, TabMovement::Right, ctx),
            RenameTab(index) => self.rename_tab(*index, ctx),
            ResetTabName(index) => self.clear_tab_name(*index, ctx),
            RenamePane(locator) => self.rename_pane(*locator, ctx),
            ResetPaneName(locator) => self.clear_pane_name(*locator, ctx),
            RenameActiveTab => self.rename_tab(self.active_tab_index, ctx),
            SetActiveTabName(name) => self.set_active_tab_name(name, ctx),
            ToggleTabRightClickMenu { tab_index, anchor } => {
                self.toggle_tab_right_click_menu(*tab_index, *anchor, ctx)
            }
            ToggleVerticalTabsPaneContextMenu {
                tab_index,
                target,
                position,
            } => self.toggle_vertical_tabs_pane_context_menu(*tab_index, *target, *position, ctx),
            ToggleTabBarOverflowMenu => self.toggle_tab_bar_overflow_menu(ctx),
            ToggleBlockSnackbar => self.toggle_block_snackbar(ctx),
            ToggleWelcomeTips => self.toggle_welcome_tips_visiblity(ctx),
            CloseTab(index) => self.close_tab(*index, false, true, ctx),
            CloseActiveTab => self.close_tab(self.active_tab_index, false, true, ctx),
            CloseOtherTabs(index) => self.close_other_tabs(*index, false, ctx),
            CloseNonActiveTabs => self.close_other_tabs(self.active_tab_index, false, ctx),
            CloseTabsRight(index) => {
                self.close_tabs_direction(*index, TabMovement::Right, false, ctx)
            }
            CloseTabsRightActiveTab => {
                self.close_tabs_direction(self.active_tab_index, TabMovement::Right, false, ctx)
            }
            AddDefaultTab => {
                let effective_mode = AISettings::as_ref(ctx).default_session_mode(ctx);
                match effective_mode {
                    DefaultSessionMode::TabConfig => {
                        let ai_settings = AISettings::as_ref(ctx);
                        if let Some(config) = ai_settings.resolved_default_tab_config(ctx) {
                            self.open_tab_config(config, ctx);
                        } else {
                            // Config missing or deleted — clear and fall through to Terminal.
                            AISettings::handle(ctx).update(ctx, |settings, ctx| {
                                report_if_error!(settings
                                    .default_session_mode_internal
                                    .set_value(DefaultSessionMode::Terminal, ctx));
                                report_if_error!(settings
                                    .default_tab_config_path
                                    .set_value(String::new(), ctx));
                            });
                            self.add_terminal_tab(false, ctx);
                        }
                    }
                    DefaultSessionMode::DockerSandbox => {
                        self.add_docker_sandbox_tab(ctx);
                    }
                    // Terminal and Agent are handled by the existing path
                    // (add_terminal_tab applies DefaultSessionMode::Agent internally).
                    DefaultSessionMode::Agent if ChannelState::channel() == Channel::Oss => {
                        self.add_terminal_tab(false, ctx);
                    }
                    DefaultSessionMode::Terminal | DefaultSessionMode::Agent => {
                        if FeatureFlag::WelcomeTab.is_enabled() {
                            self.add_welcome_tab(ctx);
                        } else {
                            self.add_terminal_tab(false, ctx);
                        }
                    }
                }
            }
            AddTerminalTab { hide_homepage } => {
                self.add_new_session_tab_internal_with_default_session_mode_behavior(
                    NewSessionSource::Tab,
                    Some(window_id),
                    None,
                    None,
                    *hide_homepage,
                    DefaultSessionModeBehavior::Ignore,
                    ctx,
                );
                ctx.notify();
            }
            AddTabWithShell { shell, .. } => self.add_tab_with_shell(shell.clone(), ctx),
            AddGetStartedTab => self.add_get_started_tab(ctx),
            AddAgentTab => self.add_terminal_tab_with_new_agent_view(ctx),
            AddDockerSandboxTab => self.add_docker_sandbox_tab(ctx),
            OpenNewSessionMenu { position } => self.open_new_session_dropdown_menu(*position, ctx),
            ToggleTabConfigsMenu => self.toggle_tab_configs_menu(ctx),
            ShowSessionConfigModal => self.show_session_config_modal(ctx),
            DismissSessionConfigTabConfigChip => {
                self.dismiss_session_config_tab_config_chip(ctx);
            }
            SaveCurrentTabAsNewConfig(tab_index) => {
                self.save_current_tab_as_new_config(*tab_index, ctx)
            }
            ToggleNewSessionMenu {
                position,
                is_vertical_tabs,
            } => self.toggle_new_session_dropdown_menu(*position, *is_vertical_tabs, ctx),
            SelectNewSessionMenuItem(new_session_menu_item) => {
                self.open_launch_config_from_menu(new_session_menu_item.clone(), ctx)
            }
            SelectTabConfig(tab_config) => {
                self.open_tab_config(tab_config.clone(), ctx);
            }
            OpenNewWorktreeModal => {
                let cwd = self
                    .active_session_view(ctx)
                    .and_then(|view| view.as_ref(ctx).pwd())
                    .map(PathBuf::from);
                self.new_worktree_modal.view.update(ctx, |modal, ctx| {
                    modal.body().update(ctx, |body, ctx| {
                        body.on_open(cwd, ctx);
                    });
                });
                self.new_worktree_modal.open();
                self.current_workspace_state.is_new_worktree_modal_open = true;
                ctx.notify();
            }
            OpenNewWorktreeRepoPicker => {
                self.open_repo_picker_for_new_worktree_modal(ctx);
            }
            OpenTabConfigErrorFile {
                #[cfg_attr(not(feature = "local_fs"), allow(unused_variables))]
                path,
                toast_object_id,
            } => {
                #[cfg(feature = "local_fs")]
                {
                    let settings = EditorSettings::as_ref(ctx);
                    let target = resolve_file_target_with_editor_choice(
                        path,
                        *settings.open_code_panels_file_editor,
                        *settings.prefer_markdown_viewer,
                        *settings.open_file_layout,
                        None,
                    );
                    self.open_file_with_target(
                        path.clone(),
                        target,
                        None,
                        CodeSource::Link {
                            path: path.clone(),
                            range_start: None,
                            range_end: None,
                        },
                        ctx,
                    );
                }
                self.dismiss_older_toasts(toast_object_id, ctx);
            }
            TabConfigSidecarMakeDefault {
                mode,
                tab_config_path,
                #[cfg_attr(not(feature = "local_tty"), allow(unused_variables))]
                shell,
            } => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(settings.default_session_mode_internal.set_value(*mode, ctx));
                    if let Some(path) = tab_config_path {
                        report_if_error!(settings
                            .default_tab_config_path
                            .set_value(path.to_string_lossy().into_owned(), ctx));
                    }
                });
                #[cfg(feature = "local_tty")]
                if let Some(shell) = shell {
                    use crate::terminal::available_shells::AvailableShells;
                    AvailableShells::handle(ctx).update(ctx, |model, ctx| {
                        let _ = model.set_user_preferred_shell(shell.clone(), ctx);
                    });
                }
                self.close_new_session_dropdown_menu(ctx);
            }
            TabConfigSidecarEditConfig {
                #[cfg_attr(not(feature = "local_fs"), allow(unused_variables))]
                path,
            } => {
                #[cfg(feature = "local_fs")]
                {
                    let settings = EditorSettings::as_ref(ctx);
                    let target = resolve_file_target_with_editor_choice(
                        path,
                        *settings.open_code_panels_file_editor,
                        *settings.prefer_markdown_viewer,
                        *settings.open_file_layout,
                        None,
                    );
                    self.open_file_with_target(
                        path.clone(),
                        target,
                        None,
                        CodeSource::Link {
                            path: path.clone(),
                            range_start: None,
                            range_end: None,
                        },
                        ctx,
                    );
                }
                self.close_new_session_dropdown_menu(ctx);
            }
            TabConfigSidecarRemoveConfig { name, path } => {
                self.remove_tab_config_confirmation_dialog
                    .update(ctx, |dialog, ctx| {
                        dialog.set_config(name.clone(), path.clone());
                        ctx.notify();
                    });
                self.close_new_session_dropdown_menu(ctx);
                self.current_workspace_state
                    .is_remove_tab_config_dialog_open = true;
                ctx.focus(&self.remove_tab_config_confirmation_dialog);
                ctx.notify();
            }
            OpenSettingsFile => {
                let path = crate::settings::user_preferences_toml_file_path();
                self.add_tab_for_code_file(path, None, ctx);
            }
            FixSettingsWithAgent { error_description } => {
                use crate::ai::skills::SkillManager;
                let modify_settings_skill = SkillManager::as_ref(ctx)
                    .active_bundled_skill("modify-settings", ctx)
                    .cloned();
                let query = format!(
                    "My settings.toml file has an error: {error_description}. Please fix it."
                );
                self.active_tab_pane_group().update(ctx, |pane_group, ctx| {
                    pane_group.add_terminal_pane_in_agent_mode(None, None, ctx);
                    if let Some(terminal_view) = pane_group.focused_session_view(ctx) {
                        terminal_view.update(ctx, |terminal_view, terminal_view_ctx| {
                            // The modify-settings skill should always be available for
                            // production builds.
                            if let Some(skill) = modify_settings_skill {
                                terminal_view.ai_controller().update(
                                    terminal_view_ctx,
                                    |controller, ctx| {
                                        controller.send_slash_command_request(
                                            SlashCommandRequest::InvokeSkill {
                                                skill,
                                                user_query: Some(query),
                                            },
                                            ctx,
                                        );
                                    },
                                );
                            } else if let Some(conversation_id) =
                                terminal_view.active_conversation_id(terminal_view_ctx)
                            {
                                terminal_view.ai_controller().update(
                                    terminal_view_ctx,
                                    |controller, ctx| {
                                        controller.send_user_query_in_conversation(
                                            query,
                                            conversation_id,
                                            ctx,
                                        );
                                    },
                                );
                            }
                        });
                    }
                });
            }
            OpenWorktreeInRepo { repo_path } => {
                self.open_worktree_in_repo(repo_path.clone(), ctx);
            }
            OpenWorktreeAddRepoPicker => {
                self.close_new_session_dropdown_menu(ctx);
                self.open_folder_picker_for_worktree_submenu(ctx);
            }
            CopyVersion(version) => self.copy_version(version, ctx),
            ConfigureKeybindingSettings { keybinding_name } => {
                self.show_keyboard_settings(keybinding_name.as_deref(), ctx)
            }
            ShowSettings => self.show_settings(ctx),
            ShowSettingsPage(section) => self.show_settings_with_section(Some(*section), ctx),
            ShowSettingsPageWithSearch {
                search_query,
                section,
            } => self.show_settings_with_search(search_query, *section, ctx),
            ShowThemeChooser(mode) => self.show_theme_chooser(Some(*mode), ctx),
            ShowThemeChooserForActiveTheme => self.show_theme_chooser_for_active_theme(ctx),
            IncreaseFontSize => self.increase_font_size(ctx),
            DecreaseFontSize => self.decrease_font_size(ctx),
            ResetFontSize => self.reset_font_size(ctx),
            IncreaseZoom => self.increase_zoom(ctx),
            DecreaseZoom => self.decrease_zoom(ctx),
            ResetZoom => self.reset_zoom(ctx),
            OpenPalette {
                mode,
                source,
                query,
            } => self.open_palette_action(*mode, *source, query.as_deref(), ctx),
            TogglePalette {
                mode: palette_mode,
                source,
            } => self.toggle_palette(*palette_mode, *source, ctx),
            ViewUserDocs => self.view_user_docs(ctx),
            ViewPrivacyPolicy => self.view_privacy_policy(ctx),
            SendFeedback => self.send_feedback(ctx),
            #[cfg(not(target_family = "wasm"))]
            ViewLogs => self.view_logs(ctx),
            ChangeCursor(cursor) => self.change_cursor(*cursor, ctx),
            ToggleErrorUnderlining => self.toggle_error_underlining(ctx),
            ToggleSyntaxHighlighting => self.toggle_syntax_highlighting(ctx),
            SetA11yVerbosityLevel(verbosity) => self.set_a11y_verbosity(*verbosity, ctx),
            ToggleNotifications => self.toggle_notifications(ctx),
            ToggleTabColor { color, tab_index } => self.toggle_tab_color(*tab_index, *color, ctx),
            DispatchToSettingsTab(action) => {
                let window_id = ctx.window_id();
                ctx.dispatch_typed_action_for_view(window_id, self.settings_pane.id(), action)
            }
            OpenLink(link) => ctx.open_url(link),
            #[cfg(target_family = "wasm")]
            OpenLinkOnDesktop(url) => self.open_link_on_desktop(url, ctx),
            DumpDebugInfo => self.dump_debug_info(ctx),
            LogReviewCommentSendStatusForActiveTab => {
                self.right_panel_view.update(ctx, |right_panel_view, ctx| {
                    right_panel_view.log_review_comment_send_status_for_active_tab(ctx);
                });
            }
            #[cfg(target_os = "macos")]
            InstallCLI => self.install_cli(ctx),
            #[cfg(target_os = "macos")]
            UninstallCLI => self.uninstall_cli(ctx),
            UndoRevertInCodeReviewPane { window_id, view_id } => {
                self.undo_revert_in_code_review_pane(*window_id, *view_id, ctx)
            }
            ToggleRecordingMode => self.toggle_recording_mode(ctx),
            ToggleInBandGenerators => self.toggle_in_band_generators(ctx),
            ToggleDebugNetworkStatus => self.toggle_debug_network_status(ctx),
            ToggleShowMemoryStats => self.toggle_show_memory_stats(ctx),
            ToggleResourceCenter => self.toggle_resource_center(ctx),
            ToggleUserMenu => self.toggle_user_menu(ctx),
            ToggleKeybindingsPage => self.toggle_keybindings_page(ctx),
            ShowCommandSearch(CommandSearchOptions {
                filter,
                init_content,
            }) => self.show_command_search(*filter, init_content, ctx),
            CreatePersonalNotebook
            | CreatePersonalEnvVarCollection
            | CreatePersonalWorkflow
            | CreatePersonalFolder => {}
            ToggleMouseReporting => self.toggle_mouse_reporting(ctx),
            ToggleScrollReporting => self.toggle_scroll_reporting(ctx),
            ToggleFocusReporting => self.toggle_focus_reporting(ctx),
            StartTabDrag => {
                // If we are renaming a tab, finish the rename before dragging.
                self.finish_tab_rename(ctx);
                self.current_workspace_state.is_tab_being_dragged = true;
            }
            ToggleLeftPanel => {
                let active_pane_group = self.active_tab_pane_group().clone();
                let was_open = active_pane_group.read(ctx, |pg, _| pg.left_panel_open);

                // Don't open the panel if no views are available.
                if !was_open && self.left_panel_views.is_empty() {
                    return;
                }

                let file_tree_active = self
                    .left_panel_view
                    .read(ctx, |lp, _| lp.is_file_tree_active());
                self.toggle_left_panel(ctx);

                let is_open = active_pane_group.read(ctx, |pg, _| pg.left_panel_open);

                if !was_open && is_open {
                    self.left_panel_view.update(ctx, |left_panel, ctx| {
                        left_panel.focus_active_view_on_entry(ctx);
                    });

                    if file_tree_active {}
                }
            }
            ToggleRightPanel => {
                let pane_group_handle = self.active_tab_pane_group().clone();
                self.toggle_right_panel(&pane_group_handle, ctx);
            }
            #[cfg(feature = "local_fs")]
            OpenCodeReviewPanel(locator) => {
                let pane_group_handle = self
                    .tabs
                    .iter()
                    .find(|tab| tab.pane_group.id() == locator.pane_group_id)
                    .map(|tab| tab.pane_group.clone());
                if let Some(pane_group_handle) = pane_group_handle {
                    let read_result = pane_group_handle.read(ctx, |pane_group, ctx| {
                        pane_group
                            .terminal_view_from_pane_id(locator.pane_id, ctx)
                            .map(|terminal_view| {
                                let repo_path =
                                    terminal_view.as_ref(ctx).current_repo_path().cloned();
                                (repo_path, terminal_view.downgrade())
                            })
                    });
                    if let Some((repo_path, terminal_view)) = read_result {
                        let diff_state_model = repo_path.as_ref().and_then(|rp| {
                            self.working_directories_model.update(ctx, |model, ctx| {
                                model.get_or_create_diff_state_model(rp.clone(), ctx)
                            })
                        });
                        if let Some(diff_state_model) = diff_state_model {
                            let context = CodeReviewPaneContext {
                                repo_path,
                                diff_state_model,
                                terminal_view,
                            };
                            self.open_right_panel(&context, &pane_group_handle, ctx);
                        }
                    }
                }
            }
            #[cfg(not(feature = "local_fs"))]
            OpenCodeReviewPanel(_) => {}
            ToggleVerticalTabsPanel => {
                self.toggle_vertical_tabs_panel(ctx);
            }
            ToggleVerticalTabsSettingsPopup => {
                if FeatureFlag::VerticalTabs.is_enabled()
                    && *TabSettings::as_ref(ctx).use_vertical_tabs
                    && self.vertical_tabs_panel_open
                {
                    self.vertical_tabs_panel.show_settings_popup =
                        !self.vertical_tabs_panel.show_settings_popup;
                    ctx.notify();
                }
            }
            SetVerticalTabsDisplayGranularity(granularity) => {
                let granularity = *granularity;
                TabSettings::handle(ctx).update(ctx, |settings, ctx| {
                    let _ = settings
                        .vertical_tabs_display_granularity
                        .set_value(granularity, ctx);
                });
                ctx.notify();
            }
            SetVerticalTabsTabItemMode(mode) => {
                let mode = *mode;
                TabSettings::handle(ctx).update(ctx, |settings, ctx| {
                    let _ = settings.vertical_tabs_tab_item_mode.set_value(mode, ctx);
                });
                ctx.notify();
            }
            SetVerticalTabsViewMode(mode) => {
                let mode = *mode;
                TabSettings::handle(ctx).update(ctx, |settings, ctx| {
                    let _ = settings.vertical_tabs_view_mode.set_value(mode, ctx);
                });
                ctx.notify();
            }
            SetVerticalTabsPrimaryInfo(primary_info) => {
                let primary_info = *primary_info;
                TabSettings::handle(ctx).update(ctx, |settings, ctx| {
                    let _ = settings
                        .vertical_tabs_primary_info
                        .set_value(primary_info, ctx);
                });
                ctx.notify();
            }
            SetVerticalTabsCompactSubtitle(subtitle) => {
                let subtitle = *subtitle;
                TabSettings::handle(ctx).update(ctx, |settings, ctx| {
                    let _ = settings
                        .vertical_tabs_compact_subtitle
                        .set_value(subtitle, ctx);
                });
                ctx.notify();
            }
            ToggleVerticalTabsShowPrLink => {
                TabSettings::handle(ctx).update(ctx, |settings, ctx| {
                    let new_value = !*settings.vertical_tabs_show_pr_link.value();
                    let _ = settings
                        .vertical_tabs_show_pr_link
                        .set_value(new_value, ctx);
                });
                ctx.notify();
            }
            ToggleVerticalTabsShowDiffStats => {
                TabSettings::handle(ctx).update(ctx, |settings, ctx| {
                    let new_value = !*settings.vertical_tabs_show_diff_stats.value();
                    let _ = settings
                        .vertical_tabs_show_diff_stats
                        .set_value(new_value, ctx);
                });
                ctx.notify();
            }
            ToggleVerticalTabsShowDetailsOnHover => {
                TabSettings::handle(ctx).update(ctx, |settings, ctx| {
                    let new_value = !*settings.vertical_tabs_show_details_on_hover.value();
                    let _ = settings
                        .vertical_tabs_show_details_on_hover
                        .set_value(new_value, ctx);
                });
                ctx.notify();
            }
            ClosePanel => {
                if self.left_panel_view.is_self_or_child_focused(ctx) {
                    self.close_left_panel(ctx);
                } else if self.right_panel_view.is_self_or_child_focused(ctx) {
                    let pane_group_handle = self.active_tab_pane_group().clone();
                    self.close_right_panel(&pane_group_handle, ctx);
                }
            }
            OpenInExplorer { path } => {
                ctx.open_file_path_in_explorer(path);
            }
            OpenFilePath { path } => {
                ctx.open_file_path(path);
            }
            NewTabInAgentMode {
                entrypoint: _,
                zero_state_prompt_suggestion_type,
            } => {
                self.add_terminal_tab_in_ai_mode(*zero_state_prompt_suggestion_type, ctx);
            }
            NewPaneInAgentMode {
                entrypoint: _,
                zero_state_prompt_suggestion_type,
            } => {
                self.add_terminal_pane_in_ai_mode(*zero_state_prompt_suggestion_type, ctx);
            }
            DragTab {
                tab_index,
                tab_position,
            } => self.on_tab_drag(*tab_index, *tab_position, ctx),
            DropTab => {
                self.current_workspace_state.is_tab_being_dragged = false;
            }
            CopyTextToClipboard(text) => {
                ctx.clipboard()
                    .write(ClipboardContent::plain_text(text.to_string()));
            }
            DismissWorkspaceBanner(banner_type) => self.dismiss_workspace_banner(ctx, banner_type),
            Panic => {
                panic!("WorkspaceAction::Panic triggered from command palette");
            }
            DumpHeapProfile => {
                #[cfg(feature = "dhat_heap_profiling")]
                crate::profiling::dump_dhat_heap_profile();
            }
            OpenViewTreeDebugWindow => {
                let window_id = ctx.window_id();
                ctx.open_view_tree_debug_window(window_id);
            }
            ToggleSyncAllTerminalInputsInAllTabs => {
                let enabled = SyncedInputState::handle(ctx).update(ctx, |status, _| {
                    status.toggle_sync_all_terminal_inputs_in_all_tabs(window_id);

                    status.is_syncing_all_inputs(window_id)
                });
                let verb = if enabled { "enabled" } else { "disabled" };
                let mut message = format!("You {verb} synchronized inputs in all tabs.");
                if let Some(keystroke) = keybinding_name_to_keystroke(
                    "workspace:toggle_sync_all_terminal_inputs_in_all_tabs",
                    ctx,
                ) {
                    let _ = write!(message, " Press {} to undo.", keystroke.displayed());
                }
                self.toast_stack.update(ctx, |view, ctx| {
                    let new_toast = DismissibleToast::default(message);
                    view.add_ephemeral_toast(new_toast, ctx);
                });

                self.process_updated_sync_state(ctx);
            }
            ToggleSyncTerminalInputsInTab => {
                let enabled = SyncedInputState::handle(ctx).update(ctx, |status, _| {
                    let current_pane_group_id = self.active_tab_pane_group().id();

                    status.toggle_sync_terminal_inputs_in_tab(
                        current_pane_group_id,
                        self.all_pane_group_ids(),
                        self.tab_count(),
                        window_id,
                    );

                    status.should_sync_this_pane_group(current_pane_group_id, window_id)
                });
                let verb = if enabled { "enabled" } else { "disabled" };
                let mut message = format!("You {verb} synchronized inputs in this tab.");
                if let Some(keystroke) = keybinding_name_to_keystroke(
                    "workspace:toggle_sync_terminal_inputs_in_tab",
                    ctx,
                ) {
                    let _ = write!(message, " Press {} to undo.", keystroke.displayed());
                }
                self.toast_stack.update(ctx, |view, ctx| {
                    let new_toast = DismissibleToast::default(message);
                    view.add_ephemeral_toast(new_toast, ctx);
                });

                self.process_updated_sync_state(ctx);
            }
            DisableTerminalInputSync => {
                SyncedInputState::handle(ctx).update(ctx, |status, _| {
                    status.disable_sync_terminal_inputs(window_id);
                });

                self.process_updated_sync_state(ctx);

                self.toast_stack.update(ctx, |view, ctx| {
                    let new_toast =
                        DismissibleToast::success("Disabled all synchronized inputs.".to_string());
                    view.add_ephemeral_toast(new_toast, ctx);
                });
            }
            HandleConflictingWorkflow(_) | HandleConflictingEnvVarCollection(_) => {}
            OpenPromptEditor { .. } => {
                self.open_prompt_editor(ctx);
            }
            OpenAgentToolbarEditor => {
                self.open_agent_toolbar_editor(AgentToolbarEditorMode::AgentView, ctx);
            }
            OpenCLIAgentToolbarEditor => {
                self.open_agent_toolbar_editor(AgentToolbarEditorMode::CLIAgent, ctx);
            }
            OpenHeaderToolbarEditor => {
                self.open_header_toolbar_editor(ctx);
            }
            ShowHeaderToolbarContextMenu { position } => {
                self.show_header_toolbar_context_menu(*position, ctx);
            }
            ReopenClosedSession => {
                // While we could grab the UndoCloseStack singleton entity and
                // directly call undo_close(), it would fail when attempting to
                // restore a closed tab as we would attempt to update the
                // workspace while we are currently updating the workspace.
                // Instead, we use a global action to ensure we don't try to
                // perform nested updates on the workspace.
                ctx.dispatch_global_action("app:undo_close", ());
            }
            AddWindow => {
                ctx.dispatch_global_action("root_view:open_new", ());
            }
            AddWindowWithShell { shell } => {
                ctx.dispatch_global_action("root_view:open_new_with_shell", Some(shell.clone()));
            }
            NavigatePrevPaneOrPanel => {
                self.navigate_pane_or_panel(PanePanelDirection::Prev, ctx);
            }
            NavigateNextPaneOrPanel => {
                self.navigate_pane_or_panel(PanePanelDirection::Next, ctx);
            }
            FocusLeftPanel => self.focus_left_panel(ctx),
            FocusRightPanel => self.focus_right_panel(ctx),
            TerminateApp => {
                ctx.terminate_app(TerminationMode::Cancellable, None);
            }
            CloseWindow => {
                if ContextFlag::CloseWindow.is_enabled() {
                    ctx.close_window();
                }
            }
            RunAISuggestedCommand(code) => {
                let command = code.trim().to_string();
                let workflow = Workflow::new("Command from AI", command);
                self.run_workflow_in_active_input(
                    &WorkflowType::AIGenerated {
                        workflow,
                        origin: AIWorkflowOrigin::AgentMode,
                    },
                    WorkflowSource::AI,
                    WorkflowSelectionSource::AI,
                    None,
                    TerminalSessionFallbackBehavior::default(),
                    ctx,
                );
                ctx.notify();
            }
            RunCommand(code) => {
                let command = code.trim().to_string();
                self.insert_in_input(&command, true, true, false, ctx);
                ctx.notify();
            }
            InsertInInput {
                content,
                replace_buffer,
                ensure_agent_mode,
            } => {
                self.insert_in_input(content, *replace_buffer, false, *ensure_agent_mode, ctx);
                ctx.notify();
            }
            #[cfg(all(enable_crash_recovery, target_os = "linux"))]
            DismissWaylandCrashRecoveryBannerAndOpenLink => {
                self.dismiss_workspace_banner(ctx, &WorkspaceBanner::WaylandCrashRecovery);
                ctx.open_url("https://docs.warp.dev/terminal/more-features/linux#native-wayland");
            }
            FixInAgentMode { query } => {
                self.active_tab_pane_group().update(ctx, |pane_group, ctx| {
                    pane_group.add_terminal_pane_in_agent_mode(None, None, ctx);
                    if let Some(terminal_view) = pane_group.focused_session_view(ctx) {
                        terminal_view.update(ctx, |terminal_view, terminal_view_ctx| {
                            terminal_view.ai_controller().update(
                                terminal_view_ctx,
                                |controller, ctx| {
                                    controller.send_user_query_in_new_conversation(
                                        query.to_owned(),
                                        None,
                                        EntrypointType::UserInitiated,
                                        ctx,
                                    );
                                },
                            );
                        });
                    }
                });
            }
            OpenAIFactCollection => {
                self.open_ai_fact_collection_pane(None, None, ctx);
            }
            OpenMCPServerCollection => {
                self.show_settings_with_section(Some(SettingsSection::MCPServers), ctx);
            }
            ToggleAIDocumentPane {
                document_id,
                document_version,
            } => {
                let conversation_id =
                    AIDocumentModel::as_ref(ctx).get_conversation_id_for_document_id(document_id);

                if let Some(conversation_id) = conversation_id {
                    self.active_tab_pane_group().update(ctx, |pane_group, ctx| {
                        pane_group.toggle_ai_document_pane(
                            conversation_id,
                            *document_id,
                            *document_version,
                            ctx,
                        );
                    });
                }
            }
            HideAIDocumentPanes => {
                self.active_tab_pane_group().update(ctx, |pane_group, ctx| {
                    pane_group.close_all_ai_document_panes(ctx);
                });
            }
            OpenAIDocumentPane {
                document_id,
                document_version,
            } => {
                let conversation_id =
                    AIDocumentModel::as_ref(ctx).get_conversation_id_for_document_id(document_id);

                if let Some(conversation_id) = conversation_id {
                    self.active_tab_pane_group().update(ctx, |pane_group, ctx| {
                        pane_group.open_ai_document_pane(
                            conversation_id,
                            *document_id,
                            *document_version,
                            ctx,
                        );
                    });
                }
            }
            TabHoverWidthStart { width } => {
                // Store the fixed width value for the tab to maintain consistent size during hover
                self.tab_fixed_width = Some(*width);
                ctx.notify();
            }
            TabHoverWidthEnd => {
                // Clear the stored width when hover ends
                self.tab_fixed_width = None;
                ctx.notify();
            }
            FocusTerminalViewInWorkspace { terminal_view_id } => {
                if !self.focus_terminal_view_locally(*terminal_view_id, ctx) {
                    self.focus_terminal_view_in_other_window(*terminal_view_id, ctx);
                }
            }
            FocusPane(locator) => {
                self.focus_pane(*locator, ctx);
            }
            StartNewConversation { terminal_view_id } => {
                Self::set_pending_query_state_for_terminal_view(
                    *terminal_view_id,
                    PendingQueryState::default(),
                    ctx,
                );

                self.handle_action(
                    &WorkspaceAction::FocusTerminalViewInWorkspace {
                        terminal_view_id: *terminal_view_id,
                    },
                    ctx,
                );
            }
            ScrollToSettingsWidget { page, widget_id } => {
                self.open_settings_pane(Some(*page), None, ctx);
                self.settings_pane.update(ctx, |settings, ctx| {
                    settings.scroll_to_settings_widget(*page, widget_id, ctx);
                });
                ctx.notify();
            }
            OpenFileInNewTab {
                full_path,
                line_and_column,
            } => {
                self.add_tab_for_code_file(full_path.clone(), *line_and_column, ctx);
            }
            OpenRepository { path } => {
                self.open_repository(path.as_deref(), ctx);
            }
            OpenTabConfigRepoPicker { param_index } => {
                self.open_repo_picker_for_tab_config_modal(*param_index, ctx);
            }
            NewCodeFile => {
                self.add_tab_for_new_code_file(ctx);
            }
            OpenNotebook { .. } => {}
            RunWorkflow {
                workflow,
                workflow_source,
                workflow_selection_source,
                argument_override,
            } => self.run_workflow_in_active_input(
                workflow,
                workflow_source.clone(),
                *workflow_selection_source,
                argument_override.clone(),
                TerminalSessionFallbackBehavior::default(),
                ctx,
            ),
            RestoreOrNavigateToConversation {
                pane_view_locator,
                window_id,
                conversation_id,
                terminal_view_id,
                restore_layout,
            } => {
                self.restore_or_navigate_to_conversation(
                    *conversation_id,
                    *window_id,
                    *pane_view_locator,
                    *terminal_view_id,
                    *restore_layout,
                    ctx,
                );
            }
            ForkAIConversation {
                conversation_id,
                fork_from_exchange,
                summarize_after_fork,
                summarization_prompt,
                initial_prompt,
                destination,
            } => {
                self.fork_ai_conversation(
                    *conversation_id,
                    *fork_from_exchange,
                    *summarize_after_fork,
                    summarization_prompt.clone(),
                    initial_prompt.clone(),
                    *destination,
                    ctx,
                );
            }
            #[cfg(not(target_family = "wasm"))]
            ContinueConversationLocally { conversation_id } => {
                self.fork_ai_conversation(
                    *conversation_id,
                    None,
                    false,
                    None,
                    None,
                    ForkedConversationDestination::SplitPane,
                    ctx,
                );
            }
            SummarizeAIConversation {
                prompt,
                initial_prompt,
            } => {
                self.summarize_active_ai_conversation(prompt.clone(), initial_prompt.clone(), ctx);
            }
            QueuePromptForConversation { prompt } => {
                let Some(terminal_view) = self
                    .active_tab_pane_group()
                    .as_ref(ctx)
                    .active_session_view(ctx)
                else {
                    return;
                };

                terminal_view.update(ctx, |terminal, ctx| {
                    terminal.send_user_query_after_next_conversation_finished(
                        prompt.clone(),
                        /* show_close_button */ true,
                        /* show_send_now_button */ true,
                        ctx,
                    );
                });
            }
            InsertForkSlashCommand => {
                self.active_tab_pane_group().update(ctx, |pane_group, ctx| {
                    if let Some(terminal_view) = pane_group.active_session_view(ctx) {
                        terminal_view.update(ctx, |terminal, ctx| {
                            terminal.input().update(ctx, |input, ctx| {
                                input.replace_buffer_content(
                                    &format!("{} ", commands::FORK.name),
                                    ctx,
                                );
                                ctx.focus_self();
                            });
                        });
                    }
                });
            }
            CreatePersonalAIPrompt => {}
            #[cfg(feature = "local_fs")]
            FileRenamed { old_path, new_path } => {
                self.rename_tabs_with_file_path(old_path, new_path, ctx);
            }
            #[cfg(feature = "local_fs")]
            FileDeleted { path } => {
                self.close_tabs_with_file_path(path, ctx);
            }
            #[cfg(debug_assertions)]
            DebugResetAwsBedrockLoginBannerDismissed => {
                // Reset the AWS Bedrock login banner dismissed state for debugging
                AISettings::handle(ctx).update(ctx, |ai_settings, ctx| {
                    if let Err(e) = ai_settings
                        .aws_bedrock_login_banner_dismissed
                        .set_value(false, ctx)
                    {
                        log::warn!(
                            "Failed to reset AWS Bedrock login banner dismissed setting: {e}"
                        );
                    }
                });
                log::info!("AWS Bedrock login banner dismissed state has been reset");
            }
            #[cfg(debug_assertions)]
            InstallOpenCodeWarpPlugin => {
                let message = set_opencode_warp_plugin("github:warpdotdev/opencode-warp-internal");
                self.toast_stack.update(ctx, |view, ctx| {
                    view.add_ephemeral_toast(DismissibleToast::default(message), ctx);
                });
            }
            #[cfg(debug_assertions)]
            UseLocalOpenCodeWarpPlugin => {
                let message = match dirs::home_dir() {
                    Some(home) => {
                        let plugin_path = home.join("opencode-warp/src/index.ts");
                        let entry = format!("file://{}", plugin_path.display());
                        set_opencode_warp_plugin(&entry)
                    }
                    None => "Failed to determine home directory".to_string(),
                };
                self.toast_stack.update(ctx, |view, ctx| {
                    view.add_ephemeral_toast(DismissibleToast::default(message), ctx);
                });
            }
            ToggleProjectExplorer => {
                if *CodeSettings::as_ref(ctx).show_project_explorer {
                    let is_showing = self.left_panel_view.as_ref(ctx).active_view()
                        == ToolPanelView::ProjectExplorer;
                    self.toggle_left_panel_view(&LeftPanelAction::ProjectExplorer, is_showing, ctx);
                }
            }
            ToggleGlobalSearch => {
                if FeatureFlag::GlobalSearch.is_enabled()
                    && *CodeSettings::as_ref(ctx).show_global_search
                {
                    let is_showing = matches!(
                        self.left_panel_view.as_ref(ctx).active_view(),
                        ToolPanelView::GlobalSearch { .. }
                    );
                    self.toggle_left_panel_view(
                        &LeftPanelAction::GlobalSearch {
                            entry_focus: GlobalSearchEntryFocus::QueryEditor,
                        },
                        is_showing,
                        ctx,
                    );
                }
            }
            OpenGlobalSearch => {
                if FeatureFlag::GlobalSearch.is_enabled()
                    && *CodeSettings::as_ref(ctx).show_global_search
                {
                    if let Some(selected_text) = self.get_selected_text_from_focused_view(ctx) {
                        if let Some(global_search_view) = self
                            .left_panel_view
                            .as_ref(ctx)
                            .active_global_search_view(ctx)
                        {
                            // If we detect selected text in the active pane, pre-populate the global search input
                            global_search_view.update(ctx, |view, ctx| {
                                view.set_initial_query(selected_text, ctx);
                            });
                        }
                    }

                    self.open_left_panel_view(
                        &LeftPanelAction::GlobalSearch {
                            entry_focus: GlobalSearchEntryFocus::QueryEditor,
                        },
                        ctx,
                    );
                }
            }
            ShowRewindConfirmationDialog {
                ai_block_view_id,
                exchange_id,
                conversation_id,
            } => {
                self.show_rewind_confirmation_dialog(
                    RewindDialogSource {
                        ai_block_view_id: *ai_block_view_id,
                        exchange_id: *exchange_id,
                        conversation_id: *conversation_id,
                    },
                    ctx,
                );
            }
            ExecuteRewindAIConversation {
                ai_block_view_id,
                exchange_id,
                conversation_id,
            } => {
                // Extract the user query before the rewind to prefill the input
                let user_query = BlocklistAIHistoryModel::as_ref(ctx)
                    .conversation(conversation_id)
                    .and_then(|c| c.root_task_exchanges().find(|e| e.id == *exchange_id))
                    .and_then(|e| {
                        e.input
                            .iter()
                            .find(|i| i.is_user_query())
                            .and_then(|i| i.user_query())
                    });

                // Dispatch to the active terminal to execute the rewind
                if let Some(terminal_view) = self
                    .active_tab_pane_group()
                    .as_ref(ctx)
                    .focused_session_view(ctx)
                {
                    terminal_view.update(ctx, |terminal, ctx| {
                        terminal.handle_action(
                            &TerminalAction::ExecuteRewindAIConversation {
                                ai_block_view_id: *ai_block_view_id,
                                exchange_id: *exchange_id,
                                conversation_id: *conversation_id,
                            },
                            ctx,
                        );
                    });
                }

                // Prefill the input after the rewind
                if let Some(query) = user_query {
                    self.insert_in_input(&query, true, false, true, ctx);
                }
            }
            ExecuteDeleteConversation {
                conversation_id,
                terminal_view_id,
            } => {
                // Exit agent view first if this conversation is currently expanded.
                // This must happen before updating BlocklistAIHistoryModel to avoid
                // circular model references.
                if let Some(controller) = ActiveAgentViewsModel::as_ref(ctx)
                    .get_controller_for_conversation(*conversation_id, ctx)
                {
                    let succesfully_exited_agent_view =
                        controller.update(ctx, |controller, ctx| {
                            controller.exit_agent_view(ctx);
                            !controller.is_active()
                        });

                    if !succesfully_exited_agent_view {
                        ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                            toast_stack.add_ephemeral_toast(
                                DismissibleToast::error(
                                    "Failed to delete conversation. Please exit the agent view and try again.".to_string(),
                                ),
                                window_id,
                                ctx,
                            );
                        });
                        return;
                    }
                }

                conversation_utils::delete_conversation(*conversation_id, *terminal_view_id, ctx);

                ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                    toast_stack.add_ephemeral_toast(
                        DismissibleToast::success("Conversation deleted".to_string()),
                        window_id,
                        ctx,
                    );
                });
            }
            OpenLightbox {
                images,
                initial_index,
            } => {
                let params = LightboxParams {
                    images: images.clone(),
                    initial_index: *initial_index,
                };
                if let Some(handle) = &self.lightbox_view {
                    handle.update(ctx, |view, ctx| view.update_params(params, ctx));
                } else {
                    let handle = ctx.add_typed_action_view(|ctx| LightboxView::new(params, ctx));
                    ctx.subscribe_to_view(&handle, |me, _, event, ctx| match event {
                        LightboxViewEvent::Close => {
                            me.lightbox_view = None;
                            me.focus_active_tab(ctx);
                            ctx.notify();
                        }
                        LightboxViewEvent::FocusLost => {
                            // Focus already moved elsewhere; just tear down the view.
                            me.lightbox_view = None;
                            ctx.notify();
                        }
                    });
                    ctx.focus(&handle);
                    self.lightbox_view = Some(handle);
                }
                ctx.notify();
            }
            UpdateLightboxImage { index, image } => {
                if let Some(handle) = &self.lightbox_view {
                    handle.update(ctx, |view, ctx| {
                        view.update_image_at(*index, image.clone(), ctx);
                    });
                    ctx.notify();
                }
            }
            HandoffPendingTransfer { .. } => {}
            ReverseHandoff { .. } => {}
            FinalizeDropTab => {}
            SyncTrafficLights => {
                self.sync_window_button_visibility(ctx);
            }
        };
        if action.should_save_app_state_on_action() {
            ctx.dispatch_global_action("workspace:save_app", ());
        }
    }
}

impl View for Workspace {
    fn ui_name() -> &'static str {
        "Workspace"
    }

    fn self_or_child_interacted_with(&self, ctx: &mut ViewContext<Self>) {
        self.sync_window_button_visibility(ctx);
    }

    fn keymap_context(&self, app: &AppContext) -> warpui::keymap::Context {
        let mut context = Self::default_keymap_context();

        if NetworkStatus::as_ref(app).is_online() {
            context.set.insert("IsOnline");
        }

        if AISettings::as_ref(app).is_any_ai_enabled(app) {
            context.set.insert(flags::IS_ANY_AI_ENABLED);
        }

        if AISettings::as_ref(app).is_active_ai_enabled(app) {
            context.set.insert(flags::IS_ACTIVE_AI_ENABLED);
        }
        if AISettings::as_ref(app).is_voice_input_enabled(app)
            && UserWorkspaces::as_ref(app).is_voice_enabled()
        {
            context.set.insert(flags::IS_VOICE_INPUT_ENABLED);
        }

        if self
            .active_tab_pane_group()
            .as_ref(app)
            .any_pane_being_dragged(app)
        {
            context.set.insert("Workspace_PaneDragging");
        }

        // TODO: This is temporary. We currently check if any code pane is open where it should
        // really be whether the code pane is opened and focused.
        if self
            .active_tab_pane_group()
            .as_ref(app)
            .pane_ids()
            .any(|id| id.is_code_pane())
        {
            context.set.insert("Workspace_TextOpen");
        }

        if matches!(
            *AccessibilitySettings::as_ref(app).a11y_verbosity,
            AccessibilityVerbosity::Verbose
        ) {
            context.set.insert("AccessibilityVerbosity_Verbose");
        }

        if ContextFlag::CloseWindow.is_enabled() {
            context.set.insert("Workspace_CloseWindow");
        }

        match self.tab_count() {
            0 => {
                debug_assert!(false, "Should always be at least one tab");
            }
            1 => {
                context.set.insert("Workspace_SingleTab");
            }
            n => {
                context.set.insert("Workspace_MultipleTabs");
                if self.active_tab_index == 0 {
                    context.set.insert("Workspace_LeftmostTabActive");
                } else if self.active_tab_index == n - 1 {
                    context.set.insert("Workspace_RightmostTabActive");
                }
            }
        };

        if AISettings::as_ref(app).is_any_ai_enabled(app)
            && *AISettings::as_ref(app).show_conversation_history
        {
            context.set.insert(flags::SHOW_CONVERSATION_HISTORY);
        }

        if *CodeSettings::as_ref(app).show_project_explorer {
            context.set.insert(flags::SHOW_PROJECT_EXPLORER);
        }
        if *CodeSettings::as_ref(app).show_global_search {
            context.set.insert(flags::SHOW_GLOBAL_SEARCH);
        }

        self.add_toggle_setting_context_flags(app, &mut context);

        let sync_state = SyncedInputState::as_ref(app);

        if sync_state.is_syncing_all_inputs(self.window_id) {
            context.set.insert(flags::SYNC_ALL_TABS_FLAG);
        } else if sync_state
            .is_syncing_all_panes_in_pane_group(self.window_id, self.active_tab_pane_group().id())
        {
            context.set.insert(flags::SYNC_ALL_PANES_IN_CURRENT_TAB);
        }

        let is_universal_developer_input_enabled =
            InputSettings::as_ref(app).is_universal_developer_input_enabled(app);

        if is_universal_developer_input_enabled {
            context.set.insert(flags::UNIVERSAL_DEVELOPER_INPUT_ENABLED);
        }

        let default_terminal = DefaultTerminal::as_ref(app);
        if default_terminal.is_warp_default() {
            context.set.insert(flags::WARP_IS_DEFAULT_TERMINAL);
        }

        if FeatureFlag::DebugMode.is_enabled() {
            let debug_settings = DebugSettings::as_ref(app);
            if *debug_settings.recording_mode.value() {
                context.set.insert(flags::RECORDING_MODE_FLAG);
            }
            if *debug_settings
                .are_in_band_generators_for_all_sessions_enabled
                .value()
            {
                context.set.insert(flags::IN_BAND_GENERATORS_FLAG);
            }

            let network_status = NetworkStatus::as_ref(app);
            if network_status.is_online() {
                context.set.insert(flags::DEBUG_NETWORK_ONLINE_FLAG);
            }

            if debug_settings.should_show_memory_stats() {
                context.set.insert(flags::DEBUG_SHOW_MEMORY_STATS_FLAG);
            }
        }

        if let Some(terminal_view) = self
            .active_tab_pane_group()
            .as_ref(app)
            .focused_session_view(app)
        {
            let terminal_view = terminal_view.as_ref(app);
            if terminal_view.is_long_running() {
                context.set.insert("LongRunningCommand");
            }

            if FeatureFlag::AgentView.is_enabled() {
                let agent_view_state = terminal_view
                    .agent_view_controller()
                    .as_ref(app)
                    .agent_view_state();
                if agent_view_state.is_fullscreen() {
                    context.set.insert(flags::ACTIVE_AGENT_VIEW);
                } else if agent_view_state.is_inline() {
                    context.set.insert(flags::ACTIVE_INLINE_AGENT_VIEW);
                }
            }
        }

        context
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);

        let tab_bar_mode = self.tab_bar_mode(app);

        // For WASM simplified tab bar views,
        // we render the tab bar outside of panels so that the details panel only affects content below the tab bar.
        cfg_if::cfg_if! {
            if #[cfg(target_family = "wasm")] {
                let use_simplified_wasm_tab_bar = self.get_simplified_wasm_tab_bar_content(app).is_some();
            } else {
                let use_simplified_wasm_tab_bar = false;
            }
        }

        let panels = if use_simplified_wasm_tab_bar {
            // For the simplified WASM tab bar, we want to render the tab bar on top of all other content
            // so that content being added/moved around in the workspace (for example the details panel being toggled)
            // does not affect the tab.
            let mut outer_column = Flex::column();
            if tab_bar_mode == ShowTabBar::Stacked {
                outer_column.add_child(self.render_tab_bar(self.tab_fixed_width, appearance, app));
            }
            let content = self.render_banner_and_active_tab(app, appearance);
            // Hide the vertical tab rail for simplified WASM views (notebooks, transcripts, etc.)
            let panels_row = self.render_panels(app, Shrinkable::new(1.0, content).finish(), true);
            outer_column.add_child(Shrinkable::new(1.0, panels_row).finish());
            outer_column.finish()
        } else {
            let mut outer_column = Flex::column();
            if tab_bar_mode == ShowTabBar::Stacked {
                outer_column.add_child(self.render_tab_bar(self.tab_fixed_width, appearance, app));
            }
            let content = self.render_banner_and_active_tab(app, appearance);
            let panels_row = self.render_panels(app, Shrinkable::new(1.0, content).finish(), false);
            outer_column.add_child(Shrinkable::new(1.0, panels_row).finish());
            Container::new(outer_column.finish())
                .with_background(util::get_terminal_background_fill(self.window_id, app))
                .finish()
        };
        let mut stack = Stack::new();

        #[cfg(target_family = "wasm")]
        {
            let pane_group = self.active_tab_pane_group().as_ref(app);
            if warpui::platform::wasm::is_mobile_device() && pane_group.left_panel_open {
                let scrim = Rect::new()
                    .with_background(Fill::Solid(ColorU::new(
                        0,
                        0,
                        0,
                        MOBILE_OVERLAY_SCRIM_ALPHA,
                    )))
                    .finish();
                let clickable_scrim = EventHandler::new(scrim)
                    .on_left_mouse_down(|ctx, _, _| {
                        ctx.dispatch_typed_action(WorkspaceAction::ToggleLeftPanel);
                        DispatchEventResult::StopPropagation
                    })
                    .finish();
                stack.add_positioned_overlay_child(
                    Percentage::width(1.0 - MOBILE_OVERLAY_PANEL_WIDTH_RATIO, clickable_scrim)
                        .finish(),
                    OffsetPositioning::offset_from_save_position_element(
                        TAB_BAR_POSITION_ID,
                        vec2f(0., 0.),
                        PositionedElementOffsetBounds::WindowBySize,
                        PositionedElementAnchor::BottomRight,
                        ChildAnchor::TopRight,
                    ),
                );

                let panel_content = Container::new(ChildView::new(&self.left_panel_view).finish())
                    .with_background(appearance.theme().surface_1())
                    .finish();
                stack.add_positioned_overlay_child(
                    Percentage::width(MOBILE_OVERLAY_PANEL_WIDTH_RATIO, panel_content).finish(),
                    OffsetPositioning::offset_from_save_position_element(
                        TAB_BAR_POSITION_ID,
                        vec2f(0., 0.),
                        PositionedElementOffsetBounds::WindowBySize,
                        PositionedElementAnchor::BottomLeft,
                        ChildAnchor::TopLeft,
                    ),
                );
            }
        }

        stack.add_child(
            Container::new(panels)
                .with_uniform_padding(WORKSPACE_PADDING)
                .finish(),
        );

        if !use_simplified_wasm_tab_bar
            && FeatureFlag::VerticalTabs.is_enabled()
            && *TabSettings::as_ref(app).use_vertical_tabs
            && self.vertical_tabs_panel_open
            && self.vertical_tabs_panel.show_settings_popup
        {
            stack.add_positioned_overlay_child(
                Dismiss::new(render_settings_popup(&self.vertical_tabs_panel, app))
                    .prevent_interaction_with_other_elements()
                    .on_dismiss(|ctx, _| {
                        ctx.dispatch_typed_action(WorkspaceAction::ToggleVerticalTabsSettingsPopup);
                    })
                    .finish(),
                OffsetPositioning::offset_from_save_position_element(
                    VERTICAL_TABS_SETTINGS_BUTTON_POSITION_ID,
                    vec2f(0., 4.),
                    PositionedElementOffsetBounds::WindowByPosition,
                    PositionedElementAnchor::BottomLeft,
                    ChildAnchor::TopLeft,
                ),
            );
        }

        if FeatureFlag::VerticalTabs.is_enabled()
            && *TabSettings::as_ref(app).use_vertical_tabs
            && self.vertical_tabs_panel_open
        {
            if let Some(vertical_tabs::DetailSidecarOverlay {
                anchor_position_id,
                offset,
                bounds,
                parent_anchor,
                child_anchor,
                sidecar,
            }) = render_detail_sidecar(
                &self.vertical_tabs_panel,
                self,
                Self::tabs_panel_side(&TabSettings::as_ref(app).header_toolbar_chip_selection),
                app,
            ) {
                stack.add_positioned_overlay_child(
                    sidecar,
                    OffsetPositioning::offset_from_save_position_element(
                        &anchor_position_id,
                        offset,
                        bounds,
                        parent_anchor,
                        child_anchor,
                    ),
                );
            }
        }

        // Conditionally render tab bar menus.
        if tab_bar_mode.has_tab_bar() && self.show_tab_bar_overflow_menu {
            stack.add_positioned_overlay_child(
                ChildView::new(&self.tab_bar_overflow_menu).finish(),
                OffsetPositioning::offset_from_save_position_element(
                    "tab_bar_overflow_button",
                    vec2f(0., 10.),
                    PositionedElementOffsetBounds::Unbounded,
                    PositionedElementAnchor::BottomRight,
                    ChildAnchor::TopRight,
                ),
            );
        }

        if let Some((tab_idx, right_click_menu_anchor)) = self.show_tab_right_click_menu {
            let is_vertical = FeatureFlag::VerticalTabs.is_enabled()
                && *TabSettings::as_ref(app).use_vertical_tabs
                && self.vertical_tabs_panel_open;
            if tab_bar_mode.has_tab_bar() || is_vertical {
                let positioning = match (is_vertical, right_click_menu_anchor) {
                    (true, TabContextMenuAnchor::VerticalTabsKebab) => {
                        // Anchor depends on which side the tabs panel is configured on.
                        let tabs_side = Self::tabs_panel_side(
                            &TabSettings::as_ref(app).header_toolbar_chip_selection,
                        );
                        let (anchor, child_anchor) = if tabs_side == PanelPosition::Left {
                            (PositionedElementAnchor::BottomLeft, ChildAnchor::TopLeft)
                        } else {
                            (PositionedElementAnchor::BottomRight, ChildAnchor::TopRight)
                        };
                        Some(OffsetPositioning::offset_from_save_position_element(
                            vertical_tabs::vtab_action_buttons_position_id(tab_idx),
                            vec2f(0., 4.),
                            PositionedElementOffsetBounds::WindowByPosition,
                            anchor,
                            child_anchor,
                        ))
                    }
                    (true, TabContextMenuAnchor::Pointer(position)) => {
                        Some(OffsetPositioning::offset_from_parent(
                            position,
                            ParentOffsetBounds::WindowByPosition,
                            ParentAnchor::TopLeft,
                            ChildAnchor::TopLeft,
                        ))
                    }
                    (false, TabContextMenuAnchor::Pointer(position)) => {
                        Some(OffsetPositioning::offset_from_parent(
                            position,
                            ParentOffsetBounds::Unbounded,
                            ParentAnchor::TopLeft,
                            ChildAnchor::TopLeft,
                        ))
                    }
                    (false, TabContextMenuAnchor::VerticalTabsKebab) => None,
                };
                if let Some(positioning) = positioning {
                    stack.add_positioned_overlay_child(
                        ChildView::new(&self.tab_right_click_menu).finish(),
                        positioning,
                    );
                }
            }
        }

        if let Some(position) = self.show_header_toolbar_context_menu {
            stack.add_positioned_overlay_child(
                ChildView::new(&self.header_toolbar_context_menu).finish(),
                OffsetPositioning::offset_from_parent(
                    position,
                    ParentOffsetBounds::WindowByPosition,
                    ParentAnchor::TopLeft,
                    ChildAnchor::TopLeft,
                ),
            );
        }

        // Render the new session dropdown menu. This is outside the tab bar visibility
        // gate because it can also be opened from the vertical tabs panel.
        if self.show_new_session_dropdown_menu.is_some() {
            let is_vertical = FeatureFlag::VerticalTabs.is_enabled()
                && *TabSettings::as_ref(app).use_vertical_tabs
                && self.vertical_tabs_panel_open;

            if is_vertical {
                // Anchor the menu below the vertical-tabs + button.
                stack.add_positioned_overlay_child(
                    ChildView::new(&self.new_session_dropdown_menu).finish(),
                    OffsetPositioning::offset_from_save_position_element(
                        vertical_tabs::VERTICAL_TABS_ADD_TAB_POSITION_ID,
                        vec2f(0., 4.),
                        PositionedElementOffsetBounds::WindowBySize,
                        PositionedElementAnchor::BottomLeft,
                        ChildAnchor::TopLeft,
                    ),
                );
            } else {
                // TODO(CORE-2300): In the new version of the shell selector, this is not a
                // context menu but a dropdown. Since it is quite wide, we need to reposition
                // it so it does not render outside the bounds of the window.
                let new_session_menu_position = self.show_new_session_dropdown_menu.unwrap();
                let bounds = if FeatureFlag::ShellSelector.is_enabled() {
                    ParentOffsetBounds::WindowByPosition
                } else {
                    ParentOffsetBounds::Unbounded
                };
                stack.add_positioned_overlay_child(
                    ChildView::new(&self.new_session_dropdown_menu).finish(),
                    OffsetPositioning::offset_from_parent(
                        new_session_menu_position,
                        bounds,
                        ParentAnchor::TopLeft,
                        ChildAnchor::TopLeft,
                    ),
                );
            }

            // Sidecar menu for submenu parents (New worktree config).
            if self.show_new_session_sidecar {
                let anchor_label = self.new_session_dropdown_menu.read(app, |menu, _| {
                    menu.hovered_index().and_then(|idx| {
                        menu.items().get(idx).and_then(|item| match item {
                            MenuItem::Item(fields) => Some(fields.label().to_string()),
                            _ => None,
                        })
                    })
                });

                if let Some(anchor_label) = anchor_label {
                    let sidecar_element = SavePosition::new(
                        ChildView::new(&self.new_session_sidecar_menu).finish(),
                        NEW_SESSION_SIDECAR_POSITION_ID,
                    )
                    .finish();

                    let render_left = self.should_render_sidecar_left(
                        &anchor_label,
                        NEW_SESSION_SIDECAR_WIDTH,
                        app,
                    );
                    let (offset, parent_anchor, child_anchor) = if render_left {
                        (
                            vec2f(-4., 0.),
                            PositionedElementAnchor::TopLeft,
                            ChildAnchor::TopRight,
                        )
                    } else {
                        (
                            vec2f(4., 0.),
                            PositionedElementAnchor::TopRight,
                            ChildAnchor::TopLeft,
                        )
                    };

                    stack.add_positioned_overlay_child(
                        sidecar_element,
                        OffsetPositioning::offset_from_save_position_element(
                            anchor_label,
                            offset,
                            PositionedElementOffsetBounds::WindowByPosition,
                            parent_anchor,
                            child_anchor,
                        ),
                    );
                }
            }

            // Action sidecar for actionable items (Terminal, Agent, tab configs).
            if let Some(sidecar_item) = &self.tab_config_action_sidecar_item {
                let anchor_label = self.new_session_dropdown_menu.read(app, |menu, _| {
                    menu.hovered_index().and_then(|idx| {
                        menu.items().get(idx).and_then(|item| match item {
                            MenuItem::Item(fields) => Some(fields.label().to_string()),
                            _ => None,
                        })
                    })
                });

                if let Some(anchor_label) = anchor_label {
                    let is_already_default = {
                        let ai_settings = AISettings::as_ref(app);
                        let current_mode = ai_settings.default_session_mode(app);
                        let current_path = ai_settings.default_tab_config_path();
                        match sidecar_item {
                            SidecarItemKind::BuiltIn {
                                default_mode,
                                shell,
                                ..
                            } => {
                                current_mode == *default_mode
                                    && *default_mode != DefaultSessionMode::TabConfig
                                    && shell.is_none()
                            }
                            SidecarItemKind::UserTabConfig { config } => {
                                current_mode == DefaultSessionMode::TabConfig
                                    && config
                                        .source_path
                                        .as_ref()
                                        .is_some_and(|p| p.to_string_lossy() == current_path)
                            }
                        }
                    };
                    let sidecar_content = crate::tab_configs::action_sidecar::render_action_sidecar(
                        sidecar_item,
                        &self.tab_config_action_sidecar_mouse_states,
                        is_already_default,
                        app,
                    );
                    let sidecar_element =
                        SavePosition::new(sidecar_content, NEW_SESSION_SIDECAR_POSITION_ID)
                            .finish();

                    let render_left = self.should_render_sidecar_left(
                        &anchor_label,
                        crate::tab_configs::action_sidecar::SIDECAR_WIDTH,
                        app,
                    );
                    let (offset, parent_anchor, child_anchor) = if render_left {
                        (
                            vec2f(-4., 0.),
                            PositionedElementAnchor::TopLeft,
                            ChildAnchor::TopRight,
                        )
                    } else {
                        (
                            vec2f(4., 0.),
                            PositionedElementAnchor::TopRight,
                            ChildAnchor::TopLeft,
                        )
                    };

                    stack.add_positioned_overlay_child(
                        sidecar_element,
                        OffsetPositioning::offset_from_save_position_element(
                            anchor_label,
                            offset,
                            PositionedElementOffsetBounds::WindowByPosition,
                            parent_anchor,
                            child_anchor,
                        ),
                    );
                }
            }
        }

        match tab_bar_mode {
            ShowTabBar::Stacked => (), // The tab bar was rendered in the content column.
            ShowTabBar::Hidden => {
                // Hide the tab bar, but include a hover area.
                stack.add_positioned_child(
                    self.render_tab_bar_hover_area(),
                    OffsetPositioning::offset_from_parent(
                        Vector2F::zero(),
                        ParentOffsetBounds::WindowByPosition,
                        ParentAnchor::TopLeft,
                        ChildAnchor::TopLeft,
                    ),
                );
            }
        }

        // If the tab bar is being shown in "stacked" mode, we want to render
        // the traffic lights relative to the full workspace, so they appear
        // in the top-right corner even if a right-side panel is open.
        if tab_bar_mode == ShowTabBar::Stacked {
            self.maybe_render_traffic_lights(&mut stack, app);
        }

        if self.current_workspace_state.is_command_search_open {
            if let Some(active_input_handle) = self.get_active_input_view_handle(app) {
                let input_position = app.view(&active_input_handle).save_position_id();
                let menu_positioning = app.view(&self.command_search_view).menu_positioning();
                // Position the CommandSearchView over the active pane's input.
                let search_panel_margin = 4.;
                let positioning = match menu_positioning {
                    MenuPositioning::AboveInputBox => {
                        OffsetPositioning::offset_from_save_position_element(
                            input_position,
                            vec2f(search_panel_margin, -search_panel_margin),
                            PositionedElementOffsetBounds::WindowBySize,
                            PositionedElementAnchor::BottomLeft,
                            ChildAnchor::BottomLeft,
                        )
                    }
                    MenuPositioning::BelowInputBox => {
                        OffsetPositioning::offset_from_save_position_element(
                            input_position,
                            vec2f(search_panel_margin, 0.),
                            PositionedElementOffsetBounds::WindowBySize,
                            PositionedElementAnchor::TopLeft,
                            ChildAnchor::TopLeft,
                        )
                    }
                };

                stack.add_positioned_child(
                    Container::new(ChildView::new(&self.command_search_view).finish())
                        .with_margin_right(search_panel_margin)
                        .finish(),
                    positioning,
                );
            }
        }

        if self.welcome_tips_view_state.is_popup_open() {
            stack.add_child(ChildView::new(&self.welcome_tips_view).finish());
        }

        if self.current_workspace_state.is_palette_open {
            stack.add_overlay_child(ChildView::new(&self.palette).finish());
        }

        if self.current_workspace_state.is_ctrl_tab_palette_open {
            stack.add_child(ChildView::new(&self.ctrl_tab_palette).finish());
        }

        if self.current_workspace_state.is_theme_creator_modal_open {
            stack.add_child(ChildView::new(&self.theme_creator_modal).finish());
        }

        if self.current_workspace_state.is_theme_deletion_modal_open {
            stack.add_child(ChildView::new(&self.theme_deletion_modal).finish());
        }

        if self.launch_config_save_modal.is_open() {
            stack.add_child(self.launch_config_save_modal.render());
        }

        if self.tab_config_params_modal.is_open() {
            stack.add_child(self.tab_config_params_modal.render());
        }

        if self.session_config_modal.is_open() {
            stack.add_child(self.session_config_modal.render());
        }

        if self.should_show_session_config_tab_config_chip() {
            let use_vertical = FeatureFlag::VerticalTabs.is_enabled()
                && *TabSettings::as_ref(app).use_vertical_tabs
                && self.vertical_tabs_panel_open;
            let chip =
                self.render_session_config_tab_config_chip(use_vertical, Appearance::as_ref(app));
            if use_vertical {
                stack.add_positioned_overlay_child(
                    chip,
                    OffsetPositioning::offset_from_save_position_element(
                        vertical_tabs::VERTICAL_TABS_ADD_TAB_POSITION_ID,
                        vec2f(8., -20.),
                        PositionedElementOffsetBounds::WindowByPosition,
                        PositionedElementAnchor::MiddleRight,
                        ChildAnchor::TopLeft,
                    ),
                );
            } else {
                let anchor_id = if FeatureFlag::ShellSelector.is_enabled() {
                    NEW_SESSION_MENU_BUTTON_POSITION_ID
                } else {
                    NEW_TAB_BUTTON_POSITION_ID
                };
                stack.add_positioned_overlay_child(
                    chip,
                    OffsetPositioning::offset_from_save_position_element(
                        anchor_id,
                        vec2f(0., 8.),
                        PositionedElementOffsetBounds::WindowByPosition,
                        PositionedElementAnchor::BottomMiddle,
                        ChildAnchor::TopMiddle,
                    ),
                );
            }
        }

        if self.new_worktree_modal.is_open() {
            stack.add_child(self.new_worktree_modal.render());
        }

        if self.current_workspace_state.is_prompt_editor_open {
            stack.add_child(ChildView::new(&self.prompt_editor_modal).finish());
        }

        if FeatureFlag::AgentToolbarEditor.is_enabled()
            && self.current_workspace_state.is_agent_toolbar_editor_open
        {
            stack.add_child(ChildView::new(&self.agent_toolbar_editor_modal).finish());
        }

        if self.current_workspace_state.is_header_toolbar_editor_open {
            stack.add_child(ChildView::new(&self.header_toolbar_editor_modal).finish());
        }

        if self.current_workspace_state.is_codex_modal_open {
            stack.add_child(ChildView::new(&self.codex_modal).finish());
        }

        if let Some(lightbox_view) = &self.lightbox_view {
            stack.add_child(ChildView::new(lightbox_view).finish());
        }

        if self
            .current_workspace_state
            .is_rewind_confirmation_dialog_open
        {
            stack.add_positioned_overlay_child(
                ChildView::new(&self.rewind_confirmation_dialog).finish(),
                OffsetPositioning::offset_from_parent(
                    Vector2F::zero(),
                    ParentOffsetBounds::WindowByPosition,
                    ParentAnchor::Center,
                    ChildAnchor::Center,
                ),
            );
        }

        if self
            .current_workspace_state
            .is_delete_conversation_confirmation_dialog_open
        {
            stack.add_positioned_overlay_child(
                ChildView::new(&self.delete_conversation_confirmation_dialog).finish(),
                OffsetPositioning::offset_from_parent(
                    Vector2F::zero(),
                    ParentOffsetBounds::WindowByPosition,
                    ParentAnchor::Center,
                    ChildAnchor::Center,
                ),
            );
        }

        if self.current_workspace_state.is_native_quit_modal_open {
            stack.add_positioned_overlay_child(
                ChildView::new(&self.native_modal).finish(),
                OffsetPositioning::offset_from_parent(
                    Vector2F::zero(),
                    ParentOffsetBounds::WindowByPosition,
                    ParentAnchor::Center,
                    ChildAnchor::Center,
                ),
            );
        }

        if self
            .current_workspace_state
            .is_remove_tab_config_dialog_open
        {
            stack.add_positioned_overlay_child(
                ChildView::new(&self.remove_tab_config_confirmation_dialog).finish(),
                OffsetPositioning::offset_from_parent(
                    Vector2F::zero(),
                    ParentOffsetBounds::WindowByPosition,
                    ParentAnchor::Center,
                    ChildAnchor::Center,
                ),
            );
        }

        if FeatureFlag::AvatarInTabBar.is_enabled() && self.is_user_menu_open {
            stack.add_positioned_overlay_child(
                ChildView::new(&self.user_menu).finish(),
                OffsetPositioning::offset_from_save_position_element(
                    USER_AVATAR_BUTTON_POSITION_ID,
                    Vector2F::zero(),
                    PositionedElementOffsetBounds::WindowByPosition,
                    PositionedElementAnchor::BottomRight,
                    ChildAnchor::TopRight,
                ),
            );
        }

        let window_corner_radius = app.windows().window_corner_radius();
        let workspace = Container::new(stack.finish()).with_corner_radius(window_corner_radius);

        let mut stack = Stack::new();
        let theme = appearance.theme();
        let window_settings = WindowSettings::as_ref(app);
        let background_opacity = window_settings
            .background_opacity
            .effective_opacity(self.window_id, app);

        if let Some(img) = util::get_terminal_background_image(app) {
            let opacity_ratio = background_opacity as f32 / 100.;
            stack.add_child(
                Shrinkable::new(
                    1.,
                    Image::new(img.source(), CacheOption::Original)
                        .cover()
                        .with_opacity(opacity_ratio)
                        .with_corner_radius(window_corner_radius)
                        .finish(),
                )
                .finish(),
            );
            stack.add_child(workspace.finish());
        } else {
            stack.add_child(
                workspace
                    .with_background(theme.surface_2().with_opacity(background_opacity))
                    .finish(),
            );
        }

        let input_position_id = self
            .get_active_input_view_handle(app)
            .map(|input| app.view(&input).save_position_id());

        stack.add_positioned_overlay_child(
            ChildView::new(&self.toast_stack).finish(),
            self.global_toast_positioning(),
        );

        if let Some(input_position_id) = input_position_id {
            if FeatureFlag::AvatarInTabBar.is_enabled() && self.is_input_box_visible(app) {
                stack.add_positioned_overlay_child(
                    ChildView::new(&self.update_toast_stack).finish(),
                    self.update_toast_positioning(input_position_id, app),
                );
            }
        }

        #[cfg(target_family = "wasm")]
        if self.show_wasm_nux_dialog {
            stack.add_positioned_overlay_child(
                ChildView::new(&self.wasm_nux_dialog).finish(),
                OffsetPositioning::offset_from_parent(
                    vec2f(-10., 67.),
                    ParentOffsetBounds::WindowByPosition,
                    ParentAnchor::TopRight,
                    ChildAnchor::TopRight,
                ),
            );
        }

        // Add workspace-wide UI event handling.
        let stack = if FeatureFlag::VerticalTabs.is_enabled()
            && *TabSettings::as_ref(app).use_vertical_tabs
            && self.vertical_tabs_panel_open
            // The vertical-tabs detail sidecar can become stale if the pointer moves through a
            // covered region (for example, its scrollbar gutter) and the row/sidecar hoverables
            // do not observe the expected hover-out transition. Install a workspace-root
            // mouse-move observer only while a detail sidecar is active so we can clear that
            // stale visibility without paying this cost during ordinary vertical-tabs usage.
            && self.vertical_tabs_panel.has_active_detail_target()
        {
            // The workspace root uses this handle bundle to compare the live mouse position
            // against the source row rect, sidecar rect, and safe triangle, then hide the
            // sidecar when the pointer has genuinely left all valid keep-open regions.
            let detail_hover_state = self.vertical_tabs_panel.detail_hover_state(self.window_id);
            EventHandler::new(stack.finish())
                .with_always_handle()
                .on_mouse_in(
                    move |ctx, app, position| {
                        if detail_hover_state.reconcile_visibility_for_mouse_position(position, app)
                        {
                            ctx.notify();
                        }
                        DispatchEventResult::PropagateToParent
                    },
                    Some(MouseInBehavior {
                        fire_on_synthetic_events: false,
                        fire_when_covered: true,
                    }),
                )
                .finish()
        } else {
            stack.finish()
        };

        #[cfg_attr(not(any(windows, target_os = "linux")), allow(unused_mut))]
        let mut event_handler = EventHandler::new(stack);

        #[cfg(any(windows, target_os = "linux"))]
        {
            event_handler =
                event_handler.on_scroll_wheel(move |ctx, _app, delta, modifiers_state| {
                    if !modifiers_state.ctrl {
                        return DispatchEventResult::PropagateToParent;
                    }

                    // If the control key is being held, scrolling should scale the zoom level or font size
                    if FeatureFlag::UIZoom.is_enabled() {
                        if delta.y() > 0.0 {
                            ctx.dispatch_typed_action(WorkspaceAction::IncreaseZoom);
                        } else if delta.y() < 0.0 {
                            ctx.dispatch_typed_action(WorkspaceAction::DecreaseZoom);
                        }
                    } else if delta.y() > 0.0 {
                        ctx.dispatch_typed_action(WorkspaceAction::IncreaseFontSize);
                    } else if delta.y() < 0.0 {
                        ctx.dispatch_typed_action(WorkspaceAction::DecreaseFontSize);
                    }
                    DispatchEventResult::StopPropagation
                });
        }

        event_handler.finish()
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            self.focus_active_tab(ctx);
        }
    }

    /// Update this workspace when it has been closed, but may still be restored.
    fn on_window_closed(&mut self, ctx: &mut ViewContext<Self>) {
        for pane_group in self.tab_views() {
            pane_group.update(ctx, |pane_group, ctx| {
                pane_group.detach_panes(ctx);
            });
        }

        let window_id = ctx.window_id();

        WorkspaceRegistry::handle(ctx).update(ctx, |registry, _| {
            registry.unregister(window_id);
        });

        ActiveSession::handle(ctx).update(ctx, |active_session, _| {
            active_session.close_workspace(window_id);
        })
    }
}

fn compute_default_panel_widths(
    app: &AppContext,
    window_id: WindowId,
    has_horizontal_split: bool,
) -> (f32, f32) {
    if let Some(bounds) = app.window_bounds(&window_id) {
        let window_width = bounds.width();
        let left_ratio = 0.15;
        let right_ratio = if has_horizontal_split { 0.3 } else { 0.5 };
        let left = window_width * left_ratio;
        let right = window_width * right_ratio;
        (left, right)
    } else {
        (DEFAULT_LEFT_PANEL_WIDTH, DEFAULT_RIGHT_PANEL_WIDTH)
    }
}

/// Idempotently sets the opencode-warp plugin entry in `~/.config/opencode/opencode.json`.
/// Removes any existing opencode-warp plugin entries (both local file:// and github:) and adds
/// the given `new_entry`. Creates the config file with a default structure if it doesn't exist.
#[cfg(debug_assertions)]
fn set_opencode_warp_plugin(new_entry: &str) -> String {
    let Some(home) = dirs::home_dir() else {
        return "Failed to determine home directory".to_string();
    };

    let config_dir = home.join(".config/opencode");
    let config_path = config_dir.join("opencode.json");

    let mut config: serde_json::Value = if config_path.exists() {
        match std::fs::read_to_string(&config_path) {
            Ok(contents) => match serde_json::from_str(&contents) {
                Ok(val) => val,
                Err(e) => return format!("Failed to parse opencode.json: {e}"),
            },
            Err(e) => return format!("Failed to read opencode.json: {e}"),
        }
    } else {
        serde_json::json!({
            "$schema": "https://opencode.ai/config.json"
        })
    };

    let plugins = config.as_object_mut().and_then(|obj| {
        obj.entry("plugin")
            .or_insert_with(|| serde_json::json!([]))
            .as_array_mut()
    });

    let Some(plugins) = plugins else {
        return "opencode.json has unexpected structure (plugin is not an array)".to_string();
    };

    // Remove any existing opencode-warp entries
    plugins.retain(|entry| {
        let s = entry.as_str().unwrap_or("");
        !s.contains("opencode-warp")
    });

    plugins.push(serde_json::Value::String(new_entry.to_string()));

    if let Err(e) = std::fs::create_dir_all(&config_dir) {
        return format!("Failed to create config directory: {e}");
    }

    match serde_json::to_string_pretty(&config) {
        Ok(json_str) => match std::fs::write(&config_path, format!("{json_str}\n")) {
            Ok(()) => format!("OpenCode plugin set to: {new_entry}"),
            Err(e) => format!("Failed to write opencode.json: {e}"),
        },
        Err(e) => format!("Failed to serialize opencode.json: {e}"),
    }
}
