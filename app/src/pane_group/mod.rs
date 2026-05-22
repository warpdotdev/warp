use std::any::Any;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::SyncSender;
use std::sync::Arc;

use itertools::Itertools;
use lazy_static::lazy_static;
use markdown_parser::FormattedTextFragment;
use parking_lot::FairMutex;
use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::{vec2f, Vector2F};
use serde::{Deserialize, Serialize};
use session_sharing_protocol::common::{
    ParticipantId, Role, RoleRequestId, RoleRequestRejectedReason, RoleRequestResponse, SessionId,
};
use settings::Setting as _;
use tree::DEFAULT_FLEX_VALUE;
use typed_path::TypedPath;
use url::Url;
use uuid::Uuid;
use warp_cli::agent::Harness;
use warp_core::command::ExitCode;
use warp_core::context_flag::ContextFlag;
use warp_terminal::shell::{ShellName, ShellType};
use warp_util::path::convert_wsl_to_windows_host_path;
#[cfg(feature = "local_fs")]
use warp_util::path::LineAndColumnArg;
use warp_util::remote_path::RemotePath;
use warpui::elements::{
    ChildView, Clipped, CrossAxisAlignment, DispatchEventResult, Element, EventHandler, Flex,
    MainAxisSize, ParentElement, Shrinkable, Stack,
};
use warpui::keymap::{Context, EditableBinding, FixedBinding};
use warpui::notification::NotificationSendError;
use warpui::windowing::WindowManager;
use warpui::{
    AppContext, Entity, EntityId, ModelHandle, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle, WeakViewHandle, WindowId,
};

use crate::ai::active_agent_views_model::ActiveAgentViewsModel;
use crate::ai::agent::api::ServerConversationToken;
use crate::ai::agent::conversation::{AIAgentHarness, AIConversation, AIConversationId};
use crate::ai::agent_conversations_model::{
    AgentConversationEntryId, AgentConversationNavigationSubject, AgentConversationsModel,
    AgentConversationsModelEvent,
};
use crate::ai::ai_document_view::AIDocumentView;
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::blocklist::agent_view::AgentViewEntryOrigin;
use crate::ai::blocklist::history_model::CloudConversationData;
use crate::ai::blocklist::inline_action::code_diff_view::CodeDiffView;
use crate::ai::blocklist::suggested_agent_mode_workflow_modal::SuggestedAgentModeWorkflowAndId;
use crate::ai::blocklist::suggested_rule_modal::SuggestedRuleAndId;
#[cfg(not(target_family = "wasm"))]
use crate::ai::blocklist::BlocklistAIHistoryEvent;
use crate::ai::blocklist::{BlocklistAIHistoryModel, InputConfig, SerializedBlockListItem};
use crate::ai::document::ai_document_model::{AIDocumentId, AIDocumentModel, AIDocumentVersion};
use crate::ai::execution_profiles::profiles::{AIExecutionProfilesModel, ClientProfileId};
use crate::ai::llms::LLMId;
use crate::ai::restored_conversations::RestoredAgentConversations;
use crate::ai_assistant::AskAIType;
#[cfg(feature = "local_fs")]
use crate::app_state::CodePaneSnapShot;
use crate::app_state::{
    self, AIFactPaneSnapshot, BranchSnapshot, EnvVarCollectionPaneSnapshot, LeafContents,
    LeafSnapshot, NotebookPaneSnapshot, PaneNodeSnapshot, PaneUuid, SettingsPaneSnapshot,
    TerminalPaneSnapshot, WorkflowPaneSnapshot,
};
use crate::appearance::Appearance;
use crate::auth::auth_manager::AuthManager;
use crate::auth::auth_view_modal::AuthViewVariant;
use crate::auth::AuthStateProvider;
use crate::banner::{Banner, BannerEvent, BannerState, BannerTextContent, DismissalType};
use crate::channel::{Channel, ChannelState};
use crate::cloud_object::Space;
use crate::code::active_file::ActiveFileModel;
use crate::code::buffer_location::LocalOrRemotePath;
#[cfg(feature = "local_fs")]
use crate::code::editor_management::CodeSource;
use crate::code::view::{CodeView, CodeViewAction};
use crate::code_review::comments::{AttachedReviewComment, PendingImportedReviewComment};
use crate::code_review::diff_state::DiffMode;
use crate::drive::items::WarpDriveItemId;
use crate::drive::{CloudObjectTypeAndId, OpenWarpDriveObjectArgs};
use crate::env_vars::EnvVarCollectionType;
use crate::features::FeatureFlag;
use crate::launch_configs::launch_config::{self, PaneMode, PaneTemplateType};
use crate::notebooks::file::FileNotebookView;
use crate::palette::PaletteMode;
use crate::pane_group::focus_state::PaneGroupFocusEvent;
use crate::pane_group::pane::get_started_pane::GetStartedPane;
#[cfg(not(target_family = "wasm"))]
use crate::pane_group::pane::terminal_pane::{
    host_terminal_shared_session_source_type, inherit_share_for_local_child,
};
use crate::pane_group::pane::welcome_pane::WelcomePane;
use crate::pane_group::pane::ActionOrigin;
use crate::persistence::ModelEvent;
use crate::quit_warning::UnsavedStateSummary;
use crate::resource_center::{
    mark_feature_used_and_write_to_user_defaults, Tip, TipAction, TipsCompleted,
};
#[cfg(target_family = "wasm")]
use crate::server::cloud_objects::update_manager::UpdateManager;
use crate::server::ids::{ObjectUid, SyncId};
use crate::server::server_api::{ServerApi, ServerApiProvider};
use crate::server::telemetry::{
    AnonymousUserSignupEntrypoint, PaletteSource, SharingDialogSource, TelemetryEvent,
};
use crate::session_management::SessionNavigationData;
use crate::settings::{AISettings, DefaultSessionMode, PaneSettings};
use crate::settings_view::mcp_servers_page::MCPServersSettingsPage;
use crate::settings_view::SettingsSection;
use crate::shell_indicator::ShellIndicatorType;
use crate::terminal::available_shells::{AvailableShell, AvailableShells};
#[cfg(not(target_family = "wasm"))]
use crate::terminal::cli_agent_sessions::plugin_manager::PluginModalKind;
use crate::terminal::focus_env::add_session_focus_env_vars;
use crate::terminal::general_settings::{GeneralSettings, GeneralSettingsChangedEvent};
#[cfg(feature = "local_tty")]
use crate::terminal::local_tty;
use crate::terminal::model::session::Session;
use crate::terminal::model::terminal_model::ConversationTranscriptViewerStatus;
use crate::terminal::session_settings::{NewSessionSource, SessionSettings};
use crate::terminal::shared_session::render_util::ParticipantAvatarParams;
use crate::terminal::shared_session::role_change_modal::{
    RoleChangeCloseSource, RoleChangeModal, RoleChangeModalEvent,
};
use crate::terminal::shared_session::share_modal::{ShareSessionModal, ShareSessionModalEvent};
use crate::terminal::shared_session::{
    self, IsSharedSessionCreator, SharedSessionActionSource, SharedSessionSource,
};
use crate::terminal::view::inline_banner::{
    ZeroStatePromptSuggestionTriggeredFrom, ZeroStatePromptSuggestionType,
};
use crate::terminal::view::load_ai_conversation::{
    RestoreConversationEntryBehavior, RestoredAIConversation,
};
use crate::terminal::view::ssh_file_upload::FileUploadId;
use crate::terminal::view::{
    BlockNotification, ConversationRestorationInNewPaneType, ExecuteCommandEvent,
    LeftPanelTargetView, SyncEvent, TerminalViewState,
};
use crate::terminal::{
    MockTerminalManager, ShareBlockModal, ShareBlockModalEvent, ShellLaunchData, ShellLaunchState,
    TerminalManager, TerminalModel, TerminalView,
};
use crate::undo_close::{UndoCloseStack, UndoCloseStackEvent};
#[cfg(target_family = "wasm")]
use crate::uri::browser_url_handler::update_browser_url;
use crate::util::bindings::{is_binding_pty_compliant, CustomAction};
#[cfg(feature = "local_fs")]
use crate::util::openable_file_type::FileTarget;
use crate::view_components::ToastFlavor;
use crate::workflows::workflow::Workflow;
use crate::workflows::{WorkflowSelectionSource, WorkflowSource, WorkflowType};
use crate::workspace::{
    self, CommandSearchOptions, PaneViewLocator, TabBarLocation, WorkspaceAction,
};
use crate::{cmd_or_ctrl_shift, report_if_error, send_telemetry_from_ctx};

mod child_agent;
pub mod focus_state;
pub mod pane;
pub mod tree;
pub mod working_directories;
use child_agent::{apply_hidden_child_agent_task_context, HiddenChildAgentTaskContext};
use focus_state::PaneGroupFocusState;

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;

pub use pane::ai_document_pane::AIDocumentPane;
pub use pane::ai_fact_pane::AIFactPane;
pub use pane::code_diff_pane::CodeDiffPane;
pub use pane::code_pane::CodePane;
pub use pane::env_var_collection_pane::EnvVarCollectionPane;
pub use pane::environment_management_pane::EnvironmentManagementPane;
pub use pane::execution_profile_editor_pane::ExecutionProfileEditorPane;
pub use pane::file_pane::FilePane;
pub use pane::network_log_pane::NetworkLogPane;
pub use pane::notebook_pane::NotebookPane;
pub use pane::settings_pane::SettingsPane;
pub use pane::terminal_pane::TerminalPane;
pub use pane::workflow_pane::WorkflowPane;
pub use pane::{
    AnyPaneContent, BackingView, PaneConfiguration, PaneConfigurationEvent, PaneContent, PaneEvent,
    PaneHeaderAction, PaneHeaderCustomAction, PaneId, PaneView, TerminalPaneId,
};
pub use tree::{Direction, PaneData, PaneFlex, PaneNode, SplitDirection};
pub use working_directories::{WorkingDirectoriesEvent, WorkingDirectoriesModel};

use self::pane::{DetachType, PaneViewEvent};
pub use crate::code_review::CodeReviewPanelArg;

lazy_static! {
    // The value to use as the initial window bounds if we are unable to
    // determine them for any reason.
    static ref FALLBACK_INITIAL_WINDOW_SIZE: Vector2F = Vector2F::new(1024., 768.);
}

const MINIMUM_PANE_SIZE: f32 = 50.;
const MINIMUM_PANE_SIZE_UDI: f32 = 190.;
const KEYBOARD_RESIZE_DELTA: f32 = 10.;

type AmbientAgentViewModelHandle =
    ModelHandle<crate::terminal::view::ambient_agent::AmbientAgentViewModel>;

trait AmbientAgentViewModelHandleExt<'a> {
    fn into_optional_handle(self) -> Option<&'a AmbientAgentViewModelHandle>;
}

impl<'a> AmbientAgentViewModelHandleExt<'a> for &'a AmbientAgentViewModelHandle {
    fn into_optional_handle(self) -> Option<&'a AmbientAgentViewModelHandle> {
        Some(self)
    }
}

impl<'a> AmbientAgentViewModelHandleExt<'a> for Option<&'a AmbientAgentViewModelHandle> {
    fn into_optional_handle(self) -> Option<&'a AmbientAgentViewModelHandle> {
        self
    }
}

fn get_minimum_pane_size(app: &AppContext) -> f32 {
    use crate::settings::InputSettings;
    if InputSettings::as_ref(app).is_universal_developer_input_enabled(app) {
        MINIMUM_PANE_SIZE_UDI
    } else {
        MINIMUM_PANE_SIZE
    }
}

/// Resolves a tab config `shell` value (e.g. `"pwsh"` or
/// `"/opt/homebrew/bin/pwsh"`) into an [`AvailableShell`], using the fallback
/// order expected by tab configs:
///
/// 1. If `name` contains a path separator, trust it directly so users can
///    still point at arbitrary binaries.
/// 2. Otherwise look up by command name in the already-discovered
///    [`AvailableShells`]. Its shell discovery supplements the process `PATH`
///    with well-known install locations (e.g. `/opt/homebrew/bin` on macOS,
///    MSYS2/WSL on Windows) that a raw `PATH` lookup would miss when Warp is
///    launched outside an interactive shell.
/// 3. As a final fallback, perform a plain `PATH` lookup via
///    [`AvailableShell::try_from`] in case the user put something exotic in
///    `shell`.
#[cfg(feature = "local_tty")]
fn resolve_tab_config_shell(name: &str, ctx: &AppContext) -> Option<AvailableShell> {
    if name.contains(std::path::MAIN_SEPARATOR) {
        return AvailableShell::try_from(name).ok();
    }

    if let Some(matched) = AvailableShells::as_ref(ctx).find_by_command_name(name) {
        return Some(matched);
    }

    AvailableShell::try_from(name).ok()
}
const WARP_SHELL_COMPATIBILITY_DOCS: &str =
    "https://docs.warp.dev/getting-started/supported-shells";
// Default minimum width for a newly created Agent Mode pane so that it is legible. Called "default"
// because this value may be too large for small windows. In that case, we fall back to 50% of the
// window width.
pub const AGENT_MODE_PANE_DEFAULT_MINIMUM_WIDTH: f32 = 400.;

#[derive(Debug, Clone, Copy)]
pub enum ActivationReason {
    Click,
    Hover,
}

#[derive(Debug, Clone)]
pub enum PaneGroupAction {
    Add(Direction),
    Remove(PaneId),
    RemoveActive,
    Activate(PaneId, ActivationReason),
    ResizeMove(Vector2F),
    StartResizing(DraggedBorder),
    ResetPaneSizes(EntityId),
    Move {
        id: PaneId,
        target_pane_id: PaneId,
        direction: Direction,
    },
    EndResizing,
    ResizeLeft,
    ResizeRight,
    ResizeUp,
    ResizeDown,
    NavigatePrev,
    NavigateNext,
    NavigateLeft,
    NavigateRight,
    NavigateUp,
    NavigateDown,
    ToggleMaximizePane,
    HandleFocusChange,
    FocusTerminalView(EntityId),
}
#[derive(PartialEq)]
enum PaneRemovalReason {
    // This pane is being removed from the pane group because it is being moved to another tab or becoming a tab of its own
    Move,
    // This pane is being removed because it is being closed
    Close,
}

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;
    app.register_binding_validator::<PaneGroup>(is_binding_pty_compliant);

    self::pane::init(app);

    app.register_fixed_bindings([
        // Also create the navigation shortcuts with `meta` in place of `alt`, to accommodate
        // the "Left Option is Meta" and "Right Option is Meta" settings
        FixedBinding::new(
            "cmdorctrl-meta-left",
            PaneGroupAction::NavigateLeft,
            id!("PaneGroup") & !id!("PaneGroup_PaneMaximized") & !id!("PaneGroup_PaneDragging"),
        ),
        FixedBinding::new(
            "cmdorctrl-meta-right",
            PaneGroupAction::NavigateRight,
            id!("PaneGroup") & !id!("PaneGroup_PaneMaximized") & !id!("PaneGroup_PaneDragging"),
        ),
        FixedBinding::new(
            "cmdorctrl-meta-up",
            PaneGroupAction::NavigateUp,
            id!("PaneGroup") & !id!("PaneGroup_PaneMaximized") & !id!("PaneGroup_PaneDragging"),
        ),
        FixedBinding::new(
            "cmdorctrl-meta-down",
            PaneGroupAction::NavigateDown,
            id!("PaneGroup") & !id!("PaneGroup_PaneMaximized") & !id!("PaneGroup_PaneDragging"),
        ),
    ]);

    app.register_editable_bindings([
        EditableBinding::new(
            "pane_group:close_current_session",
            "Close Current Session",
            PaneGroupAction::RemoveActive,
        )
        .with_custom_action(CustomAction::CloseCurrentSession)
        .with_context_predicate(id!("PaneGroup")),
        EditableBinding::new(
            "pane_group:add_left",
            "Split pane left",
            PaneGroupAction::Add(Direction::Left),
        )
        .with_context_predicate(id!("PaneGroup") & !id!("PaneGroup_PaneDragging"))
        .with_custom_action(CustomAction::SplitPaneLeft)
        .with_enabled(|| ContextFlag::CreateNewSession.is_enabled()),
        EditableBinding::new(
            "pane_group:add_up",
            "Split pane up",
            PaneGroupAction::Add(Direction::Up),
        )
        .with_context_predicate(id!("PaneGroup") & !id!("PaneGroup_PaneDragging"))
        .with_custom_action(CustomAction::SplitPaneUp)
        .with_enabled(|| ContextFlag::CreateNewSession.is_enabled()),
        EditableBinding::new(
            "pane_group:navigate_left",
            "Switch panes left",
            PaneGroupAction::NavigateLeft,
        )
        .with_context_predicate(
            id!("PaneGroup") & !id!("PaneGroup_PaneMaximized") & !id!("PaneGroup_PaneDragging"),
        )
        .with_key_binding("cmdorctrl-alt-left"),
        EditableBinding::new(
            "pane_group:navigate_right",
            "Switch panes right",
            PaneGroupAction::NavigateRight,
        )
        .with_context_predicate(
            id!("PaneGroup") & !id!("PaneGroup_PaneMaximized") & !id!("PaneGroup_PaneDragging"),
        )
        .with_key_binding("cmdorctrl-alt-right"),
        EditableBinding::new(
            "pane_group:navigate_up",
            "Switch panes up",
            PaneGroupAction::NavigateUp,
        )
        .with_context_predicate(
            id!("PaneGroup") & !id!("PaneGroup_PaneMaximized") & !id!("PaneGroup_PaneDragging"),
        )
        .with_key_binding("cmdorctrl-alt-up"),
        EditableBinding::new(
            "pane_group:navigate_down",
            "Switch panes down",
            PaneGroupAction::NavigateDown,
        )
        .with_context_predicate(
            id!("PaneGroup") & !id!("PaneGroup_PaneMaximized") & !id!("PaneGroup_PaneDragging"),
        )
        .with_key_binding("cmdorctrl-alt-down"),
    ]);

    // Register bindings to resize a pane. We only set bindings on Mac because there isn't an
    // equivalent binding on Linux/Windows that makes sense here. This matches the behavior of
    // VSCode.
    app.register_editable_bindings([
        EditableBinding::new(
            "pane_group:resize_left",
            "Resize pane > Move divider left",
            PaneGroupAction::ResizeLeft,
        )
        .with_context_predicate(
            id!("PaneGroup") & !id!("PaneGroup_PaneMaximized") & !id!("PaneGroup_PaneDragging"),
        )
        .with_mac_key_binding("cmd-ctrl-left"),
        EditableBinding::new(
            "pane_group:resize_right",
            "Resize pane > Move divider right",
            PaneGroupAction::ResizeRight,
        )
        .with_context_predicate(
            id!("PaneGroup") & !id!("PaneGroup_PaneMaximized") & !id!("PaneGroup_PaneDragging"),
        )
        .with_mac_key_binding("cmd-ctrl-right"),
        EditableBinding::new(
            "pane_group:resize_up",
            "Resize pane > Move divider up",
            PaneGroupAction::ResizeUp,
        )
        .with_context_predicate(
            id!("PaneGroup") & !id!("PaneGroup_PaneMaximized") & !id!("PaneGroup_PaneDragging"),
        )
        .with_mac_key_binding("cmd-ctrl-up"),
        EditableBinding::new(
            "pane_group:resize_down",
            "Resize pane > Move divider down",
            PaneGroupAction::ResizeDown,
        )
        .with_context_predicate(
            id!("PaneGroup") & !id!("PaneGroup_PaneMaximized") & !id!("PaneGroup_PaneDragging"),
        )
        .with_mac_key_binding("cmd-ctrl-down"),
    ]);

    app.register_editable_bindings([
        EditableBinding::new(
            "pane_group:add_down",
            "Split pane down",
            PaneGroupAction::Add(Direction::Down),
        )
        .with_context_predicate(id!("PaneGroup") & !id!("PaneGroup_PaneDragging"))
        .with_custom_action(CustomAction::SplitPaneDown)
        .with_enabled(|| ContextFlag::CreateNewSession.is_enabled()),
        EditableBinding::new(
            "pane_group:add_right",
            "Split pane right",
            PaneGroupAction::Add(Direction::Right),
        )
        .with_context_predicate(id!("PaneGroup") & !id!("PaneGroup_PaneDragging"))
        .with_custom_action(CustomAction::SplitPaneRight)
        .with_enabled(|| ContextFlag::CreateNewSession.is_enabled()),
        EditableBinding::new(
            "pane_group:toggle_maximize_pane",
            "Toggle Maximize Active Pane",
            PaneGroupAction::ToggleMaximizePane,
        )
        .with_context_predicate(id!("PaneGroup") & !id!("PaneGroup_PaneDragging"))
        .with_custom_action(CustomAction::ToggleMaximizePane),
    ]);

    if ChannelState::channel() == Channel::Integration {
        // Hack: Add explicit bindings for the tests, since the tests' injected
        // keypresses won't trigger Mac menu items. Unfortunately we can't use
        // cfg[test] because we are a separate process!
        app.register_fixed_bindings([FixedBinding::new(
            cmd_or_ctrl_shift("w"),
            PaneGroupAction::RemoveActive,
            id!("PaneGroup"),
        )]);
    }
}

