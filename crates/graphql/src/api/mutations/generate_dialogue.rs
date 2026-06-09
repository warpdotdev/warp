use crate::ai::RequestLimitInfo;
use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;

/*
mutation GenerateDialogue($input: GenerateDialogueInput!, $requestContext: RequestContext!) {
  generateDialogue(input: $input, requestContext: $requestContext) {
    ... on GenerateDialogueOutput {
      status {
        ... on GenerateDialogueSuccess {
          answer
          requestLimitInfo {
            isUnlimited
            nextRefreshTime
            requestLimit
            requestsUsedSinceLastRefresh
          }
          transcriptSummarized
          truncated
        }
        ... on GenerateDialogueFailure {
          requestLimitInfo {
            isUnlimited
            nextRefreshTime
            requestLimit
            requestsUsedSinceLastRefresh
          }
        }
      }
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
pub struct GenerateDialogueVariables {
    pub input: GenerateDialogueInput,
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "RootMutation", variables = "GenerateDialogueVariables")]
pub struct GenerateDialogue {
    #[arguments(input: $input, requestContext: $request_context)]
    pub generate_dialogue: GenerateDialogueResult,
}
crate::client::define_operation! {
    generate_dialogue(GenerateDialogueVariables) -> GenerateDialogue;
}

#[derive(cynic::QueryFragment, Debug)]
pub struct GenerateDialogueSuccess {
    pub answer: String,
    pub request_limit_info: RequestLimitInfo,
    pub transcript_summarized: bool,
    pub truncated: bool,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct GenerateDialogueOutput {
    pub status: GenerateDialogueStatus,
    pub response_context: ResponseContext,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct GenerateDialogueFailure {
    pub request_limit_info: RequestLimitInfo,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum GenerateDialogueResult {
    GenerateDialogueOutput(GenerateDialogueOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum GenerateDialogueStatus {
    GenerateDialogueSuccess(GenerateDialogueSuccess),
    GenerateDialogueFailure(GenerateDialogueFailure),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::InputObject, Debug)]
pub struct GenerateDialogueInput {
    pub prompt: String,
    pub transcript: Vec<TranscriptPart>,
}

#[derive(cynic::InputObject, Debug)]
pub struct TranscriptPart {
    pub assistant: String,
    pub user: String,
}
