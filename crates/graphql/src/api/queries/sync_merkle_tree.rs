use crate::error::UserFacingError;
use crate::full_source_code_embedding::{EmbeddingConfig, NodeHash};
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;

#[derive(cynic::InputObject, Debug)]
pub struct SyncMerkleTreeInput {
    pub hashed_nodes: Vec<NodeHash>,
    pub embedding_config: EmbeddingConfig,
}

#[derive(cynic::QueryVariables, Debug)]
pub struct SyncMerkleTreeVariables {
    pub request_context: RequestContext,
    pub input: SyncMerkleTreeInput,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "RootQuery", variables = "SyncMerkleTreeVariables")]
pub struct SyncMerkleTree {
    #[arguments(requestContext: $request_context, input: $input)]
    pub sync_merkle_tree: SyncMerkleTreeResult,
}
crate::client::define_operation! {
    sync_merkle_tree(SyncMerkleTreeVariables) -> SyncMerkleTree;
}

#[derive(cynic::QueryFragment, Debug)]
pub struct SyncMerkleTreeError {
    pub error: String,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct SyncMerkleTreeOutput {
    pub response_context: ResponseContext,
    pub changed_nodes: Vec<NodeHash>,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum SyncMerkleTreeResult {
    SyncMerkleTreeOutput(SyncMerkleTreeOutput),
    SyncMerkleTreeError(SyncMerkleTreeError),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}
