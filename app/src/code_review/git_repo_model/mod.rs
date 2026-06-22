use warpui::{AppContext, Entity, ModelContext, ModelHandle};

#[cfg(feature = "local_fs")]
mod local;
#[cfg(feature = "local_fs")]
pub use local::LocalGitRepoStatusModel;

mod remote;
pub use remote::RemoteGitRepoStatusModel;

use super::diff_state::DiffStats;
pub use super::git_repo_models::GitRepoModels;
use crate::context_chips::display_chip::GitBranchTrackingStatus;

/// Public metadata exposed to consumers — the subset of diff metadata
/// that the git chip (prompt display, agent view footer) needs.
#[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
#[derive(Debug, Clone)]
pub struct GitStatusMetadata {
    pub current_branch_name: String,
    pub main_branch_name: String,
    pub stats_against_head: DiffStats,
    pub branch_tracking_status: GitBranchTrackingStatus,
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
}

#[cfg(feature = "local_fs")]
pub(super) fn new_local_git_repo_status_model(
    repo_path: std::path::PathBuf,
    repository_model: ModelHandle<repo_metadata::Repository>,
    ctx: &mut ModelContext<GitRepoModels>,
) -> ModelHandle<GitRepoStatusModel> {
    let inner = ctx.add_model(|ctx| LocalGitRepoStatusModel::new(repo_path, repository_model, ctx));
    ctx.add_model(|ctx| {
        ctx.subscribe_to_model(&inner, |me, _, event, ctx| {
            GitRepoStatusModel::forward_event(me, event, ctx)
        });
        GitRepoStatusModel::Local(inner)
    })
}

pub(super) fn new_remote_git_repo_status_model(
    remote_path: warp_util::remote_path::RemotePath,
    ctx: &mut ModelContext<GitRepoModels>,
) -> ModelHandle<GitRepoStatusModel> {
    let inner = ctx.add_model(|ctx| RemoteGitRepoStatusModel::new(remote_path, ctx));
    ctx.add_model(|ctx| {
        ctx.subscribe_to_model(&inner, |me, _, event, ctx| {
            GitRepoStatusModel::forward_event(me, event, ctx)
        });
        GitRepoStatusModel::Remote(inner)
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
        ctx.subscribe_to_model(&inner, |me, _, event, ctx| me.forward_event(event, ctx));
        Self::Local(inner)
    }

    pub(crate) fn set_metadata_for_test(
        &mut self,
        metadata: Option<GitStatusMetadata>,
        ctx: &mut ModelContext<Self>,
    ) {
        match self {
            #[cfg(feature = "local_fs")]
            Self::Local(m) => m.update(ctx, |m, ctx| m.set_metadata_for_test(metadata, ctx)),
            Self::Remote(_) => unreachable!("remote test models are not used"),
        }
    }
}
