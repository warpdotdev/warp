//! Tests for the TUI terminal session: key encoding and input routing.

use std::sync::Arc;

use parking_lot::FairMutex;
use warpui::r#async::executor::Background;
use warpui_core::keymap::Keystroke;

use super::encode_keydown;
use crate::terminal::color;
use crate::terminal::event_listener::ChannelEventListener;
use crate::terminal::model::terminal_model::{TerminalInputState, TerminalModel};
use crate::terminal::model::test_utils::block_size;

/// Creates a bootstrapped `TerminalModel` wrapped in `FairMutex` for testing.
fn test_model() -> Arc<FairMutex<TerminalModel>> {
    let model = TerminalModel::new_for_test(
        block_size(),
        color::List::from(&color::Colors::default()),
        ChannelEventListener::new_for_test(),
        Arc::new(Background::default()),
        false,
        None,
        false,
        false,
        None,
    );
    Arc::new(FairMutex::new(model))
}

/// A freshly bootstrapped model should be in `InputEditor` state — keys go to
/// the input view, not the PTY.
#[test]
fn bootstrapped_model_is_in_input_editor_state() {
    let model = test_model();
    let state = model.lock().terminal_input_state();
    assert!(
        matches!(state, TerminalInputState::InputEditor),
        "expected InputEditor state"
    );
}

/// `InputEditor` and `NotBootstrapped` should NOT forward keys to the PTY.
/// `LongRunningCommand` and `AltScreen` SHOULD forward keys.
#[test]
fn input_routing_state_classification() {
    assert!(!should_forward_to_pty(TerminalInputState::InputEditor));
    assert!(!should_forward_to_pty(TerminalInputState::NotBootstrapped));
    assert!(should_forward_to_pty(
        TerminalInputState::LongRunningCommand
    ));
    assert!(should_forward_to_pty(TerminalInputState::AltScreen));
}

/// Mirrors the `TuiKeyInterceptor`'s dispatch decision.
fn should_forward_to_pty(state: TerminalInputState) -> bool {
    matches!(
        state,
        TerminalInputState::LongRunningCommand | TerminalInputState::AltScreen
    )
}

/// Arrow up should encode to an escape sequence (CSI or SS3 'A').
#[test]
fn encode_arrow_up_produces_escape_sequence() {
    let model = test_model();
    let keystroke = Keystroke {
        key: "up".to_string(),
        ..Default::default()
    };
    let bytes = encode_keydown(&keystroke, None, "", &model);
    assert!(bytes.is_some(), "arrow up should produce escape bytes");
    let bytes = bytes.unwrap();
    // CSI or SS3 sequence ending in 'A'
    assert!(
        bytes.ends_with(&[b'A']),
        "expected 'A' suffix, got {bytes:?}"
    );
}

/// Ctrl-C should encode to the ETX control code (0x03).
#[test]
fn encode_ctrl_c_produces_etx() {
    let model = test_model();
    let keystroke = Keystroke {
        ctrl: true,
        key: "c".to_string(),
        ..Default::default()
    };
    let bytes = encode_keydown(&keystroke, Some("c"), "c", &model);
    assert_eq!(bytes.as_deref(), Some(&[0x03][..]));
}

/// A plain printable character with no modifiers should return `None` from
/// `encode_keydown` (the caller writes `chars` as UTF-8).
#[test]
fn encode_plain_char_returns_none() {
    let model = test_model();
    let keystroke = Keystroke {
        key: "a".to_string(),
        ..Default::default()
    };
    let bytes = encode_keydown(&keystroke, Some("a"), "a", &model);
    assert!(bytes.is_none(), "plain 'a' should return None");
}

/// Backspace should encode to 0x7f (DEL) or 0x08 (BS).
#[test]
fn encode_backspace_produces_control_code() {
    let model = test_model();
    let keystroke = Keystroke {
        key: "backspace".to_string(),
        ..Default::default()
    };
    let bytes = encode_keydown(&keystroke, None, "", &model);
    assert!(bytes.is_some(), "backspace should produce bytes");
    let bytes = bytes.unwrap();
    assert!(
        bytes == [0x7f] || bytes == [0x08],
        "expected DEL (0x7f) or BS (0x08), got {bytes:?}"
    );
}

/// Enter should encode to carriage return (0x0d).
#[test]
fn encode_enter_produces_cr() {
    let model = test_model();
    let keystroke = Keystroke {
        key: "enter".to_string(),
        ..Default::default()
    };
    let bytes = encode_keydown(&keystroke, None, "", &model);
    assert_eq!(bytes.as_deref(), Some(&[0x0d][..]));
}

/// `resolve_shell_type` is a helper for session-level tests; this verifies the
/// test model is queryable without panicking.
#[test]
fn test_model_is_queryable() {
    let model = test_model();
    let _ = model.lock().terminal_input_state();
}
