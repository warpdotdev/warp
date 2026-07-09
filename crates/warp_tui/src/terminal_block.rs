//! Simple terminal block rendering for the TUI transcript.

use std::ops::Range;
use std::sync::Arc;

use parking_lot::FairMutex;
use warp::tui_export::{Block, BlockGrid, BlockId, BlockList, TerminalColorList, TerminalModel};
use warp_terminal::model::ansi::{Color, NamedColor};
use warp_terminal::model::grid::cell::{Cell, Flags};
use warp_terminal::model::grid::Dimensions as _;
use warpui_core::elements::tui::{
    Color as TuiColor, Modifier, TuiBuffer, TuiConstraint, TuiElement, TuiLayoutContext,
    TuiPaintContext, TuiRect, TuiSize, TuiStyle,
};
use warpui_core::AppContext;

/// Paints a pre-clipped row window from one terminal block.
///
/// This is a bespoke [`TuiElement`], unlike agent blocks which compose generic
/// `TuiText`/`TuiContainer`: terminal cells each carry their own fg/bg/flags,
/// which no generic single-style text element can express, and a block can be
/// thousands of rows — painting only the visible slice into the buffer avoids
/// materializing a huge element tree per frame.
pub(super) struct TerminalBlockVisibleRowsElement {
    model: Arc<FairMutex<TerminalModel>>,
    block_id: BlockId,
    visible_rows: Range<usize>,
    width: u16,
}

impl TerminalBlockVisibleRowsElement {
    /// Creates a terminal block element for a visible row window.
    pub(super) fn new(
        model: Arc<FairMutex<TerminalModel>>,
        block_id: BlockId,
        visible_rows: Range<usize>,
        width: u16,
    ) -> Self {
        Self {
            model,
            block_id,
            visible_rows,
            width,
        }
    }
}

impl TuiElement for TerminalBlockVisibleRowsElement {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        _ctx: &mut TuiLayoutContext,
        _app: &AppContext,
    ) -> TuiSize {
        constraint.clamp(TuiSize::new(
            constraint.max.width,
            self.visible_rows
                .end
                .saturating_sub(self.visible_rows.start)
                .min(usize::from(u16::MAX)) as u16,
        ))
    }

    fn render(&self, area: TuiRect, buffer: &mut TuiBuffer, _ctx: &mut TuiPaintContext) {
        let model = self.model.lock();
        let colors = model.colors();
        let Some(block) = model.block_list().block_with_id(&self.block_id) else {
            return;
        };

        // A block stacks its prompt/command grid above its output grid; each call
        // paints only that grid's rows overlapping this element's visible window,
        // positioned within `area`, so the two grids don't overlap.
        let max_width = self.width.min(area.width);
        if !block.should_hide_command_grid() {
            render_grid_rows(
                block.prompt_and_command_grid(),
                block
                    .prompt_and_command_grid_offset()
                    .as_f64()
                    .ceil()
                    .max(0.0) as usize,
                self.visible_rows.clone(),
                max_width,
                area,
                buffer,
                &colors,
            );
        }

        if !block.should_hide_output_grid() {
            render_grid_rows(
                block.output_grid(),
                block.output_grid_offset().as_f64().ceil().max(0.0) as usize,
                self.visible_rows.clone(),
                max_width,
                area,
                buffer,
                &colors,
            );
        }
    }
}

/// Returns whether the TUI transcript should include this terminal block.
pub(super) fn should_render_terminal_block(block: &Block, block_list: &BlockList) -> bool {
    block.is_visible(block_list.agent_view_state()) && (block.started() || block.finished())
}

/// Paints consecutive displayed rows of one grid starting at `*y`, advancing
/// `y` past each row drawn and stopping at the bottom of `area`.
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
            if let Some(buffer_cell) = buffer.cell_mut((area.x.saturating_add(column as u16), *y)) {
                buffer_cell
                    .set_symbol(&sanitized_symbol(cell))
                    .set_style(cell_to_style(cell, colors));
            }
        }
        *y = (*y).saturating_add(1);
    }
}

/// Paints the rows of one grid that fall within the element's visible window.
///
/// `grid_start_row` is where this grid begins relative to the top of the block
/// (the command grid starts at 0; the output grid starts below it). Only the
/// intersection of the grid's rows with `visible_rows` is drawn, offset within
/// `area` so it lands at the correct vertical position.
fn render_grid_rows(
    block_grid: &BlockGrid,
    grid_start_row: usize,
    visible_rows: Range<usize>,
    max_width: u16,
    area: TuiRect,
    buffer: &mut TuiBuffer,
    colors: &TerminalColorList,
) {
    let grid_end_row = grid_start_row.saturating_add(block_grid.len_displayed());
    let visible_start = visible_rows.start.max(grid_start_row);
    let visible_end = visible_rows.end.min(grid_end_row);
    if visible_start >= visible_end {
        return;
    }

    let displayed_rows =
        visible_start.saturating_sub(grid_start_row)..visible_end.saturating_sub(grid_start_row);
    let y_offset = visible_start.saturating_sub(visible_rows.start);
    let mut y = area
        .y
        .saturating_add(y_offset.min(usize::from(u16::MAX)) as u16);
    render_displayed_rows(
        block_grid,
        displayed_rows,
        max_width,
        area,
        buffer,
        colors,
        &mut y,
    );
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
    let mut style = TuiStyle::default().fg(cell_to_color(&cell.fg, colors));
    // Cells with the default background are left bg-unset so they inherit the
    // TUI's own background instead of painting the theme's background color;
    // explicitly-set backgrounds still paint.
    if cell.bg != Color::Named(NamedColor::Background) {
        style = style.bg(cell_to_color(&cell.bg, colors));
    }

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
