use std::collections::HashMap;

use anyhow::{anyhow, Context, Result};
use async_channel::Sender;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
// #[cfg(any(test, feature = "test-util"))]
// pub use cloud_object_client::MockObjectClient;
use cloud_object_client::{
    GetCloudObjectResponse, InitialLoadResponse, ObjectActionHistory, ObjectActionType,
    ObjectDeleteResult, ObjectMetadataUpdateResult, ObjectPermissionUpdateResult,
    ObjectPermissionsUpdateData, ObjectUpdateMessage,
};
pub use cloud_object_client::{GuestIdentifier, ObjectClient};
use cloud_object_models::JsonSerializer;
use cynic::{MutationBuilder, QueryBuilder, SubscriptionBuilder};
use warp_core::report_error;
use warp_graphql::error::UserFacingErrorInterface;
use warp_graphql::generic_string_object::GenericStringObjectInput;
use warp_graphql::mutations::add_object_guests::{
    AddObjectGuests, AddObjectGuestsInput, AddObjectGuestsResult, AddObjectGuestsVariables,
};
use warp_graphql::mutations::bulk_create_objects::{
    BulkCreateGenericStringObjectsInput, BulkCreateObjects, BulkCreateObjectsInput,
    BulkCreateObjectsResult, BulkCreateObjectsVariables,
};
use warp_graphql::mutations::create_folder::{
    CreateFolder, CreateFolderInput, CreateFolderResult, CreateFolderVariables,
};
use warp_graphql::mutations::create_generic_string_object::{
    CreateGenericStringObject, CreateGenericStringObjectInput, CreateGenericStringObjectResult,
    CreateGenericStringObjectVariables,
};
use warp_graphql::mutations::create_notebook::{
    CreateNotebook, CreateNotebookInput, CreateNotebookResult, CreateNotebookVariables,
};
use warp_graphql::mutations::create_workflow::{
    CreateWorkflow, CreateWorkflowInput, CreateWorkflowResult, CreateWorkflowVariables,
};
use warp_graphql::mutations::delete_object::{
    DeleteObject, DeleteObjectInput, DeleteObjectResult, DeleteObjectVariables,
};
use warp_graphql::mutations::empty_trash::{
    EmptyTrash, EmptyTrashInput, EmptyTrashResult, EmptyTrashVariables,
};
use warp_graphql::mutations::give_up_notebook_edit_access::{
    GiveUpNotebookEditAccess, GiveUpNotebookEditAccessVariables,
};
use warp_graphql::mutations::grab_notebook_edit_access::{
    GrabNotebookEditAccess, GrabNotebookEditAccessVariables,
};
use warp_graphql::mutations::leave_object::{
    LeaveObject, LeaveObjectInput, LeaveObjectResult, LeaveObjectVariables,
};
use warp_graphql::mutations::move_object::{
    MoveObject, MoveObjectInput, MoveObjectResult, MoveObjectVariables,
};
use warp_graphql::mutations::record_object_action::{
    RecordObjectAction, RecordObjectActionInput, RecordObjectActionResult,
    RecordObjectActionVariables,
};
use warp_graphql::mutations::remove_object_guest::{
    RemoveObjectGuest, RemoveObjectGuestInput, RemoveObjectGuestResult, RemoveObjectGuestVariables,
};
use warp_graphql::mutations::remove_object_link_permissions::{
    RemoveObjectLinkPermissions, RemoveObjectLinkPermissionsInput,
    RemoveObjectLinkPermissionsResult, RemoveObjectLinkPermissionsVariables,
};
use warp_graphql::mutations::set_object_link_permissions::{
    SetObjectLinkPermissions, SetObjectLinkPermissionsInput, SetObjectLinkPermissionsResult,
    SetObjectLinkPermissionsVariables,
};
use warp_graphql::mutations::transfer_generic_string_object_owner::{
    TransferGenericStringObjectOwner, TransferGenericStringObjectOwnerInput,
    TransferGenericStringObjectOwnerResult, TransferGenericStringObjectOwnerVariables,
};
use warp_graphql::mutations::transfer_notebook_owner::{
    TransferNotebookOwner, TransferNotebookOwnerInput, TransferNotebookOwnerResult,
    TransferNotebookOwnerVariables,
};
use warp_graphql::mutations::transfer_workflow_owner::{
    TransferWorkflowOwner, TransferWorkflowOwnerInput, TransferWorkflowOwnerResult,
    TransferWorkflowOwnerVariables,
};
use warp_graphql::mutations::trash_object::{
    TrashObject, TrashObjectInput, TrashObjectResult, TrashObjectVariables,
};
use warp_graphql::mutations::untrash_object::{
    UntrashObject, UntrashObjectInput, UntrashObjectVariables,
};
use warp_graphql::mutations::update_folder::{
    UpdateFolder, UpdateFolderInput, UpdateFolderResult, UpdateFolderVariables,
};
use warp_graphql::mutations::update_generic_string_object::{
    UpdateGenericStringObject, UpdateGenericStringObjectInput, UpdateGenericStringObjectVariables,
};
use warp_graphql::mutations::update_notebook::{
    NotebookUpdate, UpdateNotebook, UpdateNotebookInput, UpdateNotebookResult,
    UpdateNotebookVariables,
};
use warp_graphql::mutations::update_object_guests::{
    UpdateObjectGuests, UpdateObjectGuestsInput, UpdateObjectGuestsResult,
    UpdateObjectGuestsVariables,
};
use warp_graphql::mutations::update_workflow::{
    UpdateWorkflow, UpdateWorkflowInput, UpdateWorkflowResult, UpdateWorkflowVariables,
    WorkflowUpdate,
};
use warp_graphql::notebook::{UpdateNotebookEditAccessInput, UpdateNotebookEditAccessResult};
use warp_graphql::object::CloudObjectWithDescendants;
use warp_graphql::object_permissions::AccessLevel;
use warp_graphql::queries::get_cloud_environments::{
    GetCloudEnvironmentsQuery, GetCloudEnvironmentsQueryVariables, GetCloudEnvironmentsResult,
};
use warp_graphql::queries::get_cloud_object::{
    CloudObjectInput, CloudObjectResult, GetCloudObject, GetCloudObjectVariables,
};
use warp_graphql::queries::get_updated_cloud_objects::{
    GetUpdatedCloudObjects, GetUpdatedCloudObjectsVariables, UpdatedCloudObjectsInput,
    UpdatedCloudObjectsResult,
};
use warp_graphql::subscriptions::get_warp_drive_updates::GetWarpDriveUpdates;
use warp_graphql::subscriptions::start_graphql_streaming_operation;

