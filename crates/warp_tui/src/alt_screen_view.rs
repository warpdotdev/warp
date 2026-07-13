//! Full-screen alt-screen rendering + raw keyboard forwarding for the TUI.
//!
//! When a PTY app switches to the alternate screen (vim, htop, less, …), the
//! terminal model flips [`TerminalModel::is_alt_screen_active`] and populates a
//! dedicated alt-screen grid. [`TuiTerminalSessionView`] then renders this
//! element full-area instead of the block/transcript UI, and forwards
//! keystrokes straight to the PTY as escape sequences — mirroring the GUI's
//! `AltScreenElement` (`app/src/terminal/alt_screen/alt_screen_element.rs`).
//!
//! Covers rendering, the cursor, keyboard forwarding, and propagating the
//! laid-out cell dimensions to the terminal model and PTY. Mouse forwarding is
//! tracked as a follow-up.
//!
//! [`TuiTerminalSessionView`]: crate::terminal_session_view::TuiTerminalSessionView
//! [`TerminalModel::is_alt_screen_active`]: warp::tui_export::TerminalModel

use std::ops::Deref as _;
use std::sync::Arc;

use async_channel::Sender;
use parking_lot::FairMutex;
use warp::tui_export::{KeystrokeWithDetails, TermMode, TerminalModel};
use warp_terminal::model::grid::Dimensions as _;
use warpui_core::elements::tui::{
    TuiBuffer, TuiConstraint, TuiElement, TuiEvent, TuiEventContext, TuiLayoutContext,
    TuiPaintContext, TuiRect, TuiSize,
};
use warpui_core::AppContext;

use crate::terminal_block::render_grid_handler;
use crate::terminal_session_view::TuiTerminalSessionAction;

/// Renders the terminal's alt-screen grid full-area and forwards keystrokes to
/// the PTY while a full-screen app is active.
pub(crate) struct AltScreenElement {
    model: Arc<FairMutex<TerminalModel>>,
    resize_tx: Sender<TuiSize>,
}

impl AltScreenElement {
    pub(crate) fn new(model: Arc<FairMutex<TerminalModel>>, resize_tx: Sender<TuiSize>) -> Self {
        Self { model, resize_tx }
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
        let _ = self.resize_tx.try_send(size);
        size
    }

    fn render(&self, area: TuiRect, buffer: &mut TuiBuffer, _ctx: &mut TuiPaintContext) {
        let model = self.model.lock();
        let colors = model.colors();
        render_grid_handler(model.alt_screen().grid_handler(), area, buffer, &colors);
    }

    fn cursor_position(&self, area: TuiRect, _ctx: &mut TuiPaintContext) -> Option<(u16, u16)> {
        let model = self.model.lock();
        let alt = model.alt_screen();
        if !alt.is_mode_set(TermMode::SHOW_CURSOR) {
            return None;
        }
        let grid = alt.grid_handler();
        let point = grid.cursor_render_point();
        // The alt screen has no scrollback, but subtract history defensively so
        // the cursor maps to a visible (screen-relative) row.
        let row = point.row.checked_sub(grid.history_size())?;
        let col = u16::try_from(point.col).ok()?;
        let row = u16::try_from(row).ok()?;
        if col >= area.width || row >= area.height {
            return None;
        }
        Some((area.x.saturating_add(col), area.y.saturating_add(row)))
    }

    fn dispatch_event(
        &mut self,
        event: &TuiEvent,
        _area: TuiRect,
        event_ctx: &mut TuiEventContext,
        _ctx: &mut TuiLayoutContext,
        _app: &AppContext,
    ) -> bool {
        let TuiEvent::KeyDown {
            keystroke,
            chars,
            details,
            is_composing,
        } = event
        else {
            // Mouse forwarding is a follow-up slice.
            return false;
        };
        if *is_composing {
            return false;
        }
        // Forward the key to the app. `to_pty_bytes` layers the fallbacks a
        // single-`KeyDown` frontend needs — `Ctrl+<letter>` → C0, printable
        // `chars`, and named control keys — on top of the shared
        // `to_escape_sequence` encoder in `warp_terminal`. (ctrl-c never reaches
        // here: the session view's interrupt handler forwards it to the app.)
        let bytes = {
            let model = self.model.lock();
            KeystrokeWithDetails {
                keystroke,
                key_without_modifiers: details.key_without_modifiers.as_deref(),
                chars: Some(chars.as_str()),
            }
            .to_pty_bytes(model.deref())
        };
        let Some(bytes) = bytes else {
            return false;
        };
        event_ctx.dispatch_typed_action(TuiTerminalSessionAction::ForwardToPty(bytes));
        true
    }
}

#[cfg(test)]
#[path = "alt_screen_view_tests.rs"]
mod tests;
