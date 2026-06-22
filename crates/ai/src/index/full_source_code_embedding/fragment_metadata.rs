use std::collections::HashMap;
use std::ops::Range;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use string_offset::ByteOffset;

use super::merkle_tree::MerkleHash;
use crate::index::full_source_code_embedding::chunker::Fragment;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FragmentLocation {
    pub start_line: usize,
    /// End line number (inclusive).
    pub end_line: usize,
    /// The range of byte indices into the original source string for this fragment.
    pub byte_range: Range<ByteOffset>,
}

/// Fragment metadata that we persist in the tree. This helps us map from a leaf merkle node
/// to the actual content on user's disk.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FragmentMetadata {
    /// File path of the fragment.
    pub absolute_path: PathBuf,
    /// Location of the fragment within the file.
    pub location: FragmentLocation,
}

impl FragmentMetadata {
    /// Returns the estimated content size in bytes, derived from the stored byte range.
    pub fn content_byte_size(&self) -> usize {
        self.location
            .byte_range
            .end
            .as_usize()
            .saturating_sub(self.location.byte_range.start.as_usize())
    }
}

impl From<&Fragment<'_>> for FragmentMetadata {
    fn from(fragment: &Fragment<'_>) -> Self {
        FragmentMetadata {
            absolute_path: PathBuf::from(fragment.file_path),
            location: FragmentLocation {
                start_line: fragment.start_line,
                end_line: fragment.end_line,
                byte_range: fragment.start_byte_index..fragment.end_byte_index,
            },
        }
    }
}

pub type LeafToFragmentMetadataMapping = HashMap<MerkleHash, Vec<FragmentMetadata>>;

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct LeafToFragmentMetadata {
    mapping: LeafToFragmentMetadataMapping,
}

impl LeafToFragmentMetadata {
    pub(super) fn empty() -> Self {
        Self::default()
    }

    pub(super) fn new(initial_content: LeafToFragmentMetadataUpdates) -> Self {
        Self::from(initial_content)
    }

    #[cfg(test)]
    pub(super) fn new_for_test(content: HashMap<MerkleHash, Vec<FragmentMetadata>>) -> Self {
        Self { mapping: content }
    }

    pub(super) fn mapping(&self) -> &LeafToFragmentMetadataMapping {
        &self.mapping
    }

    pub fn get<T: AsRef<MerkleHash>>(&self, hash: T) -> Option<&Vec<FragmentMetadata>> {
        self.mapping.get(hash.as_ref())
    }

    pub fn apply_update(&mut self, update: LeafToFragmentMetadataUpdates) {
        let LeafToFragmentMetadataUpdates {
            to_remove,
            to_insert,
        } = update;
        for (path, hashes) in to_remove {
            for hash in hashes {
                let Some(mapping_entry) = self.mapping.get_mut(&hash) else {
                    continue;
                };
                mapping_entry.retain(|metadata| metadata.absolute_path != path);
                if mapping_entry.is_empty() {
                    self.mapping.remove(&hash);
                }
            }
        }
        to_insert.into_iter().for_each(|(hash, metadatas)| {
            self.mapping.entry(hash).or_default().extend(metadatas);
        });
    }
}

impl From<LeafToFragmentMetadataUpdates> for LeafToFragmentMetadata {
    fn from(update: LeafToFragmentMetadataUpdates) -> Self {
        Self {
            mapping: update.to_insert,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct LeafToFragmentMetadataUpdates {
    /// Since the same fragment can occur multiple times within the same file,
    /// the filepath is not enough to uniquely identify a fragment.
    /// At the moment, we only handle removing entire files, so using the path alone is okay.
    /// In the future, add the FragmentMetadata to the key to uniquely identify a fragment.
    pub(super) to_remove: HashMap<PathBuf, Vec<MerkleHash>>,
    pub(super) to_insert: LeafToFragmentMetadataMapping,
}

impl LeafToFragmentMetadataUpdates {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.to_remove.is_empty() && self.to_insert.is_empty()
    }

    pub fn merge(&mut self, other: Self) {
        other.to_remove.into_iter().for_each(|(path, hashes)| {
            self.to_remove.entry(path).or_default().extend(hashes);
        });
        other.to_insert.into_iter().for_each(|(hash, metadata)| {
            self.to_insert.entry(hash).or_default().extend(metadata);
        })
    }

    pub fn insertions(&self) -> &LeafToFragmentMetadataMapping {
        &self.to_insert
    }
}

impl Extend<LeafToFragmentMetadataUpdates> for LeafToFragmentMetadataUpdates {
    fn extend<T: IntoIterator<Item = LeafToFragmentMetadataUpdates>>(&mut self, iter: T) {
        iter.into_iter().for_each(|update| self.merge(update));
    }
}
