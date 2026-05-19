use super::{convert_key, get_input_key, us_qwerty_fallback_for_chord};
#[cfg(windows)]
use super::rdp_key_from_text;
use winit::keyboard::{Key, Key::Character, KeyCode, NativeKeyCode, PhysicalKey, SmolStr};

#[test]
fn test_get_input_key() {
    // Tests all visible ASCII characters
    // TODO: it would be nice to test the following:
    // - non-Character keys (ex: named keys, dead keys)
    // - non-ascii characters to ensure shift behavior is appropriate
    for ascii_code in 32u8..127u8 {
        let input = ascii_code as char;
        let key = Character(SmolStr::from(input.to_string()));

        for shift in [false, true] {
            match get_input_key(&key, shift) {
                Character(new_value) => {
                    let new_char = new_value
                        .chars()
                        .next()
                        .expect("string should be non-empty");

                    let expected = match (input, shift) {
                        ('A'..='Z', false) => input
                            .to_lowercase()
                            .next()
                            .expect("string should be non-empty"),
                        // Case 2: a lower case letter when shift is true
                        // Should turn into upper case version
                        ('a'..='z', true) => input
                            .to_uppercase()
                            .next()
                            .expect("string should be non-empty"),
                        // Case 3: a character that should be unchanged by caps lock
                        // - An upper-case letter when shift is true
                        // - A lower-case letter when shift is false,
                        // - A non-alpha character
                        _ => input,
                    };
                    assert_eq!(
                        expected, new_char,
                        "Expected '{input}' -> '{expected}' when shift={shift}, but got '{new_char}'"
                    )
                }
                unexpected => {
                    panic!("Key '{key:?}' somehow became non-character {unexpected:?}")
                }
            }
        }
    }
}

#[test]
fn us_qwerty_fallback_maps_letters() {
    // Letters return lowercase regardless of shift; `get_input_key` applies the
    // uppercase transform downstream.
    let cases = [
        (KeyCode::KeyA, "a"),
        (KeyCode::KeyC, "c"),
        (KeyCode::KeyV, "v"),
        (KeyCode::KeyZ, "z"),
    ];
    for (code, expected) in cases {
        for shift in [false, true] {
            assert_eq!(
                us_qwerty_fallback_for_chord(&PhysicalKey::Code(code), shift),
                Some(expected),
                "expected {code:?} -> {expected} (shift={shift})",
            );
        }
    }
}

#[test]
fn us_qwerty_fallback_maps_digits_and_punctuation() {
    let cases = [
        (KeyCode::Digit0, "0"),
        (KeyCode::Digit9, "9"),
        (KeyCode::Minus, "-"),
        (KeyCode::Equal, "="),
        (KeyCode::Slash, "/"),
        (KeyCode::Backquote, "`"),
        (KeyCode::Semicolon, ";"),
        (KeyCode::Comma, ","),
    ];
    for (code, expected) in cases {
        assert_eq!(
            us_qwerty_fallback_for_chord(&PhysicalKey::Code(code), false),
            Some(expected),
            "expected {code:?} -> {expected}",
        );
    }
}

#[test]
fn us_qwerty_fallback_maps_shifted_digits_and_punctuation() {
    let cases = [
        (KeyCode::Digit1, "!"),
        (KeyCode::Digit2, "@"),
        (KeyCode::Digit6, "^"),
        (KeyCode::Digit9, "("),
        (KeyCode::Digit0, ")"),
        (KeyCode::Minus, "_"),
        (KeyCode::Equal, "+"),
        (KeyCode::BracketLeft, "{"),
        (KeyCode::BracketRight, "}"),
        (KeyCode::Backslash, "|"),
        (KeyCode::Semicolon, ":"),
        (KeyCode::Quote, "\""),
        (KeyCode::Comma, "<"),
        (KeyCode::Period, ">"),
        (KeyCode::Slash, "?"),
        (KeyCode::Backquote, "~"),
    ];
    for (code, expected) in cases {
        assert_eq!(
            us_qwerty_fallback_for_chord(&PhysicalKey::Code(code), true),
            Some(expected),
            "expected {code:?} + shift -> {expected}",
        );
    }
}

