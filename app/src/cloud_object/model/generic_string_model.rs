use std::{fmt::Debug, sync::Arc};

use crate::server::cloud_objects::update_manager::InitiatedBy;
use crate::{
    appearance::Appearance,
    cloud_object::{
        CloudModelType, CloudObject, CloudObjectEventEntrypoint, CloudObjectUpsertParams,
        CreateCloudObjectResult, CreateObjectRequest, GenericCloudObject, GenericServerObject,
        GenericStringObjectFormat, GenericStringObjectUniqueKey, ObjectType, Revision,
        UpdateCloudObjectResult,
    },
    drive::{items::WarpDriveItem, CloudObjectTypeAndId},
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
pub use warp_server_client::cloud_object::{GenericStringModel, Serializer};
pub use warp_server_client::ids::GenericStringObjectId;

/// A trait that generic string-based objects should implement.
pub trait CloudStringObject: CloudObject + Send + Sync {
    /// Returns the object format for this object.
    fn generic_string_object_format(&self) -> GenericStringObjectFormat;

    /// Returns the id for this specific object.
    fn id(&self) -> SyncId;

    /// Returns a serialized model from this string object.
    fn serialized(&self) -> SerializedModel;

    /// Returns a cloned boxed version of this cloud object.
    /// Note that we can't force this trait to derive from Cloned
    /// directly because that would make the trait not object safe.  This
    /// is a workaround.
    fn clone_box(&self) -> Box<dyn CloudStringObject>;
}

/// A `StringModel` is a model that can be serialized and deserialized as a simple string.
///
/// Any model that has a simple string representation (e.g. JSON, markdown, yaml) that can be atomically updated
/// can implement this trait and get most cloud object functionality for free.
///
/// Objects that implement this type all share common storage and server apis.
pub trait StringModel: Clone + Debug + PartialEq + Send + Sync + 'static {
    type CloudObjectType: CloudObject + 'static;

    /// Returns the name of this model type (e.g. Workflow, Folder, Notebook)
    fn model_type_name(&self) -> &'static str;

    /// Whether we should enforce revisions for this model type.
    /// If revisions are not enforced, updates will have last-write-wins semantics.
    /// If revisions are enforced, the object will need to add logic to
    /// the update manager for how conflicts are resolved.
    fn should_enforce_revisions() -> bool;

    /// Returns the serialization format for this model.
    fn model_format() -> GenericStringObjectFormat;

    /// Whether to show update toasts for this type of model.
    fn should_show_activity_toasts() -> bool;

    /// Whether to show a warning if this type of model is unsaved at quit time
    /// (which typically blocks the user from quitting)
    fn warn_if_unsaved_at_quit() -> bool;

    /// Returns the display name for this model.
    fn display_name(&self) -> String;

    /// Returns whether to render this model as a WarpDriveItem.
    fn renders_in_warp_drive(&self) -> bool {
        false
    }

    /// Returns whether this model can be exported to a file
    fn can_export(&self) -> bool {
        false
    }

    /// Returns whether this model can be shared via a link
    fn supports_linking(&self) -> bool {
        false
    }

    /// Sets the display name for this model
    fn set_display_name(&mut self, _name: &str) {}

    /// Creates a new warp drive item for this model type. Returns None
    /// if this object does not render in Warp Drive.
    fn to_warp_drive_item(
        &self,
        _id: SyncId,
        _appearance: &Appearance,
        _object: &Self::CloudObjectType,
    ) -> Option<Box<dyn WarpDriveItem>> {
        None
    }

    /// Returns a sync queue item of this object that would allow it to be updated
    /// properly on the server.  Takes an optional revision_ts to set as the revision
    /// in the sync queue item.
    fn update_object_queue_item(
        &self,
        revision_ts: Option<Revision>,
        object: &Self::CloudObjectType,
    ) -> QueueItem;

    /// Returns whether this model type should clear on a unique key conflict.
    fn should_clear_on_unique_key_conflict(&self) -> bool {
        false
    }

    /// Returns a unique key for this object, if one exists. Unique keys are used
    /// to enforce that only one object with a given key can exist in the generic string
    /// object server database.
    fn uniqueness_key(&self) -> Option<GenericStringObjectUniqueKey>;
}

impl<M, S> CloudStringObject for GenericCloudObject<GenericStringObjectId, GenericStringModel<M, S>>
where
    M: StringModel<
        CloudObjectType = GenericCloudObject<GenericStringObjectId, GenericStringModel<M, S>>,
    >,
    S: Serializer<M>,
{
    fn generic_string_object_format(&self) -> GenericStringObjectFormat {
        M::model_format()
    }

    fn id(&self) -> SyncId {
        self.id
    }

    fn serialized(&self) -> SerializedModel {
        self.model().serialized()
    }

    fn clone_box(&self) -> Box<dyn CloudStringObject> {
        Box::new(self.clone())
    }
}

