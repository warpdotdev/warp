use winit::keyboard::Key::Character;
use winit::keyboard::{KeyCode, ModifiersState, NativeKeyCode, PhysicalKey, SmolStr};

use super::{
    chord_key_with_us_qwerty_fallback, convert_key, get_input_key, is_chord_on_non_latin_layout,
    us_qwerty_fallback_for_chord,
};

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

#[test]
fn chord_on_non_latin_layout_requires_ctrl_or_super_and_non_ascii() {
    // Russian "м" on the V key: Ctrl or Super held makes it a chord that needs the fallback.
    assert!(is_chord_on_non_latin_layout(
        "м",
        ModifiersState::CONTROL,
        false
    ));
    assert!(is_chord_on_non_latin_layout(
        "м",
        ModifiersState::SUPER,
        false
    ));
    assert!(is_chord_on_non_latin_layout(
        "м",
        ModifiersState::CONTROL | ModifiersState::SHIFT,
        false
    ));
    // Typed text (no chord modifier) must stay untouched.
    assert!(!is_chord_on_non_latin_layout(
        "м",
        ModifiersState::empty(),
        false
    ));
    assert!(!is_chord_on_non_latin_layout(
        "м",
        ModifiersState::SHIFT,
        false
    ));
    assert!(!is_chord_on_non_latin_layout(
        "м",
        ModifiersState::ALT,
        false
    ));
    // ASCII keys already match their bindings.
    assert!(!is_chord_on_non_latin_layout(
        "v",
        ModifiersState::CONTROL,
        false
    ));
}

#[test]
fn chord_on_non_latin_layout_ignores_altgr() {
    // AltGr characters such as "€" must not be rewritten into a Ctrl chord.
    assert!(!is_chord_on_non_latin_layout(
        "€",
        ModifiersState::CONTROL,
        true
    ));
    assert!(!is_chord_on_non_latin_layout(
        "€",
        ModifiersState::CONTROL | ModifiersState::ALT,
        true
    ));
}

/// Runs the same steps as `convert_keyboard_input_event` after the layout has produced a logical
/// key: resolve the chord key, apply the shift casing and render the binding string.
fn chord_binding(
    layout_char: &str,
    physical: KeyCode,
    modifiers: ModifiersState,
    right_alt_pressed: bool,
) -> String {
    let logical_key = Character(SmolStr::from(layout_char));
    let shift = modifiers.shift_key();
    let key = if is_chord_on_non_latin_layout(layout_char, modifiers, right_alt_pressed) {
        chord_key_with_us_qwerty_fallback(&logical_key, &PhysicalKey::Code(physical), shift)
    } else {
        logical_key
    };
    convert_key(get_input_key(&key, shift))
        .expect("character keys always convert")
        .to_string()
}

#[test]
fn russian_layout_chords_resolve_to_the_latin_binding_key() {
    // Regression test for pasting/copying on a Russian layout: the V and C keys report "м" and
    // "с", but the bindings are `ctrl-v` / `ctrl-shift-V` / `ctrl-c`.
    assert_eq!(
        chord_binding("м", KeyCode::KeyV, ModifiersState::CONTROL, false),
        "v"
    );
    assert_eq!(
        chord_binding(
            "м",
            KeyCode::KeyV,
            ModifiersState::CONTROL | ModifiersState::SHIFT,
            false
        ),
        "V"
    );
    assert_eq!(
        chord_binding("с", KeyCode::KeyC, ModifiersState::CONTROL, false),
        "c"
    );
    assert_eq!(
        chord_binding("м", KeyCode::KeyV, ModifiersState::SUPER, false),
        "v"
    );
}

#[test]
fn non_latin_layout_typed_text_is_not_rewritten() {
    // Without Ctrl/Super the layout character must reach the input untouched.
    assert_eq!(
        chord_binding("м", KeyCode::KeyV, ModifiersState::empty(), false),
        "м"
    );
    assert_eq!(
        chord_binding("м", KeyCode::KeyV, ModifiersState::SHIFT, false),
        "М"
    );
    // AltGr output must not be turned into a chord.
    assert_eq!(
        chord_binding("€", KeyCode::KeyE, ModifiersState::CONTROL, true),
        "€"
    );
}

#[test]
fn latin_layout_chords_are_unchanged() {
    // Dvorak puts "j" on the physical C position; ASCII keys keep their logical meaning.
    assert_eq!(
        chord_binding("j", KeyCode::KeyC, ModifiersState::CONTROL, false),
        "j"
    );
    assert_eq!(
        chord_binding("v", KeyCode::KeyV, ModifiersState::CONTROL, false),
        "v"
    );
}
