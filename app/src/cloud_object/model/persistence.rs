use crate::ai::execution_profiles::CloudAIExecutionProfile;
use crate::auth::AuthStateProvider;
use crate::cloud_object::{
    CloudModelType, CloudObjectLocation, CloudObjectPermissions, GenericCloudObject,
    GenericServerObject, GenericStringObjectFormat, JsonObjectType, ObjectIdType, ObjectType,
    ObjectsToUpdate, Owner, Revision, RevisionAndLastEditor, ServerCloudObject, ServerCreationInfo,
    ServerFolder, ServerMetadata, ServerNotebook, ServerPermissions, ServerWorkflow, Space,
};
use crate::drive::folders::{CloudFolder, CloudFolderModel};
use crate::drive::{
    should_auto_open_welcome_folder, write_has_auto_opened_welcome_folder_to_user_defaults,
    CloudObjectTypeAndId, DriveIndexVariant,
};
use crate::env_vars::{CloudEnvVarCollection, CloudEnvVarCollectionModel, EnvVarCollection};
use crate::notebooks::CloudNotebook;
use crate::persistence::ModelEvent;
use crate::server::ids::{ClientId, HashableId, ObjectUid, ServerId, SyncId, ToServerId};
use crate::settings::cloud_preferences::{CloudPreference, CloudPreferenceModel};
use crate::workflows::workflow::Workflow;
use crate::workflows::workflow_enum::{CloudWorkflowEnum, CloudWorkflowEnumModel, WorkflowEnum};
use crate::workflows::{CloudWorkflow, CloudWorkflowModel};
use crate::workspaces::user_workspaces::UserWorkspaces;

use itertools::Itertools;
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::SyncSender;
use warp_graphql::scalars::time::ServerTimestamp;

use warpui::{AppContext, Entity, ModelContext, SingletonEntity};

use crate::cloud_object::CloudObject;
use chrono::{DateTime, Duration, Utc};
use rand::Rng;
use warp_core::features::FeatureFlag;

use super::generic_string_model::GenericStringObjectId;

// Equivalent to 24 hours
const MIN_MINUTES_UNTIL_NEXT_FORCE_REFRESH: i64 = 1440;

