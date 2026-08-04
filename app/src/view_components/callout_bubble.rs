use pathfinder_color::ColorU;
use warp_core::ui::theme::Fill;
use warp_core::ui::theme::phenomenon::PhenomenonStyle;
use warpui::Element;
use warpui::elements::{
    Border, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Flex, MainAxisSize,
    MouseStateHandle, ParentElement, Radius, Stack,
};
use warpui::ui_components::checkbox::Checkbox;
use warpui::ui_components::components::UiComponentStyles;

use crate::appearance::Appearance;
use crate::ui_components::icons::Icon;

/// Which direction the callout arrow points.
#[derive(Debug, Clone, Copy)]
pub enum CalloutArrowDirection {
    Up,
    Left,
}

/// Where the arrow is positioned along the bubble edge.
#[derive(Debug, Clone, Copy)]
pub enum CalloutArrowPosition {
    /// Offset from the start of the bubble edge the arrow sits on.
    /// For Up arrows: offset from the left edge.
    /// For Left arrows: offset from the top edge.
    Start(f32),
    /// Offset from the end of the bubble edge the arrow sits on.
    /// For Up arrows: offset from the right edge.
    End(f32),
    /// Centered on the bubble edge.
    Center,
}

/// Configuration for rendering a callout bubble with an arrow.
pub struct CalloutBubbleConfig {
    pub width: f32,
    pub arrow_direction: CalloutArrowDirection,
    pub arrow_position: CalloutArrowPosition,
}

pub fn phenomenon_background_color() -> ColorU {
    PhenomenonStyle::background()
}

pub fn phenomenon_foreground_color() -> ColorU {
    PhenomenonStyle::foreground()
}

pub fn phenomenon_accent_color() -> ColorU {
    PhenomenonStyle::accent()
}

pub fn phenomenon_body_text_color() -> ColorU {
    PhenomenonStyle::body_text()
}

pub fn phenomenon_label_text_color() -> ColorU {
    PhenomenonStyle::label_text()
}

pub fn phenomenon_disabled_label_text_color() -> ColorU {
    PhenomenonStyle::disabled_label_text()
}

pub fn phenomenon_subtle_border_color() -> ColorU {
    PhenomenonStyle::subtle_border()
}

/// Returns the shared HOA callout background fill using the Phenomenon palette.
pub fn callout_background_fill(appearance: &Appearance) -> Fill {
    let _ = appearance;
    PhenomenonStyle::tinted_surface()
}

/// Returns the shared HOA callout border color using the Phenomenon palette.
pub fn callout_border_color(appearance: &Appearance) -> ColorU {
    let _ = appearance;
    PhenomenonStyle::surface_border()
}

