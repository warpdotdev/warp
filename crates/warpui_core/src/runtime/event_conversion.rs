//! Conversion from raw crossterm input events to the
//! [`TuiEvent`](crate::elements::tui::TuiEvent) vocabulary.

use std::time::Duration;

use instant::Instant;
use ratatui::crossterm::event::{
    Event as CrosstermEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton,
    MouseEvent, MouseEventKind,
};

use crate::elements::tui::{TuiEvent, TuiPoint, TuiPointExt, TuiScrollDelta};
use crate::event::{KeyEventDetails, ModifiersState};
use crate::keymap::Keystroke;

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
    // Only key presses map to a warp `KeyDown`; repeats/releases are ignored so
    // dispatch matches the GUI's press-driven keystroke model.
    if event.kind != KeyEventKind::Press {
        return None;
    }

    let key = key_name(event.code, event.modifiers)?;
    let chars = match event.code {
        KeyCode::Char(char) => char.to_string(),
        _ => String::new(),
    };

    Some(TuiEvent::KeyDown {
        keystroke: Keystroke {
            ctrl: event.modifiers.contains(KeyModifiers::CONTROL),
            alt: event.modifiers.contains(KeyModifiers::ALT),
            shift: event.modifiers.contains(KeyModifiers::SHIFT),
            cmd: event.modifiers.contains(KeyModifiers::SUPER),
            meta: event.modifiers.contains(KeyModifiers::META),
            key,
        },
        chars,
        details: KeyEventDetails {
            key_without_modifiers: key_without_modifiers(event.code),
            ..Default::default()
        },
        is_composing: false,
    })
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
        KeyCode::Tab | KeyCode::BackTab => Some("\t".to_owned()),
        KeyCode::Delete => Some("delete".to_owned()),
        KeyCode::Insert => Some("insert".to_owned()),
        KeyCode::Esc => Some("escape".to_owned()),
        KeyCode::F(number) if number <= 20 => Some(format!("f{number}")),
        KeyCode::Char(' ') => Some(" ".to_owned()),
        // Align with `Keystroke::parse` conventions: shift + letter is
        // represented as the uppercase letter. Terminals differ on whether a
        // shifted letter is reported upper- or lowercase, so normalize here.
        KeyCode::Char(char) if modifiers.contains(KeyModifiers::SHIFT) => {
            Some(char.to_uppercase().to_string())
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
        KeyCode::Char(char) => Some(char.to_lowercase().to_string()),
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
