use pathfinder_geometry::vector::{vec2f, Vector2F};

use warp_core::features::FeatureFlag;
use warpui::{
    clipboard::ClipboardContent,
    elements::{
        Align, AnchorPair, ChildAnchor, Clipped, ClippedScrollStateHandle, ClippedScrollable,
        ConstrainedBox, Container, CrossAxisAlignment, DispatchEventResult, EventHandler, Fill,
        Flex, MainAxisAlignment, MainAxisSize, MouseStateHandle, OffsetPositioning, OffsetType,
        ParentAnchor, ParentElement, ParentOffsetBounds, PositioningAxis, SavePosition,
        ScrollbarWidth, Shrinkable, Stack, XAxisAnchor, YAxisAnchor,
    },
    id,
    keymap::EditableBinding,
    platform::Cursor,
    presenter::ChildView,
    ui_components::components::UiComponent,
    AppContext, BlurContext, Element, Entity, FocusContext, ModelAsRef, ModelHandle,
    SingletonEntity, TypedActionView, View, ViewContext, ViewHandle, WindowId,
};

use crate::{
    ai::blocklist::block::secret_redaction::find_secrets_in_text_with_levels,
    cloud_object::{
        breadcrumbs::ContainingObject,
        model::persistence::{CloudModel, CloudModelEvent},
        CloudObjectEventEntrypoint, Owner,
    },
    drive::{
        items::WarpDriveItemId,
        sharing::{ContentEditability, ShareableObject},
    },
    editor::EditorView,
    env_vars::{
        active_env_var_collection_data::{
            ActiveEnvVarCollection, ActiveEnvVarCollectionData, ActiveEnvVarCollectionDataEvent,
            SavingStatus, TrashStatus,
        },
        CloudEnvVarCollection, CloudEnvVarCollectionModel, EnvVar, EnvVarCollection,
        EnvVarCollectionType, EnvVarValue,
    },
    external_secrets::SecretManager,
    menu::MenuItem,
    network::{NetworkStatus, NetworkStatusEvent},
    pane_group::{
        focus_state::PaneFocusHandle, pane::view, BackingView, PaneConfiguration, PaneEvent,
    },
    search::external_secrets::view::ExternalSecretsMenu,
    send_telemetry_from_ctx,
    server::{
        cloud_objects::update_manager::{FetchSingleObjectOption, UpdateManager},
        ids::{ServerId, SyncId},
    },
    terminal::{model::secrets::SecretLevel, safe_mode_settings::get_secret_obfuscation_mode},
    ui_components::{
        breadcrumb::{render_breadcrumbs, BreadcrumbState},
        buttons::icon_button,
        icons::Icon,
        menu_button::{
            highlight_icon_button_with_context_menu, icon_button_with_context_menu, MenuDirection,
        },
    },
    util::bindings::CustomAction,
    view_components::{alert::AlertConfig, Alert, DismissibleToast, ToastType},
    workspace::ToastStack,
    Appearance, CloudObjectTypeAndId, TelemetryEvent,
};

use super::{command_dialog::EnvVarCommandDialog, menus::Menus};

// Universal
pub(super) const CORE_HORIZONATAL_MARGIN: f32 = 24.;
pub(super) const CORE_MAX_WIDTH: f32 = 800.;
pub(super) const DESCRIPTION_EDITOR_POSITION: &str = "envvar_description_editor";

// View
const SCROLLBAR_WIDTH: ScrollbarWidth = ScrollbarWidth::Auto;
const CORE_VERTICAL_MARGIN: f32 = 36.;
const SECTION_SPACING: f32 = 16.;

// Variable rows
pub(super) const ROW_SPACING: f32 = 8.;
pub const EDUCATION_TEXT: &str = "Add secret or command. Warp never stores external secrets";
const VARIABLE_FONT_SIZE: f32 = 13.;
const DESCRIPTION_EDITOR_CUTOFF: f32 = 30.;
const DESCRIPTION_BOTTOM_MARGIN: f32 = 12.;
const DIVIDER_BOTTOM_MARGIN: f32 = 4.;
const PLACEHOLDER_FONT_SIZE: f32 = 14.;
const VARIABLE_VALUE_PLACEHOLDER_TEXT: &str = "Value";
const VARIABLE_DESCRIPTION_PLACEHOLDER_TEXT: &str = "Description";
const VARIABLE_NAME_PLACEHOLDER_TEXT: &str = "Variable";

// Text input fields
const TITLE_PLACEHOLDER_TEXT: &str = "Add a title";
const DESCRIPTION_PLACEHOLDER_TEXT: &str = "Add a description";

// Button spacing
const BUTTON_CONTAINER_HORIZONTAL_MARGIN: f32 = 36.;
const BUTTON_CONTAINER_BOTTOM_MARGIN: f32 = 10.;
const BUTTON_SPACING: f32 = 8.;

// Validation error styling
pub(super) const ERROR_BORDER_WIDTH: f32 = 1.;
pub(super) const ERROR_ALERT_MARGIN_TOP: f32 = 8.;

pub fn init(app: &mut AppContext) {
    app.register_editable_bindings([EditableBinding::new(
        "Close Env Var Collection",
        "Close",
        EnvVarCollectionAction::Close,
    )
    .with_custom_action(CustomAction::CloseCurrentSession)
    .with_context_predicate(id!(EnvVarCollectionView::ui_name()))]);
}

#[derive(PartialEq, Clone, Copy)]
pub(super) enum EditorType {
    Name,
    Value,
    Description,
}

/// Validation error for a specific field containing secrets
#[derive(Debug, Clone, PartialEq)]
pub(super) struct ValidationError {
    /// The highest priority secret level detected in this field
    pub(super) secret_level: SecretLevel,
    /// User-friendly error message
    pub(super) message: String,
}

/// Validation state for a single environment variable row
#[derive(Debug, Clone, Default)]
pub(super) struct RowValidationState {
    /// Validation error for the variable name field
    pub(super) name_error: Option<ValidationError>,
    /// Validation error for the variable value field
    pub(super) value_error: Option<ValidationError>,
    /// Validation error for the variable description field
    pub(super) description_error: Option<ValidationError>,
}

/// Validation state for the entire form, including metadata fields
#[derive(Debug, Clone, Default)]
pub(super) struct FormValidationState {
    /// Validation error for the form title field
    pub(super) title_error: Option<ValidationError>,
    /// Validation error for the form description field
    pub(super) description_error: Option<ValidationError>,
}

impl RowValidationState {
    /// Returns true if this row has any validation errors
    pub(super) fn has_errors(&self) -> bool {
        self.name_error.is_some() || self.value_error.is_some() || self.description_error.is_some()
    }

    /// Sets validation error for the specified field
    pub(super) fn set_field_error(&mut self, field: EditorType, error: Option<ValidationError>) {
        match field {
            EditorType::Name => self.name_error = error,
            EditorType::Value => self.value_error = error,
            EditorType::Description => self.description_error = error,
        }
    }

