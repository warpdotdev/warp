use crate::error::UserFacingError;
use crate::full_source_code_embedding::{ContentHash, EmbeddingConfig, NodeHash, RepoMetadata};
use crate::request_context::RequestContext;
use crate::schema;

#[derive(cynic::QueryVariables, Debug)]
pub struct GetRelevantFragmentsVariables {
    pub embedding_config: EmbeddingConfig,
    pub repo_metadata: RepoMetadata,
    pub query: String,
    pub request_context: RequestContext,
    pub root_hash: NodeHash,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootQuery",
    variables = "GetRelevantFragmentsVariables"
)]
pub struct GetRelevantFragmentsQuery {
    #[arguments(input: { embeddingConfig: $embedding_config, query: $query, rootHash: $root_hash, repoMetadata: $repo_metadata }, requestContext: $request_context)]
    pub get_relevant_fragments: GetRelevantFragmentsResult,
}
crate::client::define_operation! {
    get_relevant_fragments(GetRelevantFragmentsVariables) -> GetRelevantFragmentsQuery;
}

#[derive(cynic::QueryFragment, Debug)]
pub struct GetRelevantFragmentsError {
    pub error: String,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct GetRelevantFragmentsOutput {
    pub candidate_hashes: Vec<ContentHash>,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum GetRelevantFragmentsResult {
    GetRelevantFragmentsOutput(GetRelevantFragmentsOutput),
    GetRelevantFragmentsError(GetRelevantFragmentsError),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}
