use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::scalars::Time;
use crate::schema;

#[derive(cynic::QueryVariables, Debug)]
pub struct SimpleIntegrationsVariables {
    pub request_context: RequestContext,
    pub input: SimpleIntegrationsInput,
}

#[derive(cynic::InputObject, Debug)]
pub struct SimpleIntegrationsInput {
    pub providers: Vec<String>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "RootQuery", variables = "SimpleIntegrationsVariables")]
pub struct SimpleIntegrations {
    #[arguments(requestContext: $request_context, input: $input)]
    #[cynic(rename = "simpleIntegrations")]
    pub simple_integrations: SimpleIntegrationsResult,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct SimpleIntegrationsOutput {
    pub integrations: Vec<SimpleIntegration>,
    pub message: Option<String>,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct SimpleIntegration {
    pub provider_slug: String,
    pub description: String,
    pub connection_status: SimpleIntegrationConnectionStatus,
    pub integration_config: Option<ListedSimpleIntegrationConfig>,
    pub created_at: Option<Time>,
    pub updated_at: Option<Time>,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct ListedSimpleIntegrationConfig {
    pub environment_uid: String,
    pub base_prompt: String,
    pub model_id: String,
    pub mcp_servers_json: String,
}

#[derive(cynic::Enum, Debug, Clone, Copy)]
#[cynic(graphql_type = "SimpleIntegrationConnectionStatus")]
pub enum SimpleIntegrationConnectionStatus {
    NotConnected,
    ConnectionError,
    IntegrationNotConfigured,
    NotEnabled,
    Active,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum SimpleIntegrationsResult {
    SimpleIntegrationsOutput(SimpleIntegrationsOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

crate::client::define_operation! {
    SimpleIntegrations(SimpleIntegrationsVariables) -> SimpleIntegrations;
}