use crate::ai::ambient_agents::scheduled::ScheduledAmbientAgent;
use crate::ai::cloud_environments::AmbientAgentEnvironment;
use crate::ai::document::ai_document_model::AIDocumentId;
use crate::ai::execution_profiles::AIExecutionProfile;
use crate::ai::facts::AIFact;
use crate::ai::mcp::{MCPServer, TemplatableMCPServer};
use crate::channel::ChannelState;
use crate::cloud_object::model::generic_string_model::{
    GenericStringModel, GenericStringObjectId, Serializer, StringModel,
};
use crate::cloud_object::{
    BulkCreateCloudObjectResult, BulkCreateGenericStringObjectsRequest, CreateCloudObjectResult,
    CreateObjectRequest, CreatedCloudObject, GenericCloudObject, GenericServerObject,
    GenericStringObjectFormat, GenericStringObjectUniqueKey, JsonObjectType, ObjectIdType,
    ObjectType, ObjectsToUpdate, Owner, Revision, RevisionAndLastEditor, ServerCloudObject,
    ServerFolder, ServerMetadata, ServerNotebook, ServerObject, ServerPermissions, ServerWorkflow,
    TryFromGql as _, UpdateCloudObjectResult,
};
use crate::drive::folders::FolderId;
use crate::drive::sharing::SharingAccessLevel;
use crate::env_vars::EnvVarCollection;
use crate::notebooks::{NotebookId, SerializedNotebook};
use crate::server::graphql::schema::{
    action_type_to_gql_action_type, object_action_history_from_gql,
    object_update_success_to_update_result, update_generic_string_object_result_to_update_result,
};
use crate::server::graphql::{get_request_context, get_user_facing_error_message};
use crate::server::ids::{ClientId, HashableId, ServerId, ServerIdAndType, SyncId, ToServerId};
use crate::server::server_api::ServerApi;
use crate::server::sync_queue::SerializedModel;
use crate::settings::Preference;
use crate::workflows::workflow_enum::WorkflowEnum;
use crate::workflows::WorkflowId;
use crate::workspaces::gql_convert::object_update_message_from_gql;
use crate::workspaces::user_profiles::UserProfileWithUID;

#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
impl ObjectClient for ServerApi {
    async fn create_workflow(
        &self,
        request: CreateObjectRequest,
    ) -> Result<CreateCloudObjectResult> {
        let model = request
            .serialized_model
            .ok_or_else(|| anyhow!("missing model for creating workflow"))?;
        let variables = CreateWorkflowVariables {
            input: CreateWorkflowInput {
                data: model.take(),
                entrypoint: request.entrypoint.into(),
                initial_folder_id: request.initial_folder_id.map(|folder_id| folder_id.into()),
                owner: request.owner.into(),
            },
            request_context: get_request_context(),
        };

        let operation = CreateWorkflow::build(variables);
        let response = self.send_graphql_request(operation, None).await?;

        match response.create_workflow {
            CreateWorkflowResult::CreateWorkflowOutput(output) => {
                let metadata = output.workflow.metadata;
                let workflow_id: WorkflowId = metadata.uid.into_inner().into();

                Ok(CreateCloudObjectResult::Success {
                    created_cloud_object: CreatedCloudObject {
                        client_id: request.client_id,
                        revision_and_editor: RevisionAndLastEditor {
                            revision: output.revision_ts.into(),
                            last_editor_uid: metadata.last_editor_uid.map(|uid| uid.into_inner()),
                        },
                        metadata_ts: metadata.metadata_last_updated_ts,
                        server_id_and_type: ServerIdAndType {
                            id: workflow_id.to_server_id(),
                            id_type: ObjectIdType::Workflow,
                        },
                        creator_uid: metadata.creator_uid.map(|uid| uid.into_inner()),
                        permissions: output.workflow.permissions.try_into()?,
                    },
                })
            }
            CreateWorkflowResult::UserFacingError(e) => Ok(
                CreateCloudObjectResult::UserFacingError(get_user_facing_error_message(e)),
            ),
            CreateWorkflowResult::Unknown => {
                Err(anyhow!("Failed to create workflow due to unknown variant"))
            }
        }
    }

    async fn update_workflow(
        &self,
        workflow_id: WorkflowId,
        data: SerializedModel,
        revision: Option<Revision>,
    ) -> Result<UpdateCloudObjectResult<ServerWorkflow>> {
        let variables = UpdateWorkflowVariables {
            input: UpdateWorkflowInput {
                data: data.model_as_str().to_owned(),
                uid: cynic::Id::new(workflow_id),
                revision_ts: revision.map(|r| r.into()),
            },
            request_context: get_request_context(),
        };

        let operation = UpdateWorkflow::build(variables);
        let response = self.send_graphql_request(operation, None).await?;
        match response.update_workflow {
            UpdateWorkflowResult::UpdateWorkflowOutput(output) => match output.update {
                WorkflowUpdate::ObjectUpdateSuccess(success) => {
                    Ok(UpdateCloudObjectResult::Success {
                        revision_and_editor: RevisionAndLastEditor {
                            revision: success.revision_ts.into(),
                            last_editor_uid: Some(success.last_editor_uid.into_inner()),
                        },
                    })
                }
                WorkflowUpdate::WorkflowUpdateRejected(rejected) => {
                    Ok(UpdateCloudObjectResult::Rejected {
                        object: ServerWorkflow::try_from_gql(rejected.conflicting_workflow)?,
                    })
                }
                WorkflowUpdate::Unknown => Err(anyhow!("WorkflowUpdate has unknown variant")),
            },
            UpdateWorkflowResult::UserFacingError(e) => {
                Err(anyhow!(get_user_facing_error_message(e)))
            }
            UpdateWorkflowResult::Unknown => {
                Err(anyhow!("Failed to update workflow due to unknown variant"))
            }
        }
    }

