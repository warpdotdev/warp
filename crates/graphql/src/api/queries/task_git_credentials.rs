use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::schema;

/// A GraphQL query to fetch git credentials for a specific task.
///
/// This query is used by Agent Mode tasks to retrieve fresh provider credentials that
/// the driver uses to configure git and supported provider CLIs, and to refresh those
/// credentials periodically so long-running agents retain repository access.
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
    /// Opts into a response where one forge's credential is fresh and
    /// another's failed. The driver merges rather than rebuilding its
    /// credential stores, so it can accept a partial list.
    pub accepts_partial_refresh: Option<bool>,
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
    /// Hosts whose credential could not be issued this cycle. Distinct from a
    /// host that is merely absent from `credentials`, which needs none.
    pub failed_hosts: Vec<String>,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct TaskGitCredential {
    pub token: String,
    pub username: Option<String>,
    pub email: Option<String>,
    pub host: String,
}
