use alias_bar::{AliasBar, AliasBarEvent};
use argument_editor::{ArgumentEditorRow, DEFAULT_ARGUMENT_PREFIX};
use env_var_selector::{EnvVarSelector, EnvVarSelectorEvent};
use itertools::Itertools;
use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use string_offset::CharOffset;
use syntax_highlightable::SyntaxHighlightable;
use url::Url;

use crate::{
    ai::{blocklist::secret_redaction::find_secrets_in_text, AIRequestUsageModel},
    appearance::Appearance,
    auth::{auth_state::AuthState, AuthStateProvider, UserUid},
    cloud_object::{
        breadcrumbs::ContainingObject,
        model::{
            persistence::{CloudModel, CloudModelEvent},
            view::CloudViewModel,
        },
        CloudObject, CloudObjectEventEntrypoint, ObjectType, Owner, Revision, Space,
    },
    drive::{
        cloud_object_styling::warp_drive_icon_color,
        drive_helpers::has_feature_gated_anonymous_user_reached_workflow_limit,
        items::WarpDriveItemId,
        sharing::{ContentEditability, ShareableObject, SharingAccessLevel},
        workflows::{
            ai_assist::GeneratedCommandMetadataError,
            arguments::ArgumentsState,
            enum_creation_dialog::{EnumCreationDialog, EnumCreationDialogEvent, WorkflowEnumData},
            workflow_arg_selector::{WorkflowArgSelector, WorkflowArgSelectorEvent},
            workflow_arg_type_helpers::{self, ArgumentEditorRowIndex},
        },
        CloudObjectTypeAndId, DriveObjectType, OpenWarpDriveObjectSettings,
    },
    editor::{
        EditorOptions, EditorView, EnterAction, EnterSettings, Event as EditorEvent,
        InteractionState, PlainTextEditorViewAction as EditorAction,
        PropagateAndNoOpNavigationKeys, SingleLineEditorOptions, TextOptions, TextStyleOperation,
    },
    menu::{MenuItem, MenuItemFields},
    network::NetworkStatus,
    pane_group::{
        focus_state::PaneFocusHandle, pane::view, BackingView, PaneConfiguration, PaneEvent,
    },
    send_telemetry_from_ctx,
    server::{
        cloud_objects::update_manager::{
            FetchSingleObjectOption, ObjectOperation, OperationSuccessType, UpdateManager,
            UpdateManagerEvent,
        },
        ids::{ClientId, ServerId, SyncId},
        server_api::{ai::AIClient, ServerApiProvider},
        telemetry::{
            CloudObjectTelemetryMetadata, SharingDialogSource, TelemetryCloudObjectType,
            TelemetryEvent,
        },
    },
    settings::{
        app_installation_detection::{UserAppInstallDetectionSettings, UserAppInstallStatus},
        AISettings,
    },
    terminal::safe_mode_settings::get_secret_obfuscation_mode,
    ui_components::{
        breadcrumb::{render_breadcrumbs, BreadcrumbState},
        buttons::{accent_icon_button, icon_button},
        dialog::{dialog_styles, Dialog},
        icons::Icon,
    },
    util::bindings::CustomAction,
    view_components::{DismissibleToast, ToastLink, ToastType},
    workflows::{
        workflow::{Argument, Workflow},
        CloudWorkflow,
    },
    workspace::{ToastStack, WorkspaceAction},
    FeatureFlag, UserWorkspaces,
};

use warp_core::{context_flag::ContextFlag, settings::Setting, ui::theme::AnsiColorIdentifier};
use warp_editor::editor::NavigationKey;
use warpui::{
    clipboard::ClipboardContent,
    elements::{
        Align, Border, ChildAnchor, ChildView, Clipped, ClippedScrollStateHandle,
        ClippedScrollable, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Empty,
        Flex, Hoverable, MainAxisAlignment, MainAxisSize, MouseStateHandle, OffsetPositioning,
        ParentAnchor, ParentElement, ParentOffsetBounds, Radius, Rect, ScrollbarWidth, Shrinkable,
        Stack,
    },
    fonts::{FamilyId, Weight},
    keymap::EditableBinding,
    platform::Cursor,
    text_layout::TextStyle,
    ui_components::{
        button::{Button, ButtonVariant, TextAndIcon, TextAndIconAlignment},
        components::{Coords, UiComponent, UiComponentStyles},
    },
    AppContext, Element, Entity, FocusContext, ModelHandle, SingletonEntity, TypedActionView, View,
    ViewContext, ViewHandle, WindowId,
};

use super::{
    aliases::WorkflowAliases, command_parser::WorkflowCommandDisplayData, CloudWorkflowModel,
    WorkflowSource, WorkflowType, WorkflowViewMode,
};

#[cfg(target_family = "wasm")]
use crate::uri::web_intent_parser::open_url_on_desktop;

mod alias_argument_selector;
mod alias_bar;
mod argument_editor;
pub mod env_var_selector;
mod syntax_highlightable;

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::id;
    app.register_editable_bindings([EditableBinding::new(
        "workflowview:save",
        "Save workflow",
        WorkflowAction::Save,
    )
    .with_context_predicate(id!("WorkflowView"))
    .with_key_binding("cmdorctrl-s")]);

    app.register_editable_bindings([EditableBinding::new(
        "Close Workflow",
        "Close",
        WorkflowAction::Close,
    )
    .with_custom_action(CustomAction::CloseCurrentSession)
    .with_context_predicate(id!(WorkflowView::ui_name()))]);
}

const WORKFLOW_ICON_DIMENSIONS: f32 = 16.;

const WORKFLOW_PARAMETER_HIGHLIGHT_COLOR: u32 = 0x42C0FA4D;

const MAX_ELEMENT_WIDTH: f32 = 800.;

const SCROLLBAR_WIDTH: ScrollbarWidth = ScrollbarWidth::Auto;
const TITLE_PLACEHOLDER_TEXT: &str = "Add a title";
const DESCRIPTION_PLACEHOLDER_TEXT: &str = "Add a description";
const COMMAND_PLACEHOLDER_TEXT: &str = "echo \"Hello {{your_name}}\" # insert arguments with curly braces\n# enter a single-line command or an entire shell script";
const AGENT_MODE_QUERY_PLACEHOLDER_TEXT: &str = "Enter your prompt here... (e.g., 'Create a function to sort an array of objects by date' or 'Help me debug this React component').";
const DESCRIPTION_MARGIN_TOP: f32 = 10.;

const CORE_HORIZONATAL_MARGIN: f32 = 24.;
const CORE_VERTICAL_MARGIN_IN_PANE: f32 = 36.;

const SECTION_SPACING: f32 = 16.;
const SECTION_FONT_SIZE: f32 = 16.;

const DETAIL_TEXT_MARGIN_LEFT: f32 = 12.;
const DETAIL_BOX_PADDING_TOP_AND_LEFT: f32 = 12.;
const DETAIL_BOX_PADDING_BOTTOM_AND_RIGHT: f32 = 8.;

const WORKFLOW_CORNER_RADIUS: f32 = 10.;
const COMMAND_MARGIN_TOP: f32 = 20.;

// Padding for text_input
const VERTICAL_TEXT_INPUT_PADDING: f32 = 5.;
const HORIZONTAL_TEXT_INPUT_PADDING: f32 = 10.;

const EDITOR_FONT_SIZE: f32 = 14.;

const CREATE_BUTTON_TEXT: &str = "Create";
const SAVE_BUTTON_TEXT: &str = "Update";
const CANCEL_BUTTON_TEXT: &str = "Cancel";
const BUTTON_PADDING: f32 = 12.;
const BUTTON_FONT_SIZE: f32 = 14.;
const BUTTON_BORDER_RADIUS: f32 = 4.;
const BUTTON_HEIGHT: f32 = 32.;

const AI_ASSIST_BUTTON_SIZE: f32 = 92.;
const AI_ASSIST_BUTTON_TEXT: &str = "Autofill";
const AI_ASSIST_LOADING_TEXT: &str = "Loading";

const ALIAS_HELP_TEXT: &str = "Aliases allow you to create short strings to execute workflows. Each alias can have different argument values and environment variables, and aliases are personal to you.";

const RUN_ON_DESKTOP_BUTTON_TEXT: &str = "Run in Warp";
const RUN_ON_DESKTOP_BUTTON_WIDTH: f32 = 108.;

const UNSAVED_CHANGES_TEXT: &str = "You have unsaved changes.";
const KEEP_EDITING_TEXT: &str = "Keep editing";
const DISCARD_CHANGES_TEXT: &str = "Discard changes";
const DIALOG_WIDTH: f32 = 460.;
const MODAL_HORIZONTAL_MARGIN: f32 = 28.;

pub(super) enum AiAssistState {
    PreRequest,
    RequestInFlight,
    Generated,
}

/// A grouping of various error states the modal can be in. Any of these being
/// `true` prevents the save button from being clickable.
#[derive(Default)]
struct WorkflowEditorErrorState {
    /// The content must not be whitespace-only.
    content_empty_error: bool,
    /// The command must not have any arguments that are invalid (e.g. start
    /// with a numeric number or special character like *).
    invalid_argument_error: bool,
}

impl WorkflowEditorErrorState {
    pub fn new() -> Self {
        Self {
            content_empty_error: true,
            invalid_argument_error: false,
        }
    }

    pub fn has_any_error(&self) -> bool {
        self.content_empty_error || self.invalid_argument_error
    }
}

#[derive(Debug, Clone)]
pub enum WorkflowAction {
    ViewInWarpDrive(WarpDriveItemId),
    AddArgument,
    ToggleViewMode,
    RunWorkflow,
    CopyContent,
    Close,
    CloseUnsavedDialog,
    CloseEnumDialog,
    ForceClose,
    Save,
    Cancel,
    AiAssist,
    Duplicate,
    CopyLink(String),
    OpenLinkOnDesktop(Url),
    Trash,
    Untrash,
}

#[derive(Debug, Clone, PartialEq)]
pub enum WorkflowViewEvent {
    Pane(PaneEvent),
    CreatedWorkflow(SyncId),
    UpdatedWorkflow(SyncId),
    ViewInWarpDrive(WarpDriveItemId),
    OpenDriveObjectShareDialog {
        cloud_object_type_and_id: CloudObjectTypeAndId,
        invitee_email: Option<String>,
        source: SharingDialogSource,
    },
    RunWorkflow {
        workflow: Arc<WorkflowType>,
        source: WorkflowSource,
        argument_override: Option<HashMap<String, String>>,
    },
}

enum UnsavedChangeType {
    ForEdit,
    ForClose,
}

enum ContainerConfiguration {
    Pane(ModelHandle<PaneConfiguration>),
    SuggestionDialog,
}

#[derive(Default, Debug)]
struct EnvironmentVariablesState {
    default_env_vars: Option<SyncId>,
    is_dirty: bool,
}

#[derive(Default)]
struct UiStateHandles {
    add_variable_state: MouseStateHandle,
    cancel_mouse_state: MouseStateHandle,
    save_workflow_state: MouseStateHandle,
    // TODO: trash and restore with context menu
    restore_from_trash_button: MouseStateHandle,
    keep_editing_state: MouseStateHandle,
    discard_changes_state: MouseStateHandle,
    ai_assist_state: MouseStateHandle,
    ai_assist_tool_tip: MouseStateHandle,
    edit_mode_button_mouse_state: MouseStateHandle,
    copy_content_button_mouse_state: MouseStateHandle,
    execute_command_mouse_state: MouseStateHandle,
    alias_header_tool_tip: MouseStateHandle,
    add_environment_variables_mouse_state: MouseStateHandle,
    clipped_scroll_state: ClippedScrollStateHandle,
}

pub struct WorkflowView {
    workflow_view_mode: WorkflowViewMode,
    workflow_id: SyncId,
    container_configuration: ContainerConfiguration,
    focus_handle: Option<PaneFocusHandle>,
    name_editor: ViewHandle<EditorView>,
    description_editor: ViewHandle<EditorView>,
    content_editor: ViewHandle<EditorView>,
    content_editor_highlight_model: ModelHandle<SyntaxHighlightable>,
    view_only_content_editor: ViewHandle<EditorView>,
    view_only_content_editor_highlight_model: ModelHandle<SyntaxHighlightable>,
    arguments_state: ArgumentsState,
    arguments_rows: Vec<ArgumentEditorRow>,
    alias_bar: ViewHandle<AliasBar>,
    env_vars_selector: ViewHandle<EnvVarSelector>,
    env_vars_state: EnvironmentVariablesState,
    breadcrumbs: Vec<BreadcrumbState<ContainingObject>>,
    errors: WorkflowEditorErrorState,
    ui_state_handles: UiStateHandles,
    show_unsaved_changes: Option<UnsavedChangeType>,

    /// How many times the "add argument" button was clicked. We use this value
    /// to append a number to the default argument name (argument_1, argument_2,
    /// etc.).
    default_argument_id: usize,
    pub(super) ai_metadata_assist_state: AiAssistState,
    revision_ts: Option<Revision>,
    pub(super) auth_state: Arc<AuthState>,
    pub(super) ai_client: Arc<dyn AIClient>,
    owner: Option<Owner>,
    initial_folder_id: Option<SyncId>,

    command_display_data: WorkflowCommandDisplayData,

    pending_argument_editor_row: Option<ArgumentEditorRowIndex>,
    show_enum_creation_dialog: bool,
    enum_creation_dialog: ViewHandle<EnumCreationDialog>,
    all_workflow_enums: HashMap<SyncId, WorkflowEnumData>,

    /// `true` if this workflow view is for viewing/editing an AI workflow.
    ///
    /// This is currently internal-only, gated with the `am_workflows` feature flag.
    is_for_agent_mode: bool,
}

