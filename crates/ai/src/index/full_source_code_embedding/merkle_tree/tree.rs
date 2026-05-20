use crate::index::Entry;
use anyhow::anyhow;
use cfg_if::cfg_if;
use std::{
    collections::{HashSet, VecDeque},
    path::PathBuf,
};

use crate::index::full_source_code_embedding::fragment_metadata::{
    LeafToFragmentMetadata, LeafToFragmentMetadataUpdates,
};
use crate::index::full_source_code_embedding::Error;

use super::{
    node::{ChildrenPath, MerkleNode, NodeLens, NodeMask},
    serialized_tree::SerializedMerkleTree,
    DirEntryOrFragment,
};

pub(super) enum UpdateFileResult {
    Deleted,
    Updated,
    NoChange,
}

pub(crate) struct TreeUpdateResult<'a> {
    pub node_lens: Vec<NodeLens<'a>>,
    pub leaf_to_fragment_meta_updates: LeafToFragmentMetadataUpdates,
}

/// A merkle tree used for codebase indexing. This data structure allows us to efficiently compute
/// which parts of a repository have changed without needing to traverse every file in the tree.
///
/// The leaves of this tree are code fragments of a given file in the repository, with a
/// corresponding SHA-256 hash of the contents.
///
/// The parents nodes in this tree are either a directory or a file (where the children are all the
/// fragments of the file) and a corresponding hash of all the hashes of the children.
///
/// For example. Consider the following repository structure:
/// * `/src`
/// * `/src/foo.rs`
/// * `/src/bar.rs`
/// * `/src/bazz/buzz.rs`
///
/// The tree would roughly look:
///
/// /src (Hash: FooBarBuzzBazz)
/// ├── /src/foo.rs (Hash: Foo)
/// ├── /src/bar.rs (Hash: Bar)
/// └── /src/bazz (Hash: BuzzBazz)
///     └── /src/bazz/buzz.rs (Hash: Buzz)
/// `
#[derive(Debug)]
pub(crate) struct MerkleTree {
    root: MerkleNode,
}

impl MerkleTree {
    /// Creates a new [`MerkleTree`] given the root node of an [`Entry`].
    /// Returns an error if a node could not be created for any reason.
    pub async fn try_new(entry: Entry) -> anyhow::Result<(MerkleTree, LeafToFragmentMetadata)> {
        let build_node = move || MerkleNode::new(DirEntryOrFragment::Entry(entry));

        let (root, mapping_update) = if tokio::runtime::Handle::try_current().is_ok() {
            // Offload to a blocking thread so that the rayon `pool.install()` call inside
            // `MerkleNode::new` does not block a tokio executor thread. Blocking executor
            // threads starves other async tasks (e.g. shell history parsing during bootstrap).
            tokio::task::spawn_blocking(build_node)
                .await
                .map_err(|e| anyhow::anyhow!("spawn_blocking join error: {e}"))?
        } else {
            build_node()
        }?;

        let leaf_node_to_fragment_metadata = LeafToFragmentMetadata::new(mapping_update);
        Ok((Self { root }, leaf_node_to_fragment_metadata))
    }

