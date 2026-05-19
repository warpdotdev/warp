use std::rc::Rc;

use strum::IntoEnumIterator;
use strum_macros::{EnumIter, IntoStaticStr};
use warp_core::ui::appearance::Appearance;
use warp_editor::editor::NavigationKey;
use warpui::{
    elements::{
        Border, ClippedScrollStateHandle, ClippedScrollable, ConstrainedBox, Container,
        CornerRadius, CrossAxisAlignment, Fill, Flex, MainAxisAlignment, MainAxisSize,
        MouseStateHandle, ParentElement, Radius, ScrollbarWidth, Shrinkable,
    },
    ui_components::{
        button::ButtonVariant,
        components::{UiComponent, UiComponentStyles},
        toggle_menu::{ToggleMenuItem, ToggleMenuStateHandle},
    },
    AppContext, Element, Entity, FocusContext, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle,
};

use crate::{
    cloud_object::{model::persistence::CloudModel, Revision},
    editor::{
        EditorOptions, EditorView, Event, PropagateAndNoOpNavigationKeys, SingleLineEditorOptions,
        TextOptions,
    },
    server::ids::{ClientId, SyncId},
    ui_components::{buttons::icon_button, icons::Icon},
    workflows::workflow_enum::EnumVariants,
};

const CONTAINER_PADDING: f32 = 16.;
const CORE_WIDTH: f32 = 400.;
const CORE_HEIGHT: f32 = 250.;
const ELEMENT_SPACING: f32 = 10.;
const OFFSET_FOR_SCROLLBAR: f32 = 12.;
const ROW_MARGIN: f32 = 8.;
const ROW_SPACING: f32 = 4.;

const SECTION_SPACING: f32 = 24.;
const VARIANT_EDITOR_HEIGHT: f32 = 40.;
const COMMAND_EDITOR_HEIGHT: f32 = 120.;

const SCROLLBAR_WIDTH: ScrollbarWidth = ScrollbarWidth::Auto;

const BUTTON_FONT_SIZE: f32 = 14.;
const EDITOR_FONT_SIZE: f32 = 14.;
const SECTION_FONT_SIZE: f32 = 16.;
const SPAN_FONT_SIZE: f32 = 16.;
const VARIANT_FONT_SIZE: f32 = 13.;

const CANCEL_BUTTON_LABEL: &str = "Close";
const NEW_ENUM_SPAN: &str = "New enum";
const EXISTING_ENUM_SPAN: &str = "Edit enum";
const NAME_PLACEHOLDER_TEXT: &str = "Name";
const CREATE_BUTTON_LABEL: &str = "Create";
const SAVE_BUTTON_LABEL: &str = "Save";
const VARIANT_PLACEHOLDER_TEXT: &str = "Variant";
const STATIC_LABEL_TEXT: &str = "Variants";
const DYNAMIC_PLACEHOLDER_TEXT: &str =
    "# Enter a shell command that generates variants, delimited by newlines.\n\ngit branch -a";

#[derive(Debug, Clone)]
pub enum EnumCreationDialogAction {
    Close,
    SaveEnum,
    AddVariant,
    DeleteVariant(VariantRowIndex),
}

#[derive(Debug, Clone)]
pub enum EnumCreationDialogEvent {
    Close,
    /// Create a new enum, with the `WorkflowEnumData` included
    CreateEnum(WorkflowEnumData),
    /// Edit the enum with this ID in the list of enums stored, with the `WorkflowEnumData`
    /// The boolean value represents if the visibility of the enum changed (went from unshared to shared
    /// or vice versa), which is used when updating the selector states.
    EditEnum(WorkflowEnumData, bool),
}

/// Struct for holding workflow enum data associated with this argument
#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowEnumData {
    /// Every enum argument will have an id and a name
    pub id: SyncId,
    pub name: String,
    /// If the enum is shared or not, used when determining if an enum should be displayed in the dropdown
    pub is_shared: bool,
    /// The revision_ts of the enum, None if it has not yet been created.
    pub revision_ts: Option<Revision>,
    /// This field contains any new enum data that has not been saved,
    /// i.e. created enums or updated enums.
    pub new_data: Option<EnumVariants>,
}

#[derive(Debug, Clone)]
pub struct VariantRowIndex(usize);

