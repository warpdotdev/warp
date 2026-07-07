use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;

/*
mutation RenameTeam($input: RenameTeamInput!, $requestContext: RequestContext!) {
  renameTeam(input: $input, requestContext: $requestContext) {
    ... on RenameTeamOutput {
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
pub struct RenameTeamVariables {
    pub input: RenameTeamInput,
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "RootMutation", variables = "RenameTeamVariables")]
pub struct RenameTeam {
    #[arguments(input: $input, requestContext: $request_context)]
    pub rename_team: RenameTeamResult,
}
crate::client::define_operation! {
    rename_team(RenameTeamVariables) -> RenameTeam;
}

#[derive(cynic::QueryFragment, Debug)]
pub struct RenameTeamOutput {
    pub success: bool,
    pub response_context: ResponseContext,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum RenameTeamResult {
    RenameTeamOutput(RenameTeamOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::InputObject, Debug)]
pub struct RenameTeamInput {
    pub new_name: String,
    pub team_uid: cynic::Id,
}
