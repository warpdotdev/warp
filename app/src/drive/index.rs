#[cfg(target_family = "wasm")]
use crate::uri::web_intent_parser::open_url_on_desktop;
use crate::{
    ai::{
        document::ai_document_model::AIDocumentId,
        facts::{AIFact, AIMemory},
    },
    appearance::Appearance,
    auth::{
        auth_manager::{AuthManager, LoginGatedFeature},
        auth_state::AuthState,
        auth_view_modal::AuthViewVariant,
        AuthStateProvider,
    },
    cloud_object::{
        model::{
            persistence::{CloudModel, CloudModelEvent},
            view::{CloudViewModel, CloudViewModelEvent, UpdateTimestamp},
        },
        CloudObject, CloudObjectEventEntrypoint, CloudObjectLocation, CloudObjectSyncStatus,
        GenericCloudObject, GenericStringObjectFormat, JsonObjectType, NumInFlightRequests,
        ObjectType, Space,
    },
    editor::{EditorView, Event as EditorEvent, SingleLineEditorOptions},
    env_vars::CloudEnvVarCollection,
    features::FeatureFlag,
    menu::{Event, Menu, MenuItem, MenuItemFields},
    network::NetworkStatus,
    notebooks::CloudNotebookModel,
    report_if_error, send_telemetry_from_ctx,
    server::{
        cloud_objects::update_manager::{FetchSingleObjectOption, UpdateManager},
        ids::{ClientId, ObjectUid, ServerId, SyncId},
        sync_queue::SyncQueue,
        telemetry::{AnonymousUserSignupEntrypoint, SharingDialogSource, TelemetryEvent},
    },
    settings::app_installation_detection::{UserAppInstallDetectionSettings, UserAppInstallStatus},
    ui_components::{
        blended_colors,
        buttons::{highlight, icon_button},
        icons::{Icon, ICON_DIMENSIONS},
        menu_button::{icon_button_with_context_menu, MenuDirection},
    },
    util::color::coloru_with_opacity,
    view_components::{Dropdown, DropdownItem},
    workflows::{CloudWorkflow, WorkflowViewMode},
    workspace::active_terminal_in_window,
    workspaces::{
        update_manager::TeamUpdateManager, user_workspaces::UserWorkspaces, workspace::WorkspaceUid,
    },
    ObjectActions,
};

use super::{
    cloud_object_naming_dialog::CloudObjectNamingDialog,
    drive_helpers::{
        has_feature_gated_anonymous_user_reached_env_var_limit,
        has_feature_gated_anonymous_user_reached_notebook_limit,
        has_feature_gated_anonymous_user_reached_workflow_limit,
    },
    empty_trash_confirmation_dialog::{EmptyTrashConfirmationDialog, EmptyTrashConfirmationEvent},
    folders::CloudFolder,
    items::{
        ai_fact_collection::WarpDriveAIFactCollection,
        item::{tools_panel_menu_direction, ItemStates, WarpDriveRow},
        mcp_server_collection::WarpDriveMCPServerCollection,
        WarpDriveItemId,
    },
    settings::WarpDriveSettings,
    sharing::{
        dialog::{SharingDialog, SharingDialogEvent},
        ContentEditability, ShareableObject,
    },
    CloudObjectTypeAndId, DriveObjectType, DriveSortOrder,
};
use crate::drive::panel::DrivePanelAction;
use crate::server::cloud_objects::update_manager::InitiatedBy;
use futures::Future;
use itertools::Itertools;
use pathfinder_color::ColorU;
use pathfinder_geometry::vector::{vec2f, Vector2F};
use std::{any::Any, collections::HashMap, sync::Arc};
use url::Url;
use warp_core::{context_flag::ContextFlag, settings::Setting, ui::theme::color::internal_colors};
use warp_util::sync::Condition;
use warpui::{
    clipboard::ClipboardContent,
    elements::{
        Align, AnchorPair, Border, ChildAnchor, ChildView, ClippedScrollStateHandle,
        ClippedScrollable, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Dash,
        DropTarget, DropTargetData, Empty, Flex, Highlight, Hoverable, MainAxisAlignment,
        MainAxisSize, MouseStateHandle, OffsetPositioning, OffsetType, ParentAnchor, ParentElement,
        ParentOffsetBounds, PositionedElementAnchor, PositionedElementOffsetBounds,
        PositioningAxis, Radius, SavePosition, ScrollTarget, ScrollToPositionMode, ScrollbarWidth,
        Shrinkable, Stack, Text, XAxisAnchor, YAxisAnchor,
    },
    fonts::{Properties, Weight},
    keymap::FixedBinding,
    platform::{Cursor, OperatingSystem},
    ui_components::{
        button::ButtonVariant,
        components::{Coords, UiComponent, UiComponentStyles},
    },
    units::IntoPixels,
    AppContext, BlurContext, Element, Entity, ModelHandle, SingletonEntity, TypedActionView,
    UpdateView, View, ViewContext, ViewHandle, WindowId,
};

const WARP_DRIVE_TITLE: &str = "Warp Drive";

// Team zero state consts
const HINT_HORIZONTAL_PADDING: f32 = 18.;
const ITEM_INTERNAL_PADDING: f32 = 4.;
const HINT_VERTICAL_PADDING: f32 = 7.;
const HINT_TEXT_PADDING: f32 = 12.;

const SCROLLBAR_WIDTH: ScrollbarWidth = ScrollbarWidth::Auto;
const SECTION_HEADER_FONT_SIZE: f32 = 12.;
const TEAM_SECTIONS_TITLE_FONT_SIZE: f32 = 12.;
pub const ITEM_FONT_SIZE: f32 = 14.;
const TITLE_FONT_SIZE: f32 = 16.;
const WARNING_FONT_SIZE: f32 = 12.;
// Vertically centers the text in the header since the icon is larger than the
// header.
const HEADER_TEXT_TOP_AND_BOTTOM_MARGIN: f32 = 4.;
// The indent from the edge of the screen to the headers and general index.
pub const INDEX_CONTENT_MARGIN_LEFT: f32 = 12.;
pub const INDEX_CONTENT_MARGIN_RIGHT: f32 = 4.;
// Padding within each item, between the end of its content and the edge of the hoverable area.
// This is set to be right aligned w the x of the left panel
pub const INDEX_CONTENT_PADDING_RIGHT: f32 = 6.5;
pub const TITLE_CONTENT_PADDING_RIGHT: f32 = 8.;

// Item padding to match File Tree styling
pub const ITEM_PADDING_VERTICAL: f32 = 4.;
pub const ITEM_PADDING_HORIZONTAL: f32 = 8.;
pub const FOLDER_DEPTH_INDENT: f32 = 16.;

// Spacing between individual item rows
pub const ITEM_MARGIN_BOTTOM: f32 = 2.;
pub const SECTION_HEADER_MARGIN_BOTTOM: f32 = 2.;

const TAB_BAR_AND_CONTENT_MARGIN: f32 = 6.;

const PADDING_BETWEEN_SPACES: f32 = 8.;
const MARGIN_BETWEEN_HEADER_AND_ICON: f32 = 4.;
const SECTION_HEADER_CONTENT_HEIGHT: f32 = 32.;

const CLOUD_OBJECT_DIALOG_WIDTH: f32 = 400.;
const DIALOG_OFFSET_PIXELS: f32 = -16.;

const HOVER_PREVIEW_X_OFFSET: f32 = 4.;
const HOVER_PREVIEW_Y_OFFSET: f32 = 0.;

const CREATE_TEAM_ICON_WIDTH: f32 = 16.;
const CREATE_TEAM_ICON_HEIGHT: f32 = 16.;
const CREATE_TEAM_TEXT: &str = "Share commands & knowledge with your teammates.";

const LOADING_ICON_WIDTH: f32 = 16.;
const LOADING_ICON_HEIGHT: f32 = 16.;
const MENU_WIDTH: f32 = 194.;
const CLOUD_OFFLINE_ICON_WIDTH: f32 = 20.;
const CLOUD_OFFLINE_ICON_HEIGHT: f32 = 18.;
const OFFLINE_BANNER_ICON_SPACING: f32 = 8.;
const OFFLINE_BANNER_PADDING_HORIZONTAL: f32 = 16.;
const OFFLINE_BANNER_PADDING_VERTICAL: f32 = 4.;

const FOLDER_LABEL: &str = "Folder";
const NOTEBOOK_LABEL: &str = "Notebook";
const WORKFLOW_LABEL: &str = "Workflow";
const AGENT_MODE_WORKFLOW_LABEL: &str = "Prompt";
const ENV_VAR_COLLECTION_LABEL: &str = "Environment variables";
const INDEX_FOLDER_LABEL: &str = "New folder";
const INDEX_NOTEBOOK_LABEL: &str = "New notebook";
const INDEX_WORKFLOW_LABEL: &str = "New workflow";
const INDEX_AGENT_MODE_WORKFLOW_LABEL: &str = "New prompt";
const INDEX_ENV_VAR_COLLECTION_LABEL: &str = "New environment variables";

const IMPORT_LABEL: &str = "Import";
const REMOVE_LABEL: &str = "Remove";
const OFFLINE_BANNER_TEXT: &str = "You are offline. Some files will be read only.";

pub const DRIVE_INDEX_VIEW_POSITION_ID: &str = "drive_index_view_id";

// Sets the speed of the autoscroll that occurs when you drag an item near the Warp Drive border.
pub const AUTOSCROLL_SPEED_MULTIPLIER: f32 = 10.;
// Sets the distance from a border at which scroll events start to occur.
pub const AUTOSCROLL_DETECTION_DISTANCE: f32 = 30.0;

const ZERO_STATE_WORKFLOW_LABEL: &str = "Workflow";
const ZERO_STATE_NOTEBOOK_LABEL: &str = "Notebook";

const SORTING_BUTTON_TOOLTIP_LABEL: &str = "Sort by";

const RETRY_BUTTON_TOOLTIP_LABEL: &str = "Retry sync";

const SHARED_OBJECT_LIMIT_HIT_BANNER_LINE: &str =
    "Upgrade for access to more notebooks, workflows, shared sessions, and AI credits.";

const PAYMENT_ISSUE_BANNER_LINE_1: &str =
    "Shared objects have been restricted due to a subscription payment issue.";

const PAYMENT_ISSUE_BANNER_LINE_2_ADMIN: &str =
    "Please update your payment information to restore access.";

const PAYMENT_ISSUE_BANNER_LINE_2_ADMIN_ENTERPRISE: &str =
    "Please contact support@warp.dev to restore access.";

const PAYMENT_ISSUE_BANNER_LINE_2_NONADMIN: &str = "Please contact a team admin to restore access.";

/// Struct to hold different state-related information on per-space basis.
/// Currently, we only have 1 space (1 Team), but as we're working on personal space, and add
/// multiple teams option, we can use this struct to hold states (like mouse, menu open) for each
/// of those for things like 'add' buttons etc.
#[derive(Clone, Default)]
struct DriveIndexSectionState {
    menu_open: bool,
    collapsed: bool,
    header_hover_state: MouseStateHandle,
    collapsible_hover_state: MouseStateHandle,
    create_menu_mouse_state_handle: MouseStateHandle,
    add_teammates_mouse_state: MouseStateHandle,
    empty_trash_mouse_state: MouseStateHandle,
}

impl DropTargetData for CloudObjectLocation {
    fn as_any(&self) -> &dyn Any {
        self
    }
}

struct RenderedWarpDriveItemAndChildren {
    element: Box<dyn Element>,
    num_items: usize, // represents the total number of elements, including the parent and any children
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum DriveIndexSection {
    Space(Space),
    CreateATeam,
    JoinTeam,
}

#[derive(Debug, Clone)]
pub enum DriveIndexAction {
    OpenObject(CloudObjectTypeAndId),
    OpenWorkflowInPane {
        cloud_object_type_and_id: CloudObjectTypeAndId,
        open_mode: WorkflowViewMode,
    },
    OpenImportModal {
        space: Space,
        initial_folder_id: Option<SyncId>,
    },
    OpenAIFactCollection,
    OpenMCPServerCollection,
    CreateObject {
        object_type: DriveObjectType,
        space: Space,
        initial_folder_id: Option<SyncId>,
    },
    /// Create a workflow with pre-populated content (e.g. saved from a conversation prompt)
    CreateWorkflowWithContent {
        space: Space,
        initial_folder_id: Option<SyncId>,
        content: String,
        is_for_agent_mode: bool,
    },
    CopyObjectToClipboard(CloudObjectTypeAndId),
    CopyWorkflowId(CloudObjectTypeAndId),
    DuplicateObject(CloudObjectTypeAndId),
    CopyObjectLinkToClipboard(String),
    OpenObjectLinkOnDesktop(Url),
    ExportObject(CloudObjectTypeAndId),
    ToggleNewAssetsMenu(Space),
    ToggleSortingMenu,
    ToggleItemOverflowMenu {
        space: Space,
        warp_drive_item_id: WarpDriveItemId,
    },
    ToggleShareDialog {
        warp_drive_item_id: WarpDriveItemId,
    },
    ToggleSpaceOverflowMenu {
        space: Space,
        offset: Vector2F,
    },
    MoveObject {
        cloud_object_type_and_id: CloudObjectTypeAndId,
        new_space: Space,
    },
    LeaveSharedObject {
        cloud_object_type_and_id: CloudObjectTypeAndId,
    },
    OpenCloudObjectNamingDialog {
        space: Space,
        object_type: DriveObjectType,
        // only present when renaming an existing item
        cloud_object_type_and_id: Option<CloudObjectTypeAndId>,
        initial_folder_id: Option<SyncId>,
    },
    CloseCloudObjectNamingDialog,
    DropIndexItem {
        cloud_object_type_and_id: CloudObjectTypeAndId,
        drop_target_location: CloudObjectLocation,
    },
    UpdateCurrentDropTarget {
        drop_target_location: CloudObjectLocation,
    },
    ClearDropTarget,
    ToggleSectionCollapsed(DriveIndexSection),
    OpenTeamSettingsPage,
    RunObject(CloudObjectTypeAndId),
    OpenWorkflowModalWithNew {
        space: Space,
        initial_folder_id: Option<SyncId>,
    },
    OpenWorkflowModalWithCloudWorkflow(SyncId),
    ToggleFolderOpen(SyncId),
    CollapseAllInLocation(CloudObjectLocation),
    InvokeEnvVarCollectionInSubshell(CloudObjectTypeAndId),
    TrashObject {
        cloud_object_type_and_id: CloudObjectTypeAndId,
    },
    UntrashObject {
        cloud_object_type_and_id: CloudObjectTypeAndId,
    },
    DeleteObject {
        cloud_object_type_and_id: CloudObjectTypeAndId,
    },
    EmptyTrash {
        space: Space,
    },
    OpenEmptyTrashConfirmationDialog {
        space: Space,
    },
    Autoscroll {
        delta: f32,
    },
    RenameFolder {
        folder_id: SyncId,
    },
    UpdateSortingChoice {
        sorting_choice: DriveSortOrder,
    },
    RetryFailedObject(CloudObjectTypeAndId),
    RetryAllFailedObjects,
    RevertFailedObject(ServerId),
    OpenTrashIndex,
    CloseTrashIndex,
    FocusPreviousItem,
    FocusNextItem,
    /// Hitting one of the l/r arrow keys on a Warp Drive item.
    LeftArrowKey,
    RightArrowKey,
    /// Hitting enter key on a Warp Drive item.
    EnterKey,
    /// Hitting escape key from trash index returns to main drive index.
    EscapeKey,
    /// Hitting cmd+enter on a WD item toggles the context menu.
    ToggleDriveItemContextMenu,
    ViewPlans {
        team_uid: ServerId,
    },
    ManageBilling {
        team_uid: ServerId,
    },
    SignupAnonymousUser,
    DismissPersonalObjectLimits,
    SetCurrentWorkspace(WorkspaceUid),
    AttachPlanAsContext(AIDocumentId),
}

impl DriveIndexAction {
    pub fn create_object(
        object_type: DriveObjectType,
        space: Space,
        initial_folder_id: Option<SyncId>,
    ) -> Self {
        match (space, object_type) {
            // creating a folder requires a name, which is entered from the cloud object dialog
            (_, DriveObjectType::Folder) => DriveIndexAction::OpenCloudObjectNamingDialog {
                object_type,
                space,
                cloud_object_type_and_id: None,
                initial_folder_id,
            },
            (
                _,
                DriveObjectType::Notebook { .. }
                | DriveObjectType::EnvVarCollection
                | DriveObjectType::Workflow
                | DriveObjectType::AgentModeWorkflow
                | DriveObjectType::AIFactCollection
                | DriveObjectType::AIFact
                | DriveObjectType::MCPServer
                | DriveObjectType::MCPServerCollection,
            ) => DriveIndexAction::CreateObject {
                object_type,
                space,
                initial_folder_id,
            },
        }
    }

    pub fn blocked_for_anonymous_user(&self) -> bool {
        use DriveIndexAction::*;
        matches!(
            self,
            OpenTeamSettingsPage | ViewPlans { .. } | ManageBilling { .. }
        )
    }
}

impl From<&DriveIndexAction> for LoginGatedFeature {
    fn from(val: &DriveIndexAction) -> LoginGatedFeature {
        use DriveIndexAction::*;
        match val {
            OpenTeamSettingsPage => "Open Team Settings",
            ViewPlans { .. } => "View Plans",
            ManageBilling { .. } => "Manage Billing",
            _ => "Unknown reason",
        }
    }
}

pub enum DriveIndexEvent {
    CreateNotebook {
        space: Space,
        title: Option<String>,
        initial_folder_id: Option<SyncId>,
    },
    CreateFolder {
        space: Space,
        title: String,
        initial_folder_id: Option<SyncId>,
    },
    CreateEnvVarCollection {
        space: Space,
        title: Option<String>,
        initial_folder_id: Option<SyncId>,
    },
    CreateWorkflow {
        space: Space,
        title: Option<String>,
        initial_folder_id: Option<SyncId>,
        is_for_agent_mode: bool,
        /// Pre-populated content for the workflow (e.g. saved from a conversation prompt)
        content: Option<String>,
    },
    CreateAIFact {
        space: Space,
        fact: AIFact,
        initial_folder_id: Option<SyncId>,
    },
    OpenAIFactCollection,
    OpenMCPServerCollection,
    OpenObject(CloudObjectTypeAndId),
    OpenWorkflowInPane {
        cloud_object_type_and_id: CloudObjectTypeAndId,
        open_mode: WorkflowViewMode,
    },
    DuplicateObject(CloudObjectTypeAndId),
    ExportObject(CloudObjectTypeAndId),
    OpenTeamSettingsPage,
    OpenImportModal {
        space: Space,
        initial_folder_id: Option<SyncId>,
    },
    RunObject(CloudObjectTypeAndId),
    InvokeEnvVarCollectionInSubshell(CloudObjectTypeAndId),
    OpenWorkflowModalWithNew {
        space: Space,
        initial_folder_id: Option<SyncId>,
    },
    OpenWorkflowModalWithCloudWorkflow(SyncId),
    FocusWarpDrive,
    OpenSharedObjectsCreationDeniedModal(DriveObjectType, ServerId),
    AttachPlanAsContext(AIDocumentId),
}

#[derive(Clone, Default)]
struct MouseStateHandles {
    warp_drive_initial_load_mouse_state: MouseStateHandle,
    sorting_button_mouse_state: MouseStateHandle,
    retry_button_mouse_state: MouseStateHandle,
    trash_row_mouse_state: MouseStateHandle,
    exit_trash_button_mouse_state: MouseStateHandle,
    join_team_button_mouse_state: MouseStateHandle,
    create_team_button_mouse_state: MouseStateHandle,
    shared_object_limit_hit_banner_button_mouse_state: MouseStateHandle,
    payment_issue_banner_button_mouse_state: MouseStateHandle,
    anonymous_sign_up_button_mouse_state: MouseStateHandle,
    anonymous_object_limit_close_button_mouse_state: MouseStateHandle,
    search_button_mouse_state: MouseStateHandle,
}

#[derive(Copy, Clone, PartialEq)]
pub enum DriveIndexVariant {
    MainIndex,
    Trash,
}

#[derive(Clone)]
struct SpaceMenuState {
    space: Space,
    offset: Vector2F,
}

/// The main view for the Warp Drive sidebar.
/// `DriveIndex` is different from `DrivePanel` in that it is responsible for
/// all the logic within Warp Drive, whereas `DrivePanel` is responsible for
/// how Warp Drive interacts with the workspace and the rest of the app.
#[derive(Clone)]
pub struct DriveIndex {
    window_id: WindowId,
    /// Menu view, can be used for any right click / menu operation (doesn't have menu options by
    /// default, should get the menu fields on open, example: + button to add notebook)
    menu: ViewHandle<Menu<DriveIndexAction>>,

    sharing_dialog: ViewHandle<SharingDialog>,
    /// Variant of the index, determines whether base Warp Drive or trash is viewed.
    index_variant: DriveIndexVariant,
    /// If None, the context menu is closed. Otherwise, this contains the ID of the object it's open on.
    menu_object_id_if_open: Option<WarpDriveItemId>,
    /// If Some, the share dialog is open for the given object.
    share_dialog_open_for_object: Option<WarpDriveItemId>,
    sections: Vec<DriveIndexSection>,
    /// Selected represents an object that is open in the active pane
    selected: Option<WarpDriveItemId>,
    /// The numerical index of the item that is focused in WD (via keyboard)
    focused_index: Option<usize>,
    item_mouse_states: HashMap<Space, Vec<ItemStates>>,
    section_states: HashMap<DriveIndexSection, DriveIndexSectionState>,
    clipped_scroll_state: ClippedScrollStateHandle,
    mouse_state_handles: MouseStateHandles,
    cloud_object_naming_dialog: CloudObjectNamingDialog,
    current_drop_target: Option<CloudObjectLocation>,
    empty_trash_confirmation_dialog: ViewHandle<EmptyTrashConfirmationDialog>,
    empty_trash_confirmation_dialog_space: Option<Space>,
    sorting_button_menu_open: bool,
    sorting_choice: DriveSortOrder,
    auth_state: Arc<AuthState>,
    space_menu_open_for_space: Option<SpaceMenuState>,
    show_warp_drive_loading_icon: bool,
    should_show_personal_object_limit_status: bool,
    /// A hashmap of location (space/folder) to a list of hashed IDs of objects inside
    /// the space/folder, used for rendering our objects
    sorted_orders_by_location: HashMap<CloudObjectLocation, Vec<ObjectUid>>,
    /// A sorted list of all the items (spaces + objects) in Warp Drive
    /// Unlike sorted_orders_by_location, this is not used for rendering
    /// This is used for object focusing and WD keyboard navigation
    ordered_items: Vec<WarpDriveItemId>,

    /// Whether or not we have done an initial setting of all the section states.
    /// We need to keep track of this to make sure we don't do any opening actions on WD
    /// from links before everything has been set up.
    has_initialized_sections: Condition,

    /// The number of objects in Warp Drive that have errored.
    /// This value is cached so that we can determine whether to render the "retry all"
    /// objects button in the case of syncing failures.
    num_errored_objects: usize,