impl WorkflowView {
    pub fn is_agent_mode_workflow(&self) -> bool {
        self.is_for_agent_mode
    }
}

impl WorkflowView {
    pub fn new_in_pane(ctx: &mut ViewContext<Self>) -> Self {
        let pane_configuration = ctx.add_model(|_ctx| PaneConfiguration::new("Untitled"));

        Self::new_internal(ctx, ContainerConfiguration::Pane(pane_configuration))
    }

    pub fn new_in_suggestion_dialog(ctx: &mut ViewContext<Self>) -> Self {
        Self::new_internal(ctx, ContainerConfiguration::SuggestionDialog)
    }

    fn new_internal(
        ctx: &mut ViewContext<Self>,
        container_configuration: ContainerConfiguration,
    ) -> Self {
        let appearance = Appearance::as_ref(ctx);
        let header_font_size = appearance.header_font_size();
        let ui_font_family = appearance.ui_font_family();
        let monospace_font_family = appearance.monospace_font_family();

        let name_editor = Self::create_editor_handle(
            ctx,
            Some(header_font_size),
            Some(ui_font_family),
            Some(TITLE_PLACEHOLDER_TEXT),
            false,
            true,
            true,
        );

        let description_editor = Self::create_editor_handle(
            ctx,
            Some(EDITOR_FONT_SIZE),
            Some(ui_font_family),
            Some(DESCRIPTION_PLACEHOLDER_TEXT),
            false,
            false,
            true,
        );

        let content_editor = Self::create_editor_handle(
            ctx,
            Some(EDITOR_FONT_SIZE),
            Some(monospace_font_family),
            Some(COMMAND_PLACEHOLDER_TEXT),
            true,
            false,
            true,
        );

        let view_only_content_editor = Self::create_editor_handle(
            ctx,
            Some(EDITOR_FONT_SIZE),
            Some(monospace_font_family),
            Some(COMMAND_PLACEHOLDER_TEXT),
            true,
            false,
            true,
        );

        ctx.subscribe_to_view(&name_editor, |me, _, event, ctx| {
            me.handle_name_editor_event(event, ctx);
        });

        ctx.subscribe_to_view(&description_editor, |me, _, event, ctx| {
            me.handle_description_editor_event(event, ctx);
        });

        ctx.subscribe_to_view(&content_editor, |me, _, event, ctx| {
            me.handle_content_editor_event(event, ctx);
        });

        let ai_client = ServerApiProvider::as_ref(ctx).get_ai_client();

        let enum_creation_dialog = ctx.add_typed_action_view(EnumCreationDialog::new);
        ctx.subscribe_to_view(&enum_creation_dialog, |me, _, event, ctx| {
            me.handle_enum_creation_dialog_event(event, ctx);
        });

        let content_editor_highlight_model =
            ctx.add_model(|ctx| SyntaxHighlightable::new(content_editor.clone(), ctx));

        let view_only_content_editor_highlight_model =
            ctx.add_model(|ctx| SyntaxHighlightable::new(view_only_content_editor.clone(), ctx));

        let workflow_id = SyncId::ClientId(ClientId::default());
        let alias_bar = ctx.add_typed_action_view(|ctx| AliasBar::new(workflow_id, ctx));
        ctx.subscribe_to_view(&alias_bar, |me, _, event, ctx| {
            me.handle_alias_bar_event(event, ctx);
        });

        let env_vars_selector = ctx.add_typed_action_view(EnvVarSelector::new);
        ctx.subscribe_to_view(&env_vars_selector, |me, _, event, ctx| {
            me.handle_env_vars_selector_event(event, ctx);
        });

        let me = Self {
            workflow_view_mode: WorkflowViewMode::Edit, // defaults to view
            // setting workflow_id here so there's no chance we don't have one
            workflow_id,
            container_configuration,
            focus_handle: None,
            name_editor,
            description_editor,
            content_editor,
            content_editor_highlight_model,
            view_only_content_editor,
            view_only_content_editor_highlight_model,
            arguments_state: Default::default(),
            arguments_rows: Vec::new(),
            alias_bar,
            env_vars_selector,
            env_vars_state: Default::default(),
            breadcrumbs: Vec::new(),
            errors: WorkflowEditorErrorState::new(),
            ui_state_handles: Default::default(),
            show_unsaved_changes: None,
            default_argument_id: 0,
            ai_metadata_assist_state: AiAssistState::PreRequest,
            owner: None,
            initial_folder_id: None,
            revision_ts: None,
            command_display_data: WorkflowCommandDisplayData::new_empty(),
            auth_state: AuthStateProvider::as_ref(ctx).get().clone(),
            ai_client,
            pending_argument_editor_row: None,
            show_enum_creation_dialog: false,
            enum_creation_dialog,
            all_workflow_enums: Default::default(),
            is_for_agent_mode: false,
        };

        me.subscribe_to_model_updates(ctx);
        me
    }

    #[allow(clippy::too_many_arguments)]
    pub fn open_new_workflow(
        &mut self,
        title: Option<String>,
        content: Option<String>,
        owner: Owner,
        initial_folder_id: Option<SyncId>,
        is_for_agent_mode: bool,
        sync_id: SyncId,
        ctx: &mut ViewContext<Self>,
    ) {
        self.initial_folder_id = initial_folder_id;
        self.set_workflow_id(sync_id, ctx);
        self.workflow_view_mode = WorkflowViewMode::Create;
        self.command_display_data = WorkflowCommandDisplayData::new_empty();
        self.is_for_agent_mode = is_for_agent_mode;
        if is_for_agent_mode {
            self.content_editor.update(ctx, |editor, ctx| {
                editor.set_placeholder_text(AGENT_MODE_QUERY_PLACEHOLDER_TEXT, ctx);
                editor.set_font_family(Appearance::as_ref(ctx).ui_font_family(), ctx);
            });
        }

        if let Some(title_string) = title {
            self.name_editor.update(ctx, |editor, ctx| {
                editor.set_buffer_text(title_string.as_str(), ctx);
            });
        }

        if let Some(content) = content {
            self.content_editor.update(ctx, |editor, ctx| {
                editor.set_buffer_text(content.as_str(), ctx);
            });
        }

        self.owner = Some(owner);
        self.all_workflow_enums =
            workflow_arg_type_helpers::load_workflow_enums_with_owner(owner, ctx);

        if is_for_agent_mode {
            self.content_editor.update(ctx, |editor, ctx| {
                editor.set_placeholder_text(AGENT_MODE_QUERY_PLACEHOLDER_TEXT, ctx);
            });
        }

        self.update_editors_interactivity(ctx);
    }

    fn subscribe_to_model_updates(&self, ctx: &mut ViewContext<Self>) {
        ctx.subscribe_to_model(&CloudModel::handle(ctx), move |workflow, _, event, ctx| {
            workflow.handle_cloud_model_event(event, ctx)
        });

        let update_manager = UpdateManager::handle(ctx);
        ctx.subscribe_to_model(&update_manager, |me, _, event, ctx| {
            me.handle_update_manager_event(event, ctx);
        });
    }

    fn handle_cloud_model_event(&mut self, event: &CloudModelEvent, ctx: &mut ViewContext<Self>) {
        match event {
            CloudModelEvent::ObjectUpdated {
                type_and_id: CloudObjectTypeAndId::Workflow(sync_id),
                source: _,
            } => {
                if self.workflow_id() == *sync_id && !self.is_editable() {
                    self.reset(ctx);
                }
            }
            CloudModelEvent::ObjectTrashed { .. }
            | CloudModelEvent::ObjectDeleted { .. }
            | CloudModelEvent::ObjectUntrashed { .. } => ctx.notify(),
            _ => (),
        }
    }

    fn handle_update_manager_event(
        &mut self,
        event: &UpdateManagerEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        let UpdateManagerEvent::ObjectOperationComplete { result } = event else {
            return;
        };

        if let (ObjectOperation::Create { .. }, OperationSuccessType::Success) =
            (&result.operation, &result.success_type)
        {
            if self.workflow_id.into_client() == result.client_id {
                let server_id = result
                    .server_id
                    .expect("Expect server id on success creation");

                // The aliases were created with the old client sync id.  Update them to the new server id.
                WorkflowAliases::handle(ctx).update(ctx, |aliases, ctx| {
                    if let Result::Err(e) =
                        aliases.update_workflow_id(self.workflow_id, server_id.into(), ctx)
                    {
                        log::error!("Failed to update aliases after workflow creation: {e:?}");
                    }
                });

                if let Some(workflow) =
                    CloudModel::as_ref(ctx).get_workflow_by_uid(&server_id.uid())
                {
                    self.load(
                        workflow.clone(),
                        &OpenWarpDriveObjectSettings::default(),
                        self.workflow_view_mode,
                        ctx,
                    );
                }
                ctx.notify();
            }
        }

        if let (ObjectOperation::Update, OperationSuccessType::Success) =
            (&result.operation, &result.success_type)
        {
            if let Some(workflow) = self.get_cloud_workflow(ctx) {
                // This makes sure we get the correct updated revision_ts. So our subsequent
                // updates don't fail
                if self.workflow_id.into_client() == result.client_id
                    || self.workflow_id.uid() == result.server_id.unwrap_or_default().uid()
                {
                    self.load(
                        workflow,
                        &OpenWarpDriveObjectSettings::default(),
                        self.workflow_view_mode,
                        ctx,
                    );
                }
            }
        }
    }

    fn should_show_unsaved_changes_dialog(&self, app: &AppContext) -> bool {
        self.is_dirty(app)
    }

    fn reset(&mut self, ctx: &mut ViewContext<Self>) {
        let cloud_workflow = self.get_cloud_workflow(ctx);
        if let Some(workflow) = cloud_workflow {
            self.load(
                workflow,
                &OpenWarpDriveObjectSettings::default(),
                self.workflow_view_mode,
                ctx,
            );
        }
    }

