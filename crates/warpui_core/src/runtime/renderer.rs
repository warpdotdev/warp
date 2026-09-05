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
//!
//! # Hyperlinks
//!
//! Ratatui's `Cell` carries no hyperlink attribute, so the caller passes a
//! side table of URLs keyed by buffer cell alongside the buffer itself (built
//! during paint — see `TuiPaintContext::record_hyperlink`). Ratatui's own
//! diff only compares glyph and style, so a cell whose hyperlink changed but
//! whose glyph and style did not would otherwise never be repainted; this
//! renderer tracks the previous frame's table and unions cells whose
//! hyperlink changed into the diff before drawing. A full repaint (first
//! frame or resize) treats the previous table as empty, so every current
//! hyperlink is re-emitted along with the rest of the frame. Each repainted
//! run of cells sharing a hyperlink is bracketed in its own OSC 8 open/close
//! pair, so a hyperlink is never left open across a batch boundary and a run
//! split by wrapping still carries the full URL on every fragment.

use std::collections::{HashMap, HashSet};
use std::io::{self, Write};
use std::rc::Rc;

use ratatui::backend::{Backend, CrosstermBackend};
use ratatui::buffer::{Cell, CellWidth};
use ratatui::crossterm::queue;
use ratatui::crossterm::terminal::{BeginSynchronizedUpdate, EndSynchronizedUpdate};
use ratatui::layout::Position;

use crate::elements::tui::TuiBuffer;

/// Renders successive [`TuiBuffer`]s to a writer, emitting only the per-frame
/// diff. Construct one per output target and reuse it across frames so it can
/// track the previously painted buffer.
pub struct TuiFrameRenderer {
    previous_buffer: Option<TuiBuffer>,
    /// The hyperlink table from the last [`draw`](Self::draw) call, so the
    /// next call can tell which cells' hyperlinks changed (see the module
    /// docs).
    previous_hyperlinks: HashMap<(u16, u16), Rc<str>>,
}

impl TuiFrameRenderer {
    pub fn new() -> Self {
        Self {
            previous_buffer: None,
            previous_hyperlinks: HashMap::new(),
        }
    }

    /// Forgets the previously drawn buffer so the next [`draw`](Self::draw)
    /// repaints the whole frame (e.g. after the host terminal was cleared by
    /// something outside the renderer).
    pub fn reset(&mut self) {
        self.previous_buffer = None;
        self.previous_hyperlinks.clear();
    }

    /// Draws `buffer` to `writer`, emitting either a full repaint (first frame
    /// or a size change) or just the cells that differ from the previous frame,
    /// then positions or hides the cursor and flushes. The whole frame is
    /// wrapped in a synchronized update so it is applied atomically.
    ///
    /// `hyperlinks` carries the URL each buffer cell should be tagged with, if
    /// any (see the module docs).
    pub fn draw<W: Write>(
        &mut self,
        writer: &mut W,
        buffer: &TuiBuffer,
        cursor_position: Option<(u16, u16)>,
        hyperlinks: &HashMap<(u16, u16), Rc<str>>,
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

        // A full repaint already re-emits every current hyperlink (nothing in
        // `baseline` carries one), so only a partial redraw needs the explicit
        // union below.
        let empty_hyperlinks = HashMap::new();
        let previous_hyperlinks = if repaint {
            &empty_hyperlinks
        } else {
            &self.previous_hyperlinks
        };

        // Ratatui's diff only compares glyph and style, so a cell whose
        // hyperlink changed but whose glyph and style did not would otherwise
        // never be repainted (see the module docs).
        let mut diff: Vec<(u16, u16, &Cell)> = baseline.diff(buffer);
        let existing_diff_positions: HashSet<(u16, u16)> =
            diff.iter().map(|&(x, y, _)| (x, y)).collect();
        for position in hyperlinks.keys().chain(previous_hyperlinks.keys()) {
            if hyperlinks.get(position) != previous_hyperlinks.get(position)
                && !existing_diff_positions.contains(position)
            {
                diff.push((position.0, position.1, &buffer[(position.0, position.1)]));
            }
        }
        // Restores row-major order (ratatui's own diff order) after appending
        // the hyperlink-only positions above, which the wide-grapheme batching
        // below depends on.
        diff.sort_by_key(|&(x, y, _)| (y, x));

        // Paint continuation cells before each changed wide grapheme. Ratatui
        // omits style-only continuation diffs, while painting them afterward
        // can shift following cells or erase the grapheme. Keeping the wide
        // grapheme in its own batch also gives following cells a fresh MoveTo.
        let mut batch_start = 0;
        let mut index = 0;
        while index < diff.len() {
            let (wide_x, wide_y, cell) = diff[index];
            let wide_width = cell.cell_width();
            if wide_width <= 1 {
                index += 1;
                continue;
            }

            if batch_start < index {
                write_cell_run(
                    &mut backend,
                    diff[batch_start..index].iter().copied(),
                    hyperlinks,
                )?;
            }

            let trailing_diff_end = index
                + 1
                + diff[index + 1..]
                    .iter()
                    .take_while(|(x, y, _)| *y == wide_y && *x < wide_x.saturating_add(wide_width))
                    .count();
            let trailing_cell_end = wide_x
                .saturating_add(wide_width)
                .min(buffer.area.x.saturating_add(buffer.area.width));
            if wide_x.saturating_add(1) < trailing_cell_end {
                write_cell_run(
                    &mut backend,
                    (wide_x.saturating_add(1)..trailing_cell_end)
                        .map(|x| (x, wide_y, &buffer[(x, wide_y)])),
                    hyperlinks,
                )?;
            }
            write_cell_run(
                &mut backend,
                diff[index..=index].iter().copied(),
                hyperlinks,
            )?;

            index = trailing_diff_end;
            batch_start = index;
        }
        if batch_start < diff.len() {
            write_cell_run(
                &mut backend,
                diff[batch_start..].iter().copied(),
                hyperlinks,
            )?;
        }

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
        self.previous_hyperlinks = hyperlinks.clone();
        Ok(())
    }
}