pub struct VariantEditorRow {
    variant_editor: ViewHandle<EditorView>,
    delete_row_mouse_state_handle: MouseStateHandle,
}
#[derive(Default)]
struct MouseStateHandles {
    cancel_button_mouse_state_handle: MouseStateHandle,
    save_button_mouse_state_handle: MouseStateHandle,
    add_variant_state: MouseStateHandle,
}

pub struct EnumCreationDialog {
    variants_clipped_scroll_state: ClippedScrollStateHandle,
    mouse_state_handles: MouseStateHandles,
    name_editor: ViewHandle<EditorView>,
    variant_rows: Vec<VariantEditorRow>,
    /// The `sync_id` of the enum in the dialog if it already exists,
    /// `None` if this is a new enum.
    sync_id: Option<SyncId>,
    /// The revision timestamp of the enum, if it has been loaded in from the server.
    revision_ts: Option<Revision>,

    /// Store the base state of the enum dialog, used for determining if the dialog is dirty
    base_dialog_state: BaseEnumDialogState,

    // The handles and options used for the type toggle menu
    enum_type_handles: EnumTypeHandles,
    enum_type_options: Vec<EnumType>,
    dynamic_command_editor: ViewHandle<EditorView>,
}

#[derive(Debug, Default, PartialEq)]
struct BaseEnumDialogState {
    /// Store the number of rows to determine if a variant was removed
    variant_rows: usize,
    is_enum_shared: bool,
    selected_type: EnumType,
}

#[derive(Default, Clone)]
struct EnumTypeHandles {
    enum_type_state_handle: ToggleMenuStateHandle,
    enum_type_mouse_states: Vec<MouseStateHandle>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, IntoStaticStr, EnumIter)]
enum EnumType {
    #[default]
    Static,
    Dynamic,
}

impl EnumCreationDialog {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let name_editor = {
            ctx.add_typed_action_view(|ctx| {
                let appearance = Appearance::as_ref(ctx);
                let options = SingleLineEditorOptions {
                    text: TextOptions::ui_text(Some(EDITOR_FONT_SIZE), appearance),
                    propagate_and_no_op_vertical_navigation_keys:
                        PropagateAndNoOpNavigationKeys::Always,
                    ..Default::default()
                };

                let mut editor = EditorView::single_line(options, ctx);
                editor.set_placeholder_text(NAME_PLACEHOLDER_TEXT, ctx);
                editor
            })
        };

        ctx.subscribe_to_view(&name_editor, |me, _, event, ctx| {
            me.handle_name_editor_event(event, ctx);
        });

        let enum_type_handles = EnumTypeHandles {
            // We need one mouse state for each enum type.
            enum_type_mouse_states: vec![Default::default(), Default::default()],
            ..Default::default()
        };
        let enum_type_options = EnumType::iter().collect();
        let dynamic_command_editor = {
            ctx.add_typed_action_view(|ctx| {
                let appearance = Appearance::as_ref(ctx);
                let text = TextOptions {
                    font_size_override: Some(EDITOR_FONT_SIZE),
                    font_family_override: Some(appearance.monospace_font_family()),
                    ..Default::default()
                };
                let options = EditorOptions {
                    text,
                    propagate_and_no_op_vertical_navigation_keys:
                        PropagateAndNoOpNavigationKeys::Always,
                    soft_wrap: true,
                    placeholder_soft_wrap: true,
                    ..Default::default()
                };

                let mut editor = EditorView::new(options, ctx);
                editor.set_placeholder_text(DYNAMIC_PLACEHOLDER_TEXT, ctx);
                editor.set_autogrow(true);
                editor
            })
        };
        ctx.subscribe_to_view(&dynamic_command_editor, |me, _, event, ctx| {
            me.handle_command_editor_event(event, ctx);
        });

