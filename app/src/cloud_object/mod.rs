use self::{
    breadcrumbs::ContainingObject,
    model::{
        actions::ObjectActions,
        generic_string_model::{
            GenericStringModel, GenericStringObjectId, Serializer, StringModel,
        },
        persistence::CloudModel,
    },
};
use crate::server::cloud_objects::update_manager::InitiatedBy;
use crate::{
    ai::cloud_agent_config::CloudAgentConfigModel,
    ai::cloud_environments::CloudAmbientAgentEnvironmentModel,
    ai::{
        ambient_agents::scheduled::CloudScheduledAmbientAgentModel,
        document::ai_document_model::AIDocumentId,
        execution_profiles::CloudAIExecutionProfileModel,
        facts::CloudAIFactModel,
        mcp::{templatable::CloudTemplatableMCPServerModel, CloudMCPServerModel},
    },
    appearance::Appearance,
    auth::UserUid,
    channel::ChannelState,
    drive::{
        folders::{CloudFolderModel, FolderId},
        items::WarpDriveItem,
        CloudObjectTypeAndId, OpenWarpDriveObjectArgs, OpenWarpDriveObjectSettings,
    },
    env_vars::CloudEnvVarCollectionModel,
    notebooks::{CloudNotebookModel, NotebookId},
    persistence::ModelEvent,
    server::{
        ids::{HashableId, HashedSqliteId, ObjectUid, ServerId, SyncId, ToServerId},
        server_api::object::ObjectClient,
        sync_queue::{QueueItem, SerializedModel},
    },
    settings::cloud_preferences::CloudPreferenceModel,
    util::time_format::format_approx_duration_from_now_utc,
    workflows::{
        workflow_enum::CloudWorkflowEnumModel, CloudWorkflow, CloudWorkflowModel, WorkflowId,
        WorkflowSource,
    },
    workspaces::{
        user_profiles::{UserProfileWithUID, UserProfiles},
        user_workspaces::UserWorkspaces,
    },
};
use anyhow::Result;
use async_trait::async_trait;
use chrono::{Duration, Utc};
use derivative::Derivative;
use lazy_static::lazy_static;
use regex::Regex;
use std::{
    any::Any,
    collections::{HashMap, HashSet},
    fmt::Debug,
    sync::Arc,
};
use url::Url;
use warp_core::{channel::Channel, features::FeatureFlag};
use warp_graphql::{
    queries::get_updated_cloud_objects::UpdatedObjectInput, scalars::time::ServerTimestamp,
};
use warpui::{AppContext, SingletonEntity};

pub mod breadcrumbs;
pub mod grab_edit_access_modal;
pub mod model;
pub mod toast_message;

pub use warp_server_client::cloud_object::*;

/// A CloudObject represents
/// therefore shareable and editable (i.e. Notebooks and Workflows). In order
/// to support collaborative editing of these objects, they must each store local
/// revision numbers to ensure a stable way of accepting and rejecting edits.
///
/// Note that this trait must be object-safe and non-generic.  The reason for this
/// is that (a) we need to be able to store instances of it as trait objects in
/// CloudModel and (b) we need to be able to support mixed collections of different
/// instances of it (e.g. in the map of id -> CloudObject in CloudModel).
///
/// There are two closely related types to this:
/// 1) GenericCloudObject: This is the concrete generic implementation of CloudObject that
///    holds onto a model of type CloudModelType and an id of type SyncId.
/// 2) CloudModelType: This is a trait that defines the model type for a CloudObject -
///    this is what implementors of new cloud types typically have to implement.
///
/// These types are tightly coupled.  In an ideal world, rust would allow a mechanism
/// for us having a single interface that new model types could implement that could
/// be generic on id and model types, but as far as I (zach) can tell, that's not currently
/// possible.
///
/// The typical usage pattern for these types is to use dyn CloudObject whenever you
/// don't need access to a model or id, and to downcast to a GenericCloudObject whenever you do.
///
/// This implies that, for now, *all* CloudObjects must implement GenericCloudObject.
///
/// Additionally, they must support the "grab the baton" UX for editing, where any
/// user can grab edit access of an object, revoking it from anyone else currently
/// editing.
///
/// For more info on revisions: https://docs.google.com/document/d/1SGtX_5AiSJmUxXCRk5NzGTzrC_XrxQRsio-KZOec_ng/edit
/// And grab the baton: https://docs.google.com/document/d/1LgGaz8bB40AONTzC0ZFOw5Kg0SD8_RM10V_nyt3zOvY/edit#heading=h.tcup5oqi82p4
pub trait CloudObject: Debug {
    /// Returns the name of this model type (e.g. Workflow, Folder, Notebook)
    fn model_type_name(&self) -> &'static str;

    /// Returns the  uid for this object.
    fn uid(&self) -> ObjectUid;

    /// Returns the [`SyncId`] that currently identifies this object.
    fn sync_id(&self) -> SyncId;

    /// Returns the id used to index into sqlite, this is the object's UID with its type
    /// prefixed, such as "Workflow-{UID}"
    fn hashed_sqlite_id(&self) -> HashedSqliteId;

    /// Returns the CloudObjectMetadata struct associated with this object.
    fn metadata(&self) -> &CloudObjectMetadata;

    /// Returns a mutable reference to the CloudObjectMetadata struct associated with this object.
    fn metadata_mut(&mut self) -> &mut CloudObjectMetadata;

    /// Returns the CloudObjectPermissions struct associated with this object.
    fn permissions(&self) -> &CloudObjectPermissions;

    /// Returnsa mutable reference to the CloudObjectPermissions struct associated with this object.
    fn permissions_mut(&mut self) -> &mut CloudObjectPermissions;

    /// Returns the ObjectType i.e. 'Workflow' or 'Notebook'
    fn object_type(&self) -> ObjectType;

    /// Returns the CloudObjectTypeAndId for this object.
    fn cloud_object_type_and_id(&self) -> CloudObjectTypeAndId;

    /// Sets the server id on this object.
    fn set_server_id(&mut self, server_id: ServerId);

    /// Returns whether this object can be moved to the given space.
    fn can_move_to_space(&self, _space: Space, _app: &AppContext) -> bool {
        true
    }

    // Whether to clear this object from the local SQLite DB on a unique key conflict.
    fn should_clear_on_unique_key_conflict(&self) -> bool {
        false
    }

