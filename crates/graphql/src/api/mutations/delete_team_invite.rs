use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;

/*
mutation DeleteTeamInvite(
  $input: DeleteTeamInviteInput!,
  $requestContext: RequestContext!
) {
  deleteTeamInvite(input:$input, requestContext: $requestContext) {
    ... on DeleteTeamInviteOutput {
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
pub struct DeleteTeamInviteVariables {
    pub input: DeleteTeamInviteInput,
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "RootMutation", variables = "DeleteTeamInviteVariables")]
pub struct DeleteTeamInvite {
    #[arguments(input: $input, requestContext: $request_context)]
    pub delete_team_invite: DeleteTeamInviteResult,
}
crate::client::define_operation! {
    delete_team_invite(DeleteTeamInviteVariables) -> DeleteTeamInvite;
}

#[derive(cynic::QueryFragment, Debug)]
pub struct DeleteTeamInviteOutput {
    pub success: bool,
    pub response_context: ResponseContext,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum DeleteTeamInviteResult {
    DeleteTeamInviteOutput(DeleteTeamInviteOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::InputObject, Debug)]
pub struct DeleteTeamInviteInput {
    pub email: String,
    pub team_uid: cynic::Id,
}
