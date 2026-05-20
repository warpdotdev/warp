use warpui::keymap::Keystroke;
use warpui::platform::OperatingSystem;

use crate::model::{
    indexing::Point,
    mouse::{MouseAction, MouseButton, MouseState},
    TermMode,
};

use super::*;

fn validate_keystroke_test_cases<T: ModeProvider>(
    test_cases: &[(Keystroke, Vec<u8>)],
    mock_terminal_model: &T,
) {
    for (key, expected_result) in test_cases.iter() {
        let result = KeystrokeWithDetails {
            keystroke: key,
            key_without_modifiers: None,
            chars: None,
        }
        .to_escape_sequence(mock_terminal_model);
        log::debug!("Key: {}", key.key);
        assert_eq!(Some(expected_result), result.as_ref());
    }
}

fn validate_mouse_test_cases<T: ModeProvider>(
    test_cases: &[(MouseState, Vec<u8>)],
    mock_terminal_model: &T,
) {
    for (mouse_state, expected_result) in test_cases.iter() {
        let result = mouse_state.to_escape_sequence(mock_terminal_model);
        log::debug!(
            "Mouse action: {:#?}, Mouse button: {:#?}, Point: {:#?}",
            mouse_state.action(),
            mouse_state.button(),
            mouse_state.maybe_point()
        );
        assert_eq!(Some(expected_result), result.as_ref());
    }
}

#[test]
fn test_keystroke_to_c0_control_code() {
    // Expected mapping taken from the VT220 spec
    // [here](https://vt100.net/docs/vt220-rm/chapter3.html#S3.2.5), table 3.2.5.
    let test_cases: &[(Keystroke, Vec<u8>)] = &[
        (Keystroke::parse("ctrl- ").unwrap(), vec![C0::NUL]),
        (Keystroke::parse("ctrl-2").unwrap(), vec![C0::NUL]),
        (Keystroke::parse("ctrl-3").unwrap(), vec![C0::ESC]),
        (Keystroke::parse("ctrl-4").unwrap(), vec![C0::FS]),
        (Keystroke::parse("ctrl-5").unwrap(), vec![C0::GS]),
        (Keystroke::parse("ctrl-6").unwrap(), vec![C0::RS]),
        (Keystroke::parse("ctrl-7").unwrap(), vec![C0::US]),
        (Keystroke::parse("ctrl-8").unwrap(), vec![C0::DEL]),
    ];

    let terminal_model_mock = TerminalModelMock::new();
    validate_keystroke_test_cases(test_cases, &terminal_model_mock);
}

#[test]
fn test_mouse_actions_to_escape_sequence() {
    // Validating we produce the correct escape sequences.
    let test_cases: &[(MouseState, Vec<u8>)] = &[
        (
            MouseState::new(MouseButton::Right, MouseAction::Pressed, Default::default())
                .set_point(Point::new(10, 10)),
            // [<2;11;11M
            vec![
                C0::ESC,
                b'[',
                b'<',
                b'2',
                b';',
                b'1',
                b'1',
                b';',
                b'1',
                b'1',
                b'M',
            ],
        ),
        (
            MouseState::new(MouseButton::Left, MouseAction::Released, Default::default())
                .set_point(Point::new(5, 5)),
            // [<0;6;6m
            vec![C0::ESC, b'[', b'<', b'0', b';', b'6', b';', b'6', b'm'],
        ),
        (
            MouseState::new(
                MouseButton::LeftDrag,
                MouseAction::Pressed,
                Default::default(),
            )
            .set_point(Point::new(5, 5)),
            // [<32;6;6M
            vec![
                C0::ESC,
                b'[',
                b'<',
                b'3',
                b'2',
                b';',
                b'6',
                b';',
                b'6',
                b'M',
            ],
        ),
        (
            MouseState::new(MouseButton::Move, MouseAction::Pressed, Default::default())
                .set_point(Point::new(5, 5)),
            // [<35;6;6M
            vec![
                C0::ESC,
                b'[',
                b'<',
                b'3',
                b'5',
                b';',
                b'6',
                b';',
                b'6',
                b'M',
            ],
        ),
        (
            MouseState::new(
                MouseButton::Wheel,
                MouseAction::Scrolled { delta: 2 },
                Default::default(),
            )
            .set_point(Point::new(5, 5)),
            // [<64;6;6M[<64;6;6M
            // Repeated twice since the scroll action was for 2 lines
            vec![
                C0::ESC,
                b'[',
                b'<',
                b'6',
                b'4',
                b';',
                b'6',
                b';',
                b'6',
                b'M',
                C0::ESC,
                b'[',
                b'<',
                b'6',
                b'4',
                b';',
                b'6',
                b';',
                b'6',
                b'M',
            ],
        ),
    ];

    let terminal_model_mock = TerminalModelMock::new();
    validate_mouse_test_cases(test_cases, &terminal_model_mock);
}

