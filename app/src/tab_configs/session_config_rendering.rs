use std::path::Path;
use std::sync::Arc;

use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use warp_core::ui::theme::{Fill, WarpTheme};
use warpui::elements::{
    Border, ChildAnchor, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Expanded,
    Flex, Hoverable, MainAxisAlignment, MainAxisSize, MouseStateHandle, OffsetPositioning,
    ParentAnchor, ParentElement, ParentOffsetBounds, Radius, Stack, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::geometry::vector::Vector2F;
use warpui::platform::Cursor;
use warpui::ui_components::components::UiComponent;
use warpui::{Element, EventContext};

use crate::appearance::Appearance;
use crate::tab_configs::session_config::SessionType;
use crate::ui_components::blended_colors;
use crate::view_components::callout_bubble::{
    callout_checkbox, callout_label_color, phenomenon_accent_color, phenomenon_background_color,
    phenomenon_body_text_color, phenomenon_disabled_label_text_color, phenomenon_foreground_color,
    phenomenon_subtle_border_color,
};

const PILL_GAP: f32 = 8.;

fn session_type_item_color(
    is_selected: bool,
    on_accent_bg: bool,
    theme: &WarpTheme,
    bg_fill: Fill,
) -> ColorU {
    if on_accent_bg {
        if is_selected {
            phenomenon_background_color()
        } else {
            phenomenon_body_text_color()
        }
    } else if is_selected {
        blended_colors::text_main(theme, bg_fill)
    } else {
        blended_colors::text_sub(theme, bg_fill)
    }
}

/// Renders the session type pill selector.
///
/// Each pill dispatches `on_select(index)` when clicked.
pub fn render_session_type_pills<F>(
    session_types: &[SessionType],
    selected_index: usize,
    pill_mouse_states: &[MouseStateHandle],
    on_select: F,
    appearance: &Appearance,
) -> Box<dyn Element>
where
    F: Fn(usize, &mut EventContext, Vector2F) + 'static,
{
    render_session_type_pills_with_background(
        session_types,
        selected_index,
        pill_mouse_states,
        on_select,
        None,
        appearance,
    )
}

/// Renders session type pills with an optional background color override.
/// When `bg` is `Some`, text and border colors are computed against that background
/// (used for the accent-tinted onboarding callout).
pub fn render_session_type_pills_with_background<F>(
    session_types: &[SessionType],
    selected_index: usize,
    pill_mouse_states: &[MouseStateHandle],
    on_select: F,
    bg: Option<ColorU>,
    appearance: &Appearance,
) -> Box<dyn Element>
where
    F: Fn(usize, &mut EventContext, Vector2F) + 'static,
{
    let theme = appearance.theme();
    let bg_fill = bg.map(Fill::Solid).unwrap_or(theme.background());
    let on_accent_bg = bg.is_some();
    let on_select = Arc::new(on_select);

    let label = Text::new_inline("Session type".to_string(), appearance.ui_font_family(), 12.)
        .with_color(if on_accent_bg {
            callout_label_color(appearance)
        } else {
            blended_colors::text_disabled(theme, bg_fill)
        })
        .finish();

    let mut pills_row = Flex::row().with_spacing(PILL_GAP);

    for (i, session_type) in session_types.iter().enumerate() {
        let is_selected = i == selected_index;
        let mouse_state = pill_mouse_states[i].clone();

        let item_color = session_type_item_color(is_selected, on_accent_bg, theme, bg_fill);

        let icon = ConstrainedBox::new(
            session_type
                .icon()
                .to_warpui_icon(item_color.into())
                .finish(),
        )
        .with_width(14.)
        .with_height(14.)
        .finish();

        let name = Text::new_inline(
            session_type.pill_label().to_string(),
            appearance.ui_font_family(),
            14.,
        )
        .with_color(item_color)
        .finish();

        let pill_content = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::Center)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(icon)
            .with_child(Container::new(name).with_margin_left(8.).finish())
            .finish();

        let border_color = if is_selected {
            if on_accent_bg {
                phenomenon_accent_color()
            } else {
                theme.accent().into_solid()
            }
        } else if on_accent_bg {
            phenomenon_subtle_border_color()
        } else {
            blended_colors::neutral_4(theme)
        };

        let background = if is_selected {
            if on_accent_bg {
                Some(Fill::Solid(phenomenon_foreground_color()))
            } else {
                Some(blended_colors::accent_overlay_1(theme))
            }
        } else {
            None
        };

        let mut pill = Container::new(pill_content)
            .with_horizontal_padding(12.)
            .with_vertical_padding(8.)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
            .with_border(Border::all(1.).with_border_color(border_color));

        if let Some(bg) = background {
            pill = pill.with_background(bg);
        }

        let on_select = on_select.clone();
        let pill_element = Expanded::new(
            1.0,
            Hoverable::new(mouse_state, move |_| pill.finish())
                .with_cursor(Cursor::PointingHand)
                .on_click(move |ctx, _, position| {
                    on_select(i, ctx, position);
                })
                .finish(),
        );

        pills_row.extend([pill_element.finish()]);
    }

    Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_child(label)
        .with_child(
            Container::new(pills_row.finish())
                .with_margin_top(4.)
                .finish(),
        )
        .finish()
}

