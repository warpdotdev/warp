use ratatui::crossterm::event::{
    Event as CrosstermEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
};

use super::crossterm_event_to_warp_event;
use crate::keymap::Keystroke;
use crate::Event;

fn key(code: KeyCode, modifiers: KeyModifiers) -> Option<Event> {
    crossterm_event_to_warp_event(CrosstermEvent::Key(KeyEvent::new(code, modifiers)))
}

fn keystroke(code: KeyCode, modifiers: KeyModifiers) -> Keystroke {
    match key(code, modifiers) {
        Some(Event::KeyDown { keystroke, .. }) => keystroke,
        other => panic!("expected a KeyDown, got {other:?}"),
    }
}

#[test]
fn printable_char_maps_to_lowercase_key_and_chars() {
    let Some(Event::KeyDown {
        keystroke, chars, ..
    }) = key(KeyCode::Char('a'), KeyModifiers::empty())
    else {
        panic!("expected KeyDown");
    };
    assert_eq!(keystroke.key, "a");
    assert_eq!(chars, "a");
    assert!(!keystroke.ctrl && !keystroke.alt && !keystroke.shift);
}

#[test]
fn enter_and_escape_map_to_named_keys() {
    assert_eq!(
        keystroke(KeyCode::Enter, KeyModifiers::empty()).key,
        "enter"
    );
    assert_eq!(keystroke(KeyCode::Esc, KeyModifiers::empty()).key, "escape");
}

#[test]
fn arrow_keys_map_to_direction_names() {
    assert_eq!(keystroke(KeyCode::Left, KeyModifiers::empty()).key, "left");
    assert_eq!(
        keystroke(KeyCode::Right, KeyModifiers::empty()).key,
        "right"
    );
    assert_eq!(keystroke(KeyCode::Up, KeyModifiers::empty()).key, "up");
    assert_eq!(keystroke(KeyCode::Down, KeyModifiers::empty()).key, "down");
}

#[test]
fn ctrl_modifier_is_carried_into_keystroke() {
    let keystroke = keystroke(KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert!(keystroke.ctrl, "ctrl modifier should be set");
    assert_eq!(keystroke.key, "c");
}

#[test]
fn shifted_char_preserves_case() {
    let keystroke = keystroke(KeyCode::Char('A'), KeyModifiers::SHIFT);
    assert!(keystroke.shift);
    assert_eq!(keystroke.key, "A");
}

#[test]
fn non_press_key_events_are_ignored() {
    let mut event = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::empty());
    event.kind = KeyEventKind::Release;
    assert!(crossterm_event_to_warp_event(CrosstermEvent::Key(event)).is_none());
}

#[test]
fn pure_modifier_keys_have_no_warp_equivalent() {
    let event = KeyEvent::new(
        KeyCode::Modifier(ratatui::crossterm::event::ModifierKeyCode::LeftControl),
        KeyModifiers::empty(),
    );
    assert!(crossterm_event_to_warp_event(CrosstermEvent::Key(event)).is_none());
}

#[test]
fn resize_and_focus_events_are_ignored() {
    assert!(crossterm_event_to_warp_event(CrosstermEvent::Resize(80, 24)).is_none());
    assert!(crossterm_event_to_warp_event(CrosstermEvent::FocusGained).is_none());
}
