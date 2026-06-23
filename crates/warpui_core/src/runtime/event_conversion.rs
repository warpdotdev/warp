//! Conversion from raw crossterm input events to the shared
//! [`Event`](crate::Event) vocabulary, so TUI element/view dispatch is
//! identical to the GUI's.

use ratatui::crossterm::event::{
    Event as CrosstermEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
};

use crate::event::KeyEventDetails;
use crate::keymap::Keystroke;
use crate::Event;

/// Converts a raw crossterm event into the shared [`Event`] vocabulary, or
/// `None` if the event has no warp equivalent.
pub fn crossterm_event_to_warp_event(event: CrosstermEvent) -> Option<Event> {
    match event {
        CrosstermEvent::Key(key_event) => key_event_to_warp_event(key_event),
        // TODO: Mouse events are not converted yet. TUI coordinates are integer
        // cell (row, column) pairs that need a dedicated representation before
        // they can be mapped into Warp's float-pixel Event system.
        CrosstermEvent::Mouse(_) => None,
        // TODO: FocusGained, FocusLost, and Paste have no Warp equivalents yet.
        // If these are needed in the future, consider adding matching Warp events.
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

#[cfg(test)]
#[path = "event_conversion_tests.rs"]
mod tests;
