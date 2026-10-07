use super::task_attachments::{TaskAttachment, TaskInput};
use super::task_secrets::{TaskSecretsInput, TaskSecretsResult};
use crate::ai::AgentHarness;
use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::schema;

#[derive(cynic::QueryVariables, Debug)]
pub struct ExecutionBootstrapVariables {
    pub execution_id: cynic::Id,
    pub secrets_input: TaskSecretsInput,
    pub task_input: TaskInput,
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "RootQuery", variables = "ExecutionBootstrapVariables")]
pub struct ExecutionBootstrap {
    #[arguments(input: $secrets_input, requestContext: $request_context)]
    pub task_secrets: TaskSecretsResult,
    #[arguments(input: $task_input, requestContext: $request_context)]
    pub task: BootstrapTaskResult,
}

crate::client::define_operation! {
    execution_bootstrap(ExecutionBootstrapVariables) -> ExecutionBootstrap;
}

#[derive(cynic::InlineFragments, Debug)]
#[cynic(graphql_type = "TaskResult", variables = "ExecutionBootstrapVariables")]
pub enum BootstrapTaskResult {
    TaskOutput(Box<BootstrapTaskOutput>),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "TaskOutput", variables = "ExecutionBootstrapVariables")]
pub struct BootstrapTaskOutput {
    pub task: BootstrapTask,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Task", variables = "ExecutionBootstrapVariables")]
pub struct BootstrapTask {
    #[arguments(executionId: $execution_id)]
    pub execution_config: ExecutionConfiguration,
    pub attachments: Vec<TaskAttachment>,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct ExecutionConfiguration {
    pub task_id: cynic::Id,
    pub execution_id: cynic::Id,
    pub conversation_id: Option<cynic::Id>,
    pub parent_run_id: Option<cynic::Id>,
    pub team_id: Option<cynic::Id>,
    pub harness: AgentHarness,
    pub model_id: Option<String>,
    pub reasoning_level: Option<String>,
    pub profile_id: Option<cynic::Id>,
    pub mcp_servers_json: String,
    pub skills: Vec<ExecutionSkillSpec>,
    pub factory_skill_dirs: Vec<String>,
    pub computer_use_enabled: bool,
    pub computer_use_model_id: Option<String>,
    pub inference_providers: Option<InferenceProviderClientConfigs>,
    pub repositories: Vec<ExecutionRepository>,
    pub setup_commands: Vec<String>,
    pub providers: Option<CloudProviderClientConfigs>,
    pub session_sharing_acls: Vec<SessionSharingAclSpec>,
    pub skip_initial_turn: bool,
    pub idle_on_complete_seconds: Option<i32>,
    pub idle_on_fail_seconds: Option<i32>,
    pub snapshot_disabled: bool,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct ExecutionSkillSpec {
    pub spec: String,
}

#[derive(cynic::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodeForge {
    #[cynic(rename = "GITHUB")]
    GitHub,
    #[cynic(rename = "GITLAB")]
    GitLab,
    #[cynic(rename = "AZURE_DEVOPS")]
    AzureDevOps,
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecutionRepositoryRefType {
    #[cynic(rename = "COMMIT_SHA")]
    CommitSha,
    #[cynic(rename = "BRANCH")]
    Branch,
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct ExecutionRepositoryRef {
    #[cynic(rename = "type")]
    pub type_: ExecutionRepositoryRefType,
    pub value: String,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct SourceRepo {
    pub code_forge: CodeForge,
    pub owner: String,
    pub repo: String,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct ExecutionRepository {
    pub forge: CodeForge,
    pub owner: String,
    pub name: String,
    #[cynic(rename = "ref")]
    pub ref_: Option<ExecutionRepositoryRef>,
    pub clone_from: Option<SourceRepo>,
    pub preserve_origin: bool,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct CloudProviderClientConfigs {
    pub gcp: Option<GcpCloudProviderClientConfig>,
    pub aws: Option<AwsCloudProviderClientConfig>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "GCPCloudProviderClientConfig")]
pub struct GcpCloudProviderClientConfig {
    pub project_number: String,
    pub pool_id: String,
    pub provider_id: String,
    pub service_account_email: Option<String>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "AWSCloudProviderClientConfig")]
pub struct AwsCloudProviderClientConfig {
    pub role_arn: String,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct InferenceProviderClientConfigs {
    pub aws_bedrock: Option<AwsBedrockInferenceClientConfig>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "AWSBedrockInferenceClientConfig")]
pub struct AwsBedrockInferenceClientConfig {
    pub role_arn: String,
    pub region: Option<String>,
}

#[derive(cynic::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionSharingSubjectType {
    #[cynic(rename = "TEAM")]
    Team,
    #[cynic(rename = "USER_EMAIL")]
    UserEmail,
    #[cynic(rename = "PUBLIC")]
    Public,
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionSharingAccessLevel {
    #[cynic(rename = "VIEW")]
    View,
    #[cynic(rename = "EDIT")]
    Edit,
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct SessionSharingAclSpec {
    pub subject_type: SessionSharingSubjectType,
    pub email: Option<String>,
    pub access: SessionSharingAccessLevel,
}

#[cfg(test)]
#[path = "execution_config_tests.rs"]
mod tests;
