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
//! [`copy_to_clipboard`] returns an error only when the copy genuinely fails —
//! the native backend is unavailable *and* the OSC 52 stdout write errored.

use std::io::{self, Write};

use base64::engine::general_purpose::STANDARD;
use base64::Engine;

const ESC: char = '\x1b';
const BEL: char = '\x07';

/// Copies `text` to the clipboard, selecting the transport from the environment.
///
/// Local sessions write to the OS clipboard via `arboard`, falling back to OSC 52
/// when the native backend is unavailable; remote/SSH sessions use OSC 52
/// directly. Returns an error only when the copy genuinely fails.
pub(crate) fn copy_to_clipboard(text: &str) -> anyhow::Result<()> {
    let is_remote =
        std::env::var_os("SSH_CONNECTION").is_some() || std::env::var_os("SSH_TTY").is_some();
    let in_tmux = std::env::var_os("TMUX").is_some();

    if is_remote {
        // Remote/SSH: the local OS clipboard isn't reachable, so OSC 52 is the
        // only option.
        let mut stdout = io::stdout().lock();
        write_osc52_sequences(text, in_tmux, &mut stdout)?;
        return Ok(());
    }

    // Local: prefer a native write; fall back to OSC 52 only when the native
    // backend is unavailable (e.g. headless Linux with no display).
    if let Err(error) = set_native_text(text) {
        log::warn!("Native clipboard write failed, falling back to OSC 52: {error}");
        let mut stdout = io::stdout().lock();
        write_osc52_sequences(text, in_tmux, &mut stdout)?;
    }
    Ok(())
}

/// Writes `text` to the OS clipboard via a process-lifetime `arboard` handle
/// (the "ClipboardLease" pattern).
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
fn set_native_text(text: &str) -> anyhow::Result<()> {
    use std::sync::{Mutex, OnceLock};

    use arboard::Clipboard;

    static CLIPBOARD_LEASE: OnceLock<Mutex<Option<Clipboard>>> = OnceLock::new();

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

#[cfg(not(any(
    target_os = "macos",
    target_os = "linux",
    target_os = "freebsd",
    target_os = "windows"
)))]
fn set_native_text(text: &str) -> anyhow::Result<()> {
    let _ = text;
    anyhow::bail!("native OS clipboard is not supported on this platform")
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