    workspace_dropdown: ViewHandle<Dropdown<DriveIndexAction>>,

    /// Drive item to represent collection of AI facts.
    /// Special-cased to always render at the top of the Personal space section.
    ai_fact_collection: WarpDriveAIFactCollection,
    ai_fact_collection_item_mouse_states: ItemStates,

    /// Drive item to represent collection of MCP servers.
    /// Special-cased to always render at the top of the Personal space section.
    mcp_server_collection: WarpDriveMCPServerCollection,
    mcp_server_collection_item_mouse_states: ItemStates,
}

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    app.register_fixed_bindings(vec![
        FixedBinding::new("up", DriveIndexAction::FocusPreviousItem, id!("DriveIndex")),
        FixedBinding::new("down", DriveIndexAction::FocusNextItem, id!("DriveIndex")),
        FixedBinding::new(
            "j",
            DriveIndexAction::FocusNextItem,
            id!("DriveIndex") & !id!("DisableDriveIndexVimKeybindings"),
        ),
        FixedBinding::new(
            "k",
            DriveIndexAction::FocusPreviousItem,
            id!("DriveIndex") & !id!("DisableDriveIndexVimKeybindings"),
        ),
        FixedBinding::new("left", DriveIndexAction::LeftArrowKey, id!("DriveIndex")),
        FixedBinding::new("right", DriveIndexAction::RightArrowKey, id!("DriveIndex")),
        FixedBinding::new("enter", DriveIndexAction::EnterKey, id!("DriveIndex")),
        FixedBinding::new("escape", DriveIndexAction::EscapeKey, id!("DriveIndex")),
        FixedBinding::new(
            "cmdorctrl-enter",
            DriveIndexAction::ToggleDriveItemContextMenu,
            id!("DriveIndex"),
        ),
    ]);
}

impl DriveIndex {
    // Called whenever cloud model or user workspaces change.
    pub fn initialize_section_states(&mut self, ctx: &mut ViewContext<Self>) {
        let user_workspaces = UserWorkspaces::handle(ctx);

        self.workspace_dropdown.update(ctx, |dropdown, ctx| {
            let workspaces = user_workspaces.as_ref(ctx).workspaces();
            let selected_index =
                if let Some(current_workspace) = user_workspaces.as_ref(ctx).current_workspace() {
                    workspaces
                        .iter()
                        .position(|workspace| workspace.uid == current_workspace.uid)
                        .unwrap_or_else(|| {
                            log::error!("Could not find current workspace in dropdown option list");
                            0
                        })
                } else {
                    0
                };
            dropdown.set_items(
                workspaces
                    .iter()
                    .map(|workspace| {
                        DropdownItem::new(
                            workspace.name.clone(),
                            DriveIndexAction::SetCurrentWorkspace(workspace.uid),
                        )
                    })
                    .collect(),
                ctx,
            );
            dropdown.set_selected_by_index(selected_index, ctx);
        });

        let cloud_model = CloudModel::handle(ctx);

        let spaces = user_workspaces.update(ctx, |user_workspaces, ctx| {
            user_workspaces.all_user_spaces(ctx)
        });
        let num_cloud_objects_per_space = match self.index_variant {
            DriveIndexVariant::MainIndex => cloud_model
                .as_ref(ctx)
                .num_active_cloud_objects_per_space(spaces.iter(), ctx),
            DriveIndexVariant::Trash => cloud_model
                .as_ref(ctx)
                .num_trashed_cloud_objects_per_space(spaces.iter(), ctx),
        };
        let mut sections = spaces
            .iter()
            .map(|space| DriveIndexSection::Space(*space))
            .collect::<Vec<_>>();

        if !user_workspaces.as_ref(ctx).has_teams() {
            if user_workspaces
                .as_ref(ctx)
                .total_teammates_in_joinable_teams()
                > 0
            {
                sections.insert(0, DriveIndexSection::JoinTeam);
                sections.insert(1, DriveIndexSection::CreateATeam);
            } else {
                sections.insert(0, DriveIndexSection::CreateATeam);
            }
        }

        // Item UI state is attached by index, not by id, so this is re-initialized whenever there's any type of change
        let item_mouse_states = num_cloud_objects_per_space
            .iter()
            .map(|(space, num_cloud_objects)| {
                (
                    *space,
                    (0..*num_cloud_objects)
                        .map(|_| Default::default())
                        .collect(),
                )
            })
            .collect::<HashMap<_, Vec<ItemStates>>>();

        // Space header UI state is keyed by the space. Ideally, this persists between changes so the space doesn't collapse and un-collapse
        // as changes are received.
        let section_states = sections
            .iter()
            .map(|section| {
                if let Some(old_state) = self.section_states.get(section) {
                    (*section, old_state.clone())
                } else {
                    (*section, Default::default())
                }
            })
            .collect();

        self.sections = sections;
        self.section_states = section_states;
        self.item_mouse_states = item_mouse_states;

        // Re-sort the cloud objects in each space and store them for rendering.
        self.sorted_orders_by_location.clear();
        spaces.iter().for_each(|space| {
            self.sort_location(
                CloudObjectLocation::Space(*space),
                cloud_model.as_ref(ctx),
                CloudViewModel::as_ref(ctx),
                ctx,
            );
        });

        // Set item focusing parameters (ordered_items and focused_index) at initialization
        // If an item is already focused, retrieve focused item ID then re-sort ordered_items
        // Otherwise, re-sort ordered_items to ensure it is always accurate after a cloudmodel change
        if self.ordered_items.is_empty() {
            self.compute_ordered_items(cloud_model.as_ref(ctx));
            self.focused_index = Some(0);
        } else if let Some(focused_index) = self.focused_index {
            self.update_focused_params(focused_index, cloud_model.as_ref(ctx));
        } else {
            self.compute_ordered_items(cloud_model.as_ref(ctx));
        }
    }

    pub fn has_initialized_sections(&self) -> impl Future<Output = ()> {
        // We're not using `async fn` here so that the returned Future doesn't borrow self.
        self.has_initialized_sections.wait()
    }

    /// Recursively sorts the objects within a CloudObjectLocation and its children, storing the sorted list
    /// of HashedObjectId's in the DriveIndex's sorted_orders_by_location.
    fn sort_location(
        &mut self,
        location: CloudObjectLocation,
        cloud_model: &CloudModel,
        cloud_view_model: &CloudViewModel,
        app: &AppContext,
    ) {
        let item_iter = match (self.index_variant, location) {
            (DriveIndexVariant::MainIndex, CloudObjectLocation::Space(Space::Shared)) => {
                let user_uid = AuthStateProvider::as_ref(app).get().user_id();
                cloud_model
                    .active_cloud_objects_in_location_without_descendents(location, app)
                    .filter(move |cloud_object| {
                        cloud_object.renders_in_warp_drive()
                            && user_uid.is_some_and(|uid| {
                                cloud_object.permissions().has_direct_user_access(uid)
                            })
                    })
                    .sorted_by(self.sorting_choice.sort_by(
                        cloud_view_model,
                        UpdateTimestamp::Revision,
                        app,
                    ))
            }
            (DriveIndexVariant::MainIndex, _) => cloud_model
                .active_cloud_objects_in_location_without_descendents(location, app)
                .filter(|cloud_object| cloud_object.renders_in_warp_drive())
                .sorted_by(self.sorting_choice.sort_by(
                    cloud_view_model,
                    UpdateTimestamp::Revision,
                    app,
                )),
            (DriveIndexVariant::Trash, CloudObjectLocation::Space(space)) => cloud_model
                .directly_trashed_cloud_objects_in_space(space, app)
                .sorted_by(self.sorting_choice.sort_by(
                    cloud_view_model,
                    UpdateTimestamp::Trashed,
                    app,
                )),
            (DriveIndexVariant::Trash, _) => cloud_model
                .indirectly_trashed_cloud_objects_in_location_without_descendents(location, app)
                .sorted_by(self.sorting_choice.sort_by(
                    cloud_view_model,
                    UpdateTimestamp::Trashed,
                    app,
                )),
        };

        let mut items = vec![];
        // Add the AI fact collection object + MCP server collection object for personal space
        if matches!(location, CloudObjectLocation::Space(Space::Personal)) {
            items.push(self.mcp_server_collection.id().to_string());
            items.push(self.ai_fact_collection.id().to_string());
        }

        items.extend(
            item_iter
                .map(|object| {
                    if object.object_type() == ObjectType::Folder {
                        let folder: Option<&CloudFolder> = object.into();
                        if let Some(folder) = folder {
                            if folder.model().is_open {
                                self.sort_location(
                                    CloudObjectLocation::Folder(folder.id),
                                    cloud_model,
                                    cloud_view_model,
                                    app,
                                )
                            }
                        }
                    }
                    object.uid()
                })
                .collect_vec(),
        );

        self.sorted_orders_by_location.insert(location, items);
    }

    /// Recursively populates and sorts the ordered_items list used for WD keyboard navigation
    fn sort_ordered_items(&mut self, uids: Vec<String>, cloud_model: &CloudModel) {
        for uid in uids {
            if let Some(cloud_object) = cloud_model.get_by_uid(&uid) {
                // Add object to the list
                let cloud_id = cloud_object.cloud_object_type_and_id();
                self.ordered_items.push(WarpDriveItemId::Object(cloud_id));
                // If the item is a folder and the folder is open, recurse
                if let CloudObjectTypeAndId::Folder(folder_id) = cloud_id {
                    if self
                        .sorted_orders_by_location
                        .contains_key(&CloudObjectLocation::Folder(folder_id))
                    {
                        self.sort_ordered_items(
                            self.sorted_orders_by_location[&CloudObjectLocation::Folder(folder_id)]
                                .clone(),
                            cloud_model,
                        )
                    }
                }
            }
        }
    }

    /// Sets the ordered_items vector used for WD keyboard navigation
    fn compute_ordered_items(&mut self, cloud_model: &CloudModel) {
        self.ordered_items.clear();
        for section in self.sections.clone() {
            if let DriveIndexSection::Space(space) = section {
                // Add space to the list
                self.ordered_items.push(WarpDriveItemId::Space(space));
                // If the space is not collapsed, iterate through the items in the space
                if let Some(section_state) = self
                    .section_states
                    .get_mut(&DriveIndexSection::Space(space))
                {
                    if !section_state.collapsed {
                        // Add AI fact collection object + MCP server collection object for personal space
                        if matches!(space, Space::Personal) {
                            if FeatureFlag::McpServer.is_enabled()
                                && ContextFlag::ShowMCPServers.is_enabled()
                            {
                                self.ordered_items
                                    .push(WarpDriveItemId::MCPServerCollection);
                            }
                            self.ordered_items.push(WarpDriveItemId::AIFactCollection);
                        }
                        // Sort and add the rest of the items in the space
                        let Some(uids) = self
                            .sorted_orders_by_location
                            .get(&CloudObjectLocation::Space(space))
                        else {
                            return;
                        };
                        self.sort_ordered_items(uids.to_vec(), cloud_model);
                    }
                }
            }
        }
        if self.index_variant == DriveIndexVariant::MainIndex {
            self.ordered_items.push(WarpDriveItemId::Trash);
        }
    }

    /// Updates both ordered_items and focused_index, the parameters used for WD keyboard navigation
    fn update_focused_params(&mut self, focused_index: usize, cloud_model: &CloudModel) {
        // Error check to make sure indexing into ordered_items will be valid
        if focused_index < self.ordered_items.len() {
            // Retrieve the focused item ID, then re-sort ordered_items
            self.compute_ordered_items(cloud_model);
            let Some(focused_item_id) = self.ordered_items.get(focused_index) else {
                return;
            };
            // Update focused_index after the re-sort
            for (i, id) in self.ordered_items.iter().enumerate() {
                if *id == *focused_item_id {
                    self.focused_index = Some(i);
                    break;
                }
            }
        }
    }

    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let cloud_model = CloudModel::handle(ctx);
        ctx.observe(&cloud_model, Self::on_cloud_model_changed);

        let object_actions = ObjectActions::handle(ctx);
        ctx.observe(&object_actions, Self::on_object_actions_changed);

        ctx.subscribe_to_model(&cloud_model, |index, _, event, ctx| {
            index.handle_cloud_model_event(event, ctx);
        });

        ctx.subscribe_to_model(
            &CloudViewModel::handle(ctx),
            Self::handle_cloud_view_model_event,
        );

        let user_workspaces = UserWorkspaces::handle(ctx);
        ctx.observe(&user_workspaces, Self::on_user_workspaces_changed);

        let network_status = NetworkStatus::handle(ctx);
        ctx.subscribe_to_model(&network_status, |_me, _, _event, ctx| {
            ctx.notify();
        });

        let menu = ctx.add_typed_action_view(|_ctx| {
            Menu::new()
                .prevent_interaction_with_other_elements()
                .with_drop_shadow()
                .with_width(MENU_WIDTH)
        });

        ctx.subscribe_to_view(&menu, |me, _, event, ctx| {
            me.handle_menu_event(event, ctx);
        });

        let title_editor = ctx.add_typed_action_view(|ctx| {
            let options = SingleLineEditorOptions {
                propagate_and_no_op_vertical_navigation_keys:
                    crate::editor::PropagateAndNoOpNavigationKeys::Always,
                ..Default::default()
            };
            let mut editor = EditorView::single_line(options, ctx);
            editor.set_placeholder_text("Untitled", ctx);
            editor
        });

        ctx.subscribe_to_view(&title_editor, |me, _, event, ctx| {
            me.handle_title_editor_event(event, ctx);
        });

        let empty_trash_confirmation_dialog =
            ctx.add_typed_action_view(|_| EmptyTrashConfirmationDialog::new());
        ctx.subscribe_to_view(&empty_trash_confirmation_dialog, |me, _, event, ctx| {
            me.handle_empty_trash_confirmation_dialog_event(event, ctx);
        });

        let sorting_choice = *WarpDriveSettings::as_ref(ctx).sorting_choice.value();

        // Hide Warp Drive loading icon once initial load is complete
        let initial_load_complete = UpdateManager::as_ref(ctx).initial_load_complete();
        ctx.spawn(initial_load_complete, |me, _, ctx| {
            me.show_warp_drive_loading_icon = false;
            me.initialize_section_states(ctx);
            me.has_initialized_sections.set();
            ctx.notify();
        });

        let sharing_dialog = ctx.add_typed_action_view(|ctx| SharingDialog::new(None, ctx));
        ctx.subscribe_to_view(&sharing_dialog, |me, _, event, ctx| {
            me.handle_sharing_dialog_event(event, ctx);
        });

