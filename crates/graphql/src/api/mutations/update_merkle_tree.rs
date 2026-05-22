use crate::error::UserFacingError;
use crate::full_source_code_embedding::{EmbeddingConfig, NodeHash};
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;

#[derive(cynic::QueryVariables, Debug)]
pub struct UpdateMerkleTreeVariables {
    pub input: UpdateMerkleTreeInput,
    pub request_context: RequestContext,
}

#[derive(cynic::InputObject, Debug)]
pub struct UpdateMerkleTreeInput {
    pub embedding_config: EmbeddingConfig,
    pub nodes: Vec<MerkleTreeNode>,
}

#[derive(cynic::InputObject, Debug)]
pub struct MerkleTreeNode {
    pub hash: NodeHash,
    pub children: Vec<NodeHash>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "RootMutation", variables = "UpdateMerkleTreeVariables")]
pub struct UpdateMerkleTree {
    #[arguments(input: $input, requestContext: $request_context)]
    pub update_merkle_tree: UpdateMerkleTreeResult,
}
crate::client::define_operation! {
    update_merkle_tree(UpdateMerkleTreeVariables) -> UpdateMerkleTree;
}

#[derive(cynic::QueryFragment, Debug)]
pub struct UpdateMerkleTreeError {
    pub error: String,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct UpdateMerkleTreeOutput {
    pub response_context: ResponseContext,
    pub results: Vec<UpdateMerkleTreeNodeResult>,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct UpdateMerkleTreeNodeResult {
    pub hash: NodeHash,
    pub success: bool,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum UpdateMerkleTreeResult {
    UpdateMerkleTreeOutput(UpdateMerkleTreeOutput),
    UpdateMerkleTreeError(UpdateMerkleTreeError),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}
