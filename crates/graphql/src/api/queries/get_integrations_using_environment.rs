use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::schema;

#[derive(cynic::QueryVariables, Debug)]
pub struct GetIntegrationsUsingEnvironmentVariables {
    pub request_context: RequestContext,
    pub input: GetIntegrationsUsingEnvironmentInput,
}

#[derive(cynic::InputObject, Debug)]
pub struct GetIntegrationsUsingEnvironmentInput {
    pub environment_id: String,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootQuery",
    variables = "GetIntegrationsUsingEnvironmentVariables"
)]
pub struct GetIntegrationsUsingEnvironment {
    #[arguments(requestContext: $request_context, input: $input)]
    #[cynic(rename = "getIntegrationsUsingEnvironment")]
    pub get_integrations_using_environment: GetIntegrationsUsingEnvironmentResult,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct GetIntegrationsUsingEnvironmentOutput {
    pub provider_names: Vec<String>,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum GetIntegrationsUsingEnvironmentResult {
    GetIntegrationsUsingEnvironmentOutput(GetIntegrationsUsingEnvironmentOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

crate::client::define_operation! {
    GetIntegrationsUsingEnvironment(GetIntegrationsUsingEnvironmentVariables) -> GetIntegrationsUsingEnvironment;
}
