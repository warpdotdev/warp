use crate::ai::RequestLimitInfo;
use crate::billing::{BonusGrant, BonusGrantsInfo};
use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::scalars::Time;
use crate::schema;

/*
query GetRequestLimitInfo($requestContext: RequestContext!) {
  user(requestContext: $requestContext) {
    ... on UserOutput {
      user {
        workspaces {
          uid
          includedUsageInfo {
            includedUsageCredits
            usageCreditsUsedSinceLastRefresh
            includedUsageCents
            usageCentsUsedSinceLastRefresh
            nextRefreshTime
          }
          bonusGrantsInfo {
            grants {
              createdAt
              costCents
              expiration
              grantType
              scope
              reason
              userFacingMessage
              requestCreditsGranted
              requestCreditsRemaining
            }
            spendingInfo {
              currentMonthCreditsPurchased
              currentMonthPeriodEnd
              currentMonthSpendCents
            }
          }
        }
        requestLimitInfo {
          isUnlimited
          requestsUsedSinceLastRefresh
          requestLimit
          nextRefreshTime
          requestLimitRefreshDuration
        }
        bonusGrants {
          createdAt
          costCents
          expiration
          grantType
          scope
          reason
          userFacingMessage
          requestCreditsGranted
          requestCreditsRemaining
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
pub struct GetRequestLimitInfoVariables {
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct UserOutput {
    pub user: User,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Workspace")]
pub struct WorkspaceInfo {
    pub uid: cynic::Id,
    pub bonus_grants_info: BonusGrantsInfo,
    pub included_usage_info: Option<WorkspaceIncludedUsageInfo>,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct User {
    pub workspaces: Vec<WorkspaceInfo>,
    pub request_limit_info: RequestLimitInfo,
    pub bonus_grants: Vec<BonusGrant>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "RootQuery", variables = "GetRequestLimitInfoVariables")]
pub struct GetRequestLimitInfo {
    #[arguments(requestContext: $request_context)]
    pub user: UserResult,
}
crate::client::define_operation! {
    get_request_limit_info(GetRequestLimitInfoVariables) -> GetRequestLimitInfo;
}

#[derive(cynic::InlineFragments, Debug)]
pub enum UserResult {
    UserOutput(UserOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::QueryFragment, Clone, Debug)]
pub struct WorkspaceIncludedUsageInfo {
    pub included_usage_credits: f64,
    pub usage_credits_used_since_last_refresh: f64,
    pub included_usage_cents: f64,
    pub usage_cents_used_since_last_refresh: f64,
    pub next_refresh_time: Time,
}

#[cfg(test)]
#[path = "get_request_limit_info_tests.rs"]
mod tests;
