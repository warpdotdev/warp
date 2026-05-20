//! Server-side diff state management.
//!
//! [`RemoteDiffStateManager`] is an entity that manages per-(repo, mode)
//! `LocalDiffStateModel` instances and tracks which connections are subscribed
//! to each. It owns model creation, event subscriptions, and content reload
//! spawning. `ServerModel` subscribes to its `DiffStateUpdate` events to
//! handle proto conversion and wire delivery.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use itertools::Itertools;
use warp_util::standardized_path::StandardizedPath;
use warpui::r#async::SpawnedFutureHandle;
use warpui::{AppContext, Entity, ModelContext, ModelHandle};

use crate::code_review::diff_state::{
    DiffMetadata, DiffMode, DiffState, DiffStateModelEvent, FileDiffAndContent,
    GitDiffWithBaseContent, LocalDiffStateModel,
};

use super::protocol::RequestId;
use super::server_model::ConnectionId;

// ── Key type ────────────────────────────────────────────────────────

/// Composite key: each (repo, mode) gets its own `LocalDiffStateModel`.
#[derive(Hash, Eq, PartialEq, Clone, Debug)]
pub(super) struct DiffModelKey {
    pub repo_path: StandardizedPath,
    pub mode: DiffMode,
}

// ── Pending response tracker ────────────────────────────────────────

/// Tracks a `GetDiffState` request that arrived while the model was still loading.
/// The response is sent once `NewDiffsComputed` fires.
pub(super) struct PendingDiffStateResponse {
    pub request_id: RequestId,
    pub conn_id: ConnectionId,
}

// ── Action / outcome types ─────────────────────────────────────

/// Outcome of [`RemoteDiffStateManager::subscribe`].
#[allow(clippy::large_enum_variant)]
pub(super) enum SubscribeOutcome {
    /// Respond with this snapshot immediately.
    RespondWithSnapshot {
        key: DiffModelKey,
        state: DiffState,
        metadata: Option<DiffMetadata>,
    },
    /// An async operation is in flight (content reload or model loading).
    /// The manager tracks spawned handles internally.
    Async,
}

/// Domain-level dispatch action returned by event processing and content
/// reload completion. Proto conversion is handled by `ServerModel` at
/// dispatch time.
pub(super) enum DiffStateUpdate {
    /// Build and send a snapshot to subscribers. Entries with a `request_id`
    /// receive a `GetDiffStateResponse`; entries without receive a
    /// server-initiated push `DiffStateSnapshot`.
    Snapshot {
        repo_path: String,
        mode: DiffMode,
        state: DiffState,
        metadata: Option<DiffMetadata>,
        diffs: Option<Arc<GitDiffWithBaseContent>>,
        /// Each subscriber is a connection plus an optional request ID.
        /// `Some(request_id)` → pending `GetDiffState` response (sent with request_id).
        /// `None` → already-subscribed connection that receives a server-initiated push.
        subscribers: Vec<(ConnectionId, Option<RequestId>)>,
    },
    /// Build and send a metadata update to all subscribers.
    MetadataUpdate {
        repo_path: StandardizedPath,
        mode: DiffMode,
        metadata: DiffMetadata,
        subscribers: Vec<ConnectionId>,
    },
    /// Build and send a single-file delta to all subscribers.
    FileDelta {
        repo_path: StandardizedPath,
        mode: DiffMode,
        path: PathBuf,
        diff: Option<Arc<FileDiffAndContent>>,
        metadata: Option<DiffMetadata>,
        subscribers: Vec<ConnectionId>,
    },
}

// ── RemoteDiffStateManager ────────────────────────────────────────

