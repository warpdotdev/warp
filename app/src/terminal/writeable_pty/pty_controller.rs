use std::{borrow::Cow, collections::VecDeque, sync::Arc};

use async_channel::{Receiver, Sender};
use parking_lot::FairMutex;
use thiserror::Error;
use warpui::r#async::block_on;
use warpui::{Entity, ModelContext, ModelHandle, SingletonEntity};

use crate::ai::agent::AIAgentPtyWriteMode;
use crate::terminal::input::CommandExecutionSource;
use crate::terminal::model::completions::ShellCompletion;
use crate::terminal::model::session::{
    ExecutorCommandEvent, InBandCommandCancelledEvent, Sessions,
};
use crate::terminal::model::tmux::commands::TmuxCommand;
use crate::terminal::model_events::AnsiHandlerEvent;
use crate::terminal::view::LINEFEED_REGEX;
#[cfg(not(target_family = "wasm"))]
use crate::terminal::writeable_pty::bootstrap_file::{permanent_bootstrap_file, TempBootstrapFile};
use crate::terminal::{
    bootstrap,
    line_editor_status::{LineEditorStatus, LineEditorStatusEvent},
    model::{ansi::Handler, escape_sequences, session::SessionInfo},
    model_events::{ModelEvent, ModelEventDispatcher},
    shell::ShellType,
    SizeUpdate, TerminalModel,
};
use crate::SessionSettings;

use super::Message;

/// Byte sequence to emulate the user pressing ENTER, used to execute a command in the shell.
const COMMAND_ENTER: &[u8] = &[escape_sequences::C0::CR, escape_sequences::C0::LF];
/// Used to let the shell know we are switching to the PS1 prompt via a bindkey \ep. This will
/// restore the PS1 from the saved PS1 value (we had unset the PS1 for Warp prompt).
const SWITCH_TO_PS1_ESCAPE_SEQUENCE: &[u8] = &[escape_sequences::C0::ESC, b'p'];
/// Used to let the shell know we are switching to the Warp prompt via a bindkey \ew. This will
/// unset the PS1 to ensure we don't have a double prompt (PS1 and Warp prompt).
const SWITCH_TO_WARP_PROMPT_ESCAPE_SEQUENCE: &[u8] = &[escape_sequences::C0::ESC, b'w'];

