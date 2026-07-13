//! Full-screen alt-screen rendering + raw keyboard forwarding for the TUI.
//!
//! When a PTY app switches to the alternate screen (vim, htop, less, …), the
//! terminal model flips [`TerminalModel::is_alt_screen_active`] and populates a
//! dedicated alt-screen grid. [`TuiTerminalSessionView`] then renders this
//! element full-area instead of the block/transcript UI, and forwards
//! keystrokes straight to the PTY as escape sequences — mirroring the GUI's
//! `AltScreenElement` (`app/src/terminal/alt_screen/alt_screen_element.rs`).
//!
//! Slice 1 covers rendering, the cursor, and keyboard forwarding. Mouse
//! forwarding and resize polish are tracked as follow-ups.
//!
//! [`TuiTerminalSessionView`]: crate::terminal_session_view::TuiTerminalSessionView
//! [`TerminalModel::is_alt_screen_active`]: warp::tui_export::TerminalModel

use std::ops::Deref as _;
use std::sync::Arc;

use parking_lot::FairMutex;
use warp::tui_export::{KeystrokeWithDetails, TermMode, TerminalModel, ToEscapeSequence};
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
}

impl AltScreenElement {
    pub(crate) fn new(model: Arc<FairMutex<TerminalModel>>) -> Self {
        Self { model }
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
        constraint.max
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
        // Leave ctrl-c to the root/session fixed binding as a guaranteed escape
        // hatch, so a full-screen app can never trap the user in the TUI. (v1;
        // routing ctrl-c to the app can follow once exit UX is settled.)
        if keystroke.ctrl && keystroke.key == "c" {
            return false;
        }
        // Resolve the bytes to send to the PTY, in priority order:
        // 1. Special/control keys (arrows, ctrl-<x>, fn keys, …) that encode to
        //    an escape sequence via the shared terminal encoder.
        // 2. Plain printable keys — the GUI routes these through a separate
        //    typed-characters path, which the TUI event model folds into
        //    `chars`; forward their UTF-8 bytes.
        // 3. Named control keys the encoder doesn't cover and that carry no
        //    `chars` (escape, enter, tab, backspace) — map to their C0 bytes so
        //    e.g. Escape leaves an editor's insert mode.
        let escape_sequence = {
            let model = self.model.lock();
            KeystrokeWithDetails {
                keystroke,
                key_without_modifiers: details.key_without_modifiers.as_deref(),
                chars: Some(chars.as_str()),
            }
            .to_escape_sequence(model.deref())
        };
        let bytes = escape_sequence.or_else(|| fallback_key_bytes(&keystroke.key, chars));
        let Some(bytes) = bytes else {
            return false;
        };
        event_ctx.dispatch_typed_action(TuiTerminalSessionAction::ForwardToPty(bytes));
        true
    }
}

/// PTY bytes for a key the shared terminal encoder didn't map to an escape
/// sequence (see [`ToEscapeSequence`]). Plain printable keys carry their text in
/// `chars` and forward its UTF-8 bytes; a few named control keys carry empty
/// `chars` and no encoder mapping, so map them to their C0 bytes (notably
/// `escape`, so it can leave an editor's insert mode). Returns `None` when there
/// is nothing to send.
fn fallback_key_bytes(key: &str, chars: &str) -> Option<Vec<u8>> {
    if !chars.is_empty() {
        return Some(chars.as_bytes().to_vec());
    }
    match key {
        "enter" => Some(vec![b'\r']),
        "escape" => Some(vec![0x1b]),
        "\t" => Some(vec![b'\t']),
        "backspace" => Some(vec![0x7f]),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::fallback_key_bytes;

    #[test]
    fn printable_keys_forward_their_utf8_bytes() {
        assert_eq!(fallback_key_bytes("a", "a"), Some(b"a".to_vec()));
        assert_eq!(fallback_key_bytes("A", "A"), Some(b"A".to_vec()));
        assert_eq!(fallback_key_bytes(" ", " "), Some(b" ".to_vec()));
        // A non-ASCII grapheme forwards its full UTF-8 encoding.
        assert_eq!(fallback_key_bytes("é", "é"), Some("é".as_bytes().to_vec()));
    }

    #[test]
    fn named_control_keys_map_to_c0_bytes() {
        // Escape is the important one: it lets the user leave insert mode in a
        // full-screen editor. Enter/tab/backspace round out the common set.
        assert_eq!(fallback_key_bytes("escape", ""), Some(vec![0x1b]));
        assert_eq!(fallback_key_bytes("enter", ""), Some(vec![b'\r']));
        assert_eq!(fallback_key_bytes("\t", ""), Some(vec![b'\t']));
        assert_eq!(fallback_key_bytes("backspace", ""), Some(vec![0x7f]));
    }

    #[test]
    fn unmapped_key_without_chars_sends_nothing() {
        // Keys the encoder handles (arrows, fn keys) never reach this fallback;
        // anything else with no text produces no PTY bytes.
        assert_eq!(fallback_key_bytes("f5", ""), None);
        assert_eq!(fallback_key_bytes("insert", ""), None);
    }
}
