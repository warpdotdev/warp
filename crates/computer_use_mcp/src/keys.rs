//! Translation of key names such as `"cmd+shift+t"` into platform key presses.
//!
//! Ported from warp-server's `logic/ai/computer_use/keycodes`.

use std::fmt;

use computer_use::Key;

/// The key-name conventions of the model family driving the server.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum KeyDialect {
    /// xdotool-style names, where `delete` means Backspace.
    Anthropic,
    /// DOM-style names, where `delete` means forward delete.
    OpenAi,
}

/// The keycode namespace understood by the platform's computer-use actor.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum KeyPlatform {
    /// macOS virtual keycodes.
    Mac,
    /// X11 keysyms, used on both X11 and Wayland.
    Linux,
    /// Windows virtual-key codes.
    Windows,
}

impl KeyPlatform {
    pub fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::Mac
        } else if cfg!(windows) {
            Self::Windows
        } else {
            Self::Linux
        }
    }

    fn keycode(self, key: NamedKey) -> Option<i32> {
        match self {
            Self::Mac => mac_keycode(key),
            Self::Linux => linux_keysym(key),
            Self::Windows => windows_virtual_key(key),
        }
    }
}

/// Keys that have no single-character representation.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum NamedKey {
    Return,
    Tab,
    Space,
    Backspace,
    Escape,
    Super,
    Shift,
    CapsLock,
    Alt,
    Control,
    RightShift,
    RightAlt,
    RightControl,
    Function,
    /// A function key from F1 to F20.
    F(u8),
    VolumeUp,
    VolumeDown,
    Mute,
    Help,
    Home,
    PageUp,
    ForwardDelete,
    End,
    PageDown,
    LeftArrow,
    RightArrow,
    DownArrow,
    UpArrow,
}

/// One preferred name per named key, listed in error messages.
pub(crate) const CANONICAL_KEY_NAMES: &[&str] = &[
    "enter",
    "tab",
    "space",
    "backspace",
    "escape",
    "cmd",
    "shift",
    "shift_r",
    "alt",
    "alt_r",
    "ctrl",
    "ctrl_r",
    "capslock",
    "fn",
    "f1",
    "f2",
    "f3",
    "f4",
    "f5",
    "f6",
    "f7",
    "f8",
    "f9",
    "f10",
    "f11",
    "f12",
    "f13",
    "f14",
    "f15",
    "f16",
    "f17",
    "f18",
    "f19",
    "f20",
    "volume_up",
    "volume_down",
    "mute",
    "help",
    "home",
    "end",
    "page_up",
    "page_down",
    "forward_delete",
    "left",
    "right",
    "up",
    "down",
];

/// A key combination that could not be translated into key presses.
#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct InvalidKeysError {
    /// The full key combination text.
    input: String,
    reason: InvalidKeysReason,
}

#[derive(Debug, Clone, Eq, PartialEq)]
enum InvalidKeysReason {
    Empty,
    UnknownKey(String),
    UnsupportedOnPlatform(String),
}

impl fmt::Display for InvalidKeysError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let input = &self.input;
        match &self.reason {
            InvalidKeysReason::Empty => write!(f, "Invalid keys {input:?}: no keys were given.")?,
            InvalidKeysReason::UnknownKey(key) => {
                write!(f, "Invalid keys {input:?}: {key:?} is not a valid key.")?
            }
            InvalidKeysReason::UnsupportedOnPlatform(key) => write!(
                f,
                "Invalid keys {input:?}: {key:?} cannot be pressed on this platform."
            )?,
        }
        write!(
            f,
            "\n\nKeys must be a single key or a \"+\"-separated key combination (e.g. \"ctrl+s\" \
             or \"cmd+shift+p\"). Each key must be a single character (e.g. \"a\", \"5\", \"/\"; \
             write \"plus\" for \"+\") or a named key (case-insensitive): {}.",
            CANONICAL_KEY_NAMES.join(", ")
        )
    }
}

/// Parses a `+`-separated key combination into the keys to press, in order.
pub(crate) fn parse_chord(
    text: &str,
    platform: KeyPlatform,
    dialect: KeyDialect,
) -> Result<Vec<Key>, InvalidKeysError> {
    let error = |reason| InvalidKeysError {
        input: text.to_owned(),
        reason,
    };
    // Empty segments are skipped so that slightly malformed input like "cmd++c" still works.
    let keys = text
        .split('+')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(|name| resolve_key(name, platform, dialect).map_err(error))
        .collect::<Result<Vec<_>, _>>()?;
    if keys.is_empty() {
        return Err(error(InvalidKeysReason::Empty));
    }
    Ok(keys)
}

