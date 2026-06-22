use std::path::PathBuf;
use std::time::Duration;

use settings::Setting as _;
use warpui::r#async::SpawnedFutureHandle;
use warpui::{Entity, ModelContext, ModelHandle, SingletonEntity as _};

use super::GitHubRepoEvent;
use crate::code_review::git_repo_model::{GitRepoStatusEvent, GitRepoStatusModel};
use crate::report_if_error;
#[cfg(feature = "local_tty")]
use crate::terminal::local_shell::LocalShellState;
use crate::terminal::session_settings::{GithubPrPromptChipDefaultValidation, SessionSettings};
use crate::util::git::{
    get_pr_for_branch, get_repository_info, is_gh_auth_error, is_gh_missing_error, PrInfo,
    RepositoryInfo,
};

const PR_INFO_FETCH_TIMEOUT: Duration = Duration::from_secs(5);
const GITHUB_INFO_PERIODIC_REFRESH: Duration = Duration::from_secs(60);
const REPOSITORY_INFO_FETCH_TIMEOUT: Duration = Duration::from_secs(5);

/// Per-repository model that owns the GitHub-sourced metadata lifecycle for a
/// single repo — the values fetched through the (relatively expensive) `gh`
/// CLI rather than local `git`:
///   - `pr_info` for the current branch (`gh pr view`), and
///   - `repository_info` (name/owner) for the repo (`gh repo view`).
///
/// `GitHubRepoModel` is created lazily when a consumer asks for it via
/// [`crate::code_review::git_repo_model::GitRepoModels::subscribe_github_repo`].
/// While at least one strong `ModelHandle<GitHubRepoModel>` is alive, the model:
///   - tracks the current branch by subscribing to its sibling
///     [`GitRepoStatusModel`] for `MetadataChanged` events,
///   - fetches `gh pr view` for the current branch on creation, on branch
///     change, and on a periodic timer,
///   - fetches `gh repo view` on creation and re-checks it on the periodic
///     timer (independent of the branch), and
///   - emits [`GitHubRepoEvent`] when the cached PR or repository info moves.
///
/// `repository_info` is intentionally NOT refreshed on branch change: the
/// repo's name/owner does not depend on the checked-out branch, so a branch
/// flip must not trigger a fresh `gh repo view`.
///
/// When the last strong handle is dropped, the model is torn down and any
/// in-flight `gh` fetch is aborted. The sibling [`GitRepoStatusModel`] is
/// retained via a strong handle, so creating a `LocalGitHubRepoModel` keeps git
/// status alive for as long as GitHub info is needed.
pub struct LocalGitHubRepoModel {
    repo_path: PathBuf,
    /// Strong handle to the sibling git-status model. Keeps it alive so we
    /// always have a branch source.
    git_status: ModelHandle<GitRepoStatusModel>,
    /// Current branch name, mirrored from `git_status`. `None` until the
    /// sibling's metadata is available.
    branch: Option<String>,
    /// PR info for `branch`. `None` means no fetch has succeeded yet, the
    /// branch has no PR, or fetching is suppressed (gh missing/auth error).
    pr_info: Option<PrInfo>,
    /// Repository info (name/owner) returned by `gh repo view`. Branch-
    /// independent; fetched on creation and re-checked on the periodic tick.
    repository_info: Option<RepositoryInfo>,
    /// Handle for the in-flight `gh pr view` fetch, if any. Aborted in `Drop`.
    /// Used to avoid overlapping PR-info fetches; branch changes abort the
    /// current handle before starting a new branch's fetch.
    refreshing_pr_info_abort_handle: Option<SpawnedFutureHandle>,
    /// Handle for the in-flight `gh repo view` fetch, if any. Aborted in
    /// `Drop`. Used to avoid overlapping repository-info fetches.
    repository_info_abort_handle: Option<SpawnedFutureHandle>,
    /// Handle for the pending periodic-refresh tick. Aborted in `Drop` so
    /// the timer doesn't outlive the model.
    periodic_refresh_handle: Option<SpawnedFutureHandle>,
}

impl Entity for LocalGitHubRepoModel {
    type Event = GitHubRepoEvent;
}

