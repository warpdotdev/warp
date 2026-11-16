use super::Error;
use super::chunker::Fragment;

mod hash;
mod node;
mod serialized_tree;
mod tree;

pub(super) use hash::MerkleHash;
pub use hash::{ContentHash, NodeHash};
pub(super) use node::NodeLens;
pub(super) use serialized_tree::SerializedCodebaseIndex;
pub(super) use tree::MerkleTree;

use crate::index::Entry;
cfg_if::cfg_if! {
    if #[cfg(feature = "local_fs")] {
        pub(super) use node::NodeId;
        pub(super) use tree::TreeUpdateResult;
    }
}

#[derive(Debug)]
enum DirEntryOrFragment<'a> {
    Entry(Entry),
    Fragment(Fragment<'a>),
}

#[cfg(test)]
mod test_util;

#[cfg(test)]
pub(super) use test_util::construct_test_merkle_tree;
