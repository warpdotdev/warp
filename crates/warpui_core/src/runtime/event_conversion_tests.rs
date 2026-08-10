use std::time::Duration;

use instant::Instant;
#[cfg(not(windows))]
use ratatui::crossterm::event::KeyEventState;
use ratatui::crossterm::event::{
    Event as CrosstermEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, ModifierKeyCode,
    MouseButton, MouseEvent, MouseEventKind,
};

use super::{ClickTracker, crossterm_event_to_tui_event, produced_chars};
use crate::elements::tui::{TuiEvent, TuiPoint};
use crate::event::KeyState;
use crate::keymap::Keystroke;
use crate::platform::keyboard::KeyCode as PhysicalKeyCode;

fn key(code: KeyCode, modifiers: KeyModifiers) -> Option<TuiEvent> {
    crossterm_event_to_tui_event(CrosstermEvent::Key(KeyEvent::new(code, modifiers)))
}
#[cfg(not(windows))]
fn key_with_state(
    code: KeyCode,
    modifiers: KeyModifiers,
    state: KeyEventState,
) -> Option<TuiEvent> {
    crossterm_event_to_tui_event(CrosstermEvent::Key(KeyEvent::new_with_kind_and_state(
        code,
        modifiers,
        KeyEventKind::Press,
        state,
    )))
}
#[cfg(not(windows))]
#[test]
fn caps_lock_and_shift_apply_xor_casing_to_ascii_text() {
    let cases = [
        (
            KeyCode::Char('a'),
            KeyModifiers::empty(),
            KeyEventState::CAPS_LOCK,
            "A",
        ),
        (
            KeyCode::Char('A'),
            KeyModifiers::SHIFT,
            KeyEventState::CAPS_LOCK,
            "a",
        ),
    ];
    for (code, modifiers, state, expected_chars) in cases {
        let Some(TuiEvent::KeyDown {
            keystroke, chars, ..
        }) = key_with_state(code, modifiers, state)
        else {
            panic!("expected KeyDown");
        };
        assert_eq!(chars, expected_chars);
        assert_eq!(keystroke.shift, modifiers.contains(KeyModifiers::SHIFT));
    }
}

