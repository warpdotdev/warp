use std::borrow::Cow;
use std::collections::HashMap;

use lazy_static::lazy_static;

use winit::event::ElementState;
#[cfg(windows)]
use winit::keyboard::NativeKey;
use winit::keyboard::{Key, ModifiersState, NamedKey};
#[cfg(not(target_family = "wasm"))]
use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;

use crate::platform::KEYS_TO_IGNORE;
use crate::{event::KeyEventDetails, keymap::Keystroke};

use super::WindowState;

lazy_static! {
    /// Mapping between a printable ASCII character and its corresponding control code had `ctrl`
    /// been pressed. For example: `ctrl-c` corresponds to the `^C` control code, which has an ASCII
    /// value of 03. See <https://www.geeksforgeeks.org/control-characters/> for more details.
    static ref CONTROL_CHARACTER_MAP: HashMap<&'static str, &'static str> = HashMap::from_iter([
        ("@", "\x00"),
        ("a", "\x01"),
        ("b", "\x02"),
        ("c", "\x03"),
        ("d", "\x04"),
        ("e", "\x05"),
        ("f", "\x06"),
        ("g", "\x07"),
        ("h", "\x08"),
        ("i", "\x09"),
        ("j", "\x0A"),
        ("k", "\x0B"),
        ("l", "\x0C"),
        ("m", "\x0D"),
        ("n", "\x0E"),
        ("o", "\x0F"),
        ("p", "\x10"),
        ("q", "\x11"),
        ("r", "\x12"),
        ("s", "\x13"),
        ("t", "\x14"),
        ("u", "\x15"),
        ("v", "\x16"),
        ("w", "\x17"),
        ("x", "\x18"),
        ("y", "\x19"),
        ("z", "\x1A"),
        ("[", "\x1B"),
        ("\\", "\x1C"),
        ("]", "\x1D"),
        ("^", "\x1E"),
        ("_", "\x1F"),
    ]);
}

/// Converts a KeyboardInput event to a UI framework event, returning None
/// if no UI framework event should be emitted.
pub fn convert_keyboard_input_event(
    input: winit::event::KeyEvent,
    window_state: &WindowState,
    is_synthetic: bool,
) -> Option<crate::Event> {
    if input.state != ElementState::Pressed {
        return None;
    }

    // Ignore any synthetic keypresses that winit generated for keys that were
    // already pressed when a window gained focus.  Three examples of how these
    // cause problems:
    // 1. An alt-tab to a window can end up inserting a tab into the input if
    //    alt is released before tab.
    // 2. Using a keyboard shortcut to open a new window can open many new
    //    windows, as the new window will receive a synthetic event for the
    //    shortcut that opened it, opening _another_ new window, and so on.
    // 3. The ctrl-d shortcut for sending an EOF to the shell can end up
    //    being sent to additional sessions if there was ony one session in
    //    the window, as it will close the window and then be synthetically
    //    generated for the next window in the stack.
    if is_synthetic {
        return None;
    }

    let chars = text_with_modifiers(&input, window_state.modifiers)
        .unwrap_or_default()
        .to_owned();

    let key_without_modifiers = get_key_without_modifiers(&input);

    let shift = window_state.modifiers.shift_key();

    let logical_key = match &input.logical_key {
        // When keystrokes with ctrl-alt are pressed on Windows, `input.logical_key` is
        // Unidentified.
        #[cfg(windows)]
        Key::Unidentified(NativeKey::Windows(_))
            if window_state
                .modifiers
                .contains(ModifiersState::CONTROL | ModifiersState::ALT) =>
        {
            input.key_without_modifiers()
        }
        // On Windows, non-Latin keyboard layouts (Cyrillic, Greek, Arabic, etc.) translate
        // the physical key to a non-ASCII character even when Ctrl/Cmd is held. That makes
        // bindings like `ctrl-c` / `ctrl-v` fail to match. Fall back to the US-QWERTY
        // position so chord shortcuts work regardless of the active layout — same approach
        // used by VS Code, JetBrains, and Chromium. Issue #9036.
        //
        // Right-Alt is excluded because Windows reports AltGr as Ctrl+Alt; without this
        // guard, AltGr-produced characters (e.g. `€` on a German layout) would be rewritten
        // into a spurious chord and the typed character would be swallowed.
        #[cfg(windows)]
        Key::Character(c)
            if (window_state.modifiers.control_key() || window_state.modifiers.super_key())
                && !window_state.right_alt_pressed
                && !c.is_ascii() =>
        {
            us_qwerty_fallback_for_chord(&input.physical_key, shift)
                .map(|s| Key::Character(s.into()))
                .unwrap_or_else(|| input.logical_key.clone())
        }
        _ => input.logical_key,
    };
    let input_key = get_input_key(&logical_key, shift);

    let key = convert_key(input_key)?.to_string();

    let keystroke = Keystroke {
        ctrl: window_state.modifiers.control_key(),
        alt: window_state.modifiers.alt_key(),
        shift,
        cmd: window_state.modifiers.super_key(),
        meta: false,
        key,
    };

    // Ignore any keystrokes that we're purposefully not handling. (I.e. cmdorctrl-v needs to fall back
    // to the browser implementation on the web.)
    if KEYS_TO_IGNORE.contains(&keystroke) {
        return None;
    }

    Some(crate::event::Event::KeyDown {
        keystroke,
        chars,
        details: KeyEventDetails {
            left_alt: window_state.left_alt_pressed,
            right_alt: window_state.right_alt_pressed,
            key_without_modifiers,
        },
        is_composing: false,
    })
}

