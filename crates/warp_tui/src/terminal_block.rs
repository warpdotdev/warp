//! Simple visible-row terminal block rendering for the TUI transcript.

use std::ops::Range;
use std::sync::Arc;

use parking_lot::FairMutex;
use warp::tui_export::{BlockGrid, BlockId, TerminalColorList, TerminalModel};
use warp_terminal::model::ansi::Color;
use warp_terminal::model::grid::cell::{Cell, Flags};
use warp_terminal::model::grid::Dimensions as _;
use warpui_core::elements::tui::{
    Color as TuiColor, Modifier, TuiBuffer, TuiConstraint, TuiElement, TuiLayoutContext, TuiRect,
    TuiSize, TuiStyle,
};
use warpui_core::AppContext;

/// TUI element for a terminal block's requested logical rows.
pub(super) struct TerminalBlockRowsElement {
    model: Arc<FairMutex<TerminalModel>>,
    block_id: BlockId,
    rows: Range<usize>,
    width: u16,
}
impl TerminalBlockRowsElement {
    /// Creates a terminal block rows element.
    pub(super) fn new(
        model: Arc<FairMutex<TerminalModel>>,
        block_id: BlockId,
        rows: Range<usize>,
        width: u16,
    ) -> Self {
        Self {
            model,
            block_id,
            rows,
            width,
        }
    }
}

impl TuiElement for TerminalBlockRowsElement {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        _ctx: &mut TuiLayoutContext,
        _app: &AppContext,
    ) -> TuiSize {
        constraint.clamp(TuiSize::new(
            constraint.max.width,
            self.rows
                .end
                .saturating_sub(self.rows.start)
                .min(usize::from(u16::MAX)) as u16,
        ))
    }

    fn render(&self, area: TuiRect, buffer: &mut TuiBuffer, _ctx: &mut TuiLayoutContext) {
        let model = self.model.lock();
        let colors = model.colors();
        let Some(block) = model.block_list().block_with_id(&self.block_id) else {
            return;
        };
        let prompt_rows = if block.should_hide_command_grid() {
            0
        } else {
            block.prompt_and_command_grid().len_displayed()
        };
        let output_rows = if block.should_hide_output_grid() {
            0
        } else {
            block.output_grid().len_displayed()
        };
        let total_rows = prompt_rows.saturating_add(output_rows);
        let rows = self.rows.start.min(total_rows)..self.rows.end.min(total_rows);
        let max_width = self.width.min(area.width);
        let mut y = area.y;
        if rows.start < prompt_rows {
            render_displayed_rows(
                block.prompt_and_command_grid(),
                rows.start..rows.end.min(prompt_rows),
                max_width,
                area,
                buffer,
                &colors,
                &mut y,
            );
        }
        if rows.end > prompt_rows {
            render_displayed_rows(
                block.output_grid(),
                rows.start.saturating_sub(prompt_rows)..rows.end.saturating_sub(prompt_rows),
                max_width,
                area,
                buffer,
                &colors,
                &mut y,
            );
        }
    }
}

fn render_displayed_rows(
    block_grid: &BlockGrid,
    displayed_rows: Range<usize>,
    max_width: u16,
    area: TuiRect,
    buffer: &mut TuiBuffer,
    colors: &TerminalColorList,
    y: &mut u16,
) {
    let grid = block_grid.grid_handler();
    let end = displayed_rows.end.min(block_grid.len_displayed());
    for displayed_row in displayed_rows.start.min(end)..end {
        if *y >= area.bottom() {
            break;
        }
        let original_row = grid.maybe_translate_row_from_displayed_to_original(displayed_row);
        let Some(row) = grid.row(original_row) else {
            continue;
        };
        for column in 0..grid.columns().min(usize::from(max_width)) {
            let cell = &row[column];
            if let Some(buffer_cell) = buffer.cell_mut((area.x.saturating_add(column as u16), *y))
            {
                buffer_cell
                    .set_symbol(&sanitized_symbol(cell))
                    .set_style(cell_to_style(cell, colors));
            }
        }
        *y = (*y).saturating_add(1);
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
