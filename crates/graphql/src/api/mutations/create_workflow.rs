use crate::error::UserFacingError;
use crate::object::CloudObjectEventEntrypoint;
use crate::object_permissions::Owner;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::scalars::Time;
use crate::schema;
use crate::workflow::Workflow;

/*
mutation CreateWorkflow($input: CreateWorkflowInput!, $requestContext: RequestContext!) {
  createWorkflow(input: $input, requestContext: $requestContext) {
    ... on CreateWorkflowOutput {
      responseContext {
        serverVersion
      }
      workflow {
        data
        metadata {
          creatorUid
          currentEditorUid
          isWelcomeObject
          lastEditorUid
          metadataLastUpdatedTs
          parent {
            ... on FolderContainer {
              folderUid
            }
            ... on Space {
              uid
              type
            }
          }
          revisionTs
          trashedTs
          uid
        }
        permissions {
          guests {
            accessLevel
            source {
              ... on FolderContainer {
                folderUid
              }
              ... on Space {
                uid
                type
              }
            }
            subject {
              ... on UserGuest {
                firebaseUid
              }
            }
          }
          lastUpdatedTs
          anyoneLinkSharing {
            accessLevel
            source {
              ... on FolderContainer {
                folderUid
              }
              ... on Space {
                uid
                type
              }
            }
          }
        }
      }
      revisionTs
    }
    ... on UserFacingError {
      error {
        message
        ... on SharedObjectsLimitExceeded {
          limit
          objectType
          message
        }
      }
      responseContext {
        serverVersion
      }
    }
  }
}
*/

#[derive(cynic::QueryVariables, Debug)]
pub struct CreateWorkflowVariables {
    pub input: CreateWorkflowInput,
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "RootMutation", variables = "CreateWorkflowVariables")]
pub struct CreateWorkflow {
    #[arguments(input: $input, requestContext: $request_context)]
    pub create_workflow: CreateWorkflowResult,
}
crate::client::define_operation! {
    create_workflow(CreateWorkflowVariables) -> CreateWorkflow;
}

#[derive(cynic::QueryFragment, Debug)]
pub struct CreateWorkflowOutput {
    pub response_context: ResponseContext,
    pub workflow: Workflow,
    pub revision_ts: Time,
}

#[derive(cynic::InlineFragments, Debug)]
#[allow(clippy::large_enum_variant)]
pub enum CreateWorkflowResult {
    CreateWorkflowOutput(CreateWorkflowOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::InputObject, Debug)]
pub struct CreateWorkflowInput {
    pub data: String,
    pub entrypoint: CloudObjectEventEntrypoint,
    pub initial_folder_id: Option<cynic::Id>,
    pub owner: Owner,
}
