use std::collections::{HashMap, HashSet};
use std::iter;
use std::sync::Arc;

use ignore::gitignore::Gitignore;
use warp_util::standardized_path::StandardizedPath;

use crate::file_tree_store::{FileTreeDirectoryEntryState, FileTreeEntry, FileTreeEntryState};
use crate::{BuildTreeError, DirectoryEntry, Entry};

#[derive(Debug, Clone)]
pub(super) struct FileTreeMapStore {
    state_map: HashMap<Arc<StandardizedPath>, FileTreeEntryState>,
    parent_to_child_map: HashMap<Arc<StandardizedPath>, HashSet<Arc<StandardizedPath>>>,
}

impl From<Entry> for FileTreeMapStore {
    fn from(value: Entry) -> Self {
        // Pre-count nodes so we can allocate HashMaps with the right capacity
        // upfront, avoiding repeated rehashing for large directory trees.
        let capacity = value.count_nodes();
        match value {
            Entry::File(file) => Self::new_for_entry(FileTreeEntryState::File(file.into())),
            Entry::Directory(dir) => {
                let dir_path: Arc<StandardizedPath> = Arc::new(dir.path);
                let entry = FileTreeEntryState::Directory(FileTreeDirectoryEntryState {
                    path: dir_path.clone(),
                    ignored: dir.ignored,
                    loaded: dir.loaded,
                });

                let mut file_tree_map = Self::with_capacity_for_entry(entry, capacity);
                for child in dir.children {
                    file_tree_map.append_entry(dir_path.clone(), child);
                }
                file_tree_map
            }
        }
    }
}

impl FileTreeMapStore {
    pub fn new_for_directory(directory_path: Arc<StandardizedPath>) -> Self {
        let directory = FileTreeEntryState::Directory(FileTreeDirectoryEntryState {
            path: directory_path.clone(),
            ignored: false,
            loaded: true,
        });

        let state_map = HashMap::from_iter([(directory_path, directory)]);

        Self {
            state_map,
            parent_to_child_map: Default::default(),
        }
    }

    pub fn new_for_entry(entry: FileTreeEntryState) -> Self {
        let state_map = HashMap::from_iter([(entry.path_arc(), entry)]);

        Self {
            state_map,
            parent_to_child_map: Default::default(),
        }
    }

    /// Creates a new `FileTreeMapStore` with pre-allocated capacity.
    ///
    /// This avoids repeated HashMap rehashing when building large directory
    /// trees, reducing transient memory spikes from capacity-doubling.
    fn with_capacity_for_entry(entry: FileTreeEntryState, capacity: usize) -> Self {
        let mut state_map = HashMap::with_capacity(capacity);
        state_map.insert(entry.path_arc(), entry);

        Self {
            state_map,
            parent_to_child_map: HashMap::with_capacity(capacity / 4),
        }
    }

    fn append_entry(&mut self, parent_path: Arc<StandardizedPath>, entry: Entry) {
        match entry {
            Entry::File(file) => {
                self.insert_child(parent_path, FileTreeEntryState::File(file.into()));
            }
            Entry::Directory(dir) => {
                let directory_path: Arc<StandardizedPath> = Arc::new(dir.path);
                let entry = FileTreeEntryState::Directory(FileTreeDirectoryEntryState {
                    path: directory_path.clone(),
                    ignored: dir.ignored,
                    loaded: dir.loaded,
                });
                self.insert_child(parent_path, entry);
                for child in dir.children {
                    self.append_entry(directory_path.clone(), child);
                }
            }
        }
    }

    pub fn remove(&mut self, path: &StandardizedPath) -> Option<FileTreeEntryState> {
        let removed = self.remove_item_and_parent(path);

        let children = self.parent_to_child_map.remove(path).into_iter().flatten();
        for child in children {
            self.remove(&child);
        }

        removed
    }

    fn remove_item_and_parent(&mut self, path: &StandardizedPath) -> Option<FileTreeEntryState> {
        let removed = self.state_map.remove(path);

        if let Some(parent) = self.parent_directory(path) {
            self.parent_to_child_map
                .entry(parent)
                .or_default()
                .remove(path);
        }

        removed
    }

    /// Returns the parent directory of the current path.
    ///
    /// ## Validation
    /// No validation is done to ensure that the returned path is actually the parent of the child.
    pub fn parent_directory(&self, path: &StandardizedPath) -> Option<Arc<StandardizedPath>> {
        let parent_dir = path.parent()?;
        self.state_map
            .get(&parent_dir)
            .and_then(FileTreeEntryState::as_directory)?;
        Some(Arc::new(parent_dir))
    }

    pub fn children(
        &self,
        path: &StandardizedPath,
    ) -> impl Iterator<Item = &Arc<StandardizedPath>> {
        if self
            .state_map
            .get(path)
            .and_then(FileTreeEntryState::as_directory)
            .is_none()
        {
            return itertools::Either::Left(iter::empty());
        };

        itertools::Either::Right(self.parent_to_child_map.get(path).into_iter().flatten())
    }

    pub fn get(&self, path: &StandardizedPath) -> Option<&FileTreeEntryState> {
        self.state_map.get(path)
    }

    pub fn get_mut(&mut self, path: &StandardizedPath) -> Option<&mut FileTreeEntryState> {
        self.state_map.get_mut(path)
    }

