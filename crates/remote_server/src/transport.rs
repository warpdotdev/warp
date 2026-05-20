//! Transport abstraction for [`RemoteServerManager`].
//!
//! Separates SSH-specific concerns (ControlMaster sockets, binary install,
//! process spawning) from the transport-agnostic session lifecycle managed
//! by [`RemoteServerManager`]. Alternative transports (Docker exec,
//! in-process for tests) implement the same trait without touching the
//! manager.
//!
//! Returns boxed futures for object safety — the manager stores
//! `Arc<dyn RemoteTransport>` for reconnection.
//!
//! [`RemoteServerManager`]: crate::manager::RemoteServerManager
use std::future::Future;
#[cfg(not(target_family = "wasm"))]
use std::path::PathBuf;
use std::pin::Pin;

use async_channel::Receiver;
use warpui::r#async::executor;

use crate::client::{ClientEvent, RemoteServerClient};
use crate::manager::RemoteServerExitStatus;
use crate::setup::{PreinstallCheckResult, RemotePlatform};
use serde::Serialize;

/// How the remote server binary was installed. Used for telemetry to
/// distinguish direct remote downloads from client-side SCP uploads.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallSource {
    /// The remote host downloaded the binary directly from the CDN.
    Server,
    /// The client downloaded the binary locally and uploaded it via SCP.
    Client,
}

/// Result of [`RemoteTransport::install_binary`], bundling the install
/// result with the source that was attempted. The source is always set
/// once the install path is determined, regardless of whether the
/// install succeeded or failed.
pub struct InstallOutcome {
    /// Which install path was attempted.
    pub source: Option<InstallSource>,
    /// Whether the install succeeded.
    pub result: Result<(), Error>,
}

/// Structured error for user-facing display in the SSH remote-server
/// failed banner. Separates the always-visible body from an optional set of
/// details.
#[derive(Clone, Debug)]
pub struct UserFacingError {
    /// Always-visible explanation of what went wrong,
    /// e.g. "Failed to install SSH extension".
    pub body: String,
    /// Optional technical detail shown to the user (stderr,
    /// timeout duration, unsupported OS/arch). `None` when the
    /// underlying error doesn't carry anything useful for the user.
    pub detail: Option<String>,
}

/// The setup stage that failed, used to generate context-appropriate
/// user-facing messages from a [`Error`].
#[derive(Clone, Copy, Debug)]
pub enum SetupStage {
    DetectPlatform,
    PreinstallCheck,
    CheckBinary,
    InstallBinary,
    Launch,
}

