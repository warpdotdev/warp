use std::collections::HashMap;
use std::rc::Rc;

use itertools::Itertools;
use strum::IntoEnumIterator;
use strum_macros::{EnumIter, IntoStaticStr};
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::Fill;
use warp_editor::editor::NavigationKey;
use warpui::elements::{
    Align, Border, ChildAnchor, ChildView, ClippedScrollStateHandle, ClippedScrollable,
    ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Dismiss, Empty, EventHandler,
    Flex, Hoverable, MainAxisSize, MouseStateHandle, OffsetPositioning, ParentAnchor,
    ParentElement, ParentOffsetBounds, Radius, ScrollbarWidth, Shrinkable, Stack, Text,
};
use warpui::fonts::FamilyId;
use warpui::geometry::vector::vec2f;
use warpui::platform::Cursor;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::ui_components::text::Span;
use warpui::ui_components::toggle_menu::{ToggleMenuItem, ToggleMenuStateHandle};
use warpui::{
    AppContext, Element, Entity, FocusContext, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle,
};

use super::enum_creation_dialog::WorkflowEnumData;
use crate::editor::{
    EditorOptions, EditorView, EnterAction, EnterSettings, Event as EditorEvent, InteractionState,
    PropagateAndNoOpNavigationKeys, TextOptions,
};
use crate::server::ids::SyncId;
use crate::ui_components::buttons::{highlight, icon_button};
use crate::ui_components::icons::{self, Icon};
use crate::workflows::workflow::ArgumentType;

const ARGUMENT_DEFAULT_VALUE_PLACEHOLDER_TEXT: &str = "Default value (optional)";
const ARGUMENT_EDITOR_FONT_SIZE: f32 = 14.;
const DROPDOWN_PADDING: f32 = 8.;
const DROPDOWN_BORDER_RADIUS: f32 = 6.;
const EDIT_ICON_HEIGHT: f32 = 24.;
const ENUM_MENU_HEIGHT: f32 = 100.;
const MENU_ITEM_VERTICAL_PADDING: f32 = 4.;
const MENU_ITEM_HORIZONTAL_PADDING: f32 = 8.;
const MENU_ITEM_HORIZONTAL_MARGIN: f32 = 12.;
const TOGGLE_MENU_BOTTOM_PADDING: f32 = 4.;

pub struct WorkflowArgSelector {
    pub text_editor: ViewHandle<EditorView>,

    // `is_expanded` is true when the selector is open
    is_expanded: bool,
    is_disabled: bool,
    editor_mouse_state: MouseStateHandle,
    // The handles and options used for the type radio buttons
    arg_type_handles: ArgTypeHandles,
    arg_type_options: Vec<ArgumentSelectType>,

    styles: WorkflowArgSelectorStyles,

    /// All workflow enums accessible by this selector
    /// Corresponds with a Vec of WorkflowEnumData maintained by the parent view or modal.
    all_workflow_enums: HashMap<SyncId, EnumMenuItem>,
    /// Vector of indices of `all_workflow_enums` to display, based on the filter query in `text_editor`
    /// We also use filtered enums to display the list in alphabetical order, by sorting the filtered indices.
    filtered_enums: Vec<SyncId>,
    /// Vector of all workflow enums created before saving.
    created_enums: Vec<SyncId>,

    /// The index into the workflow enums vector of the selected enum
    selected_enum: Option<SyncId>,

    /// Base state when the row is loaded, used when determining if it is dirty
    base_selection: ArgumentType,

    /// States for the enum dropdown list
    enum_menu_mouse_state: MouseStateHandle,
    enum_search_clipped_scroll_state: ClippedScrollStateHandle,
}

struct EnumMenuItem {
    name: String,
    /// Used for determining whether the whole row has been hovered
    item_row_state_handle: MouseStateHandle,
    /// Used for determining if a row has been selected
    select_item_state_handle: MouseStateHandle,
    /// Used for determining if the edit button for a row has been clicked
    edit_item_state_handle: MouseStateHandle,
}

