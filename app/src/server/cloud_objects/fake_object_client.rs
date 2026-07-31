//! Stateful fake implementation of [`ObjectClient`] for end-to-end tests
//! of cloud preferences sync.
//!
//! Unlike the auto-generated [`MockObjectClient`] (which requires each test
//! to script per-method `expect_*()` calls up front), this fake maintains
//! an in-memory store of preferences that can be read back after the
//! syncer runs. Tests use [`FakeObjectClient::seed_preference`] to add
//! cloud-side state and [`FakeObjectClient::cloud_value`] to assert on
//! the result after a sync roundtrip.
//!
//! Only the subset of [`ObjectClient`] methods actually called by the
//! cloud preferences syncer has a real implementation — every other
//! method panics with `unimplemented!()`. This is deliberate: callers
//! that hit one of these panics have likely wired a non-preferences
//! code path through the fake by mistake.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use anyhow::{Result, anyhow};
use async_channel::Sender;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use cloud_object_client::ObjectPermissionUpdateResult;
use warp_graphql::object_permissions::AccessLevel;

use crate::cloud_object::model::actions::{ObjectActionHistory, ObjectActionType};
use crate::cloud_object::model::generic_string_model::GenericStringObjectId;
use crate::cloud_object::{
    BulkCreateCloudObjectResult, BulkCreateGenericStringObjectsRequest, CreateCloudObjectResult,
    CreateObjectRequest, CreatedCloudObject, GenericStringObjectFormat,
    GenericStringObjectUniqueKey, JsonObjectType, ObjectDeleteResult, ObjectIdType,
    ObjectMetadataUpdateResult, ObjectPermissionsUpdateData, ObjectType, ObjectsToUpdate, Owner,
    Revision, RevisionAndLastEditor, ServerFolder, ServerMetadata, ServerNotebook, ServerObject,
    ServerPermissions, ServerPreference, ServerWorkflow, UpdateCloudObjectResult,
};
use crate::drive::folders::FolderId;
use crate::drive::sharing::SharingAccessLevel;
use crate::notebooks::NotebookId;
use crate::server::cloud_objects::listener::ObjectUpdateMessage;
use crate::server::cloud_objects::update_manager::{GetCloudObjectResponse, InitialLoadResponse};
use crate::server::ids::{ServerId, ServerIdAndType, SyncId};
use crate::server::server_api::object::{GuestIdentifier, ObjectClient};
use crate::server::sync_queue::SerializedModel;
use crate::settings::cloud_preferences::{CloudPreferenceModel, Platform, Preference};
use crate::workflows::WorkflowId;

/// A stateful fake cloud preferences backend.
#[derive(Clone, Default)]
pub struct FakeObjectClient {
    state: Arc<Mutex<FakeCloudState>>,
}

#[derive(Default)]
struct FakeCloudState {
    /// Stored preferences keyed by `GenericStringObjectId`. The
    /// preferences syncer addresses updates and deletes by id, so this
    /// is the authoritative lookup.
    preferences: HashMap<GenericStringObjectId, StoredPreference>,
    /// Monotonically increasing counter used to allocate fresh
    /// [`ServerId`]s for newly created preferences.
    next_server_id: i64,
}

struct StoredPreference {
    model: CloudPreferenceModel,
    revision: Revision,
    metadata_ts: chrono::DateTime<Utc>,
}

impl FakeObjectClient {
    /// Seeds a preference into the cloud store as if another client had
    /// already written it. Returns the allocated [`ServerId`] so tests
    /// can reference it later if needed.
    ///
    /// The `value_json` argument is the JSON-serialized setting value
    /// (e.g. `"14.0"` for a float, `"true"` for a bool, `"\"Hack\""`
    /// for a string). This matches the format used by
    /// [`Preference::new`].
    pub fn seed_preference(
        &self,
        storage_key: &str,
        value_json: &str,
        platform: Platform,
    ) -> ServerId {
        let preference = build_preference(storage_key, value_json, platform);
        let model = CloudPreferenceModel::new(preference);

        let mut state = self.state.lock().unwrap();
        let server_id = state.alloc_server_id();
        state.preferences.insert(
            GenericStringObjectId::from(server_id),
            StoredPreference {
                model,
                revision: Revision::now(),
                metadata_ts: Utc::now(),
            },
        );
        server_id
    }