    /// Whether to show a warning if this object is unsaved at quit time
    /// (which typically blocks the user from quitting)
    fn warn_if_unsaved_at_quit(&self) -> bool {
        true
    }

    /// Returns the "upsert" event for inserting / updating this object in the SQLite DB.
    fn upsert_event(&self) -> ModelEvent;

    // Returns the name of the object.
    fn display_name(&self) -> String;

    /// Returns an optional UpdatedObjectInput to use during initial load, where
    /// the object's timestamps are sent to the server for comparison
    fn versions(&self, app: &AppContext) -> Option<UpdatedObjectInput>;

    /// Returns an optional sync queue item of this object that would allow it to
    /// created properly on the server. Returns None if it's already been created
    /// server-side.
    fn create_object_queue_item(
        &self,
        entrypoint: CloudObjectEventEntrypoint,
        initiated_by: InitiatedBy,
    ) -> Option<QueueItem>;

    /// Returns a sync queue item of this object that would allow it to be updated
    /// properly on the server.  Takes an optional revision_ts to set as the revision
    /// in the sync queue item.
    fn update_object_queue_item(&self, revision_ts: Option<Revision>) -> QueueItem;

    /// Returns whether this model type should render as a warp drive item.
    fn renders_in_warp_drive(&self) -> bool;

    /// Returns whether this model type should show update toasts in the UI.
    fn should_show_activity_toasts(&self) -> bool {
        true
    }

    /// Creates a new Warp Drive item for this object.  Returns None if this
    /// object is not rendered in Warp Drive.
    fn to_warp_drive_item(&self, appearance: &Appearance) -> Option<Box<dyn WarpDriveItem>>;

    /// Returns the web link of this object. Will return none if we do not support web links
    /// for this particular object (i.e. if it's not yet sync'd to the server, or if we don't
    /// yet support linking to that object type).
    ///
    /// The format of an objects link follows the pattern:
    /// {channel}/drive/{object-type}/{object-name}-{uid}. For more information on this,
    /// see the linkable objects PRD (https://docs.google.com/document/d/1VQZ4sgLs4M9r2NDYyecfOalLlPmcf2fd_rDdqG35Zd8/edit)
    /// or tech doc (https://docs.google.com/document/d/1_TK19mRcD_0eLwbr5uFRabacIzfKocfahjEvoRcs5ko/edit)
    fn object_link(&self) -> Option<String>;

    /// The space containing this object.
    ///
    /// If the object is shared with the current user, the space will reflect that, not the
    /// object's actual owner.
    fn space(&self, app: &AppContext) -> Space {
        UserWorkspaces::as_ref(app).owner_to_space(self.permissions().owner, app)
    }

    /// Whether or not this object can be "left". For shared objects, this removes all ACLs for the
    /// current user. Only top-level items in the shared space can be left.
    fn can_leave(&self, app: &AppContext) -> bool {
        if self.space(app) == Space::Shared {
            self.metadata()
                .folder_id
                .is_none_or(|parent| CloudModel::as_ref(app).get_folder(&parent).is_none())
        } else {
            false
        }
    }

    /// Returns the name of the containing "object" for this object.
    /// This could be a folder, or in the case of top-level objects,
    /// the name of the space it belongs to.
    fn containing_object_name(&self, app: &AppContext) -> String {
        self.containing_objects_path(app)
            .into_iter()
            .next_back()
            .expect("Object should have at least one ancestor")
            .name
    }

    // Returns the path of all the containing "objects" for this object.
    // This could include folders or spaces.
    fn containing_objects_path(&self, app: &AppContext) -> Vec<ContainingObject> {
        let space = self.space(app);

        match self.metadata().folder_id {
            Some(folder_id) => {
                let cloud_model = CloudModel::as_ref(app);
                if let Some(folder) = cloud_model.get_folder_by_uid(&folder_id.uid()) {
                    let mut path = vec![];
                    let ancestors = folder.containing_objects_path(app);
                    path.extend(ancestors);
                    path.push(folder.into());
                    path
                } else {
                    // if for whatever reason the folder id is messed up,
                    // just default to showing the top-level space it wound up in
                    vec![space.into_containing_object(app)]
                }
            }
            None => vec![space.into_containing_object(app)],
        }
    }

    fn breadcrumbs(&self, app: &AppContext) -> String {
        self.containing_objects_path(app)
            .into_iter()
            .map(|object| object.name)
            .collect::<Vec<String>>()
            .join(" / ")
    }

    /// Returns whether this CloudObject is in the given space
    fn is_in_space(&self, space: Space, app: &AppContext) -> bool {
        self.space(app) == space
    }

    fn is_welcome_object(&self) -> bool {
        self.metadata().is_welcome_object
    }

    /// Returns the direct location of the object. If the object
    /// is not in a folder, this will be the object's space. Otherwise, it will
    /// be the folder the object is placed in directly, even if that folder is nested.
    fn location(&self, cloud_model: &CloudModel, app: &AppContext) -> CloudObjectLocation {
        if let Some(folder_id) = self.metadata().folder_id {
            if cloud_model.get_folder(&folder_id).is_some() {
                return CloudObjectLocation::Folder(folder_id);
            }
        }

        CloudObjectLocation::Space(self.space(app))
    }

    /// Return true is this object or any of its ancestors are trashed. Also returns true
    /// if a cycle is detected.
    fn is_trashed(&self, cloud_model: &CloudModel) -> bool {
        self.is_trashed_internal(cloud_model, &mut HashSet::new())
    }

    /// Helper function for is_trashed.
    fn is_trashed_internal(
        &self,
        cloud_model: &CloudModel,
        ancestors: &mut HashSet<String>,
    ) -> bool {
        // Base case: If the object is trashed, return true.
        if self.metadata().trashed_ts.is_some() {
            return true;
        }

        // Else: return true if the object's parent is trashed. Return false if the object has no parent.
        match self.metadata().folder_id.map(|parent_id| parent_id.uid()) {
            Some(hashed_parent_id) => {
                // We need to check for cycles to avoid causing a stack overflow. If a cycle is detected, return that the object is trashed.
                if ancestors.contains(&hashed_parent_id) {
                    return true;
                }
                ancestors.insert(hashed_parent_id.clone());

                match cloud_model.get_by_uid(&hashed_parent_id) {
                    Some(parent) => parent.is_trashed_internal(cloud_model, ancestors),
                    None => {
                        // If the object has a parent, but the parent is not in CloudModel, assume
                        // the object is shared, but not its parent. For backwards compatibility,
                        // if sharing is disabled, default to trashed rather than untrashed.
                        !FeatureFlag::SharedWithMe.is_enabled()
                    }
                }
            }
            None => false,
        }
    }

