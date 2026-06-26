//! Simple visible-row terminal block rendering for the TUI transcript.

use std::ops::Range;
use std::sync::Arc;

use parking_lot::FairMutex;
use warp::tui_export::{BlockId, TerminalModel};
use warpui_core::elements::tui::RenderedViewportItem;

use super::grid_render::{snapshot_block_grid_rows, TuiGridRows};

/// Renders the requested logical rows of a terminal block.
pub(super) fn render_terminal_block(
    model: &Arc<FairMutex<TerminalModel>>,
    block_id: &BlockId,
    visible_rows: Range<usize>,
    width: u16,
) -> RenderedViewportItem {
    let rows = {
        let model = model.lock();
        let colors = model.colors();
        let Some(block) = model.block_list().block_with_id(block_id) else {
            return RenderedViewportItem {
                element: Box::new(TuiGridRows::new(Vec::new())),
                measured_full_height: None,
            };
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
        let mut rows = Vec::new();
        if visible_rows.start < prompt_rows {
            rows.extend(snapshot_block_grid_rows(
                block.prompt_and_command_grid(),
                visible_rows.start..visible_rows.end.min(prompt_rows),
                width,
                &colors,
            ));
        }
        let output_start = visible_rows.start.saturating_sub(prompt_rows);
        let output_end = visible_rows
            .end
            .saturating_sub(prompt_rows)
            .min(output_rows);
        if output_start < output_end {
            rows.extend(snapshot_block_grid_rows(
                block.output_grid(),
                output_start..output_end,
                width,
                &colors,
            ));
        }
        rows
    };

    RenderedViewportItem {
        element: Box::new(TuiGridRows::new(rows)),
        measured_full_height: None,
    }
}
