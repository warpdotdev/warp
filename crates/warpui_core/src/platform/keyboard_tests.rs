use super::*;

// Regression coverage for GH#15196 / CSAT-10277: on macOS, while a non-Latin input source (e.g.
// Korean/Hangul) is active, `charactersIgnoringModifiers` can be empty or a non-ASCII IME
// composition character for a Ctrl-modified key, even though a Ctrl chord is never IME
// composition input. `resolve_ctrl_chord_key_and_chars` is the full decision that
// `crates/warpui/src/platform/mac/event.rs`'s `NSEvent` conversion makes (it just extracts the
// raw inputs and cannot itself be unit-tested outside of a macOS host, since it requires a live
// `NSEvent`); testing this function exercises the actual key/chars outputs the conversion
// produces, not just the individual helpers it's built from.

#[test]
fn ctrl_chord_physical_letter_maps_letter_keys() {
    assert_eq!(ctrl_chord_physical_letter(KeyCode::KeyJ), Some("j"));
    assert_eq!(ctrl_chord_physical_letter(KeyCode::KeyA), Some("a"));
    assert_eq!(ctrl_chord_physical_letter(KeyCode::KeyZ), Some("z"));
}

#[test]
fn ctrl_chord_physical_letter_ignores_non_letter_keys() {
    assert_eq!(ctrl_chord_physical_letter(KeyCode::Digit1), None);
    assert_eq!(ctrl_chord_physical_letter(KeyCode::Space), None);
    assert_eq!(ctrl_chord_physical_letter(KeyCode::Enter), None);
}

#[test]
fn ctrl_chord_fallback_not_needed_without_ctrl() {
    // Without Ctrl held, always defer to whatever the input source produced (including
    // nothing), since this is ordinary IME composition input.
    assert!(!ctrl_chord_needs_physical_key_fallback(false, None));
    assert!(!ctrl_chord_needs_physical_key_fallback(
        false,
        Some('\u{314F}')
    ));
}

#[test]
fn ctrl_chord_fallback_not_needed_for_ascii_result() {
    // English/ABC input source: Ctrl+J already produces an ASCII character, so the existing
    // (possibly layout-remapped, e.g. Dvorak/AZERTY) behavior should be preserved.
    assert!(!ctrl_chord_needs_physical_key_fallback(true, Some('j')));
}

#[test]
fn ctrl_chord_fallback_needed_for_empty_or_non_ascii_result() {
    // Korean/Hangul input source active: `charactersIgnoringModifiers` is empty, or a Hangul
    // jamo (U+314F 'ㅏ') rather than 'j'.
    assert!(ctrl_chord_needs_physical_key_fallback(true, None));
    assert!(ctrl_chord_needs_physical_key_fallback(
        true,
        Some('\u{314F}')
    ));
}

#[test]
fn ctrl_letter_to_control_char_maps_ctrl_j_to_line_feed() {
    assert_eq!(ctrl_letter_to_control_char("j"), Some('\u{0A}'));
}

#[test]
fn ctrl_letter_to_control_char_covers_full_alphabet() {
    assert_eq!(ctrl_letter_to_control_char("a"), Some('\u{01}'));
    assert_eq!(ctrl_letter_to_control_char("z"), Some('\u{1A}'));
}

#[test]
fn ctrl_letter_to_control_char_rejects_non_letters() {
    assert_eq!(ctrl_letter_to_control_char("1"), None);
    assert_eq!(ctrl_letter_to_control_char(""), None);
    assert_eq!(ctrl_letter_to_control_char("ab"), None);
}

/// The reporter's actual bug: `Ctrl+J` under a Hangul input source, where
/// `charactersIgnoringModifiers` is empty (nothing composed yet) and `characters()` (`os_chars`)
/// is also empty. Both the editor keystroke and the PTY control byte must be recovered.
#[test]
fn ctrl_j_resolves_key_and_control_byte_when_input_source_produces_nothing() {
    let resolution = resolve_ctrl_chord_key_and_chars(
        /* ctrl_held */ true,
        /* alt_held */ false,
        /* cmd_held */ false,
        /* ime_first_char */ None,
        /* ime_key_candidate */ None,
        /* physical_letter */ ctrl_chord_physical_letter(KeyCode::KeyJ),
        /* os_chars */ "",
    );
    assert_eq!(resolution.key.as_deref(), Some("j"));
    assert_eq!(resolution.chars, "\u{0A}");
}

