use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;

/*
mutation SetIsInviteLinkEnabled($input: SetIsInviteLinkEnabledInput!, $requestContext: RequestContext!) {
  setIsInviteLinkEnabled(input: $input, requestContext: $requestContext) {
    ... on SetIsInviteLinkEnabledOutput {
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
pub struct SetIsInviteLinkEnabledVariables {
    pub input: SetIsInviteLinkEnabledInput,
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct SetIsInviteLinkEnabledOutput {
    pub success: bool,
    pub response_context: ResponseContext,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootMutation",
    variables = "SetIsInviteLinkEnabledVariables"
)]
pub struct SetIsInviteLinkEnabled {
    #[arguments(input: $input, requestContext: $request_context)]
    pub set_is_invite_link_enabled: SetIsInviteLinkEnabledResult,
}
crate::client::define_operation! {
    set_is_invite_link_enabled(SetIsInviteLinkEnabledVariables) -> SetIsInviteLinkEnabled;
}

#[derive(cynic::InlineFragments, Debug)]
pub enum SetIsInviteLinkEnabledResult {
    SetIsInviteLinkEnabledOutput(SetIsInviteLinkEnabledOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::InputObject, Debug)]
pub struct SetIsInviteLinkEnabledInput {
    pub new_value: bool,
    pub team_uid: cynic::Id,
}