    /// Returns the current cloud value for the preference with the
    /// given storage key and platform, or `None` if the fake has no
    /// such preference.
    ///
    /// The returned string is the JSON-serialized setting value (same
    /// shape as [`Self::seed_preference`]).
    pub fn cloud_value(&self, storage_key: &str, platform: Platform) -> Option<String> {
        let state = self.state.lock().unwrap();
        state
            .preferences
            .values()
            .find(|stored| {
                stored.model.string_model.storage_key == storage_key
                    && stored.model.string_model.platform == platform
            })
            .map(|stored| stored.model.string_model.value.to_string())
    }

    /// Builds an [`InitialLoadResponse`] that reflects the current
    /// state of the fake. Tests pass the return value to
    /// [`UpdateManager::mock_initial_load`] to simulate the syncer's
    /// initial load seeing the seeded cloud state.
    pub fn snapshot_as_initial_load_response(&self) -> InitialLoadResponse {
        let state = self.state.lock().unwrap();
        let server_objects: Vec<Box<dyn ServerObject>> = state
            .preferences
            .iter()
            .map(|(id, stored)| {
                let metadata = ServerMetadata {
                    uid: ServerId::from(*id),
                    revision: stored.revision.clone(),
                    metadata_last_updated_ts: stored.metadata_ts.into(),
                    trashed_ts: None,
                    folder_id: None,
                    is_welcome_object: false,
                    creator_uid: None,
                    last_editor_uid: None,
                    current_editor_uid: None,
                };
                let permissions = ServerPermissions {
                    space: Owner::mock_current_user(),
                    guests: Vec::new(),
                    anyone_link_sharing: None,
                    permissions_last_updated_ts: stored.metadata_ts.into(),
                };
                let server_pref = ServerPreference::new(
                    SyncId::ServerId(ServerId::from(*id)),
                    stored.model.clone(),
                    metadata,
                    permissions,
                );
                Box::new(server_pref) as Box<dyn ServerObject>
            })
            .collect();

        let mut response = InitialLoadResponse::default();
        if !server_objects.is_empty() {
            response.updated_generic_string_objects.insert(
                GenericStringObjectFormat::Json(JsonObjectType::Preference),
                server_objects,
            );
        }
        response
    }
}

impl FakeCloudState {
    fn alloc_server_id(&mut self) -> ServerId {
        self.next_server_id += 1;
        ServerId::from(self.next_server_id)
    }
}

/// Builds a [`Preference`] struct from a storage key, JSON value, and
/// explicit platform. This bypasses [`Preference::new`]'s syncing-mode
/// inference so tests can seed preferences with an arbitrary platform.
fn build_preference(storage_key: &str, value_json: &str, platform: Platform) -> Preference {
    let value: serde_json::Value = serde_json::from_str(value_json)
        .unwrap_or_else(|err| panic!("invalid JSON value {value_json:?}: {err}"));
    Preference {
        storage_key: storage_key.to_owned(),
        value,
        platform,
    }
}

#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
impl ObjectClient for FakeObjectClient {
    async fn bulk_create_generic_string_objects(
        &self,
        _owner: Owner,
        objects: &[BulkCreateGenericStringObjectsRequest],
    ) -> Result<BulkCreateCloudObjectResult> {
        let mut state = self.state.lock().unwrap();
        let mut created = Vec::with_capacity(objects.len());
        for request in objects {
            let serialized = request.serialized_model.model_as_str();
            let model = CloudPreferenceModel::deserialize_owned(serialized)
                .map_err(|err| anyhow!("fake cloud failed to deserialize preference: {err}"))?;
            let server_id = state.alloc_server_id();
            let now = Utc::now();
            state.preferences.insert(
                GenericStringObjectId::from(server_id),
                StoredPreference {
                    model,
                    revision: Revision::now(),
                    metadata_ts: now,
                },
            );
            created.push(CreatedCloudObject {
                client_id: request.id,
                revision_and_editor: RevisionAndLastEditor {
                    revision: Revision::now(),
                    last_editor_uid: None,
                },
                metadata_ts: now.into(),
                server_id_and_type: ServerIdAndType {
                    id: server_id,
                    id_type: ObjectIdType::GenericStringObject,
                },
                creator_uid: None,
                permissions: ServerPermissions::mock_personal(),
            });
        }
        Ok(BulkCreateCloudObjectResult::Success {
            created_cloud_objects: created,
        })
    }

