use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;

/*
mutation ProvideNegativeFeedbackResponseForAiConversation
($input: ProvideNegativeFeedbackResponseForAiConversationInput!, $requestContext: RequestContext!) {
  provideNegativeFeedbackResponseForAiConversation(input: $input, requestContext: $requestContext) {
    ... on RequestsRefundedOutput {
      requestsRefunded
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

#[derive(cynic::InputObject, Debug)]
pub struct ProvideNegativeFeedbackResponseForAiConversationInput {
    pub conversation_id: cynic::Id,
    pub request_ids: Vec<cynic::Id>,
}

#[derive(cynic::QueryVariables, Debug)]
pub struct ProvideNegativeFeedbackResponseForAiConversationVariables {
    pub input: ProvideNegativeFeedbackResponseForAiConversationInput,
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootMutation",
    variables = "ProvideNegativeFeedbackResponseForAiConversationVariables"
)]
pub struct ProvideNegativeFeedbackResponseForAiConversation {
    #[arguments(input: $input, requestContext: $request_context)]
    pub provide_negative_feedback_response_for_ai_conversation: RequestsRefundedResult,
}
crate::client::define_operation! {
    provide_negative_feedback_response_for_ai_conversation(ProvideNegativeFeedbackResponseForAiConversationVariables) -> ProvideNegativeFeedbackResponseForAiConversation;
}

#[derive(cynic::InlineFragments, Debug)]
pub enum RequestsRefundedResult {
    RequestsRefundedOutput(RequestsRefundedOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct RequestsRefundedOutput {
    pub requests_refunded: i32,
    pub response_context: ResponseContext,
}
