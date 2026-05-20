use crate::index::{
    THREADPOOL, {DirectoryEntry, Entry, FileMetadata},
};
use anyhow::anyhow;

use chrono::{DateTime, Utc};
use itertools::Itertools;
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use repo_metadata::entry::is_file_parsable;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use warp_util::standardized_path::StandardizedPath;

use std::{
    collections::{HashMap, HashSet},
    ops::Range,
    path::{Path, PathBuf},
};
use string_offset::ByteOffset;

use crate::index::full_source_code_embedding::{
    chunker::chunk_code,
    fragment_metadata::{FragmentMetadata, LeafToFragmentMetadataUpdates},
    Error,
};

use super::{
    hash::MerkleHash,
    serialized_tree::{SerializedFilesystemInfo, SerializedMerkleNode},
    tree::UpdateFileResult,
    ContentHash, DirEntryOrFragment, NodeHash,
};

/// ID that uniquely identifies a node in the merkle tree. It contains the node type
/// as well as metadata that distinguishes nodes of the same type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum NodeId {
    /// A file node that contains fragment children
    File {
        absolute_path: PathBuf,
        file_size: usize,
        fs_modified_time: DateTime<Utc>,
        file_contents_hash: String,
    },
    /// A directory node that contains file and directory children
    Directory { absolute_path: PathBuf },
    /// A leaf node representing a code fragment
    Fragment {
        absolute_path: PathBuf,
        content_range: Range<ByteOffset>,
    },
}

impl NodeId {
    fn absolute_path(&self) -> &PathBuf {
        match self {
            Self::Directory { absolute_path } => absolute_path,
            Self::File { absolute_path, .. } => absolute_path,
            Self::Fragment { absolute_path, .. } => absolute_path,
        }
    }
}

/// A given node in the [`MerkleTree`].
#[derive(Debug)]
pub(super) struct MerkleNode {
    /// The hash for the current node of the Merkle tree.
    hash: MerkleHash,
    /// The children of this merkle node.
    children: Vec<MerkleNode>,
    /// The ID of this node.
    node_id: NodeId,
}

impl MerkleNode {
    pub(super) fn new(
        entry: DirEntryOrFragment<'_>,
    ) -> Result<(MerkleNode, LeafToFragmentMetadataUpdates), Error> {
        match entry {
            DirEntryOrFragment::Entry(Entry::File(file)) => {
                let local_path = file.path.to_local_path_lossy();
                if !is_file_parsable(&local_path)? {
                    return Err(Error::FileSizeExceeded);
                }
                let (file_size, fs_modified_time) = match std::fs::metadata(&local_path) {
                    Ok(metadata) => {
                        let file_size = metadata.len() as usize;
                        if let Ok(fs_modified_time) = metadata.modified() {
                            // Convert the SystemTime to DateTime<Utc>
                            let fs_modified_time = fs_modified_time.into();

                            (file_size, fs_modified_time)
                        } else {
                            log::warn!("Failed to get modified time for file {}", file.path);
                            return Err(Error::FailedToGetMetadata(local_path));
                        }
                    }
                    Err(_) => {
                        log::warn!("Failed to get metadata for file {}", file.path);
                        return Err(Error::FailedToGetMetadata(local_path));
                    }
                };

                let file_contents = std::fs::read_to_string(&local_path)?;

                // Compute SHA-256 hash of the file contents
                let mut hasher = Sha256::new();
                hasher.update(file_contents.as_bytes());
                let file_contents_hash = format!("{:x}", hasher.finalize());

                let fragments = chunk_code(&file_contents, &local_path);

                let (children, mapping_updates): (Vec<_>, LeafToFragmentMetadataUpdates) =
                    fragments
                        .into_iter()
                        .filter_map(|fragment| {
                            Self::new(DirEntryOrFragment::Fragment(fragment)).ok()
                        })
                        .unzip();
                if children.is_empty() {
                    log::debug!(
                        "Found empty file {} when generating the merkle tree",
                        file.path
                    );
                    return Err(Error::EmptyNodeContent);
                }

                // Create a hash from all of the fragments of the file. We actively _do not_ sort here as the fragments
                // are an ordered function of the file content.
                let hash = MerkleHash::from_hashes(children.iter().map(|child| &child.hash));

                // Add a small delay after file processing to reduce CPU spikes
                // during large repository indexing, allowing other work to continue
                std::thread::sleep(std::time::Duration::from_millis(5));

                Ok((
                    MerkleNode {
                        hash,
                        children,
                        node_id: NodeId::File {
                            absolute_path: file.path.to_local_path_lossy(),
                            file_size,
                            fs_modified_time,
                            file_contents_hash,
                        },
                    },
                    mapping_updates,
                ))
            }
            DirEntryOrFragment::Entry(Entry::Directory(directory)) => {
                let Some(pool) = THREADPOOL.as_ref() else {
                    return Err(anyhow!("No threadpool exists for outline generation.").into());
                };

                let result = pool.install(|| {
                    directory
                        .children
                        .into_par_iter()
                        .filter_map(|node| Self::new(DirEntryOrFragment::Entry(node)).ok())
                        .collect::<Vec<_>>()
                });

                let (mut children, mapping_updates): (Vec<_>, LeafToFragmentMetadataUpdates) =
                    result.into_iter().unzip();

                if children.is_empty() {
                    return Err(Error::EmptyNodeContent);
                }

                // Sort the hashes to ensure we have consistent ordering for all files in the directory. We don't want a new
                // hash if the `DirEntry`s are the same, but returned in a different order.
                children.sort_unstable_by(|a, b| a.hash.cmp(&b.hash));

                let hash = MerkleHash::from_hashes(children.iter().map(|child| &child.hash));
                Ok((
                    MerkleNode {
                        hash,
                        children,
                        node_id: NodeId::Directory {
                            absolute_path: directory.path.to_local_path_lossy(),
                        },
                    },
                    mapping_updates,
                ))
            }
            DirEntryOrFragment::Fragment(fragment) => {
                if fragment.content.is_empty() {
                    return Err(Error::EmptyNodeContent);
                }
                let hash = MerkleHash::from_fragment(&fragment);
                let fragment_metadata = FragmentMetadata::from(&fragment);

                let mut leaf_node_to_fragment_updates = LeafToFragmentMetadataUpdates::empty();
                leaf_node_to_fragment_updates
                    .to_insert
                    .insert(hash.clone(), vec![fragment_metadata]);

                Ok((
                    MerkleNode {
                        hash,
                        children: vec![],
                        node_id: NodeId::Fragment {
                            absolute_path: fragment.file_path.to_path_buf(),
                            content_range: fragment.start_byte_index..fragment.end_byte_index,
                        },
                    },
                    leaf_node_to_fragment_updates,
                ))
            }
        }
    }

