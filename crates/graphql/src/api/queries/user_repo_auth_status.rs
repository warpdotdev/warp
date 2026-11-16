use crate::request_context::RequestContext;
use crate::schema;

#[derive(cynic::QueryVariables, Debug)]
pub struct UserRepoAuthStatusVariables {
    pub request_context: RequestContext,
    pub input: UserRepoAuthStatusInput,
}

#[derive(cynic::InputObject, Debug)]
pub struct UserRepoAuthStatusInput {
    pub repos: Vec<RepoInput>,
}

#[derive(cynic::InputObject, Debug)]
pub struct RepoInput {
    pub owner: String,
    pub repo: String,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct UserRepoAuthStatusOutput {
    pub statuses: Vec<RepoResult>,
    pub auth_url: Option<String>,
    #[cynic(rename = "txId")]
    pub tx_id: Option<cynic::Id>,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct RepoResult {
    pub owner: String,
    pub repo: String,
    pub status: UserRepoAuthStatusEnum,
    pub is_public: bool,
}

#[derive(cynic::Enum, Debug, Clone, Copy)]
#[cynic(graphql_type = "UserRepoAuthStatus")]
pub enum UserRepoAuthStatusEnum {
    NoInstallationOrAccessForRepo,
    UserNotConnectedToGithub,
    Success,
}

#[derive(cynic::InlineFragments, Debug)]
#[allow(clippy::large_enum_variant)]
pub enum UserRepoAuthStatusResult {
    UserRepoAuthStatusOutput(UserRepoAuthStatusOutput),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "RootQuery", variables = "UserRepoAuthStatusVariables")]
pub struct UserRepoAuthStatus {
    #[arguments(requestContext: $request_context, input: $input)]
    pub user_repo_auth_status: UserRepoAuthStatusResult,
}

crate::client::define_operation! {
    user_repo_auth_status(UserRepoAuthStatusVariables) -> UserRepoAuthStatus;
}