    async fn bulk_create_generic_string_objects(
        &self,
        owner: Owner,
        objects: &[BulkCreateGenericStringObjectsRequest],
    ) -> Result<BulkCreateCloudObjectResult> {
        let variables = BulkCreateObjectsVariables {
            input: BulkCreateObjectsInput {
                generic_string_objects: Some(BulkCreateGenericStringObjectsInput {
                    owner: owner.into(),
                    objects: objects
                        .iter()
                        .map(|object| GenericStringObjectInput {
                            client_id: cynic::Id::new(object.id.to_string()),
                            serialized_model: object.serialized_model.model_as_str().to_owned(),
                            format: object.format.into(),
                            uniqueness_key: object
                                .uniqueness_key
                                .clone()
                                .map(GenericStringObjectUniqueKey::into),
                            initial_folder_id: object.initial_folder_id.map(FolderId::into),
                            entrypoint: object.entrypoint.into(),
                        })
                        .collect(),
                }),
            },
            request_context: get_request_context(),
        };

        let operation = BulkCreateObjects::build(variables);
        let response = self.send_graphql_request(operation, None).await?;
        match response.bulk_create_objects {
            BulkCreateObjectsResult::BulkCreateObjectsOutput(output) => {
                if let Some(gso_result) = output.generic_string_objects {
                    let mut created_cloud_objects = Vec::new();
                    for gso in gso_result.objects {
                        let metadata = gso.generic_string_object.metadata;
                        let uid = metadata.uid.into_inner();
                        let object_id: GenericStringObjectId = uid.into();

                        created_cloud_objects.push(CreatedCloudObject {
                            client_id: ClientId::from_hash(&gso.client_id.into_inner())
                                .ok_or_else(|| anyhow!("invalid client id"))?,
                            revision_and_editor: RevisionAndLastEditor {
                                revision: metadata.revision_ts.into(),
                                last_editor_uid: metadata
                                    .last_editor_uid
                                    .map(|uid| uid.into_inner()),
                            },
                            metadata_ts: metadata.metadata_last_updated_ts,
                            server_id_and_type: ServerIdAndType {
                                id: object_id.to_server_id(),
                                id_type: ObjectIdType::GenericStringObject,
                            },
                            creator_uid: metadata.creator_uid.map(|uid| uid.into_inner()),
                            permissions: gso.generic_string_object.permissions.try_into()?,
                        });
                    }

                    Ok(BulkCreateCloudObjectResult::Success {
                        created_cloud_objects,
                    })
                } else {
                    Err(anyhow!(
                        "No generic string objects found in BulkCreateGenericStringObjectsOutput"
                    ))
                }
            }
            BulkCreateObjectsResult::UserFacingError(e) => match e.error {
                UserFacingErrorInterface::GenericStringObjectUniqueKeyConflict(_) => {
                    Ok(BulkCreateCloudObjectResult::GenericStringObjectUniqueKeyConflict)
                }
                _ => Err(anyhow!(get_user_facing_error_message(e))),
            },
            BulkCreateObjectsResult::Unknown => Err(anyhow!(
                "Failed to bulk create objects due to unknown variant"
            )),
        }
    }

    async fn create_generic_string_object(
        &self,
        format: GenericStringObjectFormat,
        uniqueness_key: Option<GenericStringObjectUniqueKey>,
        request: CreateObjectRequest,
    ) -> Result<CreateCloudObjectResult> {
        let model = request
            .serialized_model
            .ok_or_else(|| anyhow!("missing model for creating generic string model"))?;
        let variables = CreateGenericStringObjectVariables {
            input: CreateGenericStringObjectInput {
                generic_string_object: GenericStringObjectInput {
                    client_id: cynic::Id::new(request.client_id.to_hash()),
                    entrypoint: request.entrypoint.into(),
                    format: format.into(),
                    initial_folder_id: request.initial_folder_id.map(|folder_id| folder_id.into()),
                    serialized_model: model.take(),
                    uniqueness_key: uniqueness_key.map(|key| key.into()),
                },
                owner: request.owner.into(),
            },
            request_context: get_request_context(),
        };

        let operation = CreateGenericStringObject::build(variables);
        let response = self.send_graphql_request(operation, None).await?;

        match response.create_generic_string_object {
            CreateGenericStringObjectResult::CreateGenericStringObjectOutput(output) => {
                let metadata = output.generic_string_object.metadata;
                let gso_id: GenericStringObjectId = metadata.uid.into_inner().into();

                Ok(CreateCloudObjectResult::Success {
                    created_cloud_object: CreatedCloudObject {
                        revision_and_editor: RevisionAndLastEditor {
                            revision: output.revision_ts.into(),
                            last_editor_uid: metadata.last_editor_uid.map(|uid| uid.into_inner()),
                        },
                        metadata_ts: metadata.metadata_last_updated_ts,
                        server_id_and_type: ServerIdAndType {
                            id: gso_id.to_server_id(),
                            id_type: ObjectIdType::GenericStringObject,
                        },
                        creator_uid: metadata.creator_uid.map(|uid| uid.into_inner()),
                        client_id: request.client_id,
                        permissions: output.generic_string_object.permissions.try_into()?,
                    },
                })
            }
            CreateGenericStringObjectResult::UserFacingError(e) => Ok(match e.error {
                UserFacingErrorInterface::GenericStringObjectUniqueKeyConflict(_) => {
                    CreateCloudObjectResult::GenericStringObjectUniqueKeyConflict
                }
                _ => CreateCloudObjectResult::UserFacingError(get_user_facing_error_message(e)),
            }),
            CreateGenericStringObjectResult::Unknown => Err(anyhow!(
                "Failed to create generic string object due to unknown variant"
            )),
        }
    }

    async fn create_notebook(
        &self,
        request: CreateObjectRequest,
    ) -> Result<CreateCloudObjectResult> {
        let serialized = request
            .serialized_model
            .as_ref()
            .ok_or_else(|| anyhow!("Missing serialized model for notebook"))?;

        let notebook: SerializedNotebook = serde_json::from_str(serialized.model_as_str())
            .context("Failed to deserialize notebook model")?;

        let ai_document_id = notebook
            .ai_document_id
            .and_then(|id| AIDocumentId::try_from(id).ok());

        let variables = CreateNotebookVariables {
            input: CreateNotebookInput {
                data: Some(notebook.data),
                entrypoint: request.entrypoint.into(),
                initial_folder_id: request.initial_folder_id.map(|folder_id| folder_id.into()),
                owner: request.owner.into(),
                title: request.title,
                ai_document_id: ai_document_id.map(|id| id.to_string()),
                conversation_id: notebook.conversation_id,
            },
            request_context: get_request_context(),
        };

        let operation = CreateNotebook::build(variables);
        let response = self.send_graphql_request(operation, None).await?;

        match response.create_notebook {
            CreateNotebookResult::CreateNotebookOutput(output) => {
                let metadata = output.notebook.metadata;
                let notebook_id: NotebookId = metadata.uid.into_inner().into();

                Ok(CreateCloudObjectResult::Success {
                    created_cloud_object: CreatedCloudObject {
                        client_id: request.client_id,
                        revision_and_editor: RevisionAndLastEditor {
                            revision: output.revision_ts.into(),
                            last_editor_uid: metadata.last_editor_uid.map(|uid| uid.into_inner()),
                        },
                        metadata_ts: metadata.metadata_last_updated_ts,
                        server_id_and_type: ServerIdAndType {
                            id: notebook_id.to_server_id(),
                            id_type: ObjectIdType::Notebook,
                        },
                        creator_uid: metadata.creator_uid.map(|uid| uid.into_inner()),
                        permissions: output.notebook.permissions.try_into()?,
                    },
                })
            }
            CreateNotebookResult::UserFacingError(e) => Ok(
                CreateCloudObjectResult::UserFacingError(get_user_facing_error_message(e)),
            ),
            CreateNotebookResult::Unknown => {
                Err(anyhow!("Failed to create notebook due to unknown variant"))
            }
        }
    }

