use std::collections::{HashMap, HashSet};
use std::future::Future;
#[cfg(not(target_family = "wasm"))]
use std::path::PathBuf;
use std::sync::Arc;
#[cfg(not(target_family = "wasm"))]
use std::time::Duration;

use crate::auth::RemoteServerAuthContext;
#[cfg(not(target_family = "wasm"))]
use crate::client::ClientEvent;
#[cfg(not(target_family = "wasm"))]
use crate::client::InitializeParams;
use crate::client::RemoteServerClient;
use crate::codebase_index_proto::RemoteCodebaseIndexStatus;
use crate::proto::{
    diff_state, get_diff_state_response, DiffMode, DiffState, DiffStateErrorValue,
    DiffStateFileDelta, DiffStateMetadataUpdate, DiffStateSnapshot, FileStatusInfo,
    GetDiffStateResponse, TextEdit,
};
use crate::repo_metadata_proto::proto_load_repo_metadata_directory_response_to_update;
use crate::setup::PreinstallCheckResult;
#[cfg(not(target_family = "wasm"))]
use crate::setup::PreinstallStatus;
#[cfg(not(target_family = "wasm"))]
use crate::setup::RemoteOs;
use crate::setup::RemotePlatform;
use crate::setup::RemoteServerSetupState;
#[cfg(not(target_family = "wasm"))]
use crate::setup::UnsupportedReason;
#[cfg(not(target_family = "wasm"))]
use crate::transport::Connection;
use crate::transport::{Error, InstallSource, RemoteTransport};
use crate::HostId;
use repo_metadata::RepoMetadataUpdate;
use serde::Serialize;
#[cfg(not(target_family = "wasm"))]
use warp_core::channel::ChannelState;
use warp_core::SessionId;
use warp_util::remote_path::{RemoteNavigationResult, RemotePath};
use warp_util::standardized_path::StandardizedPath;
#[cfg(not(target_family = "wasm"))]
use warpui::r#async::FutureExt as _;
use warpui::{Entity, ModelContext, ModelSpawner, SingletonEntity};

/// Maximum number of reconnection attempts after a spontaneous disconnect.
pub const MAX_RECONNECT_ATTEMPTS: u32 = 2;
/// Delay between reconnection attempts.
#[cfg(not(target_family = "wasm"))]
const RECONNECT_DELAY: Duration = Duration::from_secs(2);
/// Brief timeout for awaiting a child process's exit status after a
/// connection failure. Gives the SSH subprocess time to report its exit
/// code and signal status before we give up and report `None`.
#[cfg(not(target_family = "wasm"))]
const EXIT_STATUS_WAIT_TIMEOUT: Duration = Duration::from_millis(200);

/// Parameters that travel together through the reconnection flow.
#[cfg(not(target_family = "wasm"))]
struct ReconnectParams {
    attempt: u32,
    host_id: HostId,
    exit_status: Option<RemoteServerExitStatus>,
    transport: Arc<dyn RemoteTransport>,
    auth_context: Arc<RemoteServerAuthContext>,
    control_path: Option<PathBuf>,
    identity_key: String,
}
#[cfg(not(target_family = "wasm"))]
struct InitializeHandshake {
    host_id: HostId,
    event_rx: async_channel::Receiver<ClientEvent>,
    failure_rx: async_channel::Receiver<crate::client::RequestFailedEvent>,
}

/// Error from [`RemoteServerManager::run_connect_and_handshake`] that
/// preserves which phase failed so callers can report accurate telemetry.
#[cfg(not(target_family = "wasm"))]
#[derive(Debug, thiserror::Error)]
enum ConnectAndHandshakeError {
    /// `transport.connect()` failed, or the session was deregistered
    /// before the connect phase could complete.
    #[error("{0:#}")]
    Connect(anyhow::Error),
    /// `client.initialize()` handshake failed.
    #[error("{0:#}")]
    Initialize(anyhow::Error),
}

#[cfg(not(target_family = "wasm"))]
impl ConnectAndHandshakeError {
    fn phase(&self) -> RemoteServerInitPhase {
        match self {
            Self::Connect(_) => RemoteServerInitPhase::Connect,
            Self::Initialize(_) => RemoteServerInitPhase::Initialize,
        }
    }
}

/// Which phase of the remote server connection flow failed.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteServerInitPhase {
    /// `transport.connect()` failed (SSH/process spawn level).
    Connect,
    /// `client.initialize()` failed (protocol handshake level).
    Initialize,
}

/// The remote server client operation that failed.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteServerOperation {
    NavigateToDirectory,
    LoadRepoMetadataDirectory,
    IndexCodebase,
    ResyncCodebase,
    DropCodebaseIndex,
    OpenBuffer,
    SaveBuffer,
    WriteFile,
    ReadFileContext,
    DeleteFile,
    RunCommand,
    GetFragmentMetadataFromHash,
    GetDiffState,
    DiscardFiles,
    GetBranches,
}

#[derive(Clone, Copy, Debug)]
enum RemoteCodebaseIndexMutation {
    EnsureIndexed,
    Resync,
    Drop,
}

impl RemoteCodebaseIndexMutation {
    fn operation(self) -> RemoteServerOperation {
        match self {
            Self::EnsureIndexed => RemoteServerOperation::IndexCodebase,
            Self::Resync => RemoteServerOperation::ResyncCodebase,
            Self::Drop => RemoteServerOperation::DropCodebaseIndex,
        }
    }

    async fn send(
        self,
        client: Arc<RemoteServerClient>,
        repo_path: String,
        auth_token: String,
    ) -> Result<RemoteCodebaseIndexStatus, crate::client::ClientError> {
        match self {
            Self::EnsureIndexed => client.index_codebase(repo_path, auth_token).await,
            Self::Resync => client.resync_codebase(repo_path, auth_token).await,
            Self::Drop => client.drop_codebase_index(repo_path, auth_token).await,
        }
    }
}

/// Classification of a remote server client error for telemetry.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteServerErrorKind {
    Timeout,
    Disconnected,
    ServerError,
    Other,
}

/// Exit status information captured from the remote server subprocess
/// when the connection drops. Used for diagnostics and telemetry.
#[derive(Clone, Debug, Serialize)]
pub struct RemoteServerExitStatus {
    /// Process exit code, if the process exited normally.
    pub code: Option<i32>,
    /// True if the process was killed by a signal (Unix only).
    pub signal_killed: bool,
}

impl RemoteServerErrorKind {
    /// Classify a [`ClientError`] into a telemetry error kind.
    pub fn from_client_error(error: &crate::client::ClientError) -> Self {
        use crate::client::ClientError;
        match error {
            ClientError::Timeout(_) => Self::Timeout,
            ClientError::Disconnected | ClientError::ResponseChannelClosed => Self::Disconnected,
            ClientError::ServerError { .. } => Self::ServerError,
            ClientError::Protocol(_)
            | ClientError::UnexpectedResponse
            | ClientError::FileOperationFailed(_)
            | ClientError::FragmentMetadataLookup { .. }
            | ClientError::DiscardFailed(_) => Self::Other,
        }
    }
}

/// Returns `true` if the client and server are on compatible versions for
/// the initialize handshake.
///
/// Semantics:
/// - Both sides carry a non-empty release tag (`Some(_)` client, non-empty
///   `server` string): the tags must match exactly. Mismatched releases
///   cause the manager to tear the session down and delete the stale
///   binary so the next reconnect reinstalls.
/// - Client has no version (`None`): always compatible. This covers two
///   dev-loop scenarios:
///   1. `cargo run` + `script/deploy_remote_server` — neither side
///      reports a release tag (server string is empty).
///   2. `cargo run` against a remote that has a release-tagged binary
///      already installed — the client has no tag but the server does.
///      Treating this as compatible avoids tearing down and deleting a
///      perfectly good server binary just because the local dev client
///      doesn't carry a release tag.
/// - Client has a version but server reports empty: incompatible. A
///   release-tagged client should not accept an untagged server — it
///   likely means the binary was deployed via the dev script rather
///   than the release channel.
#[cfg(not(target_family = "wasm"))]
fn version_is_compatible(client: Option<&str>, server: &str) -> bool {
    match (client, server.is_empty()) {
        (Some(c), false) => c == server,
        (None, _) => true,
        (Some(_), true) => false,
    }
}

#[cfg(not(target_family = "wasm"))]
fn client_event_kind(event: &ClientEvent) -> &'static str {
    match event {
        ClientEvent::Disconnected => "disconnected",
        ClientEvent::RepoMetadataSnapshotReceived { .. } => "repo_metadata_snapshot",
        ClientEvent::RepoMetadataUpdated { .. } => "repo_metadata_updated",
        ClientEvent::CodebaseIndexStatusesSnapshotReceived { .. } => {
            "codebase_index_statuses_snapshot"
        }
        ClientEvent::CodebaseIndexStatusUpdated { .. } => "codebase_index_status_updated",
        ClientEvent::BufferUpdated { .. } => "buffer_updated",
        ClientEvent::BufferConflictDetected { .. } => "buffer_conflict_detected",
        ClientEvent::DiffStateSnapshotReceived { .. } => "diff_state_snapshot",
        ClientEvent::DiffStateMetadataUpdateReceived { .. } => "diff_state_metadata_update",
        ClientEvent::DiffStateFileDeltaReceived { .. } => "diff_state_file_delta",
        ClientEvent::MessageDecodingError => "message_decoding_error",
    }
}

fn remote_path_for_status(
    host_id: &HostId,
    status: &RemoteCodebaseIndexStatus,
) -> Option<RemotePath> {
    StandardizedPath::try_new(&status.repo_path)
        .ok()
        .map(|path| RemotePath::new(host_id.clone(), path))
}

#[derive(Clone, Debug)]
pub struct RemoteCodebaseIndexStatusWithPath {
    pub remote_path: RemotePath,
    pub status: RemoteCodebaseIndexStatus,
}

