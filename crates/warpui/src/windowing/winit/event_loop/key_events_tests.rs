use winit::keyboard::Key::Character;
use winit::keyboard::{Key, KeyCode, NamedKey, NativeKeyCode, PhysicalKey, SmolStr};

use warpui_core::EntityId;
use warpui_core::keymap::{
    Context, ContextPredicate, FixedBinding, Keymap, Keystroke, MatchResult, Matcher,
};

use super::{convert_key, get_input_key, resolve_logical_key, us_qwerty_fallback_for_chord};

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
    // Letters resolve to the shift-applied form; `get_input_key` applies the same case
    // transform downstream, so the two compose idempotently.
    let cases = [
        (KeyCode::KeyA, 'a'),
        (KeyCode::KeyC, 'c'),
        (KeyCode::KeyV, 'v'),
        (KeyCode::KeyZ, 'z'),
    ];
    for (code, lowercase) in cases {
        assert_eq!(
            us_qwerty_fallback_for_chord(&PhysicalKey::Code(code), false),
            Some(lowercase),
            "expected {code:?} -> {lowercase} (shift=false)",
        );
        assert_eq!(
            us_qwerty_fallback_for_chord(&PhysicalKey::Code(code), true),
            Some(lowercase.to_ascii_uppercase()),
            "expected {code:?} -> {} (shift=true)",
            lowercase.to_ascii_uppercase()
        );
    }
}

