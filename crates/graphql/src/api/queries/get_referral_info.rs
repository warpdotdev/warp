use crate::request_context::RequestContext;
use crate::schema;

/*
query GetReferralInfo($requestContext: RequestContext!) {
  user(requestContext: $requestContext) {
    ... on UserOutput {
      user {
        referrals {
          referralCode
          numberClaimed
          isReferred
        }
      }
    }
  }
}
*/

#[derive(cynic::QueryVariables, Debug)]
pub struct GetReferralInfoVariables {
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct UserOutput {
    pub user: User,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct User {
    pub referrals: ReferralInfo,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum UserResult {
    UserOutput(UserOutput),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "RootQuery", variables = "GetReferralInfoVariables")]
pub struct GetReferralInfo {
    #[arguments(requestContext: $request_context)]
    pub user: UserResult,
}
crate::client::define_operation! {
    get_referral_info(GetReferralInfoVariables) -> GetReferralInfo;
}

#[derive(cynic::QueryFragment, Debug)]
pub struct ReferralInfo {
    pub referral_code: String,
    pub number_claimed: i32,
    pub is_referred: bool,
}