#[test]
fn test_cursor_movement_keystroke_without_modifier_to_escape_sequence() {
    // Expected mapping taken from the xterm spec
    // [here](https://www.xfree86.org/current/ctlseqs.html).
    let test_cases: &[(Keystroke, Vec<u8>)] = &[
        (Keystroke::parse("up").unwrap(), vec![C0::ESC, b'[', b'A']),
        (Keystroke::parse("down").unwrap(), vec![C0::ESC, b'[', b'B']),
        (
            Keystroke::parse("right").unwrap(),
            vec![C0::ESC, b'[', b'C'],
        ),
        (Keystroke::parse("left").unwrap(), vec![C0::ESC, b'[', b'D']),
        (Keystroke::parse("home").unwrap(), vec![C0::ESC, b'[', b'H']),
        (Keystroke::parse("end").unwrap(), vec![C0::ESC, b'[', b'F']),
    ];

    let mut terminal_model_mock = TerminalModelMock::new();
    validate_keystroke_test_cases(test_cases, &terminal_model_mock);

    let app_cursor_term_mode_test_cases: &[(Keystroke, Vec<u8>)] = &[
        (Keystroke::parse("up").unwrap(), vec![C0::ESC, b'O', b'A']),
        (Keystroke::parse("down").unwrap(), vec![C0::ESC, b'O', b'B']),
        (
            Keystroke::parse("right").unwrap(),
            vec![C0::ESC, b'O', b'C'],
        ),
        (Keystroke::parse("left").unwrap(), vec![C0::ESC, b'O', b'D']),
        (Keystroke::parse("home").unwrap(), vec![C0::ESC, b'O', b'H']),
        (Keystroke::parse("end").unwrap(), vec![C0::ESC, b'O', b'F']),
    ];

    // with cursor keys mode
    terminal_model_mock.set_mode(TermMode::APP_CURSOR);
    for (key, expected_result) in app_cursor_term_mode_test_cases.iter() {
        let result = KeystrokeWithDetails {
            keystroke: key,
            key_without_modifiers: None,
            chars: None,
        }
        .to_escape_sequence(&terminal_model_mock);
        log::debug!("Key: {}", key.key);
        assert_eq!(Some(expected_result), result.as_ref());
    }
    validate_keystroke_test_cases(app_cursor_term_mode_test_cases, &terminal_model_mock);
}

#[test]
fn test_cursor_movement_keystroke_with_modifier_to_escape_sequence() {
    // Expected mapping taken from the xterm spec
    // [here](https://www.xfree86.org/current/ctlseqs.html).
    let test_cases: &[(Keystroke, Vec<u8>)] = &[
        (
            Keystroke::parse("shift-up").unwrap(),
            vec![C0::ESC, b'[', b'1', b';', b'2', b'A'],
        ),
        (
            Keystroke::parse("alt-down").unwrap(),
            vec![C0::ESC, b'[', b'1', b';', b'3', b'B'],
        ),
        (
            Keystroke::parse("shift-alt-right").unwrap(),
            vec![C0::ESC, b'[', b'1', b';', b'4', b'C'],
        ),
        (
            Keystroke::parse("ctrl-left").unwrap(),
            vec![C0::ESC, b'[', b'1', b';', b'5', b'D'],
        ),
        (
            Keystroke::parse("shift-ctrl-home").unwrap(),
            vec![C0::ESC, b'[', b'1', b';', b'6', b'H'],
        ),
        (
            Keystroke::parse("ctrl-alt-end").unwrap(),
            vec![C0::ESC, b'[', b'1', b';', b'7', b'F'],
        ),
    ];

    let mut terminal_model_mock = TerminalModelMock::new();
    validate_keystroke_test_cases(test_cases, &terminal_model_mock);

    let app_cursor_term_mode_test_cases: &[(Keystroke, Vec<u8>)] = &[
        (
            Keystroke::parse("ctrl-shift-alt-up").unwrap(),
            vec![C0::ESC, b'[', b'1', b';', b'8', b'A'],
        ),
        (
            Keystroke::parse("meta-down").unwrap(),
            vec![C0::ESC, b'[', b'1', b';', b'3', b'B'],
        ),
        (
            Keystroke::parse("shift-right").unwrap(),
            vec![C0::ESC, b'[', b'1', b';', b'2', b'C'],
        ),
        (
            Keystroke::parse("alt-left").unwrap(),
            vec![C0::ESC, b'[', b'1', b';', b'3', b'D'],
        ),
        (
            Keystroke::parse("shift-alt-home").unwrap(),
            vec![C0::ESC, b'[', b'1', b';', b'4', b'H'],
        ),
        (
            Keystroke::parse("ctrl-end").unwrap(),
            vec![C0::ESC, b'[', b'1', b';', b'5', b'F'],
        ),
    ];

    // with cursor keys mode
    terminal_model_mock.set_mode(TermMode::APP_CURSOR);

    validate_keystroke_test_cases(app_cursor_term_mode_test_cases, &terminal_model_mock);
}