/// Represents a single call to write bytes to the PTY asynchronously.
enum PtyWrite {
    Command {
        command: String,
        shell_type: ShellType,
        /// The id if the command is an in-band command or `None` if the command is not an in-band
        /// command.
        in_band_command_id: Option<String>,
        /// If 'some', the given callback is called right before the bytes are written to the PTY.
        before_write_fn: Option<Box<dyn Fn() + Send + 'static>>,
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
    TmuxCommand(TmuxCommand),
    RunNativeShellCompletions(NativeShellCompletionsState),
}

enum NativeShellCompletionsState {
    AwaitingPrompt {
        buffer_text: String,
        results_tx: async_channel::Sender<Vec<ShellCompletion>>,
    },
    AwaitingResults {
        results_tx: async_channel::Sender<Vec<ShellCompletion>>,
    },
}

impl NativeShellCompletionsState {
    fn is_awaiting_prompt(&self) -> bool {
        matches!(self, Self::AwaitingPrompt { .. })
    }
}

enum TmuxControlMode {
    /// Tmux control mode is started, but we don't have the primary pane yet.
    Pending { buffer: Vec<u8> },
    /// Tmux control mode is active.
    Active { primary_pane: u32 },
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
    tmux_control_mode: Option<TmuxControlMode>,
    in_flight_native_completions_state: Option<NativeShellCompletionsState>,
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
        ctx.subscribe_to_model(&model_event_dispatcher, |me, event, ctx| match event {
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
            ModelEvent::Handler(AnsiHandlerEvent::StartTmuxControlMode) => {
                me.tmux_control_mode = Some(TmuxControlMode::Pending {
                    buffer: Default::default(),
                });
            }
            ModelEvent::Handler(AnsiHandlerEvent::RunTmuxCommand(command)) => {
                me.send_write_to_event_loop(PtyWrite::TmuxCommand(command.to_owned()), ctx);
            }
            ModelEvent::Handler(AnsiHandlerEvent::EndTmuxControlMode) => {
                me.tmux_control_mode = None;
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
            ModelEvent::Handler(AnsiHandlerEvent::TmuxControlModeReady { primary_pane }) => {
                let previous_control_mode_state = me.tmux_control_mode.replace(TmuxControlMode::Active {
                        primary_pane: *primary_pane,
                    });
                if let Some(TmuxControlMode::Pending { buffer }) = previous_control_mode_state {
                    me.send_write_to_event_loop(
                        PtyWrite::Bytes {
                            bytes: Cow::Owned(buffer),
                        },
                        ctx,
                    );
                }
            }
            ModelEvent::CompletionsFinished(data) => {
                let Some(NativeShellCompletionsState::AwaitingResults { results_tx }) = me.in_flight_native_completions_state.take() else {
                    log::warn!("Received CompletionsFinished event but didn't have a channel to send results over!");
                    return;
                };
                let _ = block_on(results_tx.send(data.clone()));
            }
            ModelEvent::SendCompletionsPrompt => {
                let Some(NativeShellCompletionsState::AwaitingPrompt {
                    buffer_text,
                    results_tx,
                }) = me.in_flight_native_completions_state.take() else {
                    log::warn!("Received SendCompletionsPrompt event but didn't have a prompt to send!");
                    return;
                };
                me.in_flight_native_completions_state = Some(NativeShellCompletionsState::AwaitingResults { results_tx });

                let mut bytes = buffer_text.into_bytes();
                // We use the EOT character to signal the end of the prompt.
                bytes.push(escape_sequences::C0::EOT);

                // We send the write directly to the event loop without
                // queueing, as we currently have exclusive control over pty
                // writes.
                me.send_write_to_event_loop(
                    PtyWrite::Bytes {
                        bytes: bytes.into(),
                    },
                    ctx,
                );

                // Now that we've provided the prompt, we can start executing
                // other queued writes.
                me.execute_next_queued_write(ctx);
            }
            _ => (),
        });

        ctx.subscribe_to_model(&line_editor_status, |me, event, ctx| {
            if let LineEditorStatusEvent::Active = event {
                let input_reporting_seq = me
                    .model_event_dispatcher
                    .as_ref(ctx)
                    .active_session_id()
                    .and_then(|id| me.sessions.as_ref(ctx).get(id))
                    .and_then(|session| session.shell().input_reporting_sequence());
                if let Some(bytes) = input_reporting_seq {
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
                ExecutorCommandEvent::ExecuteTmuxCommand(command) => {
                    me.send_write_to_event_loop(PtyWrite::TmuxCommand(command), ctx);
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
            tmux_control_mode: None,
            in_flight_native_completions_state: None,
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
        self.pending_writes.push_back(PtyWrite::Command {
            command: command.to_owned(),
            shell_type,
            in_band_command_id: Some(command_id),
            before_write_fn: Some(Box::new(move || {
                let mut terminal_model = terminal_model.lock();
                terminal_model
                    .block_list_mut()
                    .start_active_block_for_in_band_command();
            })),
        });

        self.execute_next_queued_write(ctx);
    }

    /// Returns whether we can currently write to the pty, or if we need to
    /// enqueue writes for later.
    fn can_write_to_pty(&self, ctx: &mut ModelContext<Self>) -> bool {
        self.line_editor_status.as_ref(ctx).is_line_editor_active()
            // If we're in the middle of a native completions request, we should not send any more
            // writes to the shell until we've sent the string to complete.
            && !self.in_flight_native_completions_state.as_ref().is_some_and(|state| state.is_awaiting_prompt())
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
            let is_command = matches!(write, PtyWrite::Command { .. });
            self.send_write_to_event_loop(write, ctx);
            if !is_command {
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
            if let Some(file) = create_bootstrap_file(&bootstrap, shell_type, wsl_distribution) {
                if let Some(path) = file.path_as_bytes() {
                    self.source_bootstrap_script(path, shell_type, ctx);
                } else {
                    self.write_terminating_bootstrap_bytes(ctx);
                    log::error!("Could not convert bootstrap script file path to str");
                }

                self.bootstrap_file = Some(file);
            } else {
                self.write_bytes(&b" "[..], ctx);
                self.write_bytes(escape_sequences::BRACKETED_PASTE_START, ctx);
                self.write_bytes(bootstrap, ctx);
                self.write_bytes(escape_sequences::BRACKETED_PASTE_END, ctx);
                self.write_terminating_bootstrap_bytes(ctx);
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
        use warp_util::path::ShellFamily;

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
    ) {
        {
            let mut model = self.terminal_model.lock();

            // Explicitly start the block now that the command is executed.
            match source {
                CommandExecutionSource::AI { metadata } => {
                    model.start_command_execution_with_ai_metadata(metadata)
                }
                CommandExecutionSource::SharedSession {
                    participant_id,
                    ai_metadata,
                    ..
                } => model.start_command_execution_for_shared_session(participant_id, ai_metadata),
                CommandExecutionSource::User => model.start_command_execution(),
                CommandExecutionSource::EnvVarCollection { metadata } => {
                    model.start_command_execution_from_env_var_collection(metadata)
                }
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
    fn send_write_to_event_loop(&mut self, write: PtyWrite, ctx: &mut ModelContext<Self>) {
        let (bytes_to_write, is_for_command, on_write_fn, raw_tmux_command) = match write {
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
                false,
            ),
            PtyWrite::AgentInput { bytes, mode } => {
                let decorated_bytes =
                    mode.decorate_bytes(bytes.into_owned(), self.is_bracketed_paste_enabled);
                (decorated_bytes.into(), false, None, false)
            }
            PtyWrite::Bytes { bytes } => (bytes, false, None, false),
            PtyWrite::TmuxCommand(command) => {
                let command = command.get_command_string();
                debug_assert!(
                    command.ends_with('\n'),
                    "Tmux commands must end in a newlines so they are executed"
                );
                debug_assert!(
                    self.tmux_control_mode.is_some(),
                    "Received tmux command outside of control mode."
                );
                (command.into_bytes().into(), false, None, true)
            }
            PtyWrite::RunNativeShellCompletions(state) => {
                self.in_flight_native_completions_state = Some(state);

                // Send a ^Y control code to trigger the right bindkey.  We
                // then wait for an OSC-based signal from the shell before we
                // send the text that needs to be completed.
                let bytes = vec![0x19_u8];
                (bytes.into(), false, None, false)
            }
        };

        // The terminal hangs if we send 0 bytes through.
        if bytes_to_write.is_empty() {
            return;
        }

        let bytes_to_write = match &mut self.tmux_control_mode {
            None => bytes_to_write,
            Some(_) if raw_tmux_command => bytes_to_write,
            Some(TmuxControlMode::Pending { buffer }) => {
                buffer.extend_from_slice(&bytes_to_write);
                return;
            }
            Some(TmuxControlMode::Active { primary_pane }) => {
                crate::terminal::model::tmux::format_input(*primary_pane, &bytes_to_write)
                    .as_bytes()
                    .to_owned()
                    .into()
            }
        };

        if is_for_command {
            self.line_editor_status
                .update(ctx, |line_editor_status, ctx| {
                    line_editor_status.did_execute_command(ctx)
                });
        }

        if let Some(on_write_fn) = on_write_fn {
            on_write_fn();
        }

        self.send_message_to_event_loop(Message::Input(bytes_to_write), ctx);
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
        // Make sure we only have a single pending native shell completions
        // request at a time by dropping any existing ones from the queue.
        self.pending_writes
            .retain(|write| !matches!(write, PtyWrite::RunNativeShellCompletions(_)));

        self.pending_writes
            .push_back(PtyWrite::RunNativeShellCompletions(
                NativeShellCompletionsState::AwaitingPrompt {
                    buffer_text,
                    results_tx,
                },
            ));
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

/// Returns the shell-dependent array of bytes to be written to the PTY to execute `command`.
fn bytes_to_execute_command(
    command: &str,
    shell_type: ShellType,
    is_bracketed_paste_enabled: bool,
) -> Vec<u8> {
    let mut command_bytes = shell_type.kill_buffer_bytes().to_vec();

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
        // This is a fix for PLAT-770 to allow multi-line commands in Powershell.
        // In general, shells without bracketed paste don't handle `\n` that well,
        // and in the case of PowerShell, it is explicitly ignored.
        let command_without_newlines = LINEFEED_REGEX.replace_all(&command_without_escapes, "\r");
        command_bytes.extend(command_without_newlines.as_bytes());
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