    pub(super) fn from_serialized(
        serialized_node: SerializedMerkleNode,
        parent_path: &Path,
    ) -> anyhow::Result<(MerkleNode, LeafToFragmentMetadataUpdates)> {
        let hash = serialized_node.hash();

        let mut children = vec![];
        let mut leaf_node_to_fragment_updates = LeafToFragmentMetadataUpdates::empty();

        let node_id = match serialized_node.fs_info {
            SerializedFilesystemInfo::Directory { absolute_path } => {
                NodeId::Directory { absolute_path }
            }
            SerializedFilesystemInfo::File {
                absolute_path,
                file_size,
                fs_modified_time,
                file_contents_hash,
            } => NodeId::File {
                absolute_path,
                file_size,
                fs_modified_time,
                file_contents_hash,
            },
            SerializedFilesystemInfo::Fragment { location } => {
                let file_path = parent_path.to_path_buf();
                leaf_node_to_fragment_updates
                    .to_insert
                    .entry(hash.as_ref().clone())
                    .or_default()
                    .push(FragmentMetadata {
                        absolute_path: file_path.clone(),
                        location: (&location).into(),
                    });

                NodeId::Fragment {
                    absolute_path: file_path.clone(),
                    content_range: location.byte_range,
                }
            }
        };

        let absolute_path = node_id.absolute_path();

        for child in serialized_node.children {
            let (child_node, new_fragments) = Self::from_serialized(child, absolute_path)?;
            leaf_node_to_fragment_updates.merge(new_fragments);
            children.push(child_node);
        }

        Ok((
            MerkleNode {
                hash: hash.as_ref().clone(),
                children,
                node_id,
            },
            leaf_node_to_fragment_updates,
        ))
    }

    // Recompute the hash for a given node. Note that for directories we expect the children to be sorted beforehand.
    fn recompute_hash(&mut self) {
        match &self.node_id {
            NodeId::Directory { .. } | NodeId::File { .. } => {
                self.hash = MerkleHash::from_hashes(self.children.iter().map(|child| &child.hash));
            }
            NodeId::Fragment { .. } => {
                log::error!("Shouldn't need to recompute hash for fragments");
            }
        }
    }

