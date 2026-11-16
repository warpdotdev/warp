//! The styled cell grid the element tree paints into.
//!
//! This is ratatui's `Buffer` (re-exported as [`TuiBuffer`]) with `Style`
//! re-exported as [`TuiStyle`] and `Cell` re-exported for convenience. Elements
//! paint with the buffer's own grapheme-aware writers (`set_string`,
//! `cell_mut`, `set_style`); the diff/flush to the terminal is the ratatui
//! `Terminal`'s job, wired up by the runtime.
//!
//! [`TuiBufferExt::to_lines`] is the headless assertion hook used throughout the
//! element tests: it renders each row to a `String`, skipping the trailing
//! columns of wide graphemes so every glyph appears exactly once (mirroring how
//! ratatui's own `Buffer` debug output collapses multi-width cells).
use std::ops::Range;

use ratatui::buffer::CellWidth;
pub use ratatui::buffer::{Buffer as TuiBuffer, Cell};
pub use ratatui::style::{Color, Modifier, Style as TuiStyle};
use ratatui::widgets::{Paragraph, Widget};

use super::geometry::{TuiPoint, TuiRect, TuiRectExt, TuiSize};
use super::scene::TuiScreenPosition;
/// A ratatui widget that can render a framework-computed visible row window.
///
/// Implementations translate `clipped_rows_above` into the widget's own
/// logical content offset. Elements submit the complete widget through
/// [`TuiPaintSurface::render_widget`]; the paint surface owns visibility and
/// clipping decisions.
pub trait TuiWidget {
    /// Paints the visible widget area after omitting logical rows clipped above it.
    fn render_visible(self, area: TuiRect, clipped_rows_above: u16, buffer: &mut TuiBuffer);
}

impl TuiWidget for Paragraph<'_> {
    fn render_visible(self, area: TuiRect, clipped_rows_above: u16, buffer: &mut TuiBuffer) {
        self.scroll((clipped_rows_above, 0)).render(area, buffer);
    }
}
struct VisibleWidgetArea {
    area: TuiRect,
    clipped_columns_left: u16,
    clipped_rows_above: u16,
}

/// Absolute-coordinate paint access to one ratatui buffer.
pub struct TuiPaintSurface<'a> {
    buffer: &'a mut TuiBuffer,
    screen_origin: TuiScreenPosition,
    buffer_origin: TuiPoint,
    clip: TuiRect,
}

