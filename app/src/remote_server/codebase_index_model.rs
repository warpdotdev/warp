use std::collections::HashMap;
use std::str::FromStr;

use ai::index::full_source_code_embedding::NodeHash;
use remote_server::codebase_index_proto::{RemoteCodebaseIndexState, RemoteCodebaseIndexStatus};
use warp_core::{HostId, SessionId};
use warp_util::remote_path::RemotePath;
use warp_util::standardized_path::StandardizedPath;
use warpui::{Entity, ModelContext, SingletonEntity};

use super::manager::{
    RemoteCodebaseIndexStatusWithPath, RemoteCodebaseIndexUpdateOperation, RemoteServerManager,
    RemoteServerManagerEvent,
};
use crate::ai::blocklist::SessionContext;
use crate::ai::codebase_auto_indexing::{
    CodebaseAutoIndexingSurface, auto_index_candidate_roots, should_auto_index_codebase,
    should_use_codebase_indexing,
};
use crate::server::telemetry::{
    RemoteCodebaseAutoIndexTrigger, RemoteCodebaseIndexStatusTelemetrySource,
};
use crate::workspaces::user_workspaces::{UserWorkspaces, UserWorkspacesEvent};
use crate::{TelemetryEvent, send_telemetry_from_ctx};

#[derive(Clone, Debug)]
pub struct RemoteCodebaseSearchContext {
    pub remote_path: RemotePath,
    pub root_hash: NodeHash,
    pub is_stale: bool,
}

#[derive(Clone, Debug)]
pub enum RemoteCodebaseSearchAvailability {
    NoConnectedHost,
    NoActiveRepo,
    NotIndexed {
        remote_path: RemotePath,
    },
    Indexing {
        remote_path: RemotePath,
    },
    Unavailable {
        remote_path: RemotePath,
        message: String,
    },
    Ready(RemoteCodebaseSearchContext),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteCodebaseContextEntry {
    pub name: String,
    pub path: String,
}

impl RemoteCodebaseSearchAvailability {
    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready(_))
    }

    fn repo_path(&self) -> Option<&str> {
        match self {
            Self::NoConnectedHost | Self::NoActiveRepo => None,
            Self::NotIndexed { remote_path }
            | Self::Indexing { remote_path }
            | Self::Unavailable { remote_path, .. } => Some(remote_path.path.as_str()),
            Self::Ready(context) => Some(context.remote_path.path.as_str()),
        }
    }
}

fn remote_path_from_repo_path(host_id: &HostId, repo_path: &str) -> Option<RemotePath> {
    StandardizedPath::try_new(repo_path)
        .ok()
        .map(|path| RemotePath::new(host_id.clone(), path))
}

