use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::scalars::Time;
use crate::schema;

/*
query GetChatGPTConnection($requestContext: RequestContext!) {
  user(requestContext: $requestContext) {
    ... on UserOutput {
      user {
        chatgptConnection {
          email
          connectedAt
          tokenSharingActive
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
pub struct GetChatGPTConnectionVariables {
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct UserOutput {
    pub user: User,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct User {
    pub chatgpt_connection: Option<ChatGPTConnection>,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
#[cynic(graphql_type = "ChatGPTConnection")]
pub struct ChatGPTConnection {
    pub email: Option<String>,
    pub connected_at: Time,
    pub token_sharing_active: bool,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootQuery",
    variables = "GetChatGPTConnectionVariables"
)]
pub struct GetChatGPTConnection {
    #[arguments(requestContext: $request_context)]
    pub user: UserResult,
}
crate::client::define_operation! {
    get_chatgpt_connection(GetChatGPTConnectionVariables) -> GetChatGPTConnection;
}

#[derive(cynic::InlineFragments, Debug)]
pub enum UserResult {
    UserOutput(UserOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}