impl EnumMenuItem {
    fn new(data: &WorkflowEnumData) -> Self {
        EnumMenuItem {
            name: data.name.clone(),
            item_row_state_handle: Default::default(),
            select_item_state_handle: Default::default(),
            edit_item_state_handle: Default::default(),
        }
    }

    fn from_text(text: String) -> Self {
        EnumMenuItem {
            name: text,
            item_row_state_handle: Default::default(),
            select_item_state_handle: Default::default(),
            edit_item_state_handle: Default::default(),
        }
    }
}

/// Styles used when rendering the modal vs. the panes workflow view
pub struct WorkflowArgSelectorStyles {
    /// The input padding within an editor
    pub editor_padding: Coords,
    pub width: Option<f32>,
    pub height: Option<f32>,
    pub border_radius: f32,
    /// We need to configure these colors as they differ between the workflow
    /// view and the workflow modal. They are passed as closures so the colors
    /// correctly update when a theme is changed.
    pub dropdown_background: fn(&Appearance) -> Fill,
    pub border_color: fn(&Appearance) -> Fill,
}

#[derive(Default, Clone)]
struct ArgTypeHandles {
    arg_type_state_handle: ToggleMenuStateHandle,
    arg_type_mouse_states: Vec<MouseStateHandle>,
}

#[derive(Debug, Clone, Copy, PartialEq, IntoStaticStr, EnumIter, Default)]
pub enum ArgumentSelectType {
    #[default]
    Text,
    Enum,
}

impl From<ArgumentType> for ArgumentSelectType {
    fn from(arg_type: ArgumentType) -> Self {
        match arg_type {
            ArgumentType::Text => ArgumentSelectType::Text,
            ArgumentType::Enum { .. } => ArgumentSelectType::Enum,
        }
    }
}