/// Manages the lifecycle of server-side `LocalDiffStateModel` instances and
/// per-connection subscription tracking.
///
/// A model is created when the first `GetDiffState` arrives for a given key
/// and dropped when the last connection unsubscribes (or disconnects).
pub(super) struct RemoteDiffStateManager {
    /// One model per (repo, mode). Mode is immutable — pinned at construction.
    states: HashMap<DiffModelKey, ModelHandle<LocalDiffStateModel>>,
    /// Per-key set of subscribed connections.
    key_to_connections: HashMap<DiffModelKey, HashSet<ConnectionId>>,
    /// Pending `GetDiffState` responses waiting for the model to finish loading.
    pending_responses: HashMap<DiffModelKey, Vec<PendingDiffStateResponse>>,
    /// In-progress content reload handles, keyed by request ID.
    in_progress: HashMap<RequestId, SpawnedFutureHandle>,
}

impl Entity for RemoteDiffStateManager {
    type Event = DiffStateUpdate;
}

impl RemoteDiffStateManager {
    pub fn new() -> Self {
        Self {
            states: HashMap::new(),
            key_to_connections: HashMap::new(),
            pending_responses: HashMap::new(),
            in_progress: HashMap::new(),
        }
    }

    // ── Model CRUD ──────────────────────────────────────────────────

    pub fn get_model(&self, key: &DiffModelKey) -> Option<&ModelHandle<LocalDiffStateModel>> {
        self.states.get(key)
    }

    pub fn insert_model(&mut self, key: DiffModelKey, model: ModelHandle<LocalDiffStateModel>) {
        self.states.insert(key, model);
    }

    pub fn remove_model(&mut self, key: &DiffModelKey) {
        self.states.remove(key);
        self.pending_responses.remove(key);
        self.key_to_connections.remove(key);
    }

    /// Reads the current `DiffState` and cloned `DiffMetadata` from the model
    /// for `key`. Returns `None` when the model is absent.
    pub fn read_state_and_metadata(
        &self,
        key: &DiffModelKey,
        app: &AppContext,
    ) -> Option<(DiffState, Option<DiffMetadata>)> {
        self.states.get(key).map(|model| {
            let m = model.as_ref(app);
            (m.get(), m.metadata().cloned())
        })
    }

    // ── Connection subscription tracking ────────────────────────────

    /// Records that `conn_id` is subscribed to `key`.
    pub fn subscribe_connection(&mut self, key: DiffModelKey, conn_id: ConnectionId) {
        self.key_to_connections
            .entry(key)
            .or_default()
            .insert(conn_id);
    }

    /// Removes `conn_id`'s subscription for `key`.
    /// If the key has zero remaining subscribers the model is dropped inline.
    pub fn unsubscribe_connection(&mut self, key: &DiffModelKey, conn_id: ConnectionId) {
        if let Some(pending) = self.pending_responses.get_mut(key) {
            pending.retain(|p| p.conn_id != conn_id);
        }

        if let Some(connections) = self.key_to_connections.get_mut(key) {
            connections.remove(&conn_id);
            if connections.is_empty() {
                self.remove_model(key);
            }
        }
    }

    /// Removes all subscriptions for a disconnected connection.
    /// Orphaned models (no remaining subscribers) are dropped inline.
    pub fn remove_connection(&mut self, conn_id: ConnectionId) {
        let keys = self
            .key_to_connections
            .iter()
            .filter(|(_, conns)| conns.contains(&conn_id))
            .map(|(key, _)| key.clone())
            .collect_vec();

        for key in keys {
            self.unsubscribe_connection(&key, conn_id);
        }
    }

    /// Returns the connection IDs subscribed to `key`.
    pub fn subscribed_connections(&self, key: &DiffModelKey) -> Vec<ConnectionId> {
        self.key_to_connections
            .get(key)
            .map(|conns| conns.iter().copied().collect())
            .unwrap_or_default()
    }

    // ── Pending response tracking ───────────────────────────────────

    /// Returns `true` if there are pending responses queued for `key`.
    pub fn has_pending_responses(&self, key: &DiffModelKey) -> bool {
        self.pending_responses
            .get(key)
            .is_some_and(|v| !v.is_empty())
    }

