use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;

#[derive(cynic::QueryVariables, Debug)]
pub struct DeleteAIConversationVariables {
    pub input: DeleteConversationInput,
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootMutation",
    variables = "DeleteAIConversationVariables"
)]
pub struct DeleteAIConversation {
    #[arguments(input: $input, requestContext: $request_context)]
    pub delete_conversation: DeleteConversationResult,
}
crate::client::define_operation! {
    delete_ai_conversation(DeleteAIConversationVariables) -> DeleteAIConversation;
}

#[derive(cynic::QueryFragment, Debug)]
pub struct DeleteConversationOutput {
    pub deleted_uid: cynic::Id,
    pub response_context: ResponseContext,
    pub success: bool,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum DeleteConversationResult {
    DeleteConversationOutput(DeleteConversationOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::InputObject, Debug)]
pub struct DeleteConversationInput {
    pub conversation_id: cynic::Id,
}
