use warpui_core::keymap::Keystroke;

use super::{ctrl_letter_c0, fallback_key_bytes};

fn ctrl(key: &str) -> Keystroke {
    Keystroke {
        ctrl: true,
        alt: false,
        shift: false,
        cmd: false,
        meta: false,
        key: key.to_owned(),
    }
}

#[test]
fn printable_keys_forward_their_utf8_bytes() {
    assert_eq!(fallback_key_bytes("a", "a"), Some(b"a".to_vec()));
    assert_eq!(fallback_key_bytes("A", "A"), Some(b"A".to_vec()));
    assert_eq!(fallback_key_bytes(" ", " "), Some(b" ".to_vec()));
    // A non-ASCII grapheme forwards its full UTF-8 encoding.
    assert_eq!(fallback_key_bytes("é", "é"), Some("é".as_bytes().to_vec()));
}

#[test]
fn named_control_keys_map_to_c0_bytes() {
    // Escape is the important one: it lets the user leave insert mode in a
    // full-screen editor. Enter/tab/backspace round out the common set.
    assert_eq!(fallback_key_bytes("escape", ""), Some(vec![0x1b]));
    assert_eq!(fallback_key_bytes("enter", ""), Some(vec![b'\r']));
    assert_eq!(fallback_key_bytes("\t", ""), Some(vec![b'\t']));
    assert_eq!(fallback_key_bytes("backspace", ""), Some(vec![0x7f]));
}

#[test]
fn unmapped_key_without_chars_sends_nothing() {
    // Keys the encoder handles (arrows, fn keys) never reach this fallback;
    // anything else with no text produces no PTY bytes.
    assert_eq!(fallback_key_bytes("f5", ""), None);
    assert_eq!(fallback_key_bytes("insert", ""), None);
}

#[test]
fn ctrl_letters_map_to_c0_control_bytes() {
    // Ctrl+A..Ctrl+Z must forward as 0x01..0x1A, not the printable letter
    // the TUI conversion leaves in `chars`.
    assert_eq!(ctrl_letter_c0(&ctrl("a")), Some(vec![0x01]));
    // Ctrl+C is forwarded to the app (0x03), not intercepted by the TUI.
    assert_eq!(ctrl_letter_c0(&ctrl("c")), Some(vec![0x03]));
    assert_eq!(ctrl_letter_c0(&ctrl("d")), Some(vec![0x04]));
    assert_eq!(ctrl_letter_c0(&ctrl("z")), Some(vec![0x1a]));
    // Shifted letter (Ctrl+Shift+A) folds onto the same control byte.
    assert_eq!(ctrl_letter_c0(&ctrl("A")), Some(vec![0x01]));
}

#[test]
fn ctrl_non_letters_and_plain_keys_do_not_map() {
    assert_eq!(ctrl_letter_c0(&ctrl("1")), None);
    assert_eq!(ctrl_letter_c0(&ctrl("enter")), None);
    let mut plain = ctrl("a");
    plain.ctrl = false;
    assert_eq!(ctrl_letter_c0(&plain), None);
}
