use std::slice;

use cocoa::base::{id, BOOL};
use cocoa::foundation::NSUInteger;
use objc2::rc::Retained;
use objc2_foundation::{NSArray, NSNumber, NSString};
use warpui_core::keymap::Keystroke;
use warpui_core::platform::keyboard::{KeyCode, NativeKeyCode, PhysicalKey};

// Modifier key mask values for the Carbon API.
pub const CMD_KEY: u16 = 256;
pub const SHIFT_KEY: u16 = 512;
pub const OPTION_KEY: u16 = 2048;
pub const CONTROL_KEY: u16 = 4096;

extern "C" {
    fn charToKeyCodes(keyChar: id) -> id;
    fn keyCodeToChar(keyCode: NSUInteger, shifted: BOOL) -> id;
}

pub struct Keycode(pub u16);

impl Keycode {
    pub fn try_to_key_name(self, shift_key_pressed: bool) -> Option<String> {
        unsafe {
            // The underlying core-foundation library interprets objc BOOL type as bool
            // in aarch machines but as i8 in intel machines so we need to call .into here.
            // But clippy isn't smart enough to know that so we silence it here for now.
            #[allow(clippy::useless_conversion)]
            let key = keyCodeToChar(self.0 as u64, shift_key_pressed.into());

            if key.is_null() {
                return None;
            }

            let key = &*key.cast::<NSString>();
            let cstr = key.UTF8String() as *const u8;
            std::str::from_utf8(slice::from_raw_parts(cstr, key.len()))
                .ok()
                .map(|s| s.to_string())
        }
    }

    // There could have multiple keycodes mapping to one virtual key. Return an iterator
    // to all possible values of keycode here.
    pub fn keycodes_from_key_name(key_name: &str) -> impl Iterator<Item = Keycode> {
        let key_name = NSString::from_str(key_name);
        // `charToKeyCodes` borrows the string only for the duration of the call
        // and returns an autoreleased array of NSNumber keycodes.
        let keycodes: *const NSArray<NSNumber> =
            unsafe { charToKeyCodes(Retained::as_ptr(&key_name) as id) }.cast();
        let keycodes_length = unsafe { (*keycodes).count() };

        (0..keycodes_length).map(move |i| {
            let keycode = unsafe { (*keycodes).objectAtIndex(i).unsignedIntegerValue() };
            Self(keycode as u16)
        })
    }
}

// Convert modifier flags to Carbon style modifier key mask.
pub fn modifier_code(keystroke: &Keystroke) -> u16 {
    let mut code = 0;
    if keystroke.alt {
        code |= OPTION_KEY;
    }

    if keystroke.cmd {
        code |= CMD_KEY;
    }

    if keystroke.shift {
        code |= SHIFT_KEY;
    }

    if keystroke.ctrl {
        code |= CONTROL_KEY;
    }

    code
}

// The following types and functions are taken from winit's appkit implementation.
// We redefine them here to avoid needing to include the entirety of winit as a dependency for MacOS.
// --------------------------------------------------------------------------------------------------------

