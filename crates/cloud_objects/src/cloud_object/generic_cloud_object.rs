use std::sync::Arc;

use super::{
    CloudObjectMetadata, CloudObjectPermissions, CloudObjectStatuses, CloudObjectSyncStatus,
    ConflictStatus, GenericServerObject, NumInFlightRequests, ObjectType, Owner,
};
use crate::ids::{ClientId, SyncId};

/// A portable payload for persisting or otherwise upserting a cloud object without app-local event types.
#[derive(Clone, Debug)]
pub struct CloudObjectUpsertParams<M> {
    pub id: SyncId,
    pub object_type: ObjectType,
    pub metadata: CloudObjectMetadata,
    pub permissions: CloudObjectPermissions,
    pub model: M,
}

/// A generic implementation of cloud objects that can be used for any model and id types.
///
/// For instance, rather than directly implementing the CloudObject trait, CloudObjects can
/// implement GenericCloudObject<K, M> where K is their id type and M is their model type.
///
/// For example, CloudNotebook becomes:
///
///   pub type CloudNotebook = GenericCloudObject<NotebookId, CloudNotebookModel>
///
/// The advantage of using the generic model is you get common implementations
/// of CloudObject methods like ```versions``` for free.
///
/// See the comments for CloudObject to understand the relationship between
/// this trait, CloudObject and CloudModelType.  They are tightly coupled.
#[derive(Clone, Debug)]
pub struct GenericCloudObject<K, M> {
    pub id: SyncId,
    pub metadata: CloudObjectMetadata,
    pub permissions: CloudObjectPermissions,
    /// Tracks whether this object has a conflict with the server version.
    /// This is runtime state (not persisted) - conflicts are always NoConflicts when loaded from SQLite.
    pub conflict_status: ConflictStatus<GenericServerObject<K, M>>,

    // Intentionally not public to prevent users of this class from holding
    // onto references to the model outside of this struct.
    //
    // This is an Arc in order to support clone-on-write semantics for the model.
    // By wrapping the model in an Arc, clones become cheap, and we can avoid
    // doing deep clones of the model whenever the containing object is cloned.
    //
    // Callers who want to update the model need to call set_model to update the
    // entire model atomically.
    model: Arc<M>,
}

impl<K, M> PartialEq for GenericCloudObject<K, M>
where
    M: PartialEq,
{
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && self.model() == other.model()
    }
}

impl<K, M> GenericCloudObject<K, M> {
    /// Gets a reference to the model held by the object.
    pub fn model(&self) -> &M {
        &self.model
    }

    /// Returns a shared handle to the model.
    pub fn shared_model(&self) -> Arc<M> {
        self.model.clone()
    }

    /// Sets a new version of the model on the object, replacing the old version.
    pub fn set_model(&mut self, model: M) {
        self.model = model.into();
    }

    /// Constructs a new instance of this model with the given id, model, metadata and permissions.
    pub fn new(
        id: SyncId,
        model: M,
        metadata: CloudObjectMetadata,
        permissions: CloudObjectPermissions,
    ) -> Self {
        Self {
            id,
            model: model.into(),
            metadata,
            permissions,
            conflict_status: ConflictStatus::NoConflicts,
        }
    }

    /// Creates a new GenericCloudObject with the given model, owner, and initial folder id.
    /// This is for the local creation flow, as opposed to creating from a server update.
    pub fn new_local(
        model: M,
        owner: Owner,
        initial_folder_id: Option<SyncId>,
        client_id: ClientId,
    ) -> Self {
        Self {
            id: SyncId::ClientId(client_id),
            model: model.into(),
            metadata: CloudObjectMetadata {
                pending_changes_statuses: CloudObjectStatuses {
                    content_sync_status: CloudObjectSyncStatus::InFlight(NumInFlightRequests(1)),
                    has_pending_metadata_change: false,
                    has_pending_permissions_change: false,
                    pending_untrash: false,
                    pending_delete: false,
                },
                folder_id: initial_folder_id,
                revision: Default::default(),
                metadata_last_updated_ts: Default::default(),
                current_editor_uid: Default::default(),
                trashed_ts: Default::default(),
                // Objects created from the client are never welcome objects.
                is_welcome_object: false,
                creator_uid: None,
                last_editor_uid: None,
                last_task_run_ts: None,
            },
            permissions: CloudObjectPermissions {
                owner,
                anyone_with_link: None,
                guests: Default::default(),
                permissions_last_updated_ts: None,
            },
            conflict_status: ConflictStatus::NoConflicts,
        }
    }

    /// Creates a new [`GenericCloudObject`] from a [`GenericServerObject`].
    pub fn new_from_server(server_object: GenericServerObject<K, M>) -> Self {
        Self {
            id: server_object.id,
            model: server_object.model.into(),
            metadata: CloudObjectMetadata::new_from_server(server_object.metadata),
            permissions: CloudObjectPermissions::new_from_server(server_object.permissions),
            conflict_status: ConflictStatus::NoConflicts,
        }
    }

    /// Marks this object as being in conflict with the provided object.
    pub fn set_conflicting_object(&mut self, object: Arc<GenericServerObject<K, M>>) {
        self.conflict_status = ConflictStatus::ConflictingChanges { object };
    }

    pub fn update_from_server_object(&mut self, server_object: GenericServerObject<K, M>) {
        // Check if we should create a conflict or apply the update.
        if self.metadata.has_pending_content_changes() || self.conflict_status.has_conflicts() {
            // There are pending changes, so this creates a conflict.
            self.conflict_status = ConflictStatus::ConflictingChanges {
                object: Arc::new(server_object),
            };
        } else {
            // No pending changes, apply the server update.
            self.metadata
                .update_revision_from_server(&server_object.metadata);
            self.model = server_object.model.into();
            self.conflict_status = ConflictStatus::NoConflicts;
        }
    }

    /// Returns portable upsert parameters for this object.
    pub fn upsert_params(&self, object_type: ObjectType) -> CloudObjectUpsertParams<M>
    where
        M: Clone,
    {
        CloudObjectUpsertParams {
            id: self.id,
            object_type,
            metadata: self.metadata.clone(),
            permissions: self.permissions.clone(),
            model: self.model().clone(),
        }
    }

    /// Converts this object into portable upsert parameters.
    pub fn into_upsert_params(self, object_type: ObjectType) -> CloudObjectUpsertParams<M>
    where
        M: Clone,
    {
        let Self {
            id,
            metadata,
            permissions,
            model,
            conflict_status: _,
        } = self;
        let model = Arc::try_unwrap(model).unwrap_or_else(|model| (*model).clone());
        CloudObjectUpsertParams {
            id,
            object_type,
            metadata,
            permissions,
            model,
        }
    }
}

impl<K, M> From<CloudObjectUpsertParams<M>> for GenericCloudObject<K, M> {
    fn from(params: CloudObjectUpsertParams<M>) -> Self {
        Self::new(params.id, params.model, params.metadata, params.permissions)
    }
}