/// Returns the key that presses Return on `platform`.
pub(crate) fn return_key(platform: KeyPlatform) -> Key {
    Key::Keycode(
        platform
            .keycode(NamedKey::Return)
            .expect("every platform has a Return key"),
    )
}

fn resolve_key(
    name: &str,
    platform: KeyPlatform,
    dialect: KeyDialect,
) -> Result<Key, InvalidKeysReason> {
    let mut chars = name.chars();
    if let (Some(char), None) = (chars.next(), chars.next()) {
        return Ok(Key::Char(char));
    }
    let lowercase = name.to_lowercase();
    if let Some(char) = named_char(&lowercase) {
        return Ok(Key::Char(char));
    }
    let named_key = named_key(&lowercase, dialect)
        .ok_or_else(|| InvalidKeysReason::UnknownKey(name.to_owned()))?;
    platform
        .keycode(named_key)
        .map(Key::Keycode)
        .ok_or_else(|| InvalidKeysReason::UnsupportedOnPlatform(name.to_owned()))
}

fn named_key(name: &str, dialect: KeyDialect) -> Option<NamedKey> {
    use NamedKey::*;
    let key = match name {
        "return" | "enter" => Return,
        "tab" => Tab,
        "space" => Space,
        "delete" => match dialect {
            KeyDialect::Anthropic => Backspace,
            KeyDialect::OpenAi => ForwardDelete,
        },
        "backspace" => Backspace,
        "escape" | "esc" => Escape,
        "command" | "cmd" | "super" | "win" | "windows" | "meta" => Super,
        "shift" | "shift_l" => Shift,
        "shift_r" => RightShift,
        "option" | "alt" | "option_l" | "alt_l" => Alt,
        "option_r" | "alt_r" => RightAlt,
        "control" | "ctrl" | "control_l" | "ctrl_l" => Control,
        "control_r" | "ctrl_r" => RightControl,
        "capslock" | "caps" => CapsLock,
        "function" | "fn" => Function,
        "volumeup" | "volume_up" => VolumeUp,
        "volumedown" | "volume_down" => VolumeDown,
        "mute" => Mute,
        "help" => Help,
        "home" => Home,
        "end" => End,
        "pageup" | "page_up" | "pgup" => PageUp,
        "pagedown" | "page_down" | "pgdn" | "pgdown" => PageDown,
        "forwarddelete" | "forward_delete" | "del" => ForwardDelete,
        "left" | "leftarrow" | "left_arrow" | "arrowleft" => LeftArrow,
        "right" | "rightarrow" | "right_arrow" | "arrowright" => RightArrow,
        "down" | "downarrow" | "down_arrow" | "arrowdown" => DownArrow,
        "up" | "uparrow" | "up_arrow" | "arrowup" => UpArrow,
        _ => return function_key(name),
    };
    Some(key)
}

fn function_key(name: &str) -> Option<NamedKey> {
    let number = name.strip_prefix('f')?;
    if number.starts_with('0') {
        return None;
    }
    number
        .parse()
        .ok()
        .filter(|number| (1..=20).contains(number))
        .map(NamedKey::F)
}

fn named_char(name: &str) -> Option<char> {
    let char = match name {
        "leftbracket" => '[',
        "rightbracket" => ']',
        "leftparen" => '(',
        "rightparen" => ')',
        "leftbrace" => '{',
        "rightbrace" => '}',
        "backslash" => '\\',
        "forwardslash" | "slash" => '/',
        "semicolon" => ';',
        "colon" => ':',
        "quote" | "apostrophe" => '\'',
        "doublequote" | "quotedbl" => '"',
        "comma" => ',',
        "period" | "dot" => '.',
        "grave" | "backtick" => '`',
        "tilde" => '~',
        "exclam" | "exclamation" => '!',
        "at" => '@',
        "numbersign" | "hash" => '#',
        "dollar" => '$',
        "percent" => '%',
        "caret" | "circumflex" => '^',
        "ampersand" => '&',
        "asterisk" => '*',
        "underscore" => '_',
        "pipe" | "bar" => '|',
        "question" => '?',
        "minus" | "hyphen" | "dash" => '-',
        "equal" | "equals" => '=',
        "plus" => '+',
        "less" => '<',
        "greater" => '>',
        _ => return None,
    };
    Some(char)
}