#[test]
fn test_fn_keystroke_without_modifier_to_escape_sequence() {
    // Expected mapping taken from the xterm spec
    // [here](https://www.xfree86.org/current/ctlseqs.html#PC-Style%20Function%20Keys), under
    // the section titled 'PC-Style Function Keys'.
    let test_cases: &[(Keystroke, Vec<u8>)] = &[
        (Keystroke::parse("f1").unwrap(), vec![C0::ESC, b'O', b'P']),
        (Keystroke::parse("f2").unwrap(), vec![C0::ESC, b'O', b'Q']),
        (Keystroke::parse("f3").unwrap(), vec![C0::ESC, b'O', b'R']),
        (Keystroke::parse("f4").unwrap(), vec![C0::ESC, b'O', b'S']),
        (
            Keystroke::parse("f5").unwrap(),
            vec![C0::ESC, b'[', b'1', b'5', b'~'],
        ),
        (
            Keystroke::parse("f6").unwrap(),
            vec![C0::ESC, b'[', b'1', b'7', b'~'],
        ),
        (
            Keystroke::parse("f7").unwrap(),
            vec![C0::ESC, b'[', b'1', b'8', b'~'],
        ),
        (
            Keystroke::parse("f8").unwrap(),
            vec![C0::ESC, b'[', b'1', b'9', b'~'],
        ),
        (
            Keystroke::parse("f9").unwrap(),
            vec![C0::ESC, b'[', b'2', b'0', b'~'],
        ),
        (
            Keystroke::parse("f10").unwrap(),
            vec![C0::ESC, b'[', b'2', b'1', b'~'],
        ),
        (
            Keystroke::parse("f11").unwrap(),
            vec![C0::ESC, b'[', b'2', b'3', b'~'],
        ),
        (
            Keystroke::parse("f12").unwrap(),
            vec![C0::ESC, b'[', b'2', b'4', b'~'],
        ),
        (
            Keystroke::parse("f13").unwrap(),
            vec![C0::ESC, b'[', b'2', b'5', b'~'],
        ),
        (
            Keystroke::parse("f14").unwrap(),
            vec![C0::ESC, b'[', b'2', b'6', b'~'],
        ),
        (
            Keystroke::parse("f15").unwrap(),
            vec![C0::ESC, b'[', b'2', b'8', b'~'],
        ),
        (
            Keystroke::parse("f16").unwrap(),
            vec![C0::ESC, b'[', b'2', b'9', b'~'],
        ),
        (
            Keystroke::parse("f17").unwrap(),
            vec![C0::ESC, b'[', b'3', b'1', b'~'],
        ),
        (
            Keystroke::parse("f18").unwrap(),
            vec![C0::ESC, b'[', b'3', b'2', b'~'],
        ),
        (
            Keystroke::parse("f19").unwrap(),
            vec![C0::ESC, b'[', b'3', b'3', b'~'],
        ),
        (
            Keystroke::parse("f20").unwrap(),
            vec![C0::ESC, b'[', b'3', b'4', b'~'],
        ),
    ];

    let terminal_model_mock = TerminalModelMock::new();
    validate_keystroke_test_cases(test_cases, &terminal_model_mock);
}

