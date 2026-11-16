//! Shared X11 keysym utilities for Linux keyboard input.
//!
//! Both X11 and Wayland (via the RemoteDesktop portal) use X11 keysyms
//! for keyboard input, so this module provides common conversion functions.

use std::ops::RangeInclusive;

/// Keysym range for uppercase ASCII letters (A-Z).
pub const UPPERCASE_KEYSYMS: RangeInclusive<u32> = 0x41..=0x5A;

/// X11 keysym for left shift key.
pub const XK_SHIFT_L: u32 = 0xFFE1;

/// Converts a Unicode character to an X11 keysym.
pub fn char_to_keysym(ch: char) -> u32 {
    let code = ch as u32;

    // ASCII characters map directly to keysyms for the printable range.
    if (0x20..=0x7E).contains(&code) {
        return code;
    }

    // Latin-1 supplement (0x80-0xFF) also maps directly.
    if (0xA0..=0xFF).contains(&code) {
        return code;
    }

    // For other Unicode characters, X11 uses the Unicode value + 0x01000000.
    0x01000000 | code
}

/// Returns true if the keysym requires shift to be pressed.
///
/// This is a simple heuristic based on uppercase letters. Callers with access
/// to keyboard mapping data may want to use more sophisticated logic.
pub fn keysym_needs_shift(keysym: u32) -> bool {
    UPPERCASE_KEYSYMS.contains(&keysym)
}
