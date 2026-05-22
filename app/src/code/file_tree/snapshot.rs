#![allow(dead_code)]
//! SumTree-based file tree snapshot for efficient lookups and virtualized rendering.
//!
//! This module provides a SumTree-based data model for the file tree view.

#[path = "snapshot/iterator.rs"]
mod iterator;

use std::cmp::Ordering;
use std::ops::AddAssign;
use std::path::Path;
use std::sync::Arc;

use sum_tree::{Edit, KeyedItem, SeekBias, SumTree};

/// Represents a single entry in the file tree.
#[derive(Clone, Debug)]
pub struct FileEntry {
    /// The absolute path to this entry.
    pub path: Arc<Path>,
    /// Whether this is a file or directory.
    pub kind: FileEntryKind,
    /// Whether this entry is ignored by gitignore.
    pub ignored: bool,
    /// For directories: whether the contents have been loaded.
    /// For files: always true.
    pub loaded: bool,
}

impl FileEntry {
    /// Creates a new file entry.
    pub fn file(path: impl Into<Arc<Path>>, ignored: bool) -> Self {
        let path = path.into();
        let extension = path.extension().and_then(|e| e.to_str()).map(Arc::from);
        Self {
            path,
            kind: FileEntryKind::File { extension },
            ignored,
            loaded: true,
        }
    }

    /// Creates a new directory entry.
    pub fn directory(path: impl Into<Arc<Path>>, ignored: bool, loaded: bool) -> Self {
        Self {
            path: path.into(),
            kind: FileEntryKind::Directory,
            ignored,
            loaded,
        }
    }

    /// Returns true if this is a directory.
    pub fn is_dir(&self) -> bool {
        matches!(self.kind, FileEntryKind::Directory)
    }

    /// Returns true if this is a file.
    pub fn is_file(&self) -> bool {
        matches!(self.kind, FileEntryKind::File { .. })
    }

    /// Returns the file extension if this is a file.
    #[cfg(test)]
    pub fn extension(&self) -> Option<&str> {
        match &self.kind {
            FileEntryKind::File { extension } => extension.as_deref(),
            FileEntryKind::Directory => None,
        }
    }
}

/// The kind of file tree entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileEntryKind {
    File { extension: Option<Arc<str>> },
    Directory,
}

/// Summary of file entries for aggregate queries.
#[derive(Clone, Debug)]
pub struct FileEntrySummary {
    /// The maximum (lexicographically last) path in this subtree.
    max_path: Arc<Path>,
    /// Total count of entries in this subtree.
    count: usize,
    /// Count of non-ignored entries in this subtree.
    visible_count: usize,
    /// Count of files (not directories) in this subtree.
    file_count: usize,
    /// Count of non-ignored files in this subtree.
    visible_file_count: usize,
}

impl Default for FileEntrySummary {
    fn default() -> Self {
        Self {
            max_path: Arc::from(Path::new("")),
            count: 0,
            visible_count: 0,
            file_count: 0,
            visible_file_count: 0,
        }
    }
}

impl AddAssign<&FileEntrySummary> for FileEntrySummary {
    fn add_assign(&mut self, rhs: &FileEntrySummary) {
        // Entries are sorted by path, so the rightmost (rhs) summary has the max path.
        self.max_path = rhs.max_path.clone();
        self.count += rhs.count;
        self.visible_count += rhs.visible_count;
        self.file_count += rhs.file_count;
        self.visible_file_count += rhs.visible_file_count;
    }
}

impl sum_tree::Item for FileEntry {
    type Summary = FileEntrySummary;

    fn summary(&self) -> Self::Summary {
        let is_visible = !self.ignored;
        let is_file = self.is_file();

        FileEntrySummary {
            max_path: self.path.clone(),
            count: 1,
            visible_count: if is_visible { 1 } else { 0 },
            file_count: if is_file { 1 } else { 0 },
            visible_file_count: if is_visible && is_file { 1 } else { 0 },
        }
    }
}

/// Newtype for path-based lookups in the SumTree.
///
/// We can't use `Arc<Path>` directly because:
/// 1. Orphan rule: can't impl `sum_tree::Dimension` for external type
/// 2. `Arc<Path>` has no `Default` impl, which SumTree cursors require
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathKey(pub Arc<Path>);

impl Default for PathKey {
    fn default() -> Self {
        Self(Arc::from(Path::new("")))
    }
}

impl PathKey {
    pub fn new(path: impl Into<Arc<Path>>) -> Self {
        Self(path.into())
    }
}

impl Ord for PathKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.cmp(&other.0)
    }
}

impl PartialOrd for PathKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<'a> sum_tree::Dimension<'a, FileEntrySummary> for PathKey {
    fn add_summary(&mut self, summary: &'a FileEntrySummary) {
        self.0 = summary.max_path.clone();
    }
}

impl KeyedItem for FileEntry {
    type Key = PathKey;

    fn key(&self) -> Self::Key {
        PathKey(self.path.clone())
    }
}

/// A snapshot of the file tree stored in a SumTree.
#[derive(Clone, Debug)]
pub struct FileTreeSnapshot {
    /// The root path of this file tree.
    root_path: Arc<Path>,
    /// Entries sorted by path.
    pub(super) entries_by_path: SumTree<FileEntry>,
}