    /// Returns whether this object has conflicting changes with the server.
    fn has_conflicting_changes(&self) -> bool;

    /// Returns the revision of the conflicting object, if any.
    /// This is used for object-safe access to conflict information.
    fn conflicting_object_revision(&self) -> Option<Revision>;

    /// Clears the conflict status back to NoConflicts.
    fn clear_conflict_status(&mut self);

    /// Updates the object to deal with any conflict status.
    fn replace_object_with_conflict(&mut self);

    /// Sets the content sync status of this object to `InFlight` (if it wasn't already) and
    /// increments the number of in flight requests tracked in the `InFlight` enum.
    fn increment_in_flight_request_count(&mut self) {
        let new_reqs = match &self.metadata().pending_changes_statuses.content_sync_status {
            CloudObjectSyncStatus::InFlight(reqs) => reqs.0 + 1,
            _ => 1,
        };

        self.set_pending_content_changes_status(CloudObjectSyncStatus::InFlight(
            NumInFlightRequests(new_reqs),
        ))
    }

    /// Decrements the number of in flight requests tracked in this object's `InFlight` enum. If
    /// that number becomes 0, it's no longer in flight, so it will be set to `status_if_no_reqs`.
    /// Returns true if the object is no longer in flight.
    fn decrement_in_flight_request_count(
        &mut self,
        status_if_no_reqs: CloudObjectSyncStatus,
    ) -> bool {
        match &self.metadata().pending_changes_statuses.content_sync_status {
            CloudObjectSyncStatus::InFlight(reqs) => {
                if reqs.0 - 1 == 0 {
                    self.set_pending_content_changes_status(status_if_no_reqs);
                    return true;
                } else {
                    self.set_pending_content_changes_status(CloudObjectSyncStatus::InFlight(
                        NumInFlightRequests(reqs.0 - 1),
                    ));
                    return false;
                }
            }
            _ => log::error!(
                "called decrement_in_flight_request_count with a non-`InFlight` cloud status"
            ),
        }

        true
    }

    /// Sets the content change status on this object's metadata
    fn set_pending_content_changes_status(
        &mut self,
        pending_content_changes_status: CloudObjectSyncStatus,
    ) {
        self.metadata_mut()
            .pending_changes_statuses
            .content_sync_status = pending_content_changes_status;
    }

    /// Whether or not this object can be exported.
    fn can_export(&self) -> bool;

    /// Returns this object as a ref to the Any type.  Needed for typecasts.
    fn as_any(&self) -> &dyn Any;

    /// Returns this object as a mut ref to Any type.  Needed for typecasts.
    fn as_any_mut(&mut self) -> &mut dyn Any;

    /// Returns the trait object as a concrete type reference by downcasting it.
    /// Returns None if the downcast fails.
    fn as_model_type<K, M>(cloud_object: &dyn CloudObject) -> Option<&GenericCloudObject<K, M>>
    where
        Self: Sized,
        K: HashableId + ToServerId + Debug + Into<String> + Clone + 'static,
        M: CloudModelType<IdType = K, CloudObjectType = GenericCloudObject<K, M>> + 'static,
    {
        cloud_object
            .as_any()
            .downcast_ref::<GenericCloudObject<K, M>>()
    }

    /// Returns the trait object as a concrete mutable type reference by downcasting it.
    /// Returns None if the downcast fails.
    fn as_model_type_mut<K, M>(
        cloud_object: &mut dyn CloudObject,
    ) -> Option<&mut GenericCloudObject<K, M>>
    where
        Self: Sized,
        K: HashableId + ToServerId + Debug + Into<String> + Clone + 'static,
        M: CloudModelType<IdType = K, CloudObjectType = GenericCloudObject<K, M>> + 'static,
    {
        cloud_object
            .as_any_mut()
            .downcast_mut::<GenericCloudObject<K, M>>()
    }

    /// Returns a cloned boxed version of this cloud object.
    /// Note that we can't force the CloudObject trait to derive from Cloned
    /// directly because that would make the trait not object safe.  This
    /// is a workaround.
    fn clone_box(&self) -> Box<dyn CloudObject>;
}

/// Defines a common trait for cloud models to implement.
/// The "model" is the domain specific piece of data for a cloud object,
/// e.g. it contains the notebook, workflow, or folder specific data, but has
/// no logic around metadata, permissions, or sync status.
///
/// See the comments for CloudObject to understand the relationship between
/// this trait, CloudObject and GenericCloudObject.  They are tightly coupled.
///
/// When building new model types (e.g. for settings or launch configs) we should just
/// have to implement this trait, and not the entire CloudObject trait.
#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
pub trait CloudModelType: Debug + Clone + Send + Sync {
    /// The associated CloudObject type for this model (e.g. CloudNotebook, CloudWorkflow, etc)
    type CloudObjectType: CloudObject + 'static;
    // TODO: @ianhodge - remove for sync ID refactor.
    type IdType: HashableId + ToServerId + Debug + Into<String> + Clone + 'static;

    /// Returns the name of this model type (e.g. Workflow, Folder, Notebook)
    fn model_type_name(&self) -> &'static str;

    /// Returns the CloudObjectTypeAndId for this object.
    fn cloud_object_type_and_id(&self, id: SyncId) -> CloudObjectTypeAndId;

    /// Returns the ObjectType for this model.
    fn object_type(&self) -> ObjectType;

    /// Returns whether this model type should render as a warp drive item.
    fn renders_in_warp_drive(&self) -> bool;

    /// Returns whether this model type should show update toasts in the UI.
    fn should_show_activity_toasts(&self) -> bool {
        true
    }

    /// Whether to show a warning if this model is unsaved at quit time
    /// (which typically blocks the user from quitting)
    fn warn_if_unsaved_at_quit(&self) -> bool {
        true
    }

    /// Creates a new warp drive item for this model type. Returns None
    /// if this object does not render in Warp Drive.
    fn to_warp_drive_item(
        &self,
        id: SyncId,
        appearance: &Appearance,
        object: &Self::CloudObjectType,
    ) -> Option<Box<dyn WarpDriveItem>>;

    /// Returns the display name for this model (e.g. to show in the Warp Drive index)
    fn display_name(&self) -> String;

