use std::collections::HashMap;
use std::collections::HashSet;
use std::{cmp::Ordering, sync::Arc};

use itertools::Itertools;
use pathfinder_geometry::vector::vec2f;
use string_offset::CharOffset;
use warp_core::ui::theme::Fill;
use warp_editor::editor::NavigationKey;
use warpui::elements::Clipped;
use warpui::FocusContext;
use warpui::{
    clipboard::ClipboardContent,
    elements::{
        Align, Border, ChildAnchor, ClippedScrollStateHandle, ClippedScrollable, ConstrainedBox,
        Container, CornerRadius, CrossAxisAlignment, Flex, MainAxisAlignment, MainAxisSize,
        MouseStateHandle, OffsetPositioning, ParentAnchor, ParentElement, ParentOffsetBounds,
        Radius, ScrollbarWidth, Shrinkable, Stack,
    },
    fonts::{FamilyId, Weight},
    platform::Cursor,
    presenter::ChildView,
    ui_components::{
        button::{ButtonVariant, TextAndIcon, TextAndIconAlignment},
        components::{Coords, UiComponent, UiComponentStyles},
    },
    AppContext, Element, Entity, SingletonEntity, TypedActionView, UpdateView, View, ViewContext,
    ViewHandle,
};

use crate::auth::UserUid;
use crate::{
    appearance::Appearance,
    cloud_object::{
        breadcrumbs::{ContainingObject, ContainingObjectKind},
        model::persistence::{CloudModel, CloudModelEvent},
        CloudObject, CloudObjectEventEntrypoint, ObjectType, Owner, Revision,
    },
    drive::{
        cloud_object_styling::warp_drive_icon_color, items::WarpDriveItemId, CloudObjectTypeAndId,
        DriveObjectType,
    },
    editor::{
        EditorOptions, EditorView, EnterAction, EnterSettings, Event as EditorEvent,
        InteractionState, PlainTextEditorViewAction as EditorAction,
        PropagateAndNoOpNavigationKeys, TextOptions, TextStyleOperation,
    },
    menu::{Event, Menu, MenuItem, MenuItemFields},
    network::NetworkStatus,
    server::{
        cloud_objects::update_manager::UpdateManager,
        ids::{ClientId, ServerId, SyncId},
        server_api::ai::AIClient,
    },
    themes::theme::AnsiColorIdentifier,
    ui_components::{
        blended_colors,
        breadcrumb::{self, BreadcrumbState},
        buttons::icon_button,
        dialog::{dialog_styles, Dialog},
        icons::{self, Icon, ICON_DIMENSIONS},
        menu_button::{icon_button_with_context_menu, MenuDirection},
    },
    workflows::{
        workflow::{Argument, Workflow},
        CloudWorkflow,
    },
};

use super::arguments::ArgumentsState;
use super::enum_creation_dialog::{EnumCreationDialog, EnumCreationDialogEvent, WorkflowEnumData};
use super::workflow_arg_selector::{
    WorkflowArgSelector, WorkflowArgSelectorEvent, WorkflowArgSelectorStyles,
};
use super::workflow_arg_type_helpers::{self, ArgumentEditorRowIndex};

const BREADCRUMBS_VERTICAL_MARGIN: f32 = 6.;
const MODAL_WIDTH: f32 = 900.;
const MODAL_VERTICAL_PADDING: f32 = 20.;
const MODAL_VERTICAL_MARGIN: f32 = 50.;
const MODAL_HORIZONTAL_MARGIN: f32 = 28.;
const MODAL_HORIZONTAL_PADDING: f32 = 16.;
const MODAL_BORDER_RADIUS: f32 = 8.;
const DESCRIPTION_FONT_SIZE: f32 = 14.;
const DESCRIPTION_EDITOR_TOP_PADDING: f32 = 12.;
const DESCRIPTION_EDITOR_MAX_HEIGHT: f32 = 100.;
const COMMAND_EDITOR_PADDING: f32 = 16.;
const CONTENT_EDITOR_FONT_SIZE: f32 = 14.;
const COMMAND_EDITOR_MIN_LINES: f32 = 3.;
const ARGUMENT_INPUT_WIDTH: f32 = 300.;
const ARGUMENT_INPUT_PADDING: f32 = 12.;
const ARGUMENT_INPUT_MARGIN: f32 = 8.;
const ARGUMENT_EDITOR_FONT_SIZE: f32 = 14.;
const BUTTON_PADDING: f32 = 12.;
const BUTTON_FONT_SIZE: f32 = 14.;
const BUTTON_BORDER_RADIUS: f32 = 4.;
const BORDER_WIDTH: f32 = 1.;
const DIALOG_WIDTH: f32 = 460.;
const AI_ASSIST_BUTTON_SIZE: f32 = 96.;
const SCROLLBAR_WIDTH: ScrollbarWidth = ScrollbarWidth::Auto;

const TITLE_PLACEHOLDER_TEXT: &str = "Untitled workflow";
const DESCRIPTION_PLACEHOLDER_TEXT: &str = "Add a description";
const COMMAND_EDITOR_PLACEHOLDER_TEXT: &str =
    "echo \"Hello {{your_name}}\" # insert arguments with curly braces\n# enter a single-line command or an entire shell script";
const ARGUMENT_BUTTON_TEXT: &str = "New argument";
const ARGUMENT_DESCRIPTION_PLACEHOLDER_TEXT: &str = "Description";
const ARGUMENT_DEFAULT_VALUE_PLACEHOLDER_TEXT: &str = "Default value (optional)";
const SAVE_BUTTON_TEXT: &str = "Save workflow";
const AI_ASSIST_BUTTON_TEXT: &str = "Autofill";
const AI_ASSIST_LOADING_TEXT: &str = "Loading";
const DEFAULT_ARGUMENT_PREFIX: &str = "argument";
const UNSAVED_CHANGES_TEXT: &str = "You have unsaved changes.";
const KEEP_EDITING_TEXT: &str = "Keep editing";
const DISCARD_CHANGES_TEXT: &str = "Discard changes";

#[derive(Default)]
struct MouseStateHandles {
    close_modal_state: MouseStateHandle,
    new_argument_state: MouseStateHandle,
    save_workflow_state: MouseStateHandle,
    keep_editing_state: MouseStateHandle,
    discard_changes_state: MouseStateHandle,
    ai_assist_state: MouseStateHandle,
    ai_assist_tool_tip: MouseStateHandle,
    menu_state: MouseStateHandle,
}

pub(super) enum AiAssistState {
    PreRequest,
    RequestInFlight,
    Generated,
}

/// Represents a particular row for editing a single argument.
/// Note that maintaining the order is very important. A new argument can be
/// inserted anywhere in the command, or the name of an arg can change, but we
/// should try to preserve as much state as possible (e.g., the description
/// value should be maintained even if we change the name of the argument).
pub(super) struct ArgumentEditorRow {
    name: String,
    pub(super) description_editor: ViewHandle<EditorView>,
    pub(super) default_value_editor: ViewHandle<EditorView>,
    pub(super) typed_default_value_editor: ViewHandle<WorkflowArgSelector>,
}

