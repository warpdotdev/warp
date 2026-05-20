//! Remote diff state model.
//!
//! Client-side model for a single remote repository diff state subscription
//! received from the remote server. Presents the same read API as
//! `LocalDiffStateModel` and emits the same `DiffStateModelEvent` variants.
//!
//! The active [`DiffMode`] can change; the model handles this by unsubscribing
//! from the old `(repo_path, mode)` subscription and re-subscribing with the
//! new mode.

use std::sync::Arc;

use crate::remote_server::diff_state_proto::{try_decode_file_delta, try_decode_snapshot};
use crate::remote_server::proto;
use crate::util::git::{Commit, PrInfo};
use remote_server::manager::{RemoteServerManager, RemoteServerManagerEvent};
use warp_core::{HostId, SessionId};
use warp_util::remote_path::RemotePath;
use warp_util::standardized_path::StandardizedPath;
use warpui::{ModelContext, SingletonEntity};

use super::{
    DiffMetadata, DiffMode, DiffState, DiffStateModelEvent, DiffStats, FileDiffAndContent,
    GitDiffData, GitDiffWithBaseContent,
};

// ── Internal state ────────────────────────────────────────────────

#[derive(Default)]
enum InternalRemoteDiffState {
    #[default]
    Loading,
    NotInRepository,
    Loaded(GitDiffData),
    Error(String),
    /// The remote connection was lost. Preserves stale data until the model
    /// can re-establish the server-side subscription.
    Disconnected,
}

// ── Model ────────────────────────────────────────────────────────────────────

pub struct RemoteDiffStateModel {
    remote_path: RemotePath,
    mode: DiffMode,
    state: InternalRemoteDiffState,
    metadata: Option<DiffMetadata>,
    /// The session through which the current server-side subscription was established.
    session_id: SessionId,
}

impl warpui::Entity for RemoteDiffStateModel {
    type Event = DiffStateModelEvent;
}

impl RemoteDiffStateModel {
    /// Creates a new remote diff state model and initiates the `GetDiffState`
    /// request. The model starts in `Loading` state.
    pub fn new(
        remote_path: RemotePath,
        mode: DiffMode,
        session_id: SessionId,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        // Subscribe to RemoteServerManager push events and filter by remote_path and diff_mode
        let mgr_handle = RemoteServerManager::handle(ctx);
        ctx.subscribe_to_model(&mgr_handle, Self::handle_manager_event);

        // Send the initial GetDiffState request through the provided session.
        let remote_path_clone = remote_path.clone();
        let mode_clone = mode.clone();
        mgr_handle.update(ctx, |mgr, ctx| {
            mgr.get_diff_state(
                session_id,
                remote_path_clone,
                proto::DiffMode::from(&mode_clone),
                ctx,
            );
        });

        Self {
            remote_path,
            mode,
            state: InternalRemoteDiffState::Loading,
            metadata: None,
            session_id,
        }
    }

    // ── Event handler ───────────────────────────────────────────

    fn matches_remote_path_and_mode(
        &self,
        host_id: &HostId,
        repo_path: &StandardizedPath,
        mode: &proto::DiffMode,
    ) -> bool {
        let remote_mode = proto::DiffMode::from(&self.mode);
        host_id == &self.remote_path.host_id
            && repo_path == &self.remote_path.path
            && mode == &remote_mode
    }