    /// Sets the display name to show in the Warp Drive Index.  Setting the name
    /// is not currently supported by all object types, hence the default empty
    /// implementation.
    fn set_display_name(&mut self, _name: &str) {}

    /// Returns the upsert event for putting this model into the SQLite database.
    fn upsert_event(params: CloudObjectUpsertParams<Self>) -> ModelEvent
    where
        Self: Sized;

    /// Returns a bulk upsert event for putting a list of this model into the SQLite database.
    fn bulk_upsert_event(objects: Vec<CloudObjectUpsertParams<Self>>) -> ModelEvent
    where
        Self: Sized;

    /// Returns the sync queue item for creating this model on the server.
    fn create_object_queue_item(
        &self,
        object: &Self::CloudObjectType,
        entrypoint: CloudObjectEventEntrypoint,
        initiated_by: InitiatedBy,
    ) -> Option<QueueItem>;

    /// Returns the sync queue item for updating this model on the server.
    /// Takes an optional revision timestamp to set in the queue item.
    fn update_object_queue_item(
        &self,
        revision_ts: Option<Revision>,
        object: &Self::CloudObjectType,
    ) -> QueueItem;

    /// Returns a serialized model.
    fn serialized(&self) -> SerializedModel;

    /// Sends a request to the server to create this model.
    async fn send_create_request(
        object_client: Arc<dyn ObjectClient>,
        request: CreateObjectRequest,
    ) -> Result<CreateCloudObjectResult>;

    /// Sends a request to the server to update this model.
    async fn send_update_request(
        &self,
        object_client: Arc<dyn ObjectClient>,
        server_id: ServerId,
        revision: Option<Revision>,
    ) -> Result<UpdateCloudObjectResult<GenericServerObject<Self::IdType, Self>>>;

    /// Returns whether this model type supports being moved to the given space.
    fn can_move_to_space(&self, _current_space: Space, _new_space: Space) -> bool {
        true
    }

    /// Returns whether this model type should clear on a unique key conflict.
    fn should_clear_on_unique_key_conflict(&self) -> bool {
        false
    }

    /// Returns whether this model type supports web links
    fn supports_linking(&self) -> bool {
        true
    }
    /// Returns whether this model type should be updated after a server conflict.
    /// Note that for now the only model type that this is relevant for is Notebooks,
    /// where we show a banner in case of conflicts and ask users to manually take action.
    /// For other types we typically just want to replace the local object with the server
    /// revision, which doesn't go through this code path.
    fn should_update_after_server_conflict(&self) -> bool;

    /// Whether this model type can be exported.
    fn can_export(&self) -> bool {
        false
    }
}
/// Provides app-local typed lookup helpers for generic cloud object aliases.
pub trait CloudObjectLookup: Sized + Clone {
    fn get_all(app: &AppContext) -> Vec<Self>;

    fn get_by_id<'a>(sync_id: &'a SyncId, app: &'a AppContext) -> Option<&'a Self>;
}

impl<K, M> CloudObjectLookup for GenericCloudObject<K, M>
where
    K: HashableId + ToServerId + Debug + Into<String> + Clone + 'static,
    M: CloudModelType<IdType = K, CloudObjectType = GenericCloudObject<K, M>> + 'static,
{
    fn get_all(app: &AppContext) -> Vec<Self> {
        CloudModel::as_ref(app)
            .get_all_objects_of_type::<K, M>()
            .cloned()
            .collect()
    }

    fn get_by_id<'a>(sync_id: &'a SyncId, app: &'a AppContext) -> Option<&'a Self> {
        CloudModel::as_ref(app).get_object_of_type::<K, M>(sync_id)
    }
}

/// Marks string model payloads that can be looked up by UUID.
pub trait CloudObjectUuid {
    fn uuid(&self) -> uuid::Uuid;
}

/// Provides app-local UUID lookups for cloud objects whose payload exposes a UUID.
pub trait CloudObjectUuidLookup: Sized {
    fn get_by_uuid<'a>(uuid: &'a uuid::Uuid, app: &'a AppContext) -> Option<&'a Self>;
}

impl<T, S> CloudObjectUuidLookup
    for GenericCloudObject<GenericStringObjectId, GenericStringModel<T, S>>
where
    T: StringModel<
            CloudObjectType = GenericCloudObject<GenericStringObjectId, GenericStringModel<T, S>>,
        > + CloudObjectUuid,
    S: Serializer<T>,
{
    fn get_by_uuid<'a>(uuid: &'a uuid::Uuid, app: &'a AppContext) -> Option<&'a Self> {
        CloudModel::as_ref(app)
            .get_all_objects_of_type::<GenericStringObjectId, GenericStringModel<T, S>>()
            .find(|object| object.model().string_model.uuid() == *uuid)
    }
}

lazy_static! {
    static ref SPACE_DETECT_RE: Regex = Regex::new(r"\s+").expect("Expect regex to be valid");
    static ref SAFE_URL_CHAR_RE: Regex =
        Regex::new(r"[^a-zA-Z0-9\s-]").expect("Expect regex to be valid");
}

