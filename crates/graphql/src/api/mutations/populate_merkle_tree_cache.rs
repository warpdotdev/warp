use crate::error::UserFacingError;
use crate::full_source_code_embedding::{EmbeddingConfig, NodeHash, RepoMetadata};
use crate::request_context::RequestContext;
use crate::schema;

#[derive(cynic::QueryVariables, Debug)]
pub struct PopulateMerkleTreeCacheVariables {
    pub embedding_config: EmbeddingConfig,
    pub root_hash: NodeHash,
    pub repo_metadata: RepoMetadata,
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootMutation",
    variables = "PopulateMerkleTreeCacheVariables"
)]
pub struct PopulateMerkleTreeCache {
    #[arguments(input: { embeddingConfig: $embedding_config, rootHash: $root_hash, repoMetadata: $repo_metadata }, requestContext: $request_context)]
    pub populate_merkle_tree_cache: PopulateMerkleTreeCacheResult,
}
crate::client::define_operation! {
    populate_merkle_tree_cache(PopulateMerkleTreeCacheVariables) -> PopulateMerkleTreeCache;
}

#[derive(cynic::QueryFragment, Debug)]
pub struct PopulateMerkleTreeCacheOutput {
    pub success: bool,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum PopulateMerkleTreeCacheResult {
    UserFacingError(UserFacingError),
    PopulateMerkleTreeCacheOutput(PopulateMerkleTreeCacheOutput),
    #[cynic(fallback)]
    Unknown,
}