    fn handle_manager_event(
        &mut self,
        event: &RemoteServerManagerEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        match event {
            RemoteServerManagerEvent::DiffStateSnapshotReceived {
                host_id,
                repo_path,
                mode,
                snapshot,
            } => {
                if !self.matches_remote_path_and_mode(host_id, repo_path, mode) {
                    return;
                }
                self.handle_snapshot_received(snapshot, ctx);
            }
            RemoteServerManagerEvent::DiffStateMetadataUpdateReceived {
                host_id,
                repo_path,
                mode,
                update,
            } => {
                if !self.matches_remote_path_and_mode(host_id, repo_path, mode) {
                    return;
                }
                self.handle_metadata_update_received(update, ctx);
            }
            RemoteServerManagerEvent::DiffStateFileDeltaReceived {
                host_id,
                repo_path,
                mode,
                delta,
            } => {
                if !self.matches_remote_path_and_mode(host_id, repo_path, mode) {
                    return;
                }
                self.handle_file_delta_received(delta, ctx);
            }
            RemoteServerManagerEvent::HostDisconnected { host_id }
                if host_id == &self.remote_path.host_id =>
            {
                self.mark_disconnected(ctx);
            }
            RemoteServerManagerEvent::SessionDisconnected {
                session_id,
                host_id,
                ..
            } if *session_id == self.session_id && host_id == &self.remote_path.host_id => {
                self.mark_disconnected(ctx);
            }
            RemoteServerManagerEvent::SessionReconnected {
                session_id,
                host_id,
                ..
            } if *session_id == self.session_id && host_id == &self.remote_path.host_id => {
                self.resubscribe(ctx);
            }
            _ => {}
        }
    }

    /// Marks the model as disconnected, preserving any stale data and
    /// emitting `ConnectionLost`.
    fn mark_disconnected(&mut self, ctx: &mut ModelContext<Self>) {
        if matches!(self.state, InternalRemoteDiffState::Disconnected) {
            return;
        }
        self.state = InternalRemoteDiffState::Disconnected;
        ctx.emit(DiffStateModelEvent::ConnectionLost);
    }

    /// Re-sends `GetDiffState` through the model's existing `session_id`
    /// and transitions to `Loading` while waiting for a fresh snapshot.
    fn resubscribe(&mut self, ctx: &mut ModelContext<Self>) {
        let remote_path = self.remote_path.clone();
        let mode = self.mode.clone();
        let session_id = self.session_id;
        RemoteServerManager::handle(ctx).update(ctx, |mgr, ctx| {
            mgr.get_diff_state(session_id, remote_path, proto::DiffMode::from(&mode), ctx);
        });
        self.state = InternalRemoteDiffState::Loading;
        ctx.emit(DiffStateModelEvent::NewDiffsComputed(None));
    }

    // ── Proto → state conversion helpers ────────────────────────────────────────────────

    fn handle_snapshot_received(
        &mut self,
        snapshot: &proto::DiffStateSnapshot,
        ctx: &mut ModelContext<Self>,
    ) {
        match try_decode_snapshot(snapshot) {
            Ok((metadata, state, diffs)) => self.apply_snapshot(metadata, state, diffs, ctx),
            Err(error) => {
                log::warn!("RemoteDiffStateModel: invalid diff state snapshot: {error}");
            }
        }
    }

    fn handle_metadata_update_received(
        &mut self,
        update: &proto::DiffStateMetadataUpdate,
        ctx: &mut ModelContext<Self>,
    ) {
        match update
            .metadata
            .as_ref()
            .map(DiffMetadata::try_from)
            .transpose()
        {
            Ok(Some(metadata)) => self.apply_metadata_update(&metadata, ctx),
            Ok(None) => {}
            Err(error) => {
                log::warn!("RemoteDiffStateModel: invalid diff state metadata update: {error}");
            }
        }
    }

    fn handle_file_delta_received(
        &mut self,
        delta: &proto::DiffStateFileDelta,
        ctx: &mut ModelContext<Self>,
    ) {
        match try_decode_file_delta(delta) {
            Ok((file_path, diff, metadata)) => {
                self.apply_file_delta(file_path, diff, metadata, ctx)
            }
            Err(error) => {
                log::warn!("RemoteDiffStateModel: invalid diff state file delta: {error}");
            }
        }
    }

    // ── Apply methods ──────────────────────────────────────────────────────

