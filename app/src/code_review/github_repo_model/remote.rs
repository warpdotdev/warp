use remote_server::manager::{RemoteServerManager, RemoteServerManagerEvent};
use warp_util::remote_path::RemotePath;
use warpui::{Entity, ModelContext, ModelHandle, SingletonEntity};

use super::GitHubRepoEvent;
use crate::remote_server::proto;
use crate::util::git::{PrInfo, RepositoryInfo};

/// Client-side per-repo GitHub info for a repository on an SSH host.
///
/// Presents the same read surface as [`super::LocalGitHubRepoModel`] and emits the
/// same [`GitHubRepoEvent`]s so the unified [`super::GitHubRepoModel`] can substitute
/// it transparently (mirrors `RemoteGitRepoStatusModel`).
///
/// Pure push receiver: holds the latest PR / repository info for its
/// `(host_id, repo_path)`. On construction (and again on reconnect) it sends
/// `UpdateGitHubPrInfo` / `UpdateGitHubRepoInfo` notifications asking the daemon
/// to create the per-repo model if needed and refresh; results then arrive as
/// server-broadcast push messages filtered by `(host_id, repo_path)`. The
/// daemon's `GitHubRepoModel` is the single source of truth, so there is no
/// request/response and no client-side refresh state. `HostDisconnected`
/// preserves stale data.
pub struct RemoteGitHubRepoModel {
    remote_path: RemotePath,
    pr_info: Option<PrInfo>,
    repository_info: Option<RepositoryInfo>,
}

impl Entity for RemoteGitHubRepoModel {
    type Event = GitHubRepoEvent;
}

impl RemoteGitHubRepoModel {
    pub fn new(remote_path: RemotePath, ctx: &mut ModelContext<Self>) -> Self {
        let mgr = RemoteServerManager::handle(ctx);
        ctx.subscribe_to_model(&mgr, Self::handle_manager_event);
        let model = Self {
            remote_path,
            pr_info: None,
            repository_info: None,
        };
        model.request_github_info(ctx);
        model
    }

    fn handle_manager_event(
        &mut self,
        _: ModelHandle<RemoteServerManager>,
        event: &RemoteServerManagerEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        match event {
            RemoteServerManagerEvent::GitHubPrInfoPushReceived {
                host_id,
                repo_path,
                pr_info,
            } if self.remote_path.matches(host_id, repo_path) => {
                self.apply_pr_info_push(pr_info.as_ref(), ctx);
            }
            RemoteServerManagerEvent::GitHubRepositoryInfoPushReceived {
                host_id,
                repo_path,
                repository_info,
            } if self.remote_path.matches(host_id, repo_path) => {
                self.apply_repository_info_push(repository_info.as_ref(), ctx);
            }
            RemoteServerManagerEvent::HostConnected { host_id }
                if host_id == &self.remote_path.host_id =>
            {
                self.request_github_info(ctx);
            }
            _ => {}
        }
    }

    /// Asks the daemon to (create and) refresh both PR and repository info.
    /// Fire-and-forget; results arrive as push broadcasts.
    fn request_github_info(&self, ctx: &mut ModelContext<Self>) {
        self.request_pr_info(ctx);
        self.request_repository_info(ctx);
    }

    fn request_pr_info(&self, ctx: &mut ModelContext<Self>) {
        let host_id = self.remote_path.host_id.clone();
        let repo_path = self.remote_path.path.clone();
        RemoteServerManager::handle(ctx).update(ctx, |mgr, _| {
            mgr.update_github_pr_info(host_id, &repo_path);
        });
    }

    fn request_repository_info(&self, ctx: &mut ModelContext<Self>) {
        let host_id = self.remote_path.host_id.clone();
        let repo_path = self.remote_path.path.clone();
        RemoteServerManager::handle(ctx).update(ctx, |mgr, _| {
            mgr.update_github_repo_info(host_id, &repo_path);
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
        if self.pr_info != pr_info {
            self.pr_info = pr_info;
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
        if self.repository_info != repository_info {
            self.repository_info = repository_info;
            ctx.emit(GitHubRepoEvent::RepositoryInfoChanged);
        }
    }

    pub fn pr_info(&self) -> Option<&PrInfo> {
        self.pr_info.as_ref()
    }

    pub fn repository_info(&self) -> Option<&RepositoryInfo> {
        self.repository_info.as_ref()
    }

    /// Always `false`: the remote backend does not track refresh state, since
    /// results arrive as broadcasts with no request correlation.
    pub fn is_refreshing_pr_info(&self) -> bool {
        false
    }

    pub fn refresh_pr_info(&self, ctx: &mut ModelContext<Self>) {
        self.request_pr_info(ctx);
    }

    pub fn refresh_repository_info(&self, ctx: &mut ModelContext<Self>) {
        self.request_repository_info(ctx);
    }
}