pub struct WorkflowModal {
    is_open: bool,
    owner: Option<Owner>,
    initial_folder_id: Option<SyncId>,
    /// Only present if the workflow already exists in the cloud.
    workflow_id: Option<SyncId>,
    button_mouse_states: MouseStateHandles,
    errors: WorkflowEditorErrorState,
    pub(super) title_editor: ViewHandle<EditorView>,
    pub(super) description_editor: ViewHandle<EditorView>,
    pub(super) content_editor: ViewHandle<EditorView>,
    /// How many times the "add argument" button was clicked. We use this value
    /// to append a number to the default argument name (argument_1, argument_2,
    /// etc.).
    default_argument_id: usize,
    pub(super) arguments_state: ArgumentsState,
    pub(super) arguments_rows: Vec<ArgumentEditorRow>,
    show_unsaved_changes_dialog: bool,
    revision_ts: Option<Revision>,
    pub(super) ai_client: Arc<dyn AIClient>,
    pub(super) ai_metadata_assist_state: AiAssistState,
    breadcrumbs: Option<Vec<BreadcrumbState<ContainingObject>>>,
    /// ID of the breadcrumb space/folder a user clicked on before the unsaved dialog popped up
    clicked_breadcrumb: Option<WarpDriveItemId>,
    menu: ViewHandle<Menu<WorkflowModalAction>>,
    menu_open: bool,
    arguments_clipped_scroll_state: ClippedScrollStateHandle,
    pending_argument_editor_row: Option<ArgumentEditorRowIndex>,
    show_enum_creation_dialog: bool,
    enum_creation_dialog: ViewHandle<EnumCreationDialog>,
    all_workflow_enums: HashMap<SyncId, WorkflowEnumData>,
}

#[derive(Clone, Debug)]
pub enum WorkflowModalAction {
    AddArgument,
    Close,
    Save,
    CloseUnsavedChangesDialog,
    ForceClose,
    AiAssist,
    ViewInWarpDrive(WarpDriveItemId),
    OpenOverflowMenu,
    CopyObjectToClipboard,
    TrashObject,
}

pub enum WorkflowModalEvent {
    Close,
    UpdatedWorkflow(SyncId),
    AiAssistError(String),
    AiAssistUpgradeError(Option<ServerId>, UserUid),
    ViewInWarpDrive(WarpDriveItemId),
}

/// A grouping of various error states the modal can be in. Any of these being
/// `true` prevents the save button from being clickable.
#[derive(Default)]
struct WorkflowEditorErrorState {
    /// The content must not be whitespace-only.
    content_empty_error: bool,
    /// The content must not have any arguments that are invalid (e.g. start
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

impl WorkflowModal {
    pub fn new(ai_client: Arc<dyn AIClient>, ctx: &mut ViewContext<Self>) -> Self {
        let appearance = Appearance::as_ref(ctx);
        let header_font_size = appearance.header_font_size();
        let ui_font_family = appearance.ui_font_family();

        let title_editor: ViewHandle<EditorView> = Self::create_editor_handle(
            ctx,
            Some(header_font_size),
            Some(ui_font_family),
            Some(TITLE_PLACEHOLDER_TEXT),
            false, /* vim_keybindings */
            true,  /* single_line */
        );

        ctx.subscribe_to_view(&title_editor, |me, _, event, ctx| {
            me.handle_title_editor_event(event, ctx);
        });

        let description_editor = Self::create_editor_handle(
            ctx,
            Some(DESCRIPTION_FONT_SIZE),
            Some(ui_font_family),
            Some(DESCRIPTION_PLACEHOLDER_TEXT),
            false, /* vim_keybindings */
            false, /* single_line */
        );

        ctx.subscribe_to_view(&description_editor, |me, _, event, ctx| {
            me.handle_description_editor_event(event, ctx);
        });

        let content_editor = Self::create_editor_handle(
            ctx,
            Some(CONTENT_EDITOR_FONT_SIZE),
            None,
            Some(COMMAND_EDITOR_PLACEHOLDER_TEXT),
            true,  /* vim_keybindings */
            false, /* single_line */
        );

        ctx.subscribe_to_view(&content_editor, |me, _, event, ctx| {
            me.handle_content_editor_event(event, ctx);
        });

        let cloud_model = CloudModel::handle(ctx);
        ctx.subscribe_to_model(&cloud_model, |me, _, event, ctx| {
            me.handle_cloud_model_event(event, ctx);
        });

        let menu = ctx.add_typed_action_view(|_ctx| {
            Menu::new()
                .prevent_interaction_with_other_elements()
                .with_drop_shadow()
        });

        ctx.subscribe_to_view(&menu, |me, _, event, ctx| {
            me.handle_menu_event(event, ctx);
        });

        let enum_creation_dialog = ctx.add_typed_action_view(EnumCreationDialog::new);
        ctx.subscribe_to_view(&enum_creation_dialog, |me, _, event, ctx| {
            me.handle_enum_creation_dialog_event(event, ctx);
        });

        Self {
            is_open: false,
            owner: None,
            initial_folder_id: None,
            workflow_id: None,
            button_mouse_states: Default::default(),
            errors: WorkflowEditorErrorState::new(),
            title_editor,
            description_editor,
            content_editor,
            default_argument_id: 0,
            arguments_state: Default::default(),
            arguments_rows: Vec::new(),
            show_unsaved_changes_dialog: false,
            revision_ts: None,
            ai_client,
            ai_metadata_assist_state: AiAssistState::PreRequest,
            breadcrumbs: Default::default(),
            clicked_breadcrumb: None,
            menu,
            menu_open: false,
            arguments_clipped_scroll_state: Default::default(),
            pending_argument_editor_row: None,
            show_enum_creation_dialog: false,
            enum_creation_dialog,
            all_workflow_enums: Default::default(),
        }
    }

    pub(super) fn disable_editors(&mut self, ctx: &mut ViewContext<Self>) {
        self.description_editor.update(ctx, |view, ctx| {
            view.set_interaction_state(InteractionState::Disabled, ctx)
        });

        self.title_editor.update(ctx, |view, ctx| {
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
            row.typed_default_value_editor.update(ctx, |selector, ctx| {
                selector.disable(ctx);
            })
        });
    }

    pub(super) fn enable_editors(&mut self, ctx: &mut ViewContext<Self>) {
        self.description_editor.update(ctx, |view, ctx| {
            view.set_interaction_state(InteractionState::Editable, ctx)
        });

        self.title_editor.update(ctx, |view, ctx| {
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
            row.typed_default_value_editor.update(ctx, |selector, ctx| {
                selector.enable(ctx);
            })
        });
    }

