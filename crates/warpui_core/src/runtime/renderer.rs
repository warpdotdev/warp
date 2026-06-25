//! Flushes a [`TuiBuffer`] to a terminal (or any [`io::Write`] target) using
//! ratatui's cell diff and crossterm backend.
//!
//! [`TuiFrameRenderer`] keeps the previously drawn buffer and, on each draw,
//! asks ratatui's [`Buffer::diff`](TuiBuffer::diff) for the cells that changed
//! since the last frame and writes them through ratatui's [`CrosstermBackend`]
//! (which emits the minimal cursor-move + SGR + print sequence for each run).
//!
//! The first frame, and any frame whose dimensions differ from the previous one
//! (a resize), is painted in full: the screen is cleared and every non-blank
//! cell redrawn. Clearing is required for correctness because a terminal keeps
//! its old contents across a resize while the text reflows to a new width — a
//! plain diff would leave stale fragments behind. To keep that clear + repaint
//! from flickering, the whole frame is wrapped in a terminal *synchronized
//! update*, so a supporting terminal presents the cleared-and-repainted frame
//! atomically and never shows the blank intermediate state.
//!
//! Because it writes to a generic writer, it is exercised headlessly against an
//! in-memory buffer in tests rather than requiring a real tty.

use std::io::{self, Write};

use ratatui::backend::{Backend, CrosstermBackend};
use ratatui::crossterm::queue;
use ratatui::crossterm::terminal::{BeginSynchronizedUpdate, EndSynchronizedUpdate};
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
    /// then positions or hides the cursor and flushes. The whole frame is
    /// wrapped in a synchronized update so it is applied atomically.
    pub fn draw<W: Write>(
        &mut self,
        writer: &mut W,
        buffer: &TuiBuffer,
        cursor_position: Option<(u16, u16)>,
    ) -> io::Result<()> {
        let mut backend = CrosstermBackend::new(writer);

        // Group the whole frame into one synchronized update so the terminal
        // applies it atomically — in particular, the clear + repaint on a
        // resize is presented as a single frame, never as a visible blank.
        queue!(backend, BeginSynchronizedUpdate)?;

        // First frame or a size change: clear, then diff against a blank buffer
        // of the new size. The clear overwrites the stale contents the terminal
        // keeps across a resize (the text reflows to a new width), which a plain
        // diff against the previous frame could not do.
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

        queue!(backend, EndSynchronizedUpdate)?;
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