    /// Gets validation error for the specified field
    pub(super) fn get_field_error(&self, field: EditorType) -> Option<&ValidationError> {
        match field {
            EditorType::Name => self.name_error.as_ref(),
            EditorType::Value => self.value_error.as_ref(),
            EditorType::Description => self.description_error.as_ref(),
        }
    }

    /// Gets the highest severity error in this row
    pub(super) fn get_highest_severity_error(&self) -> Option<&ValidationError> {
        [&self.name_error, &self.value_error, &self.description_error]
            .iter()
            .filter_map(|error| error.as_ref())
            .max_by_key(|error| error.secret_level.priority())
    }
}

impl FormValidationState {
    /// Returns true if the form has any validation errors
    pub(super) fn has_errors(&self) -> bool {
        self.title_error.is_some() || self.description_error.is_some()
    }

    /// Sets validation error for the title field
    pub(super) fn set_title_error(&mut self, error: Option<ValidationError>) {
        self.title_error = error;
    }

    /// Sets validation error for the description field
    pub(super) fn set_description_error(&mut self, error: Option<ValidationError>) {
        self.description_error = error;
    }

    /// Gets the highest severity error in the form (including both metadata and variable rows)
    pub(super) fn get_highest_severity_error(&self) -> Option<&ValidationError> {
        [&self.title_error, &self.description_error]
            .iter()
            .filter_map(|error| error.as_ref())
            .max_by_key(|error| error.secret_level.priority())
    }
}

#[derive(Default)]
pub(super) struct MouseStateHandles {
    // Linked to the plus button on the "Variables" section header
    pub(super) add_variable_state: MouseStateHandle,
    // Linked to the save button on the footer
    pub(super) save_mouse_state: MouseStateHandle,
    // Linked to the invoke button on the footer
    pub(super) invoke_mouse_state: MouseStateHandle,
    // Linked to an action which restores the item,
    // displayed in the "trash banner"
    pub(super) restore_from_trash_button: MouseStateHandle,
    // Both of the below are used in unsaved changes dialog
    pub(super) discard_changes_state: MouseStateHandle,
    pub(super) keep_editing_state: MouseStateHandle,
    pub(super) secret_tooltip_state: MouseStateHandle,
}

pub(super) struct VariableEditorRow {
    // The value field keeps track of the EnvVarValue this row holds.
    // Note that if the underlying value is a constant, the source
    // of truth is the buffer content of the variable_value_editor
    // (this "value" state is just used to represent the underlying
    // value is a constant)
    pub(super) value: EnvVarValue,
    // Editors for the current row
    pub(super) variable_name_editor: ViewHandle<EditorView>,
    pub(super) variable_value_editor: ViewHandle<EditorView>,
    pub(super) variable_description_editor: ViewHandle<EditorView>,
    // Linked to the circle with a line icon button which deletes the row
    delete_row_mouse_state_handle: MouseStateHandle,
    // This is linked to the "key" icon displayed when an editor
    // has no text in it
    secret_button_mouse_state: MouseStateHandle,
    // "Rendered" buttons are used for "finished" secrets, these are buttons
    // since we can click to open a menu which lets us change/remove them
    rendered_secret_button_mouse_state: MouseStateHandle,
    rendered_command_button_mouse_state: MouseStateHandle,
    // If true, indicates these indicate we should render the respective
    // menu at this row
    pub(super) secret_menu_is_focused: bool,
    pub(super) rendered_secret_menu_is_focused: bool,
    pub(super) rendered_command_menu_is_focused: bool,
    // Validation state for secret detection
    pub(super) validation_state: RowValidationState,
}
pub(super) struct DialogOpenStates {
    pub(super) secrets_dialog_open: bool,
    pub(super) env_var_command_dialog_open: bool,
    unsaved_changes_dialog_open: bool,
}

impl DialogOpenStates {
    fn has_open_dialog(&self) -> bool {
        self.secrets_dialog_open
            || self.env_var_command_dialog_open
            || self.unsaved_changes_dialog_open
    }
}

/// EnvVarCollectionView is the view backing our "Environment Variables"
/// feature.
pub struct EnvVarCollectionView {
    focused: bool,
    // Scroll state for the view-level scrollbar
    variables_clipped_scroll_state: ClippedScrollStateHandle,
    pub(super) pane_configuration: ModelHandle<PaneConfiguration>,
    pub(super) focus_handle: Option<PaneFocusHandle>,
    // Contains data pertaining to the currently open EnvVarCollection,
    // such as as the current revision and saving status
    pub(super) active_env_var_collection_data: ModelHandle<ActiveEnvVarCollectionData>,
    // State handles for buttons in the view
    pub(super) button_mouse_states: MouseStateHandles,
    // Editor view for the EnvVarCollection's title and description editor,
    // the two in conjunction often referred to as "metadata"
    pub(super) title_editor: ViewHandle<EditorView>,
    pub(super) description_editor: ViewHandle<EditorView>,
    // Contains the state needed to manage variable rows, such as editors,
    // the currently stored value, and row-level mouse state handles
    pub(super) variable_rows: Vec<VariableEditorRow>,
    // The pending variable row index is set when a user hits
    // a button linked to a variable row, such as the "secrets" button
    // (the key) or a rendered secret/command button. Once a menu item
    // is selected, we set this to Some(VariableRowIndex())
    pub(super) pending_variable_row_index: Option<VariableRowIndex>,
    pub(super) breadcrumbs: Vec<BreadcrumbState<ContainingObject>>,
    // State vars used to manage menus; pane_context_menu_offset holds
    // the offset from the parent (i.e. origin of the element saved to
    // the below view_position_id variable) on a user's right click
    pub(super) menus: Menus,
    pub(super) pane_context_menu_offset: Option<Vector2F>,
    // State vars used to dialogs, and indicate if they're
    // open or not
    pub(super) secrets_dialog: ViewHandle<ExternalSecretsMenu>,
    pub(super) env_var_command_dialog: ViewHandle<EnvVarCommandDialog>,
    pub(super) dialog_open_states: DialogOpenStates,
    // Save position id for the bounds of this view, used to calculate
    // offset for right clicks/displaying the pane context mneu
    pub(super) view_position_id: String,
    // Validation state for the entire form
    pub(super) form_validation_state: FormValidationState,
    // Alert used to display validation errors
    validation_alert: Alert,
}

#[derive(Debug, Clone, PartialEq)]
pub enum EnvVarCollectionEvent {
    Pane(PaneEvent),
    UpdatedEnvVarCollection(SyncId),
    ViewInWarpDrive(WarpDriveItemId),
    Invoke(EnvVarCollectionType),
}
#[derive(Debug, Clone)]
pub struct VariableRowIndex(pub usize);

