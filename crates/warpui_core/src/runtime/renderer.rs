//! Flushes a [`TuiBuffer`] to a terminal (or any [`io::Write`] target) as a
//! minimal stream of crossterm commands.
//!
//! [`TuiFrameRenderer`] keeps the previously drawn buffer and, on each draw,
//! emits only the cells that changed since the last frame — moving the cursor to
//! each changed run and printing it with its style. The first frame (and any
//! frame whose dimensions differ from the previous one) is painted in full.
//! Because it writes to a generic writer, it is exercised headlessly against an
//! in-memory buffer in tests rather than requiring a real tty.

use std::io::{self, Write};

use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::queue;
use crossterm::style::{
    Attribute, Color, Print, ResetColor, SetAttribute, SetBackgroundColor, SetForegroundColor,
};
use crossterm::terminal::{Clear, ClearType};

use crate::elements::tui::{TuiBuffer, TuiStyle};

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
        let runs = if self.should_repaint(buffer) {
            queue!(writer, Clear(ClearType::All))?;
            full_frame_runs(buffer)
        } else {
            changed_runs(self.previous_buffer.as_ref().unwrap(), buffer)
        };

        for run in &runs {
            draw_run(writer, run)?;
        }
        queue!(writer, ResetColor, SetAttribute(Attribute::Reset))?;
        draw_cursor(writer, cursor_position)?;

        writer.flush()?;
        self.previous_buffer = Some(buffer.clone());
        Ok(())
    }

    fn should_repaint(&self, buffer: &TuiBuffer) -> bool {
        self.previous_buffer
            .as_ref()
            .is_none_or(|previous| previous.size() != buffer.size())
    }
}

impl Default for TuiFrameRenderer {
    fn default() -> Self {
        Self::new()
    }
}

/// A horizontal run of same-styled cells to emit with a single cursor move.
#[derive(Debug, Eq, PartialEq)]
struct StyledRun {
    x: u16,
    y: u16,
    text: String,
    style: TuiStyle,
}

/// Every row's full contents, split into style runs — used for a full repaint.
fn full_frame_runs(buffer: &TuiBuffer) -> Vec<StyledRun> {
    let size = buffer.size();
    let mut runs = Vec::new();
    for y in 0..size.height {
        let mut x = 0;
        while x < size.width {
            let (run, next_x) = collect_run(buffer, x, y, size.width);
            runs.push(run);
            x = next_x;
        }
    }
    runs
}

/// The runs that differ between `previous` and `next`, coalescing adjacent
/// changed cells that share a style (and absorbing wide-grapheme continuation
/// columns) into one run per cursor move.
fn changed_runs(previous: &TuiBuffer, next: &TuiBuffer) -> Vec<StyledRun> {
    if previous.size() != next.size() {
        return full_frame_runs(next);
    }

    let size = next.size();
    let mut runs = Vec::new();
    for y in 0..size.height {
        let mut x = 0;
        while x < size.width {
            if !cell_changed(previous, next, x, y) {
                x += 1;
                continue;
            }

            let start_x = x;
            let style = cell_style(next, x, y);
            x += 1;
            while x < size.width
                && cell_changed(previous, next, x, y)
                && next
                    .get(x, y)
                    .is_some_and(|cell| cell.is_continuation() || cell.style() == style)
            {
                x += 1;
            }
            // Absorb any trailing continuation columns of the last grapheme so a
            // changed wide glyph is emitted whole.
            while x < size.width && next.get(x, y).is_some_and(|cell| cell.is_continuation()) {
                x += 1;
            }

            runs.push(run_text(next, start_x, x, y, style));
        }
    }
    runs
}

/// Collects the maximal same-styled run starting at `(start_x, y)`, returning
/// the run and the first column past it.
fn collect_run(buffer: &TuiBuffer, start_x: u16, y: u16, end_x: u16) -> (StyledRun, u16) {
    let style = cell_style(buffer, start_x, y);
    let mut x = start_x + 1;
    while x < end_x
        && buffer
            .get(x, y)
            .is_some_and(|cell| cell.is_continuation() || cell.style() == style)
    {
        x += 1;
    }
    (run_text(buffer, start_x, x, y, style), x)
}

fn run_text(buffer: &TuiBuffer, start_x: u16, end_x: u16, y: u16, style: TuiStyle) -> StyledRun {
    let mut text = String::new();
    for x in start_x..end_x {
        if let Some(cell) = buffer.get(x, y) {
            if !cell.is_continuation() {
                text.push_str(cell.symbol());
            }
        }
    }
    // A run consisting solely of continuation columns has no symbols of its own;
    // fall back to spaces so the changed columns are still cleared.
    if text.is_empty() {
        text = " ".repeat(usize::from(end_x.saturating_sub(start_x)));
    }
    StyledRun {
        x: start_x,
        y,
        text,
        style,
    }
}

fn draw_run<W: Write>(writer: &mut W, run: &StyledRun) -> io::Result<()> {
    queue!(writer, MoveTo(run.x, run.y), SetAttribute(Attribute::Reset))?;
    queue!(
        writer,
        SetForegroundColor(run.style.foreground.unwrap_or(Color::Reset)),
        SetBackgroundColor(run.style.background.unwrap_or(Color::Reset)),
    )?;
    if run.style.bold {
        queue!(writer, SetAttribute(Attribute::Bold))?;
    }
    if run.style.dim {
        queue!(writer, SetAttribute(Attribute::Dim))?;
    }
    if run.style.italic {
        queue!(writer, SetAttribute(Attribute::Italic))?;
    }
    if run.style.underline {
        queue!(writer, SetAttribute(Attribute::Underlined))?;
    }
    if run.style.reversed {
        queue!(writer, SetAttribute(Attribute::Reverse))?;
    }
    queue!(writer, Print(&run.text))
}

fn draw_cursor<W: Write>(writer: &mut W, cursor_position: Option<(u16, u16)>) -> io::Result<()> {
    match cursor_position {
        Some((x, y)) => queue!(writer, MoveTo(x, y), Show),
        None => queue!(writer, Hide),
    }
}

fn cell_changed(previous: &TuiBuffer, next: &TuiBuffer, x: u16, y: u16) -> bool {
    previous.get(x, y) != next.get(x, y)
}

fn cell_style(buffer: &TuiBuffer, x: u16, y: u16) -> TuiStyle {
    buffer
        .get(x, y)
        .map(|cell| cell.style())
        .unwrap_or_default()
}

#[cfg(test)]
#[path = "renderer_tests.rs"]
mod tests;