    async fn update_notebook(
        &self,
        notebook_id: NotebookId,
        title: Option<String>,
        data: Option<SerializedModel>,
        revision: Option<Revision>,
    ) -> Result<UpdateCloudObjectResult<ServerNotebook>> {
        let variables = UpdateNotebookVariables {
            input: UpdateNotebookInput {
                data: data.map(|data| data.model_as_str().to_owned()),
                title,
                uid: cynic::Id::new(notebook_id),
                revision_ts: revision.map(|r| r.into()),
            },
            request_context: get_request_context(),
        };

        let operation = UpdateNotebook::build(variables);
        let response = self.send_graphql_request(operation, None).await?;
        match response.update_notebook {
            UpdateNotebookResult::UpdateNotebookOutput(output) => match output.update {
                NotebookUpdate::ObjectUpdateSuccess(success) => {
                    Ok(UpdateCloudObjectResult::Success {
                        revision_and_editor: RevisionAndLastEditor {
                            revision: success.revision_ts.into(),
                            last_editor_uid: Some(success.last_editor_uid.into_inner()),
                        },
                    })
                }
                NotebookUpdate::NotebookUpdateRejected(rejected) => {
                    Ok(UpdateCloudObjectResult::Rejected {
                        object: ServerNotebook::try_from_gql(rejected.conflicting_notebook)?,
                    })
                }
                NotebookUpdate::Unknown => Err(anyhow!("NotebookUpdate has unknown variant")),
            },
            UpdateNotebookResult::UserFacingError(e) => {
                Err(anyhow!(get_user_facing_error_message(e)))
            }
            UpdateNotebookResult::Unknown => {
                Err(anyhow!("Failed to update notebook due to unknown variant"))
            }
        }
    }

    async fn create_folder(&self, request: CreateObjectRequest) -> Result<CreateCloudObjectResult> {
        let model = request
            .serialized_model
            .ok_or_else(|| anyhow!("missing serialized model for creating folder"))?;
        let variables = CreateFolderVariables {
            input: CreateFolderInput {
                initial_folder_id: request.initial_folder_id.map(|folder_id| folder_id.into()),
                name: model.take(),
                owner: request.owner.into(),
            },
            request_context: get_request_context(),
        };

        let operation = CreateFolder::build(variables);
        let response = self.send_graphql_request(operation, None).await?;

        match response.create_folder {
            CreateFolderResult::CreateFolderOutput(output) => {
                let metadata = output.folder.metadata;
                let folder_id: FolderId = metadata.uid.into_inner().into();

                Ok(CreateCloudObjectResult::Success {
                    created_cloud_object: CreatedCloudObject {
                        client_id: request.client_id,
                        revision_and_editor: RevisionAndLastEditor {
                            revision: metadata.revision_ts.into(),
                            last_editor_uid: metadata.last_editor_uid.map(|uid| uid.into_inner()),
                        },
                        metadata_ts: metadata.metadata_last_updated_ts,
                        server_id_and_type: ServerIdAndType {
                            id: folder_id.to_server_id(),
                            id_type: ObjectIdType::Folder,
                        },
                        creator_uid: metadata.creator_uid.map(|uid| uid.into_inner()),
                        permissions: output.folder.permissions.try_into()?,
                    },
                })
            }
            CreateFolderResult::UserFacingError(e) => Ok(CreateCloudObjectResult::UserFacingError(
                get_user_facing_error_message(e),
            )),
            CreateFolderResult::Unknown => {
                Err(anyhow!("Failed to create folder due to unknown variant"))
            }
        }
    }

    async fn update_folder(
        &self,
        folder_id: FolderId,
        name: SerializedModel,
    ) -> Result<UpdateCloudObjectResult<ServerFolder>> {
        let variables = UpdateFolderVariables {
            input: UpdateFolderInput {
                uid: cynic::Id::new(folder_id),
                name: name.model_as_str().to_owned(),
            },
            request_context: get_request_context(),
        };

        let operation = UpdateFolder::build(variables);
        let response = self.send_graphql_request(operation, None).await?;

        match response.update_folder {
            UpdateFolderResult::UpdateFolderOutput(output) => {
                object_update_success_to_update_result(output.update)
            }
            UpdateFolderResult::UserFacingError(e) => {
                Err(anyhow!(get_user_facing_error_message(e)))
            }
            UpdateFolderResult::Unknown => {
                Err(anyhow!("Failed to update folder due to unknown variant"))
            }
        }
    }

    async fn update_generic_string_object(
        &self,
        object_id: GenericStringObjectId,
        model: SerializedModel,
        revision: Option<Revision>,
    ) -> Result<UpdateCloudObjectResult<Box<dyn ServerObject>>> {
        let variables = UpdateGenericStringObjectVariables {
            input: UpdateGenericStringObjectInput {
                revision_ts: revision.map(|r| r.into()),
                serialized_model: model.model_as_str().to_owned(),
                uid: object_id.into(),
            },
            request_context: get_request_context(),
        };

        let operation = UpdateGenericStringObject::build(variables);
        let response = self.send_graphql_request(operation, None).await?;
        update_generic_string_object_result_to_update_result(response.update_generic_string_object)
    }

    async fn grab_notebook_edit_access(&self, notebook_id: NotebookId) -> Result<ServerMetadata> {
        let variables = GrabNotebookEditAccessVariables {
            input: UpdateNotebookEditAccessInput {
                uid: cynic::Id::new(notebook_id),
            },
            request_context: get_request_context(),
        };

        let operation = GrabNotebookEditAccess::build(variables);
        let response = self.send_graphql_request(operation, None).await?;

        match response.grab_notebook_edit_access {
            UpdateNotebookEditAccessResult::UpdateNotebookEditAccessOutput(output) => {
                // The grabNotebookEditAccess API errors if unable to grab the baton,
                // so we're always in the success case here.
                output.metadata.try_into()
            }
            UpdateNotebookEditAccessResult::UserFacingError(e) => {
                Err(anyhow!(get_user_facing_error_message(e)))
            }
            UpdateNotebookEditAccessResult::Unknown => Err(anyhow!(
                "Failed to grab notebook edit access due to unknown variant"
            )),
        }
    }

