use super::{KeyCode, NativeKeyCode, PhysicalKey};

#[test]
fn us_qwerty_char_maps_all_letters() {
    let letters = [
        (KeyCode::KeyA, 'a'),
        (KeyCode::KeyB, 'b'),
        (KeyCode::KeyC, 'c'),
        (KeyCode::KeyD, 'd'),
        (KeyCode::KeyE, 'e'),
        (KeyCode::KeyF, 'f'),
        (KeyCode::KeyG, 'g'),
        (KeyCode::KeyH, 'h'),
        (KeyCode::KeyI, 'i'),
        (KeyCode::KeyJ, 'j'),
        (KeyCode::KeyK, 'k'),
        (KeyCode::KeyL, 'l'),
        (KeyCode::KeyM, 'm'),
        (KeyCode::KeyN, 'n'),
        (KeyCode::KeyO, 'o'),
        (KeyCode::KeyP, 'p'),
        (KeyCode::KeyQ, 'q'),
        (KeyCode::KeyR, 'r'),
        (KeyCode::KeyS, 's'),
        (KeyCode::KeyT, 't'),
        (KeyCode::KeyU, 'u'),
        (KeyCode::KeyV, 'v'),
        (KeyCode::KeyW, 'w'),
        (KeyCode::KeyX, 'x'),
        (KeyCode::KeyY, 'y'),
        (KeyCode::KeyZ, 'z'),
    ];
    for (code, lowercase) in letters {
        assert_eq!(
            code.us_qwerty_char(false),
            Some(lowercase),
            "{code:?} without shift should produce '{lowercase}'"
        );
        assert_eq!(
            code.us_qwerty_char(true),
            Some(lowercase.to_ascii_uppercase()),
            "{code:?} with shift should produce '{}'",
            lowercase.to_ascii_uppercase()
        );
    }
}

#[test]
fn us_qwerty_char_maps_digits() {
    let digits = [
        (KeyCode::Digit0, '0', ')'),
        (KeyCode::Digit1, '1', '!'),
        (KeyCode::Digit2, '2', '@'),
        (KeyCode::Digit3, '3', '#'),
        (KeyCode::Digit4, '4', '$'),
        (KeyCode::Digit5, '5', '%'),
        (KeyCode::Digit6, '6', '^'),
        (KeyCode::Digit7, '7', '&'),
        (KeyCode::Digit8, '8', '*'),
        (KeyCode::Digit9, '9', '('),
    ];
    for (code, unshifted, shifted) in digits {
        assert_eq!(code.us_qwerty_char(false), Some(unshifted), "{code:?}");
        assert_eq!(code.us_qwerty_char(true), Some(shifted), "{code:?}+shift");
    }
}

#[test]
fn us_qwerty_char_maps_punctuation() {
    let punctuation = [
        (KeyCode::Minus, '-', '_'),
        (KeyCode::Equal, '=', '+'),
        (KeyCode::BracketLeft, '[', '{'),
        (KeyCode::BracketRight, ']', '}'),
        (KeyCode::Backslash, '\\', '|'),
        (KeyCode::Semicolon, ';', ':'),
        (KeyCode::Quote, '\'', '"'),
        (KeyCode::Comma, ',', '<'),
        (KeyCode::Period, '.', '>'),
        (KeyCode::Slash, '/', '?'),
        (KeyCode::Backquote, '`', '~'),
    ];
    for (code, unshifted, shifted) in punctuation {
        assert_eq!(code.us_qwerty_char(false), Some(unshifted), "{code:?}");
        assert_eq!(code.us_qwerty_char(true), Some(shifted), "{code:?}+shift");
    }
}

#[test]
fn us_qwerty_char_maps_space() {
    assert_eq!(KeyCode::Space.us_qwerty_char(false), Some(' '));
    assert_eq!(KeyCode::Space.us_qwerty_char(true), Some(' '));
}

