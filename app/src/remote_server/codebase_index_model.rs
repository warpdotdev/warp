use std::collections::HashMap;
use std::str::FromStr;

use ai::index::full_source_code_embedding::{EmbeddingConfig, NodeHash};
use remote_server::codebase_index_proto::{RemoteCodebaseIndexState, RemoteCodebaseIndexStatus};
use warp_core::HostId;
use warp_util::remote_path::RemotePath;
use warp_util::standardized_path::StandardizedPath;
use warpui::{Entity, ModelContext, SingletonEntity};

use crate::ai::blocklist::SessionContext;
use crate::features::FeatureFlag;
use crate::settings::CodeSettings;
use crate::workspaces::user_workspaces::UserWorkspaces;

use super::manager::{
    RemoteCodebaseIndexStatusWithPath, RemoteServerManager, RemoteServerManagerEvent,
};

#[derive(Clone, Debug)]
pub struct RemoteCodebaseSearchContext {
    pub remote_path: RemotePath,
    pub root_hash: NodeHash,
    pub embedding_config: EmbeddingConfig,
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

#[derive(Default)]
pub struct RemoteCodebaseIndexModel {
    statuses: HashMap<RemotePath, RemoteCodebaseIndexStatus>,
    active_repos_by_host: HashMap<HostId, RemotePath>,
    host_labels: HashMap<HostId, String>,
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

impl RemoteCodebaseIndexModel {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        let manager = RemoteServerManager::handle(ctx);
        ctx.subscribe_to_model(&manager, |me, event, ctx| {
            me.handle_remote_server_manager_event(event, ctx);
        });
        Self::default()
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
            manager.ensure_codebase_indexed(remote_path, ctx);
        });
        true
    }

    pub fn codebases_for_agent_context(&self) -> Vec<RemoteCodebaseContextEntry> {
        let mut entries = self
            .statuses
            .iter()
            .filter(|&(remote_path, status)| {
                search_availability_for_status(status, remote_path.clone()).is_ready()
            })
            .map(|(remote_path, _)| {
                let path = remote_path.path.as_str().to_string();
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
            .map(|(remote_path, status)| RemoteCodebaseIndexSettingsEntry {
                remote_path: remote_path.clone(),
                status: status.clone(),
                host_label: self
                    .host_labels
                    .get(&remote_path.host_id)
                    .cloned()
                    .unwrap_or_else(|| remote_path.host_id.to_string()),
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
                if self.apply_statuses_snapshot(host_id, statuses) {
                    ctx.emit(RemoteCodebaseIndexModelEvent::SettingsEntriesChanged);
                }
            }
            RemoteServerManagerEvent::CodebaseIndexStatusUpdated {
                remote_path,
                status,
            } => {
                if self.apply_status_update(remote_path.clone(), status.clone()) {
                    ctx.emit(RemoteCodebaseIndexModelEvent::SettingsEntriesChanged);
                }
            }
            RemoteServerManagerEvent::NavigatedToDirectory {
                session_id: _,
                remote_path,
                is_git,
            } => {
                self.record_navigated_directory(remote_path);
                if *is_git
                    && should_auto_index_remote_codebase(ctx)
                    && self.should_request_auto_index_for_navigated_git_repo(remote_path)
                {
                    // Mirrors local auto-indexing for the thin remote E2E path. TODO(APP-3792):
                    // route remote indexing through the speedbump/consent flow instead of
                    // requesting immediately on navigation.
                    let remote_path = remote_path.clone();
                    RemoteServerManager::handle(ctx).update(ctx, |manager, ctx| {
                        manager.ensure_codebase_indexed(remote_path, ctx);
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
            RemoteServerManagerEvent::SessionConnecting { .. }
            | RemoteServerManagerEvent::SessionConnectionFailed { .. }
            | RemoteServerManagerEvent::SessionDisconnected { .. }
            | RemoteServerManagerEvent::SessionDeregistered { .. }
            | RemoteServerManagerEvent::HostConnected { .. }
            | RemoteServerManagerEvent::RepoMetadataSnapshot { .. }
            | RemoteServerManagerEvent::RepoMetadataUpdated { .. }
            | RemoteServerManagerEvent::RepoMetadataDirectoryLoaded { .. }
            | RemoteServerManagerEvent::BufferUpdated { .. }
            | RemoteServerManagerEvent::BufferConflictDetected { .. }
            | RemoteServerManagerEvent::DiffStateSnapshotReceived { .. }
            | RemoteServerManagerEvent::DiffStateMetadataUpdateReceived { .. }
            | RemoteServerManagerEvent::DiffStateFileDeltaReceived { .. }
            | RemoteServerManagerEvent::GetBranchesResponse { .. }
            | RemoteServerManagerEvent::SetupStateChanged { .. }
            | RemoteServerManagerEvent::BinaryCheckComplete { .. }
            | RemoteServerManagerEvent::BinaryInstallComplete { .. }
            | RemoteServerManagerEvent::ClientRequestFailed { .. }
            | RemoteServerManagerEvent::ServerMessageDecodingError { .. } => {}
        }
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

    fn apply_statuses_snapshot(
        &mut self,
        host_id: &HostId,
        statuses: &[RemoteCodebaseIndexStatusWithPath],
    ) -> bool {
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
        let incoming_statuses = statuses
            .iter()
            .map(|status_with_path| {
                (
                    status_with_path.remote_path.clone(),
                    status_with_path.status.clone(),
                )
            })
            .collect::<HashMap<_, _>>();
        let existing_status_count = self
            .statuses
            .keys()
            .filter(|remote_path| remote_path.host_id == *host_id)
            .count();
        let snapshot_is_unchanged = existing_status_count == incoming_statuses.len()
            && self
                .statuses
                .iter()
                .filter(|(remote_path, _)| remote_path.host_id == *host_id)
                .all(|(remote_path, status)| incoming_statuses.get(remote_path) == Some(status));
        if snapshot_is_unchanged {
            return false;
        }
        self.statuses.retain(|key, _| key.host_id != *host_id);
        for (remote_path, status) in incoming_statuses {
            self.apply_status_update(remote_path, status);
        }
        true
    }

    fn apply_status_update(
        &mut self,
        remote_path: RemotePath,
        status: RemoteCodebaseIndexStatus,
    ) -> bool {
        if self.statuses.get(&remote_path) == Some(&status) {
            return false;
        }
        log::info!(
            "[Remote codebase indexing] Client applying codebase index status update: host_id={} state={:?} has_root_hash={}",
            remote_path.host_id,
            status.state,
            status
                .root_hash
                .as_deref()
                .is_some_and(|root_hash| !root_hash.is_empty()),
        );
        log::debug!(
            "[Remote codebase indexing] Client applying codebase index status update: repo_path={} state={:?}",
            status.repo_path,
            status.state,
        );
        self.statuses.insert(remote_path, status);
        true
    }

    fn record_navigated_directory(&mut self, remote_path: &RemotePath) {
        self.active_repos_by_host
            .insert(remote_path.host_id.clone(), remote_path.clone());
    }

    fn record_host_label(&mut self, host_id: &HostId, ctx: &mut ModelContext<Self>) -> bool {
        let Some(host_label) = RemoteServerManager::as_ref(ctx)
            .host_label(host_id)
            .map(ToOwned::to_owned)
        else {
            return false;
        };
        if self.host_labels.get(host_id) == Some(&host_label) {
            return false;
        }
        self.host_labels.insert(host_id.clone(), host_label);
        true
    }

    fn mark_host_unavailable(&mut self, host_id: &HostId) -> bool {
        let mut updated = false;
        for (remote_path, status) in &mut self.statuses {
            if remote_path.host_id == *host_id {
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
        self.active_repos_by_host.remove(host_id);
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
                return Some(remote_path.clone());
            }

            // Remote branch: an explicit path that does not match known index state is still
            // authoritative. Return it so callers surface `NotIndexed` (and can request indexing)
            // for the explicit target instead of silently searching the active remote repo.
            return explicit_remote_path;
        }

        if let Some(remote_path) = self.active_repos_by_host.get(host_id) {
            // Remote branch: only implicit searches (no `codebase_path`) fall back to the active
            // repo recorded by daemon navigation events.
            return Some(remote_path.clone());
        }

        if let Some((remote_path, _)) =
            current_working_directory.and_then(|cwd| self.best_status_for_path(host_id, cwd))
        {
            // Remote branch: if the remote cwd is inside a known indexed repo, use the indexed root
            // rather than re-indexing the nested directory.
            return Some(remote_path.clone());
        }

        current_working_directory.and_then(|cwd| {
            // Remote branch: only when we have no indexed/active remote repo do we fall back to the
            // remote session cwd as the candidate to index. Local sessions never use this path; they
            // resolve search roots in the local `SearchCodebase` executor branch instead.
            remote_path_from_repo_path(host_id, cwd)
        })
    }

    fn status_for_repo(&self, remote_path: &RemotePath) -> Option<&RemoteCodebaseIndexStatus> {
        self.statuses.get(remote_path)
    }

    fn best_status_for_path(
        &self,
        host_id: &HostId,
        path: &str,
    ) -> Option<(&RemotePath, &RemoteCodebaseIndexStatus)> {
        let path = StandardizedPath::try_new(path).ok()?;
        self.statuses
            .iter()
            .filter(|(key, _)| key.host_id == *host_id && path.starts_with(&key.path))
            .max_by_key(|(remote_path, _)| remote_path.path.as_str().len())
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
                embedding_config: EmbeddingConfig::default(),
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

fn should_auto_index_remote_codebase(ctx: &mut ModelContext<RemoteCodebaseIndexModel>) -> bool {
    remote_auto_indexing_enabled(
        UserWorkspaces::as_ref(ctx).is_codebase_context_enabled(ctx),
        *CodeSettings::as_ref(ctx).auto_indexing_enabled,
    )
}

fn remote_auto_indexing_enabled(
    codebase_context_enabled: bool,
    auto_indexing_enabled: bool,
) -> bool {
    FeatureFlag::RemoteCodebaseIndexing.is_enabled()
        && FeatureFlag::FullSourceCodeEmbedding.is_enabled()
        && codebase_context_enabled
        && auto_indexing_enabled
}

#[cfg(test)]
#[path = "codebase_index_model_tests.rs"]
mod tests;