/// Renders a callout bubble with an arrow indicator.
///
/// The bubble has an accent-tinted background with an accent border,
/// and a triangular arrow on the specified edge.
/// The `content` element is placed inside the bubble body.
pub fn render_callout_bubble(
    content: Box<dyn Element>,
    config: &CalloutBubbleConfig,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let background = callout_background_fill(appearance);
    let border_color = callout_border_color(appearance);

    let bubble = ConstrainedBox::new(
        Container::new(content)
            .with_background(background)
            .with_border(Border::all(1.).with_border_fill(Fill::Solid(border_color)))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
            .finish(),
    )
    .with_width(config.width)
    .finish();

    let (border_icon, fill_icon) = match config.arrow_direction {
        CalloutArrowDirection::Up => (Icon::CalloutTriangleBorderUp, Icon::CalloutTriangleFillUp),
        CalloutArrowDirection::Left => (
            Icon::CalloutTriangleBorderLeft,
            Icon::CalloutTriangleFillLeft,
        ),
    };

    let triangle = Stack::new()
        .with_child(
            ConstrainedBox::new(
                border_icon
                    .to_warpui_icon(Fill::Solid(border_color))
                    .finish(),
            )
            .with_width(24.)
            .with_height(24.)
            .finish(),
        )
        .with_child(
            ConstrainedBox::new(fill_icon.to_warpui_icon(background).finish())
                .with_width(24.)
                .with_height(24.)
                .finish(),
        )
        .finish();

    match config.arrow_direction {
        CalloutArrowDirection::Up => {
            let arrow_margin = match config.arrow_position {
                CalloutArrowPosition::Start(offset) => {
                    Container::new(triangle).with_margin_left(offset)
                }
                CalloutArrowPosition::End(offset) => {
                    let margin_left = (config.width - offset - 24.).max(0.);
                    Container::new(triangle).with_margin_left(margin_left)
                }
                CalloutArrowPosition::Center => {
                    let margin_left = (config.width - 24.) / 2.;
                    Container::new(triangle).with_margin_left(margin_left)
                }
            };

            let mut column = Flex::column().with_main_axis_size(MainAxisSize::Min);
            column.add_child(arrow_margin.with_margin_bottom(-3.).finish());
            column.add_child(bubble);
            column.finish()
        }
        CalloutArrowDirection::Left => {
            let (arrow_margin, cross_axis_alignment) = match config.arrow_position {
                CalloutArrowPosition::Start(offset) => (
                    Container::new(triangle).with_margin_top(offset),
                    CrossAxisAlignment::Start,
                ),
                CalloutArrowPosition::End(offset) => (
                    Container::new(triangle).with_margin_top(offset),
                    CrossAxisAlignment::Start,
                ),
                CalloutArrowPosition::Center => {
                    (Container::new(triangle), CrossAxisAlignment::Center)
                }
            };

            let mut row = Flex::row()
                .with_main_axis_size(MainAxisSize::Min)
                .with_cross_axis_alignment(cross_axis_alignment);
            row.add_child(arrow_margin.with_margin_right(-3.).finish());
            row.add_child(bubble);
            row.finish()
        }
    }
}

/// Title text color for callout content (foreground, 100% opacity).
pub fn callout_title_color(appearance: &Appearance) -> ColorU {
    let _ = appearance;
    phenomenon_foreground_color()
}

/// Body/description text color for callout content in the Phenomenon palette.
pub fn callout_body_color(appearance: &Appearance) -> ColorU {
    let _ = appearance;
    phenomenon_body_text_color()
}

/// Label/secondary text color for callout content in the Phenomenon palette.
pub fn callout_label_color(appearance: &Appearance) -> ColorU {
    let _ = appearance;
    phenomenon_label_text_color()
}

/// Creates a checkbox styled for callout bubbles using the Phenomenon palette.
///
/// Unchecked: foreground border, no fill.
/// Checked: foreground fill, background-colored check icon.
pub fn callout_checkbox(
    mouse_state: MouseStateHandle,
    size: Option<f32>,
    appearance: &Appearance,
) -> Checkbox {
    let _ = appearance;
    let foreground_color = phenomenon_foreground_color();
    let foreground_fill = Fill::Solid(foreground_color);
    let background_color = phenomenon_background_color();
    let disabled_color = phenomenon_subtle_border_color();
    let checkbox_size = size.or(Some(12.));
    let corner_radius = CornerRadius::with_all(Radius::Pixels(2.));

    Checkbox::new(
        mouse_state,
        UiComponentStyles {
            font_size: checkbox_size,
            border_color: Some(Fill::Solid(foreground_color).into()),
            font_color: Some(foreground_color),
            border_width: Some(1.),
            border_radius: Some(corner_radius),
            ..Default::default()
        },
        None,
        Some(UiComponentStyles {
            font_size: checkbox_size,
            background: Some(foreground_fill.into()),
            border_color: Some(foreground_fill.into()),
            font_color: Some(background_color),
            border_radius: Some(corner_radius),
            ..Default::default()
        }),
        Some(UiComponentStyles {
            font_size: checkbox_size,
            border_color: Some(Fill::Solid(disabled_color).into()),
            font_color: Some(disabled_color),
            border_width: Some(1.),
            border_radius: Some(corner_radius),
            ..Default::default()
        }),
    )
}