    pub fn wait_for_initial_load_then_load(
        &mut self,
        workflow_id: SyncId,
        settings: &OpenWarpDriveObjectSettings,
        mode: WorkflowViewMode,
        window_id: WindowId,
        ctx: &mut ViewContext<Self>,
    ) {
        let initial_load_complete = UpdateManager::as_ref(ctx).initial_load_complete();
        // TODO @ianhodge CLD-2002: it could be nice to have a loading screen here while we wait for the load
        let settings = settings.clone();
        ctx.spawn(initial_load_complete, move |me, _, ctx| {
            let workflow = CloudModel::as_ref(ctx).get_workflow(&workflow_id).cloned();
            // If either the focused folder or the workflow can't be found in cloudmodel, fetch the object from the server
            let fetch_needed = workflow.is_none()
                || settings
                    .focused_folder_id
                    .map(SyncId::ServerId)
                    .map(|folder_id| CloudModel::as_ref(ctx).get_folder(&folder_id).is_none())
                    .unwrap_or(false);
            if fetch_needed {
                if let Some(server_id) = workflow_id.into_server() {
                    me.fetch_and_load_workflow(server_id, &settings, mode, window_id, ctx);
                } else {
                    log::warn!("Tried to load workflow without server id {workflow_id:?}");
                }
            } else if let Some(workflow) = workflow {
                me.load(workflow, &settings, mode, ctx);
            } else {
                ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                    toast_stack.add_ephemeral_toast_by_type(
                        ToastType::CloudObjectNotFound,
                        window_id,
                        ctx,
                    );
                });
                log::warn!("Tried to open unknown workflow {workflow_id:?}");
            }
        });
    }

    fn fetch_and_load_workflow(
        &mut self,
        workflow_id: ServerId,
        settings: &OpenWarpDriveObjectSettings,
        mode: WorkflowViewMode,
        window_id: WindowId,
        ctx: &mut ViewContext<Self>,
    ) {
        // If we have a parent folder we are trying to load as a part of this workflow, fetch that instead
        let id_to_fetch = settings.focused_folder_id.unwrap_or(workflow_id);
        let fetch_cloud_object_rx =
            UpdateManager::handle(ctx).update(ctx, |update_manager, ctx| {
                update_manager.fetch_single_cloud_object(
                    &id_to_fetch,
                    FetchSingleObjectOption::None,
                    ctx,
                )
            });
        let settings = settings.clone();
        ctx.spawn(fetch_cloud_object_rx, move |me, _, ctx| {
            if let Some(workflow) = CloudModel::as_ref(ctx)
                .get_workflow(&SyncId::ServerId(workflow_id))
                .cloned()
            {
                me.load(workflow, &settings, mode, ctx);
            } else {
                ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                    toast_stack.add_ephemeral_toast_by_type(
                        ToastType::CloudObjectNotFound,
                        window_id,
                        ctx,
                    );
                });
                log::warn!("Tried to open unknown workflow {workflow_id:?} after fetching");
            }
        });
    }

    pub fn load(
        &mut self,
        workflow: CloudWorkflow,
        settings: &OpenWarpDriveObjectSettings,
        mode: WorkflowViewMode,
        ctx: &mut ViewContext<Self>,
    ) {
        self.set_workflow_id(workflow.id, ctx);
        self.is_for_agent_mode = workflow.model().data.is_agent_mode_workflow();
        if self.is_for_agent_mode {
            self.content_editor.update(ctx, |editor, ctx| {
                editor.set_placeholder_text(AGENT_MODE_QUERY_PLACEHOLDER_TEXT, ctx);
                editor.set_font_family(Appearance::as_ref(ctx).ui_font_family(), ctx);
            });
        }

        self.workflow_view_mode = match mode {
            // Force view mode if the user is not allowed to edit the workflow.
            WorkflowViewMode::Edit => {
                WorkflowViewMode::supported_edit_mode(Some(self.workflow_id), ctx)
            }
            // Force edit mode if we are in a context where we can run workflows and we try to use view
            // mode
            WorkflowViewMode::View => {
                WorkflowViewMode::supported_view_mode(Some(self.workflow_id), ctx)
            }
            mode => mode,
        };

        self.revision_ts = workflow.metadata.revision.clone();

        let owner = workflow.permissions.owner;
        self.owner = Some(owner);
        self.all_workflow_enums =
            workflow_arg_type_helpers::load_workflow_enums_with_owner(owner, ctx);

        let workflow_data = &workflow.model().data;
        let workflow_name = workflow_data.name();
        let workflow_description = workflow_data.description().cloned().unwrap_or_default();
        let workflow_content = workflow_data.content();

        self.command_display_data = WorkflowCommandDisplayData::new_from_workflow(workflow_data);

        if let ContainerConfiguration::Pane(pane_config) = &mut self.container_configuration {
            pane_config.update(ctx, |pane_config, ctx| {
                pane_config.set_title(workflow_name, ctx);
                if let Some(server_id) = workflow.id.into_server() {
                    pane_config.set_shareable_object(
                        Some(ShareableObject::WarpDriveObject(server_id)),
                        ctx,
                    );
                }
            });
        }

        self.name_editor.update(ctx, |editor, ctx| {
            editor.set_buffer_text_with_base_buffer(workflow_name, ctx)
        });

        self.description_editor.update(ctx, |editor, ctx| {
            editor.set_buffer_text_with_base_buffer(&workflow_description, ctx)
        });

        self.content_editor.update(ctx, |editor, ctx| {
            editor.set_buffer_text_with_base_buffer(workflow_content, ctx)
        });

        let text_style_ranges = self
            .command_display_data
            .argument_ranges()
            .into_iter()
            .map(|range| {
                (
                    range,
                    TextStyle::new().with_background_color(ColorU::from_u32(
                        WORKFLOW_PARAMETER_HIGHLIGHT_COLOR,
                    )),
                )
            })
            .collect_vec();

        self.view_only_content_editor.update(ctx, |editor, ctx| {
            editor.set_interaction_state(InteractionState::Editable, ctx);
            editor.clear_buffer(ctx);

            editor.insert_with_styles(
                self.command_display_data.to_command_string().as_str(),
                &text_style_ranges,
                EditorAction::SystemInsert,
                ctx,
            );
            editor.set_interaction_state(InteractionState::Selectable, ctx);
        });

        self.arguments_state = ArgumentsState::for_command_workflow(
            &self.arguments_state,
            workflow_content.to_string(),
        );
        self.update_arguments_rows(ctx);
        self.load_argument_data(workflow_data, ctx);

        if self.is_for_agent_mode {
            self.content_editor.update(ctx, |editor, ctx| {
                editor.set_placeholder_text(AGENT_MODE_QUERY_PLACEHOLDER_TEXT, ctx);
            });
        } else {
            self.content_editor_highlight_model
                .update(ctx, |model, ctx| {
                    model.highlight_syntax(ctx);
                });
            self.view_only_content_editor_highlight_model
                .update(ctx, |model, ctx| {
                    model.highlight_syntax(ctx);
                });

            if let Workflow::Command {
                environment_variables,
                ..
            } = workflow_data
            {
                self.env_vars_state = EnvironmentVariablesState {
                    default_env_vars: *environment_variables,
                    is_dirty: false,
                };
                self.env_vars_selector.update(ctx, |selector, ctx| {
                    selector.set_selected_env_vars(*environment_variables, ctx)
                });
            }
        }
        self.update_breadcrumb(ctx);
        self.update_editors_interactivity(ctx);
        self.refresh_pane_overflow_menu(ctx);

        if let Some(focused_folder_id) = settings.focused_folder_id.map(SyncId::ServerId) {
            self.view_in_warp_drive(
                WarpDriveItemId::Object(CloudObjectTypeAndId::Folder(focused_folder_id)),
                ctx,
            );
        }

        if let Some(invitee_email) = settings.invitee_email.clone() {
            let object_id_to_share = settings
                .focused_folder_id
                .map(|id| CloudObjectTypeAndId::Folder(SyncId::ServerId(id)))
                .unwrap_or(CloudObjectTypeAndId::Workflow(workflow.id));
            ctx.emit(WorkflowViewEvent::OpenDriveObjectShareDialog {
                cloud_object_type_and_id: object_id_to_share,
                invitee_email: Some(invitee_email),
                source: SharingDialogSource::InviteeRequest,
            });
        }

        if matches!(mode, WorkflowViewMode::View) {
            self.focus_first_argument_value(ctx);
        }
    }

    pub fn refresh_pane_overflow_menu(&self, ctx: &mut ViewContext<Self>) {
        if let ContainerConfiguration::Pane(pane_config) = &self.container_configuration {
            // Refresh the overflow menu to show actions that only apply to synced notebooks.
            pane_config.update(ctx, |pane_config, ctx| {
                pane_config.refresh_pane_header_overflow_menu_items(ctx)
            });
            ctx.notify();
        }
    }

    pub fn workflow_id(&self) -> SyncId {
        self.workflow_id
    }

    fn set_workflow_id(&mut self, id: SyncId, ctx: &mut ViewContext<Self>) {
        self.workflow_id = id;
        self.alias_bar.update(ctx, |alias_bar, ctx| {
            alias_bar.set_workflow_id(id, ctx);
        });
    }

    pub fn workflow_link(&self, ctx: &AppContext) -> Option<String> {
        let id = self.workflow_id();

        if let Some(workflow) = CloudModel::as_ref(ctx).get_workflow(&id) {
            return workflow.object_link();
        }

        None
    }

    /// Generic object telemetry metadata for the currently-open object.
    #[cfg_attr(not(target_family = "wasm"), allow(dead_code))]
    fn telemetry_metadata(&self, ctx: &mut ViewContext<Self>) -> CloudObjectTelemetryMetadata {
        let space = CloudModel::as_ref(ctx)
            .get_workflow(&self.workflow_id)
            .map(|workflow| workflow.space(ctx));

        CloudObjectTelemetryMetadata {
            object_type: TelemetryCloudObjectType::Workflow,
            object_uid: self.workflow_id.into_server(),
            space: space.map(Into::into),
            team_uid: match self.owner {
                Some(Owner::Team { team_uid, .. }) => Some(team_uid),
                _ => None,
            },
        }
    }

    pub fn is_team_workflow(&self) -> bool {
        matches!(self.owner, Some(Owner::Team { .. }))
    }

    /// The current user's access level for this workflow.
    fn access_level(&self, app: &AppContext) -> SharingAccessLevel {
        CloudViewModel::as_ref(app).access_level(&self.workflow_id.uid(), app)
    }

    /// Whether or not the current user is allowed to edit this workflow.
    fn editability(&self, app: &AppContext) -> ContentEditability {
        CloudViewModel::as_ref(app).object_editability(&self.workflow_id.uid(), app)
    }

    pub fn pane_configuration(&self) -> &ModelHandle<PaneConfiguration> {
        match &self.container_configuration {
            ContainerConfiguration::Pane(pane_config) => pane_config,
            ContainerConfiguration::SuggestionDialog => {
                panic!("No pane configuration for suggestion dialog");
            }
        }
    }

    fn can_save_new(&self, app: &AppContext) -> bool {
        if matches!(self.workflow_view_mode, WorkflowViewMode::Create)
            && !self.content_editor.as_ref(app).is_empty(app)
        {
            return true;
        }
        false
    }

    fn is_dirty(&self, app: &AppContext) -> bool {
        self.is_workflow_dirty(app) || self.are_aliases_dirty(app)
    }

    fn is_workflow_dirty(&self, app: &AppContext) -> bool {
        let name_is_dirty = self.name_editor.as_ref(app).is_dirty(app);
        let description_is_dirty = self.description_editor.as_ref(app).is_dirty(app);
        let content_is_dirty = self.content_editor.as_ref(app).is_dirty(app);
        let any_argument_editor_is_dirty = self.has_dirty_argument_editor(app);

        name_is_dirty
            || description_is_dirty
            || content_is_dirty
            || any_argument_editor_is_dirty
            || self.env_vars_state.is_dirty
    }

    fn are_aliases_dirty(&self, app: &AppContext) -> bool {
        if !FeatureFlag::WorkflowAliases.is_enabled() {
            return false;
        }
        self.alias_bar.as_ref(app).has_unsaved_changes()
    }

    fn is_save_workflow_button_disabled(&self) -> bool {
        self.errors.has_any_error() || self.show_enum_creation_dialog
    }

    fn handle_name_editor_event(&mut self, event: &EditorEvent, ctx: &mut ViewContext<Self>) {
        match event {
            EditorEvent::Edited(_) => ctx.notify(),
            EditorEvent::Navigate(NavigationKey::Tab) => {
                ctx.focus(&self.description_editor);
            }
            EditorEvent::Navigate(NavigationKey::ShiftTab) => match self.arguments_rows.last() {
                Some(row) => ctx.focus(&row.default_value_editor),
                None => ctx.focus(&self.content_editor),
            },
            _ => {}
        }
    }

    fn handle_description_editor_event(
        &mut self,
        event: &EditorEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            EditorEvent::Edited(_) => ctx.notify(),
            EditorEvent::Navigate(NavigationKey::ShiftTab) => ctx.focus(&self.name_editor),
            EditorEvent::Navigate(NavigationKey::Tab) => ctx.focus(&self.content_editor),
            EditorEvent::Navigate(NavigationKey::Up) => self
                .description_editor
                .update(ctx, |input, ctx| input.move_up(ctx)),
            EditorEvent::Navigate(NavigationKey::Down) => self
                .description_editor
                .update(ctx, |input, ctx| input.move_down(ctx)),
            _ => {}
        }
    }

    fn handle_content_editor_event(&mut self, event: &EditorEvent, ctx: &mut ViewContext<Self>) {
        match event {
            EditorEvent::Edited(_) => {
                let current_content = self
                    .content_editor
                    .read(ctx, |editor, ctx| editor.buffer_text(ctx));

                self.errors.content_empty_error = current_content.trim().is_empty();

                self.arguments_state = if self.is_for_agent_mode {
                    ArgumentsState::for_saved_prompt(&self.arguments_state, current_content.clone())
                } else {
                    ArgumentsState::for_command_workflow(
                        &self.arguments_state,
                        current_content.clone(),
                    )
                };
                self.update_arguments_rows(ctx);

                self.clear_content_formatting(current_content.chars().count(), ctx);
                self.apply_error_underlining_to_content(ctx);
                self.apply_argument_highlighting_to_content(ctx);

                if !self.is_for_agent_mode {
                    self.content_editor_highlight_model
                        .update(ctx, |model, ctx| {
                            model.highlight_syntax(ctx);
                        });
                }

                self.errors.invalid_argument_error = !self
                    .arguments_state
                    .invalid_arguments_char_ranges
                    .is_empty();

                ctx.notify();
            }
            // when the editor supports tab completions, we'll need to change this logic
            EditorEvent::Navigate(NavigationKey::Tab) => match self.arguments_rows.first() {
                Some(row) => ctx.focus(&row.description_editor),
                None => ctx.focus(&self.name_editor),
            },
            EditorEvent::Navigate(NavigationKey::ShiftTab) => ctx.focus(&self.description_editor),
            EditorEvent::Navigate(NavigationKey::Up) => self
                .content_editor
                .update(ctx, |input, ctx| input.move_up(ctx)),
            EditorEvent::Navigate(NavigationKey::Down) => self
                .content_editor
                .update(ctx, |input, ctx| input.move_down(ctx)),
            EditorEvent::Activate => {
                // TODO: bug here where the pane steals the focus and we have to click twice to
                // actually get focus
                ctx.emit(WorkflowViewEvent::Pane(PaneEvent::FocusSelf));
            }
            _ => {}
        }
    }

    fn handle_enum_creation_dialog_event(
        &mut self,
        event: &EnumCreationDialogEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            EnumCreationDialogEvent::Close => {
                self.hide_enum_creation_dialog(ctx);
                // Reopen the dropdown after the enum dialog is closed
                if let Some(ArgumentEditorRowIndex(index)) = self.pending_argument_editor_row.take()
                {
                    ctx.focus(&self.arguments_rows[index].arg_type_editor);
                }
            }
            EnumCreationDialogEvent::CreateEnum(enum_data) => {
                workflow_arg_type_helpers::create_enum(
                    enum_data,
                    &mut self.all_workflow_enums,
                    &self.arguments_rows,
                    &mut self.pending_argument_editor_row,
                    ctx,
                );
            }
            EnumCreationDialogEvent::EditEnum(enum_data, did_visibility_change) => {
                workflow_arg_type_helpers::edit_enum(
                    enum_data,
                    *did_visibility_change,
                    &mut self.all_workflow_enums,
                    &self.arguments_rows,
                    &mut self.pending_argument_editor_row,
                    ctx,
                );
            }
        }
    }

    fn handle_type_selector_event(
        &mut self,
        handle: ViewHandle<WorkflowArgSelector>,
        event: &WorkflowArgSelectorEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            WorkflowArgSelectorEvent::NewEnum => {
                // Find and store the row that emitted this event
                self.pending_argument_editor_row = self
                    .arguments_rows
                    .iter()
                    .position(|row| row.arg_type_editor.eq(&handle))
                    .map(ArgumentEditorRowIndex);

                // Initialize the creation dialog for new enum
                self.enum_creation_dialog.update(ctx, |dialog, ctx| {
                    dialog.initialize(ctx);
                });

                self.show_enum_creation_dialog(ctx);
            }
            WorkflowArgSelectorEvent::LoadEnum(index) => {
                // Find and store the row that emitted this event
                self.pending_argument_editor_row = self
                    .arguments_rows
                    .iter()
                    .position(|row| row.arg_type_editor.eq(&handle))
                    .map(ArgumentEditorRowIndex);

                // Load the enum data into the enum dialog
                let show_dialog = workflow_arg_type_helpers::load_enum(
                    index,
                    &self.all_workflow_enums,
                    &self.enum_creation_dialog,
                    ctx,
                );

                if show_dialog {
                    self.show_enum_creation_dialog(ctx);
                }
            }
            WorkflowArgSelectorEvent::Edited | WorkflowArgSelectorEvent::Close => ctx.notify(),
            WorkflowArgSelectorEvent::ToggleExpanded => {
                // Close all other rows
                self.arguments_rows.iter().for_each(|row| {
                    if !row.arg_type_editor.eq(&handle) {
                        row.arg_type_editor.update(ctx, |editor, ctx| {
                            editor.close(ctx);
                        });
                    }
                });
            }
            WorkflowArgSelectorEvent::InputTab => {
                if let Some(index) = self
                    .arguments_rows
                    .iter()
                    .position(|row| row.arg_type_editor.eq(&handle))
                {
                    match self.arguments_rows.get(index + 1) {
                        Some(next_row) => ctx.focus(&next_row.description_editor),
                        None => ctx.focus(&self.name_editor),
                    }
                }
            }
            WorkflowArgSelectorEvent::InputShiftTab => {
                if let Some(row) = self
                    .arguments_rows
                    .iter()
                    .find(|row| row.arg_type_editor.eq(&handle))
                {
                    ctx.focus(&row.description_editor);
                }
            }
        }
    }

    fn handle_alias_bar_event(&mut self, event: &AliasBarEvent, ctx: &mut ViewContext<Self>) {
        if !FeatureFlag::WorkflowAliases.is_enabled() {
            return;
        }

        match event {
            AliasBarEvent::SelectedAliasChanged => {
                // Clone the arguments so that we can update the argument editors.
                let argument_values = self
                    .alias_bar
                    .as_ref(ctx)
                    .current_argument_values()
                    .cloned();
                let arguments = self.arguments_with_metadata(ctx);

                for (row, arg) in self.arguments_rows.iter_mut().zip(arguments.iter()) {
                    row.alias_argument_selector.update(ctx, |selector, ctx| {
                        let value = argument_values
                            .as_ref()
                            .and_then(|args| args.get(&row.name));
                        selector.set_argument(&arg.arg_type, value, &self.all_workflow_enums, ctx);
                    });
                }

                self.env_vars_selector.update(ctx, |selector, ctx| {
                    let selected_env_vars = if self.alias_bar.as_ref(ctx).has_selected_alias() {
                        self.alias_bar.as_ref(ctx).current_env_vars()
                    } else {
                        self.env_vars_state.default_env_vars
                    };
                    selector.set_selected_env_vars(selected_env_vars, ctx);
                });

                ctx.notify();
            }
            AliasBarEvent::AliasesUpdated => {
                // Recompute dirty state.
                ctx.notify();
            }
        }
    }

    fn handle_env_vars_selector_event(
        &mut self,
        event: &EnvVarSelectorEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            EnvVarSelectorEvent::SelectionChanged(id) => {
                if self.alias_bar.as_ref(ctx).has_selected_alias() {
                    self.alias_bar
                        .update(ctx, |bar, ctx| bar.set_current_env_vars(*id, ctx));
                } else {
                    self.env_vars_state.default_env_vars = *id;
                    self.env_vars_state.is_dirty = true;
                    ctx.notify();
                }
            }
            EnvVarSelectorEvent::Refreshed => {
                // Re-render in case the selector visibility changed.
                ctx.notify();
            }
        }
    }

    /// Merges the arguments from arguments_state with the descriptions/default
    /// values that we store in the view layer.
    fn arguments_with_metadata(&mut self, ctx: &mut ViewContext<Self>) -> Vec<Argument> {
        self.arguments_state
            .arguments
            .iter()
            .enumerate()
            .filter_map(|(index, argument)| {
                self.arguments_rows.get(index).map(|argument_row| {
                    let description_editor = argument_row.description_editor.as_ref(ctx);

                    let description = match description_editor.is_empty(ctx) {
                        true => None,
                        false => Some(description_editor.buffer_text(ctx)),
                    };

                    let type_selector = argument_row.arg_type_editor.as_ref(ctx);
                    let text_editor = type_selector.text_editor.as_ref(ctx);

                    workflow_arg_type_helpers::extract_typed_argument_from_selector(
                        argument,
                        description,
                        type_selector,
                        text_editor,
                        ctx,
                    )
                })
            })
            .collect()
    }

    /// Iterates through the argument rows and creates/updates any relevant argument objects on the server.
    /// Returns a mapping of argument row indices to the ID of relevant objects, to be used by `arguments_with_metadata`
    /// when creating or updating a `Workflow` object.
    fn save_argument_objects(&self, ctx: &mut ViewContext<Self>) {
        let mut sent_requests: HashSet<SyncId> = HashSet::new();
        let owner = match self.workflow_view_mode {
            WorkflowViewMode::View => None,
            WorkflowViewMode::Edit => CloudModel::as_ref(ctx)
                .get_workflow(&self.workflow_id)
                .map(|workflow| workflow.permissions().owner),
            WorkflowViewMode::Create => self.owner,
        };

        self.arguments_rows.iter().for_each(|argument_row| {
            let type_selector = argument_row.arg_type_editor.as_ref(ctx);

            // Check to see if we have enum data for this id, then create a request for it
            for enum_id in type_selector.get_created_enums() {
                if !sent_requests.contains(&enum_id) {
                    if let Some(enum_data) = self.all_workflow_enums.get(&enum_id) {
                        if enum_data.new_data.is_some() {
                            workflow_arg_type_helpers::save_enum(enum_data, owner, ctx);

                            // Make sure we aren't sending duplicate requests.
                            // If an enum is used in multiple arguments, we'll only save it once
                            sent_requests.insert(enum_id);
                        }
                    }
                }
            }

            argument_row.arg_type_editor.update(ctx, |selector, ctx| {
                selector.clear_created_enums(ctx);
            });
        });
    }

    // If the title isn't supplied by the user, we use the first two words of the command or query
    // as the title.
    fn truncate_content_for_title(&self, content: String) -> String {
        content.split_ascii_whitespace().take(2).join(" ")
    }

    // Attempts to force enable all editors. Then later use the self.update_editors_interactivity to
    // potentially disable the ones that should be disabled based on if we are in view or edit mode.
    pub(super) fn enable_editors(&mut self, ctx: &mut ViewContext<Self>) {
        self.description_editor.update(ctx, |view, ctx| {
            view.set_interaction_state(InteractionState::Editable, ctx)
        });

        self.name_editor.update(ctx, |view, ctx| {
            view.set_interaction_state(InteractionState::Editable, ctx)
        });

        self.content_editor.update(ctx, |view, ctx| {
            view.set_interaction_state(InteractionState::Editable, ctx)
        });

        self.arguments_rows.iter().for_each(|row| {
            row.description_editor.update(ctx, |editor, ctx| {
                editor.set_interaction_state(InteractionState::Editable, ctx);
            });
            row.default_value_editor.update(ctx, |editor, ctx| {
                editor.set_interaction_state(InteractionState::Editable, ctx);
            });
            row.argument_editor.update(ctx, |editor, ctx| {
                editor.set_interaction_state(InteractionState::Editable, ctx);
            });
            row.arg_type_editor.update(ctx, |selector, ctx| {
                selector.enable(ctx);
            });
        });

        self.update_editors_interactivity(ctx);
    }

    /// Force disable all editors
    fn disable_editors(&mut self, ctx: &mut ViewContext<Self>) {
        self.description_editor.update(ctx, |view, ctx| {
            view.set_interaction_state(InteractionState::Disabled, ctx)
        });

        self.name_editor.update(ctx, |view, ctx| {
            view.set_interaction_state(InteractionState::Disabled, ctx)
        });

        self.content_editor.update(ctx, |view, ctx| {
            view.set_interaction_state(InteractionState::Disabled, ctx)
        });

        self.arguments_rows.iter().for_each(|row| {
            row.description_editor.update(ctx, |editor, ctx| {
                editor.set_interaction_state(InteractionState::Disabled, ctx);
            });
            row.default_value_editor.update(ctx, |editor, ctx| {
                editor.set_interaction_state(InteractionState::Disabled, ctx);
            });
            row.argument_editor.update(ctx, |editor, ctx| {
                editor.set_interaction_state(InteractionState::Disabled, ctx);
            });
            row.arg_type_editor.update(ctx, |selector, ctx| {
                selector.disable(ctx);
            });
        });
    }

    fn update_editors_interactivity(&mut self, ctx: &mut ViewContext<Self>) {
        let interaction_state = match self.workflow_view_mode {
            WorkflowViewMode::View => InteractionState::Selectable,
            WorkflowViewMode::Edit | WorkflowViewMode::Create => InteractionState::Editable,
        };
        self.name_editor.update(ctx, |editor, ctx| {
            editor.set_interaction_state(interaction_state, ctx);
        });
        self.description_editor.update(ctx, |editor, ctx| {
            editor.set_interaction_state(interaction_state, ctx);
        });
        self.content_editor.update(ctx, |editor, ctx| {
            editor.set_interaction_state(interaction_state, ctx);
        });

        // always selectable
        self.view_only_content_editor.update(ctx, |editor, ctx| {
            editor.set_interaction_state(InteractionState::Selectable, ctx);
        });

        self.arguments_rows.iter().for_each(|row| {
            row.description_editor.update(ctx, |editor, ctx| {
                editor.set_interaction_state(interaction_state, ctx);
            });
        });

        // NOTE: We don't need to update the interactive state of the argument editors because
        // they are hidden in the view mode.
    }

    fn emit_close_event(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.emit(WorkflowViewEvent::Pane(PaneEvent::Close));
    }

    fn try_set_view_mode(&mut self, ctx: &mut ViewContext<Self>) {
        self.workflow_view_mode =
            WorkflowViewMode::supported_view_mode(Some(self.workflow_id), ctx);
        // always reset with the cloudmodel version whether or not we successfully
        // transition to the view mode. This reset doesn't always set the correct revision_ts
        // we rely on the load called when we handle the update_manager's change event.
        self.reset(ctx);
        self.update_editors_interactivity(ctx);
        ctx.notify()
    }

    fn reset_view_mode_argument_values(&mut self, ctx: &mut ViewContext<Self>) {
        for row in &self.arguments_rows {
            row.argument_editor.update(ctx, |editor, ctx| {
                editor.clear_buffer(ctx);
            })
        }
    }

    fn focus_first_argument_value(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(first_row) = self.arguments_rows.first() {
            first_row.argument_editor.update(ctx, |_input, ctx| {
                ctx.focus_self();
            });
        }
    }

    // This should only be available in context where we can't run workflows
    fn toggle_view_mode(&mut self, ctx: &mut ViewContext<Self>) {
        if self.show_enum_creation_dialog {
            return;
        }

        if matches!(self.workflow_view_mode, WorkflowViewMode::Edit)
            && self.should_show_unsaved_changes_dialog(ctx)
        {
            self.show_unsaved_changes_dialog(UnsavedChangeType::ForEdit, ctx);
            return;
        }

        self.workflow_view_mode = match self.workflow_view_mode {
            WorkflowViewMode::View => {
                WorkflowViewMode::supported_edit_mode(Some(self.workflow_id), ctx)
            }
            // Attempt to toggle to view mode only if it is allowed in this context
            WorkflowViewMode::Edit => {
                WorkflowViewMode::supported_view_mode(Some(self.workflow_id), ctx)
            }
            // NOTE: prevent transition from create to any other mode
            // we also shouldn't be showing the toggle button in create view
            WorkflowViewMode::Create => WorkflowViewMode::Create,
        };

        // Always reset the view with cloud model when we transition to the view or edit mode.
        // This reset is necessary when transitioning to edit mode so that we can reset the `revision_ts`.
        // Without this, any updates after a first update will get rejected, due to a perceived conflict.
        if matches!(self.workflow_view_mode, WorkflowViewMode::View)
            || matches!(self.workflow_view_mode, WorkflowViewMode::Edit)
        {
            self.reset(ctx);
        }

        // reset the view mode argument editors when we transition away from view mode
        if !matches!(self.workflow_view_mode, WorkflowViewMode::View) {
            self.reset_view_mode_argument_values(ctx);
        }

        self.update_editors_interactivity(ctx);
        ctx.notify()
    }

    fn copy_to_command_line(&mut self, ctx: &mut ViewContext<Self>) {
        // If we are in a context where we can run workflows AND the content is dirty (e.g. not
        // saved). Copy the current workflow to the command line buffer.
        // Otherwise use the workflow that exists in cloud model cache.
        // This is because we want to use the version of the edited command that a user has in the
        // buffer if they click to execute a command from the workflow in pane.
        if self.is_workflow_dirty(ctx) {
            let new_workflow = self.create_workflow_object_from_input(ctx);
            if let Some(cloud_workflow) = self.get_cloud_workflow(ctx) {
                let mut cloned_cloud_workflow = cloud_workflow.clone();
                cloned_cloud_workflow.set_model(CloudWorkflowModel::new(new_workflow));
                if let Some(owner) = self.owner {
                    ctx.emit(WorkflowViewEvent::RunWorkflow {
                        workflow: Arc::new(WorkflowType::Cloud(Box::new(cloned_cloud_workflow))),
                        source: owner.into(),
                        argument_override: None,
                    });
                };
            } else if let Some(owner) = self.owner {
                ctx.emit(WorkflowViewEvent::RunWorkflow {
                    workflow: Arc::new(WorkflowType::Local(new_workflow)),
                    source: owner.into(),
                    argument_override: None,
                })
            }
        } else if let Some(workflow) = self.get_cloud_workflow(ctx) {
            if let Some(owner) = self.owner {
                ctx.emit(WorkflowViewEvent::RunWorkflow {
                    workflow: Arc::new(WorkflowType::Cloud(Box::new(workflow))),
                    source: owner.into(),
                    argument_override: Some(self.command_display_data.get_argument_values()),
                });
            } else {
                log::warn!("Invalid space for workflow");
            }
        } else {
            log::warn!("No valid workflow id. Can't run workflow");
        }
    }

    fn display_error_toast(&self, message: String, ctx: &mut ViewContext<Self>) {
        let window_id = ctx.window_id();
        crate::workspace::ToastStack::handle(ctx).update(ctx, |stack, ctx| {
            stack.add_ephemeral_toast(DismissibleToast::error(message), window_id, ctx);
        });
        ctx.notify();
    }

    fn create_workflow_object_from_input(&mut self, ctx: &mut ViewContext<Self>) -> Workflow {
        let content = self.content_editor.as_ref(ctx).buffer_text(ctx);
        let title_in_editor = self.name_editor.as_ref(ctx).buffer_text(ctx);
        let workflow_name = if title_in_editor.is_empty() {
            self.truncate_content_for_title(content.clone())
        } else {
            title_in_editor
        };

        let mut workflow = if self.is_for_agent_mode {
            Workflow::AgentMode {
                name: workflow_name,
                query: content,
                arguments: self.arguments_with_metadata(ctx),
                description: None,
            }
        } else {
            Workflow::Command {
                name: workflow_name,
                command: content,
                description: None,
                arguments: self.arguments_with_metadata(ctx),
                tags: vec![],
                source_url: None,
                author: None,
                author_url: None,
                shells: vec![],
                environment_variables: self.env_vars_state.default_env_vars,
            }
        };

        let workflow_description = self.description_editor.as_ref(ctx).buffer_text(ctx);
        if !workflow_description.is_empty() {
            workflow = workflow.with_description(workflow_description);
        }

        workflow
    }

    /// Save the workflow and associated state. This makes a best-effort attempt to not
    /// unnecessarily modify the backing Warp Drive object.
    fn save(&mut self, ctx: &mut ViewContext<Self>) {
        if FeatureFlag::WorkflowAliases.is_enabled() && self.are_aliases_dirty(ctx) {
            self.save_aliases(ctx);
        }
        if self.is_workflow_dirty(ctx) {
            self.save_workflow(ctx);
        }
    }

    fn save_aliases(&mut self, ctx: &mut ViewContext<Self>) {
        if let Err(e) = self.alias_bar.update(ctx, |bar, ctx| bar.save(ctx)) {
            log::error!("Error saving aliases: {e:?}");
            self.display_error_toast("Error saving aliases".to_string(), ctx);
        }
    }

    fn save_workflow(&mut self, ctx: &mut ViewContext<Self>) {
        let workflow = &self.create_workflow_object_from_input(ctx);

        // Block saving if secrets are detected in the workflow when secret redaction is enabled.
        if self.workflow_contains_secrets(ctx) {
            self.display_error_toast(
                "This workflow cannot be saved because it contains secrets".to_string(),
                ctx,
            );
            return;
        }

        self.save_argument_objects(ctx);

        match self.workflow_view_mode {
            WorkflowViewMode::Edit => {
                UpdateManager::handle(ctx).update(ctx, |update_manager, ctx| {
                    update_manager.update_workflow(
                        workflow.clone(),
                        self.workflow_id,
                        self.revision_ts.clone(),
                        ctx,
                    );
                });
                if let ContainerConfiguration::Pane(pane_config) = &mut self.container_configuration
                {
                    // update the pane title if the workflow title changes
                    pane_config.update(ctx, |pane_config, ctx| {
                        pane_config.set_title(workflow.name().to_owned(), ctx)
                    });
                }

                // after save transition to the view mode if we are allowed to
                self.try_set_view_mode(ctx);
            }
            WorkflowViewMode::Create => {
                let client_id = if let Some(id) = self.workflow_id.into_client() {
                    id
                } else {
                    log::error!("No client_id obtained for creating workflow");
                    self.display_error_toast(String::from("Could not create workflow"), ctx);
                    return;
                };

                if let Some(space) = self.owner {
                    UpdateManager::handle(ctx).update(ctx, |update_manager, ctx| {
                        update_manager.create_workflow(
                            workflow.clone(),
                            space,
                            self.initial_folder_id,
                            client_id,
                            CloudObjectEventEntrypoint::Unknown,
                            true,
                            ctx,
                        );
                    });
                    if let ContainerConfiguration::Pane(pane_config) =
                        &mut self.container_configuration
                    {
                        // update the pane title if the workflow title changes
                        pane_config.update(ctx, |pane_config, ctx| {
                            pane_config.set_title(workflow.name().to_owned(), ctx)
                        });
                    }

                    // after create transition to the view mode
                    self.try_set_view_mode(ctx);
                    ctx.emit(WorkflowViewEvent::CreatedWorkflow(self.workflow_id));
                } else {
                    log::error!("Attempting to create workflow but now space found");
                }
            }
            _ => log::error!("Did not match conditions to either create or save the workflow"),
        }
    }

    fn workflow_contains_secrets(&self, app: &AppContext) -> bool {
        let secret_redaction = get_secret_obfuscation_mode(app);
        if secret_redaction.should_redact_secret() {
            let name_secrets = find_secrets_in_text(&self.name_editor.as_ref(app).buffer_text(app));
            if !name_secrets.is_empty() {
                return true;
            }

            let content_secrets =
                find_secrets_in_text(&self.content_editor.as_ref(app).buffer_text(app));
            if !content_secrets.is_empty() {
                return true;
            }

            let description_secrets =
                find_secrets_in_text(&self.description_editor.as_ref(app).buffer_text(app));
            if !description_secrets.is_empty() {
                return true;
            }

            for arg in self.arguments_rows.iter() {
                if !find_secrets_in_text(&arg.name).is_empty() {
                    return true;
                }
                if !find_secrets_in_text(&arg.description_editor.as_ref(app).buffer_text(app))
                    .is_empty()
                {
                    return true;
                }
                if !find_secrets_in_text(&arg.default_value_editor.as_ref(app).buffer_text(app))
                    .is_empty()
                {
                    return true;
                }
                if !find_secrets_in_text(&arg.argument_editor.as_ref(app).buffer_text(app))
                    .is_empty()
                {
                    return true;
                }
                if !find_secrets_in_text(
                    &arg.arg_type_editor
                        .as_ref(app)
                        .text_editor
                        .as_ref(app)
                        .buffer_text(app),
                )
                .is_empty()
                {
                    return true;
                }
            }
            for value in self.alias_bar.as_ref(app).get_all_argument_values() {
                if !find_secrets_in_text(&value).is_empty() {
                    return true;
                }
            }
        }
        false
    }

    fn copy_content(&mut self, ctx: &mut ViewContext<Self>) {
        // If we are in view mode copy the command or query from the view_only editor
        // otherwise copy it from the content editor
        let editor = if matches!(self.workflow_view_mode, WorkflowViewMode::View) {
            self.view_only_content_editor.as_ref(ctx)
        } else {
            self.content_editor.as_ref(ctx)
        };
        let content = editor.buffer_text(ctx).to_string();
        ctx.clipboard().write(ClipboardContent::plain_text(content));

        let window_id = ctx.window_id();
        crate::workspace::ToastStack::handle(ctx).update(ctx, |stack, ctx| {
            stack.add_ephemeral_toast(
                DismissibleToast::success(if self.is_for_agent_mode {
                    "Prompt copied.".to_string()
                } else {
                    "Command copied.".to_string()
                }),
                window_id,
                ctx,
            );
            ctx.notify();
        });
    }

    fn is_editable(&self) -> bool {
        self.workflow_view_mode.is_editable()
    }

    fn is_ai_assist_button_disabled(&self, app: &AppContext) -> bool {
        // Autofill button should be disabled when there is no content or when there are secrets in the workflow.
        self.content_editor.as_ref(app).is_empty(app)
            || self.show_enum_creation_dialog
            || self.workflow_contains_secrets(app)
    }

    fn clear_content_formatting(&mut self, num_chars_content: usize, ctx: &mut ViewContext<Self>) {
        self.content_editor.update(ctx, |editor, ctx| {
            editor.update_buffer_styles(
                vec![CharOffset::from(0)..CharOffset::from(num_chars_content)],
                TextStyleOperation::default()
                    .clear_foreground_color()
                    .clear_error_underline_color(),
                ctx,
            );
        });
    }

    fn apply_error_underlining_to_content(&mut self, ctx: &mut ViewContext<Self>) {
        let error_ranges = self.arguments_state.invalid_arguments_char_ranges.clone();
        let appearance = Appearance::as_ref(ctx);
        let theme = appearance.theme();
        let ansi_colors = theme.terminal_colors().normal;

        self.content_editor.update(ctx, |editor, ctx| {
            editor.update_buffer_styles(
                error_ranges
                    .iter()
                    .map(|range| CharOffset::from(range.start)..CharOffset::from(range.end)),
                TextStyleOperation::default().set_error_underline_color(
                    AnsiColorIdentifier::Red.to_ansi_color(&ansi_colors).into(),
                ),
                ctx,
            )
        });
    }

    fn apply_argument_highlighting_to_content(&mut self, ctx: &mut ViewContext<Self>) {
        let argument_ranges = self
            .arguments_state
            .valid_arguments_char_ranges_and_arg_index
            .clone();
        let appearance = Appearance::as_ref(ctx);
        let theme = appearance.theme();
        let ansi_colors = theme.terminal_colors().normal;

        self.content_editor.update(ctx, |editor, ctx| {
            editor.update_buffer_styles(
                argument_ranges
                    .iter()
                    .map(|(range, _)| CharOffset::from(range.start)..CharOffset::from(range.end)),
                TextStyleOperation::default().set_foreground_color(
                    AnsiColorIdentifier::Blue.to_ansi_color(&ansi_colors).into(),
                ),
                ctx,
            )
        });
    }

    fn update_breadcrumb(&mut self, ctx: &mut ViewContext<Self>) {
        let workflow = self.get_cloud_workflow(ctx);

        if let Some(the_workflow) = workflow {
            self.breadcrumbs = the_workflow
                .containing_objects_path(ctx)
                .into_iter()
                .map(BreadcrumbState::new)
                .collect();
        } else {
            log::warn!("Workflow not found from cloudmodel, could not update breadcrumb");
        }
    }

    pub fn focus(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.focus(&self.name_editor);
        ctx.emit(WorkflowViewEvent::Pane(PaneEvent::FocusSelf));
    }

    fn is_online(&self, app: &AppContext) -> bool {
        NetworkStatus::as_ref(app).is_online()
    }

    /// Whether or not opening links in the desktop app is supported.
    fn can_open_on_desktop(&self, app: &AppContext) -> bool {
        !ContextFlag::HideOpenOnDesktopButton.is_enabled()
            && *UserAppInstallDetectionSettings::as_ref(app)
                .user_app_installation_detected
                .value()
                == UserAppInstallStatus::Detected
    }

    fn show_unsaved_changes_dialog(
        &mut self,
        unsave_type: UnsavedChangeType,
        ctx: &mut ViewContext<Self>,
    ) {
        self.show_unsaved_changes = Some(unsave_type);
        self.disable_editors(ctx);
        self.update_open_modal_state(ctx);
        ctx.notify();
    }

    fn hide_unsaved_changes_dialog(&mut self, ctx: &mut ViewContext<Self>) {
        self.show_unsaved_changes = None;
        self.enable_editors(ctx);
        self.update_open_modal_state(ctx);
        ctx.notify();
    }

    fn show_enum_creation_dialog(&mut self, ctx: &mut ViewContext<Self>) {
        self.show_enum_creation_dialog = true;
        self.disable_editors(ctx);
        self.update_open_modal_state(ctx);
        ctx.notify();
    }

    fn hide_enum_creation_dialog(&mut self, ctx: &mut ViewContext<Self>) {
        self.show_enum_creation_dialog = false;
        self.enable_editors(ctx);
        self.update_open_modal_state(ctx);
        ctx.notify();
    }

    /// Returns whether or not any of the workflow editor dialogs are open.
    fn has_open_dialog(&self) -> bool {
        self.show_enum_creation_dialog || self.show_unsaved_changes.is_some()
    }

    /// Set the [`PaneConfiguration`] open modal flag based on whether or not any dialogs are open.
    fn update_open_modal_state(&self, ctx: &mut ViewContext<Self>) {
        if let ContainerConfiguration::Pane(pane_config) = &self.container_configuration {
            pane_config.update(ctx, |pane_config, ctx| {
                pane_config.set_has_open_modal(self.has_open_dialog(), ctx);
            });
        }
    }

    /// Adds a default argument, either at the end with a placeholder name if
    /// no text is selected, or in place with the selected text as the arg name.
    fn add_argument(&mut self, ctx: &mut ViewContext<Self>) {
        self.content_editor.update(ctx, |editor, ctx| {
            let selected_text = editor.selected_text(ctx);

            let argument_name = if selected_text.is_empty() {
                self.default_argument_id += 1;
                format!("{}_{}", DEFAULT_ARGUMENT_PREFIX, self.default_argument_id)
            } else {
                String::from(selected_text.trim())
            };

            // this renders out as e.g. `{{argument_X}}` - we need 5 curly brace
            // pairs because we're formatting in rust AND using them literally
            let formatted_argument = format!("{{{{{argument_name}}}}}");
            editor.user_initiated_insert(
                formatted_argument.as_str(),
                EditorAction::SystemInsert,
                ctx,
            );
        });
    }

    fn render_edit_toggle_button(
        &self,
        editability: ContentEditability,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let base_text_styles = UiComponentStyles {
            ..Default::default()
        };

        let text_and_button = match self.workflow_view_mode {
            WorkflowViewMode::Edit => {
                let mode_text = appearance
                    .ui_builder()
                    .span("Editing")
                    .with_style(base_text_styles)
                    .build();
                let edit_button = accent_icon_button(
                    appearance,
                    Icon::Pencil,
                    false,
                    self.ui_state_handles.edit_mode_button_mouse_state.clone(),
                );

                Some((mode_text, edit_button))
            }
            WorkflowViewMode::View => {
                let mode_text = appearance
                    .ui_builder()
                    .span("Viewing")
                    .with_style(base_text_styles)
                    .build();
                let edit_button = icon_button(
                    appearance,
                    Icon::Pencil,
                    false,
                    self.ui_state_handles.edit_mode_button_mouse_state.clone(),
                );

                Some((mode_text, edit_button))
            }
            _ => None,
        };

        if let Some((mode_text, mut edit_button)) = text_and_button {
            if matches!(editability, ContentEditability::RequiresLogin) {
                let ui_builder = appearance.ui_builder().clone();
                edit_button = edit_button.with_tooltip(move || {
                    ui_builder
                        .tool_tip("Sign in to edit".to_string())
                        .build()
                        .finish()
                });
            }
            let edit_button = edit_button.build();

            Flex::row()
                .with_child(
                    Container::new(mode_text.finish())
                        .with_margin_right(5.)
                        .finish(),
                )
                .with_child(
                    edit_button
                        .on_click(|ctx, _, _| {
                            ctx.dispatch_typed_action(WorkflowAction::ToggleViewMode)
                        })
                        .finish(),
                )
                .with_main_axis_size(MainAxisSize::Min)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .finish()
        } else {
            Flex::row().finish()
        }
    }

    fn duplicate_object(&mut self, ctx: &mut ViewContext<Self>) {
        if self.show_enum_creation_dialog {
            return;
        }

        UpdateManager::handle(ctx).update(ctx, |update_manager, ctx| {
            update_manager.duplicate_object(
                &CloudObjectTypeAndId::from_id_and_type(self.workflow_id, ObjectType::Workflow),
                ctx,
            );
        });
        ctx.notify();
    }

    fn trash_object(&mut self, ctx: &mut ViewContext<Self>) {
        if self.show_enum_creation_dialog {
            return;
        }

        self.close(ctx);

        UpdateManager::handle(ctx).update(ctx, move |update_manager, ctx| {
            update_manager.trash_object(
                CloudObjectTypeAndId::from_id_and_type(self.workflow_id, ObjectType::Workflow),
                ctx,
            );
        });
    }

    fn untrash_object(&self, ctx: &mut ViewContext<Self>) {
        if has_feature_gated_anonymous_user_reached_workflow_limit(ctx) {
            return;
        }

        UpdateManager::handle(ctx).update(ctx, move |update_manager, ctx| {
            update_manager.untrash_object(
                CloudObjectTypeAndId::from_id_and_type(self.workflow_id, ObjectType::Workflow),
                ctx,
            );
        });
    }

    fn get_cloud_workflow(&mut self, ctx: &mut ViewContext<Self>) -> Option<CloudWorkflow> {
        if let Some(workflow) = CloudModel::as_ref(ctx).get_workflow(&self.workflow_id.clone()) {
            return Some(workflow.clone());
        } else {
            log::warn!(
                "Workflow for id: {} not found in cloudmodel",
                self.workflow_id
            );
        }

        None
    }

    fn render_workflow_details(&self, appearance: &Appearance) -> Box<dyn Element> {
        let workflow_icon = Container::new(
            ConstrainedBox::new(
                if self.is_for_agent_mode {
                    Icon::Prompt
                } else {
                    Icon::Workflow
                }
                .to_warpui_icon(
                    warp_drive_icon_color(
                        appearance,
                        if self.is_for_agent_mode {
                            DriveObjectType::AgentModeWorkflow
                        } else {
                            DriveObjectType::Workflow
                        },
                    )
                    .into(),
                )
                .finish(),
            )
            .with_width(WORKFLOW_ICON_DIMENSIONS)
            .with_height(WORKFLOW_ICON_DIMENSIONS)
            .finish(),
        )
        .finish();

        let mut command_icon_buttons = Flex::row();

        if !self.is_editable() || ContextFlag::RunWorkflow.is_enabled() {
            command_icon_buttons.add_child(
                icon_button(
                    appearance,
                    Icon::Copy,
                    false,
                    self.ui_state_handles
                        .copy_content_button_mouse_state
                        .clone(),
                )
                .build()
                .on_click(|ctx, _, _| ctx.dispatch_typed_action(WorkflowAction::CopyContent))
                .finish(),
            );

            if ContextFlag::RunWorkflow.is_enabled() {
                command_icon_buttons.add_child(
                    Container::new(
                        icon_button(
                            appearance,
                            Icon::TerminalInput,
                            false,
                            self.ui_state_handles.execute_command_mouse_state.clone(),
                        )
                        .build()
                        .on_click(|ctx, _, _| {
                            ctx.dispatch_typed_action(WorkflowAction::RunWorkflow)
                        })
                        .finish(),
                    )
                    .with_margin_left(4.)
                    .finish(),
                );
            }
        }

        let content_editor_to_use = if self.is_editable() {
            &self.content_editor
        } else {
            &self.view_only_content_editor
        };

        Container::new(
            ConstrainedBox::new(
                Flex::column()
                    .with_child(
                        Flex::row()
                            .with_cross_axis_alignment(CrossAxisAlignment::Center)
                            .with_child(workflow_icon)
                            .with_child(
                                Shrinkable::new(
                                    1.,
                                    Container::new(ChildView::new(&self.name_editor).finish())
                                        .with_margin_left(DETAIL_TEXT_MARGIN_LEFT)
                                        .finish(),
                                )
                                .finish(),
                            )
                            .finish(),
                    )
                    .with_child(
                        Container::new(ChildView::new(&self.description_editor).finish())
                            .with_margin_top(DESCRIPTION_MARGIN_TOP)
                            .with_margin_left(WORKFLOW_ICON_DIMENSIONS + DETAIL_TEXT_MARGIN_LEFT)
                            .finish(),
                    )
                    .with_child(
                        Container::new(ChildView::new(content_editor_to_use).finish())
                            .with_margin_top(COMMAND_MARGIN_TOP)
                            .with_margin_left(WORKFLOW_ICON_DIMENSIONS + DETAIL_TEXT_MARGIN_LEFT)
                            .finish(),
                    )
                    .with_child(
                        Container::new(
                            ConstrainedBox::new(
                                command_icon_buttons
                                    .with_main_axis_alignment(MainAxisAlignment::End)
                                    .with_main_axis_size(MainAxisSize::Max)
                                    .finish(),
                            )
                            .with_height(24.)
                            .finish(),
                        )
                        .with_margin_left(WORKFLOW_ICON_DIMENSIONS + DETAIL_TEXT_MARGIN_LEFT)
                        .finish(),
                    )
                    .finish(),
            )
            .with_min_height(40.)
            .finish(),
        )
        .with_padding_top(DETAIL_BOX_PADDING_TOP_AND_LEFT)
        .with_padding_left(DETAIL_BOX_PADDING_TOP_AND_LEFT)
        .with_padding_bottom(DETAIL_BOX_PADDING_BOTTOM_AND_RIGHT)
        .with_padding_right(DETAIL_BOX_PADDING_BOTTOM_AND_RIGHT)
        .with_border(Border::all(1.).with_border_fill(appearance.theme().outline()))
        .with_background(appearance.theme().surface_overlay_1())
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
            WORKFLOW_CORNER_RADIUS,
        )))
        .finish()
    }

    fn render_section_header(
        &self,
        text: &'static str,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        Container::new(
            appearance
                .ui_builder()
                .span(text)
                .with_style(UiComponentStyles {
                    font_size: Some(SECTION_FONT_SIZE),
                    font_weight: Some(Weight::Bold),
                    ..Default::default()
                })
                .build()
                .finish(),
        )
        .with_margin_top(SECTION_SPACING)
        .with_margin_bottom(SECTION_SPACING)
        .finish()
    }

    fn render_alias_section(&self, appearance: &Appearance) -> Box<dyn Element> {
        let ui_builder = appearance.ui_builder().clone();

        let help_icon = Hoverable::new(
            self.ui_state_handles.alias_header_tool_tip.clone(),
            |state| {
                let mut stack = Stack::new().with_child(
                    ConstrainedBox::new(
                        Icon::HelpCircle
                            .to_warpui_icon(
                                appearance
                                    .theme()
                                    .sub_text_color(appearance.theme().background()),
                            )
                            .finish(),
                    )
                    .with_width(12.)
                    .with_height(12.)
                    .finish(),
                );

                if state.is_hovered() {
                    let tooltip = ConstrainedBox::new(
                        ui_builder
                            .tool_tip(ALIAS_HELP_TEXT.to_string())
                            .build()
                            .finish(),
                    )
                    .with_max_width(200.)
                    .finish();
                    stack.add_positioned_child(
                        tooltip,
                        OffsetPositioning::offset_from_parent(
                            vec2f(2., -2.),
                            ParentOffsetBounds::WindowByPosition,
                            ParentAnchor::TopRight,
                            ChildAnchor::BottomLeft,
                        ),
                    );
                }

                stack.finish()
            },
        )
        .finish();

        Flex::column()
            .with_children([
                Flex::row()
                    .with_children([
                        self.render_section_header("Aliases", appearance),
                        Container::new(help_icon).with_margin_left(4.).finish(),
                    ])
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .finish(),
                ChildView::new(&self.alias_bar).finish(),
            ])
            .finish()
    }

    fn render_unsaved_changes_dialog(
        &self,
        confirm_action: WorkflowAction,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let keep_editing_button = appearance
            .ui_builder()
            .button(
                ButtonVariant::Secondary,
                self.ui_state_handles.keep_editing_state.clone(),
            )
            .with_style(UiComponentStyles {
                font_size: Some(BUTTON_FONT_SIZE),
                font_weight: Some(Weight::Bold),
                padding: Some(Coords::uniform(BUTTON_PADDING)),
                ..Default::default()
            })
            .with_text_label(KEEP_EDITING_TEXT.into())
            .build()
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(WorkflowAction::CloseUnsavedDialog)
            })
            .finish();

        let discard_changes_button = appearance
            .ui_builder()
            .button(
                ButtonVariant::Secondary,
                self.ui_state_handles.discard_changes_state.clone(),
            )
            .with_style(UiComponentStyles {
                font_size: Some(BUTTON_FONT_SIZE),
                font_weight: Some(Weight::Bold),
                padding: Some(Coords::uniform(BUTTON_PADDING)),
                ..Default::default()
            })
            .with_text_label(DISCARD_CHANGES_TEXT.into())
            .build()
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(confirm_action.clone());
            })
            .finish();

        Container::new(
            Dialog::new(
                UNSAVED_CHANGES_TEXT.to_string(),
                None,
                dialog_styles(appearance),
            )
            .with_bottom_row_child(keep_editing_button)
            .with_bottom_row_child(discard_changes_button)
            .with_width(DIALOG_WIDTH)
            .build()
            .finish(),
        )
        .with_margin_left(MODAL_HORIZONTAL_MARGIN)
        .with_margin_right(MODAL_HORIZONTAL_MARGIN)
        .finish()
    }

    fn build_footer_button(
        &self,
        variant: ButtonVariant,
        label: String,
        icon: Option<(Icon, TextAndIconAlignment)>,
        mouse_state: MouseStateHandle,
        appearance: &Appearance,
    ) -> Button {
        let default_button_styles = UiComponentStyles {
            font_size: Some(BUTTON_FONT_SIZE),
            border_radius: Some(CornerRadius::with_all(Radius::Pixels(BUTTON_BORDER_RADIUS))),
            ..Default::default()
        };

        // Hack to make the button size be exactly 32px with a font size of 14.
        let button_padding = Coords {
            top: 6.,
            bottom: 7.,
            left: 6.,
            right: 6.,
        };

        let button = appearance
            .ui_builder()
            .button(variant, mouse_state)
            .with_style(UiComponentStyles {
                height: Some(BUTTON_HEIGHT),
                font_weight: Some(Weight::Bold),
                padding: Some(button_padding),
                ..default_button_styles
            });

        match icon {
            None => button.with_text_label(label),
            Some((icon, alignment)) => {
                let text_and_icon = TextAndIcon::new(
                    alignment,
                    label,
                    icon.to_warpui_icon(appearance.theme().active_ui_text_color()),
                    MainAxisSize::Min,
                    MainAxisAlignment::Center,
                    vec2f(10., 10.),
                )
                .with_inner_padding(4.);
                button.with_text_and_icon_label(text_and_icon)
            }
        }
    }

    fn render_button_row(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        let mut save_button = self.build_footer_button(
            ButtonVariant::Accent,
            match self.workflow_view_mode {
                WorkflowViewMode::Create => CREATE_BUTTON_TEXT.into(),
                WorkflowViewMode::Edit | WorkflowViewMode::View => SAVE_BUTTON_TEXT.into(),
            },
            None,
            self.ui_state_handles.save_workflow_state.clone(),
            appearance,
        );

        // If there is a reason the button should be disabled (e.g. enum dialog open)
        // Or
        //  - if we are not in valid new mode
        //  - AND the inputs are not dirty
        if self.is_save_workflow_button_disabled()
            || (!self.is_dirty(app) && !self.can_save_new(app))
        {
            save_button = save_button.disabled();
        }

        let mut cancel_button = self.build_footer_button(
            ButtonVariant::Secondary,
            CANCEL_BUTTON_TEXT.into(),
            None,
            self.ui_state_handles.cancel_mouse_state.clone(),
            appearance,
        );

        if self.show_enum_creation_dialog {
            cancel_button = cancel_button.disabled();
        }

        let render_cancel_button = cancel_button
            .build()
            .with_cursor(Cursor::PointingHand)
            .on_click(|ctx, _, _| ctx.dispatch_typed_action(WorkflowAction::Cancel))
            .finish();

        let render_save_button = save_button
            .build()
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, _| ctx.dispatch_typed_action(WorkflowAction::Save))
            .finish();

        let mut button_row = Flex::row();

        let label_and_icon = match self.ai_metadata_assist_state {
            AiAssistState::PreRequest => Some((AI_ASSIST_BUTTON_TEXT, Icon::AiAssistant)),
            AiAssistState::RequestInFlight => Some((AI_ASSIST_LOADING_TEXT, Icon::Refresh)),
            AiAssistState::Generated => None,
        };

        if let Some((label, icon)) = label_and_icon {
            // AI-generated workflow metadata is only supported for Command workflows currently.
            if AISettings::as_ref(app).is_any_ai_enabled(app)
                && self.is_editable()
                && !self.is_for_agent_mode
            {
                let mut button = self
                    .build_footer_button(
                        ButtonVariant::Secondary,
                        label.to_string(),
                        Some((icon, TextAndIconAlignment::TextFirst)),
                        self.ui_state_handles.ai_assist_state.clone(),
                        appearance,
                    )
                    .with_style(UiComponentStyles {
                        width: Some(AI_ASSIST_BUTTON_SIZE),
                        ..Default::default()
                    });

                if self.is_ai_assist_button_disabled(app) {
                    button = button.disabled();
                }

                let rendered_button = button
                    .build()
                    .with_cursor(Cursor::PointingHand)
                    .on_click(move |ctx, _, _| ctx.dispatch_typed_action(WorkflowAction::AiAssist))
                    .finish();

                let button_with_tool_tip = appearance.ui_builder().tool_tip_on_element(
                    "Generate a title, descriptions, or parameters with Warp AI".to_string(),
                    self.ui_state_handles.ai_assist_tool_tip.clone(),
                    rendered_button,
                    ParentAnchor::TopMiddle,
                    ChildAnchor::BottomMiddle,
                    vec2f(0., 5.),
                );

                button_row.add_child(
                    Container::new(button_with_tool_tip)
                        .with_margin_right(8.)
                        .finish(),
                )
            }
        }

        if self.is_editable() {
            // If we are in a context where we can't run workflows and are in the edit mode, then
            // show the cancel button
            if !ContextFlag::RunWorkflow.is_enabled()
                && matches!(self.workflow_view_mode, WorkflowViewMode::Edit)
            {
                button_row.add_child(
                    Container::new(render_cancel_button)
                        .with_margin_right(8.)
                        .finish(),
                );
            }
            button_row.add_child(render_save_button);
        }

        // If on the web, then show a button to run this workflow on the desktop.
        if !ContextFlag::RunWorkflow.is_enabled() && self.can_open_on_desktop(app) {
            if let Some(url) = self
                .workflow_link(app)
                .and_then(|link| Url::parse(&link).ok())
            {
                let run_on_desktop_button = self
                    .build_footer_button(
                        ButtonVariant::Accent,
                        RUN_ON_DESKTOP_BUTTON_TEXT.to_string(),
                        Some((Icon::Laptop, TextAndIconAlignment::IconFirst)),
                        // Reuse the execute button's handle since it's only shown if running workflows is
                        // supported.
                        self.ui_state_handles.execute_command_mouse_state.clone(),
                        appearance,
                    )
                    .with_style(UiComponentStyles {
                        width: Some(RUN_ON_DESKTOP_BUTTON_WIDTH),
                        ..Default::default()
                    })
                    .build()
                    .with_cursor(Cursor::PointingHand)
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(WorkflowAction::OpenLinkOnDesktop(url.clone()))
                    })
                    .finish();
                button_row.add_child(
                    Container::new(run_on_desktop_button)
                        .with_margin_left(8.)
                        .finish(),
                );
            }
        }

        Flex::column()
            .with_child(
                ConstrainedBox::new(
                    button_row
                        .with_main_axis_size(MainAxisSize::Max)
                        .with_main_axis_alignment(MainAxisAlignment::End)
                        .finish(),
                )
                .with_max_width(MAX_ELEMENT_WIDTH)
                .finish(),
            )
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .finish()
    }

    fn create_editor_handle(
        ctx: &mut ViewContext<Self>,
        font_size_override: Option<f32>,
        font_family_override: Option<FamilyId>,
        placeholder_text: Option<&str>,
        supports_vim_mode: bool,
        single_line: bool,
        soft_wrap: bool,
    ) -> ViewHandle<EditorView> {
        let text = TextOptions {
            font_size_override,
            font_family_override,
            ..Default::default()
        };
        ctx.add_typed_action_view(|ctx| {
            let mut editor = if single_line {
                EditorView::single_line(
                    SingleLineEditorOptions {
                        text,
                        propagate_and_no_op_vertical_navigation_keys:
                            PropagateAndNoOpNavigationKeys::Always,
                        soft_wrap,
                        // placeholder should soft_wrap if we allow input to soft_wrap
                        placeholder_soft_wrap: soft_wrap,
                        ..Default::default()
                    },
                    ctx,
                )
            } else {
                EditorView::new(
                    EditorOptions {
                        text,
                        soft_wrap,
                        placeholder_soft_wrap: soft_wrap,
                        propagate_and_no_op_vertical_navigation_keys:
                            PropagateAndNoOpNavigationKeys::Always,
                        supports_vim_mode,
                        single_line,
                        enter_settings: EnterSettings {
                            enter: EnterAction::InsertNewLineIfMultiLine,
                            shift_enter: EnterAction::InsertNewLineIfMultiLine,
                            alt_enter: EnterAction::InsertNewLineIfMultiLine,
                            ..Default::default()
                        },
                        ..Default::default()
                    },
                    ctx,
                )
            };

            // We want to set autogrow regardless for single or non-single line as long as we allow
            // soft wrapping
            editor.set_autogrow(soft_wrap);

            if let Some(text) = placeholder_text {
                editor.set_placeholder_text(text, ctx);
            }

            editor
        })
    }

    fn view_in_warp_drive(&mut self, id: WarpDriveItemId, ctx: &mut ViewContext<Self>) {
        ctx.emit(WorkflowViewEvent::ViewInWarpDrive(id));
    }

    fn issue_request(&mut self, ctx: &mut ViewContext<Self>) {
        let ai_client = self.ai_client.clone();
        let command = self.content_editor.as_ref(ctx).buffer_text(ctx);
        let raw_request = command.trim().to_string();

        ctx.spawn(
            async move { ai_client.generate_metadata_for_command(raw_request).await },
            move |pane, response, ctx| {
                match response {
                    Ok(metadata) => {
                        pane.ai_metadata_assist_state = AiAssistState::Generated;
                        pane.enable_editors(ctx);

                        let arguments = metadata
                            .arguments
                            .into_iter()
                            .map(|parameter| Argument {
                                name: parameter.name,
                                description: Some(parameter.description),
                                default_value: Some(parameter.default_value),
                                arg_type: Default::default(),
                            })
                            .collect_vec();

                        let workflow = Workflow::Command {
                            name: metadata.title,
                            description: Some(metadata.description),
                            command: metadata.command,
                            arguments,
                            tags: vec![],
                            source_url: None,
                            author: None,
                            author_url: None,
                            shells: vec![],
                            environment_variables: None,
                        };

                        send_telemetry_from_ctx!(
                            TelemetryEvent::AutoGenerateMetadataSuccess,
                            ctx
                        );

                        pane.populate_missing_field_with_suggestion(workflow, ctx);
                        ctx.notify();
                    }
                    Err(err) => {
                        let message = err.user_facing_message();
                        if let GeneratedCommandMetadataError::RateLimited = err {
                            let current_user_id = pane.auth_state.user_id().unwrap_or_default();
                            if let Some(team) = UserWorkspaces::as_ref(ctx).current_team() {
                                let current_user_email =
                                    pane.auth_state.user_email().unwrap_or_default();
                                let has_admin_permissions = team.has_admin_permissions(&current_user_email);
                                if team.billing_metadata.can_upgrade_to_higher_tier_plan() {
                                    if has_admin_permissions {
                                        pane.display_upgrade_error(Some(team.uid), current_user_id, ctx);
                                    } else {
                                        pane.display_error_toast(
                                            "Looks like you're out of AI credits. Contact a team admin to upgrade for more credits.".to_string(),
                                            ctx,
                                        );
                                    }
                                } else {
                                    pane.display_error_toast(
                                        message.clone(),
                                        ctx,
                                    );
                                }
                            } else {
                                pane.display_upgrade_error(None, current_user_id, ctx);
                            }
                        } else {
                            pane.display_error_toast(
                                message.clone(),
                                ctx,
                            );
                        }

                        send_telemetry_from_ctx!(
                            TelemetryEvent::AutoGenerateMetadataError {
                                error_payload: serde_json::json!(err)
                            },
                            ctx
                        );

                        pane.ai_metadata_assist_state = AiAssistState::PreRequest;
                        pane.enable_editors(ctx);
                        ctx.notify();
                    }
                }
                AIRequestUsageModel::handle(ctx).update(ctx, |request_usage_model, ctx| {
                    request_usage_model.refresh_request_usage_async(ctx);
                });
            }
        );

        self.ai_metadata_assist_state = AiAssistState::RequestInFlight;
        self.disable_editors(ctx);
        ctx.notify();
    }

    fn display_upgrade_error(
        &mut self,
        team_uid: Option<ServerId>,
        user_id: UserUid,
        ctx: &mut ViewContext<Self>,
    ) {
        let upgrade_link = team_uid
            .map(UserWorkspaces::upgrade_link_for_team)
            .unwrap_or_else(|| UserWorkspaces::upgrade_link(user_id));

        let window_id = ctx.window_id();
        let toast_link = if self.auth_state.is_anonymous_or_logged_out() {
            ToastLink::new("Upgrade for more credits.".into())
                .with_onclick_action(WorkspaceAction::AttemptLoginGatedAIUpgrade)
        } else {
            ToastLink::new("Upgrade for more credits.".into()).with_href(upgrade_link)
        };

        crate::workspace::ToastStack::handle(ctx).update(ctx, |stack, ctx| {
            stack.add_ephemeral_toast(
                DismissibleToast::error("Looks like you're out of AI credits.".into())
                    .with_link(toast_link),
                window_id,
                ctx,
            );
            ctx.notify();
        });
    }

    // Populate only the missing field in the workflow editor with the generated suggestion from AI.
    fn populate_missing_field_with_suggestion(
        &mut self,
        workflow: Workflow,
        ctx: &mut ViewContext<Self>,
    ) {
        self.name_editor.update(ctx, |editor, ctx| {
            if editor.is_empty(ctx) {
                editor.set_buffer_text(workflow.name(), ctx);
            }
        });

        self.description_editor.update(ctx, |editor, ctx| {
            if editor.is_empty(ctx) {
                editor.set_buffer_text(
                    workflow
                        .description()
                        .map(String::as_str)
                        .unwrap_or_default(),
                    ctx,
                );
            }
        });

        let content_parsed = !self.arguments_state.arguments.is_empty();
        if !content_parsed {
            self.content_editor.update(ctx, |editor, ctx| {
                editor.set_buffer_text(workflow.content(), ctx);
            });

            // note: normally, we wouldn't have to do this, since editing the command
            // editor's text will trigger the event that does this automatically.
            // however, that happens in a callback, yet we need to know what the args
            // are right away to populate the description/default value editors.
            self.arguments_state = ArgumentsState::for_command_workflow(
                &self.arguments_state,
                workflow.content().to_string(),
            );
            self.update_arguments_rows(ctx);

            workflow
                .arguments()
                .iter()
                .enumerate()
                .for_each(|(index, argument)| {
                    // Since suggestion generated by AI is non-deterministic, we should make sure to handle each
                    // operation safely.
                    if index >= self.arguments_rows.len() {
                        return;
                    }

                    if let Some(description) = &argument.description {
                        self.arguments_rows[index]
                            .description_editor
                            .update(ctx, |editor, ctx| {
                                editor.set_buffer_text(description.as_str(), ctx);
                            });
                    }

                    if let Some(default_value) = &argument.default_value {
                        self.arguments_rows[index].default_value_editor.update(
                            ctx,
                            |editor, ctx| {
                                editor.set_buffer_text(default_value.as_str(), ctx);
                            },
                        );
                    }
                });
        }
    }

    pub(super) fn render_trash_banner(&self, app: &AppContext) -> Option<Box<dyn Element>> {
        let cloud_model = CloudModel::as_ref(app);
        let deleted = if matches!(self.workflow_view_mode, WorkflowViewMode::Create) {
            return None;
        } else {
            match cloud_model.get_workflow(&self.workflow_id) {
                Some(notebook) => {
                    if notebook.is_trashed(cloud_model) {
                        false
                    } else {
                        return None;
                    }
                }
                None => true,
            }
        };

        let appearance = Appearance::as_ref(app);
        let text = if deleted {
            "You no longer have access to this workflow"
        } else {
            "Workflow moved to trash"
        };

        let mut stack = Stack::new();
        stack.add_child(
            Align::new(
                Flex::row()
                    .with_children([
                        ConstrainedBox::new(
                            Icon::Trash
                                .to_warpui_icon(appearance.theme().foreground())
                                .finish(),
                        )
                        .with_width(16.)
                        .with_height(16.)
                        .finish(),
                        appearance
                            .ui_builder()
                            .span(text)
                            .with_style(UiComponentStyles {
                                font_size: Some(appearance.ui_font_size() + 2.),
                                ..Default::default()
                            })
                            .build()
                            .with_padding_left(8.)
                            .finish(),
                    ])
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_main_axis_alignment(MainAxisAlignment::Center)
                    .with_main_axis_size(MainAxisSize::Max)
                    .finish(),
            )
            .finish(),
        );

        let action_row = if deleted {
            Shrinkable::new(1., Empty::new().finish()).finish()
        } else {
            let mut action_row = Flex::row()
                .with_main_axis_alignment(MainAxisAlignment::End)
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Center);

            let ui_builder = appearance.ui_builder().clone();
            action_row.add_child(
                Align::new(
                    appearance
                        .ui_builder()
                        .button(
                            ButtonVariant::Basic,
                            self.ui_state_handles.restore_from_trash_button.clone(),
                        )
                        .with_tooltip(move || {
                            ui_builder
                                .tool_tip("Restore workflow from trash".to_string())
                                .build()
                                .finish()
                        })
                        .with_text_label("Restore".to_string())
                        .build()
                        .on_click(|ctx, _, _| ctx.dispatch_typed_action(WorkflowAction::Untrash))
                        .finish(),
                )
                .finish(),
            );

            action_row.finish()
        };

        stack.add_child(Align::new(action_row).right().finish());

        Some(
            Container::new(
                ConstrainedBox::new(stack.finish())
                    .with_min_height(40.)
                    .finish(),
            )
            .with_horizontal_padding(16.)
            .with_background(appearance.theme().surface_2())
            .finish(),
        )
    }
}

