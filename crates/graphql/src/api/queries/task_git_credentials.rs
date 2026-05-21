use crate::{error::UserFacingError, request_context::RequestContext, schema};

/// A GraphQL query to fetch git credentials for a specific task.
///
/// This query is used by Agent Mode tasks to retrieve a fresh GitHub token that the
/// driver uses to configure git and the gh CLI, and to refresh those credentials
/// periodically so long-running agents retain GitHub access for their full duration.
#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "RootQuery", variables = "TaskGitCredentialsVariables")]
pub struct TaskGitCredentials {
    #[arguments(input: $input, requestContext: $request_context)]
    pub task_git_credentials: TaskGitCredentialsResult,
}

crate::client::define_operation! {
    task_git_credentials(TaskGitCredentialsVariables) -> TaskGitCredentials;
}

#[derive(cynic::QueryVariables, Debug)]
pub struct TaskGitCredentialsVariables {
    pub input: TaskGitCredentialsInput,
    pub request_context: RequestContext,
}

#[derive(cynic::InputObject, Debug)]
pub struct TaskGitCredentialsInput {
    pub task_id: cynic::Id,
    pub workload_token: String,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum TaskGitCredentialsResult {
    TaskGitCredentialsOutput(TaskGitCredentialsOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct TaskGitCredentialsOutput {
    pub credentials: Vec<TaskGitCredential>,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct TaskGitCredential {
    pub token: String,
    pub username: Option<String>,
    pub email: Option<String>,
    pub host: String,
}
