mod changed_files;
mod chunker;
mod codebase_index;
mod fragment_metadata;
pub mod manager;
mod merkle_tree;
mod priority_queue;
pub mod search_shaping;
mod snapshot;
pub mod store_client;
mod sync_client;

use std::{
    ops::Range,
    path::{Path, PathBuf},
    time::Duration,
};
pub use sync_client::SyncTask;

pub use codebase_index::{CodebaseIndex, RetrievalID, SyncProgress};
pub use merkle_tree::{ContentHash, NodeHash};
pub use snapshot::SnapshotStorage;

pub use fragment_metadata::{FragmentLocation as FragmentMetadataLocation, FragmentMetadata};
use string_offset::ByteOffset;
use thiserror::Error;
use warp_graphql::queries::rerank_fragments::FragmentLocationInput;

#[derive(Error, Debug)]
pub enum Error {
    #[error("File I/O error {0:#}")]
    Io(#[from] std::io::Error),
    #[error("Not a git repository")]
    NotAGitRepository,
    #[error("Build tree error {0:#}")]
    BuildTreeError(#[from] crate::index::BuildTreeError),
    #[error("Unsupported platform")]
    UnsupportedPlatform,
    #[error("Invalid hash: {0:#}")]
    InvalidHash(base16ct::Error),
    #[error("Empty node content")]
    EmptyNodeContent,
    #[error("Failed to get metadata")]
    FailedToGetMetadata(PathBuf),
    #[error("File size exceeds maximum limit")]
    FileSizeExceeded,
    #[error(transparent)]
    InconsistentState(#[from] InconsistentStateError),
    #[error("Failed to generate embeddings for some hashes")]
    FailedToGenerateEmbeddings(Vec<FragmentMetadata>),
    #[error("Failed to sync some intermediate nodes")]
    FailedToSyncIntermediateNodes(Vec<NodeHash>),
    #[error("Diff merkle tree {0:#}")]
    DiffMerkleTreeError(#[from] crate::index::full_source_code_embedding::DiffMerkleTreeError),
    #[error("File system changed since merkle tree construction")]
    FileSystemStateChanged,
    #[error(transparent)]
    Other(#[from] anyhow::Error),
    #[error("Failed to parse snapshot")]
    SnapshotParsingFailed,
}

// Based off of BuildTreeError in entry.rs
#[derive(Debug, Error)]
pub enum DiffMerkleTreeError {
    #[error("Merkle tree node and file mismatch")]
    CurrentNodeMismatch(PathBuf),
    #[error("File is ignored")]
    Ignored,
    #[error("Symlink is not supported")]
    Symlink,
    #[error("Fragment node in diffing process")]
    Fragment(PathBuf),
    #[error("Max depth exceeded")]
    MaxDepthExceeded,
    #[error("Exceeded max file limit")]
    ExceededMaxFileLimit,
}

#[derive(Error, Debug)]
pub enum InconsistentStateError {
    #[error("Missing fragment metadata for {fragment_hash}")]
    MissingFragmentMetadata { fragment_hash: ContentHash },
    #[error("Can't find node index in merkle node")]
    NodeIndexNotFound,
}

#[allow(non_camel_case_types)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum EmbeddingConfig {
    OpenAiTextSmall3_256,
    VoyageCode3_512,
    Voyage3_5_Lite_512,
    #[default]
    Voyage3_5_512,
    Voyage4_512,
}

#[derive(Debug, Clone)]
pub struct RepoMetadata {
    pub path: Option<String>,
}

impl From<RepoMetadata> for warp_graphql::full_source_code_embedding::RepoMetadata {
    fn from(val: RepoMetadata) -> Self {
        Self { path: val.path }
    }
}

impl From<EmbeddingConfig> for warp_graphql::full_source_code_embedding::EmbeddingConfig {
    fn from(val: EmbeddingConfig) -> Self {
        match val {
            EmbeddingConfig::OpenAiTextSmall3_256 => {
                warp_graphql::full_source_code_embedding::EmbeddingConfig::OpenaiTextSmall3256
            }
            EmbeddingConfig::VoyageCode3_512 => {
                warp_graphql::full_source_code_embedding::EmbeddingConfig::VoyageCode3512
            }
            EmbeddingConfig::Voyage3_5_512 => {
                warp_graphql::full_source_code_embedding::EmbeddingConfig::Voyage35512
            }
            EmbeddingConfig::Voyage3_5_Lite_512 => {
                warp_graphql::full_source_code_embedding::EmbeddingConfig::Voyage35Lite512
            }
            EmbeddingConfig::Voyage4_512 => {
                warp_graphql::full_source_code_embedding::EmbeddingConfig::Voyage4512
            }
        }
    }
}

impl TryFrom<warp_graphql::full_source_code_embedding::EmbeddingConfig> for EmbeddingConfig {
    type Error = Error;

    fn try_from(
        value: warp_graphql::full_source_code_embedding::EmbeddingConfig,
    ) -> Result<Self, Self::Error> {
        match value {
            warp_graphql::full_source_code_embedding::EmbeddingConfig::OpenaiTextSmall3256 => {
                Ok(Self::OpenAiTextSmall3_256)
            }
            warp_graphql::full_source_code_embedding::EmbeddingConfig::Voyage35Lite512 => {
                Ok(Self::Voyage3_5_Lite_512)
            }
            warp_graphql::full_source_code_embedding::EmbeddingConfig::VoyageCode3512 => {
                Ok(Self::VoyageCode3_512)
            }
            warp_graphql::full_source_code_embedding::EmbeddingConfig::Voyage35512 => {
                Ok(Self::Voyage3_5_512)
            }
            warp_graphql::full_source_code_embedding::EmbeddingConfig::Voyage4512 => {
                Ok(Self::Voyage4_512)
            }
        }
    }
}

#[derive(Clone, Copy)]
pub struct CodebaseContextConfig {
    pub embedding_config: EmbeddingConfig,
    pub embedding_cadence: Duration,
}

#[derive(Clone)]
pub struct FragmentLocation {
    absolute_path: PathBuf,
    byte_range: Range<ByteOffset>,
}

#[derive(Clone)]
pub struct Fragment {
    content: String,
    content_hash: ContentHash,
    location: FragmentLocation,
}

impl Fragment {
    pub fn from_byte_range(
        content: String,
        content_hash: ContentHash,
        absolute_path: PathBuf,
        byte_range: Range<ByteOffset>,
    ) -> Self {
        Self {
            content,
            content_hash,
            location: FragmentLocation {
                absolute_path,
                byte_range,
            },
        }
    }

    pub fn content_hash(&self) -> &ContentHash {
        &self.content_hash
    }

    pub fn absolute_path(&self) -> &Path {
        &self.location.absolute_path
    }

    pub fn byte_range(&self) -> Range<ByteOffset> {
        self.location.byte_range.clone()
    }
}

impl From<Fragment> for warp_graphql::full_source_code_embedding::Fragment {
    fn from(val: Fragment) -> Self {
        Self {
            content: val.content,
            content_hash: val.content_hash.into(),
        }
    }
}

impl From<Fragment> for warp_graphql::queries::rerank_fragments::RerankFragmentInput {
    fn from(val: Fragment) -> Self {
        Self {
            content: val.content,
            content_hash: val.content_hash.into(),
            location: FragmentLocationInput {
                byte_start: val.location.byte_range.start.as_usize() as i32,
                byte_end: val.location.byte_range.end.as_usize() as i32,
                file_path: val.location.absolute_path.to_string_lossy().to_string(),
            },
        }
    }
}

impl TryFrom<warp_graphql::queries::rerank_fragments::RerankFragment> for Fragment {
    type Error = Error;

    fn try_from(
        val: warp_graphql::queries::rerank_fragments::RerankFragment,
    ) -> Result<Self, Self::Error> {
        Ok(Self {
            content: val.content,
            content_hash: val.content_hash.try_into()?,
            location: FragmentLocation {
                absolute_path: PathBuf::from(val.location.file_path),
                byte_range: ByteOffset::from(val.location.byte_start as usize)
                    ..ByteOffset::from(val.location.byte_end as usize),
            },
        })
    }
}