#[cfg(not(target_family = "wasm"))]
/// Returns the base key without any modifiers applied, or `None` if it cannot be determined.
fn get_key_without_modifiers(input: &winit::event::KeyEvent) -> Option<String> {
    let unmodified = input.key_without_modifiers();
    let unmodified_input = get_input_key(&unmodified, false);
    convert_key(unmodified_input).map(|k| k.to_string())
}

#[cfg(target_family = "wasm")]
fn get_key_without_modifiers(_input: &winit::event::KeyEvent) -> Option<String> {
    None
}

#[cfg(not(target_family = "wasm"))]
/// Returns the text of the [`winit::event::KeyEvent`] with the characters modified by `ctrl`.
/// For example,  `Ctrl+a` produces `Some("\x01")`.
fn text_with_modifiers(
    key_event: &winit::event::KeyEvent,
    _modifier_state: ModifiersState,
) -> Option<&str> {
    key_event.text_with_all_modifiers()
}

#[cfg(target_family = "wasm")]
fn text_with_modifiers(
    key_event: &winit::event::KeyEvent,
    modifier_state: ModifiersState,
) -> Option<&str> {
    // Provide the bare-minimum amount of support for mapping modifiers to their corresponding
    // ASCII character. This is not actually fully functional because keys like `@` require the
    // addition of the `SHIFT` key, which doesn't yet work here.
    // TODO(wasm): Extend this to support all of the function/shift/arrow keys.
    match (modifier_state, &key_event.logical_key) {
        (ModifiersState::CONTROL, Key::Character(character))
            if CONTROL_CHARACTER_MAP.contains_key(character.as_str()) =>
        {
            CONTROL_CHARACTER_MAP.get(character.as_str()).copied()
        }
        (_, key) => key.to_text(),
    }
}

fn get_input_key(logical_key: &Key, is_shift: bool) -> Key {
    use winit::keyboard::Key::Character;
    match (logical_key, is_shift) {
        // If the key is a character AND shift is pressed, we force the key to uppercase.
        // If the key is a character AND shift is NOT pressed, we force the key to lowercase.
        // This is to align with existing behavior where we expect bindings with shift
        // to have uppercase characters, and bindings without shift to have lowercase characters.
        // See warpui::keymap::Keystroke::parse and warp::util::bindings::cmd_or_ctrl_shift.
        (Character(character), true) => Character(character.to_uppercase().into()),
        (Character(character), false) => Character(character.to_lowercase().into()),
        (non_char_key, _) => non_char_key.clone(),
    }
}