    fn create_editor_handle(
        ctx: &mut ViewContext<Self>,
        font_size_override: Option<f32>,
        font_family_override: Option<FamilyId>,
        placeholder_text: Option<&str>,
        vim_keybindings: bool,
        single_line: bool,
    ) -> ViewHandle<EditorView> {
        ctx.add_typed_action_view(|ctx| {
            let mut editor = EditorView::new(
                EditorOptions {
                    text: TextOptions {
                        font_size_override,
                        font_family_override,
                        ..Default::default()
                    },
                    soft_wrap: true,
                    autogrow: true,
                    autocomplete_symbols: true,
                    // Ideally, we'd set this to PropagateAndNoOpNavigationKeys::AtBoundary, so
                    // that the workflow modal doesn't need to handle up/down navigation for the
                    // content and description editors. However, that breaks tab and shift-tab
                    // navigation, since those are only emitted with
                    // PropagateAndNoOpNavigationKeys::Never.
                    propagate_and_no_op_vertical_navigation_keys:
                        PropagateAndNoOpNavigationKeys::Always,
                    supports_vim_mode: vim_keybindings,
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
            );

            if let Some(text) = placeholder_text {
                editor.set_placeholder_text(text, ctx);
            }

            editor
        })
    }

    /// Opens the modal with no preexisting workflow.
    /// This represents the creation experience; saving this workflow will add
    /// a new one to the space specified.
    pub fn open_with_new(
        &mut self,
        owner: Owner,
        initial_folder_id: Option<SyncId>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.is_open = true;
        self.initial_folder_id = initial_folder_id;
        self.owner = Some(owner);
        self.workflow_id = None;
        self.compute_breadcrumbs(ctx);
        self.all_workflow_enums =
            workflow_arg_type_helpers::load_workflow_enums_with_owner(owner, ctx);
        ctx.notify();
    }

    /// Populate the modal with the data of a [`Workflow`] struct
    #[allow(dead_code)]
    fn populate(&mut self, workflow: Workflow, ctx: &mut ViewContext<Self>) {
        // Sanitize the arguments generated for the workflow by removing any illegal characters.
        // Necessary since Warp AI command search sometimes provides arguments in an invalid argument format.
        let mut sanitized_content = workflow.content().to_string();
        let sanitized_arguments = workflow
            .arguments()
            .iter()
            .map(|argument| {
                let new_argument = argument
                    .name
                    .replace('.', "_")
                    .chars()
                    .skip_while(|c| c.is_numeric())
                    .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
                    .collect::<String>();

                // Replace the old argument name in the path with the cleaned one
                sanitized_content = sanitized_content.replace(&argument.name, &new_argument);

                Argument {
                    name: new_argument,
                    ..argument.clone()
                }
            })
            .collect::<Vec<Argument>>();

        self.title_editor.update(ctx, |editor, ctx| {
            editor.set_buffer_text_with_base_buffer(workflow.name(), ctx);
        });
        self.description_editor.update(ctx, |editor, ctx| {
            editor.set_buffer_text_with_base_buffer(
                workflow
                    .description()
                    .map(String::as_str)
                    .unwrap_or_default(),
                ctx,
            );
        });
        self.content_editor.update(ctx, |editor, ctx| {
            editor.set_buffer_text_with_base_buffer(&sanitized_content, ctx);
        });

        // note: normally, we wouldn't have to do this, since editing the content
        // editor's text will trigger the event that does this automatically.
        // however, that happens in a callback, yet we need to know what the args
        // are right away to populate the description/default value editors.
        self.arguments_state =
            ArgumentsState::for_command_workflow(&self.arguments_state, sanitized_content);
        self.update_arguments_rows(ctx);

        sanitized_arguments
            .iter()
            .enumerate()
            .for_each(|(index, argument)| {
                if let Some(description) = &argument.description {
                    self.arguments_rows[index]
                        .description_editor
                        .update(ctx, |editor, ctx| {
                            editor.set_buffer_text_with_base_buffer(description.as_str(), ctx);
                        });
                }

                if let Some(default_value) = &argument.default_value {
                    self.arguments_rows[index]
                        .default_value_editor
                        .update(ctx, |editor, ctx| {
                            editor.set_buffer_text_with_base_buffer(default_value.as_str(), ctx);
                        });
                }

                self.arguments_rows[index]
                    .typed_default_value_editor
                    .update(ctx, |selector, ctx| {
                        workflow_arg_type_helpers::load_argument_into_selector(
                            selector,
                            argument,
                            &mut self.all_workflow_enums,
                            ctx,
                        );
                    });
            });
    }

    pub fn close(&mut self, force: bool, ctx: &mut ViewContext<Self>) {
        if !force && self.should_show_unsaved_changes_dialog(ctx) {
            self.show_unsaved_changes_dialog(ctx);
            return;
        }

        self.hide_unsaved_changes_dialog(ctx);
        self.ai_metadata_assist_state = AiAssistState::PreRequest;

        self.is_open = false;
        self.owner = None;
        self.initial_folder_id = None;
        self.workflow_id = None;
        self.revision_ts = None;

        self.title_editor.update(ctx, |editor, ctx| {
            editor.clear_buffer_and_reset_undo_stack(ctx);
            editor.set_base_buffer_text("".to_string(), ctx);
        });
        self.description_editor.update(ctx, |editor, ctx| {
            editor.clear_buffer_and_reset_undo_stack(ctx);
            editor.set_base_buffer_text("".to_string(), ctx);
        });
        self.content_editor.update(ctx, |editor, ctx| {
            editor.clear_buffer_and_reset_undo_stack(ctx);
            editor.set_base_buffer_text("".to_string(), ctx);
        });
        self.default_argument_id = 0;
        self.arguments_rows.clear();
        self.arguments_state = Default::default();

        ctx.emit(WorkflowModalEvent::Close);
    }

    fn show_unsaved_changes_dialog(&mut self, ctx: &mut ViewContext<Self>) {
        self.show_unsaved_changes_dialog = true;
        self.disable_editors(ctx);
        ctx.notify();
    }

    fn hide_unsaved_changes_dialog(&mut self, ctx: &mut ViewContext<Self>) {
        self.show_unsaved_changes_dialog = false;
        self.enable_editors(ctx);
        ctx.notify();
    }

    fn show_enum_creation_dialog(&mut self, ctx: &mut ViewContext<Self>) {
        self.show_enum_creation_dialog = true;
        self.disable_editors(ctx);
        ctx.notify();
    }

    fn hide_enum_creation_dialog(&mut self, ctx: &mut ViewContext<Self>) {
        self.show_enum_creation_dialog = false;
        self.enable_editors(ctx);
        ctx.notify();
    }

    pub fn is_open(&self) -> bool {
        self.is_open
    }

    fn is_dirty(&self, app: &AppContext) -> bool {
        let title_is_dirty = self.title_editor.as_ref(app).is_dirty(app);
        let description_is_dirty = self.description_editor.as_ref(app).is_dirty(app);
        let content_is_dirty = self.content_editor.as_ref(app).is_dirty(app);
        let any_argument_editor_is_dirty = self.arguments_rows.iter().any(|row| {
            let selector_is_dirty = {
                let editor = row.typed_default_value_editor.as_ref(app);
                let editor_is_dirty = editor.is_dirty(app);
                let enum_is_dirty = editor
                    .get_selected_enum()
                    .and_then(|id| self.all_workflow_enums.get(&id))
                    .map(|enum_data| enum_data.new_data.is_some())
                    .unwrap_or(false);
                editor_is_dirty || enum_is_dirty
            };

            selector_is_dirty
                || row.default_value_editor.as_ref(app).is_dirty(app)
                || row.description_editor.as_ref(app).is_dirty(app)
        });

        title_is_dirty || description_is_dirty || content_is_dirty || any_argument_editor_is_dirty
    }

    fn is_empty(&self, app: &AppContext) -> bool {
        let title_is_empty = self.title_editor.as_ref(app).is_empty(app);
        let description_is_empty = self.description_editor.as_ref(app).is_empty(app);
        let content_is_empty = self.content_editor.as_ref(app).is_empty(app);

        title_is_empty && description_is_empty && content_is_empty
    }

    fn view_in_warp_drive(&mut self, id: WarpDriveItemId, ctx: &mut ViewContext<Self>) {
        ctx.emit(WorkflowModalEvent::ViewInWarpDrive(id));
        self.close(false /* force */, ctx);
        self.clicked_breadcrumb = None;
    }

    fn handle_menu_event(&mut self, event: &Event, ctx: &mut ViewContext<Self>) {
        if let Event::Close { .. } = event {
            self.close_overflow_menu(ctx);
        }
    }

    fn open_overflow_menu(&mut self, ctx: &mut ViewContext<Self>) {
        let menu_items = self.menu_items(ctx);
        // We need to set items every time the menu is opened because we don't know whether the Trash action is available
        ctx.update_view(&self.menu, |menu, ctx| {
            menu.set_items(menu_items, ctx);
        });

        if !self.menu_open {
            self.menu_open = true;
            ctx.focus(&self.menu);
            ctx.notify();
        }
    }

    pub fn close_overflow_menu(&mut self, ctx: &mut ViewContext<Self>) {
        self.menu_open = false;
        ctx.notify();
    }

    // Identical to logic in DriveIndexAction::CopyObjectToClipboard
    fn copy_object_to_clipboard(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(workflow_id) = self.workflow_id {
            let cloud_model = CloudModel::as_ref(ctx);
            let object = cloud_model.get_by_uid(&workflow_id.uid());

            if let Some(object) = object {
                match object.object_type() {
                    ObjectType::Workflow => {
                        let workflow: Option<&CloudWorkflow> = object.into();
                        if let Some(workflow) = workflow {
                            let content = workflow.model().data.content().to_owned();
                            ctx.clipboard().write(ClipboardContent::plain_text(content));
                        }
                    }
                    ObjectType::Notebook
                    | ObjectType::Folder
                    | ObjectType::GenericStringObject(_) => {}
                }
            }

            self.close_overflow_menu(ctx);
        }
    }

    fn trash_object(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(workflow_id) = self.workflow_id {
            self.close_overflow_menu(ctx);

            // Close workflow editor
            self.close(true, ctx);

            UpdateManager::handle(ctx).update(ctx, move |update_manager, ctx| {
                update_manager.trash_object(
                    CloudObjectTypeAndId::from_id_and_type(workflow_id, ObjectType::Workflow),
                    ctx,
                );
            });
        }
    }

    fn menu_items(&self, app: &AppContext) -> Vec<MenuItem<WorkflowModalAction>> {
        let mut menu_items = Vec::new();

        // Add "Copy workflow text" to menu
        menu_items.push(
            MenuItemFields::new("Copy workflow text")
                .with_on_select_action(WorkflowModalAction::CopyObjectToClipboard)
                .with_icon(Icon::CopyMenuItem)
                .into_item(),
        );

        // Add "Trash" to menu
        if self.is_online(app) {
            menu_items.push(
                MenuItemFields::new("Trash")
                    .with_on_select_action(WorkflowModalAction::TrashObject)
                    .with_icon(Icon::Trash)
                    .into_item(),
            );
        }

        menu_items
    }

    pub fn should_show_unsaved_changes_dialog(&self, app: &AppContext) -> bool {
        // if we don't have a workflow_id, then we're creating a new workflow (not updating an
        // existing one). in that case, if there's no content in any editor, we bypass the unsaved
        // changes dialog entirely and allow them to close it.
        if self.workflow_id.is_none() && self.is_empty(app) {
            false
        } else {
            self.is_dirty(app)
        }
    }

    // If the title isn't supplied by the user, we use the first two words of the content as the title.
    fn truncate_content_for_title(&self, content: String) -> String {
        content.split_ascii_whitespace().take(2).join(" ")
    }

    fn save_workflow_and_close(&mut self, ctx: &mut ViewContext<Self>) {
        let content = self.content_editor.as_ref(ctx).buffer_text(ctx);
        let title_in_editor = self.title_editor.as_ref(ctx).buffer_text(ctx);
        let workflow_name = if title_in_editor.is_empty() {
            self.truncate_content_for_title(content.clone())
        } else {
            title_in_editor
        };
        let workflow_description = self.description_editor.as_ref(ctx).buffer_text(ctx);

        self.save_argument_objects(ctx);

        let mut workflow = Workflow::new(workflow_name, content);
        workflow = workflow.with_arguments(self.arguments_with_metadata(ctx));

        if !workflow_description.is_empty() {
            workflow = workflow.with_description(workflow_description);
        }

        match (self.workflow_id, self.owner) {
            (Some(workflow_id), None) => {
                UpdateManager::handle(ctx).update(ctx, |update_manager, ctx| {
                    update_manager.update_workflow(workflow, workflow_id, self.revision_ts.clone(), ctx);
                });
                ctx.emit(WorkflowModalEvent::UpdatedWorkflow(workflow_id));
            }
            (None, Some(owner)) => {
                UpdateManager::handle(ctx).update(ctx, |update_manager, ctx| {
                    update_manager.create_workflow(
                        workflow,
                        owner,
                        self.initial_folder_id,
                        ClientId::default(),
                        CloudObjectEventEntrypoint::Unknown,
                        true,
                        ctx,
                    );
                });
            }
            _ => log::error!("Only one of a workflow ID or space can be specified for saving workflows, but both or neither were specified instead")
        }

        self.close(true, ctx);
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

                    let type_selector = argument_row.typed_default_value_editor.as_ref(ctx);
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
        let owner = match (self.workflow_id, self.owner) {
            (Some(workflow_id), None) => CloudModel::as_ref(ctx)
                .get_workflow(&workflow_id)
                .map(|workflow| workflow.permissions.owner),
            (None, Some(owner)) => Some(owner),
            _ => {
                log::error!("Only one of a workflow ID or space can be specified");
                None
            }
        };

        self.arguments_rows.iter().for_each(|argument_row| {
            let type_selector = argument_row.typed_default_value_editor.as_ref(ctx);

            // Check to see if we have enum data for this id, then create a request for it
            if let Some(enum_data) = type_selector
                .get_selected_enum()
                .and_then(|id| self.all_workflow_enums.get(&id))
            {
                if enum_data.new_data.is_some() && !sent_requests.contains(&enum_data.id) {
                    workflow_arg_type_helpers::save_enum(enum_data, owner, ctx);

                    // Make sure we aren't sending duplicate requests
                    // We choose to do it this way so we don't end up creating/updating enums that aren't used
                    sent_requests.insert(enum_data.id);
                }
            }
        });
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

    fn handle_title_editor_event(&mut self, event: &EditorEvent, ctx: &mut ViewContext<Self>) {
        match event {
            EditorEvent::Navigate(NavigationKey::Tab) => {
                ctx.focus(&self.description_editor);
            }
            EditorEvent::Navigate(NavigationKey::ShiftTab) => match self.arguments_rows.last() {
                Some(row) => ctx.focus(&row.default_value_editor),
                None => ctx.focus(&self.content_editor),
            },
            EditorEvent::Escape => self.close(false, ctx),
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
            EditorEvent::Navigate(NavigationKey::Tab) => ctx.focus(&self.content_editor),
            EditorEvent::Navigate(NavigationKey::ShiftTab) => ctx.focus(&self.title_editor),
            EditorEvent::Navigate(NavigationKey::Up) => self
                .description_editor
                .update(ctx, |input, ctx| input.move_up(ctx)),
            EditorEvent::Navigate(NavigationKey::Down) => self
                .description_editor
                .update(ctx, |input, ctx| input.move_down(ctx)),
            EditorEvent::Escape => self.close(false, ctx),
            _ => {}
        }
    }

    // This method computes the breadcrumb data for the workflow editor. It should be called
    // every time either the cloud model or workflow ID changes.
    fn compute_breadcrumbs(&mut self, ctx: &mut ViewContext<Self>) {
        self.breadcrumbs = self.workflow_id.and_then(|workflow_id| {
            CloudModel::as_ref(ctx)
                .get_workflow(&workflow_id)
                .map(|workflow| {
                    workflow
                        .containing_objects_path(ctx)
                        .into_iter()
                        .map(BreadcrumbState::new)
                        .collect::<Vec<_>>()
                })
        });
        ctx.notify()
    }

    fn handle_cloud_model_event(&mut self, event: &CloudModelEvent, ctx: &mut ViewContext<Self>) {
        match event {
            CloudModelEvent::ObjectMoved { type_and_id, .. }
            | CloudModelEvent::ObjectPermissionsUpdated { type_and_id, .. } => {
                // Update breadcrumbs if a teammate has moved the workflow elsewhere, or if it's
                // been shared.
                if let Some(workflow_id) = self.workflow_id {
                    // Check that it's the currently active/open workflow
                    if *type_and_id
                        == CloudObjectTypeAndId::from_id_and_type(workflow_id, ObjectType::Workflow)
                    {
                        self.compute_breadcrumbs(ctx);
                    }
                }
            }
            CloudModelEvent::NotebookEditorChangedFromServer { .. }
            | CloudModelEvent::ObjectUpdated { .. }
            | CloudModelEvent::ObjectTrashed { .. }
            | CloudModelEvent::ObjectUntrashed { .. }
            | CloudModelEvent::ObjectCreated { .. }
            | CloudModelEvent::ObjectDeleted { .. }
            | CloudModelEvent::ObjectForceExpanded { .. }
            | CloudModelEvent::ObjectSynced { .. }
            | CloudModelEvent::InitialLoadCompleted => {}
        }
    }

    fn handle_content_editor_event(&mut self, event: &EditorEvent, ctx: &mut ViewContext<Self>) {
        match event {
            EditorEvent::Edited(_) => {
                let current_content = self
                    .content_editor
                    .read(ctx, |editor, ctx| editor.buffer_text(ctx));

                self.errors.content_empty_error = current_content.trim().is_empty();

                self.arguments_state = ArgumentsState::for_command_workflow(
                    &self.arguments_state,
                    current_content.clone(),
                );
                self.update_arguments_rows(ctx);

                self.clear_content_formatting(current_content.chars().count(), ctx);
                self.apply_error_underlining_to_content(ctx);
                self.apply_argument_highlighting_to_content(ctx);

                self.errors.invalid_argument_error = !self
                    .arguments_state
                    .invalid_arguments_char_ranges
                    .is_empty();

                ctx.notify();
            }
            // when the editor supports tab completions, we'll need to change this logic
            EditorEvent::Navigate(NavigationKey::Tab) => match self.arguments_rows.first() {
                Some(row) => ctx.focus(&row.description_editor),
                None => ctx.focus(&self.title_editor),
            },
            EditorEvent::Navigate(NavigationKey::ShiftTab) => ctx.focus(&self.description_editor),
            EditorEvent::Navigate(NavigationKey::Up) => self
                .content_editor
                .update(ctx, |input, ctx| input.move_up(ctx)),
            EditorEvent::Navigate(NavigationKey::Down) => self
                .content_editor
                .update(ctx, |input, ctx| input.move_down(ctx)),
            EditorEvent::Escape => self.close(false, ctx),
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
                    ctx.focus(&self.arguments_rows[index].typed_default_value_editor);
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
                    .position(|row| row.typed_default_value_editor.eq(&handle))
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
                    .position(|row| row.typed_default_value_editor.eq(&handle))
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
                    if !row.typed_default_value_editor.eq(&handle) {
                        row.typed_default_value_editor.update(ctx, |editor, ctx| {
                            editor.close(ctx);
                        });
                    }
                });
            }
            WorkflowArgSelectorEvent::InputTab => {
                if let Some(index) = self
                    .arguments_rows
                    .iter()
                    .position(|row| row.typed_default_value_editor.eq(&handle))
                {
                    match self.arguments_rows.get(index + 1) {
                        Some(next_row) => ctx.focus(&next_row.description_editor),
                        None => ctx.focus(&self.title_editor),
                    }
                }
            }
            WorkflowArgSelectorEvent::InputShiftTab => {
                if let Some(row) = self
                    .arguments_rows
                    .iter()
                    .find(|row| row.typed_default_value_editor.eq(&handle))
                {
                    ctx.focus(&row.description_editor);
                }
            }
        }
    }

