use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers, ModifierKeyCode};

use super::translate_key;
use crate::event::Event;
use crate::keymap::Keystroke;

/// Builds a key-press event (the common case from a terminal).
fn press(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, modifiers)
}

/// Builds an unmodified key-release event.
fn release(code: KeyCode) -> KeyEvent {
    KeyEvent::new_with_kind(code, KeyModifiers::NONE, KeyEventKind::Release)
}

#[test]
fn plain_character_is_typed_text() {
    match translate_key(press(KeyCode::Char('a'), KeyModifiers::NONE)) {
        Some(Event::TypedCharacters { chars }) => assert_eq!(chars, "a"),
        other => panic!("expected TypedCharacters, got {other:?}"),
    }
}

#[test]
fn shifted_letter_is_typed_uppercase_text() {
    // Terminals report the already-cased character; shift alone is still text.
    match translate_key(press(KeyCode::Char('A'), KeyModifiers::SHIFT)) {
        Some(Event::TypedCharacters { chars }) => assert_eq!(chars, "A"),
        other => panic!("expected TypedCharacters, got {other:?}"),
    }
}

#[test]
fn space_is_typed_text() {
    match translate_key(press(KeyCode::Char(' '), KeyModifiers::NONE)) {
        Some(Event::TypedCharacters { chars }) => assert_eq!(chars, " "),
        other => panic!("expected TypedCharacters, got {other:?}"),
    }
}

#[test]
fn ctrl_c_is_keydown_keystroke() {
    match translate_key(press(KeyCode::Char('c'), KeyModifiers::CONTROL)) {
        Some(Event::KeyDown {
            keystroke, chars, ..
        }) => {
            assert_eq!(
                keystroke,
                Keystroke {
                    ctrl: true,
                    alt: false,
                    shift: false,
                    cmd: false,
                    meta: false,
                    key: "c".to_string(),
                }
            );
            // Modified keys carry no inline text; the keymap matches the keystroke.
            assert!(chars.is_empty());
        }
        other => panic!("expected KeyDown, got {other:?}"),
    }
}

#[test]
fn ctrl_shift_c_normalizes_to_uppercase_key() {
    let modifiers = KeyModifiers::CONTROL | KeyModifiers::SHIFT;
    match translate_key(press(KeyCode::Char('c'), modifiers)) {
        Some(Event::KeyDown { keystroke, .. }) => {
            assert!(keystroke.ctrl);
            assert!(keystroke.shift);
            assert_eq!(keystroke.key, "C");
        }
        other => panic!("expected KeyDown, got {other:?}"),
    }
}

#[test]
fn named_keys_map_to_expected_key_strings() {
    let cases = [
        (KeyCode::Enter, "enter"),
        (KeyCode::Backspace, "backspace"),
        (KeyCode::Tab, "tab"),
        (KeyCode::Esc, "escape"),
        (KeyCode::Left, "left"),
        (KeyCode::Right, "right"),
        (KeyCode::Up, "up"),
        (KeyCode::Down, "down"),
        (KeyCode::Home, "home"),
        (KeyCode::End, "end"),
        (KeyCode::PageUp, "pageup"),
        (KeyCode::PageDown, "pagedown"),
        (KeyCode::Delete, "delete"),
        (KeyCode::Insert, "insert"),
    ];
    for (code, name) in cases {
        match translate_key(press(code, KeyModifiers::NONE)) {
            Some(Event::KeyDown { keystroke, .. }) => {
                assert_eq!(keystroke.key, name);
                assert!(keystroke.is_unmodified());
            }
            other => panic!("expected KeyDown for {code:?}, got {other:?}"),
        }
    }
}

#[test]
fn function_keys_are_named() {
    match translate_key(press(KeyCode::F(5), KeyModifiers::NONE)) {
        Some(Event::KeyDown { keystroke, .. }) => assert_eq!(keystroke.key, "f5"),
        other => panic!("expected KeyDown, got {other:?}"),
    }
}

#[test]
fn back_tab_is_shift_tab() {
    match translate_key(press(KeyCode::BackTab, KeyModifiers::NONE)) {
        Some(Event::KeyDown { keystroke, .. }) => {
            assert_eq!(keystroke.key, "tab");
            assert!(keystroke.is_shift_tab());
        }
        other => panic!("expected KeyDown, got {other:?}"),
    }
}

#[test]
fn alt_character_is_keystroke_not_text() {
    match translate_key(press(KeyCode::Char('b'), KeyModifiers::ALT)) {
        Some(Event::KeyDown { keystroke, .. }) => {
            assert!(keystroke.alt);
            assert_eq!(keystroke.key, "b");
        }
        other => panic!("expected KeyDown, got {other:?}"),
    }
}

#[test]
fn super_modifier_maps_to_cmd() {
    match translate_key(press(KeyCode::Char('a'), KeyModifiers::SUPER)) {
        Some(Event::KeyDown { keystroke, .. }) => {
            assert!(keystroke.cmd);
            assert_eq!(keystroke.key, "a");
        }
        other => panic!("expected KeyDown, got {other:?}"),
    }
}

#[test]
fn key_release_is_ignored() {
    assert!(translate_key(release(KeyCode::Char('a'))).is_none());
}

#[test]
fn lone_modifier_press_is_ignored() {
    let code = KeyCode::Modifier(ModifierKeyCode::LeftControl);
    assert!(translate_key(press(code, KeyModifiers::NONE)).is_none());
}
