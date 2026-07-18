use std::io::{self, Write};

use base64::engine::general_purpose::STANDARD;
use base64::Engine;

use super::{
    copy_with, osc52_sequences, tmux_passthrough, write_osc52_sequences, ClipboardCopy,
    NativeClipboard,
};

/// A fake native backend that records call count and returns a canned result,
/// so the transport decision is testable without touching a real clipboard.
struct FakeNativeClipboard {
    result: Result<(), &'static str>,
    calls: usize,
}

impl FakeNativeClipboard {
    fn succeeding() -> Self {
        Self {
            result: Ok(()),
            calls: 0,
        }
    }

    fn failing() -> Self {
        Self {
            result: Err("native backend unavailable"),
            calls: 0,
        }
    }
}

impl NativeClipboard for FakeNativeClipboard {
    fn set_text(&mut self, _text: &str) -> anyhow::Result<()> {
        self.calls += 1;
        self.result.map_err(|message| anyhow::anyhow!(message))
    }
}

/// A writer that always errors, to exercise the OSC 52 hard-failure path.
struct FailingWriter;

impl Write for FailingWriter {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        Err(io::Error::other("write failed"))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn local_copy_reports_copied() {
    let mut native = FakeNativeClipboard::succeeding();
    let mut osc52 = Vec::new();

    let outcome = copy_with("hello", false, false, &mut native, &mut osc52).unwrap();

    assert_eq!(outcome, ClipboardCopy::Copied);
    assert_eq!(native.calls, 1);
    // A confirmed native copy must not also emit OSC 52 (which on a remote host
    // would land on the wrong machine and muddy the feedback).
    assert!(osc52.is_empty());
}

#[test]
fn ssh_session_reports_sent_to_terminal() {
    let mut native = FakeNativeClipboard::succeeding();
    let mut osc52 = Vec::new();

    let outcome = copy_with("hello", true, false, &mut native, &mut osc52).unwrap();

    assert_eq!(outcome, ClipboardCopy::SentToTerminal);
    // Remote sessions go straight to OSC 52; the native backend is never touched.
    assert_eq!(native.calls, 0);
    assert_eq!(osc52, osc52_sequences("hello", false).into_bytes());
}

#[test]
fn native_failure_falls_back_to_osc52() {
    let mut native = FakeNativeClipboard::failing();
    let mut osc52 = Vec::new();

    let outcome = copy_with("hello", false, false, &mut native, &mut osc52).unwrap();

    assert_eq!(outcome, ClipboardCopy::SentToTerminal);
    assert_eq!(native.calls, 1);
    assert_eq!(osc52, osc52_sequences("hello", false).into_bytes());
}

#[test]
fn hard_failure_reports_err() {
    let mut native = FakeNativeClipboard::failing();

    // Local + native unavailable + OSC 52 stdout write errors => genuine failure.
    let result = copy_with("hello", false, false, &mut native, &mut FailingWriter);

    assert!(result.is_err());
    assert_eq!(native.calls, 1);
}

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
