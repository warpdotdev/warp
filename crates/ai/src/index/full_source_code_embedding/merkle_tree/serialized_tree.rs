use std::{
    ops::Range,
    path::{Path, PathBuf},
};

use super::{hash::MerkleHash, node::NodeId, MerkleTree, NodeHash, NodeLens};

use crate::index::full_source_code_embedding::fragment_metadata::{
    FragmentLocation, LeafToFragmentMetadata,
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use string_offset::ByteOffset;

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct SerializedCodebaseIndex {
    tree: SerializedMerkleTree,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct SerializedMerkleTree {
    root: SerializedMerkleNode,
}

/// A given node in the [`MerkleTree`].
#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct SerializedMerkleNode {
    /// The hash for the current node of the Merkle tree.
    pub hash: MerkleHash,
    /// The children of this merkle node.
    pub children: Vec<SerializedMerkleNode>,
    /// Node-specific details.
    pub fs_info: SerializedFilesystemInfo,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct SerializedFragmentLocation {
    /// Start line number (inclusive).
    pub start_line: usize,
    /// End line number (inclusive).
    pub end_line: usize,
    /// The range of byte indices into the original source string for this fragment.
    pub byte_range: Range<ByteOffset>,
}

impl From<&FragmentLocation> for SerializedFragmentLocation {
    fn from(location: &FragmentLocation) -> Self {
        Self {
            start_line: location.start_line,
            end_line: location.end_line,
            byte_range: location.byte_range.clone(),
        }
    }
}

impl From<&SerializedFragmentLocation> for FragmentLocation {
    fn from(serialized: &SerializedFragmentLocation) -> Self {
        Self {
            start_line: serialized.start_line,
            end_line: serialized.end_line,
            byte_range: serialized.byte_range.clone(),
        }
    }
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub(super) enum SerializedFilesystemInfo {
    Directory {
        absolute_path: PathBuf,
    },
    File {
        /// The path of this node in the filesystem.
        absolute_path: PathBuf,
        /// File size in bytes.
        file_size: usize,
        /// Time the file was last modified, according to the filesystem.
        fs_modified_time: DateTime<Utc>,
        /// SHA-256 hash of the file contents.
        file_contents_hash: String,
    },
    Fragment {
        /// Fragment location within the file.
        location: SerializedFragmentLocation,
    },
}

impl SerializedCodebaseIndex {
    pub fn new(
        tree: &MerkleTree,
        leaf_node_to_fragment_metadata: &LeafToFragmentMetadata,
    ) -> anyhow::Result<Self> {
        let root = SerializedMerkleNode::new(tree.root_node(), leaf_node_to_fragment_metadata)?;

        Ok(Self {
            tree: SerializedMerkleTree { root },
        })
    }

    /// Consumes the index and returns just the tree.
    pub(crate) fn into_tree(self) -> SerializedMerkleTree {
        self.tree
    }
}

impl SerializedMerkleTree {
    pub(super) fn into_root(self) -> SerializedMerkleNode {
        self.root
    }
}

fn node_to_filesystem_info(
    node: &NodeLens,
    fragment_metadata_mapping: &LeafToFragmentMetadata,
) -> anyhow::Result<SerializedFilesystemInfo> {
    let absolute_path = node.path().to_path_buf();
    match node.node_id() {
        NodeId::Directory { .. } => Ok(SerializedFilesystemInfo::Directory { absolute_path }),
        NodeId::File {
            file_size,
            fs_modified_time,
            file_contents_hash,
            ..
        } => Ok(SerializedFilesystemInfo::File {
            absolute_path,
            file_size: *file_size,
            fs_modified_time: *fs_modified_time,
            file_contents_hash: file_contents_hash.clone(),
        }),
        NodeId::Fragment {
            absolute_path,
            content_range,
        } => {
            let Some(metadata_mapping) = fragment_metadata_mapping.get(node.hash()) else {
                return Err(anyhow::anyhow!(
                    "did not find hash in fragment metadata mapping"
                ));
            };

            let Some(fragment) = metadata_mapping.iter().find(|fragment| {
                fragment.absolute_path == *absolute_path
                    && fragment.location.byte_range == *content_range
            }) else {
                return Err(anyhow::anyhow!(
                    "did not find fragment metadata with matching path and content range"
                ));
            };

            Ok(SerializedFilesystemInfo::Fragment {
                location: (&fragment.location).into(),
            })
        }
    }
}

impl SerializedMerkleNode {
    fn new(
        node: NodeLens,
        fragment_metadata_mapping: &LeafToFragmentMetadata,
    ) -> anyhow::Result<Self> {
        let fs_info = node_to_filesystem_info(&node, fragment_metadata_mapping)?;

        let children = node
            .children()
            .map(|child| Self::new(child, fragment_metadata_mapping))
            .collect::<anyhow::Result<Vec<_>>>()?;
        let hash = node.hash().as_ref().clone();
        Ok(Self {
            hash,
            children,
            fs_info,
        })
    }

    /// Returns the node's absolute path if it is a file or directory.
    /// Returns None if the node is a fragment.
    pub(super) fn absolute_path(&self) -> Option<&Path> {
        match &self.fs_info {
            SerializedFilesystemInfo::Directory { absolute_path }
            | SerializedFilesystemInfo::File { absolute_path, .. } => Some(absolute_path.as_path()),
            SerializedFilesystemInfo::Fragment { .. } => None,
        }
    }

    pub(super) fn hash(&self) -> NodeHash {
        NodeHash::new(self.hash.to_owned())
    }

    pub(super) fn children(&self) -> impl Iterator<Item = &SerializedMerkleNode> {
        self.children.iter()
    }

    pub(super) fn fs_info(&self) -> &SerializedFilesystemInfo {
        &self.fs_info
    }
}

#[cfg(test)]
#[path = "serialized_tree_tests.rs"]
mod tests;
