use crate::terminal::shell::ShellType;
use remote_server::proto::OpenBufferSuccess;
use repo_metadata::repositories::{DetectedRepositories, RepoDetectionSource};
use repo_metadata::{RepoMetadataEvent, RepoMetadataModel, RepositoryIdentifier};
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use warp_core::channel::ChannelState;
use warp_core::safe_error;
use warp_core::SessionId;
use warp_util::standardized_path::StandardizedPath;
use warpui::platform::TerminationMode;
use warpui::r#async::{Spawnable, SpawnableOutput, SpawnedFutureHandle};
use warpui::{Entity, ModelContext, ModelHandle, SingletonEntity};

use super::codebase_index_status::{
    codebase_index_status_to_proto, disabled_codebase_index_status,
    not_enabled_codebase_index_status, queued_codebase_index_status,
    unavailable_codebase_index_status,
};
use crate::code::global_buffer_model::{GlobalBufferModel, GlobalBufferModelEvent};
use ::ai::index::full_source_code_embedding::manager::{
    CodebaseIndexManager, CodebaseIndexManagerEvent,
    FragmentMetadataLookupError as LocalFragmentMetadataLookupError,
};
use ::ai::index::full_source_code_embedding::{
    ContentHash, FragmentMetadata as LocalFragmentMetadata, NodeHash,
};
use warp_files::{FileModel, FileModelEvent};
use warp_util::content_version::ContentVersion;
use warp_util::file::FileId;

use super::diff_state_proto;
use super::diff_state_tracker::{
    DiffModelKey, DiffStateUpdate, RemoteDiffStateManager, SubscribeOutcome,
};
use super::proto::{
    client_message, delete_file_response, discard_files_response, get_diff_state_response,
    get_fragment_metadata_from_hash_response, resolve_conflict_response, run_command_response,
    save_buffer_response, server_message, write_file_response, Abort, Authenticate, BranchInfo,
    BufferEdit, BufferUpdatedPush, ClientMessage, CloseBuffer, CodebaseIndexStatus,
    CodebaseIndexStatusState, CodebaseIndexStatusUpdated, CodebaseIndexStatusesSnapshot,
    DeleteFile, DeleteFileResponse, DeleteFileSuccess, DiscardFilesError, DiscardFilesResponse,
    DiscardFilesSuccess, DropCodebaseIndex, ErrorCode, ErrorResponse, FailedFileRead,
    FileContextProto, FileOperationError, FragmentMetadata as ProtoFragmentMetadata,
    FragmentMetadataLookupError as ProtoFragmentMetadataLookupError,
    FragmentMetadataLookupErrorCode, GetBranchesError, GetBranchesResponse, GetBranchesSuccess,
    GetDiffStateResponse, GetFragmentMetadataFromHash, GetFragmentMetadataFromHashResponse,
    GetFragmentMetadataFromHashSuccess, IndexCodebase, Initialize, InitializeResponse,
    MissingFragmentMetadata, NavigatedToDirectory, NavigatedToDirectoryResponse, OpenBuffer,
    OpenBufferResponse, ReadFileContextResponse, ResolveConflict, ResolveConflictResponse,
    ResolveConflictSuccess, ResyncCodebase, RunCommandError, RunCommandErrorCode,
    RunCommandRequest, RunCommandResponse, RunCommandSuccess, SaveBuffer, SaveBufferResponse,
    SaveBufferSuccess, ServerMessage, SessionBootstrapped, TextEdit, WriteFile, WriteFileResponse,
    WriteFileSuccess,
};
use super::server_buffer_tracker::{PendingBufferRequestKind, ServerBufferTracker};

use crate::code_review::diff_state::{DiffMode, FileStatusInfo};

/// How long the daemon waits with no connections before exiting.
pub const GRACE_PERIOD: std::time::Duration = std::time::Duration::from_secs(10 * 60);

/// Server-side cap on the number of branches returned by `GetBranches`.
/// Prevents a client from forcing the daemon to enumerate an arbitrarily
/// large ref list.
const MAX_BRANCH_COUNT_CAP: usize = 500;

/// Unique identifier for a connected proxy session in daemon mode.
pub type ConnectionId = uuid::Uuid;
use super::protocol::RequestId;
use crate::ai::agent::FileLocations;
use crate::ai::blocklist::{read_local_file_context, ReadFileContextResult};
use crate::auth::auth_state::{AuthState, AuthStateProvider};
use crate::features::FeatureFlag;
use crate::terminal::model::session::command_executor::{
    ExecuteCommandOptions, LocalCommandExecutor,
};

/// Outcome of dispatching a request-style `ClientMessage`.
///
/// Notifications (fire-and-forget messages like `SessionBootstrapped` and
/// `Abort`) do not produce a `HandlerOutcome`; they are dispatched inline in
/// `handle_message` and return early.
#[allow(clippy::large_enum_variant)]
enum HandlerOutcome {
    /// The response is ready synchronously — the caller sends it immediately.
    Sync(server_message::Message),
    /// The handler initiated async work whose response will be sent later.
    ///
    /// When the handle is `Some`, the caller inserts it into `in_progress`
    /// so the request can be cancelled via `Abort`. Removal on
    /// completion/abort is arranged by [`ServerModel::spawn_request_handler`].
    ///
    /// `None` is used for async work whose completion is delivered through
    /// a separate event subscription and is not currently cancellable via
    /// `Abort` (e.g. `FileModel` events for file writes and deletes, which
    /// are tracked by `FileId` in `pending_file_ops` rather than by
    /// `RequestId` in `in_progress`).
    Async(Option<SpawnedFutureHandle>),
}

struct CodebaseIndexRequest {
    repo_path: PathBuf,
}
struct CodebaseIndexRequestParams<'a> {
    operation_name: &'a str,
    repo_path: String,
    auth_token: String,
    auth_operation: &'a str,
    path_kind: CodebaseIndexRequestPathKind,
}

#[derive(Clone, Copy)]
enum CodebaseIndexRequestPathKind {
    Canonicalized,
    Requested,
}

/// Tracks an in-flight file write or delete so the async completion
/// event can be correlated back to the originating client request.
enum FileOpKind {
    Write,
    Delete,
}

struct PendingFileOp {
    request_id: RequestId,
    conn_id: ConnectionId,
    kind: FileOpKind,
}

/// Manages pending file operations and ensures that the corresponding
/// `FileModel` entry is always cleaned up when an operation completes
/// or fails, preventing `FileState` leaks.
struct PendingFileOps {
    ops: HashMap<FileId, PendingFileOp>,
}

impl PendingFileOps {
    fn new() -> Self {
        Self {
            ops: HashMap::new(),
        }
    }

    /// Registers a file path with `FileModel`, sets the initial version,
    /// and tracks the pending operation. Returns the `FileId` and
    /// `ContentVersion` for the caller to initiate the actual I/O.
    fn insert(
        &mut self,
        path: &Path,
        request_id: RequestId,
        conn_id: ConnectionId,
        kind: FileOpKind,
        ctx: &mut ModelContext<ServerModel>,
    ) -> (FileId, ContentVersion) {
        let file_model = FileModel::handle(ctx);
        let file_id = file_model.update(ctx, |m, ctx| m.register_file_path(path, false, ctx));
        let version = ContentVersion::new();
        file_model.update(ctx, |m, _| m.set_version(file_id, version));
        self.ops.insert(
            file_id,
            PendingFileOp {
                request_id,
                conn_id,
                kind,
            },
        );
        (file_id, version)
    }

    fn get(&self, file_id: &FileId) -> Option<&PendingFileOp> {
        self.ops.get(file_id)
    }

    /// Removes a pending operation and unsubscribes the file from `FileModel`,
    /// preventing the `FileState` entry from leaking.
    fn remove(
        &mut self,
        file_id: FileId,
        ctx: &mut ModelContext<ServerModel>,
    ) -> Option<PendingFileOp> {
        let op = self.ops.remove(&file_id)?;
        FileModel::handle(ctx).update(ctx, |m, ctx| m.unsubscribe(file_id, ctx));
        Some(op)
    }
}

/// The top-level server-side orchestrator model.
///
/// Receives `ClientMessage`s from connected proxy sessions and routes
/// `ServerMessage` responses and push notifications back through each
/// connection's dedicated sender channel.
pub struct ServerModel {
    /// Per-connection outbound channels, keyed by `ConnectionId`.
    ///
    /// The daemon can serve multiple proxy connections simultaneously — one
    /// per SSH session / Warp tab connecting to this host.  Each entry maps
    /// a connection's `Uuid` to the channel the connection task drains to
    /// write `ServerMessage`s back to its proxy.
    connection_senders: HashMap<ConnectionId, async_channel::Sender<ServerMessage>>,
    /// Per-connection set of repo roots for which we've already sent a
    /// snapshot in this connection's lifetime.
    ///
    /// Used to avoid sending duplicate snapshots on repeated
    /// `NavigatedToDirectory` calls while the user `cd`s within the same repo.
    snapshot_sent_roots_by_connection: HashMap<ConnectionId, HashSet<StandardizedPath>>,
    /// Abort handle for the active grace timer, if any.
    /// Calling `.abort()` cancels the timer before it fires.
    grace_timer_cancel: Option<SpawnedFutureHandle>,
    /// Tracks in-progress requests that can be cancelled via `Abort`.
    /// Calling `.abort()` on the handle cancels the background future and
    /// triggers its `on_abort` callback.
    in_progress: HashMap<RequestId, SpawnedFutureHandle>,
    /// Stable host identifier generated once at process startup.
    /// Returned in every `InitializeResponse` so clients can deduplicate
    /// host-scoped models.
    host_id: String,
    /// Per-session command executors created from `SessionBootstrapped` notifications.
    executors: HashMap<SessionId, Arc<LocalCommandExecutor>>,
    /// Tracks in-flight file write/delete operations and handles cleanup.
    pending_file_ops: PendingFileOps,
    /// Daemon-wide auth credentials and user identity.
    auth_state: Arc<AuthState>,
    /// Tracks open buffers, per-buffer connection sets, and pending async
    /// buffer requests (OpenBuffer, SaveBuffer).
    buffers: ServerBufferTracker,
    /// Manages per-(repo, mode) diff state models and per-connection subscriptions.
    diff_states: ModelHandle<RemoteDiffStateManager>,
}

impl Entity for ServerModel {
    type Event = ();
}

impl SingletonEntity for ServerModel {}

