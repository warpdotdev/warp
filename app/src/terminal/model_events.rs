use crate::server::telemetry::ImageProtocol;
use crate::terminal::model::session::Sessions;

use crate::terminal::event::{
    AfterBlockCompletedEvent, BlockCompletedEvent, BlockMetadataReceivedEvent, Event,
    ExecutedExecutorCommandEvent, InitSshEvent, InitSubshellEvent, SourcedRcFileInSubshellEvent,
    TerminalMode,
};

use crate::terminal::ClipboardType;
use async_channel::Receiver;
use instant::Instant;
use std::sync::Arc;

use crate::remote_server::manager::RemoteServerManager;
use warpui::SingletonEntity;
use warpui::{Entity, ModelContext, ModelHandle};

use super::event::SshLoginStatus;
use super::model::ansi::{FinishUpdateValue, WarpificationUnavailableReason};
use super::model::block::BlockId;
use super::model::completions::ShellCompletion;
use super::model::terminal_model::{ExitReason, TmuxControlModeContext, TmuxInstallationState};
use super::model::tmux::commands::TmuxCommand;
use super::{
    event::BootstrappedEvent,
    model::{
        ansi,
        session::{IsLegacySSHSession, SessionId, SessionInfo},
        terminal_model::{CommandType, HandlerEvent},
    },
};
use crate::features::FeatureFlag;
use crate::terminal::shell::ShellType;
use crate::{send_telemetry_from_ctx, TelemetryEvent};

/// Model that dispatches events that have been emitted by the [`crate::terminal::TerminalModel`],
/// allowing other models/views to subscribe to `TerminalModel` events like it would any other
/// entity within the UI framework.
pub struct ModelEventDispatcher {
    last_start_prompt_marker: Option<PromptKind>,
    active_session_id: Option<SessionId>,
    sessions: ModelHandle<Sessions>,
}

impl ModelEventDispatcher {
    pub fn new(
        model_events_rx: Receiver<Event>,
        sessions: ModelHandle<Sessions>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        ctx.spawn_stream_local(
            model_events_rx,
            Self::handle_terminal_model_event,
            |_, _| (),
        );
        Self {
            active_session_id: None,
            last_start_prompt_marker: None,
            sessions,
        }
    }

    /// Returns the active session to which the PTY is currently attached.
    ///
    /// The active session is the session corresponding to the session ID included in the most
    /// recent Precmd payload.
    pub fn active_session_id(&self) -> Option<SessionId> {
        self.active_session_id
    }

    /// Sets the active session ID directly, for use in unit tests where there's no `Precmd` event.
    #[cfg(test)]
    pub fn set_active_session_id(&mut self, session_id: SessionId) {
        self.active_session_id = Some(session_id);
    }

