use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;

#[derive(cynic::InputObject, Debug)]
pub struct PurchaseAddonCreditsInput {
    pub credits: i32,
    /// Optional server-side: omitting it lets the server auto-create a
    /// personal team for free plan purchasers.
    pub team_uid: Option<cynic::Id>,
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
    PurchaseAddonCreditsCheckoutOutput(PurchaseAddonCreditsCheckoutOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

/// The purchase was charged synchronously and credits were granted.
#[derive(cynic::QueryFragment, Debug)]
pub struct PurchaseAddonCreditsOutput {
    pub success: bool,
    pub response_context: ResponseContext,
}

/// The purchase could not be charged synchronously (no saved payment method):
/// the user must complete checkout in the browser at `checkout_url`. Credits
/// are granted via webhook after checkout completes.
#[derive(cynic::QueryFragment, Debug)]
pub struct PurchaseAddonCreditsCheckoutOutput {
    pub checkout_url: String,
    pub response_context: ResponseContext,
}