impl Entity for WorkflowView {
    type Event = WorkflowViewEvent;
}

impl View for WorkflowView {
    fn ui_name() -> &'static str {
        "WorkflowView"
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused()
            && matches!(
                self.container_configuration,
                ContainerConfiguration::SuggestionDialog
            )
        {
            ctx.focus(&self.content_editor);
        }
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let mut content = Flex::column();
        content.extend(self.render_trash_banner(app));

        let mut main_section = Flex::column();
        main_section.add_child(self.render_workflow_details(appearance));

        if FeatureFlag::WorkflowAliases.is_enabled() && !self.is_for_agent_mode {
            main_section.add_child(self.render_alias_section(appearance));
        }

        main_section.extend(self.render_arguments_section(appearance, app));

        let vertical_margin = match &self.container_configuration {
            ContainerConfiguration::Pane(_) => CORE_VERTICAL_MARGIN_IN_PANE,
            ContainerConfiguration::SuggestionDialog => 0.,
        };

        let mut row = Flex::row();
        row.add_child(
            Shrinkable::new(
                2.,
                Container::new(render_breadcrumbs(
                    self.breadcrumbs.clone(),
                    appearance,
                    |ctx, _, breadcrumb| {
                        ctx.dispatch_typed_action(WorkflowAction::ViewInWarpDrive(
                            breadcrumb.kind.into_item_id(),
                        ));
                    },
                ))
                .with_horizontal_margin(CORE_HORIZONATAL_MARGIN)
                .with_vertical_margin(vertical_margin / 2.)
                .finish(),
            )
            .finish(),
        );

