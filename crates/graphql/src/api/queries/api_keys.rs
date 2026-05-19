use crate::{
    api::object_permissions::OwnerType, error::UserFacingError, request_context::RequestContext,
    response_context::ResponseContext, scalars::Time, schema,
};

#[derive(cynic::QueryVariables, Debug)]
pub struct ApiKeysVariables {
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "APIKeyPropertiesOutput")]
pub struct ApiKeyPropertiesOutput {
    pub api_keys: Vec<ApiKeyProperties>,
    pub response_context: ResponseContext,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "APIKeyProperties")]
pub struct ApiKeyProperties {
    pub uid: cynic::Id,
    pub name: String,
    pub key_suffix: String,
    pub owner_type: OwnerType,
    pub agent_info: Option<ApiKeyAgentInfo>,
    pub expires_at: Option<Time>,
    pub last_used_at: Option<Time>,
    pub created_at: Time,
}
#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "APIKeyAgentInfo")]
pub struct ApiKeyAgentInfo {
    pub name: String,
    pub uid: cynic::Id,
}

#[derive(cynic::InlineFragments, Debug)]
#[cynic(graphql_type = "APIKeyPropertiesResult")]
pub enum ApiKeyPropertiesResult {
    ApiKeyPropertiesOutput(ApiKeyPropertiesOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "RootQuery", variables = "ApiKeysVariables")]
pub struct ApiKeys {
    #[arguments(requestContext: $request_context)]
    pub api_keys: ApiKeyPropertiesResult,
}

crate::client::define_operation! {
    api_keys(ApiKeysVariables) -> ApiKeys;
}
