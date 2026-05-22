use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;

/*
mutation ResetInviteLinks($input: ResetInviteLinksInput!, $requestContext: RequestContext!) {
  resetInviteLinks(input: $input, requestContext: $requestContext) {
    ... on ResetInviteLinksOutput {
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
pub struct ResetInviteLinksVariables {
    pub input: ResetInviteLinksInput,
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "RootMutation", variables = "ResetInviteLinksVariables")]
pub struct ResetInviteLinks {
    #[arguments(input: $input, requestContext: $request_context)]
    pub reset_invite_links: ResetInviteLinksResult,
}
crate::client::define_operation! {
    reset_invite_links(ResetInviteLinksVariables) -> ResetInviteLinks;
}

#[derive(cynic::QueryFragment, Debug)]
pub struct ResetInviteLinksOutput {
    pub success: bool,
    pub response_context: ResponseContext,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum ResetInviteLinksResult {
    ResetInviteLinksOutput(ResetInviteLinksOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::InputObject, Debug)]
pub struct ResetInviteLinksInput {
    pub team_uid: cynic::Id,
}