    fn apply_snapshot(
        &mut self,
        metadata: Option<DiffMetadata>,
        state: DiffState,
        diffs: Option<GitDiffWithBaseContent>,
        ctx: &mut ModelContext<Self>,
    ) {
        // Update metadata, detecting branch changes.
        if let Some(metadata) = &metadata {
            self.apply_metadata_update(metadata, ctx);
        }

        // Update state.
        match state {
            // Disconnected is never produced by proto deserialization.
            DiffState::Disconnected => {}
            DiffState::NotInRepository => {
                self.state = InternalRemoteDiffState::NotInRepository;
                ctx.emit(DiffStateModelEvent::NewDiffsComputed(None));
            }
            DiffState::Loading => {
                self.state = InternalRemoteDiffState::Loading;
                ctx.emit(DiffStateModelEvent::NewDiffsComputed(None));
            }
            DiffState::Error(msg) => {
                self.state = InternalRemoteDiffState::Error(msg);
                ctx.emit(DiffStateModelEvent::NewDiffsComputed(None));
            }
            DiffState::Loaded => {
                let Some(base_content) = diffs else {
                    self.state = InternalRemoteDiffState::Error(
                        "Server reported loaded state but no diff data was available".to_string(),
                    );
                    ctx.emit(DiffStateModelEvent::NewDiffsComputed(None));
                    return;
                };
                let diffs = GitDiffData::from(&base_content);
                self.state = InternalRemoteDiffState::Loaded(diffs);
                ctx.emit(DiffStateModelEvent::NewDiffsComputed(Some(Arc::new(
                    base_content,
                ))));
            }
        }
    }

    fn apply_metadata_update(&mut self, metadata: &DiffMetadata, ctx: &mut ModelContext<Self>) {
        let previous_branch = self
            .metadata
            .as_ref()
            .map(|m| m.current_branch_name.as_str());
        let branch_changed =
            previous_branch.is_some_and(|prev| prev != metadata.current_branch_name.as_str());
        self.metadata = Some(metadata.clone());

        // Only emit CurrentBranchChanged when there was a previous branch to
        // compare against. On the first metadata update (initial snapshot)
        // previous_branch is None — that's initial population, not a switch.
        if branch_changed {
            ctx.emit(DiffStateModelEvent::CurrentBranchChanged);
        }
        ctx.emit(DiffStateModelEvent::MetadataRefreshed(metadata.clone()));
    }

    fn apply_file_delta(
        &mut self,
        file_path: StandardizedPath,
        diff: Option<FileDiffAndContent>,
        metadata: Option<DiffMetadata>,
        ctx: &mut ModelContext<Self>,
    ) {
        if let Some(metadata) = &metadata {
            self.apply_metadata_update(metadata, ctx);
        }

        let InternalRemoteDiffState::Loaded(ref mut diffs) = self.state else {
            // Ignore file deltas until the initial snapshot has loaded.
            return;
        };

        let event_path = file_path.to_local_path_lossy();

        if let Some(ref new_diff) = diff {
            if let Some(pos) = diffs.files.iter().position(|f| f.file_path == event_path) {
                diffs.files[pos] = new_diff.file_diff.clone();
            } else {
                diffs.files.push(new_diff.file_diff.clone());
            }
        } else {
            diffs.files.retain(|f| f.file_path != event_path);
        }
        diffs.total_additions = diffs.files.iter().map(|f| f.additions()).sum();
        diffs.total_deletions = diffs.files.iter().map(|f| f.deletions()).sum();
        diffs.files_changed = diffs.files.len();
        ctx.emit(DiffStateModelEvent::SingleFileUpdated {
            path: event_path,
            diff: diff.map(Arc::new),
        });
    }

    // ── Cleanup ──────────────────────────────────────────────────────

    /// Sends `UnsubscribeDiffState` to the server. Call before dropping the
    /// model (the wrapper calls it during mode switch / pane close).
    pub fn unsubscribe(&self, ctx: &mut ModelContext<Self>) {
        let mgr_handle = RemoteServerManager::handle(ctx);
        let mgr = mgr_handle.as_ref(ctx);
        if mgr.client_for_session(self.session_id).is_none() {
            log::debug!(
                "RemoteDiffStateModel::unsubscribe: subscription session is no longer connected: session={:?}",
                self.session_id,
            );
            return;
        }
        mgr.unsubscribe_diff_state(
            self.session_id,
            &self.remote_path,
            proto::DiffMode::from(&self.mode),
        );
    }

    // ── Read API (matching LocalDiffStateModel interface) ────────────