    /// Registers a pending `GetDiffState` response to be sent once the model loads.
    pub fn add_pending_response(
        &mut self,
        key: DiffModelKey,
        request_id: RequestId,
        conn_id: ConnectionId,
    ) {
        self.pending_responses
            .entry(key)
            .or_default()
            .push(PendingDiffStateResponse {
                request_id,
                conn_id,
            });
    }

    /// Drains all pending responses for `key`.
    pub fn drain_pending_responses(&mut self, key: &DiffModelKey) -> Vec<PendingDiffStateResponse> {
        self.pending_responses.remove(key).unwrap_or_default()
    }

    // ── High-level operations ────────────────────────────────────

    /// Handles a `GetDiffState` subscription request.
    ///
    /// Subscribes the connection, looks up or creates the model, and returns
    /// an outcome describing the result. When a content reload is needed it
    /// is spawned internally; when a new model is created the event
    /// subscription is wired up internally.
    pub fn subscribe(
        &mut self,
        repo_path: StandardizedPath,
        mode: DiffMode,
        request_id: &RequestId,
        conn_id: ConnectionId,
        ctx: &mut ModelContext<Self>,
    ) -> SubscribeOutcome {
        let key = DiffModelKey { repo_path, mode };
        self.subscribe_connection(key.clone(), conn_id);

        if let Some(model) = self.get_model(&key) {
            let model_ref = model.as_ref(ctx);
            let state = model_ref.get();
            match state {
                DiffState::Loaded => {
                    let already_in_flight = self.has_pending_responses(&key);
                    self.add_pending_response(key.clone(), request_id.clone(), conn_id);
                    if !already_in_flight {
                        self.spawn_content_reload(key, request_id, ctx);
                    }
                    SubscribeOutcome::Async
                }
                DiffState::Error(_) | DiffState::NotInRepository => {
                    SubscribeOutcome::RespondWithSnapshot {
                        key,
                        state,
                        metadata: model_ref.metadata().cloned(),
                    }
                }
                DiffState::Loading | DiffState::Disconnected => {
                    self.add_pending_response(key, request_id.clone(), conn_id);
                    SubscribeOutcome::Async
                }
            }
        } else {
            // Model doesn't exist — create it and wire up event subscription.
            let repo_path_str = key.repo_path.to_string();
            let mode = key.mode.clone();
            let model = ctx.add_model(|ctx| {
                let mut m = LocalDiffStateModel::new(Some(repo_path_str), ctx);
                m.set_diff_mode(mode, false, ctx);
                m.set_code_review_metadata_refresh_enabled(true, ctx);
                m
            });
            self.insert_model(key.clone(), model.clone());
            self.add_pending_response(key.clone(), request_id.clone(), conn_id);

            let key_for_sub = key;
            ctx.subscribe_to_model(&model, move |me, event, ctx| {
                me.handle_model_event(&key_for_sub, event, ctx);
            });

            SubscribeOutcome::Async
        }
    }

