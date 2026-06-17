use warpui_core::keymap::Keystroke;
use warpui_core::platform::keyboard::KeyCode;
use warpui_core::platform::OperatingSystem;

use super::{ModeProvider, TermMode};

/// Checks whether the Kitty keyboard protocol requires CSI u encoding for this
/// keystroke, and if so, returns the encoded sequence.
///
/// Under REPORT_ALL_KEYS_AS_ESCAPE (flag 8), all keys use CSI u.
/// Under DISAMBIGUATE_ESCAPE_CODES (flag 1), only ambiguous keys use CSI u.
/// Returns None if CSI u is not needed (caller should fall back to legacy encoding).
pub(super) fn maybe_convert_keystroke_to_csi_u(
    keystroke: &Keystroke,
    key_without_modifiers: Option<&str>,
    chars: Option<&str>,
    mode_provider: &dyn ModeProvider,
) -> Option<Vec<u8>> {
    if !mode_provider.is_term_mode_set(TermMode::KEYBOARD_PROTOCOL) {
        return None;
    }

    // Under the DISAMBIGUATE_ESCAPE_CODES flag (flag 1), the Kitty keyboard
    // protocol says to use CSI u encoding for keys that would otherwise be
    // ambiguous in legacy terminal encoding. This includes:
    // - The Escape key (ESC byte 0x1B is also the start of all escape sequences)
    // - Modified keys where the modifier is lost in legacy encoding (e.g.,
    //   Ctrl+A → C0 code 0x01, Alt+a → ESC a on non-macOS)
    //
    // On macOS, Alt is excluded because Option generates composed characters
    // (e.g., Option+a → å) via the IME rather than acting as a modifier.
    //
    // See: https://sw.kovidgoyal.net/kitty/keyboard-protocol/#disambiguate
    let mut is_ambiguous = keystroke.key == "escape" || keystroke.ctrl || keystroke.meta;
    if !OperatingSystem::get().is_mac() {
        is_ambiguous = is_ambiguous || keystroke.alt;
    }
    // Shift alone is NOT ambiguous for printable keys — Shift changes the
    // character itself (e.g., Shift+a → A). But for functional keys like
    // Enter, Tab, and Backspace, legacy encoding drops Shift entirely
    // (Shift+Enter sends the same bytes as Enter), making them ambiguous.
    if keystroke.shift {
        is_ambiguous =
            is_ambiguous || matches!(keystroke.key.as_str(), "enter" | "tab" | "backspace");
    }

    // With flag 8 (REPORT_ALL_KEYS_AS_ESC): use CSI u for all keys.
    // With flag 1 (DISAMBIGUATE_ESC_CODES): use CSI u only when ambiguous.
    let should_use_csi_u = mode_provider.is_term_mode_set(TermMode::KEYBOARD_REPORT_ALL_AS_ESCAPE)
        || (mode_provider.is_term_mode_set(TermMode::KEYBOARD_DISAMBIGUATE_ESCAPE) && is_ambiguous);

    if !should_use_csi_u {
        return None;
    }

    keystroke_to_csi_u(keystroke, key_without_modifiers, chars, mode_provider)
}

