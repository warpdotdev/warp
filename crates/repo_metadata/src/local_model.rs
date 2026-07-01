#![cfg_attr(not(feature = "local_fs"), allow(dead_code))]
//! Repository metadata model singleton.
//!
//! This module provides a singleton model that manages repository metadata across
//! all repositories tracked by Warp.

use std::collections::HashMap;
#[cfg(feature = "local_fs")]
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures::future::{self, BoxFuture, FutureExt as _};
use warp_core::{safe_warn, send_telemetry_from_ctx};
use warp_util::sync::Condition;
use warpui_core::ModelHandle;

/// Represents either a file or directory in a repository.
#[derive(Debug, Clone)]
pub enum RepoContent<'a> {
    File(&'a FileTreeFileMetadata),
    Directory(&'a FileTreeDirectoryEntryState),
}

/// The result of [`LocalRepoMetadataModel::get_repo_contents`].
///
/// The number of returned entries is capped at [`MAX_REPO_CONTENTS_RESULTS`].
/// When the repository contains more matching entries than that cap, the
/// returned `contents` are a partial prefix and `truncated` is `true`.
#[derive(Debug, Default)]
pub struct RepoContents<'a> {
    /// The collected repository contents, capped at [`MAX_REPO_CONTENTS_RESULTS`].
    pub contents: Vec<RepoContent<'a>>,
    /// `true` if traversal stopped early because the maximum result size was
    /// reached, meaning more matching entries exist than were returned.
    pub truncated: bool,
}

use warp_util::standardized_path::StandardizedPath;

use crate::entry::{
    BudgetExceededBehavior, BuildTreeError, BuildTreeOptions, Entry, FileId, IgnoredPathStrategy,
};
use crate::repository::Repository;
use crate::standing_queries::{
    StandingQueryDefinitions, StandingQueryResults, StandingQueryResultsDelta,
};
use crate::telemetry::RepoMetadataTelemetryEvent;
use crate::{gitignores_for_directory, matches_gitignores, RepoMetadataError};
cfg_if::cfg_if! {
    if #[cfg(feature = "local_fs")] {
        use notify_debouncer_full::notify::RecursiveMode;
        use crate::entry::repo_watch_filter;
        use crate::repositories::{DetectedRepositories, DetectedRepositoriesEvent};
        use crate::watcher::DirectoryWatcher;
        use watcher::{BulkFilesystemWatcher, BulkFilesystemWatcherEvent};
        use warpui_core::SingletonEntity as _;

        /// Duration between filesystem watch events in seconds
        const FILESYSTEM_WATCHER_DEBOUNCE_SECS: u64 = 1;
    }
}

use ignore::gitignore::Gitignore;
use warpui_core::ModelContext;

use crate::file_tree_store::{
    FileTreeDirectoryEntryState, FileTreeEntry, FileTreeEntryState, FileTreeFileMetadata,
    FileTreeState,
};
use crate::file_tree_update::{
    flatten_entry_metadata, DirectoryNodeMetadata, FileNodeMetadata, FileTreeEntryUpdate,
    MetadataUpdateType, RepoMetadataUpdate, RepoNodeMetadata,
};

/// Maximum depth to traverse when building file trees
const MAX_TREE_DEPTH: usize = 200;

/// Maximum number of non-ignored files to index eagerly per repository.
///
/// This is a high safety ceiling, not the common case: gitignored directories
/// are lazy placeholders and never consume this budget, so only repositories
/// with an enormous number of *tracked* files reach it. When the budget is
/// exhausted the builder stops descending breadth-first and leaves the
/// remaining directories as unloaded placeholders (lazy-loaded on demand)
/// rather than failing or collapsing the tree to a single level.
const MAX_FILES_PER_REPO: usize = 200_000;

/// Maximum number of results to return from get_repo_contents to prevent accidentally
/// materializing the entire repository
const MAX_REPO_CONTENTS_RESULTS: usize = 100;

#[derive(Debug)]
/// Events emitted by the LocalRepoMetadataModel.
pub enum RepositoryMetadataEvent {
    /// A repository was added or updated.
    RepositoryUpdated {
        path: StandardizedPath,
    },
    /// A repository was removed.
    RepositoryRemoved {
        path: StandardizedPath,
    },
    /// The file tree for the repositories were updated.
    FileTreeUpdated {
        paths: Vec<StandardizedPath>,
    },
    /// The file tree's [`Entry`] was updated.
    FileTreeEntryUpdated {
        path: StandardizedPath,
        /// Specifies whether this event contains a precise delta or requires a conservative
        /// refresh because the entry was replaced without one.
        update_type: MetadataUpdateType,
    },
    /// The paths retained for standing queries changed.
    StandingQueryResultsUpdated {
        path: StandardizedPath,
        delta: StandingQueryResultsDelta,
    },
    UpdatingRepositoryFailed {
        path: StandardizedPath,
    },
    /// Emitted after watcher mutations are applied when
    /// `emit_incremental_updates` is enabled, containing a serializable
    /// update suitable for sending to the remote client.
    IncrementalUpdateReady {
        update: RepoMetadataUpdate,
    },
}

/// Represents the state of a repository in the metadata model.
#[derive(Debug)]
pub enum IndexedRepoState {
    /// Repository is currently being indexed.
    Pending(Condition),
    /// Repository has been successfully indexed.
    Indexed(FileTreeState),

    /// Repository indexing failed with the given error.
    Failed(RepoMetadataError),
}

impl IndexedRepoState {
    pub fn pending() -> Self {
        Self::Pending(Condition::new())
    }

    pub fn wait_until_indexed(&self) -> BoxFuture<'static, ()> {
        match self {
            Self::Indexed(_) | Self::Failed(_) => future::ready(()).boxed(),
            Self::Pending(condition) => {
                let condition = condition.clone();
                async move {
                    condition.wait().await;
                }
                .boxed()
            }
        }
    }
}

impl IndexedRepoState {
    pub(crate) fn complete_if_pending(&self) {
        if let Self::Pending(condition) = self {
            condition.set();
        }
    }
}

/// How a repository's ROOT directory is registered with the filesystem watcher.
///
/// This is orthogonal to the set of on-demand per-directory watches a repo may
/// also hold (see [`RepoWatch::extra_dirs`]): a recursive root can now carry
/// extra non-recursive watches too.
//
// Without `local_fs` there is no watcher, so `NonRecursive` is never
// constructed; the variant still exists to keep the watch-mode model intact.
#[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RootWatchMode {
    /// A single recursive watch on the root covers the whole subtree. Used for
    /// git repos (which rely on gitignore pruning in the watch descend filter)
    /// and, on macOS/Windows, for lazy non-git roots where a recursive OS watch
    /// is cheap.
    Recursive,
    /// The root is watched non-recursively; each loaded directory gets its own
    /// non-recursive watch, so the number of watches scales with what the user
    /// expands rather than the whole subtree. Used for lazy (non-git) roots on
    /// Linux, where per-directory inotify watches are otherwise prohibitively
    /// expensive.
    NonRecursive,
}

/// Tracks how a repository is registered with the filesystem watcher.
///
/// `root_mode` describes how the ROOT directory is registered. `extra_dirs`
/// holds the on-demand NON-recursive watches we register on individual
/// subdirectories that the root watch does not cover: every loaded subdir under
/// a non-recursive root, or expanded gitignored dirs under a recursive root on
/// Linux. The root itself is never stored here — it is unregistered directly by
/// its repo path on teardown. In practice `extra_dirs` is only populated on
/// Linux, since other backends deliver gitignored events through the recursive
/// root watch.
#[cfg(feature = "local_fs")]
#[derive(Debug)]
struct RepoWatch {
    root_mode: RootWatchMode,
    extra_dirs: HashSet<StandardizedPath>,
}

