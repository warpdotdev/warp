//! Common types for hashes that identify codebase embedding state.

use generic_array::GenericArray;
use serde::{Deserialize, Serialize};
use sha2::{digest::OutputSizeUser, Digest, Sha256};
use std::{fmt, str::FromStr, sync::Arc};

use crate::index::full_source_code_embedding::chunker::Fragment;

use super::Error;

/// The hash of an *intermediate* node in the [`MerkleTree`].
///
/// Unlike [`MerkleHash`], this is guaranteed to be an intermediate node.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NodeHash(MerkleHash);

impl NodeHash {
    pub(super) fn new(hash: MerkleHash) -> Self {
        Self(hash)
    }
}

impl fmt::Display for NodeHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for NodeHash {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(MerkleHash::from_str(s)?))
    }
}

impl From<NodeHash> for warp_graphql::full_source_code_embedding::NodeHash {
    fn from(value: NodeHash) -> Self {
        warp_graphql::full_source_code_embedding::NodeHash(value.0.to_string())
    }
}

impl TryFrom<warp_graphql::full_source_code_embedding::NodeHash> for NodeHash {
    type Error = Error;

    fn try_from(
        value: warp_graphql::full_source_code_embedding::NodeHash,
    ) -> Result<Self, Self::Error> {
        Ok(Self(MerkleHash::from_str(&value.0)?))
    }
}

impl AsRef<MerkleHash> for NodeHash {
    fn as_ref(&self) -> &MerkleHash {
        &self.0
    }
}

impl From<&ContentHash> for NodeHash {
    fn from(value: &ContentHash) -> Self {
        NodeHash(value.0.to_owned())
    }
}

impl From<ContentHash> for NodeHash {
    fn from(value: ContentHash) -> Self {
        NodeHash(value.0)
    }
}

/// The hash of a fragment (leaf) node in the [`MerkleTree`].
///
/// Unlike [`MerkleHash`], this is guaranteed to be a leaf node.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ContentHash(MerkleHash);

impl ContentHash {
    pub(crate) fn new(hash: MerkleHash) -> Self {
        Self(hash)
    }

    pub fn from_content(content: &str) -> Self {
        Self(MerkleHash::from_bytes(content.as_bytes()))
    }
}

impl fmt::Display for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for ContentHash {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(MerkleHash::from_str(s)?))
    }
}

impl From<ContentHash> for warp_graphql::full_source_code_embedding::ContentHash {
    fn from(value: ContentHash) -> Self {
        warp_graphql::full_source_code_embedding::ContentHash(value.0.to_string())
    }
}

impl TryFrom<warp_graphql::full_source_code_embedding::ContentHash> for ContentHash {
    type Error = Error;

    fn try_from(
        value: warp_graphql::full_source_code_embedding::ContentHash,
    ) -> Result<Self, Self::Error> {
        Ok(Self(MerkleHash::from_str(&value.0)?))
    }
}

impl AsRef<MerkleHash> for ContentHash {
    fn as_ref(&self) -> &MerkleHash {
        &self.0
    }
}

impl AsRef<ContentHash> for ContentHash {
    fn as_ref(&self) -> &ContentHash {
        self
    }
}

/// A SHA-256 hash for a node in the [`MerkleTree`].
///
/// Cloning a `MerkleHash` is cheap, and need not be avoided.
/// TODO(CODE-399): make this private to the `merkle_tree` module.
#[derive(Ord, PartialOrd, Eq, PartialEq, Hash, Clone)]
pub(crate) struct MerkleHash(Arc<GenericArray<u8, <Sha256 as OutputSizeUser>::OutputSize>>);

impl AsRef<MerkleHash> for MerkleHash {
    fn as_ref(&self) -> &MerkleHash {
        self
    }
}

/// The default serialize prints the hash as a vector of small integers,
/// which takes up more space (up to 5 characters per byte) and is more
/// difficult to read.
/// This custom serialization hex-encodes the bytes in a string instead.
impl Serialize for MerkleHash {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut buf = [0u8; 64];
        let hex_str = base16ct::lower::encode_str(&self.0, &mut buf)
            .expect("Buffer is sufficient for a SHA-256 hash");
        serializer.serialize_str(hex_str)
    }
}

impl<'de> Deserialize<'de> for MerkleHash {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let hex_string = String::deserialize(deserializer)?;
        MerkleHash::from_str(&hex_string).map_err(serde::de::Error::custom)
    }
}

impl MerkleHash {
    pub(super) fn from_hashes<'a>(iterator: impl Iterator<Item = &'a MerkleHash>) -> Self {
        let mut hasher = Sha256::new();
        for hash in iterator {
            Digest::update(&mut hasher, hash.0.as_slice());
        }

        Self::from_digest(hasher)
    }

    pub(crate) fn from_bytes(content_bytes: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        Digest::update(&mut hasher, content_bytes);

        Self::from_digest(hasher)
    }

    pub(super) fn from_fragment(fragment: &Fragment<'_>) -> Self {
        Self::from_bytes(fragment.content.as_bytes())
    }

    fn from_digest(digest: Sha256) -> Self {
        Self(Arc::new(digest.finalize()))
    }
}

impl FromStr for MerkleHash {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut buf: GenericArray<u8, <Sha256 as OutputSizeUser>::OutputSize> = Default::default();
        let decoded =
            base16ct::lower::decode(s.as_bytes(), &mut buf).map_err(Error::InvalidHash)?;
        if decoded.len() != 32 {
            return Err(Error::InvalidHash(base16ct::Error::InvalidLength));
        }

        Ok(MerkleHash(Arc::new(buf)))
    }
}

impl fmt::Debug for MerkleHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "MerkleHash({self})")
    }
}

impl fmt::Display for MerkleHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut buf = [0u8; 64];
        let hex_string = base16ct::lower::encode_str(&self.0, &mut buf)
            .expect("Buffer is sufficient for a SHA-256 hash");
        write!(f, "{hex_string}")
    }
}

#[cfg(test)]
#[path = "hash_tests.rs"]
mod hash_test;
