use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::codebase_index_proto::{
    proto_to_codebase_index_status_updated, proto_to_codebase_index_statuses_snapshot,
    RemoteCodebaseIndexStatus,
};
use dashmap::DashMap;
use futures::channel::oneshot;
use futures::io::{AsyncRead, AsyncWrite};
use warpui::r#async::{executor, FutureExt as _};

use crate::proto::{
    client_message, get_branches_response, get_fragment_metadata_from_hash_response,
    server_message, Abort, Authenticate, BranchInfo, BufferEdit, ClientMessage, CloseBuffer,
    DeleteFile, DiffMode, DiffStateFileDelta, DiffStateMetadataUpdate, DiffStateSnapshot,
    DiscardFilesRequest, DropCodebaseIndex, ErrorCode, FileStatusInfo,
    FragmentMetadataLookupErrorCode, GetBranches, GetDiffState, GetDiffStateResponse,
    GetFragmentMetadataFromHash, GetFragmentMetadataFromHashSuccess, IndexCodebase, Initialize,
    InitializeResponse, LoadRepoMetadataDirectoryResponse, NavigatedToDirectoryResponse,
    OpenBuffer, OpenBufferResponse, ReadFileContextRequest, ReadFileContextResponse,
    ResyncCodebase, RunCommandRequest, RunCommandResponse, SaveBuffer, ServerMessage,
    SessionBootstrapped, TextEdit, UnsubscribeDiffState, WriteFile,
};
use crate::repo_metadata_proto::{proto_snapshot_to_update, proto_to_repo_metadata_update};

use crate::protocol::{self, ProtocolError, RequestId};
use warp_core::{safe_error, safe_warn, SessionId};
use warp_util::standardized_path::StandardizedPath;
use warpui::r#async::TransportStream;

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

    #[error("File operation failed: {0}")]
    FileOperationFailed(String),

    #[error("Fragment metadata lookup failed ({code:?}): {message}")]
    FragmentMetadataLookup {
        code: FragmentMetadataLookupErrorCode,
        message: String,
    },

    #[error("Discard files failed: {0}")]
    DiscardFailed(String),
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
}

