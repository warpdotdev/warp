use std::marker::PhantomData;

use pathfinder_geometry::vector::vec2f;
use warpui::elements::{
    Border, ChildAnchor, ChildView, Clipped, ConstrainedBox, Container, CornerRadius, Element,
    EventDispatchMode, Icon, MouseStateHandle, OffsetPositioning, ParentAnchor, ParentElement,
    ParentOffsetBounds, PositionedElementAnchor, PositionedElementOffsetBounds, Radius,
    SavePosition, Stack,
};
use warpui::text_layout::TextAlignment;
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::{
    AppContext, BlurContext, Entity, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle,
};

use super::dropdown::{
    DROPDOWN_PADDING, DropdownAction, DropdownItem, DropdownItemAction, TOP_MENU_BAR_HEIGHT,
    TOP_MENU_BAR_MAX_WIDTH,
};
use crate::appearance::Appearance;
use crate::editor::{
    EditOrigin, EditorView, Event as EditorEvent, SingleLineEditorOptions, TextOptions,
};
use crate::menu::{Event as MenuEvent, Menu, MenuItem, MenuVariant};
use crate::themes::theme;

#[cfg(test)]
#[path = "editable_dropdown_tests.rs"]
mod tests;

type Parser<A> = Box<dyn Fn(&str) -> Option<A>>;
type RevertText = Box<dyn Fn(&AppContext) -> String>;

/// A dropdown whose current value can also be edited as free-form text.
///
/// Preset selection uses the same menu actions as [`super::Dropdown`]. Typed
/// values are parsed by a caller-provided function, keeping this component
/// generic over the action it dispatches.
pub struct EditableDropdown<A: DropdownItemAction = ()> {
    is_expanded: bool,
    is_valid: bool,
    suppress_next_blur_commit: bool,
    top_bar_mouse_state: MouseStateHandle,
    top_bar_max_width: f32,
    top_bar_height: f32,
    vertical_margin: f32,
    dropdown: ViewHandle<Menu<DropdownAction>>,
    editor: ViewHandle<EditorView>,
    selected_item: Option<MenuItem<DropdownAction>>,
    parser: Option<Parser<A>>,
    revert_text: Option<RevertText>,
    _action_type: PhantomData<A>,
    #[cfg(test)]
    last_dispatched_action: Option<Box<dyn DropdownItemAction>>,
}

pub enum EditableDropdownEvent {
    Close,
    Escape,
    ToggleExpanded,
}

