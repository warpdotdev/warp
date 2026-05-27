use std::collections::{HashMap, HashSet};
use std::fmt::Debug;
use std::time::Duration;

use async_trait::async_trait;

use super::{
    CodebaseContextConfig, ContentHash, EmbeddingConfig, Error, Fragment, NodeHash, RepoMetadata,
};

/// Client interface for a remote full source code embedding store.
#[cfg_attr(not(target_family = "wasm"), async_trait::async_trait)]
#[cfg_attr(target_family = "wasm", async_trait::async_trait(?Send))]
pub trait StoreClient: 'static + Send + Sync {
    /// Persist new intermediate Merkle tree nodes.
    async fn update_intermediate_nodes(
        &self,
        embedding_config: EmbeddingConfig,
        nodes: Vec<IntermediateNode>,
    ) -> Result<HashMap<NodeHash, bool>, Error>;

    /// Generate embeddings for individual code fragments.
    ///
    /// Embedding generation may fail on a per-fragment basis, so this returns the status of each
    /// fragment. If the overall request fails, assume that no embeddings were generated.
    async fn generate_embeddings(
        &self,
        embedding_config: EmbeddingConfig,
        fragments: Vec<Fragment>,
        root_hash: NodeHash,
        repo_metadata: RepoMetadata,
    ) -> Result<HashMap<ContentHash, bool>, Error>;

    async fn populate_merkle_tree_cache(
        &self,
        embedding_config: EmbeddingConfig,
        root_hash: NodeHash,
        repo_metadata: RepoMetadata,
    ) -> Result<bool, Error>;

    async fn sync_merkle_tree(
        &self,
        nodes: Vec<NodeHash>,
        embedding_config: EmbeddingConfig,
    ) -> Result<HashSet<NodeHash>, Error>;

    async fn rerank_fragments(
        &self,
        query: String,
        fragment: Vec<Fragment>,
    ) -> Result<Vec<Fragment>, Error>;

    async fn get_relevant_fragments(
        &self,
        embedding_config: EmbeddingConfig,
        query: String,
        root_hash: NodeHash,
        repo_metadata: RepoMetadata,
    ) -> Result<Vec<ContentHash>, Error>;

    async fn codebase_context_config(&self) -> Result<CodebaseContextConfig, Error>;
}

#[derive(Debug, Default, Clone)]
pub struct MockStoreClient;

#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
impl StoreClient for MockStoreClient {
    async fn update_intermediate_nodes(
        &self,
        _embedding_config: EmbeddingConfig,
        _nodes: Vec<IntermediateNode>,
    ) -> Result<HashMap<NodeHash, bool>, Error> {
        Ok(HashMap::new())
    }

    async fn generate_embeddings(
        &self,
        _embedding_config: EmbeddingConfig,
        _fragments: Vec<Fragment>,
        _root_hash: NodeHash,
        _repo_metadata: RepoMetadata,
    ) -> Result<HashMap<ContentHash, bool>, Error> {
        Ok(HashMap::new())
    }

    async fn sync_merkle_tree(
        &self,
        _nodes: Vec<NodeHash>,
        _embedding_config: EmbeddingConfig,
    ) -> Result<HashSet<NodeHash>, Error> {
        Ok(HashSet::new())
    }

    async fn populate_merkle_tree_cache(
        &self,
        _embedding_config: EmbeddingConfig,
        _root_hash: NodeHash,
        _repo_metadata: RepoMetadata,
    ) -> Result<bool, Error> {
        Ok(true)
    }

    async fn rerank_fragments(
        &self,
        _query: String,
        fragments: Vec<Fragment>,
    ) -> Result<Vec<Fragment>, Error> {
        // Return input as is for mock
        Ok(fragments)
    }

    async fn get_relevant_fragments(
        &self,
        _embedding_config: EmbeddingConfig,
        _query: String,
        _root_hash: NodeHash,
        _repo_metadata: RepoMetadata,
    ) -> Result<Vec<ContentHash>, Error> {
        Ok(Vec::new())
    }

    async fn codebase_context_config(&self) -> Result<CodebaseContextConfig, Error> {
        Ok(CodebaseContextConfig {
            embedding_config: EmbeddingConfig::default(),
            embedding_cadence: Duration::from_secs(300),
        })
    }
}

/// The contents of an intermediate Merkle tree node, used to sync it to the remote embedding store.
#[derive(Debug, Clone)]
pub struct IntermediateNode {
    pub hash: NodeHash,
    pub children: Vec<NodeHash>,
}