impl<K, M> CloudObject for GenericCloudObject<K, M>
where
    K: HashableId + ToServerId + Debug + Into<String> + Clone + 'static,
    M: CloudModelType<IdType = K, CloudObjectType = GenericCloudObject<K, M>> + 'static,
{
    fn model_type_name(&self) -> &'static str {
        self.model().model_type_name()
    }

    fn uid(&self) -> ObjectUid {
        self.id.uid()
    }

    fn hashed_sqlite_id(&self) -> HashedSqliteId {
        self.id.sqlite_uid_hash(self.object_type().into())
    }

    fn sync_id(&self) -> SyncId {
        self.id
    }

    fn should_show_activity_toasts(&self) -> bool {
        self.model().should_show_activity_toasts()
    }

    fn warn_if_unsaved_at_quit(&self) -> bool {
        self.model().warn_if_unsaved_at_quit()
    }

    fn metadata(&self) -> &CloudObjectMetadata {
        &self.metadata
    }

    fn metadata_mut(&mut self) -> &mut CloudObjectMetadata {
        &mut self.metadata
    }

    fn permissions(&self) -> &CloudObjectPermissions {
        &self.permissions
    }

    fn permissions_mut(&mut self) -> &mut CloudObjectPermissions {
        &mut self.permissions
    }

    fn object_type(&self) -> ObjectType {
        self.model().object_type()
    }

    fn cloud_object_type_and_id(&self) -> CloudObjectTypeAndId {
        self.model().cloud_object_type_and_id(self.id)
    }

    fn should_clear_on_unique_key_conflict(&self) -> bool {
        self.model().should_clear_on_unique_key_conflict()
    }

    fn can_move_to_space(&self, space: Space, app: &AppContext) -> bool {
        self.model().can_move_to_space(self.space(app), space)
    }

    fn has_conflicting_changes(&self) -> bool {
        self.conflict_status.has_conflicts()
    }

    fn conflicting_object_revision(&self) -> Option<Revision> {
        match &self.conflict_status {
            ConflictStatus::ConflictingChanges { object } => Some(object.metadata.revision.clone()),
            ConflictStatus::NoConflicts => None,
        }
    }

    fn clear_conflict_status(&mut self) {
        self.conflict_status = ConflictStatus::NoConflicts;
    }

    fn replace_object_with_conflict(&mut self) {
        let mut new_conflict = ConflictStatus::NoConflicts;
        std::mem::swap(&mut new_conflict, &mut self.conflict_status);

        self.set_pending_content_changes_status(CloudObjectSyncStatus::NoLocalChanges);

        if let ConflictStatus::ConflictingChanges { object } = new_conflict {
            if self.model().should_update_after_server_conflict() {
                // Update metadata revision from the server object.
                self.metadata.update_revision_from_server(&object.metadata);
                // Update the model from the server.
                self.set_model(object.model.clone());
                // Update conflict status - this may create a new conflict if there are pending changes.
                if self.metadata.has_pending_content_changes() {
                    self.conflict_status = ConflictStatus::ConflictingChanges { object };
                } else {
                    self.conflict_status = ConflictStatus::NoConflicts;
                }
            }
        }
    }

    fn set_server_id(&mut self, server_id: ServerId) {
        self.id = SyncId::ServerId(server_id);
    }

    fn object_link(&self) -> Option<String> {
        if !self.model().supports_linking() {
            return None;
        }

        let display_name = self.model().display_name();
        // First remove all the url unsafe chars
        let name_without_unsafe_chars = SAFE_URL_CHAR_RE.replace_all(display_name.trim(), "");
        // Then turn all the spaces into dashes
        let link_safe_name = SPACE_DETECT_RE.replace_all(&name_without_unsafe_chars, "-");
        match &self.id {
            SyncId::ClientId(_) => None,
            SyncId::ServerId(id) => {
                let object_type = self.object_type();
                let object_type_for_link = if self
                    .as_any()
                    .downcast_ref::<CloudWorkflow>()
                    .is_some_and(|w| w.model().data.is_agent_mode_workflow())
                {
                    "prompt".to_string()
                } else {
                    object_type.to_string()
                };

                let mut link = format!(
                    "{}/drive/{}/{}-{}",
                    ChannelState::server_root_url(),
                    object_type_for_link,
                    link_safe_name,
                    id.uid()
                );

                // If this is a preview build, ensure the link routes to a preview build.
                if matches!(ChannelState::channel(), Channel::Preview) {
                    link.push_str("?preview=true");
                }

                Some(link)
            }
        }
    }

    fn upsert_event(&self) -> ModelEvent {
        M::upsert_event(self.upsert_params(self.object_type()))
    }

    fn display_name(&self) -> String {
        self.model().display_name()
    }

    fn versions(&self, app: &AppContext) -> Option<UpdatedObjectInput> {
        match (self.id, self.metadata.revision.as_ref()) {
            (SyncId::ServerId(id), Some(revision)) => {
                let actions_ts = ObjectActions::as_ref(app)
                    .get_latest_processed_at_ts(&self.id.uid())
                    .map(|t| t.into());
                Some(UpdatedObjectInput {
                    uid: id.into(),
                    revision_ts: revision.timestamp(),
                    metadata_ts: self.metadata.metadata_last_updated_ts,
                    permissions_ts: self.permissions.permissions_last_updated_ts,
                    actions_ts,
                })
            }
            _ => None,
        }
    }

    fn create_object_queue_item(
        &self,
        entrypoint: CloudObjectEventEntrypoint,
        initiated_by: InitiatedBy,
    ) -> Option<QueueItem> {
        self.model()
            .create_object_queue_item(self, entrypoint, initiated_by)
    }

    fn update_object_queue_item(&self, revision_ts: Option<Revision>) -> QueueItem {
        self.model().update_object_queue_item(revision_ts, self)
    }

    fn renders_in_warp_drive(&self) -> bool {
        self.model().renders_in_warp_drive()
    }

    fn to_warp_drive_item(&self, appearance: &Appearance) -> Option<Box<dyn WarpDriveItem>> {
        self.model().to_warp_drive_item(self.id, appearance, self)
    }

    fn can_export(&self) -> bool {
        self.model().can_export()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn clone_box(&self) -> Box<dyn CloudObject> {
        Box::new(self.clone())
    }
}

impl ServerObjectModel for CloudFolderModel {
    fn object_type(&self) -> ObjectType {
        <Self as CloudModelType>::object_type(self)
    }
}

impl ServerObjectModel for CloudNotebookModel {
    fn object_type(&self) -> ObjectType {
        <Self as CloudModelType>::object_type(self)
    }
}

impl ServerObjectModel for CloudWorkflowModel {
    fn object_type(&self) -> ObjectType {
        <Self as CloudModelType>::object_type(self)
    }
}
/// Extracts the server id and object type from a (caller validated) Drive link.
/// Intended use is deriving metadata from links such that Warp objects
/// can be opened natively in Warp with no web interaction.
pub fn extract_server_id_and_object_type_from_warp_drive_link(
    url: &Url,
) -> Option<OpenWarpDriveObjectArgs> {
    let server_id = url
        .path_segments()
        .and_then(|mut segments| segments.next_back())
        .and_then(|last_segment| last_segment.split('-').next_back())
        .map(|id| id.to_string());

    let object_type = url.path_segments().and_then(|mut segments| segments.nth(1));

    // Parse the object portion of the path segment (warp.dev/drive/{object})
    // into an object type
    let object_type = match object_type {
        Some("notebook") => ObjectType::Notebook,
        Some("workflow") => ObjectType::Workflow,
        _ => return None,
    };
    let query_string: HashMap<_, _> = url.query_pairs().collect();
    let focused_folder_id: Option<ServerId> = query_string
        .get("focused_folder_id")
        .and_then(|s| s.to_string().try_into().ok());

    let invitee_email: Option<String> = query_string.get("invitee_email").map(|s| s.to_string());

    Some(OpenWarpDriveObjectArgs {
        object_type,
        server_id: match server_id {
            Some(server_id) => server_id.try_into().ok()?,
            _ => return None,
        },
        settings: OpenWarpDriveObjectSettings {
            focused_folder_id,
            invitee_email,
        },
    })
}

