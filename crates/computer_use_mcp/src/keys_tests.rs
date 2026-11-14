use computer_use::Key;

use super::*;

fn chord(text: &str, platform: KeyPlatform) -> Vec<Key> {
    parse_chord(text, platform, KeyDialect::Anthropic).expect("valid key combination")
}

fn chord_error(text: &str, platform: KeyPlatform) -> String {
    parse_chord(text, platform, KeyDialect::Anthropic)
        .expect_err("invalid key combination")
        .to_string()
}

#[test]
fn single_characters_are_char_keys() {
    assert_eq!(chord("a", KeyPlatform::Mac), vec![Key::Char('a')]);
    assert_eq!(chord("/", KeyPlatform::Windows), vec![Key::Char('/')]);
    assert_eq!(chord("é", KeyPlatform::Linux), vec![Key::Char('é')]);
}

#[test]
fn named_characters_are_char_keys() {
    assert_eq!(chord("leftbracket", KeyPlatform::Mac), vec![Key::Char('[')]);
    assert_eq!(
        chord("ctrl+PLUS", KeyPlatform::Linux),
        vec![Key::Keycode(0xffe3), Key::Char('+')]
    );
}

#[test]
fn chords_use_platform_keycodes() {
    assert_eq!(
        chord("cmd+shift+t", KeyPlatform::Mac),
        vec![Key::Keycode(0x37), Key::Keycode(0x38), Key::Char('t')]
    );
    assert_eq!(
        chord("cmd+shift+t", KeyPlatform::Linux),
        vec![Key::Keycode(0xffeb), Key::Keycode(0xffe1), Key::Char('t')]
    );
    assert_eq!(
        chord("ctrl+shift+t", KeyPlatform::Windows),
        vec![Key::Keycode(0x11), Key::Keycode(0x10), Key::Char('t')]
    );
}

#[test]
fn names_ignore_case_whitespace_and_empty_segments() {
    assert_eq!(
        chord(" Ctrl + Return ", KeyPlatform::Mac),
        vec![Key::Keycode(0x3B), Key::Keycode(0x24)]
    );
    assert_eq!(
        chord("cmd++c", KeyPlatform::Mac),
        vec![Key::Keycode(0x37), Key::Char('c')]
    );
}

#[test]
fn function_keys_map_per_platform() {
    assert_eq!(chord("f1", KeyPlatform::Mac), vec![Key::Keycode(0x7A)]);
    assert_eq!(chord("F12", KeyPlatform::Mac), vec![Key::Keycode(0x6F)]);
    assert_eq!(chord("f20", KeyPlatform::Mac), vec![Key::Keycode(0x5A)]);
    assert_eq!(chord("f1", KeyPlatform::Linux), vec![Key::Keycode(0xffbe)]);
    assert_eq!(chord("f20", KeyPlatform::Linux), vec![Key::Keycode(0xffd1)]);
    assert_eq!(chord("f1", KeyPlatform::Windows), vec![Key::Keycode(0x70)]);
    assert_eq!(chord("f20", KeyPlatform::Windows), vec![Key::Keycode(0x83)]);
    for invalid in ["f0", "f21", "f01"] {
        assert!(chord_error(invalid, KeyPlatform::Mac).contains("is not a valid key"));
    }
}

#[test]
fn fn_key_only_exists_on_mac() {
    assert_eq!(chord("fn", KeyPlatform::Mac), vec![Key::Keycode(0x3F)]);
    for platform in [KeyPlatform::Linux, KeyPlatform::Windows] {
        assert!(chord_error("fn", platform).contains("cannot be pressed on this platform"));
    }
}

#[test]
fn delete_depends_on_dialect() {
    let delete = |dialect| parse_chord("delete", KeyPlatform::Mac, dialect).unwrap();
    assert_eq!(delete(KeyDialect::Anthropic), vec![Key::Keycode(0x33)]);
    assert_eq!(delete(KeyDialect::OpenAi), vec![Key::Keycode(0x75)]);
}

#[test]
fn dom_style_names_are_accepted() {
    assert_eq!(
        chord("META+ARROWLEFT", KeyPlatform::Mac),
        vec![Key::Keycode(0x37), Key::Keycode(0x7B)]
    );
}

#[test]
fn every_canonical_name_is_valid() {
    for name in CANONICAL_KEY_NAMES {
        assert!(
            parse_chord(name, KeyPlatform::Mac, KeyDialect::Anthropic).is_ok(),
            "{name} should be valid on macOS"
        );
        if *name != "fn" {
            assert!(
                parse_chord(name, KeyPlatform::Linux, KeyDialect::Anthropic).is_ok(),
                "{name} should be valid on Linux"
            );
        }
    }
}

#[test]
fn invalid_keys_explain_valid_names() {
    let error = chord_error("BackSpace BackSpace", KeyPlatform::Mac);
    assert!(error.contains("\"BackSpace BackSpace\" is not a valid key"));
    assert!(error.contains("forward_delete"));
    assert!(error.contains("write \"plus\" for \"+\""));
}

#[test]
fn chords_without_keys_are_rejected() {
    for text in ["", "+", " + "] {
        assert!(chord_error(text, KeyPlatform::Mac).contains("no keys were given"));
    }
}

#[test]
fn return_key_matches_platform() {
    assert_eq!(return_key(KeyPlatform::Mac), Key::Keycode(0x24));
    assert_eq!(return_key(KeyPlatform::Linux), Key::Keycode(0xff0d));
    assert_eq!(return_key(KeyPlatform::Windows), Key::Keycode(0x0D));
}