/// Per-session connection state. Encodes which data is available at each
/// lifecycle stage so the compiler prevents invalid combinations.
///
/// For subprocess-backed transports (SSH), the `Initializing` and
/// `Connected` variants also own the transport's `Child`. Dropping or
/// replacing the state sends SIGKILL to the subprocess via
/// `kill_on_drop`, which is the authoritative teardown path -- it fires
/// on both explicit deregistration and spontaneous disconnect, and is
/// unaffected by lingering `Arc<RemoteServerClient>` clones held
/// elsewhere (e.g. the per-session command executor).
///
/// They also optionally carry a `control_path` pointing at the SSH
/// `ControlMaster` socket for this session. On explicit teardown
/// (after the user's shell exits), `deregister_session` uses this to
/// run `ssh -O exit`, forcing the master to terminate without waiting
/// for half-closed multiplexed channels to finish cleanup on the
/// remote side.
#[derive(Debug)]
pub enum RemoteSessionState {
    /// `connect_session` has been called; background task is starting the
    /// server process over SSH.
    Connecting,
    /// Server process spawned, client exists, initialize handshake in progress.
    Initializing {
        client: Arc<RemoteServerClient>,
        /// The transport's owning `Child`. Dropped when the state is
        /// replaced or removed, killing the subprocess via
        /// `kill_on_drop`.
        #[cfg(not(target_family = "wasm"))]
        _child: async_process::Child,
        /// See type-level doc.
        #[cfg(not(target_family = "wasm"))]
        control_path: Option<PathBuf>,
    },
    /// Initialize handshake succeeded. Client is ready for requests.
    Connected {
        client: Arc<RemoteServerClient>,
        host_id: HostId,
        /// Identity key that was active when this session was established.
        /// Used by `rotate_auth_token` to ensure token rotation notifications
        /// are only delivered to sessions that belong to the current user
        /// identity, preventing a stale session for a previous identity from
        /// receiving a different user's bearer token.
        identity_key: String,
        /// The transport's owning `Child`. See `Initializing::_child`.
        #[cfg(not(target_family = "wasm"))]
        _child: async_process::Child,
        /// See type-level doc.
        #[cfg(not(target_family = "wasm"))]
        control_path: Option<PathBuf>,
        /// Transport stored for reconnection after spontaneous disconnect.
        #[cfg(not(target_family = "wasm"))]
        transport: Arc<dyn RemoteTransport>,
    },
    /// A reconnection attempt is in progress after a spontaneous disconnect.
    #[cfg(not(target_family = "wasm"))]
    Reconnecting {
        attempt: u32,
        host_id: HostId,
        control_path: Option<PathBuf>,
    },
    /// The connection failed and the background task is briefly awaiting
    /// the child process's exit status before emitting the failure event.
    /// Preserves the `control_path` so `deregister_session` can still
    /// call `stop_control_master` if the user exits during this window.
    #[cfg(not(target_family = "wasm"))]
    AwaitingExitStatus { control_path: Option<PathBuf> },
    /// Connection dropped (EOF/error from the reader task).
    Disconnected,
}

/// Events emitted by [`RemoteServerManager`].
#[derive(Clone, Debug)]
pub enum RemoteServerManagerEvent {
    // --- Session-scoped events ---
    /// A connection flow has started for this session.
    SessionConnecting { session_id: SessionId },
    /// This session's server is connected and ready. Includes the `HostId`
    /// received from the initialize handshake, for model deduplication.
    SessionConnected {
        session_id: SessionId,
        host_id: HostId,
    },
    /// The remote server launch or handshake failed.
    SessionConnectionFailed {
        session_id: SessionId,
        /// Which phase of the connection flow failed.
        phase: RemoteServerInitPhase,
        /// The error message from the failed phase.
        error: String,
        /// Exit status of the SSH subprocess, if available.
        /// Used by telemetry to distinguish proxy crashes from other failures.
        exit_status: Option<RemoteServerExitStatus>,
        /// `true` when the failure is attributed to a user-initiated
        /// cancellation (session deregistered or transport-level
        /// disconnect) rather than a server-side error. Subscribers
        /// that only care about real failures (e.g. telemetry, UI
        /// banners) should skip when this is `true`.
        is_cancelled: bool,
    },
    /// This session's connection dropped. Carries `host_id` so consumers
    /// don't need to look it up from the already-transitioned state.
    /// This session's underlying connection is no longer usable: the
    /// stream closed (EOF/error), the initialize handshake failed, or the
    /// session was explicitly deregistered while `Connected`. Signals to
    /// subscribers that they should drop any `Arc<RemoteServerClient>` they
    /// hold for this session. Carries `host_id` so consumers don't need to
    /// look it up from the already-transitioned state.
    ///
    /// Note this is about *transport* state, not manager tracking: after
    /// this event fires the session may still be present in the manager
    /// in the `Disconnected` state (e.g. when the stream dropped on its
    /// own). Use `SessionDeregistered` to observe removal from the manager.
    SessionDisconnected {
        session_id: SessionId,
        host_id: HostId,
        /// Exit status of the remote server subprocess, if available.
        /// `None` when the session was explicitly deregistered or when
        /// the exit status could not be determined.
        exit_status: Option<RemoteServerExitStatus>,
        /// `true` when this disconnect follows exhausted reconnection
        /// attempts. `false` for first-time disconnects and explicit
        /// deregistrations. Used by the view layer to distinguish
        /// reconnect-exhausted telemetry from regular disconnections.
        was_reconnect_attempt: bool,
    },
    /// A reconnection attempt succeeded. Downstream owners (e.g.
    /// `RemoteServerCommandExecutor`) should swap their client reference
    /// to the new one carried in `client`.
    SessionReconnected {
        session_id: SessionId,
        host_id: HostId,
        attempt: u32,
        client: Arc<RemoteServerClient>,
    },
    /// The manager is no longer tracking this session -- it has been
    /// removed from the `sessions` map via `deregister_session`. Fires
    /// exactly once per session, and only on explicit teardown (never as
    /// a result of a spontaneous connection drop).
    ///
    /// If the session was `Connected` at the point of deregistration, a
    /// `SessionDisconnected` event is emitted first so transport-level
    /// subscribers can release their client references.
    SessionDeregistered { session_id: SessionId },

    // --- Host-scoped events ---
    /// The first session for this host reached `Connected`. Downstream
    /// features should create per-host models (e.g. `RepoMetadataModel`).
    HostConnected { host_id: HostId },
    /// The last session for this host was disconnected or deregistered.
    /// Downstream features should tear down per-host models.
    HostDisconnected { host_id: HostId },

    // --- Repo metadata events (forwarded from ClientEvent push channel) ---
    /// Response to a `navigate_to_directory` request.
    NavigatedToDirectory {
        session_id: SessionId,
        remote_path: RemotePath,
        is_git: bool,
    },
    /// A full or lazy-loaded repo metadata snapshot was pushed by the server.
    RepoMetadataSnapshot {
        host_id: HostId,
        update: RepoMetadataUpdate,
    },
    /// An incremental repo metadata update was pushed by the server.
    RepoMetadataUpdated {
        host_id: HostId,
        update: RepoMetadataUpdate,
    },
    /// A `LoadRepoMetadataDirectory` response was received from the server.
    RepoMetadataDirectoryLoaded {
        host_id: HostId,
        update: RepoMetadataUpdate,
    },
    /// A full remote codebase-index status snapshot was pushed or requested.
    CodebaseIndexStatusesSnapshot {
        host_id: HostId,
        statuses: Vec<RemoteCodebaseIndexStatusWithPath>,
    },
    /// A single remote codebase-index status update was pushed by the daemon.
    CodebaseIndexStatusUpdated {
        remote_path: RemotePath,
        status: RemoteCodebaseIndexStatus,
    },
    /// A buffer was updated on the remote host (file changed on disk).
    /// The app layer should forward this to `GlobalBufferModel::handle_buffer_updated_push`.
    BufferUpdated {
        host_id: HostId,
        path: String,
        new_server_version: u64,
        expected_client_version: u64,
        edits: Vec<TextEdit>,
    },
    /// The file changed on disk while the client had unsaved edits.
    /// The server did NOT apply the change; the client should show a
    /// conflict resolution banner.
    BufferConflictDetected { host_id: HostId, path: String },

    // --- Diff state events (forwarded from ClientEvent push channel) ---
    /// A full diff state snapshot was pushed by the server (or returned
    /// from the initial `GetDiffState` request).
    DiffStateSnapshotReceived {
        host_id: HostId,
        repo_path: StandardizedPath,
        mode: DiffMode,
        snapshot: DiffStateSnapshot,
    },
    /// A metadata-only diff state update was pushed by the server.
    DiffStateMetadataUpdateReceived {
        host_id: HostId,
        repo_path: StandardizedPath,
        mode: DiffMode,
        update: DiffStateMetadataUpdate,
    },
    /// A single-file diff delta was pushed by the server.
    DiffStateFileDeltaReceived {
        host_id: HostId,
        repo_path: StandardizedPath,
        mode: DiffMode,
        delta: DiffStateFileDelta,
    },

    // --- Branch listing ---
    /// Response to a `GetBranches` request.
    GetBranchesResponse {
        session_id: SessionId,
        repo_path: StandardizedPath,
        /// Branch list on success, error message on failure.
        result: Result<Vec<crate::proto::BranchInfo>, String>,
    },

    // --- Setup events ---
    /// Intermediate state change during the binary check/install flow.
    SetupStateChanged {
        session_id: SessionId,
        state: RemoteServerSetupState,
    },
    /// Result of [`RemoteServerManager::check_binary`]. Returns a result where:
    /// - `Ok(true)` means the binary is installed and executable,
    /// - `Ok(false)` means it is not installed, or the preinstall gate
    ///   classified the host as unsupported, and
    /// - `Err(_)` means the check itself failed (e.g. SSH error or timeout).
    BinaryCheckComplete {
        session_id: SessionId,
        result: Result<bool, Arc<Error>>,
        /// The detected remote platform (OS + arch) from `uname -sm`.
        /// `None` if detection failed or was not attempted.
        remote_platform: Option<RemotePlatform>,
        /// Outcome of the preinstall check script. Populated when the
        /// script ran successfully against a Linux host. `None` when the
        /// host is not Linux (the script is skipped) or when the SSH-level
        /// invocation failed (the controller treats that as inconclusive
        /// and falls open).
        preinstall_check: Option<PreinstallCheckResult>,
        /// `true` if the remote already has an existing install of the
        /// remote-server binary, detected by probing whether the install
        /// directory exists (see `RemoteTransport::check_has_old_binary`).
        /// Combined with `result == Ok(false)`, this tells the controller
        /// it should auto-install as an update instead of prompting the
        /// user. `false` when no prior install was detected, or when the
        /// detection itself failed.
        has_old_binary: bool,
    },
    /// Result of [`RemoteServerManager::install_binary`].
    BinaryInstallComplete {
        session_id: SessionId,
        /// Whether the install succeeded or failed.
        result: Result<(), Arc<Error>>,
        /// Which install path was attempted (`Server` for remote download,
        /// `Client` for SCP upload). `None` if the path could not be
        /// determined before the failure.
        install_source: Option<InstallSource>,
    },

    // --- Telemetry events ---
    /// A client request to the remote server failed.
    ClientRequestFailed {
        session_id: SessionId,
        operation: RemoteServerOperation,
        error_kind: RemoteServerErrorKind,
    },
    /// A server message could not be decoded (no parseable request_id).
    ServerMessageDecodingError { session_id: SessionId },
}