impl<'a> TuiPaintSurface<'a> {
    /// Creates an identity-mapped surface over `buffer`.
    pub fn new(buffer: &'a mut TuiBuffer) -> Self {
        let buffer_origin = TuiPoint::new(buffer.area.x, buffer.area.y);
        let clip = buffer.area;
        Self {
            buffer,
            screen_origin: TuiScreenPosition::new(
                i32::from(buffer_origin.x),
                i32::from(buffer_origin.y),
            ),
            buffer_origin,
            clip,
        }
    }
    /// Reborrows this surface through an additional absolute screen-space clip.
    ///
    /// All cell, style, and widget writes performed by `paint` are restricted
    /// to the intersection of this clip, the parent clip, and the backing
    /// buffer. Returns `None` without painting when the clip is fully outside
    /// the parent surface.
    pub fn with_clip<R>(
        &mut self,
        origin: TuiScreenPosition,
        size: TuiSize,
        paint: impl FnOnce(&mut TuiPaintSurface<'_>) -> R,
    ) -> Option<R> {
        let clip = self.clipped_buffer_rect(origin, size)?;
        let mut clipped = TuiPaintSurface {
            buffer: &mut *self.buffer,
            screen_origin: self.screen_origin,
            buffer_origin: self.buffer_origin,
            clip,
        };
        Some(paint(&mut clipped))
    }

    /// Maps `screen_origin` to the top-left cell of `buffer`.
    pub fn mapped(buffer: &'a mut TuiBuffer, screen_origin: TuiScreenPosition) -> Self {
        let clip = buffer.area;
        Self {
            buffer_origin: TuiPoint::new(buffer.area.x, buffer.area.y),
            buffer,
            screen_origin,
            clip,
        }
    }

    /// Renders a widget within the visible part of its absolute screen bounds.
    pub fn render_widget<W: TuiWidget>(
        &mut self,
        origin: TuiScreenPosition,
        size: TuiSize,
        widget: W,
    ) -> bool {
        let Some(visible) = self.visible_widget_buffer_area(origin, size) else {
            return false;
        };
        if visible.area.width == size.width {
            widget.render_visible(visible.area, visible.clipped_rows_above, self.buffer);
            return true;
        }

        let scratch_area = TuiRect::new(0, 0, size.width, visible.area.height);
        let mut scratch = TuiBuffer::empty(scratch_area);
        widget.render_visible(scratch_area, visible.clipped_rows_above, &mut scratch);
        for row in 0..visible.area.height {
            for column in 0..visible.area.width {
                let source_column = visible.clipped_columns_left.saturating_add(column);
                self.buffer[(visible.area.x + column, visible.area.y + row)] =
                    scratch[(source_column, row)].clone();
            }
        }
        true
    }

    /// Applies `style` to the visible part of the absolute screen bounds.
    pub fn set_style(&mut self, origin: TuiScreenPosition, size: TuiSize, style: TuiStyle) {
        let Some(area) = self.clipped_buffer_rect(origin, size) else {
            return;
        };
        if !area.is_empty() {
            self.buffer.set_style(area, style);
        }
    }

    /// Returns the cell at an absolute screen position.
    pub fn cell(&self, position: TuiScreenPosition) -> Option<&Cell> {
        self.buffer_point(position)
            .and_then(|position| self.buffer.cell(position))
    }

    /// Returns the mutable cell at an absolute screen position.
    pub fn cell_mut(&mut self, position: TuiScreenPosition) -> Option<&mut Cell> {
        self.buffer_point(position)
            .and_then(|position| self.buffer.cell_mut(position))
    }

    /// Replaces the cell at an absolute screen position.
    pub fn set_cell(&mut self, position: TuiScreenPosition, cell: Cell) -> bool {
        let Some(destination) = self.cell_mut(position) else {
            return false;
        };
        *destination = cell;
        true
    }

    /// Returns the element-local rows intersecting the active clip.
    pub fn visible_rows(&self, origin: TuiScreenPosition, size: TuiSize) -> Option<Range<u16>> {
        let visible = self.visible_widget_buffer_area(origin, size)?;
        Some(
            visible.clipped_rows_above
                ..visible
                    .clipped_rows_above
                    .saturating_add(visible.area.height),
        )
    }

    fn visible_widget_buffer_area(
        &self,
        origin: TuiScreenPosition,
        size: TuiSize,
    ) -> Option<VisibleWidgetArea> {
        let (x, y) = self.signed_buffer_point(origin)?;
        let right = x.checked_add(i64::from(size.width))?;
        let bottom = y.checked_add(i64::from(size.height))?;
        let clip_left = i64::from(self.clip.x);
        let clip_right = i64::from(self.clip.right());
        let visible_left = x.max(clip_left);
        let visible_right = right.min(clip_right);
        let visible_top = y.max(i64::from(self.clip.y));
        let visible_bottom = bottom.min(i64::from(self.clip.bottom()));
        if visible_left >= visible_right || visible_top >= visible_bottom {
            return None;
        }
        Some(VisibleWidgetArea {
            area: TuiRect::new(
                u16::try_from(visible_left).ok()?,
                u16::try_from(visible_top).ok()?,
                u16::try_from(visible_right.checked_sub(visible_left)?).ok()?,
                u16::try_from(visible_bottom.checked_sub(visible_top)?).ok()?,
            ),
            clipped_columns_left: u16::try_from(visible_left.checked_sub(x)?).ok()?,
            clipped_rows_above: u16::try_from(visible_top.checked_sub(y)?).ok()?,
        })
    }

    fn clipped_buffer_rect(&self, origin: TuiScreenPosition, size: TuiSize) -> Option<TuiRect> {
        let (x, y) = self.signed_buffer_point(origin)?;
        let right = x.checked_add(i64::from(size.width))?;
        let bottom = y.checked_add(i64::from(size.height))?;
        let left = x.max(i64::from(self.clip.x));
        let top = y.max(i64::from(self.clip.y));
        let right = right.min(i64::from(self.clip.right()));
        let bottom = bottom.min(i64::from(self.clip.bottom()));
        if left >= right || top >= bottom {
            return None;
        }
        Some(TuiRect::new(
            u16::try_from(left).ok()?,
            u16::try_from(top).ok()?,
            u16::try_from(right.checked_sub(left)?).ok()?,
            u16::try_from(bottom.checked_sub(top)?).ok()?,
        ))
    }

    fn buffer_point(&self, position: TuiScreenPosition) -> Option<TuiPoint> {
        let (x, y) = self.signed_buffer_point(position)?;
        let point = TuiPoint::new(u16::try_from(x).ok()?, u16::try_from(y).ok()?);
        self.clip.contains_point(point).then_some(point)
    }

    fn signed_buffer_point(&self, position: TuiScreenPosition) -> Option<(i64, i64)> {
        let x = i64::from(self.buffer_origin.x)
            .checked_add(i64::from(position.x).checked_sub(i64::from(self.screen_origin.x))?)?;
        let y = i64::from(self.buffer_origin.y)
            .checked_add(i64::from(position.y).checked_sub(i64::from(self.screen_origin.y))?)?;
        Some((x, y))
    }
}

/// Headless rendering of a [`TuiBuffer`] to one `String` per row.
pub trait TuiBufferExt {
    /// Renders the buffer to one `String` per row, emitting each grapheme once
    /// by skipping the trailing columns a wide grapheme occupies.
    fn to_lines(&self) -> Vec<String>;
}

impl TuiBufferExt for TuiBuffer {
    fn to_lines(&self) -> Vec<String> {
        let area = self.area;
        (0..area.height)
            .map(|row| {
                let mut line = String::new();
                let mut skip = 0u16;
                for column in 0..area.width {
                    let cell = &self[(area.x + column, area.y + row)];
                    if skip == 0 {
                        line.push_str(cell.symbol());
                        skip = cell.cell_width().max(1) - 1;
                    } else {
                        skip -= 1;
                    }
                }
                line
            })
            .collect()
    }
}

#[cfg(test)]
#[path = "buffer_tests.rs"]
mod tests;