// Equivalent to 36 hours
const MAX_MINUTES_UNTIL_NEXT_FORCE_REFRESH: i64 = 2160;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateSource {
    /// This cloud model change came from the server (i.e. an RTC message).
    Server,
    /// This cloud model change originated locally (i.e. from a user edit).
    Local,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CloudModelEvent {
    ObjectMoved {
        type_and_id: CloudObjectTypeAndId,
        source: UpdateSource,
        from_folder: Option<SyncId>,
        to_folder: Option<SyncId>,
    },
    ObjectUpdated {
        type_and_id: CloudObjectTypeAndId,
        source: UpdateSource,
    },
    ObjectTrashed {
        type_and_id: CloudObjectTypeAndId,
        source: UpdateSource,
    },
    ObjectUntrashed {
        type_and_id: CloudObjectTypeAndId,
        source: UpdateSource,
    },
    NotebookEditorChangedFromServer {
        notebook_id: SyncId,
    },
    ObjectCreated {
        type_and_id: CloudObjectTypeAndId,
    },
    /// An object was permanently deleted.
    ObjectDeleted {
        type_and_id: CloudObjectTypeAndId,
        /// The parent folder of this object, since it's no longer in the model.
        folder_id: Option<SyncId>,
    },
    /// An object's permissioned were changed.
    ObjectPermissionsUpdated {
        type_and_id: CloudObjectTypeAndId,
        source: UpdateSource,
    },
    /// An object identified by `id` was force expanded.
    ObjectForceExpanded {
        id: String,
    },
    /// A SyncId was converted from ClientId to ServerId after successful object creation on the server.
    ObjectSynced {
        type_and_id: CloudObjectTypeAndId,
        client_id: ClientId,
        server_id: ServerId,
    },
    /// The initial bulk load of cloud objects from the server has completed.
    InitialLoadCompleted,
}

enum FolderOpenState {
    Open,
    Closed,
    Reversed,
}

/// Persistence model for [CloudObject] information. In an ideal world, this singleton model
/// is a 1:1 mapping for what we persisting in sqlite, and on the server. Any logic beyond a basic update
/// or query to data in [CloudModel] should instead be stored in [CloudViewModel] and tested in
/// model_test.rs.
pub struct CloudModel {
    objects_by_id: HashMap<ObjectUid, Box<dyn CloudObject>>,
    model_event_sender: Option<SyncSender<ModelEvent>>,

    time_of_next_force_refresh: Option<DateTime<Utc>>,
}

impl CloudModel {
    pub fn new(
        model_event_sender: Option<SyncSender<ModelEvent>>,
        cached_objects: Vec<Box<dyn CloudObject>>,
        time_of_next_force_refresh: Option<DateTime<Utc>>,
    ) -> Self {
        let objects_by_id = cached_objects
            .into_iter()
            .map(|object| (object.uid().to_owned(), object))
            .collect::<HashMap<ObjectUid, Box<dyn CloudObject>>>();

        Self {
            objects_by_id,
            model_event_sender,
            time_of_next_force_refresh,
        }
    }

    /// This method updates the in-memory object after the CreateObject() endpoint returns successfully.
    /// It uses the existing client_id to locate the object and then it:
    /// (1) Sets the object's server_id
    /// (2) Sets the object's creation statistics (creator_uid)
    pub fn update_object_after_server_creation(
        &mut self,
        client_id: ClientId,
        server_creation_info: ServerCreationInfo,
        ctx: &mut ModelContext<Self>,
    ) {
        let creator_uid = server_creation_info.creator_uid;
        let server_id = server_creation_info.server_id_and_type.id;
        let server_permissions = server_creation_info.permissions;

        // Use server id for the object going forward.
        if let Some((_, mut object)) = self.objects_by_id.remove_entry(&client_id.to_string()) {
            object.set_server_id(server_id);
            object.metadata_mut().creator_uid = creator_uid;

            // Update permissions from server response
            let new_permissions = CloudObjectPermissions::new_from_server(server_permissions);
            *object.permissions_mut() = new_permissions;

            let type_and_id = object.cloud_object_type_and_id();
            self.objects_by_id.insert(object.uid(), object);

            // Emit CloudModelEvent for SyncId conversion
            ctx.emit(CloudModelEvent::ObjectSynced {
                type_and_id,
                client_id,
                server_id,
            });

            ctx.notify();
        }
    }

    /// Determines whether or not the given object_id can be moved to the given location, based on
    /// what we currently support from an API perspective.
    ///
    /// We do NOT support
    /// - Moving folders across spaces
    /// - Transferring from team space to personal space
    /// - Moving directly into a folder across spaces
    pub fn can_move_object_to_location(
        &self,
        hashed_id: &str,
        new_location: CloudObjectLocation,
        app: &AppContext,
    ) -> bool {
        // TODO(ben): Update as sharing+moving is supported in more cases.

        if let Some(object) = self.objects_by_id.get(hashed_id) {
            let object_space = object.space(app);
            if let CloudObjectLocation::Space(space) = new_location {
                if matches!(object_space, Space::Team { .. }) && space == Space::Personal {
                    return false;
                }

                if !object.can_move_to_space(space, app) {
                    return false;
                }
            }

            if let CloudObjectLocation::Folder(target_folder_id) = new_location {
                let folder_to_move: Option<&CloudFolder> = object.into();
                if let Some(folder_to_move) = folder_to_move {
                    // We do not want to move a folder into itself.
                    if folder_to_move.id == target_folder_id {
                        return false;
                    }

                    // Since we are trying to move a folder into a folder, we want to ensure that the
                    // target folder is not a child of the folder we are trying to move.
                    let mut target_folder_parent_folder_id = self
                        .get_folder(&target_folder_id)
                        .and_then(|folder| folder.metadata().folder_id);
                    while let Some(parent_id) = target_folder_parent_folder_id {
                        if parent_id == folder_to_move.id {
                            return false;
                        }
                        target_folder_parent_folder_id = self
                            .get_folder(&parent_id)
                            .and_then(|folder| folder.metadata().folder_id);
                    }
                }
                if let Some(target_folder) = self.get_folder(&target_folder_id) {
                    // TODO: @ianhodge We do not yet support moving directly into a folder from another space
                    if target_folder.permissions.owner != object.permissions().owner {
                        return false;
                    }
                }
            }

            return true;
        }
        false
    }

    /// Given a hashed object-id, returns the object's CloudObjectLocation
    /// (either a folder or top level space)
    pub fn object_location(
        &self,
        hashed_id: &str,
        app: &AppContext,
    ) -> Option<CloudObjectLocation> {
        self.objects_by_id
            .get(hashed_id)
            .map(|object| object.location(self, app))
    }

    pub fn set_latest_revision_and_editor(
        &mut self,
        uid: &str,
        revision_and_editor: RevisionAndLastEditor,
        ctx: &mut ModelContext<Self>,
    ) {
        if let Some(object) = self.objects_by_id.get_mut(uid) {
            object.metadata_mut().revision = Some(revision_and_editor.revision);
            object.metadata_mut().last_editor_uid = revision_and_editor.last_editor_uid;
        }
        ctx.notify();
    }

    /// Checks if the current object has a conflict and clears the conflict if that conflicts revision
    /// is behind the current revision of the object. We need this because occasionally echo'd back updates
    /// from RTC will result in a conflict, and we want to clear it once the server response is successful.
    ///
    /// This must only be called after the server *accepts* an update.
    pub fn check_and_maybe_clear_current_conflict(
        &mut self,
        uid: &str,
        ctx: &mut ModelContext<Self>,
    ) {
        if let Some(object) = self.objects_by_id.get_mut(uid) {
            if let Some(conflicting_revision) = object.conflicting_object_revision() {
                if let Some(current_revision) = object.metadata().revision.clone() {
                    // If the pending conflict is out of date compared to the current revision, clear it.
                    // If we received the RTC update for an edit before the server response, the
                    // conflict's revision may be the same as the current revision.
                    if conflicting_revision <= current_revision {
                        object.clear_conflict_status();
                    }
                }
            }
        }
        ctx.notify();
    }

    pub fn get_by_uid(&self, uid: &ObjectUid) -> Option<&dyn CloudObject> {
        self.objects_by_id.get(uid).map(|o| o.as_ref())
    }

    pub fn get_mut_by_uid(&mut self, uid: &ObjectUid) -> Option<&mut Box<dyn CloudObject>> {
        self.objects_by_id.get_mut(uid)
    }

    pub fn cloud_objects(&self) -> impl Iterator<Item = &Box<dyn CloudObject>> {
        self.objects_by_id.values()
    }

    pub fn cloud_objects_mut(&mut self) -> impl Iterator<Item = &mut Box<dyn CloudObject>> {
        self.objects_by_id.values_mut()
    }

    pub fn create_object(
        &mut self,
        id: SyncId,
        object: impl CloudObject + 'static,
        ctx: &mut ModelContext<CloudModel>,
    ) {
        ctx.emit(CloudModelEvent::ObjectCreated {
            type_and_id: object.cloud_object_type_and_id(),
        });
        self.create_object_internal(id, object);
        ctx.notify();
    }

    // Does not emit events or notify — used during initial load where
    // InitialLoadCompleted is emitted once afterward instead.
    fn create_object_internal(&mut self, id: SyncId, object: impl CloudObject + 'static) {
        self.objects_by_id.insert(id.uid(), Box::new(object));
    }

    pub fn delete_objects_by_id(
        &mut self,
        uids: Vec<ObjectUid>,
        ctx: &mut ModelContext<Self>,
    ) -> (Vec<(SyncId, ObjectIdType)>, i32) {
        let mut count = 0;
        let mut sync_ids_and_types: Vec<(SyncId, ObjectIdType)> = Vec::new();
        for uid in uids {
            if let Some(object) = self.objects_by_id.remove(&uid) {
                let cloud_object_type_and_id = object.cloud_object_type_and_id();
                sync_ids_and_types.push((
                    cloud_object_type_and_id.sync_id(),
                    cloud_object_type_and_id.object_id_type(),
                ));

                ctx.emit(CloudModelEvent::ObjectDeleted {
                    type_and_id: object.cloud_object_type_and_id(),
                    folder_id: object.metadata().folder_id,
                });
                count += 1;
            }
        }
        ctx.notify();
        (sync_ids_and_types, count)
    }

    /// Remove an object and all its descendants from `CloudModel` recursively.
    pub fn delete_object_and_descendants(
        &mut self,
        uid: ObjectUid,
        ctx: &mut ModelContext<Self>,
    ) -> Vec<(SyncId, ObjectIdType)> {
        let mut accumulator = Vec::new();
        self.delete_object_and_descendants_internal(uid, &mut accumulator, ctx);
        accumulator
    }

    fn delete_object_and_descendants_internal(
        &mut self,
        uid: ObjectUid,
        accumulator: &mut Vec<(SyncId, ObjectIdType)>,
        ctx: &mut ModelContext<Self>,
    ) {
        if let Some(object) = self.objects_by_id.remove(&uid) {
            accumulator.push((
                object.sync_id(),
                object.cloud_object_type_and_id().object_id_type(),
            ));
            ctx.emit(CloudModelEvent::ObjectDeleted {
                type_and_id: object.cloud_object_type_and_id(),
                folder_id: object.metadata().folder_id,
            });
            if object.object_type() == ObjectType::Folder {
                let contents = self
                    .objects_by_id
                    .iter()
                    .filter_map(|(child_uid, child)| {
                        if child
                            .metadata()
                            .folder_id
                            .is_some_and(|parent| parent.uid() == uid)
                        {
                            Some(child_uid.clone())
                        } else {
                            None
                        }
                    })
                    // Collect into a temporary Vec so that we can can call this mutable method
                    // recursively.
                    .collect_vec();
                for child in contents {
                    self.delete_object_and_descendants_internal(child, accumulator, ctx);
                }
            }
        }
    }

    pub fn check_if_object_is_in_cloudmodel(&mut self, uid: ObjectUid) -> bool {
        self.objects_by_id.contains_key(&uid)
    }

    /// Updates an existing object from a server response. Returns `None` if the object
    /// was found and updated, or `Some(id)` if it doesn't exist yet.
    /// Does not emit events or notify — callers are responsible for that.
    fn update_cloud_object_if_exists<K, M>(
        &mut self,
        server_object: GenericServerObject<K, M>,
    ) -> Option<SyncId>
    where
        K: HashableId + ToServerId + std::fmt::Debug + Into<String> + Clone + 'static,
        M: CloudModelType<IdType = K, CloudObjectType = GenericCloudObject<K, M>> + 'static,
    {
        let id = server_object.id;
        let boxed_option = self.get_mut_by_uid(&id.uid());
        if let Some(boxed) = boxed_option {
            let object: Option<&mut GenericCloudObject<K, M>> = boxed.into();
            if let Some(object) = object {
                object.update_from_server_object(server_object);
            } else {
                log::warn!(
                "Unable to update server object.  Expected object to implement GenericCloudObject"
            );
                debug_assert!(false, "Unable to update server object.  Failed downcast");
            }
            None
        } else {
            Some(id)
        }
    }

    /// Update the in-memory object with an update from the server. If the object has not been
    /// seen before a new object is created. Emits events and notifies.
    pub fn upsert_from_server_object<K, M>(
        &mut self,
        server_object: GenericServerObject<K, M>,
        ctx: &mut ModelContext<Self>,
    ) where
        K: HashableId + ToServerId + std::fmt::Debug + Into<String> + Clone + 'static,
        M: CloudModelType<IdType = K, CloudObjectType = GenericCloudObject<K, M>> + 'static,
    {
        let id_if_doesnt_exist = self.update_cloud_object_if_exists(server_object.clone());
        if let Some(id) = id_if_doesnt_exist {
            // If we haven't seen the object before--attempt to insert.
            self.create_object(
                id,
                GenericCloudObject::<K, M>::new_from_server(server_object),
                ctx,
            );
        } else {
            // Object existed and was updated — emit ObjectUpdated if no conflict.
            let uid = server_object.id.uid();
            if let Some(object) = self.get_by_uid(&uid) {
                if !object.has_conflicting_changes() {
                    ctx.emit(CloudModelEvent::ObjectUpdated {
                        type_and_id: object.cloud_object_type_and_id(),
                        source: UpdateSource::Server,
                    });
                }
            }
        }
        ctx.notify();
    }

    // Does not emit events or notify — used during initial load where
    // InitialLoadCompleted is emitted once afterward instead.
    fn upsert_from_server_object_internal<K, M>(&mut self, server_object: GenericServerObject<K, M>)
    where
        K: HashableId + ToServerId + std::fmt::Debug + Into<String> + Clone + 'static,
        M: CloudModelType<IdType = K, CloudObjectType = GenericCloudObject<K, M>> + 'static,
    {
        let id_if_doesnt_exist = self.update_cloud_object_if_exists(server_object.clone());
        if let Some(id) = id_if_doesnt_exist {
            self.create_object_internal(
                id,
                GenericCloudObject::<K, M>::new_from_server(server_object),
            );
        }
    }

    /// Update the in-memory notebook with an update from the server. If the object has not been
    /// seen before--a new object is created.
    pub fn upsert_from_server_notebook(
        &mut self,
        server_notebook: ServerNotebook,
        ctx: &mut ModelContext<Self>,
    ) {
        self.upsert_from_server_object(server_notebook, ctx);
    }

    /// Upsert either inserts a new cloud object or updates an existing one. When updating an object,
    /// this overwrites all fields that are protected by the revision. For fields protected by the metadata ts,
    /// see update_object_metadata().
    pub fn upsert_from_server_cloud_object(
        &mut self,
        server_cloud_object: ServerCloudObject,
        ctx: &mut ModelContext<Self>,
    ) {
        match server_cloud_object {
            ServerCloudObject::Notebook(notebook) => {
                self.upsert_from_server_notebook(notebook, ctx);
            }
            ServerCloudObject::Workflow(workflow) => {
                self.upsert_from_server_workflow(*workflow, ctx);
            }
            ServerCloudObject::Folder(folder) => {
                self.upsert_from_server_folder(folder, ctx);
            }
            ServerCloudObject::Preference(preferences) => {
                self.upsert_from_server_object(preferences, ctx);
            }
            ServerCloudObject::EnvVarCollection(env_var_collection) => {
                self.upsert_from_server_object(env_var_collection, ctx);
            }
            ServerCloudObject::WorkflowEnum(workflow_enum) => {
                self.upsert_from_server_object(workflow_enum, ctx);
            }
            ServerCloudObject::AIFact(aifact) => {
                self.upsert_from_server_object(aifact, ctx);
            }
            ServerCloudObject::MCPServer(mcp_server) => {
                self.upsert_from_server_object(mcp_server, ctx);
            }
            ServerCloudObject::AIExecutionProfile(ai_execution_profile) => {
                self.upsert_from_server_object(ai_execution_profile, ctx);
            }
            ServerCloudObject::TemplatableMCPServer(templatable_mcp_server) => {
                self.upsert_from_server_object(templatable_mcp_server, ctx);
            }
            ServerCloudObject::AmbientAgentEnvironment(ambient_agent_environment) => {
                self.upsert_from_server_object(ambient_agent_environment, ctx);
            }
            ServerCloudObject::ScheduledAmbientAgent(scheduled_ambient_agent) => {
                self.upsert_from_server_object(scheduled_ambient_agent, ctx);
            }
            ServerCloudObject::CloudAgentConfig(cloud_agent_config) => {
                self.upsert_from_server_object(cloud_agent_config, ctx);
            }
        }
    }

    /// Updates the in-memory folder with an update from the server. If the object has not been
    /// seen before--a new object is created.
    pub fn upsert_from_server_folder(
        &mut self,
        mut server_folder: ServerFolder,
        ctx: &mut ModelContext<Self>,
    ) {
        if let Some(folder) = self.get_folder(&server_folder.id) {
            server_folder.model.is_open = folder.model.is_open;
        }

        self.upsert_from_server_object(server_folder, ctx);
    }

    /// Updates the in-memory workflow with an update from the server. If the object has not been
    /// seen before--a new object is created.
    pub fn upsert_from_server_workflow(
        &mut self,
        server_workflow: ServerWorkflow,
        ctx: &mut ModelContext<Self>,
    ) {
        self.upsert_from_server_object(server_workflow, ctx);
    }

    /// Overwrites the trashed_ts, current_editor, etc of the object only if the new_metadata timestamp
    /// is greater than the one currently in memory. Also check to see if any important
    /// changes have occurred that should trigger a model event, like a notebook editor changing.
    /// Returns true if the update was applied and false if it was not.
    pub fn maybe_update_object_metadata(
        &mut self,
        uid: &ObjectUid,
        new_metadata: ServerMetadata,
        force_refresh: bool,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        let update_applied = self.maybe_update_object_metadata_internal(
            uid,
            new_metadata,
            force_refresh,
            true, /* emit_events */
            ctx,
        );
        if update_applied {
            ctx.notify();
        }
        update_applied
    }

    /// Internal logic of above function, without using a ctx.notify call.
    /// When `emit_events` is false (during initial load), per-object events are suppressed.
    pub fn maybe_update_object_metadata_internal(
        &mut self,
        uid: &ObjectUid,
        new_metadata: ServerMetadata,
        force_refresh: bool,
        emit_events: bool,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        if let Some(object) = self.objects_by_id.get_mut(uid) {
            if let Some(current_ts) = object.metadata().metadata_last_updated_ts {
                // Only perform the update if the new timestamp is greater than the current one.
                if new_metadata.metadata_last_updated_ts > current_ts
                    || (force_refresh && new_metadata.metadata_last_updated_ts == current_ts)
                {
                    let old_editor = object.metadata().current_editor_uid.clone();
                    let old_folder_id = object.metadata().folder_id;
                    let old_trashed_ts = object.metadata().trashed_ts;

                    object
                        .metadata_mut()
                        .update_from_new_metadata_ts(new_metadata.clone());

                    // Since we're overwriting the metadata, it should not be marked as pending anymore.
                    // This is also important to do so that the sqlite upsert doesn't skip certain metadata fields.
                    object
                        .metadata_mut()
                        .pending_changes_statuses
                        .has_pending_metadata_change = false;

                    if emit_events {
                        let new_editor = object.metadata().current_editor_uid.clone();
                        let new_folder_id = object.metadata().folder_id;
                        let new_trashed_ts = object.metadata().trashed_ts;
                        // Some metadata updates should emit custom events.
                        // For example, changes to current editor of a notebook or parent folder of an object
                        let notebook: Option<&mut CloudNotebook> = object.into();
                        if let Some(notebook) = notebook {
                            if new_editor != old_editor {
                                ctx.emit(CloudModelEvent::NotebookEditorChangedFromServer {
                                    notebook_id: notebook.id,
                                });
                            }
                        }
                        if new_folder_id != old_folder_id {
                            ctx.emit(CloudModelEvent::ObjectMoved {
                                type_and_id: object.cloud_object_type_and_id(),
                                source: UpdateSource::Server,
                                from_folder: old_folder_id,
                                to_folder: new_folder_id,
                            })
                        }

                        match (old_trashed_ts, new_trashed_ts) {
                            (None, Some(_)) => ctx.emit(CloudModelEvent::ObjectTrashed {
                                type_and_id: object.cloud_object_type_and_id(),
                                source: UpdateSource::Server,
                            }),
                            (Some(_), None) => ctx.emit(CloudModelEvent::ObjectUntrashed {
                                type_and_id: object.cloud_object_type_and_id(),
                                source: UpdateSource::Server,
                            }),
                            _ => (),
                        }
                    }

                    return true;
                } else {
                    log::debug!("in memory metadata ts is greater or equal to metadata ts from update, ignoring");
                }
            }
        } else {
            log::info!("object does not exist in-memory, ignoring");
        }
        false
    }

    /// Update an object's location (folder and owner). This is an implementation detail of
    /// `UpdateManager` to keep local state in sync with optimistic moves. It does not validate
    /// that the move is valid and MUST not be used elsewhere.
    ///
    /// If `new_space` is `None`, the space is unchanged.
    pub fn update_object_location(
        &mut self,
        uid: &ObjectUid,
        new_owner: Option<Owner>,
        new_folder: Option<SyncId>,
        ctx: &mut ModelContext<Self>,
    ) {
        // TODO(ben): This should use a container instead of a folder+owner pair.

        if let Some(object) = self.get_mut_by_uid(uid) {
            let old_folder = object.metadata().folder_id;
            let mut changed = false;

            if let Some(new_owner) = new_owner {
                if new_owner != object.permissions().owner {
                    object.permissions_mut().owner = new_owner;
                    changed = true;
                }
            }

            if new_folder != old_folder {
                object.metadata_mut().folder_id = new_folder;
                changed = true;
            }

            if changed {
                ctx.emit(CloudModelEvent::ObjectMoved {
                    type_and_id: object.cloud_object_type_and_id(),
                    source: UpdateSource::Local,
                    from_folder: old_folder,
                    to_folder: new_folder,
                });
                ctx.notify();
            }
        }
    }

    /// Overwrites the space and permissions last updated at ts of the object.
    pub fn update_object_permissions(
        &mut self,
        uid: &ObjectUid,
        new_permissions: ServerPermissions,
        source: UpdateSource,
        ctx: &mut ModelContext<Self>,
    ) {
        self.update_object_permissions_internal(uid, new_permissions);
        if let Some(object) = self.get_by_uid(uid) {
            ctx.notify();
            ctx.emit(CloudModelEvent::ObjectPermissionsUpdated {
                type_and_id: object.cloud_object_type_and_id(),
                source,
            });
        }
    }

    // Moving this to a separate function so this can be called without ctx.notify()
    // to reduce the amount of notify's made during our app initialization
    pub fn update_object_permissions_internal(
        &mut self,
        uid: &ObjectUid,
        new_permissions: ServerPermissions,
    ) {
        if let Some(object) = self.objects_by_id.get_mut(uid) {
            // Since we're overwriting the permissions, they should not be marked as pending anymore.
            // This is also important to do so that the sqlite upsert doesn't skip updating the permissions.
            object
                .metadata_mut()
                .pending_changes_statuses
                .has_pending_permissions_change = false;

            object
                .permissions_mut()
                .update_from_new_permissions_ts(new_permissions);
        }
    }

    pub fn update_notebook_current_editor(
        &mut self,
        notebook_id: SyncId,
        new_editor_uid: Option<String>,
        ctx: &mut ModelContext<Self>,
    ) {
        if let Some(notebook) = self.get_notebook_mut(&notebook_id) {
            notebook.metadata.set_current_editor(new_editor_uid.clone());
            ctx.emit(CloudModelEvent::NotebookEditorChangedFromServer { notebook_id });
            ctx.notify();
        }
    }

    /// Updates the per-environment "last used" timestamp.
    ///
    /// This timestamp is derived from `CloudEnvironment.lastTaskCreated.createdAt`.
    pub fn update_environment_last_task_run_timestamps(
        &mut self,
        timestamps: HashMap<String, DateTime<Utc>>,
        ctx: &mut ModelContext<Self>,
    ) {
        for (uid, timestamp) in timestamps {
            if let Some(object) = self.objects_by_id.get_mut(&uid) {
                object.metadata_mut().last_task_run_ts = Some(timestamp.into());
            }
        }
        ctx.notify();
    }

    pub fn update_object_metadata_last_updated_ts(
        &mut self,
        uid: &ObjectUid,
        new_ts: ServerTimestamp,
        ctx: &mut ModelContext<Self>,
    ) {
        if let Some(object) = self.objects_by_id.get_mut(uid) {
            object.metadata_mut().metadata_last_updated_ts = Some(new_ts);

            if let Some(model_event_sender) = &self.model_event_sender {
                if let Err(e) = model_event_sender.send(ModelEvent::UpdateObjectMetadata {
                    id: object.hashed_sqlite_id(),
                    metadata: object.metadata().clone(),
                }) {
                    log::error!("Error saving to cache: {e:?}");
                }
            }
            ctx.notify();
        }
    }

    /// Update an object in the cloud model as part of a local user edit. This should not be used
    /// for updates received from the server.
    pub fn update_object_from_edit<K, M>(
        &mut self,
        model: M,
        object_id: SyncId,
        ctx: &mut ModelContext<Self>,
    ) where
        K: HashableId
            + ToServerId
            + std::fmt::Debug
            + Into<String>
            + Clone
            + Copy
            + Send
            + Sync
            + 'static,
        M: CloudModelType<IdType = K, CloudObjectType = GenericCloudObject<K, M>> + 'static,
    {
        if let Some(cloud_object) = self.get_object_of_type_mut(&object_id) {
            cloud_object.set_model(model);
            ctx.emit(CloudModelEvent::ObjectUpdated {
                type_and_id: cloud_object.cloud_object_type_and_id(),
                source: UpdateSource::Local,
            });
            ctx.notify();
        }
    }

    /// Overwrite a workflow's definition. For example, if a workflow is in conflict with the
    /// server, we'll replace the local state with the server's version.
    pub fn overwrite_workflow(
        &mut self,
        workflow: Workflow,
        workflow_id: SyncId,
        ctx: &mut ModelContext<Self>,
    ) {
        if let Some(cloud_workflow) = self.get_workflow_mut(&workflow_id) {
            cloud_workflow.set_model(CloudWorkflowModel::new(workflow));
            ctx.emit(CloudModelEvent::ObjectUpdated {
                type_and_id: cloud_workflow.cloud_object_type_and_id(),
                source: UpdateSource::Server,
            });
            ctx.notify();
        }
    }

    pub fn overwrite_env_var_collection(
        &mut self,
        env_var_collection: EnvVarCollection,
        env_var_collection_id: SyncId,
        ctx: &mut ModelContext<Self>,
    ) {
        if let Some(cloud_env_var_collection) = self
            .get_object_of_type_mut::<GenericStringObjectId, CloudEnvVarCollectionModel>(
                &env_var_collection_id,
            )
        {
            cloud_env_var_collection.set_model(CloudEnvVarCollectionModel::new(env_var_collection));
            ctx.emit(CloudModelEvent::ObjectUpdated {
                type_and_id: cloud_env_var_collection.cloud_object_type_and_id(),
                source: UpdateSource::Server,
            });
            ctx.notify();
        }
    }

    pub fn overwrite_workflow_enum(
        &mut self,
        workflow_enum: WorkflowEnum,
        workflow_enum_id: SyncId,
        ctx: &mut ModelContext<Self>,
    ) {
        if let Some(cloud_workflow_enum) = self
            .get_object_of_type_mut::<GenericStringObjectId, CloudWorkflowEnumModel>(
                &workflow_enum_id,
            )
        {
            cloud_workflow_enum.set_model(CloudWorkflowEnumModel::new(workflow_enum));
            ctx.emit(CloudModelEvent::ObjectUpdated {
                type_and_id: cloud_workflow_enum.cloud_object_type_and_id(),
                source: UpdateSource::Server,
            });
            ctx.notify();
        }
    }

    fn set_folder_open_state(
        &mut self,
        folder_id: SyncId,
        open_state: FolderOpenState,
        ctx: &mut ModelContext<Self>,
    ) {
        if let Some(folder) = self.get_folder_mut(&folder_id) {
            let is_open = match open_state {
                FolderOpenState::Open => true,
                FolderOpenState::Closed => false,
                FolderOpenState::Reversed => !folder.model.is_open,
            };

            folder.set_model(CloudFolderModel {
                is_open,
                is_warp_pack: folder.model.is_warp_pack,
                name: folder.model.name.clone(),
            });

            let folder_clone = folder.clone();
            if let Some(model_event_sender) = &self.model_event_sender {
                if let Err(e) = model_event_sender.send(folder_clone.upsert_event()) {
                    log::error!("Error persisting folder: {e:?}");
                }
            }

            ctx.notify();
        }
    }

    pub fn open_folder(&mut self, folder_id: SyncId, ctx: &mut ModelContext<Self>) {
        self.set_folder_open_state(folder_id, FolderOpenState::Open, ctx)
    }

    pub fn close_folder(&mut self, folder_id: SyncId, ctx: &mut ModelContext<Self>) {
        self.set_folder_open_state(folder_id, FolderOpenState::Closed, ctx)
    }

    pub fn toggle_folder_open(&mut self, folder_id: SyncId, ctx: &mut ModelContext<Self>) {
        self.set_folder_open_state(folder_id, FolderOpenState::Reversed, ctx)
    }

    /// Collapses all folders for a given location, including the folder provided
    /// (if location is a CloudObjectLocation::Folder).
    pub fn collapse_all_in_location(
        &mut self,
        location: CloudObjectLocation,
        index_variant: DriveIndexVariant,
        ctx: &mut ModelContext<Self>,
    ) {
        let mut folder_ids: Vec<SyncId> = Vec::new();
        self.collapse_all_in_location_helper(location, index_variant, &mut folder_ids, ctx);

        folder_ids.iter().for_each(|folder_id| {
            self.set_folder_open_state(*folder_id, FolderOpenState::Closed, ctx)
        });

        ctx.notify();
    }

    /// Helper function for collapse_all_in_location. Recursively traverses through descendents,
    /// adding IDs of any folders found to the folder_ids mutable vector reference.
    fn collapse_all_in_location_helper(
        &self,
        location: CloudObjectLocation,
        index_variant: DriveIndexVariant,
        folder_ids: &mut Vec<SyncId>,
        app: &AppContext,
    ) {
        if let CloudObjectLocation::Folder(folder_id) = location {
            folder_ids.push(folder_id);
        }

        match index_variant {
            DriveIndexVariant::MainIndex => self
                .active_cloud_objects_in_location_without_descendents(location, app)
                .for_each(|object| {
                    let folder: Option<&CloudFolder> = object.into();
                    if let Some(folder) = folder {
                        self.collapse_all_in_location_helper(
                            CloudObjectLocation::Folder(folder.id),
                            index_variant,
                            folder_ids,
                            app,
                        );
                    }
                }),
            DriveIndexVariant::Trash => {
                if let CloudObjectLocation::Space(space) = location {
                    self.directly_trashed_cloud_objects_in_space(space, app)
                        .for_each(|object| {
                            let folder: Option<&CloudFolder> = object.into();
                            if let Some(folder) = folder {
                                self.collapse_all_in_location_helper(
                                    CloudObjectLocation::Folder(folder.id),
                                    index_variant,
                                    folder_ids,
                                    app,
                                );
                            }
                        })
                } else {
                    self.indirectly_trashed_cloud_objects_in_location_without_descendents(
                        location, app,
                    )
                    .for_each(|object| {
                        let folder: Option<&CloudFolder> = object.into();
                        if let Some(folder) = folder {
                            self.collapse_all_in_location_helper(
                                CloudObjectLocation::Folder(folder.id),
                                index_variant,
                                folder_ids,
                                app,
                            );
                        }
                    })
                }
            }
        }
    }

    /// Force expands the object identified by `hash_id` and any of its ancestors. If an object is
    /// identified by `id`, [`CloudModelEvent::ObjectForceExpanded`] is emitted.
    pub fn force_expand_object_and_ancestors(&mut self, id: SyncId, ctx: &mut ModelContext<Self>) {
        let hashed_id = &id.uid();
        if !self.objects_by_id.contains_key(hashed_id) {
            return;
        }

        self.force_expand_object_and_ancestors_internal(id, ctx);
        ctx.emit(CloudModelEvent::ObjectForceExpanded {
            id: hashed_id.clone(),
        });
    }

    fn force_expand_object_and_ancestors_internal(
        &mut self,
        id: SyncId,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(object) = self.objects_by_id.get(&id.uid()) else {
            return;
        };

        let parent_folder_id = object.metadata().folder_id;
        let folder: Option<&CloudFolder> = object.into();

        if let Some(folder) = folder {
            self.set_folder_open_state(folder.id, FolderOpenState::Open, ctx);
        }

        if let Some(parent_folder_id) = parent_folder_id {
            self.force_expand_object_and_ancestors_internal(parent_folder_id, ctx);
        }
    }

    /// Force expands object and its ancestors when given a CloudObjectTypeAndId input
    pub fn force_expand_object_and_ancestors_cloud_id(
        &mut self,
        id: CloudObjectTypeAndId,
        ctx: &mut ModelContext<Self>,
    ) {
        match id {
            CloudObjectTypeAndId::Notebook(sync_id) => {
                self.force_expand_object_and_ancestors(sync_id, ctx)
            }
            CloudObjectTypeAndId::Workflow(sync_id) => {
                self.force_expand_object_and_ancestors(sync_id, ctx)
            }
            CloudObjectTypeAndId::Folder(sync_id) => {
                self.force_expand_object_and_ancestors(sync_id, ctx)
            }
            CloudObjectTypeAndId::GenericStringObject { object_type, id } => {
                if let GenericStringObjectFormat::Json(JsonObjectType::EnvVarCollection) =
                    object_type
                {
                    self.force_expand_object_and_ancestors(id, ctx)
                } else {
                    log::error!("Attempted to force expand an unsupported GenericStringObject type")
                }
            }
        }
    }

    pub fn delete_object(&mut self, id: SyncId, ctx: &mut ModelContext<Self>) {
        // TODO: for now we are simply hard deleting the object from memory. When
        // we have conflict resolution. We should only mark the object as deleted
        // without deleting the content until the server returns successful response.
        if let Some(object) = self.objects_by_id.remove(&id.uid()) {
            ctx.emit(CloudModelEvent::ObjectDeleted {
                type_and_id: object.cloud_object_type_and_id(),
                folder_id: object.metadata().folder_id,
            });
        }
        ctx.notify();
    }

    /// Number of cloud objects that have not synced to the cloud
    pub fn num_unsaved_objects(&self) -> usize {
        self.objects_by_id
            .values()
            .filter(|object| object.metadata().has_pending_content_changes())
            .count()
    }

    /// Number of cloud objects that have not synced to the cloud and require a user warning before quitting
    pub fn num_unsaved_objects_to_warn_about_before_quitting(&self) -> usize {
        self.objects_by_id
            .values()
            .filter(|object| {
                object.warn_if_unsaved_at_quit() && object.metadata().has_pending_content_changes()
            })
            .count()
    }

    /// Number of cloud objects that have errored in some way and are visible in the Warp Drive index
    pub fn num_visible_errored_objects(&self) -> usize {
        self.objects_by_id
            .values()
            .filter(|object| object.renders_in_warp_drive() && object.metadata().is_errored())
            .count()
    }

    pub fn has_objects(&self) -> bool {
        !self.objects_by_id.is_empty()
    }

    pub fn has_non_welcome_objects(&self) -> bool {
        self.objects_by_id
            .iter()
            .any(|(_, object)| !object.metadata().is_welcome_object)
    }

    /// Whether or not there are any objects directly shared with the user.
    ///
    /// This takes a reference to [`UserWorkspaces`] to prevent circular model references.
    pub fn has_directly_shared_objects(
        &self,
        user_workspaces: &UserWorkspaces,
        app: &AppContext,
    ) -> bool {
        let user_uid = AuthStateProvider::as_ref(app).get().user_id();
        self.objects_by_id.values().any(|object| {
            // We can't use CloudObject::is_in_space, because that reborrows UserWorkspaces.
            user_workspaces.owner_to_space(object.permissions().owner, app) == Space::Shared
                && user_uid.is_some_and(|uid| object.permissions().has_direct_user_access(uid))
        })
    }

    pub fn get_folder_by_uid(&self, uid: &str) -> Option<&CloudFolder> {
        self.objects_by_id.get(uid).and_then(|object| object.into())
    }

    pub fn get_folder(&self, folder_id: &SyncId) -> Option<&CloudFolder> {
        self.objects_by_id
            .get(&folder_id.uid())
            .and_then(|object| object.into())
    }

    pub fn get_folder_mut(&mut self, folder_id: &SyncId) -> Option<&mut CloudFolder> {
        self.objects_by_id
            .get_mut(&folder_id.uid())
            .and_then(|object| object.into())
    }

    pub fn get_all_exportable_object_ids(&self) -> Vec<CloudObjectTypeAndId> {
        self.objects_by_id
            .values()
            .filter(|object| object.can_export())
            .map(|object| object.cloud_object_type_and_id())
            .collect()
    }

    #[allow(unused)]
    /// Returns only active (not trashed) folders in cloud model.
    pub fn get_all_active_folders(&self) -> impl Iterator<Item = &CloudFolder> {
        self.objects_by_id
            .values()
            .filter(|object| !object.is_trashed(self))
            .filter_map(|object| object.into())
    }

    /// Returns all folders (trashed or not) in cloud model.
    pub fn get_all_active_and_inactive_folders(&self) -> impl Iterator<Item = &CloudFolder> {
        self.objects_by_id
            .values()
            .filter_map(|object| object.into())
    }

    pub fn get_workflow(&self, workflow_id: &SyncId) -> Option<&CloudWorkflow> {
        self.objects_by_id
            .get(&workflow_id.uid())
            .and_then(|object| object.into())
    }

    pub fn get_workflow_by_uid(&self, uid: &str) -> Option<&CloudWorkflow> {
        self.objects_by_id.get(uid).and_then(|object| object.into())
    }

    pub fn get_workflow_enum(&self, enum_id: &SyncId) -> Option<&CloudWorkflowEnum> {
        self.objects_by_id
            .get(&enum_id.uid())
            .and_then(|object| object.into())
    }

    pub fn get_ai_execution_profile(
        &self,
        profile_id: &SyncId,
    ) -> Option<&CloudAIExecutionProfile> {
        self.objects_by_id
            .get(&profile_id.uid())
            .and_then(|object| object.into())
    }

    pub fn get_workflow_enum_mut(&mut self, enum_id: &SyncId) -> Option<&mut CloudWorkflowEnum> {
        self.objects_by_id
            .get_mut(&enum_id.uid())
            .and_then(|object| object.into())
    }

    pub fn get_workflow_mut(&mut self, workflow_id: &SyncId) -> Option<&mut CloudWorkflow> {
        self.objects_by_id
            .get_mut(&workflow_id.uid())
            .and_then(|object| object.into())
    }

    /// Returns only active (not trashed) workflows in cloud model.
    pub fn get_all_active_workflows(&self) -> impl Iterator<Item = &CloudWorkflow> {
        self.objects_by_id
            .values()
            .filter(|object| !object.is_trashed(self))
            .filter_map(|object| object.into())
    }

    /// Returns all workflows (trashed or not) in cloud model.
    pub fn get_all_active_and_inactive_workflows(&self) -> impl Iterator<Item = &CloudWorkflow> {
        self.objects_by_id
            .values()
            .filter_map(|object| object.into())
    }

    /// Returns all workflows (trashed or not) in cloud model.
    pub fn get_all_active_and_inactive_workflows_mut(
        &mut self,
    ) -> impl Iterator<Item = &mut CloudWorkflow> {
        self.objects_by_id
            .values_mut()
            .filter_map(|object| object.into())
    }

    /// Returns all active (not trashed) workflows in the space.
    pub fn active_workflows_in_space<'a>(
        &'a self,
        space: Space,
        app: &'a AppContext,
    ) -> impl Iterator<Item = &'a CloudWorkflow> + 'a {
        self.active_cloud_objects_in_space(space, app)
            .filter_map(|object| object.into())
    }

    /// Returns all active (not trashed) and non-welcome workflows (ie. non starter workflows) in the space.
    pub fn active_non_welcome_workflows_in_space<'a>(
        &'a self,
        space: Space,
        app: &'a AppContext,
    ) -> impl Iterator<Item = &'a CloudWorkflow> + 'a {
        self.active_non_welcome_cloud_objects_in_space(space, app)
            .filter_map(|object| object.into())
    }

    /// Returns all active (not trashed) notebooks in the space.
    pub fn active_notebooks_in_space<'a>(
        &'a self,
        space: Space,
        app: &'a AppContext,
    ) -> impl Iterator<Item = &'a CloudNotebook> + 'a {
        self.active_cloud_objects_in_space(space, app)
            .filter_map(|object| object.into())
    }

    /// Returns all active (not trashed) and non-welcome notebooks (ie. non starter notebooks) in the space.
    pub fn active_non_welcome_notebooks_in_space<'a>(
        &'a self,
        space: Space,
        app: &'a AppContext,
    ) -> impl Iterator<Item = &'a CloudNotebook> + 'a {
        self.active_non_welcome_cloud_objects_in_space(space, app)
            .filter_map(|object| object.into())
    }

    /// Returns all active (not trashed) and non-welcome env var collections in the space.
    pub fn active_non_welcome_env_var_collections_in_space<'a>(
        &'a self,
        space: Space,
        app: &'a AppContext,
    ) -> impl Iterator<Item = &'a CloudEnvVarCollection> + 'a {
        self.active_non_welcome_cloud_objects_in_space(space, app)
            .filter_map(|object| object.into())
    }

    /// Returns all workflow enums with a given owner.
    pub fn workflow_enums_with_owner<'a>(
        &'a self,
        owner: Owner,
        _: &'a AppContext,
    ) -> impl Iterator<Item = &'a CloudWorkflowEnum> + 'a {
        self.objects_by_id
            .values()
            .filter(move |object| !object.is_trashed(self) && object.permissions().owner == owner)
            .filter_map(|object| object.into())
    }

    /// Returns a map of CloudPreference models keyed by their storage key.
    pub fn get_all_cloud_preferences_by_storage_key(&self) -> HashMap<String, &CloudPreference> {
        let mut keys: HashSet<String> = HashSet::new();
        self.get_all_objects_of_type::<GenericStringObjectId, CloudPreferenceModel>()
            .map(|cloud_prefs| {
                if keys.contains(&cloud_prefs.model().string_model.storage_key) {
                    log::warn!(
                        "Duplicate cloud preference storage key: {}",
                        cloud_prefs.model().string_model.storage_key
                    );
                    keys.insert(cloud_prefs.model().string_model.storage_key.clone());
                }
                (
                    cloud_prefs.model().string_model.storage_key.clone(),
                    cloud_prefs,
                )
            })
            .collect::<HashMap<_, _>>()
    }

    pub fn get_object_of_type<K, M>(&self, object_id: &SyncId) -> Option<&GenericCloudObject<K, M>>
    where
        K: HashableId + ToServerId + std::fmt::Debug + Into<String> + Clone + 'static,
        M: CloudModelType<IdType = K, CloudObjectType = GenericCloudObject<K, M>> + 'static,
    {
        self.objects_by_id
            .get(&object_id.uid())
            .and_then(|object| object.into())
    }

    pub fn get_object_of_type_mut<K, M>(
        &mut self,
        object_id: &SyncId,
    ) -> Option<&mut GenericCloudObject<K, M>>
    where
        K: HashableId + ToServerId + std::fmt::Debug + Into<String> + Clone + 'static,
        M: CloudModelType<IdType = K, CloudObjectType = GenericCloudObject<K, M>> + 'static,
    {
        self.objects_by_id
            .get_mut(&object_id.uid())
            .and_then(|object| object.into())
    }

    pub fn get_all_objects_of_type<K, M>(&self) -> impl Iterator<Item = &GenericCloudObject<K, M>>
    where
        K: HashableId + ToServerId + std::fmt::Debug + Into<String> + Clone + 'static,
        M: CloudModelType<IdType = K, CloudObjectType = GenericCloudObject<K, M>> + 'static,
    {
        self.objects_by_id
            .values()
            .filter_map(|object| object.into())
    }

    /// Returns all objects the model knows about that should potentially be
    /// updated by the server.
    pub fn get_versions_for_all_objects(&self, app: &AppContext) -> ObjectsToUpdate {
        let mut objects_to_update = ObjectsToUpdate::default();
        for (versions, object_type) in self
            .objects_by_id
            .values()
            .filter_map(|object| object.versions(app).zip(Some(object.object_type())))
        {
            match object_type {
                ObjectType::Notebook => objects_to_update.notebooks.push(versions),
                ObjectType::Workflow => objects_to_update.workflows.push(versions),
                ObjectType::Folder => objects_to_update.folders.push(versions),
                ObjectType::GenericStringObject(_) => {
                    objects_to_update.generic_string_objects.push(versions)
                }
            }
        }
        objects_to_update
    }

    pub fn get_notebook(&self, notebook_id: &SyncId) -> Option<&CloudNotebook> {
        self.objects_by_id
            .get(&notebook_id.uid())
            .and_then(|object| object.into())
    }

    pub fn get_notebook_by_uid(&self, uid: &str) -> Option<&CloudNotebook> {
        self.objects_by_id.get(uid).and_then(|object| object.into())
    }

    pub fn get_notebook_mut(&mut self, notebook_id: &SyncId) -> Option<&mut CloudNotebook> {
        self.objects_by_id
            .get_mut(&notebook_id.uid())
            .and_then(|notebook| notebook.into())
    }

    pub fn get_env_var_collection(
        &self,
        env_var_collection_id: &SyncId,
    ) -> Option<&CloudEnvVarCollection> {
        self.objects_by_id
            .get(&env_var_collection_id.uid())
            .and_then(|object| object.into())
    }

    pub fn get_env_var_collection_by_uid(&self, uid: &str) -> Option<&CloudEnvVarCollection> {
        self.objects_by_id.get(uid).and_then(|object| object.into())
    }

    /// Returns only active (not trashed) EVCs in cloud model.
    pub fn get_all_active_env_var_collections(
        &self,
    ) -> impl Iterator<Item = &CloudEnvVarCollection> {
        self.objects_by_id
            .values()
            .filter(|object| !object.is_trashed(self))
            .filter_map(|object| object.into())
    }

    pub fn current_revision(&self, id: &SyncId) -> Option<&Revision> {
        self.objects_by_id
            .get(&id.uid())
            .and_then(|warp_cloud_object| warp_cloud_object.metadata().revision.as_ref())
    }

    /// Returns only active (not trashed) notebooks in cloud model.
    pub fn get_all_active_notebooks(&self) -> impl Iterator<Item = &CloudNotebook> {
        self.objects_by_id
            .values()
            .filter(|object| !object.is_trashed(self))
            .filter_map(|object| object.into())
    }

    /// Returns all notebooks (trashed or not) in cloud model.
    pub fn get_all_active_and_inactive_notebooks(&self) -> impl Iterator<Item = &CloudNotebook> {
        self.objects_by_id
            .values()
            .filter_map(|object| object.into())
    }

    #[cfg(test)]
    pub fn as_cloud_objects(&self) -> impl Iterator<Item = &'_ Box<dyn CloudObject>> {
        self.objects_by_id.values()
    }

    #[cfg(test)]
    pub fn add_object(&mut self, id: SyncId, object: impl CloudObject + 'static) {
        self.objects_by_id.insert(id.uid(), Box::new(object));
    }

    /// Pre-computes the set of UIDs for all active (non-trashed) objects using memoization.
    /// This is O(N) amortized instead of O(N × D) for the naive approach, because each
    /// object's trashed status is computed at most once and cached.
    pub fn active_object_uids(&self) -> HashSet<ObjectUid> {
        let mut cache = HashMap::new();
        let mut visiting = HashSet::new();
        let mut active = HashSet::new();
        for uid in self.objects_by_id.keys() {
            if !self.is_trashed_memoized(uid, &mut cache, &mut visiting) {
                active.insert(uid.clone());
            }
        }
        active
    }

    /// Memoized version of `is_trashed` that caches results to avoid redundant ancestor traversals.
    fn is_trashed_memoized(
        &self,
        uid: &str,
        cache: &mut HashMap<String, bool>,
        visiting: &mut HashSet<String>,
    ) -> bool {
        if let Some(&cached) = cache.get(uid) {
            return cached;
        }

        // Cycle detection: if we're already visiting this UID in the current traversal, treat as trashed.
        if visiting.contains(uid) {
            return true;
        }

        let result = match self.objects_by_id.get(uid) {
            Some(object) => {
                if object.metadata().trashed_ts.is_some() {
                    true
                } else {
                    match object.metadata().folder_id.map(|parent_id| parent_id.uid()) {
                        Some(parent_uid) => {
                            visiting.insert(uid.to_owned());
                            let r = self.is_trashed_memoized(&parent_uid, cache, visiting);
                            visiting.remove(uid);
                            r
                        }
                        None => false,
                    }
                }
            }
            None => !FeatureFlag::SharedWithMe.is_enabled(),
        };

        cache.insert(uid.to_owned(), result);
        result
    }

    /// Given a CloudObjectLocation (either a folder or a space), returns an iterator of active (not trashed) cloud objects
    /// that live directly in this location (its children). I.e. this function does NOT look into nested folders in order
    /// to return those children.
    pub fn active_cloud_objects_in_location_without_descendents<'a>(
        &'a self,
        location: CloudObjectLocation,
        app: &'a AppContext,
    ) -> impl Iterator<Item = &'a dyn CloudObject> + 'a {
        self.objects_by_id
            .values()
            .filter(move |object| {
                !object.is_trashed(self) && object.location(self, app) == location
            })
            .map(|object| object.as_ref())
    }

    /// Given a CloudObjectLocation (either a folder or a space), returns an iterator of trashed cloud objects
    /// that live directly in this location (its children). I.e. this function does NOT look into nested folders in order
    /// to return those children.
    pub fn trashed_cloud_objects_in_location_without_descendents<'a>(
        &'a self,
        location: CloudObjectLocation,
        app: &'a AppContext,
    ) -> impl Iterator<Item = &'a dyn CloudObject> + 'a {
        self.objects_by_id
            .values()
            .filter(move |object| object.is_trashed(self) && object.location(self, app) == location)
            .map(|object| object.as_ref())
    }

    pub fn trashed_cloud_object_types_in_location_with_descendants(
        &self,
        location: CloudObjectLocation,
        app: &AppContext,
    ) -> Vec<ObjectType> {
        let mut trashed_objects: Vec<ObjectType> = Vec::new();
        self.trashed_cloud_object_types_in_location_with_descendants_helper(
            location,
            &mut trashed_objects,
            app,
        );
        trashed_objects
    }

    /// Helper function for trashed_cloud_objects_in_location_with_descendants.
    /// Recursively traverses through descendants, adding object types of any trashed
    /// objects found to the trashed_objects mutable vector reference.
    fn trashed_cloud_object_types_in_location_with_descendants_helper(
        &self,
        location: CloudObjectLocation,
        trashed_objects: &mut Vec<ObjectType>,
        app: &AppContext,
    ) {
        // Fetch direct descendants of the location
        self.trashed_cloud_objects_in_location_without_descendents(location, app)
            .for_each(|object| {
                trashed_objects.push(object.object_type());
                let folder: Option<&CloudFolder> = object.into();
                // If any of the direct descendants are folders, recursively traverse through them
                if let Some(folder) = folder {
                    self.trashed_cloud_object_types_in_location_with_descendants_helper(
                        CloudObjectLocation::Folder(folder.id),
                        trashed_objects,
                        app,
                    );
                }
            });
    }

    /// Given a CloudObjectLocation (either a folder or a space), returns an iterator of cloud objects
    /// that live directly in this location (its children) are in the trash but have not been explicitly
    /// trashed by a user. I.e. this function does NOT look into nested folders in order to return those children.
    pub fn indirectly_trashed_cloud_objects_in_location_without_descendents<'a>(
        &'a self,
        location: CloudObjectLocation,
        app: &'a AppContext,
    ) -> impl Iterator<Item = &'a dyn CloudObject> {
        self.objects_by_id
            .values()
            .filter(move |object| {
                object.is_trashed(self)
                    && object.location(self, app) == location
                    && object.metadata().trashed_ts.is_none()
            })
            .map(|object| object.as_ref())
    }

    /// Returns all active (not trashed) cloud objects in the space.
    pub fn active_cloud_objects_in_space<'a>(
        &'a self,
        space: Space,
        app: &'a AppContext,
    ) -> impl Iterator<Item = &'a dyn CloudObject> + 'a {
        self.objects_by_id
            .values()
            .filter(move |object| object.is_in_space(space, app) && !object.is_trashed(self))
            .map(|object| object.as_ref())
    }

    /// Returns all active (not trashed) cloud objects in the space.
    pub fn active_non_welcome_cloud_objects_in_space<'a>(
        &'a self,
        space: Space,
        app: &'a AppContext,
    ) -> impl Iterator<Item = &'a dyn CloudObject> + 'a {
        self.objects_by_id
            .values()
            .filter(move |object| {
                object.is_in_space(space, app)
                    && !object.is_trashed(self)
                    && !object.is_welcome_object()
            })
            .map(|object| object.as_ref())
    }

    // Returns all objects, trashed or otherwise, in the space.
    pub fn all_cloud_objects_in_space<'a>(
        &'a self,
        space: Space,
        app: &'a AppContext,
    ) -> impl Iterator<Item = &'a dyn CloudObject> + 'a {
        self.objects_by_id
            .values()
            .filter(move |object| object.is_in_space(space, app))
            .map(|object| object.as_ref())
    }

    /// Returns all trashed cloud objects in the space.
    pub fn trashed_cloud_objects_in_space<'a>(
        &'a self,
        space: Space,
        app: &'a AppContext,
    ) -> impl Iterator<Item = &'a dyn CloudObject> + 'a {
        self.objects_by_id
            .values()
            .filter(move |object| object.is_in_space(space, app) && object.is_trashed(self))
            .map(|object| object.as_ref())
    }

    /// Returns all cloud objects in the space that have been explicitly trashed by a user.
    pub fn directly_trashed_cloud_objects_in_space<'a>(
        &'a self,
        space: Space,
        app: &'a AppContext,
    ) -> impl Iterator<Item = &'a dyn CloudObject> {
        self.objects_by_id
            .values()
            .filter(move |object| {
                object.is_in_space(space, app) && object.metadata().trashed_ts.is_some()
            })
            .map(|object| object.as_ref())
    }

    /// Returns a map of how many active (not trashed) objects reside within specified spaces.
    pub fn num_active_cloud_objects_per_space<'a, I>(
        &self,
        spaces: I,
        app: &AppContext,
    ) -> HashMap<Space, usize>
    where
        I: Iterator<Item = &'a Space>,
    {
        spaces
            .map(|space| {
                (
                    *space,
                    self.active_cloud_objects_in_space(*space, app).count(),
                )
            })
            .collect::<HashMap<_, _>>()
    }

    /// Returns a map of how many trashed objects reside within specified spaces.
    pub fn num_trashed_cloud_objects_per_space<'a, I>(
        &self,
        spaces: I,
        app: &AppContext,
    ) -> HashMap<Space, usize>
    where
        I: Iterator<Item = &'a Space>,
    {
        spaces
            .map(|space| {
                (
                    *space,
                    self.trashed_cloud_objects_in_space(*space, app).count(),
                )
            })
            .collect::<HashMap<_, _>>()
    }

    #[cfg(test)]
    pub fn mock(_ctx: &mut ModelContext<Self>) -> Self {
        Self::new(None, Vec::new(), None)
    }

    /// When `emit_events` is false (on the first load after login), per-object events
    /// are suppressed to avoid blocking the main thread. Subscribers react to the single
    /// `InitialLoadCompleted` event emitted afterward instead. Subsequent periodic polls
    /// pass `true` so that normal per-object events fire for incremental updates.
    pub fn update_objects_from_initial_load<K, M>(
        &mut self,
        cloud_objects: Vec<GenericServerObject<K, M>>,
        force_refresh: bool,
        emit_events: bool,
        ctx: &mut ModelContext<Self>,
    ) -> Vec<GenericCloudObject<K, M>>
    where
        K: HashableId + ToServerId + std::fmt::Debug + Into<String> + Clone + 'static,
        M: CloudModelType<IdType = K, CloudObjectType = GenericCloudObject<K, M>> + 'static,
    {
        let updated_objects = cloud_objects
            .into_iter()
            .filter_map(|object| {
                let sync_id = object.id;
                let metadata = object.metadata.clone();
                let permissions = object.permissions.clone();
                self.upsert_from_server_object_internal(object);
                self.maybe_update_object_metadata_internal(
                    &sync_id.uid(),
                    metadata,
                    force_refresh,
                    emit_events,
                    ctx,
                );
                self.update_object_permissions_internal(&sync_id.uid(), permissions);
                self.maybe_open_welcome_folder(&sync_id, ctx);
                self.get_object_of_type(&sync_id).cloned()
            })
            .collect();

        ctx.notify();
        updated_objects
    }

    // If the object is a folder and a welcome object, open it if we haven't opened a welcome folder before.
    fn maybe_open_welcome_folder(&mut self, object_id: &SyncId, ctx: &mut ModelContext<Self>) {
        if let Some(object) = self.get_by_uid(&object_id.uid()) {
            let folder: Option<&CloudFolder> = object.into();
            if let Some(folder) = folder {
                if folder.metadata().is_welcome_object {
                    // Doing this as a nested check as a slight optimization
                    if should_auto_open_welcome_folder(ctx) {
                        self.set_folder_open_state(folder.id, FolderOpenState::Open, ctx);
                        write_has_auto_opened_welcome_folder_to_user_defaults(ctx);
                    }
                }
            }
        }
    }

    #[cfg(test)]
    pub fn update_objects<K, M>(
        &mut self,
        server_objects: impl IntoIterator<Item = GenericServerObject<K, M>>,
        ctx: &mut ModelContext<Self>,
    ) where
        K: HashableId + ToServerId + std::fmt::Debug + Into<String> + Clone + 'static,
        M: CloudModelType<IdType = K, CloudObjectType = GenericCloudObject<K, M>> + 'static,
    {
        // List of all in memory objects that don't have pending changes, that potentially may
        // need to be removed from memory.
        let mut to_remove = self
            .get_all_objects_of_type::<K, M>()
            .filter(|&object| !object.metadata.has_pending_content_changes())
            .map(|object| object.id.uid())
            .collect::<HashSet<String>>();

        let objects_without_pending_changes = server_objects
            .into_iter()
            .filter_map(|server_object| {
                let id = server_object.id;
                self.upsert_from_server_object(server_object, ctx);

                // Remove the object from the set of objects that need to be deleted since we now
                // know the object still exists on the server.
                to_remove.remove(&id.uid());

                self.get_object_of_type::<K, M>(&id).cloned()
            })
            .collect::<Vec<_>>();

        // Remaining objects (those that were not synced back from the server) should be removed
        // from memory.
        to_remove.into_iter().for_each(|id| {
            self.objects_by_id.remove(&id);
        });

        if let Some(model_event_sender) = &self.model_event_sender {
            if let Err(e) = model_event_sender.send(M::bulk_upsert_event(
                objects_without_pending_changes.as_slice(),
            )) {
                log::error!("Error saving team objects to cache: {e:?}");
            }
        }
    }

    /// Whether the next object sync should force a refresh on all cloud objects
    pub fn cloud_objects_force_refresh_pending(&self) -> bool {
        // If there's no stated time for the next refresh, assume we should do one now. Otherwise,
        // check if we're at or past the time of the next refresh.
        self.time_of_next_force_refresh
            .is_none_or(|time_of_next_refresh| Utc::now() >= time_of_next_refresh)
    }

    /// After a successful force refresh, mark the state as completed by picking a
    /// time for the next refresh.
    pub fn mark_cloud_objects_refresh_as_completed(&mut self) -> DateTime<Utc> {
        // In order to offset when clients are performing the force refresh, we introduce
        // a small amount of randomness into the calculation. This is intended to distribute
        // server load to whatever extent possible.
        let mut rng = rand::thread_rng();
        let minutes_until_next_refresh = rng
            .gen_range(MIN_MINUTES_UNTIL_NEXT_FORCE_REFRESH..MAX_MINUTES_UNTIL_NEXT_FORCE_REFRESH);
        let next_refresh_time = Utc::now() + Duration::minutes(minutes_until_next_refresh);
        self.time_of_next_force_refresh = Some(next_refresh_time);
        next_refresh_time
    }

    pub fn reset(&mut self) {
        self.objects_by_id = HashMap::new();
        self.time_of_next_force_refresh = None;
    }
}

impl Entity for CloudModel {
    type Event = CloudModelEvent;
}

/// Mark CloudModel as global application state.
impl SingletonEntity for CloudModel {}

#[cfg(test)]
#[path = "model_tests.rs"]
mod tests;
