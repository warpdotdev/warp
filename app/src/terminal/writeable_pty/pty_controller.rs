use std::borrow::Cow;
use std::collections::VecDeque;
use std::mem;
use std::sync::Arc;

use async_channel::{Receiver, Sender};
use parking_lot::FairMutex;
use thiserror::Error;
#[cfg(feature = "local_fs")]
use warp_errors::report_error;
use warp_util::path::ShellFamily;
use warpui::r#async::block_on;
use warpui::{Entity, ModelContext, ModelHandle, SingletonEntity};

use super::Message;
use crate::SessionSettings;
use crate::ai::agent::AIAgentPtyWriteMode;
use crate::terminal::input::CommandExecutionSource;
use crate::terminal::line_editor_status::{LineEditorStatus, LineEditorStatusEvent};
use crate::terminal::model::ansi::Handler;
use crate::terminal::model::completions::ShellCompletion;
use crate::terminal::model::session::{
    ExecutorCommandEvent, InBandCommandCancelledEvent, SessionInfo, Sessions,
};
use crate::terminal::model::{StartCommandOutcome, escape_sequences, native_shell_completions};
use crate::terminal::model_events::{AnsiHandlerEvent, ModelEvent, ModelEventDispatcher};
use crate::terminal::shell::ShellType;
use crate::terminal::view::LINEFEED_REGEX;
#[cfg(not(target_family = "wasm"))]
use crate::terminal::writeable_pty::bootstrap_file::{TempBootstrapFile, permanent_bootstrap_file};
use crate::terminal::{SizeUpdate, TerminalModel, bootstrap};

/// Byte sequence to emulate the user pressing ENTER, used to execute a command in the shell.
const COMMAND_ENTER: &[u8] = &[escape_sequences::C0::CR, escape_sequences::C0::LF];
/// Used to let the shell know we are switching to the PS1 prompt via a bindkey \ep. This will
/// restore the PS1 from the saved PS1 value (we had unset the PS1 for Warp prompt).
const SWITCH_TO_PS1_ESCAPE_SEQUENCE: &[u8] = &[escape_sequences::C0::ESC, b'p'];
/// Used to let the shell know we are switching to the Warp prompt via a bindkey \ew. This will
/// unset the PS1 to ensure we don't have a double prompt (PS1 and Warp prompt).
const SWITCH_TO_WARP_PROMPT_ESCAPE_SEQUENCE: &[u8] = &[escape_sequences::C0::ESC, b'w'];
/// Triggers PowerShell's native-completions PSReadLine key handler (`Warp-Configure-PSReadLine`
/// in pwsh.ps1), which reads back whatever was just typed via `GetBufferState`, computes
/// completions, and reverts the buffer -- never treating anything as a command to execute. Alt+3,
/// following the same "Alt+<digit>" convention as PowerShell's other bindings (kill-buffer is
/// Alt+2, input reporting is Alt+1), to avoid the virtual-key-code/layout issue letter-based
/// bindings have on Windows (see `ShellType::input_reporting_sequence`'s doc comment). Only
/// PowerShell uses this; the other three shells drive native completions through the ordinary
/// in-band command path instead (see `send_write_to_event_loop`'s `RunNativeShellCompletions`
/// handling).
const POWERSHELL_NATIVE_COMPLETIONS_TRIGGER: &[u8] = &[escape_sequences::C0::ESC, b'3'];

/// Represents a single call to write bytes to the PTY asynchronously.
enum PtyWrite {
    Command {
        command: String,
        shell_type: ShellType,
        /// The id if the command is an in-band command or `None` if the command is not an in-band
        /// command.
        in_band_command_id: Option<String>,
        /// If 'some', the given callback is called right before the bytes are written to the PTY.
        before_write_fn: Option<Box<dyn Fn() -> StartCommandOutcome + Send + 'static>>,
    },
    Bytes {
        /// The bytes to be written.
        bytes: Cow<'static, [u8]>,
    },
    AgentInput {
        /// The bytes to be written.
        bytes: Cow<'static, [u8]>,
        /// The `mode` for the agent's write.
        mode: AIAgentPtyWriteMode,
    },
    RunNativeShellCompletions {
        /// The text to write to the PTY, computed by
        /// `native_shell_completions::generator_command_for`. For PowerShell this is just the
        /// hex-encoded buffer text (see the `POWERSHELL_NATIVE_COMPLETIONS_TRIGGER` doc comment);
        /// for the other three shells it's a full generator-command line.
        command: String,
        shell_type: ShellType,
        results_tx: async_channel::Sender<Vec<ShellCompletion>>,
        /// The input editor's buffer text this request was computed from. For the three shells
        /// that run this as a foreground command, the generator command necessarily clears the
        /// shell's real input buffer to run (see `bytes_to_execute_command`), so once results
        /// come back this is written back to the pty verbatim -- see
        /// `in_flight_native_completions_buffer_text`. PowerShell never touches the real buffer
        /// in the first place (see `send_write_to_event_loop`'s handling of this variant), so
        /// this field goes unused for it.
        buffer_text: String,
    },
}