impl ServerModel {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        let host_id = uuid::Uuid::new_v4().to_string();
        log::info!(
            "Daemon started: PID={}, host_id={}",
            std::process::id(),
            host_id
        );
        let mut model = Self {
            connection_senders: HashMap::new(),
            snapshot_sent_roots_by_connection: HashMap::new(),
            grace_timer_cancel: None,
            in_progress: HashMap::new(),
            host_id,
            executors: HashMap::new(),
            pending_file_ops: PendingFileOps::new(),
            auth_state: AuthStateProvider::as_ref(ctx).get().clone(),
            buffers: ServerBufferTracker::new(),
            diff_states: ctx.add_model(|_| RemoteDiffStateManager::new()),
        };
        // Subscribe to FileModel and RepoMetadataModel events
        // file operation results and repo metadata pushes are forwarded to all
        // connected proxy sessions.
        {
            let file_model = FileModel::handle(ctx);
            ctx.subscribe_to_model(&file_model, |me, event, ctx| {
                let file_id = event.file_id();
                let Some(pending_kind) = me.pending_file_ops.get(&file_id).map(|op| &op.kind)
                else {
                    return; // Not a file op we're tracking.
                };
                let response_message = match (event, pending_kind) {
                    (FileModelEvent::FileSaved { .. }, FileOpKind::Write) => {
                        server_message::Message::WriteFileResponse(WriteFileResponse {
                            result: Some(write_file_response::Result::Success(WriteFileSuccess {})),
                        })
                    }
                    (FileModelEvent::FileSaved { .. }, FileOpKind::Delete) => {
                        server_message::Message::DeleteFileResponse(DeleteFileResponse {
                            result: Some(delete_file_response::Result::Success(
                                DeleteFileSuccess {},
                            )),
                        })
                    }
                    (FileModelEvent::FailedToSave { error, .. }, FileOpKind::Write) => {
                        server_message::Message::WriteFileResponse(WriteFileResponse {
                            result: Some(write_file_response::Result::Error(FileOperationError {
                                message: format!("{error}"),
                            })),
                        })
                    }
                    (FileModelEvent::FailedToSave { error, .. }, FileOpKind::Delete) => {
                        server_message::Message::DeleteFileResponse(DeleteFileResponse {
                            result: Some(delete_file_response::Result::Error(FileOperationError {
                                message: format!("{error}"),
                            })),
                        })
                    }
                    (FileModelEvent::FileLoaded { .. }, _)
                    | (FileModelEvent::FailedToLoad { .. }, _)
                    | (FileModelEvent::FileUpdated { .. }, _) => return,
                };
                // Remove the pending op and unsubscribe from FileModel.
                let pending = me
                    .pending_file_ops
                    .remove(file_id, ctx)
                    .expect("pending op was confirmed present");
                me.send_server_message(
                    Some(pending.conn_id),
                    Some(&pending.request_id),
                    response_message,
                );
            });
        }
        {
            let repo_model = RepoMetadataModel::handle(ctx);
            ctx.subscribe_to_model(&repo_model, |me, event, ctx| match event {
                RepoMetadataEvent::IncrementalUpdateReady { update } => {
                    me.send_server_message(
                        None,
                        None,
                        server_message::Message::RepoMetadataUpdate(update.into()),
                    );
                }
                RepoMetadataEvent::RepositoryUpdated {
                    id: RepositoryIdentifier::Local(path),
                } => {
                    // A repo finished indexing — push the full tree as a snapshot.
                    let id = RepositoryIdentifier::local(path.clone());
                    let repo_model = RepoMetadataModel::handle(ctx);
                    if let Some(state) = repo_model.as_ref(ctx).get_repository(&id, ctx) {
                        let entries = super::repo_metadata_proto::file_tree_entry_to_snapshot_proto(
                            &state.entry,
                        );
                        me.send_server_message(
                            None,
                            None,
                            server_message::Message::RepoMetadataSnapshot(
                                super::proto::RepoMetadataSnapshot {
                                    repo_path: path.to_string(),
                                    entries,
                                    sync_complete: true,
                                },
                            ),
                        );
                        // Mark this root as snapshot-sent for all active connections
                        // so subsequent NavigatedToDirectory calls skip re-sending.
                        for sent_roots in me.snapshot_sent_roots_by_connection.values_mut() {
                            sent_roots.insert(path.clone());
                        }
                    }
                }
                RepoMetadataEvent::RepositoryRemoved { .. }
                | RepoMetadataEvent::FileTreeUpdated { .. }
                | RepoMetadataEvent::FileTreeEntryUpdated { .. }
                | RepoMetadataEvent::UpdatingRepositoryFailed { .. }
                | RepoMetadataEvent::RepositoryUpdated {
                    id: RepositoryIdentifier::Remote(_),
                } => {}
            });
        }
        let index_manager = CodebaseIndexManager::handle(ctx);
        ctx.subscribe_to_model(&index_manager, |me, event, ctx| {
            me.handle_codebase_index_manager_event(event, ctx);
        });
        // Subscribe to GlobalBufferModel events for server-local buffers.
        {
            let gbm = GlobalBufferModel::handle(ctx);
            ctx.subscribe_to_model(&gbm, |me, event, ctx| match event {
                GlobalBufferModelEvent::BufferLoaded { file_id, .. } => {
                    // Complete all pending OpenBuffer requests for this file.
                    let pending = me.buffers.take_pending_by_kind(
                        file_id,
                        PendingBufferRequestKind::OpenBuffer,
                    );
                    if !pending.is_empty() {
                        let gbm = GlobalBufferModel::handle(ctx);
                        let content = gbm.as_ref(ctx).content_for_file(*file_id, ctx);
                        let server_version = gbm
                            .as_ref(ctx)
                            .sync_clock_for_server_local(*file_id)
                            .map(|c| c.server_version.as_u64());

                        for req in pending {
                            let message = match (&content, server_version) {
                                (Some(content), Some(sv)) => {
                                    server_message::Message::OpenBufferResponse(OpenBufferResponse{
                                        result: Some(remote_server::proto::open_buffer_response::Result::Success(OpenBufferSuccess {
                                             content: content.clone(),
                                            server_version: sv,
                                        }))
                                    })
                                }
                                _ => server_message::Message::Error(ErrorResponse {
                                    code: ErrorCode::Internal.into(),
                                    message: format!(
                                        "Buffer loaded but content or sync clock unavailable for file {file_id:?}"
                                    ),
                                }),
                            };
                            me.send_server_message(
                                Some(req.connection_id),
                                Some(&req.request_id),
                                message,
                            );
                        }
                    }
                }
                GlobalBufferModelEvent::ServerLocalBufferUpdated {
                    file_id,
                    edits,
                    new_server_version,
                    expected_client_version,
                } => {
                    // Push incremental edits to all connections that have this buffer open,
                    // except connections with a pending OpenBuffer request (they will
                    // receive the content via OpenBufferResponse instead).
                    let Some(conns) = me.buffers.connections_for_buffer(file_id) else {
                        return;
                    };
                    let excluded =
                        me.buffers.pending_connections_for_open_buffer(file_id);
                    // Find the path for this file_id.
                    let path = me.buffers.path_for_file_id(*file_id).unwrap_or_default();

                    let proto_edits: Vec<TextEdit> = edits
                        .iter()
                        .map(|edit| TextEdit {
                            start_offset: edit.start.as_usize() as u64,
                            end_offset: edit.end.as_usize() as u64,
                            text: edit.text.clone(),
                        })
                        .collect();

                    for &conn_id in conns {
                        if excluded.contains(&conn_id) {
                            continue;
                        }
                        me.send_server_message(
                            Some(conn_id),
                            None,
                            server_message::Message::BufferUpdated(BufferUpdatedPush {
                                path: path.clone(),
                                new_server_version: new_server_version.as_u64(),
                                expected_client_version: expected_client_version.as_u64(),
                                edits: proto_edits.clone(),
                            }),
                        );
                    }
                }
                GlobalBufferModelEvent::FileSaved { file_id } => {
                    for req in me.buffers.take_pending_by_kind(
                        file_id,
                        PendingBufferRequestKind::SaveBuffer,
                    ) {
                        me.send_server_message(
                            Some(req.connection_id),
                            Some(&req.request_id),
                            server_message::Message::SaveBufferResponse(SaveBufferResponse {
                                result: Some(save_buffer_response::Result::Success(
                                    SaveBufferSuccess {},
                                )),
                            }),
                        );
                    }
                    for req in me.buffers.take_pending_by_kind(
                        file_id,
                        PendingBufferRequestKind::ResolveConflict,
                    ) {
                        me.send_server_message(
                            Some(req.connection_id),
                            Some(&req.request_id),
                            server_message::Message::ResolveConflictResponse(
                                ResolveConflictResponse {
                                    result: Some(
                                        resolve_conflict_response::Result::Success(
                                            ResolveConflictSuccess {},
                                        ),
                                    ),
                                },
                            ),
                        );
                    }
                }
                GlobalBufferModelEvent::FailedToSave { file_id, error } => {
                    for req in me.buffers.take_pending_by_kind(
                        file_id,
                        PendingBufferRequestKind::SaveBuffer,
                    ) {
                        me.send_server_message(
                            Some(req.connection_id),
                            Some(&req.request_id),
                            server_message::Message::SaveBufferResponse(SaveBufferResponse {
                                result: Some(save_buffer_response::Result::Error(
                                    FileOperationError {
                                        message: format!("{error}"),
                                    },
                                )),
                            }),
                        );
                    }
                    for req in me.buffers.take_pending_by_kind(
                        file_id,
                        PendingBufferRequestKind::ResolveConflict,
                    ) {
                        me.send_server_message(
                            Some(req.connection_id),
                            Some(&req.request_id),
                            server_message::Message::ResolveConflictResponse(
                                ResolveConflictResponse {
                                    result: Some(resolve_conflict_response::Result::Error(
                                        FileOperationError {
                                            message: format!("{error}"),
                                        },
                                    )),
                                },
                            ),
                        );
                    }
                }
                GlobalBufferModelEvent::FailedToLoad { file_id, error } => {
                    for req in me.buffers.take_pending_by_kind(
                        file_id,
                        PendingBufferRequestKind::OpenBuffer,
                    ) {
                        me.send_server_message(
                            Some(req.connection_id),
                            Some(&req.request_id),
                            server_message::Message::OpenBufferResponse(OpenBufferResponse{
                                        result: Some(remote_server::proto::open_buffer_response::Result::Error(FileOperationError {
                                             message: format!("Failed to load buffer: {error}"),
                                        }))
                                    }),
                        );
                    }
                }
                GlobalBufferModelEvent::BufferUpdatedFromFileEvent {
                    file_id,
                    success,
                    ..
                } => {
                    // When a file-watcher update couldn't be applied because
                    // the buffer has unsaved client edits, forward the conflict
                    // to connected clients so they can show a resolution banner.
                    if !success {
                        if let Some(conns) = me.buffers.connections_for_buffer(file_id) {
                            let path = me.buffers.path_for_file_id(*file_id).unwrap_or_default();
                            for &conn_id in conns {
                                me.send_server_message(
                                    Some(conn_id),
                                    None,
                                    server_message::Message::BufferConflictDetected(
                                        super::proto::BufferConflictDetected {
                                            path: path.clone(),
                                        },
                                    ),
                                );
                            }
                        }
                    }
                }
                GlobalBufferModelEvent::RemoteBufferConflict { .. } => {
                    // Not relevant for server-local buffers.
                }
            });
        }
        // Subscribe to diff state manager events — convert domain dispatches
        // to proto messages and send them to connected clients.
        {
            let diff_states = model.diff_states.clone();
            ctx.subscribe_to_model(&diff_states, |me, dispatch, _ctx| {
                me.handle_diff_state_update(dispatch);
            });
        }
        // Start the grace timer immediately so the daemon exits if no proxy
        // connects within GRACE_PERIOD. In practice the spawning proxy connects
        // within milliseconds, so the risk of premature shutdown is negligible;
        // register_connection will cancel the timer the moment the first proxy
        // arrives.
        model.start_grace_timer(ctx);
        model
    }

    /// Called when a proxy connects.  Inserts `conn_tx` into the connection
    /// map so `send_server_message` can route responses to this proxy, and
    /// cancels the grace timer if it was running.
    pub fn register_connection(
        &mut self,
        conn_id: ConnectionId,
        conn_tx: async_channel::Sender<ServerMessage>,
        ctx: &mut ModelContext<Self>,
    ) {
        log::info!(
            "Daemon: connection {conn_id} registered — {} active, host_id={}",
            self.connection_senders.len() + 1,
            self.host_id
        );
        if let Some(handle) = self.grace_timer_cancel.take() {
            handle.abort();
        }
        self.connection_senders.insert(conn_id, conn_tx);
        self.snapshot_sent_roots_by_connection
            .insert(conn_id, HashSet::new());
        ctx.notify();
    }

    /// Called when a proxy disconnects.  Removes it from the connection map
    /// and starts the grace timer if no connections remain.
    pub fn deregister_connection(&mut self, conn_id: ConnectionId, ctx: &mut ModelContext<Self>) {
        self.snapshot_sent_roots_by_connection.remove(&conn_id);
        // Guard against double-deregister (reader and writer tasks both call
        // this on connection close; the second call must be a safe no-op).
        if self.connection_senders.remove(&conn_id).is_none() {
            return;
        }

        // Remove this connection from all buffer connection sets.
        // Orphaned buffers (no connections left) are deallocated automatically.
        self.buffers.remove_connection(conn_id, ctx);

        // Remove this connection from diff state subscriptions.
        // Orphaned models (no subscribers) are dropped automatically.
        self.diff_states
            .update(ctx, |mgr, _| mgr.remove_connection(conn_id));

        let remaining = self.connection_senders.len();
        log::info!("Daemon: connection {conn_id} deregistered — {remaining} active remaining");
        if remaining == 0 {
            log::info!("Daemon: grace timer started ({GRACE_PERIOD:?})");
            self.start_grace_timer(ctx);
        }
        ctx.notify();
    }

    /// Starts (or restarts) a timer that shuts the daemon down after
    /// [`GRACE_PERIOD`] with no connected proxies.  If a timer is already
    /// running its abort handle is cancelled before the new one is stored.
    /// When a proxy connects, `register_connection` aborts the handle,
    /// preventing the shutdown.
    fn start_grace_timer(&mut self, ctx: &mut ModelContext<Self>) {
        if let Some(handle) = self.grace_timer_cancel.take() {
            handle.abort();
        }
        let handle = ctx.spawn_abortable(
            async_io::Timer::after(GRACE_PERIOD),
            |_, _, ctx| {
                log::info!("Daemon: grace period expired, shutting down");
                ctx.terminate_app(TerminationMode::ForceTerminate, None);
            },
            |_, _| {
                log::debug!("Daemon: grace timer cancelled");
            },
        );
        self.grace_timer_cancel = Some(handle);
    }

    /// Called by the background stdin reader task via `ModelSpawner`.
    ///
    /// Dispatches on the `oneof message` variant. Notifications are handled
    /// inline; request-style messages return a `HandlerOutcome` that is
    /// centrally acted on here: `Sync` responses are sent immediately and
    /// `Async` handles are tracked in `in_progress` so they can be aborted.
    pub fn handle_message(
        &mut self,
        conn_id: ConnectionId,
        msg: ClientMessage,
        ctx: &mut ModelContext<Self>,
    ) {
        let request_id = RequestId::from(msg.request_id);

        let outcome = match msg.message {
            Some(client_message::Message::Initialize(msg)) => {
                self.handle_initialize(msg, &request_id, ctx)
            }
            Some(client_message::Message::Authenticate(msg)) => {
                self.handle_authenticate(msg);
                return;
            }
            Some(client_message::Message::UpdatePreferences(msg)) => {
                self.handle_update_preferences(msg, ctx);
                return;
            }
            Some(client_message::Message::SessionBootstrapped(msg)) => {
                self.handle_session_bootstrapped(msg);
                return;
            }
            Some(client_message::Message::Abort(abort)) => {
                self.handle_abort(abort, &request_id, ctx);
                return;
            }
            Some(client_message::Message::RunCommand(req)) => {
                self.handle_run_command(req, &request_id, conn_id, ctx)
            }
            Some(client_message::Message::NavigatedToDirectory(msg)) => {
                self.handle_navigated_to_directory(msg, &request_id, conn_id, ctx)
            }
            Some(client_message::Message::LoadRepoMetadataDirectory(msg)) => {
                self.handle_load_repo_metadata_directory(msg, &request_id, ctx)
            }
            Some(client_message::Message::WriteFile(msg)) => {
                self.handle_write_file(msg, &request_id, conn_id, ctx)
            }
            Some(client_message::Message::DeleteFile(msg)) => {
                self.handle_delete_file(msg, &request_id, conn_id, ctx)
            }
            Some(client_message::Message::ReadFileContext(msg)) => {
                self.handle_read_file_context(msg, &request_id, conn_id, ctx)
            }
            Some(client_message::Message::OpenBuffer(msg)) => {
                self.handle_open_buffer(msg, &request_id, conn_id, ctx)
            }
            Some(client_message::Message::BufferEdit(msg)) => {
                self.handle_buffer_edit(msg, ctx);
                return; // fire-and-forget notification
            }
            Some(client_message::Message::CloseBuffer(msg)) => {
                self.handle_close_buffer(msg, conn_id, ctx);
                return; // fire-and-forget notification
            }
            Some(client_message::Message::SaveBuffer(msg)) => {
                self.handle_save_buffer(msg, &request_id, conn_id, ctx)
            }
            Some(client_message::Message::ResolveConflict(msg)) => {
                self.handle_resolve_conflict(msg, &request_id, conn_id, ctx)
            }
            Some(client_message::Message::GetDiffState(msg)) => {
                self.handle_get_diff_state(msg, &request_id, conn_id, ctx)
            }
            Some(client_message::Message::UnsubscribeDiffState(msg)) => {
                self.handle_unsubscribe_diff_state(msg, conn_id, ctx);
                return; // fire-and-forget notification
            }
            Some(client_message::Message::GetBranches(msg)) => {
                self.handle_get_branches(msg, &request_id, conn_id, ctx)
            }
            Some(client_message::Message::DiscardFiles(msg)) => {
                self.handle_discard_files(msg, &request_id, ctx)
            }
            Some(client_message::Message::IndexCodebase(msg)) => {
                self.handle_index_codebase(msg, &request_id, conn_id, ctx)
            }
            Some(client_message::Message::ResyncCodebase(msg)) => {
                self.handle_resync_codebase(msg, &request_id, conn_id, ctx)
            }
            Some(client_message::Message::DropCodebaseIndex(msg)) => {
                self.handle_drop_codebase_index(msg, &request_id, conn_id, ctx)
            }
            Some(client_message::Message::GetFragmentMetadataFromHash(msg)) => {
                self.handle_get_fragment_metadata_from_hash(msg, &request_id, conn_id, ctx)
            }
            None => {
                log::warn!(
                    "Received ClientMessage with no message variant (request_id={request_id})"
                );
                HandlerOutcome::Sync(server_message::Message::Error(ErrorResponse {
                    code: ErrorCode::InvalidRequest.into(),
                    message: "ClientMessage had no message variant set".to_string(),
                }))
            }
        };

        match outcome {
            HandlerOutcome::Sync(server_message::Message::InitializeResponse(response)) => {
                self.send_server_message(
                    Some(conn_id),
                    Some(&request_id),
                    server_message::Message::InitializeResponse(response),
                );
                self.push_codebase_index_statuses_snapshot(conn_id, ctx);
            }
            HandlerOutcome::Sync(message) => {
                self.send_server_message(Some(conn_id), Some(&request_id), message);
            }
            HandlerOutcome::Async(Some(handle)) => {
                self.in_progress.insert(request_id, handle);
            }
            HandlerOutcome::Async(None) => {
                // Async work tracked elsewhere (e.g. `pending_file_ops`);
                // the response will be sent via an event subscription.
            }
        }
    }

    fn handle_codebase_index_manager_event(
        &mut self,
        event: &CodebaseIndexManagerEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        if !FeatureFlag::RemoteCodebaseIndexing.is_enabled() {
            return;
        }

        match event {
            CodebaseIndexManagerEvent::SyncStateUpdated
            | CodebaseIndexManagerEvent::IndexMetadataUpdated { .. }
            | CodebaseIndexManagerEvent::NewIndexCreated => {
                self.push_all_codebase_index_statuses(ctx);
            }
            CodebaseIndexManagerEvent::RemoveExpiredIndexMetadata { expired_metadata } => {
                for repo_path in expired_metadata.iter() {
                    self.send_server_message(
                        None,
                        None,
                        server_message::Message::CodebaseIndexStatusUpdated(
                            CodebaseIndexStatusUpdated {
                                status: Some(disabled_codebase_index_status(
                                    repo_path.to_string_lossy().to_string(),
                                )),
                            },
                        ),
                    );
                }
            }
            CodebaseIndexManagerEvent::RetrievalRequestCompleted { .. }
            | CodebaseIndexManagerEvent::RetrievalRequestFailed { .. } => {}
        }
    }

    fn push_all_codebase_index_statuses(&self, ctx: &mut ModelContext<Self>) {
        let snapshot = self.codebase_index_statuses_snapshot(ctx);
        let status_count = snapshot.statuses.len();
        log::info!(
            "[Remote codebase indexing] Daemon pushing codebase index status updates: status_count={status_count}"
        );
        for status in snapshot.statuses {
            log::debug!(
                "[Remote codebase indexing] Daemon pushing codebase index status update: repo_path={} state={:?}",
                status.repo_path,
                CodebaseIndexStatusState::try_from(status.state)
                    .unwrap_or(CodebaseIndexStatusState::Unspecified),
            );
            self.send_server_message(
                None,
                None,
                server_message::Message::CodebaseIndexStatusUpdated(CodebaseIndexStatusUpdated {
                    status: Some(status),
                }),
            );
        }
    }

    fn push_codebase_index_statuses_snapshot(
        &self,
        conn_id: ConnectionId,
        ctx: &mut ModelContext<Self>,
    ) {
        if !FeatureFlag::RemoteCodebaseIndexing.is_enabled() {
            log::info!(
                "[Remote codebase indexing] Daemon skipping bootstrap codebase index statuses snapshot because remote indexing is disabled: conn_id={conn_id}"
            );
            return;
        }
        let snapshot = self.codebase_index_statuses_snapshot(ctx);
        let status_count = snapshot.statuses.len();
        log::info!(
            "[Remote codebase indexing] Daemon pushing bootstrap codebase index statuses snapshot: conn_id={conn_id} bootstrap_status_count={status_count}"
        );
        self.send_server_message(
            Some(conn_id),
            None,
            server_message::Message::CodebaseIndexStatusesSnapshot(snapshot),
        );
    }
    fn codebase_index_statuses_snapshot(
        &self,
        ctx: &mut ModelContext<Self>,
    ) -> CodebaseIndexStatusesSnapshot {
        let index_manager = CodebaseIndexManager::handle(ctx);
        let statuses = index_manager
            .as_ref(ctx)
            .get_codebase_index_statuses(ctx)
            .map(|(repo_path, status)| codebase_index_status_to_proto(repo_path.as_path(), &status))
            .collect();
        CodebaseIndexStatusesSnapshot { statuses }
    }

    fn handle_index_codebase(
        &mut self,
        msg: IndexCodebase,
        request_id: &RequestId,
        conn_id: ConnectionId,
        ctx: &mut ModelContext<Self>,
    ) -> HandlerOutcome {
        let IndexCodebase {
            repo_path,
            auth_token,
        } = msg;
        let request = match self.prepare_codebase_index_request(
            CodebaseIndexRequestParams {
                operation_name: "IndexCodebase",
                repo_path,
                auth_token,
                auth_operation: "remote codebase indexing",
                path_kind: CodebaseIndexRequestPathKind::Canonicalized,
            },
            request_id,
            conn_id,
        ) {
            Ok(request) => request,
            Err(outcome) => return *outcome,
        };
        let repo_path = request.repo_path;
        let status = CodebaseIndexManager::handle(ctx).update(ctx, |manager, ctx| {
            manager.with_indexed_codebase(
                &repo_path,
                |manager, indexed_repo_path, ctx| {
                    Self::current_codebase_index_status_or_queued(manager, indexed_repo_path, ctx)
                },
                |manager, repo_path, ctx| {
                    manager.index_directory(repo_path.to_path_buf(), ctx);
                    queued_codebase_index_status(repo_path.to_string_lossy().to_string())
                },
                ctx,
            )
        });

        HandlerOutcome::Sync(server_message::Message::CodebaseIndexStatusUpdated(
            CodebaseIndexStatusUpdated {
                status: Some(status),
            },
        ))
    }

    fn handle_resync_codebase(
        &mut self,
        msg: ResyncCodebase,
        request_id: &RequestId,
        conn_id: ConnectionId,
        ctx: &mut ModelContext<Self>,
    ) -> HandlerOutcome {
        let ResyncCodebase {
            repo_path,
            auth_token,
        } = msg;
        let request = match self.prepare_codebase_index_request(
            CodebaseIndexRequestParams {
                operation_name: "ResyncCodebase",
                repo_path,
                auth_token,
                auth_operation: "remote codebase resync",
                path_kind: CodebaseIndexRequestPathKind::Canonicalized,
            },
            request_id,
            conn_id,
        ) {
            Ok(request) => request,
            Err(outcome) => return *outcome,
        };
        let repo_path = request.repo_path;
        let status = CodebaseIndexManager::handle(ctx).update(ctx, |manager, ctx| {
            manager.with_indexed_codebase(
                &repo_path,
                |manager, indexed_repo_path, ctx| {
                    manager.try_manual_resync_codebase(indexed_repo_path, ctx);
                    Self::current_codebase_index_status_or_queued(manager, indexed_repo_path, ctx)
                },
                |_, repo_path, _| {
                    unavailable_codebase_index_status(
                        repo_path.to_string_lossy().to_string(),
                        "Cannot resync remote codebase because it has not been indexed."
                            .to_string(),
                    )
                },
                ctx,
            )
        });

        HandlerOutcome::Sync(server_message::Message::CodebaseIndexStatusUpdated(
            CodebaseIndexStatusUpdated {
                status: Some(status),
            },
        ))
    }

    fn current_codebase_index_status_or_queued(
        manager: &CodebaseIndexManager,
        indexed_repo_path: &Path,
        ctx: &mut ModelContext<CodebaseIndexManager>,
    ) -> CodebaseIndexStatus {
        manager
            .get_codebase_index_status_for_path(indexed_repo_path, ctx)
            .map(|status| codebase_index_status_to_proto(indexed_repo_path, &status))
            .unwrap_or_else(|| {
                queued_codebase_index_status(indexed_repo_path.to_string_lossy().to_string())
            })
    }

    fn handle_drop_codebase_index(
        &mut self,
        msg: DropCodebaseIndex,
        request_id: &RequestId,
        conn_id: ConnectionId,
        ctx: &mut ModelContext<Self>,
    ) -> HandlerOutcome {
        let DropCodebaseIndex {
            repo_path,
            auth_token,
        } = msg;
        let request = match self.prepare_codebase_index_request(
            CodebaseIndexRequestParams {
                operation_name: "DropCodebaseIndex",
                repo_path,
                auth_token,
                auth_operation: "remote codebase index removal",
                path_kind: CodebaseIndexRequestPathKind::Requested,
            },
            request_id,
            conn_id,
        ) {
            Ok(request) => request,
            Err(outcome) => return *outcome,
        };
        let CodebaseIndexRequest { repo_path } = request;
        CodebaseIndexManager::handle(ctx).update(ctx, |manager, ctx| {
            manager.drop_index(repo_path.clone(), ctx);
        });

        codebase_index_status_response(disabled_codebase_index_status(
            repo_path.to_string_lossy().to_string(),
        ))
    }

    fn handle_get_fragment_metadata_from_hash(
        &self,
        msg: GetFragmentMetadataFromHash,
        request_id: &RequestId,
        conn_id: ConnectionId,
        ctx: &mut ModelContext<Self>,
    ) -> HandlerOutcome {
        log::info!(
            "[Remote codebase indexing] Daemon handling GetFragmentMetadataFromHash: \
             request_id={request_id} conn_id={conn_id} repo_path={} root_hash={} hash_count={}",
            msg.repo_path,
            msg.root_hash,
            msg.content_hashes.len()
        );

        if !FeatureFlag::RemoteCodebaseIndexing.is_enabled() {
            return fragment_metadata_lookup_error_response(
                FragmentMetadataLookupErrorCode::RemoteCodebaseIndexingNotEnabled,
                "Remote codebase indexing is not enabled".to_string(),
                None,
            );
        }

        let repo_path = match canonicalize_index_repo_path(&msg.repo_path) {
            Ok(repo_path) => repo_path,
            Err(error) => {
                return fragment_metadata_lookup_error_response(
                    FragmentMetadataLookupErrorCode::InvalidRepoPath,
                    error,
                    None,
                );
            }
        };
        let root_hash = match msg.root_hash.parse::<NodeHash>() {
            Ok(root_hash) => root_hash,
            Err(error) => {
                return fragment_metadata_lookup_error_response(
                    FragmentMetadataLookupErrorCode::InvalidRootHash,
                    format!("Invalid root_hash: {error}"),
                    None,
                );
            }
        };
        if let Err(error) = self.validate_fragment_metadata_lookup(&repo_path, &root_hash, ctx) {
            return fragment_metadata_lookup_error_response_from_error(error);
        }

        let mut valid_hashes = Vec::new();
        let mut missing_hashes = Vec::new();
        for content_hash in msg.content_hashes {
            match content_hash.parse::<ContentHash>() {
                Ok(parsed_hash) => valid_hashes.push((content_hash, parsed_hash)),
                Err(error) => missing_hashes.push(missing_fragment_metadata(
                    content_hash,
                    format!("Invalid content hash: {error}"),
                )),
            }
        }

        let content_hashes = valid_hashes
            .iter()
            .map(|(_, hash)| hash.clone())
            .collect::<Vec<_>>();
        let metadata_by_hash = match CodebaseIndexManager::handle(ctx)
            .as_ref(ctx)
            .fragment_metadatas_from_hashes(&repo_path, &root_hash, &content_hashes, ctx)
        {
            Ok(metadata_by_hash) => metadata_by_hash,
            Err(error) => {
                return fragment_metadata_lookup_error_response_from_error(error);
            }
        };

        let mut fragments = Vec::new();
        for (content_hash_string, content_hash) in valid_hashes {
            match metadata_by_hash.get(&content_hash) {
                Some(metadata) => {
                    fragments.extend(
                        metadata
                            .iter()
                            .map(|metadata| fragment_metadata_to_proto(&content_hash, metadata)),
                    );
                }
                None => missing_hashes.push(missing_fragment_metadata(
                    content_hash_string,
                    "No fragment metadata found for content hash".to_string(),
                )),
            }
        }

        HandlerOutcome::Sync(
            server_message::Message::GetFragmentMetadataFromHashResponse(
                GetFragmentMetadataFromHashResponse {
                    result: Some(get_fragment_metadata_from_hash_response::Result::Success(
                        GetFragmentMetadataFromHashSuccess {
                            fragments,
                            missing_hashes,
                        },
                    )),
                },
            ),
        )
    }

    fn validate_fragment_metadata_lookup(
        &self,
        repo_path: &Path,
        root_hash: &NodeHash,
        ctx: &mut ModelContext<Self>,
    ) -> Result<(), LocalFragmentMetadataLookupError> {
        let Some(status) = CodebaseIndexManager::handle(ctx)
            .as_ref(ctx)
            .get_codebase_index_status_for_path(repo_path, ctx)
        else {
            return Err(LocalFragmentMetadataLookupError::IndexNotFound);
        };
        if !status.has_synced_version() {
            return Err(LocalFragmentMetadataLookupError::IndexNotSynced);
        }
        let Some(current_root_hash) = status.root_hash() else {
            return Err(LocalFragmentMetadataLookupError::IndexNotSynced);
        };
        if current_root_hash != root_hash {
            return Err(LocalFragmentMetadataLookupError::RootHashMismatch {
                requested: root_hash.clone(),
                current: current_root_hash.clone(),
            });
        }

        Ok(())
    }

    /// Routes a server message to its destination.
    ///
    /// - `conn_id = Some(id)` — sends only to the connection that originated
    ///   the request (used for all request/response pairs).
    /// - `conn_id = None` — broadcasts to every connected proxy (used for
    ///   server-initiated push notifications such as repo metadata updates).
    fn send_server_message(
        &self,
        conn_id: Option<ConnectionId>,
        request_id: Option<&RequestId>,
        message: server_message::Message,
    ) {
        let msg = ServerMessage {
            request_id: request_id.map(|id| id.clone().into()).unwrap_or_default(),
            message: Some(message),
        };
        if let Some(target) = conn_id {
            if let Some(conn_tx) = self.connection_senders.get(&target) {
                if let Err(e) = conn_tx.try_send(msg) {
                    log::warn!("Daemon: failed to send to conn {target}: {e}");
                }
            } else {
                log::debug!("Daemon: no sender for conn {target} (already disconnected)");
            }
        } else {
            // Push notification — broadcast to all connections.
            for (id, conn_tx) in &self.connection_senders {
                if let Err(e) = conn_tx.try_send(msg.clone()) {
                    log::warn!("Daemon: failed to send to conn {id}: {e}");
                }
            }
        }
    }

    /// Spawns an abortable future tied to `request_id` and wires up automatic
    /// removal from `in_progress` on completion or abort.
    ///
    /// The returned handle is intended to be returned from a handler as
    /// `HandlerOutcome::Async(Some(handle))`; the caller (`handle_message`)
    /// inserts it into `in_progress`.
    fn spawn_request_handler<S, F>(
        &mut self,
        request_id: RequestId,
        future: S,
        on_resolve: F,
        ctx: &mut ModelContext<Self>,
    ) -> SpawnedFutureHandle
    where
        S: Spawnable,
        <S as Future>::Output: SpawnableOutput,
        F: 'static + FnOnce(&mut Self, <S as Future>::Output, &mut ModelContext<Self>),
    {
        let resolve_id = request_id.clone();
        let abort_id = request_id;
        ctx.spawn_abortable(
            future,
            move |me, output, ctx| {
                me.in_progress.remove(&resolve_id);
                on_resolve(me, output, ctx);
            },
            move |me, _ctx| {
                log::info!("Request cancelled (request_id={abort_id})");
                me.in_progress.remove(&abort_id);
            },
        )
    }

    /// Handles `Initialize` by returning the server version and host id.
    ///
    /// Also configures Sentry crash reporting based on the user's identity
    /// and preferences supplied by the connecting client.
    #[cfg_attr(not(feature = "crash_reporting"), allow(unused_variables))]
    fn handle_initialize(
        &mut self,
        msg: Initialize,
        request_id: &RequestId,
        ctx: &mut ModelContext<Self>,
    ) -> HandlerOutcome {
        log::info!("Handling Initialize (request_id={request_id})");
        self.apply_initialize_auth(&msg);

        // Update crash reporting based on client-supplied preferences.
        #[cfg(feature = "crash_reporting")]
        {
            if msg.crash_reporting_enabled {
                self.apply_sentry_user_id(ctx);
            } else {
                crate::crash_reporting::uninit_sentry();
            }
        }

        let server_version = ChannelState::app_version().unwrap_or("").to_string();
        HandlerOutcome::Sync(server_message::Message::InitializeResponse(
            InitializeResponse {
                server_version,
                host_id: self.host_id.clone(),
            },
        ))
    }

    /// Applies the auth token from an `Initialize` message.
    /// Extracted so unit tests can call it without a `ModelContext`.
    fn apply_initialize_auth(&mut self, msg: &Initialize) {
        self.auth_state.apply_remote_server_auth_context(
            msg.auth_token.clone(),
            msg.user_id.clone(),
            msg.user_email.clone(),
        );
    }

    /// Sets the Sentry user identity from the stored `AuthState`.
    /// Called both during `Initialize` and when re-enabling crash reporting
    /// via `UpdatePreferences`.
    #[cfg(feature = "crash_reporting")]
    fn apply_sentry_user_id(&self, ctx: &mut warpui::AppContext) {
        if let Some(user_id) = self.auth_state.user_id() {
            crate::crash_reporting::set_user_id(user_id, self.auth_state.user_email(), ctx);
        }
    }

    /// Handles `UpdatePreferences` by dynamically enabling or disabling
    /// Sentry crash reporting. This is a notification — no response is sent.
    fn handle_update_preferences(
        &mut self,
        msg: super::proto::UpdatePreferences,
        #[allow(unused_variables)] ctx: &mut ModelContext<Self>,
    ) {
        log::info!(
            "Handling UpdatePreferences: crash_reporting_enabled={}",
            msg.crash_reporting_enabled
        );
        #[cfg(feature = "crash_reporting")]
        {
            if msg.crash_reporting_enabled {
                if !crate::crash_reporting::is_initialized() {
                    crate::crash_reporting::init(ctx);
                    self.apply_sentry_user_id(ctx);
                }
            } else {
                crate::crash_reporting::uninit_sentry();
            }
        }
    }

    /// Handles `Authenticate` by replacing the daemon-wide credential.
    /// This is a notification — no response is sent.
    fn handle_authenticate(&mut self, msg: Authenticate) {
        self.auth_state
            .set_remote_server_bearer_token(msg.auth_token);
    }

    pub fn auth_token(&self) -> Option<String> {
        self.auth_state.get_access_token_ignoring_validity()
    }

    fn validate_remote_codebase_index_auth(
        &self,
        auth_token: &str,
        operation: &str,
    ) -> Result<(), String> {
        if auth_token.is_empty() {
            return Err(format!(
                "Missing authentication credentials for {operation}"
            ));
        }

        match self.auth_token() {
            Some(cached_auth_token) if cached_auth_token == auth_token => Ok(()),
            Some(_) => Err(format!(
                "Authentication credentials for {operation} do not match daemon credentials"
            )),
            None => Err(format!(
                "Missing cached authentication credentials for {operation}"
            )),
        }
    }

    fn prepare_codebase_index_request(
        &self,
        params: CodebaseIndexRequestParams<'_>,
        request_id: &RequestId,
        conn_id: ConnectionId,
    ) -> Result<CodebaseIndexRequest, Box<HandlerOutcome>> {
        let CodebaseIndexRequestParams {
            operation_name,
            repo_path,
            auth_token,
            auth_operation,
            path_kind,
        } = params;
        let repo_path_for_log = repo_path.clone();
        if !FeatureFlag::RemoteCodebaseIndexing.is_enabled() {
            log::info!(
                "[Remote codebase indexing] Daemon rejecting {operation_name} because remote indexing is disabled: request_id={request_id} conn_id={conn_id} repo_path={repo_path_for_log}"
            );
            return Err(Box::new(codebase_index_status_response(
                not_enabled_codebase_index_status(repo_path),
            )));
        }

        let repo_path = match path_kind {
            CodebaseIndexRequestPathKind::Canonicalized => canonicalize_index_repo_path(&repo_path),
            CodebaseIndexRequestPathKind::Requested => requested_repo_path(&repo_path),
        }
        .map_err(|error| Box::new(invalid_request_response(error)))?;

        if let Err(error) = self.validate_remote_codebase_index_auth(&auth_token, auth_operation) {
            return Err(Box::new(invalid_request_response(error)));
        }

        log::info!(
            "[Remote codebase indexing] Daemon handling {operation_name}: request_id={request_id} conn_id={conn_id} repo_path={repo_path_for_log}"
        );
        Ok(CodebaseIndexRequest { repo_path })
    }

    /// Handles `Abort` by cancelling the in-progress request it targets.
    /// Checks `ServerModel`'s own in-progress map first, then delegates to
    /// the diff state manager for content reload requests.
    /// This is a notification — no response is sent.
    fn handle_abort(&mut self, abort: Abort, request_id: &RequestId, ctx: &mut ModelContext<Self>) {
        let target_id = RequestId::from(abort.request_id_to_abort);
        if let Some(handle) = self.in_progress.remove(&target_id) {
            log::info!(
                "Aborting in-progress request (request_id={target_id}, \
                 abort_request_id={request_id})"
            );
            handle.abort();
        } else {
            let found = self
                .diff_states
                .update(ctx, |mgr, _| mgr.abort_request(&target_id));
            if !found {
                log::info!(
                    "Abort for unknown/completed request (request_id={target_id}, \
                     abort_request_id={request_id})"
                );
            }
        }
    }

    /// Handles `SessionBootstrapped` by creating a `LocalCommandExecutor` for
    /// the session. This is a notification — no response is sent.
    fn handle_session_bootstrapped(&mut self, msg: SessionBootstrapped) {
        let session_id = SessionId::from(msg.session_id);
        log::info!(
            "Handling SessionBootstrapped: session_id={session_id:?}, \
             shell_type={:?}, shell_path={:?}",
            msg.shell_type,
            msg.shell_path,
        );

        let Some(shell_type) = ShellType::from_name(&msg.shell_type) else {
            safe_error!(
                safe: ("Received unknown shell_type in SessionBootstrapped: shell_type={:?}", msg.shell_type),
                full: ("Received unknown shell_type in SessionBootstrapped: shell_type={:?} session={session_id:?}", msg.shell_type)
            );
            return;
        };

        let shell_path = msg.shell_path.map(PathBuf::from);
        if shell_path.is_none() {
            log::warn!(
                "SessionBootstrapped for session {session_id:?} had no shell_path; \
                 LocalCommandExecutor will fall back to bare shell name",
            );
        }
        let executor = Arc::new(LocalCommandExecutor::new(shell_path, shell_type));
        if self.executors.insert(session_id, executor).is_some() {
            log::warn!(
                "Overwriting existing executor for session {session_id:?} \
                 (re-SessionBootstrapped with shell_type={:?})",
                msg.shell_type,
            );
        }
    }

    /// Handles `RunCommand` by delegating to the session's `LocalCommandExecutor`.
    ///
    /// On success, returns a `HandlerOutcome::Async` whose task resolves the
    /// request with a `RunCommandResponse`. On validation failure (missing
    /// executor), returns a `HandlerOutcome::Sync` error response.
    fn handle_run_command(
        &mut self,
        req: RunCommandRequest,
        request_id: &RequestId,
        conn_id: ConnectionId,
        ctx: &mut ModelContext<Self>,
    ) -> HandlerOutcome {
        let session_id = SessionId::from(req.session_id);
        log::info!(
            "Handling RunCommand (request_id={request_id}, session_id={session_id:?}): \
             command={:?}, cwd={:?}",
            req.command,
            req.working_directory,
        );

        let command = req.command;
        let cwd = req.working_directory;
        let env_vars = if req.environment_variables.is_empty() {
            None
        } else {
            Some(req.environment_variables)
        };

        let Some(executor) = self.executors.get(&session_id).cloned() else {
            safe_error!(
                safe: ("No executor for RunCommand, session was never initialized"),
                full: ("No executor for RunCommand, session was never initialized: session={session_id:?}")
            );
            return HandlerOutcome::Sync(server_message::Message::RunCommandResponse(
                RunCommandResponse {
                    result: Some(run_command_response::Result::Error(RunCommandError {
                        code: RunCommandErrorCode::SessionNotFound.into(),
                        message: format!("No executor for session {session_id:?}"),
                    })),
                },
            ));
        };

        // Call `execute_local_command` directly because the
        // `CommandExecutor::execute_command` trait method requires
        // a `&Shell` (version, options, plugins from bootstrap).
        let request_id_for_response = request_id.clone();
        let conn_id_for_response = conn_id;
        let handle = self.spawn_request_handler(
            request_id.clone(),
            async move {
                executor
                    .execute_local_command(
                        &command,
                        cwd.as_deref(),
                        env_vars,
                        ExecuteCommandOptions::default(),
                    )
                    .await
            },
            move |me, result, _ctx| {
                let result_oneof = match result {
                    Ok(output) => {
                        let mut stdout = output.stdout.clone();
                        let mut stderr = output.stderr.clone();

                        // Truncate to stay under the wire-level message size
                        // limit. Leave headroom for protobuf framing overhead.
                        const MAX_OUTPUT_BYTES: usize =
                            remote_server::protocol::MAX_MESSAGE_SIZE - 1024;
                        let total = stdout.len() + stderr.len();
                        if total > MAX_OUTPUT_BYTES {
                            log::warn!(
                                "RunCommand output too large \
                                 (request_id={request_id_for_response}): \
                                 {total} bytes, truncating to {MAX_OUTPUT_BYTES}"
                            );
                            let ratio = MAX_OUTPUT_BYTES as f64 / total as f64;
                            stdout.truncate((stdout.len() as f64 * ratio) as usize);
                            stderr.truncate((stderr.len() as f64 * ratio) as usize);
                        }

                        log::info!(
                            "RunCommand completed (request_id={request_id_for_response}): \
                             exit_code={:?}, stdout_len={}, stderr_len={}",
                            output.exit_code,
                            stdout.len(),
                            stderr.len(),
                        );
                        run_command_response::Result::Success(RunCommandSuccess {
                            stdout,
                            stderr,
                            exit_code: output.exit_code.map(|c| c.value()),
                        })
                    }
                    Err(e) => {
                        log::warn!("RunCommand failed (request_id={request_id_for_response}): {e}");
                        run_command_response::Result::Error(RunCommandError {
                            code: RunCommandErrorCode::ExecutionFailed.into(),
                            message: format!("Failed to execute command: {e}"),
                        })
                    }
                };
                me.send_server_message(
                    Some(conn_id_for_response),
                    Some(&request_id_for_response),
                    server_message::Message::RunCommandResponse(RunCommandResponse {
                        result: Some(result_oneof),
                    }),
                );
            },
            ctx,
        );
        HandlerOutcome::Async(Some(handle))
    }

    /// Handles `NavigatedToDirectory` by running git detection first, then
    /// responding. On validation failure returns a `HandlerOutcome::Sync` error;
    /// otherwise spawns a task and returns a `HandlerOutcome::Async(Some(_))`
    /// handle.
    fn handle_navigated_to_directory(
        &mut self,
        msg: NavigatedToDirectory,
        request_id: &RequestId,
        conn_id: ConnectionId,
        ctx: &mut ModelContext<Self>,
    ) -> HandlerOutcome {
        log::info!(
            "Handling NavigatedToDirectory path={} (request_id={request_id})",
            msg.path
        );

        let std_path = match StandardizedPath::from_local_canonicalized(Path::new(&msg.path)) {
            Ok(p) => p,
            Err(e) => {
                log::warn!("Invalid path for NavigatedToDirectory: {e}");
                return HandlerOutcome::Sync(server_message::Message::Error(ErrorResponse {
                    code: ErrorCode::InvalidRequest.into(),
                    message: format!("Invalid path: {e}"),
                }));
            }
        };

        // Kick off git detection. The returned future resolves with the git
        // root path (Some) or None if no git repo was found.
        let path_str = msg.path.clone();
        let git_future = DetectedRepositories::handle(ctx).update(ctx, |repos, ctx| {
            repos.detect_possible_local_git_repo(
                &path_str,
                RepoDetectionSource::TerminalNavigation,
                ctx,
            )
        });

        let request_id_for_response = request_id.clone();
        let conn_id_for_response = conn_id;
        let handle = self.spawn_request_handler(
            request_id.clone(),
            git_future,
            move |me, git_root, ctx| {
                let (indexed_path, is_git) = if let Some(root) = git_root {
                    // Git repo found. Full indexing was already triggered by
                    // DetectedGitRepo → LocalRepoMetadataModel. The client
                    // waits for RepositoryIndexedPush before FetchFileTree.
                    let root_str = root.to_string_lossy().to_string();
                    log::info!("Git repo detected at {root_str} for path {}", std_path);
                    (root_str, true)
                } else {
                    // No git repo. Lazy-load the directory for first-level data,
                    // then push the snapshot immediately.
                    RepoMetadataModel::handle(ctx).update(ctx, |repo_model, ctx| {
                        if let Err(e) = repo_model.index_lazy_loaded_path(&std_path, ctx) {
                            log::warn!("Failed to lazy-load directory {std_path}: {e}");
                        }
                    });
                    (std_path.to_string(), false)
                };

                me.send_server_message(
                    Some(conn_id_for_response),
                    Some(&request_id_for_response),
                    server_message::Message::NavigatedToDirectoryResponse(
                        NavigatedToDirectoryResponse {
                            indexed_path: indexed_path.clone(),
                            is_git,
                        },
                    ),
                );
                // After responding, push a snapshot if metadata is available.
                //
                // For git repos this is an opportunistic push for the case
                // where the repo was already indexed and RepositoryUpdated
                // won't fire again (which would otherwise leave the client
                // with only a placeholder root). We skip if a snapshot was
                // already sent for this connection+root.
                //
                // For non-git directories the lazy-loaded tree is always
                // broadcast to all connections.
                if let Ok(root_path) =
                    StandardizedPath::from_local_canonicalized(Path::new(&indexed_path))
                {
                    if is_git {
                        let already_sent = me
                            .snapshot_sent_roots_by_connection
                            .get(&conn_id_for_response)
                            .is_some_and(|roots| roots.contains(&root_path));
                        if already_sent {
                            log::debug!(
                                "Snapshot already sent for repo {indexed_path} \
                                 to conn {conn_id_for_response}, skipping"
                            );
                            return;
                        }
                    }

                    let id = RepositoryIdentifier::local(root_path.clone());
                    let repo_model = RepoMetadataModel::handle(ctx);
                    if let Some(state) = repo_model.as_ref(ctx).get_repository(&id, ctx) {
                        let entries = super::repo_metadata_proto::file_tree_entry_to_snapshot_proto(
                            &state.entry,
                        );
                        // Git snapshots target the requesting connection;
                        // non-git snapshots broadcast to all.
                        let target = if is_git {
                            Some(conn_id_for_response)
                        } else {
                            None
                        };
                        me.send_server_message(
                            target,
                            None,
                            server_message::Message::RepoMetadataSnapshot(
                                super::proto::RepoMetadataSnapshot {
                                    repo_path: indexed_path,
                                    entries,
                                    sync_complete: true,
                                },
                            ),
                        );
                        if is_git {
                            if let Some(sent_roots) = me
                                .snapshot_sent_roots_by_connection
                                .get_mut(&conn_id_for_response)
                            {
                                sent_roots.insert(root_path);
                            }
                        }
                    }
                }
            },
            ctx,
        );
        HandlerOutcome::Async(Some(handle))
    }

    /// Handles `LoadRepoMetadataDirectory` by loading a subdirectory on the
    /// server's local model and returning the children synchronously.
    fn handle_load_repo_metadata_directory(
        &mut self,
        msg: super::proto::LoadRepoMetadataDirectory,
        request_id: &RequestId,
        ctx: &mut ModelContext<Self>,
    ) -> HandlerOutcome {
        log::info!(
            "Handling LoadRepoMetadataDirectory repo_path={} dir_path={} (request_id={request_id})",
            msg.repo_path,
            msg.dir_path
        );

        let repo_path = match StandardizedPath::from_local_canonicalized(Path::new(&msg.repo_path))
        {
            Ok(p) => p,
            Err(e) => {
                return HandlerOutcome::Sync(server_message::Message::Error(ErrorResponse {
                    code: ErrorCode::InvalidRequest.into(),
                    message: format!("Invalid repo_path: {e}"),
                }));
            }
        };

        let dir_path = match StandardizedPath::from_local_canonicalized(Path::new(&msg.dir_path)) {
            Ok(p) => p,
            Err(e) => {
                return HandlerOutcome::Sync(server_message::Message::Error(ErrorResponse {
                    code: ErrorCode::InvalidRequest.into(),
                    message: format!("Invalid dir_path: {e}"),
                }));
            }
        };

        // Validate that the directory is a descendant of the repo.
        if !dir_path.starts_with(&repo_path) {
            return HandlerOutcome::Sync(server_message::Message::Error(ErrorResponse {
                code: ErrorCode::InvalidRequest.into(),
                message: format!(
                    "dir_path {dir_path} is not a descendant of repo_path {repo_path}"
                ),
            }));
        }

        // Load the directory on the server's local model.
        let load_result = RepoMetadataModel::handle(ctx).update(ctx, |model, ctx| {
            model.load_directory(&repo_path, &dir_path, ctx)
        });

        if let Err(e) = load_result {
            log::warn!("LoadRepoMetadataDirectory failed: {e}");
            return HandlerOutcome::Sync(server_message::Message::Error(ErrorResponse {
                code: ErrorCode::Internal.into(),
                message: format!("Failed to load directory: {e}"),
            }));
        }

        // Read back the loaded children and serialize them.
        let id = RepositoryIdentifier::local(repo_path.clone());
        let entries = RepoMetadataModel::handle(ctx)
            .as_ref(ctx)
            .get_repository(&id, ctx)
            .map(|state| {
                super::repo_metadata_proto::file_tree_children_to_proto_entries(
                    &state.entry,
                    &dir_path,
                )
            })
            .unwrap_or_default();

        HandlerOutcome::Sync(server_message::Message::LoadRepoMetadataDirectoryResponse(
            super::proto::LoadRepoMetadataDirectoryResponse {
                repo_path: msg.repo_path,
                dir_path: msg.dir_path,
                entries,
            },
        ))
    }

    /// Handles `WriteFile` by registering the path and triggering an async
    /// write via `FileModel`. On a successful dispatch, returns
    /// `HandlerOutcome::Async(None)` — the response is sent later by the
    /// `FileModel` event subscription, and the op is not cancellable via
    /// `Abort`. On failure to dispatch, returns a `HandlerOutcome::Sync`
    /// error response.
    fn handle_write_file(
        &mut self,
        msg: WriteFile,
        request_id: &RequestId,
        conn_id: ConnectionId,
        ctx: &mut ModelContext<Self>,
    ) -> HandlerOutcome {
        log::info!(
            "Handling WriteFile path={} (request_id={request_id})",
            msg.path
        );
        let path = Path::new(&msg.path);

        let (file_id, version) =
            self.pending_file_ops
                .insert(path, request_id.clone(), conn_id, FileOpKind::Write, ctx);

        let file_model = FileModel::handle(ctx);
        if let Err(err) =
            file_model.update(ctx, |m, ctx| m.save(file_id, msg.content, version, ctx))
        {
            self.pending_file_ops.remove(file_id, ctx);
            return HandlerOutcome::Sync(server_message::Message::WriteFileResponse(
                WriteFileResponse {
                    result: Some(write_file_response::Result::Error(FileOperationError {
                        message: format!("Failed to initiate write: {err}"),
                    })),
                },
            ));
        }

        // Response sent asynchronously via the event subscription.
        HandlerOutcome::Async(None)
    }

    /// Handles `DeleteFile` by registering the path and triggering an async
    /// delete via `FileModel`. On a successful dispatch, returns
    /// `HandlerOutcome::Async(None)` — the response is sent later by the
    /// `FileModel` event subscription, and the op is not cancellable via
    /// `Abort`. On failure to dispatch, returns a `HandlerOutcome::Sync`
    /// error response.
    fn handle_delete_file(
        &mut self,
        msg: DeleteFile,
        request_id: &RequestId,
        conn_id: ConnectionId,
        ctx: &mut ModelContext<Self>,
    ) -> HandlerOutcome {
        log::info!(
            "Handling DeleteFile path={} (request_id={request_id})",
            msg.path
        );
        let path = Path::new(&msg.path);

        let (file_id, version) = self.pending_file_ops.insert(
            path,
            request_id.clone(),
            conn_id,
            FileOpKind::Delete,
            ctx,
        );

        let file_model = FileModel::handle(ctx);
        if let Err(err) = file_model.update(ctx, |m, ctx| m.delete(file_id, version, ctx)) {
            self.pending_file_ops.remove(file_id, ctx);
            return HandlerOutcome::Sync(server_message::Message::DeleteFileResponse(
                DeleteFileResponse {
                    result: Some(delete_file_response::Result::Error(FileOperationError {
                        message: format!("Failed to initiate delete: {err}"),
                    })),
                },
            ));
        }

        // Response sent asynchronously via the event subscription.
        HandlerOutcome::Async(None)
    }

    /// Handles `ReadFileContext` by spawning an async batch file read on the
    /// background executor. Returns `HandlerOutcome::Async` with the spawned
    /// handle so the request can be cancelled via `Abort`.
    fn handle_read_file_context(
        &mut self,
        msg: super::proto::ReadFileContextRequest,
        request_id: &RequestId,
        conn_id: ConnectionId,
        ctx: &mut ModelContext<Self>,
    ) -> HandlerOutcome {
        log::info!(
            "Handling ReadFileContext ({} files, request_id={request_id})",
            msg.files.len()
        );

        let max_file_bytes = msg.max_file_bytes.map(|b| b as usize);
        let max_batch_bytes = msg.max_batch_bytes.map(|b| b as usize);
        let file_locations: Vec<FileLocations> = msg
            .files
            .into_iter()
            .map(|f| FileLocations {
                name: f.path,
                lines: f
                    .line_ranges
                    .into_iter()
                    .map(|r| r.start as usize..r.end as usize)
                    .collect(),
            })
            .collect();
        let request_id_for_response = request_id.clone();

        let handle = self.spawn_request_handler(
            request_id.clone(),
            async move {
                read_local_file_context(
                    &file_locations,
                    None,
                    None,
                    max_file_bytes,
                    max_batch_bytes,
                )
                .await
            },
            move |me, result: anyhow::Result<ReadFileContextResult>, _ctx| {
                let response = match result {
                    Ok(result) => file_context_result_to_proto(result),
                    Err(err) => ReadFileContextResponse {
                        file_contexts: vec![],
                        failed_files: vec![FailedFileRead {
                            path: String::new(),
                            error: Some(FileOperationError {
                                message: format!("{err:#}"),
                            }),
                        }],
                    },
                };
                me.send_server_message(
                    Some(conn_id),
                    Some(&request_id_for_response),
                    server_message::Message::ReadFileContextResponse(response),
                );
            },
            ctx,
        );

        HandlerOutcome::Async(Some(handle))
    }

    /// Handles `OpenBuffer` by opening the file via `GlobalBufferModel`.
    /// The response is sent asynchronously when `BufferLoaded` fires.
    ///
    /// When `force_reload` is set, the server re-reads the file from disk
    /// even if the buffer is already loaded. This broadcasts a
    /// `BufferUpdatedPush` to other connections and responds with the
    /// fresh content via `OpenBufferResponse`.
    fn handle_open_buffer(
        &mut self,
        msg: OpenBuffer,
        request_id: &RequestId,
        conn_id: ConnectionId,
        ctx: &mut ModelContext<Self>,
    ) -> HandlerOutcome {
        log::info!(
            "Handling OpenBuffer path={} force_reload={} (request_id={request_id})",
            msg.path,
            msg.force_reload,
        );

        // For force_reload on an already-tracked buffer, skip open_server_local
        // to avoid a spurious BufferLoaded event that would consume the pending
        // request before ServerLocalBufferUpdated can use it for exclusion.
        if msg.force_reload {
            if let Some(file_id) = self.buffers.file_id_for_path(&msg.path) {
                self.buffers.add_connection(file_id, conn_id);
                let gbm = GlobalBufferModel::handle(ctx);

                self.buffers.insert_pending(
                    file_id,
                    request_id.clone(),
                    conn_id,
                    PendingBufferRequestKind::OpenBuffer,
                );
                if let Err(e) =
                    gbm.update(ctx, |gbm, ctx| gbm.force_reload_server_local(file_id, ctx))
                {
                    self.buffers
                        .take_pending_by_kind(&file_id, PendingBufferRequestKind::OpenBuffer);
                    return HandlerOutcome::Sync(server_message::Message::OpenBufferResponse(
                        OpenBufferResponse {
                            result: Some(
                                remote_server::proto::open_buffer_response::Result::Error(
                                    FileOperationError { message: e },
                                ),
                            ),
                        },
                    ));
                }
                return HandlerOutcome::Async(None);
            }
            // Buffer not yet tracked — fall through to open_server_local below.
        }

        let path = PathBuf::from(&msg.path);
        let gbm = GlobalBufferModel::handle(ctx);
        let buffer_state = gbm.update(ctx, |gbm, ctx| gbm.open_server_local(path, ctx));
        let file_id = buffer_state.file_id;

        // Track path → FileId mapping and connection.
        // Retain the strong buffer handle so the model stays alive until
        // all connections close the buffer.
        self.buffers
            .track_open_buffer(msg.path.clone(), file_id, buffer_state.buffer);
        self.buffers.add_connection(file_id, conn_id);

        if gbm.as_ref(ctx).buffer_loaded(file_id) {
            let Some(content) = gbm.as_ref(ctx).content_for_file(file_id, ctx) else {
                return HandlerOutcome::Sync(server_message::Message::OpenBufferResponse(
                    OpenBufferResponse {
                        result: Some(remote_server::proto::open_buffer_response::Result::Error(
                            FileOperationError {
                                message: "Buffer loaded but has no file content".to_string(),
                            },
                        )),
                    },
                ));
            };
            let Some(server_version) = gbm
                .as_ref(ctx)
                .sync_clock_for_server_local(file_id)
                .map(|c| c.server_version.as_u64())
            else {
                return HandlerOutcome::Sync(server_message::Message::OpenBufferResponse(
                    OpenBufferResponse {
                        result: Some(remote_server::proto::open_buffer_response::Result::Error(
                            FileOperationError {
                                message: "Buffer loaded but has no sync clock".to_string(),
                            },
                        )),
                    },
                ));
            };
            return HandlerOutcome::Sync(server_message::Message::OpenBufferResponse(
                OpenBufferResponse {
                    result: Some(remote_server::proto::open_buffer_response::Result::Success(
                        OpenBufferSuccess {
                            content,
                            server_version,
                        },
                    )),
                },
            ));
        }

        // Not yet loaded — stash request info so the GlobalBufferModelEvent
        // subscription can send the response when content arrives.
        self.buffers.insert_pending(
            file_id,
            request_id.clone(),
            conn_id,
            PendingBufferRequestKind::OpenBuffer,
        );
        HandlerOutcome::Async(None)
    }

    /// Handles `BufferEdit` notification (fire-and-forget).
    /// Delegates to `GlobalBufferModel::apply_client_edit`. On rejection
    /// (stale server version), the edit is silently dropped.
    fn handle_buffer_edit(&mut self, msg: BufferEdit, ctx: &mut ModelContext<Self>) {
        log::info!(
            "Handling BufferEdit path={} expected_sv={} new_cv={} edit_count={}",
            msg.path,
            msg.expected_server_version,
            msg.new_client_version,
            msg.edits.len()
        );
        let Some(file_id) = self.buffers.file_id_for_path(&msg.path) else {
            log::warn!("BufferEdit for unknown buffer: {}", msg.path);
            return;
        };

        let expected_sv = ContentVersion::from_raw(msg.expected_server_version as usize);
        let new_cv = ContentVersion::from_raw(msg.new_client_version as usize);

        // Per spec: if the edit is rejected (stale server version),
        // the server silently drops it.
        let accepted = GlobalBufferModel::handle(ctx).update(ctx, |gbm, ctx| {
            gbm.apply_client_edit(file_id, &msg.edits, expected_sv, new_cv, ctx)
        });
        log::info!("BufferEdit result: path={} accepted={accepted}", msg.path);
    }

    /// Handles `SaveBuffer` by persisting the buffer to disk.
    fn handle_save_buffer(
        &mut self,
        msg: SaveBuffer,
        request_id: &RequestId,
        conn_id: ConnectionId,
        ctx: &mut ModelContext<Self>,
    ) -> HandlerOutcome {
        log::info!(
            "Handling SaveBuffer path={} (request_id={request_id})",
            msg.path
        );

        let Some(file_id) = self.buffers.file_id_for_path(&msg.path) else {
            return HandlerOutcome::Sync(server_message::Message::SaveBufferResponse(
                SaveBufferResponse {
                    result: Some(save_buffer_response::Result::Error(FileOperationError {
                        message: format!("Buffer not open: {}", msg.path),
                    })),
                },
            ));
        };

        let result = GlobalBufferModel::handle(ctx)
            .update(ctx, |gbm, ctx| gbm.save_server_local(file_id, ctx));

        match result {
            Ok(()) => {
                // Response will come via the FileSaved event subscription.
                // Track the file_id → (request_id, conn_id) so the event
                // handler can correlate.
                self.buffers.insert_pending(
                    file_id,
                    request_id.clone(),
                    conn_id,
                    PendingBufferRequestKind::SaveBuffer,
                );
                HandlerOutcome::Async(None)
            }
            Err(err) => HandlerOutcome::Sync(server_message::Message::SaveBufferResponse(
                SaveBufferResponse {
                    result: Some(save_buffer_response::Result::Error(FileOperationError {
                        message: format!("Failed to save: {err}"),
                    })),
                },
            )),
        }
    }

    /// Handles `ResolveConflict` by replacing the server buffer with the
    /// client's content and persisting to disk. Returns an async
    /// `HandlerOutcome` — the response is sent when `FileSaved` or
    /// `FailedToSave` fires.
    fn handle_resolve_conflict(
        &mut self,
        msg: ResolveConflict,
        request_id: &RequestId,
        conn_id: ConnectionId,
        ctx: &mut ModelContext<Self>,
    ) -> HandlerOutcome {
        log::info!(
            "Handling ResolveConflict path={} (request_id={request_id})",
            msg.path
        );

        let Some(file_id) = self.buffers.file_id_for_path(&msg.path) else {
            return HandlerOutcome::Sync(server_message::Message::ResolveConflictResponse(
                ResolveConflictResponse {
                    result: Some(resolve_conflict_response::Result::Error(
                        FileOperationError {
                            message: format!("Buffer not open: {}", msg.path),
                        },
                    )),
                },
            ));
        };

        let ack_sv = ContentVersion::from_raw(msg.acknowledged_server_version as usize);
        let current_cv = ContentVersion::from_raw(msg.current_client_version as usize);
        let result = GlobalBufferModel::handle(ctx).update(ctx, |gbm, ctx| {
            gbm.resolve_conflict(file_id, ack_sv, current_cv, &msg.client_content, ctx)
        });

        match result {
            Ok(()) => {
                self.buffers.insert_pending(
                    file_id,
                    request_id.clone(),
                    conn_id,
                    PendingBufferRequestKind::ResolveConflict,
                );
                HandlerOutcome::Async(None)
            }
            Err(err) => HandlerOutcome::Sync(server_message::Message::ResolveConflictResponse(
                ResolveConflictResponse {
                    result: Some(resolve_conflict_response::Result::Error(
                        FileOperationError {
                            message: format!("Failed to resolve conflict: {err}"),
                        },
                    )),
                },
            )),
        }
    }

    /// Handles `CloseBuffer` notification (fire-and-forget).
    /// Removes the connection from the buffer's connection set.
    /// Deallocates the buffer if no connections remain.
    fn handle_close_buffer(
        &mut self,
        msg: CloseBuffer,
        conn_id: ConnectionId,
        ctx: &mut ModelContext<Self>,
    ) {
        log::info!("Handling CloseBuffer path={} conn={conn_id}", msg.path);
        self.buffers.close_buffer(&msg.path, conn_id, ctx);
    }

    /// Handles `GetDiffState` — subscribe to a (repo, mode) pair.
    fn handle_get_diff_state(
        &mut self,
        msg: super::proto::GetDiffState,
        request_id: &RequestId,
        conn_id: ConnectionId,
        ctx: &mut ModelContext<Self>,
    ) -> HandlerOutcome {
        // Proto3 message fields are always optional on the wire, so `mode`
        // cannot be made required at the schema level — validate at runtime.
        let Some(mode_proto) = &msg.mode else {
            return HandlerOutcome::Sync(server_message::Message::Error(ErrorResponse {
                code: ErrorCode::InvalidRequest.into(),
                message: "Missing mode in GetDiffState".to_string(),
            }));
        };

        let std_path = match StandardizedPath::from_local_canonicalized(Path::new(&msg.repo_path)) {
            Ok(p) => p,
            Err(e) => {
                log::warn!("Invalid repo_path for GetDiffState: {e}");
                return HandlerOutcome::Sync(server_message::Message::Error(ErrorResponse {
                    code: ErrorCode::InvalidRequest.into(),
                    message: format!("Invalid repo_path: {e}"),
                }));
            }
        };

        let mode: DiffMode = mode_proto.into();

        log::info!(
            "Handling GetDiffState repo={} mode={mode:?} (request_id={request_id})",
            msg.repo_path,
        );

        let outcome = self.diff_states.update(ctx, |mgr, ctx| {
            mgr.subscribe(std_path, mode, request_id, conn_id, ctx)
        });

        match outcome {
            SubscribeOutcome::RespondWithSnapshot {
                key,
                state,
                metadata,
            } => {
                let snapshot = diff_state_proto::build_diff_state_snapshot(
                    key.repo_path.as_str(),
                    &key.mode,
                    metadata.as_ref(),
                    &state,
                    None,
                );
                HandlerOutcome::Sync(server_message::Message::GetDiffStateResponse(
                    GetDiffStateResponse {
                        result: Some(get_diff_state_response::Result::Snapshot(snapshot)),
                    },
                ))
            }
            SubscribeOutcome::Async => HandlerOutcome::Async(None),
        }
    }

    /// Handles `UnsubscribeDiffState` — notification (fire-and-forget).
    fn handle_unsubscribe_diff_state(
        &mut self,
        msg: super::proto::UnsubscribeDiffState,
        conn_id: ConnectionId,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(mode_proto) = &msg.mode else {
            log::warn!("UnsubscribeDiffState from conn={conn_id}: missing mode");
            return;
        };
        let Ok(std_path) = StandardizedPath::from_local_canonicalized(Path::new(&msg.repo_path))
        else {
            log::warn!(
                "UnsubscribeDiffState from conn={conn_id}: invalid repo_path={}",
                msg.repo_path
            );
            return;
        };

        let key = DiffModelKey {
            repo_path: std_path,
            mode: mode_proto.into(),
        };

        log::info!(
            "Handling UnsubscribeDiffState repo={} mode={:?} conn={conn_id}",
            msg.repo_path,
            key.mode
        );

        self.diff_states
            .update(ctx, |mgr, _| mgr.unsubscribe_connection(&key, conn_id));
    }

    /// Converts a domain-level diff state dispatch to proto messages
    /// and sends them to the appropriate connections.
    fn handle_diff_state_update(&self, update: &DiffStateUpdate) {
        match update {
            DiffStateUpdate::Snapshot {
                repo_path,
                mode,
                state,
                metadata,
                diffs,
                subscribers,
            } => {
                let snapshot = diff_state_proto::build_diff_state_snapshot(
                    repo_path,
                    mode,
                    metadata.as_ref(),
                    state,
                    diffs.as_deref(),
                );
                for (conn_id, request_id) in subscribers {
                    if let Some(request_id) = request_id {
                        self.send_server_message(
                            Some(*conn_id),
                            Some(request_id),
                            server_message::Message::GetDiffStateResponse(GetDiffStateResponse {
                                result: Some(get_diff_state_response::Result::Snapshot(
                                    snapshot.clone(),
                                )),
                            }),
                        );
                    } else {
                        self.send_server_message(
                            Some(*conn_id),
                            None,
                            server_message::Message::DiffStateSnapshot(snapshot.clone()),
                        );
                    }
                }
            }
            DiffStateUpdate::MetadataUpdate {
                repo_path,
                mode,
                metadata,
                subscribers,
            } => {
                let update = diff_state_proto::build_diff_state_metadata_update(
                    repo_path.as_str(),
                    mode,
                    metadata,
                );
                for conn_id in subscribers {
                    self.send_server_message(
                        Some(*conn_id),
                        None,
                        server_message::Message::DiffStateMetadataUpdate(update.clone()),
                    );
                }
            }
            DiffStateUpdate::FileDelta {
                repo_path,
                mode,
                path,
                diff,
                metadata,
                subscribers,
            } => {
                let delta = diff_state_proto::build_diff_state_file_delta(
                    repo_path.as_str(),
                    mode,
                    path,
                    diff.as_deref(),
                    metadata.as_ref(),
                );
                for conn_id in subscribers {
                    self.send_server_message(
                        Some(*conn_id),
                        None,
                        server_message::Message::DiffStateFileDelta(delta.clone()),
                    );
                }
            }
        }
    }

    /// Handles `GetBranches` — request/response.
    ///
    /// Runs `get_all_branches` on the remote filesystem and responds with
    /// the branch list.
    fn handle_get_branches(
        &mut self,
        msg: super::proto::GetBranches,
        request_id: &RequestId,
        conn_id: ConnectionId,
        ctx: &mut ModelContext<Self>,
    ) -> HandlerOutcome {
        let repo_path = match StandardizedPath::from_local_canonicalized(Path::new(&msg.repo_path))
        {
            Ok(p) => p.to_local_path_lossy(),
            Err(e) => {
                return HandlerOutcome::Sync(server_message::Message::GetBranchesResponse(
                    GetBranchesResponse {
                        result: Some(super::proto::get_branches_response::Result::Error(
                            GetBranchesError {
                                message: format!("Invalid repo_path: {e}"),
                            },
                        )),
                    },
                ));
            }
        };

        let max_branch_count = msg
            .max_branch_count
            .map(|c| (c as usize).min(MAX_BRANCH_COUNT_CAP));
        let include_remotes = msg.include_remotes;

        log::info!(
            "Handling GetBranches repo={} (request_id={request_id})",
            msg.repo_path,
        );

        let request_id_for_response = request_id.clone();
        let handle = self.spawn_request_handler(
            request_id.clone(),
            async move {
                crate::util::git::get_all_branches(&repo_path, max_branch_count, include_remotes)
                    .await
            },
            move |me, branches_result, _ctx| {
                let message = match branches_result {
                    Ok(branches) => {
                        server_message::Message::GetBranchesResponse(GetBranchesResponse {
                            result: Some(super::proto::get_branches_response::Result::Success(
                                GetBranchesSuccess {
                                    branches: branches
                                        .into_iter()
                                        .map(|entry| BranchInfo {
                                            name: entry.name,
                                            is_main: entry.is_main,
                                        })
                                        .collect(),
                                },
                            )),
                        })
                    }
                    Err(e) => server_message::Message::GetBranchesResponse(GetBranchesResponse {
                        result: Some(super::proto::get_branches_response::Result::Error(
                            GetBranchesError {
                                message: format!("{e:#}"),
                            },
                        )),
                    }),
                };
                me.send_server_message(Some(conn_id), Some(&request_id_for_response), message);
            },
            ctx,
        );
        HandlerOutcome::Async(Some(handle))
    }

    /// Handles `DiscardFilesRequest` — request/response.
    ///
    /// Runs git restore/stash on the remote filesystem for the specified files.
    /// The model's `discard_files` spawns async git operations internally.
    /// On success it reloads diffs, which triggers `NewDiffsComputed` pushes
    /// to subscribed connections. On failure it logs the error.
    ///
    /// We respond with success synchronously after delegating to the model,
    /// since `discard_files` does not surface completion status to the caller.
    fn handle_discard_files(
        &mut self,
        msg: super::proto::DiscardFilesRequest,
        request_id: &RequestId,
        ctx: &mut ModelContext<Self>,
    ) -> HandlerOutcome {
        log::info!(
            "Handling DiscardFiles repo={} files={} (request_id={request_id})",
            msg.repo_path,
            msg.files.len()
        );

        let std_path = match StandardizedPath::from_local_canonicalized(Path::new(&msg.repo_path)) {
            Ok(p) => p,
            Err(e) => {
                return HandlerOutcome::Sync(server_message::Message::Error(ErrorResponse {
                    code: ErrorCode::InvalidRequest.into(),
                    message: format!("Invalid repo_path: {e}"),
                }));
            }
        };

        let Some(mode_proto) = &msg.mode else {
            return HandlerOutcome::Sync(server_message::Message::Error(ErrorResponse {
                code: ErrorCode::InvalidRequest.into(),
                message: "Missing mode in DiscardFiles".to_string(),
            }));
        };

        let key = DiffModelKey {
            repo_path: std_path,
            mode: mode_proto.into(),
        };

        let model = self
            .diff_states
            .update(ctx, |mgr, _| mgr.get_model(&key).cloned());
        let Some(model) = model else {
            return HandlerOutcome::Sync(server_message::Message::DiscardFilesResponse(
                DiscardFilesResponse {
                    result: Some(discard_files_response::Result::Error(DiscardFilesError {
                        message: format!(
                            "No active diff state model for repo={} mode={:?}",
                            msg.repo_path, key.mode
                        ),
                    })),
                },
            ));
        };

        if msg.files.is_empty() {
            return HandlerOutcome::Sync(server_message::Message::DiscardFilesResponse(
                DiscardFilesResponse {
                    result: Some(discard_files_response::Result::Error(DiscardFilesError {
                        message: "No files specified in DiscardFilesRequest".to_string(),
                    })),
                },
            ));
        }

        let file_infos: Vec<_> = msg
            .files
            .iter()
            .filter_map(|f| match FileStatusInfo::try_from(f) {
                Ok(info) => Some(info),
                Err(e) => {
                    log::warn!("DiscardFiles: {e}");
                    None
                }
            })
            .collect();

        if file_infos.is_empty() {
            return HandlerOutcome::Sync(server_message::Message::DiscardFilesResponse(
                DiscardFilesResponse {
                    result: Some(discard_files_response::Result::Error(DiscardFilesError {
                        message: "No valid files after path validation".to_string(),
                    })),
                },
            ));
        }

        model.update(ctx, |m, ctx| {
            m.discard_files(file_infos, msg.should_stash, msg.branch_name, ctx);
        });

        HandlerOutcome::Sync(server_message::Message::DiscardFilesResponse(
            DiscardFilesResponse {
                result: Some(discard_files_response::Result::Success(
                    DiscardFilesSuccess {},
                )),
            },
        ))
    }
}