    /// Remove a set of target file paths from this MerkleNode.
    pub(super) fn remove_files(
        &mut self,
        paths: &mut HashSet<PathBuf>,
        node_masks: &mut NodeMask,
        node_to_fragment_updates: &mut LeafToFragmentMetadataUpdates,
    ) -> UpdateFileResult {
        match &self.node_id {
            // Only visit a directory if it is the ancestor of the target path.
            NodeId::Directory { absolute_path } => {
                let mut paths_under_directory = filter_paths_under_directory(paths, absolute_path);

                if paths_under_directory.is_empty() {
                    return UpdateFileResult::NoChange;
                }

                // Track the indices that need to be removed.
                let mut removal_idx = vec![];
                let mut updated_idx = HashMap::new();
                for (i, child) in self.children.iter_mut().enumerate() {
                    let mut node = NodeMask::new(i);
                    match child.remove_files(
                        &mut paths_under_directory,
                        &mut node,
                        node_to_fragment_updates,
                    ) {
                        // If the node should be removed, remove it from the children.
                        UpdateFileResult::Deleted => {
                            removal_idx.push(i);
                        }
                        // If the node is updated, push the current index to node paths.
                        UpdateFileResult::Updated => {
                            updated_idx.insert(i - removal_idx.len(), node);
                        }
                        UpdateFileResult::NoChange => (),
                    };

                    if paths_under_directory.is_empty() {
                        break;
                    }
                }

                for idx in removal_idx.into_iter().rev() {
                    self.children.remove(idx);
                }

                // If there is no more children to this merkle node, delete the current node.
                if self.children.is_empty() {
                    return UpdateFileResult::Deleted;
                }

                // We need to rebuild the children since the order of the children must be strictly sorted by their
                // merkle hash.
                let new_children_with_idx = self
                    .children
                    .drain(..)
                    .enumerate()
                    .sorted_by(|(_, a), (_, b)| a.hash.cmp(&b.hash));

                for (new_idx, (old_idx, child)) in new_children_with_idx.enumerate() {
                    if let Some(mut node) = updated_idx.remove(&old_idx) {
                        node.index = new_idx;
                        node_masks.add_child(node);
                    }
                    self.children.push(child);
                }

                if !updated_idx.is_empty() {
                    log::error!("Updated index should be empty after an upsert request!");
                }

                // Otherwise, update the cache.
                self.recompute_hash();
                UpdateFileResult::Updated
            }
            // Only visit a file if it matches the target path.
            NodeId::File { absolute_path, .. } if paths.remove(absolute_path) => {
                node_to_fragment_updates.to_remove.insert(
                    absolute_path.to_path_buf(),
                    self.child_hashes().cloned().collect_vec(),
                );
                UpdateFileResult::Deleted
            }
            _ => UpdateFileResult::NoChange,
        }
    }

