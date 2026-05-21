//! This module is similar to `requested_actions.rs` but specifically for scripts instead of commands.
//! Eventually, we'll probably want to refactor the functions here to lean on Views instead.

use std::rc::Rc;

use pathfinder_geometry::vector::Vector2F;
use warp_core::ui::appearance::Appearance;
use warpui::elements::{
    Align, Border, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Flex, Hoverable,
    MainAxisAlignment, MainAxisSize, MouseStateHandle, ParentElement, Radius, Shrinkable, Text,
    Wrap,
};
use warpui::keymap::Keystroke;
use warpui::platform::Cursor;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::ui_components::toggle_menu::{
    ToggleMenuCallback, ToggleMenuItem, ToggleMenuStateHandle,
};
use warpui::{AppContext, Element, EventContext, SingletonEntity};

use super::inline_action_header::{
    INLINE_ACTION_HEADER_VERTICAL_PADDING, INLINE_ACTION_HORIZONTAL_PADDING,
};
use super::requested_action::render_header_buttons;
use crate::ai::blocklist::block::view_impl::WithContentItemSpacing;
use crate::ai::blocklist::inline_action::inline_action_icons::icon_size;
use crate::ai::blocklist::inline_action::requested_action::render_requested_action_row_for_text;
use crate::ui_components::icons::Icon;

pub struct TitledScript {
    pub title: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestedScriptStatus {
    /// The action is loaded and is waiting to be acted upon by the user.
    WaitingForUser,
    /// The action is currently running, after being accepted by the user.
    Running,
}

#[derive(Default, Clone)]
pub struct RequestedScriptMouseStates {
    pub show_hide_button: MouseStateHandle,
    pub cancel_button: MouseStateHandle,
    pub run_button: MouseStateHandle,
}

/// Creates a requested script element that gives the option to hide or show the script.
#[allow(clippy::too_many_arguments)]
pub fn render_requested_script(
    header: &str,
    script: &str,
    status: RequestedScriptStatus,
    is_collapsed: bool,
    is_viewing_detail: bool,
    on_toggle_expanded: impl FnMut(&mut EventContext, &AppContext, Vector2F) + 'static,
    on_accept: impl Fn(&mut EventContext) + 'static,
    on_cancel: impl Fn(&mut EventContext) + 'static,
    run_keystroke: &Keystroke,
    cancel_keystroke: &Keystroke,
    button_states: &RequestedScriptMouseStates,
    should_highlight_border: bool,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let mut content = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);

    let is_executing = matches!(status, RequestedScriptStatus::Running);
    let should_show_accept_button = matches!(status, RequestedScriptStatus::WaitingForUser);

    // The current status of the script: whether it is executing, expanded, or collapsed.
    let (action_text, icon) = script_status(is_executing, is_collapsed, is_viewing_detail, app);

    content.add_child(render_header(
        Text::new(
            header.to_string(),
            appearance.ui_font_family(),
            appearance.monospace_font_size(),
        )
        .with_color(theme.main_text_color(theme.surface_1()).into())
        .finish(),
        on_accept,
        on_cancel,
        run_keystroke,
        cancel_keystroke,
        button_states,
        should_show_accept_button,
        !is_executing,
        app,
    ));

    content.add_child(
        Hoverable::new(button_states.show_hide_button.clone(), |_| {
            render_requested_action_row_for_text(
                action_text.into(),
                appearance.ui_font_family(),
                Some(icon),
                None,
                false,
                true,
                app,
            )
        })
        .on_click(on_toggle_expanded)
        .with_cursor(Cursor::PointingHand)
        .finish(),
    );

    // If expanded, the contents of the script.
    if !is_collapsed {
        content.add_child(render_requested_action_row_for_text(
            script.into(),
            appearance.ui_font_family(),
            None,
            None,
            false,
            true,
            app,
        ));
    }

    let border_color =
        if should_highlight_border && matches!(status, RequestedScriptStatus::WaitingForUser) {
            theme.accent().into_solid()
        } else {
            theme.surface_2().into_solid()
        };

    content
        .finish()
        .with_content_item_spacing()
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
        .with_background_color(theme.background().into_solid())
        .with_border(Border::all(1.).with_border_fill(border_color))
        .finish()
}

