//! Terminal-grid snapshots rendered as normal TUI elements.

use std::ops::Range;

use warp::tui_export::{BlockGrid, TerminalColorList};
use warp_terminal::model::ansi::Color;
use warp_terminal::model::grid::cell::{Cell, Flags};
use warp_terminal::model::grid::Dimensions as _;
use warpui_core::elements::tui::{
    Color as TuiColor, Modifier, TuiBuffer, TuiConstraint, TuiElement, TuiLayoutContext, TuiRect,
    TuiSize, TuiStyle,
};
use warpui_core::AppContext;

#[derive(Clone)]
struct TerminalCellSnapshot {
    symbol: String,
    style: TuiStyle,
}

#[cfg(test)]
#[path = "grid_render_tests.rs"]
mod tests;

/// Renderable terminal-grid rows copied out of the terminal-model lock.
pub(super) struct TerminalGridSnapshot {
    rows: Vec<Vec<TerminalCellSnapshot>>,
}

impl TerminalGridSnapshot {
    /// Creates an empty grid snapshot.
    pub(super) fn empty() -> Self {
        Self { rows: Vec::new() }
    }

    /// Appends displayed rows from a terminal block grid.
    pub(super) fn append_displayed_rows(
        &mut self,
        block_grid: &BlockGrid,
        displayed_rows: Range<usize>,
        max_width: u16,
        colors: &TerminalColorList,
    ) {
        let grid = block_grid.grid_handler();
        let end = displayed_rows.end.min(block_grid.len_displayed());
        self.rows.extend(
            (displayed_rows.start.min(end)..end).filter_map(|displayed_row| {
                let original_row =
                    grid.maybe_translate_row_from_displayed_to_original(displayed_row);
                let row = grid.row(original_row)?;
                Some(
                    (0..grid.columns().min(usize::from(max_width)))
                        .map(|column| {
                            let cell = &row[column];
                            TerminalCellSnapshot {
                                symbol: sanitized_symbol(cell),
                                style: cell_to_style(cell, colors),
                            }
                        })
                        .collect(),
                )
            }),
        );
    }
}

impl TuiElement for TerminalGridSnapshot {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        _ctx: &mut TuiLayoutContext,
        _app: &AppContext,
    ) -> TuiSize {
        constraint.clamp(TuiSize::new(
            constraint.max.width,
            self.rows.len().min(usize::from(u16::MAX)) as u16,
        ))
    }

    fn render(&self, area: TuiRect, buffer: &mut TuiBuffer, _ctx: &mut TuiLayoutContext) {
        for (row_index, row) in self.rows.iter().take(usize::from(area.height)).enumerate() {
            let y = area.y + row_index as u16;
            for (column, cell) in row.iter().take(usize::from(area.width)).enumerate() {
                if let Some(buffer_cell) = buffer.cell_mut((area.x + column as u16, y)) {
                    buffer_cell.set_symbol(&cell.symbol).set_style(cell.style);
                }
            }
        }
    }
}

fn cell_to_color(color: &Color, colors: &TerminalColorList) -> TuiColor {
    match color {
        Color::Named(named) => {
            let color = &colors[named.into_color_index()];
            TuiColor::Rgb(color.r, color.g, color.b)
        }
        Color::Spec(color) => TuiColor::Rgb(color.r, color.g, color.b),
        Color::Indexed(index) => {
            let color = &colors[*index as usize];
            TuiColor::Rgb(color.r, color.g, color.b)
        }
    }
}

fn cell_to_style(cell: &Cell, colors: &TerminalColorList) -> TuiStyle {
    let mut style = TuiStyle::default()
        .fg(cell_to_color(&cell.fg, colors))
        .bg(cell_to_color(&cell.bg, colors));
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

fn sanitized_symbol(cell: &Cell) -> String {
    let content = cell.content_for_display().to_string();
    if content.is_empty() || content.chars().any(char::is_control) {
        " ".to_owned()
    } else {
        content
    }
}