/// Converts a winit [`winit::keyboard::Key`] to the corresponding string version
/// expected by the UI framework.
fn convert_key(key: Key) -> Option<Cow<'static, str>> {
    use winit::keyboard::Key::*;

    let value = match key {
        Character(char) => return Some(char.to_string().into()),
        Named(NamedKey::Enter) => "enter",
        Named(NamedKey::Tab) => "tab",
        Named(NamedKey::Space) => " ",
        Named(NamedKey::ArrowDown) => "down",
        Named(NamedKey::ArrowLeft) => "left",
        Named(NamedKey::ArrowRight) => "right",
        Named(NamedKey::ArrowUp) => "up",
        Named(NamedKey::End) => "end",
        Named(NamedKey::Home) => "home",
        Named(NamedKey::PageDown) => "pagedown",
        Named(NamedKey::PageUp) => "pageup",
        Named(NamedKey::Backspace) => "backspace",
        Named(NamedKey::Delete) => "delete",
        Named(NamedKey::Insert) => "insert",
        Named(NamedKey::Escape) => "escape",
        Named(NamedKey::F1) => "f1",
        Named(NamedKey::F2) => "f2",
        Named(NamedKey::F3) => "f3",
        Named(NamedKey::F4) => "f4",
        Named(NamedKey::F5) => "f5",
        Named(NamedKey::F6) => "f6",
        Named(NamedKey::F7) => "f7",
        Named(NamedKey::F8) => "f8",
        Named(NamedKey::F9) => "f9",
        Named(NamedKey::F10) => "f10",
        Named(NamedKey::F11) => "f11",
        Named(NamedKey::F12) => "f12",
        Named(NamedKey::F13) => "f13",
        Named(NamedKey::F14) => "f14",
        Named(NamedKey::F15) => "f15",
        Named(NamedKey::F16) => "f16",
        Named(NamedKey::F17) => "f17",
        Named(NamedKey::F18) => "f18",
        Named(NamedKey::F19) => "f19",
        Named(NamedKey::F20) => "f20",
        Named(NamedKey::F21) => "f21",
        Named(NamedKey::F22) => "f22",
        Named(NamedKey::F23) => "f23",
        Named(NamedKey::F24) => "f24",
        Named(NamedKey::F25) => "f25",
        Named(NamedKey::F26) => "f26",
        Named(NamedKey::F27) => "f27",
        Named(NamedKey::F28) => "f28",
        Named(NamedKey::F29) => "f29",
        Named(NamedKey::F30) => "f30",
        Named(NamedKey::F31) => "f31",
        Named(NamedKey::F32) => "f32",
        Named(NamedKey::F33) => "f33",
        Named(NamedKey::F34) => "f34",
        Named(NamedKey::F35) => "f35",
        _ => return None,
    };

    Some(Cow::Borrowed(value))
}

