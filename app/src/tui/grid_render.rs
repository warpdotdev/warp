//! TUI grid rendering: maps terminal `Cell`s to ratatui `TuiStyle` and paints
//! a `GridHandler` into a `TuiBuffer` at terminal-cell granularity.

use warp_terminal::model::ansi::Color;
use warp_terminal::model::grid::cell::{Cell, Flags};
use warp_terminal::model::grid::Dimensions as _;
use warpui_core::elements::tui::{Color as TuiColor, Modifier, TuiBuffer, TuiRect, TuiStyle};

use crate::terminal::color;
use crate::terminal::model::grid::grid_handler::GridHandler;

/// Converts a terminal `Color` to a ratatui `Color` using the theme's color list.
fn cell_to_color(color: &Color, colors: &color::List) -> TuiColor {
    match color {
        Color::Named(named) => {
            let c = &colors[named.into_color_index()];
            TuiColor::Rgb(c.r, c.g, c.b)
        }
        Color::Spec(c) => TuiColor::Rgb(c.r, c.g, c.b),
        Color::Indexed(idx) => {
            let c = &colors[*idx as usize];
            TuiColor::Rgb(c.r, c.g, c.b)
        }
    }
}

/// Maps a terminal `Cell`'s foreground, background, and `Flags` to a ratatui
/// `TuiStyle`.
pub fn cell_to_style(cell: &Cell, colors: &color::List) -> TuiStyle {
    let fg = cell_to_color(&cell.fg, colors);
    let bg = cell_to_color(&cell.bg, colors);
    let mut style = TuiStyle::default().fg(fg).bg(bg);

    if cell.flags.contains(Flags::BOLD) {
        style = style.add_modifier(Modifier::BOLD);
    }
    if cell.flags.contains(Flags::ITALIC) {
        style = style.add_modifier(Modifier::ITALIC);
    }
    if cell.flags.contains(Flags::UNDERLINE) || cell.flags.contains(Flags::DOUBLE_UNDERLINE) {
        style = style.add_modifier(Modifier::UNDERLINED);
    }
    if cell.flags.contains(Flags::INVERSE) {
        style = style.add_modifier(Modifier::REVERSED);
    }
    if cell.flags.contains(Flags::DIM) {
        style = style.add_modifier(Modifier::DIM);
    }
    if cell.flags.contains(Flags::HIDDEN) {
        style = style.add_modifier(Modifier::HIDDEN);
    }
    if cell.flags.contains(Flags::STRIKEOUT) {
        style = style.add_modifier(Modifier::CROSSED_OUT);
    }

    style
}

/// Returns the cell's display glyph, substituting a space for empty or
/// control-character content. ratatui panics if asked to render a control
/// character, and terminal grids can contain them (e.g. in padding cells).
pub fn sanitized_symbol(cell: &Cell) -> String {
    let raw = cell.content_for_display().to_string();
    if raw.is_empty() || raw.chars().any(char::is_control) {
        " ".to_string()
    } else {
        raw
    }
}

/// Renders a `GridHandler` into `buffer` at `area`, iterating displayed rows
/// and columns, writing each cell's glyph with its style.
pub fn render_grid(
    grid: &GridHandler,
    area: TuiRect,
    buffer: &mut TuiBuffer,
    colors: &color::List,
) {
    if area.is_empty() {
        return;
    }

    // `len_displayed()` is `Some` only when a displayed-output filter is active;
    // for the alt screen (and ordinary grids) it is `None`, so fall back to the
    // screen's visible row count.
    let num_rows = grid.len_displayed().unwrap_or_else(|| grid.visible_rows());
    let num_cols = grid.columns().min(area.width as usize);

    for row_idx in 0..num_rows {
        let y = area.y + row_idx as u16;
        if y >= area.y + area.height {
            break;
        }
        let Some(row) = grid.row(row_idx) else {
            continue;
        };
        for col_idx in 0..num_cols {
            let x = area.x + col_idx as u16;
            let cell = &row[col_idx];
            let style = cell_to_style(cell, colors);
            let symbol = sanitized_symbol(cell);
            if let Some(buffer_cell) = buffer.cell_mut((x, y)) {
                buffer_cell.set_symbol(&symbol);
                buffer_cell.set_style(style);
            }
        }
    }
}

#[cfg(test)]
#[path = "grid_render_tests.rs"]
mod tests;
