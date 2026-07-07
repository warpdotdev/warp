use crate::ai::RequestLimitInfo;
use crate::billing::{BonusGrant, BonusGrantsInfo};
use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::schema;

/*
query GetRequestLimitInfo($requestContext: RequestContext!) {
  user(requestContext: $requestContext) {
    ... on UserOutput {
      user {
        workspaces {
          uid
          bonusGrantsInfo {
            grants {
              createdAt
              costCents
              expiration
              grantType
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
