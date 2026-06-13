use crossterm::event::{
    Event as CrosstermEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton,
    MouseEvent, MouseEventKind,
};

use super::crossterm_event_to_warp_event;
use crate::geometry::vector::vec2f;
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
        KeyCode::Modifier(crossterm::event::ModifierKeyCode::LeftControl),
        KeyModifiers::empty(),
    );
    assert!(crossterm_event_to_warp_event(CrosstermEvent::Key(event)).is_none());
}

#[test]
fn left_mouse_down_maps_to_left_mouse_down_at_position() {
    let event = CrosstermEvent::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 3,
        row: 4,
        modifiers: KeyModifiers::empty(),
    });
    let Some(Event::LeftMouseDown { position, .. }) = crossterm_event_to_warp_event(event) else {
        panic!("expected LeftMouseDown");
    };
    assert_eq!(position, vec2f(3.0, 4.0));
}

#[test]
fn scroll_up_and_down_map_to_vertical_scroll_wheel() {
    let up = CrosstermEvent::Mouse(MouseEvent {
        kind: MouseEventKind::ScrollUp,
        column: 0,
        row: 0,
        modifiers: KeyModifiers::empty(),
    });
    let Some(Event::ScrollWheel { delta, .. }) = crossterm_event_to_warp_event(up) else {
        panic!("expected ScrollWheel");
    };
    assert_eq!(delta, vec2f(0.0, 1.0));

    let down = CrosstermEvent::Mouse(MouseEvent {
        kind: MouseEventKind::ScrollDown,
        column: 0,
        row: 0,
        modifiers: KeyModifiers::empty(),
    });
    let Some(Event::ScrollWheel { delta, .. }) = crossterm_event_to_warp_event(down) else {
        panic!("expected ScrollWheel");
    };
    assert_eq!(delta, vec2f(0.0, -1.0));
}

#[test]
fn resize_and_focus_events_are_ignored() {
    assert!(crossterm_event_to_warp_event(CrosstermEvent::Resize(80, 24)).is_none());
    assert!(crossterm_event_to_warp_event(CrosstermEvent::FocusGained).is_none());
}