#[test]
fn backend_produced_text_is_not_recased() {
    let cases = [
        (KeyEvent::new(KeyCode::Char('A'), KeyModifiers::SHIFT), "A"),
        (KeyEvent::new(KeyCode::Char('a'), KeyModifiers::SHIFT), "a"),
        (
            KeyEvent::new(KeyCode::Char('A'), KeyModifiers::empty()),
            "A",
        ),
    ];
    for (event, expected_chars) in cases {
        assert_eq!(produced_chars(event, true), expected_chars);
    }
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
fn tab_maps_to_the_canonical_keybinding_name() {
    assert_eq!(keystroke(KeyCode::Tab, KeyModifiers::empty()).key, "tab");
    let back_tab = keystroke(KeyCode::BackTab, KeyModifiers::SHIFT);
    assert_eq!(back_tab.key, "tab");
    assert!(back_tab.shift);
}

#[test]
fn ctrl_modifier_is_carried_into_keystroke() {
    let keystroke = keystroke(KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert!(keystroke.ctrl, "ctrl modifier should be set");
    assert_eq!(keystroke.key, "c");
}
#[test]
fn legacy_safe_ctrl_r_chord_is_preserved_for_keybindings() {
    let keystroke = keystroke(KeyCode::Char('r'), KeyModifiers::CONTROL);
    assert!(keystroke.ctrl);
    assert_eq!(keystroke.key, "r");
    assert_eq!(keystroke.normalized(), "ctrl-r");
}

#[test]
fn non_shift_modifiers_are_preserved_from_the_key_event() {
    let keystroke = keystroke(
        KeyCode::Char('c'),
        KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER | KeyModifiers::META,
    );
    assert!(keystroke.ctrl);
    assert!(keystroke.alt);
    assert!(keystroke.cmd);
    assert!(keystroke.meta);
    assert!(!keystroke.shift);
}

#[test]
fn shifted_letter_preserves_case_and_base_key() {
    // Backends disagree on which character a shifted letter reports: terminals
    // speaking the Kitty protocol send the unshifted codepoint and leave the
    // casing to the decoder, while the Windows console sends the character the
    // layout already produced. Both must decode to the same keystroke.
    let reported = if cfg!(windows) { 'A' } else { 'a' };
    // Ctrl must not change the naming: `ctrl-shift-A` is the form the TUI's
    // select-all, copy, and redo bindings are registered under.
    for modifiers in [
        KeyModifiers::SHIFT,
        KeyModifiers::SHIFT | KeyModifiers::CONTROL,
    ] {
        let Some(TuiEvent::KeyDown {
            keystroke,
            chars,
            details,
            ..
        }) = key(KeyCode::Char(reported), modifiers)
        else {
            panic!("expected KeyDown");
        };
        assert!(keystroke.shift);
        assert_eq!(keystroke.ctrl, modifiers.contains(KeyModifiers::CONTROL));
        assert_eq!(keystroke.key, "A");
        assert_eq!(chars, "A");
        assert_eq!(details.key_without_modifiers.as_deref(), Some("a"));
    }
}

#[test]
fn alternate_key_without_shift_preserves_produced_character_semantics() {
    for (char, expected_key) in [('A', "a"), ('!', "!")] {
        let Some(TuiEvent::KeyDown {
            keystroke, chars, ..
        }) = key(KeyCode::Char(char), KeyModifiers::CONTROL)
        else {
            panic!("expected KeyDown");
        };
        assert!(keystroke.ctrl);
        assert!(!keystroke.shift);
        assert_eq!(keystroke.key, expected_key);
        assert_eq!(chars, char.to_string());
    }
}

#[test]
fn shifted_punctuation_uses_the_terminal_character() {
    let Some(TuiEvent::KeyDown {
        keystroke, chars, ..
    }) = key(KeyCode::Char('!'), KeyModifiers::SHIFT)
    else {
        panic!("expected KeyDown");
    };
    assert_eq!(keystroke.key, "!");
    assert!(keystroke.shift);
    assert_eq!(chars, "!");
}

#[test]
fn repeats_remain_key_down_events_and_non_modifier_releases_are_ignored() {
    let repeat = KeyEvent::new_with_kind(
        KeyCode::Char('a'),
        KeyModifiers::empty(),
        KeyEventKind::Repeat,
    );
    assert!(matches!(
        crossterm_event_to_tui_event(CrosstermEvent::Key(repeat)),
        Some(TuiEvent::KeyDown { .. })
    ));

    let release = KeyEvent::new_with_kind(
        KeyCode::Char('a'),
        KeyModifiers::empty(),
        KeyEventKind::Release,
    );
    assert!(crossterm_event_to_tui_event(CrosstermEvent::Key(release)).is_none());
}
#[test]
fn paste_preserves_the_complete_payload() {
    let payload = "USER:\nhello\n\nAGENT:\nHi!\n";
    let Some(TuiEvent::Paste { text }) =
        crossterm_event_to_tui_event(CrosstermEvent::Paste(payload.to_owned()))
    else {
        panic!("expected Paste");
    };
    assert_eq!(text, payload);
}

#[test]
fn modifier_keys_map_to_modifier_changed_events() {
    let cases = [
        (ModifierKeyCode::LeftControl, PhysicalKeyCode::ControlLeft),
        (ModifierKeyCode::RightControl, PhysicalKeyCode::ControlRight),
        (ModifierKeyCode::LeftAlt, PhysicalKeyCode::AltLeft),
        (ModifierKeyCode::RightAlt, PhysicalKeyCode::AltRight),
        (ModifierKeyCode::LeftShift, PhysicalKeyCode::ShiftLeft),
        (ModifierKeyCode::RightShift, PhysicalKeyCode::ShiftRight),
        (ModifierKeyCode::LeftSuper, PhysicalKeyCode::SuperLeft),
        (ModifierKeyCode::RightSuper, PhysicalKeyCode::SuperRight),
    ];

    for (modifier, expected_key_code) in cases {
        for (kind, expected_state) in [
            (KeyEventKind::Press, KeyState::Pressed),
            (KeyEventKind::Release, KeyState::Released),
        ] {
            let event =
                KeyEvent::new_with_kind(KeyCode::Modifier(modifier), KeyModifiers::empty(), kind);
            let Some(TuiEvent::ModifierKeyChanged { key_code, state }) =
                crossterm_event_to_tui_event(CrosstermEvent::Key(event))
            else {
                panic!("expected ModifierKeyChanged");
            };
            assert_eq!(key_code, expected_key_code);
            assert!(matches!(
                (state, expected_state),
                (KeyState::Pressed, KeyState::Pressed) | (KeyState::Released, KeyState::Released)
            ));
        }

        let repeat = KeyEvent::new_with_kind(
            KeyCode::Modifier(modifier),
            KeyModifiers::empty(),
            KeyEventKind::Repeat,
        );
        assert!(crossterm_event_to_tui_event(CrosstermEvent::Key(repeat)).is_none());
    }
}

#[test]
fn unsupported_modifier_keys_have_no_tui_equivalent() {
    for modifier in [
        ModifierKeyCode::LeftHyper,
        ModifierKeyCode::RightHyper,
        ModifierKeyCode::LeftMeta,
        ModifierKeyCode::RightMeta,
        ModifierKeyCode::IsoLevel3Shift,
        ModifierKeyCode::IsoLevel5Shift,
    ] {
        let event = KeyEvent::new(KeyCode::Modifier(modifier), KeyModifiers::empty());
        assert!(crossterm_event_to_tui_event(CrosstermEvent::Key(event)).is_none());
    }
}

#[test]
fn resize_is_ignored_and_focus_events_are_preserved() {
    assert!(crossterm_event_to_tui_event(CrosstermEvent::Resize(80, 24)).is_none());
    assert!(matches!(
        crossterm_event_to_tui_event(CrosstermEvent::FocusGained),
        Some(TuiEvent::FocusGained)
    ));
    assert!(matches!(
        crossterm_event_to_tui_event(CrosstermEvent::FocusLost),
        Some(TuiEvent::FocusLost)
    ));
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
    assert!(
        mouse(
            MouseEventKind::Up(MouseButton::Right),
            KeyModifiers::empty()
        )
        .is_none()
    );
    assert!(
        mouse(
            MouseEventKind::Up(MouseButton::Middle),
            KeyModifiers::empty()
        )
        .is_none()
    );
    assert!(
        mouse(
            MouseEventKind::Drag(MouseButton::Right),
            KeyModifiers::empty()
        )
        .is_none()
    );
    assert!(
        mouse(
            MouseEventKind::Drag(MouseButton::Middle),
            KeyModifiers::empty()
        )
        .is_none()
    );
}

/// Builds a `button` mouse-down at `(x, y)` via the real conversion (so it
/// starts with `click_count: 1`).
fn down_at(button: MouseButton, x: u16, y: u16) -> TuiEvent {
    crossterm_event_to_tui_event(CrosstermEvent::Mouse(MouseEvent {
        kind: MouseEventKind::Down(button),
        column: x,
        row: y,
        modifiers: KeyModifiers::empty(),
    }))
    .expect("mouse down should convert")
}

/// The synthesized click count carried by a mouse-down event.
fn count_of(event: &TuiEvent) -> u32 {
    match event {
        TuiEvent::LeftMouseDown { click_count, .. }
        | TuiEvent::MiddleMouseDown { click_count, .. }
        | TuiEvent::RightMouseDown { click_count, .. } => *click_count,
        other => panic!("expected a mouse-down, got {other:?}"),
    }
}

/// Annotates a `button` mouse-down at `(x, y)` / `now` through `tracker` and
/// returns the synthesized click count.
fn annotate_down(
    tracker: &mut ClickTracker,
    button: MouseButton,
    x: u16,
    y: u16,
    now: Instant,
) -> u32 {
    let mut event = down_at(button, x, y);
    tracker.annotate(&mut event, now);
    count_of(&event)
}

/// Convenience wrapper for the common left-button case.
fn click_count(tracker: &mut ClickTracker, x: u16, y: u16, now: Instant) -> u32 {
    annotate_down(tracker, MouseButton::Left, x, y, now)
}

#[test]
fn click_count_escalates_then_wraps_for_fast_clicks_on_same_cell() {
    let mut tracker = ClickTracker::default();
    let t = Instant::now();
    assert_eq!(click_count(&mut tracker, 5, 2, t), 1);
    assert_eq!(
        click_count(&mut tracker, 5, 2, t + Duration::from_millis(100)),
        2
    );
    assert_eq!(
        click_count(&mut tracker, 5, 2, t + Duration::from_millis(200)),
        3
    );
    // A fourth fast click wraps back to a single click.
    assert_eq!(
        click_count(&mut tracker, 5, 2, t + Duration::from_millis(300)),
        1
    );
}

#[test]
fn slow_second_click_resets_to_single() {
    let mut tracker = ClickTracker::default();
    let t = Instant::now();
    assert_eq!(click_count(&mut tracker, 5, 2, t), 1);
    // Past the multi-click interval, so the next press is a fresh single click.
    assert_eq!(
        click_count(&mut tracker, 5, 2, t + Duration::from_millis(600)),
        1
    );
}

#[test]
fn click_on_distant_cell_resets_to_single() {
    let mut tracker = ClickTracker::default();
    let t = Instant::now();
    assert_eq!(click_count(&mut tracker, 5, 2, t), 1);
    assert_eq!(
        click_count(&mut tracker, 20, 2, t + Duration::from_millis(100)),
        1
    );
}

#[test]
fn click_within_one_cell_counts_as_multi_click() {
    let mut tracker = ClickTracker::default();
    let t = Instant::now();
    assert_eq!(click_count(&mut tracker, 5, 2, t), 1);
    // One cell of jitter between presses is tolerated.
    assert_eq!(
        click_count(&mut tracker, 6, 2, t + Duration::from_millis(100)),
        2
    );
}

#[test]
fn right_and_middle_clicks_escalate_like_left() {
    let mut tracker = ClickTracker::default();
    let t = Instant::now();
    assert_eq!(annotate_down(&mut tracker, MouseButton::Right, 5, 2, t), 1);
    assert_eq!(
        annotate_down(
            &mut tracker,
            MouseButton::Right,
            5,
            2,
            t + Duration::from_millis(100)
        ),
        2
    );

    let mut tracker = ClickTracker::default();
    assert_eq!(annotate_down(&mut tracker, MouseButton::Middle, 5, 2, t), 1);
    assert_eq!(
        annotate_down(
            &mut tracker,
            MouseButton::Middle,
            5,
            2,
            t + Duration::from_millis(100)
        ),
        2
    );
}

#[test]
fn switching_button_resets_click_count() {
    let mut tracker = ClickTracker::default();
    let t = Instant::now();
    // A left press then a quick right press on the same cell are two separate
    // single clicks, not a double-click.
    assert_eq!(annotate_down(&mut tracker, MouseButton::Left, 5, 2, t), 1);
    assert_eq!(
        annotate_down(
            &mut tracker,
            MouseButton::Right,
            5,
            2,
            t + Duration::from_millis(50)
        ),
        1
    );
}

#[test]
fn annotate_leaves_non_button_events_untouched() {
    let mut tracker = ClickTracker::default();
    // A scroll-wheel event carries no click count; annotate must be a no-op.
    let mut event = crossterm_event_to_tui_event(CrosstermEvent::Mouse(MouseEvent {
        kind: MouseEventKind::ScrollUp,
        column: 5,
        row: 2,
        modifiers: KeyModifiers::empty(),
    }))
    .expect("scroll up should convert");
    tracker.annotate(&mut event, Instant::now());
    assert!(matches!(event, TuiEvent::ScrollWheel { .. }));
}