    fn get_as_directory(&self, path: &StandardizedPath) -> Option<&FileTreeDirectoryEntryState> {
        self.state_map
            .get(path)
            .and_then(FileTreeEntryState::as_directory)
    }

    pub fn contains(&self, path: &StandardizedPath) -> bool {
        self.state_map.contains_key(path)
    }

    pub fn contains_child(&self, parent_path: &StandardizedPath, child: &StandardizedPath) -> bool {
        if self
            .state_map
            .get(parent_path)
            .and_then(FileTreeEntryState::as_directory)
            .is_none()
        {
            return false;
        };

        self.parent_to_child_map
            .get(parent_path)
            .is_some_and(|children| children.contains(child))
    }

    pub fn insert_child(
        &mut self,
        parent_path: Arc<StandardizedPath>,
        child: FileTreeEntryState,
    ) -> Option<Arc<StandardizedPath>> {
        self.get_as_directory(&parent_path)?;

        let child_path: Arc<StandardizedPath> = child.path_arc();
        self.state_map.insert(child_path.clone(), child);
        self.parent_to_child_map
            .entry(parent_path)
            .or_default()
            .insert(child_path.clone());

        Some(child_path)
    }

    pub fn load_at_path(
        &mut self,
        path: &StandardizedPath,
        gitignores: &mut Vec<Gitignore>,
    ) -> Result<(), BuildTreeError> {
        let child_path: Arc<StandardizedPath> = Arc::new(path.clone());
        let mut entry = Entry::Directory(DirectoryEntry {
            path: path.clone(),
            children: vec![],
            ignored: false,
            loaded: true,
        });

        entry.load(gitignores)?;
        self.insert_entry_at_path(child_path, entry);
        Ok(())
    }

    pub fn insert_entry_at_path(&mut self, path: Arc<StandardizedPath>, entry: Entry) {
        let child_entry_map = FileTreeEntry::from(entry);
        // Reserve capacity upfront to avoid intermediate rehashes when merging
        // a large subtree into the existing map.
        self.state_map
            .reserve(child_entry_map.state_map.state_map.len());
        self.parent_to_child_map
            .reserve(child_entry_map.state_map.parent_to_child_map.len());
        self.state_map.extend(child_entry_map.state_map.state_map);
        self.parent_to_child_map
            .extend(child_entry_map.state_map.parent_to_child_map);

        // ATODO test this
        if let Some(parent) = self.parent_directory(&path) {
            self.parent_to_child_map
                .entry(parent)
                .or_default()
                .insert(path);
        }
    }

    /// Renames the path in the tree from `path` to `new_path`.
    pub fn rename_path(&mut self, path: &StandardizedPath, new_path: &StandardizedPath) -> bool {
        // 1. Check if old path exists
        let old_path_arc = if let Some((k, _)) = self.state_map.get_key_value(path) {
            k.clone()
        } else {
            return false;
        };

        // 2. Check if new parent exists
        let Some(new_parent_path) = self.parent_directory(new_path) else {
            return false;
        };

        // 3. Remove from old parent's child list and from state map
        let Some(mut item) = self.remove_item_and_parent(path) else {
            return false;
        };

        // 3. Update item path
        let new_path_arc: Arc<StandardizedPath> = Arc::new(new_path.clone());
        match &mut item {
            FileTreeEntryState::File(f) => f.path = new_path_arc.clone(),
            FileTreeEntryState::Directory(d) => d.path = new_path_arc.clone(),
        }
        let is_directory = matches!(item, FileTreeEntryState::Directory(_));

        // 6. Insert into state_map with new key
        self.state_map.insert(new_path_arc.clone(), item);

        // 7. Add to new parent's child list
        self.parent_to_child_map
            .entry(new_parent_path)
            .or_default()
            .insert(new_path_arc.clone());

        // 8. If directory, handle children
        if is_directory {
            // Move children list in parent_to_child_map
            if let Some(children) = self.parent_to_child_map.remove(&old_path_arc) {
                self.parent_to_child_map
                    .insert(new_path_arc.clone(), children.clone());

                // Recursively update paths for all descendants
                for child_path in children {
                    self.update_paths_recursive(child_path, path, new_path);
                }
            }
        }

        true
    }

    fn update_paths_recursive(
        &mut self,
        path: Arc<StandardizedPath>,
        from: &StandardizedPath,
        to: &StandardizedPath,
    ) {
        let Some(relative) = path.strip_prefix(from) else {
            return;
        };
        let new_child_path = Arc::new(to.join(relative));

        // Remove old entry
        if let Some(mut item) = self.remove_item_and_parent(&path) {
            match &mut item {
                FileTreeEntryState::File(f) => f.path = new_child_path.clone(),
                FileTreeEntryState::Directory(d) => d.path = new_child_path.clone(),
            }
            // Insert with new key.
            self.state_map.insert(new_child_path.clone(), item);

            // Update parent_to_child_map if it's a directory.
            if let Some(children) = self.parent_to_child_map.remove(&path) {
                self.parent_to_child_map
                    .insert(new_child_path.clone(), children.clone());
                for child in children {
                    self.update_paths_recursive(child, from, to);
                }
            }

            // Update parent's reference to this child
            if let Some(parent) = new_child_path.parent() {
                let parent_arc = Arc::new(parent);
                self.parent_to_child_map
                    .entry(parent_arc)
                    .and_modify(|children| {
                        children.remove(&path);
                        children.insert(new_child_path.clone());
                    });
            }
        }
    }
}
