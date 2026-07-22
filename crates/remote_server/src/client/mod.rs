use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use dashmap::DashMap;
use futures::channel::oneshot;
use futures::io::{AsyncRead, AsyncWrite};
use warpui_core::r#async::{FutureExt as _, executor};

use crate::codebase_index_proto::{
    RemoteCodebaseIndexStatus, proto_to_codebase_index_status_updated,
    proto_to_codebase_index_statuses_snapshot,
};
use crate::proto::{
    Abort, Authenticate, BufferEdit, ClientMessage, CloseBuffer, CodebaseIndexLimits, DiffMode,
    DiffStateFileDelta, DiffStateMetadataUpdate, DiffStateSnapshot, ErrorCode, GitStatusMetadata,
    Initialize, InitializeResponse, LoadRepoMetadataDirectoryResponse,
    NavigatedToDirectoryResponse, PrInfo, RemoteAgentContextSnapshot, RepositoryInfo,
    RunCommandRequest, RunCommandResponse, ServerMessage, SessionBootstrapped, TextEdit,
    UnsubscribeDiffState, UpdateGitHubPrInfo, UpdateGitHubRepoInfo, UpdateGitStatus, notification,
    server_message, session_scoped_request,
};
use crate::repo_metadata_proto::{proto_snapshot_to_update, proto_to_repo_metadata_update};

#[cfg(not(target_family = "wasm"))]
mod remote_server_log;
#[cfg(not(target_family = "wasm"))]
pub use remote_server_log::RemoteServerLog;
use warp_core::{SessionId, safe_error, safe_warn};
use warp_errors::report_error;
use warp_util::standardized_path::StandardizedPath;
use warpui_core::r#async::TransportStream;

use crate::protocol::{self, ProtocolError, RequestId};

/// Default request timeout (2 minutes).
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

/// Errors from the `RemoteServerClient`.
#[derive(thiserror::Error, Debug)]
pub enum ClientError {
    #[error("Connection was dropped")]
    Disconnected,

    #[error("Protocol error: {0}")]
    Protocol(#[from] ProtocolError),

    #[error("Response channel closed before receiving a reply")]
    ResponseChannelClosed,

    #[error("Unexpected response from server")]
    UnexpectedResponse,

    #[error("Server error ({code:?}): {message}")]
    ServerError { code: ErrorCode, message: String },

    #[error("Request timed out after {0:?}")]
    Timeout(Duration),
}

/// Events received from the remote server, delivered through the event
/// channel returned by [`RemoteServerClient::new`].
///
/// The consumer (typically `RemoteServerManager`) drains this channel to
/// react to connection lifecycle changes and server-pushed data.
#[derive(Clone, Debug)]
pub enum ClientEvent {
    /// The reader task detected EOF or a fatal error. The connection is gone.
    /// This is always the last event sent on the channel.
    Disconnected,
    /// A full or lazy-loaded repo metadata snapshot was pushed by the server.
    RepoMetadataSnapshotReceived {
        update: repo_metadata::RepoMetadataUpdate,
    },
    /// An incremental repo metadata update was pushed by the server.
    RepoMetadataUpdated {
        update: repo_metadata::RepoMetadataUpdate,
    },
    /// A full remote codebase-index status snapshot was pushed by the server.
    CodebaseIndexStatusesSnapshotReceived {
        statuses: Vec<RemoteCodebaseIndexStatus>,
    },
    /// A single remote codebase-index status update was pushed by the server.
    CodebaseIndexStatusUpdated { status: RemoteCodebaseIndexStatus },
    /// A server message could not be decoded and had no parseable request_id.
    MessageDecodingError,
    /// The writer task failed while writing a host-scoped request before it
    /// could be handed off to the daemon. The manager owns host-scoped
    /// lifecycle tracking, so it handles retrying this request through another
    /// connected session for the same host (or failing the request if none
    /// exists).
    HostScopedWriteFailed { request_id: RequestId },
    /// A server response carried a parseable `request_id` but could not be
    /// decoded, and the id did not match a session-scoped pending request (so
    /// it belongs to a host-scoped request the manager is tracking). The
    /// manager fails that pending request immediately instead of letting it
    /// hang until the request timeout. The daemon already produced a reply, so
    /// this is terminal — not retried.
    HostScopedDecodeFailed { request_id: RequestId },
    /// A buffer was updated on the server (file changed on disk).
    BufferUpdated {
        path: String,
        new_server_version: u64,
        expected_client_version: u64,
        edits: Vec<TextEdit>,
    },
    /// The file changed on disk while the client had unsaved edits.
    /// The server did NOT apply the change; the client should show a
    /// conflict resolution banner.
    BufferConflictDetected { path: String },
    /// A full diff state snapshot was pushed by the server.
    DiffStateSnapshotReceived {
        repo_path: StandardizedPath,
        mode: DiffMode,
        snapshot: DiffStateSnapshot,
    },
    /// A metadata-only diff state update was pushed by the server.
    DiffStateMetadataUpdateReceived {
        repo_path: StandardizedPath,
        mode: DiffMode,
        update: DiffStateMetadataUpdate,
    },
    /// A single-file diff delta was pushed by the server.
    DiffStateFileDeltaReceived {
        repo_path: StandardizedPath,
        mode: DiffMode,
        delta: DiffStateFileDelta,
    },
    /// The daemon pushed a revisioned full replacement of its Agent Mode context.
    RemoteAgentContextSnapshotReceived {
        snapshot: RemoteAgentContextSnapshot,
    },
    /// An aggregate git status push (branch + diff stats) was pushed by the
    /// server for the tab / prompt chips.
    GitStatusPushReceived {
        repo_path: StandardizedPath,
        metadata: GitStatusMetadata,
    },
    /// PR info for the current branch was pushed by the server for the PR chip.
    GitHubPrInfoPushReceived {
        repo_path: StandardizedPath,
        pr_info: Option<PrInfo>,
    },
    /// Repository name/owner info was pushed by the server for repository chips.
    GitHubRepositoryInfoPushReceived {
        repo_path: StandardizedPath,
        repository_info: Option<RepositoryInfo>,
    },
}

/// Parameters for the `Initialize` handshake, sent to the daemon at
/// connection time.
pub struct InitializeParams {
    pub user_id: String,
    pub user_email: String,
    pub crash_reporting_enabled: bool,
    pub codebase_index_limits: Option<CodebaseIndexLimits>,
}

/// A request-failure notification emitted by [`RemoteServerClient::send_request`].
/// Delivered on a dedicated channel separate from the lifecycle
/// [`ClientEvent`] stream so that holding this sender does not prevent
/// the lifecycle stream from closing (which would block
/// `mark_session_disconnected`).
#[derive(Clone, Debug)]
pub struct RequestFailedEvent {
    pub operation: crate::manager::RemoteServerOperation,
    pub error_kind: crate::manager::RemoteServerErrorKind,
}

/// Client for communicating with a `remote_server` process over the remote server protocol.
///
/// Exposes async request/response APIs over generic I/O streams (child-process pipes,
/// SSH channels, or in-memory streams for testing).
///
/// Designed to be wrapped in `Arc` for sharing across threads. Construction
/// returns an event receiver that delivers push events and a final
/// `Disconnected` event when the connection drops.
///
/// This type does **not** own the child subprocess whose stdio backs it.
/// For transports that spawn a subprocess (e.g. SSH), the caller is
/// responsible for holding the `Child` for the lifetime of the session
/// so that `kill_on_drop` fires when teardown occurs. In Warp this is
/// the `RemoteServerManager`, which stores the child in
/// `RemoteSessionState` alongside the `Arc<RemoteServerClient>`. That
/// way the child's lifetime is gated by the manager's session map
/// rather than by `Arc` refcount -- cloning `Arc<RemoteServerClient>`
/// into other owners (e.g. the command executor) no longer keeps the
/// child alive.
pub struct RemoteServerClient {
    /// Channel for queuing ClientMessages to send to the remote server.
    outbound_tx: async_channel::Sender<ClientMessage>,

