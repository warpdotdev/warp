use float_cmp::ApproxEq;
use string_offset::CharOffset;
use sum_tree::{SeekBias, SumTree};
use warpui_core::SizeConstraint;
use warpui_core::geometry::rect::RectF;
use warpui_core::geometry::vector::{Vector2F, vec2f};
use warpui_core::units::{IntoPixels, Pixels};

use super::positioned::PositionedCursor;
use super::{
    AUTO_SCROLL_MARGIN, BlockItem, BlockSpacing, Height, HitTestOptions, LayoutSummary, Location,
    RenderState, UNIT_MARGIN, bounds,
};
use crate::render::element::RenderContext;

/// For horizontal autoscrolling, it is very easy to "stuck" on a character if it is aligned exactly on the viewport boundary.
/// To help make scrolling more smooth, add a small margin here to overcome these boundaries.
const HORIZONTAL_SCROLL_MARGIN: f32 = 4.;

#[cfg(test)]
#[path = "viewport_tests.rs"]
mod tests;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewportState {
    /// Width of the viewport. Currently, we soft-wrap text to fit this. However,
    /// we'll eventually support horizontal scrolling if the viewport is narrower
    /// than some minimum content width.
    width: Pixels,
    /// Height of the viewport. All scrolling and viewporting is in terms of
    /// pixels, not lines, as the line height varies for different content.
    height: Pixels,

    /// Vertical scrolling offset. This is the distance from the start of the
    /// content (height 0) to the first visible content.
    scroll_top: Pixels,

    /// Horizontal scrolling offset.
    scroll_left: Pixels,
}

/// A visible, viewported item. This stores all the information needed to lay out and display a
/// block and any associated UI controls in the current viewport.
///
/// Because the viewport item is needed throughout the `Element` lifecycle, it does not directly
/// reference the rendering model. Instead, it holds offsets that refer back to the model, relying
/// on the UI framework to guarantee that the model does not change without a re-render.
#[derive(Debug)]
pub struct ViewportItem {
    /// The y-offset to display this item at, relative to the viewport origin.
    /// If this is negative, the item is partially above the viewport.
    pub viewport_offset: Pixels,
    /// The y-offset of this item, relative to the content origin.
    pub content_offset: Pixels,
    /// The size of this item's content, in pixels.
    pub content_size: Vector2F,
    /// Spacing around this item.
    pub spacing: BlockSpacing,
    /// Offset of the start of the block backing this item.
    pub block_offset: CharOffset,
}

/// A snapshot of the scroll position. This may only be used to scroll back to the original
/// position, and cannot be inspected.
#[derive(Clone, Copy, Debug)]
pub struct ScrollPositionSnapshot {
    /// The offset of the top left character in the viewport. We use this to represent the scroll
    /// position, rather than a line count, to be resilient to soft-wrapping changes. If the
    /// viewport is resized, then the content that a given line offset refers to will likely be
    /// different.
    first_character_offset: CharOffset,
}

impl ScrollPositionSnapshot {
    /// Map this snapshot back to a `scroll_top` offset for the current render state.
    pub(super) fn to_scroll_top(self, render_state: &RenderState) -> Pixels {
        render_state
            .character_bounds(self.first_character_offset)
            .map_or(Pixels::zero(), |bounds| bounds.min_y().into_pixels())
    }

    /// Snapshot the render state's current scroll position.
    pub(super) fn from_scroll_top(render_state: &RenderState) -> Self {
        let first_character_offset = match render_state.viewport_coordinates_to_location(
            Pixels::zero(),
            Pixels::zero(),
            &HitTestOptions {
                force_text_selection: true,
            },
        ) {
            Location::Text { char_offset, .. } => char_offset,
            Location::Block { start_offset, .. } => start_offset,
        };
        Self {
            first_character_offset,
        }
    }

    #[cfg(test)]
    pub fn first_character_offset(self) -> CharOffset {
        self.first_character_offset
    }
}