impl Default for TuiFrameRenderer {
    fn default() -> Self {
        Self::new()
    }
}

/// Draws `cells` to `backend`, splitting them into maximal runs that share the
/// same hyperlink (including "no hyperlink") and bracketing a hyperlinked run
/// in its own OSC 8 open/close pair. Cells may jump around the screen (ratatui
/// diffs are sparse); a `Backend::draw` call handles the cursor `MoveTo`s
/// itself, and OSC 8 is a sticky terminal attribute rather than something tied
/// to cursor position, so a non-contiguous run is still correctly tagged.
fn write_cell_run<'a, W: Write>(
    backend: &mut CrosstermBackend<&mut W>,
    cells: impl Iterator<Item = (u16, u16, &'a Cell)>,
    hyperlinks: &HashMap<(u16, u16), Rc<str>>,
) -> io::Result<()> {
    let cells: Vec<_> = cells.collect();
    let mut start = 0;
    while start < cells.len() {
        let run_url = hyperlinks.get(&(cells[start].0, cells[start].1));
        let end = cells[start..]
            .iter()
            .position(|&(x, y, _)| hyperlinks.get(&(x, y)) != run_url)
            .map_or(cells.len(), |offset| start + offset);
        if let Some(url) = run_url {
            write_hyperlink_escape(backend, Some(url))?;
        }
        let draw_result = backend.draw(cells[start..end].iter().copied());
        if run_url.is_some() {
            if draw_result.is_err() {
                // The draw itself failed after the hyperlink was opened. Still
                // attempt the close so a transient writer error can't leave
                // every later line of output clickable; the close's own
                // result is secondary; the draw error below is what the
                // caller needs to see.
                let _ = write_hyperlink_escape(backend, None);
            } else {
                write_hyperlink_escape(backend, None)?;
            }
        }
        draw_result?;
        start = end;
    }
    Ok(())
}

/// Writes an OSC 8 hyperlink escape: `Some(url)` opens a hyperlink covering
/// subsequently printed cells, `None` closes it. `CrosstermBackend` implements
/// `io::Write` by forwarding to its inner writer, so this interleaves
/// correctly with the `Backend::draw` calls around it in the same byte stream.
/// Builds the complete escape into one buffer and writes it with a single
/// `write_all` call rather than `write!`, since `write!`'s format
/// interpolation can otherwise fragment one escape across multiple
/// `Write::write` calls, which callers observing the raw byte stream (or a
/// writer that fails mid-stream) should never see as a partial escape.
fn write_hyperlink_escape<W: Write>(
    backend: &mut CrosstermBackend<&mut W>,
    url: Option<&str>,
) -> io::Result<()> {
    let escape = match url {
        // Strips control bytes so a malformed or malicious URL can't inject
        // further escape sequences into the terminal stream.
        Some(url) => format!(
            "\x1b]8;;{}\x1b\\",
            url.chars().filter(|c| !c.is_control()).collect::<String>()
        ),
        None => "\x1b]8;;\x1b\\".to_owned(),
    };
    backend.write_all(escape.as_bytes())
}

#[cfg(test)]
#[path = "renderer_tests.rs"]
mod tests;