        let workspace_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);
            dropdown.set_top_bar_max_width(400.);
            dropdown.set_menu_width(225., ctx);

            let workspaces = user_workspaces.as_ref(ctx).workspaces();

            let selected_index =
                if let Some(current_workspace) = user_workspaces.as_ref(ctx).current_workspace() {
                    workspaces
                        .iter()
                        .position(|workspace| workspace.uid == current_workspace.uid)
                        .unwrap_or_else(|| {
                            log::error!("Could not find current workspace in dropdown option list");
                            0
                        })
                } else {
                    0
                };

            dropdown.add_items(
                workspaces
                    .iter()
                    .map(|workspace| {
                        DropdownItem::new(
                            workspace.name.clone(),
                            DriveIndexAction::SetCurrentWorkspace(workspace.uid),
                        )
                    })
                    .collect(),
                ctx,
            );
            dropdown.set_selected_by_index(selected_index, ctx);

            dropdown
        });

        let ai_fact_collection = WarpDriveAIFactCollection::new(ClientId::default());
        let mcp_server_collection = WarpDriveMCPServerCollection::new(ClientId::default());

        Self {
            window_id: ctx.window_id(),
            menu,
            sharing_dialog,
            index_variant: DriveIndexVariant::MainIndex,
            menu_object_id_if_open: None,
            sections: Default::default(),
            selected: None,
            focused_index: None,
            item_mouse_states: Default::default(),
            section_states: Default::default(),
            clipped_scroll_state: Default::default(),
            mouse_state_handles: Default::default(),
            cloud_object_naming_dialog: CloudObjectNamingDialog::new(title_editor),
            current_drop_target: None,
            empty_trash_confirmation_dialog,
            empty_trash_confirmation_dialog_space: None,
            sorting_button_menu_open: false,
            sorting_choice,
            auth_state: AuthStateProvider::as_ref(ctx).get().clone(),
            space_menu_open_for_space: None,
            show_warp_drive_loading_icon: true,
            sorted_orders_by_location: Default::default(),
            ordered_items: Default::default(),
            has_initialized_sections: Default::default(),
            num_errored_objects: Default::default(),
            share_dialog_open_for_object: None,
            should_show_personal_object_limit_status: true,
            workspace_dropdown,
            ai_fact_collection,
            ai_fact_collection_item_mouse_states: Default::default(),
            mcp_server_collection,
            mcp_server_collection_item_mouse_states: Default::default(),
        }
    }

    fn edit_object_enabled(
        &self,
        cloud_object_type_and_id: &CloudObjectTypeAndId,
        app: &AppContext,
    ) -> bool {
        self.is_online(app) || !cloud_object_type_and_id.has_server_id()
    }

    fn online_only_operation_allowed(
        &self,
        cloud_object_type_and_id: &CloudObjectTypeAndId,
        app: &AppContext,
    ) -> bool {
        if let Some(object) = CloudModel::as_ref(app).get_by_uid(&cloud_object_type_and_id.uid()) {
            return self.is_online(app)
                && cloud_object_type_and_id.has_server_id()
                && !object.metadata().has_pending_online_only_change();
        }

        false
    }

    fn is_online(&self, app: &AppContext) -> bool {
        NetworkStatus::as_ref(app).is_online()
    }

    pub fn scroll_item_into_view(&mut self, item_id: WarpDriveItemId, ctx: &mut ViewContext<Self>) {
        self.clipped_scroll_state.scroll_to_position(ScrollTarget {
            position_id: item_id.drive_row_position_id(),
            mode: ScrollToPositionMode::FullyIntoView,
        });
        ctx.notify();
    }

    /// Sets focused to the index of either the selected object or the first item in WD
    pub fn reset_focused_index_in_warp_drive(
        &mut self,
        should_scroll: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        if let Some(id) = self.selected {
            // Force expand the item's ancestors and re-render WD initial states
            // Needed for when the selected object is hidden inside collapsed folder(s)

            /* NOTE: Temporary fix for not being able to open folders due to event conflicts.
             * This fixes the above but re-introduces a bug where we don't auto-expand the
             * selected notebook folder in the drive.
             * Follow:
             * https://linear.app/warpdotdev/issue/CLD-1557/unable-to-open-folder-when-selected-notebook-resides-in-closed-folder
             * for a proper fix.
            CloudModel::handle(ctx).update(ctx, |model, ctx| {
                model.force_expand_object_and_ancestors_cloud_id(id, ctx);
            });
            */
            self.initialize_section_states(ctx);
            self.set_focused_item(id, should_scroll, ctx);
        } else {
            self.set_focused_index(Some(0), should_scroll, ctx);
        }
    }

    pub fn set_focused_item(
        &mut self,
        item_id: WarpDriveItemId,
        should_scroll: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        if should_scroll {
            self.scroll_item_into_view(item_id, ctx);
        }
        // Used for error logging only
        let mut found_input_item_id = false;

        // Find the index associated with item_id, then set self.focused_index to that index
        for (i, id) in self.ordered_items.iter().enumerate() {
            if *id == item_id {
                self.focused_index = Some(i);
                found_input_item_id = true;
                break;
            }
        }
        if !found_input_item_id {
            log::warn!("Failed to set focused item: could not find it in ordered_items");
        }
        ctx.notify();
    }

    pub fn set_focused_index(
        &mut self,
        index: Option<usize>,
        should_scroll: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        self.focused_index = index;
        if let Some(index) = index {
            let Some(item_id) = self.ordered_items.get(index) else {
                return;
            };
            if should_scroll {
                self.scroll_item_into_view(*item_id, ctx);
            }
        }
        ctx.notify();
    }

    fn on_user_workspaces_changed(
        &mut self,
        _: ModelHandle<UserWorkspaces>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.initialize_section_states(ctx);
        ctx.notify();
    }

    fn on_cloud_model_changed(
        &mut self,
        cloud_model: ModelHandle<CloudModel>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.initialize_section_states(ctx);
        self.num_errored_objects = cloud_model.as_ref(ctx).num_visible_errored_objects();
        ctx.notify();
    }

    fn on_object_actions_changed(
        &mut self,
        _: ModelHandle<ObjectActions>,
        ctx: &mut ViewContext<Self>,
    ) {
        ctx.notify();
    }

    fn handle_menu_event(&mut self, event: &Event, ctx: &mut ViewContext<Self>) {
        if let Event::Close { via_select_item } = event {
            self.reset_menus(ctx);
            if !*via_select_item {
                ctx.emit(DriveIndexEvent::FocusWarpDrive);
                self.reset_focused_index_in_warp_drive(false, ctx);
            }
        }
    }

    fn handle_sharing_dialog_event(
        &mut self,
        event: &SharingDialogEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            SharingDialogEvent::Close => {
                self.share_dialog_open_for_object = None;
                ctx.notify();
            }
        }
    }

    // Todo: move the title editor into the cloud object naming dialog and make the dialog its own view.
    fn handle_title_editor_event(&mut self, event: &EditorEvent, ctx: &mut ViewContext<Self>) {
        match event {
            EditorEvent::Edited(_) => ctx.notify(),
            EditorEvent::Escape => {
                self.cloud_object_naming_dialog.close(ctx);
                ctx.notify();
            }
            EditorEvent::Enter => {
                if let Some(action) = self.cloud_object_naming_dialog.current_primary_action() {
                    self.handle_action(&action, ctx);
                }
            }
            _ => {}
        }
    }

    fn handle_cloud_model_event(&mut self, event: &CloudModelEvent, ctx: &mut ViewContext<Self>) {
        match event {
            CloudModelEvent::ObjectForceExpanded { id } => {
                self.expand_section_for_object(id, ctx);
            }
            CloudModelEvent::ObjectUpdated { .. }
            | CloudModelEvent::ObjectTrashed { .. }
            | CloudModelEvent::ObjectUntrashed { .. }
            | CloudModelEvent::ObjectCreated { .. }
            | CloudModelEvent::ObjectMoved { .. }
            | CloudModelEvent::ObjectDeleted { .. }
            | CloudModelEvent::ObjectPermissionsUpdated { .. }
            | CloudModelEvent::NotebookEditorChangedFromServer { .. }
            | CloudModelEvent::ObjectSynced { .. }
            | CloudModelEvent::InitialLoadCompleted => {}
        }
    }

    fn handle_cloud_view_model_event(
        &mut self,
        _handle: ModelHandle<CloudViewModel>,
        event: &CloudViewModelEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        let CloudViewModelEvent::SortTimestampsChanged = event;
        self.initialize_section_states(ctx);
        ctx.notify();
    }

    /// Expand the section for warp drive item. This is called when we perform deep link to warp
    /// drive items.
    pub fn expand_section_for_drive_item_id(
        &mut self,
        item_id: WarpDriveItemId,
        ctx: &mut ViewContext<DriveIndex>,
    ) {
        if let WarpDriveItemId::Object(object_id) = item_id {
            match object_id {
                CloudObjectTypeAndId::Notebook(sync_id) => {
                    self.expand_section_for_object(&sync_id.uid().clone(), ctx);
                }
                CloudObjectTypeAndId::Workflow(sync_id) => {
                    self.expand_section_for_object(&sync_id.uid().clone(), ctx);
                }
                CloudObjectTypeAndId::Folder(sync_id) => {
                    self.expand_section_for_object(&sync_id.uid().clone(), ctx);
                }
                CloudObjectTypeAndId::GenericStringObject { object_type, id } => {
                    if let GenericStringObjectFormat::Json(JsonObjectType::EnvVarCollection) =
                        object_type
                    {
                        self.expand_section_for_object(&id.uid().clone(), ctx);
                    } else {
                        log::warn!("unknown GenericStringObject type found while trying to manually expand drive section. {object_id:?}");
                    }
                }
            };
        }
    }

    /// Expand the section that contains an object identified by `id`.
    fn expand_section_for_object(&mut self, id: &ObjectUid, ctx: &mut ViewContext<DriveIndex>) {
        let Some(space) = CloudViewModel::as_ref(ctx).object_space(id, ctx) else {
            return;
        };

        let Some(section_state) = self
            .section_states
            .get_mut(&DriveIndexSection::Space(space))
        else {
            return;
        };
        section_state.collapsed = false;
        ctx.notify();
    }

    fn handle_empty_trash_confirmation_dialog_event(
        &mut self,
        event: &EmptyTrashConfirmationEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            EmptyTrashConfirmationEvent::Cancel => {
                self.empty_trash_confirmation_dialog_space = None;
                ctx.notify();
            }
            EmptyTrashConfirmationEvent::Confirm => {
                if let Some(space) = self.empty_trash_confirmation_dialog_space {
                    self.empty_trash(&space, ctx)
                }
                self.empty_trash_confirmation_dialog_space = None;
            }
        }
    }

    /// Used for 1) create team 2) join discoverable teams sections
    fn render_team_section_header(
        &self,
        text: String,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let icon = Container::new(
            ConstrainedBox::new(
                Icon::CreateTeam
                    .to_warpui_icon(
                        appearance
                            .theme()
                            .main_text_color(appearance.theme().surface_1()),
                    )
                    .finish(),
            )
            .with_width(CREATE_TEAM_ICON_WIDTH)
            .with_height(CREATE_TEAM_ICON_HEIGHT)
            .finish(),
        )
        .with_margin_right(MARGIN_BETWEEN_HEADER_AND_ICON)
        .finish();

        let title_text = Shrinkable::new(
            1.,
            appearance
                .ui_builder()
                .wrappable_text(text, true)
                .with_style(UiComponentStyles {
                    font_family_id: Some(appearance.ui_font_family()),
                    font_size: Some(TEAM_SECTIONS_TITLE_FONT_SIZE),
                    font_weight: Some(Weight::Normal),
                    ..Default::default()
                })
                .build()
                .finish(),
        )
        .finish();

        let title_row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_child(icon)
            .with_child(title_text)
            .finish();

        Container::new(
            Container::new(title_row)
                .with_margin_left(INDEX_CONTENT_MARGIN_LEFT)
                .with_padding_right(INDEX_CONTENT_PADDING_RIGHT)
                .finish(),
        )
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
        .finish()
    }

    fn render_space_section_header(
        &self,
        title: Box<dyn Element>,
        space: &Space,
        section_state: &DriveIndexSectionState,
        section: DriveIndexSection,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let space_clone = *space;
        let title_row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(Shrinkable::new(1., title).finish());

        // This represents the clickable region of the header where any mouse-up action will toggle the collapse boolean.
        let collapsible_icon =
            self.render_collapse_section_icon(section, section_state.collapsed, appearance);
        let header = Hoverable::new(section_state.collapsible_hover_state.clone(), move |_| {
            title_row.with_child(collapsible_icon).finish()
        })
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(DriveIndexAction::ToggleSectionCollapsed(section))
        })
        .on_right_click(move |ctx, _, position| {
            let position_id = &warp_drive_section_header_position_id(&section);
            let Some(prompt_rect) = ctx.element_position_by_id(position_id) else {
                return;
            };
            let offset_position = position - prompt_rect.origin();
            ctx.dispatch_typed_action(DriveIndexAction::ToggleSpaceOverflowMenu {
                space: space_clone,
                offset: offset_position,
            });
        })
        .finish();

        let mut stack = Stack::new();
        stack.add_child(header);
        if let Some(space_menu_state) = &self.space_menu_open_for_space {
            if space.eq(&space_menu_state.space) {
                stack.add_positioned_overlay_child(
                    ChildView::new(&self.menu).finish(),
                    OffsetPositioning::offset_from_save_position_element(
                        warp_drive_section_header_position_id(&section),
                        space_menu_state.offset,
                        PositionedElementOffsetBounds::WindowByPosition,
                        PositionedElementAnchor::TopLeft,
                        ChildAnchor::TopLeft,
                    ),
                );
            }
        }

        // Align items in the header to span the horizontal direction and sit in the vertical
        // center of the row.
        let mut header_row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(Shrinkable::new(1., stack.finish()).finish());

        // The teammates icon that redirects to the team settings page.
        if matches!(section, DriveIndexSection::Space(Space::Team { .. })) && self.is_online(app) {
            if let DriveIndexSection::Space(space) = section {
                let add_teammates_button =
                    self.render_add_teammates_button(appearance, section_state, space);
                header_row.add_child(add_teammates_button)
            }
        }

        // The "+" icon for adding new objects.
        if let DriveIndexSection::Space(space) = section {
            let can_create_objects = match space {
                Space::Personal => true,
                Space::Team { .. } => self.is_online(app),
                Space::Shared => false,
            };
            if can_create_objects {
                let create_object_button =
                    self.render_create_new_button(appearance, space, section_state, app);
                header_row.add_child(create_object_button);
            }
        }

        let mut container = Container::new(
            Container::new(header_row.finish())
                // Indent the header content from within the hoverable
                .with_margin_left(INDEX_CONTENT_MARGIN_LEFT)
                .with_padding_right(INDEX_CONTENT_PADDING_RIGHT)
                .finish(),
        )
        // Add some styling to the entire row: rounded edges and a small margin between the row and the border
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)));

        // If the space is focused, set background
        let mut is_focused = false;
        if let DriveIndexSection::Space(space) = section {
            if let Some(focused_index) = self.focused_index {
                if Some(&WarpDriveItemId::Space(space)) == self.ordered_items.get(focused_index) {
                    container = container.with_background(
                        warp_core::ui::theme::color::internal_colors::fg_overlay_4(
                            appearance.theme(),
                        ),
                    );
                    is_focused = true;
                }
            }
        }
        Container::new(
            Hoverable::new(
                section_state.header_hover_state.clone(),
                move |mouse_state| {
                    // If the item is hovered, set a hover background that matches the hover state of warp drive items.
                    if mouse_state.is_hovered() && !is_focused || section_state.menu_open {
                        container = container.with_background(
                            warp_core::ui::theme::color::internal_colors::fg_overlay_2(
                                appearance.theme(),
                            ),
                        );
                    }

                    container.finish()
                },
            )
            .finish(),
        )
        .with_margin_bottom(SECTION_HEADER_MARGIN_BOTTOM)
        .finish()
    }

    fn render_trash_section_header(
        &self,
        title: Box<dyn Element>,
        space: &Space,
        section_state: &DriveIndexSectionState,
        section: DriveIndexSection,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let space_clone = *space;
        let title_row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(Shrinkable::new(1., title).finish());

        // This represents the clickable region of the header where any mouse-up action will toggle the collapse boolean.
        let collapsible_icon =
            self.render_collapse_section_icon(section, section_state.collapsed, appearance);
        let collapse_all =
            Hoverable::new(section_state.collapsible_hover_state.clone(), move |_| {
                title_row.with_child(collapsible_icon).finish()
            })
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(DriveIndexAction::ToggleSectionCollapsed(section))
            })
            .on_right_click(move |ctx, _, position| {
                let position_id = &warp_drive_section_header_position_id(&section);
                let Some(prompt_rect) = ctx.element_position_by_id(position_id) else {
                    return;
                };
                let offset_position = position - prompt_rect.origin();
                ctx.dispatch_typed_action(DriveIndexAction::ToggleSpaceOverflowMenu {
                    space: space_clone,
                    offset: offset_position,
                });
            })
            .finish();

        let mut left_stack = Stack::new();
        left_stack.add_child(collapse_all);
        if let Some(space_menu_state) = &self.space_menu_open_for_space {
            if space.eq(&space_menu_state.space) {
                left_stack.add_positioned_overlay_child(
                    ChildView::new(&self.menu).finish(),
                    OffsetPositioning::offset_from_save_position_element(
                        warp_drive_section_header_position_id(&section),
                        space_menu_state.offset,
                        PositionedElementOffsetBounds::WindowByPosition,
                        PositionedElementAnchor::TopLeft,
                        ChildAnchor::TopLeft,
                    ),
                );
            }
        }

        // Empty Trash text button
        let (empty_trash_default_font_color, empty_trash_hovered_font_color): (ColorU, ColorU) =
            if self
                .focused_index
                .and_then(|idx| self.ordered_items.get(idx))
                .is_some_and(|item| item == &WarpDriveItemId::Space(space_clone))
            {
                (
                    blended_colors::text_main(appearance.theme(), appearance.theme().background()),
                    appearance.theme().active_ui_text_color().into(),
                )
            } else {
                (
                    appearance.theme().active_ui_text_color().into(),
                    blended_colors::text_main(appearance.theme(), appearance.theme().background()),
                )
            };

        let mut right_stack = Stack::new();
        let empty_trash_default_styles = UiComponentStyles {
            border_width: None,
            font_color: Some(empty_trash_default_font_color),
            font_size: Some(14.),
            font_family_id: Some(appearance.ui_font_family()),
            font_weight: Some(warpui::fonts::Weight::Semibold),
            padding: Some(Coords::uniform(6.)),
            border_radius: Some(CornerRadius::with_all(Radius::Pixels(6.))),
            ..Default::default()
        };

        let empty_trash_hover_style = UiComponentStyles {
            background: Some(appearance.theme().surface_1().into()),
            font_color: Some(empty_trash_hovered_font_color),
            ..empty_trash_default_styles
        };

        let empty_trash_disabled_style = UiComponentStyles {
            background: Some(appearance.theme().surface_3().into()),
            font_color: Some(appearance.theme().disabled_ui_text_color().into()),
            ..empty_trash_default_styles
        };

        let mut empty_trash_button = appearance
            .ui_builder()
            .button_with_custom_styles(
                ButtonVariant::Text,
                section_state.empty_trash_mouse_state.clone(),
                empty_trash_default_styles,
                Some(empty_trash_hover_style),
                Some(empty_trash_hover_style),
                Some(empty_trash_disabled_style),
            )
            .with_text_label("Empty trash".to_string());

        // Only show Empty Trash button when online, do not show for Shared space
        if self.is_online(app) && space != &Space::Shared {
            // Disable Empty Trash button if trash is empty
            let cloud_model = CloudModel::as_ref(app);
            if cloud_model
                .directly_trashed_cloud_objects_in_space(*space, app)
                .count()
                == 0
            {
                empty_trash_button = empty_trash_button.disabled();
            }

            right_stack.add_child(
                empty_trash_button
                    .build()
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(
                            DriveIndexAction::OpenEmptyTrashConfirmationDialog {
                                space: space_clone,
                            },
                        )
                    })
                    .with_cursor(Cursor::PointingHand)
                    .finish(),
            );

            if let Some(space) = self.empty_trash_confirmation_dialog_space {
                if space.eq(&space_clone) {
                    self.add_dialog_to_stack(
                        &mut right_stack,
                        ChildView::new(&self.empty_trash_confirmation_dialog).finish(),
                        warp_drive_section_header_position_id(&section).as_str(),
                        app,
                    );
                }
            }
        }

        // Align items in the header to span the horizontal direction and sit in the vertical
        // center of the row.
        let header_row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(Shrinkable::new(1., left_stack.finish()).finish())
            .with_child(right_stack.finish());

        let mut container = Container::new(
            Container::new(header_row.finish())
                // Indent the header content from within the hoverable
                .with_margin_left(INDEX_CONTENT_MARGIN_LEFT)
                .with_padding_right(INDEX_CONTENT_PADDING_RIGHT)
                .with_margin_top(2.)
                .finish(),
        )
        // Add some styling to the entire row: rounded edges and a small margin between the row and the border
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)));

        // If the space is focused, set background
        let mut is_focused = false;
        if let DriveIndexSection::Space(space) = section {
            if let Some(focused_index) = self.focused_index {
                if Some(&WarpDriveItemId::Space(space)) == self.ordered_items.get(focused_index) {
                    container = container.with_background(
                        warp_core::ui::theme::color::internal_colors::fg_overlay_4(
                            appearance.theme(),
                        ),
                    );
                    is_focused = true;
                }
            }
        }

        Hoverable::new(
            section_state.header_hover_state.clone(),
            move |mouse_state| {
                // If the item is hovered, set a hover background that matches the hover state of warp drive items.
                if mouse_state.is_hovered() && !is_focused || section_state.menu_open {
                    container = container.with_background(
                        warp_core::ui::theme::color::internal_colors::fg_overlay_2(
                            appearance.theme(),
                        ),
                    );
                }

                container.finish()
            },
        )
        .finish()
    }

    // Todo: move the header rendering into WarpDriveItem to consolidate styling logic.
    fn render_section_header(
        &self,
        section: DriveIndexSection,
        section_state: &DriveIndexSectionState,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let rendered_header = match (&self.index_variant, section) {
            (DriveIndexVariant::MainIndex, DriveIndexSection::Space(space)) => {
                let title_font_color: ColorU = if self.focused_index.is_some()
                    && self.ordered_items.get(self.focused_index.unwrap())
                        == Some(&WarpDriveItemId::Space(space))
                {
                    blended_colors::text_main(appearance.theme(), appearance.theme().background())
                } else {
                    appearance.theme().active_ui_text_color().into()
                };
                Some(self.render_space_section_header(
                    self.render_section_title(space, title_font_color, appearance, app),
                    &space,
                    section_state,
                    section,
                    appearance,
                    app,
                ))
            }
            (DriveIndexVariant::MainIndex, DriveIndexSection::CreateATeam) => {
                if self.is_online(app) {
                    Some(self.render_team_section_header(CREATE_TEAM_TEXT.to_owned(), appearance))
                } else {
                    None
                }
            }
            (DriveIndexVariant::MainIndex, DriveIndexSection::JoinTeam) => {
                if self.is_online(app) {
                    let join_teams_text = format!(
                        "Collaborate with {} of your teammates already on Warp.",
                        UserWorkspaces::handle(app)
                            .as_ref(app)
                            .total_teammates_in_joinable_teams()
                    );
                    Some(self.render_team_section_header(join_teams_text, appearance))
                } else {
                    None
                }
            }
            (DriveIndexVariant::Trash, DriveIndexSection::Space(space)) => {
                let title_font_color = self
                    .font_color_based_on_focused_state(appearance, WarpDriveItemId::Space(space));
                Some(self.render_trash_section_header(
                    self.render_section_title(space, title_font_color, appearance, app),
                    &space,
                    section_state,
                    section,
                    appearance,
                    app,
                ))
            }
            (DriveIndexVariant::Trash, DriveIndexSection::CreateATeam) => None,
            (DriveIndexVariant::Trash, DriveIndexSection::JoinTeam) => None,
        };

        if let Some(header) = rendered_header {
            SavePosition::new(
                // Finally, constrain the header row to a height slightly larger than the other objects in the list.
                ConstrainedBox::new(header)
                    .with_height(SECTION_HEADER_CONTENT_HEIGHT + SECTION_HEADER_MARGIN_BOTTOM)
                    .finish(),
                &warp_drive_section_header_position_id(&section),
            )
            .finish()
        } else {
            Empty::new().finish()
        }
    }

    fn render_section_title(
        &self,
        space: Space,
        title_font_color: ColorU,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        Container::new(
            appearance
                .ui_builder()
                .wrappable_text(space.name(app).to_uppercase(), false)
                .with_style(UiComponentStyles {
                    font_family_id: Some(appearance.ui_font_family()),
                    font_size: Some(SECTION_HEADER_FONT_SIZE),
                    font_weight: Some(Weight::Normal),
                    margin: Some(
                        Coords::default()
                            .top(HEADER_TEXT_TOP_AND_BOTTOM_MARGIN)
                            .bottom(HEADER_TEXT_TOP_AND_BOTTOM_MARGIN),
                    ),
                    font_color: Some(title_font_color),
                    ..Default::default()
                })
                .build()
                .finish(),
        )
        .with_margin_right(MARGIN_BETWEEN_HEADER_AND_ICON)
        .finish()
    }

    fn render_trash_row(&self, appearance: &Appearance, _: &AppContext) -> Box<dyn Element> {
        let font_color = self.font_color_based_on_focused_state(appearance, WarpDriveItemId::Trash);
        let icon = Container::new(
            ConstrainedBox::new(Icon::Trash.to_warpui_icon(font_color.into()).finish())
                .with_width(SECTION_HEADER_FONT_SIZE)
                .with_height(SECTION_HEADER_FONT_SIZE)
                .finish(),
        )
        .finish();

        let title = Container::new(
            appearance
                .ui_builder()
                .wrappable_text("TRASH".to_string(), false)
                .with_style(UiComponentStyles {
                    font_family_id: Some(appearance.ui_font_family()),
                    font_size: Some(SECTION_HEADER_FONT_SIZE),
                    font_weight: Some(Weight::Normal),
                    margin: Some(
                        Coords::default()
                            .top(HEADER_TEXT_TOP_AND_BOTTOM_MARGIN)
                            .bottom(HEADER_TEXT_TOP_AND_BOTTOM_MARGIN),
                    ),
                    font_color: Some(font_color),
                    ..Default::default()
                })
                .build()
                .finish(),
        )
        .with_margin_left(MARGIN_BETWEEN_HEADER_AND_ICON)
        .finish();

        let title_row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(icon)
            .with_child(Shrinkable::new(1., title).finish());

        let mut container = Container::new(
            Container::new(title_row.finish())
                .with_margin_left(INDEX_CONTENT_MARGIN_LEFT)
                .with_padding_right(INDEX_CONTENT_PADDING_RIGHT)
                .with_margin_right(INDEX_CONTENT_MARGIN_RIGHT)
                .finish(),
        )
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)));

        // If the trash row is focused, set background
        let mut is_focused = false;
        if let Some(focused_index) = self.focused_index {
            if Some(&WarpDriveItemId::Trash) == self.ordered_items.get(focused_index) {
                container = container.with_background(
                    warp_core::ui::theme::color::internal_colors::fg_overlay_4(appearance.theme()),
                );
                is_focused = true;
            }
        }

        // This represents the clickable region of the header where any mouse-up action will toggle the collapse boolean.
        let header = Hoverable::new(
            self.mouse_state_handles.trash_row_mouse_state.clone(),
            move |mouse_state| {
                if mouse_state.is_hovered() && !is_focused {
                    container
                        .with_background(
                            warp_core::ui::theme::color::internal_colors::fg_overlay_2(
                                appearance.theme(),
                            ),
                        )
                        .finish()
                } else {
                    container.finish()
                }
            },
        )
        .on_click(move |ctx, _, _| ctx.dispatch_typed_action(DriveIndexAction::OpenTrashIndex))
        .finish();

        SavePosition::new(
            Container::new(
                ConstrainedBox::new(header)
                    .with_height(SECTION_HEADER_CONTENT_HEIGHT)
                    .finish(),
            )
            .finish(),
            "WarpDrive_TrashButton",
        )
        .finish()
    }

    fn render_ai_fact_collection_item(
        &self,
        space: Space,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Option<Box<dyn Element>> {
        let warp_drive_item_id = WarpDriveItemId::AIFactCollection;
        let is_selected = self.selected == Some(warp_drive_item_id);
        let mut is_focused = false;
        if let Some(focused_index) = self.focused_index {
            if let Some(&WarpDriveItemId::AIFactCollection) = self.ordered_items.get(focused_index)
            {
                is_focused = true;
            }
        }

        let row = WarpDriveRow::new(
            Box::new(self.ai_fact_collection.clone()),
            self.ai_fact_collection_item_mouse_states.clone(),
            space,
            0,
            self.menu.clone(),
            false, /* can_move */
            !self.menu_items(&space, &warp_drive_item_id, app).is_empty(),
            false,
            false, /* share_dialog_open */
            is_selected,
            is_focused,
            false, /* sync_queue_is_dequeueing */
            tools_panel_menu_direction(app),
            appearance,
        )?;

        Some(row.build().finish())
    }

    fn render_mcp_server_collection_item(
        &self,
        space: Space,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Option<Box<dyn Element>> {
        let warp_drive_item_id = WarpDriveItemId::MCPServerCollection;
        let is_selected = self.selected == Some(warp_drive_item_id);
        let mut is_focused = false;
        if let Some(focused_index) = self.focused_index {
            if let Some(&WarpDriveItemId::MCPServerCollection) =
                self.ordered_items.get(focused_index)
            {
                is_focused = true;
            }
        }

        let row = WarpDriveRow::new(
            Box::new(self.mcp_server_collection.clone()),
            self.mcp_server_collection_item_mouse_states.clone(),
            space,
            0,
            self.menu.clone(),
            false, /* can_move */
            !self.menu_items(&space, &warp_drive_item_id, app).is_empty(),
            false,
            false, /* share_dialog_open */
            is_selected,
            is_focused,
            false, /* sync_queue_is_dequeueing */
            tools_panel_menu_direction(app),
            appearance,
        )?;

        Some(row.build().finish())
    }

    fn render_space_items(
        &self,
        space: Space,
        cloud_model: &CloudModel,
        item_mouse_states: &Vec<ItemStates>,
        active_hover_preview: bool,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Vec<Box<dyn Element>> {
        let mut items = vec![];
        let mut space_idx = 0;
        let cloud_view_model = CloudViewModel::as_ref(app);

        let results = self
            .sorted_orders_by_location
            .get(&CloudObjectLocation::Space(space))
            .cloned()
            .unwrap_or_else(|| {
                match self.index_variant {
                    DriveIndexVariant::MainIndex => {
                        let user_uid = AuthStateProvider::as_ref(app).get().user_id();
                        let is_shared_space = space == Space::Shared;
                        cloud_model
                            .active_cloud_objects_in_location_without_descendents(
                                CloudObjectLocation::Space(space),
                                app,
                            )
                            .filter(move |cloud_object| {
                                !is_shared_space
                                    || user_uid.is_some_and(|uid| {
                                        cloud_object.permissions().has_direct_user_access(uid)
                                    })
                            })
                            .sorted_by(self.sorting_choice.sort_by(
                                cloud_view_model,
                                UpdateTimestamp::Revision,
                                app,
                            ))
                    }
                    DriveIndexVariant::Trash => cloud_model
                        .directly_trashed_cloud_objects_in_space(space, app)
                        .sorted_by(self.sorting_choice.sort_by(
                            cloud_view_model,
                            UpdateTimestamp::Trashed,
                            app,
                        )),
                }
                .map(|item| item.uid())
                .collect()
            });

        let item_iter = results
            .into_iter()
            .filter_map(|uid| cloud_model.get_by_uid(&uid));

        for object in item_iter {
            if let Some(item_and_children) = self.render_item_and_children(
                object,
                item_mouse_states,
                space,
                space_idx,
                0,
                active_hover_preview,
                cloud_model,
                appearance,
                app,
            ) {
                items.push(item_and_children.element);
                space_idx += item_and_children.num_items;
            }
        }
        items
    }

    fn render_team_zero_state_hint(
        &self,
        icon: Icon,
        label: &'static str,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let rendered_icon = ConstrainedBox::new(
            icon.to_warpui_icon(appearance.theme().nonactive_ui_text_color())
                .finish(),
        )
        .with_width(ITEM_FONT_SIZE)
        .with_height(ITEM_FONT_SIZE)
        .finish();
        let hint_text = appearance
            .ui_builder()
            .wrappable_text(label, true)
            .with_style(UiComponentStyles {
                font_family_id: Some(appearance.ui_font_family()),
                font_size: Some(ITEM_FONT_SIZE),
                font_weight: Some(Weight::Light),
                font_color: Some(appearance.theme().disabled_ui_text_color().into()),
                ..Default::default()
            })
            .build()
            .with_padding_left(ITEM_FONT_SIZE)
            .finish();
        Container::new(
            Flex::row()
                .with_child(rendered_icon)
                .with_child(Shrinkable::new(1., hint_text).finish())
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_main_axis_size(MainAxisSize::Max)
                .finish(),
        )
        .with_padding_left(HINT_HORIZONTAL_PADDING)
        .with_padding_top(ITEM_INTERNAL_PADDING)
        .with_padding_bottom(ITEM_INTERNAL_PADDING)
        .with_margin_bottom(2. * ITEM_INTERNAL_PADDING)
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
        .with_background_color(coloru_with_opacity(
            appearance.theme().surface_2().into(),
            50,
        ))
        .finish()
    }

    fn render_team_space_zero_state(&self, appearance: &Appearance) -> Box<dyn Element> {
        let hint_text =
            "Drag or move a personal workflow or notebook here to share it with your team.";
        let zero_state_info = Container::new(
            appearance
                .ui_builder()
                .wrappable_text(hint_text, true)
                .with_style(UiComponentStyles {
                    font_family_id: Some(appearance.ui_font_family()),
                    font_size: Some(ITEM_FONT_SIZE),
                    font_weight: Some(Weight::ExtraLight),
                    font_color: Some(appearance.theme().nonactive_ui_text_color().into()),
                    ..Default::default()
                })
                .build()
                .finish(),
        )
        .with_padding_bottom(HINT_TEXT_PADDING)
        .finish();

        let zero_state_contents = Flex::column().with_children([
            zero_state_info,
            self.render_team_zero_state_hint(Icon::Workflow, ZERO_STATE_WORKFLOW_LABEL, appearance),
            self.render_team_zero_state_hint(Icon::Workflow, ZERO_STATE_WORKFLOW_LABEL, appearance),
            self.render_team_zero_state_hint(Icon::Notebook, ZERO_STATE_NOTEBOOK_LABEL, appearance),
            self.render_team_zero_state_hint(Icon::Notebook, ZERO_STATE_NOTEBOOK_LABEL, appearance),
        ]);

        Container::new(zero_state_contents.finish())
            .with_padding_left(HINT_HORIZONTAL_PADDING)
            .with_padding_bottom(HINT_VERTICAL_PADDING)
            .with_padding_right(HINT_HORIZONTAL_PADDING)
            .with_padding_top(16.)
            .with_margin_top(8.)
            .with_margin_left(8.)
            .with_margin_right(8.)
            .with_border(
                Border::all(1.)
                    .with_border_fill(appearance.theme().surface_2())
                    .with_dashed_border(Dash {
                        dash_length: 8.,
                        gap_length: 8.,
                        ..Default::default()
                    }),
            )
            .finish()
    }

    fn render_create_team_section(
        &self,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let button_text = "Create team".to_owned();
        let create_button = if UserWorkspaces::as_ref(app).total_teammates_in_joinable_teams() == 0
        {
            appearance
                .ui_builder()
                .button(
                    ButtonVariant::Accent,
                    self.mouse_state_handles
                        .create_team_button_mouse_state
                        .clone(),
                )
                .with_style(UiComponentStyles {
                    font_color: Some(
                        appearance
                            .theme()
                            .main_text_color(appearance.theme().accent())
                            .into_solid(),
                    ),
                    font_weight: Some(Weight::Medium),
                    height: Some(38.),
                    font_size: Some(14.),
                    ..Default::default()
                })
                .with_centered_text_label(button_text)
                .build()
                .with_cursor(Cursor::PointingHand)
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(DriveIndexAction::OpenTeamSettingsPage)
                })
                .finish()
        } else {
            appearance
                .ui_builder()
                .button(
                    ButtonVariant::Secondary,
                    self.mouse_state_handles
                        .create_team_button_mouse_state
                        .clone(),
                )
                .with_style(UiComponentStyles {
                    font_weight: Some(Weight::Medium),
                    height: Some(38.),
                    font_size: Some(14.),
                    ..Default::default()
                })
                .with_centered_text_label(button_text)
                .build()
                .with_cursor(Cursor::PointingHand)
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(DriveIndexAction::OpenTeamSettingsPage)
                })
                .finish()
        };

        Container::new(create_button)
            .with_margin_top(16.)
            .with_margin_left(INDEX_CONTENT_MARGIN_LEFT)
            .with_margin_right(INDEX_CONTENT_MARGIN_LEFT)
            .with_margin_bottom(20.)
            .finish()
    }

    fn render_join_discoverable_team_section(
        &self,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let text = if UserWorkspaces::as_ref(app).num_joinable_teams() > 1 {
            "View teams to join"
        } else {
            "View team to join"
        };

        let join_button = Container::new(
            appearance
                .ui_builder()
                .button(
                    ButtonVariant::Accent,
                    self.mouse_state_handles
                        .join_team_button_mouse_state
                        .clone(),
                )
                .with_style(UiComponentStyles {
                    font_color: Some(
                        appearance
                            .theme()
                            .main_text_color(appearance.theme().accent())
                            .into_solid(),
                    ),
                    font_weight: Some(Weight::Medium),
                    height: Some(38.),
                    font_size: Some(14.),
                    ..Default::default()
                })
                .with_centered_text_label(text.to_owned())
                .build()
                .with_cursor(Cursor::PointingHand)
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(DriveIndexAction::OpenTeamSettingsPage)
                })
                .finish(),
        )
        .with_margin_top(16.)
        .finish();

        let or_text = Container::new(
            Text::new_inline("Or", appearance.ui_font_family(), ITEM_FONT_SIZE)
                .with_color(appearance.theme().nonactive_ui_text_color().into())
                .with_style(Properties::default().weight(Weight::Medium))
                .finish(),
        )
        .with_margin_top(14.)
        .finish();

        let or_row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::Center)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(or_text)
            .finish();

        Container::new(Flex::column().with_children([join_button, or_row]).finish())
            .with_margin_left(INDEX_CONTENT_MARGIN_LEFT)
            .with_margin_right(INDEX_CONTENT_MARGIN_LEFT)
            .finish()
    }

    /// Renders the given space header as well as all the included items
    #[allow(clippy::unwrap_in_result)]
    fn render_section(
        &self,
        section: DriveIndexSection,
        appearance: &Appearance,
        cloud_model: &CloudModel,
        app: &AppContext,
    ) -> Option<impl Iterator<Item = Box<dyn Element>>> {
        let mut rendered_space = vec![];

        // Do not render "Create team" or "Join team" sections in the trash index
        if (matches!(section, DriveIndexSection::CreateATeam)
            || matches!(section, DriveIndexSection::JoinTeam))
            && matches!(self.index_variant, DriveIndexVariant::Trash)
        {
            return None;
        }

        // Do not render "Join team" sections for anonymous users
        if matches!(section, DriveIndexSection::JoinTeam)
            && self.auth_state.is_anonymous_or_logged_out()
        {
            return None;
        }

        if let Some(section_state) = self.section_states.get(&section) {
            rendered_space.push(self.render_section_header(
                section,
                section_state,
                appearance,
                app,
            ));

            if !section_state.collapsed {
                match section {
                    DriveIndexSection::Space(space) => {
                        // Check whether any item across all Spaces is actively being hovered over
                        let active_hover_preview = self.item_mouse_states.values().any(|vec| {
                            vec.iter().any(|item| {
                                item.item_hover_state
                                    .lock()
                                    .expect("Should be able to lock")
                                    .is_hovered()
                            })
                        });

                        // If the space is personal, always render the MCP Servers and Rules first
                        if matches!(space, Space::Personal)
                            && matches!(self.index_variant, DriveIndexVariant::MainIndex)
                        {
                            if FeatureFlag::McpServer.is_enabled()
                                && ContextFlag::ShowMCPServers.is_enabled()
                            {
                                if let Some(mcp_server_collection_item) =
                                    self.render_mcp_server_collection_item(space, appearance, app)
                                {
                                    rendered_space.push(mcp_server_collection_item);
                                }
                            }
                            if let Some(ai_fact_collection_item) =
                                self.render_ai_fact_collection_item(space, appearance, app)
                            {
                                rendered_space.push(ai_fact_collection_item);
                            }
                        }

                        rendered_space.extend(
                            self.item_mouse_states
                                .get(&space)
                                .map(|item_mouse_states| {
                                    let space_items = self.render_space_items(
                                        space,
                                        cloud_model,
                                        item_mouse_states,
                                        active_hover_preview,
                                        appearance,
                                        app,
                                    );

                                    // If we are rendering an empty team state in the base index, render the zero state
                                    if space_items.is_empty()
                                        && matches!(space, Space::Team { .. })
                                        && matches!(
                                            self.index_variant,
                                            DriveIndexVariant::MainIndex
                                        )
                                    {
                                        vec![self.render_team_space_zero_state(appearance)]
                                    } else {
                                        space_items
                                    }
                                })
                                .unwrap_or_default(),
                        );
                    }
                    DriveIndexSection::CreateATeam => {
                        if self.is_online(app) {
                            rendered_space.push(self.render_create_team_section(appearance, app));
                        }
                    }
                    DriveIndexSection::JoinTeam => {
                        if self.is_online(app) {
                            rendered_space
                                .push(self.render_join_discoverable_team_section(appearance, app));
                        }
                    }
                }
            }
        }

        Some(rendered_space.into_iter())
    }

    fn render_offline_banner(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);

        Container::new(
            Flex::row()
                .with_child(
                    Container::new(
                        ConstrainedBox::new(
                            Icon::CloudOffline
                                .to_warpui_icon(
                                    appearance
                                        .theme()
                                        .sub_text_color(appearance.theme().surface_2()),
                                )
                                .finish(),
                        )
                        .with_width(CLOUD_OFFLINE_ICON_WIDTH)
                        .with_height(CLOUD_OFFLINE_ICON_HEIGHT)
                        .finish(),
                    )
                    .with_padding_right(OFFLINE_BANNER_ICON_SPACING)
                    .finish(),
                )
                .with_child(
                    Shrinkable::new(
                        1.,
                        Text::new_inline(
                            OFFLINE_BANNER_TEXT,
                            appearance.ui_font_family(),
                            appearance.ui_font_size(),
                        )
                        .finish(),
                    )
                    .finish(),
                )
                .with_main_axis_alignment(MainAxisAlignment::Center)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .finish(),
        )
        .with_background(appearance.theme().surface_2())
        .with_padding_left(OFFLINE_BANNER_PADDING_HORIZONTAL)
        .with_padding_right(OFFLINE_BANNER_PADDING_HORIZONTAL)
        .with_padding_top(OFFLINE_BANNER_PADDING_VERTICAL)
        .with_padding_bottom(OFFLINE_BANNER_PADDING_VERTICAL)
        .finish()
    }

    fn render_all_sections(&self, app: &AppContext) -> impl Iterator<Item = Box<dyn Element>> {
        let appearance = Appearance::as_ref(app);
        let cloud_model = CloudModel::as_ref(app);

        let mut sections = vec![];

        if !self.is_online(app) {
            sections.push(self.render_offline_banner(app));
        }

        for section in self.sections.iter() {
            // items_in_space includes the header of the space as well as all the untrashed cloud objects.
            let rendered_section = self.render_section(*section, appearance, cloud_model, app);
            if let Some(rendered_section) = rendered_section {
                let mut section_content =
                    Container::new(Flex::column().with_children(rendered_section).finish());
                // All spaces should be separated by some padding
                section_content = section_content.with_padding_bottom(PADDING_BETWEEN_SPACES);

                if let DriveIndexSection::Space(space) = section {
                    let location = CloudObjectLocation::Space(*space);
                    sections.push(self.render_as_drop_target(
                        section_content.finish(),
                        location,
                        appearance,
                    ));
                } else {
                    sections.push(section_content.finish())
                }
            }
        }

        if self.index_variant == DriveIndexVariant::MainIndex
            && !self
                .auth_state
                .is_user_web_anonymous_user()
                .unwrap_or_default()
        {
            let trash_row = self.render_trash_row(appearance, app);
            sections.push(self.render_as_drop_target(
                trash_row,
                CloudObjectLocation::Trash,
                appearance,
            ));
        }

        sections.into_iter()
    }

    fn render_workspace_picker(&self) -> Box<dyn Element> {
        Container::new(ChildView::new(&self.workspace_dropdown).finish())
            .with_padding_bottom(6.)
            .with_padding_top(6.)
            .with_padding_left(12.)
            .with_padding_right(12.)
            .finish()
    }

    fn render_title(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        let text = Container::new(
            appearance
                .ui_builder()
                .span(WARP_DRIVE_TITLE.to_string())
                .with_style(UiComponentStyles {
                    font_family_id: Some(appearance.ui_font_family()),
                    font_size: Some(TITLE_FONT_SIZE),
                    font_weight: Some(Weight::Semibold),
                    ..Default::default()
                })
                .build()
                .finish(),
        )
        .with_padding_right(8.)
        .finish();

        let mut title = Flex::row()
            .with_child(text)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_main_axis_size(MainAxisSize::Max);

        let mut title_right_side = Flex::row();

        if self.show_warp_drive_loading_icon && self.is_online(app) {
            title_right_side.add_child(self.render_warp_drive_loading_icon(appearance));
        }

        // Only show the global retry button if there are errored objects
        if self.num_errored_objects > 0 && self.is_online(app) {
            title_right_side.add_child(self.render_retry_button(appearance));
        }

        let search_button = icon_button(
            appearance,
            Icon::Search,
            false,
            self.mouse_state_handles.search_button_mouse_state.clone(),
        )
        .build()
        .on_click(|ctx, _, _| ctx.dispatch_typed_action(DrivePanelAction::OpenSearch))
        .finish();

        title_right_side.add_child(
            Container::new(Align::new(search_button).finish())
                .with_padding_right(crate::drive::panel::styles::SEARCH_BUTTON_PADDING_RIGHT)
                .finish(),
        );

        title_right_side.add_child(self.render_sorting_button(appearance));

        title.add_child(title_right_side.finish());

        Container::new(title.finish())
            .with_padding_bottom(6.)
            .with_padding_left(INDEX_CONTENT_MARGIN_LEFT)
            .with_padding_right(TITLE_CONTENT_PADDING_RIGHT)
            .with_margin_right(INDEX_CONTENT_MARGIN_RIGHT)
            .finish()
    }

    fn render_trash_title(&self, appearance: &Appearance) -> Box<dyn Element> {
        let text = Container::new(
            appearance
                .ui_builder()
                .span("Trash".to_string())
                .with_style(UiComponentStyles {
                    font_family_id: Some(appearance.ui_font_family()),
                    font_size: Some(TITLE_FONT_SIZE),
                    font_weight: Some(Weight::Semibold),
                    ..Default::default()
                })
                .build()
                .finish(),
        )
        .finish();

        let button = icon_button(
            appearance,
            Icon::ArrowLeft,
            false,
            self.mouse_state_handles
                .exit_trash_button_mouse_state
                .clone(),
        )
        .build()
        .on_click(move |ctx, _, _| ctx.dispatch_typed_action(DriveIndexAction::CloseTrashIndex))
        .finish();

        let text_with_button = Flex::row()
            .with_child(Container::new(button).with_margin_right(8.).finish())
            .with_child(text)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .finish();

        let mut title = Flex::row()
            .with_child(text_with_button)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_main_axis_size(MainAxisSize::Max);

        title.add_child(self.render_sorting_button(appearance));

        Container::new(title.finish())
            .with_padding_bottom(6.)
            .with_padding_left(INDEX_CONTENT_MARGIN_LEFT)
            .with_margin_right(INDEX_CONTENT_MARGIN_RIGHT)
            .with_padding_right(TITLE_CONTENT_PADDING_RIGHT)
            .finish()
    }

    fn render_deletion_warning(&self, appearance: &Appearance) -> Box<dyn Element> {
        let icon_and_text = Container::new(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_child(
                    Container::new(
                        ConstrainedBox::new(
                            Icon::Info
                                .to_warpui_icon(appearance.theme().nonactive_ui_text_color())
                                .finish(),
                        )
                        .with_height(15.)
                        .with_width(15.)
                        .finish(),
                    )
                    .with_margin_right(8.)
                    .finish(),
                )
                .with_child(
                    Shrinkable::new(
                        1.,
                        appearance
                            .ui_builder()
                            .wrappable_text(
                                "Items in the trash will be deleted forever after 30 days."
                                    .to_string(),
                                true,
                            )
                            .with_style(UiComponentStyles {
                                font_family_id: Some(appearance.ui_font_family()),
                                font_size: Some(WARNING_FONT_SIZE),
                                font_weight: Some(Weight::ExtraLight),
                                font_color: Some(
                                    appearance.theme().nonactive_ui_text_color().into(),
                                ),
                                ..Default::default()
                            })
                            .build()
                            .finish(),
                    )
                    .finish(),
                )
                .finish(),
        )
        .finish();

        Container::new(icon_and_text)
            .with_margin_left(17.)
            .with_padding_bottom(6.)
            .with_padding_right(6.)
            .finish()
    }

    /// Renders a warp drive item within the index. If the item is a folder, we recursively call
    /// this function in order to render the folder's children (if it's open).
    /// This index refers to the idx within a given space, and is needed to render the context menu at the
    /// correct position. If the item should not be shown, this returns [`None`].
    #[allow(clippy::too_many_arguments)]
    fn render_item_and_children(
        &self,
        object: &dyn CloudObject,
        item_mouse_states: &Vec<ItemStates>,
        space: Space,
        space_index: usize,
        folder_depth: usize,
        active_hover_preview: bool,
        cloud_model: &CloudModel,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Option<RenderedWarpDriveItemAndChildren> {
        if !object.renders_in_warp_drive() {
            return None;
        }

        let mut stack = Stack::new();
        let row_object_id = object.cloud_object_type_and_id();
        let warp_drive_item_id = WarpDriveItemId::Object(row_object_id);
        let access_level = CloudViewModel::as_ref(app).access_level(&row_object_id.uid(), app);

        let share_dialog_open = self.share_dialog_open_for_object == Some(warp_drive_item_id);
        // If the share dialog is open, we don't want to open the menu for the same object.
        let menu_open =
            self.menu_object_id_if_open == Some(warp_drive_item_id) && !share_dialog_open;
        let can_move = self.online_only_operation_allowed(&row_object_id, app)
            && matches!(self.index_variant, DriveIndexVariant::MainIndex)
            && access_level.can_move_drive();
        let mut is_focused = false;

        let is_selected = self.selected == Some(warp_drive_item_id);
        if let Some(focused_index) = self.focused_index {
            if !self.ordered_items.is_empty() {
                if let Some(&WarpDriveItemId::Object(cloud_id)) =
                    self.ordered_items.get(focused_index)
                {
                    is_focused = row_object_id == cloud_id;
                }
            }
        }

        let row = WarpDriveRow::new_from_cloud_object(
            object,
            item_mouse_states[space_index].clone(),
            space,
            folder_depth,
            self.menu.clone(),
            can_move,
            !self.menu_items(&space, &warp_drive_item_id, app).is_empty(),
            menu_open,
            share_dialog_open,
            is_selected,
            is_focused,
            SyncQueue::as_ref(app).is_dequeueing(),
            tools_panel_menu_direction(app),
            appearance,
        )?;
        let mut total_rows_for_item = 1;

        let object_preview = row.render_preview(appearance, app);
        let is_hovered = row.should_show_preview();

        let row_position_id = row_object_id.drive_row_position_id();
        stack.add_child(row.build().finish());

        let row_object_id: CloudObjectTypeAndId = object.cloud_object_type_and_id();
        if row_object_id.as_folder_id().is_some_and(|folder_id| {
            self.cloud_object_naming_dialog
                .is_open_for_folder(folder_id)
        }) {
            self.add_dialog_to_stack(
                &mut stack,
                ConstrainedBox::new(self.cloud_object_naming_dialog.render(appearance, app))
                    .with_max_width(CLOUD_OBJECT_DIALOG_WIDTH)
                    .finish(),
                row_position_id.as_str(),
                app,
            );
        } else if share_dialog_open {
            self.add_dialog_to_stack(
                &mut stack,
                ChildView::new(&self.sharing_dialog).finish(),
                row_position_id.as_str(),
                app,
            );
        } else if is_hovered || (!active_hover_preview && is_focused && !is_selected) {
            // Show object preview when 1) object is hovered, 2) object is focused but not selected + nothing else is hovered
            // (active object in pane won't have hover preview since it's already open)
            if let Some(preview) = object_preview {
                self.add_row_overlay_to_stack(
                    &mut stack,
                    preview,
                    row_position_id.as_str(),
                    OffsetType::Pixel(HOVER_PREVIEW_X_OFFSET),
                    OffsetType::Pixel(HOVER_PREVIEW_Y_OFFSET),
                    app,
                );
            }
        }

        let mut rendered_item = stack.finish();

        // If the item is a folder and the folder is open, render all of the
        // folders children as well.
        let folder: Option<&CloudFolder> = object.into();
        if let Some(folder) = folder {
            let mut item_and_children = vec![rendered_item];

            if folder.model().is_open {
                let results = self
                    .sorted_orders_by_location
                    .get(&CloudObjectLocation::Folder(folder.id))
                    .cloned()
                    .unwrap_or_else(|| {
                        let cloud_view_model = CloudViewModel::as_ref(app);
                        match self.index_variant {
                            DriveIndexVariant::MainIndex => cloud_model
                                .active_cloud_objects_in_location_without_descendents(
                                    CloudObjectLocation::Folder(folder.id),
                                    app,
                                )
                                .sorted_by(self.sorting_choice.sort_by(
                                    cloud_view_model,
                                    UpdateTimestamp::Revision,
                                    app,
                                )),
                            DriveIndexVariant::Trash => cloud_model
                                .indirectly_trashed_cloud_objects_in_location_without_descendents(
                                    CloudObjectLocation::Folder(folder.id),
                                    app,
                                )
                                .sorted_by(self.sorting_choice.sort_by(
                                    cloud_view_model,
                                    UpdateTimestamp::Revision,
                                    app,
                                )),
                        }
                        .map(|item| item.uid())
                        .collect()
                    });

                let item_iter = results
                    .into_iter()
                    .filter_map(|uid| cloud_model.get_by_uid(&uid));

                item_iter.for_each(|object| {
                    // TODO: Remove this check once we change our permissions logic. This is a temporary
                    // solution to ensure that we don't render something in this folder that is not a part of the space.
                    // Once we move to a permissions structure where we always look to the parent - this will not
                    // be needed.
                    if object.permissions().owner == folder.permissions.owner {
                        if let Some(child) = self.render_item_and_children(
                            object,
                            item_mouse_states,
                            space,
                            space_index + total_rows_for_item,
                            folder_depth + 1,
                            active_hover_preview,
                            cloud_model,
                            appearance,
                            app,
                        ) {
                            item_and_children.push(child.element);
                            total_rows_for_item += child.num_items;
                        }
                    }
                });
            }

            let location = CloudObjectLocation::Folder(folder.id);

            // Since this is a folder, render it as a drop target
            rendered_item = self.render_as_drop_target(
                Flex::column().with_children(item_and_children).finish(),
                location,
                appearance,
            );
        }

        Some(RenderedWarpDriveItemAndChildren {
            element: rendered_item,
            num_items: total_rows_for_item,
        })
    }

    fn add_dialog_to_stack(
        &self,
        stack: &mut Stack,
        child: Box<dyn Element>,
        row_position_id: &str,
        app: &AppContext,
    ) {
        self.add_row_overlay_to_stack(
            stack,
            child,
            row_position_id,
            OffsetType::Pixel(DIALOG_OFFSET_PIXELS),
            OffsetType::Pixel(DIALOG_OFFSET_PIXELS),
            app,
        );
    }

    fn add_row_overlay_to_stack(
        &self,
        stack: &mut Stack,
        child: Box<dyn Element>,
        row_position_id: &str,
        x_offset: OffsetType,
        y_offset: OffsetType,
        app: &AppContext,
    ) {
        // NOTE: Temporaray solution to prevent crashes on Dev/Local
        // The [`PositionedElementOffsetBounds::WindowBySize`] value requires that the element this preview
        // is positioned to be in the positive cache in the previous frame. Since there are some cases where
        // these two are rendered at the exact same time for the first time (like for linking), it can cause a panic.
        let mut x_axis_bounds = PositionedElementOffsetBounds::Unbounded;
        if app
            .element_position_by_id_at_last_frame(self.window_id, row_position_id)
            .is_some()
        {
            x_axis_bounds = PositionedElementOffsetBounds::WindowBySize;
        }

        let opens_right = matches!(tools_panel_menu_direction(app), MenuDirection::Right);
        let (x_anchor, flipped_x_offset) = if opens_right {
            (
                AnchorPair::new(XAxisAnchor::Right, XAxisAnchor::Left),
                x_offset,
            )
        } else {
            (
                AnchorPair::new(XAxisAnchor::Left, XAxisAnchor::Right),
                match x_offset {
                    OffsetType::Pixel(px) => OffsetType::Pixel(-px),
                    other => other,
                },
            )
        };

        stack.add_positioned_overlay_child(
            child,
            OffsetPositioning::from_axes(
                PositioningAxis::relative_to_stack_child(
                    row_position_id,
                    x_axis_bounds,
                    flipped_x_offset,
                    x_anchor,
                ),
                PositioningAxis::relative_to_stack_child(
                    row_position_id,
                    PositionedElementOffsetBounds::WindowByPosition,
                    y_offset,
                    AnchorPair::new(YAxisAnchor::Middle, YAxisAnchor::Middle),
                ),
            ),
        );
    }

    fn render_collapse_section_icon(
        &self,
        section: DriveIndexSection,
        is_collapsed: bool,
        appearance: &Appearance,
    ) -> Box<dyn warpui::Element> {
        let icon = if is_collapsed {
            Icon::ListCollapsed
        } else {
            Icon::ListOpen
        };
        let icon_color = match section {
            DriveIndexSection::Space(space) => {
                // Set icon color contrast correctly if a space is focused
                if self.focused_index.is_some()
                    && self.ordered_items.get(self.focused_index.unwrap())
                        == Some(&WarpDriveItemId::Space(space))
                {
                    blended_colors::text_main(appearance.theme(), appearance.theme().background())
                        .into()
                } else {
                    appearance.theme().foreground()
                }
            }
            _ => appearance.theme().foreground(),
        };

        // This icon should render the same as other WarpDrive icons but with no click or hover states.
        Container::new(
            ConstrainedBox::new(icon.to_warpui_icon(icon_color).finish())
                .with_width(SECTION_HEADER_FONT_SIZE)
                .with_height(SECTION_HEADER_FONT_SIZE)
                .finish(),
        )
        .with_padding_right(6.)
        .finish()
    }

    fn render_warp_drive_loading_icon(&self, appearance: &Appearance) -> Box<dyn warpui::Element> {
        // Use same padding as icon_button (4px) to center the icon within ICON_DIMENSIONS
        let icon_button_padding = (ICON_DIMENSIONS - LOADING_ICON_WIDTH) / 2.;
        let loading_icon = Container::new(
            ConstrainedBox::new(
                Icon::Refresh
                    .to_warpui_icon(
                        appearance
                            .theme()
                            .sub_text_color(appearance.theme().surface_1()),
                    )
                    .finish(),
            )
            .with_width(LOADING_ICON_WIDTH)
            .with_height(LOADING_ICON_HEIGHT)
            .finish(),
        )
        .with_uniform_padding(icon_button_padding)
        .with_margin_right(4.)
        .finish();

        let hoverable = Hoverable::new(
            self.mouse_state_handles
                .warp_drive_initial_load_mouse_state
                .clone(),
            |mouse_state| {
                let mut stack = Stack::new().with_child(loading_icon);
                if mouse_state.is_hovered() {
                    let tooltip = appearance
                        .ui_builder()
                        .tool_tip(String::from("Syncing Warp Drive"));

                    stack.add_positioned_overlay_child(
                        tooltip.build().finish(),
                        OffsetPositioning::offset_from_parent(
                            vec2f(0., 4.),
                            ParentOffsetBounds::Unbounded,
                            ParentAnchor::BottomMiddle,
                            ChildAnchor::TopMiddle,
                        ),
                    );
                };
                stack.finish()
            },
        );

        hoverable.finish()
    }

    fn render_sorting_button(&self, appearance: &Appearance) -> Box<dyn warpui::Element> {
        let mut button = icon_button_with_context_menu(
            Icon::Sort,
            move |ctx, _, _| ctx.dispatch_typed_action(DriveIndexAction::ToggleSortingMenu),
            self.mouse_state_handles.sorting_button_mouse_state.clone(),
            &self.menu,
            self.sorting_button_menu_open,
            MenuDirection::Right,
            Some(Cursor::PointingHand),
            None,
            appearance,
        );

        let hoverable = Hoverable::new(
            self.mouse_state_handles.sorting_button_mouse_state.clone(),
            |mouse_state| {
                if mouse_state.is_hovered() {
                    let tooltip = appearance
                        .ui_builder()
                        .tool_tip(SORTING_BUTTON_TOOLTIP_LABEL.to_string());

                    button.add_positioned_overlay_child(
                        tooltip.build().finish(),
                        OffsetPositioning::offset_from_parent(
                            vec2f(0., 4.),
                            ParentOffsetBounds::Unbounded,
                            ParentAnchor::BottomMiddle,
                            ChildAnchor::TopMiddle,
                        ),
                    );
                }
                button.finish()
            },
        );

        hoverable.finish()
    }

    fn render_retry_button(&self, appearance: &Appearance) -> Box<dyn warpui::Element> {
        let ui_builder = appearance.ui_builder().clone();

        icon_button(
            appearance,
            Icon::Refresh,
            false,
            self.mouse_state_handles.retry_button_mouse_state.clone(),
        )
        .with_tooltip(move || {
            ui_builder
                .tool_tip(RETRY_BUTTON_TOOLTIP_LABEL.to_string())
                .build()
                .finish()
        })
        .build()
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(DriveIndexAction::RetryAllFailedObjects)
        })
        .finish()
    }

    fn render_create_new_button(
        &self,
        appearance: &Appearance,
        space: Space,
        state: &DriveIndexSectionState,
        app: &AppContext,
    ) -> Box<dyn warpui::Element> {
        let mut button;
        // Set color contrast correctly when focused
        if self.focused_index.is_some()
            && self.ordered_items.get(self.focused_index.unwrap())
                == Some(&WarpDriveItemId::Space(space))
        {
            button = highlight(
                icon_button(
                    appearance,
                    Icon::Plus,
                    state.menu_open,
                    state.create_menu_mouse_state_handle.clone(),
                ),
                appearance,
            );
        } else {
            button = icon_button(
                appearance,
                Icon::Plus,
                state.menu_open,
                state.create_menu_mouse_state_handle.clone(),
            );
        }

        // Override hover background to surface_1 for better visibility on section header
        button = button.with_hovered_styles(
            UiComponentStyles::default()
                .set_background(appearance.theme().surface_1().into())
                .set_border_color(appearance.theme().surface_3().into()),
        );

        let mut stack = Stack::new().with_child(
            button
                .with_cursor(Some(Cursor::PointingHand))
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(DriveIndexAction::ToggleNewAssetsMenu(space));
                })
                .finish(),
        );

        if state.menu_open {
            stack.add_positioned_overlay_child(
                ChildView::new(&self.menu).finish(),
                OffsetPositioning::offset_from_parent(
                    vec2f(0., 0.),
                    ParentOffsetBounds::WindowByPosition,
                    ParentAnchor::TopRight,
                    ChildAnchor::TopLeft,
                ),
            );
        }

        // Todo(Jack): render this as part of an item row, not the create new button.
        if self.cloud_object_naming_dialog.is_open_for_space(&space) {
            stack.add_positioned_overlay_child(
                ConstrainedBox::new(self.cloud_object_naming_dialog.render(appearance, app))
                    .with_max_width(CLOUD_OBJECT_DIALOG_WIDTH)
                    .finish(),
                OffsetPositioning::offset_from_parent(
                    vec2f(0., 0.),
                    ParentOffsetBounds::WindowByPosition,
                    ParentAnchor::TopRight,
                    ChildAnchor::TopLeft,
                ),
            );
        }
        ConstrainedBox::new(stack.finish())
            .with_height(ICON_DIMENSIONS)
            .finish()
    }

    fn render_add_teammates_button(
        &self,
        appearance: &Appearance,
        state: &DriveIndexSectionState,
        space: Space,
    ) -> Box<dyn warpui::Element> {
        let mut button = icon_button(
            appearance,
            Icon::AddTeammates,
            false,
            state.add_teammates_mouse_state.clone(),
        );
        // Set color contrast correctly when focused
        if self.focused_index.is_some()
            && self.ordered_items.get(self.focused_index.unwrap())
                == Some(&WarpDriveItemId::Space(space))
        {
            button = highlight(button, appearance)
        };

        // Override hover background to surface_1 for better visibility on section header
        button = button.with_hovered_styles(
            UiComponentStyles::default()
                .set_background(appearance.theme().surface_1().into())
                .set_border_color(appearance.theme().surface_3().into()),
        );

        Container::new(
            Align::new(
                button
                    .build()
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(DriveIndexAction::OpenTeamSettingsPage)
                    })
                    .finish(),
            )
            .finish(),
        )
        .with_margin_right(2.) // These icons at the end of a row are spaced apart with 2 pixels between them
        .finish()
    }

    fn font_color_based_on_focused_state(
        &self,
        appearance: &Appearance,
        item: WarpDriveItemId,
    ) -> ColorU {
        if self.focused_index.is_some()
            && self.ordered_items.get(self.focused_index.unwrap()) == Some(&item)
        {
            blended_colors::text_main(appearance.theme(), appearance.theme().background())
        } else {
            appearance.theme().active_ui_text_color().into()
        }
    }

    fn clear_drop_target(&mut self, ctx: &mut ViewContext<Self>) {
        self.current_drop_target = None;
        ctx.notify();
    }

    fn toggle_section_collapse(
        &mut self,
        section: &DriveIndexSection,
        ctx: &mut ViewContext<Self>,
    ) {
        if let Some(section_state) = self.section_states.get_mut(section) {
            section_state.collapsed = !section_state.collapsed;
        }

        self.refocus_section_index(section, ctx);
    }

    fn refocus_section_index(&mut self, section: &DriveIndexSection, ctx: &mut ViewContext<Self>) {
        if self.focused_index.is_some() {
            if let DriveIndexSection::Space(space) = *section {
                self.set_focused_item(WarpDriveItemId::Space(space), true, ctx);
            }
            // Need to re-render focused index in Warp Drive after a space has been toggled
            if let Some(focused_index) = self.focused_index {
                self.update_focused_params(focused_index, CloudModel::as_ref(ctx));
            }
        }
    }

    fn update_drop_target_location(
        &mut self,
        new_location: CloudObjectLocation,
        ctx: &mut ViewContext<Self>,
    ) {
        // TODO: we need to tell the user why 'move' is not working.
        let is_drop_target_valid = match new_location {
            CloudObjectLocation::Folder(folder_id) => self.online_only_operation_allowed(
                &CloudObjectTypeAndId::from_id_and_type(folder_id, ObjectType::Folder),
                ctx,
            ),
            CloudObjectLocation::Space(_) => self.is_online(ctx),
            CloudObjectLocation::Trash => self.is_online(ctx),
        };

        if is_drop_target_valid {
            self.current_drop_target = Some(new_location);
            ctx.notify();
        }
    }

    pub fn move_object_to_team_owner(
        &mut self,
        cloud_object_type_and_id: &CloudObjectTypeAndId,
        space: Space,
        ctx: &mut ViewContext<Self>,
    ) {
        self.move_object(
            cloud_object_type_and_id,
            CloudObjectLocation::Space(space),
            ctx,
        );
    }

    fn move_object(
        &mut self,
        cloud_object_type_and_id: &CloudObjectTypeAndId,
        new_location: CloudObjectLocation,
        ctx: &mut ViewContext<Self>,
    ) {
        self.current_drop_target = None;
        ctx.notify();

        if !self.online_only_operation_allowed(cloud_object_type_and_id, ctx) {
            return;
        }

        let cloud_model = CloudModel::handle(ctx);

        // Only proceed if we can move the object to this location AND the operation results in a change.
        // Even though an object technically can be moved to its current location (can_move_object_to_location returns true)
        // we do not need to do any of the update logic that follows.
        if !cloud_model.as_ref(ctx).can_move_object_to_location(
            &cloud_object_type_and_id.uid(),
            new_location,
            ctx,
        ) || cloud_model
            .as_ref(ctx)
            .object_location(&cloud_object_type_and_id.uid(), ctx)
            .is_none_or(|location| location == new_location)
        {
            return;
        }

        // Check if object is being moved into team space, if it is, then check
        // corresponding object limits for that team.
        if let CloudObjectLocation::Space(Space::Team { team_uid }) = new_location {
            match *cloud_object_type_and_id {
                CloudObjectTypeAndId::Notebook(_) => {
                    if !UserWorkspaces::has_capacity_for_shared_notebooks(team_uid, ctx, 1) {
                        // If team has reached the limit for notebooks, show the modal
                        // and return early.
                        ctx.emit(DriveIndexEvent::OpenSharedObjectsCreationDeniedModal(
                            DriveObjectType::Notebook {
                                is_ai_document: false,
                            },
                            team_uid,
                        ));
                        return;
                    }
                }
                CloudObjectTypeAndId::Workflow(_) => {
                    if !UserWorkspaces::has_capacity_for_shared_workflows(team_uid, ctx, 1) {
                        // If team has reached the limit for workflows, show the modal
                        // and return early.
                        ctx.emit(DriveIndexEvent::OpenSharedObjectsCreationDeniedModal(
                            DriveObjectType::Workflow,
                            team_uid,
                        ));
                        return;
                    }
                }
                _ => (),
            }
        }

        // Otherwise allow object move to go through.
        UpdateManager::handle(ctx).update(ctx, move |update_manager, ctx| {
            update_manager.move_object_to_location(*cloud_object_type_and_id, new_location, ctx);
        });

        match new_location {
            CloudObjectLocation::Space(space) => self.open_section_of_space(space),
            CloudObjectLocation::Folder(folder_id) => {
                cloud_model.update(ctx, |cloud_model, ctx| {
                    cloud_model.open_folder(folder_id, ctx);
                });
            }
            // If location is the trash, then the above move_[object]_to_location call already trashed the object
            CloudObjectLocation::Trash => {}
        }

        self.reset_menus(ctx);
        self.initialize_section_states(ctx);
        ctx.notify();
    }

    fn leave_object(
        &mut self,
        cloud_object_type_and_id: &CloudObjectTypeAndId,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(server_id) = cloud_object_type_and_id.server_id() else {
            return;
        };

        UpdateManager::handle(ctx).update(ctx, move |update_manager, ctx| {
            update_manager.leave_object(server_id, ctx);
        });

        // Reflect the removed objects.
        self.initialize_section_states(ctx);
        ctx.notify();
    }

    /// If the given space is tied to a section in warp drive, ensures that that section is open.
    fn open_section_of_space(&mut self, space: Space) {
        if let Some(target_section) = self
            .sections
            .iter()
            .find(|section| **section == DriveIndexSection::Space(space))
        {
            if let Some(section_state) = self.section_states.get_mut(target_section) {
                section_state.collapsed = false;
            }
        }
    }

    fn set_section_collapsed_state(
        &mut self,
        section: &DriveIndexSection,
        collapse_section: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        if let Some(section_state) = self.section_states.get_mut(section) {
            section_state.collapsed = collapse_section;
        }

        self.refocus_section_index(section, ctx);
    }

    fn create_object(
        &mut self,
        object_type: DriveObjectType,
        space: Space,
        initial_folder_id: Option<SyncId>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.reset_menus(ctx);
        let title = self.cloud_object_naming_dialog.title(ctx);

        if let Some(title) = title.clone() {
            // it's not inherently wrong to get into this function with an empty title,
            // but if we do, we'll bail unless it's a personal notebook being created
            if title.is_empty()
                && !(matches!(object_type, DriveObjectType::Notebook { .. })
                    && matches!(space, Space::Personal))
            {
                return;
            }
        }

        match object_type {
            DriveObjectType::Notebook { .. } => {
                if has_feature_gated_anonymous_user_reached_notebook_limit(ctx) {
                    return;
                }

                // If the new notebook is being created in the team space, check if the team has
                // reached the limit for notebooks.
                if let Space::Team { team_uid } = space {
                    if !UserWorkspaces::has_capacity_for_shared_notebooks(team_uid, ctx, 1) {
                        // If team has reached the limit for notebooks, show the modal
                        // and return early.
                        ctx.emit(DriveIndexEvent::OpenSharedObjectsCreationDeniedModal(
                            object_type,
                            team_uid,
                        ));
                        return;
                    }
                }
                ctx.emit(DriveIndexEvent::CreateNotebook {
                    space,
                    title,
                    initial_folder_id,
                });
            }
            DriveObjectType::Folder => {
                if let Some(title) = title {
                    ctx.emit(DriveIndexEvent::CreateFolder {
                        space,
                        title,
                        initial_folder_id,
                    });
                }
            }
            DriveObjectType::EnvVarCollection => {
                if has_feature_gated_anonymous_user_reached_env_var_limit(ctx) {
                    return;
                }

                ctx.emit(DriveIndexEvent::CreateEnvVarCollection {
                    space,
                    title,
                    initial_folder_id,
                })
            }
            DriveObjectType::Workflow => {
                if has_feature_gated_anonymous_user_reached_workflow_limit(ctx) {
                    return;
                }

                ctx.emit(DriveIndexEvent::CreateWorkflow {
                    space,
                    title,
                    initial_folder_id,
                    is_for_agent_mode: false,
                    content: None,
                })
            }
            DriveObjectType::AgentModeWorkflow => {
                if has_feature_gated_anonymous_user_reached_workflow_limit(ctx) {
                    return;
                }

                ctx.emit(DriveIndexEvent::CreateWorkflow {
                    space,
                    title,
                    initial_folder_id,
                    is_for_agent_mode: true,
                    content: None,
                })
            }
            DriveObjectType::AIFact => {
                if let Some(fact) = title {
                    ctx.emit(DriveIndexEvent::CreateAIFact {
                        space,
                        fact: AIFact::Memory(AIMemory {
                            name: None,
                            content: fact,
                            is_autogenerated: false,
                            suggested_logging_id: None,
                        }),
                        initial_folder_id,
                    })
                }
            }
            DriveObjectType::MCPServer => {
                todo!()
            }
            DriveObjectType::AIFactCollection | DriveObjectType::MCPServerCollection => {}
        }

        self.cloud_object_naming_dialog.close(ctx);
        ctx.notify();
    }

    fn rename_folder(&mut self, folder_id: SyncId, ctx: &mut ViewContext<Self>) {
        if let Some(new_name) = self.cloud_object_naming_dialog.title(ctx) {
            if !new_name.is_empty() {
                self.reset_menus(ctx);

                UpdateManager::handle(ctx).update(ctx, move |update_manager, ctx| {
                    update_manager.rename_folder(folder_id, new_name, ctx);
                });

                self.cloud_object_naming_dialog.close(ctx);
                ctx.notify();
            }
        }
    }

    fn trash_object(
        &mut self,
        cloud_object_type_and_id: CloudObjectTypeAndId,
        ctx: &mut ViewContext<Self>,
    ) {
        UpdateManager::handle(ctx).update(ctx, move |update_manager, ctx| {
            update_manager.trash_object(cloud_object_type_and_id, ctx);
        });
        self.reset_menus(ctx);
        ctx.notify();
    }

    pub fn untrash_object(
        &mut self,
        cloud_object_type_and_id: &CloudObjectTypeAndId,
        ctx: &mut ViewContext<Self>,
    ) {
        // Check if object being untrashed is in team space, if it is, then check
        // corresponding object limits for that team.
        if let Some(space) =
            CloudViewModel::as_ref(ctx).object_space(&cloud_object_type_and_id.uid(), ctx)
        {
            match space {
                Space::Team { team_uid } => {
                    match cloud_object_type_and_id {
                        CloudObjectTypeAndId::Notebook(_) => {
                            if !UserWorkspaces::has_capacity_for_shared_notebooks(team_uid, ctx, 1)
                            {
                                // If team has reached the limit for notebooks, show the modal
                                // and return early.
                                ctx.emit(DriveIndexEvent::OpenSharedObjectsCreationDeniedModal(
                                    DriveObjectType::Notebook {
                                        is_ai_document: false,
                                    },
                                    team_uid,
                                ));
                                return;
                            }
                        }
                        CloudObjectTypeAndId::Workflow(_) => {
                            if !UserWorkspaces::has_capacity_for_shared_workflows(team_uid, ctx, 1)
                            {
                                // If team has reached the limit for workflows, show the modal
                                // and return early.
                                ctx.emit(DriveIndexEvent::OpenSharedObjectsCreationDeniedModal(
                                    DriveObjectType::Workflow,
                                    team_uid,
                                ));
                                return;
                            }
                        }
                        CloudObjectTypeAndId::Folder(folder_id) => {
                            let cloud_model = CloudModel::handle(ctx);

                            // When untrashing a folder, check to see if there are any notebooks or workflows
                            // in the trashed folder and make sure they are within limits.
                            let trashed_object_types = cloud_model
                                .as_ref(ctx)
                                .trashed_cloud_object_types_in_location_with_descendants(
                                    CloudObjectLocation::Folder(*folder_id),
                                    ctx,
                                );
                            let notebooks_in_trashed_folder = trashed_object_types
                                .clone()
                                .into_iter()
                                .filter(|object_type| *object_type == ObjectType::Notebook)
                                .count();

                            // Check # of notebooks in the trashed folder and make sure they are within limits
                            if !UserWorkspaces::has_capacity_for_shared_notebooks(
                                team_uid,
                                ctx,
                                notebooks_in_trashed_folder,
                            ) {
                                // If team has reached the limit for notebooks, show the modal
                                // and return early.
                                ctx.emit(DriveIndexEvent::OpenSharedObjectsCreationDeniedModal(
                                    DriveObjectType::Notebook {
                                        is_ai_document: false,
                                    },
                                    team_uid,
                                ));
                                return;
                            }

                            // Check # of workflows in the trashed folder and make sure they are within limits
                            let workflows_in_trashed_folder = trashed_object_types
                                .into_iter()
                                .filter(|object_type| *object_type == ObjectType::Workflow)
                                .count();
                            if !UserWorkspaces::has_capacity_for_shared_workflows(
                                team_uid,
                                ctx,
                                workflows_in_trashed_folder,
                            ) {
                                // If team has reached the limit for workflows, show the modal
                                // and return early.
                                ctx.emit(DriveIndexEvent::OpenSharedObjectsCreationDeniedModal(
                                    DriveObjectType::Workflow,
                                    team_uid,
                                ));
                                return;
                            }
                        }
                        _ => (),
                    }
                }
                Space::Personal => match cloud_object_type_and_id {
                    CloudObjectTypeAndId::Notebook(_) => {
                        if has_feature_gated_anonymous_user_reached_notebook_limit(ctx) {
                            return;
                        }
                    }
                    CloudObjectTypeAndId::Workflow(_) => {
                        if has_feature_gated_anonymous_user_reached_workflow_limit(ctx) {
                            return;
                        }
                    }
                    CloudObjectTypeAndId::GenericStringObject {
                        object_type:
                            GenericStringObjectFormat::Json(JsonObjectType::EnvVarCollection),
                        id: _,
                    } => {
                        if has_feature_gated_anonymous_user_reached_env_var_limit(ctx) {
                            return;
                        }
                    }
                    _ => {}
                },
                // We have to rely on server checks here, since the client doesn't know how many
                // objects are in the owning drive.
                Space::Shared => (),
            }
        }

        // Otherwise allow object untrash to go through.
        UpdateManager::handle(ctx).update(ctx, move |update_manager, ctx| {
            update_manager.untrash_object(*cloud_object_type_and_id, ctx);
        });
        self.reset_menus(ctx);
        ctx.notify();
    }

    fn delete_object(
        &mut self,
        cloud_object_type_and_id: &CloudObjectTypeAndId,
        ctx: &mut ViewContext<Self>,
    ) {
        UpdateManager::handle(ctx).update(ctx, move |update_manager, ctx| {
            update_manager.delete_object_by_user(*cloud_object_type_and_id, ctx);
        });
        self.reset_menus(ctx);
        ctx.notify();
    }

    fn empty_trash(&mut self, space: &Space, ctx: &mut ViewContext<Self>) {
        UpdateManager::handle(ctx).update(ctx, move |update_manager, ctx| {
            update_manager.empty_trash(*space, ctx);
        });
        self.reset_menus(ctx);
        ctx.notify();
    }

    fn toggle_new_assets_menu(&mut self, space: &Space, ctx: &mut ViewContext<Self>) {
        let section_key = DriveIndexSection::Space(*space);
        let was_open = self
            .section_states
            .get(&section_key)
            .map(|space| space.menu_open)
            .unwrap_or_default();
        self.reset_menus(ctx);

        if was_open {
            ctx.focus_self();
            ctx.notify();
            return;
        }

        let is_online = self.is_online(ctx);
        let state = self.section_states.get_mut(&section_key);
        if let Some(state) = state {
            let mut menu_items = vec![];

            if is_online {
                menu_items.push(
                    MenuItemFields::new(FOLDER_LABEL)
                        .with_on_select_action(DriveIndexAction::create_object(
                            DriveObjectType::Folder,
                            *space,
                            None,
                        ))
                        .with_icon(Icon::Folder)
                        .into_item(),
                );
            }

            menu_items.push(
                MenuItemFields::new(WORKFLOW_LABEL)
                    .with_on_select_action(DriveIndexAction::create_object(
                        DriveObjectType::Workflow,
                        *space,
                        None,
                    ))
                    .with_icon(Icon::Workflow)
                    .into_item(),
            );

            if FeatureFlag::AgentModeWorkflows.is_enabled() {
                menu_items.push(
                    MenuItemFields::new(AGENT_MODE_WORKFLOW_LABEL)
                        .with_on_select_action(DriveIndexAction::create_object(
                            DriveObjectType::AgentModeWorkflow,
                            *space,
                            None,
                        ))
                        .with_icon(Icon::Prompt)
                        .into_item(),
                );
            }

            menu_items.push(
                MenuItemFields::new(NOTEBOOK_LABEL)
                    .with_on_select_action(DriveIndexAction::create_object(
                        DriveObjectType::Notebook {
                            is_ai_document: false,
                        },
                        *space,
                        None,
                    ))
                    .with_icon(Icon::Notebook)
                    .into_item(),
            );

            menu_items.push(
                MenuItemFields::new(ENV_VAR_COLLECTION_LABEL)
                    .with_on_select_action(DriveIndexAction::create_object(
                        DriveObjectType::EnvVarCollection,
                        *space,
                        None,
                    ))
                    .with_icon(Icon::EnvVarCollection)
                    .into_item(),
            );

            menu_items.push(
                MenuItemFields::new(IMPORT_LABEL)
                    .with_on_select_action(DriveIndexAction::OpenImportModal {
                        space: *space,
                        initial_folder_id: None,
                    })
                    .with_icon(Icon::Import)
                    .into_item(),
            );

            ctx.update_view(&self.menu, |menu, ctx| {
                menu.set_items(menu_items, ctx);
            });
            state.menu_open = true;
            ctx.focus(&self.menu);
            ctx.notify();
        }
    }

    fn update_sorting_choice(
        &mut self,
        sorting_choice: &DriveSortOrder,
        ctx: &mut ViewContext<Self>,
    ) {
        self.sorting_choice = *sorting_choice;
        self.initialize_section_states(ctx);
        ctx.notify();

        WarpDriveSettings::handle(ctx).update(ctx, |settings, ctx| {
            report_if_error!(settings.sorting_choice.set_value(*sorting_choice, ctx));
        });

        send_telemetry_from_ctx!(
            TelemetryEvent::UpdateSortingChoice {
                sorting_choice: *sorting_choice
            },
            ctx
        );
    }

    fn toggle_sorting_menu(&mut self, ctx: &mut ViewContext<Self>) {
        let was_open = self.sorting_button_menu_open;
        // If the sorting menu is already open, this will close it
        self.reset_menus(ctx);

        if was_open {
            ctx.focus_self();
        } else {
            self.render_sorting_menu(ctx);
            self.sorting_button_menu_open = true;
            ctx.focus(&self.menu);
        }

        ctx.notify();
    }

    fn render_sorting_menu(&mut self, ctx: &mut ViewContext<Self>) {
        let global_sort_orders = vec![
            DriveSortOrder::AlphabeticalDescending,
            DriveSortOrder::AlphabeticalAscending,
            DriveSortOrder::ByTimestamp,
            DriveSortOrder::ByObjectType,
        ];

        let mut menu_items = vec![];
        for sort_order in global_sort_orders {
            let mut menu_item = MenuItemFields::new(sort_order.menu_text(self.index_variant))
                .with_on_select_action(DriveIndexAction::UpdateSortingChoice {
                    sorting_choice: sort_order,
                });

            if sort_order == self.sorting_choice {
                menu_item = menu_item.with_icon(Icon::Check);
            } else {
                menu_item = menu_item.with_indent();
            }

            menu_items.push(menu_item.into_item());
        }

        ctx.update_view(&self.menu, |menu, ctx| {
            menu.set_items(menu_items, ctx);
        });
    }

    fn render_as_drop_target(
        &self,
        inner_element: Box<dyn Element>,
        location: CloudObjectLocation,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let drop_target = Container::new(DropTarget::new(inner_element, location).finish());

        let drop_target = if Some(location) == self.current_drop_target {
            drop_target
                .with_background(appearance.theme().surface_3())
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
                .finish()
        } else {
            drop_target.finish()
        };

        drop_target
    }

    fn retry_failed_object(
        &mut self,
        cloud_object_type_and_id: &CloudObjectTypeAndId,
        ctx: &mut ViewContext<Self>,
    ) {
        UpdateManager::handle(ctx).update(ctx, |update_manager, ctx| {
            update_manager.resync_object(cloud_object_type_and_id, ctx);
        });
    }

    fn retry_all_failed(&mut self, ctx: &mut ViewContext<Self>) {
        CloudModel::handle(ctx).update(ctx, |cloud_model, ctx| {
            for object in cloud_model.cloud_objects_mut() {
                if object.metadata().is_errored() {
                    let queue_item = object
                        .create_object_queue_item(
                            CloudObjectEventEntrypoint::default(),
                            // When adding the initiated_by parameter to this function call, InitiatedBy::User was set as a default value.
                            // It can be changed to InitiatedBy::System if this action was automatically kicked off and does not require toasts to notify the user of completion.
                            InitiatedBy::User,
                        )
                        .unwrap_or(object.update_object_queue_item(None));
                    object.set_pending_content_changes_status(CloudObjectSyncStatus::InFlight(
                        NumInFlightRequests(1),
                    ));
                    SyncQueue::handle(ctx).update(ctx, |sync_queue, ctx| {
                        sync_queue.enqueue(queue_item, ctx);
                    });
                    self.num_errored_objects -= 1;
                }
            }
        });
    }

    fn revert_failed_object(&mut self, server_id: &ServerId, ctx: &mut ViewContext<Self>) {
        UpdateManager::handle(ctx).update(ctx, |update_manager, ctx| {
            let fetch_cloud_object_rx = update_manager.fetch_single_cloud_object(
                server_id,
                FetchSingleObjectOption::ForceOverwrite,
                ctx,
            );
            // Don't need to wait for the fetch to complete, so drop the receiver
            std::mem::drop(fetch_cloud_object_rx);
        });
    }

    fn dismiss_personal_object_limit_status(&mut self, ctx: &mut ViewContext<Self>) {
        self.should_show_personal_object_limit_status = false;
        ctx.notify();
    }

    fn render_personal_limit_status(
        &self,
        appearance: &Appearance,
        ctx: &AppContext,
    ) -> Option<Box<dyn Element>> {
        let personal_object_limits = self.auth_state.personal_object_limits()?;

        let num_workflows = CloudModel::as_ref(ctx)
            .active_non_welcome_workflows_in_space(Space::Personal, ctx)
            .count();
        let num_notebooks = CloudModel::as_ref(ctx)
            .active_non_welcome_notebooks_in_space(Space::Personal, ctx)
            .count();
        let num_env_var_collections = CloudModel::as_ref(ctx)
            .active_non_welcome_env_var_collections_in_space(Space::Personal, ctx)
            .count();

        let theme = appearance.theme();
        let background_color = theme.surface_2();
        let border_color = theme.outline().into();
        let sub_text_color = blended_colors::text_sub(theme, background_color);

        let close_icon_button = Hoverable::new(
            self.mouse_state_handles
                .anonymous_object_limit_close_button_mouse_state
                .clone(),
            |_| {
                ConstrainedBox::new(
                    Icon::X
                        .to_warpui_icon(appearance.theme().main_text_color(background_color))
                        .finish(),
                )
                .with_width(12.)
                .with_height(12.)
                .finish()
            },
        )
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(DriveIndexAction::DismissPersonalObjectLimits)
        })
        .with_cursor(Cursor::PointingHand)
        .finish();

        let header = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_child(
                Text::new_inline("Warp Drive".to_string(), appearance.ui_font_family(), 14.)
                    .with_color(theme.main_text_color(background_color).into())
                    .with_style(Properties {
                        weight: warpui::fonts::Weight::Bold,
                        ..Default::default()
                    })
                    .finish(),
            )
            .with_child(Shrinkable::new(1., Empty::new().finish()).finish())
            .with_child(close_icon_button)
            .finish();

        let personal_object_limit_description =
            "Sign up for free to increase your storage limit and unlock more features.";

        let body_text = appearance
            .ui_builder()
            .wrappable_text(personal_object_limit_description, true)
            .with_style(UiComponentStyles {
                font_size: Some(12.),
                font_color: Some(sub_text_color),
                ..Default::default()
            })
            .build()
            .finish();

        let workflow_usage = Container::new(self.render_personal_object_limit_row(
            appearance,
            DriveObjectType::Workflow,
            num_workflows,
            personal_object_limits.workflow_limit,
        ))
        .with_margin_bottom(8.)
        .finish();

        let notebook_usage = Container::new(self.render_personal_object_limit_row(
            appearance,
            DriveObjectType::Notebook {
                is_ai_document: false,
            },
            num_notebooks,
            personal_object_limits.notebook_limit,
        ))
        .with_margin_bottom(8.)
        .finish();

        let env_var_usage = self.render_personal_object_limit_row(
            appearance,
            DriveObjectType::EnvVarCollection,
            num_env_var_collections,
            personal_object_limits.env_var_limit,
        );

        let usage_section = Container::new(
            Flex::column()
                .with_children([workflow_usage, notebook_usage, env_var_usage])
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .finish(),
        )
        .with_background(theme.surface_3())
        .with_border(Border::all(1.).with_border_color(border_color))
        .with_uniform_padding(8.)
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
        .finish();

        let button_styles = UiComponentStyles {
            font_size: Some(14.),
            font_family_id: Some(appearance.ui_font_family()),
            font_color: Some(
                appearance
                    .theme()
                    .main_text_color(appearance.theme().accent())
                    .into(),
            ),
            font_weight: Some(Weight::Bold),
            padding: Some(Coords {
                top: 8.,
                bottom: 8.,
                left: 64.,
                right: 64.,
            }),
            border_color: Some(appearance.theme().outline().into()),
            background: Some(appearance.theme().accent().into()),
            ..Default::default()
        };

        let hovered_and_clicked_styles = UiComponentStyles {
            background: Some(internal_colors::accent_bg_strong(appearance.theme()).into()),
            ..button_styles
        };

        let button = appearance
            .ui_builder()
            .button(
                ButtonVariant::Basic,
                self.mouse_state_handles
                    .anonymous_sign_up_button_mouse_state
                    .clone(),
            )
            .with_style(button_styles)
            .with_hovered_styles(hovered_and_clicked_styles)
            .with_active_styles(hovered_and_clicked_styles)
            .with_centered_text_label("Sign up".to_string())
            .build()
            .on_click(|ctx, _, _| ctx.dispatch_typed_action(DriveIndexAction::SignupAnonymousUser))
            .with_cursor(Cursor::PointingHand)
            .finish();

        Some(
            ConstrainedBox::new(
                Container::new(
                    Flex::column()
                        .with_main_axis_alignment(MainAxisAlignment::Start)
                        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                        .with_child(header)
                        .with_child(
                            Container::new(body_text)
                                .with_margin_top(4.)
                                .with_margin_bottom(12.)
                                .finish(),
                        )
                        .with_child(usage_section)
                        .with_child(Container::new(button).with_margin_top(12.).finish())
                        .finish(),
                )
                .with_background(background_color)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
                .with_uniform_padding(12.)
                .with_uniform_margin(12.)
                .with_border(Border::all(1.).with_border_color(border_color))
                .finish(),
            )
            .with_max_width(350.)
            .finish(),
        )
    }

    fn render_personal_object_limit_row(
        &self,
        appearance: &Appearance,
        object_type: DriveObjectType,
        amount: usize,
        max_amount: usize,
    ) -> Box<dyn Element> {
        let main_text_color = appearance
            .theme()
            .main_text_color(appearance.theme().surface_3())
            .into();
        let sub_text_color = appearance
            .theme()
            .hint_text_color(appearance.theme().surface_3())
            .into();

        let text_color = match amount {
            0 => sub_text_color,
            _ => main_text_color,
        };

        let name = match object_type {
            DriveObjectType::Notebook { .. } => "Notebooks",
            DriveObjectType::Workflow => "Workflows",
            DriveObjectType::EnvVarCollection => "Environment Variables",
            DriveObjectType::Folder => "Folders",
            DriveObjectType::AgentModeWorkflow => "Agent Workflows",
            DriveObjectType::AIFact => "AI Fact",
            DriveObjectType::AIFactCollection => "Rules",
            DriveObjectType::MCPServer => "MCP Server",
            DriveObjectType::MCPServerCollection => "MCP Servers",
        };
        let name_styles = UiComponentStyles {
            font_family_id: Some(appearance.ui_font_family()),
            font_size: Some(12.),
            font_color: Some(text_color),
            ..Default::default()
        };

        let remaining = format!("{amount}/{max_amount}");
        let remaining_styles = UiComponentStyles {
            font_family_id: Some(appearance.monospace_font_family()),
            font_size: Some(12.),
            font_color: Some(text_color),
            ..Default::default()
        };

        Flex::row()
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_main_axis_size(MainAxisSize::Max)
            .with_child(
                appearance
                    .ui_builder()
                    .span(name)
                    .with_style(name_styles)
                    .build()
                    .finish(),
            )
            .with_child(
                appearance
                    .ui_builder()
                    .span(remaining)
                    .with_style(remaining_styles)
                    .build()
                    .finish(),
            )
            .finish()
    }

    fn render_shared_object_limit_hit_banner(
        &self,
        appearance: &Appearance,
        team_uid: ServerId,
        object_type: ObjectType,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let background_color = theme.surface_2();

        let highlight =
            Highlight::new().with_properties(Properties::default().weight(Weight::Bold));

        let banner_line_1 = format!("You've run out of {object_type}s on your plan.");
        let body = Container::new(
            appearance
                .ui_builder()
                .wrappable_text(
                    format!("{banner_line_1} {SHARED_OBJECT_LIMIT_HIT_BANNER_LINE}"),
                    true,
                )
                .with_highlights((0..banner_line_1.len()).collect::<Vec<_>>(), highlight)
                .with_style(UiComponentStyles {
                    font_size: Some(12.),
                    font_color: Some(appearance.theme().main_text_color(background_color).into()),
                    ..Default::default()
                })
                .build()
                .finish(),
        )
        .with_margin_bottom(16.)
        .finish();

        let button = appearance
            .ui_builder()
            .button(
                ButtonVariant::Accent,
                self.mouse_state_handles
                    .shared_object_limit_hit_banner_button_mouse_state
                    .clone(),
            )
            .with_centered_text_label("Compare plans".into())
            .with_style(UiComponentStyles {
                font_size: Some(14.),
                font_weight: Some(Weight::Light),
                padding: Some(Coords {
                    top: 8.,
                    bottom: 8.,
                    left: 12.,
                    right: 12.,
                }),
                ..Default::default()
            })
            .build()
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(DriveIndexAction::ViewPlans { team_uid })
            })
            .finish();

        Container::new(
            Container::new(
                Flex::column()
                    .with_main_axis_alignment(MainAxisAlignment::Center)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_child(body)
                    .with_child(button)
                    .finish(),
            )
            .with_background(background_color)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
            .with_uniform_padding(16.)
            .finish(),
        )
        .with_uniform_padding(8.)
        .with_border(Border::top(1.).with_border_color(background_color.into()))
        .finish()
    }

    fn render_payment_issue_banner(
        &self,
        appearance: &Appearance,
        team_uid: ServerId,
        has_admin_permissions: bool,
        is_on_stripe_paid_plan: bool,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let background_color = theme.surface_2();

        let mut body = Flex::column()
            .with_main_axis_alignment(MainAxisAlignment::Center)
            .with_cross_axis_alignment(CrossAxisAlignment::Center);

        let highlight =
            Highlight::new().with_properties(Properties::default().weight(Weight::Bold));

        let banner_line_2 = if has_admin_permissions && is_on_stripe_paid_plan {
            PAYMENT_ISSUE_BANNER_LINE_2_ADMIN
        } else if has_admin_permissions && !is_on_stripe_paid_plan {
            PAYMENT_ISSUE_BANNER_LINE_2_ADMIN_ENTERPRISE
        } else {
            PAYMENT_ISSUE_BANNER_LINE_2_NONADMIN
        };

        body.add_child(
            Container::new(
                appearance
                    .ui_builder()
                    .wrappable_text(
                        format!("{PAYMENT_ISSUE_BANNER_LINE_1} {banner_line_2}").to_string(),
                        true,
                    )
                    .with_highlights(
                        (0..PAYMENT_ISSUE_BANNER_LINE_1.len()).collect::<Vec<_>>(),
                        highlight,
                    )
                    .with_style(UiComponentStyles {
                        font_size: Some(12.),
                        font_color: Some(
                            appearance.theme().main_text_color(background_color).into(),
                        ),
                        ..Default::default()
                    })
                    .build()
                    .finish(),
            )
            .finish(),
        );

        // Only show a manage billing button if they are an admin and on a paid stripe plan
        if has_admin_permissions && is_on_stripe_paid_plan {
            let button = appearance
                .ui_builder()
                .button(
                    ButtonVariant::Accent,
                    self.mouse_state_handles
                        .payment_issue_banner_button_mouse_state
                        .clone(),
                )
                .with_centered_text_label("Manage billing".into())
                .with_style(UiComponentStyles {
                    font_size: Some(14.),
                    font_weight: Some(Weight::Light),
                    padding: Some(Coords {
                        top: 8.,
                        bottom: 8.,
                        left: 12.,
                        right: 12.,
                    }),
                    ..Default::default()
                })
                .build()
                .with_cursor(Cursor::PointingHand)
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(DriveIndexAction::ManageBilling { team_uid })
                })
                .finish();
            body.add_child(Container::new(button).with_margin_top(16.).finish());
        }

        Container::new(
            Container::new(body.finish())
                .with_background(background_color)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                .with_uniform_padding(16.)
                .finish(),
        )
        .with_uniform_padding(8.)
        .with_border(Border::top(1.).with_border_color(background_color.into()))
        .finish()
    }

    fn menu_items(
        &self,
        space: &Space,
        warp_drive_item_id: &WarpDriveItemId,
        app: &AppContext,
    ) -> Vec<MenuItem<DriveIndexAction>> {
        match self.index_variant {
            DriveIndexVariant::MainIndex => self.index_menu_items(space, warp_drive_item_id, app),
            DriveIndexVariant::Trash => self.trash_menu_items(space, warp_drive_item_id, app),
        }
    }

    /// This function sets the items for the context menu in an individual index row.
    fn index_menu_items(
        &self,
        space: &Space,
        warp_drive_item_id: &WarpDriveItemId,
        app: &AppContext,
    ) -> Vec<MenuItem<DriveIndexAction>> {
        let mut menu_items = Vec::new();
        let WarpDriveItemId::Object(cloud_object_type_and_id) = warp_drive_item_id else {
            return menu_items;
        };
        let can_move_or_trash = self.online_only_operation_allowed(cloud_object_type_and_id, app);
        let cloud_view_model = CloudViewModel::as_ref(app);
        let access_level = cloud_view_model.access_level(&cloud_object_type_and_id.uid(), app);
        let editability = cloud_view_model.object_editability(&cloud_object_type_and_id.uid(), app);
        let object = CloudModel::as_ref(app).get_by_uid(&cloud_object_type_and_id.uid());

        if let CloudObjectTypeAndId::Folder(folder_id) = cloud_object_type_and_id {
            if let SyncId::ServerId(_) = folder_id {
                if self.is_online(app) {
                    if !FeatureFlag::SharedWithMe.is_enabled() || editability.can_edit() {
                        menu_items.push(
                            MenuItemFields::new(INDEX_FOLDER_LABEL)
                                .with_on_select_action(DriveIndexAction::create_object(
                                    DriveObjectType::Folder,
                                    *space,
                                    Some(*folder_id),
                                ))
                                .with_icon(Icon::Folder)
                                .into_item(),
                        );
                        menu_items.push(
                            MenuItemFields::new(INDEX_WORKFLOW_LABEL)
                                .with_on_select_action(DriveIndexAction::create_object(
                                    DriveObjectType::Workflow,
                                    *space,
                                    Some(*folder_id),
                                ))
                                .with_icon(Icon::Workflow)
                                .into_item(),
                        );

                        if FeatureFlag::AgentModeWorkflows.is_enabled() {
                            menu_items.push(
                                MenuItemFields::new(INDEX_AGENT_MODE_WORKFLOW_LABEL)
                                    .with_on_select_action(DriveIndexAction::create_object(
                                        DriveObjectType::AgentModeWorkflow,
                                        *space,
                                        Some(*folder_id),
                                    ))
                                    .with_icon(Icon::Prompt)
                                    .into_item(),
                            );
                        }

                        menu_items.push(
                            MenuItemFields::new(INDEX_NOTEBOOK_LABEL)
                                .with_on_select_action(DriveIndexAction::create_object(
                                    DriveObjectType::Notebook {
                                        is_ai_document: false,
                                    },
                                    *space,
                                    Some(*folder_id),
                                ))
                                .with_icon(Icon::Notebook)
                                .into_item(),
                        );

                        menu_items.push(
                            MenuItemFields::new(INDEX_ENV_VAR_COLLECTION_LABEL)
                                .with_on_select_action(DriveIndexAction::create_object(
                                    DriveObjectType::EnvVarCollection,
                                    *space,
                                    Some(*folder_id),
                                ))
                                .with_icon(Icon::EnvVarCollection)
                                .into_item(),
                        );

                        menu_items.push(MenuItem::Separator);
                    }
                    if !FeatureFlag::SharedWithMe.is_enabled() || editability.can_edit() {
                        menu_items.push(
                            MenuItemFields::new("Rename")
                                .with_on_select_action(
                                    DriveIndexAction::OpenCloudObjectNamingDialog {
                                        space: *space,
                                        object_type: DriveObjectType::Folder,
                                        initial_folder_id: Some(*folder_id),
                                        cloud_object_type_and_id: Some(*cloud_object_type_and_id),
                                    },
                                )
                                .with_icon(Icon::Rename)
                                .into_item(),
                        );
                    }
                }

                if let Some(object) = object {
                    if let Some(object_link) = object.object_link() {
                        menu_items.push(
                            MenuItemFields::new("Copy link")
                                .with_on_select_action(DriveIndexAction::CopyObjectLinkToClipboard(
                                    object_link,
                                ))
                                .with_icon(Icon::Link)
                                .into_item(),
                        );
                        if editability.can_edit() {
                            menu_items.push(
                                MenuItemFields::new("Share")
                                    .with_on_select_action(DriveIndexAction::ToggleShareDialog {
                                        warp_drive_item_id: *warp_drive_item_id,
                                    })
                                    .with_icon(Icon::Share)
                                    .into_item(),
                            );
                        }
                    }
                }

                if !FeatureFlag::SharedWithMe.is_enabled() || editability.can_edit() {
                    menu_items.push(
                        MenuItemFields::new(IMPORT_LABEL)
                            .with_on_select_action(DriveIndexAction::OpenImportModal {
                                space: *space,
                                initial_folder_id: Some(*folder_id),
                            })
                            .with_icon(Icon::Import)
                            .into_item(),
                    );
                }
                menu_items.push(
                    MenuItemFields::new("Collapse all")
                        .with_on_select_action(DriveIndexAction::CollapseAllInLocation(
                            CloudObjectLocation::Folder(*folder_id),
                        ))
                        .with_icon(Icon::ListCollapsed)
                        .into_item(),
                );

                if let Some(object) = object {
                    if FeatureFlag::SharedWithMe.is_enabled() && object.can_leave(app) {
                        menu_items.push(
                            MenuItemFields::new(REMOVE_LABEL)
                                .with_on_select_action(DriveIndexAction::LeaveSharedObject {
                                    cloud_object_type_and_id: *cloud_object_type_and_id,
                                })
                                .with_icon(Icon::Minus)
                                .into_item(),
                        )
                    }
                }
            }
        } else {
            if let Some(object) = object {
                if self.is_online(app) && object.metadata().is_errored() {
                    menu_items.push(
                        MenuItemFields::new("Retry")
                            .with_on_select_action(DriveIndexAction::RetryFailedObject(
                                *cloud_object_type_and_id,
                            ))
                            .with_icon(Icon::Refresh)
                            .into_item(),
                    );

                    if let Some(server_id) = cloud_object_type_and_id.server_id() {
                        menu_items.push(
                            MenuItemFields::new("Revert to server")
                                .with_on_select_action(DriveIndexAction::RevertFailedObject(
                                    server_id,
                                ))
                                .with_icon(Icon::ReverseLeft)
                                .into_item(),
                        );
                    }
                }

                let workflow: Option<&CloudWorkflow> = object.into();
                let env_var_collection: Option<&CloudEnvVarCollection> = object.into();

                if self.edit_object_enabled(cloud_object_type_and_id, app) {
                    if let Some(notebook) =
                        <GenericCloudObject<_, CloudNotebookModel> as CloudObject>::as_model_type::<
                            _,
                            CloudNotebookModel,
                        >(object)
                    {
                        if let Some(ai_document_id) = notebook.model().ai_document_id {
                            menu_items.push(
                                MenuItemFields::new("Attach to active session")
                                    .with_on_select_action(DriveIndexAction::AttachPlanAsContext(
                                        ai_document_id,
                                    ))
                                    .with_icon(Icon::Paperclip)
                                    .into_item(),
                            );
                        }
                    }
                    if let Some(_workflow) = workflow {
                        menu_items.push(
                            Self::pane_menu_item(
                                editability,
                                !ContextFlag::RunWorkflow.is_enabled(),
                            )
                            .with_on_select_action(DriveIndexAction::OpenWorkflowInPane {
                                cloud_object_type_and_id: object.cloud_object_type_and_id(),
                                open_mode: if (FeatureFlag::SharedWithMe.is_enabled()
                                    && !editability.can_edit())
                                    || !ContextFlag::RunWorkflow.is_enabled()
                                {
                                    WorkflowViewMode::View
                                } else {
                                    WorkflowViewMode::Edit
                                },
                            })
                            .into_item(),
                        );
                    } else if env_var_collection.is_some() {
                        menu_items.push(
                            Self::pane_menu_item(editability, false)
                                .with_on_select_action(DriveIndexAction::OpenObject(
                                    object.cloud_object_type_and_id(),
                                ))
                                .into_item(),
                        )
                    }
                }
            }

            // Copy workflow text, should appear both online/offline
            // Also adds menu item for loading EnvVars in a subshell
            if let Some(object) = object {
                match object.object_type() {
                    ObjectType::Workflow => {
                        let workflow: Option<&CloudWorkflow> = object.into();
                        let workflow = workflow.expect("Object is workflow");
                        let label = if workflow.model().data.is_agent_mode_workflow() {
                            "Copy prompt"
                        } else {
                            "Copy workflow text"
                        };
                        menu_items.push(
                            MenuItemFields::new(label)
                                .with_on_select_action(DriveIndexAction::CopyObjectToClipboard(
                                    *cloud_object_type_and_id,
                                ))
                                .with_icon(Icon::CopyMenuItem)
                                .into_item(),
                        );
                        if workflow.model().data.is_agent_mode_workflow() {
                            menu_items.push(
                                MenuItemFields::new("Copy id")
                                    .with_on_select_action(DriveIndexAction::CopyWorkflowId(
                                        *cloud_object_type_and_id,
                                    ))
                                    .with_icon(Icon::CopyMenuItem)
                                    .into_item(),
                            );
                        }
                    }
                    ObjectType::GenericStringObject(GenericStringObjectFormat::Json(
                        JsonObjectType::EnvVarCollection,
                    )) => {
                        menu_items.push(
                            MenuItemFields::new("Copy variables")
                                .with_on_select_action(DriveIndexAction::CopyObjectToClipboard(
                                    *cloud_object_type_and_id,
                                ))
                                .with_icon(Icon::CopyMenuItem)
                                .into_item(),
                        );
                        menu_items.push(
                            MenuItemFields::new("Load in subshell")
                                .with_on_select_action(
                                    DriveIndexAction::InvokeEnvVarCollectionInSubshell(
                                        object.cloud_object_type_and_id(),
                                    ),
                                )
                                .with_icon(Icon::Terminal)
                                .into_item(),
                        );
                    }
                    ObjectType::Notebook
                    | ObjectType::Folder
                    | ObjectType::GenericStringObject(_) => (),
                }
            }

            // TODO: move this out of the -else- branch. Right now, we don't support bulk actions.
            match space {
                Space::Personal => {
                    if can_move_or_trash
                        && (!FeatureFlag::SharedWithMe.is_enabled()
                            || access_level.can_move_drive())
                    {
                        menu_items.extend(self.sections.iter().filter_map(|section| {
                            if let DriveIndexSection::Space(space) = section {
                                match space {
                                    Space::Personal | Space::Shared => None,
                                    Space::Team { .. } => Some(
                                        MenuItemFields::new(format!("Move to {}", space.name(app)))
                                            .with_on_select_action(DriveIndexAction::MoveObject {
                                                cloud_object_type_and_id: *cloud_object_type_and_id,
                                                new_space: *space,
                                            })
                                            .with_icon(Icon::Move)
                                            .into_item(),
                                    ),
                                }
                            } else {
                                None
                            }
                        }));
                    }
                }
                Space::Shared => {} // TODO: Revisit these menu items with sharing in mind
                Space::Team { .. } => {} // TODO: When we do team -> personal sharing
            }

            if let Some(object) = object {
                match object.object_type() {
                    ObjectType::Workflow
                    | ObjectType::Notebook
                    | ObjectType::GenericStringObject(GenericStringObjectFormat::Json(
                        JsonObjectType::EnvVarCollection,
                    )) => {
                        if let Some(object_link) = object.object_link() {
                            menu_items.push(
                                MenuItemFields::new("Copy link")
                                    .with_on_select_action(
                                        DriveIndexAction::CopyObjectLinkToClipboard(object_link),
                                    )
                                    .with_icon(Icon::Link)
                                    .into_item(),
                            );
                        }
                        if editability.can_edit() {
                            menu_items.push(
                                MenuItemFields::new("Share")
                                    .with_on_select_action(DriveIndexAction::ToggleShareDialog {
                                        warp_drive_item_id: *warp_drive_item_id,
                                    })
                                    .with_icon(Icon::Share)
                                    .into_item(),
                            );
                        }
                        if !warpui::platform::is_mobile_device()
                            && !ContextFlag::HideOpenOnDesktopButton.is_enabled()
                            && *UserAppInstallDetectionSettings::as_ref(app)
                                .user_app_installation_detected
                                .value()
                                == UserAppInstallStatus::Detected
                        {
                            if let Some(object_link) = object.object_link() {
                                if let Ok(url) = Url::parse(&object_link) {
                                    menu_items.push(
                                        MenuItemFields::new("Open on Desktop")
                                            .with_on_select_action(
                                                DriveIndexAction::OpenObjectLinkOnDesktop(url),
                                            )
                                            .with_icon(Icon::Laptop)
                                            .into_item(),
                                    );
                                }
                            }
                        }
                        // Only allow duplicate if in Personal space, or Team space when online
                        if matches!(space, Space::Personal)
                            || (self.is_online(app) && matches!(space, Space::Team { .. }))
                        {
                            menu_items.push(
                                MenuItemFields::new("Duplicate")
                                    .with_on_select_action(DriveIndexAction::DuplicateObject(
                                        *cloud_object_type_and_id,
                                    ))
                                    .with_icon(Icon::Duplicate)
                                    .into_item(),
                            );
                        }
                    }
                    ObjectType::Folder | ObjectType::GenericStringObject(_) => (),
                }

                #[cfg(feature = "local_fs")]
                if object.can_export() {
                    menu_items.push(
                        MenuItemFields::new("Export")
                            .with_on_select_action(DriveIndexAction::ExportObject(
                                *cloud_object_type_and_id,
                            ))
                            .with_icon(Icon::Download)
                            .into_item(),
                    )
                }

                if FeatureFlag::SharedWithMe.is_enabled() && object.can_leave(app) {
                    menu_items.push(
                        MenuItemFields::new(REMOVE_LABEL)
                            .with_on_select_action(DriveIndexAction::LeaveSharedObject {
                                cloud_object_type_and_id: *cloud_object_type_and_id,
                            })
                            .with_icon(Icon::Minus)
                            .into_item(),
                    )
                }
            }
        }

        if can_move_or_trash
            && (!FeatureFlag::SharedWithMe.is_enabled() || access_level.can_trash())
        {
            menu_items.push(
                MenuItemFields::new("Trash")
                    .with_on_select_action(DriveIndexAction::TrashObject {
                        cloud_object_type_and_id: *cloud_object_type_and_id,
                    })
                    .with_icon(Icon::Trash)
                    .into_item(),
            );
        }

        menu_items
    }

    /// Builder for a menu item to open a Warp Drive object in a pane. The icon and label depend
    /// on whether the object is editable or not.
    ///
    /// If `prefer_open` is `true`, the item defaults to view/open mode rather than edit mode.
    fn pane_menu_item(
        editability: ContentEditability,
        prefer_open: bool,
    ) -> MenuItemFields<DriveIndexAction> {
        if (FeatureFlag::SharedWithMe.is_enabled() && !editability.can_edit()) || prefer_open {
            MenuItemFields::new("Open").with_icon(Icon::Eye)
        } else {
            MenuItemFields::new("Edit").with_icon(Icon::Rename)
        }
    }

    fn trash_menu_items(
        &self,
        _space: &Space,
        warp_drive_item_id: &WarpDriveItemId,
        app: &AppContext,
    ) -> Vec<MenuItem<DriveIndexAction>> {
        let mut menu_items = Vec::new();
        let WarpDriveItemId::Object(cloud_object_type_and_id) = warp_drive_item_id else {
            return menu_items;
        };

        let access_level =
            CloudViewModel::as_ref(app).access_level(&cloud_object_type_and_id.uid(), app);
        let cloud_model = CloudModel::as_ref(app);
        let object = cloud_model.get_by_uid(&cloud_object_type_and_id.uid());

        if let Some(object) = object {
            if self.is_online(app) && object.metadata().is_errored() {
                menu_items.push(
                    MenuItemFields::new("Retry")
                        .with_on_select_action(DriveIndexAction::RetryFailedObject(
                            *cloud_object_type_and_id,
                        ))
                        .with_icon(Icon::Refresh)
                        .into_item(),
                );

                if let Some(server_id) = cloud_object_type_and_id.server_id() {
                    menu_items.push(
                        MenuItemFields::new("Revert to server")
                            .with_on_select_action(DriveIndexAction::RevertFailedObject(server_id))
                            .with_icon(Icon::ReverseLeft)
                            .into_item(),
                    );
                }
            }
        }

        if self.online_only_operation_allowed(cloud_object_type_and_id, app) {
            if !FeatureFlag::SharedWithMe.is_enabled() || access_level.can_trash() {
                menu_items.push(
                    MenuItemFields::new("Restore")
                        .with_on_select_action(DriveIndexAction::UntrashObject {
                            cloud_object_type_and_id: *cloud_object_type_and_id,
                        })
                        .with_icon(Icon::ReverseLeft)
                        .into_item(),
                );
            }
            if !FeatureFlag::SharedWithMe.is_enabled() || access_level.can_delete() {
                menu_items.push(
                    MenuItemFields::new("Delete forever")
                        .with_on_select_action(DriveIndexAction::DeleteObject {
                            cloud_object_type_and_id: *cloud_object_type_and_id,
                        })
                        .with_icon(Icon::AlertTriangle)
                        .into_item(),
                );
            }
        }

        menu_items
    }

    pub fn toggle_item_menu(
        &mut self,
        space: &Space,
        warp_drive_item_id: &WarpDriveItemId,
        ctx: &mut ViewContext<Self>,
    ) {
        let menu_items: Vec<MenuItem<DriveIndexAction>> =
            self.menu_items(space, warp_drive_item_id, ctx);
        ctx.update_view(&self.menu, |menu, ctx| {
            menu.set_items(menu_items, ctx);
        });

        self.menu_object_id_if_open = Some(*warp_drive_item_id);
        ctx.focus(&self.menu);
        ctx.notify();
    }

    pub fn toggle_share_dialog(
        &mut self,
        warp_drive_item_id: &WarpDriveItemId,
        invitee_email: Option<String>,
        source: SharingDialogSource,
        ctx: &mut ViewContext<Self>,
    ) {
        let WarpDriveItemId::Object(cloud_object_type_and_id) = warp_drive_item_id else {
            return;
        };

        if self.auth_state.is_anonymous_or_logged_out() {
            AuthManager::handle(ctx).update(ctx, |auth_manager, ctx| {
                auth_manager.attempt_login_gated_feature(
                    "Share Object",
                    AuthViewVariant::ShareRequirementCloseable,
                    ctx,
                )
            });
            return;
        }

        self.reset_menus(ctx);
        if let Some(server_id) = cloud_object_type_and_id.server_id() {
            self.share_dialog_open_for_object = Some(*warp_drive_item_id);
            self.sharing_dialog.update(ctx, |sharing_dialog, ctx| {
                sharing_dialog.set_target(Some(ShareableObject::WarpDriveObject(server_id)), ctx);
                if let Some(invitee_email) = invitee_email {
                    sharing_dialog.add_invitee_email(invitee_email, ctx);
                }
                sharing_dialog.report_open(source, ctx);
            });
            ctx.focus(&self.sharing_dialog);
        }
        ctx.notify();
    }

    fn toggle_space_menu(&mut self, space: &Space, offset: Vector2F, ctx: &mut ViewContext<Self>) {
        self.space_menu_open_for_space = Some(SpaceMenuState {
            space: *space,
            offset,
        });
        let menu_items = vec![MenuItemFields::new("Collapse all")
            .with_on_select_action(DriveIndexAction::CollapseAllInLocation(
                CloudObjectLocation::Space(*space),
            ))
            .with_icon(Icon::ListCollapsed)
            .into_item()];

        ctx.update_view(&self.menu, |menu, ctx| {
            menu.set_items(menu_items, ctx);
        });

        ctx.focus(&self.menu);
        ctx.notify();
    }

    pub fn reset_menus(&mut self, ctx: &mut ViewContext<Self>) {
        self.section_states.iter_mut().for_each(|(_, state)| {
            state.menu_open = false;
        });
        self.menu_object_id_if_open = None;
        self.sorting_button_menu_open = false;
        self.space_menu_open_for_space = None;
        ctx.notify();
    }

    /// Resets the main index and opens it, ensuring any other index variant is closed
    pub fn reset_and_open_to_main_index(&mut self, ctx: &mut ViewContext<Self>) {
        if self.index_variant != DriveIndexVariant::MainIndex {
            self.index_variant = DriveIndexVariant::MainIndex;
            self.initialize_section_states(ctx);
            ctx.notify();
        }
    }

    pub fn autoscroll(&mut self, delta: f32, ctx: &mut ViewContext<Self>) {
        self.clipped_scroll_state.scroll_by(delta.into_pixels());
        ctx.notify();
    }

    pub fn set_selected_object(
        &mut self,
        id: Option<WarpDriveItemId>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.selected = id;
        ctx.notify();
    }

    /// Executes actions on index items based on key presses, such as
    /// toggling folders, executing workflows, and opening notebooks.
    fn execute_index_item_keyboard_action(
        &mut self,
        key: DriveIndexAction,
        ctx: &mut ViewContext<Self>,
    ) {
        if let Some(focused_index) = self.focused_index {
            if let DriveIndexAction::EscapeKey = key {
                if self.index_variant == DriveIndexVariant::Trash {
                    self.index_variant = DriveIndexVariant::MainIndex;
                    self.initialize_section_states(ctx);
                    self.focused_index = Some(0);
                    ctx.notify();
                }
            }

            let Some(focused_item_id) = self.ordered_items.get(focused_index) else {
                return;
            };
            match focused_item_id {
                WarpDriveItemId::AIFactCollection => {
                    if let DriveIndexAction::EnterKey = key {
                        ctx.emit(DriveIndexEvent::OpenAIFactCollection);
                    }
                }
                WarpDriveItemId::MCPServerCollection => {
                    if let DriveIndexAction::EnterKey = key {
                        ctx.emit(DriveIndexEvent::OpenMCPServerCollection);
                    }
                }
                WarpDriveItemId::Object(cloud_id) => match cloud_id {
                    CloudObjectTypeAndId::Notebook(_) => {
                        if let DriveIndexAction::EnterKey = key {
                            ctx.emit(DriveIndexEvent::OpenObject(*cloud_id))
                        }
                    }
                    CloudObjectTypeAndId::Workflow(_) => {
                        if let DriveIndexAction::EnterKey = key {
                            if !ContextFlag::RunWorkflow.is_enabled() {
                                // if on the web open in view mode by default
                                ctx.emit(DriveIndexEvent::OpenWorkflowInPane {
                                    cloud_object_type_and_id: *cloud_id,
                                    open_mode: WorkflowViewMode::View,
                                })
                            } else {
                                ctx.emit(DriveIndexEvent::RunObject(*cloud_id))
                            }
                        }
                    }
                    CloudObjectTypeAndId::Folder(id) => {
                        CloudModel::handle(ctx).update(ctx, |cloud_model, ctx| match key {
                            DriveIndexAction::EnterKey => {
                                cloud_model.toggle_folder_open(*id, ctx);
                            }
                            DriveIndexAction::LeftArrowKey => cloud_model.close_folder(*id, ctx),
                            DriveIndexAction::RightArrowKey => cloud_model.open_folder(*id, ctx),
                            _ => {}
                        });
                    }
                    CloudObjectTypeAndId::GenericStringObject { object_type, id: _ } => {
                        if let GenericStringObjectFormat::Json(JsonObjectType::EnvVarCollection) =
                            object_type
                        {
                            if let DriveIndexAction::EnterKey = key {
                                ctx.emit(DriveIndexEvent::RunObject(*cloud_id))
                            }
                        }
                    }
                },
                WarpDriveItemId::Space(space) => {
                    let section = &DriveIndexSection::Space(*space);
                    match key {
                        DriveIndexAction::EnterKey => self.toggle_section_collapse(section, ctx),
                        DriveIndexAction::LeftArrowKey => {
                            self.set_section_collapsed_state(section, true, ctx)
                        }
                        DriveIndexAction::RightArrowKey => {
                            self.set_section_collapsed_state(section, false, ctx)
                        }
                        _ => {}
                    }
                }
                WarpDriveItemId::Trash => {
                    if let DriveIndexAction::EnterKey = key {
                        self.index_variant = DriveIndexVariant::Trash;
                        self.initialize_section_states(ctx);
                        self.focused_index = Some(0);
                        ctx.notify();
                    }
                }
            }
        }
    }

    #[cfg(test)]
    pub fn sections(&self) -> &Vec<DriveIndexSection> {
        &self.sections
    }
}

