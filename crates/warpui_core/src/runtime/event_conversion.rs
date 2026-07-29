//! Conversion from raw crossterm input events to the
//! [`TuiEvent`](crate::elements::tui::TuiEvent) vocabulary.

use std::time::Duration;

use instant::Instant;
use ratatui::crossterm::event::{
    Event as CrosstermEvent, KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers,
    ModifierKeyCode, MouseButton, MouseEvent, MouseEventKind,
};

use crate::elements::tui::{TuiEvent, TuiPoint, TuiPointExt, TuiScrollDelta};
use crate::event::{KeyEventDetails, KeyState, ModifiersState};
use crate::keymap::Keystroke;
use crate::platform::keyboard::KeyCode as PhysicalKeyCode;

/// Converts a raw crossterm event into the TUI event vocabulary, or
/// `None` if the event has no TUI equivalent yet.
pub fn crossterm_event_to_tui_event(event: CrosstermEvent) -> Option<TuiEvent> {
    match event {
        CrosstermEvent::Key(key_event) => key_event_to_tui_event(key_event),
        CrosstermEvent::Mouse(mouse_event) => TuiEvent::try_from(mouse_event).ok(),
        CrosstermEvent::Paste(text) => Some(TuiEvent::Paste { text }),
        // TODO: FocusGained and FocusLost have no TUI equivalents yet.
        // If these are needed in the future, consider adding matching TuiEvent variants.
        CrosstermEvent::FocusGained | CrosstermEvent::FocusLost | CrosstermEvent::Resize(_, _) => {
            None
        }
    }
}

impl TryFrom<MouseEvent> for TuiEvent {
    type Error = ();

    fn try_from(event: MouseEvent) -> Result<Self, Self::Error> {
        let position = TuiPoint::new(event.column, event.row);
        let modifiers = modifiers_state(event.modifiers);

        match event.kind {
            MouseEventKind::ScrollUp => Ok(scroll_wheel(position, (0, 1), modifiers)),
            MouseEventKind::ScrollDown => Ok(scroll_wheel(position, (0, -1), modifiers)),
            MouseEventKind::ScrollLeft => Ok(scroll_wheel(position, (1, 0), modifiers)),
            MouseEventKind::ScrollRight => Ok(scroll_wheel(position, (-1, 0), modifiers)),
            MouseEventKind::Down(MouseButton::Left) => Ok(TuiEvent::LeftMouseDown {
                position,
                modifiers,
                click_count: 1,
                is_first_mouse: false,
            }),
            MouseEventKind::Down(MouseButton::Middle) => Ok(TuiEvent::MiddleMouseDown {
                position,
                modifiers,
                click_count: 1,
            }),
            MouseEventKind::Down(MouseButton::Right) => Ok(TuiEvent::RightMouseDown {
                position,
                modifiers,
                click_count: 1,
            }),
            MouseEventKind::Up(MouseButton::Left) => Ok(TuiEvent::LeftMouseUp {
                position,
                modifiers,
            }),
            MouseEventKind::Drag(MouseButton::Left) => Ok(TuiEvent::LeftMouseDragged {
                position,
                modifiers,
            }),
            MouseEventKind::Moved => Ok(TuiEvent::MouseMoved {
                position,
                modifiers,
                is_synthetic: false,
            }),
            // Add these variants when a concrete TUI consumer needs them.
            MouseEventKind::Up(MouseButton::Middle | MouseButton::Right)
            | MouseEventKind::Drag(MouseButton::Middle | MouseButton::Right) => Err(()),
        }
    }
}

fn scroll_wheel(position: TuiPoint, delta: TuiScrollDelta, modifiers: ModifiersState) -> TuiEvent {
    TuiEvent::ScrollWheel {
        position,
        delta,
        precise: false,
        modifiers,
    }
}

