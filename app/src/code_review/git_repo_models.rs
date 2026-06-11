#[cfg(feature = "local_fs")]
use std::collections::HashMap;

#[cfg(feature = "local_fs")]
use repo_metadata::repositories::DetectedRepositories;
#[cfg(feature = "local_fs")]
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warpui::{Entity, SingletonEntity};
#[cfg(feature = "local_fs")]
use warpui::{ModelContext, ModelHandle, WeakModelHandle};

#[cfg(feature = "local_fs")]
use super::git_repo_model::{
    new_local_git_repo_status_model, new_remote_git_repo_status_model, GitRepoStatusModel,
};
#[cfg(feature = "local_fs")]
use super::github_repo_model::{GitHubRepoModel, LocalGitHubRepoModel, RemoteGitHubRepoModel};

// ── GitRepoModels (singleton cache) ─────────────────────────────────────────

/// Singleton model that acts as a cache / factory for per-repository
/// [`GitRepoStatusModel`] and [`GitHubRepoModel`] instances.
///
/// Multiple terminals in the same repo share a single sub-model.  When the last
/// strong handle to a sub-model is dropped, the models are torn down automatically.
pub struct GitRepoModels {
    // Per-repo status / GitHub-info models, keyed by `LocalOrRemotePath` so a
    // single cache covers both local (watcher-backed) and remote (push
    // receiver) repos. Each entry stores the unified-enum handle; callers in
    // the same repo share it, and it is torn down when the last strong handle
    // is dropped.
    #[cfg(feature = "local_fs")]
    git_status_models: HashMap<LocalOrRemotePath, WeakModelHandle<GitRepoStatusModel>>,
    #[cfg(feature = "local_fs")]
    github_repo_models: HashMap<LocalOrRemotePath, WeakModelHandle<GitHubRepoModel>>,
}
impl GitRepoModels {
    pub fn new() -> Self {
        Self {
            #[cfg(feature = "local_fs")]
            git_status_models: HashMap::new(),
            #[cfg(feature = "local_fs")]
            github_repo_models: HashMap::new(),
        }
    }

    /// Get or create the per-repo status model for `repo`, returning a unified
    /// [`GitRepoStatusModel`] handle that dispatches to a local watcher-backed
    /// model or a remote push receiver based on the location.
    ///
    /// Multiple callers in the same repo share one model (cached by
    /// `LocalOrRemotePath`); it is torn down when the last strong handle is
    /// dropped.
    ///
    /// Callers hold the returned `ModelHandle` for as long as they need updates.
    #[cfg(feature = "local_fs")]
    pub fn subscribe(
        &mut self,
        repo: &LocalOrRemotePath,
        ctx: &mut ModelContext<Self>,
    ) -> anyhow::Result<ModelHandle<GitRepoStatusModel>> {
        if let Some(handle) = self
            .git_status_models
            .get(repo)
            .and_then(|weak| weak.upgrade(ctx))
        {
            return Ok(handle);
        }

        let handle = match repo {
            LocalOrRemotePath::Local(repo_path) => {
                let Some(repository_model) = DetectedRepositories::as_ref(ctx)
                    .get_local_watched_repo_for_path(repo_path, ctx)
                else {
                    anyhow::bail!(
                        "No watched repository found for path: {}",
                        repo_path.display()
                    );
                };
                new_local_git_repo_status_model(repo_path.clone(), repository_model, ctx)
            }
            LocalOrRemotePath::Remote(remote_path) => {
                new_remote_git_repo_status_model(remote_path.clone(), ctx)
            }
        };

        self.git_status_models
            .insert(repo.clone(), handle.downgrade());
        Ok(handle)
    }

    /// Get or create the per-repo GitHub-info model for `repo`, returning a
    /// unified [`GitHubRepoModel`] handle that dispatches to a local
    /// `gh`-driven model or a remote push receiver based on the location.
    ///
    /// The local backend subscribes to the sibling git status model to track
    /// the current branch and fetches PR / repository info on creation, on
    /// branch change, and on a periodic timer. Multiple callers in the same
    /// repo share one model (cached by `LocalOrRemotePath`).
    ///
    /// Callers hold the returned `ModelHandle` for as long as they need updates.
    #[cfg(feature = "local_fs")]
    pub fn subscribe_github_repo(
        &mut self,
        repo: &LocalOrRemotePath,
        ctx: &mut ModelContext<Self>,
    ) -> anyhow::Result<ModelHandle<GitHubRepoModel>> {
        if let Some(handle) = self
            .github_repo_models
            .get(repo)
            .and_then(|weak| weak.upgrade(ctx))
        {
            return Ok(handle);
        }

        let handle = match repo {
            LocalOrRemotePath::Local(repo_path) => {
                // LocalGitHubRepoModel needs a sibling GitRepoStatusModel for
                // branch info.
                let git_status = self.subscribe(repo, ctx)?;
                let repo_path = repo_path.clone();
                let inner =
                    ctx.add_model(|ctx| LocalGitHubRepoModel::new(repo_path, git_status, ctx));
                ctx.add_model(|ctx| {
                    ctx.subscribe_to_model(&inner, GitHubRepoModel::forward_event);
                    GitHubRepoModel::Local(inner)
                })
            }
            LocalOrRemotePath::Remote(remote_path) => {
                let inner =
                    ctx.add_model(|ctx| RemoteGitHubRepoModel::new(remote_path.clone(), ctx));
                ctx.add_model(|ctx| {
                    ctx.subscribe_to_model(&inner, GitHubRepoModel::forward_event);
                    GitHubRepoModel::Remote(inner)
                })
            }
        };

        self.github_repo_models
            .insert(repo.clone(), handle.downgrade());
        Ok(handle)
    }
}

impl Entity for GitRepoModels {
    type Event = ();
}

impl SingletonEntity for GitRepoModels {}
