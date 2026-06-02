mod file_tree_state;

use std::sync::Arc;

use ignore::gitignore::Gitignore;
use warp_util::standardized_path::StandardizedPath;
use warpui_core::ModelHandle;

use crate::file_tree_store::file_tree_state::FileTreeMapStore;
use crate::{BuildTreeError, Entry, FileId, FileMetadata, Repository};

#[derive(Debug, Clone)]
pub struct FileTreeEntry {
    state_map: FileTreeMapStore,
    root_path: Arc<StandardizedPath>,
}

impl FileTreeEntry {
    pub fn ignored(&self, path: &StandardizedPath) -> bool {
        let Some(entry_state) = self.state_map.get(path) else {
            return false;
        };

        match entry_state {
            FileTreeEntryState::File(file) => file.ignored,
            FileTreeEntryState::Directory(directory) => directory.ignored,
        }
    }

    pub fn get(&self, path: &StandardizedPath) -> Option<&FileTreeEntryState> {
        self.state_map.get(path)
    }

    pub fn contains(&self, path: &StandardizedPath) -> bool {
        self.state_map.contains(path)
    }

    pub fn root_directory(&self) -> &Arc<StandardizedPath> {
        &self.root_path
    }

    pub fn rename_path(&mut self, path: &StandardizedPath, new_path: &StandardizedPath) -> bool {
        self.state_map.rename_path(path, new_path)
    }

    pub fn load_at_path(
        &mut self,
        path: &StandardizedPath,
        gitignores: &mut Vec<Gitignore>,
    ) -> Result<(), BuildTreeError> {
        self.state_map.load_at_path(path, gitignores)
    }

    pub fn insert_entry_at_path(&mut self, path: Arc<StandardizedPath>, entry: Entry) {
        self.state_map.insert_entry_at_path(path, entry);
    }

    pub fn child_paths(
        &self,
        path: &StandardizedPath,
    ) -> impl Iterator<Item = &Arc<StandardizedPath>> {
        self.state_map.children(path)
    }

    pub fn get_mut(&mut self, path: &StandardizedPath) -> Option<&mut FileTreeEntryState> {
        self.state_map.get_mut(path)
    }

    pub fn remove(&mut self, path: &StandardizedPath) {
        self.state_map.remove(path);
    }

    pub fn new_for_directory(root_path: Arc<StandardizedPath>) -> Self {
        Self {
            state_map: FileTreeMapStore::new_for_directory(root_path.clone()),
            root_path,
        }
    }

    /// Similar to find_or_insert_child but specifically for creating directory entries.
    /// This is used when we know the path should be a directory (e.g., when ensuring parent directories exist).
    pub fn find_or_insert_directory(
        &mut self,
        parent_path: &StandardizedPath,
        target_path: &StandardizedPath,
    ) -> Option<&mut FileTreeEntryState> {
        if self.state_map.contains_child(parent_path, target_path) {
            return self.state_map.get_mut(target_path);
        }

        // Child not found, create new directory entry
        let new_entry = FileTreeEntryState::Directory(FileTreeDirectoryEntryState {
            path: Arc::new(target_path.clone()),
            ignored: false,
            loaded: false,
        });

        self.state_map
            .insert_child(Arc::new(parent_path.clone()), new_entry);
        self.state_map.get_mut(target_path)
    }

    pub fn find_parent_directory(&self, path: &StandardizedPath) -> Option<Arc<StandardizedPath>> {
        self.state_map.parent_directory(path)
    }

    pub fn find_or_insert_child(
        &mut self,
        parent_path: &StandardizedPath,
        child_path: &std::path::Path,
    ) -> Option<Arc<StandardizedPath>> {
        let std_child = StandardizedPath::try_from_local(child_path).ok()?;
        if self.state_map.contains_child(parent_path, &std_child) {
            return Some(Arc::new(std_child));
        }

        let child_arc = Arc::new(std_child);
        let new_entry = if child_path.is_dir() {
            FileTreeEntryState::Directory(FileTreeDirectoryEntryState {
                path: child_arc.clone(),
                loaded: false,
                ignored: false,
            })
        } else if child_path.is_file() {
            FileTreeEntryState::File(FileTreeFileMetadata {
                path: child_arc.clone(),
                file_id: FileId::new(),
                extension: child_arc.extension().map(|s| s.to_owned()),
                ignored: false,
            })
        } else {
            return None;
        };

        self.state_map
            .insert_child(Arc::new(parent_path.clone()), new_entry)
    }