/// Creates a requested script element that gives the option to hide or show the scripts.
#[allow(clippy::too_many_arguments)]
pub fn render_requested_scripts(
    first_script: TitledScript,
    second_script: TitledScript,
    is_first_script_active: bool,
    status: RequestedScriptStatus,
    is_collapsed: bool,
    is_viewing_detail: bool,
    on_toggle_expanded: impl FnMut(&mut EventContext, &AppContext, Vector2F) + 'static,
    on_accept: impl Fn(&mut EventContext) + 'static,
    on_cancel: impl Fn(&mut EventContext) + 'static,
    run_keystroke: &Keystroke,
    cancel_keystroke: &Keystroke,
    button_states: &RequestedScriptMouseStates,
    toggle_menu_mouse_states: Vec<MouseStateHandle>,
    toggle_menu_state_handle: ToggleMenuStateHandle,
    on_toggle_change: Rc<ToggleMenuCallback>,
    should_highlight_border: bool,
    toggle_menu_width: f32,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let mut content = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);

    let is_executing = matches!(status, RequestedScriptStatus::Running);
    let should_show_accept_button = matches!(status, RequestedScriptStatus::WaitingForUser);

    // The current status of the script: whether it is executing, expanded, or collapsed.
    let (action_text, icon) = script_status(is_executing, is_collapsed, is_viewing_detail, app);

    let selected_color = theme.surface_3();
    let toggle_menu: Box<dyn Element> = ConstrainedBox::new(
        appearance
            .ui_builder()
            .toggle_menu(
                toggle_menu_mouse_states,
                vec![
                    ToggleMenuItem::new(first_script.title.clone()),
                    ToggleMenuItem::new(second_script.title.clone()),
                ],
                toggle_menu_state_handle,
                Some((!is_first_script_active).into()),
                None,
                None,
                None,
                appearance.ui_font_size(),
                on_toggle_change,
            )
            .with_style(UiComponentStyles {
                padding: Some(Coords {
                    top: 10.,
                    right: 12.,
                    bottom: 10.,
                    left: 12.,
                }),
                border_radius: Some(CornerRadius::with_all(Radius::Pixels(0.))),
                background: Some(theme.background().into()),
                border_color: Some(selected_color.into()),
                margin: Some(Coords {
                    top: 0.,
                    right: 0.,
                    bottom: 0.,
                    left: 0.,
                }),
                ..Default::default()
            })
            .with_disabled(is_executing)
            .build()
            .with_uniform_padding(0.)
            .with_border(Border::all(2.).with_border_fill(selected_color.into_solid()))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
            .finish(),
    )
    .with_width(toggle_menu_width)
    .finish();

    content.add_child(render_header(
        toggle_menu,
        on_accept,
        on_cancel,
        run_keystroke,
        cancel_keystroke,
        button_states,
        should_show_accept_button,
        !is_executing,
        app,
    ));

    content.add_child(
        Hoverable::new(button_states.show_hide_button.clone(), |_| {
            render_requested_action_row_for_text(
                action_text.into(),
                appearance.ui_font_family(),
                Some(icon),
                None,
                false,
                true,
                app,
            )
        })
        .on_click(on_toggle_expanded)
        .with_cursor(Cursor::PointingHand)
        .finish(),
    );

    // If expanded, the contents of the script.
    if !is_collapsed {
        let script = if is_first_script_active {
            first_script.content
        } else {
            second_script.content
        };
        content.add_child(render_requested_action_row_for_text(
            script.into(),
            appearance.ui_font_family(),
            None,
            None,
            false,
            true,
            app,
        ));
    }

    let border_color =
        if should_highlight_border && matches!(status, RequestedScriptStatus::WaitingForUser) {
            theme.accent().into_solid()
        } else {
            theme.surface_2().into_solid()
        };

    content
        .finish()
        .with_content_item_spacing()
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
        .with_background_color(theme.background().into_solid())
        .with_border(Border::all(1.).with_border_fill(border_color))
        .finish()
}

/// Renders the header for the requested script, with optional action buttons.
#[allow(clippy::too_many_arguments)]
fn render_header(
    header: Box<dyn Element>,
    on_accept: impl Fn(&mut EventContext) + 'static,
    on_cancel: impl Fn(&mut EventContext) + 'static,
    run_keystroke: &Keystroke,
    cancel_keystroke: &Keystroke,
    mouse_states: &RequestedScriptMouseStates,
    should_show_accept_button: bool,
    is_interactive: bool,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();

    let mut row = Wrap::row()
        .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
        .with_run_spacing(8.)
        .with_main_axis_size(MainAxisSize::Max)
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(Shrinkable::new(3., header).finish());

    if is_interactive {
        row.add_child(
            Shrinkable::new(
                1.,
                Align::new(render_header_buttons(
                    on_accept,
                    on_cancel,
                    run_keystroke,
                    cancel_keystroke,
                    &mouse_states.run_button,
                    &mouse_states.cancel_button,
                    should_show_accept_button,
                    app,
                ))
                .right()
                .finish(),
            )
            .finish(),
        );
    }

    Container::new(row.finish())
        .with_vertical_padding(INLINE_ACTION_HEADER_VERTICAL_PADDING)
        .with_horizontal_padding(INLINE_ACTION_HORIZONTAL_PADDING)
        .with_background(theme.surface_1())
        .with_corner_radius(CornerRadius::with_top(Radius::Pixels(8.)))
        .finish()
}

/// Helper function to determine the correct contents for the first row in the requested script
/// element. The contents consist of a label and an icon.
fn script_status(
    is_executing: bool,
    is_collapsed: bool,
    is_viewing_detail: bool,
    app: &AppContext,
) -> (&str, Box<dyn Element>) {
    let appearance = Appearance::as_ref(app);
    let label = match (is_executing, is_collapsed) {
        (true, _) => "Running...",
        (false, true) => "Expand to show script",
        (false, false) => "Hide",
    };
    let is_expanded = (is_executing && is_viewing_detail) || (!is_executing && !is_collapsed);
    let icon = ConstrainedBox::new(
        warpui::elements::Icon::new(
            if is_expanded {
                Icon::ChevronDown.into()
            } else {
                Icon::ChevronRight.into()
            },
            appearance.theme().foreground(),
        )
        .finish(),
    )
    .with_width(icon_size(app))
    .with_height(icon_size(app))
    .finish();
    (label, icon)
}
