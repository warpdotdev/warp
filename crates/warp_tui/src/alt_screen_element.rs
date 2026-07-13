//! Alt-screen rendering and input forwarding for the TUI.
//!
//! While the session's PTY is on the alternate screen (vim, less, htop, …),
//! [`TuiTerminalSessionView`](crate::terminal_session_view::TuiTerminalSessionView)
//! renders a [`TuiAltScreenElement`] instead of the transcript UI. The element
//! paints the [`TerminalModel`]'s alt-screen grid full-bleed and consumes every
//! key and mouse event, re-encoding each one as the escape sequence the
//! running program expects (via the shared `warp_terminal` encoders) and
//! handing the bytes back to the session view as a
//! [`TuiTerminalSessionAction::WriteAltScreenInput`] typed action, which the
//! view forwards to the PTY.

use std::cell::Cell as StdCell;
use std::rc::Rc;
use std::sync::Arc;

use parking_lot::FairMutex;
use warp::tui_export::{TerminalColorList, TerminalModel};
use warp_terminal::model::escape_sequences::{
    EscCodes, KeystrokeWithDetails, ToEscapeSequence, C0,
};
use warp_terminal::model::grid::Dimensions as _;
use warp_terminal::model::mouse::{MouseAction, MouseButton, MouseState};
use warp_terminal::model::{Point, TermMode};
use warpui_core::elements::tui::{
    TuiBuffer, TuiConstraint, TuiElement, TuiEvent, TuiEventContext, TuiLayoutContext,
    TuiPaintContext, TuiPoint, TuiRect, TuiSize,
};
use warpui_core::event::ModifiersState;
use warpui_core::keymap::Keystroke;
use warpui_core::AppContext;

use crate::terminal_block::{cell_to_style, sanitized_symbol};
use crate::terminal_session_view::TuiTerminalSessionAction;

/// A shared slot the session view reads to learn the host terminal's current
/// size in cells. Both the alt-screen element and the transcript-path probe
/// record the layout constraint here every frame, so the view can push PTY
/// resizes (`SIGWINCH`) that track the real window.
pub(crate) type HostSizeSlot = Rc<StdCell<Option<TuiSize>>>;

/// Paints the terminal model's alternate screen and forwards input to it.
///
/// This is a bespoke [`TuiElement`] for the same reason as
/// [`TerminalBlockElement`](crate::terminal_block::TerminalBlockElement):
/// terminal cells each carry their own fg/bg/flags, which no generic
/// single-style text element can express.
pub(crate) struct TuiAltScreenElement {
    model: Arc<FairMutex<TerminalModel>>,
    host_size: HostSizeSlot,
}

impl TuiAltScreenElement {
    pub(crate) fn new(model: Arc<FairMutex<TerminalModel>>, host_size: HostSizeSlot) -> Self {
        Self { model, host_size }
    }

    /// Encodes a key-down event into the bytes the alt-screen program expects,
    /// honoring the terminal's live modes (application cursor keys, the kitty
    /// keyboard protocol, …) via the shared `warp_terminal` encoders, with a
    /// TUI-specific fallback for keys the legacy encoders leave to the
    /// platform layer in the GUI (plain characters, ctrl-letters, enter/tab).
    fn encode_key(
        model: &TerminalModel,
        keystroke: &Keystroke,
        chars: &str,
        key_without_modifiers: Option<&str>,
    ) -> Option<Vec<u8>> {
        let with_details = KeystrokeWithDetails {
            keystroke,
            key_without_modifiers,
            chars: (!chars.is_empty()).then_some(chars),
        };
        with_details
            .to_escape_sequence(model)
            .or_else(|| fallback_key_bytes(keystroke, chars))
    }

    /// Whether the running program asked for mouse reporting that we can
    /// encode (any tracking mode, in SGR encoding — the only encoding the
    /// shared [`MouseState`] encoder emits).
    fn wants_mouse_reporting(model: &TerminalModel) -> bool {
        model.is_term_mode_set(TermMode::SGR_MOUSE)
            && (model.is_term_mode_set(TermMode::MOUSE_REPORT_CLICK)
                || model.is_term_mode_set(TermMode::MOUSE_DRAG)
                || model.is_term_mode_set(TermMode::MOUSE_MOTION))
    }

