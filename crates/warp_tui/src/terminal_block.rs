//! Simple visible-row terminal block rendering for the TUI transcript.

use std::sync::Arc;

use super::grid_render::TerminalGridSnapshot;
use parking_lot::FairMutex;
use warp::tui_export::{BlockId, TerminalModel};
use warpui_core::elements::tui::TuiElement;

/// Snapshots a terminal block's visible grids for viewport rendering.
pub(super) fn render_terminal_block(
    model: &Arc<FairMutex<TerminalModel>>,
    block_id: &BlockId,
    width: u16,
) -> Box<dyn TuiElement> {
    let snapshot = {
        let model = model.lock();
        let colors = model.colors();
        let Some(block) = model.block_list().block_with_id(block_id) else {
            return Box::new(TerminalGridSnapshot::empty());
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
        let mut snapshot = TerminalGridSnapshot::empty();
        if prompt_rows > 0 {
            snapshot.append_displayed_rows(
                block.prompt_and_command_grid(),
                0..prompt_rows,
                width,
                &colors,
            );
        }
        if output_rows > 0 {
            snapshot.append_displayed_rows(block.output_grid(), 0..output_rows, width, &colors);
        }
        snapshot
    };
    Box::new(snapshot)
}
