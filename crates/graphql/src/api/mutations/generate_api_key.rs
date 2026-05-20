use crate::scalars::Time;
use crate::{
    error::UserFacingError, request_context::RequestContext, response_context::ResponseContext,
    schema,
};

use crate::queries::api_keys::ApiKeyProperties;

#[derive(cynic::QueryVariables, Debug)]
pub struct GenerateApiKeyVariables {
    pub input: GenerateApiKeyInput,
    pub request_context: RequestContext,
}

#[derive(cynic::InputObject, Debug)]
pub struct GenerateApiKeyInput {
    pub name: String,
    pub team_id: Option<cynic::Id>,
    pub agent_uid: Option<cynic::Id>,
    pub expires_at: Option<Time>,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct GenerateApiKeyOutput {
    pub raw_api_key: String,
    pub api_key: ApiKeyProperties,
    pub response_context: ResponseContext,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum GenerateApiKeyResult {
    GenerateApiKeyOutput(GenerateApiKeyOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "RootMutation", variables = "GenerateApiKeyVariables")]
pub struct GenerateApiKey {
    #[arguments(input: $input, requestContext: $request_context)]
    pub generate_api_key: GenerateApiKeyResult,
}

crate::client::define_operation! {
    generate_api_key(GenerateApiKeyVariables) -> GenerateApiKey;
}