/// Singleton model for managing local repository metadata.
///
/// This model tracks repositories on the local filesystem, using file watchers
/// to stay up to date and subscribing to `DetectedRepositories` for auto-indexing.
///
/// Consumers should access this through the [`RepoMetadataModel`](crate::wrapper_model::RepoMetadataModel)
/// wrapper rather than using this type directly.
pub struct LocalRepoMetadataModel {
    /// Mapping from repository path to its indexed state.
    repositories: HashMap<StandardizedPath, IndexedRepoState>,
    /// Stored context-discovery matches, independent from canonical tree materialization.
    standing_results: HashMap<StandardizedPath, StandingQueryResults>,
    /// Refcounts for lazily-loaded standalone paths tracked in the model.
    lazy_loaded_paths: HashMap<StandardizedPath, usize>,
    /// File system watcher for monitoring changes.
    #[cfg(feature = "local_fs")]
    watcher: Option<ModelHandle<BulkFilesystemWatcher>>,
    /// When true, emit [`RepositoryMetadataEvent::IncrementalUpdateReady`]
    /// events after applying watcher mutations. Only the remote server
    /// variant enables this.
    emit_incremental_updates: bool,
    /// Paths that must be loaded even when gitignored or beyond the tree's size
    /// limit. For example, a consumer can register `.foo/bar` so ignored
    /// `.foo`, `.foo/bar`, and descendants of `.foo/bar` are loaded into the
    /// tree.
    force_included_paths: Vec<PathBuf>,
    /// Configured standing-query matchers.
    standing_query_definitions: StandingQueryDefinitions,
    /// Resolved symlink target directories for direct children of ignored-path interests.
    ///
    /// One resolved target may be referenced by multiple lexical symlinks, so retain every
    /// alias rather than using a one-to-one bidirectional map.
    #[cfg(feature = "local_fs")]
    symlink_targets: HashMap<PathBuf, HashSet<SymlinkTarget>>,
    /// How each tracked repository is registered with the filesystem watcher,
    /// including any on-demand per-directory watches recorded for teardown. See
    /// [`RepoWatch`].
    #[cfg(feature = "local_fs")]
    repo_watches: HashMap<StandardizedPath, RepoWatch>,
}

#[derive(Debug, Clone, Default)]
struct RepoUpdate {
    added: Vec<PathBuf>,
    deleted: Vec<PathBuf>,
    moved: HashMap<PathBuf, PathBuf>,
}
#[cfg(feature = "local_fs")]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SymlinkTarget {
    repo_path: StandardizedPath,
    current_path: PathBuf,
}

/// Describes a single file-tree mutation computed on a background thread.
/// These are produced by `compute_file_tree_mutations` (filesystem I/O) and
/// consumed by `apply_file_tree_mutations` (tree-only, main thread).
#[derive(Debug)]
pub(crate) enum FileTreeMutation {
    /// Remove a path from the tree.
    Remove(PathBuf),
    /// Add a single file with pre-computed metadata.
    AddFile {
        path: PathBuf,
        is_ignored: bool,
        extension: Option<String>,
    },
    /// Add a directory with its fully-built subtree.
    AddDirectorySubtree { dir_path: PathBuf, subtree: Entry },
    /// Add a bare (unloaded) directory placeholder, materialized on demand when
    /// the user expands it. Used for newly created directories under lazy roots
    /// and as a fallback when `build_tree` fails.
    AddUnloadedDirectory { path: PathBuf, is_ignored: bool },
}

/// A filter function for filtering repo contents during traversal.
type RepoContentFilter = dyn for<'a> Fn(&RepoContent<'a>) -> bool + Send + Sync;

pub struct GetContentsArgs {
    pub include_folders: bool,
    pub include_ignored: bool,
    /// Optional filter applied during traversal to skip entries early.
    /// Return `true` to include the entry, `false` to skip it.
    pub filter: Option<Arc<RepoContentFilter>>,
}

impl Default for GetContentsArgs {
    fn default() -> Self {
        Self {
            include_folders: true,
            include_ignored: false,
            filter: None,
        }
    }
}

impl GetContentsArgs {
    pub fn include_ignored(mut self) -> Self {
        self.include_ignored = true;
        self
    }

    pub fn exclude_folders(mut self) -> Self {
        self.include_folders = false;
        self
    }

    /// Sets a filter closure to be applied during traversal.
    /// Only entries for which the filter returns `true` will be included.
    pub fn with_filter<F>(self, filter: F) -> Self
    where
        F: for<'a> Fn(&RepoContent<'a>) -> bool + Send + Sync + 'static,
    {
        Self {
            include_folders: self.include_folders,
            include_ignored: self.include_ignored,
            filter: Some(Arc::new(filter)),
        }
    }
}

