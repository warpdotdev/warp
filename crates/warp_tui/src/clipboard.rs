//! Clipboard writes for the headless TUI via the host terminal's OSC 52 support.

use std::io::{self, Write};

use base64::engine::general_purpose::STANDARD;
use base64::Engine;

const ESC: char = '\x1b';
const BEL: char = '\x07';

/// Copies `text` through the terminal, including Linux's PRIMARY selection.
pub(crate) fn copy_to_clipboard(text: &str) -> io::Result<()> {
    let mut stdout = io::stdout().lock();
    write_osc52_sequences(text, std::env::var_os("TMUX").is_some(), &mut stdout)
}

fn write_osc52_sequences(text: &str, in_tmux: bool, writer: &mut impl Write) -> io::Result<()> {
    let sequence = osc52_sequences(text, in_tmux);
    writer.write_all(sequence.as_bytes())?;
    writer.flush()
}

/// Encodes `text` as OSC 52 writes to clipboard (`c`) and PRIMARY (`p`).
fn osc52_sequences(text: &str, in_tmux: bool) -> String {
    let payload = STANDARD.encode(text.as_bytes());
    ["c", "p"]
        .into_iter()
        .map(|target| {
            let sequence = format!("{ESC}]52;{target};{payload}{BEL}");
            if in_tmux {
                tmux_passthrough(&sequence)
            } else {
                sequence
            }
        })
        .collect()
}

/// Wraps an escape sequence in tmux DCS passthrough, doubling inner escapes.
fn tmux_passthrough(sequence: &str) -> String {
    let escaped = sequence.replace(ESC, "\x1b\x1b");
    format!("{ESC}Ptmux;{escaped}{ESC}\\")
}

#[cfg(test)]
#[path = "clipboard_tests.rs"]
mod tests;
