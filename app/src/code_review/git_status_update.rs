#[cfg(feature = "local_fs")]
use std::path::{Path, PathBuf};

#[cfg(feature = "local_fs")]
use settings::Setting as _;
#[cfg(feature = "local_fs")]
use warpui::ModelContext;
use warpui::{Entity, SingletonEntity};
#[cfg(feature = "local_fs")]
use {
    crate::report_if_error,
    crate::terminal::session_settings::{GithubPrPromptChipDefaultValidation, SessionSettings},
    crate::throttle::throttle,
    crate::util::git::{
        detect_current_branch_display, detect_main_branch, get_pr_for_branch, get_repository_info,
        is_gh_auth_error, is_gh_missing_error,
    },
    async_channel::Sender,
    repo_metadata::{
        repositories::DetectedRepositories,
        repository::{RepositorySubscriber, SubscriberId},
        Repository, RepositoryUpdate,
    },
    std::{
        collections::{HashMap, HashSet},
        time::Duration,
    },
    warpui::{r#async::SpawnedFutureHandle, EntityId, ModelHandle, WeakModelHandle},
};

#[cfg(feature = "local_fs")]
use super::diff_state::{diff_metadata_against_head, DiffStats};
use crate::util::git::{PrInfo, RepositoryInfo};
#[cfg(feature = "local_fs")]
const PR_INFO_FETCH_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(feature = "local_fs")]
const REPOSITORY_INFO_FETCH_TIMEOUT: Duration = Duration::from_secs(5);

/// Public metadata exposed to consumers — the subset of diff metadata
/// that the git chip (prompt display, agent view footer) needs.
#[cfg(feature = "local_fs")]
#[derive(Debug, Clone)]
pub struct GitStatusMetadata {
    pub current_branch_name: String,
    pub main_branch_name: String,
    pub stats_against_head: DiffStats,
}

// ── GitStatusUpdateModel (singleton cache) ──────────────────────────────────

/// Singleton model that acts as a cache / factory for per-repository
/// [`GitRepoStatusModel`] instances.
///
/// Multiple terminals in the same repo share a single sub-model.  When the last
/// strong handle to a sub-model is dropped, the watcher is torn down
/// automatically.
pub struct GitStatusUpdateModel {
    #[cfg(feature = "local_fs")]
    repos: HashMap<PathBuf, WeakModelHandle<GitRepoStatusModel>>,
}

// ── Non-local_fs stub ───────────────────────────────────────────────────────

#[cfg(not(feature = "local_fs"))]
#[allow(dead_code)]
impl GitStatusUpdateModel {
    pub fn new() -> Self {
        Self {}
    }
}

// ── local_fs implementation ─────────────────────────────────────────────────

#[cfg(feature = "local_fs")]
impl GitStatusUpdateModel {
    pub fn new() -> Self {
        Self {
            repos: HashMap::new(),
        }
    }

    /// Get or create a per-repo status model for `repo_path`.
    ///
    /// If a live model already exists for this path, returns a new strong handle
    /// to it.  Otherwise, creates a new [`GitRepoStatusModel`] with an active
    /// filesystem watcher and returns a handle to it.
    ///
    /// Callers hold the returned `ModelHandle` for as long as they need updates.
    /// When all handles are dropped, the model (and its watcher) is torn down.
    /// Callers that need PR info should call
    /// [`GitRepoStatusModel::set_pr_info_consumer`] on the returned handle
    /// after subscribing.
    pub fn subscribe(
        &mut self,
        repo_path: &Path,
        ctx: &mut ModelContext<Self>,
    ) -> anyhow::Result<ModelHandle<GitRepoStatusModel>> {
        let repo_path_buf = repo_path.to_path_buf();

        // Check the cache for an existing live model.
        if let Some(weak) = self.repos.get(&repo_path_buf) {
            if let Some(handle) = weak.upgrade(ctx) {
                return Ok(handle);
            }
        }

        // Create a new sub-model.
        let Some(repository_model) =
            DetectedRepositories::as_ref(ctx).get_local_watched_repo_for_path(repo_path, ctx)
        else {
            anyhow::bail!(
                "No watched repository found for path: {}",
                repo_path.display()
            );
        };

        let handle = ctx
            .add_model(|ctx| GitRepoStatusModel::new(repo_path_buf.clone(), repository_model, ctx));

        self.repos.insert(repo_path_buf, handle.downgrade());
        Ok(handle)
    }
}

impl Entity for GitStatusUpdateModel {
    type Event = ();
}

impl SingletonEntity for GitStatusUpdateModel {}

// ── GitRepoStatusModel ──────────────────────────────────────────────────────