/// Carbon `kVK_F1` through `kVK_F20`, which are not sequential.
const MAC_FUNCTION_KEYS: [i32; 20] = [
    0x7A, 0x78, 0x63, 0x76, 0x60, 0x61, 0x62, 0x64, 0x65, 0x6D, 0x67, 0x6F, 0x69, 0x6B, 0x71, 0x6A,
    0x40, 0x4F, 0x50, 0x5A,
];

fn function_key_offset(number: u8) -> Option<i32> {
    (1..=20).contains(&number).then_some(i32::from(number) - 1)
}

/// macOS virtual keycodes (Carbon `kVK_*` constants).
fn mac_keycode(key: NamedKey) -> Option<i32> {
    use NamedKey::*;
    let keycode = match key {
        Return => 0x24,
        Tab => 0x30,
        Space => 0x31,
        Backspace => 0x33,
        Escape => 0x35,
        Super => 0x37,
        Shift => 0x38,
        CapsLock => 0x39,
        Alt => 0x3A,
        Control => 0x3B,
        RightShift => 0x3C,
        RightAlt => 0x3D,
        RightControl => 0x3E,
        Function => 0x3F,
        F(number) => MAC_FUNCTION_KEYS[function_key_offset(number)? as usize],
        VolumeUp => 0x48,
        VolumeDown => 0x49,
        Mute => 0x4A,
        Help => 0x72,
        Home => 0x73,
        PageUp => 0x74,
        ForwardDelete => 0x75,
        End => 0x77,
        PageDown => 0x79,
        LeftArrow => 0x7B,
        RightArrow => 0x7C,
        DownArrow => 0x7D,
        UpArrow => 0x7E,
    };
    Some(keycode)
}

/// X11 keysyms (`X11/keysymdef.h` and `XF86keysym.h`).
fn linux_keysym(key: NamedKey) -> Option<i32> {
    use NamedKey::*;
    let keysym = match key {
        Return => 0xff0d,
        Tab => 0xff09,
        Space => 0x0020,
        Backspace => 0xff08,
        Escape => 0xff1b,
        Super => 0xffeb,
        Shift => 0xffe1,
        CapsLock => 0xffe5,
        Alt => 0xffe9,
        Control => 0xffe3,
        RightShift => 0xffe2,
        RightAlt => 0xffea,
        RightControl => 0xffe4,
        // The Fn key is handled by keyboard hardware and is invisible to software on PCs.
        Function => return None,
        F(number) => 0xffbe + function_key_offset(number)?,
        VolumeUp => 0x1008ff13,
        VolumeDown => 0x1008ff11,
        Mute => 0x1008ff12,
        Help => 0xff6a,
        Home => 0xff50,
        PageUp => 0xff55,
        ForwardDelete => 0xffff,
        End => 0xff57,
        PageDown => 0xff56,
        LeftArrow => 0xff51,
        RightArrow => 0xff53,
        DownArrow => 0xff54,
        UpArrow => 0xff52,
    };
    Some(keysym)
}

/// Windows virtual-key codes (`VK_*` constants from `winuser.h`).
fn windows_virtual_key(key: NamedKey) -> Option<i32> {
    use NamedKey::*;
    let virtual_key = match key {
        Return => 0x0D,
        Tab => 0x09,
        Space => 0x20,
        Backspace => 0x08,
        Escape => 0x1B,
        Super => 0x5B,
        // The generic modifier codes act as the left-hand modifier when posting input.
        Shift => 0x10,
        CapsLock => 0x14,
        Alt => 0x12,
        Control => 0x11,
        RightShift => 0xA1,
        RightAlt => 0xA5,
        RightControl => 0xA3,
        // The Fn key is handled by keyboard hardware and is invisible to software.
        Function => return None,
        F(number) => 0x70 + function_key_offset(number)?,
        VolumeUp => 0xAF,
        VolumeDown => 0xAE,
        Mute => 0xAD,
        Help => 0x2F,
        Home => 0x24,
        PageUp => 0x21,
        ForwardDelete => 0x2E,
        End => 0x23,
        PageDown => 0x22,
        LeftArrow => 0x25,
        RightArrow => 0x27,
        DownArrow => 0x28,
        UpArrow => 0x26,
    };
    Some(virtual_key)
}

#[cfg(test)]
#[path = "keys_tests.rs"]
mod tests;