    fn handle_argument_editor_event(
        &mut self,
        handle: ViewHandle<EditorView>,
        event: &EditorEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            // because the number of editor views we have depends on how many arguments
            // are in the content, tabbing/shift-tabbing is slightly complex.
            // `handle_argument_editor_event` is used for all of these views, so broadly
            // speaking there are two steps for each interaction:
            // 1. iterate through every row, looking for which editor fired this event
            // 2. decide what editor to focus next based on what editor's ahead/behind us
            EditorEvent::Navigate(NavigationKey::Tab) => {
                self.arguments_rows
                    .iter()
                    .enumerate()
                    .for_each(|(index, row)| {
                        // tabbing in a description editor just means we focus
                        // the corresponding default value editor
                        if row.description_editor.eq(&handle) {
                            ctx.focus(&row.typed_default_value_editor);
                        } else if row.default_value_editor.eq(&handle) {
                            // if we have another row ahead of us, tabbing in the default
                            // value editor moves to the following row's description editor.
                            // otherwise, it wraps around to the title.
                            match self.arguments_rows.get(index + 1) {
                                Some(next_row) => ctx.focus(&next_row.description_editor),
                                None => ctx.focus(&self.title_editor),
                            }
                        }
                    });
            }
            EditorEvent::Navigate(NavigationKey::ShiftTab) => {
                self.arguments_rows
                    .iter()
                    .enumerate()
                    .for_each(|(index, row)| {
                        // if we have another row behind us, shift-tabbing in the description
                        // editor moves to the previous row's default value editor.
                        // otherwise, it focuses the content editor.
                        if row.description_editor.eq(&handle) {
                            if index == 0 {
                                ctx.focus(&self.content_editor);
                            } else {
                                ctx.focus(
                                    &self.arguments_rows[index - 1].typed_default_value_editor,
                                );
                            }
                        // shift-tabbing in a default value editor just means we
                        // focus the corresponding default value editor
                        } else if row.default_value_editor.eq(&handle) {
                            ctx.focus(&row.description_editor);
                        }
                    });
            }
            EditorEvent::Escape => self.close(false, ctx),
            EditorEvent::Edited(_) => ctx.notify(),
            _ => {}
        }
    }

    /// Updates our arguments rows state based on the current state of the ArgumentsState struct.
    /// There are two things that can happen:
    /// 1) The new struct reports **the same number** of arguments as we have rows. In this case,
    ///    we match up the arguments and rows based on their positions in their respective arrays,
    ///    updating the names of the rows. This lets a user change the name of an argument without
    ///    losing their default value / description content.
    /// 2) The new struct reports either **more** or **fewer** arguments than we have rows. In this
    ///    case, we toss out any rows that refer to arguments' names no longer in the arg state,
    ///    and then add rows in between matches we DO find. This handles adding/removing args at
    ///    any position in the content, preserving arg values that haven't changed.
    pub(super) fn update_arguments_rows(&mut self, ctx: &mut ViewContext<Self>) {
        let appearance = Appearance::as_ref(ctx);
        let ui_font_family = appearance.ui_font_family();

        match self
            .arguments_rows
            .len()
            .cmp(&self.arguments_state.arguments.len())
        {
            Ordering::Equal => {
                self.arguments_state
                    .arguments
                    .iter()
                    .enumerate()
                    .for_each(|(index, argument)| {
                        self.arguments_rows[index].name.clone_from(&argument.name);
                    });
            }
            Ordering::Less | Ordering::Greater => {
                // first, get rid of all rows that have names not present in the updated args state
                let argument_names = self
                    .arguments_state
                    .arguments
                    .iter()
                    .map(|argument| argument.name.clone())
                    .collect::<Vec<_>>();
                self.arguments_rows
                    .retain(|row| argument_names.contains(&row.name));

                // next, go over each item in the args state, and either add a row at this position,
                // or skip over it if we've found a match
                self.arguments_state
                    .arguments
                    .iter()
                    .enumerate()
                    .for_each(|(index, argument)| {
                        // if we reach the end of the state struct and we still
                        // haven't inserted a row OR we find a mismatched name,
                        // we know to add a row at this particular index
                        if index == self.arguments_rows.len()
                            || !argument.name.eq(&self.arguments_rows[index].name)
                        {
                            let description_editor = Self::create_editor_handle(
                                ctx,
                                Some(ARGUMENT_EDITOR_FONT_SIZE),
                                Some(ui_font_family),
                                Some(ARGUMENT_DESCRIPTION_PLACEHOLDER_TEXT),
                                false, /* vim_keybindings */
                                false,
                            );

                            ctx.subscribe_to_view(
                                &description_editor,
                                |me, emitter, event, ctx| {
                                    me.handle_argument_editor_event(emitter, event, ctx);
                                },
                            );

                            let default_value_editor = Self::create_editor_handle(
                                ctx,
                                Some(ARGUMENT_EDITOR_FONT_SIZE),
                                Some(ui_font_family),
                                Some(ARGUMENT_DEFAULT_VALUE_PLACEHOLDER_TEXT),
                                false, /* vim_keybindings */
                                false,
                            );

                            ctx.subscribe_to_view(
                                &default_value_editor,
                                |me, emitter, event, ctx| {
                                    me.handle_argument_editor_event(emitter, event, ctx);
                                },
                            );

                            let typed_default_value_editor = ctx.add_typed_action_view(|ctx| {
                                WorkflowArgSelector::new(
                                    WorkflowArgSelectorStyles {
                                        editor_padding: Coords::uniform(ARGUMENT_INPUT_PADDING),
                                        width: Some(ARGUMENT_INPUT_WIDTH),
                                        height: None,
                                        dropdown_background: |appearance| {
                                            appearance.theme().background()
                                        },
                                        border_color: |appearance| appearance.theme().outline(),
                                        border_radius: 0.0,
                                    },
                                    &self.all_workflow_enums,
                                    ctx,
                                )
                            });

                            ctx.subscribe_to_view(
                                &typed_default_value_editor,
                                |me, emitter, event, ctx| {
                                    me.handle_type_selector_event(emitter, event, ctx);
                                },
                            );

                            self.arguments_rows.insert(
                                index,
                                ArgumentEditorRow {
                                    name: argument.name.clone(),
                                    description_editor,
                                    default_value_editor,
                                    typed_default_value_editor,
                                },
                            );
                        }
                    });
            }
        }
    }

    fn clear_content_formatting(&mut self, num_chars_content: usize, ctx: &mut ViewContext<Self>) {
        self.content_editor.update(ctx, |editor, ctx| {
            editor.update_buffer_styles(
                vec![CharOffset::from(0)..CharOffset::from(num_chars_content)],
                TextStyleOperation::default().clear_error_underline_color(),
                ctx,
            );

            editor.update_buffer_styles(
                vec![CharOffset::from(0)..CharOffset::from(num_chars_content)],
                TextStyleOperation::default().clear_foreground_color(),
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

    fn is_new_argument_button_disabled(&self) -> bool {
        self.show_unsaved_changes_dialog
            || matches!(
                self.ai_metadata_assist_state,
                AiAssistState::RequestInFlight
            )
    }

    fn is_save_workflow_button_disabled(&self) -> bool {
        self.show_unsaved_changes_dialog
            || self.errors.has_any_error()
            || matches!(
                self.ai_metadata_assist_state,
                AiAssistState::RequestInFlight
            )
            || self.show_enum_creation_dialog
    }

    fn is_ai_assist_button_disabled(&self, app: &AppContext) -> bool {
        // Autofill button should be disabled when there is no content.
        self.content_editor.as_ref(app).is_empty(app)
            || self.show_unsaved_changes_dialog
            || matches!(
                self.ai_metadata_assist_state,
                AiAssistState::RequestInFlight
            )
    }

    fn is_online(&self, app: &AppContext) -> bool {
        NetworkStatus::as_ref(app).is_online()
    }

    fn render_header_menu_and_close(&self, appearance: &Appearance) -> Box<dyn Element> {
        let mut row = Flex::row();

        // The menu should not appear when we are creating a workflow
        if self.workflow_id.is_some() {
            let overflow_menu = ConstrainedBox::new(
                icon_button_with_context_menu(
                    Icon::DotsVertical,
                    move |ctx, _, _| {
                        ctx.dispatch_typed_action(WorkflowModalAction::OpenOverflowMenu);
                    },
                    self.button_mouse_states.menu_state.clone(),
                    &self.menu,
                    self.menu_open,
                    MenuDirection::Left,
                    Some(Cursor::PointingHand),
                    None,
                    appearance,
                )
                .finish(),
            )
            .with_height(ICON_DIMENSIONS)
            .finish();
            row.add_child(overflow_menu);
        }

        let close_button = icon_button(
            appearance,
            icons::Icon::X,
            false, /* active */
            self.button_mouse_states.close_modal_state.clone(),
        )
        .build()
        .on_click(|ctx, _, _| ctx.dispatch_typed_action(WorkflowModalAction::Close))
        .finish();
        row.add_child(close_button);

        row.finish()
    }

    fn render_header(&self, appearance: &Appearance) -> Box<dyn Element> {
        let workflow_icon = Container::new(
            ConstrainedBox::new(
                Icon::from(DriveObjectType::Workflow)
                    .to_warpui_icon(
                        warp_drive_icon_color(appearance, DriveObjectType::Workflow).into(),
                    )
                    .finish(),
            )
            .with_width(ICON_DIMENSIONS)
            .with_height(ICON_DIMENSIONS)
            .finish(),
        )
        .finish();

        let workflow_title_description = Shrinkable::new(
            1.,
            Container::new(
                Flex::column()
                    .with_child(
                        Container::new(ChildView::new(&self.title_editor).finish()).finish(),
                    )
                    .with_child(
                        Container::new(
                            ConstrainedBox::new(ChildView::new(&self.description_editor).finish())
                                .with_max_height(DESCRIPTION_EDITOR_MAX_HEIGHT)
                                .finish(),
                        )
                        .with_padding_top(DESCRIPTION_EDITOR_TOP_PADDING)
                        .finish(),
                    )
                    .finish(),
            )
            .with_padding_left(MODAL_HORIZONTAL_PADDING)
            .with_padding_right(MODAL_HORIZONTAL_PADDING)
            .finish(),
        )
        .finish();

        // Case 1: Has breadcrumbs, so modal header =
        // first row = breadcrumbs on left side, overflow menu + close button on right side
        // second row = workflow icon + title/description
        if let Some(breadcrumbs) = &self.breadcrumbs {
            let rendered_breadcrumbs = breadcrumb::render_breadcrumbs(
                breadcrumbs.clone(),
                appearance,
                |ctx, _, object| {
                    let item_id = match object.kind {
                        ContainingObjectKind::Object(id) => WarpDriveItemId::Object(id),
                        ContainingObjectKind::Space(space) => WarpDriveItemId::Space(space),
                    };
                    ctx.dispatch_typed_action(WorkflowModalAction::ViewInWarpDrive(item_id));
                },
            );

            Container::new(
                Flex::column()
                    .with_child(
                        Container::new(
                            Flex::row()
                                .with_main_axis_size(MainAxisSize::Max)
                                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                                .with_child(Shrinkable::new(1., rendered_breadcrumbs).finish())
                                .with_child(self.render_header_menu_and_close(appearance))
                                .finish(),
                        )
                        .with_vertical_margin(BREADCRUMBS_VERTICAL_MARGIN)
                        .finish(),
                    )
                    .with_child(
                        Flex::row()
                            .with_child(workflow_icon)
                            .with_child(workflow_title_description)
                            .finish(),
                    )
                    .finish(),
            )
            .with_padding_left(MODAL_HORIZONTAL_PADDING)
            .with_padding_right(MODAL_HORIZONTAL_PADDING)
            .with_padding_top(MODAL_VERTICAL_PADDING)
            .with_padding_bottom(MODAL_VERTICAL_PADDING)
            .finish()
        }
        // Case 2: Creating a new workflow has no menu and breadcrumbs, so modal header =
        // workflow icon + title + close button on first row
        else {
            Container::new(
                Flex::row()
                    .with_child(workflow_icon)
                    .with_child(workflow_title_description)
                    .with_child(self.render_header_menu_and_close(appearance))
                    .finish(),
            )
            .with_padding_left(MODAL_HORIZONTAL_PADDING)
            .with_padding_right(MODAL_HORIZONTAL_PADDING)
            .with_padding_top(MODAL_VERTICAL_PADDING)
            .with_padding_bottom(MODAL_VERTICAL_PADDING)
            .finish()
        }
    }

    fn render_content_editor(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        let line_height = self
            .content_editor
            .as_ref(app)
            .line_height(app.font_cache(), appearance);

        Container::new(
            ConstrainedBox::new(ChildView::new(&self.content_editor).finish())
                .with_min_height(COMMAND_EDITOR_MIN_LINES * line_height)
                .finish(),
        )
        .with_uniform_padding(COMMAND_EDITOR_PADDING)
        .with_background(appearance.theme().background())
        .finish()
    }

    fn render_arguments_editors(&self, appearance: &Appearance) -> Box<dyn Element> {
        let children: Vec<Box<dyn Element>> = self
            .arguments_state
            .arguments
            .iter()
            .enumerate()
            .map(|(index, argument)| {
                let description_handle = &self.arguments_rows[index].description_editor;
                let text = appearance
                    .ui_builder()
                    .span(argument.clone().name)
                    .with_style(UiComponentStyles {
                        font_family_id: Some(appearance.monospace_font_family()),
                        ..Default::default()
                    })
                    .build()
                    .finish();

                let new_default_value_handle =
                    &self.arguments_rows[index].typed_default_value_editor;

                Flex::row()
                    .with_child(Shrinkable::new(1., Align::new(text).left().finish()).finish())
                    .with_child(
                        Container::new(
                            ConstrainedBox::new(ChildView::new(description_handle).finish())
                                .with_width(ARGUMENT_INPUT_WIDTH)
                                .finish(),
                        )
                        .with_background(appearance.theme().background())
                        .with_border(Border::all(1.).with_border_fill(appearance.theme().outline()))
                        .with_uniform_padding(ARGUMENT_INPUT_PADDING)
                        .with_margin_left(ARGUMENT_INPUT_MARGIN)
                        .with_margin_right(ARGUMENT_INPUT_MARGIN)
                        .finish(),
                    )
                    .with_child(
                        Container::new(ChildView::new(new_default_value_handle).finish()).finish(),
                    )
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .finish()
            })
            .collect();

        let theme = appearance.theme();
        Container::new(
            ClippedScrollable::vertical(
                self.arguments_clipped_scroll_state.clone(),
                Flex::column()
                    .with_main_axis_size(MainAxisSize::Min)
                    .with_children(children)
                    .finish(),
                SCROLLBAR_WIDTH,
                theme.background().into(),
                theme.main_text_color(theme.background()).into(),
                theme.surface_1().into(),
            )
            .finish(),
        )
        .with_uniform_padding(COMMAND_EDITOR_PADDING)
        .finish()
    }

    fn render_footer(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        let default_button_styles = UiComponentStyles {
            font_size: Some(BUTTON_FONT_SIZE),
            font_family_id: Some(appearance.ui_font_family()),
            font_color: Some(
                appearance
                    .theme()
                    .main_text_color(appearance.theme().background())
                    .into(),
            ),
            font_weight: Some(Weight::Bold),
            border_radius: Some(CornerRadius::with_all(Radius::Pixels(BUTTON_BORDER_RADIUS))),
            border_color: Some(appearance.theme().outline().into()),
            border_width: Some(BORDER_WIDTH),
            padding: Some(Coords::uniform(BUTTON_PADDING)),
            background: Some(appearance.theme().surface_1().into()),
            ..Default::default()
        };

        let hovered_and_clicked_styles = UiComponentStyles {
            background: Some(appearance.theme().surface_3().into()),
            ..default_button_styles
        };

        let primary_button_styles = UiComponentStyles {
            background: Some(appearance.theme().accent_button_color().into()),
            border_color: Some(appearance.theme().accent_button_color().into()),
            font_color: Some(
                appearance
                    .theme()
                    .main_text_color(appearance.theme().accent_button_color())
                    .into(),
            ),
            ..default_button_styles
        };

        let primary_disabled_styles = UiComponentStyles {
            background: Some(appearance.theme().surface_3().into()),
            border_color: Some(appearance.theme().surface_3().into()),
            font_color: Some(
                appearance
                    .theme()
                    .disabled_text_color(appearance.theme().background())
                    .into(),
            ),
            ..primary_button_styles
        };

        let primary_hovered_and_clicked_styles = UiComponentStyles {
            background: Some(blended_colors::accent_hover(appearance.theme()).into()),
            border_color: Some(blended_colors::accent_hover(appearance.theme()).into()),
            ..primary_button_styles
        };

        let mut new_argument_button = appearance
            .ui_builder()
            .button(
                ButtonVariant::Secondary,
                self.button_mouse_states.new_argument_state.clone(),
            )
            .with_style(UiComponentStyles {
                font_size: Some(BUTTON_FONT_SIZE),
                font_weight: Some(Weight::Bold),
                padding: Some(Coords::uniform(BUTTON_PADDING)),
                ..Default::default()
            })
            .with_text_label(ARGUMENT_BUTTON_TEXT.into());

        if self.is_new_argument_button_disabled() {
            new_argument_button = new_argument_button.disabled();
        }

        let mut save_button = appearance
            .ui_builder()
            .button_with_custom_styles(
                ButtonVariant::Basic,
                self.button_mouse_states.save_workflow_state.clone(),
                primary_button_styles,
                Some(primary_hovered_and_clicked_styles),
                Some(primary_hovered_and_clicked_styles),
                Some(primary_disabled_styles),
            )
            .with_text_label(SAVE_BUTTON_TEXT.into());

        if self.is_save_workflow_button_disabled() {
            save_button = save_button.disabled();
        }

        let render_save_button = save_button
            .build()
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, _| ctx.dispatch_typed_action(WorkflowModalAction::Save))
            .finish();

        let mut button_row = Flex::row()
            .with_child(
                Shrinkable::new(
                    1.,
                    new_argument_button
                        .build()
                        .with_cursor(Cursor::PointingHand)
                        .on_click(move |ctx, _, _| {
                            ctx.dispatch_typed_action(WorkflowModalAction::AddArgument)
                        })
                        .finish(),
                )
                .finish(),
            )
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween);

        let label_and_icon = match self.ai_metadata_assist_state {
            AiAssistState::PreRequest => Some((AI_ASSIST_BUTTON_TEXT, Icon::AiAssistant)),
            AiAssistState::RequestInFlight => Some((AI_ASSIST_LOADING_TEXT, Icon::Refresh)),
            AiAssistState::Generated => None,
        };

        if let Some((label, icon)) = label_and_icon {
            let text_and_icon = TextAndIcon::new(
                TextAndIconAlignment::TextFirst,
                label.to_string(),
                icon.to_warpui_icon(appearance.theme().active_ui_text_color()),
                MainAxisSize::Min,
                MainAxisAlignment::Center,
                vec2f(16., 16.),
            )
            .with_inner_padding(4.);

            let mut button = appearance
                .ui_builder()
                .button_with_custom_styles(
                    ButtonVariant::Basic,
                    self.button_mouse_states.ai_assist_state.clone(),
                    default_button_styles.set_width(AI_ASSIST_BUTTON_SIZE),
                    Some(hovered_and_clicked_styles.set_width(AI_ASSIST_BUTTON_SIZE)),
                    Some(hovered_and_clicked_styles.set_width(AI_ASSIST_BUTTON_SIZE)),
                    Some(primary_disabled_styles.set_width(AI_ASSIST_BUTTON_SIZE)),
                )
                .with_text_and_icon_label(text_and_icon);

            if self.is_ai_assist_button_disabled(app) {
                button = button.disabled();
            }

            let rendered_button = button
                .build()
                .with_cursor(Cursor::PointingHand)
                .on_click(move |ctx, _, _| ctx.dispatch_typed_action(WorkflowModalAction::AiAssist))
                .finish();

            let button_with_tool_tip = appearance.ui_builder().tool_tip_on_element(
                "Generate a title, descriptions, or parameters with Warp AI".to_string(),
                self.button_mouse_states.ai_assist_tool_tip.clone(),
                rendered_button,
                ParentAnchor::BottomMiddle,
                ChildAnchor::TopMiddle,
                vec2f(0., 5.),
            );

            button_row.add_child(
                Flex::row()
                    .with_child(
                        Container::new(button_with_tool_tip)
                            .with_margin_right(8.)
                            .finish(),
                    )
                    .with_child(render_save_button)
                    .finish(),
            )
        } else {
            button_row.add_child(render_save_button);
        }

        Container::new(button_row.finish())
            .with_padding_left(MODAL_HORIZONTAL_PADDING)
            .with_padding_right(MODAL_HORIZONTAL_PADDING)
            .with_padding_top(MODAL_VERTICAL_PADDING)
            .with_padding_bottom(MODAL_VERTICAL_PADDING)
            .finish()
    }

    fn render_unsaved_changes_dialog(&self, appearance: &Appearance) -> Box<dyn Element> {
        let keep_editing_button = appearance
            .ui_builder()
            .button(
                ButtonVariant::Secondary,
                self.button_mouse_states.keep_editing_state.clone(),
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
                ctx.dispatch_typed_action(WorkflowModalAction::CloseUnsavedChangesDialog)
            })
            .finish();

        let discard_changes_button = appearance
            .ui_builder()
            .button(
                ButtonVariant::Secondary,
                self.button_mouse_states.discard_changes_state.clone(),
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
            .on_click(move |ctx, _, _| ctx.dispatch_typed_action(WorkflowModalAction::ForceClose))
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
}

impl Entity for WorkflowModal {
    type Event = WorkflowModalEvent;
}

impl View for WorkflowModal {
    fn ui_name() -> &'static str {
        "WorkflowModal"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let modal = Container::new(
            ConstrainedBox::new(
                Flex::column()
                    .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .with_child(self.render_header(appearance))
                    .with_child(
                        Shrinkable::new(1., self.render_content_editor(appearance, app)).finish(),
                    )
                    .with_child(
                        Shrinkable::new(1., self.render_arguments_editors(appearance)).finish(),
                    )
                    .with_child(self.render_footer(appearance, app))
                    .finish(),
            )
            .with_max_width(MODAL_WIDTH)
            .finish(),
        )
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(MODAL_BORDER_RADIUS)))
        .with_border(Border::all(BORDER_WIDTH).with_border_fill(theme.outline()))
        .with_background(theme.surface_1())
        .with_margin_left(MODAL_HORIZONTAL_MARGIN)
        .with_margin_right(MODAL_HORIZONTAL_MARGIN)
        .with_margin_top(MODAL_VERTICAL_MARGIN)
        .with_margin_bottom(MODAL_VERTICAL_MARGIN)
        .finish();

        let mut stack = Stack::new();
        stack.add_positioned_child(
            modal,
            OffsetPositioning::offset_from_parent(
                vec2f(0., 0.),
                ParentOffsetBounds::WindowByPosition,
                ParentAnchor::Center,
                ChildAnchor::Center,
            ),
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

        if self.show_unsaved_changes_dialog {
            stack.add_positioned_overlay_child(
                self.render_unsaved_changes_dialog(appearance),
                OffsetPositioning::offset_from_parent(
                    vec2f(0., 0.),
                    ParentOffsetBounds::WindowByPosition,
                    ParentAnchor::Center,
                    ChildAnchor::Center,
                ),
            )
        }

        Container::new(Align::new(stack.finish()).finish())
            .with_background_color(Fill::blur().into())
            .with_corner_radius(app.windows().window_corner_radius())
            .finish()
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            ctx.focus(&self.title_editor);
            ctx.notify();
        }
    }
}

