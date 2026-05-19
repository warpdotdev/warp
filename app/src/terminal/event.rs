use std::fmt;
use std::fmt::{Debug, Formatter};
use std::num::ParseIntError;
use std::string::FromUtf8Error;
use std::sync::Arc;
use std::time::Duration;

use instant::Instant;

use crate::server::ids::SyncId;
use crate::server::telemetry::ImageProtocol;
use crate::terminal::model::block::BlockMetadata;
use crate::terminal::model::block::SerializedBlock;
use crate::terminal::model::completions::ShellCompletion;
use crate::terminal::model::terminal_model::HandlerEvent;
use crate::terminal::shell::ShellType;
use crate::terminal::ClipboardType;
use crate::util::AsciiDebug;

use super::history::HistoryEntry;
use super::model::ansi::{FinishUpdateValue, WarpificationUnavailableReason};
use super::model::block::BlockId;
use super::model::session::{SessionId, SessionInfo};
use super::model::terminal_model::{BlockIndex, ExitReason, TmuxInstallationState};

pub use remote_server::setup::RemoteServerSetupState;

#[derive(Clone)]
/// Events sent to the main thread by the terminal model & event loop.
pub enum Event {
    CompletionsFinished(Vec<ShellCompletion>),
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
    /// Sent when a block's working directory has been updated outside of the
    /// normal precmd path (e.g. via an OSC 7 escape sequence). Subscribers
    /// that only care about CWD changes should listen for this in addition to
    /// `BlockMetadataReceived`; subscribers tied to precmd semantics (such as
    /// the requested-command finish detector) should keep listening only to
    /// `BlockMetadataReceived` so they preserve their once-per-block contract.
    BlockWorkingDirectoryUpdated(BlockWorkingDirectoryUpdatedEvent),
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
    /// Emitted when the remote shell for a session is about to exit, so
    /// per-session resources (e.g. the `ssh … remote-server-proxy` child that
    /// holds a multiplexed channel on the ControlMaster) can be torn down
    /// before the outer ssh tunnel starts closing.
    ExitShell {
        session_id: SessionId,
    },
    /// Sent when the model detects an SSH ControlMaster error, which means that
    /// completions reliant on command execution will not work.
    SSHControlMasterError,
    TerminalModeSwapped(TerminalMode),
    ExecutedInBandCommand(ExecutedExecutorCommandEvent),
    TmuxControlModeReady {
        primary_pane: u32,
    },
    /// See comment above [crate::terminal::ModelEvent::DetectedEndOfSshLogin].
    DetectedEndOfSshLogin(SshLoginStatus),
    RemoteWarpificationIsUnavailable(WarpificationUnavailableReason),
    SshTmuxInstaller(TmuxInstallationState),
    TmuxInstallFailed {
        line: String,
        command: String,
    },
    InitSsh(InitSshEvent),
    InitSubshell(InitSubshellEvent),
    /// Emitted when the user's RC file has been executed in a subshell.
    SourcedRcFileInSubshell(SourcedRcFileInSubshellEvent),
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
    /// Emitted when the agent is tagged in or out of the active block.
    /// Users "Tag an agent in" when they ask the agent to take over a long running command
    /// that was started outside of a conversation (and they tag the agent out when they take control back).
    AgentTaggedInChanged {
        is_tagged_in: bool,
    },
    Handler(HandlerEvent),
    /// Emitted when the remote server binary has been successfully checked or
    /// installed and is ready. The session is initialized independently on
    /// `Bootstrapped`; when the remote server later connects, the client is
    /// attached to the existing session's `RemoteServerCommandExecutor` via
    /// the `RemoteServerManagerEvent::SessionConnected` subscription in
    /// `Sessions::new`.
    RemoteServerReady {
        session_id: SessionId,
    },
    /// Emitted when the remote server setup failed. The session falls back to
    /// the ControlMaster-based `RemoteCommandExecutor`.
    RemoteServerFailed {
        session_id: SessionId,
        error: String,
    },
    /// Emitted when the assisted auto-update has completed and we're ready to
    /// relaunch the app.
    FinishUpdate(FinishUpdateValue),
    TextSelectionChanged,
    ShellSpawned(ShellType),
    SendCompletionsPrompt,
    ImageReceived {
        image_id: u32,
        image_data: Vec<u8>,
        image_protocol: ImageProtocol,
    },
    BootstrapPrecmdDone,
    /// A pluggable notification triggered via OSC 9 or OSC 777 escape sequences.
    /// External programs can use this to trigger notifications in Warp.
    ///
    /// References:
    /// - OSC 9: <https://conemu.github.io/en/AnsiEscapeCodes.html#OSC_Operating_system_commands>
    /// - OSC 777: <https://codeberg.org/dnkl/foot/wiki/Notify>
    PluggableNotification {
        title: Option<String>,
        body: String,
    },
}