fn modifiers_state(modifiers: KeyModifiers) -> ModifiersState {
    ModifiersState {
        alt: modifiers.contains(KeyModifiers::ALT),
        cmd: modifiers.contains(KeyModifiers::SUPER),
        shift: modifiers.contains(KeyModifiers::SHIFT),
        ctrl: modifiers.contains(KeyModifiers::CONTROL),
        func: false,
    }
}

fn key_event_to_tui_event(event: KeyEvent) -> Option<TuiEvent> {
    if let KeyCode::Modifier(code) = event.code {
        let key_code = modifier_key(code)?;
        let state = match event.kind {
            KeyEventKind::Press => KeyState::Pressed,
            KeyEventKind::Release => KeyState::Released,
            KeyEventKind::Repeat => return None,
        };
        return Some(TuiEvent::ModifierKeyChanged { key_code, state });
    }
    if event.kind == KeyEventKind::Release {
        return None;
    }
    let key = key_name(event.code, event.modifiers)?;

    Some(TuiEvent::KeyDown {
        keystroke: Keystroke {
            ctrl: event.modifiers.contains(KeyModifiers::CONTROL),
            alt: event.modifiers.contains(KeyModifiers::ALT),
            shift: event.modifiers.contains(KeyModifiers::SHIFT),
            cmd: event.modifiers.contains(KeyModifiers::SUPER),
            meta: event.modifiers.contains(KeyModifiers::META),
            key,
        },
        chars: produced_chars(event, cfg!(windows)),
        details: KeyEventDetails {
            key_without_modifiers: key_without_modifiers(event.code),
            ..Default::default()
        },
        is_composing: false,
    })
}

/// Whether Crossterm's alternate-key substitution consumed a held Shift, and
/// whether the character it substituted still yields its own base key.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ShiftRestoration {
    /// Shift was reported accurately, so nothing was restored.
    Unchanged,
    /// A letter, whose base key is recoverable by lowercasing it.
    Letter,
    /// A symbol, which encodes the shifted state in the character itself and
    /// leaves the base key unrecoverable.
    Symbol,
}

/// Tracks the physically held Shift keys so a character event whose Shift bit
/// Crossterm consumed — while substituting the layout's shifted character — can
/// have it restored.
///
/// Only a character event can lose the bit that way, so every other event
/// reports Shift accurately and re-syncs the tracked state. A dropped release
/// therefore desyncs input until the next non-character key or mouse report
/// rather than indefinitely. Re-syncing eagerly is deliberate: stale state
/// force-uppercases everything the user types, while clearing it while Shift is
/// still held only drops Shift from a symbol keystroke.
#[derive(Default)]
pub(crate) struct ShiftKeyTracker {
    left_pressed: bool,
    right_pressed: bool,
}

impl ShiftKeyTracker {
    pub(crate) fn update(&mut self, event: &mut CrosstermEvent) -> ShiftRestoration {
        match event {
            CrosstermEvent::Key(key_event) => self.update_from_key(key_event),
            CrosstermEvent::Mouse(mouse_event) => {
                self.sync(mouse_event.modifiers);
                ShiftRestoration::Unchanged
            }
            CrosstermEvent::FocusGained | CrosstermEvent::FocusLost => {
                self.release_all();
                ShiftRestoration::Unchanged
            }
            CrosstermEvent::Paste(_) | CrosstermEvent::Resize(_, _) => ShiftRestoration::Unchanged,
        }
    }

    fn update_from_key(&mut self, event: &mut KeyEvent) -> ShiftRestoration {
        match event.code {
            KeyCode::Modifier(ModifierKeyCode::LeftShift) => {
                update_pressed(&mut self.left_pressed, event.kind);
                ShiftRestoration::Unchanged
            }
            KeyCode::Modifier(ModifierKeyCode::RightShift) => {
                update_pressed(&mut self.right_pressed, event.kind);
                ShiftRestoration::Unchanged
            }
            KeyCode::Char(character)
                if (self.left_pressed || self.right_pressed)
                    && !event.modifiers.contains(KeyModifiers::SHIFT) =>
            {
                event.modifiers.insert(KeyModifiers::SHIFT);
                if character.is_alphabetic() {
                    ShiftRestoration::Letter
                } else {
                    ShiftRestoration::Symbol
                }
            }
            KeyCode::Char(_) => ShiftRestoration::Unchanged,
            _ => {
                self.sync(event.modifiers);
                ShiftRestoration::Unchanged
            }
        }
    }

