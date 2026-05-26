use std::ops::Range;

use regex::Regex;
use warpui::keymap::Keystroke;

use crate::terminal::model::escape_sequences::C0;

lazy_static::lazy_static! {
    /// Matches control code values specified in hex format delimited by angle brackets, e.g. <0x1B>.
    static ref CONTROL_CODE_REGEX: Regex = Regex::new(r"<0x([0-9a-fA-F]{2})>").expect("Regex is valid.");

    /// Matches an incomplete control code at the end of the response stream.
    static ref INCOMPLETE_CONTROL_CODE_REGEX: Regex = Regex::new(r"<(0|0x|0x[0-9a-fA-F]|0x[0-9a-fA-F]{2})$").expect("Regex is valid.");
}

/// Parsed output that separates raw bytes from display-friendly text.
#[derive(Debug, Clone, Default)]
pub struct ParsedControlCodeOutput {
    /// Human-readable display text (e.g., "<ctrl-c>" instead of 0x03).
    pub display: String,

    /// Ranges in `display` where control codes appear (for styling).
    pub control_code_ranges: Vec<Range<usize>>,
}

/// Parses raw bytes containing control codes like `\x1B`.
pub fn parse_control_codes_from_bytes(bytes: &[u8]) -> ParsedControlCodeOutput {
    let mut parsed_output = String::new();
    let mut control_code_ranges = vec![];
    let mut output_bytes = Vec::with_capacity(bytes.len());

    for &byte in bytes {
        output_bytes.push(byte);

        if let Some(keystroke) = byte_value_to_keystroke(byte) {
            let keystroke_string = format!(" <{}>", keystroke.normalized());
            control_code_ranges
                .push(parsed_output.len() + 1..parsed_output.len() + keystroke_string.len());
            parsed_output.push_str(&keystroke_string);
        } else if byte.is_ascii_graphic() || byte == b' ' {
            parsed_output.push(byte as char);
        } else {
            // For non-control, non-printable bytes, show as hex notation
            let hex_string = format!("<0x{byte:02X}>");
            parsed_output.push_str(&hex_string);
        }
    }

    ParsedControlCodeOutput {
        display: parsed_output,
        control_code_ranges,
    }
}

/// Returns the corresponding `Keystroke` for the given byte value if it represents a C0 control code.
///
/// For example, given the byte value for `C0::CR` (carriage return), returns the "enter" `Keystroke`.
fn byte_value_to_keystroke(value: u8) -> Option<Keystroke> {
    let (ctrl, key) = match value {
        C0::SOH => (true, "a".into()),
        C0::STX => (true, "b".into()),
        C0::ETX => (true, "c".into()),
        C0::EOT => (true, "d".into()),
        C0::ENQ => (true, "e".into()),
        C0::ACK => (true, "f".into()),
        C0::BEL => (true, "g".into()),
        C0::VT => (true, "k".into()),
        C0::FF => (true, "l".into()),
        C0::SO => (true, "n".into()),
        C0::SI => (true, "o".into()),
        C0::DLE => (true, "p".into()),
        C0::XON => (true, "q".into()),
        C0::DC2 => (true, "r".into()),
        C0::XOFF => (true, "s".into()),
        C0::DC4 => (true, "t".into()),
        C0::NAK => (true, "u".into()),
        C0::SYN => (true, "v".into()),
        C0::ETB => (true, "w".into()),
        C0::CAN => (true, "x".into()),
        C0::EM => (true, "y".into()),
        C0::SUB => (true, "z".into()),
        C0::FS => (true, "\\".into()),
        C0::GS => (true, "]".into()),
        C0::RS => (true, "^".into()),
        C0::US => (true, "_".into()),
        C0::HT => (false, "tab".into()),
        C0::ESC => (false, "escape".into()),
        C0::DEL => (false, "delete".into()),
        C0::BS => (false, "backspace".into()),
        C0::LF | C0::CR => (false, "enter".into()),
        _ => return None,
    };

    Some(Keystroke {
        ctrl,
        alt: false,
        shift: false,
        cmd: false,
        meta: false,
        key,
    })
}
