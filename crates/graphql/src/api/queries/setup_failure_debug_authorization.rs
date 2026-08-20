use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;

/// A GraphQL query the sharer uses to authorize a no-token REMOTE-2661 debug agent prompt
/// against a retained environment-setup-failure session. Authenticated by the sharer's own
/// workload token, since it has no way to authenticate the requesting participant itself.
#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootQuery",
    variables = "SetupFailureDebugAuthorizationVariables"
)]
pub struct SetupFailureDebugAuthorization {
    #[arguments(input: $input, requestContext: $request_context)]
    pub setup_failure_debug_authorization: SetupFailureDebugAuthorizationResult,
}

crate::client::define_operation! {
    setup_failure_debug_authorization(SetupFailureDebugAuthorizationVariables) -> SetupFailureDebugAuthorization;
}

#[derive(cynic::QueryVariables, Debug)]
pub struct SetupFailureDebugAuthorizationVariables {
    pub input: SetupFailureDebugAuthorizationInput,
    pub request_context: RequestContext,
}

#[derive(cynic::InputObject, Debug)]
pub struct SetupFailureDebugAuthorizationInput {
    pub task_id: cynic::Id,
    pub workload_token: String,
    pub participant_firebase_uid: String,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum SetupFailureDebugAuthorizationResult {
    SetupFailureDebugAuthorizationOutput(SetupFailureDebugAuthorizationOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct SetupFailureDebugAuthorizationOutput {
    pub authorized: bool,
    pub response_context: ResponseContext,
}