/// Controller for writes to the PTY.
///
/// This is responsible for coordinating writes to the PTY amongst input like user commands, non-command user
/// input, and in-band commands in conjunction with line editor status.
pub struct PtyController<T: EventLoopSender> {
    /// `Sender` for the main PTY event loop channel.
    event_loop_tx: T,
    terminal_model: Arc<FairMutex<TerminalModel>>,
    line_editor_status: ModelHandle<LineEditorStatus>,
    sessions: ModelHandle<Sessions>,
    model_event_dispatcher: ModelHandle<ModelEventDispatcher>,
    pending_writes: VecDeque<PtyWrite>,
    is_user_command_executing: bool,
    is_bracketed_paste_enabled: bool,
    /// If we're bootstrapping the shell by sourcing a file with the bootstrap
    /// script, this will hold the handle to the file.  Once bootstrapping is
    /// complete, it will be dropped to clean up the temporary file.
    #[cfg(not(target_family = "wasm"))]
    bootstrap_file: Option<TempBootstrapFile>,
    in_flight_native_completions_results_tx: Option<async_channel::Sender<Vec<ShellCompletion>>>,
    /// The buffer text of the currently in-flight native-completions request, if any. Written
    /// back to the pty verbatim once results come back (see `ModelEvent::CompletionsFinished`
    /// handling below), to undo the buffer-clearing that `bytes_to_execute_command` necessarily
    /// performs to run the request as a foreground command. Left `None` for PowerShell requests,
    /// which never touch the real buffer in the first place (see `send_write_to_event_loop`'s
    /// handling of `PtyWrite::RunNativeShellCompletions`), so nothing needs restoring.
    in_flight_native_completions_buffer_text: Option<String>,
    /// Set right when a native-completions buffer restore write is queued (see
    /// `ModelEvent::CompletionsFinished` handling below) and cleared the next time the line
    /// editor becomes active. While set, the `LineEditorStatusEvent::Active` subscription skips
    /// queueing the input-reporting sequence -- both fire from the same underlying trigger (the
    /// shell returning to a fresh prompt after the generator command completes), in an order
    /// that isn't guaranteed, and re-running input reporting right after the restore would
    /// report and clear the text this just wrote back, producing PTY output `push_expected_echo`
    /// never registered and so isn't recognized, rendering as a phantom background block.
    just_restored_native_completions_buffer: bool,
}

