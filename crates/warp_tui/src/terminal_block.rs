//! Simple terminal block rendering for the TUI transcript.

use std::ops::Range;
use std::sync::Arc;

use parking_lot::FairMutex;
use warp::tui_export::{
    Block, BlockGrid, BlockId, BlockList, GridHandler, TermMode, TerminalColorList, TerminalModel,
};
use warp_terminal::model::ansi::{Color, NamedColor};
use warp_terminal::model::grid::Dimensions as _;
use warp_terminal::model::grid::cell::{Cell, Flags};
use warpui_core::AppContext;
use warpui_core::elements::tui::{
    Color as TuiColor, Modifier, TuiConstraint, TuiElement, TuiLayoutContext, TuiPaintContext,
    TuiPaintSurface, TuiScreenPoint, TuiScreenPosition, TuiSize, TuiStyle,
};

use crate::terminal_use::user_controls_running_command;
use crate::tui_builder::TuiUiBuilder;
const SHELL_COMMAND_PREFIX: &str = "!";
const SHELL_COMMAND_PREFIX_WIDTH: u16 = 2;

/// Selects which rows of a terminal block an element paints.
enum TerminalBlockRows {
    /// A viewport-preclipped transcript window with its source width.
    Visible { rows: Range<usize>, width: u16 },
    /// Every currently displayed command/output row, derived live.
    Content,
}

/// Absolute bounds used while painting one terminal block.
#[derive(Clone, Copy)]
struct TerminalBlockPaintBounds {
    origin: TuiScreenPosition,
    size: TuiSize,
    background: Option<TuiColor>,
    content_offset: u16,
    prefix_style: Option<TuiStyle>,
}

#[derive(Clone, Copy)]
struct TerminalCommandStyle {
    background: TuiColor,
    prefix: TuiStyle,
}

/// Paints terminal cells from one block using either a pre-clipped transcript
/// window or the block's complete displayed command/output content.
///
/// This is a bespoke [`TuiElement`], unlike agent blocks which compose generic
/// `TuiText`/`TuiContainer`: terminal cells each carry their own fg/bg/flags,
/// which no generic single-style text element can express, and a block can be
/// thousands of rows — painting only the visible slice into the buffer avoids
/// materializing a huge element tree per frame. Inline shell-command bodies
/// use the same element and cell renderer, but derive their full content range
/// live so growing output is reflected without rebuilding the agent block.
pub(super) struct TerminalBlockElement {
    model: Arc<FairMutex<TerminalModel>>,
    block_id: BlockId,
    rows: TerminalBlockRows,
    size: Option<TuiSize>,
    origin: Option<TuiScreenPoint>,
    command_style: Option<TerminalCommandStyle>,
}

impl TerminalBlockElement {
    /// Creates an element for a viewport-preclipped terminal block window.
    pub(super) fn visible_rows(
        model: Arc<FairMutex<TerminalModel>>,
        block_id: BlockId,
        visible_rows: Range<usize>,
        width: u16,
    ) -> Self {
        Self {
            model,
            block_id,
            rows: TerminalBlockRows::Visible {
                rows: visible_rows,
                width,
            },
            size: None,
            origin: None,
            command_style: None,
        }
    }
    /// Creates an element for all currently displayed command/output rows.
    pub(super) fn content(model: Arc<FairMutex<TerminalModel>>, block_id: BlockId) -> Self {
        Self {
            model,
            block_id,
            rows: TerminalBlockRows::Content,
            size: None,
            origin: None,
            command_style: None,
        }
    }
}

fn terminal_block_cursor(
    block: &Block,
    visible_rows: &Range<usize>,
    size: TuiSize,
) -> Option<(u16, u16)> {
    if !user_controls_running_command(block) || !block.is_mode_set(TermMode::SHOW_CURSOR) {
        return None;
    }
    let (grid, grid_start_row) = if block.is_command_grid_active() {
        if block.should_hide_command_grid() {
            return None;
        }
        (
            block.prompt_and_command_grid(),
            block
                .prompt_and_command_grid_offset()
                .as_f64()
                .ceil()
                .max(0.0) as usize,
        )
    } else {
        if block.should_hide_output_grid() {
            return None;
        }
        (
            block.output_grid(),
            block.output_grid_offset().as_f64().ceil().max(0.0) as usize,
        )
    };
    let (column, grid_row) = grid.visible_cursor_display_position()?;
    let block_row = grid_start_row.saturating_add(grid_row);
    if !visible_rows.contains(&block_row) {
        return None;
    }
    let column = u16::try_from(column).ok()?;
    let row = u16::try_from(block_row.saturating_sub(visible_rows.start)).ok()?;
    (column < size.width && row < size.height).then_some((column, row))
}

