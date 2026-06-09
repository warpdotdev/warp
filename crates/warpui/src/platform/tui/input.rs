//! Translation of crossterm terminal input events into WarpUI [`Event`]s.
//!
//! NOTE (contract): the TUI event loop calls [`translate_key`] for each
//! [`crossterm::event::Event::Key`] and dispatches the single returned [`Event`]
//! via `callbacks.for_window(window).dispatch_event(..)`. The function itself
//! decides which kind of event to emit, so the caller does not need to
//! synthesize one from the other:
//!
//! * A printable character with no command modifier (ctrl/alt/cmd/meta) is text
//!   input, emitted as [`Event::TypedCharacters`] so a focused text input
//!   inserts it directly. Shift is intentionally not treated as a command
//!   modifier — it is already baked into the produced character (`A`, `!`, ...).
//! * Every other key — the named special keys and any key combined with a
//!   command modifier — is emitted as [`Event::KeyDown`] carrying a
//!   [`Keystroke`] for the keymap/action system to match against bindings.
//!
//! This is a pure function with no global state.
//!
//! [`Event`]: crate::Event

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::event::{Event, KeyEventDetails};
use crate::keymap::Keystroke;

/// Translates a crossterm key event into a WarpUI [`Event`], if one applies.
///
/// Returns [`None`] for key-release events and for keys with no WarpUI
/// representation (lone modifier presses, media keys, lock keys, ...).
pub(super) fn translate_key(key: KeyEvent) -> Option<Event> {
    // Terminals speaking the Kitty keyboard protocol additionally report key
    // releases and auto-repeats. Treat presses and repeats as input (so held
    // keys repeat) and ignore releases, mirroring how the winit backend acts
    // only on `ElementState::Pressed`.
    if matches!(key.kind, KeyEventKind::Release) {
        return None;
    }

    let modifiers = key.modifiers;
    let ctrl = modifiers.contains(KeyModifiers::CONTROL);
    let alt = modifiers.contains(KeyModifiers::ALT);
    let mut shift = modifiers.contains(KeyModifiers::SHIFT);
    // WarpUI's `cmd` is the platform command/"super" key; `meta` covers the
    // remaining high-level modifiers a terminal might report.
    let cmd = modifiers.contains(KeyModifiers::SUPER);
    let meta = modifiers.contains(KeyModifiers::META) || modifiers.contains(KeyModifiers::HYPER);

    // A character combined with any of these is a shortcut, not text.
    let has_command_modifier = ctrl || alt || cmd || meta;

    let key_name = match key.code {
        // A printable character with no command modifier is text input: emit it
        // as `TypedCharacters` so a focused text input inserts it directly.
        KeyCode::Char(c) if !has_command_modifier => {
            return Some(Event::TypedCharacters {
                chars: c.to_string(),
            })
        }
        KeyCode::Char(c) => char_key_name(c, shift),
        // BackTab *is* shift-tab; force the shift modifier so it matches
        // `shift-tab` bindings regardless of what the terminal reported.
        KeyCode::BackTab => {
            shift = true;
            "tab".to_string()
        }
        KeyCode::Enter => "enter".to_string(),
        KeyCode::Backspace => "backspace".to_string(),
        KeyCode::Tab => "tab".to_string(),
        KeyCode::Esc => "escape".to_string(),
        KeyCode::Left => "left".to_string(),
        KeyCode::Right => "right".to_string(),
        KeyCode::Up => "up".to_string(),
        KeyCode::Down => "down".to_string(),
        KeyCode::Home => "home".to_string(),
        KeyCode::End => "end".to_string(),
        KeyCode::PageUp => "pageup".to_string(),
        KeyCode::PageDown => "pagedown".to_string(),
        KeyCode::Delete => "delete".to_string(),
        KeyCode::Insert => "insert".to_string(),
        KeyCode::F(n) => format!("f{n}"),
        // Keys with no WarpUI keystroke representation. These only reach us when
        // the Kitty keyboard protocol is enabled; without it they never arrive.
        KeyCode::Null
        | KeyCode::CapsLock
        | KeyCode::ScrollLock
        | KeyCode::NumLock
        | KeyCode::PrintScreen
        | KeyCode::Pause
        | KeyCode::Menu
        | KeyCode::KeypadBegin
        | KeyCode::Media(_)
        | KeyCode::Modifier(_) => return None,
    };

    let keystroke = Keystroke {
        ctrl,
        alt,
        shift,
        cmd,
        meta,
        key: key_name,
    };

    Some(Event::KeyDown {
        keystroke,
        // The keymap matches on `keystroke`, not `chars`; printable text is
        // delivered separately via `TypedCharacters`, so leaving this empty
        // avoids any double-insertion.
        chars: String::new(),
        details: KeyEventDetails {
            // A terminal can't tell us which physical alt was used, nor the
            // pre-modifier base key, so leave this metadata unset.
            left_alt: false,
            right_alt: false,
            key_without_modifiers: None,
        },
        is_composing: false,
    })
}

/// Returns the keystroke key string for a character key, normalizing ASCII
/// letters to the case implied by `shift` so they match how bindings are
/// written (lowercase without shift, uppercase with shift).
fn char_key_name(c: char, shift: bool) -> String {
    if c.is_ascii_alphabetic() {
        if shift {
            c.to_ascii_uppercase().to_string()
        } else {
            c.to_ascii_lowercase().to_string()
        }
    } else {
        c.to_string()
    }
}

#[cfg(test)]
#[path = "input_tests.rs"]
mod tests;