    pub fn get(&self) -> DiffState {
        match &self.state {
            InternalRemoteDiffState::NotInRepository => DiffState::NotInRepository,
            InternalRemoteDiffState::Loading => DiffState::Loading,
            InternalRemoteDiffState::Loaded(_) => DiffState::Loaded,
            InternalRemoteDiffState::Error(msg) => DiffState::Error(msg.clone()),
            InternalRemoteDiffState::Disconnected => DiffState::Disconnected,
        }
    }

    pub fn diff_mode(&self) -> DiffMode {
        self.mode.clone()
    }

    pub fn get_uncommitted_stats(&self) -> Option<DiffStats> {
        self.metadata
            .as_ref()
            .map(|m| m.against_head.aggregate_stats)
    }

    pub fn get_main_branch_name(&self) -> Option<String> {
        self.metadata
            .as_ref()
            .map(|m| m.main_branch_name.clone())
            .filter(|s| !s.is_empty())
    }

    pub fn get_current_branch_name(&self) -> Option<String> {
        self.metadata
            .as_ref()
            .map(|m| m.current_branch_name.clone())
            .filter(|s| !s.is_empty())
    }

    pub fn is_on_main_branch(&self) -> bool {
        self.metadata.as_ref().is_some_and(|m| {
            !m.current_branch_name.is_empty() && m.current_branch_name == m.main_branch_name
        })
    }

    pub fn unpushed_commits(&self) -> &[Commit] {
        self.metadata
            .as_ref()
            .map(|m| m.unpushed_commits.as_slice())
            .unwrap_or(&[])
    }

    pub fn upstream_ref(&self) -> Option<&str> {
        self.metadata
            .as_ref()
            .and_then(|m| m.upstream_ref.as_deref())
    }

    pub fn upstream_differs_from_main(&self) -> bool {
        match (self.upstream_ref(), self.get_main_branch_name().as_deref()) {
            (Some(upstream), Some(main)) => upstream != main,
            _ => false,
        }
    }

    pub fn pr_info(&self) -> Option<&PrInfo> {
        self.metadata.as_ref().and_then(|m| m.pr_info.as_ref())
    }

    pub fn is_pr_info_refreshing(&self) -> bool {
        false
    }

    pub fn is_git_operation_blocked(&self, _ctx: &warpui::AppContext) -> bool {
        false
    }

    pub fn has_head(&self) -> bool {
        self.metadata.as_ref().is_some_and(|m| m.has_head_commit)
    }

    pub fn remote_path(&self) -> RemotePath {
        self.remote_path.clone()
    }

    /// Returns the session this model's subscription is anchored to. Set
    /// once at construction and never changed by the model itself — see
    /// the `session_id` field doc for the lifecycle contract.
    pub fn session_id(&self) -> SessionId {
        self.session_id
    }

    // ── Write API ────────────────────────────────────────────────────

    pub fn set_diff_mode(&mut self, mode: DiffMode, ctx: &mut ModelContext<Self>) {
        if self.mode == mode {
            return;
        }

        // Unsubscribe from the old mode before switching, then re-send
        // GetDiffState for the new mode through the same session.
        self.unsubscribe(ctx);
        self.mode = mode;
        self.resubscribe(ctx);
    }

    /// Sends a `DiscardFiles` request to the remote server.
    /// The server's watcher will push updated diff snapshots on success.
    pub fn discard_files(
        &self,
        file_infos: Vec<super::FileStatusInfo>,
        should_stash: bool,
        branch_name: Option<String>,
        ctx: &mut ModelContext<Self>,
    ) {
        let session_id = self.session_id;
        let remote_path = self.remote_path.clone();
        let mode = self.mode.clone();
        let proto_files = file_infos.iter().map(proto::FileStatusInfo::from).collect();
        RemoteServerManager::handle(ctx).update(ctx, |mgr, ctx| {
            mgr.discard_files(
                session_id,
                remote_path,
                proto_files,
                should_stash,
                branch_name,
                proto::DiffMode::from(&mode),
                ctx,
            );
        });
    }
}

#[cfg(test)]
#[path = "remote_tests.rs"]
mod remote_tests;