        Self {
            mouse_state_handles: Default::default(),
            variants_clipped_scroll_state: Default::default(),
            name_editor,
            variant_rows: Vec::new(),
            sync_id: None,
            base_dialog_state: Default::default(),
            revision_ts: None,
            enum_type_handles,
            enum_type_options,
            dynamic_command_editor,
        }
    }

    // This function gets called when we are creating an enum from scratch
    pub fn initialize(&mut self, ctx: &mut ViewContext<Self>) {
        self.add_variant_row(ctx);
        self.base_dialog_state = BaseEnumDialogState {
            variant_rows: 1,
            is_enum_shared: false,
            selected_type: EnumType::Static,
        }
    }

    // Internal function used to load an enum into the editor
    fn load_enum(
        &mut self,
        name: &str,
        enum_id: SyncId,
        is_shared: bool,
        variants: &EnumVariants,
        ctx: &mut ViewContext<Self>,
    ) {
        // Set the stored id to be the id of the existing enum
        self.sync_id = Some(enum_id);

        // Populate the dialog with the enum name and variants
        self.name_editor.update(ctx, |buffer, ctx| {
            buffer.set_buffer_text_with_base_buffer(name, ctx)
        });

        let base_selected_type = match variants {
            EnumVariants::Static(variants) => {
                self.set_selected_type(EnumType::Static, ctx);
                variants.iter().for_each(|variant| {
                    self.add_variant_row(ctx);
                    self.variant_rows[self.variant_rows.len() - 1]
                        .variant_editor
                        .update(ctx, |editor, ctx| {
                            editor.set_buffer_text_with_base_buffer(variant.as_str(), ctx);
                        })
                });
                EnumType::Static
            }
            EnumVariants::Dynamic(command) => {
                // We add a static variant row to set up the default state for
                // if the user switches to a static enum.
                self.add_variant_row(ctx);
                self.set_selected_type(EnumType::Dynamic, ctx);
                self.dynamic_command_editor.update(ctx, |editor, ctx| {
                    editor.set_buffer_text_with_base_buffer(command.as_str(), ctx);
                });
                EnumType::Dynamic
            }
        };

        self.base_dialog_state = BaseEnumDialogState {
            variant_rows: self.variant_rows.len(),
            is_enum_shared: is_shared,
            selected_type: base_selected_type,
        };
    }

    // Load an enum given its variants
    pub fn load_from_data(
        &mut self,
        name: &str,
        enum_id: SyncId,
        is_shared: bool,
        enum_data: &EnumVariants,
        ctx: &mut ViewContext<Self>,
    ) {
        self.load_enum(name, enum_id, is_shared, enum_data, ctx);
    }

    // Load an enum from memory
    pub fn load_from_cloud_model(&mut self, enum_id: SyncId, ctx: &mut ViewContext<Self>) {
        let cloud_model = CloudModel::as_ref(ctx);
        let workflow_enum_model = cloud_model.get_workflow_enum(&enum_id);

        self.revision_ts = workflow_enum_model.and_then(|model| model.metadata.revision.clone());

        let workflow_enum =
            workflow_enum_model.map(|workflow_enum| workflow_enum.model().string_model.clone());

        if let Some(workflow_enum) = workflow_enum {
            self.load_enum(
                &workflow_enum.name,
                enum_id,
                workflow_enum.is_shared,
                &workflow_enum.variants,
                ctx,
            );
        } else {
            // If we couldn't find an enum with this SyncId, open an empty dialog
            self.initialize(ctx);
        }
    }

    fn get_selected_type(&self) -> EnumType {
        let selected_idx = self
            .enum_type_handles
            .enum_type_state_handle
            .get_selected_idx()
            .unwrap_or(0);

        self.enum_type_options[selected_idx]
    }

    fn set_selected_type(&mut self, selected_type: EnumType, ctx: &mut ViewContext<Self>) {
        self.enum_type_handles
            .enum_type_state_handle
            .set_selected_idx(self.get_enum_type_idx(selected_type));
        ctx.notify();
    }

    fn get_enum_type_idx(&self, arg_type: EnumType) -> usize {
        self.enum_type_options
            .iter()
            .position(|type_option| *type_option == arg_type)
            .unwrap_or(0)
    }

    fn handle_name_editor_event(&mut self, event: &Event, ctx: &mut ViewContext<Self>) {
        match event {
            Event::Navigate(NavigationKey::Tab) => match self.get_selected_type() {
                EnumType::Static => {
                    if let Some(variant_row) = self.variant_rows.first() {
                        ctx.focus(&variant_row.variant_editor);
                    }
                }
                EnumType::Dynamic => ctx.focus(&self.dynamic_command_editor),
            },
            Event::Navigate(NavigationKey::ShiftTab) => match self.get_selected_type() {
                EnumType::Static => {
                    if let Some(variant_row) = self.variant_rows.last() {
                        ctx.focus(&variant_row.variant_editor);
                    }
                }
                EnumType::Dynamic => ctx.focus(&self.dynamic_command_editor),
            },
            Event::Edited(_) => {
                ctx.notify();
            }
            _ => {}
        }
    }

    fn handle_command_editor_event(&mut self, event: &Event, ctx: &mut ViewContext<Self>) {
        match event {
            Event::Navigate(NavigationKey::Tab) => ctx.focus(&self.name_editor),
            Event::Navigate(NavigationKey::ShiftTab) => ctx.focus(&self.name_editor),
            Event::Navigate(NavigationKey::Up) => self
                .dynamic_command_editor
                .update(ctx, |input, ctx| input.move_up(ctx)),
            Event::Navigate(NavigationKey::Down) => self
                .dynamic_command_editor
                .update(ctx, |input, ctx| input.move_down(ctx)),
            Event::Edited(_) => {
                ctx.notify();
            }
            _ => {}
        }
    }

    // Determine if the enum dialog is dirty
    fn is_dirty(&self, app: &AppContext) -> bool {
        let selected_type = self.get_selected_type();
        let variants_are_dirty = match selected_type {
            EnumType::Static => {
                let any_variant_is_dirty = self
                    .variant_rows
                    .iter()
                    .any(|row| row.variant_editor.as_ref(app).is_dirty(app));
                any_variant_is_dirty
                    || self.base_dialog_state.variant_rows != self.variant_rows.len()
            }
            EnumType::Dynamic => self.dynamic_command_editor.as_ref(app).is_dirty(app),
        };
        let name_is_dirty = self.name_editor.as_ref(app).is_dirty(app);
        let selected_type_is_dirty = self.base_dialog_state.selected_type != selected_type;

        variants_are_dirty || name_is_dirty || selected_type_is_dirty
    }

    fn handle_variant_event(
        &mut self,
        handle: ViewHandle<EditorView>,
        event: &Event,
        ctx: &mut ViewContext<Self>,
    ) {
        // get the index of the row where the event originated
        let index = self
            .variant_rows
            .iter()
            .enumerate()
            .find_map(|(index, editor)| {
                if editor.variant_editor == handle {
                    Some(index)
                } else {
                    None
                }
            });

        match event {
            Event::Navigate(NavigationKey::ShiftTab) => {
                self.focus_prev_variant_editor(index, ctx);
            }
            Event::Navigate(NavigationKey::Tab) => {
                self.focus_next_variant_editor(index, ctx);
            }
            Event::Edited(_) => {
                ctx.notify();
            }
            _ => {}
        }
    }

    fn focus_prev_variant_editor(&self, index: Option<usize>, ctx: &mut ViewContext<Self>) {
        let Some(index) = index else { return };

        if index == 0 {
            ctx.focus(&self.name_editor);
        } else if let Some(prev_variant_row) = self.variant_rows.get(index - 1) {
            ctx.focus(&prev_variant_row.variant_editor);
        }
    }

    fn focus_next_variant_editor(&mut self, index: Option<usize>, ctx: &mut ViewContext<Self>) {
        let Some(index) = index else { return };

        if index == self.variant_rows.len() - 1 {
            self.add_variant_row(ctx);
        }
        if let Some(next_variant_row) = self.variant_rows.get(index + 1) {
            ctx.focus(&next_variant_row.variant_editor);
        }
    }

    fn should_disable_save(&self, app: &AppContext) -> bool {
        let variants_empty = match self.get_selected_type() {
            EnumType::Static => self.editors_are_empty(app) || self.variant_rows.is_empty(),
            EnumType::Dynamic => self.dynamic_command_editor.as_ref(app).is_empty(app),
        };
        let name_empty = self.name_editor.as_ref(app).is_empty(app);
        variants_empty || name_empty || !self.is_dirty(app)
    }

    fn editors_are_empty(&self, app: &AppContext) -> bool {
        self.variant_rows
            .iter()
            .any(|row| row.variant_editor.as_ref(app).is_empty(app))
    }

    fn save_enum_and_close(&mut self, ctx: &mut ViewContext<Self>) {
        if self.should_disable_save(ctx) {
            return;
        }

        let variants = match self.get_selected_type() {
            EnumType::Static => EnumVariants::Static(
                self.variant_rows
                    .iter()
                    .map(|variant_row| variant_row.variant_editor.as_ref(ctx).buffer_text(ctx))
                    .collect(),
            ),
            EnumType::Dynamic => {
                EnumVariants::Dynamic(self.dynamic_command_editor.as_ref(ctx).buffer_text(ctx))
            }
        };

        match self.sync_id {
            // If an existing index was passed into the enum dialog and the view is dirty, we have edited the enum
            Some(id) => {
                if self.is_dirty(ctx) {
                    ctx.emit(EnumCreationDialogEvent::EditEnum(
                        WorkflowEnumData {
                            id,
                            name: self.name_editor.as_ref(ctx).buffer_text(ctx),
                            is_shared: true,
                            revision_ts: self.revision_ts.clone(),
                            new_data: Some(variants),
                        },
                        false,
                    ));
                }
            }
            // If we don't have an existing index, we are creating a new enum
            None => {
                ctx.emit(EnumCreationDialogEvent::CreateEnum(WorkflowEnumData {
                    id: SyncId::ClientId(ClientId::default()),
                    name: self.name_editor.as_ref(ctx).buffer_text(ctx),
                    is_shared: true,
                    revision_ts: self.revision_ts.clone(),
                    new_data: Some(variants),
                }));
            }
        }

        self.close(ctx);
    }

    fn close(&mut self, ctx: &mut ViewContext<Self>) {
        // Clear the enum editor fields
        self.name_editor
            .update(ctx, |buffer, ctx| buffer.clear_buffer(ctx));
        self.dynamic_command_editor
            .update(ctx, |buffer, ctx| buffer.clear_buffer(ctx));
        self.variant_rows = Vec::new();
        self.sync_id = None;
        self.revision_ts = None;
        self.set_selected_type(EnumType::default(), ctx);

        ctx.emit(EnumCreationDialogEvent::Close)
    }

    fn add_variant_row(&mut self, ctx: &mut ViewContext<Self>) {
        let appearance = Appearance::as_ref(ctx);
        let ui_font_family = appearance.ui_font_family();

        let variant_editor = ctx.add_typed_action_view(|ctx| {
            let mut editor = EditorView::single_line(
                SingleLineEditorOptions {
                    text: TextOptions {
                        font_size_override: Some(VARIANT_FONT_SIZE),
                        font_family_override: Some(ui_font_family),
                        ..Default::default()
                    },
                    propagate_and_no_op_vertical_navigation_keys:
                        PropagateAndNoOpNavigationKeys::Always,
                    ..Default::default()
                },
                ctx,
            );
            editor.set_placeholder_text(VARIANT_PLACEHOLDER_TEXT, ctx);
            editor
        });

        ctx.subscribe_to_view(&variant_editor, |me, emitter, event, ctx| {
            me.handle_variant_event(emitter, event, ctx);
        });

        self.variant_rows.push(VariantEditorRow {
            variant_editor,
            delete_row_mouse_state_handle: Default::default(),
        });

        ctx.notify();
    }

    fn delete_row(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        self.variant_rows.remove(index);

        if self.variant_rows.is_empty() {
            self.add_variant_row(ctx);
        }

        ctx.notify();
    }

    fn render_button(
        &self,
        appearance: &Appearance,
        button_mouse_state: MouseStateHandle,
        action: EnumCreationDialogAction,
        label_text: &str,
        is_save: bool,
        is_disabled: bool,
    ) -> Box<dyn Element> {
        let mut button = appearance
            .ui_builder()
            .button(
                if is_save {
                    ButtonVariant::Accent
                } else {
                    ButtonVariant::Secondary
                },
                button_mouse_state,
            )
            .with_centered_text_label(label_text.to_owned())
            .with_style(UiComponentStyles {
                font_size: Some(BUTTON_FONT_SIZE),
                font_weight: Some(warpui::fonts::Weight::Normal),
                ..Default::default()
            });

        if is_disabled {
            button = button.disabled();
        }

        button
            .build()
            .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
            .with_cursor(warpui::platform::Cursor::PointingHand)
            .finish()
    }

    fn render_name_editor(&self, appearance: &Appearance) -> Box<dyn Element> {
        Container::new(
            appearance
                .ui_builder()
                .text_input(self.name_editor.clone())
                .with_style(UiComponentStyles::default())
                .build()
                .finish(),
        )
        .with_horizontal_margin(CONTAINER_PADDING)
        .with_margin_bottom(SECTION_SPACING)
        .finish()
    }

    fn render_dialog_header(&self, appearance: &Appearance) -> Box<dyn Element> {
        let text = match self.sync_id {
            Some(_) => EXISTING_ENUM_SPAN,
            None => NEW_ENUM_SPAN,
        };

        appearance
            .ui_builder()
            .span(text)
            .with_style(UiComponentStyles {
                font_size: Some(SPAN_FONT_SIZE),
                ..Default::default()
            })
            .build()
            .finish()
    }

    fn render_toggle_buttons(&self, appearance: &Appearance) -> Box<dyn Element> {
        Container::new(
            appearance
                .ui_builder()
                .toggle_menu(
                    self.enum_type_handles.enum_type_mouse_states.clone(),
                    self.enum_type_options
                        .iter()
                        .map(|arg_type| {
                            let label: &'static str = arg_type.into();
                            ToggleMenuItem::new(label)
                        })
                        .collect(),
                    self.enum_type_handles.enum_type_state_handle.clone(),
                    Some(0),
                    Some(appearance.theme().background()),
                    Some(appearance.theme().surface_2()),
                    Some(appearance.theme().surface_3()),
                    appearance.ui_font_size(),
                    Rc::new(|_, _, _| {}),
                )
                .build()
                .finish(),
        )
        .with_horizontal_margin(CONTAINER_PADDING)
        .with_margin_bottom(ROW_MARGIN)
        .finish()
    }

    fn render_variants_section(&self, appearance: &Appearance) -> Box<dyn Element> {
        match self.get_selected_type() {
            EnumType::Static => self.render_static_section(appearance),
            EnumType::Dynamic => self.render_dynamic_section(appearance),
        }
    }

    fn render_static_section(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();

        Flex::column()
            .with_child(
                Container::new(self.render_static_section_header(appearance))
                    .with_horizontal_margin(CONTAINER_PADDING)
                    .with_margin_bottom(ROW_MARGIN)
                    .finish(),
            )
            .with_child(
                Container::new(
                    ConstrainedBox::new(
                        ClippedScrollable::vertical(
                            self.variants_clipped_scroll_state.clone(),
                            Container::new(
                                Flex::column()
                                    .with_children(self.render_variant_rows(appearance))
                                    .finish(),
                            )
                            .finish(),
                            SCROLLBAR_WIDTH,
                            theme.disabled_text_color(theme.background()).into(),
                            theme.main_text_color(theme.background()).into(),
                            Fill::None,
                        )
                        .finish(),
                    )
                    .with_max_height(200.)
                    .finish(),
                )
                .with_margin_left(CONTAINER_PADDING)
                .with_margin_right(CONTAINER_PADDING - OFFSET_FOR_SCROLLBAR)
                .with_margin_bottom(SECTION_SPACING)
                .finish(),
            )
            .finish()
    }

    fn render_dynamic_section(&self, appearance: &Appearance) -> Box<dyn Element> {
        let command_editor = ConstrainedBox::new(
            appearance
                .ui_builder()
                .text_input(self.dynamic_command_editor.clone())
                .with_style(UiComponentStyles::default())
                .build()
                .finish(),
        )
        .with_min_height(COMMAND_EDITOR_HEIGHT)
        .finish();

        Container::new(command_editor)
            .with_horizontal_margin(CONTAINER_PADDING)
            .with_margin_bottom(SECTION_SPACING)
            .finish()
    }

    fn render_variant_editor(
        &self,
        appearance: &Appearance,
        editor: ViewHandle<EditorView>,
    ) -> Box<dyn Element> {
        Shrinkable::new(
            1.,
            Container::new(
                ConstrainedBox::new(
                    appearance
                        .ui_builder()
                        .text_input(editor.clone())
                        .with_style(UiComponentStyles::default())
                        .build()
                        .finish(),
                )
                .with_max_height(VARIANT_EDITOR_HEIGHT)
                .finish(),
            )
            .with_margin_right(ROW_SPACING)
            .finish(),
        )
        .finish()
    }

    fn render_variant_rows(&self, appearance: &Appearance) -> Vec<Box<dyn Element>> {
        let variants: Vec<Box<dyn Element>> = self
            .variant_rows
            .iter()
            .enumerate()
            .map(|(index, variant_editor_row)| {
                Container::new(
                    Flex::row()
                        .with_main_axis_alignment(MainAxisAlignment::End)
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_child(self.render_variant_editor(
                            appearance,
                            variant_editor_row.variant_editor.clone(),
                        ))
                        .with_child(
                            icon_button(
                                appearance,
                                Icon::MinusCircle,
                                false,
                                variant_editor_row.delete_row_mouse_state_handle.clone(),
                            )
                            .build()
                            .on_click(move |ctx, _, _| {
                                ctx.dispatch_typed_action(EnumCreationDialogAction::DeleteVariant(
                                    VariantRowIndex(index),
                                ))
                            })
                            .finish(),
                        )
                        .finish(),
                )
                .with_margin_bottom(ROW_MARGIN)
                .finish()
            })
            .collect();

        variants
    }

    fn render_static_section_header(&self, appearance: &Appearance) -> Box<dyn Element> {
        let mut variants_header = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Center);

        variants_header.add_child(
            Shrinkable::new(
                1.,
                appearance
                    .ui_builder()
                    .span(STATIC_LABEL_TEXT.to_string())
                    .with_style(UiComponentStyles {
                        font_size: Some(SECTION_FONT_SIZE),
                        ..Default::default()
                    })
                    .build()
                    .finish(),
            )
            .finish(),
        );

        variants_header.add_child(
            Shrinkable::new(
                1.,
                Flex::row()
                    .with_main_axis_alignment(MainAxisAlignment::End)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_child(
                        icon_button(
                            appearance,
                            Icon::Plus,
                            false,
                            self.mouse_state_handles.add_variant_state.clone(),
                        )
                        .build()
                        .on_click(|ctx, _, _| {
                            ctx.dispatch_typed_action(EnumCreationDialogAction::AddVariant)
                        })
                        .finish(),
                    )
                    .finish(),
            )
            .finish(),
        );

        variants_header.finish()
    }

    fn render_footer_buttons(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        let disable_save = self.should_disable_save(app);
        let save_button_label = match self.sync_id {
            None => CREATE_BUTTON_LABEL,
            Some(_) => SAVE_BUTTON_LABEL,
        };

        Flex::row()
            .with_child(
                Shrinkable::new(
                    1.,
                    Container::new(
                        self.render_button(
                            appearance,
                            self.mouse_state_handles
                                .cancel_button_mouse_state_handle
                                .clone(),
                            EnumCreationDialogAction::Close,
                            CANCEL_BUTTON_LABEL,
                            false,
                            false,
                        ),
                    )
                    .with_margin_right(ELEMENT_SPACING)
                    .finish(),
                )
                .finish(),
            )
            .with_child(
                Shrinkable::new(
                    1.,
                    self.render_button(
                        appearance,
                        self.mouse_state_handles
                            .save_button_mouse_state_handle
                            .clone(),
                        EnumCreationDialogAction::SaveEnum,
                        save_button_label,
                        true,
                        disable_save,
                    ),
                )
                .finish(),
            )
            .finish()
    }
}

