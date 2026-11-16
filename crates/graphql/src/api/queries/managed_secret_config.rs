use crate::error::UserFacingError;
use crate::managed_secrets::ManagedSecretConfig;
use crate::request_context::RequestContext;
use crate::schema;

/// A GraphQL query to fetch all managed secret configuration for a user.
///
/// This is separate from the main user and workspace queries so that we do not make unnecessary
/// KMS calls when managed secrets are not in use.
#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootQuery",
    variables = "GetManagedSecretConfigVariables"
)]
pub struct GetManagedSecretConfig {
    #[arguments(requestContext: $request_context)]
    pub user: UserResult,
}

crate::client::define_operation! {
    get_managed_secret_config(GetManagedSecretConfigVariables) -> GetManagedSecretConfig;
}

#[derive(cynic::QueryVariables, Debug)]
pub struct GetManagedSecretConfigVariables {
    pub request_context: RequestContext,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum UserResult {
    UserOutput(UserOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct UserOutput {
    pub user: User,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct User {
    pub managed_secrets: Option<ManagedSecretConfig>,
    pub workspaces: Vec<Workspace>,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct Workspace {
    pub teams: Vec<Team>,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct Team {
    pub uid: cynic::Id,
    pub managed_secrets: Option<ManagedSecretConfig>,
}
