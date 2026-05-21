use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;

/*
mutation GenerateCommands($input: GenerateCommandsInput!, $requestContext: RequestContext!) {
  generateCommands(input: $input, requestContext: $requestContext) {
    ... on GenerateCommandsOutput {
      status {
        ... on GenerateCommandsSuccess {
          commands {
            command
            description
            parameters {
              description
              id
            }
          }
        }
        ... on GenerateCommandsFailure {
          type
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
pub struct GenerateCommandsVariables {
    pub input: GenerateCommandsInput,
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "RootMutation", variables = "GenerateCommandsVariables")]
pub struct GenerateCommands {
    #[arguments(input: $input, requestContext: $request_context)]
    pub generate_commands: GenerateCommandsResult,
}
crate::client::define_operation! {
    generate_commands(GenerateCommandsVariables) -> GenerateCommands;
}

#[derive(cynic::QueryFragment, Debug)]
pub struct GenerateCommandsSuccess {
    pub commands: Vec<GeneratedCommand>,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct GeneratedCommand {
    pub command: String,
    pub description: String,
    pub parameters: Vec<GeneratedCommandParameter>,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct GeneratedCommandParameter {
    pub description: String,
    pub id: String,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct GenerateCommandsOutput {
    pub status: GenerateCommandsStatus,
    pub response_context: ResponseContext,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct GenerateCommandsFailure {
    #[cynic(rename = "type")]
    pub type_: GenerateCommandsFailureType,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum GenerateCommandsResult {
    GenerateCommandsOutput(GenerateCommandsOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum GenerateCommandsStatus {
    GenerateCommandsSuccess(GenerateCommandsSuccess),
    GenerateCommandsFailure(GenerateCommandsFailure),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::Enum, Clone, Copy, Debug)]
pub enum GenerateCommandsFailureType {
    AiProviderError,
    BadPrompt,
    Other,
    RateLimited,
}

#[derive(cynic::InputObject, Debug)]
pub struct GenerateCommandsInput {
    pub prompt: String,
}
