//! Serializable incremental file tree update types.
//!
//! These types mirror the protobuf schema for `RepoMetadataUpdate` 1:1,
//! making encode/decode between Rust and proto trivial. They are produced
//! by the server after processing watcher events and consumed by the client
//! to update its [`RemoteRepoMetadataModel`](crate::remote_model::RemoteRepoMetadataModel).

use warp_util::standardized_path::StandardizedPath;

use crate::entry::{DirectoryEntry, Entry, FileMetadata};
/// Describes how a file-tree entry update should be interpreted by consumers.
#[derive(Debug, Clone)]
pub enum MetadataUpdateType {
    /// The updated entry was replaced without a precise metadata delta.
    /// Consumers should refresh any derived state conservatively.
    FullReplace,
    /// The updated entry includes a precise incremental metadata delta.
    IncrementalUpdate(RepoMetadataUpdate),
}

/// Mirrors `RepoMetadataUpdate` proto.
///
/// A batch of incremental changes for a single repository. Removals are
/// processed before additions so that "move" semantics (remove old + add new)
/// work correctly.
#[derive(Debug, Clone)]
pub struct RepoMetadataUpdate {
    /// Which repository this update targets.
    pub repo_path: StandardizedPath,
    /// Paths to remove from the tree.
    pub remove_entries: Vec<StandardizedPath>,
    /// Subtree patches to add or replace in the tree.
    pub update_entries: Vec<FileTreeEntryUpdate>,
}

/// Mirrors `FileTreeEntry` proto.
///
/// Describes a subtree patch rooted at a specific parent directory.
/// Applying this inserts the described nodes under `parent_path_to_replace`.
///
/// Parent→child relationships are not included because they are derived
/// implicitly: each node's parent is determined by its path, and
/// `insert_child_state` registers the child in `parent_to_child_map`
/// during application.
#[derive(Debug, Clone)]
pub struct FileTreeEntryUpdate {
    /// The parent directory whose subtree is being patched.
    pub parent_path_to_replace: StandardizedPath,
    /// Metadata for each node in the subtree.
    /// Directories must appear before their children (depth-first pre-order)
    /// so that parent entries exist when child entries are inserted.
    pub subtree_metadata: Vec<RepoNodeMetadata>,
}

/// Mirrors `RepoNodeMetadata` proto.
#[derive(Debug, Clone)]
pub enum RepoNodeMetadata {
    Directory(DirectoryNodeMetadata),
    File(FileNodeMetadata),
}

/// Mirrors `DirectoryNodeMetadata` proto.
#[derive(Debug, Clone)]
pub struct DirectoryNodeMetadata {
    pub path: StandardizedPath,
    pub ignored: bool,
    pub loaded: bool,
}

/// Mirrors `FileNodeMetadata` proto.
#[derive(Debug, Clone)]
pub struct FileNodeMetadata {
    pub path: StandardizedPath,
    pub extension: Option<String>,
    pub ignored: bool,
}

/// Flattens a recursive [`Entry`] into a `Vec<RepoNodeMetadata>` in
/// depth-first pre-order (directories before their children).
pub fn flatten_entry_metadata(entry: &Entry) -> Vec<RepoNodeMetadata> {
    let mut metadata = Vec::new();
    collect_metadata(entry, &mut metadata);
    metadata
}

fn collect_metadata(entry: &Entry, metadata: &mut Vec<RepoNodeMetadata>) {
    match entry {
        Entry::File(file) => {
            metadata.push(file_metadata_to_node(file));
        }
        Entry::Directory(dir) => {
            collect_directory_metadata(dir, metadata);
        }
    }
}

fn collect_directory_metadata(dir: &DirectoryEntry, metadata: &mut Vec<RepoNodeMetadata>) {
    // Emit metadata for the directory itself before its children.
    metadata.push(RepoNodeMetadata::Directory(DirectoryNodeMetadata {
        path: dir.path.clone(),
        ignored: dir.ignored,
        loaded: dir.loaded,
    }));

    for child in &dir.children {
        collect_metadata(child, metadata);
    }
}

fn file_metadata_to_node(file: &FileMetadata) -> RepoNodeMetadata {
    RepoNodeMetadata::File(FileNodeMetadata {
        path: file.path.clone(),
        extension: file.extension.clone(),
        ignored: file.ignored,
    })
}

#[cfg(test)]
#[path = "file_tree_update_tests.rs"]
mod tests;
