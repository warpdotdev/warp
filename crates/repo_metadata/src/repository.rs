use std::collections::HashMap;
use std::future::Future;
#[cfg(feature = "local_fs")]
use std::path::{Component, Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::future::ready;
#[cfg(feature = "local_fs")]
use ignore::gitignore::Gitignore;
use warp_util::standardized_path::StandardizedPath;
#[cfg(feature = "local_fs")]
use warpui_core::SingletonEntity;
use warpui_core::r#async::{BoxFuture, SpawnedFutureHandle};
use warpui_core::{Entity, ModelContext, ModelHandle};

#[cfg(feature = "local_fs")]
use crate::watcher::DirectoryWatcher;
use crate::watcher::TaskQueue;
use crate::{RepoMetadataError, RepositoryUpdate};
#[cfg(feature = "local_fs")]
use crate::{
    entry::{matches_gitignores, should_ignore_git_path},
    gitignores_for_directory,
};

/// Trait for entities that want to subscribe to repository file changes.
pub trait RepositorySubscriber: Send + Sync {
    /// Called when the subscriber is first added to build initial state.
    /// Returns a Future that completes when the scan is finished.
    fn on_scan(
        &mut self,
        repository: &Repository,
        ctx: &mut ModelContext<Repository>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

    /// Called when file changes are detected in the repository.
    /// Returns a Future that completes once updates are processed.
    fn on_files_updated(
        &mut self,
        repository: &Repository,
        update: &RepositoryUpdate,
        ctx: &mut ModelContext<Repository>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

    fn on_unsubscribe(&mut self, _ctx: &mut ModelContext<Repository>) {}
}

/// A unique identifier for repository subscribers.
pub type SubscriberId = usize;
/// Controls which repository changes a subscriber receives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepositoryWatchMode {
    /// Watch ordinary filesystem content without activating Git metadata tracking.
    FilesystemOnly,
    /// Watch filesystem content and Git metadata such as commits, index locks, and upstream refs.
    GitRepository,
}

pub struct StartWatching {
    pub subscriber_id: SubscriberId,
    pub registration_future: BoxFuture<'static, Result<(), RepoMetadataError>>,
}
struct RepositorySubscription {
    #[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
    mode: RepositoryWatchMode,
    subscriber: Box<dyn RepositorySubscriber>,
}

/// Model for tracking a code repository that Warp is aware of.
pub struct Repository {
    /// The root directory of the repository.
    root_dir: StandardizedPath,
    /// External git directory path (e.g., for worktrees). This is the
    /// path to the **exact** per-worktree gitdir (e.g. `.git/worktrees/foo`).
    /// For the main worktree this is `None` (the gitdir is `root_dir/.git`).
    external_git_directory: Option<StandardizedPath>,
    /// The shared `.git` root directory that all worktrees of the same repo
    /// have in common. Derived from `external_git_directory` by walking up to
    /// the `.git` component. `None` when the repo is not a linked worktree.
    common_git_directory: Option<StandardizedPath>,
    /// Collection of subscribers interested in file changes.
    subscribers: HashMap<SubscriberId, RepositorySubscription>,
    /// Counter for generating unique subscriber IDs.
    next_subscriber_id: SubscriberId,
    /// Cached gitignore patterns for this repository.
    #[cfg(feature = "local_fs")]
    gitignores: Vec<Arc<Gitignore>>,
    /// Cached loose remote-tracking ref tracked by the active branch.
    #[cfg(feature = "local_fs")]
    tracked_remote_ref: Option<TrackedRemoteRef>,
    #[cfg(test)]
    tracked_remote_ref_refresh_count: usize,

    task_queue: ModelHandle<TaskQueue>,
}

#[cfg(feature = "local_fs")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TrackedRemoteRef {
    full_ref_name: String,
}

#[cfg(feature = "local_fs")]
impl TrackedRemoteRef {
    pub(crate) fn from_full_ref_name(full_ref_name: impl Into<String>) -> Option<Self> {
        let full_ref_name = full_ref_name.into();
        if !full_ref_name.starts_with("refs/remotes/") {
            return None;
        }
        let ref_path = Path::new(&full_ref_name);
        if ref_path.has_root() {
            return None;
        }
        let mut component_count = 0;
        for component in ref_path.components() {
            match component {
                Component::Normal(_) => component_count += 1,
                Component::CurDir => {}
                Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
            }
        }
        (component_count >= 4).then_some(Self { full_ref_name })
    }

    fn full_ref_name(&self) -> &str {
        &self.full_ref_name
    }
}

impl Repository {
    /// Creates a new Repository instance.
    pub(super) fn new(
        root_dir: StandardizedPath,
        external_git_directory: Option<StandardizedPath>,
        task_queue: ModelHandle<TaskQueue>,
    ) -> Self {
        #[cfg(feature = "local_fs")]
        let gitignores = {
            let local_path = root_dir.to_local_path_lossy();
            gitignores_for_directory(&local_path)
        };

        let common_git_directory = external_git_directory
            .as_ref()
            .and_then(Self::derive_common_git_directory);

        Self {
            root_dir,
            external_git_directory,
            common_git_directory,
            subscribers: HashMap::new(),
            next_subscriber_id: 0,
            #[cfg(feature = "local_fs")]
            gitignores,
            #[cfg(feature = "local_fs")]
            tracked_remote_ref: None,
            #[cfg(test)]
            tracked_remote_ref_refresh_count: 0,
            task_queue,
        }
    }

    /// Walk ancestors of the given path to find the `.git` component and return
    /// it as the shared git root. For example,
    /// `/repo/.git/worktrees/foo` → `/repo/.git`.
    fn derive_common_git_dir(external_git_dir: &std::path::Path) -> Option<std::path::PathBuf> {
        for ancestor in external_git_dir.ancestors() {
            if ancestor.file_name().and_then(|n| n.to_str()) == Some(".git") {
                return Some(ancestor.to_path_buf());
            }
        }
        None
    }

    fn derive_common_git_directory(
        external_git_directory: &StandardizedPath,
    ) -> Option<StandardizedPath> {
        external_git_directory
            .to_local_path()
            .and_then(|local| Self::derive_common_git_dir(&local))
            .and_then(|path| StandardizedPath::try_from_local(&path).ok())
            // Only store when it differs from external_git_directory.
            .filter(|common| common != external_git_directory)
    }

    /// The root directory of this repository.
    pub fn root_dir(&self) -> &StandardizedPath {
        &self.root_dir
    }

    /// The external git directory of this repository, if any.
    /// This is used for worktrees where the .git directory is external to the working tree.
    pub fn external_git_directory(&self) -> Option<&StandardizedPath> {
        self.external_git_directory.as_ref()
    }

    /// Adds linked-worktree git metadata when it was not known at registration time.
    ///
    /// Directory registrations can be created from a raw path before git detection completes.
    /// Preserve any metadata already associated with the repository, since later raw-path
    /// registrations must not downgrade a known linked worktree.
    #[cfg_attr(not(feature = "local_fs"), allow(unused_variables))]
    pub(super) fn enrich_external_git_directory(
        &mut self,
        external_git_directory: StandardizedPath,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.external_git_directory.is_some() {
            return;
        }

        self.common_git_directory = Self::derive_common_git_directory(&external_git_directory);
        self.external_git_directory = Some(external_git_directory);

        #[cfg(feature = "local_fs")]
        if self.has_git_repository_subscribers() {
            let git_paths = self.git_watch_paths();
            if !git_paths.is_empty() {
                let gitignores = self.gitignores.clone();
                let registration_future =
                    DirectoryWatcher::handle(ctx).update(ctx, move |watcher, ctx| {
                        watcher.start_watching_directories(git_paths, gitignores, ctx)
                    });
                ctx.spawn(registration_future, |_, _, _| {});
            }
        }
    }

    /// Returns the path to the actual `.git` directory for this repository.
    ///
    /// For normal repositories this is `root_dir/.git`. For worktrees, the
    /// `.git` entry in the working tree is a file (not a directory), so this
    /// returns the resolved `external_git_directory` instead.
    /// Subscribers should use this for per-worktree files like `index.lock`.
    pub fn git_dir(&self) -> std::path::PathBuf {
        self.external_git_directory
            .as_ref()
            .and_then(|d| d.to_local_path())
            .unwrap_or_else(|| self.root_dir.to_local_path_lossy().join(".git"))
    }

    /// Returns the shared `.git` root directory.
    ///
    /// For normal repos this is the same as `git_dir()`. For linked worktrees
    /// this is the common `.git` directory that all worktrees share (e.g.
    /// `/repo/.git`), distinct from the per-worktree gitdir.
    pub fn common_git_dir(&self) -> std::path::PathBuf {
        self.common_git_directory
            .as_ref()
            .and_then(|d| d.to_local_path())
            .unwrap_or_else(|| self.git_dir())
    }

    #[cfg(feature = "local_fs")]
    pub(crate) fn tracked_remote_ref_path(&self) -> Option<PathBuf> {
        self.tracked_remote_ref
            .as_ref()
            .map(|tracked_ref| self.common_git_dir().join(tracked_ref.full_ref_name()))
    }

    #[cfg(feature = "local_fs")]
    pub(crate) fn tracks_remote_ref_path(&self, remote_ref_path: &Path) -> bool {
        self.tracked_remote_ref_path().is_some_and(|tracked_path| {
            Self::path_for_comparison(&tracked_path) == Self::path_for_comparison(remote_ref_path)
        })
    }

    #[cfg(feature = "local_fs")]
    pub(crate) fn update_tracked_remote_ref(
        &mut self,
        tracked_remote_ref: Option<TrackedRemoteRef>,
    ) -> bool {
        if self.tracked_remote_ref == tracked_remote_ref {
            return false;
        }
        self.tracked_remote_ref = tracked_remote_ref;
        true
    }

    #[cfg(feature = "local_fs")]
    fn path_for_comparison(path: &Path) -> PathBuf {
        dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
    }

    #[cfg(feature = "local_fs")]
    pub(crate) async fn resolve_tracked_remote_ref(root_dir: PathBuf) -> Option<TrackedRemoteRef> {
        let output = warp_util::git::run_git_command(
            &root_dir,
            &["rev-parse", "--symbolic-full-name", "@{u}"],
        )
        .await
        .ok()?;
        let full_ref_name = output.lines().next()?.trim();
        TrackedRemoteRef::from_full_ref_name(full_ref_name)
    }

    #[cfg(feature = "local_fs")]
    pub(crate) fn refresh_tracked_remote_ref(
        &mut self,
        notify: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        if !self.has_git_repository_subscribers() {
            return;
        }

        #[cfg(test)]
        {
            self.tracked_remote_ref_refresh_count += 1;
        }
        let root_dir = self.root_dir().to_local_path_lossy();
        ctx.spawn(
            Repository::resolve_tracked_remote_ref(root_dir),
            move |repository, tracked_remote_ref, ctx| {
                if !repository.has_git_repository_subscribers() {
                    return;
                }
                let tracked_remote_ref_changed =
                    repository.update_tracked_remote_ref(tracked_remote_ref);
                if notify && tracked_remote_ref_changed {
                    repository.enqueue_remote_ref_update(ctx);
                }
            },
        );
    }

    #[cfg(feature = "local_fs")]
    fn enqueue_remote_ref_update(&mut self, ctx: &mut ModelContext<Self>) {
        let repository_handle = ctx.handle();
        let subscriber_ids = self.get_git_repository_subscriber_ids();
        let update = RepositoryUpdate {
            remote_ref_updated: true,
            ..Default::default()
        };
        self.task_queue.update(ctx, |queue, ctx| {
            for subscriber_id in subscriber_ids {
                queue.enqueue_incremental_update(
                    repository_handle.clone(),
                    subscriber_id,
                    update.clone(),
                    ctx,
                );
            }
        });
    }

    #[cfg(feature = "local_fs")]
    pub(crate) fn git_watch_paths(&self) -> Vec<StandardizedPath> {
        let mut paths = Vec::new();
        if let Some(external_git_dir) = &self.external_git_directory {
            paths.push(external_git_dir.clone());
        }
        if let Some(common_git_dir) = &self.common_git_directory
            && let Some(common_local) = common_git_dir.to_local_path()
        {
            let refs_dir = common_local.join("refs");
            if let Ok(refs_std) = StandardizedPath::from_local_canonicalized(&refs_dir) {
                paths.push(refs_std);
            }
            let config_file = common_local.join("config");
            if let Ok(config_std) = StandardizedPath::from_local_canonicalized(&config_file) {
                paths.push(config_std);
            }
        }
        paths
    }

    #[cfg(feature = "local_fs")]
    pub(crate) fn has_git_repository_subscribers(&self) -> bool {
        self.subscribers
            .values()
            .any(|subscription| subscription.mode == RepositoryWatchMode::GitRepository)
    }

    /// Returns the current watcher count.
    pub fn watcher_count(&self) -> usize {
        self.subscribers.len()
    }

    /// Starts watching this repository with the given subscriber.
    ///
    /// If this is the first subscriber, the repository root will be added to the
    /// RepositoryWatcher's set of watched paths.
    #[cfg_attr(not(feature = "local_fs"), allow(unused_variables))]
    pub fn start_watching(
        &mut self,
        mode: RepositoryWatchMode,
        subscriber: Box<dyn RepositorySubscriber>,
        ctx: &mut ModelContext<Self>,
    ) -> StartWatching {
        let subscriber_id = self.next_subscriber_id;
        self.next_subscriber_id += 1;

        // If this is the first subscriber, we need to start watching the repository
        #[cfg(feature = "local_fs")]
        let should_start_filesystem_watching = self.subscribers.is_empty();
        #[cfg(feature = "local_fs")]
        let should_start_git_watching =
            mode == RepositoryWatchMode::GitRepository && !self.has_git_repository_subscribers();

        self.subscribers
            .insert(subscriber_id, RepositorySubscription { mode, subscriber });

        #[cfg(feature = "local_fs")]
        let registration_future: BoxFuture<'static, Result<(), RepoMetadataError>> = {
            let mut directories_to_watch = Vec::new();
            if should_start_filesystem_watching {
                directories_to_watch.push(self.root_dir.clone());
            }
            if should_start_git_watching {
                directories_to_watch.extend(self.git_watch_paths());
            }

            if directories_to_watch.is_empty() {
                Box::pin(ready(Ok(())))
            } else {
                // Reuse the gitignores we already built at construction so the
                // watch descend filter doesn't re-read `.gitignore` from disk.
                let gitignores = self.gitignores.clone();

                Box::pin(
                    DirectoryWatcher::handle(ctx).update(ctx, move |watcher, ctx| {
                        watcher.start_watching_directories(directories_to_watch, gitignores, ctx)
                    }),
                )
            }
        };

        #[cfg(not(feature = "local_fs"))]
        let registration_future: BoxFuture<'static, Result<(), RepoMetadataError>> =
            Box::pin(async move { Ok(()) });

        let self_handle = ctx.handle();
        self.task_queue.update(ctx, |queue, ctx| {
            queue.enqueue_scan(self_handle, subscriber_id, ctx);
        });
        #[cfg(feature = "local_fs")]
        if should_start_git_watching {
            self.refresh_tracked_remote_ref(false, ctx);
        }

        StartWatching {
            subscriber_id,
            registration_future,
        }
    }

    /// Stops watching this repository for the given subscriber.
    ///
    /// If this was the last subscriber, the repository root will be removed from the
    /// RepositoryWatcher's set of watched paths.
    #[cfg_attr(not(feature = "local_fs"), allow(unused_variables))]
    pub fn stop_watching(&mut self, subscriber_id: SubscriberId, ctx: &mut ModelContext<Self>) {
        let Some(mut subscription) = self.subscribers.remove(&subscriber_id) else {
            return;
        };

        subscription.subscriber.on_unsubscribe(ctx);

        #[cfg(feature = "local_fs")]
        let should_stop_git_watching = subscription.mode == RepositoryWatchMode::GitRepository
            && !self.has_git_repository_subscribers();

        #[cfg(feature = "local_fs")]
        if should_stop_git_watching {
            self.tracked_remote_ref = None;
        }

        let should_stop_filesystem_watching = self.subscribers.is_empty();
        if should_stop_filesystem_watching {
            // If this was the last subscriber, notify the RepWatcher to stop watching.
            log::debug!(
                "All subscribers removed for {}, stopping watcher",
                self.root_dir
            );
        }

        #[cfg(feature = "local_fs")]
        if should_stop_filesystem_watching || should_stop_git_watching {
            let root_dir = self.root_dir.clone();
            let git_paths = if should_stop_git_watching {
                self.git_watch_paths()
            } else {
                Vec::new()
            };
            DirectoryWatcher::handle(ctx).update(ctx, |watcher, ctx| {
                if should_stop_filesystem_watching {
                    std::mem::drop(watcher.stop_watching_directory(&root_dir, ctx));
                }
                watcher.stop_watching_unused_git_directories(&root_dir, git_paths, ctx);
            });
        }
    }

    /// Calls scan on a specific subscriber if it exists. Returns Some(Future) if the subscriber exists, None otherwise.
    pub(crate) fn scan_subscriber(
        &mut self,
        subscriber_id: SubscriberId,
        ctx: &mut ModelContext<Self>,
    ) -> Option<Pin<Box<dyn Future<Output = ()> + Send + 'static>>> {
        if let Some(mut subscription) = self.subscribers.remove(&subscriber_id) {
            let future = subscription.subscriber.on_scan(self, ctx);
            self.subscribers.insert(subscriber_id, subscription);
            Some(future)
        } else {
            None
        }
    }

    /// Notifies a specific subscriber about file changes.
    #[cfg(feature = "local_fs")]
    pub(crate) fn notify_subscriber(
        &mut self,
        subscriber_id: SubscriberId,
        update: &RepositoryUpdate,
        ctx: &mut ModelContext<Self>,
    ) -> Option<Pin<Box<dyn Future<Output = ()> + Send + 'static>>> {
        if let Some(mut subscription) = self.subscribers.remove(&subscriber_id) {
            let future = subscription.subscriber.on_files_updated(self, update, ctx);
            self.subscribers.insert(subscriber_id, subscription);
            Some(future)
        } else {
            None
        }
    }

    /// Returns updates filtered for each subscriber's watch mode.
    #[cfg(feature = "local_fs")]
    pub(crate) fn subscriber_updates(
        &self,
        update: &RepositoryUpdate,
    ) -> Vec<(SubscriberId, RepositoryUpdate)> {
        self.subscribers
            .iter()
            .filter_map(|(&subscriber_id, subscription)| {
                let mut update = update.clone();
                if subscription.mode == RepositoryWatchMode::FilesystemOnly {
                    update.commit_updated = false;
                    update.index_lock_detected = false;
                    update.remote_ref_updated = false;
                }
                (!update.is_empty()).then_some((subscriber_id, update))
            })
            .collect()
    }

    #[cfg(feature = "local_fs")]
    fn get_git_repository_subscriber_ids(&self) -> Vec<SubscriberId> {
        self.subscribers
            .iter()
            .filter_map(|(&subscriber_id, subscription)| {
                (subscription.mode == RepositoryWatchMode::GitRepository).then_some(subscriber_id)
            })
            .collect()
    }

    /// Checks if a path is gitignored within this repository.
    #[cfg(feature = "local_fs")]
    pub fn check_gitignore_status(&self, path: &Path) -> bool {
        // Check if path is a .git internal file
        if should_ignore_git_path(path) {
            return true;
        }

        // Check if path matches gitignore patterns
        let is_dir = path.is_dir();
        matches_gitignores(path, is_dir, &self.gitignores, true)
    }
}