pub fn warp_drive_section_header_position_id(section: &DriveIndexSection) -> String {
    format!("section_position_{section:?}")
}

impl View for DriveIndex {
    fn ui_name() -> &'static str {
        "DriveIndex"
    }

    fn on_blur(&mut self, blur_ctx: &BlurContext, ctx: &mut ViewContext<Self>) {
        if blur_ctx.is_self_blurred() {
            self.set_focused_index(None, false, ctx);
            ctx.notify();
        }
    }

    fn keymap_context(&self, _ctx: &AppContext) -> warpui::keymap::Context {
        let mut context = Self::default_keymap_context();

        // Disable WD Vim keybindings when a dialog is open
        // because it interferes with the ability to type all letters.
        if self.cloud_object_naming_dialog.is_open() || self.share_dialog_open_for_object.is_some()
        {
            context.set.insert("DisableDriveIndexVimKeybindings");
        }

        context
    }

    fn render(&self, app: &AppContext) -> Box<dyn warpui::Element> {
        let appearance = Appearance::as_ref(app);
        let workspaces = UserWorkspaces::as_ref(app);

        // The content of the index is all spaces rendered into a flex column.
        let content = Flex::column().with_children(self.render_all_sections(app));

        let theme = appearance.theme();
        let index = SavePosition::new(
            Container::new(
                ClippedScrollable::vertical(
                    self.clipped_scroll_state.clone(),
                    content.finish(),
                    SCROLLBAR_WIDTH,
                    blended_colors::text_disabled(theme, theme.background()).into(),
                    blended_colors::text_main(theme, theme.background()).into(),
                    theme.surface_1().into(),
                )
                .with_overlayed_scrollbar()
                .finish(),
            )
            .with_margin_top(TAB_BAR_AND_CONTENT_MARGIN)
            .with_margin_left(4.)
            .with_margin_right(INDEX_CONTENT_MARGIN_RIGHT)
            .finish(),
            DRIVE_INDEX_VIEW_POSITION_ID,
        )
        .finish();

        let index_content = if let (true, Some(personal_object_limit_card)) = (
            self.should_show_personal_object_limit_status,
            self.render_personal_limit_status(appearance, app),
        ) {
            // Render column with a spacer to ensure the tip appears at the bottom of drive
            let col = Flex::column()
                .with_child(index)
                .with_child(Shrinkable::new(1., Empty::new().finish()).finish())
                .finish();

            let mut stack = Stack::new().with_constrain_absolute_children();
            stack.add_child(col);
            stack.add_positioned_child(
                personal_object_limit_card,
                OffsetPositioning::offset_from_parent(
                    vec2f(0., 0.),
                    ParentOffsetBounds::WindowByPosition,
                    ParentAnchor::BottomMiddle,
                    ChildAnchor::BottomMiddle,
                ),
            );
            stack.finish()
        } else {
            index
        };

        let mut drive = Flex::column();

        // Only show the workspace picker if they are in multiple workspaces.
        if FeatureFlag::MultiWorkspace.is_enabled() && workspaces.workspaces().len() > 1 {
            drive.add_child(self.render_workspace_picker());
        }

        match self.index_variant {
            DriveIndexVariant::MainIndex => {
                drive.add_child(self.render_title(appearance, app));
                drive.add_child(Shrinkable::new(1., index_content).finish());
            }
            DriveIndexVariant::Trash => {
                drive.add_child(self.render_trash_title(appearance));
                drive.add_child(self.render_deletion_warning(appearance));
                drive.add_child(Shrinkable::new(1., index_content).finish());
            }
        };

        if let Some(team) = workspaces.current_team() {
            if team.billing_metadata.is_delinquent_due_to_payment_issue() {
                let current_user_email = self.auth_state.user_email().unwrap_or_default();
                let has_admin_permissions = team.has_admin_permissions(&current_user_email);
                let is_on_stripe_paid_plan = team.billing_metadata.is_on_stripe_paid_plan();
                drive.add_child(self.render_payment_issue_banner(
                    appearance,
                    team.uid,
                    has_admin_permissions,
                    is_on_stripe_paid_plan,
                ));
            } else if UserWorkspaces::is_at_tier_limit_for_object_type(
                team.uid,
                ObjectType::Workflow,
                app,
            ) {
                drive.add_child(self.render_shared_object_limit_hit_banner(
                    appearance,
                    team.uid,
                    ObjectType::Workflow,
                ));
            } else if UserWorkspaces::is_at_tier_limit_for_object_type(
                team.uid,
                ObjectType::Notebook,
                app,
            ) {
                drive.add_child(self.render_shared_object_limit_hit_banner(
                    appearance,
                    team.uid,
                    ObjectType::Notebook,
                ));
            }
        }

        drive.finish()
    }
}