pub struct ViewportIterator<'a> {
    cursor: sum_tree::Cursor<'a, BlockItem, Height, LayoutSummary>,
    /// The starting y-offset of content to display.
    content_start: Pixels,
    /// The ending y-offset of content to display (exclusive). This may be past
    /// the end of the document, but it just needs to be an upper bound.
    content_end: Pixels,
    /// Maximum width the painted object could take in the current viewport.
    max_width: Pixels,
}

#[derive(Debug, Clone, Copy)]
pub struct SizeInfo {
    /// The size of the viewport, in pixels.
    pub viewport_size: Vector2F,

    /// Whether or not text must be laid out again to fit the new viewport size.
    pub needs_layout: bool,
}

impl ViewportState {
    /// Create a new `ViewportState` with the given viewport size, scrolled to
    /// the top of the document.
    pub fn new(width: Pixels, height: Pixels) -> Self {
        Self {
            width,
            height,
            scroll_top: Pixels::zero(),
            scroll_left: Pixels::zero(),
        }
    }

    /// Width of the viewport. When rendering, it's assumed that the UI
    /// element is this wide.
    pub fn width(&self) -> Pixels {
        self.width
    }

    /// Height of the viewport. When rendering, it's assumed that the UI element
    /// is this tall.
    pub fn height(&self) -> Pixels {
        self.height
    }

    /// The current vertical scroll position of the viewport.
    pub fn scroll_top(&self) -> Pixels {
        self.scroll_top
    }

    /// The current horizontal scroll position of the viewport.
    pub fn scroll_left(&self) -> Pixels {
        self.scroll_left
    }

    /// Vertically scroll by `delta` pixels. Scrolling is capped at `content_height`,
    /// which should be the height of the buffer content.
    ///
    /// Returns whether the view should be re-rendered.
    pub(super) fn scroll(&mut self, delta: Pixels, content_height: Pixels) -> bool {
        self.scroll_to(self.scroll_top - delta, content_height)
    }

    pub(super) fn scroll_horizontally(&mut self, delta: Pixels, content_width: Pixels) -> bool {
        self.scroll_horizontally_to(self.scroll_left - delta, content_width)
    }

    /// Scroll to the given `scroll_top`, clamped to the end of the buffer.
    ///
    /// Returns whether or not the view needs to be re-rendered.
    pub(super) fn scroll_to(&mut self, scroll_top: Pixels, content_height: Pixels) -> bool {
        let scroll_top = self.clamp_scroll_offset(scroll_top, content_height, self.height);
        let changed = scroll_top.approx_ne(self.scroll_top, UNIT_MARGIN);
        if changed {
            self.scroll_top = scroll_top;
        }
        changed
    }

    pub(super) fn scroll_horizontally_to(
        &mut self,
        scroll_left: Pixels,
        content_width: Pixels,
    ) -> bool {
        let scroll_left = self.clamp_scroll_offset(scroll_left, content_width, self.width);
        let changed = scroll_left.approx_ne(self.scroll_left, UNIT_MARGIN);
        if changed {
            self.scroll_left = scroll_left;
        }
        changed
    }

    /// Set the scroll position to an exact location.
    #[cfg(test)]
    pub(super) fn set_scroll_top(&mut self, scroll_top: Pixels) {
        self.scroll_top = scroll_top;
    }

    /// Notifies the viewport model that the content height has changed, which
    /// affects the range of valid scroll positions.
    ///
    /// Returns whether the view should be re-rendered.
    pub(super) fn update_content_height(&mut self, content_height: Pixels) -> bool {
        // A scroll of 0 will reapply the clamping logic to ensure the scroll
        // position is still in bounds.
        self.scroll(Pixels::zero(), content_height)
    }

    pub(super) fn update_content_width(&mut self, content_width: Pixels) -> bool {
        // A scroll of 0 will reapply the clamping logic to ensure the scroll
        // position is still in bounds.
        self.scroll_horizontally(Pixels::zero(), content_width)
    }

