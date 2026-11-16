use crate::request_context::RequestContext;
use crate::scalars::Time;
use crate::schema;

/*
query GetBlocksForUser($requestContext: RequestContext!) {
  user(requestContext: $requestContext) {
    ... on UserOutput {
      user {
        blocks {
          uid
          timeStartedTerm
          command
        }
      }
    }
  }
}
*/

#[derive(cynic::QueryVariables, Debug)]
pub struct GetBlocksForUserVariables {
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct UserOutput {
    pub user: User,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum UserResult {
    UserOutput(UserOutput),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "RootQuery", variables = "GetBlocksForUserVariables")]
pub struct GetBlocksForUser {
    #[arguments(requestContext: $request_context)]
    pub user: UserResult,
}
crate::client::define_operation! {
    get_blocks_for_user(GetBlocksForUserVariables) -> GetBlocksForUser;
}

#[derive(cynic::QueryFragment, Debug)]
pub struct User {
    pub blocks: Vec<Block>,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct Block {
    pub uid: cynic::Id,
    pub time_started_term: Option<Time>,
    pub command: Option<String>,
}