impl<'a, K, M> From<&'a dyn CloudObject> for Option<&'a GenericCloudObject<K, M>>
where
    K: HashableId + ToServerId + Debug + Into<String> + Clone + 'static,
    M: CloudModelType<IdType = K, CloudObjectType = GenericCloudObject<K, M>> + 'static,
{
    fn from(value: &'a dyn CloudObject) -> Self {
        <GenericCloudObject<K, M> as CloudObject>::as_model_type(value)
    }
}

impl<'a, K, M> From<&'a Box<dyn CloudObject>> for Option<&'a GenericCloudObject<K, M>>
where
    K: HashableId + ToServerId + Debug + Into<String> + Clone + 'static,
    M: CloudModelType<IdType = K, CloudObjectType = GenericCloudObject<K, M>> + 'static,
{
    fn from(value: &'a Box<dyn CloudObject>) -> Self {
        <GenericCloudObject<K, M> as CloudObject>::as_model_type(value.as_ref())
    }
}

impl<'a, K, M> From<&'a mut Box<dyn CloudObject>> for Option<&'a mut GenericCloudObject<K, M>>
where
    K: HashableId + ToServerId + Debug + Into<String> + Clone + 'static,
    M: CloudModelType<IdType = K, CloudObjectType = GenericCloudObject<K, M>> + 'static,
{
    fn from(value: &'a mut Box<dyn CloudObject>) -> Self {
        <GenericCloudObject<K, M> as CloudObject>::as_model_type_mut(value.as_mut())
    }
}

impl Clone for Box<dyn CloudObject> {
    fn clone(&self) -> Self {
        self.clone_box()
    }
}

impl From<&dyn CloudObject> for ObjectType {
    fn from(value: &dyn CloudObject) -> Self {
        value.object_type()
    }
}

impl From<&Box<dyn CloudObject>> for ObjectType {
    fn from(value: &Box<dyn CloudObject>) -> Self {
        <ObjectType as From<&dyn CloudObject>>::from(value.as_ref())
    }
}

/// Extension trait for CloudObjectMetadata with methods that require AppContext.
pub trait CloudObjectMetadataExt {
    /// Returns a semantic summary of the last edit to the object. For example, "Alice edited 4 weeks ago".
    /// Returns None if the revision and last_editor are None.
    fn semantic_editing_history(&self, app: &AppContext) -> Option<String>;

    /// Returns a semantic summary of the object's creator. For example, "Alice" or "joan@warp.dev".
    #[cfg_attr(target_family = "wasm", expect(dead_code))]
    fn semantic_creator(&self, app: &AppContext) -> Option<String>;

    /// Returns semantic summary of countdown of days until permadeletion.
    /// Ex: "27 days until permanent deletion"
    fn semantic_permadeletion_countdown(&self, app: &AppContext) -> Option<String>;
}

impl CloudObjectMetadataExt for CloudObjectMetadata {
    fn semantic_editing_history(&self, app: &AppContext) -> Option<String> {
        let user_profiles = UserProfiles::as_ref(app);

        // First, the editor. For example, "Joan Didion" or "joan@warp.dev".
        let editor_string = self
            .last_editor_uid
            .as_ref()
            .and_then(|uid| user_profiles.displayable_identifier_for_uid(UserUid::new(uid)));

        // Second, the time elapsed since the edit. For example, "just now" or "3 months ago".
        let time_ago_string = self
            .revision
            .clone()
            .map(|r| format_approx_duration_from_now_utc(r.utc()));

        let full_string = match (editor_string, time_ago_string) {
            (Some(name), Some(time_ago)) if name.is_empty() => format!("Edited {time_ago}"),
            (Some(name), Some(time_ago)) => format!("{name} edited {time_ago}"),
            (None, Some(time_ago)) => format!("Edited {time_ago}"),
            (Some(name), None) => format!("Last edited by {name}"),
            _ => return None,
        };

        Some(full_string)
    }

    fn semantic_creator(&self, app: &AppContext) -> Option<String> {
        // Todo(Jack): add creation ts.
        let user_profiles = UserProfiles::as_ref(app);
        self.creator_uid
            .as_ref()
            .and_then(|uid| user_profiles.displayable_identifier_for_uid(UserUid::new(uid)))
    }

    fn semantic_permadeletion_countdown(&self, app: &AppContext) -> Option<String> {
        // 2 cases:
        // 1) Either the object is a root level object.
        // 2) Or the object is inside folder(s), call recursive function to get trashed_ts of top level folder.
        if let Some(trashed_ts) = self
            .trashed_ts
            .or_else(|| get_top_folder_trashed_ts(self.folder_id, app))
        {
            let deletion_time = trashed_ts.utc() + Duration::days(31);
            let current_time = Utc::now();
            let days_left = deletion_time.signed_duration_since(current_time).num_days();

            let full_string = match days_left {
                0 | 1 => "1 day until permanent deletion".to_string(),
                _ => format!("{days_left} days until permanent deletion"),
            };
            Some(full_string)
        } else {
            None
        }
    }
}

/// Helper function to retrieve trashed_ts of top level folder given a folder_id of an object.
fn get_top_folder_trashed_ts(
    folder_id: Option<SyncId>,
    app: &AppContext,
) -> Option<ServerTimestamp> {
    let mut folder_id = folder_id;
    let cloud_model = CloudModel::as_ref(app);
    while let Some(current_folder_id) = folder_id {
        // If the parent folder isn't in CloudModel, short-circuit so we don't loop forever.
        let folder = cloud_model.get_folder_by_uid(&current_folder_id.uid())?;

        if let Some(_parent_folder_id) = folder.metadata.folder_id {
            folder_id = folder.metadata.folder_id
        } else {
            return folder.metadata.trashed_ts;
        }
    }
    None
}

#[derive(Clone, Debug)]
pub enum ObjectPermissionUpdateResult {
    Success, // TODO: we should return the full permissions here
    Failure,
}

