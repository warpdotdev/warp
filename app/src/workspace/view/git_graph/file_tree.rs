//! Turns a commit's flat list of [`ChangedFile`]s into a collapsible directory
//! tree for the detail area, without touching the underlying data: file leaves
//! keep their original index into `detail.files`, so opening a diff and the
//! per-file mouse-state lookup stay index-based and unchanged.
//!
//! Pure logic only (no rendering): [`build_file_rows`] takes the flat files plus
//! the set of collapsed directory paths and returns the rows to display, in
//! order, with the collapsed directories' subtrees omitted.

use std::collections::{BTreeMap, HashSet};

use super::data::ChangedFile;

/// One row of the rendered file tree, already flattened into display order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FileRow {
    /// A directory row: clicking it toggles `path` in the collapsed set.
    Dir {
        /// Full path from the tree root (e.g. `src/foo`), used as the collapse
        /// key and the toggle action's payload.
        path: String,
        /// Last path segment, shown as the row label.
        name: String,
        depth: usize,
    },
    /// A file leaf: `index` is its position in the original `detail.files`, so a
    /// click still dispatches `OpenFileDiff(index)` unchanged.
    File {
        index: usize,
        /// Basename only — the directory is conveyed by the tree structure.
        name: String,
        depth: usize,
    },
}

/// A node in the intermediate tree built from the flat file paths. `BTreeMap`
/// keys give a stable alphabetical order per level for free; emitting subdirs
/// before files yields the conventional "directories first" tree ordering.
#[derive(Default)]
struct DirNode {
    subdirs: BTreeMap<String, DirNode>,
    /// Basename -> index into the original `detail.files`.
    files: BTreeMap<String, usize>,
}

/// Builds the ordered, collapse-aware list of rows for the file tree.
///
/// Ordering: at every level, directories (alphabetical) come before files
/// (alphabetical). A directory whose path is in `collapsed` is still shown, but
/// its children are omitted.
pub(crate) fn build_file_rows(files: &[ChangedFile], collapsed: &HashSet<String>) -> Vec<FileRow> {
    let mut root = DirNode::default();
    for (index, file) in files.iter().enumerate() {
        let mut segments: Vec<&str> = file.path.split('/').collect();
        // The last segment is the basename; everything before it is a directory
        // chain. A root-level file has no directory segments.
        let basename = segments.pop().unwrap_or("");
        let mut node = &mut root;
        for segment in segments {
            node = node.subdirs.entry(segment.to_string()).or_default();
        }
        node.files.insert(basename.to_string(), index);
    }

    let mut rows = Vec::new();
    emit(&root, String::new(), 0, collapsed, &mut rows);
    rows
}

/// Depth-first flatten: directories (with their subtrees, unless collapsed) then
/// files, matching `BTreeMap`'s alphabetical key order at each level.
fn emit(
    node: &DirNode,
    prefix: String,
    depth: usize,
    collapsed: &HashSet<String>,
    rows: &mut Vec<FileRow>,
) {
    for (name, sub) in &node.subdirs {
        let path = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        rows.push(FileRow::Dir {
            path: path.clone(),
            name: name.clone(),
            depth,
        });
        if !collapsed.contains(&path) {
            emit(sub, path, depth + 1, collapsed, rows);
        }
    }
    for (name, index) in &node.files {
        rows.push(FileRow::File {
            index: *index,
            name: name.clone(),
            depth,
        });
    }
}

/// Every directory path in the tree (full paths like `src/foo`), used to
/// pre-build a hover mouse-state per directory row when a commit loads.
pub(crate) fn all_dir_paths(files: &[ChangedFile]) -> Vec<String> {
    let mut paths = HashSet::new();
    for file in files {
        let segments: Vec<&str> = file.path.split('/').collect();
        // Drop the basename; accumulate each directory prefix.
        let mut prefix = String::new();
        for segment in &segments[..segments.len().saturating_sub(1)] {
            prefix = if prefix.is_empty() {
                segment.to_string()
            } else {
                format!("{prefix}/{segment}")
            };
            paths.insert(prefix.clone());
        }
    }
    paths.into_iter().collect()
}

#[cfg(test)]
#[path = "file_tree_tests.rs"]
mod tests;