#[test]
fn us_qwerty_char_returns_none_for_non_character_keys() {
    // Named keys (arrows, enter, function keys, ...), modifiers, numpad keys and other
    // keys that don't produce a printable US-layout character must return `None` so
    // that callers fall back to the layout-translated / named-key path.
    let non_character_keys = [
        KeyCode::Enter,
        KeyCode::NumpadEnter,
        KeyCode::Tab,
        KeyCode::Backspace,
        KeyCode::Escape,
        KeyCode::ArrowUp,
        KeyCode::ArrowDown,
        KeyCode::ArrowLeft,
        KeyCode::ArrowRight,
        KeyCode::Home,
        KeyCode::End,
        KeyCode::PageUp,
        KeyCode::PageDown,
        KeyCode::Insert,
        KeyCode::Delete,
        KeyCode::F1,
        KeyCode::F12,
        KeyCode::F35,
        KeyCode::ShiftLeft,
        KeyCode::ShiftRight,
        KeyCode::ControlLeft,
        KeyCode::ControlRight,
        KeyCode::AltLeft,
        KeyCode::AltRight,
        KeyCode::SuperLeft,
        KeyCode::SuperRight,
        KeyCode::CapsLock,
        KeyCode::NumLock,
        KeyCode::Numpad0,
        KeyCode::Numpad9,
        KeyCode::NumpadAdd,
        KeyCode::NumpadDecimal,
        KeyCode::IntlBackslash,
        KeyCode::IntlRo,
        KeyCode::IntlYen,
        KeyCode::Lang1,
        KeyCode::Lang2,
        KeyCode::Convert,
        KeyCode::NonConvert,
        KeyCode::Fn,
        KeyCode::AudioVolumeUp,
        KeyCode::Power,
    ];
    for code in non_character_keys {
        for shift in [false, true] {
            assert_eq!(
                code.us_qwerty_char(shift),
                None,
                "{code:?} should not map to a US-QWERTY character (shift={shift})"
            );
        }
    }
}

#[test]
fn physical_key_us_qwerty_char_delegates_to_key_code() {
    assert_eq!(
        PhysicalKey::Code(KeyCode::KeyP).us_qwerty_char(false),
        Some('p')
    );
    assert_eq!(
        PhysicalKey::Code(KeyCode::KeyP).us_qwerty_char(true),
        Some('P')
    );
    assert_eq!(
        PhysicalKey::Code(KeyCode::Comma).us_qwerty_char(false),
        Some(',')
    );
    assert_eq!(
        PhysicalKey::Unidentified(NativeKeyCode::MacOS(0x23)).us_qwerty_char(false),
        None
    );
    assert_eq!(
        PhysicalKey::Unidentified(NativeKeyCode::Unidentified).us_qwerty_char(true),
        None
    );
}

/// Regression tests for warp issue #341: keybindings must fire on non-US keyboard
/// layouts / IME input sources. On these layouts the OS translates a physical key to a
/// non-US character (e.g. physical `P` -> `з` on Ukrainian, `ㄷ` on Korean,
/// `;` on AZERTY for the US comma position). The physical key code is what identifies
/// the key, so the US-QWERTY mapping above must recover the US character for it.
#[test]
fn us_qwerty_char_recovers_us_key_for_international_layouts() {
    // (physical key, translated char on the international layout, expected US char)
    let cases: [(KeyCode, char, char); 6] = [
        // Ukrainian/Russian Cyrillic: physical P -> "з"/"п", physical R -> "к"
        (KeyCode::KeyP, 'з', 'p'),
        (KeyCode::KeyR, 'к', 'r'),
        // Korean (Hangul IME): physical E -> "ㄷ"
        (KeyCode::KeyE, 'ㄷ', 'e'),
        // French AZERTY: the US comma position is labelled ";"
        (KeyCode::Comma, ';', ','),
        // German QWERTZ: the US Y position is labelled "z"
        (KeyCode::KeyY, 'z', 'y'),
        // Greek: physical P -> "ψ"
        (KeyCode::KeyP, 'ψ', 'p'),
    ];
    for (physical, _translated, expected_us) in cases {
        assert_eq!(
            physical.us_qwerty_char(false),
            Some(expected_us),
            "physical {physical:?} must resolve to US '{expected_us}' regardless of layout"
        );
    }
}
