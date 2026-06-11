#[cfg(feature = "local_fs")]
use std::collections::HashMap;

#[cfg(feature = "local_fs")]
use repo_metadata::repositories::DetectedRepositories;
#[cfg(feature = "local_fs")]
use warp_util::local_or_remote_path::LocalOrRemotePath;
#[cfg(feature = "local_fs")]
use warpui::{AppContext, ModelContext, ModelHandle, WeakModelHandle};
use warpui::{Entity, SingletonEntity};

#[cfg(feature = "local_fs")]
mod local;
#[cfg(feature = "local_fs")]
pub use local::LocalGitRepoStatusModel;

#[cfg(feature = "local_fs")]
mod remote;
#[cfg(feature = "local_fs")]
pub use remote::RemoteGitRepoStatusModel;

#[cfg(feature = "local_fs")]
use super::diff_state::DiffStats;
#[cfg(feature = "local_fs")]
use super::github_repo_model::{GitHubRepoModel, LocalGitHubRepoModel, RemoteGitHubRepoModel};

/// Public metadata exposed to consumers — the subset of diff metadata
/// that the git chip (prompt display, agent view footer) needs.
#[cfg(feature = "local_fs")]
#[derive(Debug, Clone)]
pub struct GitStatusMetadata {
    pub current_branch_name: String,
    pub main_branch_name: String,
    pub stats_against_head: DiffStats,
}

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

    /// Get or create the per-repo status model for `repo`, returning a unified
    /// [`GitRepoStatusModel`] handle that dispatches to a local watcher-backed
    /// model or a remote push receiver based on the location.
    ///
    /// Multiple callers in the same repo share one model (cached by
    /// `LocalOrRemotePath`); it is torn down when the last strong handle is
    /// dropped.
    ///
    /// Callers hold the returned `ModelHandle` for as long as they need updates.
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
                let repo_path = repo_path.clone();
                let inner = ctx.add_model(|ctx| {
                    LocalGitRepoStatusModel::new(repo_path, repository_model, ctx)
                });
                ctx.add_model(|ctx| {
                    ctx.subscribe_to_model(&inner, GitRepoStatusModel::forward_event);
                    GitRepoStatusModel::Local(inner)
                })
            }
            LocalOrRemotePath::Remote(remote_path) => {
                let inner =
                    ctx.add_model(|ctx| RemoteGitRepoStatusModel::new(remote_path.clone(), ctx));
                ctx.add_model(|ctx| {
                    ctx.subscribe_to_model(&inner, GitRepoStatusModel::forward_event);
                    GitRepoStatusModel::Remote(inner)
                })
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

// ── GitRepoStatusModel ──────────────────────────────────────────────────────

#[cfg(not(feature = "local_fs"))]
#[allow(dead_code)]
pub struct GitRepoStatusModel;

#[cfg(not(feature = "local_fs"))]
impl Entity for GitRepoStatusModel {
    type Event = ();
}

#[cfg(feature = "local_fs")]
#[derive(Debug)]
pub enum GitRepoStatusEvent {
    /// Emitted whenever the metadata changes (branch name, diff stats, etc.).
    MetadataChanged,
}

// ── Unified GitRepoStatusModel (local or remote backend) ────────────────────

/// Unified per-repo git status model that dispatches to a local or remote
/// backend, mirroring [`crate::code_review::diff_state::DiffStateModel`].
///
/// Consumers (prompt chips, tabs, code review, agent context) hold a
/// `ModelHandle<GitRepoStatusModel>` and subscribe to its [`GitRepoStatusEvent`]s
/// without caring whether the repository is local or on an SSH host. Only one
/// variant is populated at a time.
#[cfg(feature = "local_fs")]
pub enum GitRepoStatusModel {
    Local(ModelHandle<LocalGitRepoStatusModel>),
    Remote(ModelHandle<RemoteGitRepoStatusModel>),
}

#[cfg(feature = "local_fs")]
impl Entity for GitRepoStatusModel {
    type Event = GitRepoStatusEvent;
}

#[cfg(feature = "local_fs")]
impl GitRepoStatusModel {
    /// Re-emit a sub-model event so subscribers of the unified model observe
    /// the same `GitRepoStatusEvent`s regardless of backend.
    fn forward_event(&mut self, event: &GitRepoStatusEvent, ctx: &mut ModelContext<Self>) {
        match event {
            GitRepoStatusEvent::MetadataChanged => ctx.emit(GitRepoStatusEvent::MetadataChanged),
        }
    }

    /// Mode-independent status metadata (branch names + HEAD diff stats).
    pub fn metadata<'a>(&self, ctx: &'a AppContext) -> Option<&'a GitStatusMetadata> {
        match self {
            Self::Local(m) => m.as_ref(ctx).metadata(),
            Self::Remote(m) => m.as_ref(ctx).metadata(),
        }
    }

    /// Force a metadata refresh (branch names, diff stats).
    pub fn refresh_metadata(&self, ctx: &mut ModelContext<Self>) {
        match self {
            Self::Local(m) => m.update(ctx, |m, ctx| m.refresh_metadata(ctx)),
            Self::Remote(m) => m.update(ctx, |m, ctx| m.request_snapshot(ctx)),
        }
    }
}

#[cfg(all(test, feature = "local_fs"))]
impl GitRepoStatusModel {
    /// Wraps a local-backend test model in the unified enum.
    pub(crate) fn new_local_for_test(
        repository: ModelHandle<repo_metadata::Repository>,
        metadata: Option<GitStatusMetadata>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let inner =
            ctx.add_model(move |_| LocalGitRepoStatusModel::new_for_test(repository, metadata));
        ctx.subscribe_to_model(&inner, Self::forward_event);
        Self::Local(inner)
    }

    pub(crate) fn set_metadata_for_test(
        &mut self,
        metadata: Option<GitStatusMetadata>,
        ctx: &mut ModelContext<Self>,
    ) {
        match self {
            Self::Local(m) => m.update(ctx, |m, ctx| m.set_metadata_for_test(metadata, ctx)),
            Self::Remote(_) => unreachable!("remote test models are not used"),
        }
    }
}