impl LocalRepoMetadataModel {
    /// Creates a new LocalRepoMetadataModel.
    #[cfg_attr(not(feature = "local_fs"), allow(unused_variables), allow(unused_mut))]
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        let mut model = Self {
            repositories: HashMap::new(),
            standing_results: HashMap::new(),
            lazy_loaded_paths: HashMap::new(),
            #[cfg(feature = "local_fs")]
            watcher: None,
            emit_incremental_updates: false,
            force_included_paths: Vec::new(),
            standing_query_definitions: StandingQueryDefinitions::default(),
            #[cfg(feature = "local_fs")]
            symlink_targets: HashMap::new(),
            #[cfg(feature = "local_fs")]
            repo_watches: HashMap::new(),
        };
        cfg_if::cfg_if! {
            if #[cfg(feature = "local_fs")] {
                let watcher = ctx.add_model(|ctx| {
                    BulkFilesystemWatcher::new(
                        std::time::Duration::from_secs(FILESYSTEM_WATCHER_DEBOUNCE_SECS),
                        ctx,
                    )
                });
                ctx.subscribe_to_model(&watcher, |me, _, event, ctx| me.handle_watcher_event(event, ctx));
                model.watcher = Some(watcher);

                ctx.subscribe_to_model(&DetectedRepositories::handle(ctx), |me, _, event, ctx| {
                    let DetectedRepositoriesEvent::DetectedGitRepo { repository, .. } = event;
                    let repo_path = repository.as_ref(ctx).root_dir().clone();
                    if let Err(e) = me.index_directory(repository.clone(), ctx) {
                        log::warn!(
                            "Failed to index directory {repo_path}: {e}"
                        );
                    }
                });
            }
        }

        model
    }

    /// Enables or disables emission of
    /// [`RepositoryMetadataEvent::IncrementalUpdateReady`] events after
    /// applying watcher mutations. Only the remote server variant should
    /// enable this.
    pub fn set_emit_incremental_updates(&mut self, enabled: bool) {
        self.emit_incremental_updates = enabled;
    }

    /// Registers paths that must be loaded even when gitignored or beyond the
    /// tree's size limit.
    ///
    /// This stays intentionally generic: consumers own the meaning of the paths,
    /// while repo metadata only uses them to decide which ignored subtrees should
    /// be represented eagerly instead of as lazy placeholders.
    pub fn register_force_included_paths(&mut self, paths: impl IntoIterator<Item = PathBuf>) {
        for path in paths {
            assert!(
                !path.is_absolute(),
                "force-included paths must be repository-relative"
            );
            if !self
                .force_included_paths
                .iter()
                .any(|existing| existing == &path)
            {
                self.force_included_paths.push(path);
            }
        }
    }

    pub fn set_project_skill_provider_paths(&mut self, paths: impl IntoIterator<Item = PathBuf>) {
        self.standing_query_definitions
            .set_project_skill_provider_paths(paths);
    }

    /// Adds synthetic lexical repository updates for changes beneath resolved symlink targets.
    ///
    /// Directory symlinks are intentionally absent from the canonical tree, so target changes
    /// must be replayed through their lexical paths to refresh standing-query results.
    #[cfg(feature = "local_fs")]
    fn add_symlink_target_updates(
        &self,
        event: &BulkFilesystemWatcherEvent,
        repo_updates: &mut HashMap<StandardizedPath, RepoUpdate>,
    ) -> HashSet<PathBuf> {
        let mut matched_paths = HashSet::new();
        let mut append_updates = |path: &Path| {
            for (original_path, targets) in &self.symlink_targets {
                if path != original_path && !path.starts_with(original_path) {
                    continue;
                }
                matched_paths.insert(path.to_path_buf());
                for target in targets {
                    let update = repo_updates.entry(target.repo_path.clone()).or_default();
                    if !update.deleted.contains(&target.current_path) {
                        update.deleted.push(target.current_path.clone());
                    }
                    if target.current_path.is_dir() && !update.added.contains(&target.current_path)
                    {
                        update.added.push(target.current_path.clone());
                    }
                }
            }
        };

        for path in event.added_or_updated_iter() {
            append_updates(path);
        }
        for path in &event.deleted {
            append_updates(path);
        }
        for (to_path, from_path) in &event.moved {
            append_updates(from_path);
            append_updates(to_path);
        }
        matched_paths
    }

    /// Synchronizes producer-side target watches for symlinked children of ignored-path interests.
    #[cfg(feature = "local_fs")]
    fn refresh_symlink_targets(
        &mut self,
        repo_path: &StandardizedPath,
        ctx: &mut ModelContext<Self>,
    ) {
        if !self.emit_incremental_updates {
            return;
        }
        let previously_watched = self.symlink_targets.keys().cloned().collect::<HashSet<_>>();
        self.remove_symlink_targets_for_repo(repo_path);
        let Some(local_repo_path) = repo_path.to_local_path() else {
            return;
        };
        for interest in &self.force_included_paths {
            let Ok(entries) = std::fs::read_dir(local_repo_path.join(interest)) else {
                continue;
            };
            for entry in entries.flatten() {
                let current_path = entry.path();
                if !current_path.is_symlink() || !current_path.is_dir() {
                    continue;
                }
                let Ok(original_path) = dunce::canonicalize(&current_path) else {
                    continue;
                };
                self.symlink_targets
                    .entry(original_path)
                    .or_default()
                    .insert(SymlinkTarget {
                        repo_path: repo_path.clone(),
                        current_path,
                    });
            }
        }
        self.sync_symlink_target_watches(previously_watched, ctx);
    }

    #[cfg(feature = "local_fs")]
    fn remove_symlink_targets_for_repo(&mut self, repo_path: &StandardizedPath) {
        for targets in self.symlink_targets.values_mut() {
            targets.retain(|target| &target.repo_path != repo_path);
        }
        self.symlink_targets
            .retain(|_, targets| !targets.is_empty());
    }

    #[cfg(feature = "local_fs")]
    fn clear_symlink_targets_for_repo(
        &mut self,
        repo_path: &StandardizedPath,
        ctx: &mut ModelContext<Self>,
    ) {
        let previously_watched = self.symlink_targets.keys().cloned().collect();
        self.remove_symlink_targets_for_repo(repo_path);
        self.sync_symlink_target_watches(previously_watched, ctx);
    }

    #[cfg(feature = "local_fs")]
    fn sync_symlink_target_watches(
        &self,
        previously_watched: HashSet<PathBuf>,
        ctx: &mut ModelContext<Self>,
    ) {
        let desired_target_dirs = self.symlink_targets.keys().cloned().collect::<HashSet<_>>();
        if let Some(ref watcher) = self.watcher {
            for target_dir in desired_target_dirs.difference(&previously_watched) {
                watcher.update(ctx, |watcher, _ctx| {
                    std::mem::drop(watcher.register_path(
                        target_dir,
                        repo_watch_filter(Vec::new(), Vec::new()),
                        RecursiveMode::NonRecursive,
                    ));
                });
            }
            for target_dir in previously_watched.difference(&desired_target_dirs) {
                watcher.update(ctx, |watcher, _ctx| {
                    std::mem::drop(watcher.unregister_path(target_dir));
                });
            }
        }
    }

    /// Handles events from the BulkFilesystemWatcher.
    #[cfg(feature = "local_fs")]
    fn handle_watcher_event(
        &mut self,
        event: &BulkFilesystemWatcherEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        // Create a map to collect changes per repository
        let mut repo_updates: HashMap<StandardizedPath, RepoUpdate> = HashMap::new();
        let symlink_target_paths = self.add_symlink_target_updates(event, &mut repo_updates);

        // Process added or updated files
        for path in event.added_or_updated_iter() {
            if let Some(repo_path) = self.find_repository_for_watcher_entry_path(path) {
                let repo_update = repo_updates.entry(repo_path).or_default();
                repo_update.added.push(path.to_path_buf());
            }
        }

        // Process deleted files
        for path in &event.deleted {
            if let Some(repo_path) =
                self.find_repository_for_path_string(path.to_string_lossy().as_ref())
            {
                let repo_update = repo_updates.entry(repo_path).or_default();
                repo_update.deleted.push(path.to_path_buf());
            } else if !symlink_target_paths.contains(path) {
                log::warn!("Deleted file not found in any repo: {path:?} not found in any repo");
            }
        }

        // Process moved files
        for (to_path, from_path) in &event.moved {
            if let Some(repo_path) = self.find_repository_for_watcher_entry_path(to_path) {
                let repo_update = repo_updates.entry(repo_path).or_default();
                repo_update
                    .moved
                    .insert(to_path.to_path_buf(), from_path.to_path_buf());
            }
        }

        // Collect all paths that have been updated and emit an event.
        ctx.emit(RepositoryMetadataEvent::FileTreeUpdated {
            paths: repo_updates.keys().cloned().collect(),
        });
        // Apply updates to each affected repository asynchronously.
        // Phase 1 (background thread): compute lightweight mutations via filesystem I/O.
        // Phase 2 (main thread callback): apply mutations directly to the tree — no clone needed.
        for (repo_path, repo_scoped_update) in repo_updates {
            if let Some(IndexedRepoState::Indexed(state)) = self.repositories.get_mut(&repo_path) {
                let repo_path_clone = repo_path.clone();
                // `gitignores` is an `Arc<Vec<Gitignore>>`, so this is a cheap
                // refcount bump rather than a deep copy of the compiled regexes,
                // even though a watcher event can fire very frequently.
                let gitignores_clone = state.gitignores.clone();
                let force_included_paths = self.force_included_paths.clone();
                let standing_query_definitions = self.standing_query_definitions.clone();
                let lazy_load = self.lazy_loaded_paths.contains_key(&repo_path);
                ctx.spawn(
                    async move {
                        let (mutations, standing_results, removed_roots) =
                            Self::compute_file_tree_mutations(
                                &repo_scoped_update,
                                &gitignores_clone,
                                &force_included_paths,
                                &standing_query_definitions,
                                lazy_load,
                            )
                            .await;
                        (
                            mutations,
                            standing_results,
                            removed_roots,
                            repo_path_clone,
                            lazy_load,
                        )
                    },
                    |model,
                     (mutations, discovered_results, removed_roots, repo_path, lazy_load),
                     ctx| {
                        if let Some(IndexedRepoState::Indexed(state)) =
                            model.repositories.get_mut(&repo_path)
                        {
                            let mut update = Self::apply_file_tree_mutations(
                                &mut state.entry,
                                mutations,
                                lazy_load,
                                true,
                            )
                            .expect("update tracking was enabled");
                            let standing_delta = model
                                .standing_results
                                .entry(repo_path.clone())
                                .or_default()
                                .replace_subtrees(&removed_roots, discovered_results);
                            model.refresh_symlink_targets(&repo_path, ctx);
                            update.standing_results_delta = standing_delta.clone();
                            ctx.emit(RepositoryMetadataEvent::FileTreeEntryUpdated {
                                path: repo_path.clone(),
                                update_type: MetadataUpdateType::IncrementalUpdate(update.clone()),
                            });
                            if !standing_delta.is_empty() {
                                ctx.emit(RepositoryMetadataEvent::StandingQueryResultsUpdated {
                                    path: repo_path.clone(),
                                    delta: standing_delta,
                                });
                            }
                            if model.emit_incremental_updates {
                                ctx.emit(RepositoryMetadataEvent::IncrementalUpdateReady {
                                    update,
                                });
                            }
                        }

                        // Drop per-directory watches for any directory that was
                        // deleted or moved away (along with their tracked
                        // descendants). Without this their stale `extra_dirs`
                        // entries would make `watch_subdir` skip re-watching if a
                        // directory is later recreated at the same path.
                        for removed in &removed_roots {
                            model.unwatch_removed_subtree(&repo_path, removed, ctx);
                        }
                    },
                );
            }
        }
    }

    #[cfg(feature = "local_fs")]
    fn find_repository_for_path_string(&self, path_str: &str) -> Option<StandardizedPath> {
        self.repositories
            .iter()
            .filter(|(repo_path, state)| {
                let repo_path_str = repo_path.as_str();
                path_str.starts_with(repo_path_str) && matches!(state, IndexedRepoState::Indexed(_))
            })
            .max_by_key(|(repo_path, _)| repo_path.as_str().len())
            .map(|(repo_path, _)| repo_path.clone())
    }

    #[cfg(feature = "local_fs")]
    fn find_repository_for_standardized_path(
        &self,
        path: &StandardizedPath,
    ) -> Option<StandardizedPath> {
        self.repositories
            .iter()
            .filter(|(repo_path, state)| {
                path.starts_with(repo_path) && matches!(state, IndexedRepoState::Indexed(_))
            })
            .max_by_key(|(repo_path, _)| repo_path.as_str().len())
            .map(|(repo_path, _)| repo_path.clone())
    }

    #[cfg(feature = "local_fs")]
    fn find_repository_for_watcher_entry_path(&self, path: &Path) -> Option<StandardizedPath> {
        StandardizedPath::try_from_local(path)
            .ok()
            .and_then(|path| self.find_repository_for_standardized_path(&path))
            .or_else(|| self.find_repository_for_path(path))
    }

    #[cfg(feature = "local_fs")]
    pub fn find_repository_for_path(&self, path: &Path) -> Option<StandardizedPath> {
        match StandardizedPath::from_local_canonicalized(path) {
            Ok(std_path) => self.find_repository_for_standardized_path(&std_path),
            Err(_) => None,
        }
    }

    /// Adds or updates a repository's file tree state.
    ///
    /// `root_mode` controls how the root is registered with the filesystem
    /// watcher: [`RootWatchMode::Recursive`] registers a single recursive watch
    /// (git repos, and lazy roots off Linux), while
    /// [`RootWatchMode::NonRecursive`] registers the root non-recursively so
    /// expanded subdirectories can be watched individually.
    #[cfg_attr(not(feature = "local_fs"), allow(unused_variables))]
    fn add_repository_internal(
        &mut self,
        repo_path: StandardizedPath,
        state: FileTreeState,
        root_mode: RootWatchMode,
        ctx: &mut ModelContext<Self>,
    ) -> Result<(), RepoMetadataError> {
        let local_path = repo_path
            .to_local_path()
            .ok_or_else(|| RepoMetadataError::PathEncodingMismatch(repo_path.clone()))?;

        // Validate the repository path exists
        if !local_path.exists() {
            return Err(RepoMetadataError::RepoNotFound(repo_path.to_string()));
        }

        if !local_path.is_dir() {
            return Err(RepoMetadataError::InvalidPath(
                "Repository path must be a directory".to_string(),
            ));
        }

        // Record how this root is watched and register it with the watcher (if
        // any).
        #[cfg(feature = "local_fs")]
        {
            let recursive_mode = match root_mode {
                RootWatchMode::Recursive => RecursiveMode::Recursive,
                RootWatchMode::NonRecursive => RecursiveMode::NonRecursive,
            };
            // Replace any prior registration, dropping its root watch and stale
            // per-directory watches before re-registering (e.g. a lazy
            // non-recursive root upgraded to a recursive git repo, whose old
            // per-dir watches would otherwise duplicate the recursive coverage).
            let previous = self.repo_watches.insert(
                repo_path.clone(),
                RepoWatch {
                    root_mode,
                    extra_dirs: HashSet::new(),
                },
            );
            if let Some(ref watcher) = self.watcher {
                let watch_path = local_path.clone();
                // Build the gitignore set (root + global) and force-included
                // path list so the descend filter prunes gitignored subtrees
                // while still watching registered force-included paths (e.g.
                // skills).
                let gitignores = crate::gitignores_for_directory(&watch_path);
                let force_included_paths = self.force_included_paths.clone();
                let had_previous = previous.is_some();
                let previous_extra: Vec<PathBuf> = previous
                    .map(|prev| {
                        prev.extra_dirs
                            .iter()
                            .filter_map(|dir| dir.to_local_path())
                            .collect()
                    })
                    .unwrap_or_default();
                watcher.update(ctx, |watcher, _ctx| {
                    if had_previous {
                        std::mem::drop(watcher.unregister_path(&watch_path));
                    }
                    for dir in &previous_extra {
                        std::mem::drop(watcher.unregister_path(dir));
                    }
                    std::mem::drop(watcher.register_path(
                        &watch_path,
                        repo_watch_filter(gitignores, force_included_paths),
                        recursive_mode,
                    ));
                });
            }
        }

        // Insert the repository state into the map
        let repo_path_for_event = repo_path.clone();
        self.replace_repository_state(repo_path, IndexedRepoState::Indexed(state));
        #[cfg(feature = "local_fs")]
        self.refresh_symlink_targets(&repo_path_for_event, ctx);

        ctx.emit(RepositoryMetadataEvent::RepositoryUpdated {
            path: repo_path_for_event,
        });

        Ok(())
    }

    /// Removes a repository from tracking.
    pub fn remove_repository(
        &mut self,
        repo_path: &StandardizedPath,
        ctx: &mut ModelContext<Self>,
    ) -> Result<(), RepoMetadataError> {
        if self.remove_repository_state(repo_path).is_some() {
            self.standing_results.remove(repo_path);
            #[cfg(feature = "local_fs")]
            self.clear_symlink_targets_for_repo(repo_path, ctx);
            // Drop the recorded watch entry and unregister from the watcher.
            #[cfg(feature = "local_fs")]
            {
                let removed = self.repo_watches.remove(repo_path);
                if let Some(ref watcher) = self.watcher {
                    // Uniform teardown: the root watch lives on the repo path,
                    // plus any on-demand per-directory watches in `extra_dirs`.
                    let mut paths_to_unregister: Vec<PathBuf> =
                        repo_path.to_local_path().into_iter().collect();
                    if let Some(removed) = removed {
                        paths_to_unregister.extend(
                            removed
                                .extra_dirs
                                .iter()
                                .filter_map(|dir| dir.to_local_path()),
                        );
                    }
                    watcher.update(ctx, |watcher, _ctx| {
                        for path in &paths_to_unregister {
                            std::mem::drop(watcher.unregister_path(path));
                        }
                    });
                }
            }

            ctx.emit(RepositoryMetadataEvent::RepositoryRemoved {
                path: repo_path.clone(),
            });

            Ok(())
        } else {
            Err(RepoMetadataError::RepoNotFound(repo_path.to_string()))
        }
    }

    pub fn get_repository(&self, repo_path: &StandardizedPath) -> Option<&FileTreeState> {
        match self.repositories.get(repo_path)? {
            IndexedRepoState::Indexed(state) => Some(state),
            IndexedRepoState::Pending(_) => None,
            IndexedRepoState::Failed(_) => None,
        }
    }

    pub fn standing_query_results(
        &self,
        repo_path: &StandardizedPath,
    ) -> Option<&StandingQueryResults> {
        self.standing_results.get(repo_path)
    }

    /// Returns the current [`IndexedRepoState`] for the specified repository or `None` if the
    /// repository is not being tracked.
    pub fn repository_state(&self, repo_path: &StandardizedPath) -> Option<&IndexedRepoState> {
        self.repositories.get(repo_path)
    }

    /// Checks if a repository is being tracked and indexed.
    pub fn has_repository(&self, repo_path: &StandardizedPath) -> bool {
        matches!(
            self.repositories.get(repo_path),
            Some(IndexedRepoState::Indexed(_))
        )
    }

    /// Returns whether the given path is tracked as a lazily-loaded standalone path.
    pub fn is_lazy_loaded_path(&self, path: &StandardizedPath) -> bool {
        self.lazy_loaded_paths.contains_key(path)
    }

    /// Lazily indexes a standalone path with only the first level of children.
    /// Registers the path with the file watcher for live updates.
    /// No-ops if the path is already tracked.
    #[cfg(feature = "local_fs")]
    pub fn index_lazy_loaded_path(
        &mut self,
        path: &StandardizedPath,
        ctx: &mut ModelContext<Self>,
    ) -> Result<(), RepoMetadataError> {
        // Already tracked as a lazy-loaded path — increase the refcount and keep the
        // existing watcher/model entry alive.
        if let Some(refcount) = self.lazy_loaded_paths.get_mut(path) {
            *refcount += 1;
            return Ok(());
        }

        // Already tracked as a real repo — don't overwrite it.
        if matches!(
            self.repositories.get(path),
            Some(IndexedRepoState::Indexed(_) | IndexedRepoState::Pending(_))
        ) {
            return Ok(());
        }

        let local_path = path
            .to_local_path()
            .ok_or_else(|| RepoMetadataError::PathEncodingMismatch(path.clone()))?;
        if !local_path.exists() {
            return Err(RepoMetadataError::RepoNotFound(path.to_string()));
        }
        if !local_path.is_dir() {
            return Err(RepoMetadataError::InvalidPath(
                "Path must be a directory".to_string(),
            ));
        }

        // Build first-level-only tree while collecting standing results across
        // descendants that are not materialized in the lazy file tree.
        let mut files = Vec::new();
        let mut file_limit = MAX_FILES_PER_REPO;
        let mut standing_results = StandingQueryResults::default();
        let root_entry = Entry::build_tree_with_standing_queries(
            &local_path,
            &mut files,
            &mut vec![],
            Some(&mut file_limit),
            BuildTreeOptions {
                max_depth: 1, // Only first level.
                current_depth: 0,
                ignored_path_strategy: &IgnoredPathStrategy::Include,
                force_included_paths: &self.force_included_paths,
                budget_exceeded_behavior: BudgetExceededBehavior::StopAndLazyLoad,
            },
            &mut standing_results,
            &self.standing_query_definitions,
        )
        .map_err(RepoMetadataError::BuildTree)?;

        let state = FileTreeState::new_lazy_loaded(root_entry);
        self.standing_results.insert(path.clone(), standing_results);
        // On Linux, watch lazy (non-git) roots non-recursively to avoid
        // registering an inotify watch for every directory in the subtree.
        // Subdirectories get their own non-recursive watch as they are expanded
        // (see `load_directory`). macOS/Windows watch a whole tree with a single
        // OS handle, so recursive watching stays cheap there.
        let root_mode = if cfg!(target_os = "linux") {
            RootWatchMode::NonRecursive
        } else {
            RootWatchMode::Recursive
        };
        self.add_repository_internal(path.clone(), state, root_mode, ctx)?;
        self.lazy_loaded_paths.insert(path.clone(), 1);
        Ok(())
    }

    /// Removes a lazily-loaded standalone path from tracking and unregisters the file watcher.
    #[cfg(feature = "local_fs")]
    pub fn remove_lazy_loaded_path(
        &mut self,
        path: &StandardizedPath,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(refcount) = self.lazy_loaded_paths.get_mut(path) else {
            return;
        };
        if *refcount > 1 {
            *refcount -= 1;
            return;
        }
        self.lazy_loaded_paths.remove(path);
        // remove_repository unregisters the watcher and emits RepositoryRemoved.
        let _ = self.remove_repository(path, ctx);
    }

    /// Loads a specific directory inside an already-tracked tree.
    /// Emits `FileTreeEntryUpdated` so subscribers can sync.
    #[cfg(feature = "local_fs")]
    pub fn load_directory(
        &mut self,
        repo_root: &StandardizedPath,
        dir_path: &StandardizedPath,
        ctx: &mut ModelContext<Self>,
    ) -> Result<(), RepoMetadataError> {
        let Some(IndexedRepoState::Indexed(state)) = self.repositories.get_mut(repo_root) else {
            return Err(RepoMetadataError::RepoNotFound(repo_root.to_string()));
        };

        // `load_at_path` accumulates gitignores into this vec as it descends, so
        // clone the inner `Vec` (not just the `Arc`). This deep copy only happens
        // on explicit directory expansion, not on the hot watcher-event path.
        let mut gitignores = state.gitignores.as_ref().clone();
        state
            .entry
            .load_at_path(dir_path, &mut gitignores)
            .map_err(RepoMetadataError::BuildTree)?;

        // Start watching the directory we just expanded so its direct children
        // stay fresh. For a non-recursive root this covers every expanded
        // subdir; for a recursive root it covers gitignored dirs pruned from the
        // root watch on Linux. No-op when the root watch already covers it.
        self.watch_subdir(repo_root, dir_path, ctx);

        ctx.emit(RepositoryMetadataEvent::FileTreeEntryUpdated {
            path: repo_root.clone(),
            update_type: MetadataUpdateType::FullReplace,
        });
        Ok(())
    }

    /// Registers an on-demand non-recursive watch on `dir_path` when the root
    /// watch does not already cover it, recording it in `extra_dirs` so it can
    /// be unregistered on teardown. No-ops if `repo_root` is not tracked, the
    /// directory is already watched, or the root watch already covers it.
    ///
    /// Decision by root mode:
    /// - [`RootWatchMode::NonRecursive`]: nothing under the root is covered, so
    ///   every loaded subdir gets its own watch.
    /// - [`RootWatchMode::Recursive`]: the root watch already covers non-pruned
    ///   subtrees; only gitignored dirs are pruned, and only on Linux (other
    ///   backends ignore the descend filter and still deliver their events), so
    ///   we watch those to keep expanded gitignored folders fresh.
    #[cfg(feature = "local_fs")]
    fn watch_subdir(
        &mut self,
        repo_root: &StandardizedPath,
        dir_path: &StandardizedPath,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(repo_watch) = self.repo_watches.get(repo_root) else {
            return;
        };
        if repo_watch.extra_dirs.contains(dir_path) {
            // Already watching this directory.
            return;
        }
        let root_mode = repo_watch.root_mode;

        let should_watch = match root_mode {
            RootWatchMode::NonRecursive => true,
            RootWatchMode::Recursive => {
                cfg!(target_os = "linux") && self.dir_pruned_from_root_watch(repo_root, dir_path)
            }
        };
        if !should_watch {
            return;
        }

        let Some(local_path) = dir_path.to_local_path() else {
            return;
        };
        let gitignores = crate::gitignores_for_directory(&local_path);
        let force_included_paths = self.force_included_paths.clone();
        if let Some(repo_watch) = self.repo_watches.get_mut(repo_root) {
            repo_watch.extra_dirs.insert(dir_path.clone());
        }
        if let Some(ref watcher) = self.watcher {
            watcher.update(ctx, |watcher, _ctx| {
                std::mem::drop(watcher.register_path(
                    &local_path,
                    repo_watch_filter(gitignores, force_included_paths),
                    RecursiveMode::NonRecursive,
                ));
            });
        }
    }

    /// Drops any on-demand per-directory watches at or under `removed_path` when
    /// a directory is deleted or moved away. Each stale path is unregistered from
    /// the watcher and removed from `extra_dirs`. Without this, a directory
    /// recreated at the same path would be skipped by [`watch_subdir`] (which
    /// sees the stale `extra_dirs` entry) and never receive a fresh watch.
    #[cfg(feature = "local_fs")]
    fn unwatch_removed_subtree(
        &mut self,
        repo_root: &StandardizedPath,
        removed_path: &StandardizedPath,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(repo_watch) = self.repo_watches.get_mut(repo_root) else {
            return;
        };
        // The removed directory plus any tracked descendants are now stale.
        // `starts_with` is component-aware and matches the path itself, so this
        // covers both an exact removed dir and everything expanded beneath it.
        let stale: Vec<StandardizedPath> = repo_watch
            .extra_dirs
            .iter()
            .filter(|dir| dir.starts_with(removed_path))
            .cloned()
            .collect();
        if stale.is_empty() {
            return;
        }
        for dir in &stale {
            repo_watch.extra_dirs.remove(dir);
        }
        if let Some(ref watcher) = self.watcher {
            let local_paths: Vec<PathBuf> =
                stale.iter().filter_map(|dir| dir.to_local_path()).collect();
            watcher.update(ctx, |watcher, _ctx| {
                for path in &local_paths {
                    std::mem::drop(watcher.unregister_path(path));
                }
            });
        }
    }

    /// Returns whether `dir_path` would be pruned from `repo_root`'s recursive
    /// watch by the gitignore descend filter (i.e. it is gitignored relative to
    /// the repo root). Used to decide whether an expanded directory needs its
    /// own watch.
    #[cfg(feature = "local_fs")]
    fn dir_pruned_from_root_watch(
        &self,
        repo_root: &StandardizedPath,
        dir_path: &StandardizedPath,
    ) -> bool {
        let Some(IndexedRepoState::Indexed(state)) = self.repositories.get(repo_root) else {
            return false;
        };
        let Some(local) = dir_path.to_local_path() else {
            return false;
        };
        Self::path_is_ignored(&local, &state.gitignores)
    }

    /// Checks whether the parent directory of `path` is loaded in the given entry.
    fn is_parent_loaded_in_entry(entry: &FileTreeEntry, path: &StandardizedPath) -> bool {
        let Some(parent) = path.parent() else {
            return true;
        };
        entry.get(&parent).is_some_and(|state| state.loaded())
    }

    /// Phase 1: Computes file-tree mutations on a background thread.
    ///
    /// Performs all filesystem I/O (`exists()`, `is_dir()`, `build_tree()`,
    /// gitignore checks) and returns a lightweight list of mutations that can
    /// be applied to the tree on the main thread without cloning it.
    ///
    /// When `lazy_load` is true (lazy non-git roots), newly added directories
    /// are emitted as unloaded placeholders rather than fully-materialized
    /// subtrees, matching the lazy tree model; the directory is materialized
    /// (and watched) on demand when the user expands it via `load_directory`.
    async fn compute_file_tree_mutations(
        update: &RepoUpdate,
        gitignores: &[Gitignore],
        force_included_paths: &[PathBuf],
        standing_query_definitions: &StandingQueryDefinitions,
        lazy_load: bool,
    ) -> (
        Vec<FileTreeMutation>,
        StandingQueryResults,
        Vec<StandardizedPath>,
    ) {
        let mut mutations = Vec::new();
        let mut standing_results = StandingQueryResults::default();
        let mut removed_roots = Vec::new();

        // Removals for deleted and moved-from paths
        for path_to_remove in update.deleted.iter().chain(update.moved.values()) {
            mutations.push(FileTreeMutation::Remove(path_to_remove.clone()));
            if let Ok(path) = StandardizedPath::try_from_local(path_to_remove) {
                removed_roots.push(path);
            }
            // A removed direct provider child may have been a symlinked skill directory,
            // which is intentionally absent from the canonical tree and standing file matches.
            standing_results.record_direct_project_skill_provider_child_change(
                path_to_remove,
                standing_query_definitions,
            );
        }

        // Additions for new and moved-to paths
        for path_to_add in update.added.iter().chain(update.moved.keys()) {
            if !path_to_add.exists() {
                continue;
            }

            let is_ignored = Self::path_is_ignored(path_to_add, gitignores);

            if path_to_add.is_dir() {
                if lazy_load {
                    // Lazy (non-git) roots are not materialized when a directory
                    // is created; insert it as an unloaded placeholder and build
                    // the subtree on demand when the user expands it (see
                    // `load_directory`).
                    mutations.push(FileTreeMutation::AddUnloadedDirectory {
                        path: path_to_add.clone(),
                        is_ignored,
                    });
                    continue;
                }

                let mut files = Vec::new();
                let mut gitignores = gitignores.to_owned();
                let mut file_limit = MAX_FILES_PER_REPO;
                match Entry::build_tree_with_standing_queries(
                    path_to_add,
                    &mut files,
                    &mut gitignores,
                    Some(&mut file_limit),
                    BuildTreeOptions {
                        max_depth: MAX_TREE_DEPTH,
                        current_depth: 0,
                        ignored_path_strategy: &IgnoredPathStrategy::IncludeLazy,
                        force_included_paths,
                        budget_exceeded_behavior: BudgetExceededBehavior::StopAndLazyLoad,
                    },
                    &mut standing_results,
                    standing_query_definitions,
                ) {
                    Ok(subtree) => {
                        mutations.push(FileTreeMutation::AddDirectorySubtree {
                            dir_path: path_to_add.clone(),
                            subtree,
                        });
                    }
                    Err(BuildTreeError::Symlink) => {
                        // Directory symlinks are intentionally absent from the canonical tree.
                        // Re-hydrate only when the changed entry itself can introduce a
                        // symlinked skill; ordinary descendants should not wake consumers.
                        standing_results.record_direct_project_skill_provider_child_change(
                            path_to_add,
                            standing_query_definitions,
                        );
                        standing_results.record_followed_project_skill_directory(
                            path_to_add,
                            standing_query_definitions,
                        );
                    }
                    Err(e) => {
                        log::warn!("Failed to build subtree for directory {path_to_add:?}: {e:?}");
                        mutations.push(FileTreeMutation::AddUnloadedDirectory {
                            path: path_to_add.clone(),
                            is_ignored,
                        });
                    }
                }
            } else {
                standing_results.record_path(path_to_add, false, standing_query_definitions);
                let extension = path_to_add
                    .extension()
                    .and_then(|ext| ext.to_str().map(|s| s.to_owned()));
                mutations.push(FileTreeMutation::AddFile {
                    path: path_to_add.clone(),
                    is_ignored,
                    extension,
                });
            }
        }

        (mutations, standing_results, removed_roots)
    }

    /// Phase 2: Applies pre-computed mutations to the file tree on the main thread.
    ///
    /// No filesystem I/O — only tree-structure operations. When `lazy_load` is
    /// true, additions are skipped if the parent directory has not been expanded.
    ///
    /// When `emit_updates` is true, returns a [`RepoMetadataUpdate`] built from the mutations
    /// that were actually applied (filtering out any skipped by `lazy_load`), suitable for
    /// consumers and the remote client. When false, no update tracking is performed and the
    /// function returns `None`.
    pub(crate) fn apply_file_tree_mutations(
        root_entry: &mut FileTreeEntry,
        mutations: Vec<FileTreeMutation>,
        lazy_load: bool,
        emit_updates: bool,
    ) -> Option<RepoMetadataUpdate> {
        let emit = emit_updates;
        let mut remove_entries: Vec<StandardizedPath> = Vec::new();
        let mut update_entries: Vec<FileTreeEntryUpdate> = Vec::new();

        for mutation in mutations {
            match mutation {
                FileTreeMutation::Remove(ref path) => {
                    let Some(std_path) = StandardizedPath::try_from_local(path).ok() else {
                        continue;
                    };
                    root_entry.remove(&std_path);
                    if emit {
                        remove_entries.push(std_path);
                    }
                }
                FileTreeMutation::AddFile {
                    ref path,
                    is_ignored,
                    ref extension,
                } => {
                    let Some(std_path) = StandardizedPath::try_from_local(path).ok() else {
                        continue;
                    };
                    if lazy_load && !Self::is_parent_loaded_in_entry(root_entry, &std_path) {
                        continue;
                    }
                    let Some(parent) = std_path.parent() else {
                        continue;
                    };
                    Self::ensure_parent_directories_exist(root_entry, &parent);

                    let Some(parent_dir) = root_entry.find_parent_directory(&std_path) else {
                        continue;
                    };

                    // If the file already exists in the tree, just update its ignored flag
                    // to preserve the existing FileId.
                    if let Some(entry) = root_entry.get_mut(&std_path) {
                        entry.set_ignored(is_ignored);
                    } else {
                        let file_state = FileTreeEntryState::File(FileTreeFileMetadata {
                            path: Arc::new(std_path.clone()),
                            file_id: FileId::new(),
                            extension: extension.clone(),
                            ignored: is_ignored,
                        });
                        root_entry.insert_child_state(&parent_dir, file_state);
                    }
                    if emit {
                        update_entries.push(FileTreeEntryUpdate {
                            parent_path_to_replace: parent.clone(),
                            subtree_metadata: vec![RepoNodeMetadata::File(FileNodeMetadata {
                                path: std_path,
                                extension: extension.clone(),
                                ignored: is_ignored,
                            })],
                        });
                    }
                }
                FileTreeMutation::AddDirectorySubtree {
                    ref dir_path,
                    ref subtree,
                } => {
                    let Some(std_dir) = StandardizedPath::try_from_local(dir_path).ok() else {
                        continue;
                    };
                    if lazy_load && !Self::is_parent_loaded_in_entry(root_entry, &std_dir) {
                        continue;
                    }
                    if let Some(parent) = std_dir.parent() {
                        Self::ensure_parent_directories_exist(root_entry, &parent);
                    }
                    if let Some(parent_path) = root_entry.find_parent_directory(&std_dir) {
                        if let Some(FileTreeEntryState::Directory(directory)) =
                            root_entry.get_mut(&parent_path)
                        {
                            directory.loaded = true;
                        }
                        root_entry.remove(subtree.path());
                        root_entry.insert_entry_at_path(
                            Arc::new(subtree.path().clone()),
                            subtree.clone(),
                        );
                        if emit {
                            let parent_std = std_dir.parent().unwrap_or(std_dir.clone());
                            let metadata = flatten_entry_metadata(subtree);
                            update_entries.push(FileTreeEntryUpdate {
                                parent_path_to_replace: parent_std,
                                subtree_metadata: metadata,
                            });
                        }
                    }
                }
                FileTreeMutation::AddUnloadedDirectory {
                    ref path,
                    is_ignored,
                } => {
                    let Some(std_path) = StandardizedPath::try_from_local(path).ok() else {
                        continue;
                    };
                    if lazy_load && !Self::is_parent_loaded_in_entry(root_entry, &std_path) {
                        continue;
                    }
                    let Some(parent) = std_path.parent() else {
                        continue;
                    };
                    Self::ensure_parent_directories_exist(root_entry, &parent);

                    let Some(parent_dir) = root_entry.find_parent_directory(&std_path) else {
                        continue;
                    };

                    let dir_state = FileTreeEntryState::Directory(FileTreeDirectoryEntryState {
                        path: Arc::new(std_path.clone()),
                        ignored: is_ignored,
                        loaded: false,
                    });
                    root_entry.insert_child_state(&parent_dir, dir_state);
                    if emit {
                        update_entries.push(FileTreeEntryUpdate {
                            parent_path_to_replace: parent.clone(),
                            subtree_metadata: vec![RepoNodeMetadata::Directory(
                                DirectoryNodeMetadata {
                                    path: std_path,
                                    ignored: is_ignored,
                                    loaded: false,
                                },
                            )],
                        });
                    }
                }
            }
        }

        if !emit {
            return None;
        }

        Some(RepoMetadataUpdate {
            repo_path: root_entry.root_directory().as_ref().clone(),
            remove_entries,
            update_entries,
            standing_results_delta: StandingQueryResultsDelta::default(),
        })
    }

    /// Delegates to [`FileTreeEntry::ensure_parent_directories_exist`].
    fn ensure_parent_directories_exist(
        root_entry: &mut FileTreeEntry,
        target_parent: &StandardizedPath,
    ) {
        root_entry.ensure_parent_directories_exist(target_parent);
    }

    /// Checks if a path matches any of the gitignore patterns
    fn path_is_ignored(path: &Path, gitignores: &[Gitignore]) -> bool {
        // Check if any component of the path is .git
        if path
            .components()
            .any(|component| component.as_os_str() == ".git")
        {
            return true;
        }

        // Check if path matches any gitignore patterns
        let is_dir = path.is_dir();
        matches_gitignores(path, is_dir, gitignores, true)
    }

    /// Fully indexes a local directory after registering it with the directory watcher.
    #[cfg(feature = "local_fs")]
    pub fn index_directory_path(
        &mut self,
        path: &StandardizedPath,
        ctx: &mut ModelContext<'_, Self>,
    ) -> Result<(), RepoMetadataError> {
        let path = path.clone();
        let repository = DirectoryWatcher::handle(ctx)
            .update(ctx, |watcher, ctx| watcher.add_directory(path, ctx))?;
        self.index_directory(repository, ctx)
    }

    /// Indexes a repository from the given repository handle.
    pub fn index_directory(
        &mut self,
        repository: ModelHandle<Repository>,
        ctx: &mut ModelContext<'_, Self>,
    ) -> Result<(), RepoMetadataError> {
        let std_path = repository.as_ref(ctx).root_dir().clone();
        let local_path = std_path
            .to_local_path()
            .ok_or_else(|| RepoMetadataError::PathEncodingMismatch(std_path.clone()))?;

        // Validate the repository path exists and is a directory
        if !local_path.exists() {
            return Err(RepoMetadataError::RepoNotFound(std_path.to_string()));
        }

        if !local_path.is_dir() {
            return Err(RepoMetadataError::InvalidPath(
                "Repository path must be a directory".to_string(),
            ));
        }

        let repo_path_str = std_path.to_string();

        // Check if the repository is already indexed or currently being indexed.
        // Allow re-indexing if the existing entry was a lazily-loaded path placeholder.
        match self.repositories.get(&std_path) {
            Some(IndexedRepoState::Indexed(_))
                if !self.lazy_loaded_paths.contains_key(&std_path) =>
            {
                log::debug!("Repository already indexed: {std_path}");
                return Ok(());
            }
            Some(IndexedRepoState::Indexed(_)) => {
                // Was a lazy-loaded path – allow upgrading to a fully indexed directory.
                log::info!(
                    "Upgrading lazy-loaded path to fully indexed directory: {repo_path_str}"
                );
                self.lazy_loaded_paths.remove(&std_path);
            }
            Some(IndexedRepoState::Pending(_)) => {
                log::debug!("Repository already being indexed: {repo_path_str}");
                return Ok(());
            }
            Some(IndexedRepoState::Failed(error)) => {
                log::debug!(
                    "Repository indexing previously failed: {repo_path_str}, error: {error}"
                );
                log::info!("Retrying indexing for previously failed repository: {repo_path_str}");
                // Continue to retry indexing
            }
            None => {
                // Repository is not indexed and not pending, proceed with indexing
            }
        }

        // Collect gitignore files from the repository
        let gitignores = gitignores_for_directory(&local_path);

        // Mark the repository as pending to prevent duplicate work
        self.replace_repository_state(std_path.clone(), IndexedRepoState::pending());

        // Use the provided repository handle instead of creating a new one
        let repository_handle = repository;

        // Build the complete file tree for the repository asynchronously
        let repo_path_for_build = local_path;
        let gitignores_for_build = gitignores.clone();
        let force_included_paths = self.force_included_paths.clone();
        let standing_query_definitions = self.standing_query_definitions.clone();
        let repo_path_str_for_log = std_path.to_string();
        let std_path_for_completion = std_path;
        let repository_handle_for_completion = repository_handle.clone();

        ctx.spawn(
            async move {
                let mut files: Vec<crate::entry::FileMetadata> = Vec::new();
                let mut gitignores_for_build = gitignores_for_build;
                let mut standing_results = StandingQueryResults::default();

                // Budget for non-ignored files. When it is exhausted the builder
                // stops descending breadth-first and leaves the remaining
                // directories as unloaded placeholders (lazy-loaded on demand)
                // instead of failing the whole build. Gitignored subtrees stay
                // lazy and registered force-included paths are always loaded;
                // both are handled inside the builder.
                let mut file_limit = MAX_FILES_PER_REPO;

                let build_result = Entry::build_tree_with_standing_queries(
                    &repo_path_for_build,
                    &mut files,
                    &mut gitignores_for_build,
                    Some(&mut file_limit),
                    BuildTreeOptions {
                        max_depth: MAX_TREE_DEPTH,
                        current_depth: 0,
                        ignored_path_strategy: &IgnoredPathStrategy::IncludeLazy,
                        force_included_paths: &force_included_paths,
                        budget_exceeded_behavior: BudgetExceededBehavior::StopAndLazyLoad,
                    },
                    &mut standing_results,
                    &standing_query_definitions,
                );

                // A fully-exhausted budget means the repo was too large to index
                // eagerly: the tree is partial (with a lazy-loaded remainder)
                // but still browsable and searchable as far as it goes.
                let indexed_with_limit = file_limit == 0;

                (
                    build_result,
                    files,
                    gitignores_for_build,
                    repo_path_str_for_log,
                    std_path_for_completion,
                    repository_handle_for_completion,
                    indexed_with_limit,
                    standing_results,
                )
            },
            move |model: &mut LocalRepoMetadataModel,
                  (
                      build_result,
                      files,
                      gitignores_for_build,
                      repo_path_str,
                      std_repo_path,
                      repository_handle,
                      indexed_with_limit,
                      standing_results,
                  ): (Result<Entry, _>, Vec<crate::entry::FileMetadata>, _, String, StandardizedPath, ModelHandle<Repository>, bool, StandingQueryResults),
                  ctx| {
                match build_result {
                    Ok(root_entry) => {
                        model
                            .standing_results
                            .insert(std_repo_path.clone(), standing_results);
                        let state =
                            FileTreeState::new(root_entry, gitignores_for_build, Some(repository_handle));

                        if let Err(e) = model.add_repository_internal(
                            std_repo_path.clone(),
                            state,
                            RootWatchMode::Recursive,
                            ctx,
                        ) {
                            log::warn!("Failed to add repository {repo_path_str}: {e:?}");
                            // On failure, mark the repository as failed so waiters are notified.
                            model.mark_repository_failed(std_repo_path, e, ctx);
                        } else if indexed_with_limit {
                            safe_warn!(
                                safe: ("Repository exceeded max file budget; indexed with partial coverage"),
                                full: ("Repository {repo_path_str} exceeded the max file budget ({MAX_FILES_PER_REPO}); indexed breadth-first up to the budget — remaining directories load on expand")
                            );
                            send_telemetry_from_ctx!(RepoMetadataTelemetryEvent::BuildTreeFailed { error: format!("{:#}", BuildTreeError::ExceededMaxFileLimit) }, ctx);
                        } else {
                            log::info!(
                                "Successfully indexed repository: {} with {} files",
                                repo_path_str,
                                files.len()
                            );
                        }
                    }
                    Err(e) => {
                        safe_warn!(
                            safe: ("Failed to build file tree for repository: {e:?}"),
                            full: ("Failed to build file tree for repository {repo_path_str}: {e:?}")
                        );
                        send_telemetry_from_ctx!(RepoMetadataTelemetryEvent::BuildTreeFailed { error: format!("{e:#}") }, ctx);
                        model.mark_repository_failed(
                            std_repo_path,
                            RepoMetadataError::BuildTree(e),
                            ctx,
                        );
                    }
                }
            },
        );

        Ok(())
    }

    /// Returns repository contents (files and optionally directories) in a given repository.
    ///
    /// At most [`MAX_REPO_CONTENTS_RESULTS`] entries are returned. When the
    /// repository contains more matching entries, the result is truncated to
    /// that cap and [`RepoContents::truncated`] is set to `true`.
    ///
    /// Returns an error if the repository is not indexed, indexing is pending, or indexing failed.
    pub fn get_repo_contents(
        &self,
        repo_path: &StandardizedPath,
        args: GetContentsArgs,
    ) -> Result<RepoContents<'_>, RepoMetadataError> {
        let state = match self.repositories.get(repo_path) {
            Some(IndexedRepoState::Indexed(state)) => state,
            Some(IndexedRepoState::Pending(_)) => {
                return Err(RepoMetadataError::RepositoryIndexingPending);
            }
            Some(IndexedRepoState::Failed(_)) => {
                return Err(RepoMetadataError::RepositoryIndexingFailed);
            }
            None => {
                return Err(RepoMetadataError::RepositoryNotIndexed);
            }
        };
        let mut contents = Vec::new();
        let truncated = collect_contents_recursive(
            &state.entry,
            state.entry.root_directory(),
            &mut contents,
            &args,
        );
        Ok(RepoContents {
            contents,
            truncated,
        })
    }

    /// Change the indexing state of `repo_path` to `state`.
    ///
    /// All changes to the state **must** go through this method so that
    /// waiters are properly notified.
    fn replace_repository_state(
        &mut self,
        repo_path: StandardizedPath,
        state: IndexedRepoState,
    ) -> Option<IndexedRepoState> {
        let previous = self.repositories.insert(repo_path, state);
        if let Some(previous) = &previous {
            previous.complete_if_pending();
        }
        previous
    }

    /// Drop the indexing state for `repo_path`, notifying any waiters.
    fn remove_repository_state(
        &mut self,
        repo_path: &StandardizedPath,
    ) -> Option<IndexedRepoState> {
        let previous = self.repositories.remove(repo_path);
        if let Some(previous) = &previous {
            previous.complete_if_pending();
        }
        previous
    }

    /// Mark indexing as failed for `repo_path` and emit an `UpdatingRepositoryFailed` event.
    fn mark_repository_failed(
        &mut self,
        repo_path: StandardizedPath,
        error: RepoMetadataError,
        ctx: &mut ModelContext<Self>,
    ) {
        self.standing_results.remove(&repo_path);
        #[cfg(feature = "local_fs")]
        self.clear_symlink_targets_for_repo(&repo_path, ctx);
        self.replace_repository_state(repo_path.clone(), IndexedRepoState::Failed(error));
        ctx.emit(RepositoryMetadataEvent::UpdatingRepositoryFailed { path: repo_path });
    }

    /// Returns a future that resolves once repository indexing reaches a terminal state.
    ///
    /// Callers should check [`Self::repository_state`] after awaiting this future to see whether
    /// indexing succeeded or failed.
    pub fn repository_indexed(&self, repo_path: &StandardizedPath) -> BoxFuture<'static, ()> {
        match self.repositories.get(repo_path) {
            Some(state) => state.wait_until_indexed(),
            None => future::ready(()).boxed(),
        }
    }
}

