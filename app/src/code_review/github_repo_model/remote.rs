use remote_server::manager::{RemoteServerManager, RemoteServerManagerEvent};
use warp_util::remote_path::RemotePath;
use warpui::{Entity, ModelContext, SingletonEntity};

use super::GitHubRepoEvent;
use crate::remote_server::proto;
use crate::util::git::{PrInfo, RepositoryInfo};

/// Client-side per-repo GitHub info for a repository on an SSH host.
///
/// Presents the same read surface as [`super::LocalGitHubRepoModel`] and emits the
/// same [`GitHubRepoEvent`]s so the unified [`super::GitHubRepoModel`] can substitute
/// it transparently (mirrors `RemoteGitRepoStatusModel`).
///
/// Holds the latest PR / repository info for its `(host_id, repo_path)`. On
/// construction (and again on reconnect) it sends host-scoped requests for the
/// current snapshots; live updates then arrive as server-broadcast PR-info and
/// repository-info push messages, filtered by `(host_id, repo_path)`.
/// `HostDisconnected` preserves stale data.
pub struct RemoteGitHubRepoModel {
    remote_path: RemotePath,
    pr_info: Option<PrInfo>,
    repository_info: Option<RepositoryInfo>,
    refreshing_pr_info: bool,
    refreshing_repository_info: bool,
}

impl Entity for RemoteGitHubRepoModel {
    type Event = GitHubRepoEvent;
}

impl RemoteGitHubRepoModel {
    pub fn new(remote_path: RemotePath, ctx: &mut ModelContext<Self>) -> Self {
        let mgr = RemoteServerManager::handle(ctx);
        ctx.subscribe_to_model(&mgr, Self::handle_manager_event);
        let mut model = Self {
            remote_path,
            pr_info: None,
            repository_info: None,
            refreshing_pr_info: false,
            refreshing_repository_info: false,
        };
        model.request_github_info(ctx);
        model
    }

    fn handle_manager_event(
        &mut self,
        event: &RemoteServerManagerEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        match event {
            RemoteServerManagerEvent::GitHubPrInfoPushReceived {
                host_id,
                repo_path,
                pr_info,
            } if host_id == &self.remote_path.host_id && repo_path == &self.remote_path.path => {
                self.apply_pr_info_push(pr_info.as_ref(), ctx);
            }
            RemoteServerManagerEvent::GitHubRepositoryInfoPushReceived {
                host_id,
                repo_path,
                repository_info,
            } if host_id == &self.remote_path.host_id && repo_path == &self.remote_path.path => {
                self.apply_repository_info_push(repository_info.as_ref(), ctx);
            }
            RemoteServerManagerEvent::GetGitHubPrInfoResponse {
                host_id,
                repo_path,
                result,
            } if host_id == &self.remote_path.host_id && repo_path == &self.remote_path.path => {
                self.handle_pr_info_response(result, ctx);
            }
            RemoteServerManagerEvent::GetGitHubRepoInfoResponse {
                host_id,
                repo_path,
                result,
            } if host_id == &self.remote_path.host_id && repo_path == &self.remote_path.path => {
                self.handle_repository_info_response(result, ctx);
            }
            RemoteServerManagerEvent::HostConnected { host_id }
                if host_id == &self.remote_path.host_id =>
            {
                self.request_github_info(ctx);
            }
            _ => {}
        }
    }

    fn request_github_info(&mut self, ctx: &mut ModelContext<Self>) {
        self.request_pr_info(ctx);
        self.request_repository_info(ctx);
    }

    fn request_pr_info(&mut self, ctx: &mut ModelContext<Self>) {
        if self.refreshing_pr_info {
            return;
        }
        self.refreshing_pr_info = true;
        ctx.emit(GitHubRepoEvent::PrInfoChanged);

        let host_id = self.remote_path.host_id.clone();
        let repo_path = self.remote_path.path.clone();
        RemoteServerManager::handle(ctx).update(ctx, |mgr, ctx| {
            mgr.get_github_pr_info(host_id, repo_path, ctx);
        });
    }

