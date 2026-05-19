use std::{
    collections::{hash_map::Entry, HashMap, HashSet, VecDeque},
    future::Future,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    pin::Pin,
};

#[cfg(feature = "local_fs")]
use futures::{future::OptionFuture, FutureExt as _};
use warpui::{Entity, ModelContext, ModelHandle, SingletonEntity, WeakModelHandle};

use warp_util::standardized_path::StandardizedPath;

use crate::{repository::SubscriberId, RepoMetadataError, Repository};

cfg_if::cfg_if! {
    if #[cfg(feature = "local_fs")] {
        use watcher::{BulkFilesystemWatcher, BulkFilesystemWatcherEvent};
        use crate::entry::{
            extract_worktree_git_dir, is_commit_related_git_file, is_git_internal_path,
            is_common_git_config, is_index_lock_file, is_remote_tracking_ref,
            is_shared_git_ref, is_tracking_state_git_file,
        };
        /// Duration between filesystem watch events in milliseconds
        const FILESYSTEM_WATCHER_DEBOUNCE_MILLI_SECS: u64 = 500;
    }
}

const MAX_CONCURRENT_TASKS: usize = 2;

/// A global singleton model that records and watches directory changes.
/// It is important to note that the directory here doesn't equal to a git repository. To
/// reference a whether a path is a git repository or not, check `DetectedRepositories`.
pub struct DirectoryWatcher {
    /// Map of known directories to watch.
    directories: HashMap<StandardizedPath, ModelHandle<Repository>>,

    /// The filesystem watcher for monitoring changes.
    #[cfg(feature = "local_fs")]
    watcher: Option<ModelHandle<BulkFilesystemWatcher>>,

    /// Handle to the internal processing queue model that orders scan & update tasks.
    processing_queue: ModelHandle<TaskQueue>,
}