/// Encodes a keystroke to a CSI u escape sequence for the Kitty keyboard protocol.
///
/// Full format: CSI unicode-key-code[:shifted-key] ; modifiers[:event_type] ; text-as-codepoints u
/// where modifiers is: 1 + (shift ? 1 : 0) + (alt ? 2 : 0) + (ctrl ? 4 : 0) + (super ? 8 : 0)
///
/// Key codes follow the Kitty protocol specification:
/// - Standard keys use their Unicode codepoints (Enter=13, Tab=9, etc.)
/// - Function keys F13-F35 use codes 57376-57398
/// - When REPORT_ALTERNATE_KEYS (flag 4) is active and shift is held, the shifted key
///   code is appended after a colon (e.g., `97:65` for shift+a).
/// - When REPORT_ASSOCIATED_TEXT (flag 16) is active, the OS-provided text (`chars`)
///   is appended as a colon-separated list of Unicode codepoints (e.g., `;65` for "A").
/// - Event type encoding (press=1, repeat=2, release=3) is omitted for press events
///   since press is the default. Repeat/release are not yet handled here; see
///   `modifier_key_to_csi_u` for modifier key press/release.
///
/// See functional key definitions: https://sw.kovidgoyal.net/kitty/keyboard-protocol/#functional
///
/// Returns None if the key cannot be encoded as a CSI u sequence.
fn keystroke_to_csi_u(
    keystroke: &Keystroke,
    key_without_modifiers: Option<&str>,
    chars: Option<&str>,
    mode_provider: &(impl ModeProvider + ?Sized),
) -> Option<Vec<u8>> {
    let report_alternate = mode_provider.is_term_mode_set(TermMode::KEYBOARD_REPORT_ALTERNATE_KEYS);
    let report_text = mode_provider.is_term_mode_set(TermMode::KEYBOARD_REPORT_ASSOCIATED_TEXT);

    // Track the original (possibly shifted) character for alternate key / text reporting.
    let original_char: Option<char> = match keystroke.key.as_str() {
        key if key.chars().count() == 1 => key.chars().next(),
        _ => None,
    };

    // Map keys to their key codes following the Kitty protocol specification.
    let key_code: u32 = match keystroke.key.as_str() {
        // Control characters (C0 codes)
        "enter" => 13,
        "tab" => 9,
        "escape" => 27,
        "backspace" => 127,
        "space" => 32,

        // Function keys F13-F35 (Kitty-specific codes).
        // F1-F12 are intentionally omitted: the Kitty spec keeps their legacy encoding
        // format (SS3 P/Q/R/S for F1-F4, CSI <code> ~ for F5-F12) rather than CSI u.
        // They fall through to the legacy escape sequence encoding which already handles
        // modifier encoding correctly (CSI 1;mod P/Q/R/S for F1-F4 with modifiers).
        "f13" => 57376,
        "f14" => 57377,
        "f15" => 57378,
        "f16" => 57379,
        "f17" => 57380,
        "f18" => 57381,
        "f19" => 57382,
        "f20" => 57383,
        "f21" => 57384,
        "f22" => 57385,
        "f23" => 57386,
        "f24" => 57387,
        "f25" => 57388,
        "f26" => 57389,
        "f27" => 57390,
        "f28" => 57391,
        "f29" => 57392,
        "f30" => 57393,
        "f31" => 57394,
        "f32" => 57395,
        "f33" => 57396,
        "f34" => 57397,
        "f35" => 57398,

        // For single printable characters, use the platform-provided base key
        // (without any modifiers) to get the correct Unicode codepoint.
        // Falls back to lowercasing ASCII letters when platform info isn't available.
        key if key.chars().count() == 1 => {
            if let Some(base) = key_without_modifiers.and_then(|k| k.chars().next()) {
                // Platform provided the unmodified key (e.g., '1' for Shift+1 on US layout)
                // CapsLock can cause key_without_modifiers to still report an
                // uppercase letter, so normalise to lowercase for the key code.
                let base = base.to_ascii_lowercase();
                base as u32
            } else {
                // No platform info available (e.g., tests, WASM). Lowercase ASCII letters
                // since that mapping is universal, but use the key as-is for symbols.
                let c = key.chars().next()?;
                if c.is_ascii_uppercase() {
                    c.to_ascii_lowercase() as u32
                } else {
                    c as u32
                }
            }
        }

        // Unsupported keys
        _ => return None,
    };

    // Build the key code portion: `key_code[:shifted_key]`
    // Per spec: "the shifted key must be present only if shift is also present in the modifiers"
    let alternate_key_code = if report_alternate && keystroke.shift {
        original_char.and_then(|c| {
            let shifted = c as u32;
            // Only include alternate if it differs from the base key code.
            if shifted != key_code {
                Some(shifted)
            } else {
                None
            }
        })
    } else {
        None
    };

    let key_part = match alternate_key_code {
        Some(alt) => format!("{key_code}:{alt}"),
        None => key_code.to_string(),
    };

    // Calculate modifier value per the Kitty protocol.
    // Kitty modifier bits: shift=1, alt=2, ctrl=4, super=8, hyper=16, meta=32
    // The wire value is 1 + (sum of active modifier bits).
    //
    // Keystroke field mapping:
    //   keystroke.alt  → Alt bit (2): raw Option key on macOS, Alt on other platforms
    //   keystroke.meta → Alt bit (2): "Option-as-Meta" on macOS (terminal Alt)
    //   keystroke.cmd  → Super bit (8): Cmd on macOS, Super/Win on other platforms
    //
    // Both `alt` and `meta` map to the Kitty Alt bit because they both represent
    // the terminal concept of Alt — `meta` is just the macOS user preference that
    // remaps Option to behave as a terminal Meta/Alt key. They cannot both be true
    // simultaneously in practice (Option is either raw or Meta, never both).
    let mut modifiers = 1u32;
    if keystroke.shift {
        modifiers += 1;
    }
    if keystroke.alt || keystroke.meta {
        modifiers += 2;
    }
    if keystroke.ctrl {
        modifiers += 4;
    }
    if keystroke.cmd {
        modifiers += 8;
    }

    // Compute associated text if REPORT_ASSOCIATED_TEXT is active.
    // Per spec: "The associated text must not contain control codes (control codes are code
    // points below U+0020 and codepoints in the C0 and C1 blocks)."
    let associated_text: Option<String> = if report_text {
        chars
            .filter(|text| !text.is_empty() && !text.chars().any(|c| c.is_control()))
            .map(|text| {
                text.chars()
                    .map(|c| (c as u32).to_string())
                    .collect::<Vec<_>>()
                    .join(":")
            })
    } else {
        None
    };

    // Per the Kitty spec, event type 1 (press) is the default and can be omitted.
    // We only handle press events for regular keystrokes here; repeat/release events
    // are not yet plumbed through KeystrokeWithDetails. Modifier key press/release
    // events are handled separately in modifier_key_to_csi_u.
    let modifier_part = modifiers.to_string();

    // Build the full sequence: CSI key_part [; modifiers [; text]] u
    // When associated text is present, the modifiers field must always be included
    // (even if it's 1) so the text field is correctly positioned.
    let sequence = if let Some(text) = &associated_text {
        format!("\x1b[{key_part};{modifier_part};{text}u")
    } else if modifiers > 1 {
        format!("\x1b[{key_part};{modifier_part}u")
    } else {
        format!("\x1b[{key_part}u")
    };

    log::debug!(
        "Generated CSI u sequence for key '{}': {}",
        keystroke.key,
        sequence.escape_default()
    );
    Some(sequence.into_bytes())
}