impl FileTreeSnapshot {
    /// Creates an empty snapshot with the given root path.
    pub fn new(root_path: impl Into<Arc<Path>>) -> Self {
        Self {
            root_path: root_path.into(),
            entries_by_path: SumTree::new(),
        }
    }

    /// Creates a snapshot with a root directory entry.
    pub fn with_root(root_path: impl Into<Arc<Path>>, ignored: bool, loaded: bool) -> Self {
        let root_path = root_path.into();
        let mut snapshot = Self::new(root_path.clone());
        snapshot.insert_entry(FileEntry::directory(root_path, ignored, loaded));
        snapshot
    }

    /// Returns the root path of this file tree.
    pub fn root_path(&self) -> &Arc<Path> {
        &self.root_path
    }

    /// Returns the total number of entries.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries_by_path.summary().count
    }

    /// Returns true if there are no entries.
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Looks up an entry by path. O(log n).
    pub fn entry_for_path(&self, path: &Path) -> Option<&FileEntry> {
        let key = PathKey::new(Arc::from(path));
        let mut cursor = self.entries_by_path.cursor::<PathKey, ()>();
        cursor.seek(&key, SeekBias::Left);
        cursor.item().filter(|entry| entry.path.as_ref() == path)
    }

    /// Inserts or updates an entry. O(log n).
    pub fn insert_entry(&mut self, entry: FileEntry) {
        self.entries_by_path.edit(&mut [Edit::Insert(entry)]);
    }

    /// Removes an entry by path. O(log n).
    pub fn remove_entry(&mut self, path: &Path) {
        if let Some(entry) = self.entry_for_path(path).cloned() {
            self.entries_by_path.edit(&mut [Edit::Remove(entry)]);
        }
    }

    /// Returns an iterator over direct children of the given directory path.
    pub fn child_entries<'a>(
        &'a self,
        parent_path: &'a Path,
    ) -> impl Iterator<Item = &'a FileEntry> {
        iterator::ChildEntriesIter::new(self, parent_path)
    }

    /// Checks if the parent directory of the given path is loaded.
    /// Returns true if the parent exists and is loaded, or if the path is the root.
    pub fn is_parent_loaded(&self, path: &Path) -> bool {
        let Some(parent) = path.parent() else {
            // No parent means this is a root-level path
            return true;
        };

        // If parent is the root path, check if it's loaded
        if parent == self.root_path.as_ref() {
            return self
                .entry_for_path(parent)
                .is_some_and(|e| e.is_dir() && e.loaded);
        }

        // Check if parent directory exists and is loaded
        self.entry_for_path(parent)
            .is_some_and(|e| e.is_dir() && e.loaded)
    }

    /// Handles a file/directory being added.
    /// Returns true if the entry was added, false if the parent is not loaded.
    pub fn handle_added(&mut self, path: &Path, is_dir: bool, ignored: bool) -> bool {
        if !self.is_parent_loaded(path) {
            return false;
        }

        let entry = if is_dir {
            FileEntry::directory(Arc::from(path), ignored, false)
        } else {
            FileEntry::file(Arc::from(path), ignored)
        };
        self.insert_entry(entry);
        true
    }

    /// Handles a file/directory being removed.
    /// Returns true if the entry was removed, false if it didn't exist or parent is not loaded.
    pub fn handle_removed(&mut self, path: &Path) -> bool {
        if !self.is_parent_loaded(path) {
            return false;
        }

        if self.entry_for_path(path).is_some() {
            self.remove_entry(path);
            true
        } else {
            false
        }
    }

    /// Renames an entry from old_path to new_path, preserving its properties.
    pub fn rename_entry(&mut self, old_path: &Path, new_path: &Path) {
        let Some(old_entry) = self.entry_for_path(old_path).cloned() else {
            return;
        };

        // Remove the old entry
        self.remove_entry(old_path);

        // Create a new entry at the new path with the same properties
        let new_entry = FileEntry {
            path: Arc::from(new_path),
            kind: old_entry.kind,
            ignored: old_entry.ignored,
            loaded: old_entry.loaded,
        };
        self.insert_entry(new_entry);
    }

    /// Expands a directory by marking it as loaded.
    pub fn expand_directory(&mut self, path: &Path) -> Option<()> {
        let entry = self.entry_for_path(path)?.clone();
        if !entry.is_dir() {
            return None;
        }

        let updated = FileEntry {
            loaded: true,
            ..entry
        };
        self.insert_entry(updated);
        Some(())
    }

    /// Populates a directory with its children from the filesystem.
    /// This scans the directory and adds all immediate children.
    #[cfg(feature = "local_fs")]
    pub fn load_directory_children(
        &mut self,
        dir_path: &Path,
        check_ignored: impl Fn(&Path) -> bool,
    ) -> std::io::Result<()> {
        use std::fs;

        // Mark directory as loaded
        self.expand_directory(dir_path);

        // Read directory contents
        for entry in fs::read_dir(dir_path)? {
            let entry = entry?;
            let path = entry.path();
            let is_dir = entry.file_type()?.is_dir();
            let ignored = check_ignored(&path);

            let file_entry = if is_dir {
                FileEntry::directory(Arc::from(path.as_path()), ignored, false)
            } else {
                FileEntry::file(Arc::from(path.as_path()), ignored)
            };
            self.insert_entry(file_entry);
        }

        Ok(())
    }
}

#[cfg(test)]
#[path = "snapshot_tests.rs"]
mod tests;
