use warpui::{AppContext, Entity, ModelContext, ModelHandle};

#[cfg(feature = "local_fs")]
mod local;
#[cfg(feature = "local_fs")]
pub use local::LocalGitHubRepoModel;

mod remote;
pub use remote::RemoteGitHubRepoModel;

#[cfg(all(test, feature = "local_fs"))]
use crate::code_review::git_repo_model::GitRepoStatusModel;
use crate::util::git::{PrInfo, PrStackInfo, RepositoryInfo};

#[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
// All three variants intentionally share the `*InfoChanged` shape; renaming
// to satisfy the lint would touch every match arm across the codebase for
// no readability gain.
#[allow(clippy::enum_variant_names)]
#[derive(Debug)]
pub enum GitHubRepoEvent {
    /// Emitted when `pr_info` changes value (fetch result differs from
    /// cached, branch change cleared the cache, etc.).
    PrInfoChanged,
    /// Emitted when `repository_info` changes value.
    RepositoryInfoChanged,
    /// Emitted when `stack_info` changes value.
    StackInfoChanged,
}

// ── Unified GitHubRepoModel (local or remote backend) ───────────────────────

/// Unified per-repo GitHub-info model that dispatches to a local or remote
/// backend, mirroring [`crate::code_review::git_repo_model::GitRepoStatusModel`].
///
/// Consumers (prompt chips, code review, agent context) hold a
/// `ModelHandle<GitHubRepoModel>` and subscribe to its [`GitHubRepoEvent`]s
/// without caring whether the repository is local or on an SSH host.
pub enum GitHubRepoModel {
    #[cfg(feature = "local_fs")]
    Local(ModelHandle<LocalGitHubRepoModel>),
    Remote(ModelHandle<RemoteGitHubRepoModel>),
}
impl Entity for GitHubRepoModel {
    type Event = GitHubRepoEvent;
}
impl GitHubRepoModel {
    /// Re-emit a sub-model event so subscribers of the unified model observe
    /// the same `GitHubRepoEvent`s regardless of backend.
    pub(crate) fn forward_event(&mut self, event: &GitHubRepoEvent, ctx: &mut ModelContext<Self>) {
        match event {
            GitHubRepoEvent::PrInfoChanged => ctx.emit(GitHubRepoEvent::PrInfoChanged),
            GitHubRepoEvent::RepositoryInfoChanged => {
                ctx.emit(GitHubRepoEvent::RepositoryInfoChanged)
            }
            GitHubRepoEvent::StackInfoChanged => ctx.emit(GitHubRepoEvent::StackInfoChanged),
        }
    }

    /// PR info for the current branch.
    pub fn pr_info<'a>(&self, ctx: &'a AppContext) -> Option<&'a PrInfo> {
        match self {
            #[cfg(feature = "local_fs")]
            Self::Local(m) => m.as_ref(ctx).pr_info(),
            Self::Remote(m) => m.as_ref(ctx).pr_info(),
        }
    }

    /// Repository info (name/owner) returned by `gh repo view`.
    pub fn repository_info<'a>(&self, ctx: &'a AppContext) -> Option<&'a RepositoryInfo> {
        match self {
            #[cfg(feature = "local_fs")]
            Self::Local(m) => m.as_ref(ctx).repository_info(),
            Self::Remote(m) => m.as_ref(ctx).repository_info(),
        }
    }

    /// Whether a `gh pr view` fetch is currently in flight.
    pub fn is_refreshing_pr_info(&self, ctx: &AppContext) -> bool {
        match self {
            #[cfg(feature = "local_fs")]
            Self::Local(m) => m.as_ref(ctx).is_refreshing_pr_info(),
            Self::Remote(m) => m.as_ref(ctx).is_refreshing_pr_info(),
        }
    }

    /// The GitHub-native stack containing the current branch's pull request,
    /// when it belongs to one with two or more layers. `None` covers "not
    /// stacked", discovery not yet complete, and a suppressed/unavailable
    /// discovery result alike — all of these degrade to today's single-PR
    /// experience per the product spec.
    pub fn stack_info<'a>(&self, ctx: &'a AppContext) -> Option<&'a PrStackInfo> {
        match self {
            #[cfg(feature = "local_fs")]
            Self::Local(m) => m.as_ref(ctx).stack_info(),
            Self::Remote(m) => m.as_ref(ctx).stack_info(),
        }
    }

    /// Whether a stack discovery fetch is currently in flight.
    pub fn is_refreshing_stack_info(&self, ctx: &AppContext) -> bool {
        match self {
            #[cfg(feature = "local_fs")]
            Self::Local(m) => m.as_ref(ctx).is_refreshing_stack_info(),
            Self::Remote(m) => m.as_ref(ctx).is_refreshing_stack_info(),
        }
    }

    /// Force a PR info refresh (e.g. after a `gh`/`gt` command completes).
    pub fn refresh_pr_info(&self, ctx: &mut ModelContext<Self>) {
        match self {
            #[cfg(feature = "local_fs")]
            Self::Local(m) => m.update(ctx, |m, ctx| m.refresh_pr_info(ctx)),
            Self::Remote(m) => m.update(ctx, |m, ctx| m.refresh_pr_info(ctx)),
        }
    }

    /// Force a repository-info refresh.
    pub fn refresh_repository_info(&self, ctx: &mut ModelContext<Self>) {
        match self {
            #[cfg(feature = "local_fs")]
            Self::Local(m) => m.update(ctx, |m, ctx| m.refresh_repository_info(ctx)),
            Self::Remote(m) => m.update(ctx, |m, ctx| m.refresh_repository_info(ctx)),
        }
    }
}

#[cfg(all(test, feature = "local_fs"))]
impl GitHubRepoModel {
    /// Wraps an inert local-backend test model in the unified enum.
    pub(crate) fn new_local_for_test(
        git_status: ModelHandle<GitRepoStatusModel>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let inner = ctx.add_model(move |_| LocalGitHubRepoModel::new_for_test(git_status));
        ctx.subscribe_to_model(&inner, |me, _, event, ctx| me.forward_event(event, ctx));
        Self::Local(inner)
    }

    pub(crate) fn set_pr_info_for_test(
        &mut self,
        pr_info: Option<PrInfo>,
        ctx: &mut ModelContext<Self>,
    ) {
        match self {
            #[cfg(feature = "local_fs")]
            Self::Local(m) => m.update(ctx, |m, ctx| m.set_pr_info_for_test(pr_info, ctx)),
            Self::Remote(_) => unreachable!("remote test models are not used"),
        }
    }

    pub(crate) fn set_repository_info_for_test(
        &mut self,
        repository_info: Option<RepositoryInfo>,
        ctx: &mut ModelContext<Self>,
    ) {
        match self {
            #[cfg(feature = "local_fs")]
            Self::Local(m) => m.update(ctx, |m, ctx| {
                m.set_repository_info_for_test(repository_info, ctx)
            }),
            Self::Remote(_) => unreachable!("remote test models are not used"),
        }
    }

    // Add a `set_stack_info_for_test` wrapper here (mirroring the two
    // methods above) once a `CodeReviewView`-level test needs to stub
    // `stack_info` through the unified `GitHubRepoModel`, the same way
    // `LocalGitHubRepoModel::set_stack_info_for_test` already does for
    // `local_tests.rs`.
}