    /// Maps `request_id` → oneshot sender for the correlated response from the remote server.
    /// Only used for session-scoped requests. Host-scoped requests bypass this
    /// map entirely (see `send_host_scoped`).
    pending_requests: Arc<DashMap<RequestId, oneshot::Sender<Result<ServerMessage, ClientError>>>>,

    /// Set to `true` by the reader task when the connection is lost. Checked by
    /// `send_request` after inserting into `pending_requests` to avoid hanging
    /// on a dead connection.
    disconnected: Arc<AtomicBool>,

    /// Dedicated channel for `RequestFailed` telemetry events. Separate from
    /// the lifecycle `event_tx` so that holding this sender does not keep the
    /// lifecycle stream alive (which would prevent `spawn_stream_local`'s
    /// completion callback from firing `mark_session_disconnected`).
    failure_tx: async_channel::Sender<RequestFailedEvent>,

    /// Channel for responses whose `request_id` is not in `pending_requests`.
    /// These are host-scoped responses — either the normal path (the request
    /// was sent via `send_host_scoped` on this connection) or daemon failover
    /// (the daemon re-routed a response from a dead connection to this one).
    /// The `RemoteServerManager` drains this channel to match against its
    /// `pending_host_requests`.
    ///
    /// Never read directly: the `reader_task` holds its own clone and is the
    /// only writer, so this copy exists only to keep a sender for the channel
    /// reachable from the client itself. `expect` (rather than `allow`) so we
    /// get nudged to drop the attribute if the field ever starts being read.
    #[expect(dead_code)]
    host_response_tx: async_channel::Sender<ServerMessage>,
}

impl fmt::Debug for RemoteServerClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RemoteServerClient").finish_non_exhaustive()
    }
}

