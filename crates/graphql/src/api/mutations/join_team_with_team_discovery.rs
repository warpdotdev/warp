use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;

/*
mutation JoinTeamWithTeamDiscovery($input: JoinTeamWithTeamDiscoveryInput!, $requestContext: RequestContext!) {
  joinTeamWithTeamDiscovery(input: $input, requestContext: $requestContext) {
    ... on JoinTeamWithTeamDiscoveryOutput {
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
pub struct JoinTeamWithTeamDiscoveryVariables {
    pub input: JoinTeamWithTeamDiscoveryInput,
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootMutation",
    variables = "JoinTeamWithTeamDiscoveryVariables"
)]
pub struct JoinTeamWithTeamDiscovery {
    #[arguments(input: $input, requestContext: $request_context)]
    pub join_team_with_team_discovery: JoinTeamWithTeamDiscoveryResult,
}
crate::client::define_operation! {
    join_team_with_team_discovery(JoinTeamWithTeamDiscoveryVariables) -> JoinTeamWithTeamDiscovery;
}

#[derive(cynic::QueryFragment, Debug)]
pub struct JoinTeamWithTeamDiscoveryOutput {
    pub success: bool,
    pub response_context: ResponseContext,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum JoinTeamWithTeamDiscoveryResult {
    JoinTeamWithTeamDiscoveryOutput(JoinTeamWithTeamDiscoveryOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::Enum, Clone, Copy, Debug)]
pub enum TeamDiscoveryEntrypoint {
    #[cynic(rename = "TeamSettings")]
    TeamSettings,
    #[cynic(rename = "WebSignup")]
    WebSignup,
}

#[derive(cynic::InputObject, Debug)]
pub struct JoinTeamWithTeamDiscoveryInput {
    pub entrypoint: TeamDiscoveryEntrypoint,
    pub team_uid: cynic::Id,
}
