//! Conversion from raw crossterm input events to the shared
//! [`Event`](crate::Event) vocabulary, so TUI element/view dispatch is
//! identical to the GUI's.

use crossterm::event::{
    Event as CrosstermEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton,
    MouseEvent, MouseEventKind,
};

use crate::event::{KeyEventDetails, ModifiersState};
use crate::geometry::vector::{vec2f, Vector2F};
use crate::keymap::Keystroke;
use crate::Event;

/// Converts a raw crossterm event into the shared [`Event`] vocabulary, or
/// `None` if the event has no warp equivalent.
pub fn crossterm_event_to_warp_event(event: CrosstermEvent) -> Option<Event> {
    match event {
        CrosstermEvent::Key(key_event) => key_event_to_warp_event(key_event),
        CrosstermEvent::Mouse(mouse_event) => mouse_event_to_warp_event(mouse_event),
        CrosstermEvent::FocusGained
        | CrosstermEvent::FocusLost
        | CrosstermEvent::Paste(_)
        | CrosstermEvent::Resize(_, _) => None,
    }
}

fn key_event_to_warp_event(event: KeyEvent) -> Option<Event> {
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

    Some(Event::KeyDown {
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

/// The warp keystroke `key` name for a crossterm key code, or `None` for keys
/// with no warp equivalent (pure modifiers, lock keys, media keys, etc.).
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
        KeyCode::Char(char) if modifiers.contains(KeyModifiers::SHIFT) => Some(char.to_string()),
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

fn mouse_event_to_warp_event(event: MouseEvent) -> Option<Event> {
    let position = vec2f(f32::from(event.column), f32::from(event.row));
    let modifiers = modifiers_state(event.modifiers);
    match event.kind {
        MouseEventKind::Down(MouseButton::Left) => Some(Event::LeftMouseDown {
            position,
            modifiers,
            click_count: 1,
            is_first_mouse: false,
        }),
        MouseEventKind::Up(MouseButton::Left) => Some(Event::LeftMouseUp {
            position,
            modifiers,
        }),
        MouseEventKind::Drag(MouseButton::Left) => Some(Event::LeftMouseDragged {
            position,
            modifiers,
        }),
        MouseEventKind::Down(MouseButton::Middle) => Some(Event::MiddleMouseDown {
            position,
            cmd: modifiers.cmd,
            shift: modifiers.shift,
            click_count: 1,
        }),
        MouseEventKind::Down(MouseButton::Right) => Some(Event::RightMouseDown {
            position,
            cmd: modifiers.cmd,
            shift: modifiers.shift,
            click_count: 1,
        }),
        MouseEventKind::Moved => Some(Event::MouseMoved {
            position,
            cmd: modifiers.cmd,
            shift: modifiers.shift,
            is_synthetic: false,
        }),
        MouseEventKind::ScrollUp => Some(scroll_wheel_event(position, modifiers, vec2f(0.0, 1.0))),
        MouseEventKind::ScrollDown => {
            Some(scroll_wheel_event(position, modifiers, vec2f(0.0, -1.0)))
        }
        MouseEventKind::ScrollLeft => {
            Some(scroll_wheel_event(position, modifiers, vec2f(-1.0, 0.0)))
        }
        MouseEventKind::ScrollRight => {
            Some(scroll_wheel_event(position, modifiers, vec2f(1.0, 0.0)))
        }
        MouseEventKind::Up(MouseButton::Middle | MouseButton::Right)
        | MouseEventKind::Drag(MouseButton::Middle | MouseButton::Right) => None,
    }
}

fn scroll_wheel_event(position: Vector2F, modifiers: ModifiersState, delta: Vector2F) -> Event {
    Event::ScrollWheel {
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

#[cfg(test)]
#[path = "event_conversion_tests.rs"]
mod tests;
