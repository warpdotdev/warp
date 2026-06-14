use std::io::{self, Write};

use crate::{TuiBuffer, TuiFrame, TuiStyle};
use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::queue;
use crossterm::style::{Print, ResetColor, SetForegroundColor};
use crossterm::terminal::{Clear, ClearType};

pub struct TuiFrameRenderer {
    previous_frame: Option<TuiFrame>,
}

impl TuiFrameRenderer {
    pub fn new() -> Self {
        Self {
            previous_frame: None,
        }
    }

    pub fn draw(&mut self, writer: &mut impl Write, frame: &TuiFrame) -> io::Result<()> {
        if self.should_draw_full_frame(frame) {
            draw_full_frame(writer, frame)?;
        } else if let Some(previous_frame) = &self.previous_frame {
            draw_changed_runs(writer, &previous_frame.buffer, &frame.buffer)?;
            draw_cursor(writer, frame)?;
        }

        writer.flush()?;
        self.previous_frame = Some(frame.clone());
        Ok(())
    }

    fn should_draw_full_frame(&self, frame: &TuiFrame) -> bool {
        self.previous_frame
            .as_ref()
            .is_none_or(|previous_frame| previous_frame.buffer.size() != frame.buffer.size())
    }
}

impl Default for TuiFrameRenderer {
    fn default() -> Self {
        Self::new()
    }
}

fn draw_full_frame(writer: &mut impl Write, frame: &TuiFrame) -> io::Result<()> {
    queue!(writer, MoveTo(0, 0), Clear(ClearType::All))?;
    for row in 0..frame.buffer.size().height {
        for run in styled_runs_for_range(&frame.buffer, row, 0, frame.buffer.size().width) {
            draw_styled_run(writer, &run)?;
        }
    }

    queue!(writer, ResetColor)?;
    draw_cursor(writer, frame)
}

fn draw_changed_runs(
    writer: &mut impl Write,
    previous_buffer: &TuiBuffer,
    next_buffer: &TuiBuffer,
) -> io::Result<()> {
    for run in changed_runs(previous_buffer, next_buffer) {
        draw_styled_run(writer, &run)?;
    }
    queue!(writer, ResetColor)?;
    Ok(())
}

fn draw_cursor(writer: &mut impl Write, frame: &TuiFrame) -> io::Result<()> {
    if let Some((x, y)) = frame.cursor_position {
        queue!(writer, MoveTo(x, y), Show)
    } else {
        queue!(writer, Hide)
    }
}

#[derive(Debug, Eq, PartialEq)]
struct ChangedRun {
    x: u16,
    y: u16,
    text: String,
    style: TuiStyle,
}

fn changed_runs(previous_buffer: &TuiBuffer, next_buffer: &TuiBuffer) -> Vec<ChangedRun> {
    if previous_buffer.size() != next_buffer.size() {
        return Vec::new();
    }

    let size = next_buffer.size();
    let mut runs = Vec::new();

    for y in 0..size.height {
        let mut x = 0;
        while x < size.width {
            if !cell_changed(previous_buffer, next_buffer, x, y) {
                x = x.saturating_add(1);
                continue;
            }

            let start_x = x;
            let style = next_buffer
                .cell(x, y)
                .map(|cell| cell.style())
                .unwrap_or_default();
            x = x.saturating_add(1);
            while x < size.width
                && cell_changed(previous_buffer, next_buffer, x, y)
                && next_buffer
                    .cell(x, y)
                    .is_some_and(|cell| cell.is_continuation() || cell.style() == style)
            {
                x = x.saturating_add(1);
            }
            while x < size.width
                && next_buffer
                    .cell(x, y)
                    .is_some_and(|cell| cell.is_continuation())
            {
                x = x.saturating_add(1);
            }

            let mut text = next_buffer.display_string_for_range(y, start_x, x);
            if text.is_empty() {
                text = " ".repeat(usize::from(x.saturating_sub(start_x)));
            }
            runs.push(ChangedRun {
                x: start_x,
                y,
                text,
                style,
            });
        }
    }

    runs
}