    /// Emits the corresponding `ModelEvent` for the received `HandlerEvent` emitted by
    /// `TerminalModel` when some `ansi::Handler` method is called.
    fn handle_terminal_model_event(&mut self, event: Event, ctx: &mut ModelContext<Self>) {
        let event_to_emit = match event {
            Event::Handler(HandlerEvent::InitShell {
                pending_session_info,
            }) => {
                self.sessions.update(ctx, |sessions, ctx| {
                    sessions.register_pending_session(pending_session_info.as_ref(), ctx);
                });
                let is_legacy_ssh = matches!(
                    pending_session_info.is_legacy_ssh_session,
                    IsLegacySSHSession::Yes { .. }
                );
                if FeatureFlag::SshRemoteServer.is_enabled() && is_legacy_ssh {
                    ModelEvent::SshInitShell {
                        pending_session_info,
                    }
                } else {
                    ModelEvent::Handler(AnsiHandlerEvent::InitShell {
                        pending_session_info,
                    })
                }
            }
            Event::Handler(HandlerEvent::Bootstrapped(bootstrapped_event)) => {
                let session_id = bootstrapped_event.session_info.session_id;
                let is_subshell = bootstrapped_event.session_info.subshell_info.is_some();

                // Always initialize the session synchronously. When the
                // `SshRemoteServer` flag is enabled, the remote-server client
                // is wired up independently: `Sessions::new` subscribes to
                // `RemoteServerManagerEvent::SessionConnected` and attaches the
                // client to the session's `RemoteServerCommandExecutor` when
                // the connection lands, so it's safe to initialize the session
                // before the remote server finishes connecting.
                self.complete_bootstrapped_session(bootstrapped_event, ctx);

                ModelEvent::Handler(AnsiHandlerEvent::Bootstrapped {
                    session_id,
                    is_subshell,
                })
            }
            Event::RemoteServerReady { session_id } => {
                log::info!("Remote server ready for session {session_id:?}");
                return;
            }
            Event::RemoteServerFailed { session_id, error } => {
                log::warn!(
                    "Remote server setup failed for session {session_id:?}, falling back to \
                     ControlMaster: {error}"
                );
                return;
            }
            Event::Handler(HandlerEvent::PromptStart) => {
                self.last_start_prompt_marker = Some(PromptKind::Left);
                ModelEvent::Handler(AnsiHandlerEvent::StartPrompt)
            }
            Event::Handler(HandlerEvent::RPromptStart) => {
                self.last_start_prompt_marker = Some(PromptKind::Right);
                ModelEvent::Handler(AnsiHandlerEvent::StartRPrompt)
            }
            Event::Handler(HandlerEvent::PromptEnd) => match self.last_start_prompt_marker.take() {
                None | Some(PromptKind::Left) => ModelEvent::Handler(AnsiHandlerEvent::EndPrompt),
                Some(PromptKind::Right) => ModelEvent::Handler(AnsiHandlerEvent::EndRPrompt),
            },
            Event::Handler(HandlerEvent::Precmd {
                session_id,
                handled_after_inband,
                env_vars,
            }) => {
                // Update the active session to the one that corresponds to the received SessionId.
                self.active_session_id = session_id;

                // Update the active session's environment variables
                if let Some(session_id) = session_id {
                    // We set the environment variables here, which triggers the prompt to refresh
                    // as certain chips depend on environment variables. We specifically want to
                    // avoid triggering prompt refreshes for in-band commands because otherwise we'll
                    // create a loop if updating the prompt involves running an in-band command.
                    // This is similar to how we handle ModelEvent::AfterBlockCompleted in terminal_view.rs
                    if !handled_after_inband {
                        self.sessions.update(ctx, |sessions, ctx| {
                            sessions.set_env_vars_for_session(session_id, env_vars, ctx)
                        });
                    }
                }

                ModelEvent::Handler(AnsiHandlerEvent::Precmd)
            }
            Event::Handler(HandlerEvent::Preexec) => ModelEvent::Handler(AnsiHandlerEvent::Preexec),
            Event::Handler(HandlerEvent::CommandFinished { command_type }) => match command_type {
                CommandType::InBandCommand => {
                    ModelEvent::Handler(AnsiHandlerEvent::InBandCommandFinished)
                }
                CommandType::User => ModelEvent::Handler(AnsiHandlerEvent::UserCommandFinished),
                _ => return,
            },
            Event::Handler(HandlerEvent::SetMode {
                mode: ansi::Mode::BracketedPaste,
            }) => ModelEvent::Handler(AnsiHandlerEvent::SetBracketedPaste),
            Event::Handler(HandlerEvent::UnsetMode {
                mode: ansi::Mode::BracketedPaste,
            }) => ModelEvent::Handler(AnsiHandlerEvent::UnsetBracketedPaste),
            Event::Handler(HandlerEvent::StartTmuxControlMode) => {
                ModelEvent::Handler(AnsiHandlerEvent::StartTmuxControlMode)
            }
            Event::Handler(HandlerEvent::EndTmuxControlMode) => {
                ModelEvent::Handler(AnsiHandlerEvent::EndTmuxControlMode)
            }
            Event::Handler(HandlerEvent::TmuxControlModeReady {
                primary_pane,
                context,
            }) => {
                {
                    if let Some(TmuxControlModeContext::WarpInitiatedForSsh(control_mode)) = context
                    {
                        let duration_ms = Instant::now()
                            .duration_since(control_mode.start_time)
                            .as_millis()
                            // Clip large durations to u64::MAX
                            .min(u64::MAX as u128) as u64;
                        send_telemetry_from_ctx!(
                            TelemetryEvent::SshTmuxWarpificationSuccess {
                                duration_ms,
                                tmux_installation: control_mode.tmux_installation,
                            },
                            ctx
                        );
                    }
                }
                ModelEvent::Handler(AnsiHandlerEvent::TmuxControlModeReady { primary_pane })
            }
            Event::Handler(HandlerEvent::RunTmuxCommand(command)) => {
                ModelEvent::Handler(AnsiHandlerEvent::RunTmuxCommand(command))
            }
            Event::CompletionsFinished(res) => ModelEvent::CompletionsFinished(res),
            Event::MouseCursorDirty => ModelEvent::MouseCursorDirty,
            Event::Title(title) => ModelEvent::Title(title),
            Event::VisibleBootstrapBlock => ModelEvent::VisibleBootstrapBlock,
            Event::BlockCompleted(block_completed_event) => {
                ModelEvent::BlockCompleted(block_completed_event)
            }
            Event::AfterBlockCompleted(after_block_completed_event) => {
                ModelEvent::AfterBlockCompleted(after_block_completed_event)
            }
            Event::AfterBlockStarted {
                block_id,
                command,
                is_for_in_band_command,
            } => ModelEvent::AfterBlockStarted {
                block_id,
                command,
                is_for_in_band_command,
            },
            Event::BlockMetadataReceived(block_metadata_received_event) => {
                ModelEvent::BlockMetadataReceived(block_metadata_received_event)
            }
            Event::BackgroundBlockStarted => ModelEvent::BackgroundBlockStarted,
            Event::ClipboardStore(clipboard_type, text) => {
                ModelEvent::ClipboardStore(clipboard_type, text)
            }
            Event::ClipboardLoad(clipboard_type, clipboard_load) => {
                ModelEvent::ClipboardLoad(clipboard_type, clipboard_load)
            }
            Event::CursorBlinkingChange(is_blinking) => {
                ModelEvent::CursorBlinkingChange(is_blinking)
            }
            Event::TerminalClear => ModelEvent::TerminalClear,
            Event::TmuxControlModeReady { primary_pane } => {
                ModelEvent::TmuxControlModeReady { primary_pane }
            }
            Event::DetectedEndOfSshLogin(check_type) => {
                ModelEvent::DetectedEndOfSshLogin(check_type)
            }
            Event::RemoteWarpificationIsUnavailable(reason) => {
                ModelEvent::RemoteWarpificationIsUnavailable(reason)
            }
            Event::SshTmuxInstaller(tmux_installation) => {
                ModelEvent::SshTmuxInstaller(tmux_installation)
            }
            Event::TmuxInstallFailed { line, command } => {
                ModelEvent::TmuxInstallFailed { line, command }
            }
            Event::Bell => ModelEvent::Bell,
            Event::Exit { reason } => ModelEvent::Exit { reason },
            Event::PreInteractiveSSHSession => ModelEvent::PreInteractiveSSHSession,
            Event::SSH(ssh) => ModelEvent::SSH(ssh),
            Event::SSHControlMasterError => ModelEvent::SSHControlMasterError,
            Event::TerminalModeSwapped(terminal_mode) => {
                ModelEvent::TerminalModeSwapped(terminal_mode)
            }
            Event::ExecutedInBandCommand(executed_in_band_command_event) => {
                ModelEvent::ExecutedInBandCommand(executed_in_band_command_event)
            }
            Event::InitSubshell(init_subshell_event) => {
                ModelEvent::InitSubshell(init_subshell_event)
            }
            Event::SourcedRcFileInSubshell(sourced_rc_file_in_subshell_event) => {
                ModelEvent::SourcedRcFileInSubshell(sourced_rc_file_in_subshell_event)
            }
            Event::InitSsh(init_ssh_event) => ModelEvent::InitSsh(init_ssh_event),
            Event::PromptUpdated => ModelEvent::PromptUpdated,
            Event::HonorPS1OutOfSync => ModelEvent::HonorPS1OutOfSync,
            Event::Typeahead => ModelEvent::Typeahead,
            Event::FinishUpdate(data) => ModelEvent::FinishUpdate(data),
            Event::TextSelectionChanged => ModelEvent::SelectedTextChanged,
            Event::ShellSpawned(shell_type) => ModelEvent::ShellSpawned(shell_type),
            Event::SendCompletionsPrompt => ModelEvent::SendCompletionsPrompt,
            Event::ImageReceived {
                image_id,
                image_data,
                image_protocol,
            } => ModelEvent::ImageReceived {
                image_id,
                image_data,
                image_protocol,
            },
            Event::BootstrapPrecmdDone => ModelEvent::BootstrapPrecmdDone,
            Event::AgentTaggedInChanged { is_tagged_in } => {
                ModelEvent::AgentTaggedInChanged { is_tagged_in }
            }
            Event::PluggableNotification { title, body } => {
                ModelEvent::PluggableNotification { title, body }
            }
            Event::ExitShell { session_id } => ModelEvent::ExitShell { session_id },
            _ => return,
        };

        ctx.emit(event_to_emit);
    }