    pub fn insert_child_state(
        &mut self,
        parent_path: &StandardizedPath,
        child_state: FileTreeEntryState,
    ) -> Option<Arc<StandardizedPath>> {
        self.state_map
            .insert_child(Arc::new(parent_path.clone()), child_state)
    }

    /// Ensures all ancestor directories between root and `target_parent`
    /// exist in the tree, creating unloaded directory entries as needed.
    ///
    /// This is essential for handling filesystem events that reference files deep
    /// in directory hierarchies where intermediate directories might not exist in our
    /// in-memory tree yet.
    pub fn ensure_parent_directories_exist(&mut self, target_parent: &StandardizedPath) {
        let root_directory = self.root_directory();

        // Validate that target_parent is indeed under root_entry
        if !target_parent.starts_with(root_directory) {
            return;
        }

        let Some(FileTreeEntryState::Directory(root_directory)) = self.get(root_directory).cloned()
        else {
            return;
        };

        // Get all ancestors between target parent and root (exclusive of root, inclusive of target)
        let ancestors: Vec<_> = target_parent
            .ancestors()
            .take_while(|ancestor| *ancestor != *root_directory.path.as_ref())
            .collect();

        // Create directories from root to target parent using find_or_insert_directory
        let mut current_parent = root_directory;
        for ancestor in ancestors.iter().rev() {
            match self.find_or_insert_directory(&current_parent.path, ancestor) {
                Some(FileTreeEntryState::Directory(dir)) => {
                    current_parent = dir.clone();
                }
                Some(FileTreeEntryState::File(_)) => {
                    log::warn!("Found file where directory expected: {ancestor:?}");
                    return;
                }
                None => {
                    log::warn!("Failed to create or find directory: {ancestor:?}");
                    return;
                }
            }
        }
    }

    /// Applies a [`RepoMetadataUpdate`] to this file tree entry.
    ///
    /// Removals are processed first, then subtree patches are applied.
    /// This is the core mutation path used by the remote client to apply
    /// incremental updates received from the server.
    pub fn apply_repo_metadata_update(
        &mut self,
        update: &crate::file_tree_update::RepoMetadataUpdate,
    ) {
        // 1. Process removals
        for path in &update.remove_entries {
            self.remove(path);
        }

        // 2. Process subtree patches
        for entry_update in &update.update_entries {
            self.apply_entry_update(entry_update);
        }
    }