impl WorkflowArgSelector {
    pub fn new(
        styles: WorkflowArgSelectorStyles,
        all_workflow_enums: &HashMap<SyncId, WorkflowEnumData>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let appearance = Appearance::as_ref(ctx);
        let ui_font_family: FamilyId = appearance.ui_font_family();

        let text_editor = ctx.add_typed_action_view(|ctx| {
            EditorView::new(
                EditorOptions {
                    text: TextOptions {
                        font_size_override: Some(ARGUMENT_EDITOR_FONT_SIZE),
                        font_family_override: Some(ui_font_family),
                        ..Default::default()
                    },
                    soft_wrap: true,
                    autogrow: true,
                    autocomplete_symbols: true,
                    // Ideally, we'd set this to PropagateAndNoOpNavigationKeys::AtBoundary, so
                    // that the workflow modal doesn't need to handle up/down navigation for the
                    // command and description editors. However, that breaks tab and shift-tab
                    // navigation, since those are only emitted with
                    // PropagateAndNoOpNavigationKeys::Never.
                    propagate_and_no_op_vertical_navigation_keys:
                        PropagateAndNoOpNavigationKeys::Always,
                    supports_vim_mode: false,
                    single_line: false,
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
        });

        ctx.subscribe_to_view(&text_editor, |me, _, event, ctx| {
            me.handle_text_editor_event(event, ctx);
        });

        let arg_type_handles = ArgTypeHandles {
            arg_type_mouse_states: vec![Default::default(), Default::default(), Default::default()],
            ..Default::default()
        };

        let arg_type_options = ArgumentSelectType::iter().collect();

        let mut me = WorkflowArgSelector {
            text_editor,
            is_expanded: false,
            is_disabled: false,
            editor_mouse_state: Default::default(),
            enum_menu_mouse_state: Default::default(),
            arg_type_handles,
            arg_type_options,
            styles,
            selected_enum: None,
            base_selection: Default::default(),
            all_workflow_enums: Default::default(),
            filtered_enums: Default::default(),
            created_enums: Default::default(),
            enum_search_clipped_scroll_state: Default::default(),
        };

        me.set_workflow_enums(all_workflow_enums, ctx);
        me.update_filtered_items(ctx);
        me
    }

    pub fn get_selected_type(&self) -> ArgumentSelectType {
        let selected_idx = self
            .arg_type_handles
            .arg_type_state_handle
            .get_selected_idx()
            .unwrap_or(0);

        self.arg_type_options[selected_idx]
    }

    pub fn set_selected_type(
        &mut self,
        selected_type: ArgumentSelectType,
        ctx: &mut ViewContext<Self>,
    ) {
        self.arg_type_handles
            .arg_type_state_handle
            .set_selected_idx(self.get_arg_type_idx(selected_type));
        ctx.notify();
    }

    fn get_arg_type_idx(&self, arg_type: ArgumentSelectType) -> usize {
        self.arg_type_options
            .iter()
            .position(|type_option| *type_option == arg_type)
            .unwrap_or(0)
    }

    pub fn get_selected_enum(&self) -> Option<SyncId> {
        self.selected_enum
    }

    pub fn set_selected_enum(&mut self, id: Option<SyncId>, ctx: &mut ViewContext<Self>) {
        if id.is_some() {
            self.selected_enum = id;
            self.set_selected_type(ArgumentSelectType::Enum, ctx);
        } else {
            self.selected_enum = None;
            self.set_selected_type(ArgumentSelectType::Text, ctx);
        }

        self.close(ctx);
        ctx.emit(WorkflowArgSelectorEvent::Edited);
    }

    /// Set the selected enum and base enum, used for determining if the selector is dirty
    pub fn set_selected_enum_with_base_enum(
        &mut self,
        id: Option<SyncId>,
        ctx: &mut ViewContext<Self>,
    ) {
        if let Some(id) = id {
            self.base_selection = ArgumentType::Enum { enum_id: id };
        } else {
            self.base_selection = ArgumentType::Text;
        }

        self.set_selected_enum(id, ctx);
    }

    pub fn get_created_enums(&self) -> Vec<SyncId> {
        self.created_enums.clone()
    }

    pub fn set_workflow_enums(
        &mut self,
        workflow_enums: &HashMap<SyncId, WorkflowEnumData>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.all_workflow_enums = workflow_enums
            .iter()
            .filter_map(|(id, enum_data)| {
                if enum_data.is_shared || Some(*id) == self.get_selected_enum() {
                    Some((*id, EnumMenuItem::new(enum_data)))
                } else {
                    None
                }
            })
            .collect();
        self.update_filtered_items(ctx);
        ctx.notify();
    }

    /// Add a new enum to the workflow enums list.
    /// Used when an enum is created anywhere in the parent workflow editor.
    pub fn insert_enum_into_menu(
        &mut self,
        enum_id: SyncId,
        enum_name: String,
        ctx: &mut ViewContext<Self>,
    ) {
        self.all_workflow_enums
            .insert(enum_id, EnumMenuItem::from_text(enum_name));
        self.created_enums.push(enum_id);
        self.update_filtered_items(ctx);
        ctx.notify();
    }

    /// Remove the enum with `enum_id` from the menu.
    /// Used when an enum created in another row is edited to be "unshared".
    pub fn remove_enum_from_menu(&mut self, enum_id: &SyncId, ctx: &mut ViewContext<Self>) {
        // Don't remove the item if it's currently selected
        let selected_enum = self.get_selected_enum();
        if selected_enum == Some(*enum_id) {
            return;
        }

        if self.all_workflow_enums.remove(enum_id).is_some() {
            self.update_filtered_items(ctx);
        }
        ctx.notify();
    }

    pub fn clear_data(&mut self) {
        self.base_selection = Default::default();
        self.selected_enum = None;
    }

    pub fn is_dirty(&self, app: &AppContext) -> bool {
        let text_editor_is_dirty = self.text_editor.as_ref(app).is_dirty(app);
        // TODO(CLD-2167): This could also be cleaned up if we migrate away from managing the `selected_type` and `selected_enum`
        // separately. Ideally, the selected enum and selected type can be tracked together using the `ArgumentType` enum.
        let type_is_dirty = match self.base_selection {
            ArgumentType::Text => self.get_selected_type() != ArgumentSelectType::Text,
            ArgumentType::Enum { enum_id } => {
                let selected_enum_dirty = self.get_selected_enum() != Some(enum_id);
                self.get_selected_type() != ArgumentSelectType::Enum || selected_enum_dirty
            }
        };

        text_editor_is_dirty || type_is_dirty
    }

    pub fn set_editor_text(&self, text: &str, ctx: &mut ViewContext<Self>) {
        self.text_editor.update(ctx, |editor, ctx| {
            editor.set_buffer_text_with_base_buffer(text, ctx);
        });
    }

    fn type_toggled(&self, ctx: &mut ViewContext<Self>) {
        // Clear the text when toggling to the enum type, so we start with a blank filter query
        if self.get_selected_type() == ArgumentSelectType::Enum {
            self.text_editor.update(ctx, |editor, ctx| {
                editor.clear_buffer(ctx);
            })
        }
    }

    fn toggle_expanded(&mut self, ctx: &mut ViewContext<Self>) {
        if self.is_disabled {
            return;
        }

        self.is_expanded = !self.is_expanded;
        if self.is_expanded {
            ctx.focus(&self.text_editor);
        }
        ctx.emit(WorkflowArgSelectorEvent::ToggleExpanded);
        ctx.notify();
    }

    fn new_enum(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.emit(WorkflowArgSelectorEvent::NewEnum);
        self.close(ctx);
    }

    /// Called when we want to select an existing enum from the dropdown list
    fn edit_enum(&mut self, id: SyncId, ctx: &mut ViewContext<Self>) {
        // Hide the dropdown menu while the enum dialog is open
        self.is_expanded = false;
        ctx.emit(WorkflowArgSelectorEvent::LoadEnum(id));
        ctx.notify();
    }

    /// Update the filtered items, based on the workflow_enums list and query in the text editor.
    /// This function also handles alphabetical sorting of the display, so the list is displayed alphabetically
    /// while the underlying workflow_enums remains unchanged.
    fn update_filtered_items(&mut self, app: &AppContext) {
        let filter_query = self.text_editor.as_ref(app).buffer_text(app).to_lowercase();
        self.filtered_enums = self
            .all_workflow_enums
            .iter()
            .filter(|(_, EnumMenuItem { name, .. })| name.to_lowercase().contains(&filter_query))
            .sorted_by(|(_, enum_a), (_, enum_b)| {
                enum_a.name.to_lowercase().cmp(&enum_b.name.to_lowercase())
            })
            .map(|(id, _)| *id)
            .collect();
    }

    pub fn disable(&mut self, ctx: &mut ViewContext<Self>) {
        self.text_editor.update(ctx, |editor, ctx| {
            editor.set_interaction_state(InteractionState::Disabled, ctx);
        });
        self.is_disabled = true;
    }

    pub fn enable(&mut self, ctx: &mut ViewContext<Self>) {
        self.text_editor.update(ctx, |editor, ctx| {
            editor.set_interaction_state(InteractionState::Editable, ctx);
        });
        self.is_disabled = false;
    }

    pub fn clear_created_enums(&mut self, ctx: &mut ViewContext<Self>) {
        self.created_enums.clear();
        ctx.notify();
    }

    pub fn close(&mut self, ctx: &mut ViewContext<Self>) {
        // TODO(CLD-2167): If the selected type changes, we need to remember to clean up the separate
        // `selected_enum` field. Ideally, the selected enum and selected type can be tracked together
        // using the `ArgumentType` enum.
        // If we've selected enum but don't have any saved enum, set back to default type
        if self.get_selected_type() == ArgumentSelectType::Enum && self.selected_enum.is_none() {
            self.set_selected_type(Default::default(), ctx);
        }
        // If we've selected text, erase any saved enum
        else if self.get_selected_type() == ArgumentSelectType::Text {
            self.selected_enum = None;
        }

        self.is_expanded = false;
        ctx.emit(WorkflowArgSelectorEvent::Close);
        ctx.notify();
    }

    fn handle_text_editor_event(&mut self, event: &EditorEvent, ctx: &mut ViewContext<Self>) {
        match event {
            EditorEvent::Edited(_) => {
                self.update_filtered_items(ctx);
                ctx.emit(WorkflowArgSelectorEvent::Edited);
                ctx.notify();
            }
            EditorEvent::Escape => self.close(ctx),
            EditorEvent::Navigate(NavigationKey::Tab) => {
                ctx.emit(WorkflowArgSelectorEvent::InputTab);
                self.close(ctx);
            }
            EditorEvent::Navigate(NavigationKey::ShiftTab) => {
                ctx.emit(WorkflowArgSelectorEvent::InputShiftTab);
                self.close(ctx);
            }
            _ => (),
        }
    }

    fn render_text_editor(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        let bar = if self.is_expanded {
            self.render_open_top_bar(appearance)
        } else {
            self.render_closed_top_bar(appearance, app)
        };

        let mut editor = ConstrainedBox::new(bar);

        if let Some(width) = self.styles.width {
            editor = editor.with_width(width);
        }

        if let Some(height) = self.styles.height {
            editor = editor.with_height(height);
        }

        editor.finish()
    }

    // Render the closed top bar when we have a text type argument
    fn render_closed_text_top_bar(
        &self,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let should_show_placeholder = self.text_editor.as_ref(app).is_empty(app);

        let text_label = match should_show_placeholder {
            true => ARGUMENT_DEFAULT_VALUE_PLACEHOLDER_TEXT.to_string(),
            false => self.text_editor.as_ref(app).buffer_text(app),
        };

        let editor_font_color = match should_show_placeholder {
            true => appearance
                .theme()
                .hint_text_color(appearance.theme().background())
                .into(),
            false => appearance
                .theme()
                .main_text_color(appearance.theme().background())
                .into(),
        };

        let font_styles = UiComponentStyles {
            font_size: Some(ARGUMENT_EDITOR_FONT_SIZE),
            font_family_id: Some(appearance.ui_font_family()),
            font_color: Some(editor_font_color),
            ..Default::default()
        };

        let text = Align::new(Span::new(text_label, font_styles).build().finish())
            .left()
            .finish();

        let container = Container::new(text)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
                self.styles.border_radius,
            )))
            .with_border(Border::all(1.).with_border_fill((self.styles.border_color)(appearance)))
            .with_padding_top(self.styles.editor_padding.top)
            .with_padding_bottom(self.styles.editor_padding.bottom)
            .with_padding_left(self.styles.editor_padding.left)
            .with_padding_right(self.styles.editor_padding.right)
            .with_background(appearance.theme().background());