    /// Finalizes session initialization by calling `Sessions::initialize_bootstrapped_session`.
    ///
    /// For legacy SSH sessions with the `SshRemoteServer` flag, this also
    /// sends the `SessionBootstrapped` notification to the remote server via
    /// the manager.
    ///
    /// The `SessionBootstrapped` notification is sent **before** initializing
    /// the session so the daemon creates the `LocalCommandExecutor` before any
    /// subscriber (e.g. `TerminalView::handle_session_bootstrapped`) can queue
    /// a `RunCommand` request. Without this ordering, a race exists where the
    /// daemon receives `RunCommand` before the executor is ready, producing
    /// "No executor for RunCommand, session was never initialized" errors.
    fn complete_bootstrapped_session(
        &mut self,
        event: BootstrappedEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        let BootstrappedEvent {
            session_info,
            spawning_command,
            restored_block_commands,
            rcfiles_duration_seconds,
        } = event;

        let (is_legacy_ssh, session_id, shell_type_name, shell_path) = (
            matches!(
                session_info.is_legacy_ssh_session,
                IsLegacySSHSession::Yes { .. }
            ),
            session_info.session_id,
            session_info.shell.shell_type().name().to_owned(),
            session_info.shell.shell_path().clone(),
        );

        // Send the SessionBootstrapped notification to the daemon BEFORE
        // initializing the session. `initialize_bootstrapped_session` emits
        // `SessionsEvent::SessionBootstrapped`, which causes subscribers to
        // immediately queue `RunCommand` requests (e.g. `load_external_commands`).
        // The daemon must have the executor ready before those requests arrive.
        if FeatureFlag::SshRemoteServer.is_enabled() && is_legacy_ssh {
            RemoteServerManager::handle(ctx).update(ctx, |mgr, _ctx| {
                mgr.notify_session_bootstrapped(
                    session_id,
                    &shell_type_name,
                    shell_path.as_deref(),
                );
            });
        }

        self.sessions.update(ctx, |sessions, ctx| {
            sessions.initialize_bootstrapped_session(
                *session_info,
                spawning_command,
                restored_block_commands,
                rcfiles_duration_seconds,
                ctx,
            );
        });
    }

