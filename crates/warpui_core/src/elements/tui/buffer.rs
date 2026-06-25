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