    async fn update_generic_string_object(
        &self,
        object_id: GenericStringObjectId,
        model: SerializedModel,
        _revision: Option<Revision>,
    ) -> Result<UpdateCloudObjectResult<Box<dyn ServerObject>>> {
        let mut state = self.state.lock().unwrap();
        let stored = state
            .preferences
            .get_mut(&object_id)
            .ok_or_else(|| anyhow!("fake cloud: no preference with id {object_id:?}"))?;
        stored.model = CloudPreferenceModel::deserialize_owned(model.model_as_str())
            .map_err(|err| anyhow!("fake cloud failed to deserialize preference: {err}"))?;
        stored.revision = Revision::now();
        stored.metadata_ts = Utc::now();
        Ok(UpdateCloudObjectResult::Success {
            revision_and_editor: RevisionAndLastEditor {
                revision: stored.revision.clone(),
                last_editor_uid: None,
            },
        })
    }

    async fn delete_object(&self, id: ServerId) -> Result<ObjectDeleteResult> {
        let mut state = self.state.lock().unwrap();
        state.preferences.remove(&GenericStringObjectId::from(id));
        Ok(ObjectDeleteResult::Success {
            deleted_ids: vec![SyncId::ServerId(id)],
        })
    }

    async fn fetch_changed_objects(
        &self,
        _objects_to_update: ObjectsToUpdate,
        _force_refresh: bool,
    ) -> Result<InitialLoadResponse> {
        Ok(self.snapshot_as_initial_load_response())
    }

    async fn fetch_environment_last_task_run_timestamps(
        &self,
    ) -> Result<HashMap<String, DateTime<Utc>>> {
        Ok(HashMap::new())
    }

    // ───────────────────────────────────────────────────────────────
    // The methods below are not exercised by CloudPreferencesSyncer,
    // so they intentionally panic. If a future change to the syncer
    // starts calling one of these, the test that triggered the call
    // will fail loudly with a clear message rather than silently
    // misbehave.
    // ───────────────────────────────────────────────────────────────

    async fn create_workflow(
        &self,
        _request: CreateObjectRequest,
    ) -> Result<CreateCloudObjectResult> {
        unimplemented!("FakeObjectClient::create_workflow")
    }

    async fn update_workflow(
        &self,
        _workflow_id: WorkflowId,
        _data: SerializedModel,
        _revision: Option<Revision>,
    ) -> Result<UpdateCloudObjectResult<ServerWorkflow>> {
        unimplemented!("FakeObjectClient::update_workflow")
    }

    async fn create_generic_string_object(
        &self,
        _format: GenericStringObjectFormat,
        _uniqueness_key: Option<GenericStringObjectUniqueKey>,
        _request: CreateObjectRequest,
    ) -> Result<CreateCloudObjectResult> {
        unimplemented!("FakeObjectClient::create_generic_string_object")
    }

    async fn create_notebook(
        &self,
        _request: CreateObjectRequest,
    ) -> Result<CreateCloudObjectResult> {
        unimplemented!("FakeObjectClient::create_notebook")
    }

    async fn update_notebook(
        &self,
        _notebook_id: NotebookId,
        _title: Option<String>,
        _data: Option<SerializedModel>,
        _revision: Option<Revision>,
    ) -> Result<UpdateCloudObjectResult<ServerNotebook>> {
        unimplemented!("FakeObjectClient::update_notebook")
    }

    async fn create_folder(
        &self,
        _request: CreateObjectRequest,
    ) -> Result<CreateCloudObjectResult> {
        unimplemented!("FakeObjectClient::create_folder")
    }

    async fn update_folder(
        &self,
        _folder_id: FolderId,
        _name: SerializedModel,
    ) -> Result<UpdateCloudObjectResult<ServerFolder>> {
        unimplemented!("FakeObjectClient::update_folder")
    }