#[test]
fn test_fn_keystroke_with_modifier_to_escape_sequence() {
    let test_cases: &[(Keystroke, Vec<u8>)] = &[
        (
            Keystroke::parse("shift-f1").unwrap(),
            vec![C0::ESC, b'[', b'1', b';', b'2', b'P'],
        ),
        (
            Keystroke::parse("alt-f2").unwrap(),
            vec![C0::ESC, b'[', b'1', b';', b'3', b'Q'],
        ),
        (
            Keystroke::parse("shift-alt-f3").unwrap(),
            vec![C0::ESC, b'[', b'1', b';', b'4', b'R'],
        ),
        (
            Keystroke::parse("ctrl-f4").unwrap(),
            vec![C0::ESC, b'[', b'1', b';', b'5', b'S'],
        ),
        (
            Keystroke::parse("ctrl-shift-f5").unwrap(),
            vec![C0::ESC, b'[', b'1', b'5', b';', b'6', b'~'],
        ),
        (
            Keystroke::parse("ctrl-alt-f6").unwrap(),
            vec![C0::ESC, b'[', b'1', b'7', b';', b'7', b'~'],
        ),
        (
            Keystroke::parse("ctrl-shift-alt-f7").unwrap(),
            vec![C0::ESC, b'[', b'1', b'8', b';', b'8', b'~'],
        ),
        (
            Keystroke::parse("meta-f8").unwrap(),
            vec![C0::ESC, b'[', b'1', b'9', b';', b'3', b'~'],
        ),
        (
            Keystroke::parse("shift-f9").unwrap(),
            vec![C0::ESC, b'[', b'2', b'0', b';', b'2', b'~'],
        ),
        (
            Keystroke::parse("alt-f10").unwrap(),
            vec![C0::ESC, b'[', b'2', b'1', b';', b'3', b'~'],
        ),
        (
            Keystroke::parse("shift-alt-f11").unwrap(),
            vec![C0::ESC, b'[', b'2', b'3', b';', b'4', b'~'],
        ),
        (
            Keystroke::parse("ctrl-f12").unwrap(),
            vec![C0::ESC, b'[', b'2', b'4', b';', b'5', b'~'],
        ),
        (
            Keystroke::parse("ctrl-shift-f13").unwrap(),
            vec![C0::ESC, b'[', b'2', b'5', b';', b'6', b'~'],
        ),
        (
            Keystroke::parse("ctrl-alt-f14").unwrap(),
            vec![C0::ESC, b'[', b'2', b'6', b';', b'7', b'~'],
        ),
        (
            Keystroke::parse("ctrl-shift-alt-f15").unwrap(),
            vec![C0::ESC, b'[', b'2', b'8', b';', b'8', b'~'],
        ),
        (
            Keystroke::parse("shift-f16").unwrap(),
            vec![C0::ESC, b'[', b'2', b'9', b';', b'2', b'~'],
        ),
        (
            Keystroke::parse("alt-f17").unwrap(),
            vec![C0::ESC, b'[', b'3', b'1', b';', b'3', b'~'],
        ),
        (
            Keystroke::parse("shift-alt-f18").unwrap(),
            vec![C0::ESC, b'[', b'3', b'2', b';', b'4', b'~'],
        ),
        (
            Keystroke::parse("ctrl-f19").unwrap(),
            vec![C0::ESC, b'[', b'3', b'3', b';', b'5', b'~'],
        ),
        (
            Keystroke::parse("ctrl-shift-f20").unwrap(),
            vec![C0::ESC, b'[', b'3', b'4', b';', b'6', b'~'],
        ),
    ];

    let terminal_model_mock = TerminalModelMock::new();
    validate_keystroke_test_cases(test_cases, &terminal_model_mock);
}

#[test]
fn test_meta_keystroke_to_escape_sequence() {
    fn metaify(keystroke: &str) -> String {
        if OperatingSystem::get().is_mac() {
            format!("meta-{keystroke}")
        } else {
            format!("alt-{keystroke}")
        }
    }

    let test_cases: &[(Keystroke, Vec<u8>)] = &[
        (Keystroke::parse(metaify("a")).unwrap(), vec![C0::ESC, b'a']),
        (Keystroke::parse(metaify("1")).unwrap(), vec![C0::ESC, b'1']),
        (
            Keystroke::parse(metaify("'")).unwrap(),
            vec![C0::ESC, b'\''],
        ),
    ];

    let terminal_model_mock = TerminalModelMock::new();
    validate_keystroke_test_cases(test_cases, &terminal_model_mock);
}

#[test]
fn test_unmatched_keystroke_does_not_yield_escape_sequence() {
    let test_cases: &[Keystroke] = &[
        Keystroke::parse("a").unwrap(),
        Keystroke::parse("1").unwrap(),
        Keystroke::parse("'").unwrap(),
    ];

    let terminal_model_mock = TerminalModelMock::new();
    for key in test_cases.iter() {
        let result = KeystrokeWithDetails {
            keystroke: key,
            key_without_modifiers: None,
            chars: None,
        }
        .to_escape_sequence(&terminal_model_mock);
        assert_eq!(result, None);
    }
}

struct TerminalModelMock {
    term_mode: TermMode,
}

impl TerminalModelMock {
    fn new() -> Self {
        Self {
            term_mode: TermMode::default(),
        }
    }

    fn set_mode(&mut self, mode: TermMode) {
        self.term_mode |= mode;
    }
}

impl ModeProvider for TerminalModelMock {
    fn is_term_mode_set(&self, mode: TermMode) -> bool {
        self.term_mode.intersects(mode)
    }
}

/// Creates a mock with KEYBOARD_REPORT_ALL_AS_ESCAPE so all keys use CSI u format.
fn mock_with_all_keys_as_escape() -> TerminalModelMock {
    let mut mock = TerminalModelMock::new();
    mock.set_mode(TermMode::KEYBOARD_REPORT_ALL_AS_ESCAPE);
    mock
}

