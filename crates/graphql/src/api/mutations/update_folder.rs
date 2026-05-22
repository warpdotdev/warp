use crate::error::UserFacingError;
use crate::object::ObjectUpdateSuccess;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;

/*
mutation UpdateFolder($input: UpdateFolderInput!, $requestContext: RequestContext!) {
  updateFolder(input: $input, requestContext: $requestContext) {
    ... on UpdateFolderOutput {
      responseContext {
        serverVersion
      }
      update {
        lastEditorUid
        revisionTs
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
pub struct UpdateFolderVariables {
    pub input: UpdateFolderInput,
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct UpdateFolderOutput {
    pub response_context: ResponseContext,
    pub update: ObjectUpdateSuccess,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "RootMutation", variables = "UpdateFolderVariables")]
pub struct UpdateFolder {
    #[arguments(input: $input, requestContext: $request_context)]
    pub update_folder: UpdateFolderResult,
}
crate::client::define_operation! {
    update_folder(UpdateFolderVariables) -> UpdateFolder;
}

#[derive(cynic::InlineFragments, Debug)]
pub enum UpdateFolderResult {
    UpdateFolderOutput(UpdateFolderOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::InputObject, Debug)]
pub struct UpdateFolderInput {
    pub name: String,
    pub uid: cynic::Id,
}