impl<T: EventLoopSender> PtyController<T> {
    pub fn new(
        event_loop_tx: T,
        model_event_dispatcher: ModelHandle<ModelEventDispatcher>,
        line_editor_status: ModelHandle<LineEditorStatus>,
        sessions: ModelHandle<Sessions>,
        executor_command_rx: Receiver<ExecutorCommandEvent>,
        terminal_model: Arc<FairMutex<TerminalModel>>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        ctx.subscribe_to_model(&model_event_dispatcher, |me, _, event, ctx| match event {
            ModelEvent::Handler(AnsiHandlerEvent::UserCommandFinished) => {
                me.is_user_command_executing = false;
            }
            ModelEvent::Handler(AnsiHandlerEvent::InitShell {
                pending_session_info,
            }) => {
                me.initialize_shell(pending_session_info.as_ref(), ctx);
            }
            ModelEvent::Handler(AnsiHandlerEvent::Bootstrapped { is_subshell, .. }) => {
                me.shell_bootstrapped(*is_subshell);
            }
            ModelEvent::Handler(AnsiHandlerEvent::SetBracketedPaste) => {
                me.is_bracketed_paste_enabled = true;
            }
            ModelEvent::Handler(AnsiHandlerEvent::UnsetBracketedPaste) => {
                me.is_bracketed_paste_enabled = false;
            }
            ModelEvent::HonorPS1OutOfSync => {
                // We force re-sync the PS1 state of Warp settings with the shell's environment variable, $WARP_HONOR_PS1, via
                // a bindkey (which triggers a shell function).
                let honor_ps1 = *SessionSettings::as_ref(ctx).honor_ps1;
                if honor_ps1 {
                    me.send_switch_to_ps1_bindkey(ctx);
                } else {
                    me.send_switch_to_warp_prompt_bindkey(ctx);
                }
            }
            ModelEvent::CompletionsFinished(data) => {
                log::debug!(
                    "PHANTOM_DIAG CompletionsFinished fired, {} results, in_flight_buffer_text={:?}, pending_writes_len={}",
                    data.len(),
                    me.in_flight_native_completions_buffer_text,
                    me.pending_writes.len()
                );
                let Some(results_tx) = me.in_flight_native_completions_results_tx.take() else {
                    log::warn!("Received CompletionsFinished event but didn't have a channel to send results over!");
                    return;
                };
                let _ = block_on(results_tx.send(data.clone()));

                // The generator command necessarily cleared the shell's real input buffer to
                // run in the foreground (see `bytes_to_execute_command`); write back what the
                // user had actually typed so it isn't lost. This is queued to the front so it
                // goes out as soon as the line editor is active again (i.e. once the shell has
                // returned to a fresh prompt after the generator command completes), ahead of
                // anything else queued in the meantime -- *unless* what's queued behind it is a
                // newer completions request: that request's own kill-buffer is about to clear
                // whatever's on the line again anyway, making this restore pointless to send at
                // all, and (unlike an ordinary write) sending it regardless would race the newer
                // request's kill-buffer/generator-command write, which drains immediately behind
                // it since it isn't gated by `execute_next_queued_write`'s `is_command` check
                // the way a `Command` is. Skipping a redundant restore sidesteps that race
                // without needing to gate draining on this write at all -- the newer request
                // will produce its own, current restore once it completes.
                //
                // Skipping is safe rather than a new way to lose the buffer: either the newer
                // request's write never reaches the pty at all (its `before_write_fn` rejects
                // it, or it gets retain-filtered by a still-newer request while still queued --
                // see `run_native_shell_completions`), in which case the real buffer was never
                // touched and there's nothing to restore; or its kill-buffer does go out, at
                // which point it can no longer be retain-filtered away, so it will run to
                // completion and fire its own `CompletionsFinished`, where this same check
                // repeats. That recursion is bounded by real keystrokes -- each further
                // supersession needs another character typed -- so the first request in the
                // chain that finishes with nothing newer queued behind it has its restore sent,
                // and submitting a command requires the user to stop typing regardless, which
                // is exactly what lets the chain resolve before Enter is reachable.
                //
                // The one case this doesn't cover, and it's pre-existing rather than introduced
                // here: if a request's kill-buffer goes out but its generator command then
                // hangs, crashes, or is interrupted before emitting `9280;B`, `CompletionsFinished`
                // never fires for it and the buffer is never restored -- true before this change
                // and after it, since the original code also only ever restored on that event.
                if let Some(buffer_text) = me.in_flight_native_completions_buffer_text.take()
                    && !buffer_text.is_empty()
                    && !me
                        .pending_writes
                        .iter()
                        .any(|write| matches!(write, PtyWrite::RunNativeShellCompletions { .. }))
                {
                    // Register the restored text as expected echo *before* writing it, so the
                    // shell echoing it back is recognized as typeahead -- feeding it back into
                    // the input editor the same way any other typeahead would be -- rather than
                    // unexpected background output, which would otherwise render as a phantom
                    // block mirroring the restored text.
                    log::debug!(
                        "PHANTOM_DIAG CompletionsFinished: queueing restore write {buffer_text:?}, pending_writes_len_before={}",
                        me.pending_writes.len()
                    );
                    me.terminal_model.lock().push_expected_echo(&buffer_text);
                    me.just_restored_native_completions_buffer = true;
                    me.pending_writes.push_front(PtyWrite::Bytes {
                        bytes: Cow::Owned(buffer_text.into_bytes()),
                    });
                    me.execute_next_queued_write(ctx);
                } else {
                    log::debug!(
                        "PHANTOM_DIAG CompletionsFinished: NOT restoring (empty, already consumed, or newer request queued)"
                    );
                }
            }
            _ => (),
        });

        ctx.subscribe_to_model(&line_editor_status, |me, _, event, ctx| {
            if let LineEditorStatusEvent::Active = event {
                log::debug!(
                    "PHANTOM_DIAG LineEditorStatusEvent::Active fired, just_restored_native_completions_buffer={}, pending_writes_len={}",
                    me.just_restored_native_completions_buffer,
                    me.pending_writes.len()
                );
                if mem::replace(&mut me.just_restored_native_completions_buffer, false) {
                    // Skip input reporting this one time -- see the field's doc comment.
                    log::debug!(
                        "PHANTOM_DIAG LineEditorStatusEvent::Active: skipping input reporting (just restored)"
                    );
                    me.execute_next_queued_write(ctx);
                    return;
                }
                let input_reporting_seq = me
                    .model_event_dispatcher
                    .as_ref(ctx)
                    .active_session_id()
                    .and_then(|id| me.sessions.as_ref(ctx).get(id))
                    .and_then(|session| session.shell().input_reporting_sequence());
                if let Some(bytes) = input_reporting_seq {
                    log::debug!(
                        "PHANTOM_DIAG LineEditorStatusEvent::Active: queueing input-reporting sequence ({} bytes)",
                        bytes.len()
                    );
                    me.pending_writes.push_front(PtyWrite::Bytes {
                        bytes: Cow::Owned(bytes.to_vec()),
                    });
                }
                me.execute_next_queued_write(ctx);
            }
        });

        let _ = ctx.spawn_stream_local(
            executor_command_rx,
            |me, event, ctx| match event {
                ExecutorCommandEvent::ExecuteCommand { command, cancel_tx } => {
                    me.queue_in_band_command(
                        command.command.as_str(),
                        command.shell_type,
                        command.command_id,
                        cancel_tx,
                        ctx,
                    );
                }
                ExecutorCommandEvent::CancelCommand { id } => {
                    me.cancel_in_band_command(id.as_str());
                }
            },
            |_, _| (),
        );

        Self {
            event_loop_tx,
            terminal_model,
            line_editor_status,
            sessions,
            model_event_dispatcher,
            pending_writes: VecDeque::new(),
            is_user_command_executing: false,
            is_bracketed_paste_enabled: false,
            #[cfg(not(target_family = "wasm"))]
            bootstrap_file: None,
            in_flight_native_completions_results_tx: None,
            in_flight_native_completions_buffer_text: None,
            just_restored_native_completions_buffer: false,
        }
    }

    /// Sends bindkey to notify shell process to switch to PS1 logic for prompt
    /// with the combined prompt/command grid (we restore the saved PS1 value).
    pub fn send_switch_to_ps1_bindkey(&mut self, ctx: &mut ModelContext<Self>) {
        self.pending_writes.push_back(PtyWrite::Bytes {
            bytes: SWITCH_TO_PS1_ESCAPE_SEQUENCE.into(),
        });
        self.execute_next_queued_write(ctx);

        let is_bash_shell = self
            .model_event_dispatcher
            .as_ref(ctx)
            .active_session_id()
            .and_then(|id| self.sessions.as_ref(ctx).get(id))
            .map(|session| session.shell().shell_type() == ShellType::Bash)
            .unwrap_or(false);
        if is_bash_shell {
            // We cannot repaint via shell command in bash, so we must execute an empty command to force refresh the prompt instantly
            // (avoid a 1 block delay since the current prompt has technically already been sent).
            self.pending_writes.push_back(PtyWrite::Bytes {
                bytes: COMMAND_ENTER.into(),
            });
            self.execute_next_queued_write(ctx);
        }
    }

