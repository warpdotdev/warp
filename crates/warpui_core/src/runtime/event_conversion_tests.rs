use ratatui::crossterm::event::{
    Event as CrosstermEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton,
    MouseEvent, MouseEventKind,
};

use super::crossterm_event_to_tui_event;
use crate::elements::tui::{TuiEvent, TuiPoint};
use crate::keymap::Keystroke;

fn key(code: KeyCode, modifiers: KeyModifiers) -> Option<TuiEvent> {
    crossterm_event_to_tui_event(CrosstermEvent::Key(KeyEvent::new(code, modifiers)))
}

fn mouse(kind: MouseEventKind, modifiers: KeyModifiers) -> Option<TuiEvent> {
    crossterm_event_to_tui_event(CrosstermEvent::Mouse(MouseEvent {
        kind,
        column: 7,
        row: 3,
        modifiers,
    }))
}

fn keystroke(code: KeyCode, modifiers: KeyModifiers) -> Keystroke {
    match key(code, modifiers) {
        Some(TuiEvent::KeyDown { keystroke, .. }) => keystroke,
        other => panic!("expected a KeyDown, got {other:?}"),
    }
}

#[test]
fn printable_char_maps_to_lowercase_key_and_chars() {
    let Some(TuiEvent::KeyDown {
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
    assert!(crossterm_event_to_tui_event(CrosstermEvent::Key(event)).is_none());
}

#[test]
fn pure_modifier_keys_have_no_tui_equivalent() {
    let event = KeyEvent::new(
        KeyCode::Modifier(ratatui::crossterm::event::ModifierKeyCode::LeftControl),
        KeyModifiers::empty(),
    );
    assert!(crossterm_event_to_tui_event(CrosstermEvent::Key(event)).is_none());
}

#[test]
fn resize_and_focus_events_are_ignored() {
    assert!(crossterm_event_to_tui_event(CrosstermEvent::Resize(80, 24)).is_none());
    assert!(crossterm_event_to_tui_event(CrosstermEvent::FocusGained).is_none());
}

#[test]
fn vertical_mouse_wheel_maps_to_cell_position_and_scroll_delta() {
    let Some(TuiEvent::ScrollWheel {
        position,
        delta,
        precise,
        modifiers,
    }) = crossterm_event_to_tui_event(CrosstermEvent::Mouse(MouseEvent {
        kind: MouseEventKind::ScrollUp,
        column: 7,
        row: 3,
        modifiers: KeyModifiers::SHIFT,
    }))
    else {
        panic!("expected ScrollWheel");
    };

    assert_eq!(position, TuiPoint::new(7, 3));
    assert_eq!(delta, (0, 1));
    assert!(!precise);
    assert!(modifiers.shift);

    let Some(TuiEvent::ScrollWheel { delta, .. }) =
        crossterm_event_to_tui_event(CrosstermEvent::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 7,
            row: 3,
            modifiers: KeyModifiers::empty(),
        }))
    else {
        panic!("expected ScrollWheel");
    };
    assert_eq!(delta, (0, -1));
}

#[test]
fn mouse_buttons_map_to_tui_mouse_down_events() {
    let Some(TuiEvent::LeftMouseDown {
        position,
        modifiers,
        click_count,
        is_first_mouse,
    }) = mouse(
        MouseEventKind::Down(MouseButton::Left),
        KeyModifiers::CONTROL,
    )
    else {
        panic!("expected LeftMouseDown");
    };
    assert_eq!(position, TuiPoint::new(7, 3));
    assert!(modifiers.ctrl);
    assert_eq!(click_count, 1);
    assert!(!is_first_mouse);

    let Some(TuiEvent::MiddleMouseDown {
        position,
        modifiers,
        click_count,
    }) = mouse(
        MouseEventKind::Down(MouseButton::Middle),
        KeyModifiers::SUPER | KeyModifiers::SHIFT,
    )
    else {
        panic!("expected MiddleMouseDown");
    };
    assert_eq!(position, TuiPoint::new(7, 3));
    assert!(modifiers.cmd);
    assert!(modifiers.shift);
    assert_eq!(click_count, 1);

    let Some(TuiEvent::RightMouseDown {
        modifiers,
        click_count,
        ..
    }) = mouse(
        MouseEventKind::Down(MouseButton::Right),
        KeyModifiers::SHIFT,
    )
    else {
        panic!("expected RightMouseDown");
    };
    assert!(!modifiers.cmd);
    assert!(modifiers.shift);
    assert_eq!(click_count, 1);
}

#[test]
fn left_mouse_up_and_drag_map_to_tui_mouse_events() {
    let Some(TuiEvent::LeftMouseUp {
        position,
        modifiers,
    }) = mouse(MouseEventKind::Up(MouseButton::Left), KeyModifiers::ALT)
    else {
        panic!("expected LeftMouseUp");
    };
    assert_eq!(position, TuiPoint::new(7, 3));
    assert!(modifiers.alt);

    let Some(TuiEvent::LeftMouseDragged {
        position,
        modifiers,
    }) = mouse(
        MouseEventKind::Drag(MouseButton::Left),
        KeyModifiers::CONTROL,
    )
    else {
        panic!("expected LeftMouseDragged");
    };
    assert_eq!(position, TuiPoint::new(7, 3));
    assert!(modifiers.ctrl);
}

#[test]
fn mouse_moved_maps_to_tui_mouse_moved_event() {
    let Some(TuiEvent::MouseMoved {
        position,
        modifiers,
        is_synthetic,
    }) = mouse(
        MouseEventKind::Moved,
        KeyModifiers::SUPER | KeyModifiers::SHIFT,
    )
    else {
        panic!("expected MouseMoved");
    };

    assert_eq!(position, TuiPoint::new(7, 3));
    assert!(modifiers.cmd);
    assert!(modifiers.shift);
    assert!(!is_synthetic);
}

#[test]
fn unsupported_mouse_up_and_drag_buttons_are_ignored() {
    assert!(mouse(
        MouseEventKind::Up(MouseButton::Right),
        KeyModifiers::empty()
    )
    .is_none());
    assert!(mouse(
        MouseEventKind::Up(MouseButton::Middle),
        KeyModifiers::empty()
    )
    .is_none());
    assert!(mouse(
        MouseEventKind::Drag(MouseButton::Right),
        KeyModifiers::empty()
    )
    .is_none());
    assert!(mouse(
        MouseEventKind::Drag(MouseButton::Middle),
        KeyModifiers::empty()
    )
    .is_none());
}
