//! Consistent bounds definitions for rich-text blocks.
//!
//! The terminology is loosely based on the [alternative CSS box model](https://developer.mozilla.org/en-US/docs/Learn/CSS/Building_blocks/The_box_model#parts_of_a_box).
//!
//! * The **content box** is the rectangle containing a block's content, without any margins or
//!   padding.
//! * The **visible box** is the rectangle containing a block's content, padding, and borders -
//!   everything that's visually part of the block.
//! * The **reserved box** is the rectangle containing a block's content, padding, borders, and
//!   margin - all space reserved for the block.

use warpui::geometry::rect::RectF;
use warpui::geometry::vector::{Vector2F, vec2f};
use warpui::units::Pixels;

use super::BlockSpacing;

/// The origin of a block's content box. This is relative to the buffer origin. To convert it
/// to an on-screen point, use `RenderContext::content_to_screen`.
pub fn content_origin(y_offset: Pixels, spacing: &BlockSpacing) -> Vector2F {
    vec2f(
        spacing.left_offset().as_f32(),
        (y_offset + spacing.top_offset()).as_f32(),
    )
}

/// The content box for a block, given its:
/// * y-offset relative to the start of the buffer
/// * Content size, in pixels
/// * Spacing
///
/// The box is relative to the buffer. To convert it to an on-screen rectangle, use
/// `RenderContext::content_rect_to_screen`.
pub fn content_box(y_offset: Pixels, content_size: Vector2F, spacing: &BlockSpacing) -> RectF {
    RectF::new(content_origin(y_offset, spacing), content_size)
}

/// The origin of a block's visible box. This is relative to the buffer origin. To convert it
/// to an on-screen point, use `RenderContext::content_to_screen`.
pub fn visible_origin(y_offset: Pixels, spacing: &BlockSpacing) -> Vector2F {
    vec2f(
        spacing.margin.left(),
        y_offset.as_f32() + spacing.margin.top(),
    )
}

/// The visible box for a block, given its:
/// * y-offset relative to the start of the buffer
/// * Content size, in pixels
/// * Spacing
///
/// The box is relative to the buffer. To convert it to an on-screen rectangle, use
/// `RenderContext::content_rect_to_screen`.
pub fn visible_box(y_offset: Pixels, content_size: Vector2F, spacing: &BlockSpacing) -> RectF {
    RectF::new(
        visible_origin(y_offset, spacing),
        content_size
            + vec2f(
                spacing.padding.left() + spacing.padding.right(),
                spacing.padding.top() + spacing.padding.bottom(),
            ),
    )
}

/// The origin of a block's reserved box. This is relative to the buffer origin. To convert it
/// to an on-screen point, use `RenderContext::content_to_screen`.
pub fn reserved_origin(y_offset: Pixels) -> Vector2F {
    vec2f(0., y_offset.as_f32())
}

/// The reserved box for a block, given its:
/// * y-offset relative to the start of the buffer
/// * Content size, in pixels
/// * Spacing
///
/// The box is relative to the buffer. To convert it to an on-screen rectangle, use
/// `RenderContext::content_rect_to_screen`.
pub fn reserved_box(y_offset: Pixels, content_size: Vector2F, spacing: &BlockSpacing) -> RectF {
    RectF::new(
        reserved_origin(y_offset),
        content_size
            + vec2f(
                spacing.x_axis_offset().as_f32(),
                spacing.y_axis_offset().as_f32(),
            ),
    )
}
