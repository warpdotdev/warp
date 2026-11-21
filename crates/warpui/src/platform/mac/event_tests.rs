//! Regression tests for warp issue #341: keybindings must fire when a non-US keyboard
//! layout / IME input source is active. On such input sources macOS translates a
//! physical key to a non-US character via `-[NSEvent charactersIgnoringModifiers]`
//! (e.g. physical `P` -> `з` on Ukrainian, `ㄷ` on Korean, `;` on AZERTY at the US
//! comma position), while keybindings are defined in terms of US-layout keys. Chord
//! shortcuts (ctrl/alt/cmd held) must therefore be resolved from the physical
//! (virtual) key code instead of the translated character.

use warpui_core::EntityId;
use warpui_core::keymap::{
    Context, ContextPredicate, FixedBinding, Keymap, Keystroke, MatchResult, Matcher,
};

use super::keystroke_for_key_down;

// macOS virtual key codes, mirroring `scancode_to_physicalkey` in `keycode.rs`.
const VK_ANSI_A: u16 = 0x00;
const VK_ANSI_E: u16 = 0x0E;
const VK_ANSI_R: u16 = 0x0F;
const VK_ANSI_Y: u16 = 0x10;
const VK_ANSI_P: u16 = 0x23;
const VK_ANSI_COMMA: u16 = 0x2B;
const VK_ANSI_SPACE: u16 = 0x31;
const VK_LEFT_ARROW: u16 = 0x7B;

/// Builds the keystroke for a chord on a Cyrillic (e.g. Ukrainian/Russian) input
/// source, where the physical `P` key is translated to `з`.
fn ukrainian_cmd_p() -> Keystroke {
    keystroke_for_key_down(VK_ANSI_P, false, false, false, true, "з")
}

#[test]
fn chord_uses_physical_key_on_cyrillic_layout() {
    // cmd+p on a Ukrainian input source: physical P, translated "з".
    let keystroke = ukrainian_cmd_p();
    assert!(keystroke.cmd);
    assert_eq!(keystroke.key, "p", "cmd-p must match the physical P key");

    // ctrl+r on a Russian input source: physical R, translated "к".
    let keystroke = keystroke_for_key_down(VK_ANSI_R, true, false, false, false, "к");
    assert!(keystroke.ctrl);
    assert_eq!(keystroke.key, "r", "ctrl-r must match the physical R key");
}

#[test]
fn chord_uses_physical_key_on_korean_ime() {
    // cmd+e with a Korean (Hangul) input source active: physical E, translated "ㄷ".
    let keystroke = keystroke_for_key_down(VK_ANSI_E, false, false, false, true, "ㄷ");
    assert!(keystroke.cmd);
    assert_eq!(keystroke.key, "e");
}

#[test]
fn chord_uses_physical_key_on_french_azerty_layout() {
    // cmd+, on a French AZERTY layout: the US comma position is labelled ";".
    let keystroke = keystroke_for_key_down(VK_ANSI_COMMA, false, false, false, true, ";");
    assert!(keystroke.cmd);
    assert_eq!(
        keystroke.key, ",",
        "cmd-, must match the physical comma key"
    );
}

#[test]
fn chord_uses_physical_key_on_german_qwertz_layout() {
    // The US Y position is labelled "z" on QWERTZ; ctrl+y should still match physical Y.
    let keystroke = keystroke_for_key_down(VK_ANSI_Y, true, false, false, false, "z");
    assert!(keystroke.ctrl);
    assert_eq!(keystroke.key, "y");
}

#[test]
fn chord_applies_shift_to_physical_key() {
    // cmd+shift+p on a Cyrillic layout: physical P + shift must produce uppercase "P",
    // matching how bindings like "cmd-shift-P" are parsed.
    let keystroke = keystroke_for_key_down(VK_ANSI_P, false, false, true, true, "З");
    assert!(keystroke.cmd && keystroke.shift);
    assert_eq!(keystroke.key, "P");
}

#[test]
fn chord_with_physical_space_maps_to_space() {
    let keystroke = keystroke_for_key_down(VK_ANSI_SPACE, false, false, false, true, " ");
    assert!(keystroke.cmd);
    assert_eq!(keystroke.key, " ");
}

#[test]
fn chord_falls_back_to_translated_key_for_named_keys() {
    // cmd+left-arrow: arrows are layout-stable named keys; the physical mapping returns
    // None and the (already function-key-mapped) translated key "left" is kept.
    let keystroke = keystroke_for_key_down(VK_LEFT_ARROW, false, false, false, true, "left");
    assert!(keystroke.cmd);
    assert_eq!(keystroke.key, "left");
}

