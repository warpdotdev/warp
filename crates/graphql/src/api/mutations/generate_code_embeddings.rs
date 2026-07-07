use crate::error::UserFacingError;
use crate::full_source_code_embedding::{
    ContentHash, EmbeddingConfig, Fragment, NodeHash, RepoMetadata,
};
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;

#[derive(cynic::QueryVariables, Debug)]
pub struct GenerateCodeEmbeddingsVariables {
    pub input: GenerateCodeEmbeddingsInput,
    pub request_context: RequestContext,
}

#[derive(cynic::InputObject, Debug)]
pub struct GenerateCodeEmbeddingsInput {
    pub embedding_config: EmbeddingConfig,
    pub repo_metadata: RepoMetadata,
    pub fragments: Vec<Fragment>,
    pub root_hash: NodeHash,
}

#[derive(cynic::InputObject, Debug)]
pub struct MerkleTreeNode {
    pub hash: NodeHash,
    pub children: Vec<NodeHash>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootMutation",
    variables = "GenerateCodeEmbeddingsVariables"
)]
pub struct GenerateCodeEmbeddings {
    #[arguments(input: $input, requestContext: $request_context)]
    pub generate_code_embeddings: GenerateCodeEmbeddingsResult,
}
crate::client::define_operation! {
    generate_code_embeddings(GenerateCodeEmbeddingsVariables) -> GenerateCodeEmbeddings;
}

#[derive(cynic::QueryFragment, Debug)]
pub struct GenerateCodeEmbeddingsError {
    pub error: String,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct GenerateCodeEmbeddingsOutput {
    pub response_context: ResponseContext,
    pub embedding_results: Vec<GenerateCodeEmbeddingResult>,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct GenerateCodeEmbeddingResult {
    pub hash: ContentHash,
    pub success: bool,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum GenerateCodeEmbeddingsResult {
    GenerateCodeEmbeddingsOutput(GenerateCodeEmbeddingsOutput),
    GenerateCodeEmbeddingsError(GenerateCodeEmbeddingsError),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}