impl DirectoryWatcher {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        cfg_if::cfg_if! {
            if #[cfg(feature = "local_fs")] {
                let fs_watcher = ctx.add_model(|ctx| {
                    BulkFilesystemWatcher::new(
                        std::time::Duration::from_millis(FILESYSTEM_WATCHER_DEBOUNCE_MILLI_SECS),
                        ctx,
                    )
                });
                ctx.subscribe_to_model(&fs_watcher, Self::handle_watcher_event);
            } else {
                // Silence an unused parameter warning.
                let _ = ctx;
            }
        }

        let processing_queue = ctx.add_model(TaskQueue::new);
        ctx.subscribe_to_model(&processing_queue, Self::handle_queue_event);

        Self {
            directories: Default::default(),
            #[cfg(feature = "local_fs")]
            watcher: Some(fs_watcher),
            processing_queue,
        }
    }

    /// Test-only constructor that uses a stub filesystem watcher with no background thread,
    /// preventing thread leaks in tests.
    #[cfg(any(test, feature = "test-util"))]
    pub fn new_for_testing(ctx: &mut ModelContext<Self>) -> Self {
        cfg_if::cfg_if! {
            if #[cfg(feature = "local_fs")] {
                let fs_watcher = ctx.add_model(|_ctx| BulkFilesystemWatcher::new_for_test());
                ctx.subscribe_to_model(&fs_watcher, Self::handle_watcher_event);
            } else {
                let _ = ctx;
            }
        }

        let processing_queue = ctx.add_model(TaskQueue::new);
        ctx.subscribe_to_model(&processing_queue, Self::handle_queue_event);

        Self {
            directories: Default::default(),
            #[cfg(feature = "local_fs")]
            watcher: Some(fs_watcher),
            processing_queue,
        }
    }

    /// Given a path, return the watched directory that contains it.
    pub fn get_watched_directory_for_path(&self, path: &Path) -> Option<ModelHandle<Repository>> {
        let standardized = StandardizedPath::from_local_canonicalized(path).ok()?;
        self.find_containing_directory(&standardized)
    }

    /// Find the watched directory that contains the given path, if any.
    fn find_containing_directory(
        &self,
        path: &StandardizedPath,
    ) -> Option<ModelHandle<Repository>> {
        let mut current = Some(path.clone());
        while let Some(ancestor) = current {
            if let Some(repo) = self.directories.get(&ancestor) {
                return Some(repo.clone());
            }
            current = ancestor.parent();
        }
        None
    }

    /// Check if a directory is registered for the given path.
    pub fn is_directory_watched(&self, path: &StandardizedPath) -> bool {
        self.directories.contains_key(path)
    }

    /// Find repositories affected by a git directory change using scope-aware
    /// scope-aware routing:
    ///
    /// 1. **Worktree-specific** (`.git/worktrees/<name>/…`): only the repo
    ///    whose `external_git_directory` matches the extracted worktree gitdir.
    /// 2. **Remote refs** (`.git/refs/remotes/*`): repos whose cached tracked
    ///    upstream ref resolves to the changed loose remote ref.
    /// 3. **Shared refs** (`.git/refs/heads/*`): all repos whose
    ///    `common_git_dir()` is a prefix of the event path (main repo +
    ///    all linked worktrees).
    /// 4. **Common config** (`.git/config`): all repos sharing that common Git directory.
    /// 5. **Repo-specific** (`.git/HEAD`, `.git/index.lock`, etc.): only the
    ///    repo whose working tree directly contains `.git` (main repo).
    #[cfg(feature = "local_fs")]
    fn find_repos_for_git_event(
        &self,
        git_path: &Path,
        ctx: &ModelContext<Self>,
    ) -> Vec<ModelHandle<Repository>> {
        let mut affected: Vec<ModelHandle<Repository>> = Vec::new();

        // Tier 1: route to the single linked worktree.
        if let Some(wt_dir) = extract_worktree_git_dir(git_path) {
            log::debug!(
                "[GIT_EVENT_ROUTING] tier=worktree-specific path={}",
                git_path.display()
            );
            let wt_std = StandardizedPath::from_local_canonicalized(wt_dir.as_path()).ok();
            for repo_handle in self.directories.values() {
                if let Some(ext) = repo_handle.as_ref(ctx).external_git_directory() {
                    if wt_std.as_ref() == Some(ext) && !affected.iter().any(|r| r == repo_handle) {
                        affected.push(repo_handle.clone());
                    }
                }
            }
        } else if is_remote_tracking_ref(git_path) {
            log::debug!(
                "[GIT_EVENT_ROUTING] tier=remote-ref path={}",
                git_path.display()
            );
            for repo_handle in self.directories.values() {
                if repo_handle.read(ctx, |repo, _| repo.tracks_remote_ref_path(git_path))
                    && !affected.iter().any(|r| r == repo_handle)
                {
                    affected.push(repo_handle.clone());
                }
            }
        } else if is_shared_git_ref(git_path) {
            // Tier 3: shared ref — broadcast to every repo whose
            // common_git_dir() is a prefix of the event path.
            log::debug!(
                "[GIT_EVENT_ROUTING] tier=shared-ref path={}",
                git_path.display()
            );
            let standardized = StandardizedPath::from_local_canonicalized(git_path).ok();
            if let Some(ref std_path) = standardized {
                if let Some(repo) = self.find_containing_directory(std_path) {
                    if !affected.iter().any(|r| r == &repo) {
                        affected.push(repo);
                    }
                }
                for repo_handle in self.directories.values() {
                    let common = repo_handle.read(ctx, |repo, _| repo.common_git_dir());
                    if let Ok(common_std) = StandardizedPath::try_from_local(common.as_path()) {
                        if std_path.starts_with(&common_std)
                            && !affected.iter().any(|r| r == repo_handle)
                        {
                            affected.push(repo_handle.clone());
                        }
                    }
                }
            }
        } else if is_common_git_config(git_path) {
            log::debug!(
                "[GIT_EVENT_ROUTING] tier=common-config path={}",
                git_path.display()
            );
            let Some(common_git_dir) = git_path.parent() else {
                return affected;
            };
            for repo_handle in self.directories.values() {
                let common = repo_handle.read(ctx, |repo, _| repo.common_git_dir());
                if common == common_git_dir && !affected.iter().any(|r| r == repo_handle) {
                    affected.push(repo_handle.clone());
                }
            }
        } else {
            // Tier 5: repo-specific (.git/HEAD, .git/index.lock) — only the
            // repo whose root_dir directly contains .git.
            log::debug!(
                "[GIT_EVENT_ROUTING] tier=repo-specific path={}",
                git_path.display()
            );
            let standardized = StandardizedPath::from_local_canonicalized(git_path).ok();
            if let Some(ref std_path) = standardized {
                if let Some(repo) = self.find_containing_directory(std_path) {
                    affected.push(repo);
                }
            }
        }

        let repo_roots: Vec<_> = affected
            .iter()
            .map(|r| r.as_ref(ctx).root_dir().to_string())
            .collect();
        log::debug!(
            "[GIT_EVENT_ROUTING] path={} affected_repos=[{}]",
            git_path.display(),
            repo_roots.join(", ")
        );

        affected
    }

    /// Register a known code directory. If the directory already exists, it will not be re-registered.
    pub fn add_directory(
        &mut self,
        repository_path: StandardizedPath,
        ctx: &mut ModelContext<Self>,
    ) -> Result<ModelHandle<Repository>, RepoMetadataError> {
        self.add_directory_with_git_dir(repository_path, None, ctx)
    }

    /// Register a known code directory with optional external git directory.
    /// If the directory already exists, it will not be re-registered.
    pub fn add_directory_with_git_dir(
        &mut self,
        repository_path: StandardizedPath,
        external_git_directory: Option<StandardizedPath>,
        ctx: &mut ModelContext<Self>,
    ) -> Result<ModelHandle<Repository>, RepoMetadataError> {
        let local_path = repository_path
            .to_local_path()
            .ok_or_else(|| RepoMetadataError::PathEncodingMismatch(repository_path.clone()))?;

        if !local_path.exists() {
            return Err(RepoMetadataError::RepoNotFound(repository_path.to_string()));
        }

        if !local_path.is_dir() {
            return Err(RepoMetadataError::InvalidPath(
                "Repository path must be a directory".to_string(),
            ));
        }

        // Check if there's an existing registration to reuse.
        let entry = self.directories.entry(repository_path);
        if let Entry::Occupied(ref entry) = entry {
            log::debug!("Using already-registered repository");
            return Ok(entry.get().clone());
        }

        // The repository is either not registered, or has expired.
        let queue_handle = self.processing_queue.clone();
        let repository_handle = ctx.add_model(|_ctx| {
            Repository::new(
                entry.key().clone(),
                external_git_directory.clone(),
                queue_handle,
            )
        });
        entry.insert_entry(repository_handle.clone());

        Ok(repository_handle)
    }

    /// Starts watching multiple directories for filesystem changes.
    ///
    /// The returned future resolves once all directories are registered.
    #[cfg(feature = "local_fs")]
    pub(crate) fn start_watching_directories(
        &mut self,
        directory_paths: Vec<StandardizedPath>,
        ctx: &mut ModelContext<Self>,
    ) -> impl Future<Output = Result<(), RepoMetadataError>> {
        let futures: Vec<_> = directory_paths
            .into_iter()
            .map(|path| self.start_watching_directory(&path, ctx))
            .collect();

        async move {
            for future in futures {
                future.await?;
            }
            Ok(())
        }
    }
    /// Starts watching a directory for filesystem changes.
    ///
    /// The returned future resolves once the directory is registered. Filesystem changes before
    /// this may not be observed.
    #[cfg(feature = "local_fs")]
    pub(crate) fn start_watching_directory(
        &mut self,
        directory_path: &StandardizedPath,
        ctx: &mut ModelContext<Self>,
    ) -> impl Future<Output = Result<(), RepoMetadataError>> {
        let local_path = directory_path.to_local_path();
        let registration_future = if let Some(ref watcher) = self.watcher {
            if let Some(local_path) = local_path.clone() {
                watcher.update(ctx, |watcher, _ctx| {
                    use crate::entry::should_ignore_git_path;
                    use notify_debouncer_full::notify::{RecursiveMode, WatchFilter};
                    use std::sync::Arc;

                    let watch_filter = WatchFilter::with_filter(Arc::new(move |watch_path| {
                        !should_ignore_git_path(watch_path)
                    }));

                    Some(watcher.register_path(&local_path, watch_filter, RecursiveMode::Recursive))
                })
            } else {
                log::warn!("Cannot watch non-local path: {directory_path}");
                None
            }
        } else {
            log::warn!("No watcher available");
            None
        };

        let path_display = directory_path.to_string();
        OptionFuture::from(registration_future).map(move |result| match result {
            Some(Ok(())) => {
                log::debug!("Started watching {path_display}");
                Ok(())
            }
            Some(Err(e)) => {
                log::debug!("Failed to start watching {path_display}: {e:#}");
                Err(e.into())
            }
            None => Ok(()),
        })
    }

    /// Stops watching a directory for filesystem changes.
    #[cfg(feature = "local_fs")]
    pub(crate) fn stop_watching_directory(
        &mut self,
        directory_path: &StandardizedPath,
        ctx: &mut ModelContext<Self>,
    ) -> impl Future<Output = Result<(), anyhow::Error>> {
        cfg_if::cfg_if! {
            if #[cfg(feature = "local_fs")] {
                let local_path = directory_path.to_local_path();
                let unregistration_future = if let Some(ref watcher) = self.watcher {
                    if let Some(local_path) = local_path {
                        watcher.update(ctx, |watcher, _ctx| {
                            Some(watcher.unregister_path(&local_path))
                        })
                    } else {
                        log::warn!("Cannot unwatch non-local path: {directory_path}");
                        None
                    }
                } else {
                    log::warn!("No watcher available");
                    None
                };

                let path_display = directory_path.to_string();
                OptionFuture::from(unregistration_future).map(move |result| match result {
                    Some(Ok(())) => {
                        log::debug!("Stopped watching {path_display}");
                        Ok(())
                    }
                    Some(Err(e)) => {
                        log::warn!("Failed to stop watching {path_display}: {e:#}");
                        Err(e)
                    }
                    None => Ok(()),
                })
            } else {
                async { Ok(()) }
            }
        }
    }

    /// Handles events from the internal task queue.
    fn handle_queue_event(&mut self, event: &TaskQueueEvent, ctx: &mut ModelContext<Self>) {
        let &TaskQueueEvent::TaskEnqueued = event;
        self.processing_queue.update(ctx, |queue, ctx| {
            queue.advance(ctx);
        });
    }

    #[cfg(feature = "local_fs")]
    fn find_existing_subpath(path: &PathBuf) -> Option<PathBuf> {
        // Attempt to find a subdirectory that exists in the filesystem.
        let mut current = path.to_owned();
        while !current.as_path().exists() {
            if !current.pop() {
                return None;
            }
        }
        Some(current)
    }

    #[cfg(feature = "local_fs")]
    fn record_git_internal_path_update(
        &self,
        path: &Path,
        repo_updates: &mut HashMap<ModelHandle<Repository>, RepositoryUpdate>,
        repos_to_refresh_tracked_remote_ref: &mut HashSet<ModelHandle<Repository>>,
        ctx: &ModelContext<Self>,
    ) {
        let affected = self.find_repos_for_git_event(path, ctx);
        let is_commit = is_commit_related_git_file(path);
        let is_lock = is_index_lock_file(path);
        let is_remote_ref = is_remote_tracking_ref(path);
        let is_tracking_state = is_tracking_state_git_file(path);

        for repo_handle in &affected {
            if is_commit || is_lock || is_remote_ref {
                let repo_update = repo_updates.entry(repo_handle.clone()).or_default();
                if is_commit {
                    repo_update.commit_updated = true;
                }
                if is_lock {
                    repo_update.index_lock_detected = true;
                }
                if is_remote_ref {
                    repo_update.remote_ref_updated = true;
                }
            }
            if is_tracking_state {
                repos_to_refresh_tracked_remote_ref.insert(repo_handle.clone());
            }
        }

        if !affected.is_empty() {
            log::debug!(
                "[GIT_EVENT_ROUTING] dispatched path={} commit_updated={is_commit} remote_ref_updated={is_remote_ref} index_lock={is_lock} tracking_state={is_tracking_state} to {} repo(s)",
                path.display(),
                affected.len()
            );
        }
    }

    /// Handles filesystem watcher events.
    #[cfg(feature = "local_fs")]
    fn handle_watcher_event(
        &mut self,
        event: &BulkFilesystemWatcherEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        // Group changes by repository
        let mut repo_updates: HashMap<ModelHandle<Repository>, RepositoryUpdate> = HashMap::new();
        let mut repos_to_refresh_tracked_remote_ref: HashSet<ModelHandle<Repository>> =
            HashSet::new();

        {
            let mut process_upsert_paths =
                |paths: &HashSet<PathBuf>,
                 insert: &mut dyn FnMut(&mut RepositoryUpdate, TargetFile)| {
                    for path in paths {
                        // Check if this is a .git/ internal event (e.g. HEAD, index, refs update).
                        if is_git_internal_path(path) {
                            self.record_git_internal_path_update(
                                path,
                                &mut repo_updates,
                                &mut repos_to_refresh_tracked_remote_ref,
                                ctx,
                            );
                            continue;
                        }

                        // For non-git files, use standard path lookup
                        if let Ok(standardized) =
                            StandardizedPath::from_local_canonicalized(path.as_path())
                        {
                            if let Some(repo_handle) = self.find_containing_directory(&standardized)
                            {
                                let is_ignored = repo_handle
                                    .read(ctx, |repo, _| repo.check_gitignore_status(path));
                                let target_file = TargetFile::new(path.to_path_buf(), is_ignored);
                                let repo_update = repo_updates.entry(repo_handle).or_default();
                                insert(repo_update, target_file);
                            }
                        }
                    }
                };

            // Process added files
            process_upsert_paths(&event.added, &mut |repo_update, target_file| {
                repo_update.added.insert(target_file);
            });

            // Process modified files
            process_upsert_paths(&event.modified, &mut |repo_update, target_file| {
                if !repo_update.added.contains(&target_file) {
                    repo_update.modified.insert(target_file);
                }
            });
        }

        // Process deleted files
        for path in &event.deleted {
            // Check if this is a .git/ internal event.
            if is_git_internal_path(path) {
                self.record_git_internal_path_update(
                    path,
                    &mut repo_updates,
                    &mut repos_to_refresh_tracked_remote_ref,
                    ctx,
                );
            } else {
                // Because this file will no longer exist, which will fail canonicalization.
                // We will just try the directory path instead, which hopefully still exists.
                if let Some(existing_subpath) = Self::find_existing_subpath(path) {
                    if let Ok(standardized) =
                        StandardizedPath::from_local_canonicalized(existing_subpath.as_path())
                    {
                        if let Some(repo_handle) = self.find_containing_directory(&standardized) {
                            // Gitignore checking is pattern-based and doesn't require file existence
                            let is_ignored =
                                repo_handle.read(ctx, |repo, _| repo.check_gitignore_status(path));
                            let target_file = TargetFile::new(path.to_path_buf(), is_ignored);
                            let repo_update = repo_updates.entry(repo_handle).or_default();
                            repo_update.deleted.insert(target_file);
                        }
                    }
                }
            }
        }

        // Process moved files
        for (to_path, from_path) in &event.moved {
            // Check if this is a .git/ internal event.
            if is_git_internal_path(to_path) || is_git_internal_path(from_path) {
                if is_git_internal_path(to_path) {
                    self.record_git_internal_path_update(
                        to_path,
                        &mut repo_updates,
                        &mut repos_to_refresh_tracked_remote_ref,
                        ctx,
                    );
                }
                if is_git_internal_path(from_path) {
                    self.record_git_internal_path_update(
                        from_path,
                        &mut repo_updates,
                        &mut repos_to_refresh_tracked_remote_ref,
                        ctx,
                    );
                }
            } else if let Ok(standardized) =
                StandardizedPath::from_local_canonicalized(to_path.as_path())
            {
                if let Some(repo_handle) = self.find_containing_directory(&standardized) {
                    let to_is_ignored =
                        repo_handle.read(ctx, |repo, _| repo.check_gitignore_status(to_path));
                    let from_is_ignored =
                        repo_handle.read(ctx, |repo, _| repo.check_gitignore_status(from_path));
                    let to_target = TargetFile::new(to_path.to_path_buf(), to_is_ignored);
                    let from_target = TargetFile::new(from_path.to_path_buf(), from_is_ignored);
                    let repo_update = repo_updates.entry(repo_handle).or_default();
                    repo_update.moved.insert(to_target, from_target);
                }
            }
        }

        self.processing_queue.update(ctx, |queue, ctx| {
            for (repo_handle, repo_update) in repo_updates {
                let subscriber_ids = repo_handle.read(ctx, |repo, _| repo.get_subscriber_ids());
                for subscriber_id in subscriber_ids {
                    queue.enqueue_incremental_update(
                        repo_handle.downgrade(),
                        subscriber_id,
                        repo_update.clone(),
                        ctx,
                    );
                }
            }
        });
        for repo_handle in repos_to_refresh_tracked_remote_ref {
            repo_handle.update(ctx, |repo, ctx| {
                repo.refresh_tracked_remote_ref(true, ctx);
            });
        }
    }
}