impl Entity for Repository {
    type Event = ();
}

/// Coalescing merge for RepositoryUpdate with normalization rules.
fn merge_repository_updates(acc: &mut RepositoryUpdate, incoming: &RepositoryUpdate) {
    // 1) Moves first
    for (to, from) in &incoming.moved {
        if acc.added.remove(from) {
            acc.added.insert(to.clone());
            return;
        }
        if acc.modified.remove(from) {
            acc.modified.insert(to.clone());
            return;
        }

        // Collapse chain: if `from` was a prior destination, pull its original source
        let original_from = if let Some(prev_from) = acc.moved.remove(from) {
            prev_from
        } else {
            from.clone()
        };
        acc.moved.insert(to.clone(), original_from);
    }

    // 2) Adds next
    for p in &incoming.added {
        acc.deleted.remove(p);
        acc.moved.remove(p);
        acc.modified.remove(p);
        acc.added.insert(p.clone());
    }

    // 3) Modifies next
    for p in &incoming.modified {
        if acc.added.contains(p) {
            continue;
        }
        acc.deleted.remove(p);
        acc.moved.remove(p);
        acc.modified.insert(p.clone());
    }

    // 4) Deletes last
    for p in &incoming.deleted {
        // Added then removed within window => cancel
        if acc.added.remove(p) {
            continue;
        }

        acc.modified.remove(p);

        // Removing a move target => delete original source instead
        if let Some(from) = acc.moved.remove(p) {
            acc.deleted.insert(from);
            continue;
        }
        // Deleting the source of a recorded move is redundant; move already implies source removal
        let is_from_of_some_move = acc.moved.values().any(|f| f == p);
        if is_from_of_some_move {
            continue;
        }
        acc.deleted.insert(p.clone());
    }

    acc.commit_updated |= incoming.commit_updated;
    acc.index_lock_detected |= incoming.index_lock_detected;
    acc.remote_ref_updated |= incoming.remote_ref_updated;
}

