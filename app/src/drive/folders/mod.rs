use std::sync::Arc;

use super::items::folder::WarpDriveFolder;
use super::items::WarpDriveItem;
use super::CloudObjectTypeAndId;
use crate::server::cloud_objects::update_manager::InitiatedBy;
use crate::{
    appearance::Appearance,
    cloud_object::{
        CloudModelType, CloudObjectEventEntrypoint, CreateCloudObjectResult, CreateObjectRequest,
        GenericCloudObject, GenericServerObject, ObjectType, Revision, Space,
        UpdateCloudObjectResult,
    },
    persistence::ModelEvent,
    server::{
        ids::{ServerId, SyncId},
        server_api::object::ObjectClient,
        sync_queue::{QueueItem, SerializedModel},
    },
};
use anyhow::Result;
use async_trait::async_trait;

// Re-exported from warp_server_client.
pub use warp_server_client::ids::FolderId;

/// The model for a `CloudFolder`.
#[derive(Clone, Debug, PartialEq)]
pub struct CloudFolderModel {
    pub name: String,
    // TODO: since this is local only state, we should consider only surfacing it as part of the
    // CloudViewModel. Right now, every server folder uses CloudFolderModel, which means it
    // hardcodes a value of `false` for this property since it can't know what the local state is.
    pub is_open: bool,
    pub is_warp_pack: bool,
}

impl CloudFolderModel {
    pub fn new(name: &str, is_warp_pack: bool) -> Self {
        Self {
            name: name.to_owned(),
            is_open: false,
            is_warp_pack,
        }
    }
}

/// `CloudFolder` is a folder retrieved from the server.
pub type CloudFolder = GenericCloudObject<FolderId, CloudFolderModel>;

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

    fn upsert_event(&self, folder: &CloudFolder) -> ModelEvent {
        ModelEvent::UpsertFolder {
            folder: folder.clone(),
        }
    }

    fn bulk_upsert_event(objects: &[CloudFolder]) -> ModelEvent {
        ModelEvent::UpsertFolders(objects.to_vec())
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