    /// Encodes a mouse event into the SGR report (or alternate-scroll arrow
    /// keys) the alt-screen program expects, or `None` when the event should
    /// be swallowed.
    fn encode_mouse(
        model: &TerminalModel,
        event: &TuiEvent,
        area: TuiRect,
        position: TuiPoint,
        modifiers: ModifiersState,
    ) -> Option<Vec<u8>> {
        let point = Point::new(
            usize::from(position.y.saturating_sub(area.y)),
            usize::from(position.x.saturating_sub(area.x)),
        );
        let reporting = Self::wants_mouse_reporting(model);
        let state = match event {
            TuiEvent::LeftMouseDown { .. } => {
                MouseState::new(MouseButton::Left, MouseAction::Pressed, modifiers)
            }
            TuiEvent::LeftMouseUp { .. } => {
                MouseState::new(MouseButton::Left, MouseAction::Released, modifiers)
            }
            TuiEvent::RightMouseDown { .. } => {
                MouseState::new(MouseButton::Right, MouseAction::Pressed, modifiers)
            }
            TuiEvent::LeftMouseDragged { .. } => {
                // Drag reports require button-motion (1002) or any-motion
                // (1003) tracking.
                if !(model.is_term_mode_set(TermMode::MOUSE_DRAG)
                    || model.is_term_mode_set(TermMode::MOUSE_MOTION))
                {
                    return None;
                }
                MouseState::new(MouseButton::LeftDrag, MouseAction::Pressed, modifiers)
            }
            TuiEvent::MouseMoved { .. } => {
                // Motion-without-buttons reports require any-motion (1003)
                // tracking.
                if !model.is_term_mode_set(TermMode::MOUSE_MOTION) {
                    return None;
                }
                MouseState::new(MouseButton::Move, MouseAction::Pressed, modifiers)
            }
            TuiEvent::ScrollWheel { delta, .. } => {
                let rows = i32::try_from(delta.1).unwrap_or(0);
                if rows == 0 {
                    return None;
                }
                if reporting {
                    MouseState::new(
                        MouseButton::Wheel,
                        MouseAction::Scrolled { delta: rows },
                        modifiers,
                    )
                } else if model.is_term_mode_set(TermMode::ALTERNATE_SCROLL) {
                    // Without mouse tracking, alternate-scroll mode translates
                    // wheel ticks into arrow keys (how `less` and `vim` scroll).
                    let arrow = if rows > 0 {
                        EscCodes::ARROW_UP
                    } else {
                        EscCodes::ARROW_DOWN
                    };
                    let sequence = EscCodes::build_escape_sequence(model, &[arrow]);
                    return Some(sequence.repeat(rows.unsigned_abs() as usize));
                } else {
                    return None;
                }
            }
            TuiEvent::MiddleMouseDown { .. } | TuiEvent::KeyDown { .. } => return None,
        };
        if !reporting {
            return None;
        }
        state.set_point(point).to_escape_sequence(model)
    }
}

impl TuiElement for TuiAltScreenElement {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        _ctx: &mut TuiLayoutContext,
        _app: &AppContext,
    ) -> TuiSize {
        // The layout constraint is where the host terminal size is known;
        // record it so the session view can keep the PTY winsize in sync.
        self.host_size.set(Some(constraint.max));
        constraint.max
    }

    fn render(&self, area: TuiRect, buffer: &mut TuiBuffer, _ctx: &mut TuiPaintContext) {
        let model = self.model.lock();
        if !model.is_alt_screen_active() {
            return;
        }
        let colors: TerminalColorList = model.colors();
        let grid = model.alt_screen().grid_handler();
        let history_size = grid.history_size();
        let rows = grid.visible_rows().min(usize::from(area.height));
        let columns = grid.columns().min(usize::from(area.width));
        for visible_row in 0..rows {
            let Some(row) = grid.row(history_size + visible_row) else {
                continue;
            };
            let y = area.y.saturating_add(visible_row as u16);
            for column in 0..columns {
                let cell = &row[column];
                if let Some(buffer_cell) =
                    buffer.cell_mut((area.x.saturating_add(column as u16), y))
                {
                    buffer_cell
                        .set_symbol(&sanitized_symbol(cell))
                        .set_style(cell_to_style(cell, &colors));
                }
            }
        }
    }

    fn cursor_position(&self, area: TuiRect, _ctx: &mut TuiPaintContext) -> Option<(u16, u16)> {
        let model = self.model.lock();
        if !model.is_alt_screen_active() || !model.is_term_mode_set(TermMode::SHOW_CURSOR) {
            return None;
        }
        let grid = model.alt_screen().grid_handler();
        let point = grid.cursor_render_point();
        let row = point.row.checked_sub(grid.history_size())?;
        let x = u16::try_from(point.col).ok()?;
        let y = u16::try_from(row).ok()?;
        (x < area.width && y < area.height).then_some((x, y))
    }

    fn dispatch_event(
        &mut self,
        event: &TuiEvent,
        area: TuiRect,
        event_ctx: &mut TuiEventContext,
        _ctx: &mut TuiLayoutContext,
        _app: &AppContext,
    ) -> bool {
        // Encode under a short-lived lock, then dispatch after it is dropped.
        let bytes = {
            let model = self.model.lock();
            if !model.is_alt_screen_active() {
                return false;
            }
            match event {
                TuiEvent::KeyDown {
                    keystroke,
                    chars,
                    details,
                    ..
                } => Self::encode_key(
                    &model,
                    keystroke,
                    chars,
                    details.key_without_modifiers.as_deref(),
                ),
                TuiEvent::ScrollWheel { modifiers, .. }
                | TuiEvent::LeftMouseDown { modifiers, .. }
                | TuiEvent::LeftMouseUp { modifiers, .. }
                | TuiEvent::LeftMouseDragged { modifiers, .. }
                | TuiEvent::MiddleMouseDown { modifiers, .. }
                | TuiEvent::RightMouseDown { modifiers, .. }
                | TuiEvent::MouseMoved { modifiers, .. } => {
                    let position = event.position().unwrap_or_default();
                    Self::encode_mouse(&model, event, area, position, *modifiers)
                }
            }
        };
        if let Some(bytes) = bytes {
            event_ctx.dispatch_typed_action(TuiTerminalSessionAction::WriteAltScreenInput(bytes));
        }
        // Consume every event while the alt screen owns the surface, so
        // nothing leaks into the (hidden) transcript UI.
        true
    }
}