    fn apply_entry_update(&mut self, update: &crate::file_tree_update::FileTreeEntryUpdate) {
        use crate::file_tree_update::RepoNodeMetadata;

        // Ensure parent directories exist up to parent_path_to_replace
        self.ensure_parent_directories_exist(&update.parent_path_to_replace);

        // `subtree_metadata` is in depth-first pre-order: each directory
        // appears before its children.  A single pass is sufficient because
        // by the time we encounter a file, its parent directory has already
        // been inserted.  `insert_child_state` also registers the child in
        // `parent_to_child_map`, so no separate wiring step is needed.
        for node in &update.subtree_metadata {
            match node {
                RepoNodeMetadata::Directory(dir) => {
                    let state = FileTreeEntryState::Directory(FileTreeDirectoryEntryState {
                        path: Arc::new(dir.path.clone()),
                        ignored: dir.ignored,
                        loaded: dir.loaded,
                    });
                    if let Some(parent) = self.find_parent_directory(&dir.path) {
                        self.insert_child_state(&parent, state);
                    } else {
                        log::warn!("Could not find parent directory for node during incremental update: {:?}", dir.path);
                    }
                }
                RepoNodeMetadata::File(file) => {
                    // If the file already exists, preserve its FileId and just
                    // update metadata (mirrors the local apply path).
                    if let Some(existing) = self.get_mut(&file.path) {
                        existing.set_ignored(file.ignored);
                    } else {
                        let state = FileTreeEntryState::File(FileTreeFileMetadata {
                            path: Arc::new(file.path.clone()),
                            file_id: FileId::new(),
                            extension: file.extension.clone(),
                            ignored: file.ignored,
                        });
                        if let Some(parent) = self.find_parent_directory(&file.path) {
                            self.insert_child_state(&parent, state);
                        } else {
                            log::warn!("Could not find parent directory for node during incremental update: {:?}", file.path);
                        }
                    }
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
pub enum FileTreeEntryState {
    File(FileTreeFileMetadata),
    Directory(FileTreeDirectoryEntryState),
}

impl FileTreeEntryState {
    fn as_directory(&self) -> Option<&FileTreeDirectoryEntryState> {
        match self {
            FileTreeEntryState::File(_) => None,
            FileTreeEntryState::Directory(directory) => Some(directory),
        }
    }
}

#[derive(Debug, Clone)]
pub struct FileTreeFileMetadata {
    /// Absolute path to the file.
    pub path: Arc<StandardizedPath>,
    pub file_id: FileId,
    pub extension: Option<String>,
    pub ignored: bool,
}

impl From<FileMetadata> for FileTreeFileMetadata {
    fn from(value: FileMetadata) -> Self {
        Self {
            path: Arc::new(value.path),
            file_id: value.file_id,
            extension: value.extension.clone(),
            ignored: value.ignored,
        }
    }
}

impl FileTreeEntryState {
    pub fn set_ignored(&mut self, ignored: bool) {
        match self {
            Self::File(file) => file.ignored = ignored,
            Self::Directory(directory) => directory.ignored = ignored,
        }
    }

    pub fn ignored(&self) -> bool {
        match self {
            FileTreeEntryState::File(f) => f.ignored,
            FileTreeEntryState::Directory(d) => d.ignored,
        }
    }

    pub fn path(&self) -> &StandardizedPath {
        match self {
            FileTreeEntryState::File(file) => &file.path,
            FileTreeEntryState::Directory(directory) => &directory.path,
        }
    }

    fn path_arc(&self) -> Arc<StandardizedPath> {
        match self {
            FileTreeEntryState::File(file) => file.path.clone(),
            FileTreeEntryState::Directory(directory) => directory.path.clone(),
        }
    }

    pub fn loaded(&self) -> bool {
        match self {
            FileTreeEntryState::File(_) => true,
            FileTreeEntryState::Directory(d) => d.loaded,
        }
    }
}

#[derive(Debug, Clone)]
pub struct FileTreeDirectoryEntryState {
    /// Absolute path to the directory.
    pub path: Arc<StandardizedPath>,
    pub ignored: bool,
    pub loaded: bool,
}

impl From<Entry> for FileTreeEntry {
    fn from(value: Entry) -> Self {
        let root_path = Arc::new(value.path().clone());
        let state_map = FileTreeMapStore::from(value);

        FileTreeEntry {
            state_map,
            root_path,
        }
    }
}

/// Represents the state of a file tree for a specific repository.
#[derive(Debug, Clone)]
pub struct FileTreeState {
    /// The entry representing the file tree structure.
    pub entry: FileTreeEntry,
    /// Gitignore rules applicable to this repository.
    pub gitignores: Vec<Gitignore>,

    /// Handle to the backing repository (None for lazily-loaded standalone paths).
    #[expect(unused)]
    repository: Option<ModelHandle<Repository>>,
}

impl FileTreeState {
    /// Creates a new FileTreeState.
    pub fn new(
        entry: Entry,
        gitignores: Vec<Gitignore>,
        repository: Option<ModelHandle<Repository>>,
    ) -> Self {
        Self {
            entry: entry.into(),
            gitignores,
            repository,
        }
    }

    /// Creates a new FileTreeState for a lazily-loaded standalone path.
    pub fn new_lazy_loaded(entry: Entry) -> Self {
        Self {
            entry: entry.into(),
            gitignores: vec![],
            repository: None,
        }
    }

    /// Creates a new FileTreeState from a pre-built [`FileTreeEntry`].
    ///
    /// Used by the remote model where the entry is constructed via
    /// `apply_repo_metadata_update` rather than from a local `Entry`.
    pub fn from_file_tree_entry(entry: FileTreeEntry) -> Self {
        Self {
            entry,
            gitignores: vec![],
            repository: None,
        }
    }
}

#[cfg(test)]
#[path = "file_tree_store_tests.rs"]
mod tests;