pub enum Event {
    AppStateChanged,
    Escape,
    Exited {
        add_to_undo_stack: bool,
    },
    LeftPanelToggled {
        is_open: bool,
    },
    ExecuteCommand(ExecuteCommandEvent),
    PaneTitleUpdated,
    SendNotification {
        notification: BlockNotification,
        pane_id: PaneId,
    },
    OpenSettings(SettingsSection),
    OpenAutoReloadModal {
        purchased_credits: i32,
    },
    AskAIAssistant(AskAIType),
    /// Pass input sync event up from underlying TerminalViews
    /// to the Workspace to sync throughout the window.
    SyncInput(SyncEvent),
    /// Event needs to be propagated up to WorkspaceView where the show command search panel function lives.
    ShowCommandSearch(CommandSearchOptions),
    /// Event used to propagate a state change for one of the terminal views
    /// inside this pane group.
    TerminalViewStateChanged,
    /// Event used to propagate guided onboarding tutorial completion to the workspace.
    OnboardingTutorialCompleted,
    // Tell the workspace to open the workflow modal.
    OpenWorkflowModalWithCommand(String),
    // Tell the workspace to open the workflow for edit.
    OpenCloudWorkflowForEdit(SyncId),
    // Tell the workspace to open the share dialog for the given drive object. The share dialog will
    // open in the index. If the invitee email is provided, it will be added to the share dialog.
    OpenDriveObjectShareDialog {
        cloud_object_type_and_id: CloudObjectTypeAndId,
        invitee_email: Option<String>,
        source: SharingDialogSource,
    },
    // Tell the workspace to open the workflow modal with an unsaved workflow.
    OpenWorkflowModalWithTemporary(Box<Workflow>),
    OpenPromptEditor,
    OpenAgentToolbarEditor,
    OpenCLIAgentToolbarEditor,
    /// tell the workspace to open a file within Warp.
    OpenFileInWarp {
        /// The file path to open.
        path: LocalOrRemotePath,
        /// The session that the path was opened from.
        session: Arc<Session>,
    },
    OpenWarpDriveLink {
        open_warp_drive_args: OpenWarpDriveObjectArgs,
    },
    #[cfg(feature = "local_fs")]
    OpenCodeInWarp {
        source: CodeSource,
        layout: crate::util::file::external_editor::settings::EditorLayout,
        line_col: Option<LineAndColumnArg>,
    },
    #[cfg(feature = "local_fs")]
    PreviewCodeInWarp {
        source: CodeSource,
    },
    OpenCodeDiff {
        view: ViewHandle<CodeDiffView>,
    },
    OpenCodeReviewPane(CodeReviewPanelArg),
    ToggleCodeReviewPane(CodeReviewPanelArg),
    /// Tell the workspace to run a workflow in the active tab's active session.
    RunWorkflow {
        workflow: Arc<WorkflowType>,
        workflow_source: WorkflowSource,
        workflow_selection_source: WorkflowSelectionSource,
        argument_override: Option<HashMap<String, String>>,
    },
    /// Invoke env var from pane
    InvokeEnvVarCollection {
        env_var_collection: Arc<EnvVarCollectionType>,
        in_subshell: bool,
    },
    CloseSharedSessionPaneRequested {
        pane_id: PaneId,
    },
    /// Dirty the workspace so the tab indicator shows.
    MaximizePaneToggled,
    /// A remote server resolved the repo root for a session in this pane group.
    RemoteRepoNavigated {
        remote_path: RemotePath,
    },
    /// Refresh the workspace-level active session state.
    ActiveSessionChanged,
    FocusPaneGroup,
    FocusPane {
        pane_to_focus: PaneId,
    },
    FocusPaneInWorkspace {
        locator: PaneViewLocator,
    },
    ViewInWarpDrive(WarpDriveItemId),
    MoveToSpace {
        cloud_object_type_and_id: CloudObjectTypeAndId,
        space: Space,
    },
    PaneFocused,
    DroppedOnTabBar {
        origin: ActionOrigin,
        pane_id: PaneId,
    },
    /// Switches the focus to the specified tab and moves the given
    /// pane_id into the tab as a hidden pane. This will insert it into the pane
    /// group, but it will not yet render it
    SwitchTabFocusAndMovePane {
        tab_idx: usize,
        pane_id: PaneId,
        /// The axis used for the destination tab's temporary hidden-pane
        /// preview while a cross-tab pane drag is hovering that tab.
        hidden_pane_preview_direction: Direction,
    },
    /// Updates the hovered tab index which will change what preview indicator is displayed
    /// as a header is dragged
    UpdateHoveredTabIndex {
        tab_hover_index: TabBarHoverIndex,
    },
    /// Clears the hovered tab index so it no longer appears as highlighted drop target
    ClearHoveredTabIndex,
    OpenWarpDriveObjectInPane(ObjectUid),
    /// Tell the workspace to open the given child agent conversation in a
    /// fresh tab. Bubbled up by `TerminalView::Event::OpenChildAgentInNewTab`
    /// from the orchestration pill bar's 3-dot menu.
    OpenChildAgentInNewTab {
        conversation_id: AIConversationId,
    },
    OpenSuggestedAgentModeWorkflowModal {
        workflow_and_id: SuggestedAgentModeWorkflowAndId,
    },
    OpenSuggestedRuleModal {
        rule_and_id: SuggestedRuleAndId,
    },
    OpenAIFactCollection {
        /// If set, open the fact collection to the specific rule.
        sync_id: Option<SyncId>,
    },
    AnonymousUserSignup,
    /// Request that the workspace open the command palette.
    OpenPalette {
        mode: PaletteMode,
        source: PaletteSource,
        query: Option<String>,
    },
    /// A terminal pane SSHed into a remote host has initiated a file upload
    /// using a local session.
    FileUploadCommand {
        upload_id: FileUploadId,
        command: String,
        remote_pane_id: TerminalPaneId,
        local_pane_id: TerminalPaneId,
    },
    /// A local terminal pane managing a file upload is requesting a password.
    FileUploadPasswordPending {
        local_pane_id: TerminalPaneId,
    },
    /// A local terminal pane managing a file upload has completed its task.
    FileUploadFinished {
        local_pane_id: TerminalPaneId,
        exit_code: ExitCode,
    },
    OpenFileUploadSession {
        remote_pane_id: TerminalPaneId,
        upload_id: FileUploadId,
    },
    TerminateFileUploadSession {
        remote_pane_id: TerminalPaneId,
        upload_id: FileUploadId,
    },
    ShowToast {
        message: String,
        flavor: ToastFlavor,
        pane_id: Option<PaneId>,
    },
    SignupAnonymousUser {
        entrypoint: AnonymousUserSignupEntrypoint,
    },
    OpenThemeChooser,
    InvalidatedActiveConversation,
    OpenConversationHistory,
    OpenMCPSettingsPage {
        page: Option<MCPServersSettingsPage>,
    },
    OpenAddPromptPane {
        /// The initial prompt body content.
        initial_content: Option<String>,
    },
    OpenAddRulePane,
    OpenEnvironmentManagementPane,
    OpenFilesPalette {
        source: PaletteSource,
    },
    ToggleLeftPanel {
        target_view: LeftPanelTargetView,
        force_open: bool,
    },
    #[cfg(feature = "local_fs")]
    OpenFileWithTarget {
        path: PathBuf,
        target: FileTarget,
        line_col: Option<LineAndColumnArg>,
    },
    /// File was renamed in the file tree
    #[cfg(feature = "local_fs")]
    FileRenamed {
        old_path: PathBuf,
        new_path: PathBuf,
    },
    /// File was deleted in the file tree
    #[cfg(feature = "local_fs")]
    FileDeleted {
        path: PathBuf,
    },
    OpenAgentProfileEditor {
        profile_id: ClientProfileId,
    },
    RepoChanged,
    AttachPathAsContext {
        path: PathBuf,
    },
    AttachPlanAsContext {
        ai_document_id: AIDocumentId,
    },
    CDToDirectory {
        path: PathBuf,
    },
    OpenDirectoryInNewTab {
        path: PathBuf,
    },
    InsertCodeReviewComments {
        repo_path: LocalOrRemotePath,
        comments: Vec<PendingImportedReviewComment>,
        diff_mode: DiffMode,
        open_code_review: Option<CodeReviewPanelArg>,
    },
    OpenCodeReviewPaneAndScrollToComment {
        open_code_review: CodeReviewPanelArg,
        comment: AttachedReviewComment,
        diff_mode: DiffMode,
    },
    ImportAllCodeReviewComments {
        open_code_review: CodeReviewPanelArg,
        comments: Vec<AttachedReviewComment>,
        diff_mode: DiffMode,
    },
    RunTabConfigSkill {
        path: PathBuf,
    },
    /// Request to open LSP logs in a terminal pane
    OpenLspLogs {
        log_path: PathBuf,
    },
    ShowCloudAgentCapacityModal {
        variant: crate::workspace::view::cloud_agent_capacity_modal::CloudAgentCapacityModalVariant,
    },
    FreeTierLimitCheckTriggered,
    #[cfg(not(target_family = "wasm"))]
    OpenPluginInstructionsPane(crate::terminal::CLIAgent, PluginModalKind),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabBarHoverIndex {
    BeforeTab(usize),
    OverTab(usize),
}

#[derive(Debug, Clone, Copy)]
pub struct DraggedBorder {
    border_id: EntityId,
    direction: SplitDirection,
    previous_mouse_location: Vector2F,
}

/// Options that can be set when adding a new local terminal pane.
#[derive(Debug, Default, Clone)]
pub struct NewTerminalOptions {
    /// The particular shell to spawn (if not the default).
    pub shell: Option<AvailableShell>,
    /// An initial working directory for the shell process.
    pub initial_directory: Option<PathBuf>,
    /// Additional environment variables to set in the terminal shell process.
    pub env_vars: HashMap<OsString, OsString>,
    /// If true, do not show the Code Mode homepage UX.
    pub hide_homepage: bool,
    /// Whether or not to start sharing the terminal session as soon as it's ready.
    pub is_shared_session_creator: IsSharedSessionCreator,
    /// The AI conversation to restore when the terminal is created.
    pub conversation_restoration: Option<ConversationRestorationInNewPaneType>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DefaultSessionModeBehavior {
    Apply,
    Ignore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NewPaneVisibility {
    Visible,
    HiddenForMove,
}

#[derive(Debug, Clone, Copy)]
struct AddPaneOptions {
    direction: Direction,
    base_pane_id: Option<PaneId>,
    focus_new_pane: bool,
    visibility: NewPaneVisibility,
    emit_app_state_changed: bool,
}

impl NewTerminalOptions {
    /// Return new options with the initial directory set to `path`.
    pub fn with_initial_directory(mut self, path: impl Into<PathBuf>) -> Self {
        self.initial_directory = Some(path.into());
        self
    }

    /// Returns new options with the initial directory set to `path`. If `path` is None,
    /// the initial directory is cleared.
    pub fn with_initial_directory_opt(mut self, path: Option<PathBuf>) -> Self {
        self.initial_directory = path;
        self
    }

    /// Returns new options with the homepage hidden.
    pub fn with_homepage_hidden(mut self) -> Self {
        self.hide_homepage = true;
        self
    }
}

/// The possible layouts of a pane group.
#[derive(Debug)]
pub enum PanesLayout {
    SingleTerminal(Box<NewTerminalOptions>),
    Snapshot(Box<PaneNodeSnapshot>),
    Template(PaneTemplateType),
    AmbientAgent,
}

impl Default for PanesLayout {
    fn default() -> Self {
        Self::SingleTerminal(Box::default())
    }
}

/// The potential locations where a pane can be dropped, either the tab bar, pane group, or elsewhere in the
/// app.
#[derive(Clone, Debug, Copy, Serialize, Deserialize)]
pub enum PaneDragDropLocation {
    TabBar(TabBarLocation),
    PaneGroup(PaneId),
    Other,
}

pub struct PaneGroup {
    tips_completed: ModelHandle<TipsCompleted>,
    user_default_shell_unsupported_banner_model_handle: ModelHandle<BannerState>,
    model_event_sender: Option<SyncSender<ModelEvent>>,
    panes: PaneData,
    /// Centralized focus state model. Panes subscribe to this to derive their split pane state.
    focus_state: ModelHandle<focus_state::PaneGroupFocusState>,
    pane_history: Vec<PaneId>,
    /// Mapping from pane IDs to their contents.
    pane_contents: HashMap<PaneId, Box<dyn AnyPaneContent>>,

    server_api: Arc<ServerApi>,

    /// The terminal session with an open share block modal. Only terminal panes use the share block modal.
    terminal_with_open_share_block_modal: Option<TerminalPaneId>,

    // We are only holding one instance of share modal view in the pane group and
    // update it with the correct terminal model and size info when triggered by
    // the context menu event.
    share_block_modal: ViewHandle<ShareBlockModal>,
    dragged_border: Option<DraggedBorder>,
    user_default_shell_changed_banner: ViewHandle<Banner<PaneGroupAction>>,

    /// If there is an open share session modal, the pane ID of its terminal. Only terminal panes
    /// use the share session modal. `None` if no share session modal is open.
    terminal_with_open_share_session_modal: Option<TerminalPaneId>,
    share_session_modal: ViewHandle<ShareSessionModal>,

    /// If there is a shared session role change modal open, this is the `TerminalPaneId` of the relevant session. Modal is opened whenever a shared session participant attempts to change a
    /// role. For a viewer when they request a role. For a sharer when they receive a role request,
    /// or when they attempt to grant a role.
    terminal_with_shared_session_role_change_modal_open: Option<TerminalPaneId>,
    /// Parent modal that holds views to role request/response and role grant modals.
    shared_session_role_change_modal: ViewHandle<RoleChangeModal>,
    /// Model that tracks the currently active file.
    active_file_model: ModelHandle<ActiveFileModel>,
    /// If there is an open summarization cancel dialog, the terminal pane ID where summarization is active.
    terminal_with_open_summarization_dialog: Option<TerminalPaneId>,

    /// Pane with an open environment setup mode selector modal (rendered at tab level).
    pane_with_open_environment_setup_mode_selector: Option<PaneId>,
    /// Pane with an open agent-assisted environment modal (rendered at tab level).
    pane_with_open_agent_assisted_environment_modal: Option<PaneId>,

    /// If the left panel is open for this pane group
    pub left_panel_open: bool,
    /// If the right panel is open for this pane group
    pub right_panel_open: bool,
    /// If the right panel is maximized
    pub is_right_panel_maximized: bool,

    /// Ambient agent panes whose task data was not yet cached at restoration time.
    /// Entries are removed as each task's data arrives and the pane is replaced.
    pending_ambient_agent_conversation_restorations: HashMap<AmbientAgentTaskId, PaneId>,

    /// Maps child agent conversation IDs to their hidden pane IDs, so they can
    /// be revealed from the parent's status card.
    child_agent_panes: HashMap<AIConversationId, PaneId>,

    /// Host pane id → child pane ids whose share was auto-created by
    /// `inherit_share_for_local_child`. Used by `StopSharingCurrentSession`
    /// so transitively-shared children don't outlive the host's share.
    /// Excludes cloud-SDK-managed shares (`AmbientAgent` host path).
    transitively_shared_child_panes: HashMap<PaneId, HashSet<PaneId>>,

    /// Set when this pane group hosts a split-off child agent pane that
    /// should be re-adopted by its source group on tab close.
    child_agent_origin: Option<ChildAgentOrigin>,

    /// Tab-level custom title set via the rename-tab flow.
    custom_title: Option<String>,
}

/// Origin metadata for a split-off child agent tab; used to re-adopt the
/// pane back to its source on tab close.
#[derive(Clone)]
pub struct ChildAgentOrigin {
    /// Source pane group; weak so we don't keep the source tab alive.
    pub source_pane_group: WeakViewHandle<PaneGroup>,
    /// The child agent conversation hosted in this tab's lone pane.
    pub conversation_id: AIConversationId,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum PaneState {
    /// This pane is currently focused.
    Focused,
    /// This pane is not focused.
    Unfocused,
    // In split pane with one pane maximized.
    Maximized,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum SplitPaneState {
    InSplitPane(PaneState),
    NotInSplitPane,
}

// Helper to group together certain structs necessary to instantiate a new terminal view.
#[derive(Clone)]
pub struct TerminalViewResources {
    pub tips_completed: ModelHandle<TipsCompleted>,
    pub server_api: Arc<ServerApi>,
    pub model_event_sender: Option<SyncSender<ModelEvent>>,
}

impl SplitPaneState {
    pub fn is_in_split_pane(&self) -> bool {
        match self {
            SplitPaneState::InSplitPane(_) => true,
            SplitPaneState::NotInSplitPane => false,
        }
    }

    /// Is the focused pane.
    pub fn is_focused(&self) -> bool {
        match self {
            SplitPaneState::InSplitPane(state) => match state {
                PaneState::Focused | PaneState::Maximized => true,
                PaneState::Unfocused => false,
            },
            SplitPaneState::NotInSplitPane => true,
        }
    }

    /// Is in split pane and is the focused pane.
    pub fn is_focused_pane(&self) -> bool {
        match self {
            SplitPaneState::InSplitPane(state) => match state {
                PaneState::Focused | PaneState::Maximized => true,
                PaneState::Unfocused => false,
            },
            SplitPaneState::NotInSplitPane => false,
        }
    }

    pub fn is_maximized(&self) -> bool {
        matches!(self, SplitPaneState::InSplitPane(PaneState::Maximized))
    }
}

/// Helper for reconstructing focus state when restoring a pane tree.
/// Focus/active state is stored per-leaf, and must be bubbled up the tree.
#[derive(Default)]
struct InitialFocus {
    focused_pane: Option<PaneId>,
    active_session: Option<TerminalPaneId>,
}

impl InitialFocus {
    fn merge(&mut self, other: InitialFocus) {
        if self.focused_pane.is_some() {
            if other.focused_pane.is_some() {
                log::error!("Restored pane tree has more than one focused pane");
            }
        } else {
            self.focused_pane = other.focused_pane;
        }

        if self.active_session.is_some() {
            if other.active_session.is_some() {
                log::error!("Restored pane tree has more than one active session");
            }
        } else {
            self.active_session = other.active_session;
        }
    }
}

/// Helper for retrieving leftmost pane id when restoring a pane tree.
/// Pane ID is stored per-leaf, and must be bubbled up the tree.
struct LeftmostPaneId {
    pane_id: PaneId,
    session_id: TerminalPaneId,
}

/// The [`InitialLayoutCallback`] provides state to pane group constructors
/// to build the initial layout of the pane group. Specifically, it provides
/// - resources ([`TerminalViewResources`]) to help construct terminal views,
/// - a mutable mapping from [`PaneId`] to [`AnyPaneContent`],
/// - a mutable list of [`PaneId`]s representing the pane history,
/// - the view bounds, and
/// - the mutable view context of the [`PaneGroup`].
/// It expects a return type of [`(PaneData, InitialFocus)`].
type InitialLayoutCallback = Box<
    dyn FnOnce(
        TerminalViewResources,
        &mut HashMap<PaneId, Box<dyn AnyPaneContent>>,
        &mut Vec<PaneId>,
        RectF,
        &mut ViewContext<PaneGroup>,
    ) -> (PaneData, InitialFocus),
>;

/// The restoration path for an ambient agent pane.
enum AmbientRestoreKind {
    /// Active shared session
    SharedSession { session_id: SessionId },
    /// Conversation data isn't loaded yet — show a loading pane and
    /// defer the real restoration to the pending-restoration subscription
    /// (which waits for the data to be loaded async).
    PendingRestoration { task_id: AmbientAgentTaskId },
    /// If there's no task ID to restore, we open a fresh cloud mode pane
    /// (this is a valid state from when a user quits with an empty cloud mode pane).
    NewCloudConversation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AIDocumentPaneVisibilityAction {
    /// Ensure the requested AI document pane is visible.
    ///
    /// If the requested pane is already open, this will keep it open.
    Open,
    /// Toggle visibility of the requested AI document pane.
    ///
    /// If the requested pane is open, this will close it. Otherwise it will open it.
    Toggle,
}

impl PaneGroup {
    /// Executes the provided callback for each TerminalView contained within
    /// this pane group.
    pub fn for_all_terminal_panes(
        &mut self,
        mut callback: impl FnMut(&mut TerminalView, &mut ViewContext<TerminalView>),
        ctx: &mut ViewContext<Self>,
    ) {
        for pane_id in self.pane_contents.keys() {
            if let Some(terminal_view) = self.terminal_view_from_pane_id(*pane_id, ctx) {
                terminal_view.update(ctx, &mut callback);
            }
        }
    }

    /// Executes the provided callback for each CodeView contained within
    /// this pane group.
    pub fn for_all_code_panes(
        &mut self,
        mut callback: impl FnMut(&mut CodeView, &mut ViewContext<CodeView>),
        ctx: &mut ViewContext<Self>,
    ) {
        for pane_id in self.pane_contents.keys() {
            if let Some(code_view) = self.code_view_from_pane_id(*pane_id, ctx) {
                code_view.update(ctx, &mut callback);
            }
        }
    }

    pub fn terminal_pane_ids(&self) -> impl Iterator<Item = PaneId> + '_ {
        self.pane_contents.keys().filter_map(|pane_id| {
            if pane_id.is_terminal_pane() {
                Some(*pane_id)
            } else {
                None
            }
        })
    }

    /// Returns true if this pane group contains any terminal panes.
    pub fn has_terminal_panes(&self) -> bool {
        self.pane_contents
            .keys()
            .any(|pane_id| pane_id.is_terminal_pane())
    }

    /// Returns true if this pane group contains any code panes.
    pub fn has_code_panes(&self) -> bool {
        self.pane_contents
            .keys()
            .any(|pane_id| pane_id.is_code_pane())
    }

    pub fn active_file_model(&self) -> &ModelHandle<ActiveFileModel> {
        &self.active_file_model
    }

    /// Returns true iff one of the terminal panes in this group is being shared.
    pub fn is_terminal_pane_being_shared(&self, ctx: &AppContext) -> bool {
        self.number_of_shared_sessions(ctx) > 0
    }

    pub fn smart_split_direction(
        &self,
        ctx: &mut ViewContext<Self>,
        split_ratio: f32,
    ) -> Direction {
        let size = self.size(ctx);
        // The new width if split horizontally.
        let new_width = size.x() / (self.num_splits_at_root(SplitDirection::Horizontal) + 1) as f32;
        let new_height = size.y();

        if new_width / new_height > split_ratio {
            Direction::Left
        } else {
            Direction::Up
        }
    }

    /// Total size of the pane group.
    pub fn size(&self, ctx: &mut ViewContext<Self>) -> Vector2F {
        self.panes.root.pane_size(ctx)
    }

    /// Number of splits at the root node in the given axis.
    pub fn num_splits_at_root(&self, axis: SplitDirection) -> usize {
        self.panes.root.num_splits_in_direction(axis)
    }

    /// Send a Sync Input event to the TerminalView with EntityId pane_id.
    pub fn send_sync_event_to_session(
        &self,
        terminal_pane_id: TerminalPaneId,
        sync_event: &SyncEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        if let Some(pane_view) = self.terminal_view_from_pane_id(terminal_pane_id, ctx) {
            pane_view.update(ctx, |terminal_view, ctx| {
                terminal_view.receive_sync_input_event(sync_event, ctx);
            });
        }
    }

    fn handle_pane_view_event(
        &mut self,
        pane_id: PaneId,
        event: &PaneViewEvent,
        ctx: &mut ViewContext<PaneGroup>,
    ) {
        if self.pane_contents.contains_key(&pane_id) {
            match event {
                PaneViewEvent::MovePaneWithinPaneGroup {
                    target_id,
                    direction,
                } => {
                    ctx.emit(Event::ClearHoveredTabIndex);
                    self.move_pane(pane_id, *target_id, *direction, ctx);
                }
                PaneViewEvent::DroppedOnTabBar { origin } => {
                    ctx.emit(Event::DroppedOnTabBar {
                        origin: *origin,
                        pane_id,
                    });
                    ctx.emit(Event::ClearHoveredTabIndex);
                }
                PaneViewEvent::DraggedOntoTabBar {
                    origin,
                    tab_hover_index,
                    hidden_pane_preview_direction,
                } => {
                    if matches!(origin, ActionOrigin::Pane) {
                        // Clear hidden closed panes since dragging invalidates undo functionality
                        self.clear_hidden_closed_panes(ctx);

                        match tab_hover_index {
                            TabBarHoverIndex::BeforeTab(_) => {
                                self.hide_pane_for_move(pane_id, ctx);
                            }
                            TabBarHoverIndex::OverTab(tab_idx) => {
                                self.panes.clear_hidden_panes_from_move();
                                ctx.emit(Event::SwitchTabFocusAndMovePane {
                                    tab_idx: *tab_idx,
                                    pane_id,
                                    hidden_pane_preview_direction: *hidden_pane_preview_direction,
                                })
                            }
                        };
                    }

                    ctx.emit(Event::UpdateHoveredTabIndex {
                        tab_hover_index: *tab_hover_index,
                    })
                }

                PaneViewEvent::PaneDraggedOutsideTabBarOrPaneGroup => {
                    // If we drag outside of the tab bar or pane group, ensure that there
                    // is no hidden pane
                    self.panes.clear_hidden_panes_from_move();
                    // Also clear hidden closed panes since dragging invalidates undo functionality
                    self.clear_hidden_closed_panes(ctx);
                    ctx.emit(Event::ClearHoveredTabIndex);
                    ctx.notify();
                    ctx.emit(Event::TerminalViewStateChanged);
                    ctx.emit(Event::AppStateChanged);
                }
                PaneViewEvent::PaneDragEnded => {
                    self.focus_pane_by_id(pane_id, ctx);
                    ctx.emit(Event::TerminalViewStateChanged);
                    ctx.notify();
                }
                PaneViewEvent::PaneHeaderClicked => {
                    self.focus_pane_by_id(pane_id, ctx);
                    ctx.emit(Event::TerminalViewStateChanged);
                    ctx.notify();
                }
            }
        } else {
            log::warn!("Session {pane_id:?} not found");
        }
    }

    /// Send a Sync Input event to every pane in this pane group.
    pub fn send_sync_event_to_panes(&self, sync_event: &SyncEvent, ctx: &mut ViewContext<Self>) {
        for terminal_pane_id in self
            .panes_of::<TerminalPane>()
            .map(|p| p.terminal_pane_id())
        {
            self.send_sync_event_to_session(terminal_pane_id, sync_event, ctx);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn pane_tree_from_template(
        root: PaneTemplateType,
        resources: TerminalViewResources,
        ctx: &mut ViewContext<PaneGroup>,
        pane_contents: &mut HashMap<PaneId, Box<dyn AnyPaneContent>>,
        is_left_pane: bool,
        user_default_shell_unsupported_banner_model_handle: ModelHandle<BannerState>,
        view_size: Vector2F,
        model_event_sender: Option<SyncSender<ModelEvent>>,
    ) -> (PaneData, InitialFocus) {
        let (leftmost_pane_id, pane_data, initial_focus) =
            PaneGroup::pane_tree_from_template_recursive(
                root,
                resources,
                ctx,
                pane_contents,
                is_left_pane,
                user_default_shell_unsupported_banner_model_handle,
                view_size,
                model_event_sender,
            );
        if initial_focus.focused_pane.is_some() && initial_focus.active_session.is_some() {
            (pane_data, initial_focus)
        } else {
            let initial_focus = leftmost_pane_id
                .as_ref()
                .map(|val| InitialFocus {
                    focused_pane: Some(val.pane_id),
                    active_session: Some(val.session_id),
                })
                .unwrap_or_default();
            (pane_data, initial_focus)
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn pane_tree_from_template_recursive(
        root: PaneTemplateType,
        resources: TerminalViewResources,
        ctx: &mut ViewContext<PaneGroup>,
        pane_contents: &mut HashMap<PaneId, Box<dyn AnyPaneContent>>,
        is_left_pane: bool,
        user_default_shell_unsupported_banner_model_handle: ModelHandle<BannerState>,
        view_size: Vector2F,
        model_event_sender: Option<SyncSender<ModelEvent>>,
    ) -> (Option<LeftmostPaneId>, PaneData, InitialFocus) {
        match root {
            PaneTemplateType::PaneTemplate {
                cwd,
                commands,
                is_focused,
                pane_mode,
                shell,
            } => {
                let uuid = Uuid::new_v4();

                #[cfg(feature = "local_tty")]
                let chosen_shell: Option<AvailableShell> = shell
                    .as_deref()
                    .and_then(|name| resolve_tab_config_shell(name, ctx));
                #[cfg(not(feature = "local_tty"))]
                let chosen_shell: Option<AvailableShell> = {
                    let _ = shell;
                    None
                };

                let (view, terminal_manager) = match pane_mode {
                    PaneMode::Cloud => {
                        Self::create_ambient_agent_terminal(resources, view_size, ctx)
                    }
                    PaneMode::Terminal | PaneMode::Agent => PaneGroup::create_session(
                        // Use cwd from the template iff such path exists, otherwise None
                        // TODO(CORE-3187): On Windows, support WSL directory restoration.
                        Some(cwd).filter(|p| p.exists()),
                        HashMap::new(),
                        uuid.as_bytes(),
                        IsSharedSessionCreator::No,
                        resources,
                        None,
                        None, // no conversation restoration for launch config
                        user_default_shell_unsupported_banner_model_handle,
                        view_size,
                        model_event_sender.clone(),
                        chosen_shell,
                        None,
                        ctx,
                    ),
                };

                let has_commands = !commands.is_empty();

                // Runs saved commands on start (terminal and agent modes only).
                if has_commands && !matches!(pane_mode, PaneMode::Cloud) {
                    let command_queue = commands.into_iter().map(|cmd| cmd.exec).collect();
                    view.update(ctx, |terminal, ctx| {
                        terminal.set_pending_command_queue(command_queue, ctx);
                    });
                }

                // Agent mode: enter the agent view. When setup commands are
                // pending (e.g. worktree creation), defer entry until they
                // complete so they run in terminal mode.
                if matches!(pane_mode, PaneMode::Agent) {
                    if !has_commands {
                        view.update(ctx, |terminal_view, ctx| {
                            terminal_view.enter_agent_view_for_new_conversation(
                                None,
                                AgentViewEntryOrigin::Input {
                                    was_prompt_autodetected: false,
                                },
                                ctx,
                            );
                        });
                    } else {
                        view.update(ctx, |terminal_view, _| {
                            terminal_view.set_enter_agent_view_after_pending_commands();
                        });
                    }
                }

                let pane_data = TerminalPane::new(
                    uuid.as_bytes().to_vec(),
                    terminal_manager,
                    view,
                    model_event_sender,
                    ctx,
                );

                let terminal_pane_id = pane_data.terminal_pane_id();
                let pane_id = terminal_pane_id.into();
                pane_contents.insert(pane_id, Box::new(pane_data));

                let is_focused = is_focused.unwrap_or_default();
                let focus = InitialFocus {
                    focused_pane: is_focused.then_some(pane_id),
                    active_session: is_focused.then_some(terminal_pane_id),
                };

                let leftmost_pane_id = is_left_pane.then_some(LeftmostPaneId {
                    pane_id,
                    session_id: terminal_pane_id,
                });
                (leftmost_pane_id, PaneData::new(pane_id), focus)
            }
            PaneTemplateType::PaneBranchTemplate {
                split_direction,
                panes,
            } => {
                let mut len = 0;
                let mut nodes = Vec::new();
                let mut focus = InitialFocus::default();
                let mut leftmost_pane_id = None;
                let pane_flex = 1. / panes.len() as f32;

                let num_children = panes.len() as f32;
                let total_divider_size = tree::get_divider_thickness() * (num_children - 1.);
                let view_size = match split_direction {
                    launch_config::SplitDirection::Vertical => vec2f(
                        view_size.x(),
                        (view_size.y() - total_divider_size) / num_children,
                    ),
                    launch_config::SplitDirection::Horizontal => vec2f(
                        (view_size.x() - total_divider_size) / num_children,
                        view_size.y(),
                    ),
                };

                for (idx, node) in panes.iter().enumerate() {
                    let (child_leftmost_pane_id, child, child_focus) =
                        PaneGroup::pane_tree_from_template_recursive(
                            node.clone(),
                            resources.clone(),
                            ctx,
                            pane_contents,
                            // Focus and activate the leftmost pane of the entire tree.
                            is_left_pane && idx == 0,
                            user_default_shell_unsupported_banner_model_handle.clone(),
                            view_size,
                            model_event_sender.clone(),
                        );
                    len += child.len();
                    nodes.push((PaneFlex(pane_flex), child.root));

                    focus.merge(child_focus);
                    leftmost_pane_id = leftmost_pane_id.or(child_leftmost_pane_id);
                }
                (
                    leftmost_pane_id,
                    PaneData::new_branch(split_direction.into(), nodes, len),
                    focus,
                )
            }
        }
    }

    /// Restores the pane tree with the given snapshot. This returns the restored
    /// pane tree structure as well as the focus state.
    #[allow(clippy::too_many_arguments)]
    fn restore_pane_tree(
        root: PaneNodeSnapshot,
        block_lists: Arc<HashMap<PaneUuid, Vec<SerializedBlockListItem>>>,
        resources: TerminalViewResources,
        ctx: &mut ViewContext<PaneGroup>,
        pane_contents: &mut HashMap<PaneId, Box<dyn AnyPaneContent>>,
        user_default_shell_unsupported_banner_model_handle: ModelHandle<BannerState>,
        view_size: Vector2F,
        model_event_sender: Option<SyncSender<ModelEvent>>,
        deferred_panes: &mut Vec<(PaneId, LeafSnapshot)>,
        pending_ambient_restorations: &mut Vec<(AmbientAgentTaskId, PaneId)>,
    ) -> anyhow::Result<(PaneData, InitialFocus)> {
        match root {
            PaneNodeSnapshot::Leaf(leaf) => Self::restore_pane_leaf(
                leaf,
                block_lists,
                resources,
                ctx,
                pane_contents,
                user_default_shell_unsupported_banner_model_handle,
                view_size,
                model_event_sender,
                deferred_panes,
                pending_ambient_restorations,
            ),
            PaneNodeSnapshot::Branch(pane) => {
                let mut len = 0;
                let mut nodes = Vec::new();
                let mut focus = InitialFocus::default();

                let num_children = pane.children.len() as f32;
                let total_divider_size = tree::get_divider_thickness() * (num_children - 1.);
                let view_size = match pane.direction {
                    app_state::SplitDirection::Vertical => vec2f(
                        view_size.x(),
                        (view_size.y() - total_divider_size) / num_children,
                    ),
                    app_state::SplitDirection::Horizontal => vec2f(
                        (view_size.x() - total_divider_size) / num_children,
                        view_size.y(),
                    ),
                };

                for (flex, node) in pane.children {
                    match PaneGroup::restore_pane_tree(
                        node,
                        block_lists.clone(),
                        resources.clone(),
                        ctx,
                        pane_contents,
                        user_default_shell_unsupported_banner_model_handle.clone(),
                        view_size,
                        model_event_sender.clone(),
                        deferred_panes,
                        pending_ambient_restorations,
                    ) {
                        Ok((child, child_focus)) => {
                            len += child.len();
                            nodes.push((flex.into(), child.root));

                            focus.merge(child_focus);
                        }
                        Err(err) => {
                            log::warn!("Unable to restore child pane: {err:#}");
                        }
                    }
                }

                if nodes.is_empty() {
                    anyhow::bail!("All child panes were invalid");
                }

                let axis = pane.direction;
                Ok((PaneData::new_branch(axis.into(), nodes, len), focus))
            }
        }
    }

    /// Restores a single leaf pane from a snapshot.
    #[allow(clippy::too_many_arguments)]
    fn restore_pane_leaf(
        leaf: LeafSnapshot,
        block_lists: Arc<HashMap<PaneUuid, Vec<SerializedBlockListItem>>>,
        resources: TerminalViewResources,
        ctx: &mut ViewContext<PaneGroup>,
        pane_contents: &mut HashMap<PaneId, Box<dyn AnyPaneContent>>,
        user_default_shell_unsupported_banner_model_handle: ModelHandle<BannerState>,
        view_size: Vector2F,
        model_event_sender: Option<SyncSender<ModelEvent>>,
        #[cfg_attr(not(feature = "local_fs"), allow(unused_variables, clippy::ptr_arg))]
        deferred_panes: &mut Vec<(PaneId, LeafSnapshot)>,
        pending_ambient_restorations: &mut Vec<(AmbientAgentTaskId, PaneId)>,
    ) -> anyhow::Result<(PaneData, InitialFocus)> {
        let custom_vertical_tabs_title = leaf.custom_vertical_tabs_title.clone();
        let result = match leaf.contents {
            LeafContents::AIDocument(_) => {
                // Defer AI document pane restoration until after terminal panes are restored.
                // We do this because the terminal view seeds the AIDocumentModel as part of
                // conversation restoration, and the AIDocumentView requires the data to already
                // exist in the AIDocumentModel. In practice, this will work most of the time
                // because the AIDocumentView is usually in the same tab as the terminal view containing
                // the conversation data.
                // TODO (roland): this is not ideal. If the AIDocumentView is moved to an earlier tab
                // than the terminal view with the data, the data won't exist when the AIDocumentView is restored. Right now
                // the AIDocumentView handles this case and renders with an empty buffer until the data is restored.
                // But if the AIDocumentView is leftover after the terminal view containing the conversation
                // is closed, the data would never be loaded because the conversation is never restored.
                let pane_id = PaneId::deferred_placeholder_pane_id();
                let is_focused = leaf.is_focused;
                deferred_panes.push((pane_id, leaf));
                let focus = InitialFocus {
                    focused_pane: is_focused.then_some(pane_id),
                    active_session: None,
                };
                Ok((PaneData::new(pane_id), focus))
            }
            LeafContents::Terminal(terminal_snapshot) => {
                let uuid = PaneUuid(terminal_snapshot.uuid.clone());
                let block_list = block_lists.get(&uuid);

                let chosen_shell = terminal_snapshot
                    .shell_launch_data
                    .as_ref()
                    .and_then(|shell| {
                        if FeatureFlag::ShellSelector.is_enabled() {
                            AvailableShells::as_ref(ctx).get_from_shell_launch_data(shell)
                        } else {
                            None
                        }
                    });

                let startup_directory = terminal_snapshot
                    .cwd
                    .map(PathBuf::from)
                    .filter(|path| path.is_dir());

                // Filter conversation IDs to only include those that have task messages
                // and are not entirely passive (ignored suggestions).
                // This prevents showing the "Previous session" banner when there's nothing to restore
                // and avoids restoring passive code diffs that the user never acted on.
                let filtered_conversation_ids: Vec<AIConversationId> = terminal_snapshot
                    .conversation_ids_to_restore
                    .iter()
                    .filter(|&conversation_id| {
                        RestoredAgentConversations::handle(ctx).read(ctx, |store, _| {
                            store
                                .get_conversation(conversation_id)
                                .is_some_and(|persisted_conv| {
                                    // Filter conversations that contain no tasks.
                                    if persisted_conv.all_tasks().next().is_none() {
                                        return false;
                                    }

                                    // Filter conversations that are entirely passive.
                                    !persisted_conv.is_entirely_passive()
                                })
                        })
                    })
                    .copied()
                    .collect();

                let conversation_restoration = {
                    let conversations = RestoredAgentConversations::handle(ctx)
                        .update(ctx, |store, _| {
                            store.take_conversations(&filtered_conversation_ids)
                        });
                    vec1::Vec1::try_from_vec(conversations)
                        .ok()
                        .map(
                            |conversations| ConversationRestorationInNewPaneType::Startup {
                                conversations,
                                active_conversation_id: terminal_snapshot.active_conversation_id,
                            },
                        )
                };
                let (terminal_view, terminal_manager) = PaneGroup::create_session(
                    startup_directory,
                    HashMap::new(),
                    uuid.0.as_slice(),
                    IsSharedSessionCreator::No,
                    resources,
                    block_list,
                    conversation_restoration,
                    user_default_shell_unsupported_banner_model_handle,
                    view_size,
                    model_event_sender.clone(),
                    chosen_shell,
                    terminal_snapshot.input_config,
                    ctx,
                );

                let terminal_view_id = terminal_view.id();

                let pane_data = TerminalPane::new(
                    uuid.0,
                    terminal_manager,
                    terminal_view,
                    model_event_sender,
                    ctx,
                );

                let terminal_pane_id = pane_data.terminal_pane_id();
                let pane_id = terminal_pane_id.into();
                pane_contents.insert(pane_id, Box::new(pane_data));

                if let Some(llm_override) = &terminal_snapshot.llm_model_override {
                    if let Ok(llm_id) = serde_json::from_str::<LLMId>(llm_override) {
                        log::info!("Selecting base agent model {llm_id} (from terminal snapshot)");
                        crate::ai::llms::LLMPreferences::handle(ctx).update(
                            ctx,
                            |llm_prefs, ctx| {
                                llm_prefs.update_preferred_agent_mode_llm(
                                    &llm_id,
                                    terminal_view_id,
                                    ctx,
                                );
                            },
                        );
                    }
                }

                if let Some(active_profile_sync_id) = &terminal_snapshot.active_profile_id {
                    log::info!(
                        "Attempting to restore active_profile '{active_profile_sync_id}' for terminal {terminal_view_id:?}"
                    );

                    let profiles_model = AIExecutionProfilesModel::as_ref(ctx);

                    if let Some(profile_id) =
                        profiles_model.get_profile_id_by_sync_id(active_profile_sync_id)
                    {
                        AIExecutionProfilesModel::handle(ctx).update(ctx, |profiles_model, ctx| {
                            profiles_model.set_active_profile(terminal_view_id, profile_id, ctx);
                        });
                        log::info!(
                            "Restored active profile {profile_id:?} for terminal {terminal_view_id:?}"
                        );
                    } else {
                        log::warn!(
                            "Failed to restore active profile for terminal {terminal_view_id:?}"
                        );
                    }
                }

                let focus = InitialFocus {
                    focused_pane: leaf.is_focused.then_some(pane_id),
                    active_session: terminal_snapshot.is_active.then_some(terminal_pane_id),
                };

                Ok((PaneData::new(pane_id), focus))
            }
            LeafContents::Notebook(snapshot) => {
                let pane: Box<dyn AnyPaneContent + 'static> = match snapshot {
                    NotebookPaneSnapshot::CloudNotebook {
                        notebook_id,
                        settings,
                    } => Box::new(NotebookPane::restore(notebook_id, &settings, ctx)?),
                    NotebookPaneSnapshot::LocalFileNotebook { path } => Box::new(FilePane::new(
                        path.map(LocalOrRemotePath::Local),
                        None,
                        #[cfg(feature = "local_fs")]
                        None,
                        ctx,
                    )),
                };

                let pane_id = pane.as_pane().id();
                pane_contents.insert(pane_id, pane);
                let focus = InitialFocus {
                    focused_pane: leaf.is_focused.then_some(pane_id),
                    active_session: None,
                };

                Ok((PaneData::new(pane_id), focus))
            }
            #[cfg(feature = "local_fs")]
            LeafContents::Code(snapshot) => {
                let CodePaneSnapShot::Local {
                    tabs,
                    active_tab_index,
                    source,
                } = snapshot;

                let Some(source) = source.filter(|s: &CodeSource| s.is_restorable()) else {
                    return Err(anyhow::anyhow!(
                        "Skipping code pane with non-restorable source"
                    ));
                };

                let code_view = ctx.add_typed_action_view(move |ctx| {
                    CodeView::restore(&tabs, active_tab_index, source, ctx)
                });
                let pane = CodePane::from_view(code_view, ctx);
                let pane_id = pane.id();
                pane_contents.insert(pane_id, Box::new(pane));
                let focus = InitialFocus {
                    focused_pane: leaf.is_focused.then_some(pane_id),
                    active_session: None,
                };
                Ok((PaneData::new(pane_id), focus))
            }
            #[cfg(not(feature = "local_fs"))]
            LeafContents::Code(_) => Err(anyhow::anyhow!(
                "Code pane restoration not supported on this platform"
            )),
            LeafContents::EnvVarCollection(snapshot) => {
                let pane: Box<dyn AnyPaneContent + 'static> = match snapshot {
                    EnvVarCollectionPaneSnapshot::CloudEnvVarCollection {
                        env_var_collection_id,
                    } => Box::new(EnvVarCollectionPane::restore(env_var_collection_id, ctx)?),
                };

                let pane_id = pane.as_pane().id();
                pane_contents.insert(pane_id, pane);
                let focus = InitialFocus {
                    focused_pane: leaf.is_focused.then_some(pane_id),
                    active_session: None,
                };

                Ok((PaneData::new(pane_id), focus))
            }
            LeafContents::Workflow(snapshot) => {
                let pane: Box<dyn AnyPaneContent + 'static> = match snapshot {
                    WorkflowPaneSnapshot::CloudWorkflow {
                        workflow_id,
                        settings,
                    } => Box::new(WorkflowPane::restore(workflow_id, settings, ctx)?),
                };

                let pane_id = pane.as_pane().id();
                pane_contents.insert(pane_id, pane);
                let focus = InitialFocus {
                    focused_pane: leaf.is_focused.then_some(pane_id),
                    active_session: None,
                };

                Ok((PaneData::new(pane_id), focus))
            }
            LeafContents::Settings(snapshot) => {
                let pane: Box<dyn AnyPaneContent + 'static> = match snapshot {
                    SettingsPaneSnapshot::Local {
                        current_page,
                        search_query,
                    } => Box::new(SettingsPane::new(
                        current_page,
                        search_query.as_deref(),
                        ctx.window_id(),
                        ctx,
                    )),
                };

                let pane_id = pane.as_pane().id();
                pane_contents.insert(pane_id, pane);
                let focus = InitialFocus {
                    focused_pane: leaf.is_focused.then_some(pane_id),
                    active_session: None,
                };
                Ok((PaneData::new(pane_id), focus))
            }
            LeafContents::AIFact(snapshot) => {
                if !FeatureFlag::AIRules.is_enabled() {
                    return Err(anyhow::anyhow!("AI fact pane not enabled"));
                }
                let pane: Box<dyn AnyPaneContent + 'static> = match snapshot {
                    AIFactPaneSnapshot::Personal => Box::new(AIFactPane::new(ctx)),
                };
                let pane_id = pane.as_pane().id();
                pane_contents.insert(pane_id, pane);
                let focus = InitialFocus {
                    focused_pane: leaf.is_focused.then_some(pane_id),
                    active_session: None,
                };
                Ok((PaneData::new(pane_id), focus))
            }
            LeafContents::AmbientAgent(snapshot) => {
                let task_data = snapshot.task_id.map(|task_id| {
                    let task = AgentConversationsModel::handle(ctx).update(ctx, |model, ctx| {
                        model.get_or_async_fetch_task_data(&task_id, ctx)
                    });
                    (task_id, task)
                });

                let restore_kind = match &task_data {
                    Some((task_id, Some(_))) => {
                        match AgentConversationsModel::resolve_open_action(
                            AgentConversationNavigationSubject::Entry(
                                AgentConversationEntryId::AmbientRun(*task_id),
                            ),
                            None,
                            ctx,
                        ) {
                            Some(WorkspaceAction::OpenOrAttachAmbientAgentConversation {
                                session_id,
                                ..
                            }) => AmbientRestoreKind::SharedSession { session_id },
                            // Transcript viewer and other non-session actions depend on conversation metadata from
                            // BlocklistAIHistoryModel, which is loaded asynchronously.
                            // Defer to the pending-restoration handler so it can retry once that metadata arrives.
                            _ => task_data
                                .as_ref()
                                .map(|(tid, _)| AmbientRestoreKind::PendingRestoration {
                                    task_id: *tid,
                                })
                                .unwrap_or(AmbientRestoreKind::NewCloudConversation),
                        }
                    }
                    Some((task_id, None)) => {
                        AmbientRestoreKind::PendingRestoration { task_id: *task_id }
                    }
                    None => AmbientRestoreKind::NewCloudConversation,
                };

                let mut pending_task: Option<AmbientAgentTaskId> = None;
                let (terminal_view, terminal_manager) = match restore_kind {
                    AmbientRestoreKind::SharedSession { session_id } => {
                        Self::create_shared_session_viewer(
                            session_id, resources, view_size, true, ctx,
                        )
                    }
                    AmbientRestoreKind::PendingRestoration { task_id } => {
                        let (view, manager) = Self::create_loading_terminal_manager_and_view(
                            resources,
                            view_size,
                            ctx.window_id(),
                            ctx,
                        );
                        pending_task = Some(task_id);
                        (view, manager)
                    }
                    AmbientRestoreKind::NewCloudConversation => {
                        Self::create_ambient_agent_terminal(resources, view_size, ctx)
                    }
                };

                let pane_data = TerminalPane::new(
                    snapshot.uuid,
                    terminal_manager,
                    terminal_view,
                    model_event_sender,
                    ctx,
                );
                let terminal_pane_id = pane_data.terminal_pane_id();
                let pane_id = terminal_pane_id.into();
                pane_contents.insert(pane_id, Box::new(pane_data));

                if let Some(task_id) = pending_task {
                    // Defer restoration to after the task data is loaded.
                    pending_ambient_restorations.push((task_id, pane_id));
                }

                let focus = InitialFocus {
                    focused_pane: leaf.is_focused.then_some(pane_id),
                    active_session: None,
                };
                Ok((PaneData::new(pane_id), focus))
            }
            LeafContents::CodeReview(_) => {
                Err(anyhow::anyhow!("Code review panes are no longer supported"))
            }
            LeafContents::ExecutionProfileEditor => {
                // We don't yet support restoring execution profile editor panes.
                Err(anyhow::anyhow!(
                    "Can't restore execution profile editor panes"
                ))
            }
            LeafContents::NetworkLog => {
                // Network log panes are intentionally not restored. Two
                // reasons:
                //
                // 1. The in-memory log starts empty on each launch, so a
                //    restored pane would display a blank editor anyway.
                // 2. More importantly, persisting the pane's contents would
                //    effectively regress back to a persisted, on-disk
                //    network log (via the SQLite app-state database) on app
                //    shutdown, defeating the purpose of moving the log off
                //    disk in the first place.
                //
                // `save_pane_state` in `persistence/sqlite.rs` skips network
                // log panes entirely, so reaching this arm indicates a
                // programmer error on the persistence side. Users reopen the
                // pane on demand via Privacy settings or the keybinding.
                Err(anyhow::anyhow!(
                    "Network log pane should not have been persisted, as it cannot be restored"
                ))
            }
            LeafContents::GetStarted => {
                if !FeatureFlag::GetStartedTab.is_enabled() {
                    Err(anyhow::anyhow!("GetStarted pane not supported"))
                } else {
                    let pane: Box<dyn AnyPaneContent + 'static> =
                        Box::new(GetStartedPane::new(ctx));
                    let pane_id = pane.as_pane().id();
                    pane_contents.insert(pane_id, pane);
                    let focus = InitialFocus {
                        focused_pane: leaf.is_focused.then_some(pane_id),
                        active_session: None,
                    };
                    Ok((PaneData::new(pane_id), focus))
                }
            }
            LeafContents::Welcome { startup_directory } => {
                if !FeatureFlag::WelcomeTab.is_enabled() {
                    Err(anyhow::anyhow!("Welcome pane not supported"))
                } else {
                    let pane: Box<dyn AnyPaneContent + 'static> =
                        Box::new(WelcomePane::new(startup_directory, ctx));
                    let pane_id = pane.as_pane().id();
                    pane_contents.insert(pane_id, pane);
                    let focus = InitialFocus {
                        focused_pane: leaf.is_focused.then_some(pane_id),
                        active_session: None,
                    };
                    Ok((PaneData::new(pane_id), focus))
                }
            }
            LeafContents::EnvironmentManagement(_) => {
                // Environment management panes are not restored from persistence.
                // They are opened on-demand via workspace actions.
                Err(anyhow::anyhow!(
                    "Environment management panes are not restored"
                ))
            }
        };

        if let (Ok((pane_data, _)), Some(title)) = (&result, custom_vertical_tabs_title.as_deref())
        {
            if let PaneNode::Leaf(pane_id) = &pane_data.root {
                if let Some(pane) = pane_contents.get(pane_id) {
                    pane.as_pane()
                        .pane_configuration()
                        .update(ctx, |configuration, ctx| {
                            configuration.set_custom_vertical_tabs_title(title, ctx);
                        });
                }
            }
        }

        result
    }

    #[cfg_attr(not(feature = "local_fs"), allow(unused_variables, unused_mut))]
    fn process_deferred_panes(
        deferred_panes: Vec<(PaneId, LeafSnapshot)>,
        mut result: (PaneData, InitialFocus),
        pane_contents: &mut HashMap<PaneId, Box<dyn AnyPaneContent>>,
        ctx: &mut ViewContext<Self>,
    ) -> (PaneData, InitialFocus) {
        for (placeholder_id, leaf) in deferred_panes {
            let custom_vertical_tabs_title = leaf.custom_vertical_tabs_title.clone();
            match leaf.contents {
                LeafContents::AIDocument(aidocument_snapshot) => {
                    match aidocument_snapshot {
                        crate::app_state::AIDocumentPaneSnapshot::Local {
                            document_id,
                            version,
                            content,
                            title,
                        } => {
                            // Parse the document_id from string to AIDocumentId
                            let doc_id = match AIDocumentId::try_from(document_id.as_str()) {
                                Ok(id) => id,
                                Err(err) => {
                                    log::warn!("Failed to parse AI document ID: {err:#}");
                                    continue;
                                }
                            };

                            // Apply persisted SQLite content on top of conversation-restored
                            // content. This handles user edits that weren't part of the
                            // conversation, and the cross-tab edge case where conversation
                            // restoration hasn't run yet.
                            if let Some(persisted_content) = &content {
                                AIDocumentModel::handle(ctx).update(ctx, |model, ctx| {
                                    model.apply_persisted_content(
                                        doc_id,
                                        persisted_content,
                                        title.as_deref(),
                                        ctx,
                                    );
                                });
                            }

                            let doc_version = AIDocumentVersion(version as usize);

                            let document_view = ctx.add_typed_action_view(|view_ctx| {
                                AIDocumentView::new(doc_id, doc_version, view_ctx)
                            });

                            // Create the AIDocumentPane
                            let pane: Box<dyn AnyPaneContent + 'static> =
                                Box::new(AIDocumentPane::new(document_view.clone(), ctx));

                            let real_id = pane.as_pane().id();
                            result.0.replace_pane(placeholder_id, real_id, false);
                            if result.1.focused_pane == Some(placeholder_id) {
                                result.1.focused_pane = Some(real_id);
                            }
                            if let Some(title) = custom_vertical_tabs_title.as_deref() {
                                pane.as_pane().pane_configuration().update(
                                    ctx,
                                    |configuration, ctx| {
                                        configuration.set_custom_vertical_tabs_title(title, ctx);
                                    },
                                );
                            }
                            pane_contents.insert(real_id, pane);
                        }
                    }
                }
                _ => {
                    // Ignore other pane types in deferred processing
                }
            }
        }

        result
    }

    pub fn snapshot_for_node(&self, app: &AppContext, node: &PaneNode) -> PaneNodeSnapshot {
        match node {
            PaneNode::Branch(branch) => {
                let children: Vec<_> = branch
                    .nodes
                    .iter()
                    .filter_map(|(flex, node)| {
                        if let PaneNode::Leaf(pane_id) = node {
                            if self.panes.is_hidden_closed_pane(pane_id) {
                                // Don't snapshot hidden panes (undo, move, job,
                                // child agent, etc.). Child agent panes are
                                // restored lazily once their parent agent view
                                // is re-entered.
                                return None;
                            }
                        }
                        Some((
                            app_state::PaneFlex(flex.0),
                            self.snapshot_for_node(app, node),
                        ))
                    })
                    .collect();

                PaneNodeSnapshot::Branch(BranchSnapshot {
                    direction: branch.axis().into(),
                    children,
                })
            }
            PaneNode::Leaf(pane_id) => {
                // If this leaf is the replacement side of an active swap,
                // persist the original instead. The swap is UX-only and
                // the replacement (a child agent pane) is rebuilt off-tree
                // on restart.
                let snapshot_pane_id = self
                    .panes
                    .original_pane_for_replacement(*pane_id)
                    .unwrap_or(*pane_id);
                let is_substituted = snapshot_pane_id != *pane_id;
                // Did the visible leaf hold the active session at snapshot
                // time? On restore the original takes the slot, so it
                // should inherit the active-session marker.
                let visible_leaf_is_active_session =
                    pane_id.as_terminal_pane_id() == self.active_session_id(app);
                let mut contents = match self.pane_contents.get(&snapshot_pane_id) {
                    Some(pane) => pane.as_pane().snapshot(app),
                    None => {
                        // Create a new pane uuid if we have a bug where we didn't save it
                        // properly. This approach will allow us to keep the uniqueness constraints
                        // intact so we don't fail to save the snapshot.
                        log::error!("Failed to get session data for pane, so used a new uuid");
                        LeafContents::Terminal(TerminalPaneSnapshot {
                            uuid: Uuid::new_v4().as_bytes().to_vec(),
                            cwd: None,
                            is_active: visible_leaf_is_active_session,
                            is_read_only: false,
                            shell_launch_data: None,
                            input_config: Some(InputConfig::new(app)),
                            llm_model_override: None,
                            active_profile_id: None,
                            conversation_ids_to_restore: Vec::new(),
                            active_conversation_id: None,
                        })
                    }
                };

                // After substitution, propagate the visible leaf's
                // active-session bit so restore focuses the right pane.
                if is_substituted && visible_leaf_is_active_session {
                    if let LeafContents::Terminal(ref mut snapshot) = contents {
                        snapshot.is_active = true;
                    }
                }
                let custom_vertical_tabs_title =
                    self.pane_contents.get(&snapshot_pane_id).and_then(|pane| {
                        pane.as_pane()
                            .pane_configuration()
                            .as_ref(app)
                            .custom_vertical_tabs_title()
                            .map(str::to_owned)
                    });
                PaneNodeSnapshot::Leaf(LeafSnapshot {
                    // Focus is tracked against the visible leaf, not the
                    // substituted original.
                    is_focused: *pane_id == self.focused_pane_id(app),
                    custom_vertical_tabs_title,
                    contents,
                })
            }
        }
    }

    /// Find the PaneId for a given TerminalView EntityId if it exists within this PaneGroup.
    pub fn find_pane_id_for_terminal_view(
        &self,
        terminal_view_id: EntityId,
        ctx: &AppContext,
    ) -> Option<PaneId> {
        for pane_id in self.pane_contents.keys() {
            if let Some(terminal_pane) = self.downcast_pane_by_id::<TerminalPane>(*pane_id) {
                if terminal_pane.terminal_view(ctx).id() == terminal_view_id {
                    return Some(*pane_id);
                }
            }
        }
        None
    }

    pub fn focused_pane_id(&self, ctx: &AppContext) -> PaneId {
        self.focus_state
            .read(ctx, |state, _| state.focused_pane_id())
    }

    pub fn active_session_id(&self, ctx: &AppContext) -> Option<TerminalPaneId> {
        self.focus_state
            .read(ctx, |state, _| state.active_session_id())
    }

    pub fn focus_state_handle(&self) -> ModelHandle<PaneGroupFocusState> {
        self.focus_state.clone()
    }

    pub fn snapshot(&self, app: &AppContext) -> PaneNodeSnapshot {
        self.snapshot_for_node(app, &self.panes.root)
    }

    fn panes_of<T: Any>(&self) -> impl Iterator<Item = &'_ T> {
        self.pane_contents
            .values()
            .filter_map(|contents| contents.as_any().downcast_ref::<T>())
    }

    /// Checks if any TerminalView within this pane group matches the given ID.
    pub fn contains_terminal_view(&self, terminal_view_id: EntityId, ctx: &AppContext) -> bool {
        self.panes_of::<TerminalPane>()
            .any(|pane| pane.terminal_view(ctx).id() == terminal_view_id)
    }

    /// Returns the [`PaneId`] of the terminal pane whose persistent UUID matches
    /// the given bytes, or `None` if no such pane exists in this group.
    pub fn find_terminal_pane_by_session_uuid(&self, uuid: &[u8]) -> Option<PaneId> {
        self.panes_of::<TerminalPane>()
            .find(|pane| pane.session_uuid() == uuid && !self.is_pane_hidden_for_close(pane.id()))
            .map(|pane| pane.id())
    }

    /// Iterate over the code editors in this pane group.
    pub fn code_panes<'a>(
        &'a self,
        app: &'a AppContext,
    ) -> impl Iterator<Item = (PaneId, ViewHandle<CodeView>)> + 'a {
        self.panes_of::<CodePane>()
            .map(move |pane| (pane.id(), pane.file_view(app)))
    }