/// A generic debouncing layer for any RepositorySubscriber.
pub struct BufferingRepositorySubscriber<S> {
    inner: Arc<Mutex<S>>,
    state: Arc<Mutex<BufferState>>,
    debounce: Duration,
}

#[derive(Default)]
struct BufferState {
    pending: RepositoryUpdate,
    /// Monotonic counter incremented for each incoming update; used to implement true debounce.
    version: u64,
    /// Whether the background flusher loop is currently running.
    flush_handle: Option<SpawnedFutureHandle>,
}

impl<S> BufferingRepositorySubscriber<S> {
    pub fn new(inner: S, debounce: Duration) -> Self {
        Self {
            inner: Arc::new(Mutex::new(inner)),
            state: Arc::new(Mutex::new(BufferState::default())),
            debounce,
        }
    }
}

impl<S> RepositorySubscriber for BufferingRepositorySubscriber<S>
where
    S: RepositorySubscriber + Send + Sync + 'static,
{
    fn on_scan(
        &mut self,
        repository: &Repository,
        ctx: &mut ModelContext<Repository>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
        self.inner.lock().unwrap().on_scan(repository, ctx)
    }

    fn on_files_updated(
        &mut self,
        _repository: &Repository,
        update: &RepositoryUpdate,
        ctx: &mut ModelContext<Repository>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
        {
            let mut st = self.state.lock().unwrap();
            merge_repository_updates(&mut st.pending, update);
            st.version = st.version.wrapping_add(1);

            // Start a single background flusher if it's not already running.
            if st.flush_handle.is_none() {
                let inner = Arc::clone(&self.inner);
                let state = Arc::clone(&self.state);
                let wait = self.debounce;

                st.flush_handle = Some(ctx.spawn(
                    async move {
                        // Loop until we observe a quiet period (version stable for `wait`).
                        loop {
                            // Capture current version, then wait.
                            let start_version = {
                                let st = state.lock().unwrap();
                                st.version
                            };
                            warpui_core::r#async::Timer::after(wait).await;

                            // If version unchanged, we're quiet; flush pending and exit loop.
                            let maybe_merged = {
                                // Yield before flushing to check if the current flush is cancelled.
                                futures_lite::future::yield_now().await;

                                let mut st = state.lock().unwrap();
                                if st.version == start_version {
                                    st.flush_handle = None;
                                    Some(std::mem::take(&mut st.pending))
                                } else {
                                    // Newer update arrived during the wait; try waiting again.
                                    None
                                }
                            };

                            if let Some(merged) = maybe_merged {
                                break (inner, merged);
                            }
                        }
                    },
                    |repo_model, (inner, merged), repo_ctx| {
                        if merged.is_empty() {
                            return;
                        }
                        if let Ok(mut inner) = inner.lock() {
                            let fut = inner.on_files_updated(repo_model, &merged, repo_ctx);
                            // Drive the subscriber's async update to completion.
                            repo_ctx.spawn(fut, |_, _, _| {});
                        }
                    },
                ));
            }
        }

        Box::pin(ready(()))
    }

    fn on_unsubscribe(&mut self, _ctx: &mut ModelContext<Repository>) {
        let Ok(mut st) = self.state.lock() else {
            return;
        };
        if let Some(handle) = st.flush_handle.take() {
            handle.abort();
        }
    }
}

#[cfg(test)]
#[path = "repository_tests.rs"]
mod tests;