#[cfg(not(target_family = "wasm"))]
impl RemoteServerClient {
    /// Creates a client from a child process's stdin, stdout, and stderr.
    ///
    /// The caller retains ownership of the `Child` itself. Typically the
    /// caller spawns the `Command` with `kill_on_drop(true)` and stashes
    /// the returned `Child` somewhere whose lifetime matches the
    /// session's (in Warp, on the `RemoteServerManager`'s
    /// `RemoteSessionState`). Dropping the `Child` there triggers
    /// SIGKILL on the subprocess, regardless of how many
    /// `Arc<RemoteServerClient>` clones are still alive.
    ///
    /// Internally forwards stderr lines to local logging via
    /// [`spawn_stderr_forwarder`], then delegates to [`Self::new`] for the
    /// protocol reader/writer setup.
    ///
    /// Returns the client and an event receiver that delivers push events
    /// and a final `Disconnected` event when the connection drops.
    pub fn from_child_streams(
        stdin: async_process::ChildStdin,
        stdout: async_process::ChildStdout,
        stderr: async_process::ChildStderr,
        executor: &executor::Background,
    ) -> (
        Self,
        async_channel::Receiver<ClientEvent>,
        async_channel::Receiver<RequestFailedEvent>,
        async_channel::Receiver<ServerMessage>,
        RemoteServerLog,
    ) {
        let stderr_tail = spawn_stderr_forwarder(stderr, executor);
        let (client, event_rx, failure_rx, host_response_rx) = Self::new(stdout, stdin, executor);
        (client, event_rx, failure_rx, host_response_rx, stderr_tail)
    }
}

impl RemoteServerClient {
    /// Creates a new client, spawning background reader and writer tasks on the
    /// provided executor.
    ///
    /// Returns the client and an event receiver that delivers push events
    /// and a final `Disconnected` event when the connection drops.
    pub fn new(
        reader: impl AsyncRead + TransportStream,
        writer: impl AsyncWrite + TransportStream,
        executor: &executor::Background,
    ) -> (
        Self,
        async_channel::Receiver<ClientEvent>,
        async_channel::Receiver<RequestFailedEvent>,
        async_channel::Receiver<ServerMessage>,
    ) {
        let pending_requests: Arc<
            DashMap<RequestId, oneshot::Sender<Result<ServerMessage, ClientError>>>,
        > = Arc::new(DashMap::new());
        let (outbound_tx, outbound_rx) = async_channel::unbounded::<ClientMessage>();
        let (event_tx, event_rx) = async_channel::unbounded::<ClientEvent>();
        let (failure_tx, failure_rx) = async_channel::unbounded::<RequestFailedEvent>();
        let (host_response_tx, host_response_rx) = async_channel::unbounded::<ServerMessage>();
        let disconnected = Arc::new(AtomicBool::new(false));

        executor
            .spawn(Self::writer_task(
                writer,
                outbound_rx,
                Arc::clone(&pending_requests),
                event_tx.clone(),
            ))
            .detach();
        executor
            .spawn(Self::reader_task(
                reader,
                Arc::clone(&pending_requests),
                event_tx,
                Arc::clone(&disconnected),
                host_response_tx.clone(),
            ))
            .detach();

        (
            Self {
                outbound_tx,
                pending_requests,
                disconnected,
                failure_tx,
                host_response_tx,
            },
            event_rx,
            failure_rx,
            host_response_rx,
        )
    }

    /// Returns `true` once the reader task has detected that the underlying
    /// connection is gone (EOF or fatal error). The flag is one-way: a
    /// client never transitions back to connected, since reconnection
    /// produces a brand-new client instance.
    ///
    /// Callers can use this as a cheap, non-blocking gate to skip work
    /// that would otherwise fail with [`ClientError::Disconnected`] and
    /// fire a `RequestFailed` telemetry event. Returning `false` does
    /// not guarantee the next request will succeed — it just means the
    /// reader task has not yet observed a disconnect.
    pub fn is_disconnected(&self) -> bool {
        self.disconnected.load(Ordering::Acquire)
    }

    /// Sends an `Initialize` request and awaits the `InitializeResponse`.
    pub async fn initialize(
        &self,
        auth_token: Option<&str>,
        params: InitializeParams,
    ) -> Result<InitializeResponse, ClientError> {
        let request_id = RequestId::new();
        let msg = ClientMessage::session_scoped(
            request_id.to_string(),
            session_scoped_request::Message::Initialize(Initialize {
                auth_token: auth_token.unwrap_or_default().to_owned(),
                user_id: params.user_id,
                user_email: params.user_email,
                crash_reporting_enabled: params.crash_reporting_enabled,
                codebase_index_limits: params.codebase_index_limits,
            }),
        );

        let response = self.send_request_internal(request_id, msg).await?;

        match response.message {
            Some(server_message::Message::InitializeResponse(resp)) => Ok(resp),
            other => {
                safe_error!(
                    safe: ("Remote server unexpected response for Initialize"),
                    full: ("Remote server unexpected response for Initialize: response={other:?}")
                );
                Err(ClientError::UnexpectedResponse)
            }
        }
    }

    /// Sends an `Authenticate` notification to rotate the daemon-wide
    /// credential after initialization.
    pub fn authenticate(&self, auth_token: &str) {
        let msg = ClientMessage::notification(notification::Message::Authenticate(Authenticate {
            auth_token: auth_token.to_owned(),
        }));
        self.send_notification(msg);
    }

