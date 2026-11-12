use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::scalars::Time;
use crate::schema;

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootQuery",
    variables = "RestrictedTaskGitCredentialsVariables"
)]
pub struct RestrictedTaskGitCredentials {
    #[arguments(input: $input, requestContext: $request_context)]
    pub task_git_credentials: RestrictedTaskGitCredentialsResult,
}

crate::client::define_operation! {
    restricted_task_git_credentials(RestrictedTaskGitCredentialsVariables) -> RestrictedTaskGitCredentials;
}

#[derive(cynic::QueryVariables, Debug)]
pub struct RestrictedTaskGitCredentialsVariables {
    pub input: RestrictedTaskGitCredentialsInput,
    pub request_context: RequestContext,
}

#[derive(cynic::InputObject, Debug)]
#[cynic(graphql_type = "TaskGitCredentialsInput")]
pub struct RestrictedTaskGitCredentialsInput {
    pub task_id: cynic::Id,
    pub workload_token: String,
    pub accepts_partial_refresh: Option<bool>,
    pub use_factory_repositories: Option<bool>,
    pub github_credential_contract_version: Option<i32>,
}

#[derive(cynic::InlineFragments, Debug)]
#[cynic(graphql_type = "TaskGitCredentialsResult")]
pub enum RestrictedTaskGitCredentialsResult {
    TaskGitCredentialsOutput(RestrictedTaskGitCredentialsOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "TaskGitCredentialsOutput")]
pub struct RestrictedTaskGitCredentialsOutput {
    pub credentials: Vec<RestrictedTaskGitCredential>,
    pub failed_hosts: Vec<String>,
    pub github_credential_mode: GitHubCredentialMode,
}

#[derive(cynic::Enum, Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum GitHubCredentialMode {
    #[default]
    Legacy,
    RestrictedInstallation,
}

#[derive(cynic::QueryFragment, Clone)]
#[cynic(graphql_type = "TaskGitCredential")]
pub struct RestrictedTaskGitCredential {
    pub token: String,
    pub username: Option<String>,
    pub email: Option<String>,
    pub host: String,
    pub expires_at: Option<Time>,
    pub repository_scope: Option<String>,
}

impl std::fmt::Debug for RestrictedTaskGitCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RestrictedTaskGitCredential")
            .field("host", &self.host)
            .finish_non_exhaustive()
    }
}
