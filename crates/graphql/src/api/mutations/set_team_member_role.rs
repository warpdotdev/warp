use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;
use crate::workspace::MembershipRole;

/*
mutation SetTeamMemberRole($input: SetTeamMemberRoleInput!, $requestContext: RequestContext!) {
  setTeamMemberRole(input: $input, requestContext: $requestContext) {
    ... on SetTeamMemberRoleOutput {
      __typename
      success
      responseContext {
        serverVersion
      }
    }
    ... on UserFacingError {
      __typename
      responseContext {
        serverVersion
      }
      error {
        message
      }
    }
  }
}
*/

#[derive(cynic::InputObject, Debug)]
pub struct SetTeamMemberRoleInput {
    pub role: MembershipRole,
    pub team_uid: cynic::Id,
    pub user_uid: cynic::Id,
}

#[derive(cynic::QueryVariables, Debug)]
pub struct SetTeamMemberRoleVariables {
    pub input: SetTeamMemberRoleInput,
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct SetTeamMemberRoleOutput {
    pub success: bool,
    pub response_context: ResponseContext,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootMutation",
    variables = "SetTeamMemberRoleVariables"
)]
pub struct SetTeamMemberRole {
    #[arguments(input: $input, requestContext: $request_context)]
    pub set_team_member_role: SetTeamMemberRoleResult,
}
crate::client::define_operation! {
    set_team_member_role(SetTeamMemberRoleVariables) -> SetTeamMemberRole;
}

#[derive(cynic::InlineFragments, Debug)]
pub enum SetTeamMemberRoleResult {
    SetTeamMemberRoleOutput(SetTeamMemberRoleOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}