#[derive(Debug, Clone)]
pub enum EnvVarCollectionAction {
    // Core actions
    SaveVariables,
    Invoke,
    Close,
    AddVariable,
    DeleteVariable(VariableRowIndex),
    // Overflow menu actions
    Untrash,
    CopyLink(String),
    Duplicate,
    Trash,
    Export,
    // Secret button related actions
    SelectSecretManager(SecretManager),
    DisplayCommandDialog,
    // Rendered secret button related actions
    ClearSecret,
    EditCommand,
    // Menu-related actions
    DisplaySecretMenu(VariableRowIndex),
    DisplayPaneMenu(Vector2F),
    DisplayRenderedSecretMenu(VariableRowIndex),
    DisplayRenderedCommandMenu(VariableRowIndex),
    EmitPaneEvent(PaneEvent),
    // Unsaved changes dialog actions
    ForceClose,
    CloseUnsavedChangesDialog,
    // Breadcrumbs action
    ViewInWarpDrive(WarpDriveItemId),
}

/// Defines the view for a collection of environment variables
impl ValidationError {
    /// Create validation error from detected secret level
    fn from_secret_level(secret_level: SecretLevel) -> Self {
        let message = match secret_level {
            SecretLevel::Enterprise => "This environment variable cannot be created due to conflicts with your enterprise's secret redaction settings. Contact a team admin for details.".to_string(),
            SecretLevel::User => "This environment variable cannot be created due to conflicts with your secret redaction settings. Save the secret as an environment variable (in your shell config or a .env file), or update your secret redaction settings in Settings > Privacy.".to_string(),
        };
        Self {
            secret_level,
            message,
        }
    }
}

impl EnvVarCollectionView {
    /// Validates field content for secrets and returns validation error if found
    fn validate_field_content(text: &str) -> Option<ValidationError> {
        let detected_secrets = find_secrets_in_text_with_levels(text);
        if detected_secrets.is_empty() {
            return None;
        }

        // Find the highest priority secret level
        detected_secrets
            .iter()
            .map(|(_, level)| *level)
            .max_by_key(|level| level.priority())
            .map(|highest_priority_level| {
                ValidationError::from_secret_level(highest_priority_level)
            })
    }

    /// Updates validation state for a specific field in a row
    pub(super) fn update_field_validation(
        &mut self,
        row_index: usize,
        field_type: EditorType,
        text: &str,
        ctx: &mut ViewContext<Self>,
    ) {
        if row_index >= self.variable_rows.len() {
            return;
        }

        let should_validate = get_secret_obfuscation_mode(ctx).should_redact_secret();

        let validation_error = if should_validate {
            Self::validate_field_content(text)
        } else {
            None
        };

        self.variable_rows[row_index]
            .validation_state
            .set_field_error(field_type, validation_error);
        ctx.notify();
    }

    /// Updates validation state for the title field
    pub(super) fn update_title_validation(&mut self, text: &str, ctx: &mut ViewContext<Self>) {
        let should_validate = get_secret_obfuscation_mode(ctx).should_redact_secret();

        let validation_error = if should_validate {
            Self::validate_field_content(text)
        } else {
            None
        };

        self.form_validation_state.set_title_error(validation_error);
        ctx.notify();
    }

    /// Updates validation state for the description field
    pub(super) fn update_description_validation(
        &mut self,
        text: &str,
        ctx: &mut ViewContext<Self>,
    ) {
        let should_validate = get_secret_obfuscation_mode(ctx).should_redact_secret();

        let validation_error = if should_validate {
            Self::validate_field_content(text)
        } else {
            None
        };

        self.form_validation_state
            .set_description_error(validation_error);
        ctx.notify();
    }

    /// Finds the row index and editor type for a given editor handle
    pub(super) fn find_editor_info(
        &self,
        editor_handle: &ViewHandle<EditorView>,
    ) -> Option<(usize, EditorType)> {
        for (row_index, row) in self.variable_rows.iter().enumerate() {
            if &row.variable_name_editor == editor_handle {
                return Some((row_index, EditorType::Name));
            }
            if &row.variable_value_editor == editor_handle {
                return Some((row_index, EditorType::Value));
            }
            if &row.variable_description_editor == editor_handle {
                return Some((row_index, EditorType::Description));
            }
        }
        None
    }

    /// Renders a validation error alert based on the Figma design
    pub(super) fn render_validation_error(
        &self,
        error: &ValidationError,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        Container::new(self.validation_alert.render(
            AlertConfig::error(error.message.clone()).with_main_axis_size(MainAxisSize::Max),
            appearance,
        ))
        .with_margin_top(ERROR_ALERT_MARGIN_TOP)
        .finish()
    }

    /// Renders the bottom error message showing the highest severity error across the form
    pub(super) fn render_bottom_error_message(
        &self,
        appearance: &Appearance,
    ) -> Option<Box<dyn Element>> {
        self.get_highest_severity_form_error()
            .map(|error| self.render_validation_error(error, appearance))
    }

    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let appearance = Appearance::as_ref(ctx);
        let ui_font_family = appearance.ui_font_family();

        let cloud_model = CloudModel::handle(ctx);
        ctx.subscribe_to_model(&cloud_model, |view, _handle, event, ctx| {
            view.handle_cloud_model_event(event, ctx);
        });

        let pane_configuration = ctx.add_model(|_ctx| PaneConfiguration::new("Untitled"));

        let active_env_var_collection_data = ctx.add_model(ActiveEnvVarCollectionData::new);
        ctx.subscribe_to_model(
            &active_env_var_collection_data,
            Self::handle_active_env_var_collection_event,
        );
        ctx.observe(
            &active_env_var_collection_data,
            Self::handle_active_env_var_collection_change,
        );

        ctx.subscribe_to_model(
            &NetworkStatus::handle(ctx),
            Self::handle_network_status_event,
        );

        let title_editor = Self::create_editor_handle(
            ctx,
            Some(PLACEHOLDER_FONT_SIZE),
            Some(ui_font_family),
            Some(TITLE_PLACEHOLDER_TEXT),
            true,
        );
        let description_editor = Self::create_editor_handle(
            ctx,
            Some(PLACEHOLDER_FONT_SIZE),
            Some(ui_font_family),
            Some(DESCRIPTION_PLACEHOLDER_TEXT),
            false,
        );
        ctx.subscribe_to_view(&title_editor, |me, _, event, ctx| {
            me.handle_title_editor_event(event, ctx);
        });
        ctx.subscribe_to_view(&description_editor, |me, _, event, ctx| {
            me.handle_description_editor_event(event, ctx);
        });

        let secrets_dialog = ctx.add_typed_action_view(ExternalSecretsMenu::new);
        let env_var_command_dialog = ctx.add_typed_action_view(EnvVarCommandDialog::new);
        ctx.subscribe_to_view(&secrets_dialog, |me, _, event, ctx| {
            me.handle_external_secrets_dialog_event(event, ctx);
        });
        ctx.subscribe_to_view(&env_var_command_dialog, |me, _, event, ctx| {
            me.handle_command_dialog_event(event, ctx)
        });

