use std::collections::HashMap;

use anyhow::Result;
use async_channel::Sender;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use cloud_objects::{
    drive::sharing::SharingAccessLevel,
    ids::{FolderId, GenericStringObjectId, HashedSqliteId, ObjectUid, ServerId, SyncId},
};
#[cfg(any(test, feature = "test-util"))]
use mockall::automock;
use warp_graphql::{mcp_gallery_template::MCPGalleryTemplate, object_permissions::AccessLevel};

pub use cloud_object_models::*;
pub use cloud_objects::cloud_object::*;

/// Identifies a guest to remove from an object.
#[derive(Clone, Debug)]
pub enum GuestIdentifier {
    /// Removes a user guest by their email address.
    Email(String),
    /// Removes a team guest by their team UID.
    TeamUid(ServerId),
}

#[derive(Clone, Debug, PartialEq)]
pub enum ObjectActionType {
    Execute,
}

#[allow(clippy::to_string_trait_impl)]
impl ToString for ObjectActionType {
    fn to_string(&self) -> String {
        match self {
            ObjectActionType::Execute => String::from("EXECUTE"),
        }
    }
}

impl ObjectActionType {
    pub fn singular(&self) -> String {
        match self {
            ObjectActionType::Execute => "run".to_string(),
        }
    }

