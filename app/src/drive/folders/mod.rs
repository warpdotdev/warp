use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
pub use cloud_object_models::{CloudFolder, CloudFolderModel};
// Re-exported from warp_server_client.
pub use warp_server_client::ids::FolderId;

use super::CloudObjectTypeAndId;
use super::items::WarpDriveItem;
// Re-exported from warp_server_client.
use super::items::folder::WarpDriveFolder;
use crate::appearance::Appearance;
use crate::cloud_object::{
    CloudModelType, CloudObjectEventEntrypoint, CloudObjectUpsertParams, CreateCloudObjectResult,
    CreateObjectRequest, GenericServerObject, ObjectType, Revision, Space, UpdateCloudObjectResult,
};
use crate::persistence::ModelEvent;
use crate::server::cloud_objects::update_manager::InitiatedBy;
use crate::server::ids::{ServerId, SyncId};
use crate::server::server_api::object::ObjectClient;
use crate::server::sync_queue::{QueueItem, SerializedModel};

#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
impl CloudModelType for CloudFolderModel {
    type CloudObjectType = CloudFolder;
    type IdType = FolderId;

    fn model_type_name(&self) -> &'static str {
        "Folder"
    }

    fn object_type(&self) -> ObjectType {
        ObjectType::Folder
    }

    fn cloud_object_type_and_id(&self, id: SyncId) -> CloudObjectTypeAndId {
        CloudObjectTypeAndId::Folder(id)
    }

    fn display_name(&self) -> String {
        self.name.clone()
    }

    fn upsert_event(params: CloudObjectUpsertParams<Self>) -> ModelEvent {
        ModelEvent::UpsertFolder {
            folder: CloudFolder::from(params),
        }
    }

    fn bulk_upsert_event(objects: Vec<CloudObjectUpsertParams<Self>>) -> ModelEvent {
        ModelEvent::UpsertFolders(objects.into_iter().map(CloudFolder::from).collect())
    }

    fn create_object_queue_item(
        &self,
        folder: &CloudFolder,
        entrypoint: CloudObjectEventEntrypoint,
        initiated_by: InitiatedBy,
    ) -> Option<QueueItem> {
        if let SyncId::ClientId(client_id) = folder.id {
            return Some(QueueItem::CreateObject {
                object_type: self.object_type(),
                serialized_model: Some(Arc::new(folder.model().name.clone().into())),
                title: None,
                owner: folder.permissions.owner,
                id: client_id,
                initial_folder_id: folder.metadata.folder_id,
                entrypoint,
                initiated_by,
            });
        }
        None
    }

    fn update_object_queue_item(
        &self,
        _revision_ts: Option<Revision>,
        folder: &CloudFolder,
    ) -> QueueItem {
        QueueItem::UpdateFolder {
            id: folder.id,
            model: folder.model().clone().into(),
        }
    }

    fn should_update_after_server_conflict(&self) -> bool {
        false
    }

    fn serialized(&self) -> SerializedModel {
        SerializedModel::new(self.name.to_owned())
    }

    fn can_move_to_space(&self, current_space: Space, new_space: Space) -> bool {
        // We don't currently support moving folders across spaces.
        current_space == new_space
    }

    fn supports_linking(&self) -> bool {
        true
    }

    async fn send_create_request(
        object_client: Arc<dyn ObjectClient>,
        request: CreateObjectRequest,
    ) -> Result<CreateCloudObjectResult> {
        object_client.create_folder(request).await
    }

    async fn send_update_request(
        &self,
        object_client: Arc<dyn ObjectClient>,
        server_id: ServerId,
        _revision: Option<Revision>,
    ) -> Result<UpdateCloudObjectResult<GenericServerObject<FolderId, Self>>> {
        object_client
            .update_folder(server_id.into(), self.name.clone().into())
            .await
    }

    fn renders_in_warp_drive(&self) -> bool {
        true
    }

    fn to_warp_drive_item(
        &self,
        id: SyncId,
        _appearance: &Appearance,
        folder: &CloudFolder,
    ) -> Option<Box<dyn WarpDriveItem>> {
        Some(Box::new(WarpDriveFolder::new(
            self.cloud_object_type_and_id(id),
            folder.clone(),
        )))
    }
}