/// Parameters for the `Initialize` handshake, sent to the daemon at
/// connection time.
pub struct InitializeParams {
    pub user_id: String,
    pub user_email: String,
    pub crash_reporting_enabled: bool,
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
    ) {
        spawn_stderr_forwarder(stderr, executor);
        Self::new(stdout, stdin, executor)
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
    ) {
        let pending_requests: Arc<
            DashMap<RequestId, oneshot::Sender<Result<ServerMessage, ClientError>>>,
        > = Arc::new(DashMap::new());
        let (outbound_tx, outbound_rx) = async_channel::unbounded::<ClientMessage>();
        let (event_tx, event_rx) = async_channel::unbounded::<ClientEvent>();
        let (failure_tx, failure_rx) = async_channel::unbounded::<RequestFailedEvent>();
        let disconnected = Arc::new(AtomicBool::new(false));

        executor
            .spawn(Self::writer_task(
                writer,
                outbound_rx,
                Arc::clone(&pending_requests),
            ))
            .detach();
        executor
            .spawn(Self::reader_task(
                reader,
                Arc::clone(&pending_requests),
                event_tx,
                Arc::clone(&disconnected),
            ))
            .detach();

        (
            Self {
                outbound_tx,
                pending_requests,
                disconnected,
                failure_tx,
            },
            event_rx,
            failure_rx,
        )
    }

    /// Sends an `Initialize` request and awaits the `InitializeResponse`.
    pub async fn initialize(
        &self,
        auth_token: Option<&str>,
        params: InitializeParams,
    ) -> Result<InitializeResponse, ClientError> {
        let request_id = RequestId::new();
        let msg = ClientMessage {
            request_id: request_id.to_string(),
            message: Some(client_message::Message::Initialize(Initialize {
                auth_token: auth_token.unwrap_or_default().to_owned(),
                user_id: params.user_id,
                user_email: params.user_email,
                crash_reporting_enabled: params.crash_reporting_enabled,
            })),
        };

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
    /// Sends an `IndexCodebase` request and awaits the initial status update.
    pub async fn index_codebase(
        &self,
        repo_path: String,
        auth_token: String,
    ) -> Result<RemoteCodebaseIndexStatus, ClientError> {
        let request_id = RequestId::new();
        let repo_path_for_log = repo_path.clone();
        let msg = ClientMessage {
            request_id: request_id.to_string(),
            message: Some(client_message::Message::IndexCodebase(IndexCodebase {
                repo_path,
                auth_token,
            })),
        };
        log::info!(
            "[Remote codebase indexing] Client sending IndexCodebase: request_id={request_id} \
             repo_path={repo_path_for_log}"
        );

        let response = self
            .send_request(
                request_id,
                msg,
                crate::manager::RemoteServerOperation::IndexCodebase,
            )
            .await?;
        Self::codebase_index_status_from_response("IndexCodebase", response)
    }

    /// Sends a `ResyncCodebase` request and awaits the resulting status update.
    pub async fn resync_codebase(
        &self,
        repo_path: String,
        auth_token: String,
    ) -> Result<RemoteCodebaseIndexStatus, ClientError> {
        let request_id = RequestId::new();
        let repo_path_for_log = repo_path.clone();
        let msg = ClientMessage {
            request_id: request_id.to_string(),
            message: Some(client_message::Message::ResyncCodebase(ResyncCodebase {
                repo_path,
                auth_token,
            })),
        };
        log::info!(
            "[Remote codebase indexing] Client sending ResyncCodebase: request_id={request_id} \
             repo_path={repo_path_for_log}"
        );

        let response = self
            .send_request(
                request_id,
                msg,
                crate::manager::RemoteServerOperation::ResyncCodebase,
            )
            .await?;
        Self::codebase_index_status_from_response("ResyncCodebase", response)
    }

    fn codebase_index_status_from_response(
        operation: &str,
        response: ServerMessage,
    ) -> Result<RemoteCodebaseIndexStatus, ClientError> {
        match response.message {
            Some(server_message::Message::CodebaseIndexStatusUpdated(update)) => {
                let status =
                    crate::codebase_index_proto::proto_to_codebase_index_status_updated(&update)
                        .ok_or(ClientError::UnexpectedResponse)?;
                log::info!(
                    "[Remote codebase indexing] Client received {operation} response: \
                     repo_path={} state={:?}",
                    status.repo_path,
                    status.state
                );
                Ok(status)
            }
            other => {
                log::error!("Unexpected response variant for {operation}: {other:?}");
                Err(ClientError::UnexpectedResponse)
            }
        }
    }

    /// Sends a `DropCodebaseIndex` request and awaits the resulting status update.
    pub async fn drop_codebase_index(
        &self,
        repo_path: String,
        auth_token: String,
    ) -> Result<RemoteCodebaseIndexStatus, ClientError> {
        let request_id = RequestId::new();
        let repo_path_for_log = repo_path.clone();
        let msg = ClientMessage {
            request_id: request_id.to_string(),
            message: Some(client_message::Message::DropCodebaseIndex(
                DropCodebaseIndex {
                    repo_path,
                    auth_token,
                },
            )),
        };
        log::info!(
            "[Remote codebase indexing] Client sending DropCodebaseIndex: request_id={request_id} \
             repo_path={repo_path_for_log}"
        );

        let response = self
            .send_request(
                request_id,
                msg,
                crate::manager::RemoteServerOperation::DropCodebaseIndex,
            )
            .await?;

        Self::codebase_index_status_from_response("DropCodebaseIndex", response)
    }

    /// Maps backend content hashes to server-local fragment metadata for a synced repo snapshot.
    pub async fn get_fragment_metadata_from_hash(
        &self,
        repo_path: String,
        root_hash: String,
        content_hashes: Vec<String>,
    ) -> Result<GetFragmentMetadataFromHashSuccess, ClientError> {
        let request_id = RequestId::new();
        let msg = ClientMessage {
            request_id: request_id.to_string(),
            message: Some(client_message::Message::GetFragmentMetadataFromHash(
                GetFragmentMetadataFromHash {
                    repo_path,
                    root_hash,
                    content_hashes,
                },
            )),
        };

        let response = self
            .send_request(
                request_id,
                msg,
                crate::manager::RemoteServerOperation::GetFragmentMetadataFromHash,
            )
            .await?;

        match response.message {
            Some(server_message::Message::GetFragmentMetadataFromHashResponse(resp)) => {
                match resp.result {
                    Some(get_fragment_metadata_from_hash_response::Result::Success(success)) => {
                        Ok(success)
                    }
                    Some(get_fragment_metadata_from_hash_response::Result::Error(error)) => {
                        let code = FragmentMetadataLookupErrorCode::try_from(error.code)
                            .unwrap_or(FragmentMetadataLookupErrorCode::Unspecified);
                        Err(ClientError::FragmentMetadataLookup {
                            code,
                            message: error.message,
                        })
                    }
                    None => Err(ClientError::UnexpectedResponse),
                }
            }
            other => {
                log::error!(
                    "Unexpected response variant for GetFragmentMetadataFromHash: {other:?}"
                );
                Err(ClientError::UnexpectedResponse)
            }
        }
    }

    /// Sends an `Authenticate` notification to rotate the daemon-wide
    /// credential after initialization.
    pub fn authenticate(&self, auth_token: &str) {
        let msg = ClientMessage {
            request_id: String::new(),
            message: Some(client_message::Message::Authenticate(Authenticate {
                auth_token: auth_token.to_owned(),
            })),
        };
        self.send_notification(msg);
    }

    /// Sends an `UpdatePreferences` notification when the user's privacy
    /// settings change (e.g. toggling crash reporting).
    pub fn update_preferences(&self, crash_reporting_enabled: bool) {
        let msg = ClientMessage {
            request_id: String::new(),
            message: Some(client_message::Message::UpdatePreferences(
                crate::proto::UpdatePreferences {
                    crash_reporting_enabled,
                },
            )),
        };
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
        let msg = ClientMessage {
            request_id: String::new(),
            message: Some(client_message::Message::SessionBootstrapped(
                SessionBootstrapped {
                    session_id: session_id.as_u64(),
                    shell_type: shell_type.to_owned(),
                    shell_path: shell_path.map(ToOwned::to_owned),
                },
            )),
        };
        self.send_notification(msg);
    }

    /// Sends a `NavigatedToDirectory` request and awaits the response.
    pub async fn navigate_to_directory(
        &self,
        path: String,
    ) -> Result<NavigatedToDirectoryResponse, ClientError> {
        let request_id = RequestId::new();
        let msg = ClientMessage {
            request_id: request_id.to_string(),
            message: Some(client_message::Message::NavigatedToDirectory(
                crate::proto::NavigatedToDirectory { path },
            )),
        };

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
        let msg = ClientMessage {
            request_id: request_id.to_string(),
            message: Some(client_message::Message::LoadRepoMetadataDirectory(
                crate::proto::LoadRepoMetadataDirectory {
                    repo_path,
                    dir_path,
                },
            )),
        };

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

    /// Writes content to a file on the remote host.
    /// Creates parent directories if they don't exist.
    pub async fn write_file(&self, path: String, content: String) -> Result<(), ClientError> {
        let request_id = RequestId::new();
        let msg = ClientMessage {
            request_id: request_id.to_string(),
            message: Some(client_message::Message::WriteFile(WriteFile {
                path,
                content,
            })),
        };
        let response = self
            .send_request(
                request_id,
                msg,
                crate::manager::RemoteServerOperation::WriteFile,
            )
            .await?;
        match response.message {
            Some(server_message::Message::WriteFileResponse(resp)) => match resp.result {
                Some(crate::proto::write_file_response::Result::Success(_)) | None => Ok(()),
                Some(crate::proto::write_file_response::Result::Error(e)) => {
                    Err(ClientError::FileOperationFailed(e.message))
                }
            },
            other => {
                safe_error!(
                    safe: ("Remote server unexpected response for WriteFile"),
                    full: ("Remote server unexpected response for WriteFile: response={other:?}")
                );
                Err(ClientError::UnexpectedResponse)
            }
        }
    }

    /// Batch-reads one or more files from the remote host with full context
    /// (line ranges, binary/image support, metadata, size limits).
    ///
    /// Per-file failures are reported in `ReadFileContextResponse::failed_files`
    /// rather than as a top-level error. The method only returns `Err` for
    /// transport-level failures (disconnect, timeout, etc.).
    pub async fn read_file_context(
        &self,
        request: ReadFileContextRequest,
    ) -> Result<ReadFileContextResponse, ClientError> {
        let request_id = RequestId::new();
        let msg = ClientMessage {
            request_id: request_id.to_string(),
            message: Some(client_message::Message::ReadFileContext(request)),
        };
        let response = self
            .send_request(
                request_id,
                msg,
                crate::manager::RemoteServerOperation::ReadFileContext,
            )
            .await?;
        match response.message {
            Some(server_message::Message::ReadFileContextResponse(resp)) => Ok(resp),
            other => {
                safe_error!(
                    safe: ("Remote server unexpected response for ReadFileContext"),
                    full: ("Remote server unexpected response for ReadFileContext: response={other:?}")
                );
                Err(ClientError::UnexpectedResponse)
            }
        }
    }

    /// Opens a buffer on the remote host for bidirectional syncing.
    ///
    /// When `force_reload` is true, the server discards any in-memory buffer
    /// state and re-reads the file from disk. Used to resolve conflicts.
    pub async fn open_buffer(
        &self,
        path: String,
        force_reload: bool,
    ) -> Result<OpenBufferResponse, ClientError> {
        let request_id = RequestId::new();
        let msg = ClientMessage {
            request_id: request_id.to_string(),
            message: Some(client_message::Message::OpenBuffer(OpenBuffer {
                path,
                force_reload,
            })),
        };
        let response = self
            .send_request(
                request_id,
                msg,
                crate::manager::RemoteServerOperation::OpenBuffer,
            )
            .await?;
        match response.message {
            Some(server_message::Message::OpenBufferResponse(resp)) => Ok(resp),
            other => {
                log::error!("Unexpected response variant for OpenBuffer: {other:?}");
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
        let msg = ClientMessage {
            request_id: String::new(), // notification — no response expected
            message: Some(client_message::Message::BufferEdit(BufferEdit {
                path,
                expected_server_version,
                new_client_version,
                edits,
            })),
        };
        self.send_notification(msg);
    }

    /// Saves a buffer on the remote host to disk.
    pub async fn save_buffer(&self, path: String) -> Result<(), ClientError> {
        let request_id = RequestId::new();
        let msg = ClientMessage {
            request_id: request_id.to_string(),
            message: Some(client_message::Message::SaveBuffer(SaveBuffer { path })),
        };
        let response = self
            .send_request(
                request_id,
                msg,
                crate::manager::RemoteServerOperation::SaveBuffer,
            )
            .await?;
        match response.message {
            Some(server_message::Message::SaveBufferResponse(resp)) => match resp.result {
                Some(crate::proto::save_buffer_response::Result::Success(_)) | None => Ok(()),
                Some(crate::proto::save_buffer_response::Result::Error(e)) => {
                    Err(ClientError::FileOperationFailed(e.message))
                }
            },
            other => {
                log::error!("Unexpected response variant for SaveBuffer: {other:?}");
                Err(ClientError::UnexpectedResponse)
            }
        }
    }

    /// Tells the remote host to close a buffer (stop watching).
    pub fn close_buffer(&self, path: String) {
        let msg = ClientMessage {
            request_id: String::new(),
            message: Some(client_message::Message::CloseBuffer(CloseBuffer { path })),
        };
        self.send_notification(msg);
    }

    /// Deletes a file on the remote host.
    pub async fn delete_file(&self, path: String) -> Result<(), ClientError> {
        let request_id = RequestId::new();
        let msg = ClientMessage {
            request_id: request_id.to_string(),
            message: Some(client_message::Message::DeleteFile(DeleteFile { path })),
        };
        let response = self
            .send_request(
                request_id,
                msg,
                crate::manager::RemoteServerOperation::DeleteFile,
            )
            .await?;
        match response.message {
            Some(server_message::Message::DeleteFileResponse(resp)) => match resp.result {
                Some(crate::proto::delete_file_response::Result::Success(_)) | None => Ok(()),
                Some(crate::proto::delete_file_response::Result::Error(e)) => {
                    Err(ClientError::FileOperationFailed(e.message))
                }
            },
            other => {
                safe_error!(
                    safe: ("Remote server unexpected response for DeleteFile"),
                    full: ("Remote server unexpected response for DeleteFile: response={other:?}")
                );
                Err(ClientError::UnexpectedResponse)
            }
        }
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
                     repo_path={} state={:?}",
                    status.repo_path,
                    status.state
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
            other => {
                safe_warn!(
                    safe: ("Unhandled push message variant"),
                    full: ("Unhandled push message variant: {other:?}")
                );
                None
            }
        }
    }

    /// Sends a `GetDiffState` request and awaits the response.
    pub async fn get_diff_state(
        &self,
        repo_path: &StandardizedPath,
        mode: DiffMode,
    ) -> Result<GetDiffStateResponse, ClientError> {
        let request_id = RequestId::new();
        let msg = ClientMessage {
            request_id: request_id.to_string(),
            message: Some(client_message::Message::GetDiffState(GetDiffState {
                repo_path: repo_path.to_string(),
                mode: Some(mode),
            })),
        };

        let response = self
            .send_request(
                request_id,
                msg,
                crate::manager::RemoteServerOperation::GetDiffState,
            )
            .await?;

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

    /// Sends an `UnsubscribeDiffState` notification (fire-and-forget).
    pub fn unsubscribe_diff_state(&self, repo_path: &StandardizedPath, mode: DiffMode) {
        let msg = ClientMessage {
            request_id: String::new(),
            message: Some(client_message::Message::UnsubscribeDiffState(
                UnsubscribeDiffState {
                    repo_path: repo_path.to_string(),
                    mode: Some(mode),
                },
            )),
        };
        self.send_notification(msg);
    }

    /// Sends a `GetBranches` request and awaits the response.
    pub async fn get_branches(
        &self,
        repo_path: &StandardizedPath,
        max_branch_count: Option<u32>,
        include_remotes: bool,
    ) -> Result<Vec<BranchInfo>, ClientError> {
        let request_id = RequestId::new();
        let msg = ClientMessage {
            request_id: request_id.to_string(),
            message: Some(client_message::Message::GetBranches(GetBranches {
                repo_path: repo_path.to_string(),
                max_branch_count,
                include_remotes,
            })),
        };

        let response = self
            .send_request(
                request_id,
                msg,
                crate::manager::RemoteServerOperation::GetBranches,
            )
            .await?;

        match response.message {
            Some(server_message::Message::GetBranchesResponse(resp)) => match resp.result {
                Some(get_branches_response::Result::Success(success)) => Ok(success.branches),
                Some(get_branches_response::Result::Error(e)) => Err(ClientError::ServerError {
                    code: ErrorCode::Internal,
                    message: e.message,
                }),
                None => {
                    safe_error!(
                        safe: ("Remote server empty result for GetBranches"),
                        full: ("Remote server empty result for GetBranches")
                    );
                    Err(ClientError::UnexpectedResponse)
                }
            },
            other => {
                safe_error!(
                    safe: ("Remote server unexpected response for GetBranches"),
                    full: ("Remote server unexpected response for GetBranches: response={other:?}")
                );
                Err(ClientError::UnexpectedResponse)
            }
        }
    }

    /// Sends a `DiscardFiles` request and awaits the response.
    pub async fn discard_files(
        &self,
        repo_path: &StandardizedPath,
        files: Vec<FileStatusInfo>,
        should_stash: bool,
        branch_name: Option<String>,
        mode: DiffMode,
    ) -> Result<(), ClientError> {
        let request_id = RequestId::new();
        let msg = ClientMessage {
            request_id: request_id.to_string(),
            message: Some(client_message::Message::DiscardFiles(DiscardFilesRequest {
                repo_path: repo_path.to_string(),
                files,
                should_stash,
                branch_name,
                mode: Some(mode),
            })),
        };
        let response = self
            .send_request(
                request_id,
                msg,
                crate::manager::RemoteServerOperation::DiscardFiles,
            )
            .await?;
        match response.message {
            Some(server_message::Message::DiscardFilesResponse(resp)) => match resp.result {
                Some(crate::proto::discard_files_response::Result::Success(_)) => Ok(()),
                Some(crate::proto::discard_files_response::Result::Error(e)) => {
                    Err(ClientError::DiscardFailed(e.message))
                }
                None => {
                    safe_error!(
                        safe: ("Remote server empty result for DiscardFiles"),
                        full: ("Remote server empty result for DiscardFiles")
                    );
                    Err(ClientError::UnexpectedResponse)
                }
            },
            other => {
                safe_error!(
                    safe: ("Remote server unexpected response for DiscardFiles"),
                    full: ("Remote server unexpected response for DiscardFiles: response={other:?}")
                );
                Err(ClientError::UnexpectedResponse)
            }
        }
    }

    /// Sends a `RunCommand` request
    pub async fn run_command(
        &self,
        session_id: SessionId,
        command: String,
        working_directory: Option<String>,
        environment_variables: HashMap<String, String>,
    ) -> Result<RunCommandResponse, ClientError> {
        let request_id = RequestId::new();
        let msg = ClientMessage {
            request_id: request_id.to_string(),
            message: Some(client_message::Message::RunCommand(RunCommandRequest {
                command,
                working_directory,
                environment_variables,
                session_id: session_id.as_u64(),
            })),
        };

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
        let msg = ClientMessage {
            request_id: RequestId::new().to_string(),
            message: Some(client_message::Message::Abort(Abort {
                request_id_to_abort: request_id_to_abort.to_string(),
            })),
        };
        self.send_notification(msg);
    }

    /// Sends a message without registering a pending request (fire-and-forget).
    fn send_notification(&self, msg: ClientMessage) {
        // Use try_send to avoid blocking; if the channel is full or closed,
        // the notification is best-effort.
        if let Err(e) = self.outbound_tx.try_send(msg) {
            log::debug!("Failed to send notification (best-effort): {e}");
        }
    }

    /// Background task that writes `ClientMessage`s to the underlying stream.
    async fn writer_task(
        writer: impl AsyncWrite + TransportStream,
        outbound_rx: async_channel::Receiver<ClientMessage>,
        pending_requests: Arc<
            DashMap<RequestId, oneshot::Sender<Result<ServerMessage, ClientError>>>,
        >,
    ) {
        let mut writer = futures::io::BufWriter::new(writer);
        while let Ok(msg) = outbound_rx.recv().await {
            if let Err(e) = protocol::write_client_message(&mut writer, &msg).await {
                let request_id = RequestId::from(msg.request_id);
                if !e.is_write_recoverable() {
                    log::error!("Writer task fatal error: request_id={request_id} error={e}");
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
    ) {
        let mut reader = futures::io::BufReader::new(reader);
        loop {
            match protocol::read_server_message(&mut reader).await {
                Ok(msg) => {
                    let request_id = RequestId::from(msg.request_id.clone());
                    if request_id.is_empty() {
                        // Push message — convert to a domain event and forward.
                        if let Some(event) = Self::push_message_to_event(msg) {
                            if event_tx.send(event).await.is_err() {
                                log::warn!("Event channel closed, dropping push message");
                            }
                        }
                    } else if let Some((_, tx)) = pending_requests.remove(&request_id) {
                        // Ignore send failure — the caller may have dropped the receiver.
                        let _ = tx.send(Ok(msg));
                    } else {
                        log::warn!("Received unexpected response with request_id={request_id}");
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
                        log::warn!(
                            "Reader task: malformed response for \
                             unknown request (request_id={request_id}): {err}"
                        );
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
                        _ => log::error!("Reader task fatal error: {e}"),
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

/// Spawns a background task that reads lines from the server's stderr and
/// forwards them to the client's logging.
#[cfg(not(target_family = "wasm"))]
pub fn spawn_stderr_forwarder(
    stderr: impl AsyncRead + TransportStream,
    executor: &executor::Background,
) {
    use futures::io::AsyncBufReadExt;
    use futures::StreamExt;

    executor
        .spawn(async move {
            let reader = futures::io::BufReader::new(stderr);
            let mut lines = reader.lines();
            while let Some(Ok(line)) = lines.next().await {
                log::info!("[remote_server] {line}");
            }
        })
        .detach();
}

#[cfg(test)]
#[path = "../client_tests.rs"]
mod tests;
