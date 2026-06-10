use std::io::{self, BufWriter, Write};

use crossterm::cursor;
use crossterm::execute;
use crossterm::queue;
use crossterm::style::{
    Attribute, Color as CtColor, Print, SetAttribute, SetBackgroundColor, SetForegroundColor,
};
use crossterm::terminal;

use crate::cell::{Cell, CellAttr, CellFlags, Color};

fn last_non_blank_row(grid: &[Vec<Cell>]) -> Option<usize> {
    let blank = Cell::default();
    for (i, row) in grid.iter().enumerate().rev() {
        if row.iter().any(|c| *c != blank) {
            return Some(i);
        }
    }
    None
}

pub struct StatusBar<'a> {
    pub mode: &'a str,
    pub model: &'a str,
    pub hint: &'a str,
}

pub struct Frame<'a> {
    pub completed_rows: &'a [Vec<Cell>],
    pub streaming_rows: &'a [Vec<Cell>],
    pub active_grid: &'a [Vec<Cell>],
    pub active_cursor: (usize, usize),
    pub status_bar: StatusBar<'a>,
    pub scroll_offset: usize,
    pub agent_input: Option<(&'a str, usize)>,
    pub agent_status: Option<&'a str>,
    pub total_rows: u16,
    pub total_cols: u16,
    pub show_cursor: bool,
}

pub fn enter_alt_screen() -> anyhow::Result<()> {
    let mut stdout = io::stdout();
    execute!(
        stdout,
        terminal::EnterAlternateScreen,
        cursor::Hide,
    )?;
    terminal::enable_raw_mode()?;
    Ok(())
}

pub fn leave_alt_screen() -> anyhow::Result<()> {
    let mut stdout = io::stdout();
    terminal::disable_raw_mode()?;
    execute!(
        stdout,
        cursor::Show,
        terminal::LeaveAlternateScreen,
    )?;
    Ok(())
}

struct Layout {
    scrollback_height: usize,
    active_height: usize,
    status_bar_row: u16,
}

fn compute_layout(frame: &Frame) -> Layout {
    let total = frame.total_rows as usize;
    let has_input = frame.agent_input.is_some();
    let has_status = frame.agent_status.is_some();
    let hide_active = has_input || has_status;

    let reserved = 1
        + if has_input { 1 } else { 0 }
        + if has_status { 1 } else { 0 };
    let usable = total.saturating_sub(reserved);

    let active_height = if hide_active {
        0
    } else {
        let cursor_height = frame.active_cursor.0 + 1;
        let grid_content_height = last_non_blank_row(frame.active_grid)
            .map(|r| r + 1)
            .unwrap_or(0);
        let content_height = cursor_height.max(grid_content_height).max(1);
        content_height.min(usable)
    };
    let scrollback_height = usable.saturating_sub(active_height);

    let status_bar_row = (total - 1) as u16;

    Layout {
        scrollback_height,
        active_height,
        status_bar_row,
    }
}