#[test]
fn unmodified_key_keeps_translated_character_for_ime_text_input() {
    // Without a chord modifier, the translated character must be preserved so that
    // typing with an IME / non-US layout still inserts the right text.
    let keystroke = keystroke_for_key_down(VK_ANSI_P, false, false, false, false, "з");
    assert!(!keystroke.has_any_modifier());
    assert_eq!(keystroke.key, "з");

    // Same for a Korean IME composing text.
    let keystroke = keystroke_for_key_down(VK_ANSI_E, false, false, false, false, "ㄷ");
    assert_eq!(keystroke.key, "ㄷ");

    // shift-only (no ctrl/alt/cmd) also keeps the translated, shifted character.
    let keystroke = keystroke_for_key_down(VK_ANSI_A, false, false, true, false, "Ф");
    assert!(keystroke.shift && !keystroke.cmd && !keystroke.ctrl && !keystroke.alt);
    assert_eq!(keystroke.key, "Ф");
}

#[derive(Debug, PartialEq)]
enum TestAction {
    CommandPalette,
}

/// Builds a matcher with a `cmd-p` binding registered, as the app does for the command
/// palette.
fn matcher_with_binding(binding: &str) -> Matcher {
    let mut matcher = Matcher::new(Keymap::default());
    matcher.register_fixed_bindings([FixedBinding::new(
        binding,
        TestAction::CommandPalette,
        ContextPredicate::Just(true),
    )]);
    matcher
}

fn fires_binding(matcher: &mut Matcher, keystroke: Keystroke) -> bool {
    matches!(
        matcher.push_keystroke(keystroke, EntityId::new(), &Context::default()),
        MatchResult::Action(action)
            if action.as_ref().as_any().downcast_ref::<TestAction>() == Some(&TestAction::CommandPalette)
    )
}

#[test]
fn cmd_p_binding_matches_on_ukrainian_layout() {
    let mut matcher = matcher_with_binding("cmd-p");
    assert!(
        fires_binding(&mut matcher, ukrainian_cmd_p()),
        "cmd-p must fire when a Ukrainian input source is active (issue #341)"
    );
}

#[test]
fn translated_key_alone_does_not_match_binding() {
    // Documents the original bug: if the keystroke had used the layout-translated
    // character ("з") instead of the physical key, the `cmd-p` binding would not fire.
    let mut matcher = matcher_with_binding("cmd-p");
    let translated = Keystroke {
        cmd: true,
        key: "з".to_string(),
        ..Default::default()
    };
    assert!(
        !fires_binding(&mut matcher, translated),
        "the raw translated char must NOT match; this is the pre-fix behavior"
    );
}

/// A simulated key event on a non-US input source: the binding it should trigger, the
/// physical (virtual) key code, the layout-translated character, and the modifiers.
struct LayoutCase {
    binding: &'static str,
    key_code: u16,
    translated: &'static str,
    ctrl: bool,
    alt: bool,
    shift: bool,
    cmd: bool,
}

#[test]
fn bindings_match_across_multiple_international_layouts() {
    let cases = [
        // Ukrainian Cyrillic: cmd+p, physical P -> "з"
        LayoutCase {
            binding: "cmd-p",
            key_code: VK_ANSI_P,
            translated: "з",
            ctrl: false,
            alt: false,
            shift: false,
            cmd: true,
        },
        // Russian Cyrillic: ctrl+r, physical R -> "к"
        LayoutCase {
            binding: "ctrl-r",
            key_code: VK_ANSI_R,
            translated: "к",
            ctrl: true,
            alt: false,
            shift: false,
            cmd: false,
        },
        // Korean IME: cmd+e, physical E -> "ㄷ"
        LayoutCase {
            binding: "cmd-e",
            key_code: VK_ANSI_E,
            translated: "ㄷ",
            ctrl: false,
            alt: false,
            shift: false,
            cmd: true,
        },
        // French AZERTY: cmd+comma, physical comma -> ";"
        LayoutCase {
            binding: "cmd-,",
            key_code: VK_ANSI_COMMA,
            translated: ";",
            ctrl: false,
            alt: false,
            shift: false,
            cmd: true,
        },
    ];
    for case in cases {
        let mut matcher = matcher_with_binding(case.binding);
        let keystroke = keystroke_for_key_down(
            case.key_code,
            case.ctrl,
            case.alt,
            case.shift,
            case.cmd,
            case.translated,
        );
        assert!(
            fires_binding(&mut matcher, keystroke),
            "binding '{}' must fire for translated char '{}' (issue #341)",
            case.binding,
            case.translated
        );
    }
}