    /// Update / insert a batch target file paths to this MerkleNode. Return whether the current node is updated.
    pub(super) fn upsert_files(
        &mut self,
        paths: &mut HashSet<PathBuf>,
        node_masks: &mut NodeMask,
        leaf_node_to_fragment_updates: &mut LeafToFragmentMetadataUpdates,
    ) -> UpdateFileResult {
        match &self.node_id {
            NodeId::Directory { absolute_path } => {
                let mut paths_under_directory = filter_paths_under_directory(paths, absolute_path);

                if paths_under_directory.is_empty() {
                    return UpdateFileResult::NoChange;
                }

                let mut updated_idx = HashMap::new();
                let mut removal_idx = vec![];

                for (i, child) in self.children.iter_mut().enumerate() {
                    let mut node = NodeMask::new(i);
                    let updated = child.upsert_files(
                        &mut paths_under_directory,
                        &mut node,
                        leaf_node_to_fragment_updates,
                    );

                    // Only update node_masks if the child is updated.
                    match updated {
                        UpdateFileResult::Deleted => removal_idx.push(i),
                        UpdateFileResult::Updated => {
                            updated_idx.insert(i - removal_idx.len(), node);
                        }
                        UpdateFileResult::NoChange => (),
                    }

                    // There is no more things to update. We could break early.
                    if paths_under_directory.is_empty() {
                        break;
                    }
                }

                for idx in removal_idx.into_iter().rev() {
                    self.children.remove(idx);
                }

                // If there is no more children to this merkle node and we are not creating new nodes, delete the current node.
                if self.children.is_empty() && paths_under_directory.is_empty() {
                    return UpdateFileResult::Deleted;
                }

                let mut entries_to_create = Vec::new();

                // If none of the existing child could be updated these, we need to create new nodes.
                // The algorithm works as below:
                // 1) Convert the to-be-inserted paths into a chain of ancestors (e.g. a/b/c -> [a, a/b, a/b/c]).
                // 2) Dedupe the ancestors using a mapping of path -> directory entry.
                // 3) Attach each directory node to its parent node.
                // 4) Add the root-level directory nodes to entries_to_create.
                //
                // Note that for file nodes, we add them directly to entries_to_create if it is on the root level and
                // to its corresponding parent node otherwise.
                let mut created_dirs: HashMap<PathBuf, DirectoryEntry> = HashMap::new();

                // First pass: Create all directory entries with empty children lists
                for path in paths_under_directory {
                    // Skip upserting full or non-existent directories.
                    if path.is_dir() || !path.exists() {
                        continue;
                    }

                    // Get all parent directories that need to be created
                    let mut ancestors = Vec::new();
                    let mut current = path.clone();

                    // Start from the path's parent and work up to but not including absolute_path
                    while let Some(parent) = current.parent() {
                        current = parent.to_path_buf();

                        if current == *absolute_path {
                            break;
                        }

                        ancestors.push(current.clone());
                    }

                    if current != *absolute_path {
                        log::warn!("Path should match absolute path before None");
                        continue;
                    }

                    // Process ancestors from deepest to shallowest (reverse order)
                    ancestors.reverse();

                    // Create intermediate directories if they don't exist yet
                    for ancestor in ancestors {
                        created_dirs
                            .entry(ancestor.clone())
                            .or_insert_with(|| DirectoryEntry {
                                path: StandardizedPath::try_from_local(&ancestor)
                                    .expect("ancestor paths are always absolute"),
                                children: Vec::new(),
                                ignored: false,
                                loaded: false,
                            });
                    }

                    // Add the file entry to its parent directory's children list
                    if let Some(parent_path) = path.parent() {
                        if parent_path != absolute_path {
                            if let Some(parent_dir) = created_dirs.get_mut(parent_path) {
                                parent_dir
                                    .children
                                    .push(Entry::File(FileMetadata::new(path, false)));
                            }
                        } else {
                            entries_to_create.push(Entry::File(FileMetadata::new(path, false)));
                        }
                    }
                }

                // Second pass: Sort directories by depth (deepest first) and establish relationships from bottom up
                let mut dir_paths: Vec<PathBuf> = created_dirs.keys().cloned().collect();
                // Sort by component count in reverse order - deeper paths first
                dir_paths.sort_by_key(|p| std::cmp::Reverse(p.components().count()));

                for dir_path in dir_paths {
                    if let Some(parent_path) = dir_path.parent() {
                        if let Some(child_dir) = created_dirs.remove(&dir_path) {
                            // Skip if parent is the root directory
                            if parent_path != absolute_path {
                                if let Some(parent_dir) = created_dirs.get_mut(parent_path) {
                                    // Now we have the completed child directory with all its children
                                    parent_dir.children.push(Entry::Directory(child_dir));
                                }
                            } else {
                                entries_to_create.push(Entry::Directory(child_dir));
                            }
                        }
                    }
                }

                for entry in entries_to_create {
                    let res = MerkleNode::new(DirEntryOrFragment::Entry(entry));
                    let (child, mapping_updates) = match res {
                        Ok((child, mapping)) => (child, mapping),
                        // When encountering a node construction error, instead of early returning and interrupting the rest of the update,
                        // consider it a skippable error.
                        // TODO: We should capture and log these errors in the telemetry.
                        Err(e) => {
                            log::debug!("Failed to create new node for update: {e:#}");
                            continue;
                        }
                    };

                    self.children.push(child);
                    leaf_node_to_fragment_updates.merge(mapping_updates);
                    updated_idx.insert(self.children.len() - 1, NodeMask::new(0));
                }

                // We need to rebuild the children since the order of the children must be strictly sorted by their
                // merkle hash.
                let new_children_with_idx = self
                    .children
                    .drain(..)
                    .enumerate()
                    .sorted_by(|(_, a), (_, b)| a.hash.cmp(&b.hash));

                for (new_idx, (old_idx, child)) in new_children_with_idx.enumerate() {
                    if let Some(mut node) = updated_idx.remove(&old_idx) {
                        node.index = new_idx;
                        node_masks.add_child(node);
                    }
                    self.children.push(child);
                }

                if !updated_idx.is_empty() {
                    log::error!("Updated index should be empty after an upsert request!");
                }
                self.recompute_hash();
                UpdateFileResult::Updated
            }
            // For files, only a single path can match at a single time.
            NodeId::File { absolute_path, .. } if paths.remove(absolute_path) => {
                leaf_node_to_fragment_updates.to_remove.insert(
                    absolute_path.clone(),
                    self.child_hashes().cloned().collect_vec(),
                );

                if !absolute_path.exists() {
                    return UpdateFileResult::Deleted;
                }

                let (new_node, mapping_update) = match MerkleNode::new(DirEntryOrFragment::Entry(
                    Entry::File(FileMetadata::new(absolute_path.clone(), false)),
                )) {
                    Ok(res) => res,
                    // If we run into a file permission error / empty node / exceeded max file limit, delete the node since we can't
                    // determine what's the updated content.
                    Err(_) => return UpdateFileResult::Deleted,
                };
                leaf_node_to_fragment_updates.merge(mapping_update);
                self.children = new_node.children;
                self.hash = new_node.hash;
                self.node_id = new_node.node_id;
                UpdateFileResult::Updated
            }
            _ => UpdateFileResult::NoChange,
        }
    }