        let menus = Self::initialize_menus(ctx);

        let dialog_open_states = DialogOpenStates {
            secrets_dialog_open: false,
            env_var_command_dialog_open: false,
            unsaved_changes_dialog_open: false,
        };

        let view_position_id = format!("env_var_collection_view_{}", ctx.view_id());

        EnvVarCollectionView {
            focused: false,
            pane_configuration,
            focus_handle: None,
            active_env_var_collection_data,
            button_mouse_states: Default::default(),
            variables_clipped_scroll_state: Default::default(),
            title_editor,
            description_editor,
            variable_rows: Vec::new(),
            breadcrumbs: Vec::new(),
            menus,
            pending_variable_row_index: None,
            pane_context_menu_offset: None,
            secrets_dialog,
            env_var_command_dialog,
            dialog_open_states,
            view_position_id,
            form_validation_state: Default::default(),
            validation_alert: Alert::basic(),
        }
    }

    pub fn focus(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.focus(&self.title_editor);
        ctx.emit(EnvVarCollectionEvent::Pane(PaneEvent::FocusSelf));
    }

    pub fn open_new_env_var_collection(
        &mut self,
        owner: Owner,
        initial_folder_id: Option<SyncId>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.active_env_var_collection_data
            .update(ctx, |data, ctx| {
                data.open_new(owner, initial_folder_id, ctx);
            });
        self.add_variable_row(ctx);
        // Setting this to indicate the collection isn't unsaved, and thus
        // the pane can be closed without the unsaved_command_dialog
        self.set_saving_status(SavingStatus::New, ctx);
    }

    pub fn wait_for_initial_load_then_load(
        &mut self,
        env_var_collection_id: SyncId,
        window_id: WindowId,
        ctx: &mut ViewContext<Self>,
    ) {
        let initial_load_complete = UpdateManager::handle(ctx).update(ctx, |update_manager, _| {
            update_manager.initial_load_complete()
        });
        ctx.spawn(initial_load_complete, move |me, _, ctx| {
            let env_var_collection = CloudModel::as_ref(ctx)
                .get_env_var_collection(&env_var_collection_id)
                .cloned();
            if let Some(env_var_collection) = env_var_collection {
                me.load(env_var_collection, ctx);
            } else if let Some(server_id) = env_var_collection_id.into_server() {
                me.fetch_and_load_env_var_collection(server_id, window_id, ctx);
            } else {
                ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                    toast_stack.add_ephemeral_toast_by_type(
                        ToastType::CloudObjectNotFound,
                        window_id,
                        ctx,
                    );
                });
                log::warn!("Tried to open unknown env var collection {env_var_collection_id:?}");
            }
        });
    }

    fn fetch_and_load_env_var_collection(
        &mut self,
        env_var_collection_id: ServerId,
        window_id: WindowId,
        ctx: &mut ViewContext<Self>,
    ) {
        let fetch_cloud_object_rx =
            UpdateManager::handle(ctx).update(ctx, |update_manager, ctx| {
                update_manager.fetch_single_cloud_object(
                    &env_var_collection_id,
                    FetchSingleObjectOption::None,
                    ctx,
                )
            });
        ctx.spawn(fetch_cloud_object_rx, move |me, _, ctx| {
            if let Some(env_var_collection) = CloudModel::as_ref(ctx)
                .get_env_var_collection(&SyncId::ServerId(env_var_collection_id))
                .cloned()
            {
                me.load(env_var_collection, ctx);
            } else {
                ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                    toast_stack.add_ephemeral_toast_by_type(ToastType::CloudObjectNotFound, window_id, ctx);
                });
                log::warn!("Tried to open unknown env var collection {env_var_collection_id:?} after fetching");
            }
        });
    }

    pub fn load(&mut self, env_var_collection: CloudEnvVarCollection, ctx: &mut ViewContext<Self>) {
        self.active_env_var_collection_data
            .update(ctx, |data, ctx| {
                data.open_existing(env_var_collection.id, ctx);
                data.revision_ts
                    .clone_from(&env_var_collection.metadata.revision);
            });

        let collection = &env_var_collection.model().string_model;

        let title = collection.title.clone().unwrap_or_default();

        self.set_pane_title(if title.is_empty() { "Untitled" } else { &title }, ctx);
        if let Some(server_id) = env_var_collection.id.into_server() {
            self.pane_configuration.update(ctx, |pane_config, ctx| {
                pane_config
                    .set_shareable_object(Some(ShareableObject::WarpDriveObject(server_id)), ctx);
            });
        }

        let description = collection.description.clone().unwrap_or_default();

        self.title_editor.update(ctx, |editor, ctx| {
            editor.system_reset_buffer_text(&title, ctx)
        });

        self.description_editor.update(ctx, |editor, ctx| {
            editor.system_reset_buffer_text(&description, ctx)
        });

        self.update_title_validation(&title, ctx);
        self.update_description_validation(&description, ctx);

        self.variable_rows = Vec::new();

        collection.vars.iter().enumerate().for_each(|(index, var)| {
            self.add_variable_row(ctx);
            self.variable_rows[index]
                .variable_name_editor
                .update(ctx, |editor, ctx| {
                    editor.system_reset_buffer_text(var.name.as_str(), ctx);
                });
            self.variable_rows[index]
                .variable_value_editor
                .update(ctx, |editor, ctx| {
                    editor.system_reset_buffer_text(
                        if let EnvVarValue::Constant(val) = &var.value {
                            val
                        } else {
                            ""
                        },
                        ctx,
                    );
                });
            self.variable_rows[index].value = var.value.clone();
            if let Some(description) = &var.description {
                self.variable_rows[index]
                    .variable_description_editor
                    .update(ctx, |editor, ctx| {
                        editor.system_reset_buffer_text(description.as_str(), ctx);
                    });
            }

            self.update_field_validation(index, EditorType::Name, &var.name, ctx);
            if let EnvVarValue::Constant(val) = &var.value {
                self.update_field_validation(index, EditorType::Value, val, ctx);
            }
            if let Some(description) = &var.description {
                self.update_field_validation(index, EditorType::Description, description, ctx);
            }
        });

        self.set_saving_status(SavingStatus::Saved, ctx);
        self.update_editor_interactivity(ctx);
        ctx.notify();
    }

    fn invoke_env_var_collection(&self, ctx: &mut ViewContext<Self>) {
        match &self
            .active_env_var_collection_data
            .as_ref(ctx)
            .active_env_var_collection
        {
            ActiveEnvVarCollection::CommittedEnvVarCollection(id) => {
                let cloud_model = CloudModel::as_ref(ctx);
                if let Some(cloud_env_var) = cloud_model.get_env_var_collection(id) {
                    ctx.emit(EnvVarCollectionEvent::Invoke(EnvVarCollectionType::Cloud(
                        Box::new(cloud_env_var.clone()),
                    )));
                } else {
                    log::error!("Env var not found and could not be invoked");
                    let window_id = ctx.window_id();
                    crate::workspace::ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                        toast_stack.add_ephemeral_toast(
                            DismissibleToast::error(
                                "An error occurred while trying to invoke the env var".to_owned(),
                            ),
                            window_id,
                            ctx,
                        );
                    });
                }
            }
            ActiveEnvVarCollection::NewEnvVarCollection(env_var_collection) => {
                ctx.emit(EnvVarCollectionEvent::Invoke(EnvVarCollectionType::Cloud(
                    Box::new(env_var_collection.as_ref().clone()),
                )))
            }
            ActiveEnvVarCollection::None => log::warn!("No env var to invoke"),
        }
    }

    fn save_env_var_collection(&self, ctx: &mut ViewContext<Self>) {
        if self.should_disable_save(ctx) {
            return;
        }

        let title = self.title_editor.as_ref(ctx).buffer_text(ctx);
        self.set_pane_title(&title, ctx);

        let title = if title.is_empty() { None } else { Some(title) };

        let description = self.description_editor.as_ref(ctx).buffer_text(ctx);
        let description = if description.is_empty() {
            None
        } else {
            Some(description)
        };

        let vars: Vec<EnvVar> = self
            .variable_rows
            .iter()
            .map(|variable_editor_row| {
                let name = variable_editor_row
                    .variable_name_editor
                    .as_ref(ctx)
                    .buffer_text(ctx);
                let value = if let EnvVarValue::Constant(_) = &variable_editor_row.value {
                    EnvVarValue::Constant(
                        variable_editor_row
                            .variable_value_editor
                            .as_ref(ctx)
                            .buffer_text(ctx),
                    )
                } else {
                    variable_editor_row.value.clone()
                };

                let var_description = variable_editor_row
                    .variable_description_editor
                    .as_ref(ctx)
                    .buffer_text(ctx);
                let var_description = if var_description.is_empty() {
                    None
                } else {
                    Some(var_description)
                };

                EnvVar {
                    name,
                    value,
                    description: var_description,
                }
            })
            .collect();

        // Validation errors should prevent saving - this is now handled by should_disable_save()
        // The UI will show inline validation errors instead of toast messages

        let new_env_var_collection = EnvVarCollection::new(title, description, vars);

        let active_env_var_collection = self
            .active_env_var_collection_data
            .as_ref(ctx)
            .active_env_var_collection();

        match active_env_var_collection {
            // If the EVC has already been committed, then update the local
            // memory and server data via update manager
            ActiveEnvVarCollection::CommittedEnvVarCollection(id) => UpdateManager::handle(ctx)
                .update(ctx, |update_manager, ctx| {
                    update_manager.update_env_var_collection(
                        new_env_var_collection,
                        id,
                        self.active_env_var_collection_data
                            .update(ctx, |data, _| data.revision_ts.clone()),
                        ctx,
                    );
                }),
            // If the EVC hasn't been committed yet, create the EVC through update
            // manager, and update the active EVC
            ActiveEnvVarCollection::NewEnvVarCollection(env_var_collection) => {
                if let Some(client_id) = env_var_collection.id.into_client() {
                    UpdateManager::handle(ctx).update(ctx, |update_manager, ctx| {
                        update_manager.create_env_var_collection(
                            client_id,
                            env_var_collection.permissions.owner,
                            env_var_collection.metadata.folder_id,
                            CloudEnvVarCollectionModel::new(new_env_var_collection),
                            CloudObjectEventEntrypoint::Unknown,
                            true,
                            ctx,
                        );
                    });

                    self.active_env_var_collection_data.update(ctx, |data, _| {
                        data.active_env_var_collection =
                            ActiveEnvVarCollection::CommittedEnvVarCollection(SyncId::ClientId(
                                client_id,
                            ))
                    });
                }
            }
            ActiveEnvVarCollection::None => {
                log::error!("Tried to save EVC, but none were active")
            }
        }
    }

    pub(super) fn add_variable_row(&mut self, ctx: &mut ViewContext<Self>) {
        let appearance = Appearance::as_ref(ctx);
        let ui_font_family = appearance.ui_font_family();

        let variable_name_editor = Self::create_editor_handle(
            ctx,
            Some(VARIABLE_FONT_SIZE),
            Some(ui_font_family),
            Some(VARIABLE_NAME_PLACEHOLDER_TEXT),
            true,
        );

        ctx.subscribe_to_view(&variable_name_editor, |me, emitter, event, ctx| {
            me.handle_variable_event(emitter, event, ctx);
        });

        let variable_value_editor = Self::create_editor_handle(
            ctx,
            Some(VARIABLE_FONT_SIZE),
            Some(ui_font_family),
            Some(VARIABLE_VALUE_PLACEHOLDER_TEXT),
            true,
        );

        ctx.subscribe_to_view(&variable_value_editor, |me, emitter, event, ctx| {
            me.handle_variable_event(emitter, event, ctx);
        });

        let variable_description_editor = Self::create_editor_handle(
            ctx,
            Some(VARIABLE_FONT_SIZE),
            Some(ui_font_family),
            Some(VARIABLE_DESCRIPTION_PLACEHOLDER_TEXT),
            true,
        );

        ctx.subscribe_to_view(&variable_description_editor, |me, emitter, event, ctx| {
            me.handle_variable_event(emitter, event, ctx);
        });

        self.variable_rows.push(VariableEditorRow {
            variable_name_editor,
            variable_value_editor,
            variable_description_editor,
            delete_row_mouse_state_handle: Default::default(),
            rendered_secret_button_mouse_state: Default::default(),
            secret_button_mouse_state: Default::default(),
            secret_menu_is_focused: false,
            rendered_secret_menu_is_focused: false,
            value: EnvVarValue::Constant(String::new()),
            rendered_command_button_mouse_state: Default::default(),
            rendered_command_menu_is_focused: false,
            validation_state: Default::default(),
        });

        self.set_saving_status(SavingStatus::Unsaved, ctx);
        ctx.notify();
    }

    pub(super) fn delete_row(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        self.variable_rows.remove(index);
        self.set_saving_status(SavingStatus::Unsaved, ctx);

        ctx.notify();
    }

    fn handle_active_env_var_collection_change(
        &mut self,
        _handle: ModelHandle<ActiveEnvVarCollectionData>,
        ctx: &mut ViewContext<Self>,
    ) {
        // Refresh the overflow menu to show actions that only apply to synced EVCs.
        self.pane_configuration.update(ctx, |pane_config, ctx| {
            pane_config.refresh_pane_header_overflow_menu_items(ctx)
        });
        ctx.notify();
    }

    fn handle_active_env_var_collection_event(
        &mut self,
        _handle: ModelHandle<ActiveEnvVarCollectionData>,
        event: &ActiveEnvVarCollectionDataEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            ActiveEnvVarCollectionDataEvent::BreadcrumbsChanged => {
                self.update_breadcrumbs(ctx);
                ctx.notify()
            }
            ActiveEnvVarCollectionDataEvent::CreatedOnServer(server_id) => {
                self.update_breadcrumbs(ctx);
                self.pane_configuration.update(ctx, |pane_config, ctx| {
                    pane_config.set_shareable_object(
                        Some(ShareableObject::WarpDriveObject(*server_id)),
                        ctx,
                    );
                });
            }
            ActiveEnvVarCollectionDataEvent::TrashStatusChanged => {
                self.pane_configuration.update(ctx, |pane_config, ctx| {
                    pane_config.refresh_pane_header_overflow_menu_items(ctx)
                });
            }
        }
    }

    fn handle_cloud_model_event(&mut self, event: &CloudModelEvent, ctx: &mut ViewContext<Self>) {
        match event {
            CloudModelEvent::ObjectCreated { type_and_id, .. } => {
                if self
                    .as_active_env_var_collection_id(type_and_id, ctx)
                    .is_some()
                {
                    ctx.notify();
                }
            }
            CloudModelEvent::ObjectTrashed { .. }
            | CloudModelEvent::ObjectDeleted { .. }
            | CloudModelEvent::ObjectUntrashed { .. }
            | CloudModelEvent::ObjectMoved { .. } => ctx.notify(),
            CloudModelEvent::ObjectPermissionsUpdated { type_and_id, .. }
                if self
                    .as_active_env_var_collection_id(type_and_id, ctx)
                    .is_some() =>
            {
                self.update_editor_interactivity(ctx);
            }
            _ => (),
        }
    }

    // Only enable the invoke/load button if the env var is committed and the current version is saved
    pub(super) fn should_disable_invoke(&self, app: &AppContext) -> bool {
        let active_env_var_collection_data = self.active_env_var_collection_data.as_ref(app);
        let data = (
            active_env_var_collection_data.active_env_var_collection(),
            active_env_var_collection_data.saving_status == SavingStatus::Saved,
        );

        if let (ActiveEnvVarCollection::CommittedEnvVarCollection(_), true) = data {
            return false;
        }
        true
    }

    pub(super) fn update_open_modal_state(&self, ctx: &mut ViewContext<Self>) {
        self.pane_configuration
            .update(ctx, |pane_configuration, ctx| {
                pane_configuration
                    .set_has_open_modal(self.dialog_open_states.has_open_dialog(), ctx);
            });
    }

    pub(super) fn should_disable_save(&self, app: &AppContext) -> bool {
        self.editors_are_empty(app) || self.variable_rows.is_empty() || self.has_validation_errors()
    }

    /// Checks if any fields have validation errors (including metadata and variable rows)
    fn has_validation_errors(&self) -> bool {
        self.form_validation_state.has_errors()
            || self
                .variable_rows
                .iter()
                .any(|row| row.validation_state.has_errors())
    }

    /// Gets the highest severity error across the entire form
    fn get_highest_severity_form_error(&self) -> Option<&ValidationError> {
        let form_error = self.form_validation_state.get_highest_severity_error();
        let row_errors = self
            .variable_rows
            .iter()
            .filter_map(|row| row.validation_state.get_highest_severity_error());

        std::iter::once(form_error)
            .flatten()
            .chain(row_errors)
            .max_by_key(|error| error.secret_level.priority())
    }

    pub(super) fn is_online(&self, app: &AppContext) -> bool {
        NetworkStatus::as_ref(app).is_online()
    }

    fn handle_network_status_event(
        &mut self,
        _handle: ModelHandle<NetworkStatus>,
        event: &NetworkStatusEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        let NetworkStatusEvent::NetworkStatusChanged { new_status: _ } = event;
        self.pane_configuration.update(ctx, |pane_config, ctx| {
            pane_config.refresh_pane_header_overflow_menu_items(ctx)
        });
    }

    pub fn set_saving_status(&mut self, status: SavingStatus, ctx: &mut ViewContext<Self>) {
        self.active_env_var_collection_data
            .update(ctx, |data, _| data.saving_status = status);
    }

    pub fn pane_configuration(&self) -> &ModelHandle<PaneConfiguration> {
        &self.pane_configuration
    }

    pub fn env_var_collection_id<C: ModelAsRef>(&self, ctx: &C) -> Option<SyncId> {
        self.active_env_var_collection_data.as_ref(ctx).id()
    }

    fn as_active_env_var_collection_id(
        &self,
        id: &CloudObjectTypeAndId,
        ctx: &mut ViewContext<Self>,
    ) -> Option<SyncId> {
        id.as_generic_string_object_id().filter(|id| {
            self.active_env_var_collection_data
                .as_ref(ctx)
                .is_active_env_var_collection(*id)
        })
    }

    fn set_pane_title(&self, title: &str, ctx: &mut ViewContext<Self>) {
        self.pane_configuration
            .update(ctx, |pane_configuration, ctx| {
                pane_configuration.set_title(title, ctx)
            });
    }

    fn view_in_warp_drive(&mut self, id: WarpDriveItemId, ctx: &mut ViewContext<Self>) {
        ctx.emit(EnvVarCollectionEvent::ViewInWarpDrive(id));
    }

    // This is a public re-export of close since it's a trait method
    pub(super) fn close_env_var_collection(&mut self, ctx: &mut ViewContext<Self>) {
        self.close(ctx);
    }

    fn render_variable_rows(
        &self,
        editability: ContentEditability,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Vec<Box<dyn Element>> {
        let variables: Vec<Box<dyn Element>> = self
            .variable_rows
            .iter()
            .enumerate()
            .map(|(index, variable_editor_row)| {
                let secret_menu_button = if variable_editor_row.secret_menu_is_focused {
                    highlight_icon_button_with_context_menu(
                        Icon::Key,
                        move |ctx, _, _| {
                            ctx.dispatch_typed_action(EnvVarCollectionAction::DisplaySecretMenu(
                                VariableRowIndex(index),
                            ));
                        },
                        variable_editor_row.secret_button_mouse_state.clone(),
                        &self.menus.secret_menu,
                        true,
                        MenuDirection::Right,
                        appearance,
                    )
                    .finish()
                } else {
                    appearance.ui_builder().tool_tip_on_element(
                        EDUCATION_TEXT.to_string(),
                        self.button_mouse_states.secret_tooltip_state.clone(),
                        icon_button_with_context_menu(
                            Icon::Key,
                            move |ctx, _, _| {
                                ctx.dispatch_typed_action(
                                    EnvVarCollectionAction::DisplaySecretMenu(VariableRowIndex(
                                        index,
                                    )),
                                );
                            },
                            variable_editor_row.secret_button_mouse_state.clone(),
                            &self.menus.secret_menu,
                            false,
                            MenuDirection::Right,
                            Some(Cursor::PointingHand),
                            None,
                            appearance,
                        )
                        .finish(),
                        ParentAnchor::TopLeft,
                        ChildAnchor::BottomRight,
                        vec2f(0., 5.),
                    )
                };

                let mut row_contents = Flex::row()
                    .with_child(self.render_variable_editor(
                        appearance,
                        variable_editor_row.variable_name_editor.clone(),
                        EditorType::Name,
                        None,
                        Some(index),
                    ))
                    .with_child(match &variable_editor_row.value {
                        EnvVarValue::Constant(_) => self.render_variable_editor(
                            appearance,
                            variable_editor_row.variable_value_editor.clone(),
                            EditorType::Value,
                            if variable_editor_row
                                .variable_value_editor
                                .as_ref(app)
                                .is_empty(app)
                            {
                                Some(secret_menu_button)
                            } else {
                                None
                            },
                            Some(index),
                        ),
                        secret @ EnvVarValue::Secret(_) => self.render_secret_or_command_button(
                            appearance,
                            secret,
                            variable_editor_row
                                .rendered_secret_button_mouse_state
                                .clone(),
                            index,
                            variable_editor_row.rendered_secret_menu_is_focused,
                            editability,
                        ),
                        command @ EnvVarValue::Command(_) => self.render_secret_or_command_button(
                            appearance,
                            command,
                            variable_editor_row
                                .rendered_command_button_mouse_state
                                .clone(),
                            index,
                            variable_editor_row.rendered_command_menu_is_focused,
                            editability,
                        ),
                    });

                if !FeatureFlag::SharedWithMe.is_enabled() || editability.can_edit() {
                    row_contents.add_child(
                        Container::new(
                            icon_button(
                                appearance,
                                Icon::MinusCircle,
                                false,
                                variable_editor_row.delete_row_mouse_state_handle.clone(),
                            )
                            .build()
                            .on_click(move |ctx, _, _| {
                                ctx.dispatch_typed_action(EnvVarCollectionAction::DeleteVariable(
                                    VariableRowIndex(index),
                                ))
                            })
                            .finish(),
                        )
                        .finish(),
                    );
                }

                Container::new(
                    Flex::column()
                        .with_child(
                            Container::new(row_contents.finish())
                                .with_margin_bottom(ROW_SPACING)
                                .finish(),
                        )
                        .with_child(
                            Container::new(self.render_variable_editor(
                                appearance,
                                variable_editor_row.variable_description_editor.clone(),
                                EditorType::Description,
                                None,
                                Some(index),
                            ))
                            .with_margin_right(DESCRIPTION_EDITOR_CUTOFF)
                            .with_margin_bottom(DESCRIPTION_BOTTOM_MARGIN)
                            .finish(),
                        )
                        .with_child(
                            Container::new(self.render_divider(appearance, index))
                                .with_margin_bottom(DIVIDER_BOTTOM_MARGIN)
                                .finish(),
                        )
                        .finish(),
                )
                .with_margin_bottom(ROW_SPACING)
                .finish()
            })
            .collect();

        variables
    }
}