/// Maps a winit `PhysicalKey` to the US-QWERTY character it would produce. Used on Windows
/// to recover layout-independent chord shortcuts (e.g. `ctrl-c`) when a non-Latin keyboard
/// layout has translated the logical key to a non-ASCII character.
///
/// Returns `None` for keys outside the standard letter/digit/punctuation set (function keys,
/// modifiers, navigation keys, etc.), since those either aren't typically used in chord
/// bindings as character keys or are already handled via `NamedKey` in `convert_key`.
#[cfg_attr(not(windows), allow(dead_code))]
fn us_qwerty_fallback_for_chord(
    physical_key: &winit::keyboard::PhysicalKey,
    shift: bool,
) -> Option<&'static str> {
    use winit::keyboard::{KeyCode, PhysicalKey};
    let PhysicalKey::Code(code) = physical_key else {
        return None;
    };
    // Letters always return lowercase here; `get_input_key` applies the uppercase
    // transform downstream when shift is held. Digit/punctuation keys must return
    // the shifted US-QWERTY symbol up front because `get_input_key` passes
    // non-letter characters through unchanged, so bindings like `ctrl-shift-}`
    // would otherwise see `ctrl-shift-]` on non-Latin layouts.
    Some(match (code, shift) {
        (KeyCode::KeyA, _) => "a",
        (KeyCode::KeyB, _) => "b",
        (KeyCode::KeyC, _) => "c",
        (KeyCode::KeyD, _) => "d",
        (KeyCode::KeyE, _) => "e",
        (KeyCode::KeyF, _) => "f",
        (KeyCode::KeyG, _) => "g",
        (KeyCode::KeyH, _) => "h",
        (KeyCode::KeyI, _) => "i",
        (KeyCode::KeyJ, _) => "j",
        (KeyCode::KeyK, _) => "k",
        (KeyCode::KeyL, _) => "l",
        (KeyCode::KeyM, _) => "m",
        (KeyCode::KeyN, _) => "n",
        (KeyCode::KeyO, _) => "o",
        (KeyCode::KeyP, _) => "p",
        (KeyCode::KeyQ, _) => "q",
        (KeyCode::KeyR, _) => "r",
        (KeyCode::KeyS, _) => "s",
        (KeyCode::KeyT, _) => "t",
        (KeyCode::KeyU, _) => "u",
        (KeyCode::KeyV, _) => "v",
        (KeyCode::KeyW, _) => "w",
        (KeyCode::KeyX, _) => "x",
        (KeyCode::KeyY, _) => "y",
        (KeyCode::KeyZ, _) => "z",
        (KeyCode::Digit1, true) => "!",
        (KeyCode::Digit1, false) => "1",
        (KeyCode::Digit2, true) => "@",
        (KeyCode::Digit2, false) => "2",
        (KeyCode::Digit3, true) => "#",
        (KeyCode::Digit3, false) => "3",
        (KeyCode::Digit4, true) => "$",
        (KeyCode::Digit4, false) => "4",
        (KeyCode::Digit5, true) => "%",
        (KeyCode::Digit5, false) => "5",
        (KeyCode::Digit6, true) => "^",
        (KeyCode::Digit6, false) => "6",
        (KeyCode::Digit7, true) => "&",
        (KeyCode::Digit7, false) => "7",
        (KeyCode::Digit8, true) => "*",
        (KeyCode::Digit8, false) => "8",
        (KeyCode::Digit9, true) => "(",
        (KeyCode::Digit9, false) => "9",
        (KeyCode::Digit0, true) => ")",
        (KeyCode::Digit0, false) => "0",
        (KeyCode::Minus, true) => "_",
        (KeyCode::Minus, false) => "-",
        (KeyCode::Equal, true) => "+",
        (KeyCode::Equal, false) => "=",
        (KeyCode::BracketLeft, true) => "{",
        (KeyCode::BracketLeft, false) => "[",
        (KeyCode::BracketRight, true) => "}",
        (KeyCode::BracketRight, false) => "]",
        (KeyCode::Backslash, true) => "|",
        (KeyCode::Backslash, false) => "\\",
        (KeyCode::Semicolon, true) => ":",
        (KeyCode::Semicolon, false) => ";",
        (KeyCode::Quote, true) => "\"",
        (KeyCode::Quote, false) => "'",
        (KeyCode::Comma, true) => "<",
        (KeyCode::Comma, false) => ",",
        (KeyCode::Period, true) => ">",
        (KeyCode::Period, false) => ".",
        (KeyCode::Slash, true) => "?",
        (KeyCode::Slash, false) => "/",
        (KeyCode::Backquote, true) => "~",
        (KeyCode::Backquote, false) => "`",
        _ => return None,
    })
}

#[cfg(test)]
#[path = "key_events_tests.rs"]
mod tests;