impl Entity for DirectoryWatcher {
    type Event = ();
}

impl SingletonEntity for DirectoryWatcher {}

/// Represents a file in a repository with its gitignore status.
#[derive(Debug, Clone)]
pub struct TargetFile {
    pub path: PathBuf,
    pub is_ignored: bool,
}

impl TargetFile {
    pub fn new(path: PathBuf, is_ignored: bool) -> Self {
        Self { path, is_ignored }
    }
}

impl Hash for TargetFile {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.path.hash(state);
        self.is_ignored.hash(state);
    }
}

impl PartialEq for TargetFile {
    fn eq(&self, other: &Self) -> bool {
        self.path == other.path && self.is_ignored == other.is_ignored
    }
}

impl Eq for TargetFile {}

impl PartialOrd for TargetFile {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for TargetFile {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.path
            .cmp(&other.path)
            .then_with(|| self.is_ignored.cmp(&other.is_ignored))
    }
}

/// Changes detected in a repository.
#[derive(Debug, Clone, Default)]
pub struct RepositoryUpdate {
    /// Files that were added.
    pub added: HashSet<TargetFile>,

    /// Files whose contents were modified.
    pub modified: HashSet<TargetFile>,

    /// Files that were deleted.
    pub deleted: HashSet<TargetFile>,