impl Entity for EnvVarCollectionView {
    type Event = EnvVarCollectionEvent;
}

impl View for EnvVarCollectionView {
    fn ui_name() -> &'static str {
        "EnvVarCollectionView"
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            self.focused = true;
            ctx.notify();
        }
    }

    fn on_blur(&mut self, blur_ctx: &BlurContext, ctx: &mut ViewContext<Self>) {
        if blur_ctx.is_self_blurred() {
            self.focused = false;
            ctx.notify();
        }
    }

    fn render(&self, app: &AppContext) -> Box<dyn warpui::Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let mut content = Flex::column();
        let access_level = self
            .active_env_var_collection_data
            .as_ref(app)
            .access_level(app);
        let editability = self
            .active_env_var_collection_data
            .as_ref(app)
            .editability(app);

        content.extend(self.render_trash_banner(access_level, app));

        content.add_child(
            Align::new(
                ConstrainedBox::new(
                    Align::new(
                        Container::new(render_breadcrumbs(
                            self.breadcrumbs.clone(),
                            appearance,
                            |ctx, _, breadcrumb| {
                                ctx.dispatch_typed_action(EnvVarCollectionAction::ViewInWarpDrive(
                                    breadcrumb.kind.into_item_id(),
                                ));
                            },
                        ))
                        .with_horizontal_margin(CORE_HORIZONATAL_MARGIN)
                        .with_vertical_margin(CORE_VERTICAL_MARGIN / 2.)
                        .finish(),
                    )
                    .top_left()
                    .finish(),
                )
                .with_max_width(CORE_MAX_WIDTH)
                .finish(),
            )
            .top_center()
            .finish(),
        );

        if let TrashStatus::Active = self
            .active_env_var_collection_data
            .as_ref(app)
            .trash_status(app)
        {
            let mut buttons_row = Flex::row()
                .with_child(Container::new(self.render_invoke_button(appearance, app)).finish())
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::End)
                .with_cross_axis_alignment(CrossAxisAlignment::Center);
            if !FeatureFlag::SharedWithMe.is_enabled() || editability.can_edit() {
                buttons_row.add_child(
                    Container::new(self.render_save_button(appearance, app))
                        .with_margin_left(BUTTON_SPACING)
                        .finish(),
                )
            }

            content.add_child(
                Align::new(
                    ConstrainedBox::new(
                        Clipped::new(
                            Flex::column()
                                .with_child(
                                    Container::new(buttons_row.finish())
                                        .with_horizontal_margin(BUTTON_CONTAINER_HORIZONTAL_MARGIN)
                                        .with_margin_bottom(BUTTON_CONTAINER_BOTTOM_MARGIN)
                                        .finish(),
                                )
                                .with_cross_axis_alignment(CrossAxisAlignment::End)
                                .finish(),
                        )
                        .finish(),
                    )
                    .with_max_width(CORE_MAX_WIDTH)
                    .finish(),
                )
                .top_center()
                .finish(),
            );
        }

        let mut flex = Flex::column()
            .with_child(
                Container::new(self.render_metadata(appearance))
                    .with_margin_bottom(SECTION_SPACING)
                    .finish(),
            )
            .with_child(
                Container::new(self.render_variables_section_header(editability, appearance, app))
                    .with_margin_bottom(SECTION_SPACING)
                    .finish(),
            )
            .with_child(
                Flex::column()
                    .with_children(self.render_variable_rows(editability, appearance, app))
                    .finish(),
            );
        if let Some(error_element) = self.render_bottom_error_message(appearance) {
            flex.add_child(error_element);
        }

        content.add_child(
            Shrinkable::new(
                1.,
                Align::new(
                    ConstrainedBox::new(
                        ClippedScrollable::vertical(
                            self.variables_clipped_scroll_state.clone(),
                            Container::new(flex.finish())
                                .with_horizontal_margin(CORE_HORIZONATAL_MARGIN)
                                .with_margin_bottom(CORE_VERTICAL_MARGIN)
                                .finish(),
                            SCROLLBAR_WIDTH,
                            theme.disabled_text_color(theme.background()).into(),
                            theme.main_text_color(theme.background()).into(),
                            Fill::None,
                        )
                        .finish(),
                    )
                    .with_max_width(CORE_MAX_WIDTH)
                    .finish(),
                )
                .top_center()
                .finish(),
            )
            .finish(),
        );

        let content_container = if self.dialog_open_states.has_open_dialog() {
            Container::new(content.finish())
                .with_foreground_overlay(appearance.theme().inactive_pane_overlay())
                .finish()
        } else {
            content.finish()
        };

        let mut stack = Stack::new().with_child(content_container);

        let dialog_position = OffsetPositioning::from_axes(
            PositioningAxis::relative_to_parent(
                ParentOffsetBounds::ParentBySize,
                OffsetType::Pixel(0.),
                AnchorPair::new(XAxisAnchor::Middle, XAxisAnchor::Middle),
            ),
            PositioningAxis::relative_to_parent(
                ParentOffsetBounds::ParentBySize,
                OffsetType::Pixel(-250.),
                AnchorPair::new(YAxisAnchor::Middle, YAxisAnchor::Top),
            ),
        );

        if self.dialog_open_states.unsaved_changes_dialog_open {
            stack.add_positioned_child(
                self.render_unsaved_changes_dialog(appearance),
                dialog_position,
            )
        } else if self.dialog_open_states.secrets_dialog_open {
            stack.add_positioned_overlay_child(
                ChildView::new(&self.secrets_dialog).finish(),
                dialog_position,
            );
        } else if self.dialog_open_states.env_var_command_dialog_open {
            stack.add_positioned_overlay_child(
                Clipped::new(ChildView::new(&self.env_var_command_dialog).finish()).finish(),
                dialog_position,
            );
        } else if let Some(offset) = self.pane_context_menu_offset {
            stack.add_positioned_child(
                ChildView::new(&self.menus.pane_context_menu).finish(),
                OffsetPositioning::offset_from_parent(
                    offset,
                    ParentOffsetBounds::WindowByPosition,
                    ParentAnchor::TopLeft,
                    ChildAnchor::TopLeft,
                ),
            );
        }

        EventHandler::new(
            Align::new(SavePosition::new(stack.finish(), &self.view_position_id).finish())
                .top_center()
                .finish(),
        )
        .on_right_mouse_down(|ctx, _, position| {
            ctx.dispatch_typed_action(EnvVarCollectionAction::DisplayPaneMenu(position));
            DispatchEventResult::StopPropagation
        })
        .finish()
    }
}