    /// Emits an event so `TerminalView` can render the remote server block.
    pub fn request_remote_server_block(
        &mut self,
        session_id: SessionId,
        ctx: &mut ModelContext<Self>,
    ) {
        ctx.emit(ModelEvent::RemoteServerBlockRequested { session_id });
    }
}

/// The type of prompt for which a `PromptStart` event has been received.
enum PromptKind {
    Left,
    Right,
}

/// Set of events that were dispatched from the [`crate::terminal::TerminalModel`] while parsing
/// PTY output.
pub enum ModelEvent {
    MouseCursorDirty,
    Title(String),
    VisibleBootstrapBlock,
    /// Performs the minimal work necessary to show that a block has completed.
    /// Treat this as a performance-sensitive path.
    BlockCompleted(BlockCompletedEvent),
    /// Meant for more expensive operations that can be delayed without negatively
    /// affecting the UI.
    AfterBlockCompleted(AfterBlockCompletedEvent),
    /// Send on DProtoHook::Preexec, but only for blocks after bootstrapping
    AfterBlockStarted {
        block_id: BlockId,
        command: String,
        is_for_in_band_command: bool,
    },
    /// Sent when a new block is created.
    BlockMetadataReceived(BlockMetadataReceivedEvent),
    /// Sent after a background block is started and added to the block list.
    BackgroundBlockStarted,
    ClipboardStore(ClipboardType, String),
    ClipboardLoad(
        ClipboardType,
        Arc<dyn Fn(&str) -> String + Sync + Send + 'static>,
    ),
    CursorBlinkingChange(bool),
    TerminalClear,
    Bell,
    Exit {
        reason: ExitReason,
    },
    /// An indication that we are about to initiate an interactive SSH session
    /// (which may or may not use the SSH wrapper).
    PreInteractiveSSHSession,
    /// An indication that a successful SSH connection was initiated via the
    /// SSH wrapper.  The argument is the name of the remote shell.
    SSH(String),
    /// Sent when the model detects an SSH ControlMaster error, which means that
    /// completions reliant on command execution will not work.
    SSHControlMasterError,
    TerminalModeSwapped(TerminalMode),
    ExecutedInBandCommand(ExecutedExecutorCommandEvent),
    TmuxControlModeReady {
        primary_pane: u32,
    },
    /// Sent when a line of output from an interactive ssh session indicates login is complete.
    /// A line such as "Last login: Wed Oct 30" for example indicates login is complete. This is
    /// useful for detecting when an ssh session becomes ready for warpification.
    DetectedEndOfSshLogin(SshLoginStatus),
    RemoteWarpificationIsUnavailable(WarpificationUnavailableReason),
    SshTmuxInstaller(TmuxInstallationState),
    TmuxInstallFailed {
        line: String,
        command: String,
    },
    InitSubshell(InitSubshellEvent),
    /// Emitted when the user's RC file has been executed in a subshell.
    SourcedRcFileInSubshell(SourcedRcFileInSubshellEvent),
    InitSsh(InitSshEvent),
    /// Emitted when the active block's prompt has been updated.
    PromptUpdated,
    /// Emitted when the honor_ps1 state of the shell is out-of-sync with Warp's settings.
    /// This can happen in cases such as when the user changes between PS1 and Warp prompt inside
    /// of an SSH session (the bindkeys are sent to the SSH session but not the local session, so
    /// they are out-of-sync when the user exits SSH).
    HonorPS1OutOfSync,
    /// Emitted when the terminal model receives typeahead output from the PTY.
    /// "Typeahead" are characters that were written to the PTY during long-running command execution
    /// close to the end of the its execution, such that these characters were not actually read by
    /// the running program. The shell stores these characters, inserts them into its internal line
    /// buffer, and re-echoes them after Precmd.
    Typeahead,
    /// Events that correspond to a specific ansi handler hook while parsing PTY output.
    ///
    /// These events make it possible for other models/views to subscribe to PTY output events that
    /// are otherwise solely handled on the event loop thread by `TerminalModel`; since PTY output
    /// handling logic is mostly executed on that event loop thread, they would otherwise be
    /// inaccessible to views/models.
    Handler(AnsiHandlerEvent),
    FinishUpdate(FinishUpdateValue),
    SelectedTextChanged,
    ShellSpawned(ShellType),
    CompletionsFinished(Vec<ShellCompletion>),
    SendCompletionsPrompt,
    ImageReceived {
        image_id: u32,
        image_data: Vec<u8>,
        image_protocol: ImageProtocol,
    },
    BootstrapPrecmdDone,
    AgentTaggedInChanged {
        is_tagged_in: bool,
    },
    /// A pluggable notification triggered via OSC 9 or OSC 777 escape sequences.
    PluggableNotification {
        title: Option<String>,
        body: String,
    },
    /// Emitted when an SSH session's `InitShell` is intercepted by the
    /// `SshRemoteServer` feature flag. `RemoteServerController` subscribes to
    /// this instead of `Handler(InitShell)` so `PtyController` never sees it.
    SshInitShell {
        pending_session_info: Box<SessionInfo>,
    },
    /// Emitted by `ModelEventDispatcher::request_remote_server_block`
    /// when the remote-server binary is missing and the user must choose.
    RemoteServerBlockRequested {
        session_id: SessionId,
    },
    /// Emitted right before the remote shell for a session exits. Used to
    /// tear down per-session resources (e.g. the remote-server-proxy ssh
    /// child) before the outer ssh tunnel starts closing.
    ExitShell {
        session_id: SessionId,
    },
}

#[derive(Clone, Debug)]
pub enum AnsiHandlerEvent {
    InitShell {
        pending_session_info: Box<SessionInfo>,
    },
    Bootstrapped {
        session_id: SessionId,
        is_subshell: bool,
    },
    Precmd,
    Preexec,
    UserCommandFinished,
    InBandCommandFinished,
    StartPrompt,
    StartRPrompt,
    EndPrompt,
    EndRPrompt,
    SetBracketedPaste,
    UnsetBracketedPaste,
    StartTmuxControlMode,
    TmuxControlModeReady {
        primary_pane: u32,
    },
    EndTmuxControlMode,
    RunTmuxCommand(TmuxCommand),
}

impl Entity for ModelEventDispatcher {
    type Event = ModelEvent;
}
