//! Simple visible-row terminal block rendering for the TUI transcript.

use std::sync::Arc;

use parking_lot::FairMutex;
use warp::tui_export::{BlockId, TerminalModel};
use warpui_core::elements::tui::TuiElement;

use super::grid_render::{snapshot_block_grid_rows, TuiGridRows};

/// Renders a full terminal block for viewport clipping.
pub(super) fn render_terminal_block(
    model: &Arc<FairMutex<TerminalModel>>,
    block_id: &BlockId,
    width: u16,
) -> Box<dyn TuiElement> {
    let rows = {
        let model = model.lock();
        let colors = model.colors();
        let Some(block) = model.block_list().block_with_id(block_id) else {
            return Box::new(TuiGridRows::new(Vec::new()));
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
        if prompt_rows > 0 {
            rows.extend(snapshot_block_grid_rows(
                block.prompt_and_command_grid(),
                0..prompt_rows,
                width,
                &colors,
            ));
        }
        if output_rows > 0 {
            rows.extend(snapshot_block_grid_rows(
                block.output_grid(),
                0..output_rows,
                width,
                &colors,
            ));
        }
        rows
    };
    Box::new(TuiGridRows::new(rows))
}
