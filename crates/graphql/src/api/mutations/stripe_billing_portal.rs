use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;

/*
mutation stripeBillingPortal($input: StripeBillingPortalInput!, $requestContext: RequestContext!) {
  stripeBillingPortal(input: $input, requestContext: $requestContext) {
    ... on StripeBillingPortalOutput {
      __typename
      url
      responseContext {
        serverVersion
      }
    }
    ... on UserFacingError {
      __typename
      error
      responseContext {
        serverVersion
      }
    }
  }
}
*/

#[derive(cynic::QueryVariables, Debug)]
pub struct StripeBillingPortalVariables {
    pub input: StripeBillingPortalInput,
    pub request_context: RequestContext,
}

#[derive(cynic::InputObject, Debug)]
pub struct StripeBillingPortalInput {
    pub team_uid: cynic::Id,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootMutation",
    variables = "StripeBillingPortalVariables"
)]
pub struct StripeBillingPortal {
    #[arguments(input: $input, requestContext: $request_context)]
    pub stripe_billing_portal: StripeBillingPortalResult,
}
crate::client::define_operation! {
    stripe_billing_portal(StripeBillingPortalVariables) -> StripeBillingPortal;
}

#[derive(cynic::QueryFragment, Debug)]
pub struct StripeBillingPortalOutput {
    pub url: String,
    pub response_context: ResponseContext,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum StripeBillingPortalResult {
    StripeBillingPortalOutput(StripeBillingPortalOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}