/// Renders the directory picker button.
///
/// Displays the selected directory in a bordered button. Calls `on_click` when pressed.
pub fn render_directory_picker<F>(
    selected_directory: &Path,
    mouse_state: MouseStateHandle,
    on_click: F,
    appearance: &Appearance,
) -> Box<dyn Element>
where
    F: Fn(&mut EventContext, Vector2F) + 'static,
{
    render_directory_picker_with_background(
        selected_directory,
        mouse_state,
        on_click,
        None,
        appearance,
    )
}

/// Renders a directory picker with an optional background color override.
pub fn render_directory_picker_with_background<F>(
    selected_directory: &Path,
    mouse_state: MouseStateHandle,
    on_click: F,
    bg: Option<ColorU>,
    appearance: &Appearance,
) -> Box<dyn Element>
where
    F: Fn(&mut EventContext, Vector2F) + 'static,
{
    let theme = appearance.theme();
    let bg_fill = bg.map(Fill::Solid).unwrap_or(theme.background());

    let on_accent_bg = bg.is_some();

    let label = Text::new_inline(
        "Select directory".to_string(),
        appearance.ui_font_family(),
        12.,
    )
    .with_color(if on_accent_bg {
        callout_label_color(appearance)
    } else {
        blended_colors::text_disabled(theme, bg_fill)
    })
    .finish();

    let home_dir = dirs::home_dir();
    let raw_path = selected_directory.to_string_lossy();
    let dir_display =
        warp_util::path::user_friendly_path(&raw_path, home_dir.as_ref().and_then(|h| h.to_str()))
            .into_owned();

    let dir_text = Text::new_inline(dir_display, appearance.ui_font_family(), 14.)
        .with_color(if on_accent_bg {
            phenomenon_body_text_color()
        } else {
            blended_colors::text_main(theme, bg_fill)
        })
        .with_style(Properties::default().weight(Weight::Semibold))
        .finish();

    let border_color = if on_accent_bg {
        phenomenon_subtle_border_color()
    } else {
        blended_colors::neutral_4(theme)
    };

    let button = Hoverable::new(mouse_state, move |_| {
        let content_row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(dir_text)
            .finish();

        Container::new(ConstrainedBox::new(content_row).with_height(30.).finish())
            .with_horizontal_padding(12.)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
            .with_border(Border::all(1.).with_border_color(border_color))
            .finish()
    })
    .with_cursor(Cursor::PointingHand)
    .on_click(move |ctx, _, position| {
        on_click(ctx, position);
    })
    .finish();

    Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_child(label)
        .with_child(Container::new(button).with_margin_top(4.).finish())
        .finish()
}

