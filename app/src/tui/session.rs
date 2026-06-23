//! [`TuiTerminalSession`]: the TUI's single persistent terminal session.
//!
//! Owns one bootstrapped `TerminalModel` + PTY for the app's lifetime, built
//! via the shared [`build_session_core`] helper. Exposes command execution,
//! raw input forwarding, and resize so the TUI front-end can drive the model
//! the same way the GUI does — without a view.

use std::collections::HashMap;
use std::ffi::OsString;
use std::sync::Arc;
use std::thread::JoinHandle;

use async_broadcast::InactiveReceiver;
use async_channel::Receiver;
use parking_lot::FairMutex;
use pathfinder_geometry::vector::vec2f;
use warpui::{AppContext, Entity, ModelContext, ModelHandle, SingletonEntity};

use crate::auth::AuthStateProvider;
use crate::terminal::input::CommandExecutionSource;
use crate::terminal::local_tty::mio_channel;
use crate::terminal::local_tty::terminal_manager::{
    build_session_core, get_shell_starter_internal, spawn_pty, Message, PtyController,
    PtyControllerEvent, SessionCore,
};
use crate::terminal::model::session::Sessions;
use crate::terminal::model::terminal_model::{ExitReason, TerminalModel};
use crate::terminal::model_events::ModelEventDispatcher;
use crate::terminal::{SizeInfo, SizeUpdate};

#[cfg(test)]
use crate::terminal::shell::ShellType;

/// The TUI's single terminal session: a `TerminalModel` + PTY owned for the
/// app's lifetime. Built once via [`build_session_core`]; the PTY is spawned
/// asynchronously.
pub struct TuiTerminalSession {
    model: Arc<FairMutex<TerminalModel>>,
    sessions: ModelHandle<Sessions>,
    model_events: ModelHandle<ModelEventDispatcher>,
    pty_controller: ModelHandle<PtyController>,
    event_loop_tx: mio_channel::Sender<Message>,
    event_loop_handle: Option<JoinHandle<()>>,
    /// Held to keep the PTY reads broadcast channel alive.
    _inactive_pty_reads_rx: InactiveReceiver<Arc<Vec<u8>>>,
    /// PTY redraw signals; taken once by the root view to drive repaints on
    /// shell output. If never drained, the sender errors on every wakeup and
    /// terminal output never repaints.
    wakeups_rx: Option<Receiver<()>>,
}

impl Entity for TuiTerminalSession {
    type Event = ();
}

impl SingletonEntity for TuiTerminalSession {}

impl TuiTerminalSession {
    /// Returns the shared terminal model.
    pub fn model(&self) -> Arc<FairMutex<TerminalModel>> {
        self.model.clone()
    }

    /// Returns the model event dispatcher (for repaint subscriptions).
    pub fn model_events(&self) -> &ModelHandle<ModelEventDispatcher> {
        &self.model_events
    }

    /// Takes the PTY wakeup receiver (terminal redraw signals). Consumed once
    /// by the root view to drive repaints on shell output; `None` thereafter.
    pub fn take_wakeups_rx(&mut self) -> Option<Receiver<()>> {
        self.wakeups_rx.take()
    }

    /// Executes `command` in the persistent shell session. Resolves the
    /// session's `ShellType` and delegates to `PtyController::write_command`,
    /// mirroring the GUI's `view::Event::ExecuteCommand` handler.
    pub fn run_command(&self, command: &str, ctx: &mut AppContext) {
        let session_id = match self.model.lock().block_list().active_block().session_id() {
            Some(id) => id,
            None => {
                log::warn!("TUI: cannot run command — no session ID (not bootstrapped yet)");
                return;
            }
        };

        let shell_type = self
            .sessions
            .as_ref(ctx)
            .get(session_id)
            .map(|s| s.shell().shell_type());

        let Some(shell_type) = shell_type else {
            log::warn!("TUI: cannot run command — shell type not found for session");
            return;
        };

        self.pty_controller.update(ctx, |controller, ctx| {
            controller.write_command(command, shell_type, CommandExecutionSource::User, ctx);
        });
    }

    /// Writes raw bytes to the PTY (keystroke passthrough for interactive
    /// programs and alt-screen).
    pub fn write_input_bytes(&self, bytes: Vec<u8>, ctx: &mut AppContext) {
        self.pty_controller
            .update(ctx, |controller, ctx| controller.write_bytes(bytes, ctx));
    }

    /// Resizes the terminal model and PTY to `rows` × `cols`.
    pub fn resize(&self, rows: usize, cols: usize, ctx: &mut AppContext) {
        let last_size = *self.model.lock().block_list().size();
        let new_size = SizeInfo::new_without_font_metrics(rows, cols);
        let size_update = SizeUpdate::new_for_headless_resize(last_size, new_size);

        self.pty_controller.update(ctx, |controller, ctx| {
            controller.resize_pty(size_update, ctx);
        });
        self.model.lock().resize(size_update);
    }

