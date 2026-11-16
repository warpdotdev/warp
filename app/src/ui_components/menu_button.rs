use pathfinder_geometry::vector::{Vector2F, vec2f};
use warpui::elements::{
    ChildAnchor, ChildView, MouseStateHandle, OffsetPositioning, ParentAnchor, ParentElement,
    ParentOffsetBounds, Stack,
};
use warpui::platform::Cursor;
use warpui::ui_components::components::{UiComponent, UiComponentStyles};
use warpui::{AppContext, Element, EventContext, View, ViewHandle};

use super::buttons::{highlight, icon_button};
use super::icons::Icon;
use crate::appearance::Appearance;

#[derive(Clone, Copy)]
pub enum MenuDirection {
    Left, // Menu is left of the "..." button icon
    Right,
}

#[allow(clippy::too_many_arguments)]
pub fn icon_button_with_context_menu<F, V: View>(
    icon: Icon,
    on_click_action: F,
    mouse_state_handle: MouseStateHandle,
    context_menu: &ViewHandle<V>,
    is_menu_open: bool,
    menu_direction: MenuDirection,
    cursor: Option<Cursor>,
    style: Option<UiComponentStyles>,
    appearance: &Appearance,
) -> Stack
where
    F: 'static + FnMut(&mut EventContext, &AppContext, Vector2F),
{
    let mut button = icon_button(appearance, icon, is_menu_open, mouse_state_handle);
    if let Some(style) = style {
        button = button.with_style(style);
    }

    let mut button_with_menu = Stack::new().with_child(
        button
            .with_cursor(cursor)
            .build()
            .on_click(on_click_action)
            .finish(),
    );

    if is_menu_open {
        button_with_menu.add_positioned_overlay_child(
            ChildView::new(context_menu).finish(),
            offset_positioning(menu_direction),
        );
    }

    button_with_menu
}

pub fn highlight_icon_button_with_context_menu<F, V: View>(
    icon: Icon,
    on_click_action: F,
    mouse_state_handle: MouseStateHandle,
    context_menu: &ViewHandle<V>,
    is_menu_open: bool,
    menu_direction: MenuDirection,
    appearance: &Appearance,
) -> Stack
where
    F: 'static + FnMut(&mut EventContext, &AppContext, Vector2F),
{
    let mut button_with_menu = Stack::new().with_child(
        highlight(
            icon_button(appearance, icon, is_menu_open, mouse_state_handle),
            appearance,
        )
        .build()
        .on_click(on_click_action)
        .finish(),
    );

    if is_menu_open {
        button_with_menu.add_positioned_overlay_child(
            ChildView::new(context_menu).finish(),
            offset_positioning(menu_direction),
        );
    }

    button_with_menu
}

/// Variant with surface_1 hover background for Warp Drive items
#[allow(clippy::too_many_arguments)]
pub fn icon_button_with_context_menu_drive<F, V: View>(
    icon: Icon,
    on_click_action: F,
    mouse_state_handle: MouseStateHandle,
    context_menu: &ViewHandle<V>,
    is_menu_open: bool,
    menu_direction: MenuDirection,
    cursor: Option<Cursor>,
    appearance: &Appearance,
) -> Stack
where
    F: 'static + FnMut(&mut EventContext, &AppContext, Vector2F),
{
    let button = icon_button(appearance, icon, is_menu_open, mouse_state_handle)
        .with_hovered_styles(
            warpui::ui_components::components::UiComponentStyles::default()
                .set_background(appearance.theme().surface_1().into())
                .set_border_color(appearance.theme().surface_3().into()),
        )
        .with_cursor(cursor);

    let mut button_with_menu =
        Stack::new().with_child(button.build().on_click(on_click_action).finish());

    if is_menu_open {
        button_with_menu.add_positioned_overlay_child(
            ChildView::new(context_menu).finish(),
            offset_positioning(menu_direction),
        );
    }

    button_with_menu
}

/// Variant with surface_1 hover background for Warp Drive items (highlighted)
pub fn highlight_icon_button_with_context_menu_drive<F, V: View>(
    icon: Icon,
    on_click_action: F,
    mouse_state_handle: MouseStateHandle,
    context_menu: &ViewHandle<V>,
    is_menu_open: bool,
    menu_direction: MenuDirection,
    appearance: &Appearance,
) -> Stack
where
    F: 'static + FnMut(&mut EventContext, &AppContext, Vector2F),
{
    let button = highlight(
        icon_button(appearance, icon, is_menu_open, mouse_state_handle),
        appearance,
    )
    .with_hovered_styles(
        warpui::ui_components::components::UiComponentStyles::default()
            .set_background(appearance.theme().surface_1().into())
            .set_border_color(appearance.theme().surface_3().into()),
    );

    let mut button_with_menu =
        Stack::new().with_child(button.build().on_click(on_click_action).finish());

    if is_menu_open {
        button_with_menu.add_positioned_overlay_child(
            ChildView::new(context_menu).finish(),
            offset_positioning(menu_direction),
        );
    }

    button_with_menu
}

fn offset_positioning(menu_direction: MenuDirection) -> OffsetPositioning {
    match menu_direction {
        MenuDirection::Left => OffsetPositioning::offset_from_parent(
            vec2f(0., 0.),
            ParentOffsetBounds::WindowByPosition,
            ParentAnchor::TopLeft,
            ChildAnchor::TopRight,
        ),
        MenuDirection::Right => OffsetPositioning::offset_from_parent(
            vec2f(0., 0.),
            ParentOffsetBounds::WindowByPosition,
            ParentAnchor::TopRight,
            ChildAnchor::TopLeft,
        ),
    }
}