    /// Drops tracked state when an event that reports Shift accurately shows it
    /// is not held.
    fn sync(&mut self, modifiers: KeyModifiers) {
        if !modifiers.contains(KeyModifiers::SHIFT) {
            self.release_all();
        }
    }

    fn release_all(&mut self) {
        self.left_pressed = false;
        self.right_pressed = false;
    }
}

fn update_pressed(pressed: &mut bool, kind: KeyEventKind) {
    match kind {
        KeyEventKind::Press => *pressed = true,
        KeyEventKind::Release => *pressed = false,
        KeyEventKind::Repeat => {}
    }
}

fn modifier_key(code: ModifierKeyCode) -> Option<PhysicalKeyCode> {
    match code {
        ModifierKeyCode::LeftAlt => Some(PhysicalKeyCode::AltLeft),
        ModifierKeyCode::RightAlt => Some(PhysicalKeyCode::AltRight),
        ModifierKeyCode::LeftControl => Some(PhysicalKeyCode::ControlLeft),
        ModifierKeyCode::RightControl => Some(PhysicalKeyCode::ControlRight),
        ModifierKeyCode::LeftShift => Some(PhysicalKeyCode::ShiftLeft),
        ModifierKeyCode::RightShift => Some(PhysicalKeyCode::ShiftRight),
        ModifierKeyCode::LeftSuper => Some(PhysicalKeyCode::SuperLeft),
        ModifierKeyCode::RightSuper => Some(PhysicalKeyCode::SuperRight),
        ModifierKeyCode::LeftHyper
        | ModifierKeyCode::LeftMeta
        | ModifierKeyCode::RightHyper
        | ModifierKeyCode::RightMeta
        | ModifierKeyCode::IsoLevel3Shift
        | ModifierKeyCode::IsoLevel5Shift => None,
    }
}

fn produced_chars(event: KeyEvent, key_code_contains_produced_text: bool) -> String {
    let KeyCode::Char(char) = event.code else {
        return String::new();
    };
    if key_code_contains_produced_text {
        return char.to_string();
    }
    if char.is_ascii_alphabetic() && event.state.contains(KeyEventState::CAPS_LOCK) {
        return if event.modifiers.contains(KeyModifiers::SHIFT) {
            char.to_ascii_lowercase()
        } else {
            char.to_ascii_uppercase()
        }
        .to_string();
    }
    if event.modifiers.contains(KeyModifiers::SHIFT) {
        return char.to_uppercase().collect();
    }
    char.to_string()
}

