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

use ratatui::buffer::CellWidth;
pub use ratatui::buffer::{Buffer as TuiBuffer, Cell};
pub use ratatui::style::{Color, Modifier, Style as TuiStyle};
use ratatui::widgets::Widget;

use super::geometry::{TuiPoint, TuiRect, TuiSize};
use super::scene::TuiScreenPosition;

/// Absolute-coordinate paint access to one ratatui buffer.
pub struct TuiPaintSurface<'a> {
    buffer: &'a mut TuiBuffer,
    screen_origin: TuiScreenPosition,
    buffer_origin: TuiPoint,
}

impl<'a> TuiPaintSurface<'a> {
    /// Creates an identity-mapped surface over `buffer`.
    pub fn new(buffer: &'a mut TuiBuffer) -> Self {
        let buffer_origin = TuiPoint::new(buffer.area.x, buffer.area.y);
        Self {
            buffer,
            screen_origin: TuiScreenPosition::new(
                i32::from(buffer_origin.x),
                i32::from(buffer_origin.y),
            ),
            buffer_origin,
        }
    }

    /// Maps `screen_origin` to the top-left cell of `buffer`.
    pub fn mapped(buffer: &'a mut TuiBuffer, screen_origin: TuiScreenPosition) -> Self {
        Self {
            buffer_origin: TuiPoint::new(buffer.area.x, buffer.area.y),
            buffer,
            screen_origin,
        }
    }

    /// Renders a ratatui widget within absolute screen bounds.
    pub fn render_widget(
        &mut self,
        widget: impl Widget,
        origin: TuiScreenPosition,
        size: TuiSize,
    ) -> bool {
        let Some(area) = self.contained_buffer_rect(origin, size) else {
            return false;
        };
        widget.render(area, self.buffer);
        true
    }

    /// Applies `style` to the visible part of the absolute screen bounds.
    pub fn set_style(&mut self, origin: TuiScreenPosition, size: TuiSize, style: TuiStyle) {
        let Some(area) = self.buffer_rect(origin, size) else {
            return;
        };
        let area = area.intersection(self.buffer.area);
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

    fn contained_buffer_rect(&self, origin: TuiScreenPosition, size: TuiSize) -> Option<TuiRect> {
        let area = self.buffer_rect(origin, size)?;
        (area.intersection(self.buffer.area) == area).then_some(area)
    }

    fn buffer_rect(&self, origin: TuiScreenPosition, size: TuiSize) -> Option<TuiRect> {
        let origin = self.buffer_point(origin)?;
        origin.x.checked_add(size.width)?;
        origin.y.checked_add(size.height)?;
        Some(TuiRect::new(origin.x, origin.y, size.width, size.height))
    }

    fn buffer_point(&self, position: TuiScreenPosition) -> Option<TuiPoint> {
        let x = i64::from(self.buffer_origin.x)
            .checked_add(i64::from(position.x).checked_sub(i64::from(self.screen_origin.x))?)?;
        let y = i64::from(self.buffer_origin.y)
            .checked_add(i64::from(position.y).checked_sub(i64::from(self.screen_origin.y))?)?;
        Some(TuiPoint::new(
            u16::try_from(x).ok()?,
            u16::try_from(y).ok()?,
        ))
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