/// Converts a scancode to a physical key. Logic is taken from winit appkit code.
pub(crate) fn scancode_to_physicalkey(scancode: u32) -> PhysicalKey {
    // Follows what Chromium and Firefox do:
    // https://chromium.googlesource.com/chromium/src.git/+/3e1a26c44c024d97dc9a4c09bbc6a2365398ca2c/ui/events/keycodes/dom/dom_code_data.inc
    // https://searchfox.org/mozilla-central/rev/c597e9c789ad36af84a0370d395be066b7dc94f4/widget/NativeKeyToDOMCodeName.h
    //
    // See also:
    // Carbon.framework/Versions/A/Frameworks/HIToolbox.framework/Versions/A/Headers/Events.h
    //
    // Also see https://developer.apple.com/documentation/appkit/function-key-unicode-values:
    //
    // > the system handles some function keys at a lower level and your app never sees them.
    // > Examples include the Volume Up key, Volume Down key, Volume Mute key, Eject key, and
    // > Function key found on many Macs.
    //
    // So the handling of some of these is mostly for show.
    PhysicalKey::Code(match scancode {
        0x00 => KeyCode::KeyA,
        0x01 => KeyCode::KeyS,
        0x02 => KeyCode::KeyD,
        0x03 => KeyCode::KeyF,
        0x04 => KeyCode::KeyH,
        0x05 => KeyCode::KeyG,
        0x06 => KeyCode::KeyZ,
        0x07 => KeyCode::KeyX,
        0x08 => KeyCode::KeyC,
        0x09 => KeyCode::KeyV,
        // This key is typically located near LeftShift key, roughly the same location as backquote
        // (`) on Windows' US layout.
        //
        // The keycap varies on international keyboards.
        0x0a => KeyCode::IntlBackslash,
        0x0b => KeyCode::KeyB,
        0x0c => KeyCode::KeyQ,
        0x0d => KeyCode::KeyW,
        0x0e => KeyCode::KeyE,
        0x0f => KeyCode::KeyR,
        0x10 => KeyCode::KeyY,
        0x11 => KeyCode::KeyT,
        0x12 => KeyCode::Digit1,
        0x13 => KeyCode::Digit2,
        0x14 => KeyCode::Digit3,
        0x15 => KeyCode::Digit4,
        0x16 => KeyCode::Digit6,
        0x17 => KeyCode::Digit5,
        0x18 => KeyCode::Equal,
        0x19 => KeyCode::Digit9,
        0x1a => KeyCode::Digit7,
        0x1b => KeyCode::Minus,
        0x1c => KeyCode::Digit8,
        0x1d => KeyCode::Digit0,
        0x1e => KeyCode::BracketRight,
        0x1f => KeyCode::KeyO,
        0x20 => KeyCode::KeyU,
        0x21 => KeyCode::BracketLeft,
        0x22 => KeyCode::KeyI,
        0x23 => KeyCode::KeyP,
        0x24 => KeyCode::Enter,
        0x25 => KeyCode::KeyL,
        0x26 => KeyCode::KeyJ,
        0x27 => KeyCode::Quote,
        0x28 => KeyCode::KeyK,
        0x29 => KeyCode::Semicolon,
        0x2a => KeyCode::Backslash,
        0x2b => KeyCode::Comma,
        0x2c => KeyCode::Slash,
        0x2d => KeyCode::KeyN,
        0x2e => KeyCode::KeyM,
        0x2f => KeyCode::Period,
        0x30 => KeyCode::Tab,
        0x31 => KeyCode::Space,
        0x32 => KeyCode::Backquote,
        0x33 => KeyCode::Backspace,
        // 0x34 => unknown, // kVK_Powerbook_KeypadEnter
        0x35 => KeyCode::Escape,
        0x36 => KeyCode::SuperRight,
        0x37 => KeyCode::SuperLeft,
        0x38 => KeyCode::ShiftLeft,
        0x39 => KeyCode::CapsLock,
        0x3a => KeyCode::AltLeft,
        0x3b => KeyCode::ControlLeft,
        0x3c => KeyCode::ShiftRight,
        0x3d => KeyCode::AltRight,
        0x3e => KeyCode::ControlRight,
        0x3f => KeyCode::Fn,
        0x40 => KeyCode::F17,
        0x41 => KeyCode::NumpadDecimal,
        // 0x42 -> unknown,
        0x43 => KeyCode::NumpadMultiply,
        // 0x44 => unknown,
        0x45 => KeyCode::NumpadAdd,
        // 0x46 => unknown,
        0x47 => KeyCode::NumLock, // kVK_ANSI_KeypadClear
        0x48 => KeyCode::AudioVolumeUp,
        0x49 => KeyCode::AudioVolumeDown,
        0x4a => KeyCode::AudioVolumeMute,
        0x4b => KeyCode::NumpadDivide,
        0x4c => KeyCode::NumpadEnter,
        // 0x4d => unknown,
        0x4e => KeyCode::NumpadSubtract,
        0x4f => KeyCode::F18,
        0x50 => KeyCode::F19,
        0x51 => KeyCode::NumpadEqual,
        0x52 => KeyCode::Numpad0,
        0x53 => KeyCode::Numpad1,
        0x54 => KeyCode::Numpad2,
        0x55 => KeyCode::Numpad3,
        0x56 => KeyCode::Numpad4,
        0x57 => KeyCode::Numpad5,
        0x58 => KeyCode::Numpad6,
        0x59 => KeyCode::Numpad7,
        0x5a => KeyCode::F20,
        0x5b => KeyCode::Numpad8,
        0x5c => KeyCode::Numpad9,
        0x5d => KeyCode::IntlYen,
        0x5e => KeyCode::IntlRo,
        0x5f => KeyCode::NumpadComma,
        0x60 => KeyCode::F5,
        0x61 => KeyCode::F6,
        0x62 => KeyCode::F7,
        0x63 => KeyCode::F3,
        0x64 => KeyCode::F8,
        0x65 => KeyCode::F9,
        0x66 => KeyCode::Lang2,
        0x67 => KeyCode::F11,
        0x68 => KeyCode::Lang1,
        0x69 => KeyCode::F13,
        0x6a => KeyCode::F16,
        0x6b => KeyCode::F14,
        // 0x6c => unknown,
        0x6d => KeyCode::F10,
        0x6e => KeyCode::ContextMenu,
        0x6f => KeyCode::F12,
        // 0x70 => unknown,
        0x71 => KeyCode::F15,
        0x72 => KeyCode::Insert,
        0x73 => KeyCode::Home,
        0x74 => KeyCode::PageUp,
        0x75 => KeyCode::Delete,
        0x76 => KeyCode::F4,
        0x77 => KeyCode::End,
        0x78 => KeyCode::F2,
        0x79 => KeyCode::PageDown,
        0x7a => KeyCode::F1,
        0x7b => KeyCode::ArrowLeft,
        0x7c => KeyCode::ArrowRight,
        0x7d => KeyCode::ArrowDown,
        0x7e => KeyCode::ArrowUp,
        0x7f => KeyCode::Power, // On 10.7 and 10.8 only
        _ => return PhysicalKey::Unidentified(NativeKeyCode::MacOS(scancode as u16)),
    })
}