    async fn give_up_notebook_edit_access(
        &self,
        notebook_id: NotebookId,
    ) -> Result<ServerMetadata> {
        let variables = GiveUpNotebookEditAccessVariables {
            input: UpdateNotebookEditAccessInput {
                uid: cynic::Id::new(notebook_id),
            },
            request_context: get_request_context(),
        };

        let operation = GiveUpNotebookEditAccess::build(variables);
        let response = self.send_graphql_request(operation, None).await?;

        match response.give_up_notebook_edit_access {
            UpdateNotebookEditAccessResult::UpdateNotebookEditAccessOutput(output) => {
                output.metadata.try_into()
            }
            UpdateNotebookEditAccessResult::UserFacingError(e) => {
                Err(anyhow!(get_user_facing_error_message(e)))
            }
            UpdateNotebookEditAccessResult::Unknown => Err(anyhow!(
                "Failed to give up notebook edit access due to unknown variant"
            )),
        }
    }

    /// Starts a websocket connections against the corresponding GraphQL subscription.
    /// Messages received over the socket are sent over the `message_sender`.
    /// Once the websocket is live, a one-shot message is sent over `stream_ready_sender`
    /// to indicate so. This is because this method only returns once the websocket is closed.
    async fn get_warp_drive_updates(
        &self,
        message_sender: Sender<ObjectUpdateMessage>,
        stream_ready_sender: Sender<()>,
    ) -> Result<()> {
        // The init payload is how we convey any metadata about
        // the subscription to the server (i.e. in lieu of http headers).
        // TODO (written by Suraj): we should consider consolidating the places we
        // supply this common data. GQL subscriptions use a different
        // implementation from our general server requests (which make
        // use of [`crate::http`]).
        let mut init_payload = HashMap::new();

        // Add the bearer token to the init payload when using header-based auth.
        // Session-cookie-authenticated clients rely on the websocket handshake cookies instead.
        let auth_token = self.get_or_refresh_access_token().await?;
        if let Some(token) = auth_token.as_bearer_token() {
            let bearer_token = format!("Bearer {token}");
            init_payload.insert(http_client::AUTHORIZATION.as_str(), bearer_token);
        }

        // Add the app version, if available.
        if let Some(app_version) = ChannelState::app_version() {
            init_payload.insert(
                http_client::headers::CLIENT_RELEASE_VERSION_HEADER_KEY,
                app_version.to_string(),
            );
        }

        let subscription = GetWarpDriveUpdates::build(());

        let result = start_graphql_streaming_operation(
            &ChannelState::ws_server_url(),
            init_payload,
            subscription,
            |res| {
                res.ok_or_else(|| {
                    anyhow!("missing response data for message in get_warp_drive_updates")
                })
                .and_then(|data| object_update_message_from_gql(data.warp_drive_updates))
            },
            message_sender,
            stream_ready_sender,
            self.iap_proxy_auth_header().into_iter().collect(),
        )
        .await;

        if let Err(err) = &result {
            self.report_ws_iap_challenge(err);
        }
        result
    }

    async fn fetch_changed_objects(
        &self,
        objects_to_update: ObjectsToUpdate,
        force_refresh: bool,
    ) -> Result<InitialLoadResponse> {
        log::info!("fetching updated cloud objects");
        if force_refresh {
            log::info!("forcing sync of all objects")
        }

        let variables = GetUpdatedCloudObjectsVariables {
            input: UpdatedCloudObjectsInput {
                folders: Some(objects_to_update.folders),
                force_refresh,
                generic_string_objects: Some(objects_to_update.generic_string_objects),
                notebooks: Some(objects_to_update.notebooks),
                workflows: Some(objects_to_update.workflows),
            },
            request_context: get_request_context(),
        };

        let operation = GetUpdatedCloudObjects::build(variables);
        let response_data = self.send_graphql_request(operation, None).await?;

        match response_data.updated_cloud_objects {
            UpdatedCloudObjectsResult::UpdatedCloudObjectsOutput(output) => {
                let updated_notebooks = output
                    .notebooks
                    .map(|notebooks| {
                        notebooks
                            .into_iter()
                            .filter_map(|notebook| ServerNotebook::try_from_gql(notebook).ok())
                            .collect()
                    })
                    .unwrap_or_default();

                let updated_workflows = output
                    .workflows
                    .map(|workflows| {
                        workflows
                            .into_iter()
                            .filter_map(|workflow| ServerWorkflow::try_from_gql(workflow).ok())
                            .collect()
                    })
                    .unwrap_or_default();

                let updated_folders = output
                    .folders
                    .map(|folders| {
                        folders
                            .into_iter()
                            .filter_map(|folder| ServerFolder::try_from_gql(folder).ok())
                            .collect()
                    })
                    .unwrap_or_default();

                let mut updated_generic_string_objects = HashMap::new();
                if let Some(objects) = output.generic_string_objects {
                    for gso in objects {
                        match gso.format {
                            warp_graphql::generic_string_object::GenericStringObjectFormat::JsonEnvVarCollection => {
                                parse_server_gso::<EnvVarCollection, JsonSerializer>(
                                    &mut updated_generic_string_objects,
                                    GenericStringObjectFormat::Json(JsonObjectType::EnvVarCollection),
                                    gso,
                                );
                            }
                            warp_graphql::generic_string_object::GenericStringObjectFormat::JsonPreference => {
                                parse_server_gso::<Preference, JsonSerializer>(
                                    &mut updated_generic_string_objects,
                                    GenericStringObjectFormat::Json(JsonObjectType::Preference),
                                    gso,
                                );
                            }
                            warp_graphql::generic_string_object::GenericStringObjectFormat::JsonWorkflowEnum => {
                                parse_server_gso::<WorkflowEnum, JsonSerializer>(
                                    &mut updated_generic_string_objects,
                                    GenericStringObjectFormat::Json(JsonObjectType::WorkflowEnum),
                                    gso,
                                );
                            }
                            warp_graphql::generic_string_object::GenericStringObjectFormat::JsonAIFact => {
                                parse_server_gso::<AIFact, JsonSerializer>(
                                    &mut updated_generic_string_objects,
                                    GenericStringObjectFormat::Json(JsonObjectType::AIFact),
                                    gso,
                                );
                            }
                            warp_graphql::generic_string_object::GenericStringObjectFormat::JsonMCPServer => {
                                parse_server_gso::<MCPServer, JsonSerializer>(
                                    &mut updated_generic_string_objects,
                                    GenericStringObjectFormat::Json(JsonObjectType::MCPServer),
                                    gso,
                                );
                            }
                            warp_graphql::generic_string_object::GenericStringObjectFormat::JsonAIExecutionProfile => {
                                parse_server_gso::<AIExecutionProfile, JsonSerializer>(
                                    &mut updated_generic_string_objects,
                                    GenericStringObjectFormat::Json(JsonObjectType::AIExecutionProfile),
                                    gso,
                                );
                            }
                            warp_graphql::generic_string_object::GenericStringObjectFormat::JsonTemplatableMCPServer => {
                                parse_server_gso::<TemplatableMCPServer, JsonSerializer>(
                                    &mut updated_generic_string_objects,
                                    GenericStringObjectFormat::Json(JsonObjectType::TemplatableMCPServer),
                                    gso,
                                );
                            }
                            warp_graphql::generic_string_object::GenericStringObjectFormat::JsonCloudEnvironment => {
                                parse_server_gso::<AmbientAgentEnvironment, JsonSerializer>(
                                    &mut updated_generic_string_objects,
                                    GenericStringObjectFormat::Json(JsonObjectType::CloudEnvironment),
                                    gso,
                                );
                            }
                            warp_graphql::generic_string_object::GenericStringObjectFormat::JsonScheduledAmbientAgent => {
                                parse_server_gso::<ScheduledAmbientAgent, JsonSerializer>(
                                    &mut updated_generic_string_objects,
                                    GenericStringObjectFormat::Json(JsonObjectType::ScheduledAmbientAgent),
                                    gso,
                                );
                            }
                            // GSO formats unknown to this client build (e.g. the
                            // server-only `JsonRunner`) are skipped so syncing of
                            // known objects still succeeds instead of failing to
                            // decode the whole response.
                            warp_graphql::generic_string_object::GenericStringObjectFormat::Unknown => {}
                        }
                    }
                }

                let deleted_notebooks: Vec<NotebookId> = output
                    .deleted_object_uids
                    .notebook_uids
                    .map(|uids| {
                        uids.into_iter()
                            .map(|uid| uid.into_inner().into())
                            .collect()
                    })
                    .unwrap_or_default();

                let deleted_workflows: Vec<WorkflowId> = output
                    .deleted_object_uids
                    .workflow_uids
                    .map(|uids| {
                        uids.into_iter()
                            .map(|uid| uid.into_inner().into())
                            .collect()
                    })
                    .unwrap_or_default();

                let deleted_folders: Vec<FolderId> = output
                    .deleted_object_uids
                    .folder_uids
                    .map(|uids| {
                        uids.into_iter()
                            .map(|uid| uid.into_inner().into())
                            .collect()
                    })
                    .unwrap_or_default();

                let deleted_generic_string_objects: Vec<GenericStringObjectId> = output
                    .deleted_object_uids
                    .generic_string_object_uids
                    .map(|uids| {
                        uids.into_iter()
                            .map(|uid| uid.into_inner().into())
                            .collect()
                    })
                    .unwrap_or_default();

                let user_profiles: Vec<UserProfileWithUID> = output
                    .user_profiles
                    .map(|user_profiles| {
                        user_profiles
                            .into_iter()
                            .map(|profile| profile.into())
                            .collect()
                    })
                    .unwrap_or_default();

                let action_histories: Vec<ObjectActionHistory> = output
                    .action_histories
                    .map(|histories| {
                        histories
                            .into_iter()
                            .filter_map(|history| object_action_history_from_gql(history).ok())
                            .collect()
                    })
                    .unwrap_or_default();

                let mcp_gallery = output.mcp_gallery.unwrap_or_default();

                let response = InitialLoadResponse {
                    updated_notebooks,
                    deleted_notebooks,
                    updated_workflows,
                    deleted_workflows,
                    updated_folders,
                    deleted_folders,
                    updated_generic_string_objects,
                    deleted_generic_string_objects,
                    user_profiles,
                    action_histories,
                    mcp_gallery,
                };
                Ok(response)
            }
            UpdatedCloudObjectsResult::UserFacingError(e) => {
                Err(anyhow!(get_user_facing_error_message(e)))
            }
            UpdatedCloudObjectsResult::Unknown => Err(anyhow!(
                "Failed to get updated cloud objects due to unknown variant"
            )),
        }
    }

