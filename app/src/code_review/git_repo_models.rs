use std::collections::HashMap;
use std::path::{Path, PathBuf};

use repo_metadata::repositories::DetectedRepositories;
use warpui::{Entity, ModelContext, ModelHandle, SingletonEntity, WeakModelHandle};

use super::git_repo_model::{new_local_git_repo_status_model, GitRepoStatusModel};
use super::github_repo_model::{GitHubRepoModel, LocalGitHubRepoModel};

/// Singleton model that acts as a cache / factory for per-repository
/// [`GitRepoStatusModel`] and [`GitHubRepoModel`] instances.
///
/// Multiple terminals in the same repo share a single sub-model. When the last
/// strong handle to a sub-model is dropped, the models are torn down automatically.
pub struct GitRepoModels {
    git_status_models: HashMap<PathBuf, WeakModelHandle<GitRepoStatusModel>>,
    github_repo_models: HashMap<PathBuf, WeakModelHandle<GitHubRepoModel>>,
}

impl GitRepoModels {
    pub fn new() -> Self {
        Self {
            git_status_models: HashMap::new(),
            github_repo_models: HashMap::new(),
        }
    }

    /// Get or create the per-repo status model for a local repo.
    pub fn subscribe(
        &mut self,
        repo_path: &Path,
        ctx: &mut ModelContext<Self>,
    ) -> anyhow::Result<ModelHandle<GitRepoStatusModel>> {
        if let Some(handle) = self
            .git_status_models
            .get(repo_path)
            .and_then(|weak| weak.upgrade(ctx))
        {
            return Ok(handle);
        }

        let Some(repository_model) =
            DetectedRepositories::as_ref(ctx).get_local_watched_repo_for_path(repo_path, ctx)
        else {
            anyhow::bail!(
                "No watched repository found for path: {}",
                repo_path.display()
            );
        };
        let handle =
            new_local_git_repo_status_model(repo_path.to_path_buf(), repository_model, ctx);

        self.git_status_models
            .insert(repo_path.to_path_buf(), handle.downgrade());
        Ok(handle)
    }

    /// Get or create the per-repo GitHub-info model for a local repo.
    pub fn subscribe_github_repo(
        &mut self,
        repo_path: &Path,
        ctx: &mut ModelContext<Self>,
    ) -> anyhow::Result<ModelHandle<GitHubRepoModel>> {
        if let Some(handle) = self
            .github_repo_models
            .get(repo_path)
            .and_then(|weak| weak.upgrade(ctx))
        {
            return Ok(handle);
        }

        let git_status = self.subscribe(repo_path, ctx)?;
        let repo_path = repo_path.to_path_buf();
        let repo_path_for_model = repo_path.clone();
        let inner =
            ctx.add_model(|ctx| LocalGitHubRepoModel::new(repo_path_for_model, git_status, ctx));
        let handle = ctx.add_model(|ctx| {
            ctx.subscribe_to_model(&inner, GitHubRepoModel::forward_event);
            GitHubRepoModel::Local(inner)
        });

        self.github_repo_models
            .insert(repo_path, handle.downgrade());
        Ok(handle)
    }
}

impl Entity for GitRepoModels {
    type Event = ();
}

impl SingletonEntity for GitRepoModels {}