    async fn grab_notebook_edit_access(&self, _notebook_id: NotebookId) -> Result<ServerMetadata> {
        unimplemented!("FakeObjectClient::grab_notebook_edit_access")
    }

    async fn give_up_notebook_edit_access(
        &self,
        _notebook_id: NotebookId,
    ) -> Result<ServerMetadata> {
        unimplemented!("FakeObjectClient::give_up_notebook_edit_access")
    }

    async fn get_warp_drive_updates(
        &self,
        _message_sender: Sender<ObjectUpdateMessage>,
        _stream_ready_sender: Sender<()>,
    ) -> Result<()> {
        unimplemented!("FakeObjectClient::get_warp_drive_updates")
    }

    async fn fetch_single_cloud_object(&self, _id: ServerId) -> Result<GetCloudObjectResponse> {
        unimplemented!("FakeObjectClient::fetch_single_cloud_object")
    }

    async fn transfer_notebook_owner(
        &self,
        _notebook_id: NotebookId,
        _owner: Owner,
    ) -> Result<bool> {
        unimplemented!("FakeObjectClient::transfer_notebook_owner")
    }

    async fn transfer_workflow_owner(
        &self,
        _workflow_id: WorkflowId,
        _owner: Owner,
    ) -> Result<bool> {
        unimplemented!("FakeObjectClient::transfer_workflow_owner")
    }

    async fn transfer_generic_string_object_owner(
        &self,
        _id: GenericStringObjectId,
        _owner: Owner,
    ) -> Result<bool> {
        unimplemented!("FakeObjectClient::transfer_generic_string_object_owner")
    }

    async fn trash_object(&self, _id: ServerId) -> Result<bool> {
        unimplemented!("FakeObjectClient::trash_object")
    }

    async fn untrash_object(&self, _id: ServerId) -> Result<ObjectMetadataUpdateResult> {
        unimplemented!("FakeObjectClient::untrash_object")
    }

    async fn empty_trash(&self, _owner: Owner) -> Result<ObjectDeleteResult> {
        unimplemented!("FakeObjectClient::empty_trash")
    }

    async fn move_object(
        &self,
        _id: ServerId,
        _folder_id: Option<FolderId>,
        _owner: Owner,
        _object_type: ObjectType,
    ) -> Result<bool> {
        unimplemented!("FakeObjectClient::move_object")
    }

    async fn record_object_action(
        &self,
        _id: ServerId,
        _action_type: ObjectActionType,
        _timestamp: DateTime<Utc>,
        _data: Option<String>,
    ) -> Result<ObjectActionHistory> {
        unimplemented!("FakeObjectClient::record_object_action")
    }

    async fn leave_object(&self, _id: ServerId) -> Result<ObjectDeleteResult> {
        unimplemented!("FakeObjectClient::leave_object")
    }

    async fn set_object_link_permissions(
        &self,
        _object_id: ServerId,
        _access_level: SharingAccessLevel,
    ) -> Result<ObjectPermissionUpdateResult> {
        unimplemented!("FakeObjectClient::set_object_link_permissions")
    }

    async fn remove_object_link_permissions(
        &self,
        _object_id: ServerId,
    ) -> Result<ObjectPermissionUpdateResult> {
        unimplemented!("FakeObjectClient::remove_object_link_permissions")
    }

    async fn add_object_guests(
        &self,
        _object_id: ServerId,
        _guest_emails: Vec<String>,
        _access_level: AccessLevel,
    ) -> Result<ObjectPermissionsUpdateData> {
        unimplemented!("FakeObjectClient::add_object_guests")
    }

    async fn update_object_guests(
        &self,
        _object_id: ServerId,
        _guest_emails: Vec<String>,
        _access_level: AccessLevel,
    ) -> Result<ServerPermissions> {
        unimplemented!("FakeObjectClient::update_object_guests")
    }

    async fn remove_object_guest(
        &self,
        _object_id: ServerId,
        _guest: GuestIdentifier,
    ) -> Result<ServerPermissions> {
        unimplemented!("FakeObjectClient::remove_object_guest")
    }
}