impl Entity for EnumCreationDialog {
    type Event = EnumCreationDialogEvent;
}

impl View for EnumCreationDialog {
    fn ui_name() -> &'static str {
        "EnumCreationDialog"
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            ctx.focus(&self.name_editor);
            ctx.notify();
        }
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);

        ConstrainedBox::new(
            Shrinkable::new(
                1.,
                Container::new(
                    Flex::column()
                        .with_child(
                            Container::new(self.render_dialog_header(appearance))
                                .with_horizontal_margin(CONTAINER_PADDING)
                                .with_vertical_margin(SECTION_SPACING)
                                .finish(),
                        )
                        .with_child(self.render_name_editor(appearance))
                        .with_child(Container::new(self.render_toggle_buttons(appearance)).finish())
                        .with_child(self.render_variants_section(appearance))
                        .with_child(
                            Container::new(self.render_footer_buttons(appearance, app))
                                .with_horizontal_margin(CONTAINER_PADDING)
                                .with_margin_bottom(CONTAINER_PADDING)
                                .finish(),
                        )
                        .finish(),
                )
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
                .with_border(Border::all(2.).with_border_fill(appearance.theme().surface_2()))
                .with_background(appearance.theme().surface_1())
                .finish(),
            )
            .finish(),
        )
        .with_max_width(CORE_WIDTH)
        .with_height(CORE_HEIGHT)
        .finish()
    }
}

impl TypedActionView for EnumCreationDialog {
    type Action = EnumCreationDialogAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            EnumCreationDialogAction::Close => self.close(ctx),
            EnumCreationDialogAction::SaveEnum => self.save_enum_and_close(ctx),
            EnumCreationDialogAction::AddVariant => self.add_variant_row(ctx),
            EnumCreationDialogAction::DeleteVariant(VariantRowIndex(index)) => {
                self.delete_row(*index, ctx);
            }
        }
    }
}
