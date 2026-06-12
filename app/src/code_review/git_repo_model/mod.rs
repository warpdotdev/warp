#[cfg(feature = "local_fs")]
use repo_metadata::repositories::DetectedRepositories;
use warp_util::local_or_remote_path::LocalOrRemotePath;
#[cfg(feature = "local_fs")]
use warpui::SingletonEntity as _;
use warpui::{AppContext, Entity, ModelContext, ModelHandle};

#[cfg(feature = "local_fs")]
mod local;
#[cfg(feature = "local_fs")]
pub use local::LocalGitRepoStatusModel;

mod remote;
pub use remote::RemoteGitRepoStatusModel;

use super::diff_state::DiffStats;
pub use super::git_repo_models::GitRepoModels;

/// Public metadata exposed to consumers — the subset of diff metadata
/// that the git chip (prompt display, agent view footer) needs.
#[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
#[derive(Debug, Clone)]
pub struct GitStatusMetadata {
    pub current_branch_name: String,
    pub main_branch_name: String,
    pub stats_against_head: DiffStats,
}

// ── GitRepoStatusModel ──────────────────────────────────────────────────────

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
pub enum GitRepoStatusModel {
    #[cfg(feature = "local_fs")]
    Local(ModelHandle<LocalGitRepoStatusModel>),
    Remote(ModelHandle<RemoteGitRepoStatusModel>),
}

impl Entity for GitRepoStatusModel {
    type Event = GitRepoStatusEvent;
}

#[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
impl GitRepoStatusModel {
    /// Creates the per-repo status model for `repo`, dispatching to a local
    /// watcher-backed model or a remote push receiver based on the location.
    pub(super) fn new(
        repo: &LocalOrRemotePath,
        ctx: &mut AppContext,
    ) -> anyhow::Result<ModelHandle<Self>> {
        match repo {
            LocalOrRemotePath::Local(repo_path) => {
                #[cfg(feature = "local_fs")]
                {
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
                    Ok(ctx.add_model(|ctx| {
                        ctx.subscribe_to_model(&inner, Self::forward_event);
                        Self::Local(inner)
                    }))
                }
                #[cfg(not(feature = "local_fs"))]
                {
                    anyhow::bail!(
                        "No watched repository found for path: {}",
                        repo_path.display()
                    );
                }
            }
            LocalOrRemotePath::Remote(remote_path) => {
                let remote_path = remote_path.clone();
                let inner = ctx.add_model(|ctx| RemoteGitRepoStatusModel::new(remote_path, ctx));
                Ok(ctx.add_model(|ctx| {
                    ctx.subscribe_to_model(&inner, Self::forward_event);
                    Self::Remote(inner)
                }))
            }
        }
    }

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
            #[cfg(feature = "local_fs")]
            Self::Local(m) => m.as_ref(ctx).metadata(),
            Self::Remote(m) => m.as_ref(ctx).metadata(),
        }
    }

    /// Force a metadata refresh (branch names, diff stats).
    pub fn refresh_metadata(&self, ctx: &mut ModelContext<Self>) {
        match self {
            #[cfg(feature = "local_fs")]
            Self::Local(m) => m.update(ctx, |m, ctx| m.refresh_metadata(ctx)),
            Self::Remote(m) => m.update(ctx, |m, ctx| m.request_snapshot(ctx)),
        }
    }

    /// Wraps a local-backend test model in the unified enum.
    #[cfg(all(test, feature = "local_fs"))]
    pub(crate) fn new_for_test(
        repository: ModelHandle<repo_metadata::Repository>,
        metadata: Option<GitStatusMetadata>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let inner =
            ctx.add_model(move |_| LocalGitRepoStatusModel::new_for_test(repository, metadata));
        ctx.subscribe_to_model(&inner, Self::forward_event);
        Self::Local(inner)
    }

    #[cfg(all(test, feature = "local_fs"))]
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
