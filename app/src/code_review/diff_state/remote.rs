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

use instant::Instant;
use remote_server::manager::{RemoteServerManager, RemoteServerManagerEvent};
use warp_core::{send_telemetry_from_ctx, HostId};
use warp_util::remote_path::RemotePath;
use warp_util::standardized_path::StandardizedPath;
use warpui::{ModelContext, SingletonEntity};

use super::{
    DiffMetadata, DiffMode, DiffState, DiffStateModelEvent, DiffStats, FileDiffAndContent,
    GitDiffData, GitDiffWithBaseContent,
};
use crate::code_review::telemetry_event::CodeReviewTelemetryEvent;
use crate::remote_server::diff_state_proto::{try_decode_file_delta, try_decode_snapshot};
use crate::remote_server::proto;
use crate::util::git::{BranchEntry, Commit};

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
    /// Start time for the latest caller-tracked full diff snapshot request.
    tracked_diff_load_start_time: Option<Instant>,
}

impl warpui::Entity for RemoteDiffStateModel {
    type Event = DiffStateModelEvent;
}

impl RemoteDiffStateModel {
    /// Creates a new remote diff state model.
    ///
    /// Identity is `(host_id, repo_path, mode)`. The model is session-agnostic:
    /// the manager picks a connected session for the host at every outbound
    /// RPC, and host-level connect/disconnect events drive subscription
    /// lifecycle.
    ///
    /// A session for this host is required at construction time. The model starts in `Loading` and
    /// issues the initial `GetDiffState` request. Runtime disconnects transition the model through
    /// `mark_disconnected`; subsequent reconnects re-subscribe via the `HostConnected` event handler.
    pub fn new(remote_path: RemotePath, mode: DiffMode, ctx: &mut ModelContext<Self>) -> Self {
        // Subscribe to RemoteServerManager push events and filter by remote_path and diff_mode
        let mgr_handle = RemoteServerManager::handle(ctx);
        ctx.subscribe_to_model(&mgr_handle, Self::handle_manager_event);

        let host_id = remote_path.host_id.clone();
        let repo_path = remote_path.path.clone();
        let mode_clone = mode.clone();
        mgr_handle.update(ctx, |mgr, ctx| {
            mgr.get_diff_state(host_id, repo_path, proto::DiffMode::from(&mode_clone), ctx);
        });

        Self {
            remote_path,
            mode,
            state: InternalRemoteDiffState::Loading,
            metadata: None,
            tracked_diff_load_start_time: None,
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
            RemoteServerManagerEvent::GetBranchesResponse {
                repo_path, result, ..
            } if repo_path == &self.remote_path.path => {
                let branches = match result {
                    Ok(branch_infos) => branch_infos
                        .iter()
                        .map(|info| BranchEntry {
                            name: info.name.clone(),
                            is_main: info.is_main,
                        })
                        .collect(),
                    Err(err) => {
                        log::warn!("RemoteDiffStateModel: GetBranches failed: {err}");
                        vec![]
                    }
                };
                ctx.emit(DiffStateModelEvent::BranchesReceived(branches));
            }
            RemoteServerManagerEvent::HostDisconnected { host_id }
                if host_id == &self.remote_path.host_id =>
            {
                self.mark_disconnected(ctx);
            }
            RemoteServerManagerEvent::HostConnected { host_id }
                if host_id == &self.remote_path.host_id
                    && matches!(self.state, InternalRemoteDiffState::Disconnected) =>
            {
                self.resubscribe(false, ctx);
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
        self.tracked_diff_load_start_time = None;
        self.state = InternalRemoteDiffState::Disconnected;
        ctx.emit(DiffStateModelEvent::ConnectionLost);
    }

    /// Re-sends `GetDiffState` for this model's `(host_id, repo, mode)` and
    /// transitions to `Loading` while waiting for a fresh snapshot. The
    /// manager chooses a connected session for the host at dispatch time.
    fn resubscribe(&mut self, track_load_duration: bool, ctx: &mut ModelContext<Self>) {
        // Always overwrite to avoid carrying a stale `Instant` from a prior
        // tracked load that was interrupted by a session blip.
        self.tracked_diff_load_start_time = track_load_duration.then(Instant::now);
        let host_id = self.remote_path.host_id.clone();
        let repo_path = self.remote_path.path.clone();
        let mode = self.mode.clone();
        RemoteServerManager::handle(ctx).update(ctx, |mgr, ctx| {
            mgr.get_diff_state(host_id, repo_path, proto::DiffMode::from(&mode), ctx);
        });
        self.state = InternalRemoteDiffState::Loading;
        ctx.emit(DiffStateModelEvent::NewDiffsComputed {
            diffs: None,
            load_duration: None,
        });
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
                self.tracked_diff_load_start_time = None;
                warp_core::safe_error!(
                    safe: ("RemoteDiffStateModel: failed to decode diff state snapshot"),
                    full: ("RemoteDiffStateModel: failed to decode diff state snapshot: {error}")
                );
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
                warp_core::safe_error!(
                    safe: ("RemoteDiffStateModel: failed to decode diff state metadata update"),
                    full: ("RemoteDiffStateModel: failed to decode diff state metadata update: {error}")
                );
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
                warp_core::safe_error!(
                    safe: ("RemoteDiffStateModel: failed to decode diff state file delta"),
                    full: ("RemoteDiffStateModel: failed to decode diff state file delta: {error}")
                );
            }
        }
    }

