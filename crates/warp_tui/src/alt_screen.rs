//! Alternate-screen rendering and input forwarding for the headless TUI.
//!
//! When a terminal app switches to the alternate screen (vim, less, htop, …),
//! [`TuiTerminalSessionView`] renders a [`TuiAltScreenElement`] in place of the
//! transcript + input + footer. The element paints the alt-screen grid cells
//! directly into the frame buffer (mirroring `terminal_block`'s cell renderer)
//! and forwards every key press, mouse event, and scroll notch to the PTY as
//! raw bytes, so the running app receives input exactly as a real terminal
//! would deliver it.
//!
//! Input encoding reuses the shared [`warp_terminal`] escape-sequence machinery
//! (function keys, cursor keys, modifier-encoded sequences, the kitty keyboard
//! protocol). Two cases the shared encoder leaves to the platform `chars`
//! field — which crossterm leaves empty — are filled in here: `ctrl` + ASCII
//! letter (e.g. `ctrl-c` → `0x03`) and the unmodified special keys
//! (`enter`/`tab`/`escape`/`delete`/`insert`/`pageup`/`pagedown`).

use std::sync::Arc;

use parking_lot::FairMutex;
use warp::tui_export::TerminalModel;
use warp_terminal::model::escape_sequences::{
    EscCodes, KeystrokeWithDetails, ToEscapeSequence, C1,
};
use warp_terminal::model::grid::Dimensions as _;
use warp_terminal::model::mouse::{MouseAction, MouseButton, MouseState};
use warp_terminal::model::{Point, TermMode};
use warpui_core::elements::tui::{
    TuiBuffer, TuiConstraint, TuiElement, TuiEvent, TuiEventContext, TuiLayoutContext,
    TuiPaintContext, TuiPoint, TuiRect, TuiRectExt, TuiSize,
};
use warpui_core::event::ModifiersState;
use warpui_core::keymap::Keystroke;
use warpui_core::AppContext;

use crate::terminal_block::{cell_to_style, sanitized_symbol};
use crate::terminal_session_view::TuiTerminalSessionAction;

/// A [`TuiElement`] that paints the terminal model's alternate-screen grid and
/// forwards input to the PTY.
///
/// Constructed fresh each render while `TerminalModel::is_alt_screen_active()`
/// is true. It locks the model only for the duration of `render`,
/// `cursor_position`, and the mode/geometry reads inside `dispatch_event`.
pub(super) struct TuiAltScreenElement {
    model: Arc<FairMutex<TerminalModel>>,
}

impl TuiAltScreenElement {
    pub(super) fn new(model: Arc<FairMutex<TerminalModel>>) -> Self {
        Self { model }
    }

    /// Whether the running app has enabled SGR mouse reporting, in which case
    /// mouse events are forwarded to the PTY instead of being swallowed.
    fn app_requests_mouse(model: &TerminalModel) -> bool {
        model.is_term_mode_set(TermMode::SGR_MOUSE)
    }

    /// Forwards `bytes` to the PTY by dispatching a [`ForwardBytes`] typed
    /// action up to the owning session view.
    fn forward_bytes(event_ctx: &mut TuiEventContext, bytes: Vec<u8>) {
        if bytes.is_empty() {
            return;
        }
        event_ctx.dispatch_typed_action(TuiTerminalSessionAction::ForwardBytes(bytes));
    }

    /// Forwards a mouse event to the PTY when the running app has enabled SGR
    /// mouse reporting (and shift isn't bypassing it). Motion is only sent when
    /// the app tracks `MOUSE_MOTION`. Returns whether bytes were forwarded.
    fn forward_mouse(
        &self,
        button: MouseButton,
        action: MouseAction,
        position: TuiPoint,
        modifiers: ModifiersState,
        area: TuiRect,
        event_ctx: &mut TuiEventContext,
    ) -> bool {
        // Shift bypasses mouse reporting so the host terminal (and future TUI
        // selection) can select text; don't forward then.
        if modifiers.shift {
            return false;
        }
        let bytes = {
            let model = self.model.lock();
            // Don't forward when the app isn't listening for mouse events, or
            // for motion unless the app explicitly tracks `MOUSE_MOTION`.
            let forward = Self::app_requests_mouse(&model)
                && (!matches!(button, MouseButton::Move)
                    || model.is_term_mode_set(TermMode::MOUSE_MOTION));
            if !forward {
                Vec::new()
            } else {
                grid_point(&model, area, position)
                    .map(|point| {
                        mouse_escape_bytes(
                            &model,
                            MouseState::new(button, action, modifiers).set_point(point),
                        )
                    })
                    .unwrap_or_default()
            }
        };
        let handled = !bytes.is_empty();
        Self::forward_bytes(event_ctx, bytes);
        handled
    }
}

