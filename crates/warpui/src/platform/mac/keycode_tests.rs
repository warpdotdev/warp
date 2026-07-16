//! Unit tests for the UTF-16 sequence handling in `objc/keycode.m`.
//!
//! `keyCodeToChar` / `charToKeyCodes` ultimately run every translation through
//! `KeyNameFromTranslatedChars`, which turns the raw `UCKeyTranslate` output
//! (`unicode_string` + `length`) into a key name. We exercise that helper
//! directly with crafted UTF-16 buffers so the empty, single-character,
//! surrogate-pair, and multi-character cases are covered deterministically,
//! independent of whatever keyboard layout the test machine happens to use.
use cocoa::base::id;

use super::super::utils::nsstring_as_str;
use super::super::AutoreleasePoolGuard;

// `kVK_ANSI_A` — a normal (non-control) key. Its empty translation has no
// control-key fallback, so it lets us observe the raw sequence handling.
const KEY_CODE_A: u16 = 0x00;
// `kVK_Return` — produces a carriage-return control unit that must map to "enter".
const KEY_CODE_RETURN: u16 = 0x24;
// `kVK_Tab` — another control key used to verify control-unit fallback.
const KEY_CODE_TAB: u16 = 0x30;
// `kVK_LeftArrow` — a key `UCKeyTranslate` can't translate; used to check that an
// empty translation still resolves through the control-key map.
const KEY_CODE_LEFT_ARROW: u16 = 0x7B;

extern "C" {
    // Defined in `objc/keycode.m`. `length` is a `size_t` count of UTF-16 units.
    fn KeyNameFromTranslatedChars(key_code: u16, unicode_string: *const u16, length: usize) -> id;
}

/// Calls `KeyNameFromTranslatedChars` with the given UTF-16 units and converts
/// the (autoreleased) `NSString` result into an owned `String`, or `None` when
/// the helper returns `nil`.
fn key_name(key_code: u16, units: &[u16]) -> Option<String> {
    let _pool = AutoreleasePoolGuard::new();
    let ptr = unsafe { KeyNameFromTranslatedChars(key_code, units.as_ptr(), units.len()) };
    if ptr.is_null() {
        None
    } else {
        unsafe { nsstring_as_str(ptr) }.ok().map(str::to_owned)
    }
}

#[test]
fn single_character_result_is_preserved() {
    let units = [b'a' as u16];
    assert_eq!(key_name(KEY_CODE_A, &units), Some("a".to_owned()));
}

#[test]
fn multi_character_result_is_preserved() {
    // 'e' + U+0301 COMBINING ACUTE ACCENT => "é" as two UTF-16 units. The old
    // implementation dropped the combining mark by returning only the first
    // unit.
    let units = [b'e' as u16, 0x0301];
    assert_eq!(key_name(KEY_CODE_A, &units), Some("e\u{0301}".to_owned()));
}

#[test]
fn surrogate_pair_result_is_preserved() {
    // U+1F600 GRINNING FACE is encoded as the surrogate pair D83D DE00. Both
    // units must survive to reconstruct the code point.
    let units = [0xD83D_u16, 0xDE00_u16];
    assert_eq!(key_name(KEY_CODE_A, &units), Some("\u{1F600}".to_owned()));
}

#[test]
fn empty_result_for_non_control_key_is_none() {
    // A zero-length translation must be handled safely (no out-of-bounds read)
    // and, for a key with no control-key mapping, produce no name.
    assert_eq!(key_name(KEY_CODE_A, &[]), None);
}

#[test]
fn empty_result_for_control_key_uses_control_mapping() {
    // Arrow keys can translate to nothing; the control-key map still names them.
    assert_eq!(key_name(KEY_CODE_LEFT_ARROW, &[]), Some("left".to_owned()));
    assert_eq!(key_name(KEY_CODE_RETURN, &[]), Some("enter".to_owned()));
}

#[test]
fn control_character_first_unit_uses_control_mapping() {
    // A leading control unit (here CR for Return, or tab) is not printable
    // text, so it resolves through the control-key map rather than being
    // emitted verbatim.
    let return_units = [0x0D_u16];
    assert_eq!(
        key_name(KEY_CODE_RETURN, &return_units),
        Some("enter".to_owned())
    );

    let tab_units = [b'\t' as u16];
    assert_eq!(key_name(KEY_CODE_TAB, &tab_units), Some("tab".to_owned()));
}
