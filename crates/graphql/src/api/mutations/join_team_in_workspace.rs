use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;

use super::join_team_with_team_discovery::TeamDiscoveryEntrypoint;

#[derive(cynic::QueryVariables, Debug)]
pub struct JoinTeamInWorkspaceVariables {
    pub input: JoinTeamInWorkspaceInput,
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootMutation",
    variables = "JoinTeamInWorkspaceVariables"
)]
pub struct JoinTeamInWorkspace {
    #[arguments(input: $input, requestContext: $request_context)]
    pub join_team_in_workspace: JoinTeamInWorkspaceResult,
}
crate::client::define_operation! {
    join_team_in_workspace(JoinTeamInWorkspaceVariables) -> JoinTeamInWorkspace;
}

#[derive(cynic::QueryFragment, Debug)]
pub struct JoinTeamInWorkspaceOutput {
    pub success: bool,
    pub response_context: ResponseContext,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum JoinTeamInWorkspaceResult {
    JoinTeamInWorkspaceOutput(JoinTeamInWorkspaceOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::InputObject, Debug)]
pub struct JoinTeamInWorkspaceInput {
    pub entrypoint: TeamDiscoveryEntrypoint,
    pub team_uid: cynic::Id,
}
