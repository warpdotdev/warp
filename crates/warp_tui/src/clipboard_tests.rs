use base64::engine::general_purpose::STANDARD;
use base64::Engine;

use super::{osc52_sequences, tmux_passthrough};

#[test]
fn osc52_encodes_utf8_for_clipboard_and_primary() {
    let text = "hello 日🙂";
    let payload = STANDARD.encode(text.as_bytes());
    assert_eq!(
        osc52_sequences(text, false),
        format!("\x1b]52;c;{payload}\x07\x1b]52;p;{payload}\x07")
    );
}

#[test]
fn tmux_passthrough_wraps_and_doubles_escape_bytes() {
    assert_eq!(
        tmux_passthrough("\x1b]52;c;abc\x07"),
        "\x1bPtmux;\x1b\x1b]52;c;abc\x07\x1b\\"
    );
    let wrapped = osc52_sequences("x", true);
    assert_eq!(wrapped.matches("\x1bPtmux;").count(), 2);
    assert_eq!(wrapped.matches("\x1b\x1b]52;").count(), 2);
}