    pub fn plural(&self) -> String {
        match self {
            ObjectActionType::Execute => "runs".to_string(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ObjectAction {
    pub action_type: ObjectActionType,
    pub uid: ObjectUid,
    pub hashed_sqlite_id: HashedSqliteId,
    pub action_subtype: ObjectActionSubtype,
}

impl ObjectAction {
    pub fn is_pending(&self) -> bool {
        match self.action_subtype {
            ObjectActionSubtype::SingleAction { pending, .. } => pending,
            ObjectActionSubtype::BundledActions { .. } => false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ObjectActionHistory {
    pub uid: ObjectUid,
    pub hashed_sqlite_id: HashedSqliteId,
    pub latest_processed_at_timestamp: DateTime<Utc>,
    pub actions: Vec<ObjectAction>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ObjectActionSubtype {
    SingleAction {
        timestamp: DateTime<Utc>,
        processed_at_timestamp: Option<DateTime<Utc>>,
        data: Option<String>,
        pending: bool,
    },
    BundledActions {
        count: i32,
        oldest_timestamp: DateTime<Utc>,
        latest_timestamp: DateTime<Utc>,
        latest_processed_at_timestamp: DateTime<Utc>,
    },
}

#[derive(Default)]
pub struct InitialLoadResponse {
    pub updated_notebooks: Vec<ServerNotebook>,
    pub deleted_notebooks: Vec<cloud_object_models::NotebookId>,
    pub updated_workflows: Vec<ServerWorkflow>,
    pub deleted_workflows: Vec<WorkflowId>,
    pub updated_folders: Vec<ServerFolder>,
    pub deleted_folders: Vec<FolderId>,
    pub updated_generic_string_objects:
        HashMap<GenericStringObjectFormat, Vec<Box<dyn ServerObject>>>,
    pub deleted_generic_string_objects: Vec<GenericStringObjectId>,
    pub user_profiles: Vec<UserProfileWithUID>,
    pub action_histories: Vec<ObjectActionHistory>,
    pub mcp_gallery: Vec<MCPGalleryTemplate>,
}

pub struct GetCloudObjectResponse {
    pub object: ServerCloudObject,
    pub descendants: Vec<ServerCloudObject>,
    pub action_histories: Vec<ObjectActionHistory>,
}

#[derive(Debug, Clone)]
#[allow(clippy::enum_variant_names)]
pub enum ObjectUpdateMessage {
    ObjectMetadataChanged {
        metadata: ServerMetadata,
    },
    ObjectPermissionsChanged,
    ObjectPermissionsChangedV2 {
        object_uid: ServerId,
        permissions: ServerPermissions,
        user_profiles: Vec<UserProfileWithUID>,
    },
    ObjectContentChanged {
        server_object: Box<ServerCloudObject>,
        last_editor: Option<UserProfileWithUID>,
    },
    ObjectDeleted {
        object_uid: ServerId,
    },
    ObjectActionOccurred {
        history: ObjectActionHistory,
    },
    TeamMembershipsChanged,
    AmbientTaskUpdated {
        task_id: String,
        timestamp: DateTime<Utc>,
    },
}

impl ObjectUpdateMessage {
    pub fn as_str(&self) -> &'static str {
        use ObjectUpdateMessage::*;
        match self {
            ObjectMetadataChanged { .. } => "ObjectMetadataChanged",
            ObjectPermissionsChanged => "ObjectPermissionsChanged",
            ObjectPermissionsChangedV2 { .. } => "ObjectPermissionsChanged (V2)",
            ObjectContentChanged { .. } => "ObjectContentChanged",
            ObjectDeleted { .. } => "ObjectDeleted",
            ObjectActionOccurred { .. } => "ObjectActionOccurred",
            TeamMembershipsChanged => "TeamMembershipsChanged",
            AmbientTaskUpdated { .. } => "AmbientTaskUpdated",
        }
    }
}

#[derive(Clone, Debug)]
pub enum ObjectPermissionUpdateResult {
    Success,
    Failure,
}

#[derive(Clone, Debug)]
pub struct ObjectPermissionsUpdateData {
    pub permissions: ServerPermissions,
    pub profiles: Vec<UserProfileWithUID>,
}

#[derive(Clone, Debug)]
pub enum ObjectMetadataUpdateResult {
    Success { metadata: Box<ServerMetadata> },
    Failure,
}

pub enum ObjectDeleteResult {
    Success { deleted_ids: Vec<SyncId> },
    Failure,
}

#[cfg_attr(any(test, feature = "test-util"), automock)]
#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
pub trait ObjectClient: 'static + Send + Sync {
    /// This method saves a workflow for a given owner and returns it on success.
    async fn create_workflow(
        &self,
        request: CreateObjectRequest,
    ) -> Result<CreateCloudObjectResult>;

    /// Updates a workflow with the new data. The update may be rejected if a revision is specified and that revision is not the current revision of the object in storage.
    async fn update_workflow(
        &self,
        workflow_id: WorkflowId,
        data: SerializedModel,
        revision: Option<Revision>,
    ) -> Result<UpdateCloudObjectResult<ServerWorkflow>>;

    /// Creates multiple generic string objects in a single GraphQL request. Use this rather than calling `create_generic_string_object` multiple times in a loop.
    async fn bulk_create_generic_string_objects(
        &self,
        owner: Owner,
        objects: &[BulkCreateGenericStringObjectsRequest],
    ) -> Result<BulkCreateCloudObjectResult>;

    async fn create_generic_string_object(
        &self,
        format: GenericStringObjectFormat,
        uniqueness_key: Option<GenericStringObjectUniqueKey>,
        request: CreateObjectRequest,
    ) -> Result<CreateCloudObjectResult>;

    /// Creates a notebook on the server, returning the ID and revision of the object after creation.
    async fn create_notebook(
        &self,
        request: CreateObjectRequest,
    ) -> Result<CreateCloudObjectResult>;

    /// Updates a notebook with the new title and data. The update may be rejected if a revision is specified and that revision is not the current revision of the object in storage.
    async fn update_notebook(
        &self,
        notebook_id: cloud_object_models::NotebookId,
        title: Option<String>,
        data: Option<SerializedModel>,
        revision: Option<Revision>,
    ) -> Result<UpdateCloudObjectResult<ServerNotebook>>;

    async fn create_folder(&self, request: CreateObjectRequest) -> Result<CreateCloudObjectResult>;

    async fn update_folder(
        &self,
        folder_id: FolderId,
        name: SerializedModel,
    ) -> Result<UpdateCloudObjectResult<ServerFolder>>;

    async fn update_generic_string_object(
        &self,
        object_id: GenericStringObjectId,
        model: SerializedModel,
        revision: Option<Revision>,
    ) -> Result<UpdateCloudObjectResult<Box<dyn ServerObject>>>;

    /// Sets the current editor of the notebook to be the logged-in user.
    async fn grab_notebook_edit_access(
        &self,
        notebook_id: cloud_object_models::NotebookId,
    ) -> Result<ServerMetadata>;

    /// Sets the current editor of the notebook to null.
    async fn give_up_notebook_edit_access(
        &self,
        notebook_id: cloud_object_models::NotebookId,
    ) -> Result<ServerMetadata>;

    /// Gets updates for all Warp Drive actions.
    ///
    /// Starts a WebSocket connection against the corresponding GraphQL subscription.
    /// Messages received over the socket are sent over the `message_sender`.
    /// Once the WebSocket is live, a one-shot message is sent over `stream_ready_sender` to indicate so, because this method only returns once the WebSocket is closed.
    async fn get_warp_drive_updates(
        &self,
        message_sender: Sender<ObjectUpdateMessage>,
        stream_ready_sender: Sender<()>,
    ) -> Result<()>;

    async fn fetch_changed_objects(
        &self,
        objects_to_update: ObjectsToUpdate,
        force_refresh: bool,
    ) -> Result<InitialLoadResponse>;

    async fn fetch_single_cloud_object(&self, id: ServerId) -> Result<GetCloudObjectResponse>;

    /// Transfers a notebook to the given owner.
    async fn transfer_notebook_owner(
        &self,
        notebook_id: cloud_object_models::NotebookId,
        owner: Owner,
    ) -> Result<bool>;

    async fn transfer_workflow_owner(&self, workflow_id: WorkflowId, owner: Owner) -> Result<bool>;

    async fn transfer_generic_string_object_owner(
        &self,
        workflow_id: GenericStringObjectId,
        owner: Owner,
    ) -> Result<bool>;

    async fn trash_object(&self, id: ServerId) -> Result<bool>;

    async fn untrash_object(&self, id: ServerId) -> Result<ObjectMetadataUpdateResult>;

    async fn delete_object(&self, id: ServerId) -> Result<ObjectDeleteResult>;

    async fn empty_trash(&self, owner: Owner) -> Result<ObjectDeleteResult>;

    async fn move_object(
        &self,
        id: ServerId,
        folder_id: Option<FolderId>,
        owner: Owner,
        object_type: ObjectType,
    ) -> Result<bool>;

    async fn record_object_action(
        &self,
        id: ServerId,
        action_type: ObjectActionType,
        timestamp: DateTime<Utc>,
        data: Option<String>,
    ) -> Result<ObjectActionHistory>;

    async fn leave_object(&self, id: ServerId) -> Result<ObjectDeleteResult>;

    async fn set_object_link_permissions(
        &self,
        object_id: ServerId,
        access_level: SharingAccessLevel,
    ) -> Result<ObjectPermissionUpdateResult>;

    async fn remove_object_link_permissions(
        &self,
        object_id: ServerId,
    ) -> Result<ObjectPermissionUpdateResult>;

    async fn add_object_guests(
        &self,
        object_id: ServerId,
        guest_emails: Vec<String>,
        access_level: AccessLevel,
    ) -> Result<ObjectPermissionsUpdateData>;

    async fn update_object_guests(
        &self,
        object_id: ServerId,
        guest_emails: Vec<String>,
        access_level: AccessLevel,
    ) -> Result<ServerPermissions>;

    async fn remove_object_guest(
        &self,
        object_id: ServerId,
        guest: GuestIdentifier,
    ) -> Result<ServerPermissions>;

    /// Fetches the last-used timestamps for all cloud environments.
    ///
    /// This is derived from `CloudEnvironment.lastTaskCreated.createdAt`, not `lastTaskRunTimestamp`, so that "Last used" reflects the most recently created task.
    ///
    /// Returns a map from environment UID to timestamp.
    async fn fetch_environment_last_task_run_timestamps(
        &self,
    ) -> Result<HashMap<String, DateTime<Utc>>>;
}
