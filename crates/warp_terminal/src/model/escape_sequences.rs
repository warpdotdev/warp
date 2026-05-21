use std::collections::HashMap;

use lazy_static::lazy_static;
use warpui::keymap::Keystroke;
use warpui::platform::OperatingSystem;

use super::{
    mouse::{MouseAction, MouseButton, MouseState},
    TermMode,
};

mod kitty_keyboard_protocol;

use kitty_keyboard_protocol::maybe_convert_keystroke_to_csi_u;
pub use kitty_keyboard_protocol::{maybe_kitty_keyboard_escape_sequence, modifier_key_to_csi_u};

/// C0 set of 7-bit control characters (from ANSI X3.4-1977).
#[allow(non_snake_case)]
#[allow(dead_code)]
pub mod C0 {
    /// Null filler, terminal should ignore this character.
    pub const NUL: u8 = 0x00;
    /// Start of Header.
    pub const SOH: u8 = 0x01;
    /// Start of Text, implied end of header.
    pub const STX: u8 = 0x02;
    /// End of Text, causes some terminal to respond with ACK or NAK.
    pub const ETX: u8 = 0x03;
    /// End of Transmission.
    pub const EOT: u8 = 0x04;
    /// Enquiry, causes terminal to send ANSWER-BACK ID.
    pub const ENQ: u8 = 0x05;
    /// Acknowledge, usually sent by terminal in response to ETX.
    pub const ACK: u8 = 0x06;
    /// Bell, triggers the bell, buzzer, or beeper on the terminal.
    pub const BEL: u8 = 0x07;
    /// Backspace, can be used to define overstruck characters.
    pub const BS: u8 = 0x08;
    /// Horizontal Tabulation, move to next predetermined position.
    pub const HT: u8 = 0x09;
    /// Linefeed, move to same position on next line (see also NL).
    pub const LF: u8 = 0x0A;
    /// Vertical Tabulation, move to next predetermined line.
    pub const VT: u8 = 0x0B;
    /// Form Feed, move to next form or page.
    pub const FF: u8 = 0x0C;
    /// Carriage Return, move to first character of current line.
    pub const CR: u8 = 0x0D;
    /// Shift Out, switch to G1 (other half of character set).
    pub const SO: u8 = 0x0E;
    /// Shift In, switch to G0 (normal half of character set).
    pub const SI: u8 = 0x0F;
    /// Data Link Escape, interpret next control character specially.
    pub const DLE: u8 = 0x10;
    /// (DC1) Terminal is allowed to resume transmitting.
    pub const XON: u8 = 0x11;
    /// Device Control 2, causes ASR-33 to activate paper-tape reader.
    pub const DC2: u8 = 0x12;
    /// (DC3) Terminal must pause and refrain from transmitting.
    pub const XOFF: u8 = 0x13;
    /// Device Control 4, causes ASR-33 to deactivate paper-tape reader.
    pub const DC4: u8 = 0x14;
    /// Negative Acknowledge, used sometimes with ETX and ACK.
    pub const NAK: u8 = 0x15;
    /// Synchronous Idle, used to maintain timing in Sync communication.
    pub const SYN: u8 = 0x16;
    /// End of Transmission block.
    pub const ETB: u8 = 0x17;
    /// Cancel (makes VT100 abort current escape sequence if any).
    pub const CAN: u8 = 0x18;
    /// End of Medium.
    pub const EM: u8 = 0x19;
    /// Substitute (VT100 uses this to display parity errors).
    pub const SUB: u8 = 0x1A;
    /// Prefix to an escape sequence.
    pub const ESC: u8 = 0x1B;
    /// File Separator.
    pub const FS: u8 = 0x1C;
    /// Group Separator.
    pub const GS: u8 = 0x1D;
    /// Record Separator (sent by VT132 in block-transfer mode).
    pub const RS: u8 = 0x1E;
    /// Unit Separator.
    pub const US: u8 = 0x1F;
    /// Delete, should be ignored by terminal.
    pub const DEL: u8 = 0x7f;
}

/// C1 set of control characters. These are set to their 2-byte equivalent representations (rather
/// than the 8-bit single byte representation).
///
/// See https://www.xfree86.org/current/ctlseqs.html#C1%20(8-Bit)%20Control%20Characters.
#[allow(non_snake_case)]
pub mod C1 {
    use super::C0::ESC;

