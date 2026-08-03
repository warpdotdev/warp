use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::{Vector2F, vec2f};

use super::{Axis, F32Ext, ScrollData, ScrollbarWidth, Vector2FExt};
use crate::units::{IntoPixels, Pixels};

pub const DEFAULT_SCROLLBAR_PADDING_BETWEEN_CHILD_AND_TRACK: f32 = 2.0;
pub const DEFAULT_SCROLLBAR_PADDING_AFTER_TRACK: f32 = 2.0;
pub const DEFAULT_SCROLL_WHEEL_PIXELS_PER_LINE: f32 = 40.0;
pub const MIN_SCROLLBAR_THUMB_LENGTH: f32 = 20.0;

#[derive(Clone, Copy, Debug)]
pub struct ScrollbarAppearance {
    pub scrollbar_width: ScrollbarWidth,
    pub overlaid_scrollbar: bool,
    pub padding_between_child_and_scrollbar: f32,
    pub padding_after_scrollbar: f32,
}

impl ScrollbarAppearance {
    pub fn new(scrollbar_width: ScrollbarWidth, overlaid_scrollbar: bool) -> Self {
        Self {
            scrollbar_width,
            overlaid_scrollbar,
            padding_between_child_and_scrollbar: DEFAULT_SCROLLBAR_PADDING_BETWEEN_CHILD_AND_TRACK,
            padding_after_scrollbar: DEFAULT_SCROLLBAR_PADDING_AFTER_TRACK,
        }
    }

    fn cross_axis_spacing(&self, include_overlaid_scrollbar: bool) -> f32 {
        if !include_overlaid_scrollbar && self.overlaid_scrollbar {
            0.0
        } else {
            self.padding_between_child_and_scrollbar + self.padding_after_scrollbar
        }
    }

    fn scrollbar_track_length(&self, include_overlaid_scrollbar: bool) -> f32 {
        if !include_overlaid_scrollbar && self.overlaid_scrollbar {
            0.0
        } else {
            self.scrollbar_width.as_f32()
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ScrollbarGeometry {
    pub track_bounds: RectF,
    pub thumb_bounds: RectF,
    pub scrollbar_size_percentage: f32,
    pub scrollbar_position_percentage: f32,
}

impl ScrollbarGeometry {
    pub fn has_thumb(&self) -> bool {
        self.scrollbar_size_percentage < 1.0 && !self.thumb_bounds.is_empty()
    }

    pub fn thumb_center_along(&self, axis: Axis) -> Pixels {
        self.thumb_bounds.center().along(axis).into_pixels()
    }
}

pub fn project_scroll_delta_by_sensitivity(delta: Vector2F, sensitivity: f32) -> Vector2F {
    if delta.x().abs() * sensitivity > delta.y().abs() {
        delta.project_onto(Axis::Horizontal)
    } else if delta.y().abs() * sensitivity > delta.x().abs() {
        delta.project_onto(Axis::Vertical)
    } else {
        delta
    }
}

pub fn compute_scrollbar_geometry(
    axis: Axis,
    origin: Vector2F,
    scrollable_size: Vector2F,
    scroll_data: ScrollData,
    appearance: ScrollbarAppearance,
) -> ScrollbarGeometry {
    let scrollable_size_with_padding = match axis {
        Axis::Horizontal => vec2f(
            0.0,
            appearance.scrollbar_track_length(true) + appearance.cross_axis_spacing(true),
        ),
        Axis::Vertical => vec2f(
            appearance.scrollbar_track_length(true) + appearance.cross_axis_spacing(true),
            0.0,
        ),
    };
    let viewport_size = (scrollable_size - scrollable_size_with_padding).max(Vector2F::zero());
    let scrollbar_track_length = scrollable_size_with_padding.along(axis.invert());
    let scrollbar_track_origin = origin + scrollable_size.project_onto(axis.invert())
        - scrollbar_track_length.along(axis.invert());
    let scrollbar_track_size = scrollbar_size(axis, viewport_size, scrollbar_track_length);
    let track_bounds = RectF::new(scrollbar_track_origin, scrollbar_track_size);

    let (scrollbar_size_percentage, scrollbar_position_percentage) =
        scrollbar_percentages(scroll_data, viewport_size.along(axis));

    if scrollbar_size_percentage >= 1.0 {
        return ScrollbarGeometry {
            track_bounds,
            thumb_bounds: RectF::new(vec2f(0.0, 0.0), vec2f(0.0, 0.0)),
            scrollbar_size_percentage,
            scrollbar_position_percentage,
        };
    }

    let thumb_size = scrollbar_size(
        axis,
        viewport_size * scrollbar_size_percentage,
        appearance.scrollbar_width.as_f32(),
    );
    let thumb_origin = scrollbar_track_origin
        + scrollbar_size(
            axis,
            (viewport_size - thumb_size).max(Vector2F::zero()) * scrollbar_position_percentage,
            appearance.padding_between_child_and_scrollbar,
        );

    ScrollbarGeometry {
        track_bounds,
        thumb_bounds: RectF::new(thumb_origin, thumb_size),
        scrollbar_size_percentage,
        scrollbar_position_percentage,
    }
}

pub fn scroll_delta_for_pointer_movement(
    previous_position_along_axis: Pixels,
    new_position_along_axis: Pixels,
    scroll_data: ScrollData,
) -> Pixels {
    if scroll_data.total_size <= Pixels::zero()
        || scroll_data.visible_px <= Pixels::zero()
        || scroll_data.visible_px >= scroll_data.total_size
    {
        return Pixels::zero();
    }

    let scroll_size_percentage = scroll_data.visible_px / scroll_data.total_size;
    if scroll_size_percentage <= Pixels::zero() {
        return Pixels::zero();
    }

    (previous_position_along_axis - new_position_along_axis) / scroll_size_percentage
}

fn scrollbar_percentages(scroll_data: ScrollData, scrollable_pixels: f32) -> (f32, f32) {
    if scroll_data.total_size <= Pixels::zero() {
        return (1.0, 0.0);
    }

    let minimum_size_percentage = (MIN_SCROLLBAR_THUMB_LENGTH / scrollable_pixels).min(1.0);
    let size_percentage = (scroll_data.visible_px / scroll_data.total_size)
        .max(Pixels::new(minimum_size_percentage))
        .as_f32();
    let scroll_remaining =
        scroll_data.total_size - scroll_data.scroll_start - scroll_data.visible_px;
    let position_percentage = if scroll_data.scroll_start + scroll_remaining <= Pixels::zero() {
        0.0
    } else {
        (scroll_data.scroll_start / (scroll_data.scroll_start + scroll_remaining)).as_f32()
    };

    (size_percentage, position_percentage)
}

pub(crate) fn scrollbar_size(
    axis: Axis,
    scrollable_size: Vector2F,
    scrollbar_track_length: f32,
) -> Vector2F {
    match axis {
        Axis::Horizontal => vec2f(scrollable_size.x(), scrollbar_track_length),
        Axis::Vertical => vec2f(scrollbar_track_length, scrollable_size.y()),
    }
}