fn invalid_request_response(message: String) -> HandlerOutcome {
    HandlerOutcome::Sync(server_message::Message::Error(ErrorResponse {
        code: ErrorCode::InvalidRequest.into(),
        message,
    }))
}

fn codebase_index_status_response(status: CodebaseIndexStatus) -> HandlerOutcome {
    HandlerOutcome::Sync(server_message::Message::CodebaseIndexStatusUpdated(
        CodebaseIndexStatusUpdated {
            status: Some(status),
        },
    ))
}
fn requested_repo_path(repo_path: &str) -> Result<PathBuf, String> {
    if repo_path.is_empty() {
        return Err("repo_path is required".to_string());
    }
    StandardizedPath::from_local_canonicalized(Path::new(repo_path))
        .map(|path| path.to_local_path_lossy())
        .map_err(|error| format!("Invalid repo_path {repo_path}: {error}"))
}

fn canonicalize_index_repo_path(repo_path: &str) -> Result<PathBuf, String> {
    requested_repo_path(repo_path)?;
    let standardized_path = StandardizedPath::from_local_canonicalized(Path::new(repo_path))
        .map_err(|error| format!("Invalid repo_path {repo_path}: {error}"))?;
    Ok(standardized_path
        .to_local_path()
        .unwrap_or_else(|| standardized_path.to_local_path_lossy()))
}