    async fn fetch_single_cloud_object(&self, id: ServerId) -> Result<GetCloudObjectResponse> {
        let variables = GetCloudObjectVariables {
            input: CloudObjectInput {
                uid: cynic::Id::new(id),
            },
            request_context: get_request_context(),
        };

        let operation = GetCloudObject::build(variables);
        let response = self.send_graphql_request(operation, None).await?;
        match response.cloud_object {
            CloudObjectResult::CloudObjectOutput(output) => {
                let object: ServerCloudObject = output.object.clone().try_into()?;
                let action_histories: Vec<ObjectActionHistory> = output
                    .action_histories
                    .map(|histories| {
                        histories
                            .into_iter()
                            .filter_map(|history| object_action_history_from_gql(history).ok())
                            .collect()
                    })
                    .unwrap_or_default();
                let descendants = match output.object {
                    CloudObjectWithDescendants::FolderWithDescendants(folder) => folder
                        .descendants
                        .into_iter()
                        .filter_map(|descendant| descendant.try_into().ok())
                        .collect(),
                    _ => vec![],
                };

                Ok(GetCloudObjectResponse {
                    object,
                    action_histories,
                    descendants,
                })
            }
            CloudObjectResult::UserFacingError(e) => Err(anyhow!(get_user_facing_error_message(e))),
            CloudObjectResult::Unknown => Err(anyhow!(
                "Failed to fetch single cloud object due to unknown variant"
            )),
        }
    }

    async fn transfer_notebook_owner(&self, notebook_id: NotebookId, owner: Owner) -> Result<bool> {
        let variables = TransferNotebookOwnerVariables {
            input: TransferNotebookOwnerInput {
                uid: cynic::Id::new(notebook_id),
                owner: owner.into(),
            },
            request_context: get_request_context(),
        };

        let operation = TransferNotebookOwner::build(variables);
        let response = self.send_graphql_request(operation, None).await?;
        match response.transfer_notebook_owner {
            TransferNotebookOwnerResult::TransferNotebookOwnerOutput(output) => Ok(output.success),
            _ => Ok(false),
        }
    }

    async fn transfer_workflow_owner(&self, workflow_id: WorkflowId, owner: Owner) -> Result<bool> {
        let variables = TransferWorkflowOwnerVariables {
            input: TransferWorkflowOwnerInput {
                uid: cynic::Id::new(workflow_id),
                owner: owner.into(),
            },
            request_context: get_request_context(),
        };

        let operation = TransferWorkflowOwner::build(variables);
        let response = self.send_graphql_request(operation, None).await?;
        match response.transfer_workflow_owner {
            TransferWorkflowOwnerResult::TransferWorkflowOwnerOutput(output) => Ok(output.success),
            _ => Ok(false),
        }
    }

    async fn transfer_generic_string_object_owner(
        &self,
        gso_id: GenericStringObjectId,
        owner: Owner,
    ) -> Result<bool> {
        let variables = TransferGenericStringObjectOwnerVariables {
            input: TransferGenericStringObjectOwnerInput {
                uid: cynic::Id::new(gso_id),
                owner: owner.into(),
            },
            request_context: get_request_context(),
        };

        let operation = TransferGenericStringObjectOwner::build(variables);
        let response = self.send_graphql_request(operation, None).await?;
        match response.transfer_generic_string_object_owner {
            TransferGenericStringObjectOwnerResult::TransferGenericStringObjectOwnerOutput(
                output,
            ) => Ok(output.success),
            _ => Ok(false),
        }
    }

