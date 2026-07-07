use crate::error::UserFacingError;
use crate::full_source_code_embedding::EmbeddingConfig;
use crate::request_context::RequestContext;
use crate::schema;

#[derive(cynic::QueryVariables, Debug)]
pub struct CodebaseContextConfigVariables {
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootQuery",
    variables = "CodebaseContextConfigVariables"
)]
pub struct CodebaseContextConfigQuery {
    #[arguments(requestContext: $request_context)]
    pub codebase_context_config: CodebaseContextConfigResult,
}
crate::client::define_operation! {
    codebase_context_config(CodebaseContextConfigVariables) -> CodebaseContextConfigQuery;
}

#[derive(cynic::QueryFragment, Debug)]
pub struct CodebaseContextConfigOutput {
    pub embedding_cadence: i32,
    pub embedding_config: EmbeddingConfig,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum CodebaseContextConfigResult {
    CodebaseContextConfigOutput(CodebaseContextConfigOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}
