use crate::terminal::shell::ShellType;
use repo_metadata::repositories::{DetectedRepositories, RepoDetectionSource};
use repo_metadata::{RepoMetadataEvent, RepoMetadataModel, RepositoryIdentifier};
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use warp_core::channel::ChannelState;
use warp_core::SessionId;
use warp_util::standardized_path::StandardizedPath;
use warpui::platform::TerminationMode;
use warpui::r#async::{Spawnable, SpawnableOutput, SpawnedFutureHandle};
use warpui::{Entity, ModelContext, SingletonEntity};

use warp_files::{FileModel, FileModelEvent};
use warp_util::content_version::ContentVersion;
use warp_util::file::FileId;

use super::proto::{
    client_message, delete_file_response, run_command_response, server_message,
    write_file_response, Abort, Authenticate, ClientMessage, DeleteFile, DeleteFileResponse,
    DeleteFileSuccess, ErrorCode, ErrorResponse, FailedFileRead, FileContextProto,
    FileOperationError, Initialize, InitializeResponse, NavigatedToDirectory,
    NavigatedToDirectoryResponse, ReadFileContextResponse, RunCommandError, RunCommandErrorCode,
    RunCommandRequest, RunCommandResponse, RunCommandSuccess, ServerMessage, SessionBootstrapped,
    WriteFile, WriteFileResponse, WriteFileSuccess,
};

/// How long the daemon waits with no connections before exiting.
pub const GRACE_PERIOD: std::time::Duration = std::time::Duration::from_secs(10 * 60);

/// Unique identifier for a connected proxy session in daemon mode.
pub type ConnectionId = uuid::Uuid;
use super::protocol::RequestId;
use crate::ai::agent::FileLocations;
use crate::ai::blocklist::{read_local_file_context, ReadFileContextResult};
use crate::terminal::model::session::command_executor::{
    ExecuteCommandOptions, LocalCommandExecutor,
};

/// Outcome of dispatching a request-style `ClientMessage`.
///
/// Notifications (fire-and-forget messages like `SessionBootstrapped` and
/// `Abort`) do not produce a `HandlerOutcome`; they are dispatched inline in
/// `handle_message` and return early.
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
    /// Daemon-wide bearer credential for the identity-scoped daemon.
    ///
    /// The token is written by Initialize when the client supplies a
    /// non-empty credential, or by Authenticate during token rotation. It is
    /// intentionally retained across proxy connection teardown and cleared
    /// only by daemon process exit.
    auth_token: Option<String>,
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
            auth_token: None,
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
                self.handle_initialize(msg, &request_id)
            }
            Some(client_message::Message::Authenticate(msg)) => {
                self.handle_authenticate(msg);
                return;
            }
            Some(client_message::Message::SessionBootstrapped(msg)) => {
                self.handle_session_bootstrapped(msg);
                return;
            }
            Some(client_message::Message::Abort(abort)) => {
                self.handle_abort(abort, &request_id);
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
    fn handle_initialize(&mut self, msg: Initialize, request_id: &RequestId) -> HandlerOutcome {
        log::info!("Handling Initialize (request_id={request_id})");
        if !msg.auth_token.is_empty() {
            self.auth_token = Some(msg.auth_token);
        }
        let server_version = ChannelState::app_version()
            .unwrap_or(env!("CARGO_PKG_VERSION"))
            .to_string();
        HandlerOutcome::Sync(server_message::Message::InitializeResponse(
            InitializeResponse {
                server_version,
                host_id: self.host_id.clone(),
            },
        ))
    }

    /// Handles `Authenticate` by replacing the daemon-wide credential.
    /// This is a notification — no response is sent.
    fn handle_authenticate(&mut self, msg: Authenticate) {
        if msg.auth_token.is_empty() {
            log::warn!("Received Authenticate notification with empty auth token; ignoring");
            return;
        }
        self.auth_token = Some(msg.auth_token);
    }

    pub fn auth_token(&self) -> Option<&str> {
        self.auth_token.as_deref()
    }

    /// Handles `Abort` by cancelling the in-progress request it targets.
    /// This is a notification — no response is sent.
    fn handle_abort(&mut self, abort: Abort, request_id: &RequestId) {
        let target_id = RequestId::from(abort.request_id_to_abort);
        if let Some(handle) = self.in_progress.remove(&target_id) {
            log::info!(
                "Aborting in-progress request (request_id={target_id}, \
                 abort_request_id={request_id})"
            );
            handle.abort();
        } else {
            log::info!(
                "Abort for unknown/completed request (request_id={target_id}, \
                 abort_request_id={request_id})"
            );
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
            log::error!(
                "Unknown shell_type {:?} in SessionBootstrapped for session {session_id:?}",
                msg.shell_type,
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
            log::error!("No executor for session {session_id:?}, session was never initialized");
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
                        log::info!(
                            "RunCommand completed (request_id={request_id_for_response}): \
                             exit_code={:?}, stdout_len={}, stderr_len={}",
                            output.exit_code,
                            output.stdout.len(),
                            output.stderr.len(),
                        );
                        run_command_response::Result::Success(RunCommandSuccess {
                            stdout: output.stdout.clone(),
                            stderr: output.stderr.clone(),
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
            repos.detect_possible_git_repo(&path_str, RepoDetectionSource::TerminalNavigation, ctx)
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