    /// Index
    pub const IND: &[u8] = &[ESC, b'D'];
    /// Next Line
    pub const NEL: &[u8] = &[ESC, b'E'];
    /// Tab Set
    pub const HTS: &[u8] = &[ESC, b'H'];
    /// Reverse Index
    pub const RI: &[u8] = &[ESC, b'M'];
    /// Single Shift Select of G2 Character Set
    pub const SS2: &[u8] = &[ESC, b'N'];
    /// Single Shift Select of G3 Character Set
    pub const SS3: &[u8] = &[ESC, b'O'];
    /// Device Control String
    pub const DCS: &[u8] = &[ESC, b'P'];
    /// Start of Guarded Area
    pub const SPA: &[u8] = &[ESC, b'V'];
    /// End of Guarded Area
    pub const EPA: &[u8] = &[ESC, b'W'];
    /// Start of String
    pub const SOS: &[u8] = &[ESC, b'X'];
    /// Return Terminal ID
    pub const DECID: &[u8] = &[ESC, b'Z']; //obsolete form of CSI c
    /// Control Sequence Introducer
    pub const CSI: &[u8] = &[ESC, b'['];
    /// String Terminator
    pub const ST: &[u8] = &[ESC, b'\\'];
    /// Operating System Command
    pub const OSC: &[u8] = &[ESC, b']'];
    /// Privacy Message
    pub const PM: &[u8] = &[ESC, b'^'];
    /// Application Program Command
    pub const APC: &[u8] = &[ESC, b'_'];

    /// Converts the given `c1_sequence`, which is expected to be one of the constants defined in
    /// this module, into a string. C1 sequences are ASCII-encoded (which is by definition a subset
    /// of UTF-8), so no need to return an `Option` or check the result of `std::from_utf8()`.
    pub fn to_utf8(c1_sequence: &[u8]) -> &str {
        // We are certain that CSI is valid UTF-8.
        std::str::from_utf8(c1_sequence).expect(
            "Called with an invalid C1 sequence.This method should only be called with C1 \
            sequences defined by constants in the C1 module.",
        )
    }
}

/// Escape sequences used to control 'bracketed paste' mode.
///
/// If the shell supports bracketed paste mode, these control sequences should be inserted at the
/// start and end of text written to the pty. See the[xterm spec](http://www.xfree86.org/current/ctlseqs.html#Bracketed%20Paste%20Mode)
/// for more details.
pub const BRACKETED_PASTE_START: &[u8] = &[C0::ESC, b'[', b'2', b'0', b'0', b'~'];
pub const BRACKETED_PASTE_END: &[u8] = &[C0::ESC, b'[', b'2', b'0', b'1', b'~'];

#[allow(non_snake_case)]
pub mod EscCodes {
    use super::{ModeProvider, TermMode, C0, C1};

    // Arrows-related escape codes
    pub const ARROW_UP: u8 = b'A';
    pub const ARROW_DOWN: u8 = b'B';
    pub const ARROW_RIGHT: u8 = b'C';
    pub const ARROW_LEFT: u8 = b'D';

    pub const WORD_LEFT: &[u8] = &[C0::ESC, b'b'];
    pub const WORD_RIGHT: &[u8] = &[C0::ESC, b'f'];

    // Navigation escape codes
    pub const PAGE_UP: &[u8] = b"5~";
    pub const PAGE_DOWN: &[u8] = b"6~";
    pub const BACKWARD_TABULATION: &[u8] = b"Z";

    // Special keys
    pub const HOME: u8 = b'H';
    pub const END: u8 = b'F';

    // Mouse-related escape codes
    pub const MOUSE_LEFT: u8 = 0;
    pub const MOUSE_RIGHT: u8 = 2;
    pub const MOUSE_DRAG: u8 = 32;
    pub const MOUSE_MOVE: u8 = 35;
    pub const MOUSE_WHEEL_UP: u8 = 64;
    pub const MOUSE_WHEEL_DOWN: u8 = 65;

    pub const FOCUS_IN: &[u8] = &[C0::ESC, b'[', b'I'];
    pub const FOCUS_OUT: &[u8] = &[C0::ESC, b'[', b'O'];

    pub fn build_escape_sequence_with_c1(c1: &[u8], c: &[u8]) -> Vec<u8> {
        let mut sequence = Vec::new();
        sequence.extend_from_slice(c1);
        sequence.extend_from_slice(c);
        sequence
    }

