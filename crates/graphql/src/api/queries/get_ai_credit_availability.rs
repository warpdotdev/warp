use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::schema;

/*
query GetAICreditAvailability($requestContext: RequestContext!) {
  user(requestContext: $requestContext) {
    ... on UserOutput {
      user {
        aiCreditAvailability {
          available
          denialReason
          creditSource
        }
      }
    }
    ... on UserFacingError {
      error {
        message
      }
    }
  }
}
*/

#[derive(cynic::QueryVariables, Debug)]
pub struct GetAICreditAvailabilityVariables {
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct UserOutput {
    pub user: User,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct User {
    pub ai_credit_availability: AICreditAvailability,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct AICreditAvailability {
    pub available: bool,
    pub denial_reason: AICreditAvailabilityDenialReason,
    pub credit_source: Option<AICreditAvailabilitySource>,
}

#[derive(cynic::Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum AICreditAvailabilityDenialReason {
    None,
    OutOfCredits,
    Delinquent,
    EnterpriseTeamSpendLimitHit,
    EnterprisePerUserSpendLimitHit,
    EnterpriseWorkspaceSpendLimitHit,
}

#[derive(cynic::Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum AICreditAvailabilitySource {
    BaseLimit,
    BonusGrant,
    Payg,
    Overage,
    AmbientBonusGrant,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "RootQuery", variables = "GetAICreditAvailabilityVariables")]
pub struct GetAICreditAvailability {
    #[arguments(requestContext: $request_context)]
    pub user: UserResult,
}
crate::client::define_operation! {
    get_ai_credit_availability(GetAICreditAvailabilityVariables) -> GetAICreditAvailability;
}

#[derive(cynic::InlineFragments, Debug)]
pub enum UserResult {
    UserOutput(UserOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}