impl warpui_core::Entity for LocalRepoMetadataModel {
    type Event = RepositoryMetadataEvent;
}

/// Helper function to recursively collect contents (files and optionally directories) from an Entry tree.
///
/// Collects at most [`MAX_REPO_CONTENTS_RESULTS`] entries into `contents`.
/// Returns `true` if traversal stopped early because that cap was reached,
/// indicating the collected `contents` are truncated and more matching entries
/// exist.
pub(crate) fn collect_contents_recursive<'a>(
    entry: &'a FileTreeEntry,
    current_path: &'a StandardizedPath,
    contents: &mut Vec<RepoContent<'a>>,
    args: &GetContentsArgs,
) -> bool {
    if !args.include_ignored && entry.ignored(current_path) {
        return false;
    }

    match entry.get(current_path) {
        Some(FileTreeEntryState::File(metadata)) => {
            let content = RepoContent::File(metadata);
            if args.filter.as_ref().is_none_or(|f| f(&content)) {
                // Stop before exceeding the cap, reporting that results are truncated.
                if contents.len() >= MAX_REPO_CONTENTS_RESULTS {
                    return true;
                }
                contents.push(content);
            }
        }
        Some(FileTreeEntryState::Directory(dir)) => {
            if args.include_folders {
                let content = RepoContent::Directory(dir);
                if args.filter.as_ref().is_none_or(|f| f(&content)) {
                    // Stop before exceeding the cap, reporting that results are truncated.
                    if contents.len() >= MAX_REPO_CONTENTS_RESULTS {
                        return true;
                    }
                    contents.push(content);
                }
            }

            for child in entry.child_paths(current_path) {
                if collect_contents_recursive(entry, child, contents, args) {
                    return true;
                }
            }
        }
        None => {}
    }
    false
}

// Test helpers
#[cfg(any(test, feature = "test-util"))]
impl LocalRepoMetadataModel {
    /// Insert a repository state directly for testing purposes.
    pub fn insert_test_state(&mut self, repo_path: StandardizedPath, state: FileTreeState) {
        self.replace_repository_state(repo_path, IndexedRepoState::Indexed(state));
    }

    pub fn insert_test_standing_results(
        &mut self,
        repo_path: StandardizedPath,
        standing_results: StandingQueryResults,
    ) {
        self.standing_results.insert(repo_path, standing_results);
    }
}

#[cfg(test)]
#[path = "local_model_tests.rs"]
mod tests;