        let editability = if FeatureFlag::SharedWithMe.is_enabled() {
            self.editability(app)
        } else {
            ContentEditability::Editable
        };
        let mode_toggleable = match (ContextFlag::RunWorkflow.is_enabled(), editability) {
            // If logging in would allow editing, show the toggle for discoverability.
            (_, ContentEditability::RequiresLogin) => true,
            // If workflows aren't runnable (so view mode is enabled) AND the user can edit the
            // workflow, both view and edit modes are allowed.
            (false, ContentEditability::Editable) => true,
            // Otherwise, only one of view and edit mode is allowed.
            (_, _) => false,
        };

        if mode_toggleable {
            row.add_child(
                Shrinkable::new(
                    1.,
                    Container::new(self.render_edit_toggle_button(editability, appearance))
                        .with_margin_right(CORE_HORIZONATAL_MARGIN)
                        .finish(),
                )
                .finish(),
            )
        }

        content.add_child(
            row.with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_main_axis_size(MainAxisSize::Max)
                .finish(),
        );

        // We use a stack here with two children — an expanded transparent box and
        // container with the core ui elements. The core UI elements are layered
        // on top of the box, which is used to "pin" the footer elements to the bottom.
        let mut stack = Stack::new();

        stack.add_child(
            Shrinkable::new(
                1.,
                Rect::new()
                    .with_background_color(ColorU::transparent_black())
                    .finish(),
            )
            .finish(),
        );