#[derive(Clone, Debug)]
pub struct ObjectPermissionsUpdateData {
    /// Updated permissions for the modified object.
    pub permissions: ServerPermissions,
    /// Relevant user profiles for the permissions change. This is not *all* profiles that the user
    /// should have access to.
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

/// A cloud object from the server.
#[derive(Clone, Debug)]
pub enum ServerCloudObject {
    Notebook(ServerNotebook),
    Workflow(Box<ServerWorkflow>),
    Folder(ServerFolder),
    Preference(ServerPreference),
    EnvVarCollection(ServerEnvVarCollection),
    WorkflowEnum(ServerWorkflowEnum),
    AIFact(ServerAIFact),
    MCPServer(ServerMCPServer),
    AIExecutionProfile(ServerAIExecutionProfile),
    TemplatableMCPServer(ServerTemplatableMCPServer),
    AmbientAgentEnvironment(ServerAmbientAgentEnvironment),
    ScheduledAmbientAgent(ServerScheduledAmbientAgent),
    CloudAgentConfig(ServerCloudAgentConfig),
}

impl ServerCloudObject {
    pub fn metadata(&self) -> &ServerMetadata {
        match self {
            ServerCloudObject::Notebook(notebook) => &notebook.metadata,
            ServerCloudObject::Workflow(workflow) => &workflow.metadata,
            ServerCloudObject::Folder(folder) => &folder.metadata,
            ServerCloudObject::Preference(preferences) => &preferences.metadata,
            ServerCloudObject::EnvVarCollection(env_var_collection) => &env_var_collection.metadata,
            ServerCloudObject::WorkflowEnum(workflow_enum) => &workflow_enum.metadata,
            ServerCloudObject::AIFact(aifact) => &aifact.metadata,
            ServerCloudObject::MCPServer(mcp_server) => &mcp_server.metadata,
            ServerCloudObject::TemplatableMCPServer(templatable_mcp_server) => {
                &templatable_mcp_server.metadata
            }
            ServerCloudObject::AIExecutionProfile(ai_execution_profile) => {
                &ai_execution_profile.metadata
            }
            ServerCloudObject::AmbientAgentEnvironment(ambient_agent_environment) => {
                &ambient_agent_environment.metadata
            }
            ServerCloudObject::ScheduledAmbientAgent(scheduled_ambient_agent) => {
                &scheduled_ambient_agent.metadata
            }
            ServerCloudObject::CloudAgentConfig(cloud_agent_config) => &cloud_agent_config.metadata,
        }
    }

    pub fn uid(&self) -> ObjectUid {
        match self {
            ServerCloudObject::Notebook(notebook) => notebook.id.uid(),
            ServerCloudObject::Workflow(workflow) => workflow.id.uid(),
            ServerCloudObject::Folder(folder) => folder.id.uid(),
            ServerCloudObject::Preference(preferences) => preferences.id.uid(),
            ServerCloudObject::EnvVarCollection(env_var_collection) => env_var_collection.id.uid(),
            ServerCloudObject::WorkflowEnum(workflow_enum) => workflow_enum.id.uid(),
            ServerCloudObject::AIFact(aifact) => aifact.id.uid(),
            ServerCloudObject::MCPServer(mcp_server) => mcp_server.id.uid(),
            ServerCloudObject::AIExecutionProfile(ai_execution_profile) => {
                ai_execution_profile.id.uid()
            }
            ServerCloudObject::TemplatableMCPServer(templatable_mcp_server) => {
                templatable_mcp_server.id.uid()
            }
            ServerCloudObject::AmbientAgentEnvironment(ambient_agent_environment) => {
                ambient_agent_environment.id.uid()
            }
            ServerCloudObject::ScheduledAmbientAgent(scheduled_ambient_agent) => {
                scheduled_ambient_agent.id.uid()
            }
            ServerCloudObject::CloudAgentConfig(cloud_agent_config) => cloud_agent_config.id.uid(),
        }
    }
}

impl<K, M> From<&GenericServerObject<K, M>> for ServerCloudObject
where
    K: HashableId + ToServerId + Debug + Into<String> + Clone + 'static,
    M: CloudModelType<IdType = K> + 'static,
{
    fn from(value: &GenericServerObject<K, M>) -> Self {
        let value = value as &dyn Any;
        if let Some(server_notebook) = value.downcast_ref::<ServerNotebook>() {
            ServerCloudObject::Notebook(server_notebook.clone())
        } else if let Some(server_workflow) = value.downcast_ref::<ServerWorkflow>() {
            ServerCloudObject::Workflow(Box::new(server_workflow.clone()))
        } else if let Some(server_folder) = value.downcast_ref::<ServerFolder>() {
            ServerCloudObject::Folder(server_folder.clone())
        } else if let Some(server_preferences) = value.downcast_ref::<ServerPreference>() {
            ServerCloudObject::Preference(server_preferences.clone())
        } else if let Some(server_env_var_collection) =
            value.downcast_ref::<ServerEnvVarCollection>()
        {
            ServerCloudObject::EnvVarCollection(server_env_var_collection.clone())
        } else if let Some(server_workflow_enum) = value.downcast_ref::<ServerWorkflowEnum>() {
            ServerCloudObject::WorkflowEnum(server_workflow_enum.clone())
        } else if let Some(server_aifact) = value.downcast_ref::<ServerAIFact>() {
            ServerCloudObject::AIFact(server_aifact.clone())
        } else if let Some(server_mcp_server) = value.downcast_ref::<ServerMCPServer>() {
            ServerCloudObject::MCPServer(server_mcp_server.clone())
        } else if let Some(server_ai_execution_profile) =
            value.downcast_ref::<ServerAIExecutionProfile>()
        {
            ServerCloudObject::AIExecutionProfile(server_ai_execution_profile.clone())
        } else if let Some(server_templatable_mcp_server) =
            value.downcast_ref::<ServerTemplatableMCPServer>()
        {
            ServerCloudObject::TemplatableMCPServer(server_templatable_mcp_server.clone())
        } else if let Some(server_ambient_agent_environment) =
            value.downcast_ref::<ServerAmbientAgentEnvironment>()
        {
            ServerCloudObject::AmbientAgentEnvironment(server_ambient_agent_environment.clone())
        } else if let Some(server_scheduled_ambient_agent) =
            value.downcast_ref::<ServerScheduledAmbientAgent>()
        {
            ServerCloudObject::ScheduledAmbientAgent(server_scheduled_ambient_agent.clone())
        } else if let Some(server_cloud_agent_config) =
            value.downcast_ref::<ServerCloudAgentConfig>()
        {
            ServerCloudObject::CloudAgentConfig(server_cloud_agent_config.clone())
        } else {
            panic!("Unknown server object type");
        }
    }
}

pub type ServerPreference = GenericServerObject<GenericStringObjectId, CloudPreferenceModel>;
pub type ServerFolder = GenericServerObject<FolderId, CloudFolderModel>;
pub type ServerWorkflow = GenericServerObject<WorkflowId, CloudWorkflowModel>;
pub type ServerNotebook = GenericServerObject<NotebookId, CloudNotebookModel>;
pub type ServerEnvVarCollection =
    GenericServerObject<GenericStringObjectId, CloudEnvVarCollectionModel>;
pub type ServerWorkflowEnum = GenericServerObject<GenericStringObjectId, CloudWorkflowEnumModel>;
pub type ServerAIFact = GenericServerObject<GenericStringObjectId, CloudAIFactModel>;
pub type ServerMCPServer = GenericServerObject<GenericStringObjectId, CloudMCPServerModel>;
pub type ServerAIExecutionProfile =
    GenericServerObject<GenericStringObjectId, CloudAIExecutionProfileModel>;
pub type ServerTemplatableMCPServer =
    GenericServerObject<GenericStringObjectId, CloudTemplatableMCPServerModel>;
pub type ServerAmbientAgentEnvironment =
    GenericServerObject<GenericStringObjectId, CloudAmbientAgentEnvironmentModel>;
pub type ServerScheduledAmbientAgent =
    GenericServerObject<GenericStringObjectId, CloudScheduledAmbientAgentModel>;
pub type ServerCloudAgentConfig = GenericServerObject<GenericStringObjectId, CloudAgentConfigModel>;

/// Tries to convert a GraphQL object payload into a local server object.
pub trait TryFromGql: Sized {
    type GqlType;

