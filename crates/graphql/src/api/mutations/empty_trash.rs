use crate::error::UserFacingError;
use crate::object_permissions::Owner;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;

/*
mutation EmptyTrash($input: EmptyTrashInput!, $requestContext: RequestContext!) {
  emptyTrash(input: $input, requestContext: $requestContext) {
    ... on EmptyTrashOutput {
      deletedUids
      responseContext {
        serverVersion
      }
      success
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
pub struct EmptyTrashVariables {
    pub input: EmptyTrashInput,
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "RootMutation", variables = "EmptyTrashVariables")]
pub struct EmptyTrash {
    #[arguments(input: $input, requestContext: $request_context)]
    pub empty_trash: EmptyTrashResult,
}
crate::client::define_operation! {
    empty_trash(EmptyTrashVariables) -> EmptyTrash;
}

#[derive(cynic::QueryFragment, Debug)]
pub struct EmptyTrashOutput {
    pub deleted_uids: Vec<cynic::Id>,
    pub response_context: ResponseContext,
    pub success: bool,
}
#[derive(cynic::InlineFragments, Debug)]
pub enum EmptyTrashResult {
    EmptyTrashOutput(EmptyTrashOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::InputObject, Debug)]
pub struct EmptyTrashInput {
    pub owner: Owner,
}