impl<A> EditableDropdown<A>
where
    A: DropdownItemAction,
{
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let dropdown = ctx.add_typed_action_view(|ctx| {
            let theme = Appearance::as_ref(ctx).theme();
            Menu::new()
                .with_menu_variant(MenuVariant::scrollable())
                .with_border(Border::all(1.).with_border_fill(theme.outline()))
                .prevent_interaction_with_other_elements()
        });
        ctx.subscribe_to_view(&dropdown, |me, _, event, ctx| {
            me.handle_menu_event(event, ctx);
        });

        let editor = ctx.add_typed_action_view(|ctx| {
            let appearance = Appearance::as_ref(ctx);
            EditorView::single_line(
                SingleLineEditorOptions {
                    text: TextOptions::ui_text(Some(appearance.ui_font_size()), appearance),
                    select_all_on_focus: true,
                    clear_selections_on_blur: true,
                    ..Default::default()
                },
                ctx,
            )
            .with_text_alignment(TextAlignment::Center)
        });
        ctx.subscribe_to_view(&editor, |me, _, event, ctx| {
            me.handle_editor_event(event, ctx);
        });

        Self {
            is_expanded: false,
            is_valid: true,
            suppress_next_blur_commit: false,
            top_bar_mouse_state: Default::default(),
            top_bar_max_width: TOP_MENU_BAR_MAX_WIDTH,
            top_bar_height: TOP_MENU_BAR_HEIGHT,
            vertical_margin: DROPDOWN_PADDING,
            dropdown,
            editor,
            selected_item: None,
            parser: None,
            revert_text: None,
            _action_type: PhantomData,
            #[cfg(test)]
            last_dispatched_action: None,
        }
    }

    pub fn set_validation<P, R>(&mut self, parser: P, revert_text: R)
    where
        P: Fn(&str) -> Option<A> + 'static,
        R: Fn(&AppContext) -> String + 'static,
    {
        self.parser = Some(Box::new(parser));
        self.revert_text = Some(Box::new(revert_text));
    }

    pub fn set_placeholder(&mut self, placeholder: impl Into<String>, ctx: &mut ViewContext<Self>) {
        self.editor.update(ctx, |editor, ctx| {
            editor.set_placeholder_text(placeholder, ctx);
        });
    }

    pub fn set_value_text(&mut self, text: &str, ctx: &mut ViewContext<Self>) {
        self.editor.update(ctx, |editor, ctx| {
            editor.system_reset_buffer_text(text, ctx);
        });
        self.is_valid = true;
        ctx.notify();
    }

    #[allow(dead_code)]
    pub fn add_items(&mut self, items: Vec<DropdownItem<A>>, ctx: &mut ViewContext<Self>) {
        self.dropdown.update(ctx, |dropdown, ctx| {
            dropdown.add_items(items.iter().map(|item| item.into()));
            ctx.notify();
        });
        ctx.notify();
    }

    pub fn set_items(&mut self, items: Vec<DropdownItem<A>>, ctx: &mut ViewContext<Self>) {
        self.dropdown.update(ctx, |dropdown, ctx| {
            dropdown.set_items(items.iter().map(|item| item.into()), ctx);
        });
        self.selected_item = None;
        ctx.notify();
    }

    #[allow(dead_code)]
    pub fn set_selected_by_name(
        &mut self,
        selected_item: impl AsRef<str>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.dropdown.update(ctx, |dropdown, ctx| {
            dropdown.set_selected_by_name(selected_item, ctx);
        });
        self.sync_selected_item(ctx);
    }

    #[allow(dead_code)]
    pub fn set_selected_by_index(&mut self, selected_index: usize, ctx: &mut ViewContext<Self>) {
        self.dropdown.update(ctx, |dropdown, ctx| {
            dropdown.set_selected_by_index(selected_index, ctx);
        });
        self.sync_selected_item(ctx);
    }

    pub fn set_selected_by_action(&mut self, action: A, ctx: &mut ViewContext<Self>) {
        let action = DropdownAction::SelectActionAndClose(Box::new(action));
        self.dropdown.update(ctx, |dropdown, ctx| {
            dropdown.set_selected_by_action(&action, ctx);
        });
        self.sync_selected_item(ctx);
    }

    pub fn set_top_bar_max_width(&mut self, max_width: f32) {
        self.top_bar_max_width = max_width;
    }

    pub fn set_menu_width(&mut self, width: f32, ctx: &mut ViewContext<Self>) {
        self.dropdown.update(ctx, |menu, ctx| {
            menu.set_width(width);
            ctx.notify();
        });
    }

    fn selected_item(&self, ctx: &mut ViewContext<Self>) -> Option<MenuItem<DropdownAction>> {
        self.dropdown
            .read(ctx, |dropdown, _| dropdown.selected_item())
    }

    fn selected_item_label(&self) -> Option<String> {
        match self.selected_item.as_ref() {
            Some(MenuItem::Item(fields)) => Some(fields.label().to_string()),
            _ => None,
        }
    }

    fn sync_selected_item(&mut self, ctx: &mut ViewContext<Self>) {
        self.selected_item = self.selected_item(ctx);
        if let Some(label) = self.selected_item_label() {
            self.set_value_text(&label, ctx);
        } else {
            ctx.notify();
        }
    }

    fn current_action(&self, ctx: &AppContext) -> Option<A> {
        let parser = self.parser.as_ref()?;
        parser(&self.editor.as_ref(ctx).buffer_text(ctx))
    }

    fn revert(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(revert_text) = self.revert_text.as_ref() else {
            return;
        };
        let text = revert_text(ctx);
        self.set_value_text(&text, ctx);
    }

    fn dispatch_action(&mut self, action: &dyn DropdownItemAction, ctx: &mut ViewContext<Self>) {
        #[cfg(test)]
        {
            self.last_dispatched_action = Some(action.clone_box());
        }
        ctx.dispatch_typed_action(action);
    }

    fn commit_or_revert(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(action) = self.current_action(ctx) {
            self.is_valid = true;
            self.dispatch_action(&action, ctx);
        } else {
            self.revert(ctx);
            ctx.focus(&self.editor);
        }
        ctx.notify();
    }

    fn handle_editor_event(&mut self, event: &EditorEvent, ctx: &mut ViewContext<Self>) {
        match event {
            EditorEvent::Edited(EditOrigin::UserTyped | EditOrigin::UserInitiated) => {
                self.suppress_next_blur_commit = false;
                self.is_valid = self.current_action(ctx).is_some();
                ctx.notify();
            }
            EditorEvent::Enter => self.commit_or_revert(ctx),
            EditorEvent::Blurred if self.suppress_next_blur_commit => {
                self.suppress_next_blur_commit = false;
            }
            EditorEvent::Blurred => self.commit_or_revert(ctx),
            EditorEvent::Escape => {
                self.suppress_next_blur_commit = true;
                self.revert(ctx);
                self.close(ctx);
                ctx.emit(EditableDropdownEvent::Escape);
            }
            _ => {}
        }
    }

    fn handle_menu_event(&mut self, event: &MenuEvent, ctx: &mut ViewContext<Self>) {
        match event {
            MenuEvent::Close { via_select_item: _ } => self.close(ctx),
            // Arrow-key navigation changes the menu's highlighted item and emits
            // `ItemSelected` before the user commits it. The editor stays tied
            // to the saved value until the preset action is actually dispatched
            // and the backing setting synchronizes the component.
            MenuEvent::ItemSelected => {}
            MenuEvent::ItemHovered => {}
        }
    }

    fn select_action_and_close(
        &mut self,
        action: &dyn DropdownItemAction,
        ctx: &mut ViewContext<Self>,
    ) {
        self.dispatch_action(action, ctx);
        self.close(ctx);
    }

    fn close(&mut self, ctx: &mut ViewContext<Self>) {
        self.is_expanded = false;
        ctx.emit(EditableDropdownEvent::Close);
        ctx.notify();
    }

    fn toggle_expanded(&mut self, ctx: &mut ViewContext<Self>) {
        self.is_expanded = !self.is_expanded;
        if self.is_expanded {
            ctx.focus(&self.dropdown);
            ctx.emit(EditableDropdownEvent::ToggleExpanded);
        }
        ctx.notify();
    }

    fn focus(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.focus(&self.editor);
        ctx.notify();
    }

    fn top_bar_label(&self) -> String {
        format!("editable_dropdown_top_bar_{}", self.dropdown.id())
    }

    fn render_top_bar(&self, appearance: &Appearance) -> Box<dyn Element> {
        let editor =
            Container::new(Clipped::new(ChildView::new(&self.editor).finish()).finish()).finish();

        let chevron = ConstrainedBox::new(
            Icon::new(
                "bundled/svg/chevron-down.svg",
                appearance.theme().active_ui_text_color(),
            )
            .finish(),
        )
        .with_width(15.)
        .with_height(15.)
        .finish();
        let mut button = appearance
            .ui_builder()
            .button(ButtonVariant::Text, self.top_bar_mouse_state.clone())
            .with_custom_label(chevron)
            .set_clicked_styles(None)
            .with_style(UiComponentStyles {
                width: Some(28.),
                height: Some(self.top_bar_height),
                padding: Some(Coords::uniform(6.)),
                border_radius: Some(CornerRadius::with_all(Radius::Pixels(3.))),
                ..Default::default()
            })
            .with_hovered_styles(UiComponentStyles {
                background: Some(appearance.theme().surface_3().into()),
                ..Default::default()
            })
            .build();
        if !self.is_expanded {
            button = button.on_click(|ctx, _, _| {
                ctx.dispatch_typed_action(DropdownAction::ToggleExpanded);
            });
        }

        let mut control = Stack::new()
            .with_event_dispatch_mode(EventDispatchMode::Waterfall)
            .with_child(
                ConstrainedBox::new(editor)
                    .with_width(self.top_bar_max_width)
                    .with_height(self.top_bar_height)
                    .finish(),
            );
        control.add_positioned_child(
            button.finish(),
            OffsetPositioning::offset_from_parent(
                vec2f(0., 0.),
                ParentOffsetBounds::ParentByPosition,
                ParentAnchor::MiddleRight,
                ChildAnchor::MiddleRight,
            ),
        );

        let border_fill = self
            .validation_border_fill()
            .unwrap_or_else(|| appearance.theme().outline());
        let top_bar = Container::new(control.finish())
            .with_background(appearance.theme().surface_2())
            .with_border(Border::all(1.).with_border_fill(border_fill))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
            .finish();

        SavePosition::new(
            ConstrainedBox::new(top_bar)
                .with_width(self.top_bar_max_width)
                .with_height(self.top_bar_height)
                .finish(),
            &self.top_bar_label(),
        )
        .finish()
    }

    fn validation_border_fill(&self) -> Option<theme::Fill> {
        (!self.is_valid).then(theme::Fill::error)
    }
    #[cfg(test)]
    fn editor_text(&self, ctx: &AppContext) -> String {
        self.editor.as_ref(ctx).buffer_text(ctx)
    }

    #[cfg(test)]
    fn set_editor_text(&mut self, text: &str, ctx: &mut ViewContext<Self>) {
        self.editor.update(ctx, |editor, ctx| {
            editor.system_reset_buffer_text(text, ctx);
        });
    }

    #[cfg(test)]
    fn items_len(&self, ctx: &AppContext) -> usize {
        self.dropdown.as_ref(ctx).items_len()
    }

    #[cfg(test)]
    fn item_labels(&self, ctx: &AppContext) -> Vec<String> {
        self.dropdown
            .as_ref(ctx)
            .items()
            .iter()
            .filter_map(|item| match item {
                MenuItem::Item(fields) => Some(fields.label().to_string()),
                _ => None,
            })
            .collect()
    }

    #[cfg(test)]
    fn last_dispatched_action(&self) -> Option<&A> {
        self.last_dispatched_action
            .as_ref()
            .and_then(|action| action.as_any().downcast_ref())
    }
}

