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

    // Capture the layout-independent physical key code (e.g. "KeyC") *before*
    // we move `input.logical_key` below. Used by the matcher to support
    // "match by physical key" bindings.
    let physical_code = match input.physical_key {
        winit::keyboard::PhysicalKey::Code(code) => {
            convert_winit_key_code_to_warp(code).map(|c| {
                crate::platform::keyboard::physical_key_to_string(c)
            })
        }
        winit::keyboard::PhysicalKey::Unidentified(_) => None,
    };

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
        physical_code,
    })
}

/// Convert a `winit::keyboard::KeyCode` to our internal
/// `warpui_core::platform::keyboard::KeyCode`. The two enums share the same
/// variant names (W3C UIEvents code) so a `Debug`-string round-trip is
/// reliable, but we use a small explicit match for the common letter and
/// digit keys to avoid the allocation - the matcher's hot path runs through
/// here on every keypress.
fn convert_winit_key_code_to_warp(
    code: winit::keyboard::KeyCode,
) -> Option<crate::platform::keyboard::KeyCode> {
    use crate::platform::keyboard::KeyCode as Warp;
    use winit::keyboard::KeyCode as Wk;
    Some(match code {
        Wk::Backquote => Warp::Backquote,
        Wk::Backslash => Warp::Backslash,
        Wk::BracketLeft => Warp::BracketLeft,
        Wk::BracketRight => Warp::BracketRight,
        Wk::Comma => Warp::Comma,
        Wk::Digit0 => Warp::Digit0,
        Wk::Digit1 => Warp::Digit1,
        Wk::Digit2 => Warp::Digit2,
        Wk::Digit3 => Warp::Digit3,
        Wk::Digit4 => Warp::Digit4,
        Wk::Digit5 => Warp::Digit5,
        Wk::Digit6 => Warp::Digit6,
        Wk::Digit7 => Warp::Digit7,
        Wk::Digit8 => Warp::Digit8,
        Wk::Digit9 => Warp::Digit9,
        Wk::Equal => Warp::Equal,
        Wk::IntlBackslash => Warp::IntlBackslash,
        Wk::IntlRo => Warp::IntlRo,
        Wk::IntlYen => Warp::IntlYen,
        Wk::KeyA => Warp::KeyA,
        Wk::KeyB => Warp::KeyB,
        Wk::KeyC => Warp::KeyC,
        Wk::KeyD => Warp::KeyD,
        Wk::KeyE => Warp::KeyE,
        Wk::KeyF => Warp::KeyF,
        Wk::KeyG => Warp::KeyG,
        Wk::KeyH => Warp::KeyH,
        Wk::KeyI => Warp::KeyI,
        Wk::KeyJ => Warp::KeyJ,
        Wk::KeyK => Warp::KeyK,
        Wk::KeyL => Warp::KeyL,
        Wk::KeyM => Warp::KeyM,
        Wk::KeyN => Warp::KeyN,
        Wk::KeyO => Warp::KeyO,
        Wk::KeyP => Warp::KeyP,
        Wk::KeyQ => Warp::KeyQ,
        Wk::KeyR => Warp::KeyR,
        Wk::KeyS => Warp::KeyS,
        Wk::KeyT => Warp::KeyT,
        Wk::KeyU => Warp::KeyU,
        Wk::KeyV => Warp::KeyV,
        Wk::KeyW => Warp::KeyW,
        Wk::KeyX => Warp::KeyX,
        Wk::KeyY => Warp::KeyY,
        Wk::KeyZ => Warp::KeyZ,
        Wk::Minus => Warp::Minus,
        Wk::Period => Warp::Period,
        Wk::Quote => Warp::Quote,
        Wk::Semicolon => Warp::Semicolon,
        Wk::Slash => Warp::Slash,
        Wk::Enter => Warp::Enter,
        Wk::Space => Warp::Space,
        Wk::Tab => Warp::Tab,
        Wk::Backspace => Warp::Backspace,
        Wk::Delete => Warp::Delete,
        Wk::Escape => Warp::Escape,
        Wk::ArrowDown => Warp::ArrowDown,
        Wk::ArrowLeft => Warp::ArrowLeft,
        Wk::ArrowRight => Warp::ArrowRight,
        Wk::ArrowUp => Warp::ArrowUp,
        Wk::Home => Warp::Home,
        Wk::End => Warp::End,
        Wk::PageDown => Warp::PageDown,
        Wk::PageUp => Warp::PageUp,
        Wk::F1 => Warp::F1,
        Wk::F2 => Warp::F2,
        Wk::F3 => Warp::F3,
        Wk::F4 => Warp::F4,
        Wk::F5 => Warp::F5,
        Wk::F6 => Warp::F6,
        Wk::F7 => Warp::F7,
        Wk::F8 => Warp::F8,
        Wk::F9 => Warp::F9,
        Wk::F10 => Warp::F10,
        Wk::F11 => Warp::F11,
        Wk::F12 => Warp::F12,
        // Anything not on the alphanumeric/symbol/navigation/F-key fast path
        // doesn't need physical-key matching today - return None and let the
        // matcher fall back to the logical-key path.
        _ => return None,
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

#[cfg(test)]
#[path = "key_events_tests.rs"]
mod tests;