    pub fn ai_document_panes(&self) -> impl Iterator<Item = PaneId> + '_ {
        self.panes_of::<AIDocumentPane>().map(|pane| pane.id())
    }

    fn visible_ai_document_panes(&self, ctx: &AppContext) -> Vec<(PaneId, AIDocumentId)> {
        self.panes_of::<AIDocumentPane>()
            .filter(|pane| !self.is_pane_hidden_for_close(pane.id()))
            .map(|pane| {
                let document_view = pane.document_view(ctx);
                (pane.id(), *document_view.as_ref(ctx).document_id())
            })
            .collect()
    }

    fn close_panes(&mut self, pane_ids: Vec<PaneId>, ctx: &mut ViewContext<Self>) {
        for pane_id in pane_ids {
            self.close_pane(pane_id, ctx);
        }
    }

    /// Checks if this pane group contains a visible AI document pane with the given document ID.
    pub fn contains_ai_document(&self, document_id: &AIDocumentId, ctx: &AppContext) -> bool {
        self.panes_of::<AIDocumentPane>()
            .filter(|pane| !self.is_pane_hidden_for_close(pane.id()))
            .any(|pane| *pane.document_view(ctx).as_ref(ctx).document_id() == *document_id)
    }

    /// Closes all visible AI document panes that are *not* for `document_id`, then applies the
    /// requested `action` to the pane for `document_id`.
    ///
    /// This enforces the UI invariant that only one AI document pane should be visible at a time.
    fn set_ai_document_pane_visibility(
        &mut self,
        conversation_id: AIConversationId,
        document_id: AIDocumentId,
        document_version: AIDocumentVersion,
        action: AIDocumentPaneVisibilityAction,
        ctx: &mut ViewContext<Self>,
    ) {
        // Snapshot currently-visible AI document panes so we can make decisions without
        // mutating the pane tree mid-iteration.
        let visible_panes = self.visible_ai_document_panes(ctx);

        // Is the requested document already visible?
        let is_target_visible = visible_panes
            .iter()
            .any(|(_, visible_document_id)| *visible_document_id == document_id);

        // Decide which panes to close and whether we should open the target pane.
        let (pane_ids_to_close, should_open_target) = if is_target_visible {
            match action {
                // Keep the requested document open; close any other AI document panes.
                AIDocumentPaneVisibilityAction::Open => (
                    visible_panes
                        .into_iter()
                        .filter(|(_, visible_document_id)| *visible_document_id != document_id)
                        .map(|(pane_id, _)| pane_id)
                        .collect(),
                    false,
                ),
                // Toggle semantics: if the requested document is already visible, close it (and
                // close any other AI document panes as well).
                AIDocumentPaneVisibilityAction::Toggle => (
                    visible_panes
                        .into_iter()
                        .map(|(pane_id, _)| pane_id)
                        .collect(),
                    false,
                ),
            }
        } else {
            // The requested document isn't visible. Regardless of action, close any open AI document
            // panes first (replacement semantics), then open the requested document.
            (
                visible_panes
                    .into_iter()
                    .map(|(pane_id, _)| pane_id)
                    .collect(),
                true,
            )
        };

        self.close_panes(pane_ids_to_close, ctx);

        if !should_open_target {
            return;
        }

        // Find terminal view via document -> conversation -> terminal view.
        let terminal_view = BlocklistAIHistoryModel::as_ref(ctx)
            .terminal_view_id_for_conversation(&conversation_id)
            .and_then(|terminal_view_id| {
                // Find the pane containing this terminal view.
                self.pane_contents.keys().find_map(|pane_id| {
                    self.terminal_view_from_pane_id(*pane_id, ctx)
                        .filter(|tv| tv.id() == terminal_view_id)
                })
            });

        // Unmaximize the current pane first so the new document pane is visible.
        if self.is_focused_pane_maximized(ctx) {
            self.toggle_maximize_pane(ctx);
        }

        // Construct and show the document pane.
        let document_view = ctx
            .add_typed_action_view(|ctx| AIDocumentView::new(document_id, document_version, ctx));

        document_view.update(ctx, |view, _| {
            view.set_original_terminal_view(terminal_view.clone());
        });
        let pane = AIDocumentPane::new(document_view, ctx);

        self.add_pane_with_direction(Direction::Right, pane, false, ctx);
    }

    /// Closes any other ai document panes, and opens the specified document_id.
    pub fn open_ai_document_pane(
        &mut self,
        conversation_id: AIConversationId,
        document_id: AIDocumentId,
        document_version: AIDocumentVersion,
        ctx: &mut ViewContext<Self>,
    ) {
        self.set_ai_document_pane_visibility(
            conversation_id,
            document_id,
            document_version,
            AIDocumentPaneVisibilityAction::Open,
            ctx,
        );
    }

    pub fn close_all_ai_document_panes(&mut self, ctx: &mut ViewContext<Self>) {
        let pane_ids: Vec<_> = self
            .visible_ai_document_panes(ctx)
            .into_iter()
            .map(|(pane_id, _)| pane_id)
            .collect();
        self.close_panes(pane_ids, ctx);
    }

    pub fn toggle_ai_document_pane(
        &mut self,
        conversation_id: AIConversationId,
        document_id: AIDocumentId,
        document_version: AIDocumentVersion,
        ctx: &mut ViewContext<Self>,
    ) {
        self.set_ai_document_pane_visibility(
            conversation_id,
            document_id,
            document_version,
            AIDocumentPaneVisibilityAction::Toggle,
            ctx,
        );
    }

    pub fn has_active_code_pane_with_unsaved_changes(&self, ctx: &AppContext) -> bool {
        self.focused_pane_id(ctx).is_code_pane()
            && self
                .pane_contents
                .get(&self.focused_pane_id(ctx))
                .and_then(|content| content.as_any().downcast_ref::<CodePane>())
                .map(|pane| {
                    pane.file_view(ctx)
                        .as_ref(ctx)
                        .active_tab_has_unsaved_changes(ctx)
                })
                .unwrap_or(false)
    }

    /// Returns the selected text from the focused pane, or `None` if there is no selection or the selection is empty.
    pub fn selected_text_from_focused_pane(&self, ctx: &AppContext) -> Option<String> {
        let focused_pane_id = self.focused_pane_id(ctx);

        #[cfg(feature = "local_fs")]
        {
            // If the focused pane is a code pane, return the selected text from the code view.
            if focused_pane_id.is_code_pane() {
                let text = self
                    .downcast_pane_by_id::<CodePane>(focused_pane_id)
                    .and_then(|pane| pane.file_view(ctx).as_ref(ctx).selected_text(ctx));
                // If the text is not empty and does not contain a newline, return early.
                if text.as_ref().is_some_and(|t| !t.is_empty()) {
                    return text;
                }
            }
        }

        // Finds the active pane type outof (NotebookPane, AIDocumentPane, TerminalPane)
        // and extracts selected text from it.
        let text = if let Some(pane) = self.downcast_pane_by_id::<NotebookPane>(focused_pane_id) {
            pane.notebook_view(ctx).as_ref(ctx).selected_text(ctx)
        } else if let Some(pane) = self.downcast_pane_by_id::<AIDocumentPane>(focused_pane_id) {
            pane.document_view(ctx).as_ref(ctx).selected_text(ctx)
        } else if let Some(terminal_view) = self.terminal_view_from_pane_id(focused_pane_id, ctx) {
            // NOTE: We currently don't have a way to track recency of selection events.
            // In lieu of this, we prefer selections to the input editor over the terminal view.
            // TODO(vkodithala): Once we have a way to track recency of selection events, we should use that instead.
            terminal_view
                .as_ref(ctx)
                .selected_text_from_input(ctx)
                .or_else(|| terminal_view.as_ref(ctx).selected_text(ctx))
        } else {
            None
        };

        text.filter(|text: &String| !text.is_empty())
    }