    pub fn build_escape_sequence(mode_provider: &impl ModeProvider, c: &[u8]) -> Vec<u8> {
        let c1 = get_c1_sequence(mode_provider);
        build_escape_sequence_with_c1(c1, c)
    }

    /// Returns the C1 code that should be used to start an escape sequence based on terminal's
    /// term_mode.
    pub fn get_c1_sequence(mode_provider: &impl ModeProvider) -> &'static [u8] {
        // Usually we use CSI for most escape sequences.
        // However, for programs that set CursorKeys mode we should use SS3 instead.
        // This difference is critical when we want the arrow keys to work in the interactive
        // programs as well as alt_screen during the long running commands.
        // Check https://en.wikipedia.org/wiki/ANSI_escape_code#Fe_Escape_sequences for more
        // information about CSI/SS3 and others.
        if mode_provider.is_term_mode_set(TermMode::APP_CURSOR) {
            return C1::SS3;
        }
        C1::CSI
    }
}

/// Trait for objects that can provide information about the terminal's mode.
pub trait ModeProvider {
    fn is_term_mode_set(&self, mode: TermMode) -> bool;
}

/// To be implemented on event objects (e.g. Keystroke or MouseState) that may be converted to
/// escape sequences to be sent to the pty.
pub trait ToEscapeSequence<T> {
    /// Returns the appropriate escape code to be passed to the pty corresponding to this event, if
    /// any.
    fn to_escape_sequence(&self, mode_provider: &T) -> Option<Vec<u8>>;
}

/// Pairs a keystroke with platform-provided key details for accurate escape sequence encoding.
pub struct KeystrokeWithDetails<'a> {
    pub keystroke: &'a Keystroke,
    pub key_without_modifiers: Option<&'a str>,
    /// The text that this key event would insert, as provided by the OS input system.
    /// Used for the REPORT_ASSOCIATED_TEXT enhancement (Kitty flag 16).
    pub chars: Option<&'a str>,
}

impl<T: ModeProvider> ToEscapeSequence<T> for KeystrokeWithDetails<'_> {
    fn to_escape_sequence(&self, mode_provider: &T) -> Option<Vec<u8>> {
        if let Some(csi_u) = maybe_convert_keystroke_to_csi_u(
            self.keystroke,
            self.key_without_modifiers,
            self.chars,
            mode_provider,
        ) {
            return Some(csi_u);
        }

        // Legacy encoding fallback.
        // NOTE: Order matters! We assume all fn keystrokes have been handled by the
        // time we reach meta_keystroke_to_escape_sequence.
        let keystroke = self.keystroke;
        fn_keystroke_to_escape_sequence(keystroke, mode_provider)
            .or_else(|| keystroke_to_c0_control_code(keystroke, mode_provider))
            .or_else(|| cursor_movement_keystroke_to_escape_sequence(keystroke, mode_provider))
            .or_else(|| meta_keystroke_to_escape_sequence(keystroke, mode_provider))
    }
}

impl<T: ModeProvider> ToEscapeSequence<T> for MouseState {
    fn to_escape_sequence(&self, _mode_provider: &T) -> Option<Vec<u8>> {
        let action = match self.action() {
            MouseAction::Released => 'm',
            _ => 'M',
        };
        let (button, repeats) = match self.button() {
            MouseButton::Left => (EscCodes::MOUSE_LEFT, 1),
            MouseButton::Right => (EscCodes::MOUSE_RIGHT, 1),
            MouseButton::LeftDrag => (EscCodes::MOUSE_DRAG, 1),
            MouseButton::Move => (EscCodes::MOUSE_MOVE, 1),
            MouseButton::Wheel => {
                if let MouseAction::Scrolled { delta } = self.action() {
                    let lines = delta.unsigned_abs() as usize;
                    if *delta > 0 {
                        (EscCodes::MOUSE_WHEEL_UP, lines)
                    } else {
                        (EscCodes::MOUSE_WHEEL_DOWN, lines)
                    }
                } else {
                    panic!("Currently only scroll is supported for the Wheel button")
                }
            }
        };

        let point = self.maybe_point()?;
        let msg = format!(
            "{}<{};{};{}{}",
            C1::to_utf8(C1::CSI),
            button,
            point.col + 1,
            point.row + 1,
            action
        )
        .repeat(repeats);
        Some(msg.into_bytes())
    }
}

