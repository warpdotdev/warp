#[cfg(feature = "local_fs")]
use std::collections::HashMap;
#[cfg(feature = "local_fs")]
use std::path::{Path, PathBuf};

#[cfg(feature = "local_fs")]
use repo_metadata::repositories::DetectedRepositories;
use warpui::{Entity, SingletonEntity};
#[cfg(feature = "local_fs")]
use warpui::{ModelContext, ModelHandle, WeakModelHandle};

#[cfg(feature = "local_fs")]
use super::git_repo_model::{new_local_git_repo_status_model, GitRepoStatusModel};
#[cfg(feature = "local_fs")]
use super::github_repo_model::{GitHubRepoModel, LocalGitHubRepoModel};

// ── GitRepoModels (singleton cache) ─────────────────────────────────────────

/// Singleton model that acts as a cache / factory for per-repository
/// [`GitRepoStatusModel`] and [`GitHubRepoModel`] instances.
///
/// Multiple terminals in the same repo share a single sub-model.  When the last
/// strong handle to a sub-model is dropped, the models are torn down automatically.
pub struct GitRepoModels {
    #[cfg(feature = "local_fs")]
    git_status_models: HashMap<PathBuf, WeakModelHandle<GitRepoStatusModel>>,
    #[cfg(feature = "local_fs")]
    github_repo_models: HashMap<PathBuf, WeakModelHandle<GitHubRepoModel>>,
}

// ── Non-local_fs stub ───────────────────────────────────────────────────────

#[cfg(not(feature = "local_fs"))]
#[allow(dead_code)]
impl GitRepoModels {
    pub fn new() -> Self {
        Self {}
    }
}

// ── local_fs implementation ─────────────────────────────────────────────────

#[cfg(feature = "local_fs")]
impl GitRepoModels {
    pub fn new() -> Self {
        Self {
            git_status_models: HashMap::new(),
            github_repo_models: HashMap::new(),
        }
    }

    /// Get or create the per-repo status model for `repo_path`, returning a
    /// unified [`GitRepoStatusModel`] handle.
    ///
    /// Multiple callers in the same repo share one model (cached by path); it is
    /// torn down when the last strong handle is dropped.
    ///
    /// Callers hold the returned `ModelHandle` for as long as they need updates.
    pub fn subscribe(
        &mut self,
        repo_path: &Path,
        ctx: &mut ModelContext<Self>,
    ) -> anyhow::Result<ModelHandle<GitRepoStatusModel>> {
        let repo_path_buf = repo_path.to_path_buf();

        // Check the cache for an existing live model.
        if let Some(weak) = self.git_status_models.get(&repo_path_buf) {
            if let Some(handle) = weak.upgrade(ctx) {
                return Ok(handle);
            }
        }

        // Create a new sub-model with an active filesystem watcher.
        let Some(repository_model) =
            DetectedRepositories::as_ref(ctx).get_local_watched_repo_for_path(repo_path, ctx)
        else {
            anyhow::bail!(
                "No watched repository found for path: {}",
                repo_path.display()
            );
        };

        let handle = new_local_git_repo_status_model(repo_path_buf.clone(), repository_model, ctx);

        self.git_status_models
            .insert(repo_path_buf, handle.downgrade());
        Ok(handle)
    }

    /// Get or create the per-repo GitHub-info model for `repo_path`, returning a
    /// unified [`GitHubRepoModel`] handle.
    ///
    /// The local backend subscribes to the sibling git status model to track
    /// the current branch and fetches PR / repository info on creation, on
    /// branch change, and on a periodic timer. Multiple callers in the same
    /// repo share one model (cached by path).
    ///
    /// Callers hold the returned `ModelHandle` for as long as they need updates.
    pub fn subscribe_github_repo(
        &mut self,
        repo_path: &Path,
        ctx: &mut ModelContext<Self>,
    ) -> anyhow::Result<ModelHandle<GitHubRepoModel>> {
        let repo_path_buf = repo_path.to_path_buf();

        // Check the cache for an existing live model.
        if let Some(weak) = self.github_repo_models.get(&repo_path_buf) {
            if let Some(handle) = weak.upgrade(ctx) {
                return Ok(handle);
            }
        }

        // LocalGitHubRepoModel needs a sibling GitRepoStatusModel for branch info.
        let git_status = self.subscribe(repo_path, ctx)?;
        let inner =
            ctx.add_model(|ctx| LocalGitHubRepoModel::new(repo_path_buf.clone(), git_status, ctx));
        let handle = ctx.add_model(|ctx| {
            ctx.subscribe_to_model(&inner, GitHubRepoModel::forward_event);
            GitHubRepoModel::Local(inner)
        });

        self.github_repo_models
            .insert(repo_path_buf, handle.downgrade());
        Ok(handle)
    }
}

impl Entity for GitRepoModels {
    type Event = ();
}

impl SingletonEntity for GitRepoModels {}
