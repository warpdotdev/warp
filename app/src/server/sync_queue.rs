use chrono::{DateTime, Utc};
use derivative::Derivative;
use http::StatusCode;
use std::borrow::Cow;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};
use warp_graphql::scalars::time::ServerTimestamp;
use warpui::{r#async::FutureId, Entity, ModelContext, RequestState, RetryOption, SingletonEntity};

use lazy_static::lazy_static;
use uuid::Uuid;

use super::{
    graphql::GraphQLError,
    ids::{ClientId, HashableId, ObjectUid, ServerId, SyncId, ToServerId},
    server_api::{auth::UserAuthenticationError, object::ObjectClient},
};

use crate::ai::mcp::templatable::CloudTemplatableMCPServerModel;
use crate::server::cloud_objects::update_manager::InitiatedBy;
use crate::{
    ai::cloud_agent_config::CloudAgentConfigModel,
    ai::cloud_environments::CloudAmbientAgentEnvironmentModel,
    ai::{
        ambient_agents::scheduled::CloudScheduledAmbientAgentModel,
        execution_profiles::CloudAIExecutionProfileModel, facts::CloudAIFactModel,
        mcp::CloudMCPServerModel,
    },
    cloud_object::{
        model::{
            actions::{ObjectAction, ObjectActionHistory, ObjectActionSubtype, ObjectActionType},
            generic_string_model::GenericStringObjectId,
        },
        BulkCreateCloudObjectResult, BulkCreateGenericStringObjectsRequest, CloudModelType,
        CloudObject, CloudObjectEventEntrypoint, CreateCloudObjectResult, CreateObjectRequest,
        GenericCloudObject, GenericStringObjectFormat, GenericStringObjectUniqueKey,
        JsonObjectType, ObjectType, Owner, Revision, RevisionAndLastEditor, ServerCloudObject,
        ServerCreationInfo, UpdateCloudObjectResult,
    },
    drive::{folders::CloudFolderModel, CloudObjectTypeAndId},
    env_vars::CloudEnvVarCollectionModel,
    notebooks::CloudNotebookModel,
    settings::cloud_preferences::CloudPreferenceModel,
    workflows::{workflow_enum::CloudWorkflowEnumModel, CloudWorkflowModel},
};

lazy_static! {
    static ref DEFAULT_RETRY_OPTION: RetryOption =
        RetryOption::exponential(Duration::from_secs(1), 2., 3);
}

/// Type of successful update response from server.
#[allow(clippy::large_enum_variant)]
enum ResponseType {
    Creation {
        creation_result: CreationResponseType,
    },
    Update {
        update_result: UpdateResponseType,
    },
    ObjectAction {
        action_timestamp: DateTime<Utc>,
        action_history: ObjectActionHistory,
    },
}

/// Allow large enum variant because success is the most common by far
#[allow(clippy::large_enum_variant)]
enum CreationResponseType {
    Success {
        client_id: ClientId,
        revision_and_editor: RevisionAndLastEditor,
        metadata_ts: ServerTimestamp,
        server_creation_info: ServerCreationInfo,
    },
    UserFacingError {
        message: String,
        client_id: ClientId,
    },
}

enum UpdateResponseType {
    Success {
        revision_and_editor: RevisionAndLastEditor,
    },
    /// The update was rejected because the update was not sent from the current revision in
    /// storage. The object and revision in storage are returned.
    Rejected { object: Box<ServerCloudObject> },
}

// A newtype for a serialized model that wraps a plain string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SerializedModel(String);

impl SerializedModel {
    pub fn new(s: String) -> Self {
        Self(s)
    }

    pub fn model_as_str(&self) -> &str {
        &self.0
    }

    pub fn take(self) -> String {
        self.0
    }
}

impl From<String> for SerializedModel {
    fn from(s: String) -> Self {
        Self(s)
    }
}

#[derive(Debug, PartialEq, Clone)]
pub struct GenericStringObjectToCreate {
    pub id: ClientId,
    pub format: GenericStringObjectFormat,
    pub serialized_model: Arc<SerializedModel>,
    pub initial_folder_id: Option<SyncId>,
    pub entrypoint: CloudObjectEventEntrypoint,
    pub uniqueness_key: Option<GenericStringObjectUniqueKey>,
    pub initiated_by: InitiatedBy,
}

/// An ID for a `QueueItem` in the sync queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QueueItemId(Uuid);

impl QueueItemId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

/// Item that sync queue supports.
#[derive(Derivative, Debug)]
#[derivative(PartialEq, Eq, Clone)]
pub enum QueueItem {
    CreateObject {
        object_type: ObjectType,
        owner: Owner,
        id: ClientId,
        title: Option<Arc<String>>,
        serialized_model: Option<Arc<SerializedModel>>,
        initial_folder_id: Option<SyncId>,
        entrypoint: CloudObjectEventEntrypoint,
        initiated_by: InitiatedBy,
    },
    // Separate CreateWorkflow type that should be removed when we do the SyncId refactor.
    // Stores a reference to a CloudWorkflowModel, rather than a SerializedModel, which is needed
    // for updating an enqueued workflow with the server IDs of enums it references as they are created.
    CreateWorkflow {
        object_type: ObjectType,
        owner: Owner,
        id: ClientId,
        #[derivative(PartialEq = "ignore")]
        model: Arc<CloudWorkflowModel>,
        initial_folder_id: Option<SyncId>,
        entrypoint: CloudObjectEventEntrypoint,
        initiated_by: InitiatedBy,
    },
    BulkCreateGenericStringObjects {
        owner: Owner,
        objects: Vec<GenericStringObjectToCreate>,
    },
    // Note, we continue to do UpdateXXX items per object type here
    // because it's the most type safe way to handle the different id types
    // and the different update payloads.  Most of the logic is still shared
    // via a single update_object method.
    UpdateNotebook {
        model: Arc<CloudNotebookModel>,
        id: SyncId,
        revision: Option<Revision>,
    },
    UpdateWorkflow {
        model: Arc<CloudWorkflowModel>,
        id: SyncId,
        revision: Option<Revision>,
    },
    UpdateFolder {
        id: SyncId,
        model: Arc<CloudFolderModel>,
    },
    UpdateCloudPreferences {
        model: Arc<CloudPreferenceModel>,
        id: SyncId,
        revision: Option<Revision>,
    },
    UpdateEnvVarCollection {
        model: Arc<CloudEnvVarCollectionModel>,
        id: SyncId,
        revision: Option<Revision>,
    },
    UpdateWorkflowEnum {
        model: Arc<CloudWorkflowEnumModel>,
        id: SyncId,
        revision: Option<Revision>,
    },
    UpdateAIFact {
        model: Arc<CloudAIFactModel>,
        id: SyncId,
        revision: Option<Revision>,
    },
    UpdateMCPServer {
        model: Arc<CloudMCPServerModel>,
        id: SyncId,
        revision: Option<Revision>,
    },
    UpdateAIExecutionProfile {
        model: Arc<CloudAIExecutionProfileModel>,
        id: SyncId,
        revision: Option<Revision>,
    },
    UpdateTemplatableMCPServer {
        model: Arc<CloudTemplatableMCPServerModel>,
        id: SyncId,
        revision: Option<Revision>,
    },
    UpdateCloudEnvironment {
        model: Arc<CloudAmbientAgentEnvironmentModel>,
        id: SyncId,
        revision: Option<Revision>,
    },
    UpdateScheduledAmbientAgent {
        model: Arc<CloudScheduledAmbientAgentModel>,
        id: SyncId,
        revision: Option<Revision>,
    },
    UpdateCloudAgentConfig {
        model: Arc<CloudAgentConfigModel>,
        id: SyncId,
        revision: Option<Revision>,
    },
    RecordObjectAction {
        id_and_type: CloudObjectTypeAndId,
        action_type: ObjectActionType,
        action_timestamp: DateTime<Utc>,
        data: Option<String>,
    },
}

impl QueueItem {
    pub fn from_cached_objects(
        objects: impl Iterator<Item = Box<dyn CloudObject>>,
    ) -> Vec<QueueItem> {
        objects
            .map(|object| {
                if let Some(create_object_queue_item) = object.create_object_queue_item(
                    CloudObjectEventEntrypoint::default(),
                    // InitiatedBy::User was added as a default value since we do not save the initiated_by values in the Sqlite cache.
                    // InitiatedBy::User is a safer default option because it will show toasts.
                    // In the future, if System events are common, we may want to save the initiated_by field in Sqlite.
                    InitiatedBy::User,
                ) {
                    create_object_queue_item
                } else {
                    object.update_object_queue_item(None)
                }
            })
            .collect::<Vec<_>>()
    }

    // Converts a list of pending actions into SyncQueue items that can be sent to the server.
    pub fn from_unsynced_actions(
        actions: impl Iterator<Item = (CloudObjectTypeAndId, ObjectAction)>,
    ) -> Vec<QueueItem> {
        actions
            .filter_map(|(id_and_type, action)| match action.action_subtype {
                ObjectActionSubtype::SingleAction {
                    timestamp,
                    data,
                    pending: true,
                    ..
                } => Some(QueueItem::RecordObjectAction {
                    id_and_type,
                    action_type: action.action_type,
                    action_timestamp: timestamp,
                    data,
                }),
                _ => None,
            })
            .collect::<Vec<_>>()
    }
}

#[derive(Derivative, Clone, Debug)]
#[derivative(PartialEq, Eq)]
#[allow(clippy::enum_variant_names)]
pub enum CreationFailureReason {
    UniqueKeyConflict {
        id: String,
        initiated_by: InitiatedBy,
    },
    Denied {
        message: String,
        client_id: ClientId,
        initiated_by: InitiatedBy,
    },
    Other {
        id: String,
        initiated_by: InitiatedBy,
    },
}

#[derive(Derivative, Clone, Debug)]
#[derivative(PartialEq, Eq)]
#[allow(clippy::enum_variant_names)]
#[allow(clippy::large_enum_variant)]
pub enum SyncQueueEvent {
    /// Emitted when we receive a successful response on creating a shareable object.
    ObjectCreationSuccessful {
        server_creation_info: ServerCreationInfo,
        /// The client id of the object that was successfully created.
        client_id: ClientId,
        /// The revision of the object after creation.
        revision_and_editor: RevisionAndLastEditor,
        /// The timestamp of the object metadata after creation.
        metadata_ts: ServerTimestamp,
        /// Whether the creation was initiated by the user or the system.
        initiated_by: InitiatedBy,
    },
    /// Emitted when we receive a successful response on updating a shareable object.
    ObjectUpdateSuccessful {
        /// The server id of the object
        server_id: ServerId,
        /// The revision of the object after updating.
        revision_and_editor: RevisionAndLastEditor,
    },
    ObjectUpdateRejected {
        id: String,
        #[derivative(PartialEq = "ignore")]
        object: Arc<ServerCloudObject>,
    },
    #[allow(dead_code)]
    ObjectUpdateFeatureNotAvailable { id: String },
    /// Request to server for creating a queue item has failed.
    ObjectCreationFailure { reason: CreationFailureReason },
    /// Request to server for updating a queue item has failed.
    ObjectUpdateFailure { id: SyncId },
    ReportObjectActionFailed {
        uid: ObjectUid,
        action_timestamp: DateTime<Utc>,
    },
    ReportObjectActionSucceeded {
        uid: ObjectUid,
        action_timestamp: DateTime<Utc>,
        action_history: ObjectActionHistory,
    },
}

/// Central mechanism for syncing shareable objects to the server. All CRUD actions
/// on team objects should go through here to make sure we update the server in correct
/// serial order and handle errors with exponential backoff.
pub struct SyncQueue {
    queue: Vec<(QueueItemId, QueueItem)>,
    waiting_response: HashMap<String, HashSet<QueueItemId>>,
    object_client: Arc<dyn ObjectClient>,
    client_id_to_server: HashMap<ClientId, String>,
    server_id_to_client_hash: HashMap<String, String>,
    should_dequeue: bool,
    // Vec of futures that have been spawned after dequeueing items. Used within tests to ensure
    // calls to `ctx#spawn` have finished before asserting.
    spawned_futures: Vec<FutureId>,

    // For each QueueItem, store the set of QueueItems it depends upon finishing first
    queue_dependencies: HashMap<QueueItemId, HashSet<QueueItemId>>,
}

impl SyncQueue {
    #[cfg(test)]
    pub fn mock(ctx: &mut ModelContext<Self>) -> Self {
        use super::server_api::ServerApiProvider;

        Self::new(
            Default::default(),
            ServerApiProvider::new_for_test().get(),
            ctx,
        )
    }

    pub fn new(
        queue_items: Vec<QueueItem>,
        object_client: Arc<dyn ObjectClient>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let mut sync_queue = Self {
            queue: queue_items
                .into_iter()
                .map(|queue_item| (QueueItemId::new(), queue_item))
                .collect(),
            waiting_response: Default::default(),
            object_client,
            client_id_to_server: Default::default(),
            server_id_to_client_hash: Default::default(),
            should_dequeue: false,
            spawned_futures: vec![],
            queue_dependencies: Default::default(),
        };

        sync_queue.initialize_queue_dependencies(ctx);
        sync_queue
    }

    pub fn is_dequeueing(&self) -> bool {
        self.should_dequeue
    }

    pub fn stop_dequeueing(&mut self) {
        self.should_dequeue = false
    }

    pub fn start_dequeueing(&mut self, ctx: &mut ModelContext<Self>) {
        self.should_dequeue = true;
        self.dequeue(ctx)
    }

    // Clear the SyncQueue. This is used during Logout.
    pub fn clear(&mut self) {
        self.queue.clear();
        self.waiting_response.clear();
        self.queue_dependencies.clear();
        self.client_id_to_server.clear();
        self.server_id_to_client_hash.clear();
    }

    // Remove the (id, item) pair that matches `remove_id` from the SyncQueue
    fn remove_id_from_queue(&mut self, remove_id: &QueueItemId) {
        self.queue.retain(|(id, _)| id != remove_id);
    }

    /// Enqueue a new request.
    pub fn enqueue(&mut self, item: QueueItem, ctx: &mut ModelContext<Self>) -> QueueItemId {
        let queue_id = QueueItemId::new();
        let mut queue_item = item;

        self.add_inferred_dependencies(&mut queue_item, &queue_id, ctx);
        self.queue.push((queue_id, queue_item));
        self.dequeue(ctx);
        queue_id
    }

    /// Given a queue item, infer its dependencies based on the current state of the queue and add them to queue_dependencies.
    pub fn add_inferred_dependencies(
        &mut self,
        item: &mut QueueItem,
        item_id: &QueueItemId,
        ctx: &mut ModelContext<Self>,
    ) {
        let mut dependencies = match item {
            // Update requests will depend on any existing create/updates to the same object
            QueueItem::UpdateNotebook { id, .. } => self.get_update_dependencies(id),
            QueueItem::UpdateFolder { id, .. } => self.get_update_dependencies(id),
            QueueItem::UpdateCloudPreferences { id, .. } => self.get_update_dependencies(id),
            QueueItem::UpdateEnvVarCollection { id, .. } => self.get_update_dependencies(id),
            QueueItem::UpdateWorkflowEnum { id, .. } => self.get_update_dependencies(id),
            QueueItem::UpdateCloudEnvironment { id, .. } => self.get_update_dependencies(id),
            QueueItem::UpdateScheduledAmbientAgent { id, .. } => self.get_update_dependencies(id),
            QueueItem::UpdateCloudAgentConfig { id, .. } => self.get_update_dependencies(id),

            // Update workflow requests should depend on existing requests to that object, as well as
            // any enums or env vars they reference.
            QueueItem::UpdateWorkflow {
                id: workflow_id,
                model,
                ..
            } => {
                let mut dependencies = self.get_update_dependencies(workflow_id);
                let enum_dependencies = self.get_workflow_object_dependencies(
                    Arc::make_mut(model),
                    *workflow_id,
                    item_id,
                    ctx,
                );
                match enum_dependencies {
                    Ok(deps) => dependencies.extend(deps),
                    Err(_) => self.handle_update_failure_response(*workflow_id, *item_id, ctx),
                }

                dependencies
            }

            // Workflow creation requests should depend on every enum they reference.
            // We should never dequeue a Workflow that references ClientIds rather than ServerIds.
            QueueItem::CreateWorkflow {
                model,
                id: workflow_id,
                initiated_by,
                ..
            } => {
                let mut dependencies = HashSet::new();
                let enum_dependencies = self.get_workflow_object_dependencies(
                    Arc::make_mut(model),
                    SyncId::ClientId(*workflow_id),
                    item_id,
                    ctx,
                );
                match enum_dependencies {
                    Ok(deps) => dependencies.extend(deps),
                    Err(_) => self.handle_creation_failure_response(
                        workflow_id.to_string(),
                        *item_id,
                        *initiated_by,
                        ctx,
                    ),
                }

                dependencies
            }

            // Other queue item types don't have dependencies supported right now
            _ => HashSet::new(),
        };

        // We never want an object to be dependent on itself, which is possible when we infer dependencies on startup from SQLite
        dependencies.remove(item_id);

        self.queue_dependencies.insert(*item_id, dependencies);
    }

    /// Given a queue, will initialize queue dependencies for every item in the queue.
    /// Intended to be run only after loading in queue items from SQLite on startup.
    pub fn initialize_queue_dependencies(&mut self, ctx: &mut ModelContext<Self>) {
        let mut queue = self.queue.clone();
        for (queue_item_id, queue_item) in queue.iter_mut() {
            self.add_inferred_dependencies(queue_item, queue_item_id, ctx);
        }
        self.queue = queue
    }

    /// Given an object ID, return the set of queue IDs of all queue items that operate on that object
    fn get_items_with_object_id(&self, item_id: String) -> HashSet<QueueItemId> {
        // Get a list of in flight QueueItems that operate on objects with `item_id`
        // We still want to map these in our dependencies, so that when they succeed or fail, we can respond accordingly
        // even if a subsequent request was enqueued after the relevant request was dequeued and is already in flight
        let in_flight_dependencies = self
            .waiting_response
            .get(&item_id)
            .into_iter()
            .flat_map(|dependencies| dependencies.iter())
            .copied();

        self.queue
            .iter()
            .filter(|(_, queue_item)| match queue_item {
                QueueItem::CreateObject { id, .. } => id.to_string() == item_id,
                QueueItem::CreateWorkflow { id, .. } => id.to_string() == item_id,
                QueueItem::BulkCreateGenericStringObjects { objects, .. } => {
                    objects.iter().any(|data| data.id.to_string() == item_id)
                }
                QueueItem::UpdateCloudPreferences { id, .. } => id.uid() == item_id,
                QueueItem::UpdateNotebook { id, .. } => id.uid() == item_id,
                QueueItem::UpdateWorkflow { id, .. } => id.uid() == item_id,
                QueueItem::UpdateFolder { id, .. } => id.uid() == item_id,
                QueueItem::UpdateEnvVarCollection { id, .. } => id.uid() == item_id,
                QueueItem::UpdateWorkflowEnum { id, .. } => id.uid() == item_id,
                QueueItem::UpdateAIFact { id, .. } => id.uid() == item_id,
                QueueItem::UpdateMCPServer { id, .. } => id.uid() == item_id,
                QueueItem::UpdateAIExecutionProfile { id, .. } => id.uid() == item_id,
                QueueItem::UpdateTemplatableMCPServer { id, .. } => id.uid() == item_id,
                QueueItem::UpdateCloudEnvironment { id, .. } => id.uid() == item_id,
                QueueItem::UpdateScheduledAmbientAgent { id, .. } => id.uid() == item_id,
                QueueItem::UpdateCloudAgentConfig { id, .. } => id.uid() == item_id,
                // We don't depend on object actions, since they don't affect an object's own content or metadata
                QueueItem::RecordObjectAction { .. } => false,
            })
            .map(|(queue_item_id, _)| *queue_item_id)
            .chain(in_flight_dependencies)
            .collect()
    }

    /// Get dependencies for an update request, which should be every create or update request to the same object that is already enqueued
    fn get_update_dependencies(&self, id: &SyncId) -> HashSet<QueueItemId> {
        // Get all objects with the same ID as the update request
        let mut dependencies = self.get_items_with_object_id(id.uid());

        // See if we can convert the update ID to a server/client ID of the other type,
        // since the update request should be dependent on anything that touches the same object
        let converted_id = match id {
            SyncId::ClientId(client_id) => self.client_id_to_server.get(client_id),
            SyncId::ServerId(_) => self.server_id_to_client_hash.get(&id.uid()),
        };

        if let Some(associated_id) = converted_id {
            dependencies.extend(self.get_items_with_object_id(associated_id.clone()));
        }

        dependencies
    }

    /// Get dependencies on objects referenced within any workflow model, and update the model with the server IDs of any referenced objects
    /// that have already been created. Returns an error if there is a referenced object that does not already exist and is not currently waiting
    /// to be created.
    fn get_workflow_object_dependencies(
        &mut self,
        workflow_model: &mut CloudWorkflowModel,
        workflow_id: SyncId,
        item_id: &QueueItemId,
        ctx: &mut ModelContext<Self>,
    ) -> anyhow::Result<HashSet<QueueItemId>> {
        let mut dependencies = HashSet::new();

        let mut object_ids = workflow_model.data.get_enum_ids();
        object_ids.extend(workflow_model.data.default_env_vars());

        // For every object ID referenced in the workflow, see if a server ID already exists. If it does, update the workflow.
        for id in object_ids.into_iter() {
            if let Some(server_id) = self.try_server_id(id) {
                workflow_model
                    .data
                    .replace_object_id(id, SyncId::ServerId(server_id));
            } else {
                // If we don't find any dependencies and we have a client ID, this request should fail immediately
                let queue_items = self.get_items_with_object_id(id.uid());
                if queue_items.is_empty() {
                    self.handle_creation_failure_response(
                        workflow_id.uid(),
                        *item_id,
                        // When adding the initiated_by parameter to this function call, InitiatedBy::User was set as a default value.
                        // It can be changed to propagate initiated_by value from the queue object in the future if desired.
                        InitiatedBy::User,
                        ctx,
                    );
                    return Err(anyhow::anyhow!("No object with this client ID exists"));
                } else {
                    dependencies.extend(queue_items);
                }
            }
        }

        Ok(dependencies)
    }

    fn update_items_with_new_revision(&mut self, server_id: &str, new_revision: Revision) {
        for (_item_id, item) in &mut self.queue {
            match item {
                QueueItem::UpdateNotebook { revision, id, .. } => {
                    Self::maybe_update_queue_item_with_new_revision(
                        &self.client_id_to_server,
                        id,
                        server_id,
                        revision,
                        &new_revision,
                    );
                }
                QueueItem::UpdateWorkflow { id, revision, .. } => {
                    Self::maybe_update_queue_item_with_new_revision(
                        &self.client_id_to_server,
                        id,
                        server_id,
                        revision,
                        &new_revision,
                    );
                }
                // TODO: should we support this for generic string objects?
                _ => {}
            };
        }
    }

    fn maybe_update_queue_item_with_new_revision(
        client_id_to_server: &HashMap<ClientId, String>,
        id: &mut SyncId,
        server_id: &str,
        current_revision: &mut Option<Revision>,
        new_revision: &Revision,
    ) {
        let sync_id = match id {
            SyncId::ClientId(client_id) => client_id_to_server.get(client_id).map(Cow::Borrowed),
            SyncId::ServerId(server_id) => Some(Cow::Owned(server_id.uid())),
        };
        if sync_id.as_ref().map(|id| id.as_str()) == Some(server_id) {
            *current_revision = Some(new_revision.clone())
        }
    }

    /// Dequeue a request from the queue.
    fn dequeue(&mut self, ctx: &mut ModelContext<Self>) {
        // In some cases, we shouldn't dequeue any items, such as when we're offline
        // or when initial load was unsuccessful
        if !self.should_dequeue {
            return;
        }
        let object_client = self.object_client.clone();

        // Find the first update to an object that is not dependent on other queue items.
        if let Some(idx) = self.queue.iter().position(|(item_id, _item)| {
            self.queue_dependencies
                .get(item_id)
                .is_some_and(HashSet::is_empty)
        }) {
            let (dequeued_item_id, dequeued_item) = self.queue.remove(idx);
            self.queue_dependencies.remove(&dequeued_item_id);

            match dequeued_item {
                QueueItem::UpdateNotebook {
                    model,
                    id,
                    revision,
                } => {
                    self.update_object(
                        model.clone(),
                        id,
                        revision,
                        object_client,
                        dequeued_item_id,
                        ctx,
                    );
                }
                QueueItem::UpdateWorkflow {
                    model,
                    id,
                    revision,
                } => {
                    self.update_object(
                        model.clone(),
                        id,
                        revision,
                        object_client,
                        dequeued_item_id,
                        ctx,
                    );
                }
                QueueItem::UpdateFolder { id, model } => {
                    self.update_object(
                        model.clone(),
                        id,
                        None,
                        object_client,
                        dequeued_item_id,
                        ctx,
                    );
                }
                QueueItem::UpdateCloudPreferences {
                    model,
                    id,
                    revision,
                } => {
                    self.update_object(
                        model.clone(),
                        id,
                        revision,
                        object_client,
                        dequeued_item_id,
                        ctx,
                    );
                }
                QueueItem::UpdateEnvVarCollection {
                    model,
                    id,
                    revision,
                } => {
                    self.update_object(
                        model.clone(),
                        id,
                        revision,
                        object_client,
                        dequeued_item_id,
                        ctx,
                    );
                }
                QueueItem::UpdateAIFact {
                    model,
                    id,
                    revision,
                } => {
                    self.update_object(
                        model.clone(),
                        id,
                        revision,
                        object_client,
                        dequeued_item_id,
                        ctx,
                    );
                }
                QueueItem::UpdateAIExecutionProfile {
                    id,
                    model,
                    revision,
                } => {
                    self.update_object(
                        model.clone(),
                        id,
                        revision,
                        object_client,
                        dequeued_item_id,
                        ctx,
                    );
                }
                QueueItem::UpdateWorkflowEnum {
                    model,
                    id,
                    revision,
                } => {
                    self.update_object(
                        model.clone(),
                        id,
                        revision,
                        object_client,
                        dequeued_item_id,
                        ctx,
                    );
                }
                QueueItem::UpdateMCPServer {
                    model,
                    id,
                    revision,
                } => {
                    self.update_object(
                        model.clone(),
                        id,
                        revision,
                        object_client,
                        dequeued_item_id,
                        ctx,
                    );
                }
                QueueItem::UpdateTemplatableMCPServer {
                    model,
                    id,
                    revision,
                } => {
                    self.update_object(
                        model.clone(),
                        id,
                        revision,
                        object_client,
                        dequeued_item_id,
                        ctx,
                    );
                }
                QueueItem::UpdateCloudEnvironment {
                    model,
                    id,
                    revision,
                } => {
                    self.update_object(
                        model.clone(),
                        id,
                        revision,
                        object_client,
                        dequeued_item_id,
                        ctx,
                    );
                }
                QueueItem::UpdateScheduledAmbientAgent {
                    model,
                    id,
                    revision,
                } => {
                    self.update_object(
                        model.clone(),
                        id,
                        revision,
                        object_client,
                        dequeued_item_id,
                        ctx,
                    );
                }
                QueueItem::UpdateCloudAgentConfig {
                    model,
                    id,
                    revision,
                } => {
                    self.update_object(
                        model.clone(),
                        id,
                        revision,
                        object_client,
                        dequeued_item_id,
                        ctx,
                    );
                }
                QueueItem::CreateWorkflow {
                    object_type,
                    owner,
                    id,
                    model,
                    initial_folder_id,
                    entrypoint,
                    initiated_by,
                } => {
                    let serialized_model = Some(model.serialized());

                    self.create_object(
                        object_type,
                        serialized_model,
                        None,
                        owner,
                        id,
                        initial_folder_id,
                        entrypoint,
                        object_client,
                        dequeued_item_id,
                        initiated_by,
                        ctx,
                    );
                }
                QueueItem::CreateObject {
                    object_type,
                    owner,
                    id,
                    title,
                    serialized_model,
                    initial_folder_id,
                    entrypoint,
                    initiated_by,
                } => {
                    let serialized_model = serialized_model
                        .as_ref()
                        .map(|data| SerializedModel(data.model_as_str().to_owned()));

                    self.create_object(
                        object_type,
                        serialized_model,
                        title,
                        owner,
                        id,
                        initial_folder_id,
                        entrypoint,
                        object_client,
                        dequeued_item_id,
                        initiated_by,
                        ctx,
                    );
                }
                QueueItem::BulkCreateGenericStringObjects { owner, objects } => {
                    for data in &objects {
                        self.waiting_response
                            .entry(data.id.to_string())
                            .or_default()
                            .insert(dequeued_item_id);
                    }

                    let mut initial_folder_ids = Vec::new();
                    for object in &objects {
                        let initial_folder_id = match object.initial_folder_id {
                            Some(sync_id) => {
                                if let Some(initial_folder_id) = self.try_server_id(sync_id) {
                                    Some(initial_folder_id)
                                } else {
                                    log::error!("Couldn't find corresponding folder id: skipping");
                                    // Dequeue the next item
                                    self.dequeue(ctx);
                                    return;
                                }
                            }
                            None => None,
                        };
                        initial_folder_ids.push(initial_folder_id);
                    }

                    let bulk_create_request: Arc<Vec<BulkCreateGenericStringObjectsRequest>> =
                        Arc::new(
                            objects
                                .into_iter()
                                .zip(initial_folder_ids)
                                .map(|(data, initial_folder_id)| {
                                    BulkCreateGenericStringObjectsRequest {
                                        id: data.id,
                                        format: data.format,
                                        serialized_model: SerializedModel(
                                            data.serialized_model
                                                .as_ref()
                                                .model_as_str()
                                                .to_owned(),
                                        ),
                                        initial_folder_id: initial_folder_id.map(Into::into),
                                        entrypoint: data.entrypoint,
                                        uniqueness_key: data.uniqueness_key,
                                    }
                                })
                                .collect(),
                        );
                    let bulk_request_clone = bulk_create_request.clone();

                    let future = ctx.spawn_with_retry_on_error(
                        move || {
                            let cloned_server: Arc<dyn ObjectClient> = object_client.clone();
                            let cloned_request = bulk_create_request.clone();
                            async move {
                                cloned_server
                                    .bulk_create_generic_string_objects(owner, &cloned_request)
                                    .await
                            }
                        },
                        *DEFAULT_RETRY_OPTION,
                        move |me, res, ctx| match res {
                            RequestState::RequestSucceeded(result) => {
                                match result {
                                    BulkCreateCloudObjectResult::Success { created_cloud_objects } => {
                                        for created_object in created_cloud_objects {
                                            me.handle_success_response(
                                                &created_object.server_id_and_type.id.uid(),
                                                ResponseType::Creation {
                                                    creation_result: CreationResponseType::Success {
                                                        client_id: created_object.client_id,
                                                        revision_and_editor: created_object
                                                            .revision_and_editor,
                                                        metadata_ts: created_object.metadata_ts,
                                                        server_creation_info: ServerCreationInfo {
                                                            creator_uid: created_object.creator_uid,
                                                            server_id_and_type: created_object.server_id_and_type,
                                                            permissions: created_object.permissions,
                                                        },
                                                    },
                                                },
                                                dequeued_item_id,
                                                // When adding the initiated_by parameter to this function call, InitiatedBy::User was set as a default value.
                                                // initiated_by values currently do not propagate through the sync queue for bulk create operations, but can be added in the future
                                                InitiatedBy::User,
                                                ctx,
                                            );
                                        }
                                    },
                                    BulkCreateCloudObjectResult::GenericStringObjectUniqueKeyConflict => {
                                        log::warn!("Failed to bulk create generic objects because of a unique key conflict");
                                        for object in bulk_request_clone.as_ref().iter() {
                                            // When adding the initiated_by parameter to this function call, InitiatedBy::User was set as a default value.
                                            // initiated_by values currently do not propagate through the sync queue for bulk create operations, but can be added in the future
                                            me.handle_unique_key_creation_failure_response(object.id.to_string(), dequeued_item_id, InitiatedBy::User, ctx);
                                        }
                                        // Continue dequeueing, as this failure doesn't affect
                                        // other objects.
                                        me.dequeue(ctx);
                                    },
                                }
                            }
                            RequestState::RequestFailedRetryPending(e) => {
                                log::warn!(
                                    "Failed to bulk create generic objects {e}, retrying..."
                                );
                            }
                            RequestState::RequestFailed(e) => {
                                log::warn!("Failed to bulk create generic objects {e}");
                                for object in bulk_request_clone.as_ref().iter() {
                                    // When adding the initiated_by parameter to this function call, InitiatedBy::User was set as a default value.
                                    // initiated_by values currently do not propagate through the sync queue for bulk create operations, but can be added in the future
                                    me.handle_creation_failure_response(object.id.to_string(), dequeued_item_id, InitiatedBy::User, ctx);
                                }
                                me.maybe_dequeue_on_error(e, ctx);
                            }
                        },
                    );

                    self.spawned_futures.push(future.future_id());
                }
                QueueItem::RecordObjectAction {
                    id_and_type,
                    action_type,
                    action_timestamp,
                    data,
                } => self.record_object_action(
                    id_and_type.sync_id(),
                    action_type,
                    action_timestamp,
                    data,
                    object_client,
                    dequeued_item_id,
                    ctx,
                ),
            }
        };
    }

    #[allow(clippy::too_many_arguments)]
    fn record_object_action(
        &mut self,
        id: SyncId,
        action_type: ObjectActionType,
        action_timestamp: DateTime<Utc>,
        data: Option<String>,
        object_client: Arc<dyn ObjectClient>,
        queue_item_id: QueueItemId,
        ctx: &mut ModelContext<Self>,
    ) {
        let server_id = match self.try_server_id(id) {
            Some(id) => id,
            None => {
                log::error!("Couldn't find corresponding server id: skipping");
                // Dequeue the next item.
                self.dequeue(ctx);
                return;
            }
        };
        let uid = server_id.uid();

        let future = ctx.spawn_with_retry_on_error(
            move || {
                let object_client_clone = object_client.clone();
                let action_type_clone = action_type.clone();
                let data_clone = data.clone();
                async move {
                    object_client_clone
                        .record_object_action(
                            server_id,
                            action_type_clone,
                            action_timestamp,
                            data_clone,
                        )
                        .await
                }
            },
            *DEFAULT_RETRY_OPTION,
            move |me, res, ctx| match res {
                RequestState::RequestSucceeded(action_history) => {
                    me.handle_success_response(
                        &server_id.uid(),
                        ResponseType::ObjectAction {
                            action_timestamp,
                            action_history,
                        },
                        queue_item_id,
                        // When adding the initiated_by parameter to this function call, InitiatedBy::User was set as a default value.
                        // initiated_by values currently do not propagate through the sync queue for update operations, but can be added in the future
                        InitiatedBy::User,
                        ctx,
                    );
                }
                RequestState::RequestFailedRetryPending(e) => {
                    log::warn!("Failed to record object action: {e}, retrying...");
                }
                RequestState::RequestFailed(e) => {
                    log::warn!("Failed to record object action: {e}");
                    me.handle_object_action_failure_response(
                        uid.clone(),
                        action_timestamp,
                        queue_item_id,
                        ctx,
                    );
                    me.maybe_dequeue_on_error(e, ctx);
                }
            },
        );

        self.spawned_futures.push(future.future_id());
    }

    /// Creates the given object on the server and handles the response.
    #[allow(clippy::too_many_arguments)]
    fn create_object(
        &mut self,
        object_type: ObjectType,
        serialized_model: Option<SerializedModel>,
        title: Option<Arc<String>>,
        owner: Owner,
        id: ClientId,
        initial_folder_id: Option<SyncId>,
        entrypoint: CloudObjectEventEntrypoint,
        object_client: Arc<dyn ObjectClient>,
        queue_item_id: QueueItemId,
        initiated_by: InitiatedBy,
        ctx: &mut ModelContext<Self>,
    ) {
        self.waiting_response
            .entry(id.to_string())
            .or_default()
            .insert(queue_item_id);

        let initial_folder_id = match initial_folder_id {
            Some(sync_id) => {
                if let Some(initial_folder_id) = self.try_server_id(sync_id) {
                    Some(initial_folder_id)
                } else {
                    log::error!("Couldn't find corresponding folder id: skipping");
                    // Dequeue the next item
                    self.dequeue(ctx);
                    return;
                }
            }
            None => None,
        };

        let future = ctx.spawn_with_retry_on_error(
            move || {
                let object_client_clone: Arc<dyn ObjectClient> = object_client.clone();
                let title = title.as_ref().map(|title| title.to_string());
                let create_request = CreateObjectRequest {
                    serialized_model: serialized_model.clone(),
                    title,
                    owner,
                    client_id: id,
                    initial_folder_id: initial_folder_id.map(Into::into),
                    entrypoint,
                };
                async move {
                    match object_type {
                        ObjectType::Notebook => {
                            CloudNotebookModel::send_create_request(
                                object_client_clone,
                                create_request,
                            )
                            .await
                        }
                        ObjectType::Workflow => {
                            CloudWorkflowModel::send_create_request(
                                object_client_clone,
                                create_request,
                            )
                            .await
                        }
                        ObjectType::Folder => {
                            CloudFolderModel::send_create_request(
                                object_client_clone,
                                create_request,
                            )
                            .await
                        }
                        ObjectType::GenericStringObject(GenericStringObjectFormat::Json(
                            json_object_type,
                        )) => match json_object_type {
                            JsonObjectType::Preference => {
                                CloudPreferenceModel::send_create_request(
                                    object_client_clone,
                                    create_request,
                                )
                                .await
                            }
                            JsonObjectType::EnvVarCollection => {
                                CloudEnvVarCollectionModel::send_create_request(
                                    object_client_clone,
                                    create_request,
                                )
                                .await
                            }
                            JsonObjectType::WorkflowEnum => {
                                CloudWorkflowEnumModel::send_create_request(
                                    object_client_clone,
                                    create_request,
                                )
                                .await
                            }
                            JsonObjectType::AIFact => {
                                CloudAIFactModel::send_create_request(
                                    object_client_clone,
                                    create_request,
                                )
                                .await
                            }
                            JsonObjectType::AIExecutionProfile => {
                                CloudAIExecutionProfileModel::send_create_request(
                                    object_client_clone,
                                    create_request,
                                )
                                .await
                            }
                            JsonObjectType::MCPServer => {
                                CloudMCPServerModel::send_create_request(
                                    object_client_clone,
                                    create_request,
                                )
                                .await
                            }
                            JsonObjectType::TemplatableMCPServer => {
                                CloudTemplatableMCPServerModel::send_create_request(
                                    object_client_clone,
                                    create_request,
                                )
                                .await
                            }
                            JsonObjectType::CloudEnvironment => {
                                CloudAmbientAgentEnvironmentModel::send_create_request(
                                    object_client_clone,
                                    create_request,
                                )
                                .await
                            }
                            JsonObjectType::ScheduledAmbientAgent => {
                                CloudScheduledAmbientAgentModel::send_create_request(
                                    object_client_clone,
                                    create_request,
                                )
                                .await
                            }
                            // CloudAgentConfig is not created from the client
                            JsonObjectType::CloudAgentConfig => Err(anyhow::anyhow!(
                                "CloudAgentConfig creation not supported from client"
                            )),
                        },
                    }
                }
            },
            *DEFAULT_RETRY_OPTION,
            move |me, res, ctx| match res {
                RequestState::RequestSucceeded(create_object_result) => {
                    match create_object_result {
                        CreateCloudObjectResult::Success {
                            created_cloud_object,
                        } => {
                            // TODO(alokedesai): Update existing items in the sync queue with
                            // the new revision.

                            // After an object is created, go through any objects dependent on them and update the associated queue items accordingly.
                            me.update_dependencies_on_creation(
                                &queue_item_id,
                                id,
                                created_cloud_object.server_id_and_type.id,
                                object_type,
                            );

                            me.handle_success_response(
                                &created_cloud_object.server_id_and_type.id.uid(),
                                ResponseType::Creation {
                                    creation_result: CreationResponseType::Success {
                                        client_id: id,
                                        revision_and_editor: created_cloud_object
                                            .revision_and_editor,
                                        metadata_ts: created_cloud_object.metadata_ts,
                                        server_creation_info: ServerCreationInfo {
                                            creator_uid: created_cloud_object.creator_uid,
                                            server_id_and_type: created_cloud_object
                                                .server_id_and_type,
                                            permissions: created_cloud_object.permissions,
                                        },
                                    },
                                },
                                queue_item_id,
                                initiated_by,
                                ctx,
                            );
                        }
                        CreateCloudObjectResult::UserFacingError(message) => {
                            me.handle_success_response(
                                &id.to_string(),
                                ResponseType::Creation {
                                    creation_result: CreationResponseType::UserFacingError {
                                        message,
                                        client_id: id,
                                    },
                                },
                                queue_item_id,
                                initiated_by,
                                ctx,
                            );
                        }
                        CreateCloudObjectResult::GenericStringObjectUniqueKeyConflict => {
                            log::warn!(
                                "Failed to create {object_type:?} because of conflicting unique key"
                            );
                            me.handle_unique_key_creation_failure_response(
                                id.to_string(),
                                queue_item_id,
                                initiated_by,
                                ctx,
                            );

                            // Continue dequeueing, as this failure doesn't affect other items.
                            me.dequeue(ctx);
                        }
                    }
                }
                RequestState::RequestFailedRetryPending(e) => {
                    log::warn!("Failed to create {object_type:?} {e}, retrying...");
                }
                RequestState::RequestFailed(e) => {
                    log::warn!("Failed to create {object_type:?} {e}");
                    me.handle_creation_failure_response(
                        id.to_string(),
                        queue_item_id,
                        initiated_by,
                        ctx,
                    );
                    me.maybe_dequeue_on_error(e, ctx);
                }
            },
        );

        self.spawned_futures.push(future.future_id());
    }

    /// Updates the given object on the server and handles the response.
    /// Generic on all model and id types.
    fn update_object<K, M>(
        &mut self,
        model: Arc<
            impl CloudModelType<IdType = K, CloudObjectType = GenericCloudObject<K, M>> + 'static,
        >,
        sync_id: SyncId,
        revision: Option<Revision>,
        object_client: Arc<dyn ObjectClient>,
        queue_item_id: QueueItemId,
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
        M: CloudModelType<IdType = K> + 'static,
    {
        let object_id = match self.try_server_id(sync_id) {
            Some(object_id) => object_id,
            None => {
                log::error!("Couldn't find corresponding object id: skipping");

                // Fail the update
                self.handle_update_failure_response(sync_id, queue_item_id, ctx);

                // Dequeue the next item.
                self.dequeue(ctx);
                return;
            }
        };

        self.waiting_response
            .entry(object_id.uid())
            .or_default()
            .insert(queue_item_id);

        let future = ctx.spawn_with_retry_on_error(
            move || {
                let model_clone = model.clone();
                let revision_clone = revision.clone();
                let object_client_clone = object_client.clone();
                async move {
                    model_clone
                        .send_update_request(object_client_clone, object_id, revision_clone)
                        .await
                }
            },
            *DEFAULT_RETRY_OPTION,
            move |me, res, ctx| match res {
                RequestState::RequestSucceeded(update_result) => match update_result {
                    UpdateCloudObjectResult::Success {
                        revision_and_editor,
                    } => {
                        me.handle_success_response(
                            &object_id.uid(),
                            ResponseType::Update {
                                update_result: UpdateResponseType::Success {
                                    revision_and_editor,
                                },
                            },
                            queue_item_id,
                            // When adding the initiated_by parameter to this function call, InitiatedBy::User was set as a default value.
                            // initiated_by values currently do not propagate through the sync queue for update operations, but can be added in the future
                            InitiatedBy::User,
                            ctx,
                        );
                    }
                    UpdateCloudObjectResult::Rejected { object } => {
                        me.handle_success_response(
                            &object_id.uid(),
                            ResponseType::Update {
                                update_result: UpdateResponseType::Rejected {
                                    object: Box::new((&object).into()),
                                },
                            },
                            queue_item_id,
                            // When adding the initiated_by parameter to this function call, InitiatedBy::User was set as a default value.
                            // initiated_by values currently do not propagate through the sync queue for update operations, but can be added in the future
                            InitiatedBy::User,
                            ctx,
                        );
                    }
                },
                RequestState::RequestFailedRetryPending(_) => {
                    log::warn!("Failed to update object data, retrying...");
                }
                RequestState::RequestFailed(e) => {
                    log::warn!("Failed to update object data {e}");
                    me.handle_update_failure_response(
                        SyncId::ServerId(object_id),
                        queue_item_id,
                        ctx,
                    );
                    me.maybe_dequeue_on_error(e, ctx);
                }
            },
        );

        self.spawned_futures.push(future.future_id());
    }

    fn handle_success_response(
        &mut self,
        uid: &str,
        response_type: ResponseType,
        queue_item_id: QueueItemId,
        initiated_by: InitiatedBy,
        ctx: &mut ModelContext<Self>,
    ) {
        match response_type {
            ResponseType::Update {
                update_result:
                    UpdateResponseType::Success {
                        revision_and_editor,
                    },
            } => {
                if let Some(dependencies) = self.waiting_response.get_mut(uid) {
                    dependencies.remove(&queue_item_id);
                }

                self.handle_dependency_success(&queue_item_id);

                self.update_items_with_new_revision(uid, revision_and_editor.revision.clone());

                ctx.emit(SyncQueueEvent::ObjectUpdateSuccessful {
                    server_id: ServerId::from_string_lossy(uid),
                    revision_and_editor,
                })
            }
            ResponseType::Update {
                update_result: UpdateResponseType::Rejected { object },
            } => {
                let uid = object.uid();
                ctx.emit(SyncQueueEvent::ObjectUpdateRejected {
                    id: uid.clone(),
                    object: Arc::new(*object),
                });
                if let Some(dependencies) = self.waiting_response.get_mut(&uid.clone()) {
                    dependencies.remove(&queue_item_id);
                }
                self.handle_dependency_success(&queue_item_id);
            }
            ResponseType::Creation {
                creation_result:
                    CreationResponseType::Success {
                        client_id,
                        revision_and_editor,
                        metadata_ts,
                        server_creation_info,
                    },
            } => {
                self.client_id_to_server
                    .insert(client_id, server_creation_info.server_id_and_type.id.uid());
                self.server_id_to_client_hash.insert(
                    server_creation_info.server_id_and_type.id.uid(),
                    client_id.to_string(),
                );
                if let Some(dependencies) = self.waiting_response.get_mut(&client_id.to_string()) {
                    dependencies.remove(&queue_item_id);
                }
                self.handle_dependency_success(&queue_item_id);

                self.update_items_with_new_revision(
                    &server_creation_info.server_id_and_type.id.uid(),
                    revision_and_editor.revision.clone(),
                );
                ctx.emit(SyncQueueEvent::ObjectCreationSuccessful {
                    client_id,
                    revision_and_editor,
                    metadata_ts,
                    server_creation_info,
                    initiated_by,
                })
            }
            ResponseType::Creation {
                creation_result: CreationResponseType::UserFacingError { message, client_id },
            } => {
                if let Some(dependencies) = self.waiting_response.get_mut(uid) {
                    dependencies.remove(&queue_item_id);
                }
                self.handle_dependency_success(&queue_item_id);
                ctx.emit(SyncQueueEvent::ObjectCreationFailure {
                    reason: CreationFailureReason::Denied {
                        message,
                        client_id,
                        initiated_by,
                    },
                });
            }
            ResponseType::ObjectAction {
                action_timestamp,
                action_history,
            } => {
                if let Some(dependencies) = self.waiting_response.get_mut(uid) {
                    dependencies.remove(&queue_item_id);
                }
                self.handle_dependency_success(&queue_item_id);
                ctx.emit(SyncQueueEvent::ReportObjectActionSucceeded {
                    uid: uid.to_string(),
                    action_timestamp,
                    action_history,
                })
            }
        }

        self.dequeue(ctx);
    }

    fn handle_creation_failure_response(
        &mut self,
        id: String,
        queue_item_id: QueueItemId,
        initiated_by: InitiatedBy,
        ctx: &mut ModelContext<Self>,
    ) {
        self.handle_creation_failure(&id, queue_item_id, ctx);
        ctx.emit(SyncQueueEvent::ObjectCreationFailure {
            reason: CreationFailureReason::Other { id, initiated_by },
        })
    }

    fn handle_unique_key_creation_failure_response(
        &mut self,
        id: String,
        queue_item_id: QueueItemId,
        initiated_by: InitiatedBy,
        ctx: &mut ModelContext<Self>,
    ) {
        self.handle_creation_failure(&id, queue_item_id, ctx);
        ctx.emit(SyncQueueEvent::ObjectCreationFailure {
            reason: CreationFailureReason::UniqueKeyConflict { id, initiated_by },
        })
    }

    fn handle_creation_failure(
        &mut self,
        id: &String,
        queue_item_id: QueueItemId,
        ctx: &mut ModelContext<Self>,
    ) {
        if let Some(dependencies) = self.waiting_response.get_mut(id) {
            dependencies.remove(&queue_item_id);
        }
        self.handle_dependency_failure(&queue_item_id, ctx);
    }

    fn handle_update_failure_response(
        &mut self,
        id: SyncId,
        queue_item_id: QueueItemId,
        ctx: &mut ModelContext<Self>,
    ) {
        // Determine the final sync ID for the update failure response (since the an object creation request
        // might have come in in the meatime)
        let updated_sync_id = match self.try_server_id(id) {
            Some(server_id) => SyncId::ServerId(server_id),
            None => id,
        };
        if let Some(dependencies) = self.waiting_response.get_mut(&updated_sync_id.uid()) {
            dependencies.remove(&queue_item_id);
        }
        self.handle_dependency_failure(&queue_item_id, ctx);
        ctx.emit(SyncQueueEvent::ObjectUpdateFailure {
            id: updated_sync_id,
        })
    }

    fn handle_object_action_failure_response(
        &mut self,
        uid: ObjectUid,
        timestamp: DateTime<Utc>,
        queue_item_id: QueueItemId,
        ctx: &mut ModelContext<Self>,
    ) {
        if let Some(dependencies) = self.waiting_response.get_mut(&uid) {
            dependencies.remove(&queue_item_id);
        }
        self.handle_dependency_failure(&queue_item_id, ctx);
        ctx.emit(SyncQueueEvent::ReportObjectActionFailed {
            uid,
            action_timestamp: timestamp,
        })
    }

    /// Conditionally dequeue the next item, depending on the reason for failure.
    fn maybe_dequeue_on_error(&mut self, error: anyhow::Error, ctx: &mut ModelContext<Self>) {
        // Check if the underlying cause is a persistent error (i.e. one where we expect other
        // queue items to fail as well).
        // At some point, a generic "is retryable" policy for errors would be useful.
        let persistent_error = error.chain().any(|cause| {
            #[cfg(not(target_family = "wasm"))]
            if let Some(err) = cause.downcast_ref::<reqwest::Error>() {
                // This is adapted from the implementation of `ErrorExt` for `anyhow::Error`.
                // If the server is unavailable, stop dequeueing.
                if err.is_connect() {
                    return true;
                }
            }

            if cause.is::<UserAuthenticationError>() {
                return true;
            }

            if let Some(err) = cause.downcast_ref::<GraphQLError>() {
                match err {
                    // This only applies to WarpDev, but if someone's IP address is blocked, there's no
                    // point in continuing to dequeue.
                    GraphQLError::StagingAccessBlocked => return true,
                    // If the user isn't authorized, stop dequeuing. In general, this should
                    // manifest as a UserAuthenticationError instead.
                    GraphQLError::HttpError {
                        status: StatusCode::FORBIDDEN | StatusCode::UNAUTHORIZED,
                        ..
                    } => return true,
                    _ => (),
                }
            }

            false
        });

        if !persistent_error {
            self.dequeue(ctx);
        }
    }

    /// Given a successful object creation, update any dependent objects to refer to the object's new server ID
    fn update_dependencies_on_creation(
        &mut self,
        queue_item_id: &QueueItemId,
        client_id: ClientId,
        server_id: ServerId,
        object_type: ObjectType,
    ) {
        if let ObjectType::GenericStringObject(GenericStringObjectFormat::Json(
            JsonObjectType::WorkflowEnum | JsonObjectType::EnvVarCollection,
        )) = object_type
        {
            let server_id: GenericStringObjectId = server_id.into();

            for (item_id, queue_item) in self.queue.iter_mut() {
                match queue_item {
                    QueueItem::CreateWorkflow { model, .. } => {
                        // Only update the workflow if it depends on `queue_item_id`
                        if self
                            .queue_dependencies
                            .get(item_id)
                            .is_some_and(|deps| deps.contains(queue_item_id))
                        {
                            let workflow_model = Arc::make_mut(model);
                            workflow_model.data.replace_object_id(
                                SyncId::ClientId(client_id),
                                SyncId::from(server_id),
                            );
                        }
                    }
                    QueueItem::UpdateWorkflow { model, .. } => {
                        // Only update the workflow if it depends on `queue_item_id`
                        if self
                            .queue_dependencies
                            .get(item_id)
                            .is_some_and(|deps| deps.contains(queue_item_id))
                        {
                            let workflow_model = Arc::make_mut(model);
                            workflow_model.data.replace_object_id(
                                SyncId::ClientId(client_id),
                                SyncId::from(server_id),
                            );
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    /// If a request succeeds, update the dependency map by removing it from all dependency sets
    fn handle_dependency_success(&mut self, item_id: &QueueItemId) {
        for (_, dependencies) in self.queue_dependencies.iter_mut() {
            dependencies.remove(item_id);
        }
    }

    /// If a request fails, emit a failure for every dependent QueueItem
    fn handle_dependency_failure(
        &mut self,
        queue_item_id: &QueueItemId,
        ctx: &mut ModelContext<SyncQueue>,
    ) {
        // Filter the queue to get a list of all items dependent on this queue item
        let dependent_objects: Vec<(QueueItemId, QueueItem)> = self
            .queue
            .iter()
            .filter(|(item_id, _)| {
                self.queue_dependencies
                    .get(item_id)
                    .is_some_and(|dependency_set| dependency_set.contains(queue_item_id))
            })
            .cloned()
            .collect();

        // For every dependent queue items, remove from the queue and emit a failure response
        for (item_id, item) in dependent_objects {
            self.remove_id_from_queue(&item_id);

            // Since we're potentially handling chains of dependencies, only propagate a failure response if we haven't done it yet
            if self.queue_dependencies.remove(&item_id).is_none() {
                continue;
            }

            match item {
                QueueItem::CreateObject {
                    id, initiated_by, ..
                } => {
                    self.handle_creation_failure_response(
                        id.to_string(),
                        item_id,
                        initiated_by,
                        ctx,
                    );
                }
                QueueItem::CreateWorkflow {
                    id, initiated_by, ..
                } => {
                    self.handle_creation_failure_response(
                        id.to_string(),
                        item_id,
                        initiated_by,
                        ctx,
                    );
                }
                QueueItem::BulkCreateGenericStringObjects { objects, .. } => {
                    for object in objects.iter() {
                        self.handle_creation_failure_response(
                            object.id.to_string(),
                            item_id,
                            object.initiated_by,
                            ctx,
                        );
                    }
                }
                QueueItem::UpdateWorkflow { id, .. } => {
                    self.handle_update_failure_response(id, item_id, ctx);
                }
                QueueItem::UpdateNotebook { id, .. } => {
                    self.handle_update_failure_response(id, item_id, ctx);
                }
                QueueItem::UpdateFolder { id, .. } => {
                    self.handle_update_failure_response(id, item_id, ctx);
                }
                QueueItem::UpdateCloudPreferences { id, .. } => {
                    self.handle_update_failure_response(id, item_id, ctx);
                }
                QueueItem::UpdateEnvVarCollection { id, .. } => {
                    self.handle_update_failure_response(id, item_id, ctx);
                }
                QueueItem::UpdateWorkflowEnum { id, .. } => {
                    self.handle_update_failure_response(id, item_id, ctx);
                }
                QueueItem::UpdateAIFact { id, .. } => {
                    self.handle_update_failure_response(id, item_id, ctx);
                }
                QueueItem::UpdateMCPServer { id, .. } => {
                    self.handle_update_failure_response(id, item_id, ctx);
                }
                QueueItem::UpdateAIExecutionProfile { id, .. } => {
                    self.handle_update_failure_response(id, item_id, ctx);
                }
                QueueItem::UpdateTemplatableMCPServer { id, .. } => {
                    self.handle_update_failure_response(id, item_id, ctx);
                }
                QueueItem::UpdateCloudEnvironment { id, .. } => {
                    self.handle_update_failure_response(id, item_id, ctx);
                }
                QueueItem::UpdateScheduledAmbientAgent { id, .. } => {
                    self.handle_update_failure_response(id, item_id, ctx);
                }
                QueueItem::UpdateCloudAgentConfig { id, .. } => {
                    self.handle_update_failure_response(id, item_id, ctx);
                }
                QueueItem::RecordObjectAction {
                    id_and_type,
                    action_timestamp,
                    ..
                } => {
                    self.handle_object_action_failure_response(
                        id_and_type.uid(),
                        action_timestamp,
                        item_id,
                        ctx,
                    );
                }
            };
        }
    }

    /// Try converting a SyncID into the server ID. Return None if the item is still
    /// in flight.
    fn try_server_id(&self, id: SyncId) -> Option<ServerId> {
        match id {
            SyncId::ClientId(client_id) => self
                .client_id_to_server
                .get(&client_id)
                .map(ServerId::from_string_lossy),
            SyncId::ServerId(server_id) => Some(server_id),
        }
    }

    #[cfg(test)]
    pub fn queue(&self) -> &Vec<(QueueItemId, QueueItem)> {
        &self.queue
    }

    #[cfg(test)]
    pub fn queue_dependencies(&self) -> &HashMap<QueueItemId, HashSet<QueueItemId>> {
        &self.queue_dependencies
    }

    #[cfg(test)]
    pub fn spawned_futures(&self) -> &Vec<FutureId> {
        &self.spawned_futures
    }
}

impl Entity for SyncQueue {
    type Event = SyncQueueEvent;
}

impl SingletonEntity for SyncQueue {}

#[cfg(test)]
#[path = "sync_queue_tests.rs"]
mod tests;
