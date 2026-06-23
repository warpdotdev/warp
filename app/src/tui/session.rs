//! Key encoding for the TUI surface.
//!
//! The TUI's terminal session is owned by a `TerminalManager<RootTuiView>`
//! (see [`crate::tui`]); this module only holds the keystroke→PTY-bytes encoder
//! the root view uses when forwarding keys to a running command or the
//! alt-screen.

use parking_lot::FairMutex;
use warpui_core::keymap::Keystroke;

use crate::terminal::model::terminal_model::TerminalModel;

/// Encodes a `KeyDown` event into PTY bytes using the GUI's shared encoder.
///
/// Builds a [`KeystrokeWithDetails`] and calls `to_escape_sequence` with the
/// model as the mode provider. Returns `Some(bytes)` for keys with an escape
/// sequence, or `None` for plain printable input (caller should write `chars`
/// as UTF-8).
///
/// On macOS, `NSEvent` sets `chars` to the control code for Ctrl+letter (e.g.
/// Ctrl-C → `"\x03"`). Crossterm instead sets `chars` to the letter itself
/// (`"c"`), so we compute the C0 control code here as a fallback. Similarly,
/// Enter/Tab/Escape have empty `chars` in crossterm but should produce their
/// control codes (CR/HT/ESC).
pub(crate) fn encode_keydown(
    keystroke: &Keystroke,
    key_without_modifiers: Option<&str>,
    chars: &str,
    model: &FairMutex<TerminalModel>,
) -> Option<Vec<u8>> {
    use crate::terminal::model::escape_sequences::{KeystrokeWithDetails, ToEscapeSequence};

    let details = KeystrokeWithDetails {
        keystroke,
        key_without_modifiers,
        chars: if chars.is_empty() { None } else { Some(chars) },
    };
    let guard = model.lock();
    details
        .to_escape_sequence(&*guard)
        .or_else(|| fallback_control_bytes(keystroke, chars))
}

/// Computes PTY bytes for keys that `to_escape_sequence` doesn't handle but
/// crossterm's event conversion doesn't map to `chars` either.
fn fallback_control_bytes(keystroke: &Keystroke, chars: &str) -> Option<Vec<u8>> {
    // Ctrl+letter → C0 control code (letter - 'a' + 1).
    if keystroke.ctrl
        && !keystroke.alt
        && !keystroke.shift
        && !keystroke.meta
        && keystroke.key.len() == 1
    {
        let c = keystroke.key.chars().next()?;
        if c.is_ascii_alphabetic() {
            return Some(vec![c.to_ascii_lowercase() as u8 - b'a' + 1]);
        }
    }

    // Special keys with empty `chars` that should produce control codes.
    if chars.is_empty() {
        return match keystroke.key.as_str() {
            "enter" => Some(vec![0x0d]),
            "tab" => Some(vec![0x09]),
            "escape" => Some(vec![0x1b]),
            _ => None,
        };
    }

    None
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