    /// Iterate over the terminal sessions in this pane group.
    pub fn pane_sessions<'a>(
        &'a self,
        pane_group_id: EntityId,
        window_id: WindowId,
        app: &'a AppContext,
    ) -> impl Iterator<Item = SessionNavigationData> + 'a {
        self.panes_of::<TerminalPane>()
            .map(move |pane| pane.session_navigation_data(pane_group_id, window_id, app))
    }

    /// Send prompt change bindkey events to all terminal sessions in this pane group. This
    /// is used for intra-session prompt switching between Warp prompt and PS1.
    #[cfg_attr(not(feature = "local_tty"), allow(unused_variables))]
    pub fn send_prompt_change_bindkey_to_all_sessions(
        &self,
        honor_ps1: bool,
        app: &mut AppContext,
    ) {
        self.panes_of::<TerminalPane>()
            .for_each(move |session_data| {
                #[cfg(feature = "local_tty")]
                {
                    session_data
                        .terminal_manager(app)
                        .update(app, |terminal_manager, ctx| {
                            if let Some(manager) = terminal_manager
                                .as_any()
                                .downcast_ref::<local_tty::TerminalManager>()
                            {
                                if honor_ps1 {
                                    manager.send_switch_to_ps1_bindkey(ctx);
                                } else {
                                    manager.send_switch_to_warp_prompt_bindkey(ctx);
                                }
                            }
                        });
                }
                // TODO: Potentially handle remote_tty and mock TerminalManager cases here as well?
            });
    }

    /// Returns the most recent state across this pane group's terminal views.
    pub fn most_recent_pane_state(&self, ctx: &AppContext) -> TerminalViewState {
        let (_, most_recent_state) = self
            .pane_contents
            .iter()
            .filter_map(|(pane_id, pane_content)| {
                // Skip panes that are hidden for undo close
                if self.panes.is_hidden_closed_pane(pane_id) {
                    return None;
                }

                // Only consider terminal panes
                pane_content.as_any().downcast_ref::<TerminalPane>()
            })
            .filter_map(|session_data| {
                let state_change = session_data.terminal_view(ctx).as_ref(ctx).current_state();
                (!matches!(state_change.state, TerminalViewState::Normal)).then_some(state_change)
            })
            .fold(
                (None, TerminalViewState::Normal),
                |(timestamp, current_state), state_change| {
                    if timestamp < Some(state_change.timestamp) {
                        (Some(state_change.timestamp), state_change.state)
                    } else {
                        (timestamp, current_state)
                    }
                },
            );

        most_recent_state
    }

    fn open_share_session_modal(
        &mut self,
        terminal_pane_id: TerminalPaneId,
        open_source: SharedSessionActionSource,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(terminal_view) = self.terminal_view_from_pane_id(terminal_pane_id, ctx) else {
            log::warn!("Tried to open share session modal for non-existent terminal pane");
            return;
        };

        if AuthStateProvider::as_ref(ctx)
            .get()
            .is_anonymous_or_logged_out()
        {
            AuthManager::handle(ctx).update(ctx, |auth_manager, ctx| {
                auth_manager.attempt_login_gated_feature(
                    "Share Session",
                    AuthViewVariant::ShareRequirementCloseable,
                    ctx,
                )
            });
            return;
        }

        self.share_session_modal.update(ctx, |modal, ctx| {
            modal.open(
                terminal_pane_id,
                open_source,
                terminal_view.as_ref(ctx).model.clone(),
                terminal_view.id(),
                ctx,
            );
        });
        self.terminal_with_open_share_session_modal = Some(terminal_pane_id);
        ctx.focus(&self.share_session_modal);
        ctx.notify();
    }

    fn open_share_session_denied_modal(
        &mut self,
        terminal_pane_id: TerminalPaneId,
        ctx: &mut ViewContext<Self>,
    ) {
        self.share_session_modal.update(ctx, |modal, ctx| {
            modal.open_denied(terminal_pane_id, ctx);
        });
        self.terminal_with_open_share_session_modal = Some(terminal_pane_id);
        ctx.focus(&self.share_session_modal);
        ctx.notify();
    }

    /// Closes the share session modal if it is open. Does nothing otherwise. Does not change
    /// which element is focused.
    fn close_share_session_modal(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(terminal_pane_id) = self.terminal_with_open_share_session_modal.take() else {
            return;
        };

        if let Some(terminal_view) = self.terminal_view_from_pane_id(terminal_pane_id, ctx) {
            terminal_view.update(ctx, |view, ctx| {
                view.set_show_pane_accent_border(false, ctx)
            });
        }
        ctx.notify();
    }

    fn handle_share_session_modal_event(
        &mut self,
        event: &ShareSessionModalEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            ShareSessionModalEvent::Close => {
                let Some(terminal_pane_id) = self.terminal_with_open_share_session_modal.take()
                else {
                    return;
                };

                if let Some(pane) = self.focused_pane_content(ctx) {
                    pane.focus(ctx);
                }

                if let Some(terminal_view) = self.terminal_view_from_pane_id(terminal_pane_id, ctx)
                {
                    terminal_view.update(ctx, |view, ctx| {
                        view.set_show_pane_accent_border(false, ctx)
                    });
                }
                ctx.notify();
            }
            ShareSessionModalEvent::StartSharing {
                terminal_pane_id,
                scrollback_type,
                source,
            } => {
                self.terminal_with_open_share_session_modal = None;
                ctx.notify();

                let Some(terminal_view) = self.terminal_view_from_pane_id(*terminal_pane_id, ctx)
                else {
                    return;
                };

                terminal_view.update(ctx, |view, ctx| {
                    let share_source = SharedSessionSource::user(
                        view.active_conversation_task_id(ctx).map(|t| t.to_string()),
                    );
                    view.attempt_to_share_session(
                        *scrollback_type,
                        Some(*source),
                        share_source,
                        false,
                        ctx,
                    );
                });
            }
            ShareSessionModalEvent::Upgrade => {
                self.terminal_with_open_share_session_modal = None;
                if let Some(pane) = self.focused_pane_content(ctx) {
                    pane.focus(ctx);
                }
                ctx.emit(Event::OpenSettings(SettingsSection::Teams));
                ctx.notify();

                send_telemetry_from_ctx!(TelemetryEvent::SharedSessionModalUpgradePressed, ctx);
            }
        }
    }

    fn open_shared_session_viewer_request_modal(
        &mut self,
        terminal_pane_id: TerminalPaneId,
        role: Role,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(terminal_view) = self.terminal_view_from_pane_id(terminal_pane_id, ctx) else {
            log::warn!("Tried to open role request modal for non-existent terminal pane");
            return;
        };

        let Some(presence_manager) =
            terminal_view.read(ctx, |view, _| view.shared_session_presence_manager())
        else {
            log::warn!("Tried to open role request modal for non-existent presence manager");
            return;
        };

        let Some(sharer) = presence_manager.as_ref(ctx).get_sharer() else {
            log::warn!("Tried to open role request modal with non-existent sharer");
            return;
        };

        let display_name = sharer.info.profile_data.display_name.clone();
        self.shared_session_role_change_modal
            .update(ctx, |modal, ctx| {
                modal.open_for_viewer_request(terminal_pane_id, display_name, role, ctx);
            });

        self.terminal_with_shared_session_role_change_modal_open = Some(terminal_pane_id);
        ctx.focus(&self.shared_session_role_change_modal);
        ctx.notify();
    }

    /// If modal is already open, we update it with the new role request
    fn open_shared_session_sharer_response_modal(
        &mut self,
        terminal_pane_id: TerminalPaneId,
        viewer_id: ParticipantId,
        role_request_id: RoleRequestId,
        role: Role,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(terminal_view) = self.terminal_view_from_pane_id(terminal_pane_id, ctx) else {
            log::warn!("Tried to open role request modal for non-existent terminal pane");
            return;
        };

        let Some(presence_manager) =
            terminal_view.read(ctx, |view, _| view.shared_session_presence_manager())
        else {
            log::warn!("Tried to open role request modal for non-existent presence manager");
            return;
        };

        let Some(participant) = presence_manager.as_ref(ctx).get_participant(&viewer_id) else {
            log::warn!("Tried to open role request modal with non-existent participant");
            return;
        };

        let params = ParticipantAvatarParams::new(participant, false);
        let firebase_uid = participant.info.profile_data.firebase_uid.clone();
        self.shared_session_role_change_modal
            .update(ctx, |modal, ctx| {
                modal.open_for_sharer_response(
                    terminal_pane_id,
                    viewer_id,
                    firebase_uid,
                    role_request_id,
                    params,
                    role,
                    ctx,
                );
            });

        self.terminal_with_shared_session_role_change_modal_open = Some(terminal_pane_id);
        ctx.focus(&self.shared_session_role_change_modal);
        ctx.notify();
    }

    fn open_shared_session_sharer_grant_modal(
        &mut self,
        terminal_pane_id: TerminalPaneId,
        participant_id: ParticipantId,
        ctx: &mut ViewContext<Self>,
    ) {
        self.shared_session_role_change_modal
            .update(ctx, |modal, ctx| {
                modal.open_for_sharer_grant(terminal_pane_id, participant_id, ctx);
            });
        self.terminal_with_shared_session_role_change_modal_open = Some(terminal_pane_id);
        ctx.focus(&self.shared_session_role_change_modal);
        ctx.notify();
    }

    /// Closes the parent shared session role change modal if it is open. Does nothing otherwise.
    fn close_shared_session_role_change_modal(
        &mut self,
        source: RoleChangeCloseSource,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(terminal_pane_id) = self
            .terminal_with_shared_session_role_change_modal_open
            .take()
        else {
            return;
        };

        let should_close_modal = self
            .shared_session_role_change_modal
            .update(ctx, |modal, ctx| {
                match source {
                    RoleChangeCloseSource::ViewerRequest => modal.close_for_viewer_request(ctx),
                    RoleChangeCloseSource::SharerResponse => modal.close_for_sharer_response(ctx),
                    RoleChangeCloseSource::SharerGrant => modal.close_for_sharer_grant(ctx),
                }

                modal.all_child_modals_are_closed()
            });

        if should_close_modal {
            if let Some(terminal_view) = self.terminal_view_from_pane_id(terminal_pane_id, ctx) {
                terminal_view.update(ctx, |view, ctx| {
                    view.set_show_pane_accent_border(false, ctx)
                });
            }
            if let Some(pane) = self.focused_pane_content(ctx) {
                pane.focus(ctx);
            }
        }
        ctx.notify();
    }

    fn remove_shared_session_role_request(
        &mut self,
        role_request_id: RoleRequestId,
        ctx: &mut ViewContext<Self>,
    ) {
        self.shared_session_role_change_modal
            .update(ctx, |modal, ctx| {
                modal.remove_role_request(role_request_id, ctx);
            });
    }

    fn set_shared_session_role_change_modal_request_id(
        &mut self,
        role_request_id: RoleRequestId,
        ctx: &mut ViewContext<Self>,
    ) {
        self.shared_session_role_change_modal
            .update(ctx, |modal, _| {
                modal.set_role_request_id(role_request_id);
            });
    }

    fn handle_shared_session_role_change_modal_event(
        &mut self,
        event: &RoleChangeModalEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            RoleChangeModalEvent::CancelRequest {
                terminal_pane_id,
                role_request_id,
            } => {
                self.close_shared_session_role_change_modal(
                    RoleChangeCloseSource::ViewerRequest,
                    ctx,
                );
                if let Some(terminal_view) = self.terminal_view_from_pane_id(*terminal_pane_id, ctx)
                {
                    terminal_view.update(ctx, |view, ctx| {
                        view.cancel_shared_session_role_request(role_request_id.clone(), ctx)
                    });
                }
                ctx.notify();
            }
            RoleChangeModalEvent::ApproveRequest {
                terminal_pane_id,
                participant_id,
                role_request_id,
                role,
            } => {
                let response = RoleRequestResponse::Approved { new_role: *role };

                if let Some(terminal_view) = self.terminal_view_from_pane_id(*terminal_pane_id, ctx)
                {
                    terminal_view.update(ctx, |view, ctx| {
                        view.respond_to_shared_session_role_request(
                            participant_id.clone(),
                            role_request_id.clone(),
                            response,
                            ctx,
                        );
                    });
                }
                ctx.notify();
            }
            RoleChangeModalEvent::DenyRequest {
                terminal_pane_id,
                participant_id,
                role_request_id,
            } => {
                let response = RoleRequestResponse::Rejected {
                    reason: RoleRequestRejectedReason::RejectedBySharer,
                };
                if let Some(terminal_view) = self.terminal_view_from_pane_id(*terminal_pane_id, ctx)
                {
                    terminal_view.update(ctx, |view, ctx| {
                        view.respond_to_shared_session_role_request(
                            participant_id.clone(),
                            role_request_id.clone(),
                            response,
                            ctx,
                        );
                    });
                }
                ctx.notify();
            }
            RoleChangeModalEvent::Close { source } => {
                self.close_shared_session_role_change_modal(*source, ctx)
            }
            RoleChangeModalEvent::CancelGrant => {
                self.close_shared_session_role_change_modal(
                    RoleChangeCloseSource::SharerGrant,
                    ctx,
                );
                send_telemetry_from_ctx!(
                    TelemetryEvent::SharerCancelledGrantRole {
                        role: Role::Executor
                    },
                    ctx
                );
            }
            RoleChangeModalEvent::GrantRole {
                terminal_pane_id,
                participant_id,
                dont_show_again,
            } => {
                if *dont_show_again {
                    if let Err(e) = SessionSettings::handle(ctx).update(ctx, |settings, ctx| {
                        settings
                            .should_confirm_shared_session_edit_access
                            .set_value(false, ctx)
                    }) {
                        log::error!(
                            "Failed to set should_confirm_shared_session_edit_access setting to false: {e}"
                        );
                    }
                    send_telemetry_from_ctx!(TelemetryEvent::SharerGrantModalDontShowAgain, ctx);
                }

                let Some(terminal_view) = self.terminal_view_from_pane_id(*terminal_pane_id, ctx)
                else {
                    log::error!("Tried to grant role for non existent terminal pane");
                    return;
                };

                let role_request_id = terminal_view.read(ctx, |view, ctx| {
                    view.shared_session_presence_manager().and_then(|manager| {
                        manager
                            .as_ref(ctx)
                            .get_role_request(participant_id)
                            .cloned()
                    })
                });

                // If participant has a pending role request, we respond to it here instead of in the role request modal
                if let Some(role_request_id) = role_request_id {
                    terminal_view.update(ctx, |view, ctx| {
                        let response = RoleRequestResponse::Approved {
                            new_role: Role::Executor,
                        };
                        view.respond_to_shared_session_role_request(
                            participant_id.clone(),
                            role_request_id.clone(),
                            response,
                            ctx,
                        );
                    });
                    self.remove_shared_session_role_request(role_request_id.clone(), ctx);
                // Otherwise, just update their role
                } else {
                    terminal_view.update(ctx, |view, ctx| {
                        view.update_role(participant_id.clone(), Role::Executor, ctx)
                    });
                }

                self.close_shared_session_role_change_modal(
                    RoleChangeCloseSource::SharerGrant,
                    ctx,
                );
            }
        }
    }

    fn new_internal(
        tips_completed: ModelHandle<TipsCompleted>,
        user_default_shell_unsupported_banner_model_handle: ModelHandle<BannerState>,
        server_api: Arc<ServerApi>,
        model_event_sender: Option<SyncSender<ModelEvent>>,
        initial_layout_callback: InitialLayoutCallback,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let windowing_state = WindowManager::handle(ctx);
        ctx.observe(&windowing_state, Self::handle_windowing_state_update);

        let mut pane_contents = HashMap::new();

        let resources = TerminalViewResources {
            tips_completed: tips_completed.clone(),
            server_api: server_api.clone(),
            model_event_sender: model_event_sender.clone(),
        };

        let view_bounds = Self::estimated_view_bounds(ctx);

        let mut pane_history = Vec::new();

        let (pane_data, initial_focus) = initial_layout_callback(
            resources,
            &mut pane_contents,
            &mut pane_history,
            view_bounds,
            ctx,
        );

        let focused_pane = initial_focus
            .focused_pane
            .or_else(|| pane_contents.keys().min().copied())
            .expect("At least one pane should have been created");

        let active_session_id = initial_focus.active_session.or_else(|| {
            pane_contents
                .keys()
                .filter_map(|id| id.as_terminal_pane_id())
                .min()
        });

        let in_split_pane = pane_data.visible_pane_count() > 1;
        let focus_state = ctx.add_model(|_| {
            focus_state::PaneGroupFocusState::new(focused_pane, active_session_id, in_split_pane)
        });
        ctx.subscribe_to_model(&focus_state, |me, _, event, ctx| {
            me.handle_focus_state_event(event, ctx);
        });

        let block_client = ServerApiProvider::as_ref(ctx).get_block_client();
        let share_modal =
            ctx.add_typed_action_view(|ctx| ShareBlockModal::new(None, block_client, ctx));
        ctx.subscribe_to_view(&share_modal, move |me, _, event, ctx| {
            me.handle_share_block_modal_event(event, ctx);
        });

        ctx.subscribe_to_model(&PaneSettings::handle(ctx), |_, _, _, ctx| {
            ctx.notify();
        });

        let user_default_shell_changed_banner = ctx.add_typed_action_view(|_| {
            Banner::<PaneGroupAction>::new_permanently_dismissible(
                BannerTextContent::formatted_text(vec![
                    FormattedTextFragment::plain_text(
                        "Warp doesn't currently support your default shell, falling back to zsh.  ",
                    ),
                    FormattedTextFragment::hyperlink("Learn more", WARP_SHELL_COMPATIBILITY_DOCS),
                ]),
            )
        });

        ctx.subscribe_to_model(&GeneralSettings::handle(ctx), |me, _, event, ctx| {
            if matches!(
                event,
                GeneralSettingsChangedEvent::UserDefaultShellUnsupportedBannerState { .. }
            ) {
                me.user_default_shell_unsupported_banner_model_handle
                    .update(ctx, |banner_state, ctx| {
                        *banner_state = *GeneralSettings::as_ref(ctx)
                            .user_default_shell_unsupported_banner_state;
                        ctx.notify();
                    })
            }
        });

        ctx.subscribe_to_view(&user_default_shell_changed_banner, |me, _, event, ctx| {
            me.handle_user_default_shell_changed_banner_event(event, ctx);
        });
        ctx.observe(
            &user_default_shell_unsupported_banner_model_handle,
            |_, _, ctx| {
                ctx.notify();
            },
        );

        let share_session_modal = ctx.add_typed_action_view(ShareSessionModal::new);
        ctx.subscribe_to_view(&share_session_modal, |me, _, event, ctx| {
            me.handle_share_session_modal_event(event, ctx);
        });

        let shared_session_role_change_modal = ctx.add_view(RoleChangeModal::new);
        ctx.subscribe_to_view(&shared_session_role_change_modal, |me, _, event, ctx| {
            me.handle_shared_session_role_change_modal_event(event, ctx);
        });

        ctx.subscribe_to_model(&UndoCloseStack::handle(ctx), |me, _, event, ctx| {
            let UndoCloseStackEvent::DiscardPane(pane_id) = event;
            me.discard_pane(*pane_id, ctx);
        });

        // Catch-up share for children that existed before the parent
        // started sharing — `inherit_share_for_local_child` only fires at
        // child-pane creation time.
        #[cfg(not(target_family = "wasm"))]
        ctx.subscribe_to_model(
            &BlocklistAIHistoryModel::handle(ctx),
            |me, _, event, ctx| {
                if let BlocklistAIHistoryEvent::LocalSharedSessionEstablished {
                    conversation_id,
                    ..
                } = event
                {
                    me.transitively_share_existing_local_children(*conversation_id, ctx);
                }
            },
        );

        let active_file_model = ctx.add_model(|_| ActiveFileModel::new());

        let mut pane_group = Self {
            tips_completed,
            user_default_shell_unsupported_banner_model_handle,
            model_event_sender,
            panes: pane_data,
            focus_state,
            pane_history,
            pane_contents,
            server_api,
            terminal_with_open_share_block_modal: None,
            share_block_modal: share_modal,
            dragged_border: None,
            user_default_shell_changed_banner,
            terminal_with_open_share_session_modal: None,
            share_session_modal,
            terminal_with_shared_session_role_change_modal_open: None,
            shared_session_role_change_modal,
            active_file_model,
            terminal_with_open_summarization_dialog: None,
            pane_with_open_environment_setup_mode_selector: None,
            pane_with_open_agent_assisted_environment_modal: None,
            right_panel_open: false,
            left_panel_open: false,
            is_right_panel_maximized: false,
            pending_ambient_agent_conversation_restorations: HashMap::new(),
            child_agent_panes: HashMap::new(),
            transitively_shared_child_panes: HashMap::new(),
            child_agent_origin: None,
            custom_title: None,
        };

        // Notify any restored panes that they belong to this pane group.
        pane_group.reattach_panes(ctx);
        if FeatureFlag::DragTabsToWindows.is_enabled() {
            pane_group.focus(ctx);
        }
        ctx.notify();

        pane_group
    }

    /// Returns the terminal view currently owning `conversation_id`, even if
    /// that owner lives outside this pane group.
    fn terminal_view_id_for_owned_conversation(
        &self,
        conversation_id: AIConversationId,
        ctx: &AppContext,
    ) -> Option<EntityId> {
        BlocklistAIHistoryModel::as_ref(ctx).terminal_view_id_for_conversation(&conversation_id)
    }

    fn pane_id_for_owned_conversation(
        &self,
        conversation_id: AIConversationId,
        ctx: &AppContext,
    ) -> Option<PaneId> {
        self.terminal_view_id_for_owned_conversation(conversation_id, ctx)
            .and_then(|terminal_view_id| self.find_pane_id_for_terminal_view(terminal_view_id, ctx))
    }
    fn is_conversation_owned_outside_pane(
        &self,
        conversation_id: AIConversationId,
        pane_id: PaneId,
        ctx: &AppContext,
    ) -> bool {
        self.terminal_view_id_for_owned_conversation(conversation_id, ctx)
            .is_some_and(|terminal_view_id| {
                match self.find_pane_id_for_terminal_view(terminal_view_id, ctx) {
                    Some(owner_pane_id) => owner_pane_id != pane_id,
                    None => true,
                }
            })
    }

    /// Lazily restores hidden child panes for the given parent conversation.
    ///
    /// Unlike the old startup sweep, this runs only when the parent agent view
    /// is actually restored or entered. Children that already belong to some
    /// other pane or tab are left alone.
    fn restore_missing_child_agent_panes_for_parent(
        &mut self,
        parent_conversation_id: AIConversationId,
        parent_pane_id: PaneId,
        ctx: &mut ViewContext<Self>,
    ) {
        let child_ids = BlocklistAIHistoryModel::as_ref(ctx)
            .child_conversation_ids_of(&parent_conversation_id)
            .to_vec();

        for child_id in child_ids {
            if self
                .child_agent_panes
                .get(&child_id)
                .is_some_and(|pane_id| self.has_pane_id(*pane_id))
            {
                continue;
            }

            if self.is_conversation_owned_outside_pane(child_id, parent_pane_id, ctx) {
                continue;
            }

            let child_conversation = BlocklistAIHistoryModel::as_ref(ctx)
                .conversation(&child_id)
                .cloned()
                .or_else(|| {
                    RestoredAgentConversations::handle(ctx)
                        .update(ctx, |store, _| store.take_conversation(&child_id))
                });
            let Some(child_conversation) = child_conversation else {
                log::warn!("Child conversation {child_id:?} not found in memory or restored store");
                continue;
            };

            self.create_hidden_child_agent_pane(child_conversation, parent_pane_id, ctx);
        }
    }

    /// Restores hidden child panes if this terminal pane is already showing a
    /// fullscreen agent view. This covers restored or replaced panes whose
    /// terminal view entered agent view before pane-group attachment finished.
    fn restore_missing_child_agent_panes_for_terminal_pane_if_needed(
        &mut self,
        pane_id: PaneId,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(terminal_pane_id) = pane_id.as_terminal_pane_id() else {
            return;
        };
        let Some(parent_conversation_id) = self
            .terminal_view_from_pane_id(terminal_pane_id, ctx)
            .and_then(|terminal_view| {
                let terminal_view = terminal_view.as_ref(ctx);
                let agent_view_state = terminal_view
                    .agent_view_controller()
                    .as_ref(ctx)
                    .agent_view_state();
                if agent_view_state.is_fullscreen() {
                    agent_view_state.active_conversation_id()
                } else {
                    None
                }
            })
        else {
            return;
        };

        self.restore_missing_child_agent_panes_for_parent(
            parent_conversation_id,
            terminal_pane_id.into(),
            ctx,
        );
    }

    /// Ensures `child_conversation_id` has a hidden child pane if it still
    /// belongs under a parent conversation in this pane group.
    ///
    /// Returns true if the conversation is already reachable through an
    /// existing pane or if lazy restoration successfully materialized the child
    /// pane.
    fn ensure_hidden_child_agent_pane_for_conversation(
        &mut self,
        child_conversation_id: AIConversationId,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        if self
            .child_agent_panes
            .get(&child_conversation_id)
            .is_some_and(|pane_id| self.has_pane_id(*pane_id))
        {
            return true;
        }

        let parent_conversation_id =
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, ctx| {
                history_model
                    .conversation(&child_conversation_id)
                    .and_then(|conversation| {
                        history_model.resolved_parent_conversation_id_for_conversation(conversation)
                    })
                    .or_else(|| {
                        RestoredAgentConversations::handle(ctx).read(ctx, |store, _| {
                            store.get_conversation(&child_conversation_id).and_then(
                                |conversation| {
                                    history_model.resolved_parent_conversation_id_for_conversation(
                                        conversation,
                                    )
                                },
                            )
                        })
                    })
            });

        let Some(parent_conversation_id) = parent_conversation_id else {
            return self
                .terminal_view_id_for_owned_conversation(child_conversation_id, ctx)
                .is_some();
        };

        let child_owner_terminal_view_id =
            self.terminal_view_id_for_owned_conversation(child_conversation_id, ctx);
        let Some(parent_pane_id) = self.pane_id_for_owned_conversation(parent_conversation_id, ctx)
        else {
            return child_owner_terminal_view_id.is_some();
        };

        if self.is_conversation_owned_outside_pane(child_conversation_id, parent_pane_id, ctx) {
            return true;
        }

        self.restore_missing_child_agent_panes_for_parent(
            parent_conversation_id,
            parent_pane_id,
            ctx,
        );

        self.child_agent_panes
            .get(&child_conversation_id)
            .is_some_and(|pane_id| self.has_pane_id(*pane_id))
            || self.is_conversation_owned_outside_pane(child_conversation_id, parent_pane_id, ctx)
    }

    /// Creates a hidden child agent pane for an existing child conversation,
    /// restoring the conversation and tracking it in `child_agent_panes`.
    fn create_hidden_child_agent_pane(
        &mut self,
        child_conversation: AIConversation,
        parent_pane_id: PaneId,
        ctx: &mut ViewContext<Self>,
    ) {
        let child_id = child_conversation.id();

        // Viewer-side child clicked before `OrchestrationViewerModel`
        // surfaced a `session_id`: render a loading placeholder; the real
        // pane gets swapped in by `ensure_shared_session_viewer_child_pane`.
        if child_conversation.is_viewing_shared_session() {
            let resources = TerminalViewResources {
                tips_completed: self.tips_completed.clone(),
                server_api: self.server_api.clone(),
                model_event_sender: self.model_event_sender.clone(),
            };
            let view_size = Self::estimated_view_bounds(ctx).size();
            let (loading_view, loading_manager) = Self::create_loading_terminal_manager_and_view(
                resources,
                view_size,
                ctx.window_id(),
                ctx,
            );
            let pane_data = TerminalPane::new(
                Uuid::new_v4().as_bytes().to_vec(),
                loading_manager,
                loading_view.clone(),
                self.model_event_sender.clone(),
                ctx,
            );
            let new_pane_id = pane_data.terminal_pane_id();
            if self
                .attach_child_pane_off_tree(Box::new(pane_data), ctx)
                .is_none()
            {
                log::error!(
                    "create_hidden_child_agent_pane: failed to attach loading placeholder for \
                     viewer-side child {child_id:?}"
                );
                return;
            }

            // Restore the conversation and enter agent view so the pill bar
            // renders (its gate requires `is_fullscreen()`). The output area
            // stays a loading spinner because the loading view's
            // `ConversationTranscriptViewerStatus::Loading` short-circuits
            // the block list render in `TerminalView::render`.
            loading_view.update(ctx, |terminal_view, ctx| {
                terminal_view.restore_conversation_after_view_creation(
                    RestoredAIConversation::new(child_conversation),
                    true,
                    RestoreConversationEntryBehavior::PreserveAgentViewState,
                    ctx,
                );
                terminal_view.enter_agent_view(
                    None,
                    Some(child_id),
                    AgentViewEntryOrigin::SharedSessionSelection,
                    ctx,
                );
            });

            self.child_agent_panes.insert(child_id, new_pane_id.into());
            return;
        }

        if child_conversation.is_remote_child() {
            let Some(task_id) = child_conversation.task_id() else {
                log::warn!(
                    "Cannot restore remote child conversation {child_id:?} without a task ID"
                );
                return;
            };

            let new_pane_id =
                self.insert_ambient_agent_pane_hidden_for_child_agent(parent_pane_id, ctx);

            if let Some(new_terminal_view) = self.terminal_view_from_pane_id(new_pane_id, ctx) {
                let mut restored = false;
                new_terminal_view.update(ctx, |terminal_view, ctx| {
                    terminal_view.restore_conversation_after_view_creation(
                        RestoredAIConversation::new(child_conversation),
                        true,
                        RestoreConversationEntryBehavior::PreserveAgentViewState,
                        ctx,
                    );
                    terminal_view.enter_agent_view(
                        None,
                        Some(child_id),
                        AgentViewEntryOrigin::CloudAgent,
                        ctx,
                    );
                    let Some(ambient_agent_view_model) = terminal_view
                        .ambient_agent_view_model()
                        .into_optional_handle()
                        .cloned()
                    else {
                        return;
                    };
                    ambient_agent_view_model.update(ctx, |model, ctx| {
                        model.set_conversation_id(Some(child_id));
                        model.enter_viewing_existing_session(task_id, ctx);
                    });
                    restored = true;
                });
                if restored {
                    self.child_agent_panes.insert(child_id, new_pane_id.into());
                } else {
                    log::error!(
                        "Failed to restore remote child agent pane {child_id:?}: missing ambient agent view model"
                    );
                    self.discard_pane(new_pane_id.into(), ctx);
                }
            } else {
                log::error!("Failed to get terminal view for remote child agent pane {child_id:?}");
                self.discard_pane(new_pane_id.into(), ctx);
            }
            return;
        }
        let child_task_context =
            child_conversation
                .task_id()
                .map(|task_id| HiddenChildAgentTaskContext {
                    task_id,
                    working_dir: child_conversation
                        .current_working_directory()
                        .or_else(|| child_conversation.initial_working_directory())
                        .map(PathBuf::from),
                });
        // Restored hidden child panes don't inherit the host's shared
        // session — the host's share decision is handled at original
        // dispatch time, not on subsequent restores.
        let new_pane_id = self.insert_terminal_pane_hidden_for_child_agent(
            parent_pane_id,
            HashMap::new(),
            IsSharedSessionCreator::No,
            ctx,
        );

        if let Some(new_terminal_view) = self.terminal_view_from_pane_id(new_pane_id, ctx) {
            if let Some(task_context) = child_task_context.as_ref() {
                apply_hidden_child_agent_task_context(&new_terminal_view, task_context, ctx);
            }
            new_terminal_view.update(ctx, |terminal_view, ctx| {
                terminal_view.restore_conversation_after_view_creation(
                    RestoredAIConversation::new(child_conversation),
                    true,
                    RestoreConversationEntryBehavior::PreserveAgentViewState,
                    ctx,
                );
                terminal_view.enter_agent_view(
                    None,
                    Some(child_id),
                    AgentViewEntryOrigin::ChildAgent,
                    ctx,
                );
            });

            self.child_agent_panes.insert(child_id, new_pane_id.into());
        } else {
            log::error!("Failed to get terminal view for child agent pane {child_id:?}");
            self.discard_pane(new_pane_id.into(), ctx);
        }
    }

    /// Materializes a hidden shared-session viewer pane for a viewer-
    /// discovered child agent. Triggered by
    /// `Event::EnsureSharedSessionViewerChildPane`, which
    /// `OrchestrationViewerModel` emits on the parent's view the first
    /// time it observes a `session_id` for a child. The new pane gets its
    /// own `BlocklistAIController` and viewer-side `Network` so child
    /// traffic doesn't cross the parent's single-stream state.
    fn ensure_shared_session_viewer_child_pane(
        &mut self,
        child_conversation_id: AIConversationId,
        child_session_id: SessionId,
        ctx: &mut ViewContext<Self>,
    ) {
        // Race recovery: a pill click before materialization had a
        // `session_id` falls through to `create_hidden_child_agent_pane`,
        // which leaves a loading placeholder in `child_agent_panes`. The
        // emission gate in `OrchestrationViewerModel` guarantees this
        // helper runs at most once per child per model lifetime, so any
        // existing entry must be that fallback — safe to discard.
        let fallback_was_swapped_anchor = if let Some(prior_pane_id) = self
            .child_agent_panes
            .get(&child_conversation_id)
            .copied()
            .filter(|pane_id| self.has_pane_id(*pane_id))
        {
            let anchor = self.panes.original_pane_for_replacement(prior_pane_id);
            self.discard_child_agent_pane_for_conversation(child_conversation_id, ctx);
            anchor
        } else {
            None
        };

        let Some(child_conversation) = BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(&child_conversation_id)
            .cloned()
        else {
            log::warn!(
                "ensure_shared_session_viewer_child_pane: no local conversation {child_conversation_id:?}"
            );
            return;
        };
        let child_task_id = child_conversation.task_id();

        let resources = TerminalViewResources {
            tips_completed: self.tips_completed.clone(),
            server_api: self.server_api.clone(),
            model_event_sender: self.model_event_sender.clone(),
        };
        let view_size = Self::estimated_view_bounds(ctx).size();
        let (new_terminal_view, terminal_manager) = Self::create_shared_session_viewer(
            child_session_id,
            resources,
            view_size,
            // Per-child viewer: parent's model already discovers descendants.
            false,
            ctx,
        );

        let pane_data = TerminalPane::new(
            Uuid::new_v4().as_bytes().to_vec(),
            terminal_manager,
            new_terminal_view.clone(),
            self.model_event_sender.clone(),
            ctx,
        );
        let new_pane_id = pane_data.terminal_pane_id();
        if self
            .attach_child_pane_off_tree(Box::new(pane_data), ctx)
            .is_none()
        {
            log::error!(
                "ensure_shared_session_viewer_child_pane: failed to attach pane for conv={child_conversation_id:?}"
            );
            return;
        }

        new_terminal_view.update(ctx, |terminal_view, ctx| {
            terminal_view.suppress_initial_conversation_details_panel_auto_open();
            terminal_view.restore_conversation_after_view_creation(
                RestoredAIConversation::new(child_conversation),
                true,
                RestoreConversationEntryBehavior::PreserveAgentViewState,
                ctx,
            );
            terminal_view.enter_agent_view(
                None,
                Some(child_conversation_id),
                AgentViewEntryOrigin::SharedSessionSelection,
                ctx,
            );
            // Shared-session viewer is `is_cloud_mode=false`, so
            // `ambient_agent_view_model()` is typically `None`. Update
            // opportunistically; the network's `JoinedSuccessfully` is the
            // authoritative source for ambient agent state.
            if let Some(ambient_agent_view_model) = terminal_view
                .ambient_agent_view_model()
                .into_optional_handle()
                .cloned()
            {
                ambient_agent_view_model.update(ctx, |model, ctx| {
                    model.set_conversation_id(Some(child_conversation_id));
                    if let Some(task_id) = child_task_id {
                        model.enter_viewing_existing_session(task_id, ctx);
                    }
                });
            }
        });

        self.child_agent_panes
            .insert(child_conversation_id, new_pane_id.into());

        // If the discarded fallback was occupying a tree slot via temporary
        // replacement, re-swap so the user lands on the new pane.
        if let Some(anchor) = fallback_was_swapped_anchor {
            self.swap_active_pane_to_conversation(anchor, child_conversation_id, ctx);
        }
    }

    /// Helper that creates the initial [`PaneData`] and [`InitialFocus`] given a terminal view.
    /// This is a common case in creating a new pane group with a single terminal session.
    fn terminal_pane_data(
        uuid: Vec<u8>,
        view: ViewHandle<TerminalView>,
        terminal_manager: ModelHandle<Box<dyn TerminalManager>>,
        model_event_sender: Option<SyncSender<ModelEvent>>,
        pane_contents: &mut HashMap<PaneId, Box<dyn AnyPaneContent>>,
        pane_history: &mut Vec<PaneId>,
        ctx: &mut ViewContext<Self>,
    ) -> (PaneData, InitialFocus) {
        let pane_data = TerminalPane::new(uuid, terminal_manager, view, model_event_sender, ctx);
        let terminal_pane_id = pane_data.terminal_pane_id();
        let pane_id = terminal_pane_id.into();
        pane_contents.insert(pane_id, Box::new(pane_data));
        pane_history.push(pane_id);
        let focus = InitialFocus {
            focused_pane: Some(pane_id),
            active_session: Some(terminal_pane_id),
        };
        (PaneData::new(pane_id), focus)
    }

    fn create_cloud_mode_terminal(
        resources: TerminalViewResources,
        view_bounds_size: Vector2F,
        enable_orchestration_polling: bool,
        ctx: &mut ViewContext<Self>,
    ) -> (
        ViewHandle<TerminalView>,
        ModelHandle<Box<dyn TerminalManager>>,
    ) {
        let window_id = ctx.window_id();
        crate::terminal::view::ambient_agent::create_cloud_mode_view(
            resources,
            view_bounds_size,
            window_id,
            enable_orchestration_polling,
            ctx,
        )
    }

    /// Helper to create the terminal manager and view for an ambient agent pane.
    fn create_ambient_agent_terminal(
        resources: TerminalViewResources,
        view_bounds_size: Vector2F,
        ctx: &mut ViewContext<Self>,
    ) -> (
        ViewHandle<TerminalView>,
        ModelHandle<Box<dyn TerminalManager>>,
    ) {
        let (terminal_view, terminal_manager) =
            Self::create_cloud_mode_terminal(resources, view_bounds_size, true, ctx);

        terminal_view.update(ctx, |view, ctx| {
            view.enter_ambient_agent_setup(None, ctx);
        });

        (terminal_view, terminal_manager)
    }

    /// Stores the pending ambient agent restorations, triggers async fetches for
    /// their task data, and sets up a single long-lived subscription that will
    /// process each pane as its task data arrives.
    fn register_pending_ambient_restorations(
        &mut self,
        pending: Vec<(AmbientAgentTaskId, PaneId)>,
        ctx: &mut ViewContext<Self>,
    ) {
        for (task_id, _) in &pending {
            AgentConversationsModel::handle(ctx).update(ctx, |model, ctx| {
                model.get_or_async_fetch_task_data(task_id, ctx);
            });
        }

        self.pending_ambient_agent_conversation_restorations = pending.into_iter().collect();

        let conversations_model = AgentConversationsModel::handle(ctx);
        ctx.subscribe_to_model(&conversations_model, |me, _, event, ctx| {
            me.handle_pending_ambient_restoration_event(event, ctx);
        });
    }

    /// Subscription handler that processes pending ambient agent pane restorations
    /// whenever task data is updated or conversations finish loading.
    fn handle_pending_ambient_restoration_event(
        &mut self,
        event: &AgentConversationsModelEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        if !matches!(
            event,
            AgentConversationsModelEvent::TasksUpdated
                | AgentConversationsModelEvent::ConversationsLoaded
        ) {
            return;
        }

        if self
            .pending_ambient_agent_conversation_restorations
            .is_empty()
        {
            return;
        }

        let ready_tasks: Vec<_> = self
            .pending_ambient_agent_conversation_restorations
            .keys()
            .filter(|task_id| {
                AgentConversationsModel::as_ref(ctx)
                    .get_task_data(task_id)
                    .is_some()
            })
            .copied()
            .collect();

        let resources = TerminalViewResources {
            tips_completed: self.tips_completed.clone(),
            server_api: self.server_api.clone(),
            model_event_sender: self.model_event_sender.clone(),
        };
        let view_size = Self::estimated_view_bounds(ctx).size();

        for task_id in ready_tasks {
            let Some(pane_id) = self
                .pending_ambient_agent_conversation_restorations
                .remove(&task_id)
            else {
                continue;
            };
            let Some(task) = AgentConversationsModel::as_ref(ctx).get_task_data(&task_id) else {
                continue;
            };

            match AgentConversationsModel::resolve_open_action(
                AgentConversationNavigationSubject::Entry(AgentConversationEntryId::AmbientRun(
                    task.task_id,
                )),
                None,
                ctx,
            ) {
                Some(WorkspaceAction::OpenOrAttachAmbientAgentConversation {
                    session_id,
                    task_id: _,
                }) => {
                    let (view, terminal_manager) = Self::create_shared_session_viewer(
                        session_id,
                        resources.clone(),
                        view_size,
                        true, // root orchestrator viewer
                        ctx,
                    );
                    let new_pane = TerminalPane::new(
                        Uuid::new_v4().as_bytes().to_vec(),
                        terminal_manager,
                        view,
                        self.model_event_sender.clone(),
                        ctx,
                    );
                    self.replace_pane(pane_id, new_pane, false, ctx);
                }
                Some(WorkspaceAction::OpenConversationTranscriptViewer {
                    conversation_id,
                    ambient_agent_task_id,
                }) => {
                    if let Some(target_view) = self.terminal_view_from_pane_id(pane_id, ctx) {
                        Self::fetch_and_load_transcript(
                            target_view,
                            conversation_id,
                            ambient_agent_task_id,
                            ctx,
                        );
                    } else {
                        self.pending_ambient_agent_conversation_restorations
                            .insert(task_id, pane_id);
                    }
                }
                _ => {
                    self.replace_pane_with_new_cloud_conversation(pane_id, ctx);
                }
            }
        }
    }

    /// Fetches conversation data and loads it into the given transcript viewer.
    fn fetch_and_load_transcript(
        target_view: ViewHandle<TerminalView>,
        server_conversation_token: ServerConversationToken,
        ambient_agent_task_id: Option<AmbientAgentTaskId>,
        ctx: &mut ViewContext<Self>,
    ) {
        let history_model_handle = BlocklistAIHistoryModel::handle(ctx);

        let future = history_model_handle.update(ctx, |history_model, ctx| {
            history_model.load_conversation_by_server_token(&server_conversation_token, ctx)
        });
        ctx.spawn(future, move |group, conversation, ctx| {
            if let Some(conversation) = conversation {
                group.load_data_into_transcript_viewer(
                    target_view,
                    conversation,
                    ambient_agent_task_id,
                    ctx,
                );
            } else if let Some(pane_id) =
                group.find_pane_id_for_terminal_view(target_view.id(), ctx)
            {
                log::error!(
                    "Failed to restore ambient agent pane, replacing with new cloud conversation"
                );
                group.replace_pane_with_new_cloud_conversation(pane_id, ctx);
            }
        });
    }

    /// Replaces a pane with a new cloud conversation.
    fn replace_pane_with_new_cloud_conversation(
        &mut self,
        pane_id: PaneId,
        ctx: &mut ViewContext<Self>,
    ) {
        let resources = TerminalViewResources {
            tips_completed: self.tips_completed.clone(),
            server_api: self.server_api.clone(),
            model_event_sender: self.model_event_sender.clone(),
        };
        let view_size = Self::estimated_view_bounds(ctx).size();
        let (view, terminal_manager) =
            Self::create_ambient_agent_terminal(resources, view_size, ctx);
        let new_pane = TerminalPane::new(
            Uuid::new_v4().as_bytes().to_vec(),
            terminal_manager,
            view,
            self.model_event_sender.clone(),
            ctx,
        );
        self.replace_pane(pane_id, new_pane, false, ctx);
    }

    /// Initial layout for a [`PaneGroup`] with a single ambient agent pane.
    fn initial_ambient_agent_pane(
        resources: TerminalViewResources,
        view_bounds: RectF,
        model_event_sender: Option<SyncSender<ModelEvent>>,
        pane_contents: &mut HashMap<PaneId, Box<dyn AnyPaneContent>>,
        pane_history: &mut Vec<PaneId>,
        ctx: &mut ViewContext<Self>,
    ) -> (PaneData, InitialFocus) {
        let uuid = Uuid::new_v4();

        let (terminal_view, terminal_manager) =
            Self::create_ambient_agent_terminal(resources, view_bounds.size(), ctx);

        Self::terminal_pane_data(
            uuid.into_bytes().to_vec(),
            terminal_view,
            terminal_manager,
            model_event_sender,
            pane_contents,
            pane_history,
            ctx,
        )
    }

    /// Initial layout for a [`PaneGroup`] with a single terminal pane.
    #[allow(clippy::too_many_arguments)]
    fn initial_single_terminal_pane(
        options: NewTerminalOptions,
        resources: TerminalViewResources,
        unsupported_banner_model_handle: ModelHandle<BannerState>,
        view_bounds: RectF,
        model_event_sender: Option<SyncSender<ModelEvent>>,
        pane_contents: &mut HashMap<PaneId, Box<dyn AnyPaneContent>>,
        pane_history: &mut Vec<PaneId>,
        ctx: &mut ViewContext<Self>,
    ) -> (PaneData, InitialFocus) {
        let uuid = Uuid::new_v4();
        let (view, terminal_manager) = PaneGroup::create_session(
            options.initial_directory,
            options.env_vars,
            uuid.as_bytes(),
            options.is_shared_session_creator,
            resources,
            None,
            options.conversation_restoration,
            unsupported_banner_model_handle,
            view_bounds.size(),
            model_event_sender.clone(),
            options.shell,
            None,
            ctx,
        );

        let pane_data = TerminalPane::new(
            uuid.as_bytes().to_vec(),
            terminal_manager,
            view,
            model_event_sender,
            ctx,
        );
        let terminal_pane_id = pane_data.terminal_pane_id();
        let pane_id = terminal_pane_id.into();
        pane_contents.insert(pane_id, Box::new(pane_data));
        pane_history.push(pane_id);
        let focus = InitialFocus {
            focused_pane: Some(pane_id),
            active_session: Some(terminal_pane_id),
        };
        (PaneData::new(pane_id), focus)
    }

    /// Constructs a new [`PaneGroup`] with a layout that adheres
    /// to the specification of the provided [`PanesLayout`].
    #[allow(clippy::too_many_arguments)]
    pub fn new_with_panes_layout(
        tips_completed: ModelHandle<TipsCompleted>,
        user_default_shell_unsupported_banner_model_handle: ModelHandle<BannerState>,
        server_api: Arc<ServerApi>,
        panes_layout: PanesLayout,
        block_lists: Arc<HashMap<PaneUuid, Vec<SerializedBlockListItem>>>,
        model_event_sender: Option<SyncSender<ModelEvent>>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let unsupported_banner_model_handle =
            user_default_shell_unsupported_banner_model_handle.clone();
        let model_event_sender_clone = model_event_sender.clone();

        // Shared container so pending ambient restorations collected inside the
        // layout closure can be accessed after `new_internal` returns.
        let pending_ambient = Rc::new(RefCell::new(Vec::new()));
        let pending_ambient_for_closure = pending_ambient.clone();

        let initial_layout = move |resources,
                                   pane_contents: &mut HashMap<PaneId, Box<dyn AnyPaneContent>>,
                                   pane_history: &mut Vec<PaneId>,
                                   view_bounds: RectF,
                                   ctx: &mut ViewContext<Self>| {
            match panes_layout {
                PanesLayout::Template(template) => Self::pane_tree_from_template(
                    template,
                    resources,
                    ctx,
                    pane_contents,
                    true, // initialize as the leftmost pane
                    unsupported_banner_model_handle,
                    view_bounds.size(),
                    model_event_sender_clone,
                ),
                PanesLayout::Snapshot(panes_snapshot) => {
                    let mut deferred_panes = Vec::new();
                    let mut pending_restorations = Vec::new();
                    let result = Self::restore_pane_tree(
                        *panes_snapshot,
                        block_lists,
                        resources.clone(),
                        ctx,
                        pane_contents,
                        unsupported_banner_model_handle.clone(),
                        view_bounds.size(),
                        model_event_sender_clone.clone(),
                        &mut deferred_panes,
                        &mut pending_restorations,
                    )
                    .unwrap_or_else(|err| {
                        log::warn!("Error restoring pane tree: {err:#}");
                        Self::initial_single_terminal_pane(
                            NewTerminalOptions::default(),
                            resources,
                            unsupported_banner_model_handle,
                            view_bounds,
                            model_event_sender_clone,
                            pane_contents,
                            pane_history,
                            ctx,
                        )
                    });

                    *pending_ambient_for_closure.borrow_mut() = pending_restorations;

                    Self::process_deferred_panes(deferred_panes, result, pane_contents, ctx)
                }
                PanesLayout::SingleTerminal(options) => Self::initial_single_terminal_pane(
                    *options,
                    resources,
                    unsupported_banner_model_handle,
                    view_bounds,
                    model_event_sender_clone,
                    pane_contents,
                    pane_history,
                    ctx,
                ),
                PanesLayout::AmbientAgent => Self::initial_ambient_agent_pane(
                    resources,
                    view_bounds,
                    model_event_sender_clone,
                    pane_contents,
                    pane_history,
                    ctx,
                ),
            }
        };

        let mut pane_group = Self::new_internal(
            tips_completed,
            user_default_shell_unsupported_banner_model_handle,
            server_api,
            model_event_sender.clone(),
            Box::new(initial_layout),
            ctx,
        );

        // The closure has now run — register any pending ambient restorations
        // that need to wait for task data from the server.
        let pending = pending_ambient.take();
        if !pending.is_empty() {
            pane_group.register_pending_ambient_restorations(pending, ctx);
        }

        pane_group
    }

    pub fn new_from_existing_pane(
        pane: Box<dyn AnyPaneContent>,
        tips_completed: ModelHandle<TipsCompleted>,
        user_default_shell_unsupported_banner_model_handle: ModelHandle<BannerState>,
        server_api: Arc<ServerApi>,
        model_event_sender: Option<SyncSender<ModelEvent>>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let pane_id = pane.as_pane().id();
        let initial_layout = move |_,
                                   pane_contents: &mut HashMap<PaneId, Box<dyn AnyPaneContent>>,
                                   pane_history: &mut Vec<PaneId>,
                                   _: RectF,

                                   _: &mut ViewContext<Self>| {
            pane_contents.insert(pane_id, pane);
            pane_history.push(pane_id);
            let initial_focus = InitialFocus {
                focused_pane: Some(pane_id),
                active_session: pane_id.as_terminal_pane_id(),
            };
            (PaneData::new(pane_id), initial_focus)
        };
        Self::new_internal(
            tips_completed,
            user_default_shell_unsupported_banner_model_handle,
            server_api,
            model_event_sender,
            Box::new(initial_layout),
            ctx,
        )
    }

    pub fn new_for_shared_session_viewer(
        session_id: SessionId,
        tips_completed: ModelHandle<TipsCompleted>,
        user_default_shell_unsupported_banner_model_handle: ModelHandle<BannerState>,
        server_api: Arc<ServerApi>,
        model_event_sender: Option<SyncSender<ModelEvent>>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let model_event_sender_clone = model_event_sender.clone();
        let initial_layout = move |resources,
                                   pane_contents: &mut HashMap<PaneId, Box<dyn AnyPaneContent>>,
                                   pane_history: &mut Vec<PaneId>,
                                   view_bounds: RectF,
                                   ctx: &mut ViewContext<Self>| {
            let (view, terminal_manager) = PaneGroup::create_shared_session_viewer(
                session_id,
                resources,
                view_bounds.size(),
                true, // root orchestrator viewer
                ctx,
            );

            Self::terminal_pane_data(
                Uuid::new_v4().as_bytes().to_vec(),
                view,
                terminal_manager,
                model_event_sender_clone,
                pane_contents,
                pane_history,
                ctx,
            )
        };
        Self::new_internal(
            tips_completed,
            user_default_shell_unsupported_banner_model_handle,
            server_api,
            model_event_sender,
            Box::new(initial_layout),
            ctx,
        )
    }

    /// Create a new pane group for a view-only cloud conversation.
    pub fn new_for_conversation_transcript_viewer(
        conversation: AIConversation,
        ambient_agent_task_id: Option<AmbientAgentTaskId>,
        tips_completed: ModelHandle<TipsCompleted>,
        user_default_shell_unsupported_banner_model_handle: ModelHandle<BannerState>,
        server_api: Arc<ServerApi>,
        model_event_sender: Option<SyncSender<ModelEvent>>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let model_event_sender_clone = model_event_sender.clone();
        let initial_layout = move |resources,
                                   pane_contents: &mut HashMap<PaneId, Box<dyn AnyPaneContent>>,
                                   pane_history: &mut Vec<PaneId>,
                                   view_bounds: RectF,
                                   ctx: &mut ViewContext<Self>| {
            let (view, terminal_manager) = PaneGroup::create_conversation_viewer(
                conversation.clone(),
                ambient_agent_task_id,
                resources,
                view_bounds.size(),
                ctx,
            );

            Self::terminal_pane_data(
                Uuid::new_v4().as_bytes().to_vec(),
                view,
                terminal_manager,
                model_event_sender_clone,
                pane_contents,
                pane_history,
                ctx,
            )
        };
        Self::new_internal(
            tips_completed,
            user_default_shell_unsupported_banner_model_handle,
            server_api,
            model_event_sender,
            Box::new(initial_layout),
            ctx,
        )
    }

    /// Create a new pane group with a loading state for a conversation viewer.
    /// The actual conversation data will be loaded asynchronously.
    pub fn new_for_conversation_transcript_viewer_loading(
        tips_completed: ModelHandle<TipsCompleted>,
        user_default_shell_unsupported_banner_model_handle: ModelHandle<BannerState>,
        server_api: Arc<ServerApi>,
        model_event_sender: Option<SyncSender<ModelEvent>>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let model_event_sender_clone = model_event_sender.clone();
        let initial_layout = move |resources,
                                   pane_contents: &mut HashMap<PaneId, Box<dyn AnyPaneContent>>,
                                   pane_history: &mut Vec<PaneId>,
                                   view_bounds: RectF,
                                   ctx: &mut ViewContext<Self>| {
            let (terminal_view, terminal_manager) = Self::create_loading_terminal_manager_and_view(
                resources,
                view_bounds.size(),
                ctx.window_id(),
                ctx,
            );

            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, _ctx| {
                history_model
                    .mark_terminal_view_as_conversation_transcript_viewer(terminal_view.id());
            });

            Self::terminal_pane_data(
                Uuid::new_v4().as_bytes().to_vec(),
                terminal_view,
                terminal_manager,
                model_event_sender_clone,
                pane_contents,
                pane_history,
                ctx,
            )
        };
        Self::new_internal(
            tips_completed,
            user_default_shell_unsupported_banner_model_handle,
            server_api,
            model_event_sender,
            Box::new(initial_layout),
            ctx,
        )
    }

    /// Load conversation data into a conversation viewer that was created with a loading state.
    /// Uses the active session view as the target.
    pub fn load_data_into_conversation_transcript_viewer(
        &mut self,
        conversation: CloudConversationData,
        ambient_agent_task_id: Option<AmbientAgentTaskId>,
        ctx: &mut ViewContext<Self>,
    ) {
        // Get the active terminal view
        let Some(terminal_view) = self.active_session_view(ctx) else {
            log::error!("No active terminal view to load conversation into");
            return;
        };
        self.load_data_into_transcript_viewer(
            terminal_view,
            conversation,
            ambient_agent_task_id,
            ctx,
        );
    }

    /// Load conversation data into a specific transcript viewer terminal view.
    fn load_data_into_transcript_viewer(
        &mut self,
        terminal_view: ViewHandle<TerminalView>,
        cloud_conversation: CloudConversationData,
        ambient_agent_task_id: Option<AmbientAgentTaskId>,
        ctx: &mut ViewContext<Self>,
    ) {
        let terminal_manager = self
            .find_pane_id_for_terminal_view(terminal_view.id(), ctx)
            .and_then(|pid| pid.as_terminal_pane_id())
            .and_then(|tpid| self.terminal_session_by_id(tpid))
            .map(|session| session.terminal_manager(ctx));

        let ambient_agent_task_id =
            ambient_agent_task_id.or_else(|| Self::ambient_agent_task_id(&cloud_conversation));

        if FeatureFlag::HandoffCloudCloud.is_enabled() {
            if let Some(task_id) = ambient_agent_task_id {
                if terminal_view
                    .as_ref(ctx)
                    .ambient_agent_view_model()
                    .is_some()
                {
                    Self::load_data_into_restored_ambient_cloud_mode_view(
                        terminal_view,
                        cloud_conversation,
                        task_id,
                        ctx,
                    );
                    ctx.notify();
                    return;
                }

                if let Some(pane_id) = self.find_pane_id_for_terminal_view(terminal_view.id(), ctx)
                {
                    self.replace_loading_pane_with_restored_ambient_cloud_mode_pane(
                        pane_id,
                        cloud_conversation,
                        task_id,
                        ctx,
                    );
                    ctx.notify();
                    return;
                }
            }
        }

        BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, _ctx| {
            history_model.mark_terminal_view_as_conversation_transcript_viewer(terminal_view.id());
        });

        if let Some(ref terminal_manager) = terminal_manager {
            let status = if let Some(task_id) = ambient_agent_task_id {
                ConversationTranscriptViewerStatus::ViewingAmbientConversation(task_id)
            } else {
                ConversationTranscriptViewerStatus::ViewingLocalConversation
            };

            terminal_manager.update(ctx, |terminal_manager, _ctx| {
                terminal_manager
                    .model()
                    .lock()
                    .set_conversation_transcript_viewer_status(Some(status));
            });
        }

        match cloud_conversation {
            CloudConversationData::Oz(mut conversation) => {
                if ambient_agent_task_id.is_some() {
                    conversation.set_is_viewing_shared_session(true);
                }
                terminal_view.update(ctx, |view, ctx| {
                    view.restore_conversation_after_view_creation(
                        RestoredAIConversation::new(*conversation),
                        true,
                        RestoreConversationEntryBehavior::EnterRestoredConversation,
                        ctx,
                    );
                });
            }
            CloudConversationData::CLIAgent(cli_conversation) => {
                if !FeatureFlag::AgentHarness.is_enabled() {
                    log::warn!("AgentHarness flag is disabled; ignoring CLI agent conversation");
                    return;
                }
                let harness = match cli_conversation.metadata.harness {
                    AIAgentHarness::ClaudeCode => Some(Harness::Claude),
                    AIAgentHarness::Gemini => Some(Harness::Gemini),
                    AIAgentHarness::Codex => Some(Harness::Codex),
                    AIAgentHarness::Oz => None,
                    AIAgentHarness::Unknown => Some(Harness::Unknown),
                };
                let fallback_title = cli_conversation.metadata.title.clone();
                terminal_view.update(ctx, |view, ctx| {
                    view.restore_conversation_and_directory_context(
                        CloudConversationData::CLIAgent(cli_conversation),
                        true,
                        RestoreConversationEntryBehavior::PreserveAgentViewState,
                        |_, _| {},
                        ctx,
                    );
                    // Keep the viewer's AmbientAgentViewModel harness in sync with the loaded run.
                    if let Some(harness) = harness {
                        if let Some(ambient_agent_view_model) =
                            view.ambient_agent_view_model().cloned()
                        {
                            ambient_agent_view_model.update(ctx, |model, ctx| {
                                model.set_harness(harness, ctx);
                            });
                        }
                    }
                    // 3p runs have no materialized AIConversation, so enter agent view with a
                    // fresh vehicle conversation and retag the restored snapshot block onto it so
                    // it passes `should_hide_block`'s agent view filter.
                    if let Some(vehicle_conversation_id) =
                        view.enter_agent_view_for_restored_cli_agent(fallback_title, ctx)
                    {
                        view.model
                            .lock()
                            .block_list_mut()
                            .attach_non_startup_blocks_to_conversation(vehicle_conversation_id);
                    }
                });
            }
        };

        // Register the transcript viewer as an ambient session so it appears in the Active section
        // of the conversation list.
        if let Some(task_id) = ambient_agent_task_id {
            ActiveAgentViewsModel::handle(ctx).update(ctx, |active_views, ctx| {
                active_views.register_ambient_session(terminal_view.id(), task_id, ctx);
            });
        }

        // Insert the conversation ended tombstone (includes Open in Warp button on WASM).
        if terminal_manager.is_some() {
            terminal_view.update(ctx, |view, ctx| {
                view.insert_conversation_ended_tombstone_with_resolved_cta(ctx);
            });
        }

        ctx.notify();
    }

    fn handle_windowing_state_update(
        &mut self,
        _handle: ModelHandle<WindowManager>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.update_session_visibility(ctx);
    }

    fn handle_focus_state_event(
        &mut self,
        event: &PaneGroupFocusEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            PaneGroupFocusEvent::FocusChanged { .. } => ctx.notify(),
            PaneGroupFocusEvent::ActiveSessionChanged { .. } => {
                ctx.emit(Event::ActiveSessionChanged);
                ctx.notify();
            }
            PaneGroupFocusEvent::InSplitPaneChanged => ctx.notify(),
            PaneGroupFocusEvent::FocusedPaneMaximizedChanged => ctx.notify(),
        }
    }

    fn handle_share_block_modal_event(
        &mut self,
        event: &ShareBlockModalEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            ShareBlockModalEvent::Close => {
                self.focus(ctx);
                self.terminal_with_open_share_block_modal = None;
                ctx.notify();
            }
            ShareBlockModalEvent::ShowToast { message, flavor } => ctx.emit(Event::ShowToast {
                message: message.clone(),
                flavor: *flavor,
                pane_id: None,
            }),
        }
    }

    /// Used to add a new pane but not splitting panes.
    pub fn add_terminal_pane(
        &mut self,
        direction: Direction,
        chosen_shell: Option<AvailableShell>,
        ctx: &mut ViewContext<Self>,
    ) -> TerminalPaneId {
        let new_pane_id = self.add_session(
            direction,
            Some(self.focused_pane_id(ctx)),
            self.active_session_id(ctx),
            chosen_shell,
            None, /* conversation_restoration */
            ctx,
        );
        ctx.emit(Event::AppStateChanged);
        new_pane_id
    }

    /// Adds a terminal split pane without applying the user's default session mode.
    pub fn add_terminal_pane_ignoring_default_session_mode(
        &mut self,
        direction: Direction,
        chosen_shell: Option<AvailableShell>,
        ctx: &mut ViewContext<Self>,
    ) -> TerminalPaneId {
        let new_pane_id = self.add_session_with_default_session_mode_behavior(
            direction,
            Some(self.focused_pane_id(ctx)),
            self.active_session_id(ctx),
            chosen_shell,
            None, /* conversation_restoration */
            DefaultSessionModeBehavior::Ignore,
            ctx,
        );
        ctx.emit(Event::AppStateChanged);
        new_pane_id
    }

    /// Used when splitting panes.
    fn insert_terminal_pane(
        &mut self,
        direction: Direction,
        base_pane_id: PaneId,
        chosen_shell: Option<AvailableShell>,
        ctx: &mut ViewContext<Self>,
    ) -> TerminalPaneId {
        let base_session_id = base_pane_id
            .as_terminal_pane_id()
            .or(self.active_session_id(ctx));
        let new_pane_id = self.add_session(
            direction,
            Some(base_pane_id),
            base_session_id,
            chosen_shell,
            None, /* conversation_restoration */
            ctx,
        );
        ctx.emit(Event::AppStateChanged);
        new_pane_id
    }

    /// Creates a terminal pane that lives off-tree as a child agent pane.
    /// Unlike `insert_terminal_pane`, the new pane is never inserted into the
    /// layout tree at creation time — it lives only in `pane_contents` and
    /// `child_agent_panes`. The orchestration pill bar later inserts it into
    /// the tree on demand via `replace_pane` (in-place swap) or `panes.split`
    /// ("Open in new pane").
    ///
    /// When `is_shared_session_creator` is `Yes`, the new pane is recorded
    /// in `transitively_shared_child_panes` keyed by `base_pane_id` so the
    /// host's `StopSharingCurrentSession` cleans it up.
    fn insert_terminal_pane_hidden_for_child_agent(
        &mut self,
        base_pane_id: PaneId,
        env_vars: HashMap<OsString, OsString>,
        is_shared_session_creator: IsSharedSessionCreator,
        ctx: &mut ViewContext<Self>,
    ) -> TerminalPaneId {
        let base_session_id = base_pane_id
            .as_terminal_pane_id()
            .or(self.active_session_id(ctx));
        let startup_directory = self.startup_path_for_new_session(base_session_id, ctx);
        let is_transitively_shared = matches!(
            &is_shared_session_creator,
            IsSharedSessionCreator::Yes { .. }
        );
        let (pane_data, _view) = self.create_terminal_pane_data(
            startup_directory,
            env_vars,
            is_shared_session_creator,
            None,
            None,
            ctx,
        );
        let new_pane_id = pane_data.terminal_pane_id();
        if is_transitively_shared {
            self.transitively_shared_child_panes
                .entry(base_pane_id)
                .or_default()
                .insert(new_pane_id.into());
        }
        self.attach_child_pane_off_tree(Box::new(pane_data), ctx);
        new_pane_id
    }

    /// Dispatches a share on every direct child agent pane in this group
    /// that isn't already sharing, mirroring
    /// `terminal_pane::inherit_share_for_local_child` for children that
    /// existed before the host started sharing.
    #[cfg(not(target_family = "wasm"))]
    fn transitively_share_existing_local_children(
        &mut self,
        host_conversation_id: AIConversationId,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(host_pane_id) = self.pane_id_for_owned_conversation(host_conversation_id, ctx)
        else {
            return;
        };
        let Some(host_terminal_view) = self.terminal_view_from_pane_id(host_pane_id, ctx) else {
            return;
        };
        let Some(host_source) = host_terminal_shared_session_source_type(&host_terminal_view, ctx)
        else {
            return;
        };
        if host_source.orchestrator_task_id().is_none() {
            return;
        }

        let direct_child_ids: Vec<AIConversationId> = BlocklistAIHistoryModel::as_ref(ctx)
            .child_conversation_ids_of(&host_conversation_id)
            .to_vec();

        let mut planned: Vec<(PaneId, AmbientAgentTaskId)> = Vec::new();
        for child_conversation_id in direct_child_ids {
            let Some(child_pane_id) = self
                .child_agent_panes
                .get(&child_conversation_id)
                .copied()
                .filter(|pane_id| self.has_pane_id(*pane_id))
            else {
                continue;
            };
            let Some(child_task_id) = BlocklistAIHistoryModel::as_ref(ctx)
                .conversation(&child_conversation_id)
                .and_then(|c| c.task_id())
            else {
                continue;
            };
            planned.push((child_pane_id, child_task_id));
        }

        for (child_pane_id, child_task_id) in planned {
            let Some(child_terminal_view) = self.terminal_view_from_pane_id(child_pane_id, ctx)
            else {
                continue;
            };
            // Skip if the child is already sharing / pending / viewing.
            let already_in_shared_state = child_terminal_view
                .as_ref(ctx)
                .model
                .lock()
                .shared_session_status()
                .is_sharer_or_viewer();
            if already_in_shared_state {
                continue;
            }

            let creator = inherit_share_for_local_child(Some(&host_source), child_task_id);
            let IsSharedSessionCreator::Yes { source } = creator else {
                continue;
            };

            // Record in the host's transitive-share tracking set so the
            // host's stop-share also stops this child.
            self.transitively_shared_child_panes
                .entry(host_pane_id)
                .or_default()
                .insert(child_pane_id);

            child_terminal_view.update(ctx, |view, ctx| {
                view.attempt_to_share_session(
                    shared_session::SharedSessionScrollbackType::All,
                    None,
                    source,
                    /* bypass_conversation_guard = */ false,
                    ctx,
                );
            });
        }
    }

    /// Stop the shared session on every child pane that was transitively
    /// shared from `host_pane_id`. Only called from a non-wasm dispatch arm
    /// (`Event::StopSharingCurrentSession`), so the definition mirrors that
    /// cfg gate to keep wasm builds warning-clean.
    #[cfg(not(target_family = "wasm"))]
    fn stop_transitively_shared_child_shares(
        &mut self,
        host_pane_id: PaneId,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(child_pane_ids) = self.transitively_shared_child_panes.remove(&host_pane_id)
        else {
            return;
        };
        for child_pane_id in child_pane_ids {
            let Some(terminal_view) = self.terminal_view_from_pane_id(child_pane_id, ctx) else {
                continue;
            };
            let is_sharing = terminal_view
                .as_ref(ctx)
                .model
                .lock()
                .shared_session_status()
                .is_sharer();
            if !is_sharing {
                continue;
            }
            terminal_view.update(ctx, |view, ctx| {
                view.stop_sharing_session(SharedSessionActionSource::NonUser, ctx);
            });
        }
    }

    /// Removes `pane_id` from the transitive-share tracking map.
    fn forget_transitively_shared_pane(&mut self, pane_id: PaneId) {
        // The pane may be a host (key) or a transitively-shared child (value).
        self.transitively_shared_child_panes.remove(&pane_id);
        self.transitively_shared_child_panes
            .retain(|_host, children| {
                children.remove(&pane_id);
                !children.is_empty()
            });
    }

    /// Creates a cloud-mode pane that lives off-tree as a child agent pane.
    /// Unlike `create_ambient_agent_pane`, this leaves the new terminal view
    /// uninitialized so callers can create and select the child conversation
    /// explicitly before the deferred shared-session viewer binds to it.
    fn insert_ambient_agent_pane_hidden_for_child_agent(
        &mut self,
        _base_pane_id: PaneId,
        ctx: &mut ViewContext<Self>,
    ) -> TerminalPaneId {
        let uuid = Uuid::new_v4();
        let resources = TerminalViewResources {
            tips_completed: self.tips_completed.clone(),
            server_api: self.server_api.clone(),
            model_event_sender: self.model_event_sender.clone(),
        };
        let view_bounds = Self::estimated_view_bounds(ctx);
        // Per-child cloud-mode pane: the parent already polls for
        // descendants, so disable polling on this child.
        let (view, terminal_manager) =
            Self::create_cloud_mode_terminal(resources, view_bounds.size(), false, ctx);
        view.update(ctx, |view, _| {
            view.suppress_initial_conversation_details_panel_auto_open();
        });
        let pane_data = TerminalPane::new(
            uuid.as_bytes().to_vec(),
            terminal_manager,
            view,
            self.model_event_sender.clone(),
            ctx,
        );
        let new_pane_id = pane_data.terminal_pane_id();
        self.attach_child_pane_off_tree(Box::new(pane_data), ctx);
        new_pane_id
    }

    /// Inserts `pane` into `pane_contents` and attaches it (so subscriptions,
    /// focus handle, etc. are wired up) without adding it to the layout tree.
    /// Used for child agent panes which only enter the tree later via the
    /// pill bar's swap or split-off paths.
    fn attach_child_pane_off_tree(
        &mut self,
        pane: Box<dyn AnyPaneContent>,
        ctx: &mut ViewContext<Self>,
    ) -> Option<PaneId> {
        let pane_id = pane.as_pane().id();
        self.pane_contents.insert(pane_id, pane);
        let pane = self
            .pane_contents
            .get(&pane_id)
            .expect("Just inserted pane");
        if !self.try_attach_pane(pane.as_ref(), ctx) {
            self.pane_contents.remove(&pane_id);
            return None;
        }
        Some(pane_id)
    }

    /// Get the [`PaneView<TerminalView>`] for the pane at `pane_index`, if that pane is:
    /// 1. In bounds
    /// 2. A terminal pane
    #[cfg(any(test, feature = "integration_tests"))]
    pub fn terminal_pane_view_at_pane_index(
        &self,
        pane_index: usize,
    ) -> Option<ViewHandle<self::pane::terminal_pane::TerminalPaneView>> {
        self.terminal_session_by_pane_index(pane_index)
            .map(|session| session.pane_view())
    }

    /// Get the [`TerminalView`] within the pane at `pane_index`, if that pane is:
    /// 1. In bounds
    /// 2. A terminal pane
    #[cfg(any(test, feature = "integration_tests"))]
    pub fn terminal_view_at_pane_index(
        &self,
        pane_index: usize,
        ctx: &AppContext,
    ) -> Option<ViewHandle<TerminalView>> {
        self.terminal_session_by_pane_index(pane_index)
            .map(|session| session.terminal_view(ctx))
    }

    /// Gets the pane ID for the pane at `pane_index`, if any.
    /// Only considers visible panes (excludes panes hidden for close, move, job, etc.).
    pub fn pane_id_from_index(&self, pane_index: usize) -> Option<PaneId> {
        self.panes.visible_pane_ids().get(pane_index).copied()
    }

    pub fn visible_pane_ids(&self) -> Vec<PaneId> {
        self.panes.visible_pane_ids()
    }

    pub fn original_pane_for_replacement(&self, replacement_pane_id: PaneId) -> Option<PaneId> {
        self.panes
            .original_pane_for_replacement(replacement_pane_id)
    }

    pub fn pane_ids(&self) -> impl Iterator<Item = PaneId> + '_ {
        self.pane_contents.keys().copied()
    }

    pub fn has_pane_id(&self, pane_id: PaneId) -> bool {
        self.pane_contents.contains_key(&pane_id)
    }

    /// Get the notebook view within the pane at `pane_index`.
    #[cfg(any(test, feature = "integration_tests"))]
    pub fn notebook_view_at_pane_index(
        &self,
        pane_index: usize,
        ctx: &AppContext,
    ) -> Option<ViewHandle<crate::notebooks::notebook::NotebookView>> {
        self.content_by_pane_index(pane_index)
            .and_then(|pane| pane.as_any().downcast_ref::<NotebookPane>())
            .map(|pane| pane.notebook_view(ctx))
    }

    /// Get the notebook view within the pane at `pane_index`.
    #[cfg(any(test, feature = "integration_tests"))]
    pub fn workflow_view_at_pane_index(
        &self,
        pane_index: usize,
        ctx: &AppContext,
    ) -> Option<ViewHandle<crate::workflows::workflow_view::WorkflowView>> {
        self.content_by_pane_index(pane_index)
            .and_then(|pane| pane.as_any().downcast_ref::<WorkflowPane>())
            .map(|pane| pane.get_view(ctx))
    }

    /// Find the ID of the pane at an index (going left to right, top to bottom).
    /// Only considers visible panes (excludes panes hidden for close, move, job, etc.).
    pub fn pane_id_by_index(&self, pane_index: usize) -> Option<PaneId> {
        self.panes.visible_pane_ids().get(pane_index).copied()
    }

    pub fn set_dim_even_if_focused_for_all_panes(
        &mut self,
        dim_even_if_focused: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        for pane in self.pane_contents.values() {
            let pane = pane.as_pane();
            let configuration = pane.pane_configuration();
            configuration.update(ctx, |config, ctx| {
                config.set_dim_even_if_focused(dim_even_if_focused, ctx);
            });
        }
    }

    pub fn set_left_panel_open(&mut self, is_open: bool, ctx: &mut ViewContext<Self>) {
        if self.left_panel_open != is_open {
            self.left_panel_open = is_open;
            ctx.emit(Event::LeftPanelToggled { is_open });
        }
        ctx.notify();
    }

    pub fn focus_first_pane(&mut self, ctx: &mut ViewContext<Self>) -> bool {
        if let Some(first) = self.panes.visible_pane_ids().first().copied() {
            return self.focus_pane_and_record_in_history(first, ctx);
        }
        false
    }

    pub fn focus_last_pane(&mut self, ctx: &mut ViewContext<Self>) -> bool {
        if let Some(last) = self.panes.visible_pane_ids().last().copied() {
            return self.focus_pane_and_record_in_history(last, ctx);
        }
        false
    }

    /// The current working directory of the active terminal session, if it's local.
    pub fn active_session_path(&self, ctx: &AppContext) -> Option<PathBuf> {
        self.session_path(&self.active_session_id(ctx)?, ctx)
    }

    fn session_path(&self, pane_id: &TerminalPaneId, ctx: &AppContext) -> Option<PathBuf> {
        self.terminal_view_from_pane_id(*pane_id, ctx)?
            .as_ref(ctx)
            .active_session_path_if_local(ctx)
    }

    fn content_by_pane_index(&self, index: usize) -> Option<&dyn AnyPaneContent> {
        self.content_by_pane_id(self.pane_id_by_index(index)?)
    }

    fn content_by_pane_id(&self, pane_id: PaneId) -> Option<&dyn AnyPaneContent> {
        self.pane_contents.get(&pane_id).map(|pane| pane.as_ref())
    }

    fn terminal_session_by_pane_index(&self, index: usize) -> Option<&TerminalPane> {
        self.content_by_pane_index(index)
            .and_then(|pane| pane.as_any().downcast_ref())
    }

    pub fn any_pane_being_dragged(&self, app: &AppContext) -> bool {
        self.pane_contents
            .iter()
            .any(|(_, pane_content)| pane_content.as_pane().is_pane_being_dragged(app))
    }

    /// Removes the given pane id from the pane group, focusing the previous active session
    /// and pane and returning the Box<dyn AnyPaneContent> of the removed pane. Note that this
    /// is primarily used for pane management, and should not be used if you are planning on closing
    /// the session as this does not call the needed clean up code and does not add the tab
    /// to the undo stack if it gets closed.
    pub fn remove_pane_for_move(
        &mut self,
        pane_id: &PaneId,
        ctx: &mut ViewContext<Self>,
    ) -> Option<Box<dyn AnyPaneContent>> {
        // Clear any hidden pane entry since the pane is being permanently removed from this group.
        self.panes.remove_hidden_pane(*pane_id);

        let was_focused = self.focus_state.as_ref(ctx).is_pane_focused(*pane_id);
        self.focus_next_terminal_pane_and_activate_session(*pane_id, PaneRemovalReason::Move, ctx);
        if self.pane_count() == 1 {
            ctx.emit(Event::Exited {
                add_to_undo_stack: false,
            });
        }

        match self.pane_contents.get(pane_id) {
            Some(data) => {
                let pane = data.as_pane();
                pane.detach(self, DetachType::Moved, ctx);
            }
            None => log::error!("Could not find data for pane id: {pane_id:?}"),
        };

        if !self.panes.remove(*pane_id) {
            log::error!("Pane not found");
        }

        let pane_content = self.pane_contents.remove(pane_id);

        let in_split_pane = self.panes.visible_pane_count() > 1;
        self.focus_state.update(ctx, |focus_state, ctx| {
            focus_state.set_in_split_pane(in_split_pane, ctx);
            // If the focused+maximized pane was removed, stop maximizing panes.
            if was_focused {
                focus_state.set_focused_pane_maximized(false, ctx);
            }
        });

        ctx.notify();
        ctx.emit(Event::TerminalViewStateChanged);
        ctx.emit(Event::AppStateChanged);
        pane_content
    }

    pub fn notebook_pane_by_pane_id(&self, pane_id: Option<PaneId>) -> Option<&NotebookPane> {
        self.downcast_pane_by_id(pane_id?)
    }

    pub fn env_var_collection_pane_by_pane_id(
        &self,
        pane_id: Option<PaneId>,
    ) -> Option<&EnvVarCollectionPane> {
        self.downcast_pane_by_id(pane_id?)
    }

    pub fn workflow_pane_by_pane_id(&self, pane_id: Option<PaneId>) -> Option<&WorkflowPane> {
        self.downcast_pane_by_id(pane_id?)
    }

    pub fn ai_fact_pane_by_pane_id(&self, pane_id: Option<PaneId>) -> Option<&AIFactPane> {
        self.downcast_pane_by_id(pane_id?)
    }

    pub fn code_pane_by_id(&self, pane_id: PaneId) -> Option<&CodePane> {
        self.downcast_pane_by_id(pane_id)
    }

    /// Removes an editor tab from a code pane for moving to another location.
    /// Returns the removed tab as a CodePane if the operation succeeds.
    pub fn remove_editor_tab_for_move(
        &mut self,
        pane_id: PaneId,
        editor_tab_index: usize,
        ctx: &mut ViewContext<Self>,
    ) -> Option<Box<dyn AnyPaneContent>> {
        self.code_pane_by_id(pane_id)
            .and_then(|pane| {
                pane.file_view(ctx).update(ctx, |file_view, ctx| {
                    file_view.remove_tab_for_move(editor_tab_index, ctx)
                })
            })
            .map(|p| Box::new(p) as Box<dyn AnyPaneContent>)
    }

    /// The generic pane at `index`, if it exists.
    pub fn pane_by_index(&self, index: usize) -> Option<&dyn PaneContent> {
        self.content_by_pane_index(index).map(|pane| pane.as_pane())
    }

    /// The generic pane with the given pane ID, if it exists.
    pub fn pane_by_id(&self, pane_id: PaneId) -> Option<&dyn PaneContent> {
        self.content_by_pane_id(pane_id).map(|pane| pane.as_pane())
    }

    /// Get a pane's contents by ID. This returns `None` if the pane does not exist or is of the
    /// wrong type.
    pub fn downcast_pane_by_id<T: Any + 'static>(&self, pane_id: PaneId) -> Option<&T> {
        self.content_by_pane_id(pane_id)?.as_any().downcast_ref()
    }

    /// Returns true if the given pane is hidden for close (undo functionality).
    pub fn is_pane_hidden_for_close(&self, pane_id: PaneId) -> bool {
        self.panes.is_hidden_closed_pane(&pane_id)
    }

    /// Emits an event for the workspace to show a confirmation dialog if necessary, or closes immediately if not.
    /// If a dialog is opened, the workspace may call back into pane group to close the pane after the user confirms.
    pub fn close_pane_with_confirmation(&mut self, pane_id: PaneId, ctx: &mut ViewContext<Self>) {
        // Child agent panes are just hidden when closed, so skip the
        // "process running" warning—it doesn't apply.
        if self.is_child_agent_pane(pane_id) {
            self.close_pane(pane_id, ctx);
            return;
        }

        if let Some(terminal_manager) = self
            .terminal_session_by_id(pane_id)
            .map(|session| session.terminal_manager(ctx))
        {
            if terminal_manager.read(ctx, |terminal_manager, _ctx| {
                terminal_manager
                    .model()
                    .lock()
                    .shared_session_status()
                    .is_sharer()
            }) {
                ctx.emit(Event::CloseSharedSessionPaneRequested { pane_id });
                return;
            }
        }

        let summary = UnsavedStateSummary::for_pane(self, pane_id, ctx);
        if summary.should_display_warning(ctx) && ChannelState::channel() != Channel::Integration {
            log::info!("Displaying unsaved changes warning for pane");
            let confirm_self = ctx.handle();
            let show_process_self = ctx.handle();
            let dialog = summary
                .dialog()
                .on_confirm(move |ctx| {
                    if let Some(pane_group) = confirm_self.upgrade(ctx) {
                        pane_group.update(ctx, |pane_group, ctx| {
                            pane_group.close_pane(pane_id, ctx);
                        });
                    }
                })
                .on_show_processes(move |ctx| {
                    if let Some(pane_group) = show_process_self.upgrade(ctx) {
                        pane_group.update(ctx, |_, ctx| {
                            ctx.emit(Event::OpenPalette {
                                mode: PaletteMode::Navigation,
                                source: PaletteSource::QuitModal,
                                query: Some("running".to_string()),
                            });
                        })
                    }
                })
                .on_cancel(|_ctx| {});

            if dialog.show(ctx) {
                return;
            }
        }

        self.close_pane(pane_id, ctx);
    }

    /// Definitively close the pane. This does not go through the undo close check where we might hide the pane instead of
    /// discarding it.
    fn discard_pane(&mut self, pane_id: PaneId, ctx: &mut ViewContext<Self>) {
        // Skip ownership transfer for child agent panes (their view
        // canonically owns the conversation).
        if !self.is_child_agent_pane(pane_id) {
            self.transfer_child_agent_conversations_to_parents_on_close(pane_id, ctx);
        }

        if let Some(terminal_view) = self.terminal_view_from_pane_id(pane_id, ctx) {
            let terminal_view_id = terminal_view.id();

            // Discard any child agent panes parented by this terminal view.
            self.remove_child_agent_panes(terminal_view_id, ctx);

            // Preserve conversations from terminal views before cleaning up the pane
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, _| {
                history_model.mark_conversations_historical_for_terminal_view(terminal_view_id);
            });
        }

        self.cleanup_closed_pane(pane_id, ctx);
    }

    /// Best-effort: re-bind each live child agent conversation on the
    /// closing view to the pane that owns its parent. Defensive plumbing
    /// for paths where the parent's view actually contains the child;
    /// no-ops otherwise.
    fn transfer_child_agent_conversations_to_parents_on_close(
        &mut self,
        pane_id: PaneId,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(terminal_view) = self.terminal_view_from_pane_id(pane_id, ctx) else {
            return;
        };
        let closing_view_id = terminal_view.id();

        let history_handle = BlocklistAIHistoryModel::handle(ctx);
        let transfers: Vec<(AIConversationId, EntityId)> = history_handle
            .as_ref(ctx)
            .all_live_conversations_for_terminal_view(closing_view_id)
            .filter_map(|conversation| {
                let parent_id = conversation.parent_conversation_id()?;
                let parent_owner = history_handle
                    .as_ref(ctx)
                    .terminal_view_id_for_conversation(&parent_id)?;
                if parent_owner == closing_view_id {
                    return None;
                }
                Some((conversation.id(), parent_owner))
            })
            .collect();

        if transfers.is_empty() {
            return;
        }
        history_handle.update(ctx, |history_model, ctx| {
            for (child_id, parent_owner) in transfers {
                history_model.set_active_conversation_id(child_id, parent_owner, ctx);
            }
        });
    }

    /// If this pane was the active session and or focused pane, focuses the previous session and pane.
    ///
    /// Called before removing a pane from a pane group (either because the pane is being closed or because it is being moved
    /// to another pane group). Also does some other pane clean up actions like remove the pane from history.
    fn focus_next_terminal_pane_and_activate_session(
        &mut self,
        pane_id_to_remove: PaneId,
        pane_removal_reason: PaneRemovalReason,
        ctx: &mut ViewContext<Self>,
    ) {
        // If we're removing the latest active terminal pane, activate the last focused session. If
        // focus changes to another terminal pane, that will become focused instead.
        if Some(pane_id_to_remove) == self.active_session_id(ctx).map(Into::into) {
            let new_active_session = self.choose_active_session(pane_id_to_remove);
            self.focus_state.update(ctx, |focus_state, ctx| {
                focus_state.set_active_session(new_active_session, ctx);
            });
        }

        // Only change the focus if we're removing the focused pane
        if pane_id_to_remove == self.focused_pane_id(ctx) {
            match self.prev_pane_id(pane_id_to_remove) {
                Some(id) => {
                    self.focus_pane(id, pane_removal_reason == PaneRemovalReason::Close, ctx);
                }
                None => {
                    log::error!("[PaneGroup] Unable to locate a panel to activate after close");
                }
            };
        } else {
            // If not, we still need to call notify to let the UI framework know about changes
            ctx.notify();
        }

        self.remove_from_pane_history(pane_id_to_remove);
    }

    /// Returns true if the given pane is a child agent pane tracked in `child_agent_panes`.
    fn is_child_agent_pane(&self, pane_id: PaneId) -> bool {
        self.child_agent_panes.values().any(|&id| id == pane_id)
    }

    /// Collects the child agent pane IDs whose conversations are parented by
    /// a conversation on the given terminal view.
    fn child_pane_ids_for_parent(
        &self,
        parent_terminal_view_id: EntityId,
        ctx: &AppContext,
    ) -> Vec<(AIConversationId, PaneId)> {
        let history_model = BlocklistAIHistoryModel::as_ref(ctx);
        self.child_agent_panes
            .iter()
            .filter(|(conv_id, _)| {
                history_model
                    .conversation(conv_id)
                    .and_then(|c| c.parent_conversation_id())
                    .and_then(|parent_id| {
                        history_model.terminal_view_id_for_conversation(&parent_id)
                    })
                    .is_some_and(|tv_id| tv_id == parent_terminal_view_id)
            })
            .map(|(conv_id, pane_id)| (*conv_id, *pane_id))
            .collect()
    }

    /// Removes and discards all child agent panes whose parent conversation
    /// lives on the given terminal view.  Used by both `close_pane` and
    /// `discard_pane` to ensure children are cleaned up regardless of which
    /// path removes the parent.
    fn remove_child_agent_panes(
        &mut self,
        parent_terminal_view_id: EntityId,
        ctx: &mut ViewContext<Self>,
    ) {
        let children = self.child_pane_ids_for_parent(parent_terminal_view_id, ctx);
        for (conv_id, child_pane_id) in children {
            self.child_agent_panes.remove(&conv_id);
            self.panes.remove_hidden_pane(child_pane_id);
            self.discard_pane(child_pane_id, ctx);
        }
    }

    /// Permanently discards the pane backing a child agent conversation.
    pub fn discard_child_agent_pane_for_conversation(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        let tracked_child_pane = self.child_agent_panes.remove(&conversation_id);
        let split_off_child_pane = self.child_agent_origin.as_ref().and_then(|origin| {
            (origin.conversation_id == conversation_id)
                .then(|| self.pane_id_for_conversation_owner(conversation_id, ctx))
                .flatten()
        });
        let owner_child_pane = tracked_child_pane
            .or(split_off_child_pane)
            .or_else(|| self.pane_id_for_conversation_owner(conversation_id, ctx));
        let Some(child_pane_id) = owner_child_pane else {
            return false;
        };

        if self
            .child_agent_origin
            .as_ref()
            .is_some_and(|origin| origin.conversation_id == conversation_id)
        {
            // Killed split-off tabs should not be re-adopted.
            self.child_agent_origin = None;
        }

        let was_focused = self.focus_state.as_ref(ctx).is_pane_focused(child_pane_id);

        if let Some(terminal_view) = self.terminal_view_from_pane_id(child_pane_id, ctx) {
            terminal_view.update(ctx, |view, ctx| {
                view.clear_orchestration_split_off(ctx);
                view.shutdown_pty(ctx);
            });
        }

        if let Some(original_pane_id) = self.panes.original_pane_for_replacement(child_pane_id) {
            self.panes.revert_temporary_replacement(child_pane_id);
            if was_focused {
                self.focus_pane(original_pane_id, true, ctx);
            }
        } else {
            // Drop any hidden entry that could restore the killed pane.
            self.panes.remove_hidden_pane(child_pane_id);
        }

        let is_in_tree = self.panes.is_pane_in_tree(child_pane_id);
        if is_in_tree && self.panes.visible_pane_count() <= 1 {
            // A lone split-off child closes by removing its tab.
            ctx.emit(Event::Exited {
                add_to_undo_stack: false,
            });
            return true;
        }

        if is_in_tree {
            self.focus_next_terminal_pane_and_activate_session(
                child_pane_id,
                PaneRemovalReason::Close,
                ctx,
            );
        }

        let discarded = self.cleanup_closed_pane(child_pane_id, ctx);
        self.handle_pane_count_change(ctx);
        discarded
    }

    pub fn close_pane(&mut self, pane_id: PaneId, ctx: &mut ViewContext<Self>) {
        // Don't close a pane that doesn't exist
        if !self.pane_contents.contains_key(&pane_id) {
            return;
        }

        // Child agent panes return to off-tree state instead of being
        // destroyed; future pill clicks re-host the same view. The view
        // keeps ownership of its conversation, so we skip the
        // transfer-on-close step below.
        if self.is_child_agent_pane(pane_id) {
            // Revert the swap if the child is currently swapped in.
            if self.panes.original_pane_for_replacement(pane_id).is_some() {
                self.panes.revert_temporary_replacement(pane_id);
            }
            // Or remove the child from the tree if it was split off.
            else if self.panes.is_pane_in_tree(pane_id) && !self.panes.remove(pane_id) {
                log::error!("close_pane: failed to remove split-off child pane from tree");
            }
            // Drop any leftover swap entry recording this child as the
            // original side. Otherwise a later revert of the surviving
            // sibling could resurrect the just-closed pane.
            self.panes.remove_hidden_pane(pane_id);

            // Clear the split-off marker so the next reveal renders pills
            // rather than breadcrumbs.
            if let Some(terminal_view) = self.terminal_view_from_pane_id(pane_id, ctx) {
                terminal_view.update(ctx, |view, ctx| {
                    view.clear_orchestration_split_off(ctx);
                });
            }
            self.focus_next_terminal_pane_and_activate_session(
                pane_id,
                PaneRemovalReason::Close,
                ctx,
            );
            self.handle_pane_count_change(ctx);
            ctx.emit(Event::TerminalViewStateChanged);
            ctx.emit(Event::AppStateChanged);
            return;
        }

        // Best-effort: re-bind any child conversations on this view back
        // to the pane that owns their parent so the pill bar keeps
        // working after this pane closes.
        self.transfer_child_agent_conversations_to_parents_on_close(pane_id, ctx);

        // If this is a parent with child agents, discard the children first.
        if let Some(terminal_view) = self.terminal_view_from_pane_id(pane_id, ctx) {
            self.remove_child_agent_panes(terminal_view.id(), ctx);
        }

        if FeatureFlag::UndoClosedPanes.is_enabled() {
            // Don't clase a pane that's already been hidden to allow for undo functionality
            if self.is_pane_hidden_for_close(pane_id) {
                return;
            }

            if self.panes.visible_pane_count() == 1 {
                // Tell the workspace that this pane group is now empty without
                // doing any additional clean-up work.  This ensures we don't
                // pre-emptively delete any state that we might want to retain
                // if the user re-opens the closed tab.
                ctx.emit(Event::Exited {
                    add_to_undo_stack: true,
                });

                return;
            }

            if let Some(pane_data) = self.pane_contents.get(&pane_id) {
                let pane = pane_data.as_pane();
                pane.detach(self, DetachType::HiddenForClose, ctx);

                let pane_group_handle = ctx.handle();
                UndoCloseStack::handle(ctx).update(ctx, |stack, ctx| {
                    stack.handle_pane_closed_by_id(pane_group_handle, pane_id, ctx);
                });
                self.hide_closed_pane(pane_id, ctx);
            }

            // Remove opened share modal associated with the closing session.
            if Some(pane_id) == self.terminal_with_open_share_block_modal.map(Into::into) {
                self.terminal_with_open_share_block_modal = None;
            }

            if self.pane_with_open_environment_setup_mode_selector == Some(pane_id) {
                self.pane_with_open_environment_setup_mode_selector = None;
            }
            if self.pane_with_open_agent_assisted_environment_modal == Some(pane_id) {
                self.pane_with_open_agent_assisted_environment_modal = None;
            }

            self.focus_next_terminal_pane_and_activate_session(
                pane_id,
                PaneRemovalReason::Close,
                ctx,
            );
        } else {
            if self.pane_count() == 1 {
                // Tell the workspace that this pane group is now empty without
                // doing any additional clean-up work.  This ensures we don't
                // pre-emptively delete any state that we might want to retain
                // if the user re-opens the closed tab.
                ctx.emit(Event::Exited {
                    add_to_undo_stack: true,
                });

                return;
            }

            self.clean_up_pane(pane_id, ctx);

            // Remove opened share modal associated with the closing session.
            if Some(pane_id) == self.terminal_with_open_share_block_modal.map(Into::into) {
                self.terminal_with_open_share_block_modal = None;
            }

            if self.pane_with_open_environment_setup_mode_selector == Some(pane_id) {
                self.pane_with_open_environment_setup_mode_selector = None;
            }
            if self.pane_with_open_agent_assisted_environment_modal == Some(pane_id) {
                self.pane_with_open_agent_assisted_environment_modal = None;
            }

            self.focus_next_terminal_pane_and_activate_session(
                pane_id,
                PaneRemovalReason::Close,
                ctx,
            );

            self.pane_contents.remove(&pane_id);

            // We should only remove the session id from the tree after we queried
            // and got the previous session id.
            if !self.panes.remove(pane_id) {
                log::error!("Pane not found");
            }

            // Mirror cleanup_closed_pane's transitive-share map cleanup so
            // the non-undo close path doesn't leak stale entries.
            self.forget_transitively_shared_pane(pane_id);
        }

        self.handle_pane_count_change(ctx);

        ctx.emit(Event::TerminalViewStateChanged);
        ctx.emit(Event::AppStateChanged);
    }

    pub fn close_pane_and_focus(
        &mut self,
        pane_id: PaneId,
        pane_to_focus: PaneId,
        ctx: &mut ViewContext<Self>,
    ) {
        // Child agent panes go through the normal close path so the
        // underlying view is preserved (the temp-replacement close path
        // would destroy it).
        if self.is_child_agent_pane(pane_id) {
            self.close_pane(pane_id, ctx);
            ctx.emit(Event::FocusPane { pane_to_focus });
            return;
        }

        // Check if this is a temporary replacement that should be reverted
        if self.panes.is_temporary_replacement(pane_id) {
            // Remove the replacement pane and focus the original pane
            let focused_pane_id = self
                .close_temporary_replacement_pane(pane_id, ctx)
                .unwrap_or(pane_to_focus);
            ctx.emit(Event::FocusPane {
                pane_to_focus: focused_pane_id,
            });
            ctx.notify();
        } else {
            // Normal pane close behavior
            self.close_pane(pane_id, ctx);
            ctx.emit(Event::FocusPane { pane_to_focus });
        }
    }

    /// Revert a temporary-replacement swap and clear the orchestration
    /// split-off marker on the replacement's view, so a later reveal
    /// renders pills rather than breadcrumbs.
    fn revert_swap_clearing_split_off(
        &mut self,
        replacement_id: PaneId,
        ctx: &mut ViewContext<Self>,
    ) {
        if let Some(terminal_view) = self.terminal_view_from_pane_id(replacement_id, ctx) {
            terminal_view.update(ctx, |view, ctx| {
                view.clear_orchestration_split_off(ctx);
            });
        }
        self.panes.revert_temporary_replacement(replacement_id);
    }

    /// Reveal `pane_id` if it's currently the original of an active swap,
    /// then focus it. Used by cross-tab navigation paths that may resolve
    /// to a swapped-out pane; without the reveal, focus would land on an
    /// off-tree pane the user can't see. Logs a warning if the pane is
    /// neither in the tree nor swap-hidden.
    pub fn reveal_and_focus_pane(&mut self, pane_id: PaneId, ctx: &mut ViewContext<Self>) {
        if let Some(replacement_id) = self.panes.replacement_pane_for_original(pane_id) {
            self.revert_swap_clearing_split_off(replacement_id, ctx);
            self.handle_pane_count_change(ctx);
            // The visible content of this slot changed; refresh agent-view
            // back-button labels on both sides.
            for refresh_pane_id in [pane_id, replacement_id] {
                if let Some(terminal_view) = self.terminal_view_from_pane_id(refresh_pane_id, ctx) {
                    terminal_view.update(ctx, |view, ctx| {
                        view.update_agent_view_back_button_state(ctx);
                    });
                }
            }
            ctx.emit(Event::TerminalViewStateChanged);
            ctx.emit(Event::AppStateChanged);
        } else if !self.panes.is_pane_in_tree(pane_id) {
            log::warn!(
                "reveal_and_focus_pane: pane {pane_id:?} is off-tree; focus will land on a non-visible pane"
            );
        }
        self.focus_pane_by_id(pane_id, ctx);
    }

    /// Temporarily replace a pane with another pane.
    /// The original pane is hidden and can be restored later.
    /// Returns true if the replacement was successful, false otherwise.
    pub fn replace_pane<C: PaneContent>(
        &mut self,
        original_pane_id: PaneId,
        replacement_pane: C,
        is_temporary: bool,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        // Ensure original pane exists before attempting replacement
        if !self.pane_contents.contains_key(&original_pane_id) {
            log::error!(
                "Attempted to replace pane {original_pane_id:?} that doesn't exist in contents"
            );
            return false;
        }

        let Some(replacement_pane_id) = self.add_pane_for_replacement(replacement_pane, ctx) else {
            log::error!(
                "Failed to create replacement pane for {original_pane_id:?} because attachment was prevented"
            );
            return false;
        };
        let success = self
            .panes
            .replace_pane(original_pane_id, replacement_pane_id, is_temporary);

        if success {
            // For permanent replacements, clean up the original pane
            if !is_temporary {
                self.clean_up_pane(original_pane_id, ctx);
                self.pane_contents.remove(&original_pane_id);
            }
            self.restore_missing_child_agent_panes_for_terminal_pane_if_needed(
                replacement_pane_id,
                ctx,
            );

            // Focus the replacement pane to ensure proper user interaction
            self.focus_pane_by_id(replacement_pane_id, ctx);
        } else {
            // If tree replacement failed, clean up the replacement pane we just created
            log::error!(
                "Failed to replace pane {original_pane_id:?} with {replacement_pane_id:?} in tree structure"
            );
            self.clean_up_pane(replacement_pane_id, ctx);
            self.pane_contents.remove(&replacement_pane_id);
        }

        ctx.notify();
        ctx.emit(Event::AppStateChanged);
        success
    }

    fn close_temporary_replacement_pane(
        &mut self,
        replacement_pane_id: PaneId,
        ctx: &mut ViewContext<Self>,
    ) -> Option<PaneId> {
        let original_pane_id = self.panes.revert_temporary_replacement(replacement_pane_id);
        self.clean_up_pane(replacement_pane_id, ctx);
        self.pane_contents.remove(&replacement_pane_id);

        if let Some(original_id) = original_pane_id {
            // Focus the original pane to ensure proper user interaction
            self.focus_pane_by_id(original_id, ctx);
        }

        original_pane_id
    }

    #[cfg(feature = "local_fs")]
    fn replace_file_pane_with_code_pane(
        &mut self,
        file_pane_id: PaneId,
        path: LocalOrRemotePath,
        source: Option<crate::code::editor_management::CodeSource>,
        ctx: &mut ViewContext<Self>,
    ) {
        use crate::code::editor_management::CodeSource;
        use crate::pane_group::CodePane;

        // Use the provided source if available, or construct from the path.
        let source = source.unwrap_or(CodeSource::FileTree { location: path });

        let code_pane = CodePane::new(source, None, ctx);
        let success = self.replace_pane(file_pane_id, code_pane, false, ctx);

        if !success {
            log::error!("Failed to replace file pane {file_pane_id:?} with code pane");
        }
    }

    #[cfg(feature = "local_fs")]
    fn replace_code_pane_with_file_pane(
        &mut self,
        code_pane_id: PaneId,
        path: LocalOrRemotePath,
        source: Option<crate::code::editor_management::CodeSource>,
        ctx: &mut ViewContext<Self>,
    ) {
        // Get the active session to pass to the FilePane, if any
        let session = self.active_session_view(ctx).and_then(|view| {
            // Use the active session if it's local
            let view_ref = view.as_ref(ctx);
            if view_ref.active_session_is_local(ctx) == Some(true) {
                view_ref
                    .active_block_session_id()
                    .and_then(|session_id| view_ref.sessions_model().as_ref(ctx).get(session_id))
            } else {
                None
            }
        });

        let file_pane = FilePane::new(Some(path), session, source, ctx);
        let success = self.replace_pane(code_pane_id, file_pane, false, ctx);

        if !success {
            log::error!("Failed to replace code pane {code_pane_id:?} with file pane");
        }
    }

    /// Handle a common pane event, such as splitting off another pane.
    fn handle_pane_event(
        &mut self,
        pane_id: PaneId,
        event: &PaneEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            PaneEvent::Close => self.close_pane(pane_id, ctx),
            PaneEvent::CloseAndFocus { pane_to_focus } => {
                self.close_pane_and_focus(pane_id, *pane_to_focus, ctx);
            }
            // Pane-splitting events always create a new terminal pane, regardless of the original
            // pane's type. This makes it easy to get a terminal session next to a non-terminal
            // pane like a notebook. Once it's possible to open the same notebook more than once,
            // we may revisit this so that splitting from a terminal pane starts a new session, but
            // splitting from a notebook pane reopens the notebook side-by-side.
            PaneEvent::SplitLeft(chosen_shell) => {
                self.insert_terminal_pane(Direction::Left, pane_id, chosen_shell.clone(), ctx);
            }
            PaneEvent::SplitRight(chosen_shell) => {
                self.insert_terminal_pane(Direction::Right, pane_id, chosen_shell.clone(), ctx);
            }
            PaneEvent::SplitUp(chosen_shell) => {
                self.insert_terminal_pane(Direction::Up, pane_id, chosen_shell.clone(), ctx);
            }
            PaneEvent::SplitDown(chosen_shell) => {
                self.insert_terminal_pane(Direction::Down, pane_id, chosen_shell.clone(), ctx);
            }
            PaneEvent::ToggleMaximized => {
                // The toggled pane might not be the active pane -- focus it first.
                self.focus_pane_by_id(pane_id, ctx);
                self.toggle_maximize_pane(ctx);
            }
            PaneEvent::FocusSelf => self.focus_pane_by_id(pane_id, ctx),
            PaneEvent::FocusActiveSession => self.focus_active_session(ctx),
            PaneEvent::AppStateChanged => {
                ctx.emit(Event::AppStateChanged);
            }
            PaneEvent::NewPaneInAIMode { initial_query } => {
                self.add_terminal_pane_in_agent_mode(initial_query.as_deref(), None, ctx)
            }
            PaneEvent::ClearHoveredTabIndex => ctx.emit(Event::ClearHoveredTabIndex),
            #[cfg(feature = "local_fs")]
            PaneEvent::ReplaceWithCodePane { path, source } => {
                self.replace_file_pane_with_code_pane(pane_id, path.clone(), source.clone(), ctx);
            }
            #[cfg(feature = "local_fs")]
            PaneEvent::ReplaceWithFilePane { path, source } => {
                self.replace_code_pane_with_file_pane(pane_id, path.clone(), source.clone(), ctx);
            }
            PaneEvent::RepoChanged => {
                ctx.emit(Event::RepoChanged);
            }
            PaneEvent::RemoteRepoNavigated { remote_path } => {
                ctx.emit(Event::RemoteRepoNavigated {
                    remote_path: remote_path.clone(),
                });
            }
        }
    }

    /// The current pane group title, based on the focused pane.
    pub(crate) fn title(&self, ctx: &AppContext) -> String {
        self.focused_pane_content(ctx)
            .map(|pane| pane.pane_configuration().as_ref(ctx).title().to_owned())
            .unwrap_or_default()
    }

    /// The resolved display title for this pane group —
    /// custom title if set, otherwise the focused pane's title.
    pub fn display_title(&self, ctx: &AppContext) -> String {
        self.custom_title(ctx).unwrap_or_else(|| self.title(ctx))
    }

    /// The tab-level custom title, if one has been set via the rename-tab flow.
    pub fn custom_title(&self, _ctx: &AppContext) -> Option<String> {
        self.custom_title.clone()
    }

    /// The original title of the active terminal session (without custom title override).
    /// This returns the title that would be displayed if no custom title was set.
    pub fn original_title(&self, ctx: &AppContext) -> Option<String> {
        self.active_session_view(ctx)
            .map(|view| {
                let model = view.as_ref(ctx).model.lock();
                model
                    .terminal_title()
                    .or_else(|| Some(model.shell_launch_state().display_name().to_string()))
            })
            .unwrap_or_default()
    }

    pub fn set_title(&mut self, title: &str, ctx: &mut ViewContext<Self>) {
        self.custom_title = Some(title.to_string()).filter(|t| !t.is_empty());

        // refocus on the focused pane
        if let Some(pane) = self.focused_pane_content(ctx) {
            pane.focus(ctx);
        }
    }

    pub fn clear_title(&mut self, ctx: &mut ViewContext<Self>) {
        self.custom_title = None;

        // refocus on the focused pane
        if let Some(pane) = self.focused_pane_content(ctx) {
            pane.focus(ctx);
        }
    }

    fn close_active_pane_with_confirmation(&mut self, ctx: &mut ViewContext<Self>) {
        if self.focused_pane_id(ctx).is_code_pane() {
            // If focused on a CodePane, close its active editor tab (optionally, the entire pane if it only has 1 tab).
            if let Some(code_view) = self.code_view_from_pane_id(self.focused_pane_id(ctx), ctx) {
                code_view.update(ctx, |view, ctx| {
                    let index = view.active_tab_index();
                    view.handle_action(&CodeViewAction::RemoveTabAtIndex { index }, ctx);
                });
            } else {
                self.close_pane_with_confirmation(self.focused_pane_id(ctx), ctx);
            }
        } else {
            self.close_pane_with_confirmation(self.focused_pane_id(ctx), ctx);
        }
    }

    pub fn add_pane_as_hidden(
        &mut self,
        pane: Box<dyn AnyPaneContent>,
        direction: Direction,
        ctx: &mut ViewContext<Self>,
    ) {
        // Since we are hiding the pane before adding to the tree, use the requested
        // direction for the temporary preview split without affecting focus.
        let _ = self.add_pane_with_options(
            pane,
            AddPaneOptions {
                direction,
                base_pane_id: None,
                focus_new_pane: false,
                visibility: NewPaneVisibility::HiddenForMove,
                emit_app_state_changed: true,
            },
            ctx,
        );
    }

    /// We return a pane_id if the pane successfully attached
    /// Otherwise, we return None
    pub fn add_pane_for_replacement<C: PaneContent>(
        &mut self,
        pane: C,
        ctx: &mut ViewContext<Self>,
    ) -> Option<PaneId> {
        let pane_id = self.init_pane(Box::new(pane), ctx)?;
        ctx.emit(Event::AppStateChanged);
        Some(pane_id)
    }

    pub fn hide_pane_for_move(&mut self, id: PaneId, ctx: &mut ViewContext<Self>) {
        self.panes.hide_pane_for_move(id);

        ctx.notify();
        ctx.emit(Event::TerminalViewStateChanged);
        ctx.emit(Event::AppStateChanged);
    }

    /// Hide a pane for the purposes of running some hidden work. For example, uploading a file to a
    /// remote session.
    pub fn hide_pane_for_job(&mut self, id: PaneId, ctx: &mut ViewContext<Self>) {
        self.panes.hide_pane_for_job(id);

        ctx.notify();
        ctx.emit(Event::TerminalViewStateChanged);
        ctx.emit(Event::AppStateChanged);
    }

    /// Show a pane that was running some job. Undoes `PaneGroup::hide_pane_for_job`.
    pub fn show_pane_for_job(&mut self, id: PaneId, ctx: &mut ViewContext<Self>) {
        self.panes.show_pane_for_job(id);

        ctx.notify();
        ctx.emit(Event::TerminalViewStateChanged);
        ctx.emit(Event::AppStateChanged);
    }

    /// Toggles the visibility of a pane running some job and returns its new state:
    /// `true` if the pane is now visible, and `false` if it's now hidden.
    pub fn toggle_pane_visibility_for_job(
        &mut self,
        id: PaneId,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        let pane_open = self.panes.toggle_pane_visibility_for_job(id);

        ctx.notify();
        ctx.emit(Event::TerminalViewStateChanged);
        ctx.emit(Event::AppStateChanged);

        pane_open
    }

    /// Hide a pane for close/undo functionality without removing it from the tree.
    fn hide_closed_pane(&mut self, id: PaneId, ctx: &mut ViewContext<Self>) {
        self.panes.hide_closed_pane(id);

        ctx.notify();
        ctx.emit(Event::TerminalViewStateChanged);
        ctx.emit(Event::AppStateChanged);
    }

    /// Show a pane that was hidden for close. Used for undo functionality.
    /// Returns true if the pane was successfully shown, false if it wasn't found.
    fn unhide_closed_pane(&mut self, id: PaneId, ctx: &mut ViewContext<Self>) -> bool {
        let success = self.panes.unhide_closed_pane(id);
        if success {
            ctx.notify();
            ctx.emit(Event::TerminalViewStateChanged);
            ctx.emit(Event::AppStateChanged);
        }

        success
    }

    fn ambient_agent_task_id(
        cloud_conversation: &CloudConversationData,
    ) -> Option<AmbientAgentTaskId> {
        match cloud_conversation {
            CloudConversationData::Oz(conversation) => conversation
                .server_metadata()
                .and_then(|metadata| metadata.ambient_agent_task_id),
            CloudConversationData::CLIAgent(cli_conversation) => {
                cli_conversation.metadata.ambient_agent_task_id
            }
        }
    }

    fn replace_loading_pane_with_restored_ambient_cloud_mode_pane(
        &mut self,
        loading_pane_id: PaneId,
        cloud_conversation: CloudConversationData,
        task_id: AmbientAgentTaskId,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        let resources = TerminalViewResources {
            tips_completed: self.tips_completed.clone(),
            server_api: self.server_api.clone(),
            model_event_sender: self.model_event_sender.clone(),
        };
        let view_bounds = Self::estimated_view_bounds(ctx);
        let (terminal_view, terminal_manager) =
            Self::create_cloud_mode_terminal(resources, view_bounds.size(), true, ctx);
        let terminal_view_id = terminal_view.id();

        Self::load_data_into_restored_ambient_cloud_mode_view(
            terminal_view.clone(),
            cloud_conversation,
            task_id,
            ctx,
        );

        let pane_data = TerminalPane::new(
            Uuid::new_v4().as_bytes().to_vec(),
            terminal_manager,
            terminal_view,
            self.model_event_sender.clone(),
            ctx,
        );

        let success = self.replace_pane(loading_pane_id, pane_data, false, ctx);
        if success {
            let new_pane_id = self
                .find_pane_id_for_terminal_view(terminal_view_id, ctx)
                .unwrap_or(loading_pane_id);
            self.restore_missing_child_agent_panes_for_terminal_pane_if_needed(new_pane_id, ctx);
        }

        success
    }

    fn load_data_into_restored_ambient_cloud_mode_view(
        terminal_view: ViewHandle<TerminalView>,
        cloud_conversation: CloudConversationData,
        task_id: AmbientAgentTaskId,
        ctx: &mut ViewContext<Self>,
    ) {
        // URL-loaded conversation transcripts (e.g. Warp-on-Web deep links)
        // restore from conversation data before the ambient task cache is
        // guaranteed to contain this task. Native continuation usually reaches
        // this path after task-backed navigation, but the restored cloud-mode
        // pane still needs task ownership/harness data to resolve the correct
        // inline follow-up or tombstone CTA. Request the task and let
        // TerminalView's TasksUpdated subscription re-resolve once it arrives.
        AgentConversationsModel::handle(ctx).update(ctx, |model, ctx| {
            model.get_or_async_fetch_task_data(&task_id, ctx);
        });
        let mut conversation_id = None;
        terminal_view.update(ctx, |view, ctx| {
            // The cloud-mode terminal model starts with
            // `is_executing_oz_environment_startup_commands = true`. Clear it
            // before restoring so that `maybe_insert_setup_command_blocks`
            // doesn't wrap restored command blocks in a "Running setup
            // commands..." group.
            view.model
                .lock()
                .block_list_mut()
                .set_is_executing_oz_environment_startup_commands(false);

            match cloud_conversation {
                CloudConversationData::Oz(mut conversation) => {
                    let id = conversation.id();
                    conversation.set_is_viewing_shared_session(true);
                    view.restore_conversation_after_view_creation(
                        RestoredAIConversation::new(*conversation),
                        true,
                        RestoreConversationEntryBehavior::PreserveAgentViewState,
                        ctx,
                    );
                    view.enter_agent_view(None, Some(id), AgentViewEntryOrigin::CloudAgent, ctx);
                    conversation_id = Some(id);
                }
                CloudConversationData::CLIAgent(cli_conversation) => {
                    if !FeatureFlag::AgentHarness.is_enabled() {
                        log::warn!(
                            "AgentHarness flag is disabled; ignoring CLI agent conversation"
                        );
                        return;
                    }
                    let harness = match cli_conversation.metadata.harness {
                        AIAgentHarness::ClaudeCode => Some(Harness::Claude),
                        AIAgentHarness::Gemini => Some(Harness::Gemini),
                        AIAgentHarness::Codex => Some(Harness::Codex),
                        AIAgentHarness::Oz => None,
                        AIAgentHarness::Unknown => Some(Harness::Unknown),
                    };
                    let fallback_title = cli_conversation.metadata.title.clone();
                    view.restore_conversation_and_directory_context(
                        CloudConversationData::CLIAgent(cli_conversation),
                        true,
                        RestoreConversationEntryBehavior::PreserveAgentViewState,
                        |_, _| {},
                        ctx,
                    );
                    if let Some(harness) = harness {
                        if let Some(ambient_agent_view_model) =
                            view.ambient_agent_view_model().cloned()
                        {
                            ambient_agent_view_model.update(ctx, |model, ctx| {
                                model.set_harness(harness, ctx);
                            });
                        }
                    }
                    if let Some(vehicle_conversation_id) =
                        view.enter_agent_view_for_restored_cli_agent(fallback_title, ctx)
                    {
                        view.model
                            .lock()
                            .block_list_mut()
                            .attach_non_startup_blocks_to_conversation(vehicle_conversation_id);
                    }
                }
            }

            if let Some(ambient_agent_view_model) = view.ambient_agent_view_model().cloned() {
                ambient_agent_view_model.update(ctx, |model, ctx| {
                    model.set_conversation_id(conversation_id);
                    model.enter_viewing_existing_session(task_id, ctx);
                });
            }
            let status = if view.owned_ambient_agent_task_id(ctx).is_some() {
                shared_session::SharedSessionStatus::NotShared
            } else {
                shared_session::SharedSessionStatus::FinishedViewer
            };
            view.model.lock().set_shared_session_status(status);
            view.insert_conversation_ended_tombstone_with_resolved_cta(ctx);
        });

        ActiveAgentViewsModel::handle(ctx).update(ctx, |active_views, ctx| {
            active_views.register_ambient_session(terminal_view.id(), task_id, ctx);
        });
    }

    /// Clear all panes that were hidden due to being closed (for undo functionality)
    /// This is typically called when starting pane rearrangement operations
    fn clear_hidden_closed_panes(&mut self, ctx: &mut ViewContext<Self>) {
        let closed_pane_ids = self.panes.get_closed_pane_ids();
        for pane_id in closed_pane_ids {
            self.cleanup_closed_pane(pane_id, ctx);
        }
    }

    /// Clean up a close-hidden pane completely (used when grace period expires)
    /// Returns true if the pane was successfully cleaned up, false if it was already cleaned up
    pub fn cleanup_closed_pane(&mut self, pane_id: PaneId, ctx: &mut ViewContext<Self>) -> bool {
        self.panes.remove_hidden_pane(pane_id);

        let Some(pane_data) = self.pane_contents.get(&pane_id) else {
            return false;
        };

        let pane = pane_data.as_pane();
        pane.detach(self, DetachType::Closed, ctx);

        if !self.panes.remove(pane_id) {
            log::warn!("Attempted to cleanup pane {pane_id} but it was not found in the tree");
        }
        self.pane_contents.remove(&pane_id);
        // Drop any transitive-share tracking entry for this pane so the
        // map doesn't accumulate stale ids.
        self.forget_transitively_shared_pane(pane_id);

        ctx.notify();
        ctx.emit(Event::TerminalViewStateChanged);
        ctx.emit(Event::AppStateChanged);

        true
    }

    /// Restore a pane that was closed by showing it, attaching it, and focusing it.
    /// Returns true if the pane was successfully restored, false otherwise.
    pub fn restore_closed_pane(&mut self, pane_id: PaneId, ctx: &mut ViewContext<Self>) -> bool {
        if self.unhide_closed_pane(pane_id, ctx) {
            if let Some(pane_content) = self
                .pane_contents
                .get(&pane_id)
                .map(|content| content.as_ref())
            {
                if !self.try_attach_pane(pane_content, ctx) {
                    self.cleanup_closed_pane(pane_id, ctx);
                    return false;
                }
                self.restore_missing_child_agent_panes_for_terminal_pane_if_needed(pane_id, ctx);

                self.focus_pane_and_record_in_history(pane_id, ctx);

                ctx.emit(Event::TerminalViewStateChanged);
                ctx.emit(Event::AppStateChanged);
                return true;
            }
        }
        false
    }

    /// If the given pane id exists in this pane group, performs a root split in the given direction
    /// to move it to a new location.
    pub fn move_pane_with_root_split(
        &mut self,
        id: PaneId,
        direction: Direction,
        ctx: &mut ViewContext<Self>,
    ) {
        self.panes.clear_hidden_panes_from_move();
        // Also clear hidden closed panes since rearranging invalidates undo functionality
        self.clear_hidden_closed_panes(ctx);

        if !self.panes.remove(id) {
            log::error!("Pane not found when attempting to move");
            return;
        }

        self.panes.split_root(id, direction);
        self.handle_pane_count_change(ctx);
        ctx.notify();
        ctx.emit(Event::TerminalViewStateChanged);
        ctx.emit(Event::AppStateChanged);
    }

    pub fn move_pane(
        &mut self,
        id: PaneId,
        target_pane_id: PaneId,
        direction: Direction,
        ctx: &mut ViewContext<Self>,
    ) {
        // Before we do a move, clear any hidden panes
        self.panes.clear_hidden_panes_from_move();
        // Also clear hidden closed panes since rearranging invalidates undo functionality
        self.clear_hidden_closed_panes(ctx);

        self.panes.move_pane(id, target_pane_id, direction);

        self.handle_pane_count_change(ctx);
        ctx.notify();
        ctx.emit(Event::TerminalViewStateChanged);
        ctx.emit(Event::AppStateChanged);
    }

    /// Returns the ID of the pane immediately previous to the given view
    ///
    /// Will retrieve from the history of selected panes when available
    fn prev_pane_id(&self, current_pane: PaneId) -> Option<PaneId> {
        let pane_ids = self.panes.pane_ids();

        let candidate = pane_ids
            .iter()
            .position(|pane_id| *pane_id == current_pane)
            .and_then(|pane_idx| {
                let history_len = self.pane_history.len();
                if history_len > 2 {
                    // We have enough history, use the previous value.
                    let prev_idx = history_len - 2;
                    self.pane_history.get(prev_idx).copied()
                } else if pane_idx == 0 {
                    // We have limited history and are focused on the first pane, focus the session to the right/down.
                    pane_ids.get(1).copied()
                } else {
                    // We have limited history and are focused on a different pane, focus the session to the left/up.
                    pane_ids.get(pane_idx - 1).copied()
                }
            });

        if let Some(id) = candidate {
            if self.has_pane_id(id) && !self.is_pane_hidden_for_close(id) {
                return Some(id);
            }
        }

        // Fall back to the most recently focused pane that still exists and is visible.
        self.pane_history
            .iter()
            .rfind(|&&id| {
                id != current_pane && self.has_pane_id(id) && !self.is_pane_hidden_for_close(id)
            })
            .copied()
    }

    /// Returns of the ID of the previous pane, like iTerm does
    /// Specifically used in the navigate_prev_pane function
    fn prev_pane_id_navigation(&self, current_pane: PaneId) -> Option<PaneId> {
        let pane_ids = self.panes.visible_pane_ids();
        if pane_ids.is_empty() {
            return None;
        }

        match pane_ids.iter().position(|pane_id| *pane_id == current_pane) {
            Some(0) => pane_ids.last().copied(),
            Some(idx) => pane_ids.get(idx - 1).copied(),
            None => None,
        }
    }

    /// Choose a new active session pane, to handle the current one closing.
    ///
    /// This returns the most-recently-focused terminal pane in the pane navigation history. If
    /// there isn't one (for example, because the tab was created from a launch configuration and
    /// some panes haven't been focused yet), it will instead search for the closest terminal pane
    /// to the previous active session, first to the left/up and then to the right/down.
    fn choose_active_session(&self, closing_session_pane: PaneId) -> Option<TerminalPaneId> {
        if let Some(terminal_pane) = self
            .pane_history
            .iter()
            .rev()
            // Don't re-activate the pane being closed.
            .filter(|pane_id| **pane_id != closing_session_pane)
            .filter(|pane_id| {
                self.has_pane_id(**pane_id) && !self.is_pane_hidden_for_close(**pane_id)
            })
            .find_map(PaneId::as_terminal_pane_id)
        {
            return Some(terminal_pane);
        }

        // In most cases, the next active session will be in `pane_history`. However, if the pane
        // group was created from a launch configuration or restored session, it might have
        // terminal panes that haven't been focused yet and therefore aren't in `pane_history`. In
        // that case, we fall back to searching by position.
        let pane_ids = self.panes.visible_pane_ids();
        let pane_idx = pane_ids
            .iter()
            .position(|pane_id| *pane_id == closing_session_pane)?;

        // If there's not enough history, prefer activating a session to the left/up.
        if let Some(terminal_pane) = pane_ids
            .iter()
            .take(pane_idx)
            .rev()
            .find_map(PaneId::as_terminal_pane_id)
        {
            return Some(terminal_pane);
        }

        // Finally, fall back to a a session that's to the right/down.
        pane_ids
            .iter()
            .skip(pane_idx + 1)
            .find_map(PaneId::as_terminal_pane_id)
    }

    /// Returns the ID of the pane immediately after the given view
    ///
    /// Will wrap around to the first pane if the given view is the last pane
    fn next_pane_id(&self, current_pane: PaneId) -> Option<PaneId> {
        let pane_ids = self.panes.visible_pane_ids();
        if pane_ids.is_empty() {
            return None;
        }

        let last_position = pane_ids.len() - 1;

        match pane_ids.iter().position(|pane_id| *pane_id == current_pane) {
            Some(idx) if idx == last_position => pane_ids.first().copied(),
            Some(idx) => pane_ids.get(idx + 1).copied(),
            None => None,
        }
    }

    fn navigate_prev_pane(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(id) = self.prev_pane_id_navigation(self.focused_pane_id(ctx)) {
            if self.focus_pane(id, true, ctx) {
                ctx.emit(Event::AppStateChanged);
            }
        }
    }

    fn navigate_next_pane(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(id) = self.next_pane_id(self.focused_pane_id(ctx)) {
            if self.focus_pane(id, true, ctx) {
                ctx.emit(Event::AppStateChanged);
            }
        }
    }

    fn navigate_pane_by_direction(&mut self, direction: Direction, ctx: &mut ViewContext<Self>) {
        let ids = self
            .panes
            .panes_by_direction(self.focused_pane_id(ctx), direction, ctx);
        if !ids.is_empty() {
            // If there is more than one candidate pane in the direction of travel, pick the one that was most recently
            // focused.  This makes a better experience when navigating back and forth between two panes.
            let recent_id = self
                .pane_history
                .iter()
                .rfind(|id| ids.contains(*id))
                .unwrap_or_else(|| &ids[0]);
            self.focus_pane_and_record_in_history(*recent_id, ctx);
            ctx.emit(Event::AppStateChanged);
        }
    }

    /// Whether or not the focused pane is maximized.
    pub fn is_focused_pane_maximized(&self, ctx: &AppContext) -> bool {
        self.focus_state.as_ref(ctx).is_focused_pane_maximized()
    }

    pub fn focused_shell_indicator_type(&self, ctx: &AppContext) -> Option<ShellIndicatorType> {
        self.pane_contents
            .get(&self.focused_pane_id(ctx))
            .and_then(|pane| pane.as_any().downcast_ref::<TerminalPane>())
            .and_then(|terminal_pane| {
                terminal_pane
                    .terminal_view(ctx)
                    .as_ref(ctx)
                    .shell_indicator_type()
            })
    }

    /// Toggles whether or not the focused pane is maximized.
    fn toggle_maximize_pane(&mut self, ctx: &mut ViewContext<Self>) {
        if self.pane_count() > 1 {
            self.focus_state.update(ctx, |focus_state, ctx| {
                focus_state.toggle_focused_pane_maximized(ctx);
            });
            ctx.notify();
            ctx.emit(Event::MaximizePaneToggled);
        }
    }

    fn focus_pane_on_mouse_event(
        &mut self,
        id: PaneId,
        reason: ActivationReason,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(content) = self.pane_contents.get(&id) else {
            return;
        };

        if matches!(reason, ActivationReason::Hover) {
            if !ctx.windows().app_is_active() {
                // Don't focus panes on hover if the app is not active.
                return;
            }

            if self.is_being_resized() || self.any_pane_being_dragged(ctx) {
                // Don't focus panes on hover if the app is being resized or a pane is being dragged.
                return;
            }

            #[cfg(target_os = "macos")]
            {
                // if the app is active, but the window is not active, activate the target window.
                let current_window_id: WindowId = ctx.window_id();
                let active_window_id = ctx.windows().state().active_window;
                if active_window_id != Some(current_window_id) {
                    ctx.windows()
                        .show_window_and_focus_app_without_ordering_front(current_window_id);
                }
            }
        }

        if let Some(session) = content.as_any().downcast_ref::<TerminalPane>() {
            // Only activate the session if link tooltip is disabled or there is no highlighted link.
            if *GeneralSettings::as_ref(ctx).link_tooltip
                && session
                    .terminal_view(ctx)
                    .as_ref(ctx)
                    .has_highlighted_link()
            {
                return;
            }
        }

        self.focus_pane_by_id(id, ctx);
    }

    pub fn focus_pane_by_id(&mut self, id: PaneId, ctx: &mut ViewContext<Self>) {
        // If user clicks on a pane quickly after dragging the border, a race condition
        // could happen where the mouse down movement is considered as part of dragging.
        // We clear the dragging state here to avoid such conditions.
        self.dragged_border = None;
        if self.focus_pane_and_record_in_history(id, ctx) {
            ctx.emit(Event::AppStateChanged);
            ctx.emit(Event::PaneFocused);
        }
    }

    /// Focused the specified terminal view, if it belongs to this pane group.
    pub fn focus_terminal_view(&mut self, terminal_view_id: EntityId, ctx: &mut ViewContext<Self>) {
        let pane_id = self
            .pane_contents
            .keys()
            .find(|id| {
                if let Some(terminal_view) = self.terminal_view_from_pane_id(**id, ctx) {
                    terminal_view_id == terminal_view.id()
                } else {
                    false
                }
            })
            .cloned();

        if let Some(pane_id) = pane_id {
            self.focus_pane_by_id(pane_id, ctx);
        }
    }

    /// Show a notification error for the pane that we tried to send a notification for.
    pub fn show_notification_error(
        &mut self,
        error: NotificationSendError,
        pane_id: PaneId,
        ctx: &mut ViewContext<Self>,
    ) {
        if let Some(view) = self.terminal_view_from_pane_id(pane_id, ctx) {
            view.update(ctx, |view, ctx| {
                view.show_notification_error(error, ctx);
            })
        }
    }

    pub fn is_being_resized(&self) -> bool {
        self.dragged_border.is_some()
    }

    // The drag event pertains to the divider being dragged between panes.
    // If there's no active dragging state on the pane, the event is propagated up
    // to workspace in case there the sidebar divider is being dragged.
    fn maybe_resize_pane(&mut self, position: Vector2F, ctx: &mut ViewContext<Self>) {
        if self.dragged_border.is_some() {
            self.resize_pane(position, ctx);
        }
    }

    fn resize_pane(&mut self, position: Vector2F, ctx: &mut ViewContext<Self>) {
        if let Some(border) = &mut self.dragged_border {
            let delta = match border.direction {
                SplitDirection::Horizontal => position.x() - border.previous_mouse_location.x(),
                SplitDirection::Vertical => position.y() - border.previous_mouse_location.y(),
            };

            self.panes.adjust_pane_size(border.border_id, delta, ctx);

            border.previous_mouse_location = position;
            ctx.notify();
        }
    }

    pub fn start_resizing(&mut self, info: DraggedBorder, ctx: &mut ViewContext<Self>) {
        // Clear hidden closed panes since resizing invalidates undo functionality
        self.clear_hidden_closed_panes(ctx);
        self.dragged_border = Some(info);
    }

    pub fn reset_pane_sizes(&mut self, border_id: EntityId, ctx: &mut ViewContext<Self>) {
        self.dragged_border = None;
        if self.panes.reset_pane_sizes(border_id) {
            self.clear_hidden_closed_panes(ctx);
            ctx.notify();
            ctx.emit(Event::AppStateChanged);
        }
    }

    pub fn end_resizing(&mut self, ctx: &mut ViewContext<Self>) {
        self.dragged_border = None;
        ctx.emit(Event::AppStateChanged);
    }

    pub fn resize_left(&mut self, ctx: &mut ViewContext<Self>) {
        self.panes.adjust_pane_size_by_id(
            self.focused_pane_id(ctx),
            SplitDirection::Horizontal,
            -KEYBOARD_RESIZE_DELTA,
            ctx,
        );
        ctx.notify();
        ctx.emit(Event::AppStateChanged);
    }

    pub fn resize_right(&mut self, ctx: &mut ViewContext<Self>) {
        self.panes.adjust_pane_size_by_id(
            self.focused_pane_id(ctx),
            SplitDirection::Horizontal,
            KEYBOARD_RESIZE_DELTA,
            ctx,
        );
        ctx.notify();
        ctx.emit(Event::AppStateChanged);
    }

    pub fn resize_up(&mut self, ctx: &mut ViewContext<Self>) {
        self.panes.adjust_pane_size_by_id(
            self.focused_pane_id(ctx),
            SplitDirection::Vertical,
            -KEYBOARD_RESIZE_DELTA,
            ctx,
        );
        ctx.notify();
        ctx.emit(Event::AppStateChanged);
    }

    pub fn resize_down(&mut self, ctx: &mut ViewContext<Self>) {
        self.panes.adjust_pane_size_by_id(
            self.focused_pane_id(ctx),
            SplitDirection::Vertical,
            KEYBOARD_RESIZE_DELTA,
            ctx,
        );
        ctx.notify();
        ctx.emit(Event::AppStateChanged);
    }

    fn handle_user_default_shell_changed_banner_event(
        &mut self,
        event: &BannerEvent<PaneGroupAction>,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            BannerEvent::Dismiss(DismissalType::Temporary) => {
                self.user_default_shell_unsupported_banner_model_handle
                    .update(ctx, |model, model_ctx| {
                        *model = BannerState::Dismissed;
                        model_ctx.notify();
                    });
            }
            BannerEvent::Dismiss(DismissalType::Permanent) => {
                self.user_default_shell_unsupported_banner_model_handle
                    .update(ctx, |model, model_ctx| {
                        *model = BannerState::Dismissed;
                        model_ctx.notify();
                    });

                GeneralSettings::handle(ctx).update(ctx, |general_settings, ctx| {
                    report_if_error!(general_settings
                        .user_default_shell_unsupported_banner_state
                        .set_value(BannerState::Dismissed, ctx));
                });
            }
            BannerEvent::Action(_) => {
                #[cfg(debug_assertions)]
                unimplemented!("User default shell change banner doesn't support actions");
            }
        }
        ctx.notify();
    }

    /// Sync changes in the visible pane count to the [`focus_state::PaneGroupFocusState`] model.
    fn handle_pane_count_change(&mut self, ctx: &mut ViewContext<Self>) {
        let in_split_pane = self.panes.visible_pane_count() > 1;
        self.focus_state.update(ctx, |focus_state, ctx| {
            focus_state.set_in_split_pane(in_split_pane, ctx);
        });
    }

    // Instantiate the terminal view with the given parameters. Note that the active
    // session path here needs to be a valid os path otherwise the app will crash.
    // Environment variables are merged into the default environment for the terminal process,
    // and do not completely replace it.
    #[allow(clippy::too_many_arguments, unused_variables)]
    fn create_session(
        startup_directory: Option<PathBuf>,
        mut env_vars: HashMap<OsString, OsString>,
        terminal_session_uuid: &[u8],
        is_shared_session: IsSharedSessionCreator,
        resources: TerminalViewResources,
        restored_blocks: Option<&Vec<SerializedBlockListItem>>,
        conversation_restoration: Option<ConversationRestorationInNewPaneType>,
        user_default_shell_unsupported_banner_model_handle: ModelHandle<BannerState>,
        initial_size: Vector2F,
        model_event_sender: Option<SyncSender<ModelEvent>>,
        chosen_shell: Option<AvailableShell>,
        initial_input_config: Option<InputConfig>,
        ctx: &mut ViewContext<Self>,
    ) -> (
        ViewHandle<TerminalView>,
        ModelHandle<Box<dyn TerminalManager>>,
    ) {
        add_session_focus_env_vars(&mut env_vars, terminal_session_uuid);

        cfg_if::cfg_if! {
            if #[cfg(feature = "remote_tty")] {
                let terminal_manager: ModelHandle<Box<dyn TerminalManager>> = crate::terminal::remote_tty::TerminalManager::create_model(
                    resources,
                    initial_size,
                    model_event_sender,
                    ctx.window_id(),
                    initial_input_config,
                    ctx,
                );
            } else if #[cfg(feature = "local_tty")] {
                let terminal_manager: ModelHandle<Box<dyn TerminalManager>> = crate::terminal::local_tty::TerminalManager::create_model(
                    startup_directory,
                    env_vars,
                    is_shared_session,
                    resources,
                    restored_blocks,
                    conversation_restoration,
                    user_default_shell_unsupported_banner_model_handle,
                    initial_size,
                    model_event_sender,
                    ctx.window_id(),
                    chosen_shell,
                    initial_input_config,
                    ctx,
                );
            } else {
                use crate::terminal::{ShellLaunchState, shell::{ShellName, ShellType}};

                let terminal_manager: ModelHandle<Box<dyn TerminalManager>> = crate::terminal::MockTerminalManager::create_model(
                    ShellLaunchState::ShellSpawned {
                        available_shell: chosen_shell,
                        display_name: ShellName::blank(),
                        shell_type: ShellType::Zsh
                    },
                    resources,
                    None,
                    conversation_restoration,
                    initial_size,
                    ctx.window_id(),
                    ctx,
                );
            }
        }

        let terminal_view = terminal_manager.as_ref(ctx).view();
        (terminal_view, terminal_manager)
    }

    #[allow(clippy::too_many_arguments)]
    fn create_shared_session_viewer(
        session_id: SessionId,
        resources: TerminalViewResources,
        initial_size: Vector2F,
        enable_orchestration_polling: bool,
        ctx: &mut ViewContext<Self>,
    ) -> (
        ViewHandle<TerminalView>,
        ModelHandle<Box<dyn TerminalManager>>,
    ) {
        let window_id = ctx.window_id();
        let terminal_manager = ctx.add_model(|ctx| {
            let terminal_manager: Box<dyn TerminalManager> =
                Box::new(shared_session::viewer::TerminalManager::new(
                    session_id,
                    resources,
                    initial_size,
                    window_id,
                    enable_orchestration_polling,
                    ctx,
                ));
            terminal_manager
        });

        let terminal_view = terminal_manager.as_ref(ctx).view();
        (terminal_view, terminal_manager)
    }

    fn create_conversation_viewer(
        conversation: AIConversation,
        ambient_agent_task_id: Option<AmbientAgentTaskId>,
        resources: TerminalViewResources,
        initial_size: Vector2F,
        ctx: &mut ViewContext<Self>,
    ) -> (
        ViewHandle<TerminalView>,
        ModelHandle<Box<dyn TerminalManager>>,
    ) {
        let restored_blocks = conversation.to_serialized_blocklist_items();
        let terminal_manager = MockTerminalManager::create_model(
            ShellLaunchState::ShellSpawned {
                available_shell: None,
                display_name: ShellName::blank(),
                shell_type: ShellType::Zsh,
            },
            resources,
            Some(&restored_blocks),
            Some(ConversationRestorationInNewPaneType::Historical {
                conversation,
                should_use_live_appearance: true,
                ambient_agent_task_id,
            }),
            initial_size,
            ctx.window_id(),
            ctx,
        );
        // Set the conversation viewer status based on whether this is an ambient agent conversation
        let viewer_status = ambient_agent_task_id
            .map(ConversationTranscriptViewerStatus::ViewingAmbientConversation)
            .unwrap_or(ConversationTranscriptViewerStatus::ViewingLocalConversation);

        terminal_manager.update(ctx, |terminal_manager, _ctx| {
            terminal_manager
                .model()
                .lock()
                .set_conversation_transcript_viewer_status(Some(viewer_status.clone()));
        });

        let terminal_view = terminal_manager.as_ref(ctx).view();
        // Insert the conversation ended tombstone (includes Open in Warp button on WASM)
        terminal_view.update(ctx, |view, ctx| {
            view.insert_conversation_ended_tombstone_with_resolved_cta(ctx);
        });

        BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, _ctx| {
            history_model.mark_terminal_view_as_conversation_transcript_viewer(terminal_view.id());
        });

        // Register the transcript viewer as an ambient session so it appears in the Active section
        // of the conversation list.
        if let Some(task_id) = ambient_agent_task_id {
            ActiveAgentViewsModel::handle(ctx).update(ctx, |active_views, ctx| {
                active_views.register_ambient_session(terminal_view.id(), task_id, ctx);
            });
        }

        (terminal_view, terminal_manager)
    }

    /// Creates a loading terminal view with MockTerminalManager in loading state.
    /// This is used by both `new_for_conversation_transcript_viewer_loading` and `create_loading_terminal_pane`.
    fn create_loading_terminal_manager_and_view(
        resources: TerminalViewResources,
        view_bounds_size: Vector2F,
        window_id: WindowId,
        ctx: &mut ViewContext<Self>,
    ) -> (
        ViewHandle<TerminalView>,
        ModelHandle<Box<dyn TerminalManager>>,
    ) {
        let terminal_manager = MockTerminalManager::create_model(
            ShellLaunchState::ShellSpawned {
                available_shell: None,
                display_name: ShellName::blank(),
                shell_type: ShellType::Zsh,
            },
            resources,
            None, // No restored blocks
            None, // No conversation restoration
            view_bounds_size,
            window_id,
            ctx,
        );

        // Set the conversation transcript viewer status to Loading
        terminal_manager.update(ctx, |terminal_manager, _ctx| {
            terminal_manager
                .model()
                .lock()
                .set_conversation_transcript_viewer_status(Some(
                    ConversationTranscriptViewerStatus::Loading,
                ));
        });

        let terminal_view = terminal_manager.as_ref(ctx).view();
        (terminal_view, terminal_manager)
    }

    /// Whether to use the user-specified startup directory when starting
    /// a new session. On Windows, we ignore this custom directory setting in
    /// WSL sessions. On all other systems, we honor the custom directory.
    #[cfg(feature = "local_tty")]
    fn should_ignore_custom_startup_directory(
        &self,
        chosen_shell: &Option<AvailableShell>,
        ctx: &ViewContext<Self>,
    ) -> bool {
        let wsl_distro = chosen_shell
            .to_owned()
            .unwrap_or_else(move || {
                AvailableShells::handle(ctx)
                    .read(ctx, |shells, ctx| shells.get_user_preferred_shell(ctx))
            })
            .wsl_distro();
        wsl_distro.is_some()
    }

    #[cfg(not(feature = "local_tty"))]
    const fn should_ignore_custom_startup_directory(
        &self,
        _chosen_shell: &Option<AvailableShell>,
        _ctx: &ViewContext<Self>,
    ) -> bool {
        false
    }

    /// Creates a loading terminal pane that shows a spinner while conversation data is being fetched.
    /// Returns the pane ID so it can be replaced later with the real terminal pane.
    pub fn add_loading_conversation_pane(
        &mut self,
        direction: Direction,
        base_pane_id: Option<PaneId>,
        ctx: &mut ViewContext<Self>,
    ) -> PaneId {
        let uuid = Uuid::new_v4();
        let resources = TerminalViewResources {
            tips_completed: self.tips_completed.clone(),
            server_api: self.server_api.clone(),
            model_event_sender: self.model_event_sender.clone(),
        };

        let view_bounds = Self::estimated_view_bounds(ctx);
        let (terminal_view, terminal_manager) = Self::create_loading_terminal_manager_and_view(
            resources,
            view_bounds.size(),
            ctx.window_id(),
            ctx,
        );

        let pane_data = TerminalPane::new(
            uuid.as_bytes().to_vec(),
            terminal_manager,
            terminal_view,
            self.model_event_sender.clone(),
            ctx,
        );
        let pane_id: PaneId = pane_data.terminal_pane_id().into();

        let _ = self.add_pane(direction, base_pane_id, Box::new(pane_data), true, ctx);

        pane_id
    }

    /// Replaces a loading pane with a real terminal pane that has a conversation restored.
    /// Returns true if replacement was successful.
    pub fn replace_loading_pane_with_terminal(
        &mut self,
        loading_pane_id: PaneId,
        cloud_conversation: CloudConversationData,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        if FeatureFlag::HandoffCloudCloud.is_enabled() {
            if let Some(task_id) = Self::ambient_agent_task_id(&cloud_conversation) {
                return self.replace_loading_pane_with_restored_ambient_cloud_mode_pane(
                    loading_pane_id,
                    cloud_conversation,
                    task_id,
                    ctx,
                );
            }
        }
        let restoration = match cloud_conversation {
            CloudConversationData::Oz(conversation) => {
                ConversationRestorationInNewPaneType::Historical {
                    conversation: *conversation,
                    should_use_live_appearance: true,
                    ambient_agent_task_id: None,
                }
            }
            CloudConversationData::CLIAgent(cli_conversation) => {
                if !FeatureFlag::AgentHarness.is_enabled() {
                    log::warn!("AgentHarness flag is disabled; ignoring CLI agent conversation");
                    return false;
                }
                ConversationRestorationInNewPaneType::HistoricalCLIAgent {
                    conversation: *cli_conversation,
                    should_use_live_appearance: true,
                }
            }
        };

        // Get the initial working directory from the restored conversation.
        let startup_directory = restoration
            .initial_working_directory()
            .map(PathBuf::from)
            .filter(|path| path.is_dir());

        let uuid = Uuid::new_v4();
        let resources = TerminalViewResources {
            tips_completed: self.tips_completed.clone(),
            server_api: self.server_api.clone(),
            model_event_sender: self.model_event_sender.clone(),
        };

        let view_bounds = Self::estimated_view_bounds(ctx);
        let (view, terminal_manager) = PaneGroup::create_session(
            startup_directory,
            HashMap::new(),
            uuid.as_bytes(),
            IsSharedSessionCreator::No,
            resources,
            None,
            Some(restoration),
            self.user_default_shell_unsupported_banner_model_handle
                .clone(),
            view_bounds.size(),
            self.model_event_sender.clone(),
            None, // chosen_shell
            None, // initial_input_config
            ctx,
        );

        let pane_data = TerminalPane::new(
            uuid.as_bytes().to_vec(),
            terminal_manager,
            view,
            self.model_event_sender.clone(),
            ctx,
        );

        // Use replace_pane to swap loading pane with new terminal pane
        self.replace_pane(loading_pane_id, pane_data, false, ctx)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn add_session(
        &mut self,
        direction: Direction,
        base_pane_id_for_split: Option<PaneId>,
        base_pane_id_for_context: Option<TerminalPaneId>,
        chosen_shell: Option<AvailableShell>,
        conversation_restoration: Option<ConversationRestorationInNewPaneType>,
        ctx: &mut ViewContext<Self>,
    ) -> TerminalPaneId {
        self.add_session_with_default_session_mode_behavior(
            direction,
            base_pane_id_for_split,
            base_pane_id_for_context,
            chosen_shell,
            conversation_restoration,
            DefaultSessionModeBehavior::Apply,
            ctx,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn add_session_with_default_session_mode_behavior(
        &mut self,
        direction: Direction,
        base_pane_id_for_split: Option<PaneId>,
        base_pane_id_for_context: Option<TerminalPaneId>,
        chosen_shell: Option<AvailableShell>,
        conversation_restoration: Option<ConversationRestorationInNewPaneType>,
        default_session_mode_behavior: DefaultSessionModeBehavior,
        ctx: &mut ViewContext<Self>,
    ) -> TerminalPaneId {
        // If restoring a conversation, use its initial working directory if it exists
        let startup_directory_from_conversation = conversation_restoration
            .as_ref()
            .and_then(|restoration| restoration.initial_working_directory())
            .map(PathBuf::from)
            .filter(|path| path.is_dir());

        let startup_directory = startup_directory_from_conversation.or_else(|| {
            let ignore_custom_startup_directory =
                self.should_ignore_custom_startup_directory(&chosen_shell, ctx);

            let initial_directory_from_current_session =
                self.startup_path_for_new_session(base_pane_id_for_context, ctx);

            SessionSettings::handle(ctx).read(ctx, |settings, _ctx| {
                settings
                    .working_directory_config
                    .initial_directory_for_new_session(
                        NewSessionSource::SplitPane,
                        initial_directory_from_current_session,
                        ignore_custom_startup_directory,
                    )
            })
        });
        self.add_session_in_directory(
            direction,
            base_pane_id_for_split,
            chosen_shell,
            startup_directory,
            conversation_restoration,
            default_session_mode_behavior,
            ctx,
        )
    }

    /// Creates a new terminal session and wraps it in a `TerminalPane`.
    /// This is the shared session-creation boilerplate used by both
    /// `add_session_in_directory` and `insert_terminal_pane_hidden_for_child_agent`.
    #[allow(clippy::too_many_arguments)]
    fn create_terminal_pane_data(
        &self,
        startup_directory: Option<PathBuf>,
        env_vars: HashMap<OsString, OsString>,
        is_shared_session_creator: IsSharedSessionCreator,
        chosen_shell: Option<AvailableShell>,
        conversation_restoration: Option<ConversationRestorationInNewPaneType>,
        ctx: &mut ViewContext<Self>,
    ) -> (TerminalPane, ViewHandle<TerminalView>) {
        let uuid = Uuid::new_v4();
        let resources = TerminalViewResources {
            tips_completed: self.tips_completed.clone(),
            server_api: self.server_api.clone(),
            model_event_sender: self.model_event_sender.clone(),
        };

        let view_bounds = Self::estimated_view_bounds(ctx);
        let (view, terminal_manager) = PaneGroup::create_session(
            startup_directory,
            env_vars,
            uuid.as_bytes(),
            is_shared_session_creator,
            resources,
            None,
            conversation_restoration,
            self.user_default_shell_unsupported_banner_model_handle
                .clone(),
            view_bounds.size(),
            self.model_event_sender.clone(),
            chosen_shell,
            None,
            ctx,
        );

        let pane_data = TerminalPane::new(
            uuid.as_bytes().to_vec(),
            terminal_manager,
            view.clone(),
            self.model_event_sender.clone(),
            ctx,
        );

        (pane_data, view)
    }

    #[allow(clippy::too_many_arguments)]
    fn add_session_in_directory(
        &mut self,
        direction: Direction,
        base_pane_id: Option<PaneId>,
        chosen_shell: Option<AvailableShell>,
        startup_directory: Option<PathBuf>,
        conversation_restoration: Option<ConversationRestorationInNewPaneType>,
        default_session_mode_behavior: DefaultSessionModeBehavior,
        ctx: &mut ViewContext<Self>,
    ) -> TerminalPaneId {
        let should_immediately_enter_agent_view = matches!(
            default_session_mode_behavior,
            DefaultSessionModeBehavior::Apply
        ) && conversation_restoration.is_none()
            && AISettings::as_ref(ctx).default_session_mode(ctx) == DefaultSessionMode::Agent;

        let (pane_data, view) = self.create_terminal_pane_data(
            startup_directory,
            HashMap::new(),
            IsSharedSessionCreator::No,
            chosen_shell,
            conversation_restoration,
            ctx,
        );
        let new_pane_id = pane_data.terminal_pane_id();

        let _ = self.add_pane(direction, base_pane_id, Box::new(pane_data), true, ctx);

        // Enter agent view if default session mode is Agent and AI is enabled
        if should_immediately_enter_agent_view {
            view.update(ctx, |terminal_view, ctx| {
                terminal_view.enter_agent_view_for_new_conversation(
                    None,
                    AgentViewEntryOrigin::DefaultSessionMode,
                    ctx,
                );
            });
        }

        new_pane_id
    }

    /// Adds a new side-pane to this group, at the root of the pane tree.
    pub fn add_pane_with_direction<C: PaneContent>(
        &mut self,
        direction: Direction,
        pane: C,
        focus_new_pane: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        let _ = self.add_pane(direction, None, Box::new(pane), focus_new_pane, ctx);
    }

    /// Adds a new pane to this group, relative to an existing pane.
    pub fn add_pane_sibling(
        &mut self,
        relative_to: PaneId,
        direction: Direction,
        pane: impl Into<Box<dyn AnyPaneContent>>,
        focus_new_pane: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        let _ = self.add_pane(
            direction,
            Some(relative_to),
            pane.into(),
            focus_new_pane,
            ctx,
        );
    }

    fn init_pane(
        &mut self,
        pane: Box<dyn AnyPaneContent>,
        ctx: &mut ViewContext<Self>,
    ) -> Option<PaneId> {
        let pane_id = pane.as_pane().id();
        self.pane_contents.insert(pane_id, pane);
        // The HashMap entry API would let us insert and then get a mutable reference to the
        // just-added pane. However, this would mean that attach() can't take the pane group
        // as an argument.
        let pane = self
            .pane_contents
            .get(&pane_id)
            .expect("Just inserted pane");

        if !self.try_attach_pane(pane.as_ref(), ctx) {
            // Remove the pane we didn't end up attaching the pane.
            self.pane_contents.remove(&pane_id);
            return None;
        }
        Some(pane_id)
    }

    /// Adds a new pane to the tree with configurable visibility/focus/event behavior.
    fn add_pane_with_options(
        &mut self,
        new_pane: Box<dyn AnyPaneContent>,
        options: AddPaneOptions,
        ctx: &mut ViewContext<Self>,
    ) -> Option<PaneId> {
        let pane_id = new_pane.as_pane().id();
        match options.visibility {
            NewPaneVisibility::Visible => {}
            NewPaneVisibility::HiddenForMove => self.panes.hide_pane_for_move(pane_id),
        }

        let pane_id = self.init_pane(new_pane, ctx)?;
        let split_succeeded = match options.base_pane_id {
            Some(base_pane_id) => self.panes.split(base_pane_id, pane_id, options.direction),
            None => {
                self.panes.split_root(pane_id, options.direction);
                true
            }
        };

        if !split_succeeded {
            log::error!(
                "Failed to split pane tree when adding pane {:?} relative to {:?}",
                pane_id,
                options.base_pane_id
            );
            self.panes.remove_hidden_pane(pane_id);
            self.clean_up_pane(pane_id, ctx);
            self.pane_contents.remove(&pane_id);
            return None;
        }
        self.restore_missing_child_agent_panes_for_terminal_pane_if_needed(pane_id, ctx);

        if options.focus_new_pane {
            self.focus_pane_and_record_in_history(pane_id, ctx);
        }

        self.handle_pane_count_change(ctx);

        ctx.notify();
        if options.emit_app_state_changed {
            ctx.emit(Event::AppStateChanged);
        }
        Some(pane_id)
    }

    /// Adds a new pane to the tree. If `base_pane_id` is `Some`, the new pane is inserted relative
    /// to that pane. Otherwise, it's inserted at the root of the pane tree.
    fn add_pane(
        &mut self,
        direction: Direction,
        base_pane_id: Option<PaneId>,
        new_pane: Box<dyn AnyPaneContent>,
        focus_new_pane: bool,
        ctx: &mut ViewContext<Self>,
    ) -> Option<PaneId> {
        if self.pane_count() == 1 {
            // Only sending telemetry event the first time a user enters split pane in a session.
            send_telemetry_from_ctx!(TelemetryEvent::SplitPane, ctx);
        }

        self.tips_completed.update(ctx, |tips_completed, ctx| {
            mark_feature_used_and_write_to_user_defaults(
                Tip::Action(TipAction::SplitPane),
                tips_completed,
                ctx,
            );
            ctx.notify();
        });
        self.add_pane_with_options(
            new_pane,
            AddPaneOptions {
                direction,
                base_pane_id,
                focus_new_pane,
                visibility: NewPaneVisibility::Visible,
                emit_app_state_changed: true,
            },
            ctx,
        )
    }

    pub fn pane_count(&self) -> usize {
        self.panes.len()
    }

    pub fn has_horizontal_split(&self) -> bool {
        self.panes.has_horizontal_split()
    }

    pub fn try_navigate_next(&mut self, ctx: &mut ViewContext<Self>) -> bool {
        let pane_ids = self.panes.visible_pane_ids();
        if pane_ids.len() <= 1 {
            return false;
        }

        // Only move to the next pane if we're not already at the last index.
        if let Some(idx) = pane_ids
            .iter()
            .position(|pane_id| *pane_id == self.focused_pane_id(ctx))
        {
            if idx < pane_ids.len() - 1 {
                self.navigate_next_pane(ctx);
                return true;
            }
        }

        false
    }

    pub fn try_navigate_prev(&mut self, ctx: &mut ViewContext<Self>) -> bool {
        let pane_ids = self.panes.visible_pane_ids();
        if pane_ids.len() <= 1 {
            return false;
        }

        // Only move to the previous pane if we're not already at the first index.
        if let Some(idx) = pane_ids
            .iter()
            .position(|pane_id| *pane_id == self.focused_pane_id(ctx))
        {
            if idx > 0 {
                self.navigate_prev_pane(ctx);
                return true;
            }
        }

        false
    }

    /// Returns the count of visible panes (excluding hidden panes).
    #[cfg(any(test, feature = "integration_tests"))]
    pub fn visible_pane_count(&self) -> usize {
        self.panes.visible_pane_count()
    }

    /// Returns the path of the directory in which a newly created session should start, if any.
    /// On Windows, this path will be in native Windows format (including the WSL prefix and
    /// distribution, if applicable).
    ///
    /// This returns the active (parent) session's current directory if the active session is local
    /// (not an SSH session) and if the active session is done bootstrapping. Else, it returns the
    /// the current session's startup directory.
    pub fn startup_path_for_new_session(
        &self,
        base_pane_id: Option<TerminalPaneId>,
        ctx: &AppContext,
    ) -> Option<PathBuf> {
        let pane_id = base_pane_id?;
        if let Some(current_session_path) = self.session_path(&pane_id, ctx) {
            return Some(current_session_path);
        }

        self.terminal_view_from_pane_id(pane_id, ctx)
            .and_then(|terminal_handle| {
                terminal_handle.read(ctx, |view, _| {
                    let model = view.model.lock();
                    let session_startup_path = model.session_startup_path();
                    if let (Some(distribution_name), Some(path)) =
                        (view.active_session_wsl_distro(ctx), &session_startup_path)
                    {
                        path.to_str().and_then(|path| {
                            convert_wsl_to_windows_host_path(
                                &TypedPath::unix(path),
                                &distribution_name,
                            )
                            .inspect_err(|err| {
                                log::warn!(
                                    "unable to convert WSL path to Windows host path: {err:?}"
                                );
                            })
                            .ok()
                        })
                    } else {
                        session_startup_path
                    }
                })
            })
    }

    pub fn launch_data_for_session(
        &self,
        pane_id: TerminalPaneId,
        ctx: &AppContext,
    ) -> Option<ShellLaunchData> {
        self.terminal_view_from_pane_id(pane_id, ctx)
            .and_then(|terminal_handle| {
                terminal_handle.read(ctx, |view, ctx| {
                    view.active_block_session_id()
                        .and_then(|id| view.sessions_model().as_ref(ctx).get(id))
                        .and_then(|s| s.launch_data().cloned())
                })
            })
    }

    /// Updates visibility of sessions contained within this pane group based
    /// on window visibility and view focus state.
    fn update_session_visibility(&mut self, ctx: &mut ViewContext<Self>) {
        if !ctx.is_self_or_child_focused() {
            return;
        }

        let active_window_id = ctx.windows().state().active_window;
        if active_window_id == Some(ctx.window_id()) {
            for session in self.panes_of::<TerminalPane>() {
                session.terminal_view(ctx).update(ctx, |view, _ctx| {
                    view.mark_as_visible();
                });
            }
        }
    }

    pub fn focus(&mut self, ctx: &mut ViewContext<Self>) {
        self.update_session_visibility(ctx);

        // We're adding a new pane to a tab that potentially has set the custom tab title.
        // Lets ensure the new pane will honor it, otherwise, we'd want to change the title based
        // on the default title for the pane.
        if let Some(pane) = self.focused_pane_content(ctx) {
            pane.focus(ctx);
        }

        #[cfg(target_family = "wasm")]
        {
            if ContextFlag::DynamicBrowserUrl.is_enabled() {
                self.update_browser_url(ctx);
            }
        }
    }

    fn handle_pane_link_updated(&self, pane_id: PaneId, url: Option<Url>, ctx: &AppContext) {
        log::debug!("Url for pane should be updated pane_id: {pane_id:?}, url: {url:?}");
        #[cfg(target_family = "wasm")]
        if pane_id == self.focused_pane_id(ctx) {
            update_browser_url(url, false);
        }

        let _ = ctx;
    }

    #[cfg(target_family = "wasm")]
    fn update_browser_url(&self, ctx: &mut ViewContext<Self>) {
        // We need to wait for the app to be loaded before we attempt to get the
        // shareable links. This is because the links come from CloudModel objects

        let initial_load_complete = UpdateManager::as_ref(ctx).initial_load_complete();
        ctx.spawn(initial_load_complete, move |me, _, ctx| {
            if let Some(pane) = me.focused_pane_content(ctx) {
                match pane.shareable_link(ctx) {
                    Ok(crate::pane_group::pane::ShareableLink::Base) => {
                        update_browser_url(None, false)
                    }
                    Ok(crate::pane_group::pane::ShareableLink::Pane { url }) => {
                        update_browser_url(Some(url), false)
                    }
                    Err(crate::pane_group::pane::ShareableLinkError::Expected) => {}
                    Err(crate::pane_group::pane::ShareableLinkError::Unexpected(message)) => {
                        log::error!("Failed to updated browser url. {message}")
                    }
                }
            }
        });
    }

    /// Focus the active terminal session, if there is one.
    pub fn focus_active_session(&mut self, ctx: &mut ViewContext<Self>) {
        self.update_session_visibility(ctx);

        if let Some(session_id) = self.active_session_id(ctx) {
            if self.focus_pane(session_id.into(), true, ctx) {
                ctx.emit(Event::AppStateChanged);
            }
        }
    }

    pub fn active_session_terminal_model(
        &self,
        app: &AppContext,
    ) -> Option<Arc<FairMutex<TerminalModel>>> {
        self.active_session_id(app)
            .and_then(|id| self.terminal_session_by_id(id))
            .map(|session| session.terminal_manager(app).as_ref(app).model())
    }

    fn focused_pane_content(&self, app: &AppContext) -> Option<&dyn PaneContent> {
        self.pane_contents
            .get(&self.focused_pane_id(app))
            .map(|pane| pane.as_pane())
    }

    /// The terminal view backing the active terminal session. This may not be the same as the
    /// focused pane, if a non-terminal pane is focused.
    pub fn active_session_view(&self, ctx: &AppContext) -> Option<ViewHandle<TerminalView>> {
        self.terminal_view_from_pane_id(self.active_session_id(ctx)?, ctx)
    }

    /// The terminal view backing the _focused_ terminal session. This will be the same
    /// as the active_session_view if the focused pane is a terminal pane.
    pub fn focused_session_view(&self, ctx: &AppContext) -> Option<ViewHandle<TerminalView>> {
        self.terminal_view_from_pane_id(self.focused_pane_id(ctx), ctx)
    }

    /// If the active session slot holds a swapped-in replacement (e.g. a
    /// child agent pane), returns the displaced original's pane ID and
    /// terminal view. Returns `None` when no swap is active.
    pub fn original_session_if_swapped(
        &self,
        ctx: &AppContext,
    ) -> Option<(PaneId, ViewHandle<TerminalView>)> {
        let active_pane_id = PaneId::from(self.active_session_id(ctx)?);
        let original_pane_id = self.panes.original_pane_for_replacement(active_pane_id)?;
        let view = self.terminal_view_from_pane_id(original_pane_id, ctx)?;
        Some((original_pane_id, view))
    }

    /// Given a pane ID, retrieve its backing terminal pane contents, if the pane is a terminal pane.
    fn terminal_session_by_id(&self, pane_id: impl Into<PaneId>) -> Option<&TerminalPane> {
        self.pane_contents
            .get(&pane_id.into())
            .and_then(|contents| contents.as_any().downcast_ref::<TerminalPane>())
    }

    /// Given a pane ID, retrieve its backing terminal view, if the pane is a terminal pane.
    pub fn terminal_view_from_pane_id(
        &self,
        pane_id: impl Into<PaneId>,
        ctx: &AppContext,
    ) -> Option<ViewHandle<TerminalView>> {
        self.terminal_session_by_id(pane_id)
            .map(|session| session.terminal_view(ctx))
    }

    pub fn attach_execution_session_to_ambient_pane(
        &mut self,
        pane_id: PaneId,
        session_id: SessionId,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        let Some(terminal_view) = self.terminal_view_from_pane_id(pane_id, ctx) else {
            log::warn!("Tried to attach execution session to non-terminal pane {pane_id:?}");
            return false;
        };

        if let Some(ambient_agent_view_model) = terminal_view
            .as_ref(ctx)
            .ambient_agent_view_model()
            .cloned()
        {
            ambient_agent_view_model.update(ctx, |model, ctx| {
                model.attach_execution_session(session_id, ctx);
            });
            return true;
        }

        let Some(terminal_manager) = self
            .terminal_session_by_id(pane_id)
            .map(|session| session.terminal_manager(ctx))
        else {
            log::warn!("Tried to attach execution session to pane without terminal manager");
            return false;
        };

        terminal_manager.update(ctx, |terminal_manager, ctx| {
            let Some(manager) = terminal_manager
                .as_any_mut()
                .downcast_mut::<shared_session::viewer::TerminalManager>()
            else {
                log::warn!("Tried to attach execution session to non-viewer terminal manager");
                return;
            };
            manager.attach_execution_session(session_id, ctx);
        });
        true
    }

    /// Resolve the pane id that owns a given conversation's `TerminalView`,
    /// without applying any visibility filtering. Used by the pill-bar swap
    /// path to find the orchestrator pane (or any non-child conversation's
    /// owner) regardless of whether it's currently visible. Walks the pane
    /// contents and returns the first terminal pane whose terminal view id
    /// matches the history model's owner for `conversation_id`.
    fn pane_id_for_conversation_owner(
        &self,
        conversation_id: AIConversationId,
        ctx: &AppContext,
    ) -> Option<PaneId> {
        let owner_view_id = BlocklistAIHistoryModel::as_ref(ctx)
            .terminal_view_id_for_conversation(&conversation_id)?;
        for pane_id in self.pane_contents.keys() {
            if let Some(terminal_view) = self.terminal_view_from_pane_id(*pane_id, ctx) {
                if terminal_view.id() == owner_view_id {
                    return Some(*pane_id);
                }
            }
        }
        None
    }

    /// Make the pane that owns `conversation_id` the visible one in the
    /// focused pane's slot via temporary replacement. The previous occupant
    /// is restored on revert (back-button, ESC, pill-click, close, split-off).
    pub fn swap_active_pane_to_conversation(
        &mut self,
        focused_pane_id: PaneId,
        conversation_id: AIConversationId,
        ctx: &mut ViewContext<Self>,
    ) {
        let from_child_panes = self.child_agent_panes.get(&conversation_id).copied();
        let from_visible_pane = self
            .find_visible_terminal_pane_for_conversation(conversation_id, ctx)
            .map(PaneId::from);
        let from_owner_lookup = self.pane_id_for_conversation_owner(conversation_id, ctx);
        let target_pane_id = from_child_panes.or(from_visible_pane).or(from_owner_lookup);

        let Some(target_pane_id) = target_pane_id else {
            // No owning pane in this group (e.g. the conversation lives
            // in another tab). Fall back to workspace-level navigation.
            if let Some(owner_view_id) = BlocklistAIHistoryModel::as_ref(ctx)
                .terminal_view_id_for_conversation(&conversation_id)
            {
                ctx.dispatch_typed_action(&WorkspaceAction::FocusTerminalViewInWorkspace {
                    terminal_view_id: owner_view_id,
                });
                return;
            }
            self.log_swap_resolution_failure(focused_pane_id, conversation_id, ctx);
            return;
        };

        // No-op when the active pill is clicked.
        if target_pane_id == focused_pane_id {
            return;
        }

        // If the target is currently swapped out (some other pane sits in
        // its slot), revert that swap and just focus the target. Skipping
        // this would put the target in two tree positions and corrupt the
        // layout on a later revert.
        if let Some(replacement_id) = self.panes.replacement_pane_for_original(target_pane_id) {
            self.revert_swap_clearing_split_off(replacement_id, ctx);
            self.handle_pane_count_change(ctx);
            self.focus_pane(target_pane_id, true, ctx);
            for pane_id in [replacement_id, target_pane_id] {
                if let Some(terminal_view) = self.terminal_view_from_pane_id(pane_id, ctx) {
                    terminal_view.update(ctx, |view, ctx| {
                        view.update_agent_view_back_button_state(ctx);
                    });
                }
            }
            ctx.emit(Event::TerminalViewStateChanged);
            ctx.emit(Event::AppStateChanged);
            return;
        }

        // If a swap is already active in this slot, revert it first; the
        // anchor for the new operation becomes the original pane.
        let anchor =
            if let Some(original) = self.panes.original_pane_for_replacement(focused_pane_id) {
                self.revert_swap_clearing_split_off(focused_pane_id, ctx);
                original
            } else {
                focused_pane_id
            };

        // If revert landed us on the target, just focus and return.
        if anchor == target_pane_id {
            self.handle_pane_count_change(ctx);
            self.focus_pane(anchor, true, ctx);
            if let Some(terminal_view) = self.terminal_view_from_pane_id(anchor, ctx) {
                terminal_view.update(ctx, |view, ctx| {
                    view.update_agent_view_back_button_state(ctx);
                });
            }
            ctx.emit(Event::TerminalViewStateChanged);
            ctx.emit(Event::AppStateChanged);
            return;
        }

        // If the target is already a visible sibling, just focus it.
        if self.panes.is_pane_in_tree(target_pane_id) && !self.panes.is_pane_hidden(&target_pane_id)
        {
            self.handle_pane_count_change(ctx);
            self.focus_pane(target_pane_id, true, ctx);
            if let Some(terminal_view) = self.terminal_view_from_pane_id(target_pane_id, ctx) {
                terminal_view.update(ctx, |view, ctx| {
                    view.update_agent_view_back_button_state(ctx);
                });
            }
            ctx.emit(Event::TerminalViewStateChanged);
            ctx.emit(Event::AppStateChanged);
            return;
        }

        // Substitute the target into the anchor's slot via temporary
        // replacement; revert restores the anchor.
        let success = self.panes.replace_pane(anchor, target_pane_id, true);
        if !success {
            log::warn!(
                "swap_active_pane_to_conversation: replace_pane failed for anchor={anchor:?} target={target_pane_id:?}"
            );
            return;
        }

        self.handle_pane_count_change(ctx);
        self.focus_pane(target_pane_id, true, ctx);

        // Refresh the back-button label on both swapped panes; otherwise
        // a stale label would persist until the next agent-view entry.
        for pane_id in [anchor, target_pane_id] {
            if let Some(terminal_view) = self.terminal_view_from_pane_id(pane_id, ctx) {
                terminal_view.update(ctx, |view, ctx| {
                    view.update_agent_view_back_button_state(ctx);
                });
            }
        }

        ctx.emit(Event::TerminalViewStateChanged);
        ctx.emit(Event::AppStateChanged);
    }

    /// Reveal the child agent pane for `conversation_id` as a visible
    /// sibling of its orchestrator ("Open in new pane"). Reuses the
    /// existing view to avoid cancelling in-flight commands. Reverts any
    /// swap on the target's orchestrator first; swaps belonging to other
    /// orchestrators in the same group are left alone.
    pub fn unhide_child_agent_pane_for_split_off(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ViewContext<Self>,
    ) -> Option<PaneId> {
        let child_pane_id = self.child_agent_panes.get(&conversation_id).copied()?;

        // If the child was previously split off and then swapped over,
        // it's recorded as the original of an active swap. Revert that
        // swap first — the child returns to its old slot — so we don't
        // splice it into the tree a second time.
        if let Some(replacement_pane_id) = self.panes.replacement_pane_for_original(child_pane_id) {
            self.revert_swap_clearing_split_off(replacement_pane_id, ctx);
            self.handle_pane_count_change(ctx);
        }

        // If the child is already a visible sibling, just focus it.
        if self.panes.is_pane_in_tree(child_pane_id)
            && !self.panes.is_pane_hidden(&child_pane_id)
            && self
                .panes
                .original_pane_for_replacement(child_pane_id)
                .is_none()
        {
            self.focus_pane(child_pane_id, true, ctx);
            return Some(child_pane_id);
        }

        // Resolve the target child's orchestrator. Used to scope swap
        // reverts so we don't disturb swaps owned by other orchestrators.
        let parent_pane_id = BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(&conversation_id)
            .and_then(|c| c.parent_conversation_id())
            .and_then(|parent_conv_id| self.pane_id_for_conversation_owner(parent_conv_id, ctx));

        // If the orchestrator is swapped out, revert it so it returns to
        // its slot before we split next to it.
        let split_base = if let Some(parent_pane_id) = parent_pane_id {
            if let Some(replacement_pane_id) =
                self.panes.replacement_pane_for_original(parent_pane_id)
            {
                self.revert_swap_clearing_split_off(replacement_pane_id, ctx);
            }
            parent_pane_id
        } else {
            self.focused_pane_id(ctx)
        };
        let split_ok = self
            .panes
            .split(split_base, child_pane_id, Direction::Right);
        if !split_ok {
            self.panes.split_root(child_pane_id, Direction::Right);
        }

        if let Some(child_terminal_view) = self.terminal_view_from_pane_id(child_pane_id, ctx) {
            child_terminal_view.update(ctx, |view, ctx| {
                view.mark_as_orchestration_split_off(ctx);
            });
        }

        // Refresh the back-button label on the orchestrator pane.
        if let Some(terminal_view) = self.terminal_view_from_pane_id(split_base, ctx) {
            terminal_view.update(ctx, |view, ctx| {
                view.update_agent_view_back_button_state(ctx);
            });
        }

        self.handle_pane_count_change(ctx);
        self.focus_pane(child_pane_id, true, ctx);
        ctx.emit(Event::TerminalViewStateChanged);
        ctx.emit(Event::AppStateChanged);
        Some(child_pane_id)
    }

    /// Detach the child agent pane for `conversation_id` so it can be
    /// re-parented into a new tab ("Open in new tab"). Reuses the
    /// existing view to avoid cancelling in-flight commands.
    pub fn take_child_agent_pane_for_split_off(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ViewContext<Self>,
    ) -> Option<Box<dyn AnyPaneContent>> {
        let child_pane_id = self.child_agent_panes.remove(&conversation_id)?;

        // Capture focus before mutating the tree so we can shift focus
        // correctly afterwards regardless of how the child leaves it.
        let was_focused = self.focus_state.as_ref(ctx).is_pane_focused(child_pane_id);

        // Revert the swap if the child is currently swapped in.
        if self
            .panes
            .original_pane_for_replacement(child_pane_id)
            .is_some()
        {
            self.panes.revert_temporary_replacement(child_pane_id);
        }

        // Remove the child from the tree if it was a real sibling.
        if self.panes.is_pane_in_tree(child_pane_id) && !self.panes.remove(child_pane_id) {
            log::error!("take_child_agent_pane_for_split_off: failed to remove pane from tree");
        }

        // Drop any leftover swap entry naming this child as the original
        // side. Otherwise a later revert of a surviving sibling would
        // splice the now-detached child back into this group's tree.
        self.panes.remove_hidden_pane(child_pane_id);

        // Shift focus and active session away from the departing child.
        self.focus_next_terminal_pane_and_activate_session(
            child_pane_id,
            PaneRemovalReason::Move,
            ctx,
        );
        let in_split_pane = self.panes.visible_pane_count() > 1;
        self.focus_state.update(ctx, |focus_state, ctx| {
            focus_state.set_in_split_pane(in_split_pane, ctx);
            if was_focused {
                focus_state.set_focused_pane_maximized(false, ctx);
            }
        });

        if let Some(child_terminal_view) = self.terminal_view_from_pane_id(child_pane_id, ctx) {
            child_terminal_view.update(ctx, |view, ctx| {
                view.mark_as_orchestration_split_off(ctx);
            });
        }

        // Detach so the destination group can re-attach cleanly.
        if let Some(pane_data) = self.pane_contents.get(&child_pane_id) {
            let pane = pane_data.as_pane();
            pane.detach(self, DetachType::Moved, ctx);
        }

        let pane_content = self.pane_contents.remove(&child_pane_id);
        ctx.notify();
        ctx.emit(Event::TerminalViewStateChanged);
        ctx.emit(Event::AppStateChanged);
        pane_content
    }

    /// Stamp this pane group as the destination of a split-off child
    /// agent pane, so closing the tab re-adopts the live view back to
    /// the source group.
    pub fn set_child_agent_origin(&mut self, origin: ChildAgentOrigin) {
        self.child_agent_origin = Some(origin);
    }

    /// Returns the origin metadata if this group is hosting a split-off
    /// child agent tab.
    pub fn child_agent_origin(&self) -> Option<&ChildAgentOrigin> {
        self.child_agent_origin.as_ref()
    }

    /// Re-adopt a previously detached child agent pane back into this
    /// group as off-tree, and clear its split-off marker so the next
    /// reveal renders pills instead of breadcrumbs.
    pub fn re_adopt_child_agent_pane(
        &mut self,
        pane_content: Box<dyn AnyPaneContent>,
        conversation_id: AIConversationId,
        ctx: &mut ViewContext<Self>,
    ) {
        let pane_id = pane_content.as_pane().id();

        if let Some(returned_pane_id) = self.attach_child_pane_off_tree(pane_content, ctx) {
            debug_assert_eq!(returned_pane_id, pane_id);
            self.child_agent_panes.insert(conversation_id, pane_id);
        } else {
            log::error!("re_adopt_child_agent_pane: failed to attach pane {pane_id:?}");
            return;
        }

        // Clear the split-off marker so the next reveal via the
        // orchestration pill bar renders the full pill bar rather than
        // the parent → child breadcrumb. The pane is once again an
        // off-tree child agent of the orchestrator, not a top-level
        // split-off.
        if let Some(terminal_view) = self.terminal_view_from_pane_id(pane_id, ctx) {
            terminal_view.update(ctx, |view, ctx| {
                view.clear_orchestration_split_off(ctx);
            });
        }

        ctx.notify();
        ctx.emit(Event::TerminalViewStateChanged);
        ctx.emit(Event::AppStateChanged);
    }

    /// Diagnostic logging for [`swap_active_pane_to_conversation`] when none of
    /// the three resolvers (`child_agent_panes`, visible-pane lookup, history
    /// model owner lookup) finds a pane in this group for the target
    /// conversation. Dumps enough state to identify which step is wrong.
    fn log_swap_resolution_failure(
        &self,
        focused_pane_id: PaneId,
        conversation_id: AIConversationId,
        ctx: &AppContext,
    ) {
        let history_model = BlocklistAIHistoryModel::as_ref(ctx);
        let history_owner_view_id =
            history_model.terminal_view_id_for_conversation(&conversation_id);
        let conversation_in_memory = history_model.conversation(&conversation_id).is_some();
        let parent_id = history_model
            .conversation(&conversation_id)
            .and_then(|c| c.parent_conversation_id());
        let is_remote_child = history_model
            .conversation(&conversation_id)
            .map(|c| c.is_remote_child())
            .unwrap_or(false);

        let focused_view_id = self
            .terminal_view_from_pane_id(focused_pane_id, ctx)
            .map(|v| v.id());

        let child_pane_keys: Vec<AIConversationId> =
            self.child_agent_panes.keys().copied().collect();
        let has_child_entry = self.child_agent_panes.contains_key(&conversation_id);

        // Per-pane summary: pane_id, terminal_view_id, hidden state,
        // agent_view active conversation id.
        let mut pane_summaries: Vec<String> = Vec::new();
        for pane_id in self.pane_contents.keys().copied() {
            let Some(terminal_view) = self.terminal_view_from_pane_id(pane_id, ctx) else {
                pane_summaries.push(format!("{pane_id:?}=non_terminal"));
                continue;
            };
            let view_id = terminal_view.id();
            let active_conv = terminal_view
                .as_ref(ctx)
                .agent_view_controller()
                .as_ref(ctx)
                .agent_view_state()
                .active_conversation_id();
            let hidden_for_close = self.is_pane_hidden_for_close(pane_id);
            let hidden_for_child_agent = self.panes.is_pane_hidden_for_child_agent(pane_id);
            let in_tree = self.panes.is_pane_in_tree(pane_id);
            let is_temp_replacement = self.panes.original_pane_for_replacement(pane_id).is_some();
            let is_hidden_any = self.panes.is_pane_hidden(&pane_id);
            pane_summaries.push(format!(
                "{pane_id:?}{{view={view_id:?},active={active_conv:?},in_tree={in_tree},hidden={is_hidden_any},close={hidden_for_close},child_agent={hidden_for_child_agent},temp_replacement={is_temp_replacement}}}"
            ));
        }

        log::warn!(
            "swap_active_pane_to_conversation: no pane found for conversation {conversation_id:?} \
             [focused_pane={focused_pane_id:?}, focused_view={focused_view_id:?}, \
             history_owner_view={history_owner_view_id:?}, in_memory={conversation_in_memory}, \
             parent={parent_id:?}, remote_child={is_remote_child}, \
             child_agent_panes_has_entry={has_child_entry}, \
             child_agent_panes_keys={child_pane_keys:?}, panes=[{}]]",
            pane_summaries.join(", ")
        );
    }

    /// Walk the visible terminal panes in this group looking for one whose
    /// terminal view has the given AI conversation as its active agent-view
    /// conversation. Used by the orchestration pill bar to focus an
    /// already-visible pane (e.g. "Open in new pane" was already used and the
    /// user is now clicking the pinned pill in the orchestrator's view).
    ///
    /// "Visible" here strictly means "present as a leaf in the layout tree
    /// AND not hidden". Iterating over `terminal_pane_ids()` (which reads
    /// from `pane_contents`) would erroneously include off-tree child
    /// agent panes, since under the orchestration model those panes
    /// remain in `pane_contents` even when they are not in the tree.
    pub(crate) fn find_visible_terminal_pane_for_conversation(
        &self,
        conversation_id: AIConversationId,
        ctx: &AppContext,
    ) -> Option<TerminalPaneId> {
        for pane_id in self.panes.visible_pane_ids() {
            if FeatureFlag::UndoClosedPanes.is_enabled() && self.is_pane_hidden_for_close(pane_id) {
                continue;
            }
            let Some(terminal_pane_id) = pane_id.as_terminal_pane_id() else {
                continue;
            };
            let Some(terminal_view) = self.terminal_view_from_pane_id(pane_id, ctx) else {
                continue;
            };
            let active_id = terminal_view
                .as_ref(ctx)
                .agent_view_controller()
                .as_ref(ctx)
                .agent_view_state()
                .active_conversation_id();
            if active_id == Some(conversation_id) {
                return Some(terminal_pane_id);
            }
        }
        None
    }

    /// Given a pane ID, retrieve its backing code view, if the pane is a code pane.
    pub fn code_view_from_pane_id(
        &self,
        pane_id: impl Into<PaneId>,
        ctx: &AppContext,
    ) -> Option<ViewHandle<CodeView>> {
        self.pane_contents
            .get(&pane_id.into())
            .and_then(|contents| contents.as_any().downcast_ref::<CodePane>())
            .map(|pane| pane.file_view(ctx))
    }

    fn update_pane_history(&mut self, new_pane: PaneId) {
        self.pane_history.retain(|&x| x != new_pane);
        self.pane_history.push(new_pane);
    }

    fn remove_from_pane_history(&mut self, pane: PaneId) {
        self.pane_history.retain(|&x| x != pane);
    }

    /// Switch focus to a pane. If the pane is a terminal session, it also becomes the active terminal session.
    /// If focus_pane_contents is true, then the pane's contents will be focused in the UI framework.
    /// Returns whether the pane was actually focused.
    pub fn focus_pane(
        &mut self,
        id: PaneId,
        focus_pane_contents: bool,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        // No-op if the pane is hidden-for-close (undo stack) or no longer present.
        if FeatureFlag::UndoClosedPanes.is_enabled() && self.is_pane_hidden_for_close(id) {
            return false;
        }
        if !self.pane_contents.contains_key(&id) {
            return false;
        }
        // Saves the handle of a currently focused terminal pane before switching away from it.
        let maybe_origin_terminal_view =
            self.terminal_view_from_pane_id(self.focused_pane_id(ctx), ctx);

        if self.focused_pane_id(ctx) == id
            // As a safeguard, don't allow switching to unknown panes.
            || !self.pane_contents.contains_key(&id)
        {
            return false;
        }

        self.focus_state.update(ctx, |focus_state, ctx| {
            focus_state.set_focused_pane(id, ctx);
        });

        ctx.emit(Event::PaneTitleUpdated);
        // Update the active session if the newly focused pane is a terminal pane.
        if let Some(terminal_pane_id) = id.as_terminal_pane_id() {
            self.focus_state.update(ctx, |focus_state, ctx| {
                focus_state.set_active_session(Some(terminal_pane_id), ctx);
            });
        }
        ctx.notify();

        // Dismisses tooltips on a terminal pane that we've switched away from.
        if let Some(view) = maybe_origin_terminal_view {
            view.update(ctx, |terminal_view, ctx| {
                terminal_view.dismiss_tooltips(ctx);
                ctx.notify();
            });
        }

        // There are some instances of focusing a pane where we don't actually want to focus the pane contents
        // immediately within the UI framework. For instance, if this pane is being focused in the pane
        // group as a result of another pane being move, then we don't actually need the contents
        // to take focus in the ui framework.
        if focus_pane_contents {
            self.focus(ctx);
        }
        true
    }

    fn focus_pane_and_record_in_history(
        &mut self,
        id: PaneId,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        let focused = self.focus_pane(id, true, ctx);
        if focused {
            self.update_pane_history(id);
        }
        focused
    }

    pub fn terminal_manager(
        &self,
        pane_index: usize,
        app: &AppContext,
    ) -> Option<ModelHandle<Box<dyn TerminalManager>>> {
        self.terminal_session_by_pane_index(pane_index)
            .map(|session| session.terminal_manager(app))
    }

    // When user clicked on the close tab button, we should wind down the existing panes
    // by deleting all the saved blocks in each pane from the database.
    pub fn clean_up_panes(&self, ctx: &mut ViewContext<Self>) {
        for pane in self.pane_contents.values() {
            let pane = pane.as_pane();
            pane.detach(self, DetachType::Closed, ctx);
        }
    }

    fn clean_up_pane(&self, pane_id: PaneId, ctx: &mut ViewContext<Self>) {
        match self.pane_contents.get(&pane_id) {
            Some(data) => {
                let pane = data.as_pane();
                pane.detach(self, DetachType::Closed, ctx);
            }
            None => log::error!("Could not find data for pane id: {pane_id:?}"),
        };
    }

    /// Detach all panes from this group. This is called when a tab is closed, but may still
    /// be restored.
    pub fn detach_panes(&self, ctx: &mut ViewContext<Self>) {
        for pane in self.pane_contents.values() {
            let pane = pane.as_pane();
            pane.detach(self, DetachType::HiddenForClose, ctx);
        }
    }

    /// Detach all panes and clean up associated state when closing a tab.
    /// This should be called instead of `detach_panes` when the pane group is being destroyed.
    pub fn detach_panes_for_close(
        &self,
        working_directories_model: &ModelHandle<WorkingDirectoriesModel>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.detach_panes(ctx);

        // Clean up any state associated with this pane group (global search views, etc.)
        let pane_group_id = ctx.view_id();
        working_directories_model.update(ctx, |model, ctx| {
            model.remove_pane_group(pane_group_id, ctx);
        });
    }

    /// Reattach all panes to this group. This is called when a closed tab is restored.
    pub fn reattach_panes(&mut self, ctx: &mut ViewContext<Self>) {
        let pane_ids = self.pane_contents.keys().copied().collect_vec();
        for pane_id in pane_ids {
            let Some(pane) = self.pane_contents.get(&pane_id) else {
                continue;
            };
            self.attach_pane(pane.as_ref(), ctx);
            self.restore_missing_child_agent_panes_for_terminal_pane_if_needed(pane_id, ctx);
        }
    }

    /// Attempts to attach a pane, calling pre_attach first.
    /// Returns true if attachment succeeded, false if pre_attach prevented it.
    fn try_attach_pane(&self, pane: &dyn AnyPaneContent, ctx: &mut ViewContext<Self>) -> bool {
        if pane.pre_attach(self, ctx) {
            self.attach_pane(pane, ctx);
            true
        } else {
            false
        }
    }

    /// Attaches a pane and does some post-attach work. All internal pane attachments
    /// should go through this API.
    pub fn attach_pane(&self, pane: &dyn AnyPaneContent, ctx: &mut ViewContext<Self>) {
        // Attach the pane.
        let pane = pane.as_pane();
        let focus_handle = focus_state::PaneFocusHandle::new(pane.id(), self.focus_state.clone());
        pane.attach(self, focus_handle, ctx);

        // Title updates need to get propagated up to workspace (to update tab bar and window title).
        ctx.subscribe_to_model(&pane.pane_configuration(), |_group, _, event, ctx| {
            if matches!(
                event,
                PaneConfigurationEvent::TitleUpdated
                    | PaneConfigurationEvent::VerticalTabsTitleUpdated
            ) {
                ctx.emit(Event::PaneTitleUpdated);
            }
        });
    }

    fn estimated_view_bounds(ctx: &mut ViewContext<Self>) -> RectF {
        let window_id = ctx.window_id();
        let window_bounds = match ctx.window_bounds(&window_id) {
            Some(rect) => {
                let size = rect.size();
                if size.x() == 0. || size.y() == 0. {
                    RectF::new(Vector2F::zero(), *FALLBACK_INITIAL_WINDOW_SIZE)
                } else {
                    rect
                }
            }
            None => RectF::new(Vector2F::zero(), *FALLBACK_INITIAL_WINDOW_SIZE),
        };

        // Subtract the padding used in the workspace view for the panel
        // border.
        let window_bounds = window_bounds.contract(crate::workspace::WORKSPACE_PADDING);

        let tab_bar_offset = vec2f(0.0, workspace::TOTAL_TAB_BAR_HEIGHT);
        RectF::new(
            window_bounds.origin() + tab_bar_offset,
            window_bounds.size() - tab_bar_offset,
        )
    }

    pub fn number_of_shared_sessions(&self, ctx: &AppContext) -> usize {
        self.shared_session_view_ids(ctx).len()
    }

    pub fn shared_session_view_ids(&self, ctx: &AppContext) -> Vec<EntityId> {
        self.panes_of::<TerminalPane>()
            .filter_map(|p| {
                let terminal_view = p.terminal_view(ctx);
                let is_shared = terminal_view.as_ref(ctx).is_sharing_session();
                is_shared.then(|| terminal_view.id())
            })
            .collect()
    }

    /// Filters out any hidden panes that aren't yet deleted (due to undo functionality).
    pub fn terminal_views(&self, ctx: &AppContext) -> Vec<ViewHandle<TerminalView>> {
        self.panes_of::<TerminalPane>()
            .filter(|p| !self.is_pane_hidden_for_close(p.terminal_pane_id().into()))
            .map(|p| p.terminal_view(ctx))
            .collect()
    }

    /// Returns terminal views from layout-tree-visible panes only.
    /// Unlike `terminal_views()`, this excludes off-tree child agent panes
    /// and panes hidden for any reason (temporary replacement, child agent, etc.).
    pub fn visible_terminal_views(&self, ctx: &AppContext) -> Vec<ViewHandle<TerminalView>> {
        self.panes
            .visible_pane_ids()
            .into_iter()
            .filter_map(|pane_id| self.terminal_view_from_pane_id(pane_id, ctx))
            .collect()
    }

    pub fn code_views(&self, ctx: &AppContext) -> Vec<ViewHandle<CodeView>> {
        self.panes_of::<CodePane>()
            .map(|p| p.file_view(ctx))
            .collect()
    }

    pub fn code_diff_views(&self, ctx: &AppContext) -> Vec<ViewHandle<CodeDiffView>> {
        self.panes_of::<CodeDiffPane>()
            .map(|p| p.diff_view(ctx))
            .collect()
    }

    pub fn file_notebook_views(&self, ctx: &AppContext) -> Vec<ViewHandle<FileNotebookView>> {
        self.panes_of::<FilePane>()
            .map(|p| p.file_view(ctx))
            .collect()
    }

    /// Get all terminal CWDs for this pane group.
    /// This is used by the Workspace to refresh the active directories model.
    pub fn terminal_view_working_directories<'a>(
        &'a self,
        ctx: &'a AppContext,
    ) -> impl Iterator<Item = (EntityId, Option<LocalOrRemotePath>)> + 'a {
        self.terminal_views(ctx).into_iter().map(|terminal_view| {
            let terminal_id = terminal_view.id();
            let cwd = terminal_view.as_ref(ctx).pwd_as_local_or_remote(ctx);
            (terminal_id, cwd)
        })
    }

    /// Get all code editor paths (local and remote) for this pane group.
    /// This is used by the Workspace to refresh the active directories model.
    pub fn code_view_paths<'a>(
        &'a self,
        ctx: &'a AppContext,
    ) -> impl Iterator<Item = (EntityId, Option<LocalOrRemotePath>)> + 'a {
        self.code_views(ctx).into_iter().map(move |code_view| {
            let id = code_view.id();
            let location = code_view
                .as_ref(ctx)
                .tab_at(code_view.as_ref(ctx).active_tab_index())
                .and_then(|tab| tab.location().cloned());
            (id, location)
        })
    }

    pub fn code_diff_view_paths<'a>(
        &'a self,
        ctx: &'a AppContext,
    ) -> impl Iterator<Item = (EntityId, Option<LocalOrRemotePath>)> + 'a {
        self.code_diff_views(ctx).into_iter().map(move |diff_view| {
            let id = diff_view.id();
            let location = diff_view.as_ref(ctx).primary_file_location(ctx);
            (id, location)
        })
    }

    pub fn file_notebook_paths<'a>(
        &'a self,
        ctx: &'a AppContext,
    ) -> impl Iterator<Item = (EntityId, Option<LocalOrRemotePath>)> + 'a {
        self.file_notebook_views(ctx)
            .into_iter()
            .map(move |file_view| {
                let id = file_view.id();
                let path = file_view.as_ref(ctx).path().cloned();
                (id, path)
            })
    }

    #[cfg(test)]
    pub fn is_share_session_modal_open(&self) -> bool {
        self.terminal_with_open_share_session_modal.is_some()
    }

    #[cfg(test)]
    pub fn share_session_modal(&self) -> &ViewHandle<ShareSessionModal> {
        &self.share_session_modal
    }

    pub(crate) fn start_agent_mode_in_new_pane(
        &mut self,
        initial_query: Option<&str>,
        zero_state_prompt_suggestion_type: Option<ZeroStatePromptSuggestionType>,
        ctx: &mut ViewContext<Self>,
    ) {
        if let Some(terminal_view) = self.focused_session_view(ctx) {
            terminal_view.update(ctx, |terminal_view, terminal_view_ctx| {
                terminal_view.enter_agent_view_for_new_conversation(
                    None,
                    // TODO(zachbai): This is just a placeholder origin - I'm not even sure
                    // if this is called in live codepaths beyond the create-environment deep
                    // link flow.
                    AgentViewEntryOrigin::Input {
                        was_prompt_autodetected: false,
                    },
                    terminal_view_ctx,
                );

                if let Some(initial_query) = initial_query {
                    terminal_view
                        .input()
                        .update(terminal_view_ctx, |input, ctx| {
                            input.replace_buffer_content(initial_query, ctx);
                            input.focus_input_box(ctx);
                        });
                }
                if let Some(zero_state_prompt_suggestion_type) = zero_state_prompt_suggestion_type {
                    terminal_view
                        .input()
                        .update(terminal_view_ctx, |input, ctx| {
                            input.insert_zero_state_prompt_suggestion(
                                zero_state_prompt_suggestion_type,
                                ZeroStatePromptSuggestionTriggeredFrom::TryAgentModeBanner,
                                ctx,
                            );
                        });
                }
            });
        }
    }

    /// Add and focus a terminal pane in AI mode. Adds the pane to the right of all other panes as
    /// a split on the root node. If `initial_query` is `Some` pre-fill the input with its value.
    pub(crate) fn add_terminal_pane_in_agent_mode(
        &mut self,
        initial_query: Option<&str>,
        zero_state_prompt_suggestion_type: Option<ZeroStatePromptSuggestionType>,
        ctx: &mut ViewContext<Self>,
    ) {
        // We can only control the size of a pane that hasn't been laid out by setting `PaneFlex`
        // ratios because we don't have element sizes until we lay out the view. Here we make sure
        // the Agent Mode pane will have at least the desired minimum width by checking the size of
        // the already laid out panes.
        let root_pane_width = self.panes.root.pane_size(ctx).x();
        // The Agent Mode pane should take up no more than 50% of the root pane's width.
        let new_pane_min_width = AGENT_MODE_PANE_DEFAULT_MINIMUM_WIDTH.min(root_pane_width / 2.);

        let flex_for_min_width = {
            let root_horizontal_flex_values_sum = self
                .panes
                .root
                .pane_flex_sum_along_axis(SplitDirection::Horizontal);
            let default_new_pane_width =
                root_pane_width / (root_horizontal_flex_values_sum + DEFAULT_FLEX_VALUE);

            let remaining_width_for_existing_panes = root_pane_width - new_pane_min_width;
            let ratio = new_pane_min_width / remaining_width_for_existing_panes;

            if default_new_pane_width < new_pane_min_width && ratio.is_normal() && ratio > 0. {
                Some(PaneFlex(root_horizontal_flex_values_sum * ratio))
            } else {
                None
            }
        };

        self.add_session_with_default_session_mode_behavior(
            Direction::Right,
            None,
            self.focused_pane_id(ctx).as_terminal_pane_id(),
            None, /* chosen_shell */
            None, /* conversation_restoration */
            DefaultSessionModeBehavior::Ignore,
            ctx,
        );

        // Now that the Agent Mode pane has been inserted into the pane tree, we can update its
        // `PaneFlex` value.
        if let Some(custom_flex) = flex_for_min_width {
            if let PaneNode::Branch(ref mut root_branch) = self.panes.root {
                if let Some((agent_mode_pane_flex, PaneNode::Leaf(_))) =
                    root_branch.nodes.last_mut()
                {
                    *agent_mode_pane_flex = custom_flex;
                }
            }
        }

        ctx.emit(Event::AppStateChanged);

        self.start_agent_mode_in_new_pane(initial_query, zero_state_prompt_suggestion_type, ctx);
    }

    /// Creates an ambient agent pane with the given initial prompt.
    fn create_ambient_agent_pane(&self, ctx: &mut ViewContext<Self>) -> TerminalPane {
        let uuid = Uuid::new_v4();
        let resources = TerminalViewResources {
            tips_completed: self.tips_completed.clone(),
            server_api: self.server_api.clone(),
            model_event_sender: self.model_event_sender.clone(),
        };

        let view_bounds = Self::estimated_view_bounds(ctx);

        let (terminal_view, terminal_manager) =
            Self::create_ambient_agent_terminal(resources, view_bounds.size(), ctx);

        TerminalPane::new(
            uuid.into_bytes().to_vec(),
            terminal_manager,
            terminal_view,
            self.model_event_sender.clone(),
            ctx,
        )
    }

    /// Add and focus a cloud mode pane.
    pub fn add_ambient_agent_pane(&mut self, ctx: &mut ViewContext<Self>) {
        if !FeatureFlag::AgentView.is_enabled() || !FeatureFlag::CloudMode.is_enabled() {
            return;
        }

        let pane_data = self.create_ambient_agent_pane(ctx);

        // Add the pane to the right
        let _ = self.add_pane(Direction::Right, None, Box::new(pane_data), true, ctx);
    }

    /// Close overlays whose state is managed by this pane group or its terminal panes. Does not
    /// change what element is focused.
    pub fn close_overlays(&mut self, ctx: &mut ViewContext<Self>) {
        self.for_all_terminal_panes(
            |terminal_view, ctx| {
                terminal_view.close_overlays(ctx);
            },
            ctx,
        );

        self.for_all_code_panes(
            |code_view, ctx| {
                code_view.close_overlays(ctx);
            },
            ctx,
        );

        self.close_share_session_modal(ctx);
        self.close_shared_session_role_change_modal(RoleChangeCloseSource::ViewerRequest, ctx);
        self.terminal_with_open_share_block_modal = None;
        ctx.notify();
    }

    /// Updates the pane group's state in response to a view within a pane
    /// receiving focus.
    fn handle_focus_change(&mut self, ctx: &mut ViewContext<Self>) {
        for pane_index in 0..self.pane_count() {
            if let Some(content) = self.pane_by_index(pane_index) {
                if content.has_application_focus(ctx) {
                    if let Some(pane_id) = self.pane_id_from_index(pane_index) {
                        // Mark the pane as the focused pane _without_ moving
                        // application focus to it.
                        //
                        // DO NOT CHANGE FALSE TO TRUE HERE!  It can create an
                        // infinite loop of panes getting focused.  This
                        // codepath should only be invoked when focus has
                        // already changed, so we only want to update our own
                        // state, and not manipulate application focus.
                        self.focus_pane(pane_id, false, ctx);
                        self.update_pane_history(pane_id);
                        ctx.emit(Event::PaneFocused);
                    };
                    break;
                }
            }
        }
    }
}

