use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootMutation",
    variables = "ClaimFactoriesLaunchIntroVariables"
)]
pub struct ClaimFactoriesLaunchIntro {
    #[arguments(requestContext: $request_context)]
    pub claim_factories_launch_intro: ClaimFactoriesLaunchIntroResult,
}
crate::client::define_operation! {
    claim_factories_launch_intro(ClaimFactoriesLaunchIntroVariables) -> ClaimFactoriesLaunchIntro;
}

#[derive(cynic::QueryVariables, Debug)]
pub struct ClaimFactoriesLaunchIntroVariables {
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct ClaimFactoriesLaunchIntroOutput {
    pub claimed: bool,
    pub response_context: ResponseContext,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum ClaimFactoriesLaunchIntroResult {
    ClaimFactoriesLaunchIntroOutput(ClaimFactoriesLaunchIntroOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}
