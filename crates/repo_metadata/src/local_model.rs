#![cfg_attr(not(feature = "local_fs"), allow(dead_code))]
//! Repository metadata model singleton.
//!
//! This module provides a singleton model that manages repository metadata across
//! all repositories tracked by Warp.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use futures::future::{self, BoxFuture, FutureExt as _};
use warp_core::{safe_warn, send_telemetry_from_ctx};
use warp_util::sync::Condition;
use warpui::ModelHandle;

/// Represents either a file or directory in a repository.
#[derive(Debug, Clone)]
pub enum RepoContent<'a> {
    File(&'a FileTreeFileMetadata),
    Directory(&'a FileTreeDirectoryEntryState),
}

use warp_util::standardized_path::StandardizedPath;

use crate::{
    entry::{BuildTreeError, Entry, FileId, IgnoredPathStrategy},
    gitignores_for_directory, matches_gitignores,
    repository::Repository,
    telemetry::RepoMetadataTelemetryEvent,
    RepoMetadataError,
};
cfg_if::cfg_if! {
    if #[cfg(feature = "local_fs")] {
        use notify_debouncer_full::notify::{RecursiveMode, WatchFilter};
        use crate::repositories::{DetectedRepositories, DetectedRepositoriesEvent};
        use watcher::{BulkFilesystemWatcher, BulkFilesystemWatcherEvent};
        use warpui::SingletonEntity as _;

        /// Duration between filesystem watch events in seconds
        const FILESYSTEM_WATCHER_DEBOUNCE_SECS: u64 = 1;
    }
}

use crate::file_tree_store::{
    FileTreeDirectoryEntryState, FileTreeEntry, FileTreeEntryState, FileTreeFileMetadata,
    FileTreeState,
};
use crate::file_tree_update::{
    flatten_entry_metadata, DirectoryNodeMetadata, FileNodeMetadata, FileTreeEntryUpdate,
    RepoMetadataUpdate, RepoNodeMetadata,
};
use ignore::gitignore::Gitignore;
use warpui::ModelContext;

/// Maximum depth to traverse when building file trees
const MAX_TREE_DEPTH: usize = 200;

/// Maximum number of files to index per repository to guard against really large codebases
const MAX_FILES_PER_REPO: usize = 100_000;

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

    pub(crate) fn complete_if_pending(&self) {
        if let Self::Pending(condition) = self {
            condition.set();
        }
    }
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
    /// Refcounts for lazily-loaded standalone paths tracked in the model.
    lazy_loaded_paths: HashMap<StandardizedPath, usize>,
    /// File system watcher for monitoring changes.
    #[cfg(feature = "local_fs")]
    watcher: Option<ModelHandle<BulkFilesystemWatcher>>,
    /// When true, emit [`RepositoryMetadataEvent::IncrementalUpdateReady`]
    /// events after applying watcher mutations. Only the remote server
    /// variant enables this.
    emit_incremental_updates: bool,
}

