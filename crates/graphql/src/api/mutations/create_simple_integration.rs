use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::schema;

#[derive(cynic::QueryVariables, Debug)]
pub struct CreateSimpleIntegrationVariables {
    pub config: SimpleIntegrationConfig,
    pub enabled: bool,
    pub integration_type: String,
    pub is_update: bool,
    pub request_context: RequestContext,
}

#[derive(cynic::InputObject, Debug)]
pub struct SimpleIntegrationConfig {
    // For these fields, None means "don't change".
    // For base_prompt/environment_uid/model_id, Some("") means "clear".
    // Note: mcp_servers_json is treated as patch data; on update, an empty string is a no-op.
    pub base_prompt: Option<String>,
    pub environment_uid: Option<String>,
    pub model_id: Option<String>,
    pub mcp_servers_json: Option<String>,
    pub remove_mcp_server_names: Option<Vec<String>>,
    pub worker_host: Option<String>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootMutation",
    variables = "CreateSimpleIntegrationVariables"
)]
pub struct CreateSimpleIntegration {
    #[arguments(input: { config: $config, enabled: $enabled, integrationType: $integration_type, isUpdate: $is_update }, requestContext: $request_context)]
    pub create_simple_integration: CreateSimpleIntegrationResult,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct CreateSimpleIntegrationOutput {
    pub auth_url: Option<String>,
    pub success: bool,
    pub message: String,
    #[cynic(rename = "txId")]
    pub tx_id: Option<cynic::Id>,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum CreateSimpleIntegrationResult {
    CreateSimpleIntegrationOutput(CreateSimpleIntegrationOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

crate::client::define_operation! {
    CreateSimpleIntegration(CreateSimpleIntegrationVariables) -> CreateSimpleIntegration;
}