pub trait ToModifierEscapeByte {
    /// Returns the modifier escape byte represented by this T.
    ///
    /// The returned modifier byte is typically meant to be inserted into escape sequence
    /// corresponding to the keystroke. See the implementation of this trait for
    /// `Keystroke` for more details.
    fn to_modifier_escape_byte(&self) -> Option<u8>;
}

impl ToModifierEscapeByte for Keystroke {
    // Mirrors the [xterm implementation](https://www.xfree86.org/current/ctlseqs.html#PC-Style%20Function%20Keys).
    fn to_modifier_escape_byte(&self) -> Option<u8> {
        match self {
            Keystroke {
                shift: true,
                alt: false,
                ctrl: false,
                meta: _,
                cmd: _,
                key: _,
            } => Some(b'2'),
            Keystroke {
                shift: false,
                alt: true,
                ctrl: false,
                meta: _,
                cmd: _,
                key: _,
            } => Some(b'3'),
            Keystroke {
                shift: true,
                alt: true,
                ctrl: false,
                meta: _,
                cmd: _,
                key: _,
            } => Some(b'4'),
            Keystroke {
                shift: false,
                alt: false,
                ctrl: true,
                meta: _,
                cmd: _,
                key: _,
            } => Some(b'5'),
            Keystroke {
                shift: true,
                alt: false,
                ctrl: true,
                meta: _,
                cmd: _,
                key: _,
            } => Some(b'6'),
            Keystroke {
                shift: false,
                alt: true,
                ctrl: true,
                meta: _,
                cmd: _,
                key: _,
            } => Some(b'7'),
            Keystroke {
                shift: true,
                alt: true,
                ctrl: true,
                meta: _,
                cmd: _,
                key: _,
            } => Some(b'8'),
            // meta can be basically treated the same way as alt...
            Keystroke {
                meta: true,
                ctrl: _,
                alt: _,
                shift: _,
                cmd: _,
                key: _,
            } => Some(b'3'),
            _ => None,
        }
    }
}

/// Returns the appropriate escape sequence for the given fn key, which may or may not be modified
/// via modifier key(s).
///
/// If the given keystroke is not an fn key, returns None.
fn fn_keystroke_to_escape_sequence(
    keystroke: &Keystroke,
    _mode_provider: &impl ModeProvider,
) -> Option<Vec<u8>> {
    match keystroke.key.as_str() {
        "f1" | "f2" | "f3" | "f4" | "f5" | "f6" | "f7" | "f8" | "f9" | "f10" | "f11" | "f12"
        | "f13" | "f14" | "f15" | "f16" | "f17" | "f18" | "f19" | "f20" => {
            let modifier_byte = keystroke.to_modifier_escape_byte();
            match modifier_byte {
                Some(modifier_byte) => fn_keystroke_with_modifier_to_escape_sequence(
                    keystroke.key.as_str(),
                    modifier_byte,
                ),
                None => fn_keystroke_without_modifier_to_escape_sequence(keystroke.key.as_str()),
            }
        }
        _ => None,
    }
}

/// Returns the escape sequence for the given fn key with no additional modifier key. If `key` is
/// not a fn key, returns None.
///
/// Mapping from key to sequence is adapted from the xterm spec
/// [here](https://www.xfree86.org/current/ctlseqs.html).
fn fn_keystroke_without_modifier_to_escape_sequence(key: &str) -> Option<Vec<u8>> {
    match key {
        "f1" => Some([C1::SS3, b"P"].concat()),
        "f2" => Some([C1::SS3, b"Q"].concat()),
        "f3" => Some([C1::SS3, b"R"].concat()),
        "f4" => Some([C1::SS3, b"S"].concat()),
        "f5" => Some([C1::CSI, b"15~"].concat()),
        "f6" => Some([C1::CSI, b"17~"].concat()),
        "f7" => Some([C1::CSI, b"18~"].concat()),
        "f8" => Some([C1::CSI, b"19~"].concat()),
        "f9" => Some([C1::CSI, b"20~"].concat()),
        "f10" => Some([C1::CSI, b"21~"].concat()),
        "f11" => Some([C1::CSI, b"23~"].concat()),
        "f12" => Some([C1::CSI, b"24~"].concat()),
        "f13" => Some([C1::CSI, b"25~"].concat()),
        "f14" => Some([C1::CSI, b"26~"].concat()),
        "f15" => Some([C1::CSI, b"28~"].concat()),
        "f16" => Some([C1::CSI, b"29~"].concat()),
        "f17" => Some([C1::CSI, b"31~"].concat()),
        "f18" => Some([C1::CSI, b"32~"].concat()),
        "f19" => Some([C1::CSI, b"33~"].concat()),
        "f20" => Some([C1::CSI, b"34~"].concat()),
        _ => None,
    }
}

