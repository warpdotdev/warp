//! Full-screen alt-screen rendering for the TUI.
//!
//! When a PTY app switches to the alternate screen (vim, htop, less, …), the
//! terminal model flips [`TerminalModel::is_alt_screen_active`] and populates a
//! dedicated alt-screen grid. [`TuiTerminalSessionView`] then renders this
//! element full-area instead of the block/transcript UI — mirroring the GUI's
//! `AltScreenElement` (`app/src/terminal/alt_screen/alt_screen_element.rs`).
//!
//! Covers rendering and the cursor. PTY sizing and keyboard forwarding are
//! handled by the session view's `TuiTerminalContentElement` wrapper. Mouse
//! forwarding remains a follow-up.
//!
//! [`TuiTerminalSessionView`]: crate::terminal_session_view::TuiTerminalSessionView
//! [`TerminalModel::is_alt_screen_active`]: warp::tui_export::TerminalModel

use std::sync::Arc;

use parking_lot::FairMutex;
use warp::tui_export::{TermMode, TerminalModel};
use warp_terminal::model::grid::Dimensions as _;
use warpui_core::elements::tui::{
    TuiConstraint, TuiElement, TuiLayoutContext, TuiPaintContext, TuiPaintSurface, TuiScreenPoint,
    TuiScreenPosition, TuiSize,
};
use warpui_core::AppContext;

use crate::terminal_block::render_grid_handler;

/// Renders the terminal's alt-screen grid full-area while a full-screen app is
/// active.
pub(crate) struct AltScreenElement {
    model: Arc<FairMutex<TerminalModel>>,
    size: Option<TuiSize>,
    origin: Option<TuiScreenPoint>,
}

impl AltScreenElement {
    pub(crate) fn new(model: Arc<FairMutex<TerminalModel>>) -> Self {
        Self {
            model,
            size: None,
            origin: None,
        }
    }
}

impl TuiElement for AltScreenElement {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        _ctx: &mut TuiLayoutContext,
        _app: &AppContext,
    ) -> TuiSize {
        // The alt-screen app owns the whole pane.
        let size = constraint.max;
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
        let alt = model.alt_screen();
        render_grid_handler(alt.grid_handler(), origin, size, surface, &colors);

        // Submit the hardware cursor if the alt-screen app is showing it. The
        // alt screen has no scrollback, but subtract history defensively so the
        // cursor maps to a visible (screen-relative) row.
        let cursor = if alt.is_mode_set(TermMode::SHOW_CURSOR) {
            let grid = alt.grid_handler();
            let point = grid.cursor_render_point();
            point.row.checked_sub(grid.history_size()).and_then(|row| {
                let col = u16::try_from(point.col).ok()?;
                let row = u16::try_from(row).ok()?;
                (col < size.width && row < size.height).then_some((col, row))
            })
        } else {
            None
        };
        drop(model);
        if let Some((col, row)) = cursor {
            let cursor_point = ctx.scene_point(origin.offset(i32::from(col), i32::from(row)));
            ctx.set_terminal_cursor(cursor_point);
        }
    }

    fn size(&self) -> Option<TuiSize> {
        self.size
    }

    fn origin(&self) -> Option<TuiScreenPoint> {
        self.origin
    }
}

#[cfg(test)]
#[path = "alt_screen_view_tests.rs"]
mod tests;
