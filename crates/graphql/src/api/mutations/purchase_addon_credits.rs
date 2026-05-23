use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;

#[derive(cynic::InputObject, Debug)]
pub struct PurchaseAddonCreditsInput {
    pub credits: i32,
    pub team_uid: cynic::Id,
}

#[derive(cynic::QueryVariables, Debug)]
pub struct PurchaseAddonCreditsVariables {
    pub input: PurchaseAddonCreditsInput,
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootMutation",
    variables = "PurchaseAddonCreditsVariables"
)]
pub struct PurchaseAddonCredits {
    #[arguments(input: $input, requestContext: $request_context)]
    pub purchase_addon_credits: PurchaseAddonCreditsResult,
}
crate::client::define_operation! {
    purchase_addon_credits(PurchaseAddonCreditsVariables) -> PurchaseAddonCredits;
}

#[derive(cynic::InlineFragments, Debug)]
pub enum PurchaseAddonCreditsResult {
    PurchaseAddonCreditsOutput(PurchaseAddonCreditsOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct PurchaseAddonCreditsOutput {
    pub success: bool,
    pub response_context: ResponseContext,
}
