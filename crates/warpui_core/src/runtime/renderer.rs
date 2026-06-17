//! Flushes a [`TuiBuffer`] to a terminal (or any [`io::Write`] target) using
//! ratatui's cell diff and crossterm backend.
//!
//! [`TuiFrameRenderer`] keeps the previously drawn buffer and, on each draw,
//! asks ratatui's [`Buffer::diff`](TuiBuffer::diff) for the cells that changed
//! since the last frame and writes them through ratatui's [`CrosstermBackend`]
//! (which emits the minimal cursor-move + SGR + print sequence for each run).
//! The first frame — and any frame whose dimensions differ from the previous
//! one — clears the screen and repaints in full. Because it writes to a generic
//! writer, it is exercised headlessly against an in-memory buffer in tests
//! rather than requiring a real tty.

use std::io::{self, Write};

use ratatui::backend::{Backend, CrosstermBackend};
use ratatui::layout::Position;

use crate::elements::tui::TuiBuffer;

/// Renders successive [`TuiBuffer`]s to a writer, emitting only the per-frame
/// diff. Construct one per output target and reuse it across frames so it can
/// track the previously painted buffer.
pub struct TuiFrameRenderer {
    previous_buffer: Option<TuiBuffer>,
}

impl TuiFrameRenderer {
    pub fn new() -> Self {
        Self {
            previous_buffer: None,
        }
    }

    /// Forgets the previously drawn buffer so the next [`draw`](Self::draw)
    /// repaints the whole frame (e.g. after the host terminal was cleared by
    /// something outside the renderer).
    pub fn reset(&mut self) {
        self.previous_buffer = None;
    }

    /// Draws `buffer` to `writer`, emitting either a full repaint (first frame
    /// or a size change) or just the cells that differ from the previous frame,
    /// then positions or hides the cursor and flushes.
    pub fn draw<W: Write>(
        &mut self,
        writer: &mut W,
        buffer: &TuiBuffer,
        cursor_position: Option<(u16, u16)>,
    ) -> io::Result<()> {
        let mut backend = CrosstermBackend::new(writer);

        // First frame or a size change: clear, then repaint every cell by
        // diffing against a blank buffer of the new size. Otherwise diff
        // against the previously drawn frame so only changed cells are emitted.
        let repaint = self
            .previous_buffer
            .as_ref()
            .is_none_or(|previous| previous.area != buffer.area);
        let baseline = if repaint {
            backend.clear()?;
            TuiBuffer::empty(buffer.area)
        } else {
            self.previous_buffer
                .take()
                .expect("previous buffer present when not repainting")
        };

        backend.draw(baseline.diff(buffer).into_iter())?;

        match cursor_position {
            Some((x, y)) => {
                backend.set_cursor_position(Position::new(x, y))?;
                backend.show_cursor()?;
            }
            None => backend.hide_cursor()?,
        }

        Backend::flush(&mut backend)?;
        self.previous_buffer = Some(buffer.clone());
        Ok(())
    }
}

impl Default for TuiFrameRenderer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[path = "renderer_tests.rs"]
mod tests;