/// Creates a mock with only KEYBOARD_DISAMBIGUATE_ESCAPE — CSI u only for ambiguous/modified keys.
fn mock_with_disambiguate_only() -> TerminalModelMock {
    let mut mock = TerminalModelMock::new();
    mock.set_mode(TermMode::KEYBOARD_DISAMBIGUATE_ESCAPE);
    mock
}

/// Creates a mock with REPORT_ALL_KEYS_AS_ESCAPE + REPORT_ALTERNATE_KEYS.
fn mock_with_alternate_keys() -> TerminalModelMock {
    let mut mock = TerminalModelMock::new();
    mock.set_mode(TermMode::KEYBOARD_REPORT_ALL_AS_ESCAPE);
    mock.set_mode(TermMode::KEYBOARD_REPORT_ALTERNATE_KEYS);
    mock
}

/// Creates a mock with REPORT_ALL_KEYS_AS_ESCAPE + REPORT_ASSOCIATED_TEXT.
fn mock_with_associated_text() -> TerminalModelMock {
    let mut mock = TerminalModelMock::new();
    mock.set_mode(TermMode::KEYBOARD_REPORT_ALL_AS_ESCAPE);
    mock.set_mode(TermMode::KEYBOARD_REPORT_ASSOCIATED_TEXT);
    mock
}

/// Creates a mock with REPORT_ALL_KEYS_AS_ESCAPE + REPORT_EVENT_TYPES.
fn mock_with_event_types() -> TerminalModelMock {
    let mut mock = TerminalModelMock::new();
    mock.set_mode(TermMode::KEYBOARD_REPORT_ALL_AS_ESCAPE);
    mock.set_mode(TermMode::KEYBOARD_REPORT_EVENT_TYPES);
    mock
}

/// Creates a mock with all enhancement flags enabled.
fn mock_with_all_flags() -> TerminalModelMock {
    let mut mock = TerminalModelMock::new();
    mock.set_mode(TermMode::KEYBOARD_REPORT_ALL_AS_ESCAPE);
    mock.set_mode(TermMode::KEYBOARD_REPORT_EVENT_TYPES);
    mock.set_mode(TermMode::KEYBOARD_REPORT_ALTERNATE_KEYS);
    mock.set_mode(TermMode::KEYBOARD_REPORT_ASSOCIATED_TEXT);
    mock
}

#[test]
fn test_keyboard_enhancement_basic_keys() {
    // Test basic keys with REPORT_ALL_KEYS_AS_ESCAPE flag (flag 8)
    // This flag makes ALL keys use CSI u format
    // Note: Per Kitty protocol, ;1 is omitted when there are no modifiers
    let test_cases: &[(Keystroke, Vec<u8>)] = &[
        // Enter (key code 13)
        (Keystroke::parse("enter").unwrap(), b"\x1b[13u".to_vec()),
        (
            Keystroke::parse("shift-enter").unwrap(),
            b"\x1b[13;2u".to_vec(),
        ),
        (
            Keystroke::parse("ctrl-enter").unwrap(),
            b"\x1b[13;5u".to_vec(),
        ),
        (
            Keystroke::parse("alt-enter").unwrap(),
            b"\x1b[13;3u".to_vec(),
        ),
        // Tab (key code 9)
        (Keystroke::parse("tab").unwrap(), b"\x1b[9u".to_vec()),
        (
            Keystroke::parse("shift-tab").unwrap(),
            b"\x1b[9;2u".to_vec(),
        ),
        // Escape (key code 27)
        (Keystroke::parse("escape").unwrap(), b"\x1b[27u".to_vec()),
        (
            Keystroke::parse("shift-escape").unwrap(),
            b"\x1b[27;2u".to_vec(),
        ),
        // Backspace (key code 127)
        (
            Keystroke::parse("backspace").unwrap(),
            b"\x1b[127u".to_vec(),
        ),
        (
            Keystroke::parse("ctrl-backspace").unwrap(),
            b"\x1b[127;5u".to_vec(),
        ),
        // Space (key code 32)
        (Keystroke::parse(" ").unwrap(), b"\x1b[32u".to_vec()),
        (Keystroke::parse("ctrl- ").unwrap(), b"\x1b[32;5u".to_vec()),
    ];

    let terminal_model_mock = mock_with_all_keys_as_escape();
    validate_keystroke_test_cases(test_cases, &terminal_model_mock);
}