    /// Sends an `UpdatePreferences` notification when the user's privacy
    /// settings change (e.g. toggling crash reporting).
    pub fn update_preferences(
        &self,
        crash_reporting_enabled: bool,
        codebase_index_limits: Option<CodebaseIndexLimits>,
    ) {
        let msg = ClientMessage::notification(notification::Message::UpdatePreferences(
            crate::proto::UpdatePreferences {
                crash_reporting_enabled,
                codebase_index_limits,
            },
        ));
        self.send_notification(msg);
    }

    /// Sends a `SessionBootstrapped` notification (fire-and-forget) so the
    /// server can create a `LocalCommandExecutor` for the session.
    pub fn notify_session_bootstrapped(
        &self,
        session_id: SessionId,
        shell_type: &str,
        shell_path: Option<&str>,
    ) {
        let msg = ClientMessage::notification(notification::Message::SessionBootstrapped(
            SessionBootstrapped {
                session_id: session_id.as_u64(),
                shell_type: shell_type.to_owned(),
                shell_path: shell_path.map(ToOwned::to_owned),
            },
        ));
        self.send_notification(msg);
    }

    /// Sends a `NavigatedToDirectory` request and awaits the response.
    pub async fn navigate_to_directory(
        &self,
        path: String,
    ) -> Result<NavigatedToDirectoryResponse, ClientError> {
        let request_id = RequestId::new();
        let msg = ClientMessage::session_scoped(
            request_id.to_string(),
            session_scoped_request::Message::NavigatedToDirectory(
                crate::proto::NavigatedToDirectory { path },
            ),
        );

        let response = self
            .send_request(
                request_id,
                msg,
                crate::manager::RemoteServerOperation::NavigateToDirectory,
            )
            .await?;

        match response.message {
            Some(server_message::Message::NavigatedToDirectoryResponse(resp)) => Ok(resp),
            other => {
                safe_error!(
                    safe: ("Remote server unexpected response for NavigatedToDirectory"),
                    full: ("Remote server unexpected response for NavigatedToDirectory: response={other:?}")
                );
                Err(ClientError::UnexpectedResponse)
            }
        }
    }

    /// Sends a `LoadRepoMetadataDirectory` request and awaits the response.
    pub async fn load_repo_metadata_directory(
        &self,
        repo_path: String,
        dir_path: String,
    ) -> Result<LoadRepoMetadataDirectoryResponse, ClientError> {
        let request_id = RequestId::new();
        let msg = ClientMessage::session_scoped(
            request_id.to_string(),
            session_scoped_request::Message::LoadRepoMetadataDirectory(
                crate::proto::LoadRepoMetadataDirectory {
                    repo_path,
                    dir_path,
                },
            ),
        );

        let response = self
            .send_request(
                request_id,
                msg,
                crate::manager::RemoteServerOperation::LoadRepoMetadataDirectory,
            )
            .await?;

        match response.message {
            Some(server_message::Message::LoadRepoMetadataDirectoryResponse(resp)) => Ok(resp),
            other => {
                safe_error!(
                    safe: ("Remote server unexpected response for LoadRepoMetadataDirectory"),
                    full: ("Remote server unexpected response for LoadRepoMetadataDirectory: response={other:?}")
                );
                Err(ClientError::UnexpectedResponse)
            }
        }
    }

    /// Sends a session-scoped `GetDiffState` request and awaits the response.
    ///
    /// Session-scoped (not host-scoped): the daemon registers a per-connection
    /// diff-state subscription, so the response — and every subsequent diff
    /// push — must travel on this connection. If the connection drops, the
    /// pending request resolves promptly with a transport error via
    /// `pending_requests` cleanup, instead of hanging until the timeout.
    pub async fn get_diff_state(
        &self,
        repo_path: String,
        mode: DiffMode,
    ) -> Result<crate::proto::GetDiffStateResponse, ClientError> {
        let request_id = RequestId::new();
        let msg = ClientMessage::session_scoped(
            request_id.to_string(),
            session_scoped_request::Message::GetDiffState(crate::proto::GetDiffState {
                repo_path,
                mode: Some(mode),
            }),
        );

        let response = self.send_request_internal(request_id, msg).await?;

        match response.message {
            Some(server_message::Message::GetDiffStateResponse(resp)) => Ok(resp),
            other => {
                safe_error!(
                    safe: ("Remote server unexpected response for GetDiffState"),
                    full: ("Remote server unexpected response for GetDiffState: response={other:?}")
                );
                Err(ClientError::UnexpectedResponse)
            }
        }
    }

    /// Sends a session-scoped `OpenBuffer` request and awaits the response.
    ///
    /// Session-scoped for the same reason as [`Self::get_diff_state`]: the
    /// daemon registers a per-connection buffer subscription, so the buffer's
    /// content response and subsequent `BufferUpdatedPush`es must stay on this
    /// connection. When `force_reload` is true the server re-reads the file
    /// from disk, discarding in-memory buffer state.
    pub async fn open_buffer(
        &self,
        path: String,
        force_reload: bool,
    ) -> Result<crate::proto::OpenBufferResponse, ClientError> {
        let request_id = RequestId::new();
        let msg = ClientMessage::session_scoped(
            request_id.to_string(),
            session_scoped_request::Message::OpenBuffer(crate::proto::OpenBuffer {
                path,
                force_reload,
            }),
        );

        let response = self.send_request_internal(request_id, msg).await?;

        match response.message {
            Some(server_message::Message::OpenBufferResponse(resp)) => Ok(resp),
            other => {
                safe_error!(
                    safe: ("Remote server unexpected response for OpenBuffer"),
                    full: ("Remote server unexpected response for OpenBuffer: response={other:?}")
                );
                Err(ClientError::UnexpectedResponse)
            }
        }
    }

