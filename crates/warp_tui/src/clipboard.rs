//! Clipboard writes for the headless TUI.
//!
//! The transport is selected up front from the environment rather than by
//! trying one and falling back on an (unobservable) failure:
//! - **Remote/SSH sessions** (`SSH_CONNECTION` / `SSH_TTY` set) use OSC 52, since
//!   the local OS clipboard isn't reachable from the remote machine.
//! - **Local sessions** write directly to the OS clipboard via `arboard`; OSC 52
//!   is used only as a last-resort fallback when the native backend errors
//!   (e.g. a displayless Linux box).
//!
//! OSC 52 is fire-and-forget — the host sends no acknowledgment and may silently
//! drop the write (Warp's own terminal denies programmatic clipboard writes by
//! default) — so a native write is the only transport the TUI can *confirm*.
//! [`copy_to_clipboard`] therefore returns a [`ClipboardCopy`] distinguishing a
//! confirmed native copy from a best-effort OSC 52 send, so callers never report
//! a best-effort send as a guaranteed copy.

use std::io::{self, Write};

use base64::engine::general_purpose::STANDARD;
use base64::Engine;

const ESC: char = '\x1b';
const BEL: char = '\x07';

/// The outcome of a successful [`copy_to_clipboard`] call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClipboardCopy {
    /// Text was written to the OS clipboard and confirmed (native path).
    Copied,
    /// Text was emitted via OSC 52 to the host terminal (best-effort; the host
    /// may silently reject it, e.g. Warp's default `Deny`).
    SentToTerminal,
}

/// A native OS-clipboard backend. Abstracted so the transport decision can be
/// unit-tested without touching a real clipboard.
trait NativeClipboard {
    fn set_text(&mut self, text: &str) -> anyhow::Result<()>;
}

/// Copies `text` to the clipboard, selecting the transport from the environment.
///
/// Returns [`ClipboardCopy::Copied`] on a confirmed native write,
/// [`ClipboardCopy::SentToTerminal`] on a best-effort OSC 52 send, or an error
/// only when copy genuinely fails (native unavailable *and* the OSC 52 stdout
/// write errored).
pub(crate) fn copy_to_clipboard(text: &str) -> anyhow::Result<ClipboardCopy> {
    let is_remote =
        std::env::var_os("SSH_CONNECTION").is_some() || std::env::var_os("SSH_TTY").is_some();
    let in_tmux = std::env::var_os("TMUX").is_some();
    let mut native = RealNativeClipboard;
    let mut stdout = io::stdout().lock();
    copy_with(text, is_remote, in_tmux, &mut native, &mut stdout)
}

/// Core transport decision, factored out for testing. `native` is only consulted
/// for local sessions; remote sessions go straight to OSC 52.
fn copy_with(
    text: &str,
    is_remote: bool,
    in_tmux: bool,
    native: &mut dyn NativeClipboard,
    osc52_writer: &mut impl Write,
) -> anyhow::Result<ClipboardCopy> {
    if is_remote {
        // Remote/SSH: the local OS clipboard isn't reachable, so OSC 52 is the
        // only option. A confirmed copy is impossible here.
        write_osc52_sequences(text, in_tmux, osc52_writer)?;
        return Ok(ClipboardCopy::SentToTerminal);
    }

    // Local: prefer a confirmed native write; fall back to OSC 52 only when the
    // native backend is unavailable (e.g. headless Linux with no display).
    match native.set_text(text) {
        Ok(()) => Ok(ClipboardCopy::Copied),
        Err(error) => {
            log::warn!("Native clipboard write failed, falling back to OSC 52: {error}");
            write_osc52_sequences(text, in_tmux, osc52_writer)?;
            Ok(ClipboardCopy::SentToTerminal)
        }
    }
}

/// The real native backend, backed by a process-lifetime `arboard` handle.
struct RealNativeClipboard;

impl NativeClipboard for RealNativeClipboard {
    fn set_text(&mut self, text: &str) -> anyhow::Result<()> {
        #[cfg(any(
            target_os = "macos",
            target_os = "linux",
            target_os = "freebsd",
            target_os = "windows"
        ))]
        {
            native_arboard::set_text(text)
        }
        #[cfg(not(any(
            target_os = "macos",
            target_os = "linux",
            target_os = "freebsd",
            target_os = "windows"
        )))]
        {
            let _ = text;
            anyhow::bail!("native OS clipboard is not supported on this platform")
        }
    }
}

/// Process-lifetime `arboard` handle (the "ClipboardLease" pattern).
///
/// On Linux (X11/Wayland) the clipboard contents are *served by the process that
/// owns the selection*, so dropping the `arboard::Clipboard` handle makes the
/// copied text disappear. The TUI is long-lived, so the handle is initialised
/// lazily and retained for the process's entire lifetime and reused for every
/// copy — never constructed-and-dropped per copy.
#[cfg(any(
    target_os = "macos",
    target_os = "linux",
    target_os = "freebsd",
    target_os = "windows"
))]
mod native_arboard {
    use std::sync::{Mutex, OnceLock};

    use arboard::Clipboard;

    static CLIPBOARD_LEASE: OnceLock<Mutex<Option<Clipboard>>> = OnceLock::new();

    pub(super) fn set_text(text: &str) -> anyhow::Result<()> {
        let lease = CLIPBOARD_LEASE.get_or_init(|| Mutex::new(None));
        let mut guard = lease.lock().unwrap_or_else(|poison| poison.into_inner());
        if guard.is_none() {
            *guard = Some(Clipboard::new()?);
        }
        // Present because it is set immediately above when absent.
        let clipboard = guard
            .as_mut()
            .expect("clipboard handle initialised above when absent");
        clipboard.set_text(text.to_owned())?;
        Ok(())
    }
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