impl TypedActionView for WorkflowModal {
    type Action = WorkflowModalAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            WorkflowModalAction::AddArgument => self.add_argument(ctx),
            WorkflowModalAction::Close => self.close(false, ctx),
            WorkflowModalAction::Save => self.save_workflow_and_close(ctx),
            WorkflowModalAction::CloseUnsavedChangesDialog => self.hide_unsaved_changes_dialog(ctx),
            WorkflowModalAction::ForceClose => {
                self.close(true, ctx);
                if let Some(id) = self.clicked_breadcrumb {
                    self.view_in_warp_drive(id, ctx);
                }
            }
            WorkflowModalAction::AiAssist => self.issue_request(ctx),
            WorkflowModalAction::ViewInWarpDrive(id) => {
                if self.should_show_unsaved_changes_dialog(ctx) {
                    self.clicked_breadcrumb = Some(*id);
                    self.show_unsaved_changes_dialog(ctx);
                    return;
                }
                self.view_in_warp_drive(*id, ctx)
            }
            WorkflowModalAction::OpenOverflowMenu => self.open_overflow_menu(ctx),
            WorkflowModalAction::CopyObjectToClipboard => self.copy_object_to_clipboard(ctx),
            WorkflowModalAction::TrashObject => self.trash_object(ctx),
        }
    }
}

#[cfg(test)]
#[path = "modal_tests.rs"]
mod tests;