#[test]
fn us_qwerty_fallback_returns_none_for_unmapped_keys() {
    // Keys outside the chord-shortcut set should fall through so the original
    // logical_key is preserved.
    let unmapped = [
        KeyCode::F1,
        KeyCode::F13,
        KeyCode::AltLeft,
        KeyCode::ShiftRight,
        KeyCode::ControlLeft,
        KeyCode::Enter,
        KeyCode::Escape,
        KeyCode::ArrowUp,
        KeyCode::Tab,
    ];
    for code in unmapped {
        for shift in [false, true] {
            assert_eq!(
                us_qwerty_fallback_for_chord(&PhysicalKey::Code(code), shift),
                None,
                "{code:?} should not have a chord fallback (shift={shift})",
            );
        }
    }
}

#[test]
fn us_qwerty_fallback_returns_none_for_unidentified_physical_key() {
    let unidentified = PhysicalKey::Unidentified(NativeKeyCode::Unidentified);
    for shift in [false, true] {
        assert_eq!(
            us_qwerty_fallback_for_chord(&unidentified, shift),
            None,
            "unidentified key should not have a chord fallback (shift={shift})",
        );
    }
}

// Tests for RDP Unicode mode fallback: exercises the Unidentified+text path
// via rdp_key_from_text, which is the exact function called when both
// physical_key and logical_key are Unidentified(Windows(_)) and text is Some.
#[cfg(windows)]
#[test]
fn rdp_key_from_text_produces_lowercase_character() {
    // Verify that rdp_key_from_text lowercases the input so that get_input_key
    // can re-apply case from the shift modifier state downstream.
    let cases = [
        ("h", "h"),
        ("H", "h"), // Shift held — text arrives as uppercase, must be lowercased
        ("k", "k"),
        ("1", "1"),
        ("!", "!"), // punctuation — no case change
    ];
    for (text, expected) in cases {
        let key = rdp_key_from_text(text);
        let result = convert_key(key);
        assert_eq!(
            result.as_deref(),
            Some(expected),
            "rdp_key_from_text({text:?}) should produce key '{expected}'",
        );
    }
}

#[cfg(windows)]
#[test]
fn rdp_key_from_text_roundtrips_through_get_input_key_without_shift() {
    // Without shift, get_input_key should lowercase — same as rdp_key_from_text output.
    let key = rdp_key_from_text("h");
    let input_key = get_input_key(&key, false);
    let result = convert_key(input_key);
    assert_eq!(result.as_deref(), Some("h"));
}

#[cfg(windows)]
#[test]
fn rdp_key_from_text_roundtrips_through_get_input_key_with_shift() {
    // With shift held, get_input_key should uppercase the lowercased character,
    // producing the correct final key for shift+letter chords.
    let key = rdp_key_from_text("H"); // Shift+H arrives as "H" in text
    let input_key = get_input_key(&key, true); // shift=true
    let result = convert_key(input_key);
    assert_eq!(result.as_deref(), Some("H"));
}

#[test]
fn rdp_unicode_mode_named_keys_still_convert() {
    // Named keys (Enter, Escape, arrows) go through convert_key via the Named
    // variant and must work regardless of physical_key being Unidentified.
    use winit::keyboard::NamedKey;
    let cases = [
        (Key::Named(NamedKey::Enter), "enter"),
        (Key::Named(NamedKey::Escape), "escape"),
        (Key::Named(NamedKey::ArrowUp), "up"),
        (Key::Named(NamedKey::ArrowDown), "down"),
        (Key::Named(NamedKey::Tab), "tab"),
        (Key::Named(NamedKey::Backspace), "backspace"),
    ];
    for (key, expected) in cases {
        let result = convert_key(key.clone());
        assert_eq!(
            result.as_deref(),
            Some(expected),
            "convert_key({key:?}) should return '{expected}'",
        );
    }
}