impl<A> Entity for EditableDropdown<A>
where
    A: DropdownItemAction,
{
    type Event = EditableDropdownEvent;
}

impl<A> TypedActionView for EditableDropdown<A>
where
    A: DropdownItemAction,
{
    type Action = DropdownAction;

    fn handle_action(&mut self, action: &DropdownAction, ctx: &mut ViewContext<Self>) {
        match action {
            DropdownAction::Focus(_) => self.focus(ctx),
            DropdownAction::Close => self.close(ctx),
            DropdownAction::SelectActionAndClose(action) => {
                self.select_action_and_close(action.as_ref(), ctx)
            }
            DropdownAction::ToggleExpanded => self.toggle_expanded(ctx),
        }
    }
}

impl<A> View for EditableDropdown<A>
where
    A: DropdownItemAction,
{
    fn ui_name() -> &'static str {
        "EditableDropdown"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let mut dropdown_stack = Stack::new().with_child(self.render_top_bar(appearance));
        if self.is_expanded {
            dropdown_stack.add_positioned_overlay_child(
                ChildView::new(&self.dropdown).finish(),
                OffsetPositioning::offset_from_save_position_element(
                    self.top_bar_label(),
                    vec2f(0., 0.),
                    PositionedElementOffsetBounds::WindowByPosition,
                    PositionedElementAnchor::BottomRight,
                    ChildAnchor::TopRight,
                ),
            );
        }
        Container::new(dropdown_stack.finish())
            .with_margin_top(self.vertical_margin)
            .with_margin_bottom(self.vertical_margin)
            .finish()
    }

    fn on_blur(&mut self, blur_ctx: &BlurContext, ctx: &mut ViewContext<Self>) {
        if blur_ctx.is_self_blurred() {
            self.close(ctx);
        }
    }
}
