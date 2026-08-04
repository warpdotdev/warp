use crate::scalars::Time;
use crate::schema;

#[derive(cynic::QueryVariables, Debug)]
pub struct CreateMockMcpClientConfigVariables {
    pub input: CreateMockMcpClientConfigInput,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootMutation",
    variables = "CreateMockMcpClientConfigVariables"
)]
pub struct CreateMockMcpClientConfig {
    #[arguments(input: $input)]
    #[cynic(rename = "createMockMCPClientConfig")]
    pub create_mock_mcp_client_config: MockMcpClientConfig,
}

crate::client::define_operation! {
    create_mock_mcp_client_config(CreateMockMcpClientConfigVariables) -> CreateMockMcpClientConfig;
}

#[derive(cynic::InputObject, Debug)]
#[cynic(graphql_type = "CreateMockMCPClientConfigInput")]
pub struct CreateMockMcpClientConfigInput {
    pub template: String,
    pub model_id: Option<String>,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
#[cynic(graphql_type = "MockMCPClientConfig")]
pub struct MockMcpClientConfig {
    pub mcp_url: String,
    pub token: String,
    pub expires_at: Time,
}