impl TuiElement for TerminalBlockElement {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        _ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> TuiSize {
        let rows = match &self.rows {
            TerminalBlockRows::Visible { rows, .. } => {
                let builder = TuiUiBuilder::from_app(app);
                self.command_style = Some(TerminalCommandStyle {
                    background: builder.shell_command_background(),
                    prefix: builder.shell_command_prefix_style(),
                });
                rows.clone()
            }
            TerminalBlockRows::Content => {
                self.command_style = None;
                let model = self.model.lock();
                model
                    .block_list()
                    .block_with_id(&self.block_id)
                    .map(block_content_rows)
                    .unwrap_or_default()
            }
        };
        let size = constraint.clamp(TuiSize::new(
            constraint.max.width,
            rows.end
                .saturating_sub(rows.start)
                .min(usize::from(u16::MAX)) as u16,
        ));
        self.size = Some(size);
        size
    }

    fn render(
        &mut self,
        origin: TuiScreenPosition,
        surface: &mut TuiPaintSurface<'_>,
        ctx: &mut TuiPaintContext,
    ) {
        self.origin = Some(ctx.scene_point(origin));
        let Some(size) = self.size else {
            return;
        };
        let model = self.model.lock();
        let colors = model.colors();
        let Some(block) = model.block_list().block_with_id(&self.block_id) else {
            return;
        };
        let (rows, width) = match &self.rows {
            TerminalBlockRows::Visible { rows, width } => (rows.clone(), (*width).min(size.width)),
            TerminalBlockRows::Content => (block_content_rows(block), size.width),
        };
        let cursor = terminal_block_cursor(block, &rows, size).and_then(|(column, row)| {
            let column = if self.command_style.is_some() && block.is_command_grid_active() {
                column.saturating_add(SHELL_COMMAND_PREFIX_WIDTH)
            } else {
                column
            };
            (column < size.width).then_some((column, row))
        });
        render_block_rows(
            block,
            rows,
            width,
            TerminalBlockPaintBounds {
                origin,
                size,
                background: None,
                content_offset: 0,
                prefix_style: None,
            },
            surface,
            &colors,
            self.command_style,
        );
        drop(model);
        if let Some((col, row)) = cursor {
            ctx.set_terminal_cursor(ctx.scene_point(origin.offset(i32::from(col), i32::from(row))));
        }
    }

    fn size(&self) -> Option<TuiSize> {
        self.size
    }

    fn origin(&self) -> Option<TuiScreenPoint> {
        self.origin
    }
}

/// Returns the smallest block-relative row range containing every displayed
/// command/output cell. Block-list padding outside the grids is intentionally
/// excluded because an inline command body already gets its spacing from the
/// surrounding tool-call section.
pub(super) fn block_content_rows(block: &Block) -> Range<usize> {
    let mut start = usize::MAX;
    let mut end = 0;
    let mut include_grid = |hidden: bool, offset: f64, displayed_rows: usize| {
        if hidden || displayed_rows == 0 {
            return;
        }
        let grid_start = offset.ceil().max(0.0) as usize;
        let grid_end = grid_start.saturating_add(displayed_rows);
        start = start.min(grid_start);
        end = end.max(grid_end);
    };
    include_grid(
        block.should_hide_command_grid() || block.prompt_and_command_height().as_f64() <= 0.0,
        block.prompt_and_command_grid_offset().as_f64(),
        block.prompt_and_command_grid().len_displayed(),
    );
    include_grid(
        block.should_hide_output_grid() || block.output_grid_displayed_height().as_f64() <= 0.0,
        block.output_grid_offset().as_f64(),
        block.output_grid().len_displayed(),
    );
    if start == usize::MAX {
        0..0
    } else {
        start..end
    }
}

/// Paints the requested block-relative rows from a terminal block. A block
/// stacks its prompt/command grid above its output grid; each call paints only
/// rows overlapping `visible_rows`, positioned within `size` so the two
/// grids don't overlap.
fn render_block_rows(
    block: &Block,
    visible_rows: Range<usize>,
    max_width: u16,
    bounds: TerminalBlockPaintBounds,
    surface: &mut TuiPaintSurface<'_>,
    colors: &TerminalColorList,
    command_style: Option<TerminalCommandStyle>,
) {
    if !block.should_hide_command_grid() {
        let command_bounds = match command_style {
            Some(style) => TerminalBlockPaintBounds {
                background: Some(style.background),
                content_offset: SHELL_COMMAND_PREFIX_WIDTH,
                prefix_style: Some(style.prefix),
                ..bounds
            },
            None => bounds,
        };
        render_grid_rows(
            block.prompt_and_command_grid(),
            block
                .prompt_and_command_grid_offset()
                .as_f64()
                .ceil()
                .max(0.0) as usize,
            visible_rows.clone(),
            max_width,
            command_bounds,
            surface,
            colors,
        );
    }

    if !block.should_hide_output_grid() {
        render_grid_rows(
            block.output_grid(),
            block.output_grid_offset().as_f64().ceil().max(0.0) as usize,
            visible_rows,
            max_width,
            TerminalBlockPaintBounds {
                background: None,
                content_offset: 0,
                prefix_style: None,
                ..bounds
            },
            surface,
            colors,
        );
    }
}