    async fn trash_object(&self, id: ServerId) -> Result<bool> {
        let variables = TrashObjectVariables {
            input: TrashObjectInput {
                uid: cynic::Id::new(id),
            },
            request_context: get_request_context(),
        };

        let operation = TrashObject::build(variables);
        let response = self.send_graphql_request(operation, None).await?;
        let success = match response.trash_object {
            TrashObjectResult::TrashObjectOutput(output) => output.success,
            _ => false,
        };
        Ok(success)
    }

    async fn untrash_object(&self, id: ServerId) -> Result<ObjectMetadataUpdateResult> {
        let variables = UntrashObjectVariables {
            input: UntrashObjectInput {
                uid: cynic::Id::new(id),
            },
            request_context: get_request_context(),
        };

        let operation = UntrashObject::build(variables);
        let response = self.send_graphql_request(operation, None).await?;

        let result = match response.untrash_object {
            warp_graphql::mutations::untrash_object::UntrashObjectResult::UntrashObjectOutput(
                output,
            ) => {
                if output.success {
                    ObjectMetadataUpdateResult::Success {
                        metadata: Box::new(output.metadata.try_into()?),
                    }
                } else {
                    ObjectMetadataUpdateResult::Failure
                }
            }
            _ => ObjectMetadataUpdateResult::Failure,
        };
        Ok(result)
    }

    async fn delete_object(&self, id: ServerId) -> Result<ObjectDeleteResult> {
        let variables = DeleteObjectVariables {
            input: DeleteObjectInput {
                uid: cynic::Id::new(id),
            },
            request_context: get_request_context(),
        };

        let operation = DeleteObject::build(variables);
        let response = self.send_graphql_request(operation, None).await?;
        let status = match response.delete_object {
            DeleteObjectResult::DeleteObjectOutput(output) => {
                let mut deleted_ids: Vec<SyncId> = Vec::new();
                for uid in output.deleted_uids {
                    deleted_ids.push(SyncId::ServerId(ServerId::from_string_lossy(uid.inner())))
                }
                ObjectDeleteResult::Success { deleted_ids }
            }
            _ => ObjectDeleteResult::Failure,
        };
        Ok(status)
    }

    async fn empty_trash(&self, owner: Owner) -> Result<ObjectDeleteResult> {
        let variables = EmptyTrashVariables {
            input: EmptyTrashInput {
                owner: owner.into(),
            },
            request_context: get_request_context(),
        };

        let operation = EmptyTrash::build(variables);
        let response = self.send_graphql_request(operation, None).await?;
        let status = match response.empty_trash {
            EmptyTrashResult::EmptyTrashOutput(output) => {
                let mut deleted_ids: Vec<SyncId> = Vec::new();
                for uid in output.deleted_uids {
                    deleted_ids.push(SyncId::ServerId(ServerId::from_string_lossy(
                        uid.into_inner(),
                    )))
                }
                ObjectDeleteResult::Success { deleted_ids }
            }
            _ => ObjectDeleteResult::Failure,
        };
        Ok(status)
    }

    async fn move_object(
        &self,
        id: ServerId,
        folder_id: Option<FolderId>,
        owner: Owner,
        object_type: ObjectType,
    ) -> Result<bool> {
        let variables = MoveObjectVariables {
            input: MoveObjectInput {
                new_folder_uid: folder_id.map(cynic::Id::new),
                new_owner: owner.into(),
                object_type: object_type.into(),
                uid: cynic::Id::new(id),
            },
            request_context: get_request_context(),
        };

        let operation = MoveObject::build(variables);
        let response = self.send_graphql_request(operation, None).await?;
        match response.move_object {
            MoveObjectResult::MoveObjectOutput(output) => Ok(output.success),
            MoveObjectResult::UserFacingError(e) => Err(anyhow!(get_user_facing_error_message(e))),
            MoveObjectResult::Unknown => {
                Err(anyhow!("Failed to move object due to unknown variant"))
            }
        }
    }

    async fn record_object_action(
        &self,
        id: ServerId,
        action_type: ObjectActionType,
        timestamp: DateTime<Utc>,
        data: Option<String>,
    ) -> Result<ObjectActionHistory> {
        let variables = RecordObjectActionVariables {
            input: RecordObjectActionInput {
                action: action_type_to_gql_action_type(action_type),
                json_data: data,
                timestamp: timestamp.into(),
                uid: id.into(),
            },
            request_context: get_request_context(),
        };

        let operation = RecordObjectAction::build(variables);
        let response = self.send_graphql_request(operation, None).await?;
        match response.record_object_action {
            RecordObjectActionResult::RecordObjectActionOutput(output) => {
                object_action_history_from_gql(output.history)
            }
            RecordObjectActionResult::UserFacingError(e) => {
                Err(anyhow!(get_user_facing_error_message(e)))
            }
            RecordObjectActionResult::Unknown => Err(anyhow!(
                "Failed to record object action due to unknown variant"
            )),
        }
    }

    async fn leave_object(&self, id: ServerId) -> Result<ObjectDeleteResult> {
        let variables = LeaveObjectVariables {
            input: LeaveObjectInput {
                object_uid: id.into(),
            },
            request_context: get_request_context(),
        };

        let operation = LeaveObject::build(variables);
        let response = self.send_graphql_request(operation, None).await?;
        match response.leave_object {
            LeaveObjectResult::LeaveObjectOutput(output) => Ok(ObjectDeleteResult::Success {
                deleted_ids: vec![SyncId::ServerId(ServerId::from_string_lossy(
                    output.object_uid.into_inner(),
                ))],
            }),
            LeaveObjectResult::UserFacingError(e) => Err(anyhow!(get_user_facing_error_message(e))),
            LeaveObjectResult::Unknown => Err(anyhow!("Unknown variant leaving object")),
        }
    }

    async fn set_object_link_permissions(
        &self,
        object_id: ServerId,
        access_level: SharingAccessLevel,
    ) -> Result<ObjectPermissionUpdateResult> {
        let variables = SetObjectLinkPermissionsVariables {
            input: SetObjectLinkPermissionsInput {
                uid: cynic::Id::new(object_id),
                access_level: access_level.into(),
            },
            request_context: get_request_context(),
        };

        let operation = SetObjectLinkPermissions::build(variables);
        let response = self.send_graphql_request(operation, None).await?;
        match response.set_object_link_permissions {
            SetObjectLinkPermissionsResult::SetObjectLinkPermissionsOutput(_) => {
                Ok(ObjectPermissionUpdateResult::Success)
            }
            SetObjectLinkPermissionsResult::UserFacingError(e) => {
                Err(anyhow!(get_user_facing_error_message(e)))
            }
            SetObjectLinkPermissionsResult::Unknown => Err(anyhow!(
                "Failed to set object link permissions due to unknown variant"
            )),
        }
    }