    /// Builds and registers the singleton: creates the session core, subscribes
    /// to PTY disconnect, and async-spawns the PTY.
    pub(crate) fn register(ctx: &mut AppContext) {
        let startup_directory = std::env::current_dir().ok();
        let (cols, rows) = current_terminal_cells().unwrap_or((80, 24));
        let initial_size = vec2f(cols as f32, rows as f32);

        // The TUI app is headless, so `build_session_core` would otherwise size
        // the model with a hardcoded default. Pass the real terminal size so the
        // shell's PTY winsize matches the actual terminal (e.g. `ls` columns).
        let headless_size = SizeInfo::new_without_font_metrics(rows as usize, cols as usize);
        let core = build_session_core(
            startup_directory,
            None,
            initial_size,
            Some(headless_size),
            None,
            ctx,
        );

        ctx.add_singleton_model(|ctx| Self::from_core(core, ctx));
    }

    /// Consumes a [`SessionCore`], wires up the PTY disconnect subscription,
    /// and async-spawns the PTY.
    fn from_core(core: SessionCore, ctx: &mut ModelContext<Self>) -> Self {
        let SessionCore {
            model,
            sessions,
            model_events,
            pty_controller,
            event_loop_tx,
            event_loop_rx,
            wakeups_rx,
            inactive_pty_reads_rx,
            channel_event_proxy,
            wsl_name_or_shell_starter,
        } = core;

        // PtyDisconnected → exit the model (mirrors the GUI's wiring).
        let model_for_disconnect = model.clone();
        ctx.subscribe_to_model(
            &pty_controller,
            move |_, _emitter, event, ctx| match event {
                PtyControllerEvent::PtyDisconnected => {
                    model_for_disconnect
                        .lock()
                        .exit(ExitReason::PtyDisconnected);
                    ctx.notify();
                }
            },
        );

        // Async-determine the shell, then spawn the PTY.
        let model_for_spawn = model.clone();
        let event_loop_tx_for_spawn = event_loop_tx.clone();
        let startup_directory = std::env::current_dir().ok();
        let env_vars: HashMap<OsString, OsString> = std::env::vars_os().collect();

        ctx.spawn(
            async move {
                match wsl_name_or_shell_starter {
                    Some(starter_source) => starter_source.to_shell_starter_source().await,
                    None => None,
                }
            },
            move |session, shell_starter_source, ctx| {
                let bg_executor = ctx.background_executor().clone();
                let auth_state = AuthStateProvider::as_ref(ctx).get();
                let shell_starter = shell_starter_source
                    .map(|source| get_shell_starter_internal(source, bg_executor, auth_state));

                let Some(shell_starter) = shell_starter else {
                    log::error!("TUI: could not compute fallback shell");
                    session.model.lock().exit(ExitReason::ShellNotFound);
                    return;
                };

                let spawned = match spawn_pty(
                    shell_starter,
                    startup_directory,
                    env_vars,
                    &event_loop_tx_for_spawn,
                    event_loop_rx,
                    channel_event_proxy,
                    model_for_spawn.clone(),
                    ctx,
                ) {
                    Ok(spawned) => spawned,
                    Err(err) => {
                        log::error!("TUI: failed to spawn pty: {err:#}");
                        session.model.lock().exit(ExitReason::PtySpawnFailed);
                        return;
                    }
                };

                session.event_loop_handle = Some(spawned.event_loop_handle);
                ctx.notify();
            },
        );

        TuiTerminalSession {
            model,
            sessions,
            model_events,
            pty_controller,
            event_loop_tx,
            event_loop_handle: None,
            _inactive_pty_reads_rx: inactive_pty_reads_rx,
            wakeups_rx: Some(wakeups_rx),
        }
    }
}

impl Drop for TuiTerminalSession {
    fn drop(&mut self) {
        let _ = self.event_loop_tx.send(Message::Shutdown);
        if let Some(handle) = self.event_loop_handle.take() {
            let _ = handle.join();
        }
    }
}

/// Reads the current terminal size in cells from crossterm.
fn current_terminal_cells() -> Option<(u16, u16)> {
    crossterm::terminal::size().ok()
}

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
    keystroke: &warpui_core::keymap::Keystroke,
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
fn fallback_control_bytes(
    keystroke: &warpui_core::keymap::Keystroke,
    chars: &str,
) -> Option<Vec<u8>> {
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

/// Resolves the `ShellType` for the session's active block, for testing.
#[cfg(test)]
pub(crate) fn resolve_shell_type(
    model: &FairMutex<TerminalModel>,
    sessions: &ModelHandle<Sessions>,
    ctx: &AppContext,
) -> Option<ShellType> {
    let session_id = model.lock().block_list().active_block().session_id()?;
    sessions
        .as_ref(ctx)
        .get(session_id)
        .map(|s| s.shell().shell_type())
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