fn styled_runs_for_range(buffer: &TuiBuffer, y: u16, start_x: u16, end_x: u16) -> Vec<ChangedRun> {
    let mut runs = Vec::new();
    let mut x = start_x;
    let end_x = end_x.min(buffer.size().width);
    while x < end_x {
        let Some(cell) = buffer.cell(x, y) else {
            break;
        };
        let style = cell.style();
        let run_start_x = x;
        x = x.saturating_add(1);
        while x < end_x
            && buffer
                .cell(x, y)
                .is_some_and(|cell| cell.is_continuation() || cell.style() == style)
        {
            x = x.saturating_add(1);
        }

        let mut text = buffer.display_string_for_range(y, run_start_x, x);
        if text.is_empty() {
            text = " ".repeat(usize::from(x.saturating_sub(run_start_x)));
        }
        runs.push(ChangedRun {
            x: run_start_x,
            y,
            text,
            style,
        });
    }
    runs
}

fn draw_styled_run(writer: &mut impl Write, run: &ChangedRun) -> io::Result<()> {
    match run.style.foreground_color() {
        Some(color) => queue!(writer, MoveTo(run.x, run.y), SetForegroundColor(color))?,
        None => queue!(writer, MoveTo(run.x, run.y), ResetColor)?,
    }
    queue!(writer, Print(&run.text))
}

fn cell_changed(previous_buffer: &TuiBuffer, next_buffer: &TuiBuffer, x: u16, y: u16) -> bool {
    previous_buffer.cell(x, y) != next_buffer.cell(x, y)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TuiSize;
    use crossterm::style::Color;

    fn run(x: u16, y: u16, text: &str) -> ChangedRun {
        styled_run(x, y, text, TuiStyle::default())
    }

    fn styled_run(x: u16, y: u16, text: &str, style: TuiStyle) -> ChangedRun {
        ChangedRun {
            x,
            y,
            text: text.to_owned(),
            style,
        }
    }

    fn buffer(lines: &[&str], width: u16) -> TuiBuffer {
        let mut buffer = TuiBuffer::new(TuiSize::new(width, lines.len().try_into().unwrap()));
        for (row, line) in lines.iter().enumerate() {
            buffer.write_str(0, row.try_into().unwrap(), width, line);
        }
        buffer
    }

    #[test]
    fn unchanged_frames_emit_no_runs() {
        let previous = buffer(&["abc"], 3);
        let next = buffer(&["abc"], 3);

        assert_eq!(changed_runs(&previous, &next), Vec::new());
    }

    #[test]
    fn single_row_edits_are_coalesced() {
        let previous = buffer(&["abcde"], 5);
        let next = buffer(&["abXYe"], 5);

        assert_eq!(changed_runs(&previous, &next), vec![run(2, 0, "XY")]);
    }

    #[test]
    fn scroll_like_changes_emit_changed_rows() {
        let previous = buffer(&["one", "two", "thr"], 3);
        let next = buffer(&["two", "thr", "fou"], 3);

        assert_eq!(
            changed_runs(&previous, &next),
            vec![run(0, 0, "two"), run(1, 1, "hr"), run(0, 2, "fou")]
        );
    }

    #[test]
    fn wide_cell_changes_include_continuation_columns() {
        let previous = buffer(&["ab "], 3);
        let next = buffer(&["界 "], 3);

        assert_eq!(changed_runs(&previous, &next), vec![run(0, 0, "界")]);
    }

    #[test]
    fn replacing_wide_cells_clears_leftover_columns() {
        let previous = buffer(&["界 "], 3);
        let next = buffer(&["ab "], 3);

        assert_eq!(changed_runs(&previous, &next), vec![run(0, 0, "ab")]);
    }

    #[test]
    fn style_changes_emit_styled_runs() {
        let previous = buffer(&["abc"], 3);
        let mut next = buffer(&["abc"], 3);
        let style = TuiStyle::default().with_foreground_color(Color::Yellow);
        next.write_str_with_style(1, 0, 1, "b", style);

        assert_eq!(
            changed_runs(&previous, &next),
            vec![styled_run(1, 0, "b", style)]
        );
    }

    #[test]
    fn adjacent_style_changes_split_runs_by_style() {
        let previous = buffer(&["abc"], 3);
        let mut next = TuiBuffer::new(TuiSize::new(3, 1));
        let yellow = TuiStyle::default().with_foreground_color(Color::Yellow);
        let blue = TuiStyle::default().with_foreground_color(Color::Blue);
        next.write_str_with_style(0, 0, 1, "a", yellow);
        next.write_str_with_style(1, 0, 1, "b", blue);
        next.write_str(2, 0, 1, "c");

        assert_eq!(
            changed_runs(&previous, &next),
            vec![styled_run(0, 0, "a", yellow), styled_run(1, 0, "b", blue)]
        );
    }
}