    fn request_repository_info(&mut self, ctx: &mut ModelContext<Self>) {
        if self.refreshing_repository_info {
            return;
        }
        self.refreshing_repository_info = true;

        let host_id = self.remote_path.host_id.clone();
        let repo_path = self.remote_path.path.clone();
        RemoteServerManager::handle(ctx).update(ctx, |mgr, ctx| {
            mgr.get_github_repo_info(host_id, repo_path, ctx);
        });
    }

    /// Replace the stored PR info from a push, emitting `PrInfoChanged` only
    /// when the value moved.
    fn apply_pr_info_push(
        &mut self,
        pr_info: Option<&proto::PrInfo>,
        ctx: &mut ModelContext<Self>,
    ) {
        let pr_info = pr_info.map(PrInfo::from);
        let pr_changed = if self.refreshing_pr_info && pr_info.is_none() {
            false
        } else {
            let changed = self.pr_info != pr_info;
            self.pr_info = pr_info;
            changed
        };
        if pr_changed {
            ctx.emit(GitHubRepoEvent::PrInfoChanged);
        }
    }

    /// Replace the stored repository info from a push, emitting
    /// `RepositoryInfoChanged` only when the value moved.
    fn apply_repository_info_push(
        &mut self,
        repository_info: Option<&proto::RepositoryInfo>,
        ctx: &mut ModelContext<Self>,
    ) {
        let repository_info = repository_info.map(RepositoryInfo::from);
        let repo_changed = if self.refreshing_repository_info && repository_info.is_none() {
            false
        } else {
            let changed = self.repository_info != repository_info;
            self.repository_info = repository_info;
            changed
        };
        if repo_changed {
            ctx.emit(GitHubRepoEvent::RepositoryInfoChanged);
        }
    }

    fn handle_pr_info_response(
        &mut self,
        result: &Result<Option<proto::PrInfo>, String>,
        ctx: &mut ModelContext<Self>,
    ) {
        let mut pr_changed = false;
        match result {
            Ok(pr_info) => {
                let pr_info = pr_info.as_ref().map(PrInfo::from);
                pr_changed = self.pr_info != pr_info;
                self.pr_info = pr_info;
            }
            Err(error) => {
                log::debug!("RemoteGitHubRepoModel: PR info load failed: {error}");
            }
        }

        let refreshing_changed = self.refreshing_pr_info;
        self.refreshing_pr_info = false;
        if pr_changed || refreshing_changed {
            ctx.emit(GitHubRepoEvent::PrInfoChanged);
        }
    }

    fn handle_repository_info_response(
        &mut self,
        result: &Result<Option<proto::RepositoryInfo>, String>,
        ctx: &mut ModelContext<Self>,
    ) {
        let mut repo_changed = false;
        match result {
            Ok(repository_info) => {
                let repository_info = repository_info.as_ref().map(RepositoryInfo::from);
                repo_changed = self.repository_info != repository_info;
                self.repository_info = repository_info;
            }
            Err(error) => {
                log::debug!("RemoteGitHubRepoModel: repository info load failed: {error}");
            }
        }
        self.refreshing_repository_info = false;
        if repo_changed {
            ctx.emit(GitHubRepoEvent::RepositoryInfoChanged);
        }
    }

    pub fn pr_info(&self) -> Option<&PrInfo> {
        self.pr_info.as_ref()
    }

    pub fn repository_info(&self) -> Option<&RepositoryInfo> {
        self.repository_info.as_ref()
    }

    pub fn is_refreshing_pr_info(&self) -> bool {
        self.refreshing_pr_info
    }

    pub fn refresh_pr_info(&mut self, ctx: &mut ModelContext<Self>) {
        self.request_pr_info(ctx);
    }

    pub fn refresh_repository_info(&mut self, ctx: &mut ModelContext<Self>) {
        self.request_repository_info(ctx);
    }
}
