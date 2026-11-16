use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::scalars::Time;
use crate::schema;

#[derive(cynic::QueryVariables, Debug)]
pub struct GetCloudEnvironmentsQueryVariables {
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootQuery",
    variables = "GetCloudEnvironmentsQueryVariables"
)]
pub struct GetCloudEnvironmentsQuery {
    #[arguments(requestContext: $request_context)]
    pub get_cloud_environments: GetCloudEnvironmentsResult,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct GetCloudEnvironmentsOutput {
    pub __typename: String,
    pub cloud_environments: Vec<CloudEnvironment>,
    pub response_context: ResponseContext,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct ResponseContext {
    pub server_version: Option<String>,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct CloudEnvironment {
    pub config: CloudEnvironmentConfig,
    pub last_editor: Option<PublicUserProfile>,
    pub creator: Option<PublicUserProfile2>,
    pub last_task_created: Option<AgentTask>,
    pub last_updated: Time,
    pub uid: cynic::Id,
    pub scope: Space,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "AgentTask")]
pub struct AgentTask {
    pub created_at: Time,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct Space {
    #[cynic(rename = "type")]
    pub type_: SpaceType,
    pub uid: cynic::Id,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "PublicUserProfile")]
pub struct PublicUserProfile2 {
    pub uid: String,
    pub email: Option<String>,
    pub photo_url: Option<String>,
    pub display_name: Option<String>,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct PublicUserProfile {
    pub uid: String,
    pub photo_url: Option<String>,
    pub email: Option<String>,
    pub display_name: Option<String>,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct CloudEnvironmentConfig {
    pub setup_commands: Option<Vec<String>>,
    pub name: String,
    pub github_repos: Vec<GitHubRepo>,
    pub docker_image: String,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct GitHubRepo {
    pub repo: String,
    pub owner: String,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum GetCloudEnvironmentsResult {
    UserFacingError(UserFacingError),
    GetCloudEnvironmentsOutput(GetCloudEnvironmentsOutput),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::Enum, Clone, Copy, Debug)]
pub enum SpaceType {
    #[cynic(rename = "Team")]
    Team,
    #[cynic(rename = "User")]
    User,
}

#[derive(cynic::InputObject, Debug)]
pub struct ClientContext<'a> {
    pub version: Option<&'a str>,
}

#[derive(cynic::InputObject, Debug)]
#[cynic(graphql_type = "OSContext")]
pub struct Oscontext<'a> {
    pub category: Option<&'a str>,
    pub linux_kernel_version: Option<&'a str>,
    pub name: Option<&'a str>,
    pub version: Option<&'a str>,
}

crate::client::define_operation! {
    get_cloud_environments(GetCloudEnvironmentsQueryVariables) -> GetCloudEnvironmentsQuery;
}