    pub(super) fn absolute_path(&self) -> &Path {
        self.node_id.absolute_path()
    }

    fn child_hashes(&self) -> impl Iterator<Item = &MerkleHash> {
        self.children.iter().map(|child| &child.hash)
    }

    pub(super) fn children(&self) -> impl Iterator<Item = &MerkleNode> {
        self.children.iter()
    }

    pub(super) fn count_children(&self) -> usize {
        self.children.len()
    }

    pub(super) fn child_at(&self, index: usize) -> &MerkleNode {
        &self.children[index]
    }

    pub(super) fn hash(&self) -> &MerkleHash {
        &self.hash
    }

    pub(super) fn node_id(&self) -> &NodeId {
        &self.node_id
    }

    pub(super) fn is_fragment(&self) -> bool {
        matches!(self.node_id, NodeId::Fragment { .. })
    }
}

fn filter_paths_under_directory(
    paths: &mut HashSet<PathBuf>,
    curr_path: &PathBuf,
) -> HashSet<PathBuf> {
    let mut paths_under_directory = HashSet::new();

    // Construct and filter out paths that are under the current directory.
    for path in paths.iter() {
        if path.starts_with(curr_path) {
            paths_under_directory.insert(path.clone());
        }
    }

    for filter_path in paths_under_directory.iter() {
        paths.remove(filter_path);
    }

    paths_under_directory
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct NodeLens<'a> {
    node: &'a MerkleNode,
}

impl<'a> NodeLens<'a> {
    pub(super) fn new(node: &'a MerkleNode) -> Self {
        Self { node }
    }

    pub fn children(&self) -> impl Iterator<Item = NodeLens<'a>> {
        self.node.children().map(|node| NodeLens { node })
    }

    pub fn hash(&self) -> NodeHash {
        NodeHash::new(self.node.hash().clone())
    }

    pub fn content_hash(&self) -> Option<ContentHash> {
        self.is_leaf()
            .then(|| ContentHash::new(self.node.hash().clone()))
    }

    pub fn is_leaf(&self) -> bool {
        self.node.is_fragment()
    }

    pub fn path(&self) -> &Path {
        self.node.absolute_path()
    }

    pub(crate) fn node_id(&self) -> &NodeId {
        self.node.node_id()
    }
}

#[derive(Default)]
pub(super) enum ChildrenPath {
    #[default]
    All,
    SpecificChildren(Vec<NodeMask>),
}

/// A path from the leaf changed file node to the root.
///
/// Each NodeMask corresponds to a node in the tree. It contains the index of the node
/// it is referencing in the parent node's children.
///
/// Note a NodeMask strictly couples with a snapshot of a Merkle tree. If the tree
/// has been edited afterwards, the NodeMask will no longer be valid.
#[derive(Default)]
pub(super) struct NodeMask {
    pub(super) index: usize,
    pub(super) children: ChildrenPath,
}

impl NodeMask {
    pub(super) fn new(index: usize) -> Self {
        Self {
            index,
            children: Default::default(),
        }
    }

    pub(super) fn add_child(&mut self, node: Self) {
        match &mut self.children {
            ChildrenPath::All => {
                self.children = ChildrenPath::SpecificChildren(vec![node]);
            }
            ChildrenPath::SpecificChildren(children) => children.push(node),
        }
    }
}

#[cfg(test)]
#[path = "node_tests.rs"]
mod tests;
