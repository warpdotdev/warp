use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::schema;
use crate::user::DiscoverableTeamData;

/*
query GetDiscoverableTeams($requestContext: RequestContext!) {
  user(requestContext: $requestContext) {
    ... on UserOutput {
      user {
        discoverableTeams {
          name
          numMembers
          teamAcceptingInvites
          teamUid
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
pub struct GetDiscoverableTeamsVariables {
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct UserOutput {
    pub user: User,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct User {
    pub discoverable_teams: Vec<DiscoverableTeamData>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootQuery",
    variables = "GetDiscoverableTeamsVariables"
)]
pub struct GetDiscoverableTeams {
    #[arguments(requestContext: $request_context)]
    pub user: UserResult,
}
crate::client::define_operation! {
    get_discoverable_teams(GetDiscoverableTeamsVariables) -> GetDiscoverableTeams;
}

#[derive(cynic::InlineFragments, Debug)]
pub enum UserResult {
    UserOutput(UserOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}