    /// Sends bindkey to notify shell process to switch to Warp prompt logic for prompt
    /// with the combined prompt/command grid (we unset the PS1, but save the value for potential
    /// future restoration).
    pub fn send_switch_to_warp_prompt_bindkey(&mut self, ctx: &mut ModelContext<Self>) {
        self.pending_writes.push_back(PtyWrite::Bytes {
            bytes: SWITCH_TO_WARP_PROMPT_ESCAPE_SEQUENCE.into(),
        });
        self.execute_next_queued_write(ctx);

        let is_bash_shell = self
            .model_event_dispatcher
            .as_ref(ctx)
            .active_session_id()
            .and_then(|id| self.sessions.as_ref(ctx).get(id))
            .map(|session| session.shell().shell_type() == ShellType::Bash)
            .unwrap_or(false);
        if is_bash_shell {
            // We cannot repaint via shell command in bash, so we must execute an empty command to force refresh the prompt instantly
            // (avoid a 1 block delay since the current prompt has technically already been sent).
            self.pending_writes.push_back(PtyWrite::Bytes {
                bytes: COMMAND_ENTER.into(),
            });
            self.execute_next_queued_write(ctx);
        }
    }

    fn cancel_in_band_command(&mut self, command_id: &str) {
        self.pending_writes.retain(|pty_write| {
            !matches!(pty_write, PtyWrite::Command {
                 in_band_command_id, ..
             } if in_band_command_id.as_deref() == Some(command_id))
        });
    }

    /// Queues an in-band command to be written to the PTY, either immediately, or when the line
    /// editor next becomes active.
    ///
    /// If a user command is currently executing, this short-circuits since the in-band command
    /// request is likely stale. However, we still need to signal that the command will not be
    /// executed so the executor knows to clear it.
    fn queue_in_band_command(
        &mut self,
        command: &str,
        shell_type: ShellType,
        command_id: String,
        cancel_tx: Sender<InBandCommandCancelledEvent>,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.is_user_command_executing {
            // Send blocking should be okay b/c this is an unbound channel
            if let Err(err) = block_on(cancel_tx.send(InBandCommandCancelledEvent { command_id })) {
                log::warn!("Pty Controller failed to cancel in band command: {err:?}");
            }
            return;
        }

        let terminal_model = self.terminal_model.clone();
        let callback_command_id = command_id.clone();
        self.pending_writes.push_back(PtyWrite::Command {
            command: command.to_owned(),
            shell_type,
            in_band_command_id: Some(command_id),
            before_write_fn: Some(Box::new(move || {
                let mut terminal_model = terminal_model.lock();
                let outcome = terminal_model.start_in_band_command_execution();
                if !outcome.is_accepted()
                    && let Err(err) = block_on(cancel_tx.send(InBandCommandCancelledEvent {
                        command_id: callback_command_id.clone(),
                    }))
                {
                    log::warn!("Pty Controller failed to cancel rejected in band command: {err:?}");
                }
                outcome
            })),
        });

        self.execute_next_queued_write(ctx);
    }

    /// Returns whether we can currently write to the pty, or if we need to
    /// enqueue writes for later.
    fn can_write_to_pty(&self, ctx: &mut ModelContext<Self>) -> bool {
        self.line_editor_status.as_ref(ctx).is_line_editor_active()
    }

    /// Executes the next queued `PtyWrite`, if able.
    ///
    /// This is a no-op if the line editor is currently inactive; in the constructor of
    /// PtyController, a subscription is registered on `LineEditorStatus` which calls this function
    /// when the line editor becomes active.
    fn execute_next_queued_write(&mut self, ctx: &mut ModelContext<Self>) {
        if !self.can_write_to_pty(ctx) {
            return;
        }

        if let Some(write) = self.pending_writes.pop_front() {
            // `RunNativeShellCompletions` must be treated like `Command` here for the three
            // shells that put the shell into a synchronous foreground read (e.g. zsh's
            // `select`): draining the next queued write immediately would deliver it into that
            // read instead of a normal prompt, where it can be consumed and lost. PowerShell is
            // the exception -- its `RunNativeShellCompletions` write never puts the shell into
            // any such state (see `send_write_to_event_loop`'s handling of it), and nothing ever
            // transitions the line editor back to active for it the way a real command's precmd
            // hook would, so gating draining on it here would stall the queue forever.
            let is_command = match &write {
                PtyWrite::Command { .. } => true,
                PtyWrite::RunNativeShellCompletions { shell_type, .. } => {
                    *shell_type != ShellType::PowerShell
                }
                _ => false,
            };
            let did_write = self.send_write_to_event_loop(write, ctx);
            if !is_command || !did_write {
                self.execute_next_queued_write(ctx);
            }
        }
    }