/// Same as above, but the input source produced a non-ASCII Hangul jamo instead of nothing.
#[test]
fn ctrl_j_resolves_key_and_control_byte_when_input_source_produces_a_jamo() {
    let resolution = resolve_ctrl_chord_key_and_chars(
        true,
        false,
        false,
        Some('\u{314F}'),
        Some("\u{314F}"),
        ctrl_chord_physical_letter(KeyCode::KeyJ),
        "",
    );
    assert_eq!(resolution.key.as_deref(), Some("j"));
    assert_eq!(resolution.chars, "\u{0A}");
}

/// Baseline: under English/ABC, `Ctrl+J` already works today (the input source produces `'j'`
/// and the OS already places the control byte in `characters()`). The fallback must be a no-op
/// here, and the existing values must be passed through unchanged.
#[test]
fn ctrl_j_already_working_is_left_unchanged() {
    let resolution = resolve_ctrl_chord_key_and_chars(
        true,
        false,
        false,
        Some('j'),
        Some("j"),
        ctrl_chord_physical_letter(KeyCode::KeyJ),
        "\u{0A}",
    );
    assert_eq!(resolution.key.as_deref(), Some("j"));
    assert_eq!(resolution.chars, "\u{0A}");
}

/// Regression test for review finding 1 (part 1): when the input source already produced a
/// usable ASCII key (so the fallback is not needed), the `chars` field must not be rewritten to
/// a synthesized control byte, even if `os_chars` doesn't look like a control sequence for some
/// other reason. Rewriting here would alter a chord the OS is already handling correctly.
#[test]
fn chars_are_not_rewritten_when_fallback_is_not_needed() {
    let resolution = resolve_ctrl_chord_key_and_chars(
        true,
        false,
        false,
        Some('j'), // Input source already produced an ASCII character...
        Some("j"),
        ctrl_chord_physical_letter(KeyCode::KeyJ),
        "x", // ...so this (contrived) non-control `os_chars` must survive untouched.
    );
    assert_eq!(resolution.chars, "x");
}

/// Regression test for review finding 1 (part 2): Ctrl+Alt chords must not be forced through the
/// plain-Ctrl C0 mapping, even when the fallback would otherwise apply (e.g. Hangul active). The
/// `key` still gets the physical-letter fallback, since keybinding matching accounts for the Alt
/// modifier via exact `Keystroke` equality, but `chars` (raw PTY bytes) must be left alone.
#[test]
fn ctrl_alt_chord_does_not_rewrite_chars() {
    let resolution = resolve_ctrl_chord_key_and_chars(
        true,
        /* alt_held */ true,
        false,
        None,
        None,
        ctrl_chord_physical_letter(KeyCode::KeyJ),
        "",
    );
    assert_eq!(resolution.key.as_deref(), Some("j"));
    assert_eq!(resolution.chars, "");
}

/// Same as above, for Ctrl+Cmd.
#[test]
fn ctrl_cmd_chord_does_not_rewrite_chars() {
    let resolution = resolve_ctrl_chord_key_and_chars(
        true,
        false,
        /* cmd_held */ true,
        None,
        None,
        ctrl_chord_physical_letter(KeyCode::KeyJ),
        "",
    );
    assert_eq!(resolution.key.as_deref(), Some("j"));
    assert_eq!(resolution.chars, "");
}

/// Unmodified-key IME composition (e.g. typing a Hangul syllable with no Ctrl held) must be
/// completely unaffected: the physical-key fallback never applies, so the input source's own
/// candidate key is used verbatim and `chars` is untouched.
#[test]
fn unmodified_ime_composition_is_unaffected() {
    let resolution = resolve_ctrl_chord_key_and_chars(
        false,
        false,
        false,
        Some('\u{314F}'),
        Some("\u{314F}"),
        ctrl_chord_physical_letter(KeyCode::KeyJ),
        "",
    );
    assert_eq!(resolution.key.as_deref(), Some("\u{314F}"));
    assert_eq!(resolution.chars, "");
}

/// When nothing usable is available at all -- no IME candidate and no physical-letter fallback
/// (e.g. a non-letter key with an unmapped/empty input-source character) -- the event should be
/// dropped entirely, matching the pre-existing behavior.
#[test]
fn event_is_dropped_when_nothing_usable() {
    let resolution = resolve_ctrl_chord_key_and_chars(false, false, false, None, None, None, "");
    assert_eq!(resolution.key, None);
}