/// State for an in-flight `gh pr view` lookup. The branch and abort handle
/// are always set together: a non-`None` value means a fetch is currently
/// running for that branch.
#[cfg(feature = "local_fs")]
struct RefreshingPrInfo {
    /// Branch name that the in-flight `refresh_pr_info` is fetching for.
    branch: String,
    /// Handle to abort the in-flight fetch.
    abort_handle: SpawnedFutureHandle,
}

/// Per-repository model that owns the filesystem watcher and exposes git status
/// metadata.  Consumers hold a `ModelHandle<GitRepoStatusModel>` and subscribe
/// to its events directly — no path-filtering required.
///
/// When all strong handles are dropped the model (and its watcher) is
/// automatically torn down.
#[cfg(feature = "local_fs")]
pub struct GitRepoStatusModel {
    repo_path: PathBuf,
    repository: ModelHandle<Repository>,
    subscriber_id: Option<SubscriberId>,
    metadata: Option<GitStatusMetadata>,
    computing_metadata_abort_handle: Option<SpawnedFutureHandle>,
    computing_repository_info_abort_handle: Option<SpawnedFutureHandle>,
    repository_info: Option<RepositoryInfo>,
    /// In-flight PR info fetch state. Used to make `refresh_pr_info`
    /// idempotent: while a fetch for the current branch is in flight,
    /// additional calls are no-ops.
    refreshing_pr_info: Option<RefreshingPrInfo>,
    /// Consumers that currently need PR info. Git diff/branch consumers can
    /// share this model without paying for `gh pr view`.
    pr_info_consumers: HashSet<EntityId>,
    /// PR info for the current branch.
    pr_info: Option<PrInfo>,
}

#[cfg(not(feature = "local_fs"))]
pub struct GitRepoStatusModel;

#[cfg(not(feature = "local_fs"))]
impl Entity for GitRepoStatusModel {
    type Event = ();
}

#[cfg(not(feature = "local_fs"))]
impl GitRepoStatusModel {
    pub fn repository_info(&self) -> Option<&RepositoryInfo> {
        None
    }
    pub fn pr_info(&self) -> Option<&PrInfo> {
        None
    }
}

#[cfg(feature = "local_fs")]
#[derive(Debug)]
pub enum GitRepoStatusEvent {
    /// Emitted whenever the metadata changes (branch name, diff stats, etc.).
    MetadataChanged,
    /// Emitted when PR info changes (fetched, cleared on branch change, etc.).
    PrInfoChanged,
}

#[cfg(feature = "local_fs")]
impl Entity for GitRepoStatusModel {
    type Event = GitRepoStatusEvent;
}

#[cfg(feature = "local_fs")]
impl GitRepoStatusModel {
    /// Create a new per-repo status model, set up the filesystem watcher, and
    /// kick off the initial metadata computation.
    fn new(
        repo_path: PathBuf,
        repository_model: ModelHandle<Repository>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let mut model = Self {
            repo_path: repo_path.clone(),
            repository: repository_model.clone(),
            subscriber_id: None,
            metadata: None,
            computing_metadata_abort_handle: None,
            computing_repository_info_abort_handle: None,
            repository_info: None,
            refreshing_pr_info: None,
            pr_info_consumers: HashSet::new(),
            pr_info: None,
        };

        // Kick off initial metadata computation.
        // The first `refresh_pr_info` is triggered from `handle_metadata_result`
        // once metadata lands (branch is known), avoiding a race where PR info
        // arrives before metadata exists to store it.
        model.refresh_metadata(ctx);

        // Start watching for filesystem changes.
        let (repository_update_tx, repository_update_rx) = async_channel::unbounded();
        let (throttled_tx, throttled_rx) = async_channel::unbounded();
        let start = repository_model.update(ctx, |repo, ctx| {
            repo.start_watching(
                Box::new(GitStatusRepositorySubscriber {
                    repository_update_tx,
                }),
                ctx,
            )
        });
        model.subscriber_id = Some(start.subscriber_id);

        // Handle watcher registration.
        ctx.spawn(start.registration_future, |me, result, ctx| {
            if let Err(err) = result {
                log::warn!("GitRepoStatusModel: watcher registration failed: {err}");
                if let Some(subscriber_id) = me.subscriber_id.take() {
                    me.repository.update(ctx, |repo, ctx| {
                        repo.stop_watching(subscriber_id, ctx);
                    });
                }
            }
        });

        // Stream raw updates; determine whether a throttled metadata refresh is warranted.
        {
            let throttled_tx_clone = throttled_tx;
            ctx.spawn_stream_local(
                repository_update_rx,
                move |_me, update: RepositoryUpdate, _ctx| {
                    if Self::should_refresh_metadata(&update) {
                        let _ = throttled_tx_clone.try_send(());
                    }
                },
                |_, _| {},
            );
        }

        // Throttled metadata refresh (at most once every 5 seconds).
        ctx.spawn_stream_local(
            throttle(Duration::from_secs(5), throttled_rx),
            |me, _, ctx| {
                me.refresh_metadata(ctx);
            },
            |_, _| {},
        );

        // Periodic PR info re-check (every 60 seconds). This also lets a
        // previously suppressed default PR chip recover after `gh` is installed
        // or authenticated.
        {
            let (pr_tick_tx, pr_tick_rx) = async_channel::unbounded();
            ctx.spawn_stream_local(
                pr_tick_rx,
                |me, _: (), ctx| {
                    me.refresh_pr_info(ctx);
                },
                |_, _| {},
            );
            ctx.spawn(
                async move {
                    loop {
                        async_io::Timer::after(Duration::from_secs(60)).await;
                        if pr_tick_tx.send(()).await.is_err() {
                            break;
                        }
                    }
                },
                |_, _, _| {},
            );
        }

        model
    }

