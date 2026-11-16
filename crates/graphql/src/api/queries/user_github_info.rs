use crate::request_context::RequestContext;
use crate::schema;

#[derive(cynic::QueryVariables, Debug)]
pub struct UserGithubInfoVariables {
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct GithubConnectedOutput {
    pub username: Option<String>,
    pub installed_repos: Vec<RepoResult>,
    pub app_install_link: String,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct GithubAuthRequiredOutput {
    pub auth_url: String,
    #[cynic(rename = "txId")]
    pub tx_id: cynic::Id,
    pub app_install_link: String,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct RepoResult {
    pub owner: String,
    pub repo: String,
    pub is_public: bool,
}

#[derive(cynic::InlineFragments, Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum UserGithubInfoResult {
    GithubConnectedOutput(GithubConnectedOutput),
    GithubAuthRequiredOutput(GithubAuthRequiredOutput),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "RootQuery", variables = "UserGithubInfoVariables")]
pub struct UserGithubInfo {
    #[arguments(requestContext: $request_context)]
    pub user_github_info: UserGithubInfoResult,
}

crate::client::define_operation! {
    user_github_info(UserGithubInfoVariables) -> UserGithubInfo;
}