/// Returns the escape sequence for the given fn key with the given modifier_byte, which is mapped
/// from the modifiers in the original keystroke. If `key` is not a function key, returns None.
///
/// Mapping from key to sequence is adapted from the xterm spec
/// [here](https://www.xfree86.org/current/ctlseqs.html).
fn fn_keystroke_with_modifier_to_escape_sequence(key: &str, modifier_byte: u8) -> Option<Vec<u8>> {
    match key {
        "f1" => Some([C1::CSI, format!("1;{}P", modifier_byte as char).as_bytes()].concat()),
        "f2" => Some([C1::CSI, format!("1;{}Q", modifier_byte as char).as_bytes()].concat()),
        "f3" => Some([C1::CSI, format!("1;{}R", modifier_byte as char).as_bytes()].concat()),
        "f4" => Some([C1::CSI, format!("1;{}S", modifier_byte as char).as_bytes()].concat()),
        "f5" => Some([C1::CSI, format!("15;{}~", modifier_byte as char).as_bytes()].concat()),
        "f6" => Some([C1::CSI, format!("17;{}~", modifier_byte as char).as_bytes()].concat()),
        "f7" => Some([C1::CSI, format!("18;{}~", modifier_byte as char).as_bytes()].concat()),
        "f8" => Some([C1::CSI, format!("19;{}~", modifier_byte as char).as_bytes()].concat()),
        "f9" => Some([C1::CSI, format!("20;{}~", modifier_byte as char).as_bytes()].concat()),
        "f10" => Some([C1::CSI, format!("21;{}~", modifier_byte as char).as_bytes()].concat()),
        "f11" => Some([C1::CSI, format!("23;{}~", modifier_byte as char).as_bytes()].concat()),
        "f12" => Some([C1::CSI, format!("24;{}~", modifier_byte as char).as_bytes()].concat()),
        "f13" => Some([C1::CSI, format!("25;{}~", modifier_byte as char).as_bytes()].concat()),
        "f14" => Some([C1::CSI, format!("26;{}~", modifier_byte as char).as_bytes()].concat()),
        "f15" => Some([C1::CSI, format!("28;{}~", modifier_byte as char).as_bytes()].concat()),
        "f16" => Some([C1::CSI, format!("29;{}~", modifier_byte as char).as_bytes()].concat()),
        "f17" => Some([C1::CSI, format!("31;{}~", modifier_byte as char).as_bytes()].concat()),
        "f18" => Some([C1::CSI, format!("32;{}~", modifier_byte as char).as_bytes()].concat()),
        "f19" => Some([C1::CSI, format!("33;{}~", modifier_byte as char).as_bytes()].concat()),
        "f20" => Some([C1::CSI, format!("34;{}~", modifier_byte as char).as_bytes()].concat()),
        _ => None,
    }
}

/// Returns the C0 control code for the given keystroke.
///
/// These control codes are emitted on ctrl-modified keystrokes. Note that the spec explicitly
/// specifies ctrl-only modified keystrokes. The excat control code mapping is taken from the
/// VT-220 spec [here](https://vt100.net/docs/vt220-rm/chapter3.html#S3.2.5).
///
/// Note that C0 control codes are (definitionally) a single byte, so the returned vector, if any,
/// is always length 1.
fn keystroke_to_c0_control_code(
    keystroke: &Keystroke,
    _mode_provider: &impl ModeProvider,
) -> Option<Vec<u8>> {
    lazy_static! {
        static ref KEYSTROKE_TO_C0_CODE: HashMap<&'static str, u8> = HashMap::from([
            (" ", C0::NUL),
            ("2", C0::NUL),
            ("3", C0::ESC),
            ("4", C0::FS),
            ("5", C0::GS),
            ("6", C0::RS),
            ("7", C0::US),
            ("8", C0::DEL),
        ]);
    }

    // Only emit C0 codes on ctrl-modified keystrokes, without other modifiers, per the VT-220
    // spec.
    if !(keystroke.ctrl && !keystroke.alt && !keystroke.shift && !keystroke.meta) {
        // Return None if the keystroke is not ctrl-key.
        return None;
    }

    if KEYSTROKE_TO_C0_CODE.contains_key(keystroke.key.as_str()) {
        return Some(vec![KEYSTROKE_TO_C0_CODE[keystroke.key.as_str()]]);
    }
    None
}