#[test]
fn test_keyboard_enhancement_function_keys() {
    // Test F13-F20 with keyboard enhancement protocol
    // Note: Only F13-F20 are currently parseable by Keystroke::parse()
    // F21-F35 support exists in keystroke_to_csi_u() but requires runtime key events
    // Per Kitty protocol, ;1 is omitted when there are no modifiers
    let test_cases: &[(Keystroke, Vec<u8>)] = &[
        // F13-F20 (codes 57376-57383)
        (Keystroke::parse("f13").unwrap(), b"\x1b[57376u".to_vec()),
        (
            Keystroke::parse("shift-f14").unwrap(),
            b"\x1b[57377;2u".to_vec(),
        ),
        (Keystroke::parse("f15").unwrap(), b"\x1b[57378u".to_vec()),
        (
            Keystroke::parse("ctrl-f16").unwrap(),
            b"\x1b[57379;5u".to_vec(),
        ),
        (Keystroke::parse("f20").unwrap(), b"\x1b[57383u".to_vec()),
    ];

    let terminal_model_mock = mock_with_all_keys_as_escape();
    validate_keystroke_test_cases(test_cases, &terminal_model_mock);
}

#[test]
fn test_keyboard_enhancement_disambiguate_only() {
    // Test with only DISAMBIGUATE_ESC_CODES flag (flag 1)
    // Only ambiguous keys (like Escape) and modified keys should use CSI u
    // Unmodified non-ambiguous keys should fall back to legacy encoding
    let terminal_model_mock = mock_with_disambiguate_only();

    // Escape is ambiguous - should always use CSI u with flag 1
    let escape = Keystroke::parse("escape").unwrap();
    assert_eq!(
        KeystrokeWithDetails {
            keystroke: &escape,
            key_without_modifiers: None,
            chars: None,
        }
        .to_escape_sequence(&terminal_model_mock),
        Some(b"\x1b[27u".to_vec())
    );

    // Shift+Enter is ambiguous: legacy encoding sends the same bytes for Enter
    // and Shift+Enter, so CSI u is needed to preserve the Shift modifier.
    let shift_enter = Keystroke::parse("shift-enter").unwrap();
    assert_eq!(
        KeystrokeWithDetails {
            keystroke: &shift_enter,
            key_without_modifiers: None,
            chars: None,
        }
        .to_escape_sequence(&terminal_model_mock),
        Some(b"\x1b[13;2u".to_vec())
    );

    // Shift+Tab is also ambiguous for the same reason.
    let shift_tab = Keystroke::parse("shift-tab").unwrap();
    assert_eq!(
        KeystrokeWithDetails {
            keystroke: &shift_tab,
            key_without_modifiers: None,
            chars: None,
        }
        .to_escape_sequence(&terminal_model_mock),
        Some(b"\x1b[9;2u".to_vec())
    );

    // But Shift+a is NOT ambiguous — Shift changes the character to 'A'.
    let shift_a = Keystroke::parse("shift-A").unwrap();
    assert_eq!(
        KeystrokeWithDetails {
            keystroke: &shift_a,
            key_without_modifiers: None,
            chars: None,
        }
        .to_escape_sequence(&terminal_model_mock),
        None
    );

    // Ctrl+a has modifiers - should use CSI u.
    let ctrl_a = Keystroke::parse("ctrl-a").unwrap();
    assert_eq!(
        KeystrokeWithDetails {
            keystroke: &ctrl_a,
            key_without_modifiers: None,
            chars: None,
        }
        .to_escape_sequence(&terminal_model_mock),
        Some(b"\x1b[97;5u".to_vec())
    );
    // Plain Enter without modifiers - NOT ambiguous, should not use CSI u.
    let enter = Keystroke::parse("enter").unwrap();
    assert_eq!(
        KeystrokeWithDetails {
            keystroke: &enter,
            key_without_modifiers: None,
            chars: None,
        }
        .to_escape_sequence(&terminal_model_mock),
        None
    );
}

#[test]
fn test_keyboard_enhancement_unshifted_keycode_for_shifted_printables() {
    let terminal_model_mock = mock_with_all_keys_as_escape();

    // Shifted symbols should use the unshifted keycode in CSI u.
    // The key_without_modifiers is provided by the platform (e.g., winit or macOS UCKeyTranslate).
    assert_eq!(
        KeystrokeWithDetails {
            keystroke: &Keystroke::parse("shift-@").unwrap(),
            key_without_modifiers: Some("2"),
            chars: None,
        }
        .to_escape_sequence(&terminal_model_mock),
        Some(b"\x1b[50;2u".to_vec())
    );
    assert_eq!(
        KeystrokeWithDetails {
            keystroke: &Keystroke::parse("shift-%").unwrap(),
            key_without_modifiers: Some("5"),
            chars: None,
        }
        .to_escape_sequence(&terminal_model_mock),
        Some(b"\x1b[53;2u".to_vec())
    );
    // Uppercase letters are lowercased universally, so platform info is not required.
    assert_eq!(
        KeystrokeWithDetails {
            keystroke: &Keystroke::parse("shift-A").unwrap(),
            key_without_modifiers: Some("a"),
            chars: None,
        }
        .to_escape_sequence(&terminal_model_mock),
        Some(b"\x1b[97;2u".to_vec())
    );
    // Verify the letter case also works without platform info.
    assert_eq!(
        KeystrokeWithDetails {
            keystroke: &Keystroke::parse("shift-A").unwrap(),
            key_without_modifiers: None,
            chars: None,
        }
        .to_escape_sequence(&terminal_model_mock),
        Some(b"\x1b[97;2u".to_vec())
    );
}