impl TuiElement for TuiAltScreenElement {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        _ctx: &mut TuiLayoutContext,
        _app: &AppContext,
    ) -> TuiSize {
        // The alt-screen app owns the whole pane: fill whatever is available.
        constraint.max
    }

    fn render(&self, area: TuiRect, buffer: &mut TuiBuffer, _ctx: &mut TuiPaintContext) {
        if area.is_empty() {
            return;
        }
        let model = self.model.lock();
        let colors = model.colors();
        let grid = model.alt_screen().grid_handler();
        let max_columns = (area.width as usize).min(grid.columns());
        let visible_rows = (area.height as usize).min(grid.visible_rows());
        for displayed_row in 0..visible_rows {
            let original_row = grid.maybe_translate_row_from_displayed_to_original(displayed_row);
            let Some(row) = grid.row(original_row) else {
                continue;
            };
            let y = area.y.saturating_add(displayed_row as u16);
            for column in 0..max_columns {
                let cell = &row[column];
                let x = area.x.saturating_add(column as u16);
                if let Some(buffer_cell) = buffer.cell_mut((x, y)) {
                    buffer_cell
                        .set_symbol(&sanitized_symbol(cell))
                        .set_style(cell_to_style(cell, &colors));
                }
            }
        }
    }

    fn cursor_position(&self, area: TuiRect, _ctx: &mut TuiPaintContext) -> Option<(u16, u16)> {
        let model = self.model.lock();
        if !model.is_term_mode_set(TermMode::SHOW_CURSOR) {
            return None;
        }
        let grid = model.alt_screen().grid_handler();
        let cursor = grid.cursor_render_point();
        let x = u16::try_from(cursor.col).ok()?;
        let y = u16::try_from(cursor.row).ok()?;
        if x >= area.width || y >= area.height {
            return None;
        }
        Some((x, y))
    }

    fn dispatch_event(
        &mut self,
        event: &TuiEvent,
        area: TuiRect,
        event_ctx: &mut TuiEventContext,
        _ctx: &mut TuiLayoutContext,
        _app: &AppContext,
    ) -> bool {
        match event {
            TuiEvent::KeyDown {
                keystroke,
                chars,
                details,
                is_composing,
            } => {
                if *is_composing {
                    // While composing, the IME owns the text; forward the
                    // composed chars once they arrive.
                    Self::forward_bytes(event_ctx, chars.as_bytes().to_vec());
                    return !chars.is_empty();
                }
                let bytes = {
                    let model = self.model.lock();
                    encode_keydown(
                        &model,
                        keystroke,
                        details.key_without_modifiers.as_deref(),
                        chars,
                    )
                };
                let handled = !bytes.is_empty();
                Self::forward_bytes(event_ctx, bytes);
                handled
            }
            TuiEvent::ScrollWheel {
                position,
                delta,
                modifiers,
                ..
            } => {
                let bytes = {
                    let model = self.model.lock();
                    if !Self::app_requests_mouse(&model) {
                        // No mouse reporting: translate scroll into arrow-key
                        // sequences so pagers/editors that don't track the
                        // mouse (less, vim without mouse) still scroll.
                        alt_scroll_sequences(delta.1)
                    } else {
                        grid_point(&model, area, *position)
                            .map(|point| {
                                mouse_escape_bytes(
                                    &model,
                                    MouseState::new(
                                        MouseButton::Wheel,
                                        MouseAction::Scrolled {
                                            delta: i32::try_from(delta.1).unwrap_or(0),
                                        },
                                        *modifiers,
                                    )
                                    .set_point(point),
                                )
                            })
                            .unwrap_or_default()
                    }
                };
                let handled = !bytes.is_empty();
                Self::forward_bytes(event_ctx, bytes);
                handled
            }
            TuiEvent::LeftMouseDown {
                position,
                modifiers,
                ..
            } => self.forward_mouse(
                MouseButton::Left,
                MouseAction::Pressed,
                *position,
                *modifiers,
                area,
                event_ctx,
            ),
            TuiEvent::LeftMouseUp {
                position,
                modifiers,
            } => self.forward_mouse(
                MouseButton::Left,
                MouseAction::Released,
                *position,
                *modifiers,
                area,
                event_ctx,
            ),
            TuiEvent::LeftMouseDragged {
                position,
                modifiers,
            } => self.forward_mouse(
                MouseButton::LeftDrag,
                MouseAction::Pressed,
                *position,
                *modifiers,
                area,
                event_ctx,
            ),
            TuiEvent::RightMouseDown {
                position,
                modifiers,
                ..
            } => self.forward_mouse(
                MouseButton::Right,
                MouseAction::Pressed,
                *position,
                *modifiers,
                area,
                event_ctx,
            ),
            TuiEvent::MouseMoved {
                position,
                modifiers,
                ..
            } => self.forward_mouse(
                MouseButton::Move,
                MouseAction::Pressed,
                *position,
                *modifiers,
                area,
                event_ctx,
            ),
            // Middle-click (paste) and other buttons have no alt-screen
            // behavior yet; let them propagate/no-op.
            _ => false,
        }
    }
}