/// The TUI keystroke `key` name for a crossterm key code, or `None` for keys
/// with no TUI equivalent (pure modifiers, lock keys, media keys, etc.).
fn key_name(code: KeyCode, modifiers: KeyModifiers) -> Option<String> {
    match code {
        KeyCode::Backspace => Some("backspace".to_owned()),
        KeyCode::Enter => Some("enter".to_owned()),
        KeyCode::Left => Some("left".to_owned()),
        KeyCode::Right => Some("right".to_owned()),
        KeyCode::Up => Some("up".to_owned()),
        KeyCode::Down => Some("down".to_owned()),
        KeyCode::Home => Some("home".to_owned()),
        KeyCode::End => Some("end".to_owned()),
        KeyCode::PageUp => Some("pageup".to_owned()),
        KeyCode::PageDown => Some("pagedown".to_owned()),
        KeyCode::Tab | KeyCode::BackTab => Some("tab".to_owned()),
        KeyCode::Delete => Some("delete".to_owned()),
        KeyCode::Insert => Some("insert".to_owned()),
        KeyCode::Esc => Some("escape".to_owned()),
        KeyCode::F(number) if number <= 20 => Some(format!("f{number}")),
        KeyCode::Char(' ') => Some(" ".to_owned()),
        // Align with `Keystroke::parse` conventions: shift + letter is
        // represented as the uppercase letter. Terminals differ on whether a
        // shifted letter is reported upper- or lowercase, so normalize here.
        KeyCode::Char(char) if modifiers.contains(KeyModifiers::SHIFT) => {
            Some(char.to_uppercase().collect())
        }
        KeyCode::Char(char) => Some(char.to_lowercase().to_string()),
        KeyCode::Null
        | KeyCode::CapsLock
        | KeyCode::ScrollLock
        | KeyCode::NumLock
        | KeyCode::PrintScreen
        | KeyCode::Pause
        | KeyCode::Menu
        | KeyCode::KeypadBegin
        | KeyCode::Media(_)
        | KeyCode::Modifier(_)
        | KeyCode::F(_) => None,
    }
}

fn key_without_modifiers(code: KeyCode) -> Option<String> {
    match code {
        KeyCode::Char(char) => Some(char.to_lowercase().collect()),
        _ => None,
    }
}

/// Maximum delay between consecutive presses of the same button for them to
/// count as part of the same multi-click (double/triple). Roughly the standard
/// desktop double-click window.
const MULTI_CLICK_INTERVAL: Duration = Duration::from_millis(400);

/// The pointer button a [`ClickTracker`] is tracking a multi-click run for.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ClickButton {
    Left,
    Middle,
    Right,
}

/// Synthesizes multi-click counts for mouse presses.
///
/// crossterm only reports raw button presses, so the `*MouseDown` events arrive
/// with `click_count: 1`. This tracker remembers the previous press and, when
/// the next one is the **same button**, lands within [`MULTI_CLICK_INTERVAL`],
/// and on (or within one cell of) the same position, escalates the count
/// `1 -> 2 -> 3` before wrapping back to `1`. Anything else — a different
/// button, a slower press, or a press elsewhere — resets to a single click.
/// This mirrors the GUI, where the OS supplies a click count for every button.
#[derive(Default)]
pub(crate) struct ClickTracker {
    last: Option<LastClick>,
}

#[derive(Clone, Copy)]
struct LastClick {
    button: ClickButton,
    at: Instant,
    position: TuiPoint,
    count: u32,
}

impl ClickTracker {
    /// Fills in the synthesized `click_count` on any mouse-down event, leaving
    /// non-button events (scroll, move, up, drag) untouched.
    pub(crate) fn annotate(&mut self, event: &mut TuiEvent, now: Instant) {
        let (button, position, click_count) = match event {
            TuiEvent::LeftMouseDown {
                position,
                click_count,
                ..
            } => (ClickButton::Left, *position, click_count),
            TuiEvent::MiddleMouseDown {
                position,
                click_count,
                ..
            } => (ClickButton::Middle, *position, click_count),
            TuiEvent::RightMouseDown {
                position,
                click_count,
                ..
            } => (ClickButton::Right, *position, click_count),
            _ => return,
        };
        *click_count = self.register(button, position, now);
    }

    fn register(&mut self, button: ClickButton, position: TuiPoint, now: Instant) -> u32 {
        let count = match self.last {
            Some(last)
                if last.button == button
                    && now.duration_since(last.at) <= MULTI_CLICK_INTERVAL
                    && last.position.is_adjacent(position) =>
            {
                // Wrap 3 -> 1 so a fourth fast click starts a fresh cycle.
                last.count % 3 + 1
            }
            _ => 1,
        };
        self.last = Some(LastClick {
            button,
            at: now,
            position,
            count,
        });
        count
    }
}

#[cfg(test)]
#[path = "event_conversion_tests.rs"]
mod tests;