        let hoverable = Hoverable::new(self.editor_mouse_state.clone(), |_| container.finish())
            .with_cursor(Cursor::IBeam)
            .on_click(|ctx, _, _| {
                ctx.dispatch_typed_action(WorkflowArgSelectorAction::ToggleExpanded);
            });

        hoverable.finish()
    }

    // Render the closed top bar when we have a enum type argument
    fn render_closed_enum_top_bar(&self, appearance: &Appearance) -> Box<dyn Element> {
        let font_styles = UiComponentStyles {
            font_size: Some(ARGUMENT_EDITOR_FONT_SIZE),
            font_family_id: Some(appearance.ui_font_family()),
            font_color: Some(
                appearance
                    .theme()
                    .main_text_color(appearance.theme().background())
                    .into(),
            ),
            ..Default::default()
        };

        let text_label = match &self
            .selected_enum
            .and_then(|id| self.all_workflow_enums.get(&id))
        {
            Some(menu_item) => menu_item.name.clone(),
            _ => Default::default(),
        };

        let enum_text = Align::new(Span::new(text_label, font_styles).build().finish()).finish();
        let mut container = Container::new(enum_text)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
                self.styles.border_radius,
            )))
            .with_border(Border::all(1.).with_border_fill((self.styles.border_color)(appearance)))
            .with_padding_top(self.styles.editor_padding.top)
            .with_padding_bottom(self.styles.editor_padding.bottom)
            .with_padding_left(self.styles.editor_padding.left)
            .with_padding_right(self.styles.editor_padding.right)
            .with_background(appearance.theme().background());

        let hoverable = Hoverable::new(self.editor_mouse_state.clone(), |state| {
            if state.is_hovered() {
                container = container.with_background(appearance.theme().surface_2())
            }
            container.finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action(WorkflowArgSelectorAction::ToggleExpanded);
        });

        hoverable.finish()
    }

    // Render the text editor when it is not active
    fn render_closed_top_bar(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        match self.get_selected_type() {
            ArgumentSelectType::Text => self.render_closed_text_top_bar(appearance, app),
            ArgumentSelectType::Enum => self.render_closed_enum_top_bar(appearance),
        }
    }

    fn render_search_icon(&self, appearance: &Appearance) -> Box<dyn Element> {
        ConstrainedBox::new(
            icons::Icon::SearchSmall
                .to_warpui_icon(appearance.theme().active_ui_text_color())
                .finish(),
        )
        .with_width(12.)
        .with_height(12.)
        .finish()
    }

    // Render the text editor when it is active
    fn render_open_top_bar(&self, appearance: &Appearance) -> Box<dyn Element> {
        let mut filter_bar = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max);

        let selected_type = self.get_selected_type();

        if selected_type == ArgumentSelectType::Enum {
            filter_bar.add_child(
                Container::new(self.render_search_icon(appearance))
                    .with_padding_right(6.)
                    .finish(),
            );
        }

        let filter_editor = ChildView::new(&self.text_editor).finish();
        filter_bar.add_child(Shrinkable::new(1., filter_editor).finish());

        Container::new(filter_bar.finish())
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
                self.styles.border_radius,
            )))
            .with_border(Border::all(1.).with_border_fill((self.styles.border_color)(appearance)))
            .with_padding_top(self.styles.editor_padding.top)
            .with_padding_bottom(self.styles.editor_padding.bottom)
            .with_padding_left(self.styles.editor_padding.left)
            .with_padding_right(self.styles.editor_padding.right)
            .with_background(appearance.theme().background())
            .finish()
    }

    // Render the entire section that drops below the text editor
    fn render_dropdown(&self, appearance: &Appearance) -> Box<dyn Element> {
        let toggle_default = Some(self.get_arg_type_idx(ArgumentSelectType::default()));

        let mut dropdown = Flex::column().with_child(
            Container::new(
                appearance
                    .ui_builder()
                    .toggle_menu(
                        self.arg_type_handles.arg_type_mouse_states.clone(),
                        self.arg_type_options
                            .iter()
                            .map(|arg_type| {
                                let label: &'static str = arg_type.into();
                                ToggleMenuItem::new(label)
                            })
                            .collect(),
                        self.arg_type_handles.arg_type_state_handle.clone(),
                        toggle_default,
                        None,
                        None,
                        None,
                        appearance.ui_font_size(),
                        Rc::new(|ctx, _, _| {
                            ctx.dispatch_typed_action(WorkflowArgSelectorAction::TypeToggled);
                        }),
                    )
                    .build()
                    .finish(),
            )
            .with_horizontal_margin(DROPDOWN_PADDING)
            .with_padding_bottom(TOGGLE_MENU_BOTTOM_PADDING)
            .finish(),
        );

        if let Some(type_dropdown) =
            self.render_arg_type_dropdown(appearance, self.get_selected_type())
        {
            dropdown.add_child(type_dropdown);
        }

        Container::new(dropdown.finish())
            .with_background((self.styles.dropdown_background)(appearance))
            .with_vertical_padding(DROPDOWN_PADDING)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
                DROPDOWN_BORDER_RADIUS,
            )))
            .finish()
    }

    // Render the type-specific portion of the dropdown
    fn render_arg_type_dropdown(
        &self,
        appearance: &Appearance,
        arg_type: ArgumentSelectType,
    ) -> Option<Box<dyn Element>> {
        match arg_type {
            ArgumentSelectType::Enum => Some(self.render_enum_menu(appearance)),
            _ => None,
        }
    }

    fn render_enum_search_items(&self, appearance: &Appearance) -> Vec<Box<dyn Element>> {
        let current_enum_id = self.get_selected_enum();

        let menu_items = self.filtered_enums.iter().filter_map(|id| {
            self.all_workflow_enums.get(id).map(
                |EnumMenuItem {
                     name,
                     item_row_state_handle,
                     select_item_state_handle,
                     edit_item_state_handle,
                 }| {
                    let enum_id = *id;

                    let mut menu_item = Hoverable::new(item_row_state_handle.clone(), |state| {
                        let button = Hoverable::new(select_item_state_handle.clone(), |_| {
                            Align::new(
                                Container::new(
                                    Text::new_inline(
                                        name.clone(),
                                        appearance.ui_font_family(),
                                        ARGUMENT_EDITOR_FONT_SIZE,
                                    )
                                    .with_color(appearance.theme().active_ui_text_color().into())
                                    .finish(),
                                )
                                .with_vertical_padding(MENU_ITEM_VERTICAL_PADDING)
                                .finish(),
                            )
                            .left()
                            .finish()
                        })
                        .on_click(move |ctx, _, _| {
                            ctx.dispatch_typed_action(WorkflowArgSelectorAction::SelectEnum(
                                enum_id,
                            ));
                        })
                        .finish();

                        let mut flex = Flex::row()
                            .with_main_axis_size(MainAxisSize::Max)
                            .with_cross_axis_alignment(CrossAxisAlignment::Center)
                            .with_child(Shrinkable::new(1., button).finish());

                        if state.is_hovered() {
                            let edit_hoverable = ConstrainedBox::new(
                                highlight(
                                    icon_button(
                                        appearance,
                                        Icon::Pencil,
                                        false,
                                        edit_item_state_handle.clone(),
                                    ),
                                    appearance,
                                )
                                .with_style(UiComponentStyles::default().set_font_color(
                                    appearance.theme().active_ui_text_color().into(),
                                ))
                                .build()
                                .on_click(move |ctx, _, _| {
                                    ctx.dispatch_typed_action(WorkflowArgSelectorAction::LoadEnum(
                                        enum_id,
                                    ));
                                })
                                .finish(),
                            )
                            .with_height(EDIT_ICON_HEIGHT);

                            flex.add_child(edit_hoverable.finish());
                        }

                        let mut container = Container::new(flex.finish())
                            .with_horizontal_padding(MENU_ITEM_HORIZONTAL_PADDING);

                        if Some(*id) == current_enum_id || state.is_hovered() {
                            container = container
                                .with_background(appearance.theme().foreground_button_color())
                        }

                        container.finish()
                    });

                    menu_item = menu_item.with_cursor(Cursor::PointingHand);
                    menu_item.finish()
                },
            )
        });

        menu_items.collect()
    }

    fn render_enum_menu(&self, appearance: &Appearance) -> Box<dyn Element> {
        let mut flex_col = Flex::column();

        let mut menu = Hoverable::new(self.enum_menu_mouse_state.clone(), |state| {
            let button = Text::new_inline(
                "New".to_string(),
                appearance.ui_font_family(),
                ARGUMENT_EDITOR_FONT_SIZE,
            )
            .with_color(appearance.theme().active_ui_text_color().into())
            .finish();

            let mut container = Container::new(
                Flex::row()
                    .with_child(
                        Container::new(button)
                            .with_vertical_padding(MENU_ITEM_VERTICAL_PADDING)
                            .with_horizontal_padding(MENU_ITEM_HORIZONTAL_PADDING)
                            .finish(),
                    )
                    .with_main_axis_size(MainAxisSize::Max)
                    .finish(),
            );

            if state.is_hovered() {
                container = container.with_background(appearance.theme().foreground_button_color())
            };

            Container::new(container.finish())
                .with_horizontal_margin(MENU_ITEM_HORIZONTAL_MARGIN)
                .finish()
        });

        menu = menu.with_cursor(Cursor::PointingHand);
        menu = menu.on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(WorkflowArgSelectorAction::NewEnum);
        });

        flex_col.add_child(menu.finish());

        let enum_menu_items = self.render_enum_search_items(appearance);

        if !enum_menu_items.is_empty() {
            // add a separator
            flex_col.add_child(
                Container::new(Empty::new().finish())
                    .with_border(Border::bottom(1.).with_border_fill(appearance.theme().outline()))
                    .with_horizontal_margin(MENU_ITEM_HORIZONTAL_PADDING)
                    .with_vertical_margin(MENU_ITEM_VERTICAL_PADDING)
                    .finish(),
            );

            let theme = appearance.theme();
            flex_col.add_child(
                ConstrainedBox::new(
                    ClippedScrollable::vertical(
                        self.enum_search_clipped_scroll_state.clone(),
                        Container::new(Flex::column().with_children(enum_menu_items).finish())
                            .with_margin_left(MENU_ITEM_HORIZONTAL_MARGIN)
                            .finish(),
                        ScrollbarWidth::Auto,
                        theme.disabled_text_color(theme.background()).into(),
                        theme.main_text_color(theme.background()).into(),
                        warpui::elements::Fill::None,
                    )
                    .finish(),
                )
                .with_max_height(ENUM_MENU_HEIGHT)
                .finish(),
            );
        }

        flex_col.finish()
    }
}

