use crate::error::UserFacingError;
use crate::object::CloudObjectEventEntrypoint;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;

/*
mutation RemoveUserFromTeam($input: RemoveUserFromTeamInput!, $requestContext: RequestContext!) {
  removeUserFromTeam(input: $input, requestContext: $requestContext) {
    ... on RemoveUserFromTeamOutput {
      success
      responseContext {
        serverVersion
      }
    }
    ... on UserFacingError {
      error {
        message
      }
      responseContext {
        serverVersion
      }
    }
  }
}
*/

#[derive(cynic::QueryVariables, Debug)]
pub struct RemoveUserFromTeamVariables {
    pub input: RemoveUserFromTeamInput,
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootMutation",
    variables = "RemoveUserFromTeamVariables"
)]
pub struct RemoveUserFromTeam {
    #[arguments(input: $input, requestContext: $request_context)]
    pub remove_user_from_team: RemoveUserFromTeamResult,
}
crate::client::define_operation! {
    remove_user_from_team(RemoveUserFromTeamVariables) -> RemoveUserFromTeam;
}

#[derive(cynic::QueryFragment, Debug)]
pub struct RemoveUserFromTeamOutput {
    pub success: bool,
    pub response_context: ResponseContext,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum RemoveUserFromTeamResult {
    RemoveUserFromTeamOutput(RemoveUserFromTeamOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::InputObject, Debug)]
pub struct RemoveUserFromTeamInput {
    pub entrypoint: CloudObjectEventEntrypoint,
    pub team_uid: cynic::Id,
    pub user_uid: cynic::Id,
}