        // In the pane configuration, we render the button row above the main section.
        if matches!(
            self.container_configuration,
            ContainerConfiguration::Pane(_)
        ) {
            content.add_child(
                Container::new(self.render_button_row(appearance, app))
                    .with_margin_bottom(SECTION_SPACING)
                    .with_horizontal_margin(CORE_HORIZONATAL_MARGIN + ScrollbarWidth::Auto.as_f32())
                    .finish(),
            );
        }

        stack.add_child(
            ClippedScrollable::vertical(
                self.ui_state_handles.clipped_scroll_state.clone(),
                Container::new(
                    Flex::column()
                        .with_child(
                            ConstrainedBox::new(main_section.finish())
                                .with_max_width(MAX_ELEMENT_WIDTH)
                                .finish(),
                        )
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .finish(),
                )
                .with_horizontal_margin(CORE_HORIZONATAL_MARGIN)
                .with_margin_bottom(vertical_margin)
                .finish(),
                SCROLLBAR_WIDTH,
                theme.nonactive_ui_detail().into(),
                theme.active_ui_detail().into(),
                warpui::elements::Fill::None,
            )
            .finish(),
        );

        if self.show_enum_creation_dialog {
            stack.add_positioned_overlay_child(
                Clipped::new(ChildView::new(&self.enum_creation_dialog).finish()).finish(),
                OffsetPositioning::offset_from_parent(
                    vec2f(0., 0.),
                    ParentOffsetBounds::WindowByPosition,
                    ParentAnchor::Center,
                    ChildAnchor::Center,
                ),
            )
        }