fn missing_fragment_metadata(content_hash: String, message: String) -> MissingFragmentMetadata {
    MissingFragmentMetadata {
        content_hash,
        error: Some(FileOperationError { message }),
    }
}
fn fragment_metadata_lookup_error_response(
    code: FragmentMetadataLookupErrorCode,
    message: String,
    current_root_hash: Option<String>,
) -> HandlerOutcome {
    HandlerOutcome::Sync(
        server_message::Message::GetFragmentMetadataFromHashResponse(
            GetFragmentMetadataFromHashResponse {
                result: Some(get_fragment_metadata_from_hash_response::Result::Error(
                    ProtoFragmentMetadataLookupError {
                        code: code.into(),
                        message,
                        current_root_hash,
                    },
                )),
            },
        ),
    )
}

fn fragment_metadata_lookup_error_response_from_error(
    error: LocalFragmentMetadataLookupError,
) -> HandlerOutcome {
    let (code, message, current_root_hash) = match error {
        LocalFragmentMetadataLookupError::IndexNotFound => (
            FragmentMetadataLookupErrorCode::IndexNotFound,
            "Codebase index not found".to_string(),
            None,
        ),
        LocalFragmentMetadataLookupError::IndexNotSynced => (
            FragmentMetadataLookupErrorCode::IndexNotSynced,
            "Codebase index has no synced root hash".to_string(),
            None,
        ),
        LocalFragmentMetadataLookupError::RootHashMismatch { requested, current } => (
            FragmentMetadataLookupErrorCode::RootHashMismatch,
            format!("Codebase index root hash mismatch: requested {requested}, current {current}"),
            Some(current.to_string()),
        ),
    };

    fragment_metadata_lookup_error_response(code, message, current_root_hash)
}