    pub(super) fn autoscroll(
        &mut self,
        item_start: Vector2F,
        item_end: Vector2F,
        content_height: Pixels,
        content_width: Pixels,
        should_autoscroll_horizontally: bool,
    ) -> bool {
        let mut changed = false;

        if should_autoscroll_horizontally {
            if (item_start.x() - HORIZONTAL_SCROLL_MARGIN).into_pixels() < self.scroll_left {
                changed = self.scroll_horizontally(
                    self.scroll_left - item_start.x().into_pixels()
                        + AUTO_SCROLL_MARGIN.into_pixels(),
                    content_width,
                ) || changed;
            } else if (item_end.x() + HORIZONTAL_SCROLL_MARGIN).into_pixels()
                > self.scroll_left + self.width
            {
                changed = self.scroll_horizontally(
                    self.scroll_left - item_end.x().into_pixels() + self.width
                        - AUTO_SCROLL_MARGIN.into_pixels(),
                    content_width,
                ) || changed;
            }
        }

        if item_start.y().into_pixels() < self.scroll_top {
            // The position we want to scroll to is `item_start - AUTO_SCROLL_MARGIN.into_pixels()`.
            changed = self.scroll(
                self.scroll_top - item_start.y().into_pixels() + AUTO_SCROLL_MARGIN.into_pixels(),
                content_height,
            ) || changed;
        } else if item_end.y().into_pixels() > self.scroll_top + self.height {
            // The position we want to scroll to is `item_end - self.height + AUTO_SCROLL_MARGIN.into_pixels()`.
            changed = self.scroll(
                self.scroll_top - item_end.y().into_pixels() + self.height
                    - AUTO_SCROLL_MARGIN.into_pixels(),
                content_height,
            ) || changed;
        }

        changed
    }

    /// Clamps a scroll position to a valid value. The scroll top must be positive,
    /// and is at most the content height minus the viewport height. The viewport
    /// is scrolled all the way to the top if the scroll position is 0 and
    /// all the way to the bottom if the last viewport's worth of content is
    /// visible.
    fn clamp_scroll_offset(
        &self,
        scroll_top: Pixels,
        content_height: Pixels,
        viewport_height: Pixels,
    ) -> Pixels {
        scroll_top
            .min(content_height - viewport_height)
            .max(Pixels::zero())
    }

    /// Calculates the viewport size given layout constraints.
    ///
    /// Because we do not have mutable model access when laying out UI elements,
    /// size changes are handled in two steps:
    /// 1. [`crate::render::element::RichTextElement`] calls `viewport_size` as
    ///    part of its `layout` implementation.
    /// 2. `RichTextElement` then updates the model with the size it computed
    ///    in `after_layout`. Since `after_layout` runs before painting and
    ///    event handling, the model still has enough information to viewport,
    ///    hit-test, and scroll.
    pub(in crate::render) fn viewport_size(
        &self,
        constraint: SizeConstraint,
        size_buffer: Vector2F,
        max_width: Option<Pixels>,
    ) -> SizeInfo {
        // TODO(ben): We should have a minimum soft-wrap width. If the constraint's
        //    maximum size is below this, we start horizontal scrolling rather
        //    than trying to soft-wrap further.

        let mut max_constraint = constraint.max;
        if let Some(max_width) = max_width {
            max_constraint.set_x(constraint.max.x().min(max_width.as_f32()));
        }

        let content_constraint = SizeConstraint::new(
            (constraint.min - size_buffer).max(Vector2F::zero()),
            (max_constraint - size_buffer).max(Vector2F::zero()),
        );

        let width = content_constraint.max.x();
        let height = content_constraint.max.y();

        let needs_layout = width.approx_ne(self.width.as_f32(), UNIT_MARGIN);

        SizeInfo {
            viewport_size: vec2f(width, height),
            needs_layout,
        }
    }

