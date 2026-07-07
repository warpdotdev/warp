use pathfinder_geometry::vector::vec2f;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::icons::Icon;
use warp_core::ui::theme::color::internal_colors;
use warp_core::ui::theme::Fill;
use warpui_core::elements::{
    Align, Border, ChildAnchor, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Flex,
    Hoverable, MainAxisSize, MouseStateHandle, OffsetPositioning, ParentAnchor, ParentElement,
    ParentOffsetBounds, Radius, Stack, Text,
};
use warpui_core::fonts::{Properties, Weight};
use warpui_core::platform::Cursor;
use warpui_core::Element;

use super::agent_slide::AgentSlideAction;

pub(super) struct TwoLineButtonSpec {
    pub(super) is_selected: bool,
    pub(super) title: String,
    pub(super) subtitle: String,
    pub(super) height: f32,
    pub(super) mouse_state: MouseStateHandle,
    pub(super) click_action: AgentSlideAction,
    pub(super) subtitle_font_size: f32,
    pub(super) title_color: Fill,
    pub(super) subtitle_color: Fill,
    /// Optional icon to display before the title.
    pub(super) icon: Option<Icon>,
    /// If set, the button is disabled and this text is shown as a badge.
    pub(super) disabled_badge: Option<String>,
    /// When true, the button is fully disabled: muted colors, no selected
    /// state, no click cursor, and no click handler.
    pub(super) is_disabled: bool,
}

pub(super) fn render_two_line_button(
    appearance: &Appearance,
    spec: TwoLineButtonSpec,
) -> Box<dyn Element> {
    const RADIUS: f32 = 8.;

    let TwoLineButtonSpec {
        is_selected,
        title,
        subtitle,
        height,
        mouse_state,
        click_action,
        subtitle_font_size,
        title_color,
        subtitle_color,
        icon,
        disabled_badge,
        is_disabled,
    } = spec;

    let theme = appearance.theme();
    let is_disabled = is_disabled || disabled_badge.is_some();

    // Disabled models always use muted colors and never show as selected.
    let effective_selected = is_selected && !is_disabled;

    let (title_fill, subtitle_fill, background) = match (effective_selected, is_disabled) {
        (true, _) => {
            let bg_color = internal_colors::accent_overlay_1(theme);
            let selected_color = internal_colors::accent_fg_strong(theme);
            (selected_color, selected_color, Some(bg_color))
        }
        (false, true) => {
            let bg_color = theme.surface_2();
            let disabled_color = theme.disabled_text_color(bg_color);
            (disabled_color, disabled_color, Some(bg_color))
        }
        (false, false) => (title_color, subtitle_color, None),
    };

    let border_color = if effective_selected {
        theme.accent()
    } else {
        Fill::Solid(internal_colors::neutral_4(theme))
    };

    let ui_font_family = appearance.ui_font_family();

    let hoverable = Hoverable::new(mouse_state, move |_| {
        let title_text = Text::new(title.clone(), ui_font_family, 14.0)
            .with_color(title_fill.into_solid())
            .with_style(Properties {
                weight: Weight::Normal,
                ..Default::default()
            })
            .with_line_height_ratio(1.0)
            .finish();

        // Build title row with optional icon
        let title_el: Box<dyn Element> = if let Some(icon) = icon {
            const ICON_SIZE: f32 = 14.;
            let icon_el = ConstrainedBox::new(Box::new(icon.to_warpui_icon(title_fill)))
                .with_width(ICON_SIZE)
                .with_height(ICON_SIZE)
                .finish();
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(icon_el)
                .with_child(Container::new(title_text).with_margin_left(4.).finish())
                .finish()
        } else {
            title_text
        };

        let subtitle_el = Text::new(subtitle.clone(), ui_font_family, subtitle_font_size)
            .with_color(subtitle_fill.into_solid())
            .with_style(Properties {
                weight: Weight::Normal,
                ..Default::default()
            })
            .with_line_height_ratio(1.0)
            .finish();

        let content = Flex::column()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(title_el)
            .with_child(Container::new(subtitle_el).with_margin_top(8.).finish())
            .finish();

        let aligned = Align::new(content).left().finish();

        let mut container = Container::new(aligned)
            .with_uniform_padding(24.)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(RADIUS)))
            .with_border(Border::all(1.).with_border_fill(border_color));

        if let Some(bg) = background {
            container = container.with_background(bg);
        }

        let button_el = ConstrainedBox::new(container.finish())
            .with_min_height(height)
            .finish();

        // Overlay the badge pill at bottom-right if disabled
        if let Some(ref badge_text) = disabled_badge {
            let badge_label = Text::new(badge_text.clone(), ui_font_family, 11.0)
                .with_color(internal_colors::neutral_1(theme))
                .with_style(Properties {
                    weight: Weight::Medium,
                    ..Default::default()
                })
                .with_line_height_ratio(1.0)
                .finish();

            let badge = Container::new(badge_label)
                .with_padding_left(6.)
                .with_padding_right(6.)
                .with_padding_top(2.)
                .with_padding_bottom(2.)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                .with_background_color(theme.ansi_fg_green())
                .finish();

            let mut stack = Stack::new();
            stack.add_child(button_el);
            stack.add_positioned_child(
                badge,
                OffsetPositioning::offset_from_parent(
                    vec2f(-6., -6.),
                    ParentOffsetBounds::ParentByPosition,
                    ParentAnchor::BottomRight,
                    ChildAnchor::BottomRight,
                ),
            );
            stack.finish()
        } else {
            button_el
        }
    });

    if is_disabled {
        // Disabled: no click handler, default cursor
        hoverable.finish()
    } else {
        hoverable
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(click_action.clone());
            })
            .finish()
    }
}
