use std::marker::PhantomData;

use pathfinder_geometry::vector::vec2f;
use warpui::elements::{
    Border, ChildAnchor, ConstrainedBox, CornerRadius, CrossAxisAlignment, Flex,
    Icon as WarpUiIcon, MainAxisAlignment, MouseStateHandle, OffsetPositioning, ParentElement,
    PositionedElementAnchor, PositionedElementOffsetBounds, Radius, SavePosition, Stack,
};
use warpui::presenter::ChildView;
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::{
    AppContext, BlurContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle,
};

use super::dropdown::{DropdownAction, DropdownItemAction};
use crate::appearance::Appearance;
use crate::menu::{Event as MenuEvent, Menu, MenuItem, MenuItemFields, MenuVariant};
use crate::themes::theme::Fill;
use crate::ui_components::icons::Icon;

#[cfg(test)]
#[path = "compact_dropdown_tests.rs"]
mod tests;

/// A compact dropdown view. Each item has a corresponding icon, which is shown
/// when the dropdown is closed.
///
/// This is useful instead of [`crate::dropdown::Dropdown`] when showing a
/// dropdown alongside other controls, such as in a formatting UI.
pub struct CompactDropdown<A: DropdownItemAction = ()> {
    /// Whether the dropdown is open.
    is_expanded: bool,
    /// Mouse state for the dropdown button.
    top_bar_mouse_state: MouseStateHandle,
    /// Dropdown menu.
    dropdown: ViewHandle<Menu<DropdownAction>>,
    /// The size that icons are scaled to. Defaults to the UI font size if not set.
    icon_size: Option<f32>,
    _action_type: PhantomData<A>,
}

pub struct CompactDropdownItem<A: DropdownItemAction = ()> {
    /// Icon identifier for this item.
    icon: Icon,
    /// Optional override color for the icon.
    icon_color: Option<Fill>,
    /// Text to display for this item when the dropdown is open.
    display_text: String,
    /// Typed action dispatched when this item is selected.
    action: A,
}

impl<A: DropdownItemAction> CompactDropdown<A> {
    /// Create a new, empty compact dropdown. The [`MenuVariant`] determines whether or not the
    /// dropdown is scrollable when expanded.
    pub fn new(menu_variant: MenuVariant, ctx: &mut ViewContext<Self>) -> Self {
        let dropdown = ctx.add_typed_action_view(|ctx| {
            let theme = Appearance::as_ref(ctx).theme();
            Menu::new()
                .with_menu_variant(menu_variant)
                .with_border(Border::all(1.).with_border_fill(theme.outline()))
                .prevent_interaction_with_other_elements()
        });

        ctx.subscribe_to_view(&dropdown, move |me, _, event, ctx| {
            me.handle_menu_event(event, ctx);
        });

        Self {
            is_expanded: false,
            dropdown,
            top_bar_mouse_state: Default::default(),
            icon_size: None,
            _action_type: PhantomData,
        }
    }

    /// Sets the size of the icons in the dropdown top bar. This defaults
    /// to the UI font size.
    pub fn set_icon_size(&mut self, icon_size: f32) {
        self.icon_size = Some(icon_size);
    }

    /// Replaces the items in the dropdown.
    pub fn set_items(
        &mut self,
        items: impl IntoIterator<Item = CompactDropdownItem<A>>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.dropdown.update(ctx, |dropdown, ctx| {
            dropdown.set_items(items.into_iter().map(CompactDropdownItem::menu_item), ctx);
        });
        ctx.notify();
    }

    /// Change the selected item by name, if it exists.
    pub fn set_selected_by_name(
        &mut self,
        selected_item: impl AsRef<str>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.dropdown.update(ctx, |dropdown, ctx| {
            dropdown.set_selected_by_name(selected_item, ctx);
            ctx.notify();
        });
        ctx.notify();
    }

    /// Render an icon at the configured icon size.
    fn render_sized_icon(&self, appearance: &Appearance, icon: WarpUiIcon) -> Box<dyn Element> {
        let icon_size = self.icon_size.unwrap_or(appearance.ui_font_size());
        ConstrainedBox::new(icon.finish())
            .with_width(icon_size)
            .with_height(icon_size)
            .finish()
    }

    /// Render the top bar, the part of the dropdown that is always visible.
    fn render_top_bar(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);