/// Implements the CloudModelType trait for all generic string models.
///
/// This has common logic for storing string models to SQLite, sending them to the server
/// updating from the server -- basically for anything not specific to the contents
/// of the string model.
#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
impl<M, S> CloudModelType for GenericStringModel<M, S>
where
    M: StringModel<
        CloudObjectType = GenericCloudObject<GenericStringObjectId, GenericStringModel<M, S>>,
    >,
    S: Serializer<M>,
{
    type CloudObjectType = GenericCloudObject<GenericStringObjectId, Self>;
    type IdType = GenericStringObjectId;

    fn model_type_name(&self) -> &'static str {
        self.string_model.model_type_name()
    }

    fn object_type(&self) -> ObjectType {
        ObjectType::GenericStringObject(M::model_format())
    }

    fn cloud_object_type_and_id(&self, id: SyncId) -> CloudObjectTypeAndId {
        CloudObjectTypeAndId::GenericStringObject {
            object_type: M::model_format(),
            id,
        }
    }

    fn display_name(&self) -> String {
        self.string_model.display_name()
    }

    fn set_display_name(&mut self, name: &str) {
        self.string_model.set_display_name(name);
    }

    fn upsert_event(params: CloudObjectUpsertParams<Self>) -> ModelEvent {
        let object = GenericCloudObject::<GenericStringObjectId, Self>::from(params);
        let object = &object as &dyn CloudStringObject;
        ModelEvent::UpsertGenericStringObject {
            object: CloudStringObject::clone_box(object),
        }
    }

    fn supports_linking(&self) -> bool {
        self.string_model.supports_linking()
    }

    fn should_show_activity_toasts(&self) -> bool {
        M::should_show_activity_toasts()
    }

    fn warn_if_unsaved_at_quit(&self) -> bool {
        M::warn_if_unsaved_at_quit()
    }

    fn can_export(&self) -> bool {
        self.string_model.can_export()
    }

    fn bulk_upsert_event(objects: Vec<CloudObjectUpsertParams<Self>>) -> ModelEvent {
        ModelEvent::UpsertGenericStringObjects(
            objects
                .into_iter()
                .map(|params| {
                    Box::new(GenericCloudObject::<GenericStringObjectId, Self>::from(
                        params,
                    )) as Box<dyn CloudStringObject>
                })
                .collect(),
        )
    }

    fn create_object_queue_item(
        &self,
        object: &GenericCloudObject<GenericStringObjectId, Self>,
        entrypoint: CloudObjectEventEntrypoint,
        initiated_by: InitiatedBy,
    ) -> Option<QueueItem> {
        if let SyncId::ClientId(client_id) = object.id {
            return Some(QueueItem::CreateObject {
                object_type: self.object_type(),
                owner: object.permissions.owner,
                id: client_id,
                title: None,
                serialized_model: Some(object.model().serialized().into()),
                initial_folder_id: object.metadata.folder_id,
                entrypoint,
                initiated_by,
            });
        }
        None
    }

    fn update_object_queue_item(
        &self,
        revision_ts: Option<Revision>,
        object: &GenericCloudObject<GenericStringObjectId, Self>,
    ) -> QueueItem {
        self.string_model
            .update_object_queue_item(revision_ts, object)
    }

    fn should_clear_on_unique_key_conflict(&self) -> bool {
        self.string_model.should_clear_on_unique_key_conflict()
    }

    fn should_update_after_server_conflict(&self) -> bool {
        true
    }

    fn serialized(&self) -> SerializedModel {
        S::serialize(&self.string_model)
    }

    async fn send_create_request(
        object_client: Arc<dyn ObjectClient>,
        request: CreateObjectRequest,
    ) -> Result<CreateCloudObjectResult> {
        let model_as_str = request
            .serialized_model
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Missing serialized model"))?
            .model_as_str();
        let model = S::deserialize_owned(model_as_str)?;
        object_client
            .create_generic_string_object(M::model_format(), model.uniqueness_key(), request)
            .await
    }

    async fn send_update_request(
        &self,
        object_client: Arc<dyn ObjectClient>,
        server_id: ServerId,
        revision: Option<Revision>,
    ) -> Result<UpdateCloudObjectResult<GenericServerObject<GenericStringObjectId, Self>>> {
        let revision =
            if M::should_enforce_revisions() {
                Some(revision.ok_or_else(|| {
                    anyhow::anyhow!("Missing revision on update of generic object")
                })?)
            } else {
                None
            };
        let res = object_client
            .update_generic_string_object(server_id.into(), self.serialized(), revision)
            .await;
        res.and_then(|update_result| match update_result {
            UpdateCloudObjectResult::Success {
                revision_and_editor,
            } => Ok(UpdateCloudObjectResult::Success {
                revision_and_editor,
            }),
            UpdateCloudObjectResult::Rejected { object } => {
                // Downcast to the concrete type to handle an update rejection (should be rare)
                let concrete_object: Option<&GenericServerObject<GenericStringObjectId, Self>> =
                    (&object).into();
                let object = concrete_object
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("Failed to convert object to concrete type"))?;
                Ok(UpdateCloudObjectResult::Rejected { object })
            }
        })
    }

    fn renders_in_warp_drive(&self) -> bool {
        self.string_model.renders_in_warp_drive()
    }

    fn to_warp_drive_item(
        &self,
        id: SyncId,
        appearance: &Appearance,
        object: &GenericCloudObject<GenericStringObjectId, Self>,
    ) -> Option<Box<dyn WarpDriveItem>> {
        self.string_model.to_warp_drive_item(id, appearance, object)
    }
}
