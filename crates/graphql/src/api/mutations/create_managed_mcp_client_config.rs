use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::scalars::Time;
use crate::schema;

#[derive(cynic::QueryVariables, Debug)]
pub struct CreateManagedMcpClientConfigVariables {
    pub input: CreateManagedMcpClientConfigInput,
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootMutation",
    variables = "CreateManagedMcpClientConfigVariables"
)]
pub struct CreateManagedMcpClientConfig {
    #[arguments(input: $input, requestContext: $request_context)]
    pub create_managed_mcp_client_config: CreateManagedMcpClientConfigResult,
}

crate::client::define_operation! {
    create_managed_mcp_client_config(CreateManagedMcpClientConfigVariables) -> CreateManagedMcpClientConfig;
}

#[derive(cynic::InputObject, Debug)]
pub struct CreateManagedMcpClientConfigInput {
    pub uid: cynic::Id,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct CreateManagedMcpClientConfigOutput {
    pub transport_kind: ManagedMcpTransportKind,
    pub mcp_config_json: String,
    pub proxy_url: Option<String>,
    pub proxy_token: Option<String>,
    pub authorization_header_name: Option<String>,
    pub authorization_header_value: Option<String>,
    pub expires_at: Option<Time>,
    pub response_context: ResponseContext,
}

#[derive(cynic::InlineFragments, Debug)]
#[allow(clippy::large_enum_variant)]
pub enum CreateManagedMcpClientConfigResult {
    CreateManagedMcpClientConfigOutput(CreateManagedMcpClientConfigOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::Enum, Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManagedMcpTransportKind {
    #[cynic(rename = "URL")]
    Url,
    #[cynic(rename = "COMMAND")]
    Command,
}