    /// Sends a buffer edit notification (fire-and-forget) to the remote host.
    pub fn send_buffer_edit(
        &self,
        path: String,
        expected_server_version: u64,
        new_client_version: u64,
        edits: Vec<TextEdit>,
    ) {
        let msg = ClientMessage::notification(notification::Message::BufferEdit(BufferEdit {
            path,
            expected_server_version,
            new_client_version,
            edits,
        }));
        self.send_notification(msg);
    }

    /// Tells the remote host to close a buffer (stop watching).
    pub fn close_buffer(&self, path: String) {
        let msg =
            ClientMessage::notification(notification::Message::CloseBuffer(CloseBuffer { path }));
        self.send_notification(msg);
    }

    /// Converts a server push message (empty request_id) into a domain event.
    fn push_message_to_event(msg: ServerMessage) -> Option<ClientEvent> {
        match msg.message? {
            server_message::Message::RepoMetadataSnapshot(snapshot) => {
                let update = proto_snapshot_to_update(&snapshot)?;
                Some(ClientEvent::RepoMetadataSnapshotReceived { update })
            }
            server_message::Message::RepoMetadataUpdate(push) => {
                let update = proto_to_repo_metadata_update(&push)?;
                Some(ClientEvent::RepoMetadataUpdated { update })
            }
            server_message::Message::CodebaseIndexStatusesSnapshot(snapshot) => {
                let statuses = proto_to_codebase_index_statuses_snapshot(&snapshot);
                log::info!(
                    "[Remote codebase indexing] Client received codebase index statuses push: \
                     status_count={}",
                    statuses.len()
                );
                for status in &statuses {
                    log::info!(
                        "[Remote codebase indexing] Client received codebase index status in snapshot: \
                         repo_path={} state={:?} root_hash_present={} \
                         progress_completed={:?} progress_total={:?} \
                         failure_message={:?}",
                        status.repo_path,
                        status.state,
                        status.root_hash.is_some(),
                        status.progress_completed,
                        status.progress_total,
                        status.failure_message,
                    );
                }
                Some(ClientEvent::CodebaseIndexStatusesSnapshotReceived { statuses })
            }
            server_message::Message::CodebaseIndexStatusUpdated(update) => {
                let status = proto_to_codebase_index_status_updated(&update)?;
                log::info!(
                    "[Remote codebase indexing] Client received codebase index status push: \
                     repo_path={} state={:?} root_hash_present={} failure_message={:?}",
                    status.repo_path,
                    status.state,
                    status.root_hash.is_some(),
                    status.failure_message,
                );
                Some(ClientEvent::CodebaseIndexStatusUpdated { status })
            }
            server_message::Message::BufferUpdated(push) => Some(ClientEvent::BufferUpdated {
                path: push.path,
                new_server_version: push.new_server_version,
                expected_client_version: push.expected_client_version,
                edits: push.edits,
            }),
            server_message::Message::BufferConflictDetected(push) => {
                Some(ClientEvent::BufferConflictDetected { path: push.path })
            }
            server_message::Message::DiffStateSnapshot(snapshot) => {
                let Some(repo_path) = StandardizedPath::try_new(&snapshot.repo_path).ok() else {
                    log::warn!(
                        "DiffStateSnapshot: invalid repo_path: {}",
                        snapshot.repo_path
                    );
                    return None;
                };
                let Some(mode) = snapshot.mode.clone() else {
                    log::warn!(
                        "DiffStateSnapshot: missing mode for repo_path: {}",
                        snapshot.repo_path
                    );
                    return None;
                };
                Some(ClientEvent::DiffStateSnapshotReceived {
                    repo_path,
                    mode,
                    snapshot,
                })
            }
            server_message::Message::DiffStateMetadataUpdate(update) => {
                let Some(repo_path) = StandardizedPath::try_new(&update.repo_path).ok() else {
                    log::warn!(
                        "DiffStateMetadataUpdate: invalid repo_path: {}",
                        update.repo_path
                    );
                    return None;
                };
                let Some(mode) = update.mode.clone() else {
                    log::warn!(
                        "DiffStateMetadataUpdate: missing mode for repo_path: {}",
                        update.repo_path
                    );
                    return None;
                };
                Some(ClientEvent::DiffStateMetadataUpdateReceived {
                    repo_path,
                    mode,
                    update,
                })
            }
            server_message::Message::DiffStateFileDelta(delta) => {
                let Some(repo_path) = StandardizedPath::try_new(&delta.repo_path).ok() else {
                    log::warn!("DiffStateFileDelta: invalid repo_path: {}", delta.repo_path);
                    return None;
                };
                let Some(mode) = delta.mode.clone() else {
                    log::warn!(
                        "DiffStateFileDelta: missing mode for repo_path: {}",
                        delta.repo_path
                    );
                    return None;
                };
                Some(ClientEvent::DiffStateFileDeltaReceived {
                    repo_path,
                    mode,
                    delta,
                })
            }
            server_message::Message::RemoteAgentContextSnapshot(snapshot) => {
                Some(ClientEvent::RemoteAgentContextSnapshotReceived { snapshot })
            }
            server_message::Message::GitStatusPush(push) => {
                let Some(repo_path) = StandardizedPath::try_new(&push.repo_path).ok() else {
                    log::warn!("GitStatusPush: invalid repo_path: {}", push.repo_path);
                    return None;
                };
                let Some(metadata) = push.metadata else {
                    log::warn!(
                        "GitStatusPush: missing metadata for repo_path: {}",
                        push.repo_path
                    );
                    return None;
                };
                Some(ClientEvent::GitStatusPushReceived {
                    repo_path,
                    metadata,
                })
            }
            server_message::Message::GithubPrInfoPush(push) => {
                let Some(repo_path) = StandardizedPath::try_new(&push.repo_path).ok() else {
                    log::warn!("GitHubPrInfoPush: invalid repo_path: {}", push.repo_path);
                    return None;
                };
                Some(ClientEvent::GitHubPrInfoPushReceived {
                    repo_path,
                    pr_info: push.pr_info,
                })
            }
            server_message::Message::GithubRepositoryInfoPush(push) => {
                let Some(repo_path) = StandardizedPath::try_new(&push.repo_path).ok() else {
                    log::warn!(
                        "GitHubRepositoryInfoPush: invalid repo_path: {}",
                        push.repo_path
                    );
                    return None;
                };
                Some(ClientEvent::GitHubRepositoryInfoPushReceived {
                    repo_path,
                    repository_info: push.repository_info,
                })
            }
            other => {
                safe_warn!(
                    safe: ("Unhandled push message variant"),
                    full: ("Unhandled push message variant: {other:?}")
                );
                None
            }
        }
    }