fn remote_codebase_name(repo_path: &str) -> String {
    repo_path
        .rsplit('/')
        .find(|segment| !segment.is_empty())
        .unwrap_or(repo_path)
        .to_string()
}
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct HostLabel {
    label: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct PathAtHost {
    host: HostLabel,
    path: StandardizedPath,
}

#[derive(Default)]
pub struct RemoteCodebaseIndexModel {
    statuses: HashMap<PathAtHost, RemoteCodebaseIndexStatus>,
    active_repos_by_host: HashMap<HostId, RemotePath>,
    host_labels: HashMap<HostId, HostLabel>,
    active_git_repos_by_session: HashMap<SessionId, RemotePath>,
    last_git_repos_by_host: HashMap<HostId, RemotePath>,
}

#[derive(Clone, Debug)]
pub enum RemoteCodebaseIndexModelEvent {
    SettingsEntriesChanged,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteCodebaseIndexSettingsEntry {
    pub remote_path: RemotePath,
    pub status: RemoteCodebaseIndexStatus,
    pub host_label: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RemoteCodebaseIndexStatusTelemetryUpdate {
    state: RemoteCodebaseIndexState,
    previous_state: Option<RemoteCodebaseIndexState>,
    has_root_hash: bool,
    has_failure_message: bool,
    progress_completed: Option<u64>,
    progress_total: Option<u64>,
}

impl RemoteCodebaseIndexStatusTelemetryUpdate {
    fn new(
        status: &RemoteCodebaseIndexStatus,
        previous_state: Option<RemoteCodebaseIndexState>,
    ) -> Self {
        Self {
            state: status.state,
            previous_state,
            has_root_hash: status
                .root_hash
                .as_deref()
                .is_some_and(|root_hash| !root_hash.is_empty()),
            has_failure_message: status
                .failure_message
                .as_deref()
                .is_some_and(|message| !message.is_empty()),
            progress_completed: status.progress_completed,
            progress_total: status.progress_total,
        }
    }
}

impl RemoteCodebaseIndexModel {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        let manager = RemoteServerManager::handle(ctx);
        ctx.subscribe_to_model(&manager, |me, _, event, ctx| {
            me.handle_remote_server_manager_event(event, ctx);
        });

        let user_workspaces = UserWorkspaces::handle(ctx);
        ctx.subscribe_to_model(&user_workspaces, |me, _, event, ctx| {
            if let UserWorkspacesEvent::CodebaseContextEnablementChanged = event {
                me.handle_codebase_context_enablement_changed(ctx);
            }
        });
        Self::default()
    }

    fn host_label_for_host(&self, host_id: &HostId) -> HostLabel {
        self.host_labels
            .get(host_id)
            .cloned()
            .unwrap_or_else(|| HostLabel {
                label: host_id.to_string(),
            })
    }

    fn status_key_for_remote_path(&self, remote_path: &RemotePath) -> PathAtHost {
        PathAtHost {
            host: self.host_label_for_host(&remote_path.host_id),
            path: remote_path.path.clone(),
        }
    }

    fn remote_path_for_status_key(&self, key: &PathAtHost) -> RemotePath {
        RemotePath::new(self.host_id_for_label(&key.host), key.path.clone())
    }

    fn host_id_for_label(&self, host_label: &HostLabel) -> HostId {
        self.host_labels
            .iter()
            .find_map(|(host_id, label)| (label == host_label).then_some(host_id.clone()))
            .unwrap_or_else(|| HostId::new(host_label.label.clone()))
    }

    fn move_statuses_to_resolved_host_label(
        &mut self,
        old_host_label: HostLabel,
        new_host_label: HostLabel,
    ) -> bool {
        if old_host_label == new_host_label {
            return false;
        }

        let mut moved_statuses = vec![];
        self.statuses.retain(|key, status| {
            if key.host == old_host_label {
                moved_statuses.push((
                    PathAtHost {
                        host: new_host_label.clone(),
                        path: key.path.clone(),
                    },
                    status.clone(),
                ));
                false
            } else {
                true
            }
        });

        let statuses_moved = !moved_statuses.is_empty();
        for (key, status) in moved_statuses {
            self.statuses.entry(key).or_insert(status);
        }
        statuses_moved
    }

    pub fn active_repo_availability(
        &self,
        session_context: &SessionContext,
        explicit_repo_path: Option<&str>,
    ) -> RemoteCodebaseSearchAvailability {
        let Some(host_id) = session_context.host_id() else {
            return RemoteCodebaseSearchAvailability::NoConnectedHost;
        };

        self.availability_for_remote(
            host_id,
            session_context.current_working_directory().as_deref(),
            explicit_repo_path,
        )
    }

    pub fn active_repo_path(
        &self,
        session_context: &SessionContext,
        explicit_repo_path: Option<&str>,
    ) -> Option<String> {
        self.active_repo_availability(session_context, explicit_repo_path)
            .repo_path()
            .map(ToOwned::to_owned)
    }

    pub fn request_active_repo_index(
        &self,
        session_context: &SessionContext,
        explicit_repo_path: Option<&str>,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        if !should_use_codebase_indexing(CodebaseAutoIndexingSurface::Remote, ctx) {
            return false;
        }
        let Some(host_id) = session_context.host_id() else {
            return false;
        };
        let Some(remote_path) = self.resolve_remote_repo_path(
            host_id,
            session_context.current_working_directory().as_deref(),
            explicit_repo_path,
        ) else {
            return false;
        };

        RemoteServerManager::handle(ctx).update(ctx, |manager, ctx| {
            manager.ensure_codebase_indexed(
                remote_path,
                RemoteCodebaseIndexUpdateOperation::IndexNewRepo {
                    is_auto_index: false,
                },
                ctx,
            );
        });
        true
    }

    pub fn codebases_for_agent_context(&self, host_id: &HostId) -> Vec<RemoteCodebaseContextEntry> {
        let host_label = self.host_label_for_host(host_id);
        let mut entries = self
            .statuses
            .iter()
            .filter(|&(key, status)| {
                key.host == host_label
                    && search_availability_for_status(
                        status,
                        RemotePath::new(host_id.clone(), key.path.clone()),
                    )
                    .is_ready()
            })
            .map(|(key, _)| {
                let path = key.path.as_str().to_string();
                RemoteCodebaseContextEntry {
                    name: remote_codebase_name(&path),
                    path,
                }
            })
            .collect::<Vec<_>>();
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        entries
    }

    pub fn request_index(&self, remote_path: RemotePath, ctx: &mut ModelContext<Self>) {
        if !should_use_codebase_indexing(CodebaseAutoIndexingSurface::Remote, ctx) {
            return;
        }
        RemoteServerManager::handle(ctx).update(ctx, |manager, ctx| {
            manager.ensure_codebase_indexed(
                remote_path,
                RemoteCodebaseIndexUpdateOperation::IndexNewRepo {
                    is_auto_index: false,
                },
                ctx,
            );
        });
    }

    pub fn resync_index(&self, remote_path: RemotePath, ctx: &mut ModelContext<Self>) {
        if !should_use_codebase_indexing(CodebaseAutoIndexingSurface::Remote, ctx) {
            return;
        }
        RemoteServerManager::handle(ctx).update(ctx, |manager, ctx| {
            manager.resync_codebase(remote_path, ctx);
        });
    }

    pub fn drop_index(&self, remote_path: RemotePath, ctx: &mut ModelContext<Self>) {
        RemoteServerManager::handle(ctx).update(ctx, |manager, ctx| {
            manager.drop_codebase_index(remote_path, ctx);
        });
    }

    pub fn entries_for_settings(&self) -> Vec<RemoteCodebaseIndexSettingsEntry> {
        let mut entries = self
            .statuses
            .iter()
            .map(|(key, status)| RemoteCodebaseIndexSettingsEntry {
                remote_path: self.remote_path_for_status_key(key),
                status: status.clone(),
                host_label: key.host.label.clone(),
            })
            .collect::<Vec<_>>();
        entries.sort_by(|a, b| {
            a.host_label
                .cmp(&b.host_label)
                .then_with(|| a.remote_path.path.as_str().cmp(b.remote_path.path.as_str()))
        });
        entries
    }
    fn handle_remote_server_manager_event(
        &mut self,
        event: &RemoteServerManagerEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        match event {
            RemoteServerManagerEvent::CodebaseIndexStatusesSnapshot { host_id, statuses } => {
                if !should_use_codebase_indexing(CodebaseAutoIndexingSurface::Remote, ctx) {
                    return;
                }
                let (changed, telemetry_updates) =
                    self.apply_statuses_snapshot_with_telemetry(host_id, statuses);
                for update in telemetry_updates {
                    emit_status_changed_telemetry(
                        update,
                        None,
                        RemoteCodebaseIndexStatusTelemetrySource::Snapshot,
                        ctx,
                    );
                }
                if changed {
                    ctx.emit(RemoteCodebaseIndexModelEvent::SettingsEntriesChanged);
                }
            }
            RemoteServerManagerEvent::CodebaseIndexStatusUpdated {
                remote_path,
                status,
                mutation_kind,
                session_id: _,
            } => {
                if !should_use_codebase_indexing(CodebaseAutoIndexingSurface::Remote, ctx) {
                    return;
                }
                if let Some(update) =
                    self.apply_status_update_with_telemetry(remote_path.clone(), status.clone())
                {
                    let source = if mutation_kind.is_some() {
                        RemoteCodebaseIndexStatusTelemetrySource::MutationResponse
                    } else {
                        RemoteCodebaseIndexStatusTelemetrySource::PushUpdate
                    };
                    emit_status_changed_telemetry(update, *mutation_kind, source, ctx);
                    ctx.emit(RemoteCodebaseIndexModelEvent::SettingsEntriesChanged);
                }
            }
            RemoteServerManagerEvent::NavigatedToDirectory {
                session_id,
                remote_path,
                is_git,
            } => {
                self.record_navigated_directory(*session_id, remote_path, *is_git);
                if *is_git
                    && should_auto_index_codebase(CodebaseAutoIndexingSurface::Remote, ctx)
                    && self.should_request_auto_index_for_navigated_git_repo(remote_path)
                {
                    // Mirrors local auto-indexing: remote navigation silently requests indexing
                    // only when the shared auto-index setting allows it.
                    let remote_path = remote_path.clone();
                    emit_auto_index_requested_telemetry(
                        RemoteCodebaseAutoIndexTrigger::NavigatedToGitRepo,
                        1,
                        ctx,
                    );
                    RemoteServerManager::handle(ctx).update(ctx, |manager, ctx| {
                        manager.ensure_codebase_indexed(
                            remote_path,
                            RemoteCodebaseIndexUpdateOperation::IndexNewRepo {
                                is_auto_index: true,
                            },
                            ctx,
                        );
                    });
                }
            }
            RemoteServerManagerEvent::HostDisconnected { host_id } => {
                if self.mark_host_unavailable(host_id) {
                    ctx.emit(RemoteCodebaseIndexModelEvent::SettingsEntriesChanged);
                }
            }
            RemoteServerManagerEvent::SessionConnected {
                session_id: _,
                host_id,
            }
            | RemoteServerManagerEvent::SessionReconnected {
                session_id: _,
                host_id,
                attempt: _,
                client: _,
            } => {
                if self.record_host_label(host_id, ctx) {
                    ctx.emit(RemoteCodebaseIndexModelEvent::SettingsEntriesChanged);
                }
            }
            RemoteServerManagerEvent::SessionDisconnected { session_id, .. }
            | RemoteServerManagerEvent::SessionDeregistered { session_id } => {
                self.clear_active_git_repo_for_session(*session_id);
            }
            RemoteServerManagerEvent::SessionConnecting { .. }
            | RemoteServerManagerEvent::SessionConnectionFailed { .. }
            | RemoteServerManagerEvent::HostConnected { .. }
            | RemoteServerManagerEvent::RemoteAgentContextSnapshot { .. }
            | RemoteServerManagerEvent::RepoMetadataSnapshot { .. }
            | RemoteServerManagerEvent::RepoMetadataUpdated { .. }
            | RemoteServerManagerEvent::RepoMetadataDirectoryLoaded { .. }
            | RemoteServerManagerEvent::BufferUpdated { .. }
            | RemoteServerManagerEvent::BufferConflictDetected { .. }
            | RemoteServerManagerEvent::DiffStateSnapshotReceived { .. }
            | RemoteServerManagerEvent::DiffStateMetadataUpdateReceived { .. }
            | RemoteServerManagerEvent::DiffStateFileDeltaReceived { .. }
            | RemoteServerManagerEvent::GitStatusPushReceived { .. }
            | RemoteServerManagerEvent::GitHubPrInfoPushReceived { .. }
            | RemoteServerManagerEvent::GitHubRepositoryInfoPushReceived { .. }
            | RemoteServerManagerEvent::GetBranchesResponse { .. }
            | RemoteServerManagerEvent::CommitChainResponse { .. }
            | RemoteServerManagerEvent::GitPushResponse { .. }
            | RemoteServerManagerEvent::CreatePrResponse { .. }
            | RemoteServerManagerEvent::GenerateCommitMessageResponse { .. }
            | RemoteServerManagerEvent::GetCommittedBranchFilesResponse { .. }
            | RemoteServerManagerEvent::SetupStateChanged { .. }
            | RemoteServerManagerEvent::BinaryCheckComplete { .. }
            | RemoteServerManagerEvent::BinaryInstallComplete { .. }
            | RemoteServerManagerEvent::ClientRequestFailed { .. }
            | RemoteServerManagerEvent::CodebaseIndexMutationFailed { .. }
            | RemoteServerManagerEvent::ServerMessageDecodingError { .. } => {}
        }
    }

    fn handle_codebase_context_enablement_changed(&mut self, ctx: &mut ModelContext<Self>) {
        if !should_use_codebase_indexing(CodebaseAutoIndexingSurface::Remote, ctx) {
            let remote_paths = self.clear_remote_codebase_indexing_state();
            if !remote_paths.is_empty() {
                ctx.emit(RemoteCodebaseIndexModelEvent::SettingsEntriesChanged);
            }
            for remote_path in remote_paths {
                RemoteServerManager::handle(ctx).update(ctx, |manager, ctx| {
                    manager.drop_codebase_index(remote_path, ctx);
                });
            }
            return;
        }

        let remote_paths = self.active_git_repo_paths_needing_auto_index();
        if remote_paths.is_empty()
            || !should_auto_index_codebase(CodebaseAutoIndexingSurface::Remote, ctx)
        {
            return;
        }

        emit_auto_index_requested_telemetry(
            RemoteCodebaseAutoIndexTrigger::CodebaseContextEnablementChanged,
            remote_paths.len(),
            ctx,
        );

        for remote_path in remote_paths {
            RemoteServerManager::handle(ctx).update(ctx, |manager, ctx| {
                manager.ensure_codebase_indexed(
                    remote_path,
                    RemoteCodebaseIndexUpdateOperation::IndexNewRepo {
                        is_auto_index: true,
                    },
                    ctx,
                );
            });
        }
    }

    fn clear_remote_codebase_indexing_state(&mut self) -> Vec<RemotePath> {
        let statuses = std::mem::take(&mut self.statuses);
        statuses
            .into_keys()
            .map(|key| self.remote_path_for_status_key(&key))
            .collect()
    }
    fn should_request_auto_index_for_navigated_git_repo(&self, remote_path: &RemotePath) -> bool {
        let Some(status) = self.status_for_repo(remote_path) else {
            return true;
        };

        match search_availability_for_status(status, remote_path.clone()) {
            RemoteCodebaseSearchAvailability::Ready(_)
            | RemoteCodebaseSearchAvailability::Indexing { .. } => false,
            RemoteCodebaseSearchAvailability::NoConnectedHost
            | RemoteCodebaseSearchAvailability::NoActiveRepo
            | RemoteCodebaseSearchAvailability::NotIndexed { .. }
            | RemoteCodebaseSearchAvailability::Unavailable { .. } => true,
        }
    }

    fn active_git_repo_paths_needing_auto_index(&self) -> Vec<RemotePath> {
        auto_index_candidate_roots(
            self.active_git_repos_by_session.values().cloned(),
            |remote_path| self.should_request_auto_index_for_navigated_git_repo(remote_path),
        )
    }

    fn apply_statuses_snapshot(
        &mut self,
        host_id: &HostId,
        statuses: &[RemoteCodebaseIndexStatusWithPath],
    ) -> bool {
        self.apply_statuses_snapshot_with_telemetry(host_id, statuses)
            .0
    }

    fn apply_statuses_snapshot_with_telemetry(
        &mut self,
        host_id: &HostId,
        statuses: &[RemoteCodebaseIndexStatusWithPath],
    ) -> (bool, Vec<RemoteCodebaseIndexStatusTelemetryUpdate>) {
        let status_count = statuses.len();
        log::info!(
            "[Remote codebase indexing] Client received bootstrap codebase index statuses snapshot: host_id={host_id} status_count={status_count}"
        );
        for status_with_path in statuses {
            log::debug!(
                "[Remote codebase indexing] Client received bootstrap codebase index status: repo_path={} state={:?} has_root_hash={}",
                status_with_path.status.repo_path,
                status_with_path.status.state,
                status_with_path
                    .status
                    .root_hash
                    .as_deref()
                    .is_some_and(|root_hash| !root_hash.is_empty()),
            );
        }
        let host_label = self.host_label_for_host(host_id);
        let incoming_statuses = statuses
            .iter()
            .map(|status_with_path| {
                (
                    PathAtHost {
                        host: host_label.clone(),
                        path: status_with_path.remote_path.path.clone(),
                    },
                    status_with_path.status.clone(),
                )
            })
            .collect::<HashMap<_, _>>();
        let existing_status_count = self
            .statuses
            .keys()
            .filter(|key| key.host == host_label)
            .count();
        let snapshot_is_unchanged = existing_status_count == incoming_statuses.len()
            && self
                .statuses
                .iter()
                .filter(|(key, _)| key.host == host_label)
                .all(|(key, status)| incoming_statuses.get(key) == Some(status));
        if snapshot_is_unchanged {
            return (false, vec![]);
        }
        let previous_statuses = self
            .statuses
            .iter()
            .filter(|(key, _)| key.host == host_label)
            .map(|(key, status)| (key.clone(), status.clone()))
            .collect::<HashMap<_, _>>();
        self.statuses.retain(|key, _| key.host != host_label);
        let mut telemetry_updates = vec![];
        for (key, status) in incoming_statuses {
            if previous_statuses.get(&key) == Some(&status) {
                self.statuses.insert(key, status);
                continue;
            }
            let previous_state = previous_statuses
                .get(&key)
                .map(|previous_status| previous_status.state);
            let remote_path = RemotePath::new(host_id.clone(), key.path.clone());
            self.log_status_update(&remote_path, &status);
            self.statuses.insert(key, status.clone());
            telemetry_updates.push(RemoteCodebaseIndexStatusTelemetryUpdate::new(
                &status,
                previous_state,
            ));
        }
        (true, telemetry_updates)
    }

    fn apply_status_update(
        &mut self,
        remote_path: RemotePath,
        status: RemoteCodebaseIndexStatus,
    ) -> bool {
        self.apply_status_update_with_telemetry(remote_path, status)
            .is_some()
    }

    fn apply_status_update_with_telemetry(
        &mut self,
        remote_path: RemotePath,
        status: RemoteCodebaseIndexStatus,
    ) -> Option<RemoteCodebaseIndexStatusTelemetryUpdate> {
        let key = self.status_key_for_remote_path(&remote_path);
        if self.statuses.get(&key) == Some(&status) {
            return None;
        }
        let previous_state = self
            .statuses
            .get(&key)
            .map(|previous_status| previous_status.state);
        self.log_status_update(&remote_path, &status);
        self.statuses.insert(key, status.clone());
        Some(RemoteCodebaseIndexStatusTelemetryUpdate::new(
            &status,
            previous_state,
        ))
    }

    fn log_status_update(&self, remote_path: &RemotePath, status: &RemoteCodebaseIndexStatus) {
        log::info!(
            "[Remote codebase indexing] Client applying codebase index status update: host_id={} repo_path={} state={:?} has_root_hash={}",
            remote_path.host_id,
            status.repo_path,
            status.state,
            status
                .root_hash
                .as_deref()
                .is_some_and(|root_hash| !root_hash.is_empty()),
        );
    }

    fn record_navigated_directory(
        &mut self,
        session_id: SessionId,
        remote_path: &RemotePath,
        is_git: bool,
    ) {
        self.active_repos_by_host
            .insert(remote_path.host_id.clone(), remote_path.clone());
        if is_git {
            self.active_git_repos_by_session
                .insert(session_id, remote_path.clone());
            self.last_git_repos_by_host
                .insert(remote_path.host_id.clone(), remote_path.clone());
        } else {
            self.active_git_repos_by_session.remove(&session_id);
        }
    }

    fn clear_active_git_repo_for_session(&mut self, session_id: SessionId) {
        self.active_git_repos_by_session.remove(&session_id);
    }

    fn record_host_label(&mut self, host_id: &HostId, ctx: &mut ModelContext<Self>) -> bool {
        let Some(host_label) = RemoteServerManager::as_ref(ctx)
            .host_label(host_id)
            .map(|label| HostLabel {
                label: label.to_string(),
            })
        else {
            return false;
        };
        if self.host_labels.get(host_id) == Some(&host_label) {
            return false;
        }
        let previous_host_label = self.host_label_for_host(host_id);
        self.host_labels.insert(host_id.clone(), host_label);
        self.move_statuses_to_resolved_host_label(
            previous_host_label,
            self.host_label_for_host(host_id),
        );
        true
    }

    fn mark_host_unavailable(&mut self, host_id: &HostId) -> bool {
        let host_label = self.host_label_for_host(host_id);
        self.active_repos_by_host.remove(host_id);
        self.active_git_repos_by_session
            .retain(|_, remote_path| remote_path.host_id != *host_id);
        self.last_git_repos_by_host.remove(host_id);

        let mut updated = false;
        for (key, status) in &mut self.statuses {
            if key.host == host_label {
                let failure_message = "The remote host is currently disconnected.".to_string();
                if status.state != RemoteCodebaseIndexState::Unavailable
                    || status.failure_message.as_ref() != Some(&failure_message)
                {
                    status.state = RemoteCodebaseIndexState::Unavailable;
                    status.failure_message = Some(failure_message);
                    updated = true;
                }
            }
        }
        updated
    }

    fn availability_for_remote(
        &self,
        host_id: &HostId,
        current_working_directory: Option<&str>,
        explicit_repo_path: Option<&str>,
    ) -> RemoteCodebaseSearchAvailability {
        let remote_path =
            self.resolve_remote_repo_path(host_id, current_working_directory, explicit_repo_path);

        let Some(remote_path) = remote_path else {
            return RemoteCodebaseSearchAvailability::NoActiveRepo;
        };
        let Some(status) = self.status_for_repo(&remote_path) else {
            return RemoteCodebaseSearchAvailability::NotIndexed { remote_path };
        };
        search_availability_for_status(status, remote_path)
    }

    fn resolve_remote_repo_path(
        &self,
        host_id: &HostId,
        current_working_directory: Option<&str>,
        explicit_repo_path: Option<&str>,
    ) -> Option<RemotePath> {
        if let Some(explicit_repo_path) = explicit_repo_path {
            let explicit_remote_path = remote_path_from_repo_path(host_id, explicit_repo_path);
            if let Some(remote_path) = explicit_remote_path
                .as_ref()
                .filter(|remote_path| self.status_for_repo(remote_path).is_some())
            {
                // Remote branch: exact explicit matches are authoritative, mirroring local
                // `SearchCodebase` behavior where a provided `codebase_path` targets that repo
                // instead of the current working directory.
                return Some(remote_path.clone());
            }

            if let Some((remote_path, _)) = self.best_status_for_path(host_id, explicit_repo_path) {
                // Remote branch: an explicit path inside an indexed remote repo should search that
                // indexed repo root. This preserves remote cross-repo search for paths that can be
                // matched against daemon-reported index state.
                return Some(remote_path);
            }

            // Remote branch: an explicit path that does not match known index state is still
            // authoritative. Return it so callers surface `NotIndexed` (and can request indexing)
            // for the explicit target instead of silently searching the active remote repo.
            return explicit_remote_path;
        }

        if let Some((remote_path, _)) =
            current_working_directory.and_then(|cwd| self.best_status_for_path(host_id, cwd))
        {
            // Remote branch: if the remote cwd is inside a known indexed repo, use the indexed root
            // rather than re-indexing the nested directory.
            return Some(remote_path);
        }
        if let Some(remote_path) = self.active_repos_by_host.get(host_id)
            && self.status_for_repo(remote_path).is_some()
        {
            // Remote branch: only implicit searches (no `codebase_path`) fall back to the
            // active repo recorded by daemon navigation events.
            return Some(remote_path.clone());
        }

        if let Some(remote_path) = self.last_git_repo_for_context(
            host_id,
            current_working_directory,
            self.active_repos_by_host
                .get(host_id)
                .map(|remote_path| remote_path.path.as_str()),
        ) {
            return Some(remote_path);
        }

        if let Some((remote_path, _)) = current_working_directory
            .and_then(|cwd| self.single_descendant_status_for_path(host_id, cwd))
        {
            return Some(remote_path);
        }

        if let Some(remote_path) = self.active_repos_by_host.get(host_id) {
            // Remote branch: only implicit searches (no `codebase_path`) fall back to the active
            // repo recorded by daemon navigation events.
            return Some(remote_path.clone());
        }

        current_working_directory.and_then(|cwd| {
            // Remote branch: only when we have no indexed/active remote repo do we fall back to the
            // remote session cwd as the candidate to index. Local sessions never use this path; they
            // resolve search roots in the local `SearchCodebase` executor branch instead.
            remote_path_from_repo_path(host_id, cwd)
        })
    }
    fn resolve_known_remote_repo_path(
        &self,
        host_id: &HostId,
        current_working_directory: Option<&str>,
        requested_codebase_path: Option<&str>,
    ) -> Option<RemotePath> {
        let remote_path = self.resolve_remote_repo_path(
            host_id,
            current_working_directory,
            requested_codebase_path,
        )?;
        self.status_for_repo(&remote_path)?;
        Some(remote_path)
    }

    fn status_for_repo(&self, remote_path: &RemotePath) -> Option<&RemoteCodebaseIndexStatus> {
        self.statuses
            .get(&self.status_key_for_remote_path(remote_path))
    }

    fn best_status_for_path(
        &self,
        host_id: &HostId,
        path: &str,
    ) -> Option<(RemotePath, &RemoteCodebaseIndexStatus)> {
        let host_label = self.host_label_for_host(host_id);
        let path = StandardizedPath::try_new(path).ok()?;
        self.statuses
            .iter()
            .filter(|(key, _)| key.host == host_label && path.starts_with(&key.path))
            .max_by_key(|(key, _)| key.path.as_str().len())
            .map(|(key, status)| (RemotePath::new(host_id.clone(), key.path.clone()), status))
    }

    fn single_descendant_status_for_path(
        &self,
        host_id: &HostId,
        path: &str,
    ) -> Option<(RemotePath, &RemoteCodebaseIndexStatus)> {
        let host_label = self.host_label_for_host(host_id);
        let path = StandardizedPath::try_new(path).ok()?;
        let mut descendants = self
            .statuses
            .iter()
            .filter(|(key, _)| key.host == host_label && key.path.starts_with(&path));
        let (key, status) = descendants.next()?;
        descendants
            .next()
            .is_none()
            .then(|| (RemotePath::new(host_id.clone(), key.path.clone()), status))
    }

    fn last_git_repo_for_context(
        &self,
        host_id: &HostId,
        current_working_directory: Option<&str>,
        active_repo_path: Option<&str>,
    ) -> Option<RemotePath> {
        let remote_path = self.last_git_repos_by_host.get(host_id)?;
        let repo_path = &remote_path.path;
        let is_related_to_context = current_working_directory
            .and_then(|cwd| StandardizedPath::try_new(cwd).ok())
            .is_some_and(|cwd| cwd.starts_with(repo_path) || repo_path.starts_with(&cwd))
            || active_repo_path
                .and_then(|active_path| StandardizedPath::try_new(active_path).ok())
                .is_some_and(|active_path| {
                    active_path.starts_with(repo_path) || repo_path.starts_with(&active_path)
                });
        is_related_to_context.then(|| remote_path.clone())
    }
}

impl Entity for RemoteCodebaseIndexModel {
    type Event = RemoteCodebaseIndexModelEvent;
}

impl SingletonEntity for RemoteCodebaseIndexModel {}

fn search_availability_for_status(
    status: &RemoteCodebaseIndexStatus,
    remote_path: RemotePath,
) -> RemoteCodebaseSearchAvailability {
    match status.state {
        RemoteCodebaseIndexState::Ready | RemoteCodebaseIndexState::Stale => {
            let Some(root_hash) = status
                .root_hash
                .as_deref()
                .and_then(|hash| NodeHash::from_str(hash).ok())
            else {
                return RemoteCodebaseSearchAvailability::Unavailable {
                    remote_path,
                    message: "The remote codebase index is missing its root hash.".to_string(),
                };
            };
            RemoteCodebaseSearchAvailability::Ready(RemoteCodebaseSearchContext {
                remote_path,
                root_hash,
                is_stale: status.state == RemoteCodebaseIndexState::Stale,
            })
        }
        RemoteCodebaseIndexState::Queued | RemoteCodebaseIndexState::Indexing => {
            RemoteCodebaseSearchAvailability::Indexing { remote_path }
        }
        RemoteCodebaseIndexState::Failed
        | RemoteCodebaseIndexState::NotEnabled
        | RemoteCodebaseIndexState::Unavailable
        | RemoteCodebaseIndexState::Disabled => RemoteCodebaseSearchAvailability::Unavailable {
            remote_path,
            message: status
                .failure_message
                .clone()
                .unwrap_or_else(|| "Remote codebase search is not available.".to_string()),
        },
    }
}
fn emit_status_changed_telemetry(
    update: RemoteCodebaseIndexStatusTelemetryUpdate,
    mutation_kind: Option<RemoteCodebaseIndexUpdateOperation>,
    source: RemoteCodebaseIndexStatusTelemetrySource,
    ctx: &mut ModelContext<RemoteCodebaseIndexModel>,
) {
    send_telemetry_from_ctx!(
        TelemetryEvent::RemoteCodebaseIndexStatusChanged {
            state: update.state,
            previous_state: update.previous_state,
            has_root_hash: update.has_root_hash,
            has_failure_message: update.has_failure_message,
            progress_completed: update.progress_completed,
            progress_total: update.progress_total,
            mutation_kind,
            source,
            remote_os: None,
            remote_arch: None,
        },
        ctx
    );
}
fn emit_auto_index_requested_telemetry(
    trigger: RemoteCodebaseAutoIndexTrigger,
    requested_count: usize,
    ctx: &mut ModelContext<RemoteCodebaseIndexModel>,
) {
    if requested_count == 0 {
        return;
    }

    send_telemetry_from_ctx!(
        TelemetryEvent::RemoteCodebaseAutoIndexRequested {
            trigger,
            requested_count,
            remote_os: None,
            remote_arch: None,
        },
        ctx
    );
}
#[cfg(test)]
#[path = "codebase_index_model_tests.rs"]
mod tests;
