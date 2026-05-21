use super::list_ai_conversations::ListAIConversationsInput;
use crate::ai::AIConversationFormat;
use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;

#[derive(cynic::QueryVariables, Debug)]
pub struct GetAIConversationFormatVariables {
    pub input: ListAIConversationsInput,
    pub request_context: RequestContext,
}

/// A minimal fragment that only selects the `format` field from an AIConversation.
#[derive(cynic::QueryFragment, Debug, Clone)]
#[cynic(graphql_type = "AIConversation")]
pub struct AIConversationFormatOnly {
    pub conversation_id: cynic::Id,
    pub format: AIConversationFormat,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "ListAIConversationsOutput")]
pub struct GetAIConversationFormatOutput {
    pub conversations: Vec<AIConversationFormatOnly>,
    pub response_context: ResponseContext,
}

#[derive(cynic::InlineFragments, Debug)]
#[cynic(graphql_type = "ListAIConversationsResult")]
pub enum GetAIConversationFormatResult {
    ListAIConversationsOutput(GetAIConversationFormatOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootQuery",
    variables = "GetAIConversationFormatVariables"
)]
pub struct GetAIConversationFormat {
    #[arguments(input: $input, requestContext: $request_context)]
    #[cynic(rename = "listAIConversations")]
    pub list_ai_conversations: GetAIConversationFormatResult,
}

crate::client::define_operation! {
    get_ai_conversation_format(GetAIConversationFormatVariables) -> GetAIConversationFormat;
}