#[derive(Debug, Clone)]
pub struct InitSubshellEvent {
    pub shell_type: ShellType,
    pub uname: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SourcedRcFileInSubshellEvent {
    pub shell_type: ShellType,
    pub uname: Option<String>,
    pub tmux: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct InitSshEvent {
    pub shell_type: ShellType,
    pub uname: Option<String>,
}

#[derive(Clone)]
pub enum TerminalMode {
    AltScreen,
    BlockList,
}

#[derive(Clone, Debug)]
pub enum SshLoginStatus {
    /// We have some evidence login is complete but should check again.
    RecheckBeforeWarpifying,
    /// We have high confidence login is complete.
    ReadyToWarpify,
}

#[derive(Clone, Debug)]
pub struct InitShellEvent {
    pub pending_session_info: SessionInfo,
}

#[derive(Clone, Debug)]
pub struct BootstrappedEvent {
    /// The command which spawned the shell.
    pub spawning_command: String,
    // This is wrapped in an `Box` to surpress clippy's large-enum-variant warning, not because it
    // functionally needs to be wrapped in an `Box`.
    pub session_info: Box<SessionInfo>,
    pub restored_block_commands: Vec<HistoryEntry>,
    /// The time we spent sourcing the user's rcfiles, in seconds.  This may be
    /// None if the information was not provided by the shell.
    pub rcfiles_duration_seconds: Option<f64>,
}

#[derive(Clone)]
pub struct BlockCompletedEvent {
    /// This will be None when we don't want to collect telemetry
    /// for this block's latency.
    pub block_latency_data: Option<BlockLatencyData>,
    pub block_type: BlockType,
    pub num_secrets_obfuscated: usize,
    pub block_index: BlockIndex,
    pub block_id: BlockId,
    pub session_id: Option<SessionId>,
    pub restored_block_was_local: Option<bool>,
}

#[derive(Clone)]
pub struct AfterBlockCompletedEvent {
    /// The delay from the CommandFinished ansi hook to the Precmd hook.
    /// This value is only provided for the blocks that the user directly
    /// executes (so it's not provided if this is a restored block from session
    /// restoration or a bootstrapping block).
    pub command_finished_to_precmd_delay: Option<Duration>,
    pub block_type: BlockType,
    pub num_secrets_obfuscated: usize,

    /// If the completed block was a workflow, this is its id.
    pub cloud_workflow_id: Option<SyncId>,

    /// If the completed block had an env var object associated.
    pub cloud_env_var_collection_id: Option<SyncId>,
}

#[derive(Clone)]
pub struct BlockLatencyData {
    pub command: &'static str,
    /// When the block's command grid was started (i.e. when the user hit enter).
    pub started_at: Instant,
}

#[derive(Clone, Debug)]
/// Different types of blocks. `User` is for normal execution.
/// Everything else is earlier in the bootstrapping sequence.
pub enum BlockType {
    /// When there are blocks that finish in the bootstrap sequence,
    /// we don't want to propagate the event around our app.
    BootstrapHidden,
    /// This is a special case where the user's rcfiles resulted in outputs. We
    /// will want some of the view logic to execute, and not all of it.
    BootstrapVisible(Arc<SerializedBlock>),
    /// This was a block we restored through session restoration.
    Restored,
    /// This was a block created for execution of an in-band command.
    InBandCommand,
    /// This is a normal block that the user executed.
    User(UserBlockCompleted),

    /// This is a block containing background process output.
    Background(Arc<SerializedBlock>),

    /// This is a block containing static/hardcoded content (e.g. the subshell Warpification
    /// welcome block).
    Static,
}

impl BlockType {
    pub fn is_bootstrap_block(&self) -> bool {
        matches!(self, Self::BootstrapHidden | Self::BootstrapVisible(_))
    }
}

#[derive(Clone, Debug)]
/// A notification that the metadata for the active block & prompt is now
/// available.
pub struct BlockMetadataReceivedEvent {
    pub block_metadata: BlockMetadata,
    pub block_index: BlockIndex,
    /// Whether the previous block was an in-band command.
    pub is_after_in_band_command: bool,
    /// Whether the session has fully completed the bootstrapping process.
    pub is_done_bootstrapping: bool,
}

#[derive(Clone, Debug)]
/// A notification that an existing block's working directory has been updated
/// out-of-band (e.g. by an OSC 7 escape sequence) without a fresh precmd. The
/// payload mirrors `BlockMetadataReceivedEvent` so CWD-dependent listeners can
/// reuse the same handling, but listeners that rely on precmd semantics should
/// keep using `BlockMetadataReceivedEvent`.
///
/// Note: `is_for_in_band_command` here describes the block carrying the update,
/// while the similarly-spelled `is_after_in_band_command` on
/// `BlockMetadataReceivedEvent` describes the *previous* block. The semantics
/// differ because precmd fires after a block runs, while OSC 7 fires while the
/// block is alive.
pub struct BlockWorkingDirectoryUpdatedEvent {
    pub block_metadata: BlockMetadata,
    pub block_index: BlockIndex,
    /// Whether the block carrying this update is for an in-band command.
    pub is_for_in_band_command: bool,
    /// Whether the session has fully completed the bootstrapping process.
    pub is_done_bootstrapping: bool,
}

#[derive(Clone, Debug)]
/// Contents of a normal block that a user executed.
pub struct UserBlockCompleted {
    pub index: BlockIndex,

    pub serialized_block: Arc<SerializedBlock>,

    /// The input lines for a block without any escape sequences.
    pub command: String,

    /// The command with secrets obfuscated.
    pub command_with_obfuscated_secrets: String,

    /// The output lines for a block without any escape sequences.
    /// They are truncated to the number of lines specificed by the caller.
    pub output_truncated: String,

    /// The output lines for a block without any escape sequences.
    /// They are truncated to the number of lines specificed by the caller.
    /// Forced secrets to be obfuscated as well.
    pub output_truncated_with_obfuscated_secrets: String,

    /// `true` if the block was run as a requested command or was part of a CLI subagent interaction.
    pub was_part_of_agent_interaction: bool,

    /// Time that we started the command grid (i.e. immediately after the user
    /// hit enter).
    pub started_at: Option<Instant>,

    /// The number of lines in the output grid when it was finished.
    pub num_output_lines: u64,

    /// The number of lines of output that were truncated while the block
    /// was active and receiving output.
    pub num_output_lines_truncated: u64,
}

/// Emitted upon completion of an executor command that goes through the pty, such as the
/// InBandCommandExecutor or the TmuxCommandExecutor.
#[derive(Clone)]
pub struct ExecutedExecutorCommandEvent {
    pub command_id: String,
    pub exit_code: usize,
    pub output: Vec<u8>,
}

impl ExecutedExecutorCommandEvent {
    /// Parses the given `payload` (expected to be the payload of a generator output OSC) into a
    /// `ExecutedGeneratorCommandValue`.
    ///
    /// The given `string` is expected to follow the following format:
    ///     <commmand_id>;<output>;<exit_code>
    ///
    /// Returns a `ParseGeneratorCommandValueError` if payload cannot be successfully parsed.
    ///
    pub fn parse_generator_payload(payload: Vec<u8>) -> Result<Self, ParseGeneratorOutputError> {
        // Break the payload apart at the first and last semicolons.
        let mut payload_initial_split = payload.splitn(2, |&byte| byte == b';');

        let Some(before_first_semicolon) = payload_initial_split.next() else {
            return Err(ParseGeneratorOutputError::Corrupted);
        };

        let Some(after_first_semicolon) = payload_initial_split.next() else {
            return Err(ParseGeneratorOutputError::Corrupted);
        };

        let mut payload_final_split = after_first_semicolon.rsplitn(2, |&byte| byte == b';');
        let Some(after_final_semicolon) = payload_final_split.next() else {
            return Err(ParseGeneratorOutputError::Corrupted);
        };

        let Some(payload_middle) = payload_final_split.next() else {
            return Err(ParseGeneratorOutputError::Corrupted);
        };

        let command_id = String::from_utf8(before_first_semicolon.to_vec())
            .map_err(ParseGeneratorOutputError::Utf8DecodingFailure)?;

        let exit_code = String::from_utf8(after_final_semicolon.to_vec())
            .map_err(ParseGeneratorOutputError::Utf8DecodingFailure)?
            .parse::<usize>()
            .map_err(ParseGeneratorOutputError::ExitCodeParseFailure)?;

        // The output of the command remains as bytes. This is so we can operate on the bytes higher in
        // the stack if we need to, such as in the case of parsing out the zsh history file where we want to
        // transform the byte array before converting to a string.
        let output = payload_middle.to_vec();

        Ok(Self {
            command_id,
            exit_code,
            output,
        })
    }
}

impl Debug for ExecutedExecutorCommandEvent {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExecutedExecutorCommandEvent")
            .field("command_id", &self.command_id)
            .field("exit_code", &self.exit_code)
            .field("output", &AsciiDebug(&self.output))
            .finish()
    }
}

#[derive(thiserror::Error, Debug)]
pub enum ParseGeneratorOutputError {
    #[error("Failed to parse exit code: {0:?}")]
    ExitCodeParseFailure(ParseIntError),
    #[error("Corrupted DCS. Should be of the format <command_id>;<exit_code>;<output>. ")]
    Corrupted,
    #[error("Failed to convert to Utf8: {0:?}")]
    Utf8DecodingFailure(FromUtf8Error),
}

impl Debug for Event {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Event::CompletionsFinished(_) => write!(f, "CompletionsFinished"),
            Event::MouseCursorDirty => write!(f, "MouseCursorDirty"),
            Event::BlockCompleted(_) => write!(f, "BlockCompleted"),
            Event::AfterBlockCompleted(_) => write!(f, "AfterBlockCompleted"),
            Event::BlockMetadataReceived(event) => write!(
                f,
                "BlockStarted({:?}, Done bootstrapping: {:?})",
                event.block_metadata, event.is_done_bootstrapping
            ),
            Event::BlockWorkingDirectoryUpdated(event) => write!(
                f,
                "BlockWorkingDirectoryUpdated({:?}, Done bootstrapping: {:?})",
                event.block_metadata, event.is_done_bootstrapping
            ),
            Event::AfterBlockStarted { .. } => write!(f, "BlockExecutionStarted"),
            Event::BackgroundBlockStarted => write!(f, "BackgroundBlockStarted"),
            Event::VisibleBootstrapBlock => write!(f, "VisibleBootstrapBlock"),
            Event::Title(title) => write!(f, "Title({title})"),
            Event::ClipboardStore(_, text) => write!(f, "ClipboardStore({text})"),
            Event::ClipboardLoad(_, _) => write!(f, "ClipboardLoad()"),
            Event::TerminalClear => write!(f, "TerminalClear"),
            Event::Bell => write!(f, "Bell"),
            Event::Exit { reason } => write!(f, "Exit({reason:?})"),
            Event::CursorBlinkingChange(blinking) => write!(f, "CursorBlinking({blinking})"),
            Event::PreInteractiveSSHSession => write!(f, "Pre-Interactive SSH Session"),
            Event::SSH(remote_shell) => write!(f, "SSH(remote shell: {remote_shell}"),
            Event::SSHControlMasterError => write!(f, "SSH ControlMaster error"),
            Event::TerminalModeSwapped(_) => write!(f, "Terminal mode swapped"),
            Event::TmuxControlModeReady { primary_pane } => {
                write!(f, "TmuxControlModeReady(primary_pane: {primary_pane})")
            }
            Event::DetectedEndOfSshLogin(check_type) => {
                write!(f, "DetectedEndOfSshLogin: {check_type:?}")
            }
            Event::RemoteWarpificationIsUnavailable(_) => {
                write!(f, "RemoteWarpificationIsUnavailable")
            }
            Event::SshTmuxInstaller(installer) => {
                write!(f, "SshTmuxInstaller({installer:?})")
            }
            Event::TmuxInstallFailed { line, command } => {
                write!(f, "TmuxInstallFailed(line: {line}, command: {command})")
            }
            Event::ExecutedInBandCommand(event) => write!(
                f,
                "Executed in-band command with ID {} and exit code {}",
                event.command_id, event.exit_code
            ),
            Event::InitSubshell(event) => {
                write!(f, "InitSubshell({event:?})")
            }
            Event::SourcedRcFileInSubshell(event) => {
                write!(f, "SourcedRcFileInSubshell({event:?})")
            }
            Event::InitSsh(event) => {
                write!(f, "InitSsh({event:?})")
            }
            Event::PromptUpdated => write!(f, "PromptUpdated"),
            Event::HonorPS1OutOfSync => write!(f, "HonorPS1OutOfSync"),
            Event::Typeahead => write!(f, "Typeahead"),
            Event::AgentTaggedInChanged { is_tagged_in } => {
                write!(f, "AgentTaggedInChanged(is_tagged_in: {is_tagged_in})")
            }
            Event::Handler(handler_event) => write!(f, "Handler({handler_event:?}))"),
            Event::RemoteServerReady { session_id } => {
                write!(f, "RemoteServerReady(session: {session_id:?})")
            }
            Event::RemoteServerFailed {
                session_id,
                ref error,
            } => {
                write!(
                    f,
                    "RemoteServerFailed(session: {session_id:?}, error: {error})"
                )
            }
            Event::FinishUpdate(data) => write!(f, "FinishUpdate({})", data.update_id),
            Event::TextSelectionChanged => write!(f, "TextSelectionChanged"),
            Event::ShellSpawned(shell_type) => write!(f, "ShellSpawned({shell_type:?})"),
            Event::SendCompletionsPrompt => write!(f, "SendCompletionsPrompt"),
            Event::ImageReceived { image_id, .. } => {
                write!(f, "ImageReceived(image_id: {image_id})")
            }
            Event::BootstrapPrecmdDone => write!(f, "BootstrapPrecmdDone"),
            Event::PluggableNotification { title, body } => {
                write!(f, "PluggableNotification(title: {title:?}, body: {body})")
            }
            Event::ExitShell { session_id } => {
                write!(f, "ExitShell(session: {session_id:?})")
            }
        }
    }
}