    /// Processes a `DiffStateModelEvent`, builds domain-level dispatch
    /// actions, and emits them as entity events for `ServerModel` to handle.
    fn handle_model_event(
        &mut self,
        key: &DiffModelKey,
        event: &DiffStateModelEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        match event {
            DiffStateModelEvent::NewDiffsComputed(diffs) => {
                let Some((state, metadata)) = self.read_state_and_metadata(key, ctx) else {
                    log::warn!("NewDiffsComputed for absent model key={key:?}");
                    return;
                };

                let pending = self.drain_pending_responses(key);
                let responded_conns: HashSet<ConnectionId> =
                    pending.iter().map(|p| p.conn_id).collect();
                let mut subscribers: Vec<(ConnectionId, Option<RequestId>)> = pending
                    .into_iter()
                    .map(|p| (p.conn_id, Some(p.request_id)))
                    .collect();
                subscribers.extend(
                    self.subscribed_connections(key)
                        .into_iter()
                        .filter(|c| !responded_conns.contains(c))
                        .map(|c| (c, None)),
                );

                ctx.emit(DiffStateUpdate::Snapshot {
                    repo_path: key.repo_path.to_string(),
                    mode: key.mode.clone(),
                    state,
                    metadata,
                    diffs: diffs.clone(),
                    subscribers,
                });
            }
            DiffStateModelEvent::MetadataRefreshed(metadata) => {
                ctx.emit(DiffStateUpdate::MetadataUpdate {
                    repo_path: key.repo_path.clone(),
                    mode: key.mode.clone(),
                    metadata: metadata.clone(),
                    subscribers: self.subscribed_connections(key),
                });
            }
            DiffStateModelEvent::CurrentBranchChanged => {
                let Some(model) = self.get_model(key) else {
                    return;
                };
                let Some(metadata) = model.as_ref(ctx).metadata() else {
                    return;
                };
                ctx.emit(DiffStateUpdate::MetadataUpdate {
                    repo_path: key.repo_path.clone(),
                    mode: key.mode.clone(),
                    metadata: metadata.clone(),
                    subscribers: self.subscribed_connections(key),
                });
            }
            DiffStateModelEvent::SingleFileUpdated { path, diff } => {
                let metadata = self
                    .get_model(key)
                    .and_then(|m| m.as_ref(ctx).metadata().cloned());
                ctx.emit(DiffStateUpdate::FileDelta {
                    repo_path: key.repo_path.clone(),
                    mode: key.mode.clone(),
                    path: path.clone(),
                    diff: diff.clone(),
                    metadata,
                    subscribers: self.subscribed_connections(key),
                });
            }
            DiffStateModelEvent::ConnectionLost => {
                // Client-only event — should not occur on the server side.
                log::warn!("Unexpected ConnectionLost event on server-side model key={key:?}");
            }
        }
    }

    /// Reads model state, drains pending responses, and emits a `Snapshot`
    /// dispatch so `ServerModel` can deliver the results to waiting clients.
    fn resolve_pending_responses(
        &mut self,
        key: &DiffModelKey,
        diffs: Option<GitDiffWithBaseContent>,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some((state, metadata)) = self.read_state_and_metadata(key, ctx) else {
            log::warn!("Content reload completed for absent model key={key:?}");
            return;
        };
        let diffs_arc = diffs.map(Arc::new);
        let subscribers = self
            .drain_pending_responses(key)
            .into_iter()
            .map(|p| (p.conn_id, Some(p.request_id)))
            .collect();

        ctx.emit(DiffStateUpdate::Snapshot {
            repo_path: key.repo_path.to_string(),
            mode: key.mode.clone(),
            state,
            metadata,
            diffs: diffs_arc,
            subscribers,
        });
    }

    /// Spawns an async diff reload with `content_at_base` for late-joining subscribers.
    fn spawn_content_reload(
        &mut self,
        key: DiffModelKey,
        request_id: &RequestId,
        ctx: &mut ModelContext<Self>,
    ) {
        let diff_mode = key.mode.clone();
        let repo_path = PathBuf::from(key.repo_path.as_str());
        let resolve_id = request_id.clone();
        let abort_id = request_id.clone();
        let handle = ctx.spawn_abortable(
            async move {
                LocalDiffStateModel::load_diffs_with_content_for_mode(diff_mode, repo_path).await
            },
            move |me, diffs, ctx| {
                me.in_progress.remove(&resolve_id);
                me.resolve_pending_responses(&key, diffs, ctx);
            },
            move |me, _ctx| {
                log::info!("Request cancelled (request_id={abort_id})");
                me.in_progress.remove(&abort_id);
            },
        );
        self.in_progress.insert(request_id.clone(), handle);
    }

    /// Cancels an in-progress content reload, if one exists for this request.
    /// Returns `true` if a request was found and aborted.
    pub fn abort_request(&mut self, request_id: &RequestId) -> bool {
        if let Some(handle) = self.in_progress.remove(request_id) {
            handle.abort();
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
#[path = "diff_state_tracker_tests.rs"]
mod tests;