/// Encodes a modifier key press/release to a CSI u escape sequence for the Kitty keyboard protocol.
///
/// This is used when REPORT_ALL_KEYS_AS_ESC mode is active to report standalone modifier key
/// press and release events. The format follows the Kitty protocol specification:
/// - Format: CSI <key_code> ; <modifiers> [: <event_type>] u
/// - Modifier key codes: ShiftLeft=57441, ControlLeft=57442, AltLeft=57443, SuperLeft=57444,
///   ShiftRight=57447, ControlRight=57448, AltRight=57449, SuperRight=57450
/// - Event types: 1=press, 2=repeat, 3=release
///
/// Returns None if the key is not a modifier key.
pub fn modifier_key_to_csi_u(
    key_code: &KeyCode,
    is_press: bool,
    report_event_types: bool,
) -> Option<Vec<u8>> {
    let kitty_key_code = match key_code {
        KeyCode::ShiftLeft => 57441,
        KeyCode::ControlLeft => 57442,
        KeyCode::AltLeft => 57443,
        KeyCode::SuperLeft => 57444,
        KeyCode::ShiftRight => 57447,
        KeyCode::ControlRight => 57448,
        KeyCode::AltRight => 57449,
        KeyCode::SuperRight => 57450,
        KeyCode::CapsLock => 57358,
        KeyCode::NumLock => 57360,
        _ => return None,
    };

    // Per the Kitty spec, when a modifier key is pressed alone, its own modifier
    // bit must be included (the "self" bit). For example, pressing Left Shift
    // produces modifiers = 1 + 1 = 2 (base 1 + shift bit 1).
    let modifiers = 1u32
        + match key_code {
            KeyCode::ShiftLeft | KeyCode::ShiftRight => 1,
            KeyCode::AltLeft | KeyCode::AltRight => 2,
            KeyCode::ControlLeft | KeyCode::ControlRight => 4,
            KeyCode::SuperLeft | KeyCode::SuperRight => 8,
            _ => 0,
        };

    let sequence = if report_event_types {
        // Event type: 1 = press, 3 = release
        let event_type = if is_press { 1 } else { 3 };
        // With event types, we need the modifier field for the colon syntax
        format!("\x1b[{};{}:{}u", kitty_key_code, modifiers, event_type)
    } else {
        // Without event type reporting, only report press events
        if !is_press {
            return None;
        }
        // Modifiers always > 1 for modifier keys due to the self-bit
        format!("\x1b[{};{}u", kitty_key_code, modifiers)
    };

    log::debug!(
        "Generated CSI u sequence for modifier key {:?}: {}",
        key_code,
        sequence.escape_default()
    );
    Some(sequence.into_bytes())
}

/// Returns a CSI u escape sequence for a modifier key event if the terminal mode requires it.
///
/// Checks whether the REPORT_ALL_KEYS_AS_ESCAPE flag is active (which means standalone
/// modifier key presses should be reported) and, if so, encodes the modifier key event.
/// Returns `None` if the mode is not active or the key is not a modifier key.
pub fn maybe_kitty_keyboard_escape_sequence(
    mode_provider: &dyn ModeProvider,
    key_code: &KeyCode,
    is_press: bool,
) -> Option<Vec<u8>> {
    if !mode_provider.is_term_mode_set(TermMode::KEYBOARD_REPORT_ALL_AS_ESCAPE) {
        return None;
    }

    let report_event_types = mode_provider.is_term_mode_set(TermMode::KEYBOARD_REPORT_EVENT_TYPES);
    modifier_key_to_csi_u(key_code, is_press, report_event_types)
}