impl LocalGitHubRepoModel {
    /// Create a new per-repo GitHub-info model.
    ///
    /// Subscribes to `git_status` for `MetadataChanged` events to track the
    /// current branch. Seeds `branch` from the sibling's current metadata
    /// (if any) and kicks off an initial PR fetch when the branch is known.
    /// Also schedules a periodic refresh so a previously-suppressed default
    /// chip can recover after the user installs/authenticates `gh`, and kicks
    /// off the one-shot `gh repo view` fetch.
    pub(crate) fn new(
        repo_path: PathBuf,
        git_status: ModelHandle<GitRepoStatusModel>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let branch = git_status
            .as_ref(ctx)
            .metadata(ctx)
            .map(|m| m.current_branch_name.clone());

        // Track branch changes from the sibling. Only PR info depends on the
        // branch — repository info is deliberately left untouched here.
        ctx.subscribe_to_model(&git_status, |me, _, event, ctx| match event {
            GitRepoStatusEvent::MetadataChanged => {
                let new_branch = me
                    .git_status
                    .as_ref(ctx)
                    .metadata(ctx)
                    .map(|m| m.current_branch_name.clone());
                if new_branch != me.branch {
                    me.branch = new_branch;
                    if me.pr_info.take().is_some() {
                        ctx.emit(GitHubRepoEvent::PrInfoChanged);
                    }
                    if let Some(handle) = me.refreshing_pr_info_abort_handle.take() {
                        handle.abort();
                    }
                    me.refresh_pr_info(ctx);
                }
            }
        });

        let mut model = Self {
            repo_path,
            git_status,
            branch,
            pr_info: None,
            repository_info: None,
            refreshing_pr_info_abort_handle: None,
            repository_info_abort_handle: None,
            periodic_refresh_handle: None,
        };

        // Schedule periodic refresh of PR info and repository info.
        // This is necessary to recover from transient `gh` command failures.
        model.schedule_periodic_refresh(ctx);

        // Fetch repository info which is branch-independent.
        model.refresh_repository_info(ctx);

        // Fetch PR info if the branch is known.
        if model.branch.is_some() {
            model.refresh_pr_info(ctx);
        }
        model
    }

    /// Schedules a periodic timer that refreshes PR info and repository info.
    fn schedule_periodic_refresh(&mut self, ctx: &mut ModelContext<Self>) {
        let handle = ctx.spawn(
            async {
                async_io::Timer::after(GITHUB_INFO_PERIODIC_REFRESH).await;
            },
            |me, _, ctx| {
                me.refresh_pr_info(ctx);
                me.refresh_repository_info(ctx);
                me.schedule_periodic_refresh(ctx);
            },
        );
        self.periodic_refresh_handle = Some(handle);
    }

    /// PR info for the current branch.
    pub fn pr_info(&self) -> Option<&PrInfo> {
        self.pr_info.as_ref()
    }

    /// Repository info (name/owner) returned by `gh repo view`.
    pub fn repository_info(&self) -> Option<&RepositoryInfo> {
        self.repository_info.as_ref()
    }

    /// Whether a `gh pr view` fetch is currently in flight.
    pub fn is_refreshing_pr_info(&self) -> bool {
        self.refreshing_pr_info_abort_handle.is_some()
    }

    /// Manually trigger a PR-info refresh. Called after `gh`/`gt` commands
    /// complete, since those don't touch `.git/` so the filesystem watcher won't
    /// catch them.
    pub fn refresh_pr_info(&mut self, ctx: &mut ModelContext<Self>) {
        let Some(branch) = self.branch.clone() else {
            return;
        };
        // Branch changes abort in-flight fetches, so any handle
        // here is already for the current branch.
        if self.refreshing_pr_info_abort_handle.is_some() {
            return;
        }
        let repo_path = self.repo_path.clone();
        #[cfg(feature = "local_tty")]
        let path_future = {
            // Use the shell's interactive PATH so `gh` can be found when Warp
            // was launched outside of a login shell, e.g. from the macOS GUI.
            LocalShellState::handle(ctx).update(ctx, |shell_state, ctx| {
                shell_state.get_interactive_path_env_var(ctx)
            })
        };
        #[cfg(not(feature = "local_tty"))]
        let path_future = futures::future::ready(None);
        let branch_for_callback = branch.clone();
        let abort_handle = ctx.spawn(
            async move {
                let path_env = path_future.await;
                let fetch = get_pr_for_branch(&repo_path, path_env.as_deref());
                let timeout = async_io::Timer::after(PR_INFO_FETCH_TIMEOUT);
                futures::pin_mut!(fetch);
                match futures::future::select(fetch, timeout).await {
                    futures::future::Either::Left((result, _)) => result,
                    futures::future::Either::Right((_, _)) => {
                        Err(anyhow::anyhow!("PR info fetch timed out"))
                    }
                }
            },
            move |me, result, ctx| {
                me.refreshing_pr_info_abort_handle = None;
                me.handle_fetch_result(result, branch_for_callback, ctx);
            },
        );
        self.refreshing_pr_info_abort_handle = Some(abort_handle);
    }

    /// Fetch repository info (`gh repo view`). Branch-independent: kicked off
    /// on creation and re-checked by the periodic timer on each tick. Never
    /// called from the branch-change path, so switching branches does not
    /// trigger a `gh repo view`.
    pub fn refresh_repository_info(&mut self, ctx: &mut ModelContext<Self>) {
        // Guard against overlapping fetches.
        if self.repository_info_abort_handle.is_some() {
            return;
        }
        let repo_path = self.repo_path.clone();
        #[cfg(feature = "local_tty")]
        let path_future = {
            // Use the shell's interactive PATH so `gh` can be found when Warp
            // was launched outside of a login shell, e.g. from the macOS GUI.
            LocalShellState::handle(ctx).update(ctx, |shell_state, ctx| {
                shell_state.get_interactive_path_env_var(ctx)
            })
        };
        #[cfg(not(feature = "local_tty"))]
        let path_future = futures::future::ready(None);
        self.repository_info_abort_handle = Some(ctx.spawn(
            async move {
                let path_env = path_future.await;
                let fetch = get_repository_info(&repo_path, path_env.as_deref());
                let timeout = async_io::Timer::after(REPOSITORY_INFO_FETCH_TIMEOUT);
                futures::pin_mut!(fetch);
                match futures::future::select(fetch, timeout).await {
                    futures::future::Either::Left((result, _)) => result,
                    futures::future::Either::Right((_, _)) => {
                        Err(anyhow::anyhow!("Repository info fetch timed out"))
                    }
                }
            },
            |me, result, ctx| {
                me.repository_info_abort_handle = None;
                me.handle_repository_info_result(result, ctx);
            },
        ));
    }