    fn try_from_gql(value: Self::GqlType) -> Result<Self>;
}

impl<T, S> TryFromGql for GenericServerObject<GenericStringObjectId, GenericStringModel<T, S>>
where
    T: StringModel<
        CloudObjectType = GenericCloudObject<GenericStringObjectId, GenericStringModel<T, S>>,
    >,
    S: Serializer<T>,
{
    type GqlType = warp_graphql::generic_string_object::GenericStringObject;

    fn try_from_gql(value: Self::GqlType) -> Result<Self> {
        let uid = ServerId::from_string_lossy(value.metadata.uid.inner());
        let model = GenericStringModel::<T, S>::deserialize_owned(&value.serialized_model)?;
        Ok(Self::new(
            SyncId::ServerId(uid),
            model,
            value.metadata.try_into()?,
            value.permissions.try_into()?,
        ))
    }
}

impl TryFromGql for ServerFolder {
    type GqlType = warp_graphql::folder::Folder;

    fn try_from_gql(value: Self::GqlType) -> Result<Self> {
        let uid = ServerId::from_string_lossy(value.metadata.uid.inner());
        Ok(Self::new(
            SyncId::ServerId(uid),
            CloudFolderModel::new(&value.name, value.is_warp_pack),
            value.metadata.try_into()?,
            value.permissions.try_into()?,
        ))
    }
}

impl TryFromGql for ServerNotebook {
    type GqlType = warp_graphql::notebook::Notebook;

    fn try_from_gql(value: Self::GqlType) -> Result<Self> {
        let uid = ServerId::from_string_lossy(value.metadata.uid.inner());
        let ai_document_id: Option<AIDocumentId> = value
            .ai_document_id
            .map(|id| AIDocumentId::try_from(&id[..]))
            .transpose()?;
        Ok(Self::new(
            SyncId::ServerId(uid),
            CloudNotebookModel {
                title: value.title,
                data: value.data,
                ai_document_id,
                conversation_id: None,
            },
            value.metadata.try_into()?,
            value.permissions.try_into()?,
        ))
    }
}

impl TryFromGql for ServerWorkflow {
    type GqlType = warp_graphql::workflow::Workflow;

    fn try_from_gql(value: Self::GqlType) -> Result<Self> {
        let uid = ServerId::from_string_lossy(value.metadata.uid.inner());
        let workflow = serde_json::from_str(value.data.as_str())?;
        Ok(Self::new(
            SyncId::ServerId(uid),
            CloudWorkflowModel { data: workflow },
            value.metadata.try_into()?,
            value.permissions.try_into()?,
        ))
    }
}

#[derive(Default, Clone, Copy, Debug, Eq, Derivative)]
#[derivative(PartialEq, Hash)]
pub enum Space {
    /// The current user's personal drive.
    #[default]
    Personal,
    /// A team that the current user belongs to.
    Team { team_uid: ServerId },
    /// An object shared from a drive the user is not a member of.
    Shared,
}

impl Space {
    pub fn name(&self, app: &AppContext) -> String {
        match self {
            Space::Personal => "Personal".to_string(),
            Space::Team { team_uid, .. } => {
                let user_workspaces = UserWorkspaces::as_ref(app);
                if let Some(team) = user_workspaces.team_from_uid(*team_uid) {
                    team.name.clone()
                } else {
                    "Team".to_string()
                }
            }
            Space::Shared => "Shared with me".to_string(),
        }
    }
}

/// Enum for specifying the location of a warp drive object.
/// Objects can live in top level spaces, or a specific folder.
#[derive(Eq, PartialEq, Copy, Clone, Debug, Hash)]
pub enum CloudObjectLocation {
    Space(Space),
    Folder(SyncId),
    Trash,
}

impl From<Space> for WorkflowSource {
    fn from(space: Space) -> Self {
        match space {
            Space::Personal => WorkflowSource::PersonalCloud,
            Space::Team { team_uid } => WorkflowSource::Team { team_uid },
            // TODO(ben): Model sharing in workflow telemetry.
            Space::Shared => WorkflowSource::PersonalCloud,
        }
    }
}

impl From<Owner> for WorkflowSource {
    fn from(owner: Owner) -> WorkflowSource {
        match owner {
            // TODO(ben): Represent shared objects in telemetry.
            Owner::User { .. } => Self::PersonalCloud,
            Owner::Team { team_uid } => Self::Team { team_uid },
        }
    }
}