        match self.show_unsaved_changes {
            Some(UnsavedChangeType::ForEdit) => stack.add_positioned_overlay_child(
                self.render_unsaved_changes_dialog(WorkflowAction::Cancel, appearance),
                OffsetPositioning::offset_from_parent(
                    vec2f(0., 0.),
                    ParentOffsetBounds::WindowByPosition,
                    ParentAnchor::Center,
                    ChildAnchor::Center,
                ),
            ),
            Some(UnsavedChangeType::ForClose) => stack.add_positioned_child(
                self.render_unsaved_changes_dialog(WorkflowAction::ForceClose, appearance),
                OffsetPositioning::offset_from_parent(
                    vec2f(0., 0.),
                    ParentOffsetBounds::WindowByPosition,
                    ParentAnchor::Center,
                    ChildAnchor::Center,
                ),
            ),
            None => {}
        }

        content.add_child(Shrinkable::new(1., stack.finish()).finish());

        // In the suggestion dialog configuration, we render the button row below the main section.
        if matches!(
            self.container_configuration,
            ContainerConfiguration::SuggestionDialog
        ) {
            content.add_child(
                Container::new(self.render_button_row(appearance, app))
                    .with_horizontal_margin(CORE_HORIZONATAL_MARGIN + ScrollbarWidth::Auto.as_f32())
                    .finish(),
            );
        }