fn fragment_metadata_to_proto(
    content_hash: &ContentHash,
    metadata: &LocalFragmentMetadata,
) -> ProtoFragmentMetadata {
    ProtoFragmentMetadata {
        content_hash: content_hash.to_string(),
        path: metadata.absolute_path.to_string_lossy().to_string(),
        start_line: metadata.location.start_line as u32,
        end_line: metadata.location.end_line as u32,
        byte_start: metadata.location.byte_range.start.as_usize() as u64,
        byte_end: metadata.location.byte_range.end.as_usize() as u64,
    }
}

/// Converts a [`ReadFileContextResult`] into its protobuf equivalent.
fn file_context_result_to_proto(result: ReadFileContextResult) -> ReadFileContextResponse {
    use crate::ai::agent::AnyFileContent;

    let file_contexts = result
        .file_contexts
        .into_iter()
        .map(|fc| {
            let content = match fc.content {
                AnyFileContent::StringContent(text) => {
                    super::proto::file_context_proto::Content::TextContent(text)
                }
                AnyFileContent::BinaryContent(bytes) => {
                    super::proto::file_context_proto::Content::BinaryContent(bytes)
                }
            };
            let last_modified_epoch_millis = fc
                .last_modified
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as u64);
            FileContextProto {
                file_name: fc.file_name,
                content: Some(content),
                line_range_start: fc.line_range.as_ref().map(|r| r.start as u32),
                line_range_end: fc.line_range.as_ref().map(|r| r.end as u32),
                last_modified_epoch_millis,
                line_count: fc.line_count as u32,
            }
        })
        .collect();

    let failed_files = result
        .missing_files
        .into_iter()
        .map(|path| FailedFileRead {
            path,
            error: Some(FileOperationError {
                message: "File not found or could not be read".to_string(),
            }),
        })
        .collect();

    ReadFileContextResponse {
        file_contexts,
        failed_files,
    }
}

#[cfg(test)]
#[path = "server_model_tests.rs"]
mod tests;
