use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;

/*
mutation SetTeamDiscoverability($input: SetTeamDiscoverabilityInput!, $requestContext: RequestContext!) {
  setTeamDiscoverability(input: $input, requestContext: $requestContext) {
    ... on SetTeamDiscoverabilityOutput {
      success
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

#[derive(cynic::QueryVariables, Debug)]
pub struct SetTeamDiscoverabilityVariables {
    pub input: SetTeamDiscoverabilityInput,
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct SetTeamDiscoverabilityOutput {
    pub success: bool,
    pub response_context: ResponseContext,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootMutation",
    variables = "SetTeamDiscoverabilityVariables"
)]
pub struct SetTeamDiscoverability {
    #[arguments(input: $input, requestContext: $request_context)]
    pub set_team_discoverability: SetTeamDiscoverabilityResult,
}
crate::client::define_operation! {
    set_team_discoverability(SetTeamDiscoverabilityVariables) -> SetTeamDiscoverability;
}

#[derive(cynic::InlineFragments, Debug)]
pub enum SetTeamDiscoverabilityResult {
    SetTeamDiscoverabilityOutput(SetTeamDiscoverabilityOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::InputObject, Debug)]
pub struct SetTeamDiscoverabilityInput {
    pub discoverable: bool,
    pub team_uid: cynic::Id,
}
