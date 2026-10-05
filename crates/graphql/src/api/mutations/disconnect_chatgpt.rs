use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;

/*
mutation DisconnectChatGPT($requestContext: RequestContext!) {
  disconnectChatGPT(requestContext: $requestContext) {
    ... on DisconnectChatGPTOutput {
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

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootMutation",
    variables = "DisconnectChatGPTVariables"
)]
pub struct DisconnectChatGPT {
    #[arguments(requestContext: $request_context)]
    #[cynic(rename = "disconnectChatGPT")]
    pub disconnect_chatgpt: DisconnectChatGPTResult,
}
crate::client::define_operation! {
    disconnect_chatgpt(DisconnectChatGPTVariables) -> DisconnectChatGPT;
}

#[derive(cynic::QueryVariables, Debug)]
pub struct DisconnectChatGPTVariables {
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "DisconnectChatGPTOutput")]
pub struct DisconnectChatGPTOutput {
    pub response_context: ResponseContext,
}

#[derive(cynic::InlineFragments, Debug)]
#[cynic(graphql_type = "DisconnectChatGPTResult")]
pub enum DisconnectChatGPTResult {
    DisconnectChatGPTOutput(DisconnectChatGPTOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}