pub fn render(frame: &Frame) -> anyhow::Result<()> {
    let mut stdout = BufWriter::with_capacity(65536, io::stdout().lock());
    let layout = compute_layout(frame);
    let cols = frame.total_cols as usize;

    // --- Scrollback region (pinned to top) ---
    let mut next_row: u16 = 0;
    if layout.scrollback_height > 0 && !frame.completed_rows.is_empty() {
        let total_completed = frame.completed_rows.len();
        let end = total_completed.saturating_sub(frame.scroll_offset);
        let start = end.saturating_sub(layout.scrollback_height);
        let visible = &frame.completed_rows[start..end];

        for row in visible {
            queue!(stdout, cursor::MoveTo(0, next_row))?;
            render_cell_row(&mut stdout, row, cols)?;
            next_row += 1;
        }
    }

    // --- Streaming rows (ephemeral agent text, after scrollback) ---
    for streaming_row in frame.streaming_rows {
        if next_row >= layout.status_bar_row {
            break;
        }
        queue!(stdout, cursor::MoveTo(0, next_row))?;
        render_cell_row(&mut stdout, streaming_row, cols)?;
        next_row += 1;
    }

    // --- Active block region (immediately after streaming) ---
    let active_start_row = next_row;
    let grid_offset = (frame.active_cursor.0 + 1).saturating_sub(layout.active_height);
    for i in 0..layout.active_height {
        let screen_row = active_start_row + i as u16;
        queue!(stdout, cursor::MoveTo(0, screen_row))?;
        let grid_row_idx = grid_offset + i;
        if grid_row_idx < frame.active_grid.len() {
            render_cell_row(&mut stdout, &frame.active_grid[grid_row_idx], cols)?;
        } else {
            render_blank_row(&mut stdout, cols)?;
        }
        next_row += 1;
    }

    // --- Agent status (ephemeral, inline after content) ---
    if let Some(status_text) = frame.agent_status {
        queue!(stdout, cursor::MoveTo(0, next_row))?;
        render_agent_status(&mut stdout, status_text, cols)?;
        next_row += 1;
    }

    // --- Agent input line (inline after content) ---
    if let Some((buf, cursor_pos)) = frame.agent_input {
        queue!(stdout, cursor::MoveTo(0, next_row))?;
        render_agent_input(&mut stdout, buf, cursor_pos, cols)?;
        next_row += 1;
    }

    // --- Blank stale rows between content and status bar ---
    while next_row < layout.status_bar_row {
        queue!(stdout, cursor::MoveTo(0, next_row))?;
        render_blank_row(&mut stdout, cols)?;
        next_row += 1;
    }

    // --- Status bar ---
    queue!(stdout, cursor::MoveTo(0, layout.status_bar_row))?;
    render_status_bar_row(&mut stdout, &frame.status_bar, cols)?;

    // --- Cursor ---
    if frame.show_cursor {
        let (cursor_grid_row, cursor_col) = frame.active_cursor;
        let screen_cursor_row =
            active_start_row + cursor_grid_row.saturating_sub(grid_offset) as u16;
        queue!(
            stdout,
            cursor::MoveTo(cursor_col as u16, screen_cursor_row),
            cursor::Show,
        )?;
    } else {
        queue!(stdout, cursor::Hide)?;
    }

    // Reset style at the end to avoid bleeding
    queue!(
        stdout,
        SetAttribute(Attribute::Reset),
    )?;

    stdout.flush()?;
    Ok(())
}

fn cell_fg(color: Color) -> CtColor {
    match color {
        Color::Default => CtColor::Reset,
        Color::Indexed(i) => CtColor::AnsiValue(i),
        Color::Rgb(r, g, b) => CtColor::Rgb { r, g, b },
    }
}

fn cell_bg(color: Color) -> CtColor {
    match color {
        Color::Default => CtColor::Reset,
        Color::Indexed(i) => CtColor::AnsiValue(i),
        Color::Rgb(r, g, b) => CtColor::Rgb { r, g, b },
    }
}

fn render_cell_row(stdout: &mut impl Write, row: &[Cell], cols: usize) -> anyhow::Result<()> {
    let mut prev_attr: Option<CellAttr> = None;

    for (i, cell) in row.iter().enumerate().take(cols) {
        if prev_attr != Some(cell.attr) {
            apply_cell_attr(stdout, &cell.attr)?;
            prev_attr = Some(cell.attr);
        }
        if i == 0 && prev_attr.is_none() {
            apply_cell_attr(stdout, &cell.attr)?;
            prev_attr = Some(cell.attr);
        }
        queue!(stdout, Print(cell.ch))?;
    }

    // Fill remaining columns with spaces
    let rendered = row.len().min(cols);
    if rendered < cols {
        queue!(
            stdout,
            SetAttribute(Attribute::Reset),
            SetForegroundColor(CtColor::Reset),
            SetBackgroundColor(CtColor::Reset),
        )?;
        for _ in rendered..cols {
            queue!(stdout, Print(' '))?;
        }
        prev_attr = None;
    }

    if prev_attr.is_some() {
        queue!(stdout, SetAttribute(Attribute::Reset))?;
    }

    Ok(())
}

