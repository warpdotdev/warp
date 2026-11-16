use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;

/*
mutation StartChatGPTLink($requestContext: RequestContext!, $continueUrl: String!) {
  startChatGPTLink(requestContext: $requestContext, continueUrl: $continueUrl) {
    ... on StartChatGPTLinkOutput {
      authorizationUrl
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
#[cynic(graphql_type = "RootMutation", variables = "StartChatGPTLinkVariables")]
pub struct StartChatGPTLink {
    #[arguments(requestContext: $request_context, continueUrl: $continue_url)]
    #[cynic(rename = "startChatGPTLink")]
    pub start_chatgpt_link: StartChatGPTLinkResult,
}
crate::client::define_operation! {
    start_chatgpt_link(StartChatGPTLinkVariables) -> StartChatGPTLink;
}

#[derive(cynic::QueryVariables, Debug)]
pub struct StartChatGPTLinkVariables {
    pub request_context: RequestContext,
    pub continue_url: String,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "StartChatGPTLinkOutput")]
pub struct StartChatGPTLinkOutput {
    pub authorization_url: String,
    pub response_context: ResponseContext,
}

#[derive(cynic::InlineFragments, Debug)]
#[cynic(graphql_type = "StartChatGPTLinkResult")]
pub enum StartChatGPTLinkResult {
    StartChatGPTLinkOutput(StartChatGPTLinkOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}