    // ── Apply methods ──────────────────────────────────────────────────────

    /// Requests a fresh diff snapshot from the remote server, including file
    /// content. Unlike the former `replay_latest_diffs` (which reconstructed
    /// data from cached `GitDiffData` and lost `content_at_head`), this sends
    /// an actual `GetDiffState` RPC so the server can reload content from disk.
    ///
    /// Does NOT transition to `Loading` or emit `NewDiffsComputed(None)` first,
    /// so existing views subscribed to this model won't flash a loading state.
    /// The server response arrives as a `DiffStateSnapshotReceived` event and
    /// flows through `apply_snapshot` normally.
    pub(crate) fn fetch_fresh_snapshot(
        &mut self,
        track_load_duration: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        if track_load_duration {
            self.tracked_diff_load_start_time = Some(Instant::now());
        }
        let host_id = self.remote_path.host_id.clone();
        let repo_path = self.remote_path.path.clone();
        let mode = self.mode.clone();
        RemoteServerManager::handle(ctx).update(ctx, |mgr, ctx| {
            mgr.get_diff_state(host_id, repo_path, proto::DiffMode::from(&mode), ctx);
        });
    }

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
                self.tracked_diff_load_start_time = None;
                self.state = InternalRemoteDiffState::NotInRepository;
                ctx.emit(DiffStateModelEvent::NewDiffsComputed {
                    diffs: None,
                    load_duration: None,
                });
            }
            DiffState::Loading => {
                self.state = InternalRemoteDiffState::Loading;
                ctx.emit(DiffStateModelEvent::NewDiffsComputed {
                    diffs: None,
                    load_duration: None,
                });
            }
            DiffState::Error(msg) => {
                let load_duration = self
                    .tracked_diff_load_start_time
                    .take()
                    .map(|start| start.elapsed());
                send_telemetry_from_ctx!(
                    CodeReviewTelemetryEvent::LoadDiffFailed {
                        is_local: Some(false),
                        mode: self.mode.clone(),
                        error: "Server reported diff error".to_string(),
                        load_duration,
                    },
                    ctx
                );
                self.state = InternalRemoteDiffState::Error(msg);
                ctx.emit(DiffStateModelEvent::NewDiffsComputed {
                    diffs: None,
                    load_duration: None,
                });
            }
            DiffState::Loaded => {
                let Some(base_content) = diffs else {
                    let error =
                        "Server reported loaded state but no diff data was available".to_string();
                    let load_duration = self
                        .tracked_diff_load_start_time
                        .take()
                        .map(|start| start.elapsed());
                    send_telemetry_from_ctx!(
                        CodeReviewTelemetryEvent::LoadDiffFailed {
                            is_local: Some(false),
                            mode: self.mode.clone(),
                            error: "Empty diff data".to_string(),
                            load_duration,
                        },
                        ctx
                    );
                    self.state = InternalRemoteDiffState::Error(error);
                    ctx.emit(DiffStateModelEvent::NewDiffsComputed {
                        diffs: None,
                        load_duration: None,
                    });
                    return;
                };
                let diffs = GitDiffData::from(&base_content);
                let load_duration = self
                    .tracked_diff_load_start_time
                    .take()
                    .map(|start| start.elapsed());
                self.state = InternalRemoteDiffState::Loaded(diffs);
                ctx.emit(DiffStateModelEvent::NewDiffsComputed {
                    diffs: Some(Arc::new(base_content)),
                    load_duration,
                });
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
        ctx.emit(DiffStateModelEvent::MetadataRefreshed(Box::new(
            metadata.clone(),
        )));
    }

    fn apply_file_delta(
        &mut self,
        file_path: String,
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

        if let Some(ref new_diff) = diff {
            if let Some(pos) = diffs.files.iter().position(|f| f.file_path == file_path) {
                diffs.files[pos] = new_diff.file_diff.clone();
            } else {
                diffs.files.push(new_diff.file_diff.clone());
            }
        } else {
            diffs.files.retain(|f| f.file_path != file_path);
        }
        diffs.total_additions = diffs.files.iter().map(|f| f.additions()).sum();
        diffs.total_deletions = diffs.files.iter().map(|f| f.deletions()).sum();
        diffs.files_changed = diffs.files.len();
        ctx.emit(DiffStateModelEvent::SingleFileUpdated {
            path: file_path,
            diff: diff.map(Arc::new),
        });
    }

    // ── Cleanup ──────────────────────────────────────────────────────

    /// Sends `UnsubscribeDiffState` to the server. Call before dropping the
    /// model (the wrapper calls it during mode switch / pane close).
    pub fn unsubscribe(&self, ctx: &mut ModelContext<Self>) {
        RemoteServerManager::handle(ctx)
            .as_ref(ctx)
            .unsubscribe_diff_state(
                self.remote_path.host_id.clone(),
                &self.remote_path.path,
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

    pub fn is_git_operation_blocked(&self, _ctx: &warpui::AppContext) -> bool {
        false
    }

    pub fn has_head(&self) -> bool {
        self.metadata.as_ref().is_some_and(|m| m.has_head_commit)
    }

    pub fn remote_path(&self) -> RemotePath {
        self.remote_path.clone()
    }

    // ── Write API ────────────────────────────────────────────────────

    pub fn set_diff_mode(
        &mut self,
        mode: DiffMode,
        track_load_duration: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.mode == mode {
            return;
        }

        // Unsubscribe from the old mode before switching, then re-send
        // GetDiffState for the new mode. The manager resolves a connected
        // session for the host at dispatch time.
        self.unsubscribe(ctx);
        self.mode = mode;
        self.resubscribe(track_load_duration, ctx);
    }

    /// Fetches branches for the remote repository via the `GetBranches` RPC.
    /// The response is handled in `handle_manager_event` which emits
    /// `DiffStateModelEvent::BranchesReceived`.
    pub fn fetch_branches(&self, ctx: &mut ModelContext<Self>) {
        let host_id = self.remote_path.host_id.clone();
        let repo_path = self.remote_path.path.clone();
        RemoteServerManager::handle(ctx).update(ctx, |mgr, ctx| {
            mgr.get_branches(host_id, repo_path, None, false, ctx);
        });
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
        let host_id = self.remote_path.host_id.clone();
        let repo_path = self.remote_path.path.clone();
        let mode = self.mode.clone();
        let proto_files = file_infos.iter().map(proto::FileStatusInfo::from).collect();
        RemoteServerManager::handle(ctx).update(ctx, |mgr, ctx| {
            mgr.discard_files(
                host_id,
                repo_path,
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