fn apply_cell_attr(stdout: &mut impl Write, attr: &CellAttr) -> anyhow::Result<()> {
    queue!(stdout, SetAttribute(Attribute::Reset))?;

    let (fg, bg) = if attr.flags.contains(CellFlags::INVERSE) {
        (attr.bg, attr.fg)
    } else {
        (attr.fg, attr.bg)
    };

    queue!(
        stdout,
        SetForegroundColor(cell_fg(fg)),
        SetBackgroundColor(cell_bg(bg)),
    )?;

    if attr.flags.contains(CellFlags::BOLD) {
        queue!(stdout, SetAttribute(Attribute::Bold))?;
    }
    if attr.flags.contains(CellFlags::DIM) {
        queue!(stdout, SetAttribute(Attribute::Dim))?;
    }
    if attr.flags.contains(CellFlags::ITALIC) {
        queue!(stdout, SetAttribute(Attribute::Italic))?;
    }
    if attr.flags.contains(CellFlags::UNDERLINE) {
        queue!(stdout, SetAttribute(Attribute::Underlined))?;
    }
    if attr.flags.contains(CellFlags::HIDDEN) {
        queue!(stdout, SetAttribute(Attribute::Hidden))?;
    }

    Ok(())
}

fn render_blank_row(stdout: &mut impl Write, cols: usize) -> anyhow::Result<()> {
    queue!(
        stdout,
        SetAttribute(Attribute::Reset),
        SetForegroundColor(CtColor::Reset),
        SetBackgroundColor(CtColor::Reset),
    )?;
    for _ in 0..cols {
        queue!(stdout, Print(' '))?;
    }
    Ok(())
}

fn render_agent_status(
    stdout: &mut impl Write,
    text: &str,
    cols: usize,
) -> anyhow::Result<()> {
    queue!(
        stdout,
        SetAttribute(Attribute::Reset),
        SetAttribute(Attribute::Dim),
        SetForegroundColor(CtColor::AnsiValue(5)), // magenta
        Print(text),
    )?;
    let used = text.chars().count();
    if used < cols {
        for _ in used..cols {
            queue!(stdout, Print(' '))?;
        }
    }
    queue!(stdout, SetAttribute(Attribute::Reset))?;
    Ok(())
}

fn render_agent_input(
    stdout: &mut impl Write,
    buf: &str,
    cursor_pos: usize,
    cols: usize,
) -> anyhow::Result<()> {
    queue!(stdout, SetAttribute(Attribute::Reset))?;

    let prefix = "🤖 > ";
    let prefix_display_width = 5; // emoji(2) + space + > + space

    queue!(stdout, SetAttribute(Attribute::Bold), Print(prefix), SetAttribute(Attribute::Reset))?;

    let before: String = buf.chars().take(cursor_pos).collect();
    queue!(stdout, Print(&before))?;

    let cursor_char = buf.chars().nth(cursor_pos).unwrap_or(' ');
    queue!(
        stdout,
        SetAttribute(Attribute::Reverse),
        Print(cursor_char),
        SetAttribute(Attribute::Reset),
    )?;

    let after: String = buf.chars().skip(cursor_pos + 1).collect();
    if !after.is_empty() {
        queue!(stdout, Print(&after))?;
    }

    // Pad to fill width
    let used = prefix_display_width + buf.chars().count().max(cursor_pos + 1);
    if used < cols {
        for _ in used..cols {
            queue!(stdout, Print(' '))?;
        }
    }

    Ok(())
}

fn format_status_bar_content(mode: &str, model: &str, hint: &str, width: usize) -> String {
    let left = format!(" {mode} │ {model}");
    let right = format!("{hint} ");

    let left_len = left.chars().count();
    let right_len = right.chars().count();
    let gap = width.saturating_sub(left_len + right_len);

    let mut bar = String::with_capacity(width);
    bar.push_str(&left);
    for _ in 0..gap {
        bar.push(' ');
    }
    bar.push_str(&right);
    bar
}