    /// Sends an `UnsubscribeDiffState` notification (fire-and-forget).
    pub fn unsubscribe_diff_state(&self, repo_path: &StandardizedPath, mode: DiffMode) {
        let msg = ClientMessage::notification(notification::Message::UnsubscribeDiffState(
            UnsubscribeDiffState {
                repo_path: repo_path.to_string(),
                mode: Some(mode),
            },
        ));
        self.send_notification(msg);
    }

    /// Sends an `UpdateGitStatus` notification (fire-and-forget).
    pub fn update_git_status(&self, repo_path: &StandardizedPath) {
        let msg =
            ClientMessage::notification(notification::Message::UpdateGitStatus(UpdateGitStatus {
                repo_path: repo_path.to_string(),
            }));
        self.send_notification(msg);
    }

    /// Sends an `UpdateGitHubPrInfo` notification (fire-and-forget). The daemon
    /// refreshes PR info and broadcasts the result as a `GitHubPrInfoPush`.
    pub fn update_github_pr_info(&self, repo_path: &StandardizedPath) {
        let msg = ClientMessage::notification(notification::Message::UpdateGithubPrInfo(
            UpdateGitHubPrInfo {
                repo_path: repo_path.to_string(),
            },
        ));
        self.send_notification(msg);
    }

    /// Sends an `UpdateGitHubRepoInfo` notification (fire-and-forget). The
    /// daemon refreshes repository info and broadcasts the result as a
    /// `GitHubRepositoryInfoPush`.
    pub fn update_github_repo_info(&self, repo_path: &StandardizedPath) {
        let msg = ClientMessage::notification(notification::Message::UpdateGithubRepoInfo(
            UpdateGitHubRepoInfo {
                repo_path: repo_path.to_string(),
            },
        ));
        self.send_notification(msg);
    }

    /// Sends a `RunCommand` request.
    pub async fn run_command(
        &self,
        session_id: SessionId,
        command: String,
        working_directory: Option<String>,
        environment_variables: HashMap<String, String>,
    ) -> Result<RunCommandResponse, ClientError> {
        let request_id = RequestId::new();
        let msg = ClientMessage::session_scoped(
            request_id.to_string(),
            session_scoped_request::Message::RunCommand(RunCommandRequest {
                command,
                working_directory,
                environment_variables,
                session_id: session_id.as_u64(),
            }),
        );

        let response = self
            .send_request(
                request_id,
                msg,
                crate::manager::RemoteServerOperation::RunCommand,
            )
            .await?;

        match response.message {
            Some(server_message::Message::RunCommandResponse(resp)) => Ok(resp),
            other => {
                safe_error!(
                    safe: ("Remote server unexpected response for RunCommand"),
                    full: ("Remote server unexpected response for RunCommand: response={other:?}")
                );
                Err(ClientError::UnexpectedResponse)
            }
        }
    }

    /// Wrapper around [`send_request_internal`] that automatically fires a
    /// [`ClientEvent::RequestFailed`] event on error, so transport-level
    /// failures are tracked for telemetry without requiring each caller
    /// to instrument its own error path.
    async fn send_request(
        &self,
        request_id: RequestId,
        msg: ClientMessage,
        operation: crate::manager::RemoteServerOperation,
    ) -> Result<ServerMessage, ClientError> {
        let result = self.send_request_internal(request_id, msg).await;
        if let Err(ref e) = result {
            let error_kind = crate::manager::RemoteServerErrorKind::from_client_error(e);
            let _ = self.failure_tx.try_send(RequestFailedEvent {
                operation,
                error_kind,
            });
        }
        result
    }