#[test]
fn us_qwerty_fallback_maps_digits_and_punctuation() {
    let cases = [
        (KeyCode::Digit0, '0'),
        (KeyCode::Digit9, '9'),
        (KeyCode::Minus, '-'),
        (KeyCode::Equal, '='),
        (KeyCode::Slash, '/'),
        (KeyCode::Backquote, '`'),
        (KeyCode::Semicolon, ';'),
        (KeyCode::Comma, ','),
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
        (KeyCode::Digit1, '!'),
        (KeyCode::Digit2, '@'),
        (KeyCode::Digit6, '^'),
        (KeyCode::Digit9, '('),
        (KeyCode::Digit0, ')'),
        (KeyCode::Minus, '_'),
        (KeyCode::Equal, '+'),
        (KeyCode::BracketLeft, '{'),
        (KeyCode::BracketRight, '}'),
        (KeyCode::Backslash, '|'),
        (KeyCode::Semicolon, ':'),
        (KeyCode::Quote, '"'),
        (KeyCode::Comma, '<'),
        (KeyCode::Period, '>'),
        (KeyCode::Slash, '?'),
        (KeyCode::Backquote, '~'),
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

/// Simulates a key event as reported by the OS when a non-US keyboard layout / IME
/// input source is active: `logical` is the layout-translated character, `physical` is
/// the physical key that was pressed. Returns the keystroke key that
/// `convert_keyboard_input_event` would produce for a chord with the given modifiers.
fn simulated_keystroke_key(
    logical: &str,
    physical: KeyCode,
    ctrl: bool,
    alt: bool,
    shift: bool,
    cmd: bool,
) -> Option<String> {
    let logical_key = Character(SmolStr::from(logical));
    let physical_key = PhysicalKey::Code(physical);
    let is_chord = ctrl || alt || cmd;
    let resolved = resolve_logical_key(&logical_key, &physical_key, is_chord, shift);
    let input_key = get_input_key(&resolved, shift);
    convert_key(input_key).map(|k| k.to_string())
}

#[test]
fn chord_resolves_physical_key_on_cyrillic_layout() {
    // Ukrainian/Russian layout: physical P translates to "з", physical R to "к".
    // cmd+p and ctrl+r must still resolve to "p" and "r" (issue #341).
    assert_eq!(
        simulated_keystroke_key("з", KeyCode::KeyP, false, false, false, true),
        Some("p".to_string())
    );
    assert_eq!(
        simulated_keystroke_key("к", KeyCode::KeyR, true, false, false, false),
        Some("r".to_string())
    );
}

#[test]
fn chord_resolves_physical_key_on_korean_ime() {
    // Korean (Hangul) input source: physical E translates to "ㄷ".
    assert_eq!(
        simulated_keystroke_key("ㄷ", KeyCode::KeyE, false, false, false, true),
        Some("e".to_string())
    );
}

#[test]
fn chord_resolves_physical_key_on_french_azerty_layout() {
    // French AZERTY: the US comma position is labelled ";"; cmd+, must still resolve
    // to ",".
    assert_eq!(
        simulated_keystroke_key(";", KeyCode::Comma, false, false, false, true),
        Some(",".to_string())
    );
    // AZERTY also swaps A/Q: the physical A position is labelled "q".
    assert_eq!(
        simulated_keystroke_key("q", KeyCode::KeyA, true, false, false, false),
        Some("a".to_string())
    );
}

#[test]
fn chord_resolves_physical_key_on_german_qwertz_layout() {
    // QWERTZ: the US Y position is labelled "z".
    assert_eq!(
        simulated_keystroke_key("z", KeyCode::KeyY, true, false, false, false),
        Some("y".to_string())
    );
}

#[test]
fn chord_applies_shift_to_physical_key() {
    // cmd+shift+p on a Cyrillic layout: physical P + shift -> "P".
    assert_eq!(
        simulated_keystroke_key("З", KeyCode::KeyP, false, false, true, true),
        Some("P".to_string())
    );
}

#[test]
fn non_chord_key_keeps_translated_character_for_ime_text_input() {
    // Without a ctrl/alt/cmd modifier, the layout-translated character must be kept so
    // that typing with an IME / non-US layout still produces the right text.
    assert_eq!(
        simulated_keystroke_key("з", KeyCode::KeyP, false, false, false, false),
        Some("з".to_string())
    );
    assert_eq!(
        simulated_keystroke_key("ㄷ", KeyCode::KeyE, false, false, false, false),
        Some("ㄷ".to_string())
    );
    // shift-only is not a chord either.
    assert_eq!(
        simulated_keystroke_key("З", KeyCode::KeyP, false, false, true, false),
        Some("З".to_string())
    );
}

#[test]
fn chord_with_named_or_unmapped_key_keeps_logical_key() {
    // Named keys (enter, arrows, ...) are layout-stable and must pass through.
    let enter = Key::Named(NamedKey::Enter);
    let resolved = resolve_logical_key(&enter, &PhysicalKey::Code(KeyCode::Enter), true, false);
    assert_eq!(resolved, enter);

    // A physical key with no US-QWERTY character mapping falls back to the logical key.
    let f5 = Character(SmolStr::from("F5"));
    let resolved = resolve_logical_key(&f5, &PhysicalKey::Code(KeyCode::F5), true, false);
    assert_eq!(resolved, f5);

    // Unidentified physical keys also fall back to the logical key.
    let cyrillic = Character(SmolStr::from("ж"));
    let resolved = resolve_logical_key(
        &cyrillic,
        &PhysicalKey::Unidentified(NativeKeyCode::Xkb(0x2c)),
        true,
        false,
    );
    assert_eq!(resolved, cyrillic);
}

#[derive(Debug, PartialEq)]
enum TestAction {
    CommandPalette,
}

/// Builds a matcher with a single fixed binding, as the app does for e.g. the command
/// palette (`cmd-p`).
fn matcher_with_binding(binding: &str) -> Matcher {
    let mut matcher = Matcher::new(Keymap::default());
    matcher.register_fixed_bindings([FixedBinding::new(
        binding,
        TestAction::CommandPalette,
        ContextPredicate::Just(true),
    )]);
    matcher
}

/// End-to-end: convert a simulated non-US-layout key event to a keystroke (the same
/// steps `convert_keyboard_input_event` performs) and push it through the matcher.
fn fires_binding(binding: &str, logical: &str, physical: KeyCode, cmd: bool) -> bool {
    let mut matcher = matcher_with_binding(binding);
    let key = simulated_keystroke_key(logical, physical, false, false, false, cmd)
        .expect("key event should produce a keystroke key");
    let keystroke = Keystroke {
        cmd,
        key,
        ..Default::default()
    };
    matches!(
        matcher.push_keystroke(keystroke, EntityId::new(), &Context::default()),
        MatchResult::Action(action)
            if action.as_ref().as_any().downcast_ref::<TestAction>() == Some(&TestAction::CommandPalette)
    )
}

#[test]
fn cmd_p_binding_fires_on_non_us_layouts() {
    // Regression test for issue #341: `cmd-p` must open the command palette even when a
    // non-US input source translates the physical P key to another character.
    assert!(
        fires_binding("cmd-p", "з", KeyCode::KeyP, true),
        "Ukrainian"
    );
    assert!(fires_binding("cmd-p", "ㄱ", KeyCode::KeyP, true), "Korean");
    assert!(
        fires_binding("cmd-p", "p", KeyCode::KeyP, true),
        "US (unchanged)"
    );
    assert!(
        fires_binding("cmd-,", ";", KeyCode::Comma, true),
        "French AZERTY"
    );
}

#[test]
fn translated_key_alone_does_not_fire_binding() {
    // Documents the original bug: a keystroke built from the raw translated character
    // ("з" + cmd) does not match the `cmd-p` binding.
    let mut matcher = matcher_with_binding("cmd-p");
    let keystroke = Keystroke {
        cmd: true,
        key: "з".to_string(),
        ..Default::default()
    };
    assert!(
        !matches!(
            matcher.push_keystroke(keystroke, EntityId::new(), &Context::default()),
            MatchResult::Action(_)
        ),
        "the raw translated char must NOT match; this is the pre-fix behavior"
    );
}