impl Entity for PaneGroup {
    type Event = Event;
}

impl TypedActionView for PaneGroup {
    type Action = PaneGroupAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        use PaneGroupAction::*;
        match action {
            Add(direction) => {
                let chosen_shell = {
                    if let Some(model) = self.active_session_terminal_model(ctx) {
                        let model = model.lock();
                        model.shell_launch_state().available_shell()
                    } else {
                        None
                    }
                };
                self.add_terminal_pane(*direction, chosen_shell, ctx);
            }
            Remove(view_id) => self.close_pane_with_confirmation(*view_id, ctx),
            RemoveActive => self.close_active_pane_with_confirmation(ctx),
            Activate(view_id, reason) => self.focus_pane_on_mouse_event(*view_id, *reason, ctx),
            ResizeMove(position) => self.maybe_resize_pane(*position, ctx),
            StartResizing(border) => self.start_resizing(*border, ctx),
            ResetPaneSizes(border_id) => self.reset_pane_sizes(*border_id, ctx),
            EndResizing => self.end_resizing(ctx),
            ResizeLeft => self.resize_left(ctx),
            ResizeRight => self.resize_right(ctx),
            ResizeUp => self.resize_up(ctx),
            ResizeDown => self.resize_down(ctx),
            NavigatePrev => self.navigate_prev_pane(ctx),
            NavigateNext => self.navigate_next_pane(ctx),
            NavigateLeft => self.navigate_pane_by_direction(Direction::Left, ctx),
            NavigateRight => self.navigate_pane_by_direction(Direction::Right, ctx),
            NavigateUp => self.navigate_pane_by_direction(Direction::Up, ctx),
            NavigateDown => self.navigate_pane_by_direction(Direction::Down, ctx),
            ToggleMaximizePane => self.toggle_maximize_pane(ctx),
            Move {
                id,
                target_pane_id,
                direction,
            } => self.move_pane(*id, *target_pane_id, *direction, ctx),
            HandleFocusChange => self.handle_focus_change(ctx),
            FocusTerminalView(terminal_view_id) => self.focus_terminal_view(*terminal_view_id, ctx),
        }
    }
}