impl Entity for DriveIndex {
    type Event = DriveIndexEvent;
}

impl TypedActionView for DriveIndex {
    type Action = DriveIndexAction;

    fn handle_action(&mut self, action: &DriveIndexAction, ctx: &mut ViewContext<Self>) {
        // Block anonymous users from performing team actions
        if self.auth_state.is_anonymous_or_logged_out() && action.blocked_for_anonymous_user() {
            AuthManager::handle(ctx).update(ctx, |auth_manager, ctx| {
                auth_manager.attempt_login_gated_feature(
                    action.into(),
                    AuthViewVariant::RequireLoginCloseable,
                    ctx,
                )
            });
            return;
        }

        match action {
            DriveIndexAction::CreateObject {
                object_type,
                space,
                initial_folder_id,
            } => {
                self.create_object(*object_type, *space, *initial_folder_id, ctx);
            }
            DriveIndexAction::CreateWorkflowWithContent {
                space,
                initial_folder_id,
                content,
                is_for_agent_mode,
            } => {
                if has_feature_gated_anonymous_user_reached_workflow_limit(ctx) {
                    return;
                }

                ctx.emit(DriveIndexEvent::CreateWorkflow {
                    space: *space,
                    title: None,
                    initial_folder_id: *initial_folder_id,
                    is_for_agent_mode: *is_for_agent_mode,
                    content: Some(content.clone()),
                });
            }
            DriveIndexAction::OpenImportModal {
                space,
                initial_folder_id,
            } => ctx.emit(DriveIndexEvent::OpenImportModal {
                space: *space,
                initial_folder_id: *initial_folder_id,
            }),
            DriveIndexAction::RenameFolder { folder_id } => {
                self.rename_folder(*folder_id, ctx);
            }
            DriveIndexAction::OpenAIFactCollection => {
                ctx.emit(DriveIndexEvent::OpenAIFactCollection);
            }
            DriveIndexAction::OpenMCPServerCollection => {
                ctx.emit(DriveIndexEvent::OpenMCPServerCollection);
            }
            DriveIndexAction::OpenObject(cloud_object_type_and_id) => {
                if !matches!(self.index_variant, DriveIndexVariant::Trash) {
                    self.set_selected_object(
                        Some(WarpDriveItemId::Object(*cloud_object_type_and_id)),
                        ctx,
                    );
                    ctx.emit(DriveIndexEvent::OpenObject(*cloud_object_type_and_id))
                }
            }
            DriveIndexAction::OpenWorkflowInPane {
                cloud_object_type_and_id,
                open_mode,
            } => {
                if !matches!(self.index_variant, DriveIndexVariant::Trash) {
                    self.set_selected_object(
                        Some(WarpDriveItemId::Object(*cloud_object_type_and_id)),
                        ctx,
                    );
                    ctx.emit(DriveIndexEvent::OpenWorkflowInPane {
                        cloud_object_type_and_id: *cloud_object_type_and_id,
                        open_mode: *open_mode,
                    });
                }
            }
            DriveIndexAction::CopyObjectToClipboard(cloud_object_type_and_id) => {
                send_telemetry_from_ctx!(
                    TelemetryEvent::CopyObjectToClipboard(cloud_object_type_and_id.into()),
                    ctx
                );

                let shell_family =
                    active_terminal_in_window(ctx.window_id(), ctx, |terminal, ctx| {
                        terminal.shell_family(ctx)
                    })
                    .unwrap_or_else(|| OperatingSystem::get().default_shell_family());

                let cloud_model = CloudModel::as_ref(ctx);
                let object = cloud_model.get_by_uid(&cloud_object_type_and_id.uid());

                if let Some(object) = object {
                    match object.object_type() {
                        ObjectType::Workflow => {
                            let workflow: Option<&CloudWorkflow> = object.into();
                            if let Some(workflow) = workflow {
                                let content = workflow.model().data.content().to_owned();
                                ctx.clipboard().write(ClipboardContent::plain_text(content));
                            }
                        }
                        ObjectType::GenericStringObject(GenericStringObjectFormat::Json(
                            JsonObjectType::EnvVarCollection,
                        )) => {
                            let env_var_collection: Option<&CloudEnvVarCollection> = object.into();
                            if let Some(env_var_collection) = env_var_collection {
                                let vars = env_var_collection
                                    .model()
                                    .string_model
                                    .export_variables(" ", shell_family);
                                ctx.clipboard().write(ClipboardContent::plain_text(vars));
                            }
                        }
                        ObjectType::Notebook
                        | ObjectType::Folder
                        | ObjectType::GenericStringObject(_) => (),
                    }
                }
            }
            DriveIndexAction::CopyWorkflowId(cloud_object_type_and_id) => {
                let workflow_id = cloud_object_type_and_id.uid();
                ctx.clipboard()
                    .write(ClipboardContent::plain_text(workflow_id));
            }
            DriveIndexAction::DuplicateObject(cloud_object_type_and_id) => {
                send_telemetry_from_ctx!(
                    TelemetryEvent::DuplicateObject(cloud_object_type_and_id.into()),
                    ctx
                );
                ctx.emit(DriveIndexEvent::DuplicateObject(*cloud_object_type_and_id));
            }
            DriveIndexAction::ExportObject(type_and_id) => {
                send_telemetry_from_ctx!(TelemetryEvent::ExportObject(type_and_id.into()), ctx);
                ctx.emit(DriveIndexEvent::ExportObject(*type_and_id));
            }
            DriveIndexAction::ToggleNewAssetsMenu(space) => {
                self.toggle_new_assets_menu(space, ctx);
            }
            DriveIndexAction::ToggleSortingMenu => {
                self.toggle_sorting_menu(ctx);
            }
            DriveIndexAction::ToggleItemOverflowMenu {
                space,
                warp_drive_item_id,
            } => {
                self.toggle_item_menu(space, warp_drive_item_id, ctx);
            }
            DriveIndexAction::ToggleSpaceOverflowMenu { space, offset } => {
                self.toggle_space_menu(space, *offset, ctx);
            }
            DriveIndexAction::OpenCloudObjectNamingDialog {
                object_type,
                space,
                initial_folder_id,
                cloud_object_type_and_id,
            } => {
                self.reset_menus(ctx);

                // If attempting to rename a folder, we can start with the existing name.
                let existing_name = cloud_object_type_and_id.and_then(|id| {
                    let model: &CloudModel = CloudModel::as_ref(ctx);
                    match id {
                        CloudObjectTypeAndId::Folder(folder_id) => {
                            model.get_folder(&folder_id).map(|f| f.model().name.clone())
                        }
                        CloudObjectTypeAndId::Notebook(notebook_id) => model
                            .get_notebook(&notebook_id)
                            .map(|n| n.model().title.clone()),
                        _ => None,
                    }
                });

                let is_rename = cloud_object_type_and_id.is_some();
                match *object_type {
                    DriveObjectType::Notebook { .. } | DriveObjectType::Folder => {
                        self.cloud_object_naming_dialog.open(
                            *object_type,
                            *space,
                            *initial_folder_id,
                            is_rename,
                            existing_name,
                            ctx,
                        );
                        ctx.focus(&self.cloud_object_naming_dialog.title_editor);
                    }
                    DriveObjectType::Workflow | DriveObjectType::AgentModeWorkflow => {
                        log::error!(
                            "Use DriveIndexAction::OpenWorkflowModal to open the modal instead"
                        )
                    }
                    DriveObjectType::EnvVarCollection => {
                        log::error!("Creation of EnvVarCollections is not yet supported")
                    }
                    DriveObjectType::AIFact | DriveObjectType::AIFactCollection => {
                        log::error!("Use DriveIndexAction::OpenAIFactCollection to open the pane view instead");
                    }
                    DriveObjectType::MCPServer | DriveObjectType::MCPServerCollection => {
                        log::error!(
                            "Use DriveIndexAction::OpenMCPServerCollection to open the pane view instead"
                        );
                    }
                }

                ctx.notify();
            }
            DriveIndexAction::LeaveSharedObject {
                cloud_object_type_and_id,
            } => {
                self.leave_object(cloud_object_type_and_id, ctx);
            }
            DriveIndexAction::CloseCloudObjectNamingDialog => {
                self.cloud_object_naming_dialog.close(ctx);
                ctx.notify();
            }
            DriveIndexAction::MoveObject {
                cloud_object_type_and_id,
                new_space,
            } => self.move_object(
                cloud_object_type_and_id,
                CloudObjectLocation::Space(*new_space),
                ctx,
            ),
            DriveIndexAction::DropIndexItem {
                cloud_object_type_and_id,
                drop_target_location,
            } => {
                self.move_object(cloud_object_type_and_id, *drop_target_location, ctx);
            }
            DriveIndexAction::UpdateCurrentDropTarget {
                drop_target_location,
            } => {
                self.update_drop_target_location(*drop_target_location, ctx);
            }
            DriveIndexAction::ClearDropTarget => self.clear_drop_target(ctx),
            DriveIndexAction::ToggleSectionCollapsed(section) => {
                self.toggle_section_collapse(section, ctx);
            }
            DriveIndexAction::OpenTeamSettingsPage => {
                ctx.emit(DriveIndexEvent::OpenTeamSettingsPage);
            }
            DriveIndexAction::RunObject(id) => {
                if !matches!(self.index_variant, DriveIndexVariant::Trash) {
                    ctx.emit(DriveIndexEvent::RunObject(*id));
                }
            }
            DriveIndexAction::OpenWorkflowModalWithNew {
                space,
                initial_folder_id,
            } => ctx.emit(DriveIndexEvent::OpenWorkflowModalWithNew {
                space: *space,
                initial_folder_id: *initial_folder_id,
            }),
            DriveIndexAction::OpenWorkflowModalWithCloudWorkflow(workflow_id) => {
                ctx.emit(DriveIndexEvent::OpenWorkflowModalWithCloudWorkflow(
                    *workflow_id,
                ));
            }
            DriveIndexAction::ToggleFolderOpen(id) => {
                // If WD is focused, then clicking a folder will set that folder to be focused
                if self.focused_index.is_some() {
                    self.set_focused_item(
                        WarpDriveItemId::Object(CloudObjectTypeAndId::Folder(*id)),
                        true,
                        ctx,
                    );
                }
                CloudModel::handle(ctx).update(ctx, |cloud_model, ctx| {
                    cloud_model.toggle_folder_open(*id, ctx);
                });
            }
            DriveIndexAction::CollapseAllInLocation(location) => {
                CloudModel::handle(ctx).update(ctx, |cloud_model, ctx| {
                    cloud_model.collapse_all_in_location(*location, self.index_variant, ctx);
                });
            }
            DriveIndexAction::TrashObject {
                cloud_object_type_and_id,
            } => {
                self.trash_object(*cloud_object_type_and_id, ctx);
            }
            DriveIndexAction::UntrashObject {
                cloud_object_type_and_id,
            } => {
                self.untrash_object(cloud_object_type_and_id, ctx);
            }
            DriveIndexAction::DeleteObject {
                cloud_object_type_and_id,
            } => {
                self.delete_object(cloud_object_type_and_id, ctx);
            }
            DriveIndexAction::EmptyTrash { space } => {
                self.empty_trash(space, ctx);
            }
            DriveIndexAction::OpenEmptyTrashConfirmationDialog { space } => {
                ctx.focus(&self.empty_trash_confirmation_dialog);
                ctx.notify();
                self.empty_trash_confirmation_dialog_space = Some(*space);
            }
            DriveIndexAction::Autoscroll { delta } => {
                self.autoscroll(*delta, ctx);
            }
            DriveIndexAction::UpdateSortingChoice { sorting_choice } => {
                self.update_sorting_choice(sorting_choice, ctx);
            }
            DriveIndexAction::RetryFailedObject(cloud_object_type_and_id) => {
                self.retry_failed_object(cloud_object_type_and_id, ctx);
            }
            DriveIndexAction::RetryAllFailedObjects => {
                self.retry_all_failed(ctx);
            }
            DriveIndexAction::RevertFailedObject(server_id) => {
                self.revert_failed_object(server_id, ctx);
            }
            DriveIndexAction::OpenTrashIndex => {
                self.index_variant = DriveIndexVariant::Trash;
                self.initialize_section_states(ctx);
                ctx.notify();
            }
            DriveIndexAction::CloseTrashIndex => {
                self.index_variant = DriveIndexVariant::MainIndex;
                self.initialize_section_states(ctx);
                ctx.notify();
            }
            DriveIndexAction::FocusPreviousItem => {
                if let Some(current_focused_index) = self.focused_index {
                    if current_focused_index > 0 {
                        self.set_focused_index(Some(current_focused_index - 1), true, ctx);
                    }
                }
            }
            DriveIndexAction::FocusNextItem => {
                if let Some(current_focused_index) = self.focused_index {
                    if current_focused_index < self.ordered_items.len() - 1 {
                        self.set_focused_index(Some(current_focused_index + 1), true, ctx);
                    }
                }
            }
            DriveIndexAction::LeftArrowKey => {
                self.execute_index_item_keyboard_action(DriveIndexAction::LeftArrowKey, ctx);
            }
            DriveIndexAction::RightArrowKey => {
                self.execute_index_item_keyboard_action(DriveIndexAction::RightArrowKey, ctx);
            }
            DriveIndexAction::EnterKey => {
                self.execute_index_item_keyboard_action(DriveIndexAction::EnterKey, ctx);
            }
            DriveIndexAction::EscapeKey => {
                self.execute_index_item_keyboard_action(DriveIndexAction::EscapeKey, ctx);
            }
            DriveIndexAction::ToggleDriveItemContextMenu => {
                if let Some(focused_index) = self.focused_index {
                    if let Some(&warp_drive_item_id) = self.ordered_items.get(focused_index) {
                        // Retrieve space of the WD item (because context menu options depend on the space)
                        // by finding the last space before the focused item in ordered_items
                        if let Some(space) = self
                            .ordered_items
                            .iter()
                            .take(focused_index)
                            .filter_map(|id| {
                                if let WarpDriveItemId::Space(space) = *id {
                                    Some(space)
                                } else {
                                    None
                                }
                            })
                            .next_back()
                        {
                            self.toggle_item_menu(&space, &warp_drive_item_id, ctx);
                        }
                    }
                }
            }
            DriveIndexAction::CopyObjectLinkToClipboard(link) => {
                send_telemetry_from_ctx!(
                    TelemetryEvent::ObjectLinkCopied { link: link.clone() },
                    ctx
                );
                ctx.clipboard()
                    .write(ClipboardContent::plain_text(link.to_owned()));
            }
            #[cfg(target_family = "wasm")]
            DriveIndexAction::OpenObjectLinkOnDesktop(url) => {
                open_url_on_desktop(url);
            }
            #[cfg(not(target_family = "wasm"))]
            DriveIndexAction::OpenObjectLinkOnDesktop(_) => {
                // No-op when not on wasm
            }
            DriveIndexAction::InvokeEnvVarCollectionInSubshell(id) => {
                ctx.emit(DriveIndexEvent::InvokeEnvVarCollectionInSubshell(*id))
            }
            DriveIndexAction::ViewPlans { team_uid } => {
                ctx.open_url(UserWorkspaces::upgrade_link_for_team(*team_uid).as_str());
                send_telemetry_from_ctx!(
                    TelemetryEvent::SharedObjectLimitHitBannerViewPlansButtonClicked,
                    ctx
                );
            }
            DriveIndexAction::ManageBilling { team_uid } => {
                UserWorkspaces::handle(ctx).update(ctx, move |user_workspaces, ctx| {
                    user_workspaces.generate_stripe_billing_portal_link(*team_uid, ctx);
                });
            }
            DriveIndexAction::ToggleShareDialog { warp_drive_item_id } => {
                self.toggle_share_dialog(
                    warp_drive_item_id,
                    None,
                    SharingDialogSource::DriveIndex,
                    ctx,
                );
            }
            DriveIndexAction::SignupAnonymousUser => {
                let entrypoint = AnonymousUserSignupEntrypoint::SignUpButton;
                AuthManager::handle(ctx).update(ctx, |auth_manager, ctx| {
                    auth_manager.initiate_anonymous_user_linking(entrypoint, ctx);
                });
            }
            DriveIndexAction::DismissPersonalObjectLimits => {
                self.dismiss_personal_object_limit_status(ctx);
            }
            DriveIndexAction::SetCurrentWorkspace(workspace_uid) => {
                TeamUpdateManager::handle(ctx).update(ctx, |manager, ctx| {
                    manager.set_current_workspace_uid(*workspace_uid, ctx)
                });
            }
            DriveIndexAction::AttachPlanAsContext(id) => {
                ctx.emit(DriveIndexEvent::AttachPlanAsContext(*id))
            }
        }
    }
}

#[cfg(test)]
#[path = "index_tests.rs"]
mod tests;