#[test]
fn test_keyboard_enhancement_mac_option_without_meta_mapping_is_not_disambiguated() {
    if !OperatingSystem::get().is_mac() {
        return;
    }

    let terminal_model_mock = mock_with_disambiguate_only();
    // On macOS with Option-as-Meta disabled, Alt should not force CSI u in disambiguate-only mode.
    let alt_a = Keystroke::parse("alt-a").unwrap();
    assert_eq!(
        KeystrokeWithDetails {
            keystroke: &alt_a,
            key_without_modifiers: None,
            chars: None,
        }
        .to_escape_sequence(&terminal_model_mock),
        None
    );
}

#[test]
fn test_keyboard_enhancement_alternate_keys() {
    // With REPORT_ALTERNATE_KEYS (flag 4): shifted keys include the shifted code after a colon.
    let mock = mock_with_alternate_keys();

    // shift+a: base=97 (a), alternate=65 (A) → CSI 97:65;2u
    let shift_a = Keystroke::parse("shift-A").unwrap();
    assert_eq!(
        KeystrokeWithDetails {
            keystroke: &shift_a,
            key_without_modifiers: Some("a"),
            chars: None,
        }
        .to_escape_sequence(&mock),
        Some(b"\x1b[97:65;2u".to_vec())
    );

    // shift+@ (shift+2): base=50 (2), alternate=64 (@) → CSI 50:64;2u
    let shift_at = Keystroke::parse("shift-@").unwrap();
    assert_eq!(
        KeystrokeWithDetails {
            keystroke: &shift_at,
            key_without_modifiers: Some("2"),
            chars: None,
        }
        .to_escape_sequence(&mock),
        Some(b"\x1b[50:64;2u".to_vec())
    );

    // shift+% (shift+5): base=53 (5), alternate=37 (%) → CSI 53:37;2u
    let shift_pct = Keystroke::parse("shift-%").unwrap();
    assert_eq!(
        KeystrokeWithDetails {
            keystroke: &shift_pct,
            key_without_modifiers: Some("5"),
            chars: None,
        }
        .to_escape_sequence(&mock),
        Some(b"\x1b[53:37;2u".to_vec())
    );

    // Without shift, no alternate key is reported.
    let a = Keystroke::parse("a").unwrap();
    assert_eq!(
        KeystrokeWithDetails {
            keystroke: &a,
            key_without_modifiers: None,
            chars: None,
        }
        .to_escape_sequence(&mock),
        Some(b"\x1b[97u".to_vec())
    );

    // ctrl+a: no shift → no alternate key even with flag 4.
    let ctrl_a = Keystroke::parse("ctrl-a").unwrap();
    assert_eq!(
        KeystrokeWithDetails {
            keystroke: &ctrl_a,
            key_without_modifiers: None,
            chars: None,
        }
        .to_escape_sequence(&mock),
        Some(b"\x1b[97;5u".to_vec())
    );

    // Special keys (enter) don't have alternate keys.
    let shift_enter = Keystroke::parse("shift-enter").unwrap();
    assert_eq!(
        KeystrokeWithDetails {
            keystroke: &shift_enter,
            key_without_modifiers: None,
            chars: None,
        }
        .to_escape_sequence(&mock),
        Some(b"\x1b[13;2u".to_vec())
    );
}

