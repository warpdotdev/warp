use std::io::{self, Write};

use base64::engine::general_purpose::STANDARD;
use base64::Engine;

use super::{osc52_sequences, tmux_passthrough, write_osc52_sequences};

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

#[test]
fn clipboard_writer_emits_exact_markdown_payload() {
    let markdown = "# Conversation\n\nHello";
    let mut output = Vec::new();

    write_osc52_sequences(markdown, false, &mut output).unwrap();

    assert_eq!(output, osc52_sequences(markdown, false).into_bytes());
}

#[test]
fn clipboard_writer_propagates_output_errors() {
    struct FailingWriter;

    impl Write for FailingWriter {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::other("write failed"))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    assert_eq!(
        write_osc52_sequences("conversation", false, &mut FailingWriter)
            .unwrap_err()
            .kind(),
        io::ErrorKind::Other
    );
}