    /// Generic request/response correlation.
    ///
    /// Registers a oneshot channel keyed by `request_id`, sends the message
    /// through the outbound channel, and awaits the correlated response.
    /// Times out after `REQUEST_TIMEOUT` and sends an `Abort` to the server.
    async fn send_request_internal(
        &self,
        request_id: RequestId,
        msg: ClientMessage,
    ) -> Result<ServerMessage, ClientError> {
        let (tx, rx) = oneshot::channel();
        self.pending_requests.insert(request_id.clone(), tx);

        // Check if the reader task has already marked the connection as dead.
        // The DashMap lock from `insert` above synchronizes with the lock from
        // `clear` in `reader_task`, so if `clear` ran before our insert the
        // flag is guaranteed to be visible here.
        if self.disconnected.load(Ordering::Acquire) {
            self.pending_requests.clear();
            return Err(ClientError::Disconnected);
        }

        if self.outbound_tx.send(msg).await.is_err() {
            self.pending_requests.remove(&request_id);
            return Err(ClientError::Disconnected);
        }

        let result = match rx.with_timeout(REQUEST_TIMEOUT).await {
            Ok(Ok(inner)) => inner,
            Ok(Err(_)) => return Err(ClientError::ResponseChannelClosed),
            Err(_) => {
                // Timed out — clean up and send abort.
                self.pending_requests.remove(&request_id);
                self.send_abort(&request_id);
                return Err(ClientError::Timeout(REQUEST_TIMEOUT));
            }
        };

        // Unwrap the inner Result (reader task may send Err for decode failures).
        let response = result?;

        // Convert server-reported ErrorResponse into ClientError so callers
        // only need to match on success variants.
        if let Some(server_message::Message::Error(ref e)) = response.message {
            return Err(ClientError::ServerError {
                code: e.code(),
                message: e.message.clone(),
            });
        }

        Ok(response)
    }

    /// Sends an `Abort` notification for the given request ID.
    fn send_abort(&self, request_id_to_abort: &RequestId) {
        let msg = ClientMessage::notification(notification::Message::Abort(Abort {
            request_id_to_abort: request_id_to_abort.to_string(),
        }));
        self.send_notification(msg);
    }

    /// Sends an `Abort` notification for a host-scoped request.
    ///
    /// Used by the `RemoteServerManager` when a host-scoped request times out:
    /// the manager owns the lifecycle of host-scoped requests, so it (not the
    /// client) decides when to abort one. Fire-and-forget, like all
    /// notifications.
    pub fn abort_request(&self, request_id_to_abort: &RequestId) {
        self.send_abort(request_id_to_abort);
    }

    /// Sends a message without registering a pending request (fire-and-forget).
    fn send_notification(&self, msg: ClientMessage) {
        // Use try_send to avoid blocking; if the channel is full or closed,
        // the notification is best-effort.
        if let Err(e) = self.outbound_tx.try_send(msg) {
            log::debug!("Failed to send notification (best-effort): {e}");
        }
    }

    /// Sends a host-scoped request without registering it in `pending_requests`.
    ///
    /// The response lifecycle for host-scoped requests is owned by the
    /// `RemoteServerManager`, not this client. The daemon may deliver the
    /// response on a different connection if this one disconnects. The
    /// `reader_task` forwards unmatched responses to `host_response_tx`
    /// so the manager can match them.
    ///
    /// Returns `Err(ClientError::Disconnected)` if the outbound channel is
    /// closed (the writer task has exited after a fatal connection error).
    /// The manager relies on this to fail the caller and avoid leaking a
    /// pending request that could never be matched. Logging is left to the
    /// manager, which has the host and request-id context.
    pub fn send_host_scoped(&self, msg: ClientMessage) -> Result<(), ClientError> {
        self.outbound_tx
            .try_send(msg)
            .map_err(|_| ClientError::Disconnected)
    }

    /// Background task that writes `ClientMessage`s to the underlying stream.
    async fn writer_task(
        writer: impl AsyncWrite + TransportStream,
        outbound_rx: async_channel::Receiver<ClientMessage>,
        pending_requests: Arc<
            DashMap<RequestId, oneshot::Sender<Result<ServerMessage, ClientError>>>,
        >,
        event_tx: async_channel::Sender<ClientEvent>,
    ) {
        let mut writer = futures::io::BufWriter::new(writer);
        while let Ok(msg) = outbound_rx.recv().await {
            let request_id = RequestId::from(msg.request_id.clone());
            let is_host_scoped = matches!(
                &msg.message,
                Some(crate::proto::client_message::Message::HostScoped(_))
            );
            if let Err(e) = protocol::write_client_message(&mut writer, &msg).await {
                if is_host_scoped
                    && event_tx
                        .try_send(ClientEvent::HostScopedWriteFailed {
                            request_id: request_id.clone(),
                        })
                        .is_err()
                {
                    log::warn!(
                        "Failed to notify manager about host-scoped write failure: request_id={request_id}"
                    );
                }
                if !e.is_write_recoverable() {
                    report_error!(
                        anyhow::Error::new(e).context("Writer task fatal error"),
                        extra: { "request_id" => %request_id }
                    );
                    pending_requests.clear();
                    break;
                }
                log::warn!("Remote server writer task error: request_id={request_id} error={e}");
                // Drop the sender so the caller receives ResponseChannelClosed.
                pending_requests.remove(&request_id);
            }
        }
    }