    async fn remove_object_link_permissions(
        &self,
        object_id: ServerId,
    ) -> Result<ObjectPermissionUpdateResult> {
        let variables = RemoveObjectLinkPermissionsVariables {
            input: RemoveObjectLinkPermissionsInput {
                uid: cynic::Id::new(object_id),
            },
            request_context: get_request_context(),
        };

        let operation = RemoveObjectLinkPermissions::build(variables);
        let response = self.send_graphql_request(operation, None).await?;
        match response.remove_object_link_permissions {
            RemoveObjectLinkPermissionsResult::RemoveObjectLinkPermissionsOutput(_) => {
                Ok(ObjectPermissionUpdateResult::Success)
            }
            RemoveObjectLinkPermissionsResult::UserFacingError(e) => {
                Err(anyhow!(get_user_facing_error_message(e)))
            }
            RemoveObjectLinkPermissionsResult::Unknown => Err(anyhow!(
                "Failed to remove object link permissions due to unknown variant"
            )),
        }
    }

    async fn add_object_guests(
        &self,
        object_id: ServerId,
        guest_emails: Vec<String>,
        access_level: AccessLevel,
    ) -> Result<ObjectPermissionsUpdateData> {
        let variables = AddObjectGuestsVariables {
            input: AddObjectGuestsInput {
                object_uid: cynic::Id::new(object_id),
                access_level,
                user_emails: guest_emails,
            },
            request_context: get_request_context(),
        };

        let operation = AddObjectGuests::build(variables);
        let response = self.send_graphql_request(operation, None).await?;

        match response.add_object_guests {
            AddObjectGuestsResult::AddObjectGuestsOutput(output) => {
                let permissions = output.object_permissions.try_into()?;
                let profiles = output
                    .user_profiles
                    .into_iter()
                    .flatten()
                    .map(Into::into)
                    .collect();
                Ok(ObjectPermissionsUpdateData {
                    permissions,
                    profiles,
                })
            }
            AddObjectGuestsResult::UserFacingError(e) => {
                Err(anyhow!(get_user_facing_error_message(e)))
            }
            AddObjectGuestsResult::Unknown => Err(anyhow!(
                "Failed to add object guests due to unknown variant"
            )),
        }
    }

    async fn update_object_guests(
        &self,
        object_id: ServerId,
        guest_emails: Vec<String>,
        access_level: AccessLevel,
    ) -> Result<ServerPermissions> {
        let variables = UpdateObjectGuestsVariables {
            input: UpdateObjectGuestsInput {
                object_uid: cynic::Id::new(object_id),
                access_level,
                emails: Some(guest_emails),
            },
            request_context: get_request_context(),
        };

        let operation = UpdateObjectGuests::build(variables);
        let response = self.send_graphql_request(operation, None).await?;

        match response.update_object_guests {
            UpdateObjectGuestsResult::UpdateObjectGuestsOutput(output) => {
                Ok(output.object_permissions.try_into()?)
            }
            UpdateObjectGuestsResult::UserFacingError(e) => {
                Err(anyhow!(get_user_facing_error_message(e)))
            }
            UpdateObjectGuestsResult::Unknown => Err(anyhow!(
                "Failed to update object guests due to unknown variant"
            )),
        }
    }

    async fn remove_object_guest(
        &self,
        object_id: ServerId,
        guest: GuestIdentifier,
    ) -> Result<ServerPermissions> {
        let (email, team_uid) = match guest {
            GuestIdentifier::Email(email) => (Some(email), None),
            GuestIdentifier::TeamUid(uid) => (None, Some(cynic::Id::new(uid))),
        };

        let variables = RemoveObjectGuestVariables {
            input: RemoveObjectGuestInput {
                email,
                object_uid: cynic::Id::new(object_id),
                team_uid,
            },
            request_context: get_request_context(),
        };

        let operation = RemoveObjectGuest::build(variables);
        let response = self.send_graphql_request(operation, None).await?;

        match response.remove_object_guest {
            RemoveObjectGuestResult::RemoveObjectGuestOutput(output) => {
                Ok(output.object_permissions.try_into()?)
            }
            RemoveObjectGuestResult::UserFacingError(e) => {
                Err(anyhow!(get_user_facing_error_message(e)))
            }
            RemoveObjectGuestResult::Unknown => Err(anyhow!(
                "Failed to remove object guest due to unknown variant"
            )),
        }
    }

    async fn fetch_environment_last_task_run_timestamps(
        &self,
    ) -> Result<HashMap<String, DateTime<Utc>>> {
        let variables = GetCloudEnvironmentsQueryVariables {
            request_context: get_request_context(),
        };

        let operation = GetCloudEnvironmentsQuery::build(variables);
        let response = self.send_graphql_request(operation, None).await?;

        match response.get_cloud_environments {
            GetCloudEnvironmentsResult::GetCloudEnvironmentsOutput(output) => {
                let mut timestamps = HashMap::new();
                for env in output.cloud_environments {
                    if let Some(task) = env.last_task_created {
                        timestamps.insert(env.uid.into_inner(), task.created_at.utc());
                    }
                }
                Ok(timestamps)
            }
            GetCloudEnvironmentsResult::UserFacingError(e) => {
                Err(anyhow!(get_user_facing_error_message(e)))
            }
            GetCloudEnvironmentsResult::Unknown => Err(anyhow!(
                "Failed to fetch cloud environments due to unknown variant"
            )),
        }
    }
}

/// Parse the serialized model for a GSO and add it to the format-specific entry in `map`,
/// or report an error if parsing fails.
fn parse_server_gso<T, S>(
    map: &mut HashMap<GenericStringObjectFormat, Vec<Box<dyn ServerObject>>>,
    format: GenericStringObjectFormat,
    gso: warp_graphql::generic_string_object::GenericStringObject,
) where
    T: StringModel<
        CloudObjectType = GenericCloudObject<GenericStringObjectId, GenericStringModel<T, S>>,
    >,
    S: Serializer<T>,
{
    let uid = ServerId::from_string_lossy(gso.metadata.uid.inner());
    match GenericServerObject::<GenericStringObjectId, GenericStringModel<T, S>>::try_from_gql(gso)
    {
        Ok(object) => {
            map.entry(format).or_default().push(Box::new(object));
        }
        Err(err) => report_error!(err.context(format!("Failed to convert {format:?} {uid}"))),
    }
}