impl RemoteServerManagerEvent {
    /// Returns the [`SessionId`] this event pertains to, or `None` for
    /// host-scoped variants.
    pub fn session_id(&self) -> Option<SessionId> {
        match self {
            RemoteServerManagerEvent::SessionConnecting { session_id }
            | RemoteServerManagerEvent::SessionConnected { session_id, .. }
            | RemoteServerManagerEvent::SessionConnectionFailed { session_id, .. }
            | RemoteServerManagerEvent::SessionDisconnected { session_id, .. }
            | RemoteServerManagerEvent::SessionReconnected { session_id, .. }
            | RemoteServerManagerEvent::SessionDeregistered { session_id }
            | RemoteServerManagerEvent::NavigatedToDirectory { session_id, .. }
            | RemoteServerManagerEvent::SetupStateChanged { session_id, .. }
            | RemoteServerManagerEvent::BinaryCheckComplete { session_id, .. }
            | RemoteServerManagerEvent::BinaryInstallComplete { session_id, .. }
            | RemoteServerManagerEvent::ClientRequestFailed { session_id, .. }
            | RemoteServerManagerEvent::ServerMessageDecodingError { session_id }
            | RemoteServerManagerEvent::GetBranchesResponse { session_id, .. } => Some(*session_id),
            RemoteServerManagerEvent::HostConnected { .. }
            | RemoteServerManagerEvent::HostDisconnected { .. }
            | RemoteServerManagerEvent::RepoMetadataSnapshot { .. }
            | RemoteServerManagerEvent::RepoMetadataUpdated { .. }
            | RemoteServerManagerEvent::RepoMetadataDirectoryLoaded { .. }
            | RemoteServerManagerEvent::CodebaseIndexStatusesSnapshot { .. }
            | RemoteServerManagerEvent::CodebaseIndexStatusUpdated { .. }
            | RemoteServerManagerEvent::BufferUpdated { .. }
            | RemoteServerManagerEvent::BufferConflictDetected { .. }
            | RemoteServerManagerEvent::DiffStateSnapshotReceived { .. }
            | RemoteServerManagerEvent::DiffStateMetadataUpdateReceived { .. }
            | RemoteServerManagerEvent::DiffStateFileDeltaReceived { .. } => None,
        }
    }
}

/// Cached navigation state per session. Stores the last requested path
/// (for dedup) and the result from the last successful response (so dedup
/// returns a meaningful value instead of `None`).
struct NavigationCache {
    /// The path string last sent to `navigate_to_directory`.
    path: String,
    /// Populated by the spawner callback when the server responds
    /// successfully. `None` until the first successful response.
    result: Option<RemoteNavigationResult>,
}

/// Shell info recorded by [`RemoteServerManager::notify_session_bootstrapped`].
///
/// Persists for the lifetime of the session (removed only in
/// `deregister_session`) so that `mark_session_connected` can re-send
/// the notification after a reconnect.
#[cfg_attr(target_family = "wasm", allow(dead_code))]
struct SessionBootstrapInfo {
    shell_type: String,
    shell_path: Option<String>,
}

/// Singleton model that manages connections to `remote_server` processes on
/// remote hosts.
///
/// Each SSH session gets its own `RemoteServerClient` and SSH connection.
/// Deduplication of the underlying long-lived server process happens on the
/// remote host. The `HostId` returned by the server's `InitializeResponse`
/// is used on the client to deduplicate host-scoped models (e.g.
/// `RepoMetadataModel`), not connections.
pub struct RemoteServerManager {
    /// Per-session connection state. Each SSH session gets its own dedicated
    /// connection to the remote server.
    sessions: HashMap<SessionId, RemoteSessionState>,
    /// Reverse index: host → sessions for O(1) lookup by `HostId`.
    host_to_sessions: HashMap<HostId, HashSet<SessionId>>,
    /// User-facing connection labels by session, applied after the initialize
    /// handshake returns a host ID.
    session_labels: HashMap<SessionId, String>,
    /// Spawner for running closures back on the main thread.
    spawner: ModelSpawner<Self>,
    /// Per-session navigation cache for dedup. Avoids redundant
    /// `navigate_to_directory` calls when `update_active_session` fires
    /// repeatedly for the same CWD, and returns the cached result on
    /// dedup so callers don't misinterpret the skip as "not a git repo".
    last_navigation: HashMap<SessionId, NavigationCache>,
    /// Per-session shell info recorded at bootstrap time and re-sent to the
    /// remote server daemon on every (re)connect. Persists until
    /// `deregister_session`.
    session_bootstrap_info: HashMap<SessionId, SessionBootstrapInfo>,
    /// App auth context used for connection-time `Initialize` and future
    /// reconnect handshakes.
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    auth_context: Option<Arc<RemoteServerAuthContext>>,
    /// Detected remote platform per session, populated during the binary check
    /// phase via `detect_platform()`. Used for telemetry.
    session_platforms: HashMap<SessionId, RemotePlatform>,
}

impl Entity for RemoteServerManager {
    type Event = RemoteServerManagerEvent;
}

impl SingletonEntity for RemoteServerManager {}