fn render_status_bar_row(
    stdout: &mut impl Write,
    sb: &StatusBar<'_>,
    cols: usize,
) -> anyhow::Result<()> {
    let content = format_status_bar_content(sb.mode, sb.model, sb.hint, cols);
    queue!(
        stdout,
        SetAttribute(Attribute::Reset),
        SetAttribute(Attribute::Reverse),
        Print(&content),
        SetAttribute(Attribute::Reset),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(ch: char) -> Cell {
        Cell::with_char(ch, CellAttr::default())
    }

    fn make_row(s: &str) -> Vec<Cell> {
        s.chars().map(|c| cell(c)).collect()
    }

    // -- Layout tests --

    #[test]
    fn layout_basic_no_agent_input() {
        let active_grid: Vec<Vec<Cell>> = vec![make_row("hello"); 5];
        let completed: Vec<Vec<Cell>> = vec![make_row("done"); 10];
        let frame = Frame {
            completed_rows: &completed,
            streaming_rows: &[],
            active_grid: &active_grid,
            active_cursor: (0, 0),
            status_bar: StatusBar {
                mode: "SHELL",
                model: "",
                hint: "",
            },
            scroll_offset: 0,
            agent_input: None, agent_status: None,
            total_rows: 24,
            total_cols: 80,
            show_cursor: true,
        };
        let layout = compute_layout(&frame);
        // cursor at (0,0) but 5 non-blank rows → content_height = 5, usable = 23
        assert_eq!(layout.active_height, 5);
        assert_eq!(layout.scrollback_height, 18);
        assert_eq!(layout.status_bar_row, 23);
    }

    #[test]
    fn layout_with_agent_input() {
        let active_grid: Vec<Vec<Cell>> = vec![make_row("x"); 3];
        let frame = Frame {
            completed_rows: &[],
            streaming_rows: &[],
            active_grid: &active_grid,
            active_cursor: (0, 0),
            status_bar: StatusBar {
                mode: "AGENT",
                model: "gpt-4",
                hint: "",
            },
            scroll_offset: 0,
            agent_input: Some(("hello", 5)), agent_status: None,
            total_rows: 24,
            total_cols: 80,
            show_cursor: false,
        };
        let layout = compute_layout(&frame);
        // active hidden when agent_input present; usable = 22, all scrollback
        assert_eq!(layout.active_height, 0);
        assert_eq!(layout.scrollback_height, 22);
        assert_eq!(layout.status_bar_row, 23);
    }

    #[test]
    fn layout_active_grid_larger_than_usable() {
        let active_grid: Vec<Vec<Cell>> = vec![make_row("y"); 100];
        let frame = Frame {
            completed_rows: &[],
            streaming_rows: &[],
            active_grid: &active_grid,
            active_cursor: (0, 0),
            status_bar: StatusBar {
                mode: "SHELL",
                model: "",
                hint: "",
            },
            scroll_offset: 0,
            agent_input: None, agent_status: None,
            total_rows: 10,
            total_cols: 80,
            show_cursor: true,
        };
        let layout = compute_layout(&frame);
        // 100 non-blank rows but usable = 9 → clamped
        assert_eq!(layout.active_height, 9);
        assert_eq!(layout.scrollback_height, 0);
    }

    #[test]
    fn layout_empty_everything() {
        let frame = Frame {
            completed_rows: &[],
            streaming_rows: &[],
            active_grid: &[],
            active_cursor: (0, 0),
            status_bar: StatusBar {
                mode: "",
                model: "",
                hint: "",
            },
            scroll_offset: 0,
            agent_input: None, agent_status: None,
            total_rows: 24,
            total_cols: 80,
            show_cursor: false,
        };
        let layout = compute_layout(&frame);
        // cursor at (0,0) → content_height = 1, usable = 23
        assert_eq!(layout.active_height, 1);
        assert_eq!(layout.scrollback_height, 22);
    }

    #[test]
    fn layout_tiny_terminal() {
        let active_grid: Vec<Vec<Cell>> = vec![make_row("a"); 2];
        let frame = Frame {
            completed_rows: &[],
            streaming_rows: &[],
            active_grid: &active_grid,
            active_cursor: (0, 0),
            status_bar: StatusBar {
                mode: "S",
                model: "",
                hint: "",
            },
            scroll_offset: 0,
            agent_input: Some(("", 0)), agent_status: None,
            total_rows: 3,
            total_cols: 40,
            show_cursor: false,
        };
        let layout = compute_layout(&frame);
        // usable = 3 - 1(status) - 1(input) = 1, active hidden
        assert_eq!(layout.active_height, 0);
        assert_eq!(layout.scrollback_height, 1);
    }

    #[test]
    fn layout_with_agent_status() {
        let frame = Frame {
            completed_rows: &[],
            streaming_rows: &[],
            active_grid: &[],
            active_cursor: (0, 0),
            status_bar: StatusBar {
                mode: "AGENT",
                model: "warp",
                hint: "",
            },
            scroll_offset: 0,
            agent_input: None,
            agent_status: Some("⠋ working..."),
            total_rows: 24,
            total_cols: 80,
            show_cursor: false,
        };
        let layout = compute_layout(&frame);
        // active hidden; usable = 22, all scrollback
        assert_eq!(layout.active_height, 0);
        assert_eq!(layout.scrollback_height, 22);
        assert_eq!(layout.status_bar_row, 23);
    }

    #[test]
    fn layout_content_below_cursor_shown() {
        // Simulates zsh completions: cursor on row 0, content on rows 1-3
        let mut active_grid: Vec<Vec<Cell>> = vec![vec![Cell::default(); 80]; 23];
        active_grid[0] = make_row("$ cd ");
        active_grid[1] = make_row("dir1/");
        active_grid[2] = make_row("dir2/");
        active_grid[3] = make_row("dir3/");
        let frame = Frame {
            completed_rows: &[],
            streaming_rows: &[],
            active_grid: &active_grid,
            active_cursor: (0, 5),
            status_bar: StatusBar {
                mode: "SHELL",
                model: "",
                hint: "",
            },
            scroll_offset: 0,
            agent_input: None, agent_status: None,
            total_rows: 24,
            total_cols: 80,
            show_cursor: true,
        };
        let layout = compute_layout(&frame);
        // cursor at row 0, but content extends to row 3 → active_height = 4
        assert_eq!(layout.active_height, 4);
        assert_eq!(layout.scrollback_height, 19);
    }

    #[test]
    fn layout_blank_grid_stays_compact() {
        // All-blank grid with cursor at row 0 → stays compact at 1 row
        let active_grid: Vec<Vec<Cell>> = vec![vec![Cell::default(); 80]; 23];
        let frame = Frame {
            completed_rows: &[],
            streaming_rows: &[],
            active_grid: &active_grid,
            active_cursor: (0, 0),
            status_bar: StatusBar {
                mode: "SHELL",
                model: "",
                hint: "",
            },
            scroll_offset: 0,
            agent_input: None, agent_status: None,
            total_rows: 24,
            total_cols: 80,
            show_cursor: true,
        };
        let layout = compute_layout(&frame);
        assert_eq!(layout.active_height, 1);
        assert_eq!(layout.scrollback_height, 22);
    }

    // -- Color conversion tests --

    #[test]
    fn color_default_maps_to_reset() {
        assert_eq!(cell_fg(Color::Default), CtColor::Reset);
        assert_eq!(cell_bg(Color::Default), CtColor::Reset);
    }

    #[test]
    fn color_indexed_maps_to_ansi_value() {
        assert_eq!(cell_fg(Color::Indexed(196)), CtColor::AnsiValue(196));
    }

    #[test]
    fn color_rgb_maps_correctly() {
        assert_eq!(
            cell_fg(Color::Rgb(10, 20, 30)),
            CtColor::Rgb {
                r: 10,
                g: 20,
                b: 30
            }
        );
    }

    // -- Status bar formatting tests --

    #[test]
    fn status_bar_format_basic() {
        let content = format_status_bar_content("AGENT", "gpt-4", "Ctrl+C cancel", 60);
        assert_eq!(content.chars().count(), 60);
        assert!(content.starts_with(" AGENT │ gpt-4"));
        assert!(content.ends_with("Ctrl+C cancel "));
    }

    #[test]
    fn status_bar_format_narrow() {
        let content = format_status_bar_content("S", "m", "h", 10);
        // left = " S │ m" (6), right = "h " (2), gap = 2
        assert_eq!(content.chars().count(), 10);
    }

    #[test]
    fn status_bar_format_overflow() {
        let content = format_status_bar_content("AGENT", "some-long-model-name", "hint", 10);
        // When content overflows, gap saturates to 0, total exceeds width
        // (no truncation in prototype)
        assert!(content.contains("AGENT"));
        assert!(content.contains("some-long-model-name"));
    }

    #[test]
    fn status_bar_format_wide() {
        let content = format_status_bar_content("SHELL", "claude", "q: quit", 120);
        assert_eq!(content.chars().count(), 120);
    }
}