/// Returns the appropriate escape sequence for the given "cursor movement" keystroke.
///
/// "cursor movement" keystroke is defined as one of the arrow keys, "home" or "end". If the given
/// keystroke is not a "cursor movement" keystroke, returns None.
///
/// Mapping from button to sequence is adapted from the xterm spec
/// [here](https://www.xfree86.org/current/ctlseqs.html).
fn cursor_movement_keystroke_to_escape_sequence(
    keystroke: &Keystroke,
    mode_provider: &impl ModeProvider,
) -> Option<Vec<u8>> {
    lazy_static! {
        static ref CURSOR_KEYSTROKE_TO_CONTROL_CODE: HashMap<&'static str, u8> = HashMap::from([
            ("up", b'A'),
            ("down", b'B'),
            ("right", b'C'),
            ("left", b'D'),
            ("home", b'H'),
            ("end", b'F')
        ]);
    }

    let key = keystroke.key.as_str();
    if !CURSOR_KEYSTROKE_TO_CONTROL_CODE.contains_key(key) {
        return None;
    }
    let modifier_bytes = keystroke.to_modifier_escape_byte();
    match modifier_bytes {
        Some(modifier_bytes) => Some(
            [
                C1::CSI,
                b"1;",
                &[modifier_bytes, CURSOR_KEYSTROKE_TO_CONTROL_CODE[key]],
            ]
            .concat(),
        ),
        None => Some(
            [
                EscCodes::get_c1_sequence(mode_provider),
                &[CURSOR_KEYSTROKE_TO_CONTROL_CODE[key]],
            ]
            .concat(),
        ),
    }
}

/// Returns the byte array corresponding to a special key, if a special key is provided.
/// Otherwise, returns None.
/// We prefer using match over a HashMap due to LLVM being able to optimize this
/// further than a HashMap.
fn map_special_key_to_bytes(key: &str) -> Option<&[u8]> {
    match key {
        "enter" | "numpadenter" => Some("\r".as_bytes()),
        "tab" => Some("\t".as_bytes()),
        "escape" => Some("\x1b".as_bytes()),
        "backspace" => Some("\x7f".as_bytes()),
        "insert" => Some("\x1b[2~".as_bytes()),
        "delete" => Some("\x1b[3~".as_bytes()),
        "pageup" => Some("\x1b[5~".as_bytes()),
        "pagedown" => Some("\x1b[6~".as_bytes()),
        _ => None,
    }
}

/// Returns the appropriate escape sequence for the given meta-modified keystroke.
///
/// If the given keystroke is not meta-modified, returns None.
fn meta_keystroke_to_escape_sequence(
    keystroke: &Keystroke,
    _mode_provider: &impl ModeProvider,
) -> Option<Vec<u8>> {
    // On mac, we have a setting that allows users to map the Option keys to
    // meta.
    if OperatingSystem::get().is_mac() {
        if !keystroke.meta {
            return None;
        }
    } else {
        // On other platforms, interpret the alt key as the meta modifier.
        if !keystroke.alt {
            return None;
        }
    }

    let key = &keystroke.key;

    // We check if the key pressed was a special key i.e. not a normal character first.
    // If it is, we look up the correct byte sequence for that special key and combine that with Meta.
    // Note that we purposely do not check for fn keys here since we expect fn_keystroke_to_escape_sequence
    // already captured fn + Meta combos!
    if let Some(bytes) = map_special_key_to_bytes(key) {
        Some([&[C0::ESC], bytes].concat())
    } else {
        Some([&[C0::ESC], key.as_bytes()].concat())
    }
}

#[cfg(test)]
#[path = "escape_sequences_tests.rs"]
mod tests;
