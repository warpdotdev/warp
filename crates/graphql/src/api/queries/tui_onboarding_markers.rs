use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::schema;

#[derive(cynic::QueryVariables, Debug)]
pub struct TuiOnboardingMarkersVariables {
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct TuiOnboardingMarkersOutput {
    pub first_zero_state_shown: bool,
    pub first_credit_gate_shown: bool,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum TuiOnboardingMarkersResult {
    TuiOnboardingMarkersOutput(TuiOnboardingMarkersOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootQuery",
    variables = "TuiOnboardingMarkersVariables"
)]
pub struct TuiOnboardingMarkers {
    #[arguments(requestContext: $request_context)]
    pub tui_onboarding_markers: TuiOnboardingMarkersResult,
}

crate::client::define_operation! {
    tui_onboarding_markers(TuiOnboardingMarkersVariables) -> TuiOnboardingMarkers;
}