        let mut button_label = Flex::row()
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Center);

        if let Some(MenuItem::Item(fields)) = self.dropdown.as_ref(app).selected_item() {
            if let Some(icon) = fields.icon() {
                let icon_color = fields
                    .override_icon_color()
                    .unwrap_or_else(|| appearance.theme().active_ui_text_color());
                button_label
                    .add_child(self.render_sized_icon(appearance, icon.to_warpui_icon(icon_color)));
            }
        }

        button_label.add_child(self.render_sized_icon(
            appearance,
            WarpUiIcon::new(
                "bundled/svg/chevron-down.svg",
                appearance.theme().active_ui_text_color(),
            ),
        ));

        let mut top_bar = appearance
            .ui_builder()
            .button(ButtonVariant::Text, self.top_bar_mouse_state.clone())
            .with_custom_label(button_label.finish())
            .set_clicked_styles(None)
            .with_style(UiComponentStyles {
                padding: Some(Coords::uniform(4.)),
                border_radius: Some(CornerRadius::with_all(Radius::Pixels(4.))),
                ..Default::default()
            })
            .with_hovered_styles(UiComponentStyles {
                background: Some(appearance.theme().surface_3().into()),
                ..Default::default()
            })
            .build();

        // See the Dropdown implementation for why this callback is only added
        // if the dropdown is not expanded.
        if !self.is_expanded {
            top_bar = top_bar.on_click(|ctx, _, _| {
                ctx.dispatch_typed_action(DropdownAction::ToggleExpanded);
            });
        }

        SavePosition::new(top_bar.finish(), &self.top_bar_label()).finish()
    }

    /// Saved position label for the top bar, used to position the expanded menu.
    fn top_bar_label(&self) -> String {
        format!("compact_dropdown_top_bar_{}", self.dropdown.id())
    }

    fn handle_menu_event(&mut self, event: &MenuEvent, ctx: &mut ViewContext<Self>) {
        match event {
            MenuEvent::Close { via_select_item: _ } => self.close(ctx),
            MenuEvent::ItemSelected => {
                // If the selection changes, we should re-render, but don't need
                // to do anything else unless the item is actively clicked.
                ctx.notify();
            }
            MenuEvent::ItemHovered => {}
        }
    }

    /// Toggles whether or not the dropdown is expanded.
    fn toggle_expanded(&mut self, ctx: &mut ViewContext<Self>) {
        self.is_expanded = !self.is_expanded;
        if self.is_expanded {
            ctx.focus(&self.dropdown);
        }
        ctx.notify();
    }

    fn focus(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.focus(&self.dropdown);
        ctx.notify();
    }

    /// Adapter between [`MenuItem`] click callbacks and the parent action type.
    /// When a dropdown menu item is selected, we dispatch its action and close
    /// the dropdown.
    fn select_action_and_close(
        &mut self,
        action: &dyn DropdownItemAction,
        ctx: &mut ViewContext<Self>,
    ) {
        ctx.dispatch_typed_action(action);
        self.close(ctx);
    }

    /// Close the dropdown.
    fn close(&mut self, ctx: &mut ViewContext<Self>) {
        self.is_expanded = false;
        ctx.notify();
        ctx.emit(CompactDropdownEvent::Close);
    }
}

impl<A: DropdownItemAction> Entity for CompactDropdown<A> {
    type Event = CompactDropdownEvent;
}

impl<A: DropdownItemAction> View for CompactDropdown<A> {
    fn ui_name() -> &'static str {
        "CompactDropdown"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let mut dropdown_stack = Stack::new().with_child(self.render_top_bar(app));

        if self.is_expanded {
            dropdown_stack.add_positioned_overlay_child(
                ChildView::new(&self.dropdown).finish(),
                OffsetPositioning::offset_from_save_position_element(
                    self.top_bar_label(),
                    vec2f(0., 0.),
                    PositionedElementOffsetBounds::WindowByPosition,
                    PositionedElementAnchor::BottomLeft,
                    ChildAnchor::TopLeft,
                ),
            );
        }

        dropdown_stack.finish()
    }

    fn on_blur(&mut self, blur_ctx: &BlurContext, ctx: &mut ViewContext<Self>) {
        if blur_ctx.is_self_blurred() {
            self.close(ctx);
        }
    }
}

impl<A: DropdownItemAction> TypedActionView for CompactDropdown<A> {
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

impl<A: DropdownItemAction> CompactDropdownItem<A> {
    pub fn new(icon: Icon, display_text: impl Into<String>, action: A) -> Self {
        Self {
            display_text: display_text.into(),
            icon,
            icon_color: None,
            action,
        }
    }

    /// Override the fill of the item's icon. If not set, the default is to
    /// match the active text color.
    pub fn with_icon_color(mut self, color: Fill) -> Self {
        self.icon_color = Some(color);
        self
    }

    fn menu_item(self) -> MenuItem<DropdownAction> {
        let mut item = MenuItemFields::new(self.display_text)
            .with_icon(self.icon)
            .with_on_select_action(DropdownAction::SelectActionAndClose(Box::new(self.action)));
        if let Some(color) = self.icon_color {
            item = item.with_override_icon_color(color);
        }
        item.into_item()
    }
}

/// Events sent from the [`CompactDropdown`] to its parent.
pub enum CompactDropdownEvent {
    /// Sent when the dropdown is closed. Generally, the parent view will take back focus when this happens.
    Close,
}
