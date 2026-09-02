use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;

/*
mutation ClaimFeatureIntroImpression($input: ClaimFeatureIntroImpressionInput!, $requestContext: RequestContext!) {
  claimFeatureIntroImpression(input: $input, requestContext: $requestContext) {
    ... on ClaimFeatureIntroImpressionOutput {
      claimed
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
    variables = "ClaimFeatureIntroImpressionVariables"
)]
pub struct ClaimFeatureIntroImpression {
    #[arguments(input: $input, requestContext: $request_context)]
    pub claim_feature_intro_impression: ClaimFeatureIntroImpressionResult,
}
crate::client::define_operation! {
    claim_feature_intro_impression(ClaimFeatureIntroImpressionVariables) -> ClaimFeatureIntroImpression;
}

#[derive(cynic::QueryVariables, Debug)]
pub struct ClaimFeatureIntroImpressionVariables {
    pub input: ClaimFeatureIntroImpressionInput,
    pub request_context: RequestContext,
}

#[derive(cynic::InputObject, Debug)]
pub struct ClaimFeatureIntroImpressionInput {
    pub intro_key: String,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct ClaimFeatureIntroImpressionOutput {
    pub claimed: bool,
    pub response_context: ResponseContext,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum ClaimFeatureIntroImpressionResult {
    ClaimFeatureIntroImpressionOutput(ClaimFeatureIntroImpressionOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}