#[derive(Debug, Clone, Default)]
struct RepoUpdate {
    added: Vec<PathBuf>,
    deleted: Vec<PathBuf>,
    moved: HashMap<PathBuf, PathBuf>,
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
    /// Fallback: add a bare (unloaded) directory entry when `build_tree` fails.
    AddEmptyDirectory { path: PathBuf, is_ignored: bool },
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
            lazy_loaded_paths: HashMap::new(),
            #[cfg(feature = "local_fs")]
            watcher: None,
            emit_incremental_updates: false,
        };
        cfg_if::cfg_if! {
            if #[cfg(feature = "local_fs")] {
                let watcher = ctx.add_model(|ctx| {
                    BulkFilesystemWatcher::new(
                        std::time::Duration::from_secs(FILESYSTEM_WATCHER_DEBOUNCE_SECS),
                        ctx,
                    )
                });
                ctx.subscribe_to_model(&watcher, Self::handle_watcher_event);
                model.watcher = Some(watcher);

                ctx.subscribe_to_model(&DetectedRepositories::handle(ctx), |me, event, ctx| {
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

    /// Handles events from the BulkFilesystemWatcher.
    #[cfg(feature = "local_fs")]
    fn handle_watcher_event(
        &mut self,
        event: &BulkFilesystemWatcherEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        // Create a map to collect changes per repository
        let mut repo_updates: HashMap<StandardizedPath, RepoUpdate> = HashMap::new();

        // Process added or updated files
        for path in event.added_or_updated_iter() {
            if let Some(repo_path) = self.find_repository_for_path(path) {
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
            } else {
                log::warn!("Deleted file not found in any repo: {path:?} not found in any repo");
            }
        }

        // Process moved files
        for (to_path, from_path) in &event.moved {
            if let Some(repo_path) = self.find_repository_for_path(to_path) {
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
                let gitignores_clone = state.gitignores.clone();
                let lazy_load = self.lazy_loaded_paths.contains_key(&repo_path);
                ctx.spawn(
                    async move {
                        let mutations = Self::compute_file_tree_mutations(
                            &repo_scoped_update,
                            &gitignores_clone,
                        )
                        .await;
                        (mutations, repo_path_clone, lazy_load)
                    },
                    |model, (mutations, repo_path, lazy_load), ctx| {
                        if let Some(IndexedRepoState::Indexed(state)) =
                            model.repositories.get_mut(&repo_path)
                        {
                            let update = Self::apply_file_tree_mutations(
                                &mut state.entry,
                                mutations,
                                lazy_load,
                                model.emit_incremental_updates,
                            );
                            ctx.emit(RepositoryMetadataEvent::FileTreeEntryUpdated {
                                path: repo_path,
                            });

                            if let Some(update) = update {
                                ctx.emit(RepositoryMetadataEvent::IncrementalUpdateReady {
                                    update,
                                });
                            }
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
    pub fn find_repository_for_path(&self, path: &Path) -> Option<StandardizedPath> {
        match StandardizedPath::from_local_canonicalized(path) {
            Ok(std_path) => self.find_repository_for_path_string(std_path.as_str()),
            Err(_) => None,
        }
    }

    /// Adds or updates a repository's file tree state.
    fn add_repository_internal(
        &mut self,
        repo_path: StandardizedPath,
        state: FileTreeState,
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

        // Register this path with the watcher if we have one
        #[cfg(feature = "local_fs")]
        {
            if let Some(ref watcher) = self.watcher {
                let watch_path = local_path.clone();
                watcher.update(ctx, |watcher, _ctx| {
                    use crate::entry::should_ignore_git_path;
                    let watch_filter = WatchFilter::with_filter(Arc::new(move |watch_path| {
                        !should_ignore_git_path(watch_path)
                    }));
                    std::mem::drop(watcher.register_path(
                        &watch_path,
                        watch_filter,
                        RecursiveMode::Recursive,
                    ));
                });
            }
        }

        // Insert the repository state into the map
        let repo_path_for_event = repo_path.clone();
        self.replace_repository_state(repo_path, IndexedRepoState::Indexed(state));

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
            // Unregister from watcher
            #[cfg(feature = "local_fs")]
            {
                if let Some(ref watcher) = self.watcher {
                    if let Some(local_path) = repo_path.to_local_path() {
                        watcher.update(ctx, |watcher, _ctx| {
                            std::mem::drop(watcher.unregister_path(&local_path));
                        });
                    }
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

        // Build first-level-only tree.
        let mut files = Vec::new();
        let mut file_limit = MAX_FILES_PER_REPO;
        let root_entry = Entry::build_tree(
            &local_path,
            &mut files,
            &mut vec![],
            Some(&mut file_limit),
            1, // max_depth — only first level
            0,
            &IgnoredPathStrategy::Include,
        )
        .map_err(RepoMetadataError::BuildTree)?;

        let state = FileTreeState::new_lazy_loaded(root_entry);
        self.add_repository_internal(path.clone(), state, ctx)?;
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

        let mut gitignores = state.gitignores.clone();
        state
            .entry
            .load_at_path(dir_path, &mut gitignores)
            .map_err(RepoMetadataError::BuildTree)?;

        ctx.emit(RepositoryMetadataEvent::FileTreeEntryUpdated {
            path: repo_root.clone(),
        });
        Ok(())
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
    async fn compute_file_tree_mutations(
        update: &RepoUpdate,
        gitignores: &[Gitignore],
    ) -> Vec<FileTreeMutation> {
        let mut mutations = Vec::new();

        // Removals for deleted and moved-from paths
        for path_to_remove in update.deleted.iter().chain(update.moved.values()) {
            mutations.push(FileTreeMutation::Remove(path_to_remove.clone()));
        }

        // Additions for new and moved-to paths
        for path_to_add in update.added.iter().chain(update.moved.keys()) {
            if !path_to_add.exists() {
                continue;
            }

            let is_ignored = Self::path_is_ignored(path_to_add, gitignores);

            if path_to_add.is_dir() {
                let mut files = Vec::new();
                let mut gitignores = gitignores.to_owned();
                let mut file_limit = MAX_FILES_PER_REPO;
                match Entry::build_tree(
                    path_to_add,
                    &mut files,
                    &mut gitignores,
                    Some(&mut file_limit),
                    MAX_TREE_DEPTH,
                    0,
                    &IgnoredPathStrategy::IncludeLazy,
                ) {
                    Ok(subtree) => {
                        mutations.push(FileTreeMutation::AddDirectorySubtree {
                            dir_path: path_to_add.clone(),
                            subtree,
                        });
                    }
                    Err(e) => {
                        log::warn!("Failed to build subtree for directory {path_to_add:?}: {e:?}");
                        mutations.push(FileTreeMutation::AddEmptyDirectory {
                            path: path_to_add.clone(),
                            is_ignored,
                        });
                    }
                }
            } else {
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

        mutations
    }

    /// Phase 2: Applies pre-computed mutations to the file tree on the main thread.
    ///
    /// No filesystem I/O — only tree-structure operations. When `lazy_load` is
    /// true, additions are skipped if the parent directory has not been expanded.
    ///
    /// When `emit_updates` is true,
    /// from the mutations that were actually applied (filtering out any skipped
    /// by `lazy_load`), suitable for sending to the remote client. When false,
    /// no update tracking is performed and the function returns `None`.
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
                FileTreeMutation::AddEmptyDirectory {
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
                // Was a lazy-loaded path – allow upgrading to a real repo.
                log::info!("Upgrading lazy-loaded path to git repo: {repo_path_str}");
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
        let repo_path_str_for_log = std_path.to_string();
        let std_path_for_completion = std_path;
        let repository_handle_for_completion = repository_handle.clone();

        ctx.spawn(
            async move {
                let mut files: Vec<crate::entry::FileMetadata> = Vec::new();
                let mut gitignores_for_build = gitignores_for_build;
                // Snapshot the initial gitignores so we can retry from a clean
                // state if the full-depth build is partially populated before
                // it hits the file limit.
                let initial_gitignores = gitignores_for_build.clone();

                let mut file_limit = MAX_FILES_PER_REPO;

                let mut build_result = Entry::build_tree(
                    &repo_path_for_build,
                    &mut files,
                    &mut gitignores_for_build,
                    Some(&mut file_limit),
                    MAX_TREE_DEPTH,        // max_depth
                    0,                 // current_depth
                    &IgnoredPathStrategy::IncludeLazy,
                );

                // Repos with more than MAX_FILES_PER_REPO tracked files can't
                // be indexed at full depth. Fall back to a single-level scan
                // (with the file quota disabled — direct-child files at
                // depth=1 still consume the quota, so reusing it would
                // re-trigger ExceededMaxFileLimit on repos with >MAX_FILES_PER_REPO
                // files directly under the root) so the user can still browse
                // the tree; subdirectories are loaded on expand via
                // LAZY_LOAD_FILE_LIMIT.
                let mut indexed_with_limit = false;
                if matches!(build_result, Err(BuildTreeError::ExceededMaxFileLimit)) {
                    files.clear();
                    gitignores_for_build = initial_gitignores;
                    build_result = Entry::build_tree(
                        &repo_path_for_build,
                        &mut files,
                        &mut gitignores_for_build,
                        None,
                        1, // max_depth — only first level
                        0,
                        &IgnoredPathStrategy::IncludeLazy,
                    );
                    if build_result.is_ok() {
                        indexed_with_limit = true;
                    }
                }

                (
                    build_result,
                    files,
                    gitignores_for_build,
                    repo_path_str_for_log,
                    std_path_for_completion,
                    repository_handle_for_completion,
                    indexed_with_limit,
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
                  ): (Result<Entry, _>, Vec<crate::entry::FileMetadata>, _, String, StandardizedPath, ModelHandle<Repository>, bool),
                  ctx| {
                match build_result {
                    Ok(root_entry) => {
                        let state =
                            FileTreeState::new(root_entry, gitignores_for_build, Some(repository_handle));

                        if let Err(e) =
                            model.add_repository_internal(std_repo_path.clone(), state, ctx)
                        {
                            log::warn!("Failed to add repository {repo_path_str}: {e:?}");
                            // On failure, mark the repository as failed so waiters are notified.
                            model.mark_repository_failed(std_repo_path, e, ctx);
                        } else if indexed_with_limit {
                            safe_warn!(
                                safe: ("Repository exceeded max file limit; indexed in degraded mode"),
                                full: ("Repository {repo_path_str} exceeded max file limit ({MAX_FILES_PER_REPO}); indexed only first level — subdirectories load on expand")
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
    pub fn get_repo_contents(
        &self,
        repo_path: &StandardizedPath,
        args: GetContentsArgs,
    ) -> Option<Vec<RepoContent<'_>>> {
        let state = match self.repositories.get(repo_path)? {
            IndexedRepoState::Indexed(state) => state,
            IndexedRepoState::Pending(_) => return None,
            IndexedRepoState::Failed(_) => return None,
        };
        let mut contents = Vec::new();
        collect_contents_recursive(
            &state.entry,
            state.entry.root_directory(),
            &mut contents,
            &args,
        );
        Some(contents)
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

impl warpui::Entity for LocalRepoMetadataModel {
    type Event = RepositoryMetadataEvent;
}

/// Helper function to recursively collect contents (files and optionally directories) from an Entry tree.
pub(crate) fn collect_contents_recursive<'a>(
    entry: &'a FileTreeEntry,
    current_path: &'a StandardizedPath,
    contents: &mut Vec<RepoContent<'a>>,
    args: &GetContentsArgs,
) {
    if !args.include_ignored && entry.ignored(current_path) {
        return;
    }

    match entry.get(current_path) {
        Some(FileTreeEntryState::File(metadata)) => {
            let content = RepoContent::File(metadata);
            if args.filter.as_ref().is_none_or(|f| f(&content)) {
                contents.push(content);
            }
        }
        Some(FileTreeEntryState::Directory(dir)) => {
            if args.include_folders {
                let content = RepoContent::Directory(dir);
                if args.filter.as_ref().is_none_or(|f| f(&content)) {
                    contents.push(content);
                }
            }

            for child in entry.child_paths(current_path) {
                collect_contents_recursive(entry, child, contents, args);
            }
        }
        None => {}
    }
}

// Test helpers
#[cfg(any(test, feature = "test-util"))]
impl LocalRepoMetadataModel {
    /// Insert a repository state directly for testing purposes.
    pub fn insert_test_state(&mut self, repo_path: StandardizedPath, state: FileTreeState) {
        self.replace_repository_state(repo_path, IndexedRepoState::Indexed(state));
    }
}

#[cfg(test)]
#[path = "local_model_tests.rs"]
mod tests;