        let content = content
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .finish();
        if self.has_open_dialog() {
            Container::new(content)
                .with_foreground_overlay(theme.inactive_pane_overlay())
                .finish()
        } else {
            content
        }
    }
}

impl TypedActionView for WorkflowView {
    type Action = WorkflowAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            WorkflowAction::ViewInWarpDrive(id) => self.view_in_warp_drive(*id, ctx),
            WorkflowAction::AddArgument => self.add_argument(ctx),
            WorkflowAction::ToggleViewMode => self.toggle_view_mode(ctx),
            WorkflowAction::CloseUnsavedDialog => self.hide_unsaved_changes_dialog(ctx),
            WorkflowAction::Close => {
                self.close(ctx);
            }
            WorkflowAction::ForceClose => {
                self.hide_unsaved_changes_dialog(ctx);
                self.emit_close_event(ctx);
            }
            WorkflowAction::Save => self.save(ctx),
            WorkflowAction::Cancel => {
                if !self.show_enum_creation_dialog {
                    // we reset when we toggle to the view mode so no need to call reset here
                    self.hide_unsaved_changes_dialog(ctx);
                    self.try_set_view_mode(ctx);
                }
            }
            WorkflowAction::RunWorkflow => self.copy_to_command_line(ctx),
            WorkflowAction::CopyContent => self.copy_content(ctx),
            WorkflowAction::AiAssist => self.issue_request(ctx),
            WorkflowAction::Duplicate => self.duplicate_object(ctx),
            WorkflowAction::CopyLink(link) => {
                send_telemetry_from_ctx!(
                    TelemetryEvent::ObjectLinkCopied { link: link.clone() },
                    ctx
                );
                ctx.clipboard()
                    .write(ClipboardContent::plain_text(link.to_owned()));
            }
            #[cfg(target_family = "wasm")]
            WorkflowAction::OpenLinkOnDesktop(url) => {
                send_telemetry_from_ctx!(
                    TelemetryEvent::WebCloudObjectOpenedOnDesktop {
                        object_metadata: self.telemetry_metadata(ctx)
                    },
                    ctx
                );
                open_url_on_desktop(url);
            }
            #[cfg(not(target_family = "wasm"))]
            WorkflowAction::OpenLinkOnDesktop(_) => {
                // No-op when not on wasm
            }
            WorkflowAction::Trash => self.trash_object(ctx),
            WorkflowAction::Untrash => self.untrash_object(ctx),
            WorkflowAction::CloseEnumDialog => self.hide_enum_creation_dialog(ctx),
        }
    }
}

impl BackingView for WorkflowView {
    type PaneHeaderOverflowMenuAction = WorkflowAction;
    type CustomAction = ();
    type AssociatedData = ();

    fn handle_pane_header_overflow_menu_action(
        &mut self,
        action: &Self::PaneHeaderOverflowMenuAction,
        ctx: &mut ViewContext<Self>,
    ) {
        self.handle_action(action, ctx);
    }

    fn pane_header_overflow_menu_items(&self, ctx: &AppContext) -> Vec<MenuItem<WorkflowAction>> {
        let mut menu_items = Vec::new();

        // Add "Copy Link" to menu
        if let Some(link) = self.workflow_link(ctx) {
            menu_items.push(
                MenuItemFields::new("Copy link")
                    .with_on_select_action(WorkflowAction::CopyLink(link))
                    .with_icon(Icon::Link)
                    .into_item(),
            );
        }

        if self.can_open_on_desktop(ctx) {
            if let Some(link) = self.workflow_link(ctx) {
                if let Ok(url) = Url::parse(&link) {
                    menu_items.push(
                        MenuItemFields::new("Open on Desktop")
                            .with_on_select_action(WorkflowAction::OpenLinkOnDesktop(url))
                            .with_icon(Icon::Laptop)
                            .into_item(),
                    );
                }
            }
        }

        let space = CloudViewModel::as_ref(ctx).object_space(&self.workflow_id.uid(), ctx);

        // Add "Duplicate" to menu
        if space != Some(Space::Shared) {
            menu_items.push(
                MenuItemFields::new("Duplicate")
                    .with_on_select_action(WorkflowAction::Duplicate)
                    .with_icon(Icon::Duplicate)
                    .into_item(),
            );
        }

        // Add "Trash" to menu
        let access_level = self.access_level(ctx);
        if self.is_online(ctx)
            && (!FeatureFlag::SharedWithMe.is_enabled() || access_level.can_trash())
        {
            menu_items.push(
                MenuItemFields::new("Trash")
                    .with_on_select_action(WorkflowAction::Trash)
                    .with_icon(Icon::Trash)
                    .into_item(),
            );
        }

        menu_items
    }

    fn close(&mut self, ctx: &mut ViewContext<Self>) {
        if self.show_enum_creation_dialog {
            return;
        }

        if self.should_show_unsaved_changes_dialog(ctx) {
            self.show_unsaved_changes_dialog(UnsavedChangeType::ForClose, ctx)
        } else {
            self.emit_close_event(ctx);
        }
    }

    fn focus_contents(&mut self, ctx: &mut ViewContext<Self>) {
        self.focus(ctx);
    }

    fn render_header_content(
        &self,
        _ctx: &view::HeaderRenderContext<'_>,
        app: &AppContext,
    ) -> view::HeaderContent {
        let mut content =
            view::HeaderContent::simple(self.pane_configuration().as_ref(app).title());
        if let view::HeaderContent::Standard(header) = &mut content {
            header.options.always_show_icons = true;
        }
        content
    }

    fn set_focus_handle(&mut self, focus_handle: PaneFocusHandle, _ctx: &mut ViewContext<Self>) {
        self.focus_handle = Some(focus_handle);
    }
}