    /// Writes a set of bytes to the PTY to begin bootstrapping a shell.
    pub(super) fn initialize_shell(
        &mut self,
        pending_session_info: &SessionInfo,
        ctx: &mut ModelContext<Self>,
    ) {
        let shell_type = pending_session_info.shell.shell_type();

        #[cfg(feature = "local_fs")]
        if let Some(path) = permanent_bootstrap_file(shell_type, pending_session_info) {
            // If there is a permanent bootstrap file, source it directly. We
            // currently only do this for local PowerShell sessions on Windows.
            self.source_bootstrap_script(path, shell_type, ctx);
            return;
        }

        let bootstrap = bootstrap::script_for_shell(shell_type, &crate::ASSETS);
        self.write_bootstrap_script_to_shell(pending_session_info, ctx, shell_type, bootstrap);
    }

    /// Writes the bytes to to terminate and run the bootstrap script.
    #[cfg(feature = "local_fs")]
    fn write_terminating_bootstrap_bytes(&mut self, ctx: &mut ModelContext<PtyController<T>>) {
        cfg_if::cfg_if! {
            if #[cfg(unix)] {
                self.write_bytes(&b"\n"[..], ctx);
            } else if #[cfg(target_os = "windows")] {
                self.write_bytes(&b"\r"[..], ctx);
            }
        }
    }

    #[cfg(feature = "local_fs")]
    fn write_bootstrap_script_to_shell(
        &mut self,
        pending_session_info: &SessionInfo,
        ctx: &mut ModelContext<PtyController<T>>,
        shell_type: ShellType,
        bootstrap: Cow<'static, [u8]>,
    ) {
        use super::bootstrap_file::create_bootstrap_file;
        use crate::terminal::ShellLaunchData;

        if bootstrap::should_use_rc_file_bootstrap_method(shell_type, pending_session_info) {
            let wsl_distribution = match (
                &pending_session_info.launch_data,
                pending_session_info.wsl_name.as_ref(),
            ) {
                (_, Some(wsl_name)) => Some(wsl_name),
                (Some(ShellLaunchData::WSL { distro }), _) => Some(distro),
                (
                    Some(ShellLaunchData::Executable { .. })
                    | Some(ShellLaunchData::MSYS2 { .. })
                    | Some(ShellLaunchData::DockerSandbox { .. })
                    | None,
                    _,
                ) => None,
            };
            // If creating the temporary file fails for any reason, we fall
            // back to the existing bracketed paste logic. Using bracketed paste
            // reduces the amount of reformatting that Fish tries to do and so improves
            // bootstrap speed. We need to add an explicit leading space, since Fish
            // automatically trims the input when performing a bracketed paste.
            match create_bootstrap_file(&bootstrap, shell_type, wsl_distribution) {
                Some(file) => {
                    if let Some(path) = file.path_as_bytes() {
                        self.source_bootstrap_script(path, shell_type, ctx);
                    } else {
                        self.write_terminating_bootstrap_bytes(ctx);
                        report_error!("Could not convert bootstrap script file path to str");
                    }

                    self.bootstrap_file = Some(file);
                }
                _ => {
                    self.write_bytes(&b" "[..], ctx);
                    self.write_bytes(escape_sequences::BRACKETED_PASTE_START, ctx);
                    self.write_bytes(bootstrap, ctx);
                    self.write_bytes(escape_sequences::BRACKETED_PASTE_END, ctx);
                    self.write_terminating_bootstrap_bytes(ctx);
                }
            }
        } else if bootstrap::is_container_subshell(pending_session_info) {
            // Write in 4KB chunks with 50ms delays to avoid overwhelming
            // PTY buffers in container exec sessions (podman/docker exec -it),
            // where the double-PTY proxy drops data for large writes.
            const CHUNK_SIZE: usize = 4096;
            let bytes: Vec<u8> = bootstrap.into_owned();
            let chunks: Vec<Vec<u8>> = bytes.chunks(CHUNK_SIZE).map(|c| c.to_vec()).collect();
            for (i, chunk) in chunks.into_iter().enumerate() {
                ctx.spawn(
                    warpui::r#async::Timer::after(std::time::Duration::from_millis(i as u64 * 50)),
                    move |me, _, ctx| me.write_bytes(chunk, ctx),
                );
            }
        } else {
            self.write_bytes(bootstrap, ctx);
        }
    }

    #[cfg(feature = "local_fs")]
    /// Sources the bootstrap script at the given path. Assumes that the path
    /// contains a valid file.
    fn source_bootstrap_script(
        &mut self,
        path_to_script: Vec<u8>,
        shell_type: ShellType,
        ctx: &mut ModelContext<Self>,
    ) {
        // TODO(CORE-2099): Figure out a more robust solution here. Fish users
        // can redefine these functions via fish functions. Ideally this won't
        // break if the user redefines the `source` or `.` built-in.
        match shell_type {
            ShellType::PowerShell => {
                let path_str = String::from_utf8_lossy(&path_to_script);
                let escaped = ShellFamily::PowerShell.escape(&path_str).into_owned();
                self.write_bytes(b" . ", ctx);
                self.write_bytes(escaped.into_bytes(), ctx);
            }
            _ => {
                self.write_bytes(b" source '", ctx);
                self.write_bytes(path_to_script, ctx);
                self.write_bytes(b"'", ctx);
            }
        }
        self.write_terminating_bootstrap_bytes(ctx);
    }

    #[cfg(not(feature = "local_fs"))]
    fn write_bootstrap_script_to_shell(
        &mut self,
        _pending_session_info: &SessionInfo,
        ctx: &mut ModelContext<PtyController<T>>,
        _shell_type: ShellType,
        bootstrap: Cow<'static, [u8]>,
    ) {
        self.write_bytes(bootstrap, ctx);
    }

    /// Handles the shell having finished bootstrapping.
    fn shell_bootstrapped(&mut self, is_subshell: bool) {
        if is_subshell {
            self.is_user_command_executing = false;
        }

        // Now that we have bootstrapped, we can be sure that the bootstrap
        // file is no longer needed.
        #[cfg(not(target_family = "wasm"))]
        self.bootstrap_file.take();
    }

    /// Converts the given `command` into a byte array and writes its corresponding bytes to the PTY.
    ///
    /// If the line editor is active, the command is written immediately. Otherwise, the command is
    /// written when the line editor becomes active.
    ///
    /// This also clears pending_writes, since the priority is to execute the user's command.
    ///
    /// The exact sequence of corresponding bytes depends on the given `shell`. For example, if the
    /// shell supports bracketed paste, the command's bytes may be wrapped in bracketed paste byte
    /// sequences.
    pub fn write_command(
        &mut self,
        command: &str,
        shell_type: ShellType,
        source: CommandExecutionSource,
        ctx: &mut ModelContext<Self>,
    ) -> StartCommandOutcome {
        {
            let mut model = self.terminal_model.lock();

            // Explicitly start the block now that the command is executed.
            let outcome = match source {
                CommandExecutionSource::AI { metadata } => {
                    model.start_command_execution_with_ai_metadata(metadata)
                }
                CommandExecutionSource::SharedSession {
                    participant_id,
                    ai_metadata,
                    ..
                } => model.start_command_execution_for_shared_session(participant_id, ai_metadata),
                CommandExecutionSource::User | CommandExecutionSource::QueuedCommand => {
                    model.start_command_execution()
                }
                CommandExecutionSource::EnvVarCollection { metadata } => {
                    model.start_command_execution_from_env_var_collection(metadata)
                }
            };
            if !outcome.is_accepted() {
                return outcome;
            }

            // Ensure that the `TerminalModel` doesn't interpret any of the PTY output from the
            // following commands as in-band command output. If the in-band command output is not
            // currently being received by the `TerminalModel`, this is a no-op.
            model.end_in_band_command_output(false);
        }

        self.pending_writes.clear();
        self.is_user_command_executing = true;

        // Send the write to the PTY event loop.
        let write = PtyWrite::Command {
            command: command.to_owned(),
            shell_type,
            in_band_command_id: None,
            before_write_fn: None,
        };
        if self.can_write_to_pty(ctx) {
            // Cancel the async writer task and clear the async write queue.
            // Check if line editor is active
            self.send_write_to_event_loop(write, ctx);
        } else {
            self.pending_writes.push_back(write);
        }
        StartCommandOutcome::Accepted
    }

    /// Synchronously writes the EOT (End-of-Transmission) char to the PTY.
    pub fn write_end_of_transmission_char(&mut self, ctx: &mut ModelContext<Self>) {
        self.write_bytes(&[escape_sequences::C0::EOT][..], ctx);

        // Consider the active block to be "started" since a user performed an action that
        // results in bytes being written to the pty. This makes the output from ctrl-d during ssh
        // get written to the active block.
        // TODO: reconsider this behavior since the output was not the result of a command, and given the function name
        // is start_command_execution and no command was executed.
        self.terminal_model.lock().start_command_execution();
    }

    /// Interrupts the foreground PTY process.
    #[cfg(not(target_family = "wasm"))]
    pub fn write_interrupt(&mut self, ctx: &mut ModelContext<Self>) {
        self.write_bytes(&[escape_sequences::C0::ETX][..], ctx);
    }

    /// Resizes the PTY's size (i.e. its notion of the number of columns and rows in the screen) via
    /// ioctl system call and updates the terminal model as appropriate.
    pub fn resize_pty(&self, size_update: SizeUpdate, ctx: &mut ModelContext<Self>) {
        // Send a message to the PTY event loop to resize the PTY.
        // We also need to resize when rows/cols changed without a pane size change
        // (e.g. ViewerSizeReported on the sharer side).
        if size_update.pane_size_changed()
            || size_update.is_refresh()
            || size_update.rows_or_columns_changed()
        {
            self.send_message_to_event_loop(Message::Resize(size_update.new_size), ctx);
        }
    }

    /// Writes agent input to the PTY.
    pub fn write_agent_bytes<B: Into<Cow<'static, [u8]>>>(
        &mut self,
        bytes: B,
        mode: &AIAgentPtyWriteMode,
        ctx: &mut ModelContext<Self>,
    ) {
        self.send_write_to_event_loop(
            PtyWrite::AgentInput {
                bytes: bytes.into(),
                mode: *mode,
            },
            ctx,
        );
    }

    /// Writes user input to the PTY.
    ///
    /// This should only be called for non-command input (e.g. input that should be passed through
    /// in a long-running command or in the alt screen, rather than from the input editor).
    pub fn write_bytes<B: Into<Cow<'static, [u8]>>>(
        &mut self,
        bytes: B,
        ctx: &mut ModelContext<Self>,
    ) {
        self.send_write_to_event_loop(
            PtyWrite::Bytes {
                bytes: bytes.into(),
            },
            ctx,
        );
    }

    /// Shuts down the pty and event loop.
    pub fn shutdown_pty(&mut self, ctx: &mut ModelContext<Self>) {
        self.send_message_to_event_loop(Message::Shutdown, ctx);
    }

    /// Sends a message to the event loop thread requesting a PTY write for the given `bytes`.
    ///
    /// If the write corresponds to a command, this also calls
    /// [`LineEditorStatus::did_execute_command()`].
    fn send_write_to_event_loop(&mut self, write: PtyWrite, ctx: &mut ModelContext<Self>) -> bool {
        let (bytes_to_write, is_for_command, on_write_fn, shell_type_for_split) = match write {
            PtyWrite::Command {
                command,
                shell_type,
                before_write_fn: on_write_fn,
                ..
            } => (
                Cow::Owned(bytes_to_execute_command(
                    command.as_str(),
                    shell_type,
                    self.is_bracketed_paste_enabled,
                )),
                true,
                on_write_fn,
                Some(shell_type),
            ),
            PtyWrite::AgentInput { bytes, mode } => {
                let decorated_bytes =
                    mode.decorate_bytes(bytes.into_owned(), self.is_bracketed_paste_enabled);
                (decorated_bytes.into(), false, None, None)
            }
            PtyWrite::Bytes { bytes } => (bytes, false, None, None),
            PtyWrite::RunNativeShellCompletions {
                command,
                shell_type,
                results_tx,
                buffer_text,
            } => {
                self.in_flight_native_completions_results_tx = Some(results_tx);

                if shell_type == ShellType::PowerShell {
                    // PowerShell can reach its completion engine directly from a PSReadLine key
                    // handler (see POWERSHELL_NATIVE_COMPLETIONS_TRIGGER's doc comment) without
                    // ever treating anything as a command to execute -- structurally the same
                    // trick zsh's `select` uses to reach a real completion context without
                    // faking a command. `command` here is just the hex-encoded buffer text (see
                    // `generator_command_for`'s `ShellType::PowerShell` case), typed as ordinary
                    // characters immediately followed by the trigger chord. Because nothing ever
                    // executes -- no kill-buffer, no Enter, no preexec/precmd -- there is nothing
                    // to restore afterward, unlike the other three shells: deliberately leave
                    // `in_flight_native_completions_buffer_text` unset.
                    self.terminal_model.lock().push_expected_echo(&command);
                    let mut bytes_to_write = command.into_bytes();
                    bytes_to_write.extend_from_slice(POWERSHELL_NATIVE_COMPLETIONS_TRIGGER);
                    (Cow::Owned(bytes_to_write), false, None, None)
                } else {
                    self.in_flight_native_completions_buffer_text = Some(buffer_text);

                    // Write the generator command exactly as any other in-band command: the
                    // shell's own bootstrap logic (matched by name, see
                    // `native_shell_completions`) hides it from history and treats its output as
                    // in-band rather than a new block.
                    let terminal_model = self.terminal_model.clone();
                    (
                        Cow::Owned(bytes_to_execute_command(
                            command.as_str(),
                            shell_type,
                            self.is_bracketed_paste_enabled,
                        )),
                        true,
                        Some(Box::new(move || {
                            terminal_model.lock().start_in_band_command_execution()
                        })
                            as Box<dyn Fn() -> StartCommandOutcome + Send + 'static>),
                        Some(shell_type),
                    )
                }
            }
        };

        // The terminal hangs if we send 0 bytes through.
        if bytes_to_write.is_empty() {
            return false;
        }

        if let Some(on_write_fn) = on_write_fn
            && !on_write_fn().is_accepted()
        {
            return false;
        }

        if is_for_command {
            self.line_editor_status
                .update(ctx, |line_editor_status, ctx| {
                    line_editor_status.did_execute_command(ctx)
                });
        }

        // PowerShell's kill-buffer chord (`Alt+2`, an ESC-prefixed two-byte sequence -- see
        // `ShellType::kill_buffer_bytes`) is not reliably disambiguated by PSReadLine when it
        // arrives concatenated with the command text that follows it in a single write/read:
        // empirically, PSReadLine sometimes fails to recognize the chord at all in that case,
        // leaving the existing buffer untouched and the command text typed literally on top of
        // it. Splitting the chord into its own pty write -- even with no explicit delay before
        // the second write -- reliably avoids this. The other three shells use a single,
        // unambiguous control byte for this (no escape-sequence parsing involved), so no split
        // is needed for them.
        if let Some(shell_type) = shell_type_for_split
            && let Some((kill_buffer, rest)) = split_kill_buffer_write(&bytes_to_write, shell_type)
        {
            self.send_message_to_event_loop(Message::Input(Cow::Owned(kill_buffer.to_vec())), ctx);
            self.send_message_to_event_loop(Message::Input(Cow::Owned(rest.to_vec())), ctx);
            return true;
        }

        self.send_message_to_event_loop(Message::Input(bytes_to_write), ctx);
        true
    }

    /// Sends a message to the event loop. If the send fails with `SendError::Disconnected`, emits
    /// a `PtyDisconnected` event.
    fn send_message_to_event_loop(&self, message: Message, ctx: &mut ModelContext<Self>) {
        match self.event_loop_tx.send(message) {
            Err(EventLoopSendError::Disconnected) => {
                ctx.emit(PtyControllerEvent::PtyDisconnected);
            }
            Err(e) => {
                log::warn!("Unable to send event loop msg {e:?}");
            }
            _ => (),
        }
    }

    pub fn run_native_shell_completions(
        &mut self,
        buffer_text: String,
        results_tx: async_channel::Sender<Vec<ShellCompletion>>,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(shell_type) = self
            .model_event_dispatcher
            .as_ref(ctx)
            .active_session_id()
            .and_then(|id| self.sessions.as_ref(ctx).get(id))
            .map(|session| session.shell().shell_type())
        else {
            let _ = results_tx.try_send(Vec::new());
            return;
        };
        let command = native_shell_completions::generator_command_for(shell_type, &buffer_text);

        // Make sure we only have a single pending native shell completions
        // request at a time by dropping any existing ones from the queue.
        let len_before = self.pending_writes.len();
        self.pending_writes
            .retain(|write| !matches!(write, PtyWrite::RunNativeShellCompletions { .. }));
        let dropped = len_before - self.pending_writes.len();
        log::debug!(
            "PHANTOM_DIAG run_native_shell_completions({buffer_text:?}): in_flight_buffer_text_already_set={:?}, in_flight_results_tx_already_set={}, dropped_stale_queued_requests={dropped}",
            self.in_flight_native_completions_buffer_text,
            self.in_flight_native_completions_results_tx.is_some()
        );

        self.pending_writes
            .push_back(PtyWrite::RunNativeShellCompletions {
                command,
                shell_type,
                results_tx,
                buffer_text,
            });
        self.execute_next_queued_write(ctx);
    }
}