/// Records the layout constraint into the shared host-size slot and otherwise
/// delegates to its child. Wrapped around the transcript-path render tree so
/// the host terminal size is known *before* the alt screen activates (the
/// mode-swap handler needs it to size the PTY immediately).
pub(crate) struct TuiHostSizeProbe {
    child: Box<dyn TuiElement>,
    host_size: HostSizeSlot,
}

impl TuiHostSizeProbe {
    pub(crate) fn new(child: Box<dyn TuiElement>, host_size: HostSizeSlot) -> Self {
        Self { child, host_size }
    }
}

impl TuiElement for TuiHostSizeProbe {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> TuiSize {
        self.host_size.set(Some(constraint.max));
        self.child.layout(constraint, ctx, app)
    }

    fn render(&self, area: TuiRect, buffer: &mut TuiBuffer, ctx: &mut TuiPaintContext) {
        self.child.render(area, buffer, ctx);
    }

    fn cursor_position(&self, area: TuiRect, ctx: &mut TuiPaintContext) -> Option<(u16, u16)> {
        self.child.cursor_position(area, ctx)
    }

    fn present(&mut self, ctx: &mut warpui_core::elements::tui::TuiPresentationContext<'_>) {
        self.child.present(ctx);
    }

    fn dispatch_event(
        &mut self,
        event: &TuiEvent,
        area: TuiRect,
        event_ctx: &mut TuiEventContext,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> bool {
        self.child.dispatch_event(event, area, event_ctx, ctx, app)
    }
}

/// Legacy byte encodings for keys the shared escape-sequence encoders leave
/// to the GUI's platform input layer (which the TUI does not have): enter,
/// tab, escape, editing/navigation keys, ctrl-letter control codes, and plain
/// typed characters (alt-prefixed with ESC when the alt modifier is held).
fn fallback_key_bytes(keystroke: &Keystroke, chars: &str) -> Option<Vec<u8>> {
    let special: Option<&[u8]> = match keystroke.key.as_str() {
        "enter" => Some(b"\r"),
        "escape" => Some(b"\x1b"),
        "\t" => {
            if keystroke.shift {
                Some(b"\x1b[Z")
            } else {
                Some(b"\t")
            }
        }
        "delete" => Some(b"\x1b[3~"),
        "insert" => Some(b"\x1b[2~"),
        "pageup" => Some(b"\x1b[5~"),
        "pagedown" => Some(b"\x1b[6~"),
        _ => None,
    };
    if let Some(special) = special {
        let mut bytes = Vec::with_capacity(special.len() + 1);
        if keystroke.alt {
            bytes.push(C0::ESC);
        }
        bytes.extend_from_slice(special);
        return Some(bytes);
    }

    // Ctrl-modified single characters map to C0 control codes. The shared
    // table only covers ctrl-space and ctrl-digits; letters arrive here as
    // plain characters (the GUI receives the control byte from the OS).
    if keystroke.ctrl && !keystroke.cmd {
        let mut key_chars = keystroke.key.chars();
        if let (Some(c), None) = (key_chars.next(), key_chars.next()) {
            if c == '?' {
                return Some(vec![C0::DEL]);
            }
            let upper = c.to_ascii_uppercase();
            if matches!(upper, '@'..='_') {
                let control = (upper as u8) & 0x1f;
                let bytes = if keystroke.alt {
                    vec![C0::ESC, control]
                } else {
                    vec![control]
                };
                return Some(bytes);
            }
        }
        return None;
    }

    // Plain typed characters pass through as-is (ESC-prefixed for alt).
    if !chars.is_empty() && !keystroke.cmd {
        let mut bytes = Vec::with_capacity(chars.len() + 1);
        if keystroke.alt {
            bytes.push(C0::ESC);
        }
        bytes.extend_from_slice(chars.as_bytes());
        return Some(bytes);
    }
    None
}

#[cfg(test)]
#[path = "alt_screen_element_tests.rs"]
mod tests;
