//! Simple visible-row terminal block rendering for the TUI transcript.

use std::ops::Range;
use std::sync::Arc;

use parking_lot::FairMutex;
use warp::tui_export::{BlockId, TerminalModel};
use warpui_core::elements::tui::TuiElement;

use super::grid_render::TerminalGridSnapshot;

/// Snapshots the requested logical rows from a terminal block's visible grids.
pub(super) fn render_terminal_block_rows(
    model: &Arc<FairMutex<TerminalModel>>,
    block_id: &BlockId,
    rows: Range<usize>,
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
        let total_rows = prompt_rows.saturating_add(output_rows);
        let rows = rows.start.min(total_rows)..rows.end.min(total_rows);
        let mut snapshot = TerminalGridSnapshot::empty();
        if rows.start < prompt_rows {
            snapshot.append_displayed_rows(
                block.prompt_and_command_grid(),
                rows.start..rows.end.min(prompt_rows),
                width,
                &colors,
            );
        }
        if rows.end > prompt_rows {
            snapshot.append_displayed_rows(
                block.output_grid(),
                rows.start.saturating_sub(prompt_rows)..rows.end.saturating_sub(prompt_rows),
                width,
                &colors,
            );
        }
        snapshot
    };
    Box::new(snapshot)
}
