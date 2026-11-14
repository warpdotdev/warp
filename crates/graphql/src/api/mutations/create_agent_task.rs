use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;

#[derive(cynic::QueryVariables, Debug)]
pub struct CreateAgentTaskVariables {
    pub input: CreateAgentTaskInput,
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "RootMutation", variables = "CreateAgentTaskVariables")]
pub struct CreateAgentTask {
    #[arguments(input: $input, requestContext: $request_context)]
    pub create_agent_task: CreateAgentTaskResult,
}

crate::client::define_operation! {
    create_agent_task(CreateAgentTaskVariables) -> CreateAgentTask;
}

#[derive(cynic::InlineFragments, Debug)]
pub enum CreateAgentTaskResult {
    CreateAgentTaskOutput(CreateAgentTaskOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct CreateAgentTaskOutput {
    pub response_context: ResponseContext,
    pub task_id: cynic::Id,
}

#[derive(cynic::InputObject, Debug)]
#[cynic(graphql_type = "CreateAgentTaskInput")]
pub struct CreateAgentTaskInput {
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub local_github_contract_version: Option<i32>,
    pub prompt: String,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub environment_uid: Option<cynic::Id>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub parent_run_id: Option<cynic::Id>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub agent_config_snapshot: Option<String>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<String>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "RootMutation", variables = "CreateAgentTaskVariables")]
pub struct CreateLocalAgentTask {
    #[arguments(input: $input, requestContext: $request_context)]
    pub create_agent_task: CreateLocalAgentTaskResult,
}

#[derive(cynic::InlineFragments, Debug)]
#[cynic(graphql_type = "CreateAgentTaskResult")]
pub enum CreateLocalAgentTaskResult {
    CreateAgentTaskOutput(LocalAgentTaskOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "CreateAgentTaskOutput")]
pub struct LocalAgentTaskOutput {
    pub task_id: cynic::Id,
    pub github_credential_mode:
        Option<crate::queries::task_restricted_git_credentials::GitHubCredentialMode>,
    pub execution_id: Option<cynic::Id>,
    pub mcp_servers: Option<String>,
}

crate::client::define_operation! {
    create_local_agent_task(CreateAgentTaskVariables) -> CreateLocalAgentTask;
}
