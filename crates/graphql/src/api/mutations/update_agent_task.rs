use crate::ai::{AgentTaskState, PlatformErrorCode};
use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;

#[derive(cynic::QueryVariables, Debug)]
pub struct UpdateAgentTaskVariables {
    pub input: UpdateAgentTaskInput,
    pub request_context: RequestContext,
}

#[derive(cynic::InputObject, Debug)]
pub struct UpdateAgentTaskInput {
    pub task_id: cynic::Id,

    // Important: for our server-side changeset logic to work, any fields which we aren't trying to
    // update must be omitted, NOT set to null.
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub task_state: Option<AgentTaskState>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<cynic::Id>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<cynic::Id>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub status_message: Option<AgentTaskStatusMessageInput>,
}

#[derive(cynic::InputObject, Debug)]
pub struct AgentTaskStatusMessageInput {
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<PlatformErrorCode>,
    pub message: String,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "RootMutation", variables = "UpdateAgentTaskVariables")]
pub struct UpdateAgentTask {
    #[arguments(input: $input, requestContext: $request_context)]
    pub update_agent_task: UpdateAgentTaskResult,
}

crate::client::define_operation! {
    update_agent_task(UpdateAgentTaskVariables) -> UpdateAgentTask;
}

#[derive(cynic::InlineFragments, Debug)]
pub enum UpdateAgentTaskResult {
    UpdateAgentTaskOutput(UpdateAgentTaskOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct UpdateAgentTaskOutput {
    pub response_context: ResponseContext,
}