/// Renders the worktree checkbox with label and tooltip.
///
/// `on_toggle` is called when the checkbox is clicked (only when enabled).
pub fn render_worktree_checkbox<F>(
    enabled: bool,
    is_git_repo: bool,
    checkbox_mouse_state: MouseStateHandle,
    tooltip_mouse_state: MouseStateHandle,
    on_toggle: F,
    appearance: &Appearance,
) -> Box<dyn Element>
where
    F: Fn(&mut EventContext, Vector2F) + 'static,
{
    render_worktree_checkbox_with_background(
        enabled,
        is_git_repo,
        checkbox_mouse_state,
        tooltip_mouse_state,
        on_toggle,
        None,
        appearance,
    )
}

/// Renders a worktree checkbox with an optional background color override.
pub fn render_worktree_checkbox_with_background<F>(
    enabled: bool,
    is_git_repo: bool,
    checkbox_mouse_state: MouseStateHandle,
    tooltip_mouse_state: MouseStateHandle,
    on_toggle: F,
    bg: Option<ColorU>,
    appearance: &Appearance,
) -> Box<dyn Element>
where
    F: Fn(&mut warpui::EventContext, warpui::geometry::vector::Vector2F) + 'static,
{
    let disabled = !is_git_repo;
    let on_accent_bg = bg.is_some();

    let mut checkbox = if on_accent_bg {
        callout_checkbox(checkbox_mouse_state, Some(10.5), appearance).check(enabled)
    } else {
        appearance
            .ui_builder()
            .checkbox(checkbox_mouse_state, Some(10.5))
            .check(enabled)
    };

    if disabled {
        checkbox = checkbox.disabled();
    }

    let checkbox_el = if disabled {
        checkbox.build().finish()
    } else {
        checkbox
            .build()
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, position| {
                on_toggle(ctx, position);
            })
            .finish()
    };

    let checkbox_el = if disabled {
        let theme = appearance.theme();
        let font_family = appearance.ui_font_family();
        Hoverable::new(tooltip_mouse_state, move |state| {
            let mut stack = Stack::new();
            stack.add_child(checkbox_el);
            if state.is_hovered() {
                let tooltip = Container::new(
                    Text::new_inline(
                        "Select a git repository to enable worktree support".to_string(),
                        font_family,
                        12.,
                    )
                    .with_color(theme.background().into_solid())
                    .finish(),
                )
                .with_horizontal_padding(14.)
                .with_vertical_padding(6.)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                .with_border(Border::all(1.).with_border_fill(theme.outline()))
                .with_background_color(theme.tooltip_background())
                .finish();

                stack.add_positioned_overlay_child(
                    tooltip,
                    OffsetPositioning::offset_from_parent(
                        vec2f(0., -4.),
                        ParentOffsetBounds::WindowByPosition,
                        ParentAnchor::TopLeft,
                        ChildAnchor::BottomLeft,
                    ),
                );
            }
            stack.finish()
        })
        .finish()
    } else {
        checkbox_el
    };

    let theme = appearance.theme();
    let label_color = if on_accent_bg {
        if disabled {
            phenomenon_disabled_label_text_color()
        } else {
            callout_label_color(appearance)
        }
    } else if disabled {
        blended_colors::text_disabled(theme, theme.background())
    } else {
        blended_colors::text_sub(theme, theme.background())
    };
    let label = Text::new(
        "Automatically create a worktree when opening a new tab",
        appearance.ui_font_family(),
        12.,
    )
    .with_color(label_color)
    .finish();

    Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(checkbox_el)
        .with_child(Container::new(label).with_margin_left(8.).finish())
        .finish()
}