    /// Files that were moved (to_path, from_path).
    pub moved: HashMap<TargetFile, TargetFile>,

    /// Whether a commit-related file changed (`.git/HEAD` or `.git/refs/heads/*`).
    pub commit_updated: bool,

    /// Whether the git index lock file was created or removed (`.git/index.lock`).
    pub index_lock_detected: bool,

    /// Whether the tracked upstream ref changed or the current tracked remote ref was updated.
    pub remote_ref_updated: bool,
}

impl RepositoryUpdate {
    /// Returns true if this update contains no changes.
    pub fn is_empty(&self) -> bool {
        self.added.is_empty()
            && self.modified.is_empty()
            && self.deleted.is_empty()
            && self.moved.is_empty()
            && !self.commit_updated
            && !self.index_lock_detected
            && !self.remote_ref_updated
    }

    /// Iterator over all created and modified files.
    ///
    /// Most consumers don't care about the added-vs-modified distinction.
    pub fn added_or_modified(&self) -> impl Iterator<Item = &TargetFile> {
        self.added.iter().chain(self.modified.iter())
    }

    /// Owned iterator over all created and modified files.
    pub fn into_added_or_modified(self) -> impl Iterator<Item = TargetFile> {
        self.added.into_iter().chain(self.modified)
    }

    pub fn contains_added_or_modified(&self, file: &TargetFile) -> bool {
        self.added.contains(file) || self.modified.contains(file)
    }
}

