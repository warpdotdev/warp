use crate::error::UserFacingError;
use crate::full_source_code_embedding::ContentHash;
use crate::request_context::RequestContext;
use crate::schema;

#[derive(cynic::QueryVariables, Debug)]
pub struct RerankFragmentsVariables {
    pub fragments: Vec<RerankFragmentInput>,
    pub query: String,
    pub request_context: RequestContext,
}

#[derive(cynic::InputObject, Debug)]
pub struct RerankFragmentInput {
    pub content: String,
    pub content_hash: ContentHash,
    pub location: FragmentLocationInput,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "RootQuery", variables = "RerankFragmentsVariables")]
pub struct RerankFragments {
    #[arguments(input: { fragments: $fragments, query: $query }, requestContext: $request_context)]
    pub rerank_fragments: RerankFragmentsResult,
}
crate::client::define_operation! {
    rerank_fragments(RerankFragmentsVariables) -> RerankFragments;
}

#[derive(cynic::QueryFragment, Debug)]
pub struct RerankFragmentsError {
    pub error: String,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct RerankFragmentsOutput {
    pub ranked_fragments: Vec<RerankFragment>,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct RerankFragment {
    pub content: String,
    pub content_hash: ContentHash,
    pub location: FragmentLocation,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct FragmentLocation {
    pub byte_end: i32,
    pub byte_start: i32,
    pub file_path: String,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum RerankFragmentsResult {
    RerankFragmentsOutput(RerankFragmentsOutput),
    RerankFragmentsError(RerankFragmentsError),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::InputObject, Debug)]
pub struct FragmentLocationInput {
    pub byte_end: i32,
    pub byte_start: i32,
    pub file_path: String,
}