#[test]
fn test_keyboard_enhancement_associated_text() {
    // With REPORT_ASSOCIATED_TEXT (flag 16): text codepoints appended as third parameter.
    let mock = mock_with_associated_text();

    // Plain 'a': OS text is "a" (97) → CSI 97;1;97u (modifiers=1 must be present for text field)
    let a = Keystroke::parse("a").unwrap();
    assert_eq!(
        KeystrokeWithDetails {
            keystroke: &a,
            key_without_modifiers: None,
            chars: Some("a"),
        }
        .to_escape_sequence(&mock),
        Some(b"\x1b[97;1;97u".to_vec())
    );

    // shift+A: OS text is "A" (65) → CSI 97;2;65u
    let shift_a = Keystroke::parse("shift-A").unwrap();
    assert_eq!(
        KeystrokeWithDetails {
            keystroke: &shift_a,
            key_without_modifiers: Some("a"),
            chars: Some("A"),
        }
        .to_escape_sequence(&mock),
        Some(b"\x1b[97;2;65u".to_vec())
    );

    // ctrl+a: OS text is control character "\x01" → filtered, no associated text → CSI 97;5u
    let ctrl_a = Keystroke::parse("ctrl-a").unwrap();
    assert_eq!(
        KeystrokeWithDetails {
            keystroke: &ctrl_a,
            key_without_modifiers: None,
            chars: Some("\x01"),
        }
        .to_escape_sequence(&mock),
        Some(b"\x1b[97;5u".to_vec())
    );

    // Enter: OS text is "\r" (control character) → no associated text → CSI 13u
    let enter = Keystroke::parse("enter").unwrap();
    assert_eq!(
        KeystrokeWithDetails {
            keystroke: &enter,
            key_without_modifiers: None,
            chars: Some("\r"),
        }
        .to_escape_sequence(&mock),
        Some(b"\x1b[13u".to_vec())
    );

    // Escape: OS text is "\x1b" (control character) → no associated text → CSI 27u
    let escape = Keystroke::parse("escape").unwrap();
    assert_eq!(
        KeystrokeWithDetails {
            keystroke: &escape,
            key_without_modifiers: None,
            chars: Some("\x1b"),
        }
        .to_escape_sequence(&mock),
        Some(b"\x1b[27u".to_vec())
    );
}

#[test]
fn test_keyboard_enhancement_alternate_keys_with_associated_text() {
    // Both flags 4 and 16 active simultaneously.
    let mock = mock_with_all_flags();

    // shift+A: alternate=65, text="A"=65 → CSI 97:65;2;65u (press event type omitted)
    let shift_a = Keystroke::parse("shift-A").unwrap();
    assert_eq!(
        KeystrokeWithDetails {
            keystroke: &shift_a,
            key_without_modifiers: Some("a"),
            chars: Some("A"),
        }
        .to_escape_sequence(&mock),
        Some(b"\x1b[97:65;2;65u".to_vec())
    );

    // shift+@ (shift+2): alternate=64, text="@"=64 → CSI 50:64;2;64u
    let shift_at = Keystroke::parse("shift-@").unwrap();
    assert_eq!(
        KeystrokeWithDetails {
            keystroke: &shift_at,
            key_without_modifiers: Some("2"),
            chars: Some("@"),
        }
        .to_escape_sequence(&mock),
        Some(b"\x1b[50:64;2;64u".to_vec())
    );

    // Plain 'z': no alternate (no shift), text="z"=122 → CSI 122;1;122u
    let z = Keystroke::parse("z").unwrap();
    assert_eq!(
        KeystrokeWithDetails {
            keystroke: &z,
            key_without_modifiers: None,
            chars: Some("z"),
        }
        .to_escape_sequence(&mock),
        Some(b"\x1b[122;1;122u".to_vec())
    );
}

#[test]
fn test_keyboard_enhancement_event_types() {
    // With REPORT_EVENT_TYPES (flag 2): press is the default event type and is omitted.
    // This flag only changes behavior for repeat/release events (not yet handled for
    // regular keystrokes). Verify that enabling it doesn't change press encoding.
    let mock = mock_with_event_types();

    // Plain 'a': press is default, omitted → CSI 97u (same as without flag 2)
    let a = Keystroke::parse("a").unwrap();
    assert_eq!(
        KeystrokeWithDetails {
            keystroke: &a,
            key_without_modifiers: None,
            chars: None,
        }
        .to_escape_sequence(&mock),
        Some(b"\x1b[97u".to_vec())
    );

    // ctrl+a: modifiers=5 → CSI 97;5u
    let ctrl_a = Keystroke::parse("ctrl-a").unwrap();
    assert_eq!(
        KeystrokeWithDetails {
            keystroke: &ctrl_a,
            key_without_modifiers: None,
            chars: None,
        }
        .to_escape_sequence(&mock),
        Some(b"\x1b[97;5u".to_vec())
    );

    // Enter: no modifiers → CSI 13u
    let enter = Keystroke::parse("enter").unwrap();
    assert_eq!(
        KeystrokeWithDetails {
            keystroke: &enter,
            key_without_modifiers: None,
            chars: None,
        }
        .to_escape_sequence(&mock),
        Some(b"\x1b[13u".to_vec())
    );

    // shift+enter: modifiers=2 → CSI 13;2u
    let shift_enter = Keystroke::parse("shift-enter").unwrap();
    assert_eq!(
        KeystrokeWithDetails {
            keystroke: &shift_enter,
            key_without_modifiers: None,
            chars: None,
        }
        .to_escape_sequence(&mock),
        Some(b"\x1b[13;2u".to_vec())
    );
}
