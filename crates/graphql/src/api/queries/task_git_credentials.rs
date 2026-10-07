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
    #[cynic(skip_serializing_if = "Option::is_none")]
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
    pub failed_hosts: Vec<String>,
}

/// Pre-#16215 operation: omits `acceptsPartialRefresh` and `failedHosts` so a
/// server that has not deployed those fields can still validate the query.
pub mod legacy {
    use super::TaskGitCredential;
    use crate::error::UserFacingError;
    use crate::request_context::RequestContext;
    use crate::schema;

    #[derive(cynic::QueryFragment, Debug)]
    #[cynic(
        graphql_type = "RootQuery",
        variables = "TaskGitCredentialsLegacyVariables"
    )]
    pub struct TaskGitCredentialsLegacy {
        #[arguments(input: $input, requestContext: $request_context)]
        pub task_git_credentials: TaskGitCredentialsLegacyResult,
    }

    crate::client::define_operation! {
        task_git_credentials_legacy(TaskGitCredentialsLegacyVariables) -> TaskGitCredentialsLegacy;
    }

    #[derive(cynic::QueryVariables, Debug)]
    pub struct TaskGitCredentialsLegacyVariables {
        pub input: TaskGitCredentialsLegacyInput,
        pub request_context: RequestContext,
    }

    #[derive(cynic::InputObject, Debug)]
    #[cynic(graphql_type = "TaskGitCredentialsInput")]
    pub struct TaskGitCredentialsLegacyInput {
        pub task_id: cynic::Id,
        pub workload_token: String,
    }

    #[derive(cynic::InlineFragments, Debug)]
    #[cynic(graphql_type = "TaskGitCredentialsResult")]
    pub enum TaskGitCredentialsLegacyResult {
        TaskGitCredentialsOutput(TaskGitCredentialsLegacyOutput),
        UserFacingError(UserFacingError),
        #[cynic(fallback)]
        Unknown,
    }

    #[derive(cynic::QueryFragment, Debug)]
    #[cynic(graphql_type = "TaskGitCredentialsOutput")]
    pub struct TaskGitCredentialsLegacyOutput {
        pub credentials: Vec<TaskGitCredential>,
    }
}

pub use legacy::{
    TaskGitCredentialsLegacy, TaskGitCredentialsLegacyInput, TaskGitCredentialsLegacyResult,
    TaskGitCredentialsLegacyVariables,
};

#[derive(cynic::QueryFragment, Debug)]
pub struct TaskGitCredential {
    pub token: String,
    pub username: Option<String>,
    pub email: Option<String>,
    pub host: String,
}

#[cfg(test)]
#[path = "task_git_credentials_tests.rs"]
mod tests;