impl RemoteServerManager {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        Self {
            sessions: HashMap::new(),
            host_to_sessions: HashMap::new(),
            session_labels: HashMap::new(),
            spawner: ctx.spawner(),
            last_navigation: HashMap::new(),
            session_bootstrap_info: HashMap::new(),
            auth_context: None,
            session_platforms: HashMap::new(),
        }
    }

    /// Returns a connected client for the given host by picking an arbitrary
    /// session from the host's session pool.
    pub fn client_for_host(&self, host_id: &HostId) -> Option<&Arc<RemoteServerClient>> {
        self.any_connected_session_for_host(host_id)
            .map(|(_, client)| client)
    }

    /// Returns the [`SessionId`] of an arbitrary currently-connected session
    /// for the given host, if any.
    pub fn find_connected_session(&self, host_id: &HostId) -> Option<SessionId> {
        self.any_connected_session_for_host(host_id)
            .map(|(session_id, _)| session_id)
    }

    /// Returns an arbitrary connected `(session_id, client)` pair for the
    /// given host. Backs both [`Self::client_for_host`] and
    /// [`Self::find_connected_session`] so they share a single source of
    /// truth for iteration order and connection-state filtering.
    fn any_connected_session_for_host(
        &self,
        host_id: &HostId,
    ) -> Option<(SessionId, &Arc<RemoteServerClient>)> {
        let sessions = self.host_to_sessions.get(host_id)?;
        sessions
            .iter()
            .copied()
            .find_map(|sid| self.client_for_session(sid).map(|client| (sid, client)))
    }

    /// Returns the user-facing connection label for a connected host, if one
    /// has been recorded on any active session for that host.
    pub fn host_label(&self, host_id: &HostId) -> Option<&str> {
        self.host_to_sessions
            .get(host_id)?
            .iter()
            .find_map(|session_id| self.session_labels.get(session_id).map(String::as_str))
    }

    /// Checks if the remote server binary is installed and executable.
    /// Emits `BinaryCheckComplete { result }`.
    ///
    /// Returns Ok(true) if the binary is installed and executable,
    /// Ok(false) if it is definitively not installed or unsupported setup
    /// should skip install decisions, and
    /// Err(_) if the check failed (e.g. SSH timeout/unreachable).
    #[cfg_attr(target_family = "wasm", allow(unused_variables))]
    pub fn check_binary<T>(
        &mut self,
        session_id: SessionId,
        transport: T,
        ctx: &mut ModelContext<Self>,
    ) where
        T: RemoteTransport + 'static,
    {
        #[cfg(target_family = "wasm")]
        {
            log::warn!("Remote server check_binary is a no-op on WASM");
        }

        #[cfg(not(target_family = "wasm"))]
        {
            ctx.emit(RemoteServerManagerEvent::SetupStateChanged {
                session_id,
                state: RemoteServerSetupState::Checking,
            });
            let spawner = self.spawner.clone();
            ctx.background_executor()
                .spawn(async move {
                    // Run platform detection and the preinstall gate before
                    // any binary, update, prompt, or install decision. The
                    // later binary and old-binary checks run sequentially on
                    // supported hosts so each step reuses the same SSH
                    // ControlMaster connection instead of opening parallel
                    // channels.
                    let platform_result = transport.detect_platform().await;
                    let platform = match platform_result {
                        Ok(p) => Some(p),
                        Err(e) => {
                            if let Some(reason) = UnsupportedReason::from_transport_error(&e) {
                                log::info!(
                                    "Remote server platform is unsupported, falling back to legacy SSH: session={session_id:?}"
                                );
                                Self::emit_unsupported_preinstall_check(
                                    &spawner,
                                    session_id,
                                    None,
                                    PreinstallCheckResult::unsupported(reason),
                                )
                                .await;
                                return;
                            }
                            log::warn!(
                                "Remote server platform detection failed: session={session_id:?} error={e}"
                            );
                            None
                        }
                    };
                    // Run the preinstall check after platform detection
                    // resolves, only on Linux. macOS hosts pay zero extra
                    // round-trips. SSH-level failures are logged and
                    // surfaced as `None`, which the controller treats as
                    // inconclusive (fail open).
                    let preinstall = match &platform {
                        Some(p) if matches!(p.os, RemoteOs::Linux) => {
                            match transport.run_preinstall_check().await {
                                Ok(r) => Some(r),
                                Err(e) => {
                                    log::warn!(
                                        "Remote server preinstall check failed: session={session_id:?} error={e}"
                                    );
                                    None
                                }
                            }
                        }
                        _ => None,
                    };
                    match preinstall {
                        Some(
                            preinstall @ PreinstallCheckResult {
                                status: PreinstallStatus::Unsupported { .. },
                                ..
                            },
                        ) => {
                            log::info!(
                                "Remote server preinstall check classified as unsupported, falling back to legacy SSH: session={session_id:?}"
                            );
                            Self::emit_unsupported_preinstall_check(
                                &spawner, session_id, platform, preinstall,
                            )
                            .await;
                        }
                        preinstall => {
                            Self::check_if_binary_is_installed(
                                &spawner, session_id, transport, platform, preinstall,
                            )
                            .await;
                        }
                    }
                })
                .detach();
        }
    }

    /// Checks whether the remote server binary is already installed on a host
    /// that has passed the support gate. Callers must only invoke this after
    /// platform detection and the preinstall check have ruled out unsupported
    /// OS, architecture, and libc cases.
    #[cfg(not(target_family = "wasm"))]
    async fn check_if_binary_is_installed<T>(
        spawner: &ModelSpawner<Self>,
        session_id: SessionId,
        transport: T,
        platform: Option<RemotePlatform>,
        preinstall: Option<PreinstallCheckResult>,
    ) where
        T: RemoteTransport,
    {
        let check_result = transport.check_binary().await;
        let old_binary_result = transport.check_has_old_binary().await;
        let has_old_binary = match old_binary_result {
            Ok(has) => has,
            Err(e) => {
                log::warn!(
                    "Remote server old-binary detection failed, treating as fresh install: session={session_id:?} error={e}"
                );
                false
            }
        };
        let _ = spawner
            .spawn(move |me, ctx| {
                if let Some(p) = &platform {
                    me.session_platforms.insert(session_id, p.clone());
                }
                if let Err(error) = &check_result {
                    ctx.emit(RemoteServerManagerEvent::SetupStateChanged {
                        session_id,
                        state: RemoteServerSetupState::from(error),
                    });
                }
                ctx.emit(RemoteServerManagerEvent::BinaryCheckComplete {
                    session_id,
                    result: check_result.map_err(Arc::new),
                    remote_platform: platform,
                    preinstall_check: preinstall,
                    has_old_binary,
                });
            })
            .await;
    }
    #[cfg(not(target_family = "wasm"))]
    async fn emit_unsupported_preinstall_check(
        spawner: &ModelSpawner<Self>,
        session_id: SessionId,
        platform: Option<RemotePlatform>,
        preinstall: PreinstallCheckResult,
    ) {
        let PreinstallStatus::Unsupported { reason } = &preinstall.status else {
            return;
        };
        let reason = reason.clone();
        let _ = spawner
            .spawn(move |me, ctx| {
                if let Some(p) = &platform {
                    me.session_platforms.insert(session_id, p.clone());
                }
                ctx.emit(RemoteServerManagerEvent::SetupStateChanged {
                    session_id,
                    state: RemoteServerSetupState::Unsupported {
                        reason: reason.clone(),
                    },
                });
                ctx.emit(RemoteServerManagerEvent::BinaryCheckComplete {
                    session_id,
                    result: Ok(false),
                    remote_platform: platform,
                    preinstall_check: Some(preinstall),
                    has_old_binary: false,
                });
            })
            .await;
    }

    /// Installs the remote server binary.
    /// Emits `BinaryInstallComplete { result }`.
    ///
    /// Returns Ok(method) with the install method on success, and
    /// Err(_) if the install failed (e.g. SSH timeout/unreachable).
    #[cfg_attr(target_family = "wasm", allow(unused_variables))]
    pub fn install_binary<T>(
        &mut self,
        session_id: SessionId,
        transport: T,
        is_update: bool,
        ctx: &mut ModelContext<Self>,
    ) where
        T: RemoteTransport + 'static,
    {
        #[cfg(target_family = "wasm")]
        {
            log::warn!("Remote server install_binary is a no-op on WASM");
        }

        #[cfg(not(target_family = "wasm"))]
        {
            let setup_state = if is_update {
                RemoteServerSetupState::Updating
            } else {
                RemoteServerSetupState::Installing {
                    progress_percent: None,
                }
            };
            ctx.emit(RemoteServerManagerEvent::SetupStateChanged {
                session_id,
                state: setup_state,
            });
            let spawner = self.spawner.clone();
            ctx.background_executor()
                .spawn(async move {
                    let outcome = transport.install_binary().await;
                    let _ = spawner
                        .spawn(move |_me, ctx| {
                            if let Err(error) = &outcome.result {
                                ctx.emit(RemoteServerManagerEvent::SetupStateChanged {
                                    session_id,
                                    state: RemoteServerSetupState::from(error),
                                });
                            }
                            ctx.emit(RemoteServerManagerEvent::BinaryInstallComplete {
                                session_id,
                                result: outcome.result.map_err(Arc::new),
                                install_source: outcome.source,
                            });
                        })
                        .await;
                })
                .detach();
        }
    }

    /// Entry point for establishing a remote server connection for a session.
    /// This assumes the binary is already installed and executable.
    /// Callers should first call `check_binary` and `install_binary` to ensure the binary is present.
    ///
    /// The full flow is:
    /// 1. **Connect** — `transport.connect()` establishes the I/O streams and
    ///    creates the `RemoteServerClient`.
    /// 2. **Handshake** — perform the initialize handshake (which returns the
    ///    `HostId`) and transition to `Connected`.
    ///
    /// No-op on WASM (remote server connections use a different transport).
    #[cfg_attr(target_family = "wasm", allow(unused_variables, unused_mut))]
    pub fn connect_session<T>(
        &mut self,
        session_id: SessionId,
        transport: T,
        auth_context: Arc<RemoteServerAuthContext>,
        connection_label: Option<String>,
        ctx: &mut ModelContext<Self>,
    ) where
        T: RemoteTransport + 'static,
    {
        #[cfg(target_family = "wasm")]
        {
            log::warn!("Remote server connect_session is a no-op on WASM");
        }

        #[cfg(not(target_family = "wasm"))]
        {
            log::info!("Starting remote server connection: session={session_id:?}");

            // Advance the user-visible setup pipeline.
            ctx.emit(RemoteServerManagerEvent::SetupStateChanged {
                session_id,
                state: RemoteServerSetupState::Initializing,
            });

            self.sessions
                .insert(session_id, RemoteSessionState::Connecting);
            if let Some(connection_label) = connection_label {
                self.session_labels.insert(session_id, connection_label);
            }
            self.auth_context = Some(Arc::clone(&auth_context));
            ctx.emit(RemoteServerManagerEvent::SessionConnecting { session_id });

            let spawner = self.spawner.clone();
            let executor = ctx.background_executor().clone();
            // Wrap the transport in an Arc so it can be stored on `Connected`
            // for reconnection after a spontaneous disconnect.
            let transport: Arc<dyn RemoteTransport> = Arc::new(transport);
            let auth_context_for_task = Arc::clone(&auth_context);
            // Capture the identity key synchronously so it travels with the
            // session and can be used to filter token-rotation notifications.
            let identity_key = auth_context.remote_server_identity_key();

            ctx.background_executor()
                .spawn(async move {
                    match Self::run_connect_and_handshake(
                        session_id,
                        &*transport,
                        &auth_context_for_task,
                        &spawner,
                        &executor,
                    )
                    .await
                    {
                        Ok(handshake) => {
                            let _ = spawner
                                .spawn(move |me, ctx| {
                                    me.mark_session_connected(
                                        session_id,
                                        handshake,
                                        identity_key,
                                        transport,
                                        ctx,
                                    );
                                })
                                .await;
                        }
                        Err(e) => {
                            log::warn!(
                                "Remote server connection failed: session={session_id:?} error={e}"
                            );
                            let phase = e.phase();
                            let error = format!("{e}");

                            // Extract the child process from the Initializing
                            // session state so we can asynchronously await its
                            // exit status on this background thread. Transition
                            // to Disconnected while we wait so the session slot
                            // is not empty (an empty slot would be misread as
                            // "user deregistered" by the is_cancelled check).
                            let maybe_child = spawner
                                .spawn(move |me, _ctx| {
                                    match me.sessions.remove(&session_id) {
                                        Some(RemoteSessionState::Initializing {
                                            _child,
                                            control_path,
                                            ..
                                        }) => {
                                            me.sessions.insert(
                                                session_id,
                                                RemoteSessionState::AwaitingExitStatus {
                                                    control_path,
                                                },
                                            );
                                            Some(_child)
                                        }
                                        other => {
                                            // Put back whatever was there
                                            // (including None for deregistered).
                                            if let Some(state) = other {
                                                me.sessions.insert(session_id, state);
                                            }
                                            None
                                        }
                                    }
                                })
                                .await
                                .ok()
                                .flatten();

                            // Await the subprocess exit status with a short
                            // timeout. This gives the SSH process time to
                            // terminate and report its exit code / signal,
                            // which is critical for ResponseChannelClosed
                            // errors where the non-blocking try_status()
                            // previously returned None due to a timing race.
                            let exit_status = match maybe_child {
                                Some(child) => Self::await_exit_status(child, session_id).await,
                                None => None,
                            };

                            let _ = spawner
                                .spawn(move |me, ctx| {
                                    // Classify: user cancellation vs real failure.
                                    //
                                    // Signal A: session was already deregistered
                                    // (user exited before the error handler ran,
                                    // or deregistered during the exit status wait).
                                    //
                                    // Signal B: the transport reports the
                                    // connection is unrecoverable (e.g. SSH
                                    // exit 255 = ControlMaster death) and the
                                    // process was not signal-killed (OOM etc.
                                    // are real failures).
                                    let is_cancelled = !me.sessions.contains_key(&session_id)
                                        || exit_status.as_ref().is_some_and(|s| {
                                            !transport.is_reconnectable(Some(s)) && !s.signal_killed
                                        });

                                    ctx.emit(RemoteServerManagerEvent::SetupStateChanged {
                                        session_id,
                                        state: RemoteServerSetupState::Failed {
                                            error: error.clone(),
                                        },
                                    });
                                    ctx.emit(RemoteServerManagerEvent::SessionConnectionFailed {
                                        session_id,
                                        phase,
                                        error,
                                        exit_status,
                                        is_cancelled,
                                    });
                                    me.mark_session_disconnected(session_id, ctx);
                                })
                                .await;
                        }
                    }
                })
                .detach();
        }
    }

    /// Shared connect + handshake logic used by both `connect_session` and
    /// `attempt_reconnect`.
    ///
    /// 1. Calls `transport.connect()` to establish streams.
    /// 2. Transitions the session to `Initializing` while the handshake runs.
    /// 3. Runs the initialize handshake with the current auth token, if any.
    ///
    /// Returns `Ok(InitializeHandshake)` on success, or a phase-tagged error.
    #[cfg(not(target_family = "wasm"))]
    async fn run_connect_and_handshake(
        session_id: SessionId,
        transport: &dyn RemoteTransport,
        auth_context: &RemoteServerAuthContext,
        spawner: &ModelSpawner<Self>,
        executor: &Arc<warpui::r#async::executor::Background>,
    ) -> Result<InitializeHandshake, ConnectAndHandshakeError> {
        // Phase 1: Connect (establish streams, create client).
        let Connection {
            client,
            event_rx,
            failure_rx,
            child,
            control_path,
        } = transport
            .connect(executor.clone())
            .await
            .map_err(ConnectAndHandshakeError::Connect)?;

        let client = Arc::new(client);
        let client_for_init = Arc::clone(&client);

        // Transition to Initializing while the initialize request is in flight.
        // Guard: if the session was deregistered during `transport.connect()`,
        // the entry will have been removed; don't re-insert it.
        let was_inserted = spawner
            .spawn(move |me, _ctx| {
                if !me.sessions.contains_key(&session_id) {
                    return false;
                }
                me.sessions.insert(
                    session_id,
                    RemoteSessionState::Initializing {
                        client: client_for_init,
                        _child: child,
                        control_path,
                    },
                );
                true
            })
            .await
            .unwrap_or(false);

        if !was_inserted {
            return Err(ConnectAndHandshakeError::Connect(anyhow::anyhow!(
                "Session {session_id:?} was deregistered during connect"
            )));
        }

        // Phase 2: Initialize handshake.
        let auth_token = auth_context.get_auth_token().await;
        let resp = client
            .initialize(
                auth_token.as_deref(),
                InitializeParams {
                    user_id: auth_context.user_id().to_owned(),
                    user_email: auth_context.user_email().to_owned(),
                    crash_reporting_enabled: auth_context.crash_reporting_enabled(),
                },
            )
            .await
            .map_err(|e| ConnectAndHandshakeError::Initialize(anyhow::anyhow!("{e:#}")))?;

        // Version compatibility check. If the server reports a different
        // release tag than the client expects, the binary on disk is stale.
        // Remove it so the next reconnect (or explicit reconnect by the
        // user) will reinstall.
        //
        // For versioned channels (Stable, Preview, Dev, Integration) the
        // version is also encoded in the binary path and verified by the
        // pre-connect `check_binary` / post-install verification steps,
        // so this is a belt-and-suspenders check at zero extra cost (it
        // uses data already received in the InitializeResponse).
        let client_version = ChannelState::app_version();
        if !version_is_compatible(client_version, &resp.server_version) {
            log::warn!(
                "Remote server version mismatch, removing stale binary: session={session_id:?} \
                 client={client_version:?} server={:?}",
                resp.server_version
            );

            const REMOVAL_TIMEOUT: Duration = Duration::from_secs(5);

            if let Err(e) = transport
                .remove_remote_server_binary()
                .with_timeout(REMOVAL_TIMEOUT)
                .await
                .unwrap_or_else(|_| Err(anyhow::anyhow!("timed out after {REMOVAL_TIMEOUT:?}")))
            {
                log::warn!(
                    "Remote server stale binary removal failed: session={session_id:?} error={e:#}"
                );
            }
            return Err(ConnectAndHandshakeError::Initialize(anyhow::anyhow!(
                "remote server version mismatch (client: {client_version:?}, \
                 server: {:?}); reconnect to reinstall",
                resp.server_version
            )));
        }

        log::info!(
            "[Remote codebase indexing] Remote server initialize handshake complete: session={session_id:?} \
             host={} server_version={:?}",
            resp.host_id,
            resp.server_version,
        );

        Ok(InitializeHandshake {
            host_id: HostId::new(resp.host_id),
            event_rx,
            failure_rx,
        })
    }

    /// Removes a session from the manager and tears down its connection.
    ///
    /// Assumes the caller has already observed that the user's shell
    /// has exited (in practice this is only invoked from the
    /// `ExitShell` teardown path). Under that assumption we also force
    /// the local SSH `ControlMaster` to exit immediately via
    /// `ssh -O exit`, which is required because the master is the
    /// user's interactive ssh process and, without the explicit
    /// `-O exit`, it hangs waiting for remote-side cleanup of
    /// multiplexed channels (see [`crate::ssh::stop_control_master`]).
    ///
    /// Mechanically:
    /// 1. Remove the session entry. Dropping the `RemoteSessionState`
    ///    drops the transport's owned `Child`, which SIGKILLs the
    ///    `ssh … remote-server-proxy` subprocess via `kill_on_drop`.
    /// 2. If the session had a ControlMaster `control_path`, spawn a
    ///    background task that runs `ssh -O exit` against it.
    ///
    /// The `Child` is owned by the manager's state, *not* by
    /// `Arc<RemoteServerClient>`. Lingering `Arc` clones held elsewhere
    /// (e.g. by the per-session command executor) do *not* keep the
    /// subprocess alive -- removing the state here always SIGKILLs the
    /// child, regardless of client refcount.
    ///
    /// Two separate events can fire here, and they mean different things:
    ///
    /// * `SessionDisconnected` -- the *transport* went away. Emitted only
    ///   when the session was `Connected` at the time of deregistration.
    ///   Subscribers can use this to drop their
    ///   `Arc<RemoteServerClient>` references and cancel in-flight
    ///   requests. The same event also fires independently from
    ///   `mark_session_disconnected` when the stream drops on its own.
    /// * `SessionDeregistered` -- the manager is no longer *tracking* this
    ///   session. Always emitted, regardless of which state the session
    ///   was in, because the entry is being removed from `sessions`
    ///   outright. Unlike `SessionDisconnected`, this one never fires for
    ///   spontaneous drops -- only for explicit teardown.
    pub fn deregister_session(&mut self, session_id: SessionId, ctx: &mut ModelContext<Self>) {
        self.last_navigation.remove(&session_id);
        self.session_bootstrap_info.remove(&session_id);
        self.session_platforms.remove(&session_id);
        self.session_labels.remove(&session_id);

        // Remove the session entry. Dropping the `RemoteSessionState`
        // here drops the transport's owned `Child` (if any), which
        // SIGKILLs the `ssh … remote-server-proxy` subprocess via
        // `kill_on_drop`.
        let prev = self.sessions.remove(&session_id);

        // Extract the ControlMaster socket path (if any) so we can
        // force the master to exit below. Safe to do under the
        // "caller already observed ExitShell" assumption documented
        // above.
        #[cfg(not(target_family = "wasm"))]
        let control_path = match &prev {
            Some(RemoteSessionState::Connected { control_path, .. })
            | Some(RemoteSessionState::Initializing { control_path, .. })
            | Some(RemoteSessionState::AwaitingExitStatus { control_path, .. }) => {
                control_path.clone()
            }
            Some(RemoteSessionState::Reconnecting { control_path, .. }) => control_path.clone(),
            _ => None,
        };

        // Extract `host_id` from states that track a host connection.
        let host_id = match &prev {
            Some(RemoteSessionState::Connected { host_id, .. }) => Some(host_id.clone()),
            #[cfg(not(target_family = "wasm"))]
            Some(RemoteSessionState::Reconnecting { host_id, .. }) => Some(host_id.clone()),
            _ => None,
        };
        if let Some(host_id) = host_id {
            self.remove_from_host_index(&host_id, session_id);
            ctx.emit(RemoteServerManagerEvent::SessionDisconnected {
                session_id,
                host_id: host_id.clone(),
                exit_status: None,
                was_reconnect_attempt: false,
            });
            if !self.host_to_sessions.contains_key(&host_id) {
                ctx.emit(RemoteServerManagerEvent::HostDisconnected { host_id });
            }
        }
        ctx.emit(RemoteServerManagerEvent::SessionDeregistered { session_id });

        // Force the local SSH ControlMaster to exit after teardown.
        // Spawned detached because the ssh subcommand may take a moment
        // to complete and we don't want to block the main thread on it.
        #[cfg(not(target_family = "wasm"))]
        if let Some(control_path) = control_path {
            ctx.background_executor()
                .spawn(async move {
                    crate::ssh::stop_control_master(&control_path).await;
                })
                .detach();
        }
    }

    /// Returns the client for this session, if connected.
    pub fn client_for_session(&self, session_id: SessionId) -> Option<&Arc<RemoteServerClient>> {
        match self.sessions.get(&session_id) {
            Some(RemoteSessionState::Connected { client, .. }) => Some(client),
            _ => None,
        }
    }

    /// Returns an iterator over all currently connected clients.
    pub fn all_connected_clients(&self) -> impl Iterator<Item = &Arc<RemoteServerClient>> {
        self.sessions.values().filter_map(|state| match state {
            RemoteSessionState::Connected { client, .. } => Some(client),
            _ => None,
        })
    }

    /// Rotates the daemon-wide auth credential on each connected remote host.
    ///
    /// Only sessions whose stored `identity_key` matches the current identity
    /// (from `auth_context`) receive the notification. This prevents a stale
    /// session established under a previous user identity from receiving a
    /// newly-rotated bearer token that belongs to a different user.
    ///
    /// Within the matching identity, a daemon may have multiple client
    /// connections. The credential is stored daemon-wide, so sending one
    /// notification per connected host is sufficient.
    pub fn rotate_auth_token(&self, token: String) {
        let Some(ref auth_context) = self.auth_context else {
            log::warn!("Remote server rotate_auth_token: no auth_context available, skipping");
            return;
        };
        let current_identity_key = auth_context.remote_server_identity_key();
        let mut authenticated_hosts = HashSet::new();
        for state in self.sessions.values() {
            let RemoteSessionState::Connected {
                client,
                host_id,
                identity_key,
                ..
            } = state
            else {
                continue;
            };
            if identity_key != &current_identity_key {
                continue;
            }
            if authenticated_hosts.insert(host_id.clone()) {
                client.authenticate(&token);
            }
        }
    }

    /// Returns the connection state for this session.
    pub fn session(&self, session_id: SessionId) -> Option<&RemoteSessionState> {
        self.sessions.get(&session_id)
    }

    /// Returns `true` when the session exists and is in a state where the
    /// remote server might still deliver data (`Connecting`, `Initializing`,
    /// `Connected`, or `Reconnecting`). Returns `false` for `Disconnected`
    /// sessions and sessions not tracked by the manager.
    pub fn is_session_potentially_active(&self, session_id: SessionId) -> bool {
        match self.sessions.get(&session_id) {
            Some(RemoteSessionState::Disconnected) | None => false,
            #[cfg(not(target_family = "wasm"))]
            Some(RemoteSessionState::AwaitingExitStatus { .. }) => false,
            Some(
                RemoteSessionState::Connecting
                | RemoteSessionState::Initializing { .. }
                | RemoteSessionState::Connected { .. },
            ) => true,
            #[cfg(not(target_family = "wasm"))]
            Some(RemoteSessionState::Reconnecting { .. }) => true,
        }
    }

    /// Returns the detected remote platform for this session, if available.
    pub fn platform_for_session(&self, session_id: SessionId) -> Option<&RemotePlatform> {
        self.session_platforms.get(&session_id)
    }

    /// Returns the `HostId` for this session, if the initialize handshake
    /// has completed. Downstream features use this to deduplicate
    /// host-scoped models (e.g. `RepoMetadataModel`).
    pub fn host_id_for_session(&self, session_id: SessionId) -> Option<&HostId> {
        match self.sessions.get(&session_id) {
            Some(RemoteSessionState::Connected { host_id, .. }) => Some(host_id),
            _ => None,
        }
    }

    /// Returns all session IDs connected to a given host. O(1) via the
    /// reverse index.
    pub fn sessions_for_host(&self, host_id: &HostId) -> Option<&HashSet<SessionId>> {
        self.host_to_sessions.get(host_id)
    }

    fn connected_session_for_host(
        &self,
        host_id: &HostId,
        expected_identity_key: &str,
    ) -> Option<(SessionId, Arc<RemoteServerClient>, String)> {
        let sessions = self.host_to_sessions.get(host_id)?;
        sessions.iter().find_map(|session_id| {
            let RemoteSessionState::Connected {
                client,
                identity_key,
                ..
            } = self.sessions.get(session_id)?
            else {
                return None;
            };
            if identity_key != expected_identity_key {
                return None;
            }
            Some((*session_id, client.clone(), identity_key.clone()))
        })
    }

    /// Ensures a codebase index exists for this remote path without resyncing an existing index.
    pub fn ensure_codebase_indexed(
        &mut self,
        remote_path: RemotePath,
        ctx: &mut ModelContext<Self>,
    ) {
        self.mutate_codebase_index(remote_path, RemoteCodebaseIndexMutation::EnsureIndexed, ctx);
    }

    /// Sends a `ResyncCodebase` request to a connected daemon for this remote path.
    pub fn resync_codebase(&mut self, remote_path: RemotePath, ctx: &mut ModelContext<Self>) {
        self.mutate_codebase_index(remote_path, RemoteCodebaseIndexMutation::Resync, ctx);
    }

    /// Sends a `DropCodebaseIndex` request to a connected daemon for this remote path.
    pub fn drop_codebase_index(&mut self, remote_path: RemotePath, ctx: &mut ModelContext<Self>) {
        self.mutate_codebase_index(remote_path, RemoteCodebaseIndexMutation::Drop, ctx);
    }

    fn mutate_codebase_index(
        &mut self,
        remote_path: RemotePath,
        mutation: RemoteCodebaseIndexMutation,
        ctx: &mut ModelContext<Self>,
    ) {
        let operation = mutation.operation();
        let host_id = remote_path.host_id.clone();
        let repo_path = remote_path.path.as_str().to_string();

        let Some(auth_context) = self.auth_context.clone() else {
            log::warn!(
                "Remote server codebase index mutation: no auth context \
                 operation={operation:?} host={host_id} repo_path={repo_path}"
            );
            return;
        };
        let current_identity_key = auth_context.remote_server_identity_key();
        let Some((session_id, client, remote_identity_key)) =
            self.connected_session_for_host(&host_id, &current_identity_key)
        else {
            log::warn!(
                "Remote server codebase index mutation: no connected client for current identity \
                 operation={operation:?} host={host_id} repo_path={repo_path}"
            );
            return;
        };
        log::info!(
            "[Remote codebase indexing] Manager requesting codebase index mutation: \
             operation={operation:?} host={host_id} session={session_id:?} \
             remote_identity_key={remote_identity_key} repo_path={repo_path}"
        );

        let spawner = self.spawner.clone();
        ctx.background_executor()
            .spawn(async move {
                let repo_path_for_log = repo_path.clone();
                let Some(auth_token) = auth_context.get_auth_token().await else {
                    log::warn!(
                        "Remote server codebase index mutation: missing auth token \
                         operation={operation:?} host={host_id} session={session_id:?} \
                         repo_path={repo_path_for_log}"
                    );
                    let _ = spawner
                        .spawn(move |_me, ctx| {
                            ctx.emit(RemoteServerManagerEvent::ClientRequestFailed {
                                session_id,
                                operation,
                                error_kind: RemoteServerErrorKind::Other,
                            });
                        })
                        .await;
                    return;
                };

                match mutation.send(client, repo_path, auth_token).await {
                    Ok(status) => {
                        log::info!(
                            "[Remote codebase indexing] Manager received codebase index mutation response: \
                             operation={operation:?} host={host_id} session={session_id:?} \
                             remote_identity_key={remote_identity_key} repo_path={} state={:?}",
                            status.repo_path,
                            status.state
                        );
                        let remote_path = remote_path_for_status(&host_id, &status).unwrap_or(remote_path);
                        let _ = spawner
                            .spawn(move |_me, ctx| {
                                ctx.emit(RemoteServerManagerEvent::CodebaseIndexStatusUpdated {
                                    remote_path,
                                    status,
                                });
                            })
                            .await;
                    }
                    Err(e) => {
                        log::warn!(
                            "Remote server codebase index mutation failed: \
                             operation={operation:?} host={host_id} session={session_id:?} \
                             repo_path={repo_path_for_log} error={e}"
                        );
                        // Transport-level telemetry is emitted automatically
                        // by send_tracked_request via ClientEvent::RequestFailed.
                    }
                }
            })
            .detach();
    }

    /// Sends a `NavigatedToDirectory` request to the remote server for
    /// the given session and returns a future that resolves with the
    /// navigation result on success, or `None` on failure. The
    /// `NavigatedToDirectory` event is still emitted for other
    /// subscribers (file tree, etc.).
    ///
    /// Deduplicates: if the same `(session_id, path)` was already requested,
    /// returns the cached result from the last successful navigation instead
    /// of re-issuing the request.
    ///
    /// Callers that don't need the result can simply drop the future.
    pub fn navigate_to_directory(
        &mut self,
        session_id: SessionId,
        path: String,
        ctx: &mut ModelContext<Self>,
    ) -> impl Future<Output = Option<RemoteNavigationResult>> {
        use futures::future::ready;

        match self.navigate_to_directory_impl(session_id, path, ctx) {
            Some(rx) => futures::future::Either::Left(async move { rx.await.ok().flatten() }),
            None => {
                // Dedup skip or missing client — return the cached result
                // from the last successful navigation so callers don't
                // misinterpret the skip as "not a git repo".
                let cached = self
                    .last_navigation
                    .get(&session_id)
                    .and_then(|c| c.result.clone());
                futures::future::Either::Right(ready(cached))
            }
        }
    }

    /// Returns `Some(receiver)` when a request was dispatched, `None` when
    /// skipped (dedup or missing client).
    fn navigate_to_directory_impl(
        &mut self,
        session_id: SessionId,
        path: String,
        ctx: &mut ModelContext<Self>,
    ) -> Option<futures::channel::oneshot::Receiver<Option<RemoteNavigationResult>>> {
        // Dedup: skip if this session already navigated to the same path.
        if self
            .last_navigation
            .get(&session_id)
            .is_some_and(|c| c.path == path)
        {
            return None;
        }

        let client = self.client_for_session(session_id).cloned()?;
        let host_id = self.host_id_for_session(session_id).cloned()?;

        // Record only after confirming the client is connected, so that a
        // retry after SessionConnected is not incorrectly deduplicated.
        self.last_navigation.insert(
            session_id,
            NavigationCache {
                path: path.clone(),
                result: None,
            },
        );

        let (tx, rx) = futures::channel::oneshot::channel();
        let spawner = self.spawner.clone();
        ctx.background_executor()
            .spawn(async move {
                match client.navigate_to_directory(path).await {
                    Ok(resp) => {
                        let _ = spawner
                            .spawn(move |me, ctx| {
                                let Some(remote_path) = StandardizedPath::try_new(&resp.indexed_path)
                                    .ok()
                                    .map(|path| RemotePath::new(host_id, path))
                                else {
                                    log::warn!(
                                        "Remote server dropped navigation event with invalid indexed path: \
                                         session={session_id:?} indexed_path={}",
                                        resp.indexed_path
                                    );
                                    let _ = tx.send(None);
                                    return;
                                };
                                let result = RemoteNavigationResult {
                                    remote_path: remote_path.clone(),
                                    is_git: resp.is_git,
                                };
                                if let Some(cache) = me.last_navigation.get_mut(&session_id) {
                                    cache.result = Some(result.clone());
                                }
                                let _ = tx.send(Some(result));
                                ctx.emit(RemoteServerManagerEvent::NavigatedToDirectory {
                                    session_id,
                                    remote_path,
                                    is_git: resp.is_git,
                                });
                            })
                            .await;
                    }
                    Err(e) => {
                        log::warn!("Remote server navigate_to_directory failed: session={session_id:?} error={e}");
                        let _ = tx.send(None);
                    }
                }
            })
            .detach();

        Some(rx)
    }

    /// Sends a `SessionBootstrapped` notification to the remote server.
    ///
    /// If the session is already in `Connected` state the notification is sent
    /// immediately. Otherwise it is stashed and automatically flushed when
    /// `mark_session_connected` transitions the session to `Connected`.
    pub fn notify_session_bootstrapped(
        &mut self,
        session_id: SessionId,
        shell_type: &str,
        shell_path: Option<&str>,
    ) {
        // Always persist so we can re-send after a reconnect.
        self.session_bootstrap_info.insert(
            session_id,
            SessionBootstrapInfo {
                shell_type: shell_type.to_owned(),
                shell_path: shell_path.map(ToOwned::to_owned),
            },
        );

        if let Some(client) = self.client_for_session(session_id) {
            client.notify_session_bootstrapped(session_id, shell_type, shell_path);
        } else {
            log::info!(
                "notify_session_bootstrapped: session {session_id:?} not yet connected, \
                 will send on connect"
            );
        }
    }

    /// Sends a `GetDiffState` request to the remote server for the given
    /// session and emits the snapshot response as a manager event.
    pub fn get_diff_state(
        &mut self,
        session_id: SessionId,
        remote_path: RemotePath,
        mode: DiffMode,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(client) = self.client_for_session(session_id).cloned() else {
            log::warn!("Remote server get_diff_state: no connected client session={session_id:?}");
            return;
        };

        let RemotePath {
            host_id,
            path: repo_path,
        } = remote_path;
        let mode_for_event = mode.clone();
        let repo_path_for_event = repo_path.clone();
        let spawner = self.spawner.clone();
        ctx.background_executor()
            .spawn(async move {
                let result = client.get_diff_state(&repo_path, mode).await;
                match result {
                    Ok(GetDiffStateResponse {
                        result: Some(get_diff_state_response::Result::Snapshot(snapshot)),
                    }) => {
                        let _ = spawner
                            .spawn(move |_me, ctx| {
                                ctx.emit(RemoteServerManagerEvent::DiffStateSnapshotReceived {
                                    host_id,
                                    repo_path: repo_path_for_event,
                                    mode: mode_for_event,
                                    snapshot,
                                });
                            })
                            .await;
                    }
                    other => {
                        let error_message = match other {
                            Ok(GetDiffStateResponse {
                                result: Some(get_diff_state_response::Result::Error(e)),
                            }) => {
                                log::warn!("Remote server get_diff_state error: {}", e.message);
                                e.message
                            }
                            Ok(_) => {
                                let message = "Remote server returned an empty \
                                                GetDiffStateResponse"
                                    .to_string();
                                log::warn!("{message}");
                                message
                            }
                            Err(e) => {
                                // Transport-level telemetry is emitted automatically
                                // by send_request via RequestFailedEvent.
                                log::warn!(
                                    "Remote server get_diff_state failed: \
                                     session={session_id:?} error={e}"
                                );
                                e.to_string()
                            }
                        };
                        let error_snapshot = DiffStateSnapshot {
                            repo_path: repo_path_for_event.to_string(),
                            mode: Some(mode_for_event.clone()),
                            metadata: None,
                            state: Some(DiffState {
                                state: Some(diff_state::State::Error(DiffStateErrorValue {
                                    message: error_message,
                                })),
                            }),
                            diffs: None,
                        };
                        let _ = spawner
                            .spawn(move |_me, ctx| {
                                ctx.emit(RemoteServerManagerEvent::DiffStateSnapshotReceived {
                                    host_id,
                                    repo_path: repo_path_for_event,
                                    mode: mode_for_event,
                                    snapshot: error_snapshot,
                                });
                            })
                            .await;
                    }
                }
            })
            .detach();
    }

    /// Sends a `GetBranches` request to the remote server for the given
    /// session and emits the result as a manager event.
    pub fn get_branches(
        &mut self,
        session_id: SessionId,
        remote_path: RemotePath,
        max_branch_count: Option<u32>,
        include_remotes: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(client) = self.client_for_session(session_id).cloned() else {
            log::warn!("Remote server get_branches: no connected client session={session_id:?}");
            return;
        };

        let repo_path = remote_path.path;
        let repo_path_for_event = repo_path.clone();
        let spawner = self.spawner.clone();
        ctx.background_executor()
            .spawn(async move {
                let result = client
                    .get_branches(&repo_path, max_branch_count, include_remotes)
                    .await
                    .map_err(|e| e.to_string());
                let _ = spawner
                    .spawn(move |_me, ctx| {
                        ctx.emit(RemoteServerManagerEvent::GetBranchesResponse {
                            session_id,
                            repo_path: repo_path_for_event,
                            result,
                        });
                    })
                    .await;
            })
            .detach();
    }

    /// Sends an `UnsubscribeDiffState` notification (fire-and-forget) to the
    /// remote server for the given session.
    pub fn unsubscribe_diff_state(
        &self,
        session_id: SessionId,
        remote_path: &RemotePath,
        mode: DiffMode,
    ) {
        if let Some(client) = self.client_for_session(session_id) {
            client.unsubscribe_diff_state(&remote_path.path, mode);
        } else {
            log::debug!(
                "Remote server unsubscribe_diff_state: no client for session={session_id:?}"
            );
        }
    }

    /// Sends a `DiscardFiles` request to the remote server. On success the
    /// server's watcher will push updated diff snapshots.
    #[allow(clippy::too_many_arguments)]
    pub fn discard_files(
        &mut self,
        session_id: SessionId,
        remote_path: RemotePath,
        files: Vec<FileStatusInfo>,
        should_stash: bool,
        branch_name: Option<String>,
        mode: DiffMode,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(client) = self.client_for_session(session_id).cloned() else {
            log::warn!("Remote server discard_files: no connected client session={session_id:?}");
            return;
        };

        let repo_path = remote_path.path;
        ctx.background_executor()
            .spawn(async move {
                match client
                    .discard_files(&repo_path, files, should_stash, branch_name, mode)
                    .await
                {
                    Ok(()) => {
                        log::info!("Remote server discard_files succeeded");
                    }
                    Err(e) => {
                        log::warn!(
                            "Remote server discard_files failed: session={session_id:?} error={e}"
                        );
                        // Transport-level telemetry is emitted automatically
                        // by send_request via RequestFailedEvent.
                    }
                }
            })
            .detach();
    }

    /// Sends a `LoadRepoMetadataDirectory` request to the remote server for
    /// the given session and emits the response as a manager event.
    pub fn load_remote_repo_metadata_directory(
        &mut self,
        session_id: SessionId,
        repo_path: String,
        dir_path: String,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(client) = self.client_for_session(session_id).cloned() else {
            log::warn!(
                "Remote server load_remote_repo_metadata_directory: no connected client session={session_id:?}"
            );
            return;
        };
        let Some(host_id) = self.host_id_for_session(session_id).cloned() else {
            log::warn!(
                "Remote server load_remote_repo_metadata_directory: no host_id session={session_id:?}"
            );
            return;
        };

        let spawner = self.spawner.clone();
        ctx.background_executor()
            .spawn(async move {
                match client
                    .load_repo_metadata_directory(repo_path, dir_path)
                    .await
                {
                    Ok(resp) => {
                        if let Some(update) =
                            proto_load_repo_metadata_directory_response_to_update(&resp)
                        {
                            let _ = spawner
                                .spawn(move |_me, ctx| {
                                    ctx.emit(
                                        RemoteServerManagerEvent::RepoMetadataDirectoryLoaded {
                                            host_id,
                                            update,
                                        },
                                    );
                                })
                                .await;
                        }
                    }
                    Err(e) => {
                        log::warn!(
                            "Remote server load_repo_metadata_directory failed: session={session_id:?} error={e}"
                        );
                        // Transport-level telemetry is emitted automatically
                        // by send_tracked_request via ClientEvent::RequestFailed.
                    }
                }
            })
            .detach();
    }

    /// Forwards a push event from the client event channel as a manager event.
    /// No-ops if the session is not in `Connected` state.
    #[cfg(not(target_family = "wasm"))]
    fn forward_client_event(
        &mut self,
        session_id: SessionId,
        event: ClientEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(RemoteSessionState::Connected { host_id, .. }) = self.sessions.get(&session_id)
        else {
            let event_kind = client_event_kind(&event);
            log::info!(
                "Dropping remote server push event for non-connected session: \
                 session={session_id:?} event={event_kind}"
            );
            return;
        };
        let host_id = host_id.clone();

        match event {
            ClientEvent::RepoMetadataSnapshotReceived { update } => {
                ctx.emit(RemoteServerManagerEvent::RepoMetadataSnapshot { host_id, update });
            }
            ClientEvent::RepoMetadataUpdated { update } => {
                ctx.emit(RemoteServerManagerEvent::RepoMetadataUpdated { host_id, update });
            }
            ClientEvent::CodebaseIndexStatusesSnapshotReceived { statuses } => {
                let statuses = statuses
                    .into_iter()
                    .filter_map(|status| {
                        let Some(remote_path) = remote_path_for_status(&host_id, &status) else {
                            log::warn!(
                                "Remote server dropped codebase index snapshot status with invalid repo path: \
                                 host={host_id} repo_path={}",
                                status.repo_path
                            );
                            return None;
                        };
                        Some(RemoteCodebaseIndexStatusWithPath {
                            remote_path,
                            status,
                        })
                    })
                    .collect();
                ctx.emit(RemoteServerManagerEvent::CodebaseIndexStatusesSnapshot {
                    host_id,
                    statuses,
                });
            }
            ClientEvent::CodebaseIndexStatusUpdated { status } => {
                let Some(remote_path) = remote_path_for_status(&host_id, &status) else {
                    log::warn!(
                        "Remote server dropped codebase index status update with invalid repo path: \
                         host={host_id} repo_path={}",
                        status.repo_path
                    );
                    return;
                };
                ctx.emit(RemoteServerManagerEvent::CodebaseIndexStatusUpdated {
                    remote_path,
                    status,
                });
            }
            ClientEvent::MessageDecodingError => {
                ctx.emit(RemoteServerManagerEvent::ServerMessageDecodingError { session_id });
            }
            ClientEvent::BufferUpdated {
                path,
                new_server_version,
                expected_client_version,
                edits,
            } => {
                ctx.emit(RemoteServerManagerEvent::BufferUpdated {
                    host_id: host_id.clone(),
                    path,
                    new_server_version,
                    expected_client_version,
                    edits,
                });
            }
            ClientEvent::BufferConflictDetected { path } => {
                ctx.emit(RemoteServerManagerEvent::BufferConflictDetected { host_id, path });
            }
            ClientEvent::DiffStateSnapshotReceived {
                repo_path,
                mode,
                snapshot,
            } => {
                ctx.emit(RemoteServerManagerEvent::DiffStateSnapshotReceived {
                    host_id,
                    repo_path,
                    mode,
                    snapshot,
                });
            }
            ClientEvent::DiffStateMetadataUpdateReceived {
                repo_path,
                mode,
                update,
            } => {
                ctx.emit(RemoteServerManagerEvent::DiffStateMetadataUpdateReceived {
                    host_id,
                    repo_path,
                    mode,
                    update,
                });
            }
            ClientEvent::DiffStateFileDeltaReceived {
                repo_path,
                mode,
                delta,
            } => {
                ctx.emit(RemoteServerManagerEvent::DiffStateFileDeltaReceived {
                    host_id,
                    repo_path,
                    mode,
                    delta,
                });
            }
            ClientEvent::Disconnected => {
                // Handled by the drain loop's completion callback.
            }
        }
    }

    /// Transitions a session from `Initializing` to `Connected`. Stores the
    /// `transport` for reconnection support after a spontaneous disconnect.
    #[cfg(not(target_family = "wasm"))]
    fn mark_session_connected(
        &mut self,
        session_id: SessionId,
        handshake: InitializeHandshake,
        identity_key: String,
        transport: Arc<dyn RemoteTransport>,
        ctx: &mut ModelContext<Self>,
    ) {
        let InitializeHandshake {
            host_id,
            event_rx,
            failure_rx,
        } = handshake;
        log::info!("Remote server connected: session={session_id:?} host={host_id}");

        // Only transition if the session is still in Initializing state.
        let Some(RemoteSessionState::Initializing {
            client,
            _child,
            control_path,
        }) = self.sessions.remove(&session_id)
        else {
            return;
        };

        let is_first_session = !self.host_to_sessions.contains_key(&host_id);
        self.sessions.insert(
            session_id,
            RemoteSessionState::Connected {
                client: client.clone(),
                host_id: host_id.clone(),
                identity_key,
                _child,
                control_path,
                transport,
            },
        );
        self.host_to_sessions
            .entry(host_id.clone())
            .or_default()
            .insert(session_id);
        ctx.spawn_stream_local(
            event_rx,
            move |me, event, ctx| {
                me.forward_client_event(session_id, event, ctx);
            },
            move |me, ctx| {
                me.mark_session_disconnected(session_id, ctx);
            },
        );
        // Drain the separate failure channel for request-failed telemetry.
        // This stream is independent of the lifecycle stream above, so
        // holding its sender on the client does not block disconnect.
        ctx.spawn_stream_local(
            failure_rx,
            move |_me, event, ctx| {
                ctx.emit(RemoteServerManagerEvent::ClientRequestFailed {
                    session_id,
                    operation: event.operation,
                    error_kind: event.error_kind,
                });
            },
            |_, _| {}, // no-op on done
        );
        if is_first_session {
            ctx.emit(RemoteServerManagerEvent::HostConnected {
                host_id: host_id.clone(),
            });
        }
        ctx.emit(RemoteServerManagerEvent::SetupStateChanged {
            session_id,
            state: RemoteServerSetupState::Ready,
        });
        ctx.emit(RemoteServerManagerEvent::SessionConnected {
            session_id,
            host_id,
        });

        // (Re-)send the SessionBootstrapped notification so the daemon
        // registers an executor for this session. This fires on both the
        // initial connect and every reconnect.
        if let Some(info) = self.session_bootstrap_info.get(&session_id) {
            if let Some(client) = self.client_for_session(session_id) {
                log::info!(
                    "Remote server sending SessionBootstrapped notification: session={session_id:?}"
                );
                client.notify_session_bootstrapped(
                    session_id,
                    &info.shell_type,
                    info.shell_path.as_deref(),
                );
            }
        }
    }

    /// Captures the exit status from a `Child` process, if available.
    #[cfg(not(target_family = "wasm"))]
    fn capture_exit_status(
        child: &mut async_process::Child,
        session_id: SessionId,
    ) -> Option<RemoteServerExitStatus> {
        match child.try_status() {
            Ok(Some(status)) => {
                let code = status.code();
                #[cfg(unix)]
                let signal_killed = {
                    use std::os::unix::process::ExitStatusExt;
                    status.signal().is_some()
                };
                #[cfg(not(unix))]
                let signal_killed = false;
                log::warn!(
                    "Remote server process exited: session={session_id:?} code={code:?} signal_killed={signal_killed}"
                );
                Some(RemoteServerExitStatus {
                    code,
                    signal_killed,
                })
            }
            Ok(None) => {
                log::warn!(
                    "Remote server process still running despite EOF on reader task: session={session_id:?}"
                );
                None
            }
            Err(e) => {
                log::warn!(
                    "Remote server exit status read failed: session={session_id:?} error={e}"
                );
                None
            }
        }
    }

    /// Asynchronously awaits the exit status of a `Child` process with a
    /// short timeout.
    ///
    /// Unlike [`capture_exit_status`] which uses the non-blocking
    /// `try_status()`, this method waits for the process to exit, giving
    /// the SSH subprocess a window to report its exit code and signal
    /// status. This is important for connection failures like
    /// `ResponseChannelClosed` where the pipe breaks before the
    /// subprocess has fully exited, causing `try_status()` to return
    /// `None` due to the timing race.
    #[cfg(not(target_family = "wasm"))]
    async fn await_exit_status(
        mut child: async_process::Child,
        session_id: SessionId,
    ) -> Option<RemoteServerExitStatus> {
        match child.status().with_timeout(EXIT_STATUS_WAIT_TIMEOUT).await {
            Ok(Ok(status)) => {
                let code = status.code();
                #[cfg(unix)]
                let signal_killed = {
                    use std::os::unix::process::ExitStatusExt;
                    status.signal().is_some()
                };
                #[cfg(not(unix))]
                let signal_killed = false;
                log::info!(
                    "Remote server process exited (async): session={session_id:?} \
                     code={code:?} signal_killed={signal_killed}"
                );
                Some(RemoteServerExitStatus {
                    code,
                    signal_killed,
                })
            }
            Ok(Err(e)) => {
                log::warn!(
                    "Remote server exit status read failed: session={session_id:?} error={e}"
                );
                None
            }
            Err(_) => {
                log::warn!(
                    "Remote server process did not exit within \
                     {EXIT_STATUS_WAIT_TIMEOUT:?}: session={session_id:?}"
                );
                None
            }
        }
    }

    #[cfg(not(target_family = "wasm"))]
    pub(crate) fn mark_session_disconnected(
        &mut self,
        session_id: SessionId,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(prev) = self.sessions.remove(&session_id) else {
            return;
        };

        // Only attempt reconnect for sessions that were in Connected state
        // with a transport available, and not being explicitly deregistered.
        if let RemoteSessionState::Connected {
            host_id,
            identity_key,
            mut _child,
            control_path,
            transport,
            ..
        } = prev
        {
            let exit_status = Self::capture_exit_status(&mut _child, session_id);
            // Drop the old child process explicitly before reconnecting.
            drop(_child);

            // Ask the transport whether a reconnect is viable given the
            // exit status. For example, SSH returns false when exit code
            // 255 indicates the ControlMaster's TCP connection is dead.
            if !transport.is_reconnectable(exit_status.as_ref()) {
                log::warn!(
                    "Transport reports disconnect is not reconnectable, skipping reconnect: session={session_id:?} exit_status={exit_status:?}"
                );
                self.finalize_disconnect(session_id, host_id, exit_status, ctx);
                return;
            }

            let Some(auth_context) = self.auth_context.clone() else {
                log::warn!(
                    "Remote server spontaneous disconnect without auth context: session={session_id:?}"
                );
                self.finalize_disconnect(session_id, host_id, exit_status, ctx);
                return;
            };
            log::info!(
                "Remote server spontaneous disconnect, will attempt reconnect: session={session_id:?} host={host_id:?}"
            );

            // Clear stale repo metadata and host index so downstream
            // models don't hold onto data from the dead server process.
            self.remove_from_host_index(&host_id, session_id);
            if !self.host_to_sessions.contains_key(&host_id) {
                ctx.emit(RemoteServerManagerEvent::HostDisconnected {
                    host_id: host_id.clone(),
                });
            }

            // Clear navigation cache so navigate_to_directory re-fires
            // after reconnect. The cached path only dedupes for the
            // current remote server session.
            self.last_navigation.remove(&session_id);

            self.attempt_reconnect(
                session_id,
                ReconnectParams {
                    attempt: 1,
                    host_id,
                    exit_status,
                    transport,
                    auth_context,
                    control_path,
                    identity_key,
                },
                ctx,
            );
        } else {
            // Non-Connected states (Initializing, Connecting,
            // AwaitingExitStatus, etc.) — no reconnect, just mark
            // disconnected.
            self.sessions
                .insert(session_id, RemoteSessionState::Disconnected);
        }
    }

    /// Attempt to re-establish the remote server connection.
    #[cfg(not(target_family = "wasm"))]
    fn attempt_reconnect(
        &mut self,
        session_id: SessionId,
        params: ReconnectParams,
        ctx: &mut ModelContext<Self>,
    ) {
        let ReconnectParams {
            attempt,
            host_id,
            exit_status,
            transport,
            auth_context,
            control_path,
            identity_key,
        } = params;

        log::info!(
            "Attempting reconnect: session={session_id:?} attempt={attempt}/{MAX_RECONNECT_ATTEMPTS}"
        );

        self.sessions.insert(
            session_id,
            RemoteSessionState::Reconnecting {
                attempt,
                host_id: host_id.clone(),
                control_path: control_path.clone(),
            },
        );

        let spawner = self.spawner.clone();
        let executor = ctx.background_executor().clone();
        let transport_clone = Arc::clone(&transport);
        let auth_context_for_task = Arc::clone(&auth_context);

        ctx.background_executor()
            .spawn(async move {
                async_io::Timer::after(RECONNECT_DELAY).await;

                // Check if the session was deregistered during the delay.
                // (Checked via spawner since sessions lives on the main thread.)
                let was_removed = spawner
                    .spawn(move |me, _ctx| !me.sessions.contains_key(&session_id))
                    .await
                    .unwrap_or(true);
                if was_removed {
                    log::info!("Remote server session removed during reconnect delay: session={session_id:?}");
                    return;
                }

                match Self::run_connect_and_handshake(
                    session_id,
                    &*transport_clone,
                    &auth_context_for_task,
                    &spawner,
                    &executor,
                )
                .await
                {
                    Ok(handshake) => {
                        let _ = spawner
                            .spawn(move |me, ctx| {
                                // If the session was deregistered during the
                                // handshake, don't resurrect it.
                                if !me.sessions.contains_key(&session_id) {
                                    log::info!(
                                        "Remote server session deregistered during reconnect handshake, aborting: session={session_id:?}"
                                    );
                                    return;
                                }
                                let host_id = handshake.host_id.clone();
                                me.mark_session_connected(
                                    session_id,
                                    handshake,
                                    identity_key,
                                    transport,
                                    ctx,
                                );
                                if let Some(client) = me.client_for_session(session_id).cloned() {
                                    ctx.emit(RemoteServerManagerEvent::SessionReconnected {
                                        session_id,
                                        host_id,
                                        attempt,
                                        client,
                                    });
                                }
                            })
                            .await;
                    }
                    Err(e) => {
                        log::warn!(
                            "Remote server reconnect failed: session={session_id:?} attempt={attempt} error={e}"
                        );
                        let _ = spawner
                            .spawn(move |me, ctx| {
                                // If the session was deregistered during the
                                // handshake, don't retry or insert Disconnected.
                                if !me.sessions.contains_key(&session_id) {
                                    log::info!(
                                        "Remote server session deregistered during reconnect handshake, aborting: session={session_id:?}"
                                    );
                                    return;
                                }
                                me.handle_reconnect_failure(
                                    session_id,
                                    ReconnectParams {
                                        attempt,
                                        host_id,
                                        exit_status,
                                        transport,
                                        auth_context,
                                        control_path,
                                        identity_key,
                                    },
                                    ctx,
                                );
                            })
                            .await;
                    }
                }
            })
            .detach();
    }

    /// Handle a failed reconnection attempt: either retry or give up.
    #[cfg(not(target_family = "wasm"))]
    fn handle_reconnect_failure(
        &mut self,
        session_id: SessionId,
        params: ReconnectParams,
        ctx: &mut ModelContext<Self>,
    ) {
        if params.attempt < MAX_RECONNECT_ATTEMPTS {
            self.attempt_reconnect(
                session_id,
                ReconnectParams {
                    attempt: params.attempt + 1,
                    ..params
                },
                ctx,
            );
        } else {
            log::warn!(
                "Remote server reconnect exhausted: session={session_id:?} attempts={}",
                params.attempt
            );
            self.sessions
                .insert(session_id, RemoteSessionState::Disconnected);
            ctx.emit(RemoteServerManagerEvent::SessionDisconnected {
                session_id,
                host_id: params.host_id,
                exit_status: params.exit_status,
                was_reconnect_attempt: true,
            });
            // Note: HostDisconnected was already emitted by
            // mark_session_disconnected when entering the reconnect flow.
        }
    }

    /// Marks a session as `Disconnected`, cleans up the host index, and
    /// emits the appropriate disconnect events. Used by
    /// `mark_session_disconnected` when reconnection is not possible
    /// (SSH transport failure, missing auth context).
    ///
    /// Not used by `handle_reconnect_failure` because that path enters
    /// from `attempt_reconnect`, which already cleared the host index
    /// and emitted `HostDisconnected` when entering the reconnect flow.
    #[cfg(not(target_family = "wasm"))]
    fn finalize_disconnect(
        &mut self,
        session_id: SessionId,
        host_id: HostId,
        exit_status: Option<RemoteServerExitStatus>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.sessions
            .insert(session_id, RemoteSessionState::Disconnected);
        self.remove_from_host_index(&host_id, session_id);
        ctx.emit(RemoteServerManagerEvent::SessionDisconnected {
            session_id,
            host_id: host_id.clone(),
            exit_status,
            was_reconnect_attempt: false,
        });
        if !self.host_to_sessions.contains_key(&host_id) {
            ctx.emit(RemoteServerManagerEvent::HostDisconnected { host_id });
        }
    }

    /// Removes a session from the host → sessions reverse index.
    /// Cleans up the entry entirely if the set becomes empty.
    fn remove_from_host_index(&mut self, host_id: &HostId, session_id: SessionId) {
        if let Some(set) = self.host_to_sessions.get_mut(host_id) {
            set.remove(&session_id);
            if set.is_empty() {
                self.host_to_sessions.remove(host_id);
            }
        }
    }
}