    pub fn root_node(&self) -> NodeLens<'_> {
        NodeLens::new(&self.root)
    }

    pub fn from_serialized_tree(
        serialized_tree: SerializedMerkleTree,
    ) -> anyhow::Result<(Self, LeafToFragmentMetadata)> {
        let serialized_root = serialized_tree.into_root();
        let Some(root_path) = serialized_root.absolute_path() else {
            return Err(anyhow::anyhow!("root node should never be a fragment"));
        };
        let root_path = root_path.to_path_buf();
        let (root, mapping_update) =
            MerkleNode::from_serialized(serialized_root, root_path.as_path())?;
        let leaf_node_to_fragment_metadata = LeafToFragmentMetadata::new(mapping_update);
        Ok((Self { root }, leaf_node_to_fragment_metadata))
    }

    /// Construct the changed nodes' NodeLens from a NodeMask.
    /// NodeLens are returned in reverse-BFS order (children first).
    pub(super) fn nodes_from_mask(&self, node_mask: NodeMask) -> Result<Vec<NodeLens<'_>>, Error> {
        let mut result = vec![];
        let mut queue = VecDeque::new();
        queue.push_back((&self.root, node_mask));

        while let Some((current_node, path)) = queue.pop_front() {
            result.push(NodeLens::new(current_node));

            match path.children {
                ChildrenPath::All => {
                    for (idx, child_node) in current_node.children().enumerate() {
                        queue.push_back((child_node, NodeMask::new(idx)));
                    }
                }
                ChildrenPath::SpecificChildren(children_path) => {
                    for child_path in children_path {
                        if child_path.index < current_node.count_children() {
                            let child_node = &current_node.child_at(child_path.index);
                            queue.push_back((child_node, child_path));
                        } else {
                            return Err(Error::Other(anyhow!(
                                "Invalid child index {} in node with {} child(ren)",
                                child_path.index,
                                current_node.count_children(),
                            )));
                        }
                    }
                }
            }
        }

        result.reverse();
        Ok(result)
    }

    /// Given the paths to a set of changed files, for each file, update the node if it exists in the tree, or
    /// create a node if it doesn't exist. This also updates all the intermediate nodes.
    ///
    /// Return all the updated nodes.
    pub async fn upsert_files(
        &mut self,
        mut paths: HashSet<PathBuf>,
    ) -> Result<TreeUpdateResult<'_>, Error> {
        let mut node_path = NodeMask::default();
        let mut leaf_to_fragment_meta_updates = LeafToFragmentMetadataUpdates::empty();

        let mut do_upsert = || {
            self.root.upsert_files(
                &mut paths,
                &mut node_path,
                &mut leaf_to_fragment_meta_updates,
            )
        };

        cfg_if! {
            if #[cfg(not(target_family = "wasm"))] {
                let upsert_result = if tokio::runtime::Handle::try_current().is_ok() {
                    // `upsert_files` is expensive and can block a background thread for a while,
                    // so use `block_in_place` to tell tokio to move any tasks enqueued for this
                    // thread to another thread (so that they might be able to run on a different
                    // thread).
                    tokio::task::block_in_place(do_upsert)
                } else {
                    do_upsert()
                };
            } else {
                let upsert_result = do_upsert();
            }
        }

        // Note that we cannot early return directly here on error. We need to make sure leaf_node_to_fragment_metadatas
        // is properly written so the tree remains valid.
        let node_lens = if !matches!(upsert_result, UpdateFileResult::NoChange) {
            self.nodes_from_mask(node_path)?
        } else {
            vec![self.root_node()]
        };

        Ok(TreeUpdateResult {
            node_lens,
            leaf_to_fragment_meta_updates,
        })
    }

    /// Given the path to a removed file, remove the node if it exists in the tree.
    ///
    /// Return all the updated nodes.
    pub async fn remove_files(
        &mut self,
        mut paths: HashSet<PathBuf>,
    ) -> Result<TreeUpdateResult<'_>, Error> {
        let mut node_path = NodeMask::default();
        let mut leaf_to_fragment_meta_updates = LeafToFragmentMetadataUpdates::empty();

        // Note that we cannot early return directly here on error. We need to make sure leaf_node_to_fragment_metadatas
        // is properly written so the tree remains valid.
        let node_lens = match self.root.remove_files(
            &mut paths,
            &mut node_path,
            &mut leaf_to_fragment_meta_updates,
        ) {
            UpdateFileResult::NoChange => vec![self.root_node()],
            _ => self.nodes_from_mask(node_path)?,
        };

        Ok(TreeUpdateResult {
            node_lens,
            leaf_to_fragment_meta_updates,
        })
    }
}

#[cfg(test)]
#[path = "tree_tests.rs"]
mod tests;