impl TypedActionView for EnvVarCollectionView {
    type Action = EnvVarCollectionAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            EnvVarCollectionAction::SaveVariables => self.save_env_var_collection(ctx),
            EnvVarCollectionAction::Invoke => self.invoke_env_var_collection(ctx),
            EnvVarCollectionAction::Close => self.close(ctx),
            EnvVarCollectionAction::AddVariable => self.add_variable_row(ctx),
            EnvVarCollectionAction::DeleteVariable(VariableRowIndex(index)) => {
                self.delete_row(*index, ctx);
            }
            EnvVarCollectionAction::Untrash => self.untrash_env_var_collection(ctx),
            EnvVarCollectionAction::CopyLink(link) => {
                send_telemetry_from_ctx!(
                    TelemetryEvent::ObjectLinkCopied { link: link.clone() },
                    ctx
                );
                ctx.clipboard()
                    .write(ClipboardContent::plain_text(link.to_owned()));
            }
            EnvVarCollectionAction::Duplicate => self.duplicate_env_var_collection(ctx),
            EnvVarCollectionAction::Trash => self.trash_env_var_collection(ctx),
            EnvVarCollectionAction::Export => self.export_env_var_collection(ctx),
            EnvVarCollectionAction::SelectSecretManager(secret_manager) => {
                self.fetch_secret(secret_manager.clone(), ctx)
            }
            EnvVarCollectionAction::DisplayCommandDialog => self.display_command_dialog(None, ctx),
            EnvVarCollectionAction::ClearSecret => {
                self.clear_secret(ctx);
            }
            EnvVarCollectionAction::EditCommand => {
                self.display_command_dialog(self.pending_variable_row_index.clone(), ctx);
            }
            EnvVarCollectionAction::DisplaySecretMenu(VariableRowIndex(index)) => {
                self.display_secret_menu(*index)
            }
            EnvVarCollectionAction::DisplayPaneMenu(position) => {
                self.display_pane_context_menu(position, ctx);
            }
            EnvVarCollectionAction::DisplayRenderedSecretMenu(VariableRowIndex(index)) => {
                self.display_rendered_secret_menu(*index)
            }
            EnvVarCollectionAction::DisplayRenderedCommandMenu(VariableRowIndex(index)) => {
                self.display_rendered_command_menu(*index)
            }
            EnvVarCollectionAction::EmitPaneEvent(pane_event) => {
                ctx.emit(EnvVarCollectionEvent::Pane(pane_event.clone()));
            }
            EnvVarCollectionAction::ForceClose => {
                self.dialog_open_states.unsaved_changes_dialog_open = false;
                self.update_open_modal_state(ctx);
                ctx.emit(EnvVarCollectionEvent::Pane(PaneEvent::Close));
            }
            EnvVarCollectionAction::CloseUnsavedChangesDialog => {
                self.dialog_open_states.unsaved_changes_dialog_open = false;
                self.update_open_modal_state(ctx);
                ctx.notify();
            }
            EnvVarCollectionAction::ViewInWarpDrive(id) => self.view_in_warp_drive(*id, ctx),
        }
    }
}

