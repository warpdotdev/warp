use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;

/*
mutation SetUserIsOnboarded($requestContext: RequestContext!) {
  setUserIsOnboarded(requestContext: $requestContext) {
    ... on SetUserIsOnboardedOutput {
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

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootMutation",
    variables = "SetUserIsOnboardedVariables"
)]
pub struct SetUserIsOnboarded {
    #[arguments(requestContext: $request_context)]
    pub set_user_is_onboarded: SetUserIsOnboardedResult,
}
crate::client::define_operation! {
    set_user_is_onboarded(SetUserIsOnboardedVariables) -> SetUserIsOnboarded;
}

#[derive(cynic::QueryVariables, Debug)]
pub struct SetUserIsOnboardedVariables {
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct SetUserIsOnboardedOutput {
    pub response_context: ResponseContext,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum SetUserIsOnboardedResult {
    SetUserIsOnboardedOutput(SetUserIsOnboardedOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}