    /// Background task that reads `ServerMessage`s and resolves pending
    /// requests by `request_id`, or converts push messages to events.
    ///
    /// Sends `ClientEvent::Disconnected` as the final event when the
    /// connection is lost.
    async fn reader_task(
        reader: impl AsyncRead + TransportStream,
        pending_requests: Arc<
            DashMap<RequestId, oneshot::Sender<Result<ServerMessage, ClientError>>>,
        >,
        event_tx: async_channel::Sender<ClientEvent>,
        disconnected: Arc<AtomicBool>,
        host_response_tx: async_channel::Sender<ServerMessage>,
    ) {
        let mut reader = futures::io::BufReader::new(reader);
        loop {
            match protocol::read_server_message(&mut reader).await {
                Ok(msg) => {
                    let request_id = RequestId::from(msg.request_id.clone());
                    if request_id.is_empty() {
                        // Push message — convert to a domain event and forward.
                        if let Some(event) = Self::push_message_to_event(msg)
                            && event_tx.send(event).await.is_err()
                        {
                            log::warn!("Event channel closed, dropping push message");
                        }
                    } else if let Some((_, tx)) = pending_requests.remove(&request_id) {
                        // Session-scoped response — resolve the caller's oneshot.
                        let _ = tx.send(Ok(msg));
                    } else {
                        // Host-scoped response (either normal path or daemon
                        // failover). Forward to the manager for matching.
                        if host_response_tx.try_send(msg).is_err() {
                            log::warn!(
                                "Host response channel closed, dropping response \
                                 with request_id={request_id}"
                            );
                        }
                    }
                }
                Err(ProtocolError::Decode(ref err, Some(ref request_id))) => {
                    if let Some((_, tx)) = pending_requests.remove(request_id) {
                        log::warn!(
                            "Reader task: malformed response \
                             (request_id={request_id}): {err}"
                        );
                        let _ = tx.send(Err(ClientError::Protocol(ProtocolError::Decode(
                            err.clone(),
                            Some(request_id.clone()),
                        ))));
                    } else {
                        // Not a session-scoped pending request — this is a
                        // host-scoped response the manager is tracking. Tell
                        // it so the caller fails fast rather than waiting for
                        // the request timeout.
                        log::warn!(
                            "Reader task: malformed host-scoped response \
                             (request_id={request_id}): {err}"
                        );
                        let _ = event_tx
                            .send(ClientEvent::HostScopedDecodeFailed {
                                request_id: request_id.clone(),
                            })
                            .await;
                    }
                }
                Err(ProtocolError::Decode(ref err, None)) => {
                    log::warn!(
                        "Reader task: skipping malformed response \
                         (no parseable request_id): {err}"
                    );
                    let _ = event_tx.send(ClientEvent::MessageDecodingError).await;
                }
                Err(e) if e.is_read_recoverable() => {
                    log::warn!("Reader task: skipping message: {e}");
                }
                Err(e) => {
                    match e {
                        ProtocolError::UnexpectedEof => {
                            log::info!("Reader task: server disconnected (EOF)");
                        }
                        _ => {
                            report_error!(anyhow::Error::new(e).context("Reader task fatal error"))
                        }
                    }
                    break;
                }
            }
        }

        // Mark the connection as dead so that any new `send_request` calls
        // fail immediately rather than hanging forever. This prevents a race
        // where `pending_requests.clear()` runs before `send_request` has
        // inserted its oneshot entry.
        disconnected.store(true, Ordering::Release);

        // Notify all pending requests that the connection is gone.
        pending_requests.clear();

        // Signal disconnection as the final event.
        let _ = event_tx.send(ClientEvent::Disconnected).await;
    }
}

/// Spawns a background task that reads lines from the server's stderr,
/// forwards them to the client's logging, and retains the last few lines
/// in a shared buffer for telemetry.
#[cfg(not(target_family = "wasm"))]
pub fn spawn_stderr_forwarder(
    stderr: impl AsyncRead + TransportStream,
    executor: &executor::Background,
) -> RemoteServerLog {
    use futures::StreamExt;
    use futures::io::AsyncBufReadExt;

    let tail = RemoteServerLog::new();
    let tail_writer = tail.clone();

    executor
        .spawn(async move {
            let reader = futures::io::BufReader::new(stderr);
            let mut lines = reader.lines();
            while let Some(Ok(line)) = lines.next().await {
                log::info!("[remote_server] {line}");
                tail_writer.push(line);
            }
        })
        .detach();

    tail
}

#[cfg(test)]
#[path = "../client_tests.rs"]
mod tests;
