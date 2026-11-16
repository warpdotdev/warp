use warp_core::ui::icons::ICON_DIMENSIONS;
use warp_core::ui::theme::Fill;
use warpui::Element;
use warpui::elements::{
    Align, Clipped, ConstrainedBox, Container, CrossAxisAlignment, Flex, Hoverable,
    MainAxisAlignment, MainAxisSize, MouseStateHandle, ParentElement, SavePosition, Shrinkable,
    Text,
};
use warpui::text_layout::ClipConfig;
use warpui::ui_components::components::UiComponent;

use super::super::header_content::HeaderRenderContext;
use super::{ActionPayload, PaneHeaderAction};
use crate::appearance::Appearance;
use crate::ui_components::buttons::{icon_button, icon_button_with_color};
use crate::ui_components::icons::Icon;

/// Horizontal padding applied inside each edge column of the three-column header.
pub const HEADER_EDGE_PADDING: f32 = 4.;

fn build_icon_button(
    appearance: &Appearance,
    icon: Icon,
    mouse_state: MouseStateHandle,
    icon_color: Option<Fill>,
) -> Hoverable {
    if let Some(color) = icon_color {
        icon_button_with_color(appearance, icon, false, mouse_state, color)
    } else {
        icon_button(appearance, icon, false, mouse_state)
    }
    .build()
}

fn apply_size_constraint(element: Box<dyn Element>, size: Option<f32>) -> Box<dyn Element> {
    if let Some(size) = size {
        ConstrainedBox::new(element)
            .with_width(size)
            .with_height(size)
            .finish()
    } else {
        element
    }
}

/// Renders the standard pane close button to dispatch the close action.
pub fn render_pane_close_button<A: ActionPayload, B: ActionPayload>(
    appearance: &Appearance,
    mouse_state: MouseStateHandle,
    icon_color: Option<Fill>,
    button_size: Option<f32>,
) -> Box<dyn Element> {
    let button = build_icon_button(appearance, Icon::X, mouse_state, icon_color)
        .on_click(|ctx, _, _| ctx.dispatch_typed_action(PaneHeaderAction::<A, B>::Close));
    apply_size_constraint(button.finish(), button_size)
}

/// Renders the standard pane overflow menu button to dispatch the pane overflow menu action.
pub fn render_pane_overflow_button<A: ActionPayload, B: ActionPayload>(
    appearance: &Appearance,
    mouse_state: MouseStateHandle,
    position_id: &str,
    icon_color: Option<Fill>,
    button_size: Option<f32>,
) -> Box<dyn Element> {
    let button = build_icon_button(appearance, Icon::DotsVertical, mouse_state, icon_color)
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action(PaneHeaderAction::<A, B>::OpenOverflowMenu)
        });
    let button = apply_size_constraint(button.finish(), button_size);
    SavePosition::new(button, position_id).finish()
}

/// Renders a row containing the standard pane overflow and close buttons.
#[cfg_attr(target_family = "wasm", allow(dead_code))]
pub fn render_pane_header_buttons<A: ActionPayload, B: ActionPayload>(
    header_ctx: &HeaderRenderContext<'_>,
    appearance: &Appearance,
    show_close_button: bool,
    icon_color: Option<Fill>,
    button_size: Option<f32>,
) -> Box<dyn Element> {
    let mut row = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_main_axis_size(MainAxisSize::Min);

    if header_ctx.has_overflow_items {
        row.add_child(render_pane_overflow_button::<A, B>(
            appearance,
            header_ctx.overflow_button_mouse_state.clone(),
            &header_ctx.overflow_button_position_id,
            icon_color,
            button_size,
        ));
    }

    if show_close_button {
        row.add_child(render_pane_close_button::<A, B>(
            appearance,
            header_ctx.close_button_mouse_state.clone(),
            icon_color,
            button_size,
        ));
    }

    row.finish()
}

/// Renders a title text element with the standard pane header font, color, and clipping.
pub fn render_pane_header_title_text(
    title: impl Into<std::borrow::Cow<'static, str>>,
    appearance: &Appearance,
    clip_config: ClipConfig,
) -> Box<dyn Element> {
    let font_size = appearance.ui_font_size();
    let font_color = appearance
        .theme()
        .sub_text_color(appearance.theme().background());
    Text::new_inline(title, appearance.ui_font_family(), font_size)
        .with_color(font_color.into())
        .with_clip(clip_config)
        .finish()
}

/// Estimates the minimum width needed for a header edge column containing
/// `icon_button_count` standard icon buttons, accounting for the edge
/// column's internal padding.
pub fn header_edge_min_width(icon_button_count: u32) -> f32 {
    icon_button_count as f32 * ICON_DIMENSIONS + HEADER_EDGE_PADDING
}

/// Width constraints applied to both the left and right edge columns in
/// [`render_three_column_header`].  Giving both edges the same min/max
/// keeps the center title visually centered regardless of how much content
/// each side actually contains.
pub struct CenteredHeaderEdgeWidth {
    /// Minimum width — typically the width of the always-visible buttons
    /// so they are never clipped.
    pub min: f32,
    /// Maximum width — caps how far the edge columns can grow so they
    /// don't eat into the title's space.
    pub max: f32,
}

/// Renders a 3-column header layout: `[left] [title] [right]`.
///
/// The `left` and `right` edge columns share equal min/max width constraints
/// so the title stays centered. Each edge gets inner padding (left or right).
/// `extra_left_inset` adds additional left padding inside the left column
/// (e.g. to make room for a floating overlay button) without affecting centering.
pub fn render_three_column_header(
    left: Box<dyn Element>,
    title: Box<dyn Element>,
    right: Box<dyn Element>,
    edge_width: CenteredHeaderEdgeWidth,
    extra_left_inset: f32,
    is_pane_dragging: bool,
) -> Box<dyn Element> {
    let main_axis_size = if is_pane_dragging {
        MainAxisSize::Min
    } else {
        MainAxisSize::Max
    };

    let left_constrained = ConstrainedBox::new(
        Container::new(left)
            .with_padding_left(HEADER_EDGE_PADDING + extra_left_inset)
            .finish(),
    )
    .with_min_width(edge_width.min)
    .with_max_width(edge_width.max)
    .finish();

    let right_constrained = ConstrainedBox::new(
        Container::new(right)
            .with_padding_right(HEADER_EDGE_PADDING)
            .finish(),
    )
    .with_min_width(edge_width.min)
    .with_max_width(edge_width.max)
    .finish();

    let mut center_row = Flex::row()
        .with_main_axis_alignment(MainAxisAlignment::Center)
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_main_axis_size(main_axis_size);
    center_row.add_child(if is_pane_dragging {
        title
    } else {
        Shrinkable::new(1., Clipped::new(title).finish()).finish()
    });
    let center = Align::new(center_row.finish()).finish();

    let mut row = Flex::row()
        .with_main_axis_size(main_axis_size)
        .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch);

    row.add_child(if is_pane_dragging {
        left_constrained
    } else {
        Shrinkable::new(1., left_constrained).finish()
    });
    row.add_child(if is_pane_dragging {
        center
    } else {
        Shrinkable::new(1., center).finish()
    });
    row.add_child(right_constrained);

    row.finish()
}
