use crate::error::UserFacingError;
use crate::queries::execution_config::{
    CloudProviderClientConfigs, ExecutionRepository, SourceRepo,
};
use crate::queries::task_git_credentials::TaskGitCredential;
use crate::queries::task_secrets::TaskSecretEntry;
use crate::request_context::RequestContext;
use crate::scalars::Time;
use crate::schema;

#[derive(cynic::QueryVariables)]
pub struct PrewarmedSandboxConfigVariables {
    pub input: PrewarmedSandboxConfigInput,
    pub request_context: RequestContext,
}

#[derive(cynic::InputObject)]
pub struct PrewarmedSandboxConfigInput {
    pub sandbox_id: cynic::Id,
}

#[derive(cynic::QueryFragment)]
#[cynic(
    graphql_type = "RootMutation",
    variables = "PrewarmedSandboxConfigVariables"
)]
pub struct PrewarmedSandboxConfig {
    #[arguments(input: $input, requestContext: $request_context)]
    pub prewarmed_sandbox_config: PrewarmedSandboxConfigResult,
}

crate::client::define_operation! {
    prewarmed_sandbox_config(PrewarmedSandboxConfigVariables) -> PrewarmedSandboxConfig;
}

#[derive(cynic::InlineFragments)]
pub enum PrewarmedSandboxConfigResult {
    PrewarmedSandboxConfiguration(Box<PrewarmedSandboxConfiguration>),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::QueryFragment)]
pub struct PrewarmedSandboxConfiguration {
    pub sandbox_id: cynic::Id,
    pub fingerprint: String,
    pub spec: PrewarmedSandboxSpec,
    pub environment_variables: Vec<PrewarmedSandboxVariable>,
    pub git_credentials: Vec<TaskGitCredential>,
    pub managed_secrets: Vec<TaskSecretEntry>,
    pub expires_at: Time,
}

#[derive(cynic::QueryFragment)]
pub struct PrewarmedSandboxSpec {
    pub harness: String,
    pub repositories: Vec<ExecutionRepository>,
    pub deferred_repositories: Vec<SourceRepo>,
    pub setup_commands: Vec<String>,
    pub mcp_servers_json: String,
    pub skills: Vec<String>,
    pub factory_skill_dirs: Vec<String>,
    pub computer_use_enabled: bool,
    pub providers: Option<CloudProviderClientConfigs>,
}

#[derive(cynic::QueryFragment)]
pub struct PrewarmedSandboxVariable {
    pub name: String,
    pub value: String,
}

#[cfg(test)]
#[path = "prewarmed_sandbox_config_tests.rs"]
mod tests;
