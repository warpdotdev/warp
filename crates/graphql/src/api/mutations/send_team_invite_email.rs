use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;

/*
mutation SendTeamInviteEmail($input: SendTeamInviteEmailInput!, $requestContext: RequestContext!) {
  sendTeamInviteEmail(input: $input, requestContext: $requestContext) {
    ... on SendTeamInviteEmailOutput {
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
pub struct SendTeamInviteEmailVariables {
    pub input: SendTeamInviteEmailInput,
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct SendTeamInviteEmailOutput {
    pub success: bool,
    pub response_context: ResponseContext,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootMutation",
    variables = "SendTeamInviteEmailVariables"
)]
pub struct SendTeamInviteEmail {
    #[arguments(input: $input, requestContext: $request_context)]
    pub send_team_invite_email: SendTeamInviteEmailResult,
}
crate::client::define_operation! {
    send_team_invite_email(SendTeamInviteEmailVariables) -> SendTeamInviteEmail;
}

#[derive(cynic::InlineFragments, Debug)]
pub enum SendTeamInviteEmailResult {
    SendTeamInviteEmailOutput(SendTeamInviteEmailOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::InputObject, Debug)]
pub struct SendTeamInviteEmailInput {
    pub email: String,
    pub team_uid: cynic::Id,
}