impl SetupStage {
    fn action_description(self) -> &'static str {
        match self {
            Self::DetectPlatform => "detect remote platform",
            Self::PreinstallCheck => "run preinstall check",
            Self::CheckBinary => "verify SSH extension",
            Self::InstallBinary => "install SSH extension",
            Self::Launch => "start SSH extension",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The operation timed out.
    #[error("timed out")]
    TimedOut,
    /// The remote host reported an OS not supported by the prebuilt binary.
    #[error("unsupported OS: {os}")]
    UnsupportedOs { os: String },
    /// The remote host reported a CPU architecture not supported by the prebuilt binary.
    #[error("unsupported architecture: {arch}")]
    UnsupportedArch { arch: String },
    /// A remote script ran but exited with a non-zero code.
    #[error("script failed (exit {exit_code}): {stderr}")]
    ScriptFailed { exit_code: i32, stderr: String },
    /// Any other transport-level or unexpected failure.
    #[error(transparent)]
    Other(anyhow::Error),
}

/// Maximum number of stderr characters to include in the user-facing
/// detail for `ScriptFailed` errors. Keeps the banner reasonable even
/// when a remote script dumps a large amount of output.
const MAX_STDERR_DISPLAY_CHARS: usize = 512;

impl Error {
    /// Converts this error into a [`UserFacingError`] suitable for the
    /// SSH remote-server failed banner, using `stage` to provide
    /// context-appropriate copy.
    pub fn user_facing_error(&self, stage: SetupStage) -> UserFacingError {
        let body = format!("Failed to {}", stage.action_description());
        let detail = match self {
            Self::TimedOut => {
                Some("The operation timed out — check your network connection".into())
            }
            Self::UnsupportedOs { os } => Some(format!("Unsupported OS: {os}")),
            Self::UnsupportedArch { arch } => Some(format!("Unsupported architecture: {arch}")),
            Self::ScriptFailed { exit_code, stderr } => {
                let truncated = if stderr.chars().count() > MAX_STDERR_DISPLAY_CHARS {
                    let end: usize = stderr
                        .char_indices()
                        .nth(MAX_STDERR_DISPLAY_CHARS)
                        .map(|(i, _)| i)
                        .unwrap_or(stderr.len());
                    format!("{}…", &stderr[..end])
                } else {
                    stderr.clone()
                };
                Some(format!("Script exited with code {exit_code}: {truncated}"))
            }
            Self::Other(_) => None,
        };
        UserFacingError { body, detail }
    }
}

/// A successful return from [`RemoteTransport::connect`].
///
/// Bundles the live [`RemoteServerClient`] and its [`ClientEvent`]
/// receiver together with any transport-specific resources whose
/// lifetime must match the session (notably an owning `Child` for
/// subprocess-backed transports). The caller -- typically
/// [`RemoteServerManager`] -- stashes the whole `Connection` on its
/// per-session state so that dropping the state cleans everything up at
/// once.
///
/// [`RemoteServerManager`]: crate::manager::RemoteServerManager
#[cfg_attr(target_family = "wasm", allow(dead_code))]
pub struct Connection {
    pub client: RemoteServerClient,
    pub event_rx: Receiver<ClientEvent>,
    /// Receiver for request-failure telemetry events. Separate from
    /// `event_rx` so the failure sender on the client doesn't keep the
    /// lifecycle event channel alive.
    pub failure_rx: async_channel::Receiver<crate::client::RequestFailedEvent>,
    /// The subprocess whose stdio backs the client (e.g.
    /// `ssh … remote-server-proxy`). Spawned with `kill_on_drop(true)`
    /// by the transport, so dropping this `Child` sends SIGKILL to the
    /// subprocess. The [`RemoteServerManager`] holds it for the
    /// lifetime of the session and drops it on teardown.
    ///
    /// [`RemoteServerManager`]: crate::manager::RemoteServerManager
    #[cfg(not(target_family = "wasm"))]
    pub child: async_process::Child,
    /// For transports that multiplex through a local SSH
    /// `ControlMaster` socket: the path to that socket, used on
    /// explicit teardown (after the user's shell exits) to run
    /// `ssh -O exit` and force the master to terminate without
    /// waiting for half-closed channels. `None` for transports with
    /// no separate master process (in-process tests, etc.).
    ///
    /// See [`crate::ssh::stop_control_master`] for the exact command.
    #[cfg(not(target_family = "wasm"))]
    pub control_path: Option<PathBuf>,
}

/// Transport abstraction for remote server connections.
///
/// Object-safe: returns boxed futures so implementations can be stored
/// as `Arc<dyn RemoteTransport>` for reconnection.
pub trait RemoteTransport: Send + Sync + std::fmt::Debug {
    /// Detects the remote host's OS and architecture by running `uname -sm`.
    ///
    /// Returns the parsed [`RemotePlatform`] on success, or a
    /// [`Error`] if the command fails or the output cannot
    /// be parsed.
    fn detect_platform(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<RemotePlatform, Error>> + Send>>;

    /// Runs the preinstall check script ([`crate::setup::PREINSTALL_CHECK_SCRIPT`])
    /// over the existing connection and parses its structured stdout into
    /// a [`PreinstallCheckResult`].
    ///
    /// This runs **before** any user-visible install affordance (the
    /// install choice block, auto-install, auto-update, or connect) and
    /// is the gate that decides whether to proceed with the install
    /// pipeline or fall back to the legacy SSH flow.
    ///
    /// Returns `Ok(_)` on success (including when the script reported
    /// `Unknown` — that's a parser-level outcome, not a transport-level
    /// failure). Returns `Err(_)` only on transport-level failure (timeout,
    /// broken pipe, non-zero exit with no parseable summary), which the
    /// caller treats as inconclusive (fail open).
    fn run_preinstall_check(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<PreinstallCheckResult, Error>> + Send>>;

    /// Checks whether the remote server binary is present on the remote host.
    ///
    /// Pure I/O — does not emit any events. The caller
    /// ([`RemoteServerManager::check_binary`]) is responsible for emitting
    /// [`SetupStateChanged`] and [`BinaryCheckComplete`].
    ///
    /// Returns `Ok(true)` if the binary is installed and executable,
    /// `Ok(false)` if it is definitively not installed, and
    /// `Err(_)` if the check failed (e.g. timeout or unreachable).
    fn check_binary(&self) -> Pin<Box<dyn Future<Output = Result<bool, Error>> + Send>>;

    /// Checks whether the remote host already has an existing install
    /// of the remote server binary.
    ///
    /// Used by the manager to distinguish a fresh install (no prior
    /// install on disk, user should be prompted) from an update (prior
    /// install present, install should happen automatically).
    ///
    /// Returns `Ok(true)` if a prior install was detected, `Ok(false)`
    /// if not, and `Err(_)` on SSH failure.
    fn check_has_old_binary(&self) -> Pin<Box<dyn Future<Output = anyhow::Result<bool>> + Send>>;

    /// Installs the remote server binary on the remote host.
    ///
    /// Pure I/O — does not emit any events. The caller
    /// ([`RemoteServerManager::install_binary`]) is responsible for emitting
    /// [`SetupStateChanged`] and [`BinaryInstallComplete`].
    ///
    /// Returns an [`InstallOutcome`] containing the install result and
    /// the [`InstallSource`] that was attempted (if known).
    fn install_binary(&self) -> Pin<Box<dyn Future<Output = InstallOutcome> + Send>>;

    /// Establish a new connection to the remote server.
    ///
    /// Called on both the initial connect and every subsequent reconnect
    /// attempt. Returns a [`Connection`] carrying the live client, its
    /// event channel, and any transport-specific resources (e.g. an
    /// owning `Child`) whose lifetime must match the session.
    ///
    /// The implementation is responsible for any transport-specific setup
    /// required before messages can flow (e.g. spawning a process, connecting
    /// a socket). Stderr forwarding to local logging should also happen here.
    fn connect(
        &self,
        executor: std::sync::Arc<executor::Background>,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Connection>> + Send>>;

    /// Remove the remote server binary, forcing a reinstall on the next
    /// [`install_binary`] call.
    ///
    /// Called by the manager after the initialize handshake reports a
    /// version that disagrees with the client's: the file at the expected
    /// path is stale/wrong, so we remove it so the next setup sees a miss
    /// and reinstalls from the CDN instead of looping on the same bad
    /// binary.
    ///
    /// [`install_binary`]: RemoteTransport::install_binary
    fn remove_remote_server_binary(
        &self,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send>>;

    /// Returns `true` if the transport considers a reconnect viable after
    /// a spontaneous disconnect with the given exit status.
    ///
    /// Transports that can determine the underlying connection is
    /// unrecoverable (e.g. SSH detecting a dead ControlMaster via exit
    /// code 255) should return `false`, which tells the manager to skip
    /// the reconnect loop entirely.
    fn is_reconnectable(&self, exit_status: Option<&RemoteServerExitStatus>) -> bool;
}