pub enum PtyControllerEvent {
    /// Emitted when the event loop thread has exited.
    PtyDisconnected,
}

impl<T: EventLoopSender> Entity for PtyController<T> {
    type Event = PtyControllerEvent;
}

/// If `shell_type`'s kill-buffer bytes need to be written to the pty as their own write, separate
/// from the rest of `bytes` (the full output of `bytes_to_execute_command`), returns
/// `Some((kill_buffer_bytes, rest))`. Returns `None` if no split is needed, in which case `bytes`
/// should be sent as a single write. See the call site in `send_write_to_event_loop` for why this
/// is currently only needed for PowerShell.
fn split_kill_buffer_write(bytes: &[u8], shell_type: ShellType) -> Option<(&[u8], &[u8])> {
    if shell_type != ShellType::PowerShell {
        return None;
    }
    let kill_buffer_len = shell_type.kill_buffer_bytes().len();
    if bytes.len() <= kill_buffer_len {
        return None;
    }
    Some(bytes.split_at(kill_buffer_len))
}

/// Returns the shell-dependent array of bytes to be written to the PTY to execute `command`.
fn bytes_to_execute_command(
    command: &str,
    shell_type: ShellType,
    is_bracketed_paste_enabled: bool,
) -> Vec<u8> {
    let mut command_bytes = shell_type.kill_buffer_bytes().to_vec();
    let command = match ShellFamily::from(shell_type) {
        ShellFamily::Posix if cfg!(windows) => LINEFEED_REGEX.replace_all(command, "\n"),
        ShellFamily::PowerShell => LINEFEED_REGEX.replace_all(command, "\r"),
        _ => Cow::Borrowed(command),
    };

    // Only execute the command via bracketed paste if the command is not empty. Some ZSH
    // bracketed paste magic functions return errors if bracketed paste is used without text
    // in-between the bracketed paste escape sequences.
    if is_bracketed_paste_enabled && !command.is_empty() {
        match shell_type {
            ShellType::Fish => {
                // Fish strips leading (and trailing) whitespace from pasted commands (entered via
                // bracketed paste). To ensure that leading whitespace is preserved, first append
                // leading whitespace bytes and then surround the remaining command string with the
                // bracketed paste sequence. Conceptually, this would be like manually typing in
                // the whitespace into the fish line editor, and then pasting in the command.
                //
                // The leading whitespace is particularly meaningful in fish because it causes the
                // following command to be omitted from history (like the HISTIGNORESPACE option
                // in zsh).
                //
                // We don't care about preserving trailing whitespace; it would just take up
                // unnecessary space in the blocklist.
                let (leading_whitespace, rest_of_command) =
                    command.split_at(command.len() - command.trim_start().len());
                command_bytes.extend(leading_whitespace.as_bytes());
                command_bytes.extend(wrap_bytes_in_bracketed_paste(
                    rest_of_command
                        .replace(escape_sequences::C0::ESC as char, "")
                        .into_bytes(),
                ));
            }
            _ => command_bytes.extend(wrap_bytes_in_bracketed_paste(
                command
                    .replace(escape_sequences::C0::ESC as char, "")
                    .into_bytes(),
            )),
        }
    } else {
        let command_without_escapes = command.replace(escape_sequences::C0::ESC as char, "");
        command_bytes.extend(command_without_escapes.as_bytes());
    }
    command_bytes.extend(shell_type.execute_command_bytes().to_vec());
    command_bytes
}

/// Returns a vector containing the given `bytes` wrapped in bracketed paste start and end
/// sequences.
fn wrap_bytes_in_bracketed_paste(bytes: impl IntoIterator<Item = u8>) -> impl Iterator<Item = u8> {
    escape_sequences::BRACKETED_PASTE_START
        .iter()
        .copied()
        .chain(bytes)
        .chain(escape_sequences::BRACKETED_PASTE_END.iter().copied())
}

#[cfg(test)]
#[path = "pty_controller_command_bytes_tests.rs"]
mod command_bytes_tests;
#[cfg(test)]
#[path = "pty_controller_lifecycle_tests.rs"]
mod lifecycle_tests;

#[derive(Error, Debug)]
pub enum EventLoopSendError {
    #[error("Unable to send message: receiver is disconnected")]
    Disconnected,
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub trait EventLoopSender: 'static {
    fn send(&self, message: Message) -> Result<(), EventLoopSendError>;
}