    /// Read the current metadata.  Returns `None` if metadata hasn't been
    /// computed yet.
    pub fn metadata(&self) -> Option<&GitStatusMetadata> {
        self.metadata.as_ref()
    }

    /// Repository info returned by `gh repo view`.
    pub fn repository_info(&self) -> Option<&RepositoryInfo> {
        self.repository_info.as_ref()
    }

    /// Whether a PR info fetch is currently in flight.
    pub fn is_refreshing_pr_info(&self) -> bool {
        self.refreshing_pr_info.is_some()
    }

    /// PR info for the current branch.
    pub fn pr_info(&self) -> Option<&PrInfo> {
        self.pr_info.as_ref()
    }

    pub(crate) fn should_refresh_pr_info(&self) -> bool {
        !self.pr_info_consumers.is_empty()
    }

    pub fn set_pr_info_consumer(
        &mut self,
        consumer_id: EntityId,
        enabled: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        let was_enabled = self.should_refresh_pr_info();
        if enabled {
            self.pr_info_consumers.insert(consumer_id);
        } else {
            self.pr_info_consumers.remove(&consumer_id);
        }

        if self.should_refresh_pr_info() && !was_enabled {
            self.refresh_pr_info(ctx);
        } else if !self.should_refresh_pr_info() {
            if let Some(refreshing) = self.refreshing_pr_info.take() {
                refreshing.abort_handle.abort();
            }
        }
    }

    /// The path to the repository root.
    pub fn repo_path(&self) -> &Path {
        &self.repo_path
    }

    /// Manually trigger a metadata refresh.  Called by the terminal view after
    /// events that may have changed git state (block completed, agent file
    /// edits, etc.).
    pub fn refresh_metadata(&mut self, ctx: &mut ModelContext<Self>) {
        self.refresh_repository_info(ctx);
        if let Some(handle) = self.computing_metadata_abort_handle.take() {
            handle.abort();
        }
        let repo_path_buf = self.repo_path.clone();
        self.computing_metadata_abort_handle = Some(ctx.spawn(
            async move { Self::load_metadata(repo_path_buf).await },
            |me, result, ctx| {
                me.handle_metadata_result(result, ctx);
            },
        ));
    }

    // ── internal helpers ────────────────────────────────────────────────

