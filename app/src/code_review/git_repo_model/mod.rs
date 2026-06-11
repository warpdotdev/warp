#[cfg(feature = "local_fs")]
use warpui::ModelHandle;
use warpui::{AppContext, Entity, ModelContext};

#[cfg(feature = "local_fs")]
mod local;
#[cfg(feature = "local_fs")]
pub use local::LocalGitRepoStatusModel;

use super::diff_state::DiffStats;
#[cfg(feature = "local_fs")]
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

#[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
#[derive(Debug)]
pub enum GitRepoStatusEvent {
    /// Emitted whenever the metadata changes (branch name, diff stats, etc.).
    MetadataChanged,
}

/// Unified per-repo git status model. PR 1 only contains the local backend;
/// remote support is added in the stacked PR.
#[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
pub enum GitRepoStatusModel {
    #[cfg(feature = "local_fs")]
    Local(ModelHandle<LocalGitRepoStatusModel>),
}

impl Entity for GitRepoStatusModel {
    type Event = GitRepoStatusEvent;
}

#[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
impl GitRepoStatusModel {
    /// Re-emit a sub-model event so subscribers of the unified model observe
    /// the same `GitRepoStatusEvent`s regardless of backend.
    #[cfg(feature = "local_fs")]
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
            // Without `local_fs` the enum has no variants, so a value can
            // never be constructed and this arm is unreachable.
            #[cfg(not(feature = "local_fs"))]
            _ => {
                let _ = ctx;
                unreachable!("GitRepoStatusModel cannot be constructed without local_fs")
            }
        }
    }

    /// Force a metadata refresh (branch names, diff stats).
    pub fn refresh_metadata(&self, ctx: &mut ModelContext<Self>) {
        match self {
            #[cfg(feature = "local_fs")]
            Self::Local(m) => m.update(ctx, |m, ctx| m.refresh_metadata(ctx)),
            // Without `local_fs` the enum has no variants, so a value can
            // never be constructed and this arm is unreachable.
            #[cfg(not(feature = "local_fs"))]
            _ => {
                let _ = ctx;
                unreachable!("GitRepoStatusModel cannot be constructed without local_fs")
            }
        }
    }
}

#[cfg(feature = "local_fs")]
pub(super) fn new_local_git_repo_status_model(
    repo_path: std::path::PathBuf,
    repository_model: ModelHandle<repo_metadata::Repository>,
    ctx: &mut ModelContext<GitRepoModels>,
) -> ModelHandle<GitRepoStatusModel> {
    let inner = ctx.add_model(|ctx| LocalGitRepoStatusModel::new(repo_path, repository_model, ctx));
    ctx.add_model(|ctx| {
        ctx.subscribe_to_model(&inner, GitRepoStatusModel::forward_event);
        GitRepoStatusModel::Local(inner)
    })
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
        }
    }
}