    /// Save the viewport size that was calculated by a call to [`viewport_size`]
    /// during layout. This should only be called by [`crate::render::element::RichTextElement`],
    /// otherwise there's no guarantee that content is soft-wrapped to the correct bounds.
    ///
    /// This may also adjust the scroll position, if it's not valid in the new viewport size.
    pub(super) fn set_size(
        &mut self,
        size: Vector2F,
        content_width: Pixels,
        content_height: Pixels,
    ) {
        self.width = size.x().into_pixels();
        self.height = size.y().into_pixels();
        // If set_size is called, the view is already being re-rendered, so we can ignore the
        // return value of update_content_height.
        self.update_content_height(content_height);
        self.update_content_width(content_width);
    }
}

impl<'a> ViewportIterator<'a> {
    /// Begin an iterator over the current viewport.
    pub(super) fn new(
        content: &'a SumTree<BlockItem>,
        scroll_top: Pixels,
        viewport_height: Pixels,
        viewport_width: Pixels,
    ) -> Self {
        let mut cursor = content.cursor();
        cursor.seek_clamped(&scroll_top.into(), SeekBias::Left);

        Self {
            cursor,
            content_start: scroll_top,
            content_end: scroll_top + viewport_height,
            max_width: viewport_width,
        }
    }
}

impl<'a> Iterator for ViewportIterator<'a> {
    type Item = (ViewportItem, &'a BlockItem);

    fn next(&mut self) -> Option<Self::Item> {
        let item = self.cursor.positioned_item()?;
        // Stop rendering once the current item is completely outside the viewport.
        if item.start_y_offset > self.content_end {
            return None;
        }
        self.cursor.next();

        let spacing = item.item.spacing();
        let content_width = self.max_width - spacing.x_axis_offset();
        let viewport_item = ViewportItem {
            viewport_offset: item.start_y_offset - self.content_start,
            content_offset: item.start_y_offset,
            content_size: vec2f(content_width.as_f32(), item.item.content_height().as_f32()),
            spacing,
            block_offset: item.start_char_offset,
        };
        Some((viewport_item, item.item))
    }
}

impl ViewportItem {
    /// The block backing this viewport item.
    pub fn block_offset(&self) -> CharOffset {
        self.block_offset
    }

    pub fn height(&self) -> f64 {
        // We sometimes encounter floating point errors when since we are seeking exactly on the edge of
        // a block item. Add a small buffer here so we could consistently seek to the right element.
        self.content_offset.as_f32() as f64 + 0.1
    }

    /// The content bounds of this item (see [`bounds::content_box`]).
    pub fn content_bounds(&self, ctx: &RenderContext) -> RectF {
        ctx.content_rect_to_screen(bounds::content_box(
            self.content_offset,
            self.content_size,
            &self.spacing,
        ))
    }

    /// The visible bounds of this item (see [`bounds::visible_box`]).
    pub fn visible_bounds(&self, ctx: &RenderContext) -> RectF {
        ctx.content_rect_to_screen(bounds::visible_box(
            self.content_offset,
            self.content_size,
            &self.spacing,
        ))
    }

    /// The reserved bounds of this item (see [`bounds::reserved_box`]).
    pub fn reserved_bounds(&self, ctx: &RenderContext) -> RectF {
        ctx.content_rect_to_screen(bounds::reserved_box(
            self.content_offset,
            self.content_size,
            &self.spacing,
        ))
    }
}

#[macro_export]
macro_rules! extract_block {
    ($viewport_item:expr, $content:expr, $match:pat => $value:expr) => {{
        let offset = $viewport_item.block_offset();
        match $content.block_at_offset(offset) {
            Some(block) => match (&block, block.item) {
                $match => $value,
                other => {
                    log::trace!("Unexpected block {other:?} at {}", offset);
                    return;
                }
            },
            None => return,
        }
    }};
}
