use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;

/*
mutation GenerateMetadataForCommand($input: GenerateMetadataForCommandInput!, $requestContext: RequestContext!) {
  generateMetadataForCommand(input: $input, requestContext: $requestContext) {
    ... on GenerateMetadataForCommandOutput {
      responseContext {
        serverVersion
      }
      status {
        ... on GenerateMetadataForCommandSuccess {
          description
          parameterizedCommand
          parameters {
            description
            name
            value
          }
          title
        }
        ... on GenerateMetadataForCommandFailure {
          type
        }
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
pub struct GenerateMetadataForCommandVariables {
    pub input: GenerateMetadataForCommandInput,
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootMutation",
    variables = "GenerateMetadataForCommandVariables"
)]
pub struct GenerateMetadataForCommand {
    #[arguments(input: $input, requestContext: $request_context)]
    pub generate_metadata_for_command: GenerateMetadataForCommandResult,
}
crate::client::define_operation! {
    generate_metadata_for_command(GenerateMetadataForCommandVariables) -> GenerateMetadataForCommand;
}

#[derive(cynic::QueryFragment, Debug)]
pub struct GenerateMetadataForCommandSuccess {
    pub description: String,
    pub parameterized_command: String,
    pub parameters: Vec<GeneratedMetadataForCommand>,
    pub title: String,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct GeneratedMetadataForCommand {
    pub description: String,
    pub name: String,
    pub value: String,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct GenerateMetadataForCommandOutput {
    pub response_context: ResponseContext,
    pub status: GenerateMetadataForCommandStatus,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct GenerateMetadataForCommandFailure {
    #[cynic(rename = "type")]
    pub type_: GenerateMetadataForCommandFailureType,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum GenerateMetadataForCommandResult {
    GenerateMetadataForCommandOutput(GenerateMetadataForCommandOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum GenerateMetadataForCommandStatus {
    GenerateMetadataForCommandSuccess(GenerateMetadataForCommandSuccess),
    GenerateMetadataForCommandFailure(GenerateMetadataForCommandFailure),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::Enum, Clone, Copy, Debug)]
pub enum GenerateMetadataForCommandFailureType {
    AiProviderError,
    BadCommand,
    Other,
    RateLimited,
}

#[derive(cynic::InputObject, Debug)]
pub struct GenerateMetadataForCommandInput {
    pub command: String,
}