/// Converts a screen-cell `position` to an alt-screen grid `Point`, clamped to
/// the grid's visible bounds. Returns `None` when the position falls outside
/// `area` or the grid is empty. The caller must already hold the model lock
/// (passed in as `model`) — re-locking the `FairMutex` from within a held lock
/// would deadlock.
fn grid_point(model: &TerminalModel, area: TuiRect, position: TuiPoint) -> Option<Point> {
    if !area.contains_point(position) {
        return None;
    }
    let grid = model.alt_screen().grid_handler();
    let columns = grid.columns();
    let visible_rows = grid.visible_rows();
    if columns == 0 || visible_rows == 0 {
        return None;
    }
    let col = (position.x.saturating_sub(area.x) as usize).min(columns - 1);
    let row = (position.y.saturating_sub(area.y) as usize).min(visible_rows - 1);
    Some(Point::new(row, col))
}

/// Encodes a mouse event to its SGR mouse reporting byte sequence. The caller
/// must already hold the model lock (for the `ModeProvider` modes); `model` is
/// passed by reference so this never re-locks.
fn mouse_escape_bytes(model: &TerminalModel, state: MouseState) -> Vec<u8> {
    state.to_escape_sequence(model).unwrap_or_default()
}

/// Encodes a TUI key-down event into the bytes a PTY-hosted app expects.
///
/// Mirrors the GUI alt-screen path: try the shared escape-sequence encoder
/// first, then fill the two gaps crossterm leaves (the GUI relies on the
/// platform `chars` for these, but crossterm reports the bare key code with an
/// empty `chars`): `ctrl` + ASCII letter → C0 control byte, and the unmodified
/// special keys. Finally, fall back to the inserted text.
fn encode_keydown(
    mode_provider: &TerminalModel,
    keystroke: &Keystroke,
    key_without_modifiers: Option<&str>,
    chars: &str,
) -> Vec<u8> {
    if let Some(bytes) = (KeystrokeWithDetails {
        keystroke,
        key_without_modifiers,
        chars: Some(chars),
    })
    .to_escape_sequence(mode_provider)
    {
        return bytes;
    }
    if let Some(bytes) = ctrl_letter_to_c0(keystroke) {
        return bytes;
    }
    if let Some(bytes) = special_key_to_bytes(keystroke) {
        return bytes;
    }
    if !chars.is_empty() {
        return chars.as_bytes().to_vec();
    }
    Vec::new()
}

/// Returns the C0 control byte for a `ctrl` + ASCII letter keystroke
/// (`ctrl-c` → `0x03`, `ctrl-z` → `0x1a`, …), or `None` for anything else.
fn ctrl_letter_to_c0(keystroke: &Keystroke) -> Option<Vec<u8>> {
    // Only a lone ctrl modifier maps letters to C0 codes; other modifiers
    // (alt/shift/meta/cmd) produce escape sequences handled above.
    if !(keystroke.ctrl && !keystroke.alt && !keystroke.meta && !keystroke.cmd && !keystroke.shift)
    {
        return None;
    }
    let mut chars = keystroke.key.chars();
    let c = chars.next()?;
    if chars.next().is_some() || !c.is_ascii_alphabetic() {
        return None;
    }
    Some(vec![(c.to_ascii_lowercase() as u8) & 0x1f])
}

/// Returns the byte sequence for an unmodified special key the shared
/// escape-sequence encoder doesn't cover (because crossterm reports these with
/// an empty `chars`, unlike the GUI's platform events).
fn special_key_to_bytes(keystroke: &Keystroke) -> Option<Vec<u8>> {
    if keystroke.ctrl || keystroke.alt || keystroke.meta || keystroke.cmd || keystroke.shift {
        return None;
    }
    let bytes: &[u8] = match keystroke.key.as_str() {
        "enter" | "numpadenter" => b"\r",
        "tab" => b"\t",
        "escape" => b"\x1b",
        "delete" => b"\x1b[3~",
        "insert" => b"\x1b[2~",
        "pageup" => b"\x1b[5~",
        "pagedown" => b"\x1b[6~",
        _ => return None,
    };
    Some(bytes.to_vec())
}

/// Builds the arrow-key sequence(s) for an alt-screen scroll: one
/// `SS3`+`Up`/`Down` per line, matching the GUI's `alt_scroll` fallback used
/// for pagers/editors that don't enable mouse reporting.
fn alt_scroll_sequences(lines: isize) -> Vec<u8> {
    let lines = i32::try_from(lines).unwrap_or(0);
    if lines == 0 {
        return Vec::new();
    }
    let cmd = if lines > 0 {
        EscCodes::ARROW_UP
    } else {
        EscCodes::ARROW_DOWN
    };
    let one = EscCodes::build_escape_sequence_with_c1(C1::SS3, &[cmd]);
    let count = lines.unsigned_abs() as usize;
    let mut content = Vec::with_capacity(count * one.len());
    for _ in 0..count {
        content.extend_from_slice(&one);
    }
    content
}