impl View for PaneGroup {
    fn ui_name() -> &'static str {
        "PaneGroup"
    }

    fn keymap_context(&self, app: &AppContext) -> Context {
        let mut ctx = Self::default_keymap_context();

        if self.is_focused_pane_maximized(app) {
            ctx.set.insert("PaneGroup_PaneMaximized");
        }

        if self.any_pane_being_dragged(app) {
            ctx.set.insert("PaneGroup_PaneDragging");
        }

        match self.panes.len() {
            0 => {
                debug_assert!(false, "Should always be at least one pane");
            }
            1 => {
                ctx.set.insert("PaneGroup_SinglePane");
            }
            _ => {
                ctx.set.insert("PaneGroup_MultiplePanes");
            }
        };

        ctx
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);

        let mut column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Max);

        if self
            .user_default_shell_unsupported_banner_model_handle
            .as_ref(app)
            == &BannerState::Open
        {
            column.add_child(ChildView::new(&self.user_default_shell_changed_banner).finish());
        }

        let main_content = if self.is_focused_pane_maximized(app) {
            self.focused_pane_id(app).render(app)
        } else {
            EventHandler::new(self.panes.render(appearance.theme(), app))
                .on_mouse_dragged(move |ctx, _, position| {
                    ctx.dispatch_typed_action(PaneGroupAction::ResizeMove(position));
                    DispatchEventResult::StopPropagation
                })
                .on_left_mouse_up(move |ctx, _, _| {
                    ctx.dispatch_typed_action(PaneGroupAction::EndResizing);
                    DispatchEventResult::StopPropagation
                })
                .finish()
        };
        column.add_child(Shrinkable::new(1., main_content).finish());

        let mut stack = Stack::new().with_child(column.finish());

        // Render the share modals on the pane group level so that their
        // size is not restricted to within the terminal view.
        if self.terminal_with_open_share_block_modal.is_some() {
            stack
                .add_child(Clipped::new(ChildView::new(&self.share_block_modal).finish()).finish());
        } else if FeatureFlag::CreatingSharedSessions.is_enabled()
            && self.terminal_with_open_share_session_modal.is_some()
        {
            stack.add_child(ChildView::new(&self.share_session_modal).finish());
        } else if self
            .terminal_with_shared_session_role_change_modal_open
            .is_some()
        {
            stack.add_child(ChildView::new(&self.shared_session_role_change_modal).finish());
        }

        // Render the summarization cancel dialog at tab level when open.
        if let Some(terminal_pane_id) = self.terminal_with_open_summarization_dialog {
            if let Some(terminal_view) = self.terminal_view_from_pane_id(terminal_pane_id, app) {
                if let Some(dialog_handle) = terminal_view.read(app, |view, ctx| {
                    view.summarization_cancel_dialog_handle(ctx)
                }) {
                    stack.add_child(ChildView::new(&dialog_handle).finish());
                }
            }
        }

        // Render environment setup mode selector at tab level when open.
        if let Some(pane_id) = self.pane_with_open_environment_setup_mode_selector {
            let selector_handle = self
                .terminal_view_from_pane_id(pane_id, app)
                .and_then(|tv| {
                    tv.as_ref(app)
                        .environment_setup_mode_selector_handle()
                        .cloned()
                })
                .or_else(|| {
                    self.downcast_pane_by_id::<EnvironmentManagementPane>(pane_id)
                        .and_then(|emp| {
                            emp.environments_page_view(app)
                                .as_ref(app)
                                .environment_setup_mode_selector_handle()
                                .cloned()
                        })
                });
            if let Some(handle) = selector_handle {
                stack.add_child(ChildView::new(&handle).finish());
            }
        }

        // Render agent-assisted environment modal at tab level when open.
        if let Some(pane_id) = self.pane_with_open_agent_assisted_environment_modal {
            if let Some(handle) = self
                .downcast_pane_by_id::<EnvironmentManagementPane>(pane_id)
                .and_then(|emp| {
                    emp.environments_page_view(app)
                        .as_ref(app)
                        .agent_assisted_environment_modal_handle(app)
                        .cloned()
                })
            {
                stack.add_child(ChildView::new(&handle).finish());
            }
        }

        stack.finish()
    }

    fn on_window_transferred(
        &mut self,
        _old_window_id: WindowId,
        _new_window_id: WindowId,
        _ctx: &mut ViewContext<Self>,
    ) {
    }
}
