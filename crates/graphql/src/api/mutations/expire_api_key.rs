use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::schema;

#[derive(cynic::QueryVariables, Debug)]
pub struct ExpireApiKeyVariables {
    pub key_uid: cynic::Id,
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "RootMutation", variables = "ExpireApiKeyVariables")]
pub struct ExpireApiKey {
    #[arguments(input: { keyUID: $key_uid }, requestContext: $request_context)]
    pub expire_api_key: ExpireApiKeyResult,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct ExpireApiKeyOutput {
    pub __typename: String,
    pub success: bool,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum ExpireApiKeyResult {
    ExpireApiKeyOutput(ExpireApiKeyOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

crate::client::define_operation! {
    expire_api_key(ExpireApiKeyVariables) -> ExpireApiKey;
}