/// An asynchronous task in a watched repository.
#[derive(Clone)]
enum Task {
    /// Perform an initial (or re-)scan for the given subscriber on the repository.
    Scan {
        repository: WeakModelHandle<Repository>,
        subscriber_id: SubscriberId,
    },
    #[cfg(feature = "local_fs")]
    /// Deliver an incremental update (filesystem changes) to a specific repository subscriber.
    Update {
        repository: WeakModelHandle<Repository>,
        subscriber_id: SubscriberId,
        update: RepositoryUpdate,
    },
}

impl Task {
    fn execute(
        self,
        ctx: &mut ModelContext<TaskQueue>,
    ) -> Option<Pin<Box<dyn Future<Output = ()> + Send>>> {
        match self {
            Task::Scan {
                repository,
                subscriber_id,
            } => {
                if let Some(repository) = repository.upgrade(ctx) {
                    repository.update(ctx, |repository, ctx| {
                        repository.scan_subscriber(subscriber_id, ctx)
                    })
                } else {
                    None
                }
            }
            #[cfg(feature = "local_fs")]
            Task::Update {
                repository,
                subscriber_id,
                update,
            } => {
                if let Some(repository) = repository.upgrade(ctx) {
                    repository.update(ctx, |repository, ctx| {
                        repository.notify_subscriber(subscriber_id, &update, ctx)
                    })
                } else {
                    None
                }
            }
        }
    }
}