impl BackingView for EnvVarCollectionView {
    type PaneHeaderOverflowMenuAction = EnvVarCollectionAction;
    type CustomAction = ();
    type AssociatedData = ();

    fn pane_header_overflow_menu_items(
        &self,
        ctx: &AppContext,
    ) -> Vec<MenuItem<EnvVarCollectionAction>> {
        self.overflow_menu_items(ctx)
    }

    fn handle_pane_header_overflow_menu_action(
        &mut self,
        action: &Self::PaneHeaderOverflowMenuAction,
        ctx: &mut ViewContext<Self>,
    ) {
        self.handle_action(action, ctx);
    }

    fn close(&mut self, ctx: &mut ViewContext<Self>) {
        if self
            .active_env_var_collection_data
            .as_ref(ctx)
            .saving_status
            == SavingStatus::Unsaved
        {
            self.dialog_open_states.unsaved_changes_dialog_open = true;
            self.update_open_modal_state(ctx);
            ctx.notify();
        } else {
            ctx.emit(EnvVarCollectionEvent::Pane(PaneEvent::Close));
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
        let title = self.title_editor.as_ref(app).buffer_text(app);
        let title = if title.is_empty() { "Untitled" } else { &title };
        view::HeaderContent::simple(title)
    }

    fn set_focus_handle(&mut self, focus_handle: PaneFocusHandle, _ctx: &mut ViewContext<Self>) {
        self.focus_handle = Some(focus_handle);
    }
}
