use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;

/*
mutation TransferTeamOwnership($input: TransferTeamOwnershipInput!, $requestContext: RequestContext!) {
  transferTeamOwnership(input: $input, requestContext: $requestContext) {
    ... on TransferTeamOwnershipOutput {
      __typename
      success
      responseContext {
        serverVersion
      }
    }
    ... on UserFacingError {
      __typename
      responseContext {
        serverVersion
      }
      error {
        message
      }
    }
  }
}
*/

#[derive(cynic::InputObject, Debug)]
pub struct TransferTeamOwnershipInput {
    pub new_owner_email: String,
}

#[derive(cynic::QueryVariables, Debug)]
pub struct TransferTeamOwnershipVariables {
    pub input: TransferTeamOwnershipInput,
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct TransferTeamOwnershipOutput {
    pub success: bool,
    pub response_context: ResponseContext,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootMutation",
    variables = "TransferTeamOwnershipVariables"
)]
pub struct TransferTeamOwnership {
    #[arguments(input: $input, requestContext: $request_context)]
    pub transfer_team_ownership: TransferTeamOwnershipResult,
}
crate::client::define_operation! {
    transfer_team_ownership(TransferTeamOwnershipVariables) -> TransferTeamOwnership;
}

#[derive(cynic::InlineFragments, Debug)]
pub enum TransferTeamOwnershipResult {
    TransferTeamOwnershipOutput(TransferTeamOwnershipOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}