#[derive(Clone)]
pub enum TaskQueueEvent {
    TaskEnqueued,
}

/// Lightweight task queue model for watched repositories. The [`RepositoryWatcher`] model uses this
/// to limit throughput for CPU- or disk-intensive update operations.
#[derive(Default)]
pub(crate) struct TaskQueue {
    /// Tasks which have not yet been executed.
    pending_tasks: VecDeque<Task>,
    active_tasks: usize,
}

impl TaskQueue {
    fn new(_ctx: &mut ModelContext<Self>) -> Self {
        Self::default()
    }

    /// Enqueue a new task.
    fn enqueue(&mut self, task: Task, ctx: &mut ModelContext<Self>) {
        self.pending_tasks.push_back(task);

        // Notify the watcher that a new task has been enqueued. This prevents circular model
        // updates by ensuring new tasks aren't immediately dequeued.
        ctx.emit(TaskQueueEvent::TaskEnqueued);
    }

    /// Advance through the queue by executing new tasks, up to the concurrency limit.
    fn advance(&mut self, ctx: &mut ModelContext<Self>) {
        while self.active_tasks < MAX_CONCURRENT_TASKS {
            let Some(task) = self.pending_tasks.pop_front() else {
                break;
            };

            if let Some(future) = task.execute(ctx) {
                self.active_tasks += 1;
                ctx.spawn(future, move |me, _, ctx| {
                    me.handle_task_completion(ctx);
                });
            }
        }
    }

    fn handle_task_completion(&mut self, ctx: &mut ModelContext<Self>) {
        self.active_tasks -= 1;
        self.advance(ctx);
    }

    /// Convenience helpers for enqueuing specific task kinds.
    pub(crate) fn enqueue_scan(
        &mut self,
        repository: WeakModelHandle<Repository>,
        subscriber_id: SubscriberId,
        ctx: &mut ModelContext<Self>,
    ) {
        self.enqueue(
            Task::Scan {
                repository,
                subscriber_id,
            },
            ctx,
        );
    }

    #[cfg(feature = "local_fs")]
    pub(crate) fn enqueue_incremental_update(
        &mut self,
        repository: WeakModelHandle<Repository>,
        subscriber_id: SubscriberId,
        update: RepositoryUpdate,
        ctx: &mut ModelContext<Self>,
    ) {
        self.enqueue(
            Task::Update {
                repository,
                subscriber_id,
                update,
            },
            ctx,
        );
    }
}

impl Entity for TaskQueue {
    type Event = TaskQueueEvent;
}

#[cfg(test)]
#[path = "watcher_tests.rs"]
mod tests;