/// Paints the visible rows of a raw [`GridHandler`] (e.g. the alt screen,
/// which has no scrollback) at `origin`, reusing the same per-cell styling as
/// the block renderer. Unlike a block grid, the alt screen is a plain viewport,
/// so rows map directly to screen rows (offset past any history defensively).
pub(super) fn render_grid_handler(
    grid: &GridHandler,
    origin: TuiScreenPosition,
    size: TuiSize,
    surface: &mut TuiPaintSurface<'_>,
    colors: &TerminalColorList,
) {
    let history = grid.history_size();
    let rows = grid.visible_rows().min(usize::from(size.height));
    let cols = grid.columns().min(usize::from(size.width));
    for screen_row in 0..rows {
        render_grid_row(
            grid,
            history + screen_row,
            cols,
            origin.offset(0, screen_row as i32),
            surface,
            colors,
            None,
        );
    }
}

/// Returns whether the TUI transcript should include this terminal block.
pub(super) fn should_render_terminal_block(block: &Block, block_list: &BlockList) -> bool {
    // Agent-requested command blocks are rendered inline inside their agent
    // block's shell-command view (see `TuiShellCommandView`), so they must not
    // also appear as a standalone terminal block in the transcript. Their
    // interaction mode normally hides them, but once a long-running agent
    // command becomes agent-monitored that hide flag flips off
    // (`InteractionMode::to_agent_monitored`), which would otherwise surface the
    // block a second time.
    !block.is_agent_requested_command()
        && block.is_visible(block_list.transcript_scope())
        && (block.started() || block.finished())
}

/// Paints consecutive displayed rows of one grid starting at `*y`, advancing
/// `y` past each row drawn and stopping at the bottom of `size`.
fn render_displayed_rows(
    block_grid: &BlockGrid,
    displayed_rows: Range<usize>,
    max_width: u16,
    bounds: TerminalBlockPaintBounds,
    surface: &mut TuiPaintSurface<'_>,
    colors: &TerminalColorList,
    y: &mut u16,
) {
    let grid = block_grid.grid_handler();
    let end = displayed_rows.end.min(block_grid.len_displayed());
    for displayed_row in displayed_rows.start.min(end)..end {
        if *y >= bounds.size.height {
            break;
        }
        let row_origin = bounds.origin.offset(0, i32::from(*y));
        if let Some(background) = bounds.background {
            for column in 0..max_width.min(bounds.size.width) {
                if let Some(cell) = surface.cell_mut(row_origin.offset(i32::from(column), 0)) {
                    cell.set_style(TuiStyle::default().bg(background));
                }
            }
        }
        if displayed_row == 0
            && let Some(prefix_style) = bounds.prefix_style
            && let Some(cell) = surface.cell_mut(row_origin)
        {
            cell.set_symbol(SHELL_COMMAND_PREFIX)
                .set_style(prefix_style);
        }
        let original_row = grid.maybe_translate_row_from_displayed_to_original(displayed_row);
        let content_width = max_width.saturating_sub(bounds.content_offset);
        render_grid_row(
            grid,
            original_row,
            grid.columns().min(usize::from(content_width)),
            row_origin.offset(i32::from(bounds.content_offset), 0),
            surface,
            colors,
            bounds.background,
        );
        *y = (*y).saturating_add(1);
    }
}

/// Paints one grid row with terminal cell styling.
fn render_grid_row(
    grid: &GridHandler,
    row: usize,
    columns: usize,
    origin: TuiScreenPosition,
    surface: &mut TuiPaintSurface<'_>,
    colors: &TerminalColorList,
    background: Option<TuiColor>,
) {
    let Some(row) = grid.row(row) else {
        return;
    };
    for column in 0..columns {
        let cell = &row[column];
        if let Some(buffer_cell) =
            surface.cell_mut(origin.offset(i32::try_from(column).unwrap_or(i32::MAX), 0))
        {
            let mut style = cell_to_style(cell, colors);
            if let Some(background) = background {
                style = style.bg(background);
            }
            buffer_cell
                .set_symbol(&sanitized_symbol(cell))
                .set_style(style);
        }
    }
}

/// Paints the rows of one grid that fall within the element's visible window.
///
/// `grid_start_row` is where this grid begins relative to the top of the block
/// (the command grid starts at 0; the output grid starts below it). Only the
/// intersection of the grid's rows with `visible_rows` is drawn, offset within
/// `size` so it lands at the correct vertical position.
fn render_grid_rows(
    block_grid: &BlockGrid,
    grid_start_row: usize,
    visible_rows: Range<usize>,
    max_width: u16,
    bounds: TerminalBlockPaintBounds,
    surface: &mut TuiPaintSurface<'_>,
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
    let mut y = y_offset.min(usize::from(u16::MAX)) as u16;
    render_displayed_rows(
        block_grid,
        displayed_rows,
        max_width,
        bounds,
        surface,
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

#[cfg(test)]
#[path = "terminal_block_tests.rs"]
mod tests;