/// Renders the "Autogenerate worktree branch name" checkbox with label and tooltip.
pub fn render_autogenerate_worktree_branch_name_checkbox<F>(
    checked: bool,
    enable_worktree: bool,
    checkbox_mouse_state: MouseStateHandle,
    tooltip_mouse_state: MouseStateHandle,
    on_toggle: F,
    appearance: &Appearance,
) -> Box<dyn Element>
where
    F: Fn(&mut EventContext, Vector2F) + 'static,
{
    render_autogenerate_worktree_branch_name_checkbox_with_background(
        checked,
        enable_worktree,
        checkbox_mouse_state,
        tooltip_mouse_state,
        on_toggle,
        None,
        appearance,
    )
}

/// Renders the autogenerate checkbox with an optional background color override.
pub fn render_autogenerate_worktree_branch_name_checkbox_with_background<F>(
    checked: bool,
    enable_worktree: bool,
    checkbox_mouse_state: MouseStateHandle,
    tooltip_mouse_state: MouseStateHandle,
    on_toggle: F,
    bg: Option<ColorU>,
    appearance: &Appearance,
) -> Box<dyn Element>
where
    F: Fn(&mut EventContext, Vector2F) + 'static,
{
    let disabled = !enable_worktree;
    let on_accent_bg = bg.is_some();

    let mut checkbox = if on_accent_bg {
        callout_checkbox(checkbox_mouse_state, Some(10.5), appearance).check(checked)
    } else {
        appearance
            .ui_builder()
            .checkbox(checkbox_mouse_state, Some(10.5))
            .check(checked)
    };

    if disabled {
        checkbox = checkbox.disabled();
    }

    let checkbox_el = if disabled {
        checkbox.build().finish()
    } else {
        checkbox
            .build()
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, position| {
                on_toggle(ctx, position);
            })
            .finish()
    };

    let checkbox_el = if disabled {
        let theme = appearance.theme();
        let font_family = appearance.ui_font_family();
        Hoverable::new(tooltip_mouse_state, move |state| {
            let mut stack = Stack::new();
            stack.add_child(checkbox_el);
            if state.is_hovered() {
                let tooltip = Container::new(
                    Text::new_inline(
                        "You must select that you want to automatically create a \
                         worktree in order to select this"
                            .to_string(),
                        font_family,
                        12.,
                    )
                    .with_color(theme.background().into_solid())
                    .finish(),
                )
                .with_horizontal_padding(14.)
                .with_vertical_padding(6.)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                .with_border(Border::all(1.).with_border_fill(theme.outline()))
                .with_background_color(theme.tooltip_background())
                .finish();

                stack.add_positioned_overlay_child(
                    tooltip,
                    OffsetPositioning::offset_from_parent(
                        vec2f(0., -4.),
                        ParentOffsetBounds::WindowByPosition,
                        ParentAnchor::TopLeft,
                        ChildAnchor::BottomLeft,
                    ),
                );
            }
            stack.finish()
        })
        .finish()
    } else {
        checkbox_el
    };

    let theme = appearance.theme();
    let label_color = if on_accent_bg {
        if disabled {
            phenomenon_disabled_label_text_color()
        } else {
            callout_label_color(appearance)
        }
    } else if disabled {
        blended_colors::text_disabled(theme, theme.background())
    } else {
        blended_colors::text_sub(theme, theme.background())
    };

    let label = Text::new(
        "Auto-generate worktree branch name",
        appearance.ui_font_family(),
        12.,
    )
    .with_color(label_color)
    .finish();

    Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(checkbox_el)
        .with_child(Container::new(label).with_margin_left(8.).finish())
        .finish()
}

/// All possible session types, in display order.
const ALL_SESSION_TYPES: &[SessionType] = &[SessionType::Oz, SessionType::Terminal];

/// Returns the session types to display, filtering out Oz when AI is disabled.
pub fn visible_session_types(show_oz: bool) -> Vec<SessionType> {
    ALL_SESSION_TYPES
        .iter()
        .filter(|st| show_oz || !matches!(st, SessionType::Oz))
        .copied()
        .collect()
}
