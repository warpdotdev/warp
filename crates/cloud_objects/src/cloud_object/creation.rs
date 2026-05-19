use warp_graphql::scalars::time::ServerTimestamp;

use crate::ids::{ClientId, FolderId, ServerIdAndType};

use super::{
    CloudObjectEventEntrypoint, GenericStringObjectFormat, GenericStringObjectUniqueKey, Owner,
    RevisionAndLastEditor, SerializedModel, ServerPermissions,
};

/// Helper struct that contains all the info needed to create an object on the server.
pub struct CreateObjectRequest {
    pub serialized_model: Option<SerializedModel>,
    pub title: Option<String>,
    pub owner: Owner,
    pub client_id: ClientId,
    pub initial_folder_id: Option<FolderId>,
    pub entrypoint: CloudObjectEventEntrypoint,
}

#[derive(PartialEq, Eq, Debug)]
pub struct BulkCreateGenericStringObjectsRequest {
    pub id: ClientId,
    pub format: GenericStringObjectFormat,
    pub uniqueness_key: Option<GenericStringObjectUniqueKey>,
    pub serialized_model: SerializedModel,
    pub initial_folder_id: Option<FolderId>,
    pub entrypoint: CloudObjectEventEntrypoint,
}

/// The data returned by the server when an object is created, generic to any object type.
#[derive(Debug)]
pub struct CreatedCloudObject {
    pub client_id: ClientId,
    pub revision_and_editor: RevisionAndLastEditor,
    pub metadata_ts: ServerTimestamp,
    pub server_id_and_type: ServerIdAndType,
    pub creator_uid: Option<String>,
    pub permissions: ServerPermissions,
}

/// Result of attempting to create a cloud object.
#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub enum CreateCloudObjectResult {
    /// The object creation was successful.
    Success {
        created_cloud_object: CreatedCloudObject,
    },
    /// The object creation was denied due to an expected user error.
    UserFacingError(String),
    /// The object creation was rejected because the generic string object had
    /// already been created by another client.
    GenericStringObjectUniqueKeyConflict,
}

/// Result of attempting to bulk create a cloud object.
#[derive(Debug)]
pub enum BulkCreateCloudObjectResult {
    /// The bulk object creation was successful.
    Success {
        created_cloud_objects: Vec<CreatedCloudObject>,
    },
    /// The bulk object creation was rejected because at least one generic string object had
    /// already been created by another client.
    GenericStringObjectUniqueKeyConflict,
}

/// The creation-specific data returned by the server, which is inserted into CloudModel and persisted
/// just once.
#[derive(Debug, PartialEq, Clone)]
pub struct ServerCreationInfo {
    pub server_id_and_type: ServerIdAndType,
    pub creator_uid: Option<String>,
    pub permissions: ServerPermissions,
}