    fn handle_repository_info_result(
        &mut self,
        result: anyhow::Result<Option<RepositoryInfo>>,
        ctx: &mut ModelContext<Self>,
    ) {
        match result {
            Ok(repository_info) => {
                if self.repository_info != repository_info {
                    self.repository_info = repository_info;
                    ctx.emit(GitHubRepoEvent::RepositoryInfoChanged);
                }
            }
            Err(err) => {
                log::debug!("GitHubRepoModel: repository info load failed: {err}");
            }
        }
    }

    fn handle_fetch_result(
        &mut self,
        result: anyhow::Result<Option<PrInfo>>,
        branch: String,
        ctx: &mut ModelContext<Self>,
    ) {
        match result {
            Ok(pr_info) => {
                Self::maybe_validate_github_pr_default(ctx);
                // Only emit when the updated branch is still current.
                if self.branch.as_deref() == Some(branch.as_str()) {
                    let changed = self.pr_info.as_ref() != pr_info.as_ref();
                    self.pr_info = pr_info;
                    if changed {
                        ctx.emit(GitHubRepoEvent::PrInfoChanged);
                    }
                }
            }
            Err(e) => {
                let error_msg = e.to_string();
                if is_gh_missing_error(&error_msg) || is_gh_auth_error(&error_msg) {
                    log::info!(
                        "GitHubRepoModel: suppressing default PR chip \
                         due to deterministic gh setup error"
                    );
                    if self.pr_info.take().is_some() {
                        ctx.emit(GitHubRepoEvent::PrInfoChanged);
                    }
                    Self::maybe_suppress_github_pr_default(ctx);
                }
                // On transient errors, keep existing PR info to avoid
                // flashing the UI.
            }
        }
    }

    fn maybe_suppress_github_pr_default(ctx: &mut ModelContext<Self>) {
        let current = *SessionSettings::as_ref(ctx).github_pr_chip_default_validation;
        if current != GithubPrPromptChipDefaultValidation::Suppressed {
            SessionSettings::handle(ctx).update(ctx, |settings, ctx| {
                report_if_error!(settings
                    .github_pr_chip_default_validation
                    .set_value(GithubPrPromptChipDefaultValidation::Suppressed, ctx));
            });
        }
    }

    fn maybe_validate_github_pr_default(ctx: &mut ModelContext<Self>) {
        let current = *SessionSettings::as_ref(ctx).github_pr_chip_default_validation;
        if current != GithubPrPromptChipDefaultValidation::Validated {
            SessionSettings::handle(ctx).update(ctx, |settings, ctx| {
                report_if_error!(settings
                    .github_pr_chip_default_validation
                    .set_value(GithubPrPromptChipDefaultValidation::Validated, ctx));
            });
        }
    }
}

#[cfg(test)]
impl LocalGitHubRepoModel {
    /// Inert constructor: no branch-tracking subscription, timers, or `gh`
    /// fetch, so tests stay deterministic and never spawn a real subprocess.
    /// Drive state via the `set_*_for_test` helpers.
    pub(crate) fn new_for_test(git_status: ModelHandle<GitRepoStatusModel>) -> Self {
        Self {
            repo_path: PathBuf::from("/test"),
            git_status,
            branch: None,
            pr_info: None,
            repository_info: None,
            refreshing_pr_info_abort_handle: None,
            repository_info_abort_handle: None,
            periodic_refresh_handle: None,
        }
    }

    pub(crate) fn set_pr_info_for_test(
        &mut self,
        pr_info: Option<PrInfo>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.pr_info = pr_info;
        ctx.emit(GitHubRepoEvent::PrInfoChanged);
    }

    pub(crate) fn set_repository_info_for_test(
        &mut self,
        repository_info: Option<RepositoryInfo>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.repository_info = repository_info;
        ctx.emit(GitHubRepoEvent::RepositoryInfoChanged);
    }
}

#[cfg(test)]
#[path = "local_tests.rs"]
mod tests;

impl Drop for LocalGitHubRepoModel {
    fn drop(&mut self) {
        if let Some(h) = self.refreshing_pr_info_abort_handle.take() {
            h.abort();
        }
        if let Some(h) = self.repository_info_abort_handle.take() {
            h.abort();
        }
        if let Some(h) = self.periodic_refresh_handle.take() {
            h.abort();
        }
    }
}