#[derive(Debug, Clone)]
pub enum WorkflowArgSelectorEvent {
    Close,
    NewEnum,
    Edited,
    LoadEnum(SyncId),
    ToggleExpanded,
    InputTab,
    InputShiftTab,
}

#[derive(Debug, Clone)]
pub enum WorkflowArgSelectorAction {
    Close,
    NewEnum,
    LoadEnum(SyncId),
    SelectEnum(SyncId),
    ToggleExpanded,
    TypeToggled,
}

impl View for WorkflowArgSelector {
    fn ui_name() -> &'static str {
        "WorkflowArgSelector"
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            self.toggle_expanded(ctx);
        }
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);

        let mut stack = Stack::new()
            .with_constrain_absolute_children()
            .with_child(self.render_text_editor(appearance, app));
        if self.is_expanded {
            let dropdown = self.render_dropdown(appearance);

            stack.add_positioned_overlay_child(
                dropdown,
                OffsetPositioning::offset_from_parent(
                    vec2f(0., 0.),
                    ParentOffsetBounds::WindowByPosition,
                    ParentAnchor::BottomLeft,
                    ChildAnchor::TopLeft,
                ),
            );

            Dismiss::new(EventHandler::new(stack.finish()).finish())
                .on_dismiss(|ctx, _app| ctx.dispatch_typed_action(WorkflowArgSelectorAction::Close))
                .finish()
        } else {
            stack.finish()
        }
    }
}

impl Entity for WorkflowArgSelector {
    type Event = WorkflowArgSelectorEvent;
}

impl TypedActionView for WorkflowArgSelector {
    type Action = WorkflowArgSelectorAction;

    fn handle_action(&mut self, action: &WorkflowArgSelectorAction, ctx: &mut ViewContext<Self>) {
        match action {
            WorkflowArgSelectorAction::Close => self.close(ctx),
            WorkflowArgSelectorAction::ToggleExpanded => self.toggle_expanded(ctx),
            WorkflowArgSelectorAction::NewEnum => self.new_enum(ctx),
            WorkflowArgSelectorAction::LoadEnum(index) => self.edit_enum(*index, ctx),
            WorkflowArgSelectorAction::SelectEnum(index) => {
                self.set_selected_enum(Some(*index), ctx)
            }
            WorkflowArgSelectorAction::TypeToggled => self.type_toggled(ctx),
        }
    }
}