    fn refresh_repository_info(&mut self, ctx: &mut ModelContext<Self>) {
        if let Some(handle) = self.computing_repository_info_abort_handle.take() {
            handle.abort();
        }
        let repo_path = self.repo_path.clone();
        #[cfg(feature = "local_tty")]
        let path_future = {
            // Use the shell's interactive PATH so `gh` can be found when Warp
            // was launched outside of a login shell, e.g. from the macOS GUI.
            use crate::terminal::local_shell::LocalShellState;
            LocalShellState::handle(ctx).update(ctx, |shell_state, ctx| {
                shell_state.get_interactive_path_env_var(ctx)
            })
        };
        #[cfg(not(feature = "local_tty"))]
        let path_future = futures::future::ready(None);
        self.computing_repository_info_abort_handle = Some(ctx.spawn(
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
                let repository_info = match result {
                    Ok(repository_info) => repository_info,
                    Err(err) => {
                        log::debug!("GitRepoStatusModel: repository info load failed: {err}");
                        None
                    }
                };
                if me.repository_info != repository_info {
                    me.repository_info = repository_info;
                    ctx.emit(GitRepoStatusEvent::MetadataChanged);
                }
            },
        ));
    }

    /// Fetch PR info. Called by the periodic timer and after `gh`/`gt`
    /// commands. Missing/auth setup failures suppress the default chip, but
    /// polling continues so the chip can recover after the user's `gh` setup
    /// changes.
    pub(crate) fn refresh_pr_info(&mut self, ctx: &mut ModelContext<Self>) {
        if !self.should_refresh_pr_info() {
            return;
        }

        // Skip if metadata hasn't loaded yet — there's no branch context to
        // store the result against. The initial fetch is triggered from
        // `handle_metadata_result` once metadata lands.
        let Some(branch) = self
            .metadata
            .as_ref()
            .map(|m| m.current_branch_name.clone())
        else {
            return;
        };

        // If we're already fetching for the current branch, let that fetch
        // complete. Otherwise, abort the stale fetch and start a fresh one.
        if self
            .refreshing_pr_info
            .as_ref()
            .is_some_and(|r| r.branch == branch)
        {
            return;
        }
        if let Some(refreshing) = self.refreshing_pr_info.take() {
            refreshing.abort_handle.abort();
        }
        let repo_path = self.repo_path.clone();
        #[cfg(feature = "local_tty")]
        let path_future = {
            // Use the shell's interactive PATH so `gh` can be found when Warp
            // was launched outside of a login shell, e.g. from the macOS GUI.
            use crate::terminal::local_shell::LocalShellState;
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
                me.refreshing_pr_info = None;
                match result {
                    Ok(pr_info) => {
                        Self::maybe_validate_github_pr_default(ctx);
                        // Only emit when the updated branch is still current.
                        if me
                            .metadata
                            .as_ref()
                            .is_some_and(|m| m.current_branch_name == branch_for_callback)
                        {
                            let changed = me.pr_info.as_ref() != pr_info.as_ref();
                            me.pr_info = pr_info;
                            if changed {
                                ctx.emit(GitRepoStatusEvent::PrInfoChanged);
                            }
                        }
                    }
                    Err(e) => {
                        let error_msg = e.to_string();
                        if is_gh_missing_error(&error_msg) || is_gh_auth_error(&error_msg) {
                            log::info!(
                                "GitRepoStatusModel: suppressing default PR chip \
                                 due to deterministic gh setup error"
                            );
                            if me.pr_info.take().is_some() {
                                ctx.emit(GitRepoStatusEvent::PrInfoChanged);
                            }
                            Self::maybe_suppress_github_pr_default(ctx);
                        }
                        // On error, keep existing PR info to avoid flashing
                        // the UI on transient network failures.
                    }
                }
            },
        );
        self.refreshing_pr_info = Some(RefreshingPrInfo {
            branch,
            abort_handle,
        });
    }

    fn handle_metadata_result(
        &mut self,
        result: anyhow::Result<GitStatusMetadata>,
        ctx: &mut ModelContext<Self>,
    ) {
        let previous_branch = self
            .metadata
            .as_ref()
            .map(|m| m.current_branch_name.clone());

        match result {
            Ok(metadata) => {
                self.metadata = Some(metadata);
            }
            Err(e) => {
                log::warn!("GitRepoStatusModel: metadata load failed: {e}");
                self.metadata = None;
                ctx.emit(GitRepoStatusEvent::MetadataChanged);
                if self.pr_info.take().is_some() {
                    ctx.emit(GitRepoStatusEvent::PrInfoChanged);
                }
                return;
            }
        }
        ctx.emit(GitRepoStatusEvent::MetadataChanged);

        let current_branch = self
            .metadata
            .as_ref()
            .map(|m| m.current_branch_name.clone());

        // Refresh PR info on branch change. Also handles the initial metadata
        // load (previous_branch: None → current_branch: Some). The 60-second
        // periodic timer handles picking up externally created PRs.
        if previous_branch != current_branch {
            if self.pr_info.take().is_some() {
                ctx.emit(GitRepoStatusEvent::PrInfoChanged);
            }
            self.refresh_pr_info(ctx);
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

    /// Decide whether a `RepositoryUpdate` warrants a metadata refresh.
    fn should_refresh_metadata(update: &RepositoryUpdate) -> bool {
        if update.is_empty() {
            return false;
        }
        if update.commit_updated || update.index_lock_detected || update.remote_ref_updated {
            return true;
        }
        // Check if any non-ignored file was touched.
        let changed_count = update
            .added
            .iter()
            .chain(&update.modified)
            .chain(&update.deleted)
            .chain(update.moved.keys())
            .chain(update.moved.values())
            .filter(|f| !f.is_ignored)
            .count();
        changed_count > 0
    }

    /// Compute metadata for a repo — branch names and diff stats against HEAD.
    ///
    /// This reuses logic extracted from `DiffStateModel::load_metadata_for_repo`
    /// but only computes the HEAD (uncommitted) stats since that's all the git
    /// chip needs.
    async fn load_metadata(repo_path: PathBuf) -> anyhow::Result<GitStatusMetadata> {
        // Detect main branch.
        let main_branch_name = detect_main_branch(&repo_path).await?;
        // Detect current branch (using the display variant so detached HEAD
        // shows the short SHA instead of the literal "HEAD").
        let current_branch_name = detect_current_branch_display(&repo_path).await?;
        // Diff stats against HEAD.
        let stats_against_head = diff_metadata_against_head(&repo_path).await?;

        Ok(GitStatusMetadata {
            current_branch_name,
            main_branch_name,
            stats_against_head: stats_against_head.aggregate_stats,
        })
    }
}

#[cfg(all(test, feature = "local_fs"))]
impl GitRepoStatusModel {
    pub(crate) fn new_for_test(
        repository: ModelHandle<Repository>,
        metadata: Option<GitStatusMetadata>,
    ) -> Self {
        Self {
            repo_path: PathBuf::from("/test"),
            repository,
            subscriber_id: None,
            metadata,
            computing_metadata_abort_handle: None,
            computing_repository_info_abort_handle: None,
            repository_info: None,
            refreshing_pr_info: None,
            pr_info_consumers: HashSet::new(),
            pr_info: None,
        }
    }

    pub(crate) fn set_metadata_for_test(
        &mut self,
        metadata: Option<GitStatusMetadata>,
        ctx: &mut ModelContext<Self>,
    ) {
        let previous_branch = self
            .metadata
            .as_ref()
            .map(|m| m.current_branch_name.clone());
        let current_branch = metadata.as_ref().map(|m| m.current_branch_name.clone());
        self.metadata = metadata;
        ctx.emit(GitRepoStatusEvent::MetadataChanged);
        if previous_branch != current_branch && self.pr_info.take().is_some() {
            ctx.emit(GitRepoStatusEvent::PrInfoChanged);
        }
    }

    pub(crate) fn set_repository_info_for_test(
        &mut self,
        repository_info: Option<RepositoryInfo>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.repository_info = repository_info;
        ctx.emit(GitRepoStatusEvent::MetadataChanged);
    }

    #[cfg(test)]
    pub(crate) fn set_pr_info_for_test(
        &mut self,
        pr_info: Option<PrInfo>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.pr_info = pr_info;
        ctx.emit(GitRepoStatusEvent::PrInfoChanged);
    }
}

#[cfg(all(test, feature = "local_fs"))]
#[path = "git_status_update_tests.rs"]
mod tests;

#[cfg(feature = "local_fs")]
impl Drop for GitRepoStatusModel {
    fn drop(&mut self) {
        // Note: we cannot call `repository.update()` here because `Drop` does
        // not have access to `ModelContext`.  The `Repository` model will clean
        // up the subscriber when it notices the channel has been dropped.
        if let Some(handle) = self.computing_metadata_abort_handle.take() {
            handle.abort();
        }
        if let Some(handle) = self.computing_repository_info_abort_handle.take() {
            handle.abort();
        }
        if let Some(refreshing) = self.refreshing_pr_info.take() {
            refreshing.abort_handle.abort();
        }
    }
}

// ── Repository subscriber adapter ───────────────────────────────────────────

#[cfg(feature = "local_fs")]
struct GitStatusRepositorySubscriber {
    repository_update_tx: Sender<RepositoryUpdate>,
}

#[cfg(feature = "local_fs")]
impl RepositorySubscriber for GitStatusRepositorySubscriber {
    fn on_scan(
        &mut self,
        _repository: &Repository,
        _ctx: &mut ModelContext<Repository>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'static>> {
        Box::pin(async {})
    }

    fn on_files_updated(
        &mut self,
        repository: &Repository,
        update: &RepositoryUpdate,
        _ctx: &mut ModelContext<Repository>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'static>> {
        let tx = self.repository_update_tx.clone();
        let update = update.clone();
        let index_lock_path = repository.git_dir().join("index.lock");
        Box::pin(async move {
            // Suppress commit_updated events while the git index is locked to
            // avoid reacting to stale intermediate state during git operations.
            if update.commit_updated && async_fs::metadata(&index_lock_path).await.is_ok() {
                return;
            }
            let _ = tx.send(update).await;
        })
    }
}
