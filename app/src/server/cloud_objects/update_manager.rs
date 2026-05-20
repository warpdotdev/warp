#[cfg(not(target_family = "wasm"))]
use crate::ai::mcp::templatable::{CloudTemplatableMCPServerModel, TemplatableMCPServer};
use crate::{
    ai::{
        agent::conversation::AIConversationId,
        ambient_agents::scheduled::{CloudScheduledAmbientAgentModel, ScheduledAmbientAgent},
        blocklist::BlocklistAIHistoryModel,
        cloud_environments::{AmbientAgentEnvironment, CloudAmbientAgentEnvironmentModel},
        execution_profiles::{
            profiles::AIExecutionProfilesModel, AIExecutionProfile, CloudAIExecutionProfileModel,
        },
        facts::{AIFact, CloudAIFactModel},
    },
    auth::{auth_manager::AuthManager, AuthStateProvider},
    cloud_object::{
        model::{
            actions::{ObjectAction, ObjectActionHistory, ObjectActionType, ObjectActions},
            generic_string_model::{
                GenericStringModel, GenericStringObjectId, Serializer, StringModel,
            },
            persistence::{CloudModel, CloudModelEvent, UpdateSource},
            view::{CloudViewModel, Editor, EditorState},
        },
        CloudLinkSharing, CloudModelType, CloudObject, CloudObjectEventEntrypoint,
        CloudObjectLocation, CloudObjectSyncStatus, CreateCloudObjectResult, CreateObjectRequest,
        GenericCloudObject, GenericServerObject, GenericStringObjectFormat, JsonObjectType,
        NumInFlightRequests, ObjectDeleteResult, ObjectIdType, ObjectMetadataUpdateResult,
        ObjectPermissionsUpdateData, ObjectType, Owner, Revision, RevisionAndLastEditor,
        ServerAIExecutionProfile, ServerAIFact, ServerAmbientAgentEnvironment,
        ServerCloudAgentConfig, ServerCloudObject, ServerEnvVarCollection, ServerFolder,
        ServerMCPServer, ServerMetadata, ServerNotebook, ServerObject, ServerPermissions,
        ServerPreference, ServerScheduledAmbientAgent, ServerTemplatableMCPServer, ServerWorkflow,
        ServerWorkflowEnum, Space, UpdateCloudObjectResult,
    },
    drive::{
        folders::{CloudFolderModel, FolderId},
        sharing::SharingAccessLevel,
        CloudObjectTypeAndId,
    },
    env_vars::{CloudEnvVarCollectionModel, EnvVarCollection},
    network::{NetworkStatus, NetworkStatusEvent, NetworkStatusKind},
    notebooks::{CloudNotebookModel, NotebookId},
    persistence::ModelEvent,
    server::{
        ids::{
            parse_sqlite_id_to_uid, ClientId, HashableId, HashedSqliteId, ObjectUid, ServerId,
            SyncId, ToServerId,
        },
        retry_strategies::{
            OUT_OF_BAND_REQUEST_RETRY_STRATEGY, PERIODIC_POLL, PERIODIC_POLL_RETRY_STRATEGY,
        },
        server_api::object::{GuestIdentifier, ObjectClient},
        sync_queue::{
            CreationFailureReason, GenericStringObjectToCreate, QueueItem, SyncQueue,
            SyncQueueEvent,
        },
    },
    settings::cloud_preferences::Preference,
    workflows::{
        workflow::Workflow,
        workflow_enum::{CloudWorkflowEnum, CloudWorkflowEnumModel, WorkflowEnum},
        CloudWorkflowModel, WorkflowId,
    },
    workspaces::{
        team_tester::{TeamTesterStatus, TeamTesterStatusEvent},
        update_manager::TeamUpdateManager,
        user_profiles::{UserProfileWithUID, UserProfiles},
        user_workspaces::UserWorkspaces,
    },
};
use chrono::{DateTime, Utc};
use futures::channel::oneshot::{self, Receiver};
use futures::stream::AbortHandle;
use itertools::Itertools;
use lazy_static::lazy_static;
use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::{mpsc::SyncSender, Arc};
use std::time::Duration;
use warp_core::features::FeatureFlag;
use warp_graphql::mcp_gallery_template::MCPGalleryTemplate;
use warp_graphql::object_permissions::AccessLevel;
use warp_graphql::scalars::time::ServerTimestamp;
use warp_util::sync::Condition;
use warpui::r#async::{FutureId, Timer};
use warpui::{duration_with_jitter, AppContext};
use warpui::{Entity, ModelContext, RequestState, RetryOption, SingletonEntity};

use super::listener::ObjectUpdateMessage;

lazy_static! {
    /// For online-only operations, we want to quickly determine if the operation can succeed,
    /// so that if it can't, we can put the user back into the known good state.
    /// So we try 3 times to prevent any transient failures.
    static ref ONLINE_ONLY_OPERATION_RETRY_STRATEGY: RetryOption =
        RetryOption::exponential(Duration::from_millis(500) /* interval */, 2. /* exponential factor */, 3 /* max retry count */);

    static ref DUPLICATE_OBJECT_NAME_REGEX: Regex = Regex::new(r" \((\d+)\)$").expect("regex should not fail to compile");

}

#[derive(Debug, PartialEq)]
pub enum OperationSuccessType {
    Success,
    Failure,
    Rejection,
    Denied(String),
    FeatureNotAvailable,
}

#[derive(Debug, PartialEq)]
pub enum ObjectOperation {
    Create { initiated_by: InitiatedBy },
    Update,
    MoveToFolder,
    MoveToDrive,
    Trash,
    TakeEditAccess,
    Untrash,
    Delete { initiated_by: InitiatedBy },
    EmptyTrash,
    UpdatePermissions,
    Leave,
}

#[derive(Debug)]
pub struct ObjectOperationResult {
    pub success_type: OperationSuccessType,
    pub operation: ObjectOperation,
    pub client_id: Option<ClientId>,
    pub server_id: Option<ServerId>,
    pub num_objects: Option<i32>, // counts number of objects (including descendants) deleted for permadeletion
}

#[derive(Debug)]
pub enum UpdateManagerEvent {
    ObjectOperationComplete { result: ObjectOperationResult },
    CloudPreferencesUpdated { updated: Vec<Preference> },
    MCPGalleryUpdated { templates: Vec<MCPGalleryTemplate> },
    AmbientTaskUpdated { timestamp: DateTime<Utc> },
}

/// An enum for choosing the behavior of the fetch_single_cloud_object function.
pub enum FetchSingleObjectOption {
    /// Perform the normal upsert behavior.
    None,
    /// Perform the normal upsert behavior, but additionally force overwrite the
    /// in-memory object to whatever the server object is.
    ForceOverwrite,
    /// Only perform the normal upsert behavior if the object doesn't already
    /// exist in-memory.
    IgnoreIfExists,
}

/// An enum that defines whether the action was initiated by the user or the system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitiatedBy {
    User,
    System,
}
#[derive(Default)]
pub struct InitialLoadResponse {
    pub updated_notebooks: Vec<ServerNotebook>,
    pub deleted_notebooks: Vec<NotebookId>,
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

#[derive(Debug)]
pub struct GenericStringObjectInput<T, S>
where
    T: StringModel<
            CloudObjectType = GenericCloudObject<GenericStringObjectId, GenericStringModel<T, S>>,
        > + 'static,
    S: Serializer<T> + 'static,
{
    pub id: ClientId,
    pub model: GenericStringModel<T, S>,
    pub initial_folder_id: Option<SyncId>,
    pub entrypoint: CloudObjectEventEntrypoint,
}

/// The UpdateManager is responsible for delegating work
/// when there is an update to an object (e.g. via a user interaction or
/// a message from the server). Specifically, it will
/// - write to SQLite
/// - interact with the CloudModel to update the in-memory state used by the object views
/// - interact with the SyncQueue by enqueueing an event
pub struct UpdateManager {
    model_event_sender: Option<SyncSender<ModelEvent>>,
    object_client: Arc<dyn ObjectClient>,
    next_poll_abort_handle: Option<AbortHandle>,
    in_flight_request_abort_handle: Option<AbortHandle>,
    should_poll_for_updated_objects: bool,
    spawned_futures: Vec<FutureId>,
    has_initial_load: Condition,
}

impl UpdateManager {
    pub fn new(
        model_event_sender: Option<SyncSender<ModelEvent>>,
        object_client: Arc<dyn ObjectClient>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let network_status = NetworkStatus::handle(ctx);
        ctx.subscribe_to_model(&network_status, |me, event, ctx| {
            me.handle_network_status_changed(event, ctx);
        });

        let team_tester_status = TeamTesterStatus::handle(ctx);
        ctx.subscribe_to_model(&team_tester_status, Self::handle_team_tester_status_changed);

        let sync_queue = SyncQueue::handle(ctx);
        ctx.subscribe_to_model(&sync_queue, |me, event, ctx| {
            me.handle_model_event(event, ctx);
        });

        Self {
            model_event_sender,
            object_client,
            next_poll_abort_handle: None,
            in_flight_request_abort_handle: None,
            should_poll_for_updated_objects: false,
            spawned_futures: Default::default(),
            has_initial_load: Condition::new(),
        }
    }

    #[cfg(test)]
    pub fn mock(ctx: &mut ModelContext<Self>) -> Self {
        use crate::server::server_api::ServerApiProvider;

        Self::new(
            None,
            ServerApiProvider::new_for_test().get_cloud_objects_client(),
            ctx,
        )
    }

    /// Simulate the initial load of changed objects completing.
    #[cfg(test)]
    pub fn mock_initial_load(
        &mut self,
        response: InitialLoadResponse,
        ctx: &mut ModelContext<Self>,
    ) {
        self.on_changed_objects_fetched(response, false /* force_refresh */, ctx);
    }

    #[cfg(any(test, feature = "integration_tests"))]
    pub fn spawned_futures(&self) -> &[FutureId] {
        &self.spawned_futures
    }

    fn save_to_db(&self, events: impl IntoIterator<Item = ModelEvent>) {
        let model_event_sender = self.model_event_sender.clone();
        if let Some(model_event_sender) = &model_event_sender {
            for event in events {
                if let Err(e) = model_event_sender.send(event) {
                    log::error!("Error saving to database: {e:?}");
                }
            }
        }
    }

    /// Remove team-owned objects in response to leaving a team.
    pub fn remove_team_objects(&mut self, left_team_uid: ServerId, ctx: &mut ModelContext<Self>) {
        let cloud_model = CloudModel::handle(ctx);
        let objects_to_remove = cloud_model
            .as_ref(ctx)
            .all_cloud_objects_in_space(
                Space::Team {
                    team_uid: left_team_uid,
                },
                ctx,
            )
            .map(|object| object.cloud_object_type_and_id())
            .collect_vec();

        // First, delete in-memory from CloudModel and object actions.
        cloud_model.update(ctx, |cloud_model, ctx| {
            for object in objects_to_remove.iter() {
                cloud_model.delete_object(object.sync_id(), ctx);
            }
        });
        ObjectActions::handle(ctx).update(ctx, |object_actions, ctx| {
            for object in objects_to_remove.iter() {
                object_actions.delete_actions_for_object(&object.uid(), ctx);
            }
        });

        // Then, delete from SQLite.
        let object_ids_and_types = objects_to_remove
            .into_iter()
            .map(|object| (object.sync_id(), object.object_id_type()))
            .collect();
        self.save_to_db([ModelEvent::DeleteObjects {
            ids: object_ids_and_types,
        }]);
    }

    fn handle_team_tester_status_changed(
        &mut self,
        event: &TeamTesterStatusEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        let TeamTesterStatusEvent::InitiateDataPollers { force_refresh } = event;
        if *force_refresh {
            self.refresh_updated_objects(ctx);
        }

        self.start_polling_for_updated_objects(ctx);
    }

    fn handle_model_event(&mut self, event: &SyncQueueEvent, ctx: &mut ModelContext<Self>) {
        match event {
            SyncQueueEvent::ObjectCreationSuccessful {
                server_creation_info,
                client_id,
                revision_and_editor,
                metadata_ts,
                initiated_by,
            } => {
                let server_id = &server_creation_info.server_id_and_type.id;

                // Update server ID in sqlite.
                self.save_to_db([ModelEvent::UpdateObjectAfterServerCreation {
                    client_id: client_id.sqlite_hash(),
                    server_creation_info: server_creation_info.clone(),
                }]);

                // Update in-memory model.
                CloudModel::handle(ctx).update(ctx, |cloud_model, ctx| {
                    cloud_model.update_object_after_server_creation(
                        *client_id,
                        server_creation_info.clone(),
                        ctx,
                    );
                    if let Some(object) = cloud_model.get_mut_by_uid(&server_id.uid()) {
                        let is_no_longer_in_flight = {
                            let status_if_no_reqs = CloudObjectSyncStatus::NoLocalChanges;
                            object.decrement_in_flight_request_count(status_if_no_reqs)
                        };

                        if is_no_longer_in_flight {
                            // Update sync status in sqlite.

                            self.save_to_db([ModelEvent::MarkObjectAsSynced {
                                hashed_sqlite_id: server_creation_info
                                    .server_id_and_type
                                    .sqlite_type_and_uid_hash(),
                                revision_and_editor: revision_and_editor.clone(),
                                metadata_ts: Some(*metadata_ts),
                            }]);
                        }

                        ctx.notify();
                    }

                    cloud_model.set_latest_revision_and_editor(
                        &server_id.uid(),
                        revision_and_editor.clone(),
                        ctx,
                    );

                    // When an object is created and we get a successful server response, part of marking the object as synced is accepting the
                    // canonical metadata_ts.
                    cloud_model.update_object_metadata_last_updated_ts(
                        &server_id.uid(),
                        *metadata_ts,
                        ctx,
                    );

                    // If we have created a GSO, we need to update the in-memory model for any dependent workflows.
                    // Go through every workflow and try to replace the client ID with the new server ID.
                    if server_creation_info.server_id_and_type.id_type
                        == ObjectIdType::GenericStringObject
                    {
                        let client_id = SyncId::ClientId(*client_id);
                        let server_id = SyncId::ServerId(*server_id);

                        if cloud_model.get_workflow_enum(&server_id).is_some() {
                            cloud_model
                                .get_all_active_and_inactive_workflows_mut()
                                .for_each(|workflow_object| {
                                    let mut workflow = workflow_object.model().clone();
                                    let updated_model =
                                        workflow.data.replace_object_id(client_id, server_id);

                                    // If we changed anything, then update the in-memory model, emit a CloudEvent, and update the DB
                                    if updated_model {
                                        workflow_object.set_model(workflow);

                                        ctx.emit(CloudModelEvent::ObjectUpdated {
                                            type_and_id: workflow_object.cloud_object_type_and_id(),
                                            source: UpdateSource::Local,
                                        });

                                        self.save_to_db([workflow_object.upsert_event()]);
                                    }
                                });
                        } else if cloud_model.get_ai_execution_profile(&server_id).is_some() {
                            AIExecutionProfilesModel::handle(ctx).update(ctx, |model, _| {
                                model.replace_client_id_with_server_id(server_id, client_id);
                            });
                        }
                    }
                });

                // Delete the actions on the client ID. Once we get a server ID for an object, we start dequeuing any pending object actions and those
                // directly populate the ObjectActions model with the server ID, so we don't need to worry about any conversion or anything like that.
                ObjectActions::handle(ctx).update(ctx, |object_actions, ctx| {
                    object_actions.delete_actions_for_object(&client_id.to_string(), ctx);
                });
                self.sync_actions_for_objects_to_sqlite(vec![&client_id.to_string()], ctx);

                ctx.emit(UpdateManagerEvent::ObjectOperationComplete {
                    result: ObjectOperationResult {
                        success_type: OperationSuccessType::Success,
                        operation: ObjectOperation::Create {
                            initiated_by: *initiated_by,
                        },
                        client_id: Some(*client_id),
                        server_id: Some(*server_id),
                        num_objects: None,
                    },
                });
            }
            SyncQueueEvent::ObjectUpdateSuccessful {
                server_id,
                revision_and_editor,
            } => {
                CloudModel::handle(ctx).update(ctx, |cloud_model, ctx| {
                    // Update the object's revision to the latest one from the server
                    cloud_model.set_latest_revision_and_editor(
                        &server_id.uid(),
                        revision_and_editor.clone(),
                        ctx,
                    );
                    // After we update the revision, check if we can now clear the conflicting object
                    cloud_model.check_and_maybe_clear_current_conflict(&server_id.uid(), ctx);

                    // Decrement the object's request count and save it to sqlite if it's sync'd
                    if let Some(object) = cloud_model.get_mut_by_uid(&server_id.uid()) {
                        let is_no_longer_in_flight = {
                            object.decrement_in_flight_request_count(
                                CloudObjectSyncStatus::NoLocalChanges,
                            )
                        };

                        if is_no_longer_in_flight {
                            self.save_to_db([ModelEvent::MarkObjectAsSynced {
                                hashed_sqlite_id: server_id
                                    .sqlite_type_and_uid_hash(object.object_type().into()),
                                revision_and_editor: revision_and_editor.clone(),
                                metadata_ts: None,
                            }]);
                        }

                        ctx.notify();
                    }
                });

                ctx.emit(UpdateManagerEvent::ObjectOperationComplete {
                    result: ObjectOperationResult {
                        success_type: OperationSuccessType::Success,
                        operation: ObjectOperation::Update,
                        client_id: None,
                        server_id: Some(*server_id),
                        num_objects: None,
                    },
                });
            }
            SyncQueueEvent::ObjectCreationFailure {
                reason: CreationFailureReason::UniqueKeyConflict { id, initiated_by },
            } => {
                self.handle_failure_response(id, true, ctx);
                ctx.emit(UpdateManagerEvent::ObjectOperationComplete {
                    result: ObjectOperationResult {
                        success_type: OperationSuccessType::Failure,
                        operation: ObjectOperation::Create {
                            initiated_by: *initiated_by,
                        },
                        client_id: ClientId::from_hash(id),
                        server_id: None,
                        num_objects: None,
                    },
                });
            }
            SyncQueueEvent::ObjectCreationFailure {
                reason: CreationFailureReason::Other { id, initiated_by },
            } => {
                self.handle_failure_response(id, false, ctx);
                ctx.emit(UpdateManagerEvent::ObjectOperationComplete {
                    result: ObjectOperationResult {
                        success_type: OperationSuccessType::Failure,
                        operation: ObjectOperation::Create {
                            initiated_by: *initiated_by,
                        },
                        client_id: ClientId::from_hash(id),
                        server_id: None,
                        num_objects: None,
                    },
                });
            }
            SyncQueueEvent::ObjectCreationFailure {
                reason:
                    CreationFailureReason::Denied {
                        message,
                        client_id,
                        initiated_by,
                    },
            } => {
                self.handle_creation_denied_response(client_id, ctx);
                ctx.emit(UpdateManagerEvent::ObjectOperationComplete {
                    result: ObjectOperationResult {
                        success_type: OperationSuccessType::Denied(message.to_string()),
                        operation: ObjectOperation::Create {
                            initiated_by: *initiated_by,
                        },
                        client_id: Some(*client_id),
                        server_id: None,
                        num_objects: None,
                    },
                });
            }
            SyncQueueEvent::ObjectUpdateFailure { id } => {
                self.handle_failure_response(&id.uid(), false, ctx);
                match id {
                    SyncId::ClientId(id) => ctx.emit(UpdateManagerEvent::ObjectOperationComplete {
                        result: ObjectOperationResult {
                            success_type: OperationSuccessType::Failure,
                            operation: ObjectOperation::Update,
                            client_id: Some(*id),
                            server_id: None,
                            num_objects: None,
                        },
                    }),
                    SyncId::ServerId(id) => ctx.emit(UpdateManagerEvent::ObjectOperationComplete {
                        result: ObjectOperationResult {
                            success_type: OperationSuccessType::Failure,
                            operation: ObjectOperation::Update,
                            client_id: None,
                            server_id: Some(*id),
                            num_objects: None,
                        },
                    }),
                }
            }
            SyncQueueEvent::ObjectUpdateRejected {
                id,
                object: conflicting_object,
            } => {
                self.handle_conflicting_object(conflicting_object, id, ctx);
                ctx.emit(UpdateManagerEvent::ObjectOperationComplete {
                    result: ObjectOperationResult {
                        success_type: OperationSuccessType::Rejection,
                        operation: ObjectOperation::Update,
                        client_id: None,
                        server_id: Some(ServerId::from_string_lossy(id)),
                        num_objects: None,
                    },
                });
            }
            SyncQueueEvent::ObjectUpdateFeatureNotAvailable { id } => {
                ctx.emit(UpdateManagerEvent::ObjectOperationComplete {
                    result: ObjectOperationResult {
                        success_type: OperationSuccessType::FeatureNotAvailable,
                        operation: ObjectOperation::Update,
                        client_id: None,
                        server_id: Some(ServerId::from_string_lossy(id)),
                        num_objects: None,
                    },
                });
            }
            SyncQueueEvent::ReportObjectActionFailed {
                uid,
                action_timestamp,
            } => {
                self.remove_pending_object_action(uid, action_timestamp, ctx);
                self.sync_actions_for_objects_to_sqlite(vec![uid], ctx);
            }
            SyncQueueEvent::ReportObjectActionSucceeded {
                uid,
                action_timestamp,
                action_history,
            } => {
                self.remove_pending_object_action(uid, action_timestamp, ctx);
                self.maybe_overwrite_object_action_history(action_history, ctx);
                self.sync_actions_for_objects_to_sqlite(vec![uid], ctx);
            }
        }
    }

    pub fn resync_object(
        &mut self,
        cloud_object_type_and_id: &CloudObjectTypeAndId,
        ctx: &mut ModelContext<Self>,
    ) {
        CloudModel::handle(ctx).update(ctx, |cloud_model, ctx| {
            if let Some(object) = cloud_model.get_mut_by_uid(&cloud_object_type_and_id.uid()) {
                let queue_item = object
                    .create_object_queue_item(
                        CloudObjectEventEntrypoint::default(),
                        // When adding the initiated_by parameter to this function call, InitiatedBy::User was set as a default value.
                        // This can be changed to InitiatedBy::System if this action was automatically kicked off by the system and we do not want a user facing toast.
                        InitiatedBy::User,
                    )
                    .unwrap_or(object.update_object_queue_item(None));
                object.set_pending_content_changes_status(CloudObjectSyncStatus::InFlight(
                    NumInFlightRequests(1),
                ));
                SyncQueue::handle(ctx).update(ctx, |sync_queue, ctx| {
                    sync_queue.enqueue(queue_item, ctx);
                });
            }
        });
    }

    pub fn start_polling_for_updated_objects(&mut self, ctx: &mut ModelContext<Self>) {
        let is_online = NetworkStatus::as_ref(ctx).is_online();

        if !self.should_poll_for_updated_objects && is_online {
            self.should_poll_for_updated_objects = true;
            self.poll_for_updated_objects(ctx);
        }
    }

    /// Out-of-band (from the regular poll) refresh of updated objects.
    pub fn refresh_updated_objects(&mut self, ctx: &mut ModelContext<Self>) {
        let object_client = self.object_client.clone();
        let cloud_model = CloudModel::as_ref(ctx);
        let versions_for_all_objects = cloud_model.get_versions_for_all_objects(ctx);
        let spawned_handle = ctx.spawn_with_retry_on_error(
            move || {
                let object_client = object_client.clone();
                let cloned_objects_to_update = versions_for_all_objects.clone();
                async move {
                    object_client
                        .fetch_changed_objects(
                            cloned_objects_to_update,
                            false, /* force_refresh */
                        )
                        .await
                }
            },
            OUT_OF_BAND_REQUEST_RETRY_STRATEGY,
            |update_manager, res, ctx| {
                update_manager.handle_fetch_changed_objects_with_request_state(
                    res, false, /* force_refresh */
                    ctx,
                );
            },
        );
        self.spawned_futures.push(spawned_handle.future_id());
    }

    pub fn stop_polling_for_updated_objects(&mut self) {
        self.should_poll_for_updated_objects = false;
        self.abort_existing_poll();
    }

    fn abort_existing_poll(&mut self) {
        if let Some(abort_handle) = self.in_flight_request_abort_handle.take() {
            abort_handle.abort();
        }

        if let Some(abort_handle) = self.next_poll_abort_handle.take() {
            abort_handle.abort();
        }
    }

    fn poll_for_updated_objects(&mut self, ctx: &mut ModelContext<Self>) {
        self.abort_existing_poll();

        if !self.should_poll_for_updated_objects {
            return;
        }

        // Don't poll when the user is logged out to avoid spamming auth errors in the logs.
        // Polling will be restarted when the user logs in via `initiate_data_pollers`.
        if !AuthStateProvider::as_ref(ctx).get().is_logged_in() {
            self.should_poll_for_updated_objects = false;
            return;
        }

        let object_client = self.object_client.clone();
        let cloud_model = CloudModel::as_ref(ctx);
        let versions_for_all_objects = cloud_model.get_versions_for_all_objects(ctx);

        // If there's a force refresh for cloud objects pending, we'll execute the refresh now
        let force_refresh = cloud_model.cloud_objects_force_refresh_pending();
        // We retry a few times here in case there are any transient network errors.
        let spawned_handle = ctx.spawn_with_retry_on_error(
            move || {
                let object_client = object_client.clone();
                let cloned_objects_to_update = versions_for_all_objects.clone();
                async move {
                    object_client
                        .fetch_changed_objects(cloned_objects_to_update, force_refresh)
                        .await
                }
            },
            PERIODIC_POLL_RETRY_STRATEGY,
            move |update_manager, res, ctx| {
                // Only poll if `spawn_with_retry_on_error` is not going to retry again so we don't end up with multiple
                // polls running simultaneously.
                let should_poll_again = !res.has_pending_retries();
                update_manager.handle_fetch_changed_objects_with_request_state(
                    res,
                    force_refresh,
                    ctx,
                );

                if should_poll_again {
                    let next_poll_handle = ctx.spawn(
                        async move {
                            Timer::after(duration_with_jitter(
                                PERIODIC_POLL,
                                0.2, /* max_jitter_multiplier */
                            ))
                            .await
                        },
                        |update_manager, _, ctx| {
                            update_manager.poll_for_updated_objects(ctx);
                        },
                    );
                    update_manager.next_poll_abort_handle = Some(next_poll_handle.abort_handle());
                }
            },
        );

        self.in_flight_request_abort_handle = Some(spawned_handle.abort_handle());
    }

    fn handle_network_status_changed(
        &mut self,
        event: &NetworkStatusEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        match event {
            NetworkStatusEvent::NetworkStatusChanged { new_status } => match new_status {
                NetworkStatusKind::Online => {
                    self.start_polling_for_updated_objects(ctx);
                }
                NetworkStatusKind::Offline => {
                    self.stop_polling_for_updated_objects();
                    SyncQueue::handle(ctx).update(ctx, |queue, _ctx| queue.stop_dequeueing())
                }
            },
        }
    }

    fn handle_fetch_changed_objects_with_request_state(
        &mut self,
        request_state: RequestState<InitialLoadResponse>,
        force_refresh: bool,
        ctx: &mut ModelContext<UpdateManager>,
    ) {
        match request_state {
            RequestState::RequestSucceeded(response) => {
                self.on_changed_objects_fetched(response, force_refresh, ctx);
            }
            RequestState::RequestFailedRetryPending(err) => {
                log::warn!(
                    "fetch_changed_objects: request failed with error {err:#}. Trying again."
                );
            }
            RequestState::RequestFailed(err) => {
                log::warn!(
                    "fetch_changed_objects: request failed with error {err:#}. Retries exhausted."
                );
            }
        }
    }

    fn on_changed_objects_fetched(
        &mut self,
        response: InitialLoadResponse,
        force_refresh: bool,
        ctx: &mut ModelContext<UpdateManager>,
    ) {
        let is_first_load = !self.has_initial_load.is_set();
        let cloud_model = CloudModel::as_ref(ctx);
        // any folder from the server will have its `is_open` model parameter set to false,
        // since the server doesn't know about open/closed states. so in order to not clobber
        // the user's local state, we'll modify any updated folders to match the open state
        // of whatever's in the cloud model (if it exists).
        let folders = response
            .updated_folders
            .into_iter()
            .map(|mut folder| {
                if let Some(cloud_model_folder) = cloud_model.get_folder(&folder.id) {
                    folder.model.is_open = cloud_model_folder.model().is_open;
                }
                folder
            })
            .collect::<Vec<_>>();

        let deleted_notebook_ids = Self::handle_object_deletions(response.deleted_notebooks, ctx);
        let deleted_workflow_ids = Self::handle_object_deletions(response.deleted_workflows, ctx);
        let deleted_folder_ids = Self::handle_object_deletions(response.deleted_folders, ctx);
        let deleted_generic_string_ids =
            Self::handle_object_deletions(response.deleted_generic_string_objects, ctx);

        let deleted_object_ids_and_types: Vec<(SyncId, ObjectIdType)> = deleted_notebook_ids
            .into_iter()
            .map(|id| (id, ObjectIdType::Notebook))
            .chain(
                deleted_workflow_ids
                    .into_iter()
                    .map(|id| (id, ObjectIdType::Workflow)),
            )
            .chain(
                deleted_folder_ids
                    .into_iter()
                    .map(|id| (id, ObjectIdType::Folder)),
            )
            .chain(
                deleted_generic_string_ids
                    .into_iter()
                    .map(|id| (id, ObjectIdType::GenericStringObject)),
            )
            .collect();

        let deleted_hashed_sqlite_object_ids = deleted_object_ids_and_types
            .clone()
            .into_iter()
            .map(|(id, object_id_type)| id.sqlite_uid_hash(object_id_type))
            .collect::<Vec<HashedSqliteId>>();

        let mut sqlite_events = vec![
            Self::handle_object_updates(
                response.updated_notebooks,
                force_refresh,
                !is_first_load,
                ctx,
            ),
            Self::handle_object_updates(
                response.updated_workflows,
                force_refresh,
                !is_first_load,
                ctx,
            ),
            Self::handle_object_updates(folders, force_refresh, !is_first_load, ctx),
            ModelEvent::DeleteObjects {
                ids: deleted_object_ids_and_types,
            },
        ];

        let mut updated_preferences: Vec<Preference> = Vec::new();
        // Handle generic string object updates.
        for (format, objects) in response.updated_generic_string_objects {
            match format {
                GenericStringObjectFormat::Json(JsonObjectType::Preference) => {
                    let typed_objects = objects
                        .iter()
                        .filter_map(|obj| {
                            let server_obj: Option<&ServerPreference> = obj.into();
                            if let Some(server_obj) = server_obj {
                                updated_preferences.push(server_obj.model.string_model.clone());
                            }
                            server_obj.cloned()
                        })
                        .collect::<Vec<_>>();

                    let event = Self::handle_object_updates(
                        typed_objects,
                        force_refresh,
                        !is_first_load,
                        ctx,
                    );
                    sqlite_events.push(event);
                }
                GenericStringObjectFormat::Json(JsonObjectType::EnvVarCollection) => {
                    let typed_objects = objects
                        .iter()
                        .filter_map(|obj| {
                            let server_obj: Option<&ServerEnvVarCollection> = obj.into();
                            server_obj.cloned()
                        })
                        .collect::<Vec<_>>();
                    sqlite_events.push(Self::handle_object_updates(
                        typed_objects,
                        force_refresh,
                        !is_first_load,
                        ctx,
                    ));
                }
                GenericStringObjectFormat::Json(JsonObjectType::WorkflowEnum) => {
                    let typed_objects = objects
                        .iter()
                        .filter_map(|obj| {
                            let server_obj: Option<&ServerWorkflowEnum> = obj.into();
                            server_obj.cloned()
                        })
                        .collect::<Vec<_>>();
                    sqlite_events.push(Self::handle_object_updates(
                        typed_objects,
                        force_refresh,
                        !is_first_load,
                        ctx,
                    ));
                }
                GenericStringObjectFormat::Json(JsonObjectType::AIFact) => {
                    let typed_objects = objects
                        .iter()
                        .filter_map(|obj| {
                            let server_obj: Option<&ServerAIFact> = obj.into();
                            server_obj.cloned()
                        })
                        .collect::<Vec<_>>();
                    sqlite_events.push(Self::handle_object_updates(
                        typed_objects,
                        force_refresh,
                        !is_first_load,
                        ctx,
                    ));
                }
                GenericStringObjectFormat::Json(JsonObjectType::MCPServer) => {
                    let typed_objects = objects
                        .iter()
                        .filter_map(|obj| {
                            let server_obj: Option<&ServerMCPServer> = obj.into();
                            server_obj.cloned()
                        })
                        .collect::<Vec<_>>();
                    sqlite_events.push(Self::handle_object_updates(
                        typed_objects,
                        force_refresh,
                        !is_first_load,
                        ctx,
                    ));
                }
                GenericStringObjectFormat::Json(JsonObjectType::AIExecutionProfile) => {
                    let typed_objects = objects
                        .iter()
                        .filter_map(|obj| {
                            let server_obj: Option<&ServerAIExecutionProfile> = obj.into();
                            server_obj.cloned()
                        })
                        .collect::<Vec<_>>();
                    sqlite_events.push(Self::handle_object_updates(
                        typed_objects,
                        force_refresh,
                        !is_first_load,
                        ctx,
                    ));
                }
                GenericStringObjectFormat::Json(JsonObjectType::TemplatableMCPServer) => {
                    let typed_objects = objects
                        .iter()
                        .filter_map(|obj| {
                            let server_obj: Option<&ServerTemplatableMCPServer> = obj.into();
                            server_obj.cloned()
                        })
                        .collect::<Vec<_>>();
                    sqlite_events.push(Self::handle_object_updates(
                        typed_objects,
                        force_refresh,
                        !is_first_load,
                        ctx,
                    ));
                }
                GenericStringObjectFormat::Json(JsonObjectType::CloudEnvironment) => {
                    let typed_objects = objects
                        .iter()
                        .filter_map(|obj| {
                            let server_obj: Option<&ServerAmbientAgentEnvironment> = obj.into();
                            server_obj.cloned()
                        })
                        .collect::<Vec<_>>();
                    sqlite_events.push(Self::handle_object_updates(
                        typed_objects,
                        force_refresh,
                        !is_first_load,
                        ctx,
                    ));
                }
                GenericStringObjectFormat::Json(JsonObjectType::ScheduledAmbientAgent) => {
                    let typed_objects = objects
                        .iter()
                        .filter_map(|obj| {
                            let server_obj: Option<&ServerScheduledAmbientAgent> = obj.into();
                            server_obj.cloned()
                        })
                        .collect::<Vec<_>>();
                    sqlite_events.push(Self::handle_object_updates(
                        typed_objects,
                        force_refresh,
                        !is_first_load,
                        ctx,
                    ));
                }
                GenericStringObjectFormat::Json(JsonObjectType::CloudAgentConfig) => {
                    let typed_objects = objects
                        .iter()
                        .filter_map(|obj| {
                            let server_obj: Option<&ServerCloudAgentConfig> = obj.into();
                            server_obj.cloned()
                        })
                        .collect::<Vec<_>>();
                    sqlite_events.push(Self::handle_object_updates(
                        typed_objects,
                        force_refresh,
                        !is_first_load,
                        ctx,
                    ));
                }
            }
        }

        // If this is a force refresh, clear out all cached user profiles in memory / in SQLite.
        // This prevents a stale user (e.g. someone who went through user data deletion) from still
        // appearing in the object history of other users on the team.
        if force_refresh {
            UserProfiles::handle(ctx).update(ctx, move |user_profiles_model, _| {
                user_profiles_model.clear_profiles()
            });
            sqlite_events.push(ModelEvent::ClearUserProfiles);
        }

        let user_profiles = UserProfiles::handle(ctx).update(ctx, move |user_profiles_model, _| {
            user_profiles_model.insert_profiles(&response.user_profiles);
            response.user_profiles
        });

        sqlite_events.push(ModelEvent::UpsertUserProfiles {
            profiles: user_profiles,
        });

        // Update the action histories with the new versions sent by the server. Note we collect these as `HashedSqliteId` but them
        // convert them to UIDs because that's what object actions are indexed in in memory.
        let mut ids_with_new_action_histories: Vec<HashedSqliteId> = Vec::new();
        for history in &response.action_histories {
            self.maybe_overwrite_object_action_history(history, ctx);
            ids_with_new_action_histories.push(history.uid.clone());
        }
        ids_with_new_action_histories.extend(deleted_hashed_sqlite_object_ids);
        // Before we pass the ids to the sync actions API, parse them into uids since
        // we store the object actions in memory in cloud model
        self.sync_actions_for_objects_to_sqlite(
            ids_with_new_action_histories
                .into_iter()
                .filter_map(|hashed_id| parse_sqlite_id_to_uid(hashed_id).ok().clone())
                .collect::<Vec<_>>()
                .iter()
                .collect(),
            ctx,
        );

        // If the objects sync was a force refresh, we should set a new time for the next periodic refresh.
        if force_refresh {
            let time_of_next_refresh = CloudModel::handle(ctx).update(ctx, move |model, _| {
                model.mark_cloud_objects_refresh_as_completed()
            });
            sqlite_events.push(ModelEvent::RecordTimeOfNextRefresh {
                timestamp: time_of_next_refresh,
            });
        }

        self.save_to_db(sqlite_events);

        // Sequentially start dequeueing the sync queue after we pull updates from the server.
        SyncQueue::handle(ctx).update(ctx, |queue, ctx| queue.start_dequeueing(ctx));

        self.has_initial_load.set();

        // Only emit InitialLoadCompleted on the very first load after login.
        // Subsequent periodic polls should emit normal per-object events instead
        // of suppressing them and triggering a full rebuild.
        if is_first_load {
            CloudModel::handle(ctx).update(ctx, |_, ctx| {
                ctx.emit(CloudModelEvent::InitialLoadCompleted);
            });
        }

        // Fetch environment "last used" timestamps separately and merge them into the environments.
        // This is done as a separate call because the timestamps come from GetCloudEnvironments query
        // rather than the generic object sync.
        self.fetch_and_merge_environment_timestamps(ctx);

        if !response.mcp_gallery.is_empty() {
            ctx.emit(UpdateManagerEvent::MCPGalleryUpdated {
                templates: response.mcp_gallery,
            });
        }

        if !updated_preferences.is_empty() {
            ctx.emit(UpdateManagerEvent::CloudPreferencesUpdated {
                updated: updated_preferences,
            });
        }
    }

    fn handle_team_memberships_changed(&mut self, ctx: &mut ModelContext<UpdateManager>) {
        // Immediately check for updates in workspace metadata
        TeamUpdateManager::handle(ctx).update(ctx, |manager, ctx| {
            std::mem::drop(manager.refresh_workspace_metadata(ctx));
        });
        self.refresh_updated_objects(ctx);
    }

    fn handle_ambient_task_changed(
        &mut self,
        _task_id: String,
        timestamp: DateTime<Utc>,
        ctx: &mut ModelContext<UpdateManager>,
    ) {
        ctx.emit(UpdateManagerEvent::AmbientTaskUpdated { timestamp });
    }

    /// Fetches environment "last used" timestamps from the server and merges them
    /// into the in-memory environment objects.
    fn fetch_and_merge_environment_timestamps(&mut self, ctx: &mut ModelContext<UpdateManager>) {
        let object_client = self.object_client.clone();
        let future = ctx.spawn(
            async move {
                object_client
                    .fetch_environment_last_task_run_timestamps()
                    .await
            },
            |_update_manager, result, ctx| {
                if let Ok(timestamps) = result {
                    CloudModel::handle(ctx).update(ctx, |cloud_model, ctx| {
                        cloud_model.update_environment_last_task_run_timestamps(timestamps, ctx);
                    });
                } else if let Err(e) = result {
                    log::warn!("Failed to fetch environment last task run timestamps: {e:#}");
                }
            },
        );
        self.spawned_futures.push(future.future_id());
    }

    /// Generic handler updating all objects of a given model type from the server (e.g. all updated/deleted notebooks or workflows).
    /// Updates the CloudModel directly and returns a model event for updating SQLite.
    fn handle_object_updates<K, M>(
        updated_objects: Vec<GenericServerObject<K, M>>,
        force_refresh: bool,
        emit_events: bool,
        ctx: &mut ModelContext<UpdateManager>,
    ) -> ModelEvent
    where
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
        let objects_without_pending_changes =
            CloudModel::handle(ctx).update(ctx, move |model, ctx| {
                model.update_objects_from_initial_load(
                    updated_objects,
                    force_refresh,
                    emit_events,
                    ctx,
                )
            });

        GenericCloudObject::<K, M>::bulk_upsert_event(&objects_without_pending_changes)
    }

    /// Generic handler deleting all objects of a given model type from the server (e.g. all updated/deleted notebooks or workflows).
    /// Updates the CloudModel directly and returns a model event for updating SQLite.
    fn handle_object_deletions<K>(
        deleted_objects: Vec<K>,
        ctx: &mut ModelContext<UpdateManager>,
    ) -> Vec<SyncId>
    where
        K: HashableId
            + ToServerId
            + std::fmt::Debug
            + Into<String>
            + Clone
            + Copy
            + Send
            + Sync
            + 'static,
    {
        let deleted_object_ids = deleted_objects
            .iter()
            .map(|id| SyncId::ServerId((*id).to_server_id()))
            .collect();

        CloudModel::handle(ctx).update(ctx, |cloud_model, ctx| {
            for id in deleted_objects.clone() {
                cloud_model.delete_object(SyncId::ServerId(id.to_server_id()), ctx);
            }
        });

        // Delete actions for this object too.
        let mut ids = vec![];
        ObjectActions::handle(ctx).update(ctx, |object_actions, ctx| {
            for id in deleted_objects {
                let id = SyncId::ServerId(id.to_server_id()).uid();
                object_actions.delete_actions_for_object(&id, ctx);
                ids.push(id.clone());
            }
        });

        deleted_object_ids
    }

    /// Wait for an initial load to complete.
    pub fn initial_load_complete(&self) -> impl Future<Output = ()> {
        // We're not using `async fn` here so that the returned Future doesn't borrow self.
        self.has_initial_load.wait()
    }

    /// Reset the initial-load condition so that subsequent callers of
    /// [`initial_load_complete`](Self::initial_load_complete) will block until
    /// the next load finishes. Call this when the user identity changes (e.g.
    /// after signup/login) to prevent stale cloud data from a previous session
    /// being used.
    pub fn reset_initial_load(&self) {
        log::info!("Resetting initial_load_complete condition for fresh cloud object fetch");
        self.has_initial_load.reset();
    }

    pub fn received_message_from_server(
        &mut self,
        message: ObjectUpdateMessage,
        ctx: &mut ModelContext<Self>,
    ) {
        match message {
            ObjectUpdateMessage::ObjectContentChanged {
                server_object,
                last_editor,
            } => self.handle_cloud_object_changed_event(*server_object, last_editor, ctx),
            ObjectUpdateMessage::ObjectMetadataChanged { metadata } => {
                self.handle_cloud_object_metadata_changed_event(metadata, ctx);
            }
            ObjectUpdateMessage::ObjectPermissionsChanged => {
                // TODO(CLD-2425): Do nothing, this is handled by ObjectPermissionsChangedV2.
            }
            ObjectUpdateMessage::ObjectPermissionsChangedV2 {
                object_uid,
                permissions,
                user_profiles,
            } => {
                self.handle_cloud_object_permissions_changed_v2_event(
                    object_uid,
                    permissions,
                    user_profiles,
                    ctx,
                );
            }
            ObjectUpdateMessage::ObjectDeleted { object_uid } => {
                self.handle_cloud_object_deleted_event(object_uid, ctx);
            }
            ObjectUpdateMessage::ObjectActionOccurred { history } => {
                self.handle_object_action_event(&history, ctx);
            }
            ObjectUpdateMessage::TeamMembershipsChanged => {
                self.handle_team_memberships_changed(ctx);
            }
            ObjectUpdateMessage::AmbientTaskUpdated { task_id, timestamp } => {
                if FeatureFlag::AmbientAgentsRTC.is_enabled() {
                    self.handle_ambient_task_changed(task_id, timestamp, ctx);
                }
            }
        }
    }

    /// Handles an update to a cloud object by updating the in-memory model and sqlite. The update
    /// is split into two parts. (1) If the incoming revision is > in-memory revision (or there is no in-memory revision),
    /// we update the data and fields in metadata that are tied to the revision. (2) If the incoming metadata_ts is >
    /// in-memory metadata_ts, we update the metadata fields that are tied to the metadata ts (current_editor, trashed_ts, etc.)
    fn handle_cloud_object_changed_event(
        &mut self,
        cloud_object: ServerCloudObject,
        last_editor: Option<UserProfileWithUID>,
        ctx: &mut ModelContext<UpdateManager>,
    ) {
        let uid = cloud_object.uid();

        // Update in-memory model
        let mut updated = false;
        CloudModel::handle(ctx).update(ctx, |cloud_model, ctx| {
            if let Some(current_object) = cloud_model.get_by_uid(&uid) {

                // The revision and metadata_ts determine which parts of this update to accept
                let current_object_revision = current_object.metadata().revision.clone();

                // First, check if the incoming revision is greater than the in-memory revision.
                if let Some(in_memory_revision) = current_object_revision {
                    if cloud_object.metadata().revision > in_memory_revision {
                        // Because the object has a greater revision, we should upsert it, essentially meaning overwrite its data.
                        cloud_model.upsert_from_server_cloud_object(cloud_object.clone(), ctx);
                        updated = true;
                    } else {
                        log::info!("in memory revision is greater or equal to metadata from update, ignoring");
                    }
                }
                // Overwrite its relevant metadata fields { trashed_ts, current_editor, etc. } if the ts is greater
                if cloud_model.maybe_update_object_metadata(&uid, cloud_object.metadata().clone(), false, ctx) {
                    updated = true;
                }
            } else {
                // Because the object is new, we should upsert.
                cloud_model.upsert_from_server_cloud_object(cloud_object.clone(), ctx);
                updated = true;
            }

        });

        // Upsert the last editor into the UserProfiles singleton
        if let Some(last_editor_profile) = last_editor {
            let profiles = vec![last_editor_profile];
            UserProfiles::handle(ctx).update(ctx, |user_profiles, _| {
                user_profiles.insert_profiles(&profiles)
            });
            self.save_to_db([ModelEvent::UpsertUserProfiles { profiles }]);
        }

        if !updated {
            return;
        }

        // Update sqlite.
        let cloud_model = CloudModel::as_ref(ctx);
        self.save_in_memory_object_to_sqlite(cloud_model, &uid);

        if let ServerCloudObject::Preference(preference) = &cloud_object {
            let preference = preference.model.string_model.clone();
            ctx.emit(UpdateManagerEvent::CloudPreferencesUpdated {
                updated: vec![preference],
            });
        }
    }

    /// Compare incoming metadata_ts and in_memory metadata_ts to determine whether to accept a new incoming metadata
    /// for a given object. This is a message that pertains just to the fields protected by the metadata ts
    fn handle_cloud_object_metadata_changed_event(
        &mut self,
        new_metadata: ServerMetadata,
        ctx: &mut ModelContext<UpdateManager>,
    ) {
        let uid = new_metadata.uid.uid();

        // Update in-memory model.
        CloudModel::handle(ctx).update(ctx, |cloud_model, ctx| {
            // Overwrite metadata fields. Additionally, check if any important changes have occurred that should
            // trigger a custom UI treatment. For example, changing editors might trigger conflict resolution.
            if cloud_model.maybe_update_object_metadata(&uid.clone(), new_metadata, false, ctx) {
                // Update sqlite.
                if let Some(cloud_object) = cloud_model.get_by_uid(&uid) {
                    let metadata = cloud_object.metadata().clone();
                    let id = cloud_object.cloud_object_type_and_id();
                    let Some(server_id) = id.server_id() else {
                        return;
                    };
                    let hashed_sqlite_id = server_id.sqlite_type_and_uid_hash(id.object_id_type());
                    self.save_to_db([ModelEvent::UpdateObjectMetadata {
                        id: hashed_sqlite_id,
                        metadata,
                    }]);
                }
            }
        });
    }

    fn handle_cloud_object_deleted_event(
        &mut self,
        object_uid: ServerId,
        ctx: &mut ModelContext<UpdateManager>,
    ) {
        self.on_object_delete_success(vec![object_uid.into()], ctx);
        ctx.notify();
    }

    fn handle_object_action_event(
        &mut self,
        history: &ObjectActionHistory,
        ctx: &mut ModelContext<UpdateManager>,
    ) {
        self.maybe_overwrite_object_action_history(history, ctx);
        self.sync_actions_for_objects_to_sqlite(vec![&history.uid], ctx);
    }

    fn save_in_memory_object_to_sqlite(&mut self, cloud_model: &CloudModel, uid: &ObjectUid) {
        if let Some(cloud_object) = cloud_model.get_by_uid(uid) {
            self.save_to_db([cloud_object.upsert_event()]);
        }
    }

    fn save_in_memory_object_metadata_to_sqlite(
        &mut self,
        cloud_model: &CloudModel,
        uid: &ObjectUid,
        hashed_sqlite_id: &str,
    ) {
        if let Some(cloud_object) = cloud_model.get_by_uid(uid) {
            let metadata = cloud_object.metadata().clone();
            let event = ModelEvent::UpdateObjectMetadata {
                id: hashed_sqlite_id.to_string(),
                metadata,
            };
            self.save_to_db([event]);
        }
    }

    /// Save the results of a permissions update (from APIs that use [`ObjectPermissionsUpdateData`]) in-memory and
    /// to SQLite.
    ///
    /// This will overwrite the pending permissions change flag, so should *only* be called with
    /// API responses.
    fn save_permissions_update(
        &self,
        uid: &ObjectUid,
        update: ObjectPermissionsUpdateData,
        ctx: &mut ModelContext<Self>,
    ) {
        // Store the updated permissions in-memory.
        let permissions_upsert = CloudModel::handle(ctx).update(ctx, |cloud_model, ctx| {
            cloud_model.update_object_permissions(
                uid,
                update.permissions,
                UpdateSource::Local,
                ctx,
            );
            cloud_model
                .get_by_uid(uid)
                .map(|object| object.upsert_event())
        });

        // Store any new user profiles in memory.
        UserProfiles::handle(ctx).update(ctx, |user_profiles, _| {
            user_profiles.insert_profiles(&update.profiles);
        });

        let profile_upsert = if update.profiles.is_empty() {
            None
        } else {
            Some(ModelEvent::UpsertUserProfiles {
                profiles: update.profiles,
            })
        };

        let events = permissions_upsert.into_iter().chain(profile_upsert);
        self.save_to_db(events);
    }

    /// Fetches a single cloud object from the server and updates the local model.
    ///
    /// Returns A `Receiver<()>` that completes when the fetch operation is done.
    /// This receiver can be used to wait for the fetch operation to complete before proceeding.
    pub fn fetch_single_cloud_object(
        &mut self,
        server_id: &ServerId,
        fetch_single_object_option: FetchSingleObjectOption,
        ctx: &mut ModelContext<Self>,
    ) -> Receiver<()> {
        let object_client = self.object_client.clone();
        let server_id_copy = *server_id;
        let (fetch_cloud_object_tx, fetch_cloud_object_rx) = oneshot::channel::<()>();
        let future = ctx.spawn(
            async move {
                object_client
                    .fetch_single_cloud_object(server_id_copy)
                    .await
            },
            move |me, cloud_object_result, ctx| match cloud_object_result {
                Ok(result) => {
                    // First, upsert the object and any of its descendents
                    let mut objects = vec![result.object];
                    objects.extend(result.descendants);
                    CloudModel::handle(ctx).update(ctx, |cloud_model, ctx| {
                        for object in objects {
                            let uid = object.uid();
                            let object_is_some = cloud_model.get_by_uid(&uid).is_some();
                            let should_skip = matches!(
                                fetch_single_object_option,
                                FetchSingleObjectOption::IgnoreIfExists
                            ) && object_is_some;

                            if should_skip {
                                continue;
                            }

                            cloud_model.upsert_from_server_cloud_object(object.clone(), ctx);

                            if matches!(
                                fetch_single_object_option,
                                FetchSingleObjectOption::ForceOverwrite
                            ) {
                                if let Some(object) = cloud_model.get_mut_by_uid(&uid) {
                                    let had_conflict = object.has_conflicting_changes();
                                    object.replace_object_with_conflict();
                                    // If there was a conflict, `upsert_from_server_cloud_object` won't
                                    // have emitted an update event. Do it here instead.
                                    if had_conflict {
                                        ctx.emit(CloudModelEvent::ObjectUpdated {
                                            type_and_id: object.cloud_object_type_and_id(),
                                            source: UpdateSource::Server,
                                        });
                                    }
                                }
                            }

                            Self::save_in_memory_object_to_sqlite(
                                me,
                                cloud_model,
                                &server_id_copy.uid(),
                            );
                        }
                        let _ = fetch_cloud_object_tx.send(());
                    });

                    // Second, insert the actions for the object
                    let mut ids_with_new_action_histories: Vec<&HashedSqliteId> = Vec::new();
                    for history in &result.action_histories {
                        me.maybe_overwrite_object_action_history(history, ctx);
                        ids_with_new_action_histories.push(&history.hashed_sqlite_id);
                    }
                    me.sync_actions_for_objects_to_sqlite(
                        ids_with_new_action_histories
                            .iter()
                            .filter_map(|hashed_id| {
                                parse_sqlite_id_to_uid(hashed_id.to_string()).ok()
                            })
                            .collect::<Vec<_>>()
                            .iter()
                            .collect(),
                        ctx,
                    );
                }
                Err(err) => log::error!("error getting cloud object: {err:?}"),
            },
        );

        self.spawned_futures.push(future.future_id());
        fetch_cloud_object_rx
    }

    // Only process the permissions message if the timestamp is newer than the one we have in-memory or we don't
    // have this object in memory. We won't have this object in memory in the case where this
    // object is newly granted.
    //
    // Permissions messages actually can't be out-of-order because the rtc server ignores
    // stale messages, but we could get a message that's staler than info compared to the initial load.
    fn should_ignore_permissions_message(
        &self,
        object_uid: &ObjectUid,
        last_updated_at: ServerTimestamp,
        ctx: &mut ModelContext<UpdateManager>,
    ) -> bool {
        let cloud_model = CloudModel::as_ref(ctx);
        if let Some(object) = cloud_model.get_by_uid(object_uid) {
            if let Some(current_timestamp) = object.permissions().permissions_last_updated_ts {
                if current_timestamp >= last_updated_at {
                    return true;
                }
            }
        }
        false
    }

    fn handle_cloud_object_permissions_changed_v2_event(
        &mut self,
        object_uid: ServerId,
        permissions: ServerPermissions,
        profiles: Vec<UserProfileWithUID>,
        ctx: &mut ModelContext<Self>,
    ) {
        let uid = object_uid.uid();
        if self.should_ignore_permissions_message(
            &uid,
            permissions.permissions_last_updated_ts,
            ctx,
        ) {
            return;
        }

        // The server only sends these messages if the user has access to the object.
        // If the object is already in memory, we update its permissions. If not,
        // assume we were granted access and fetch it.
        let granted_access = CloudModel::handle(ctx).update(ctx, |cloud_model, ctx| {
            let has_object = cloud_model.get_by_uid(&uid).is_some();
            if has_object {
                cloud_model.update_object_permissions(&uid, permissions, UpdateSource::Server, ctx);
                self.save_in_memory_object_to_sqlite(cloud_model, &uid);
            }

            !has_object
        });

        if granted_access {
            // If, between sending this request and receiving a response, we receive another
            // message with the object content, ignore it. This might happen if someone shares and
            // then immediately updates an object.
            let fetch_object_rx = self.fetch_single_cloud_object(
                &object_uid,
                FetchSingleObjectOption::IgnoreIfExists,
                ctx,
            );
            std::mem::drop(fetch_object_rx);
        }

        if !profiles.is_empty() {
            UserProfiles::handle(ctx).update(ctx, |user_profiles, _| {
                user_profiles.insert_profiles(&profiles);
            });
            self.save_to_db([ModelEvent::UpsertUserProfiles { profiles }]);
        }
    }

    fn handle_creation_denied_response(&self, client_id: &ClientId, ctx: &mut ModelContext<Self>) {
        let uid = client_id.to_string();

        let in_personal_drive = CloudModel::handle(ctx).read(ctx, |cloud_model, ctx| {
            cloud_model
                .get_by_uid(&uid)
                .is_none_or(|object| object.space(ctx) == Space::Personal)
        });

        // If not in personal space, move object to personal space and attempt to re-create it.
        if !in_personal_drive {
            // Update in-memory model. Move object to personal space.
            CloudModel::handle(ctx).update(ctx, |cloud_model, ctx| {
                let personal_drive = UserWorkspaces::as_ref(ctx).personal_drive(ctx);
                cloud_model.update_object_location(&uid, personal_drive, None, ctx);
            });

            // Persist changes in sqlite. Moved object to personal space.
            let cloud_model = CloudModel::as_ref(ctx);
            if let Some(cloud_object) = cloud_model.get_by_uid(&uid) {
                self.save_to_db([cloud_object.upsert_event()]);
            }

            // Populate sync queue. Try to re-create the object now that it's in the personal space.
            CloudModel::handle(ctx).update(ctx, |cloud_model, ctx| {
                if let Some(object) = cloud_model.get_mut_by_uid(&uid) {
                    let queue_item = object
                        .create_object_queue_item(
                            CloudObjectEventEntrypoint::default(),
                            // When adding the initiated_by parameter to this function call, InitiatedBy::User was set as a default value.
                            // This can be changed to InitiatedBy::System if this action was automatically kicked off by the system and we do not want a user facing toast.
                            InitiatedBy::User,
                        )
                        .unwrap_or(object.update_object_queue_item(None));
                    SyncQueue::handle(ctx).update(ctx, |sync_queue, ctx| {
                        sync_queue.enqueue(queue_item, ctx);
                    });
                }
            });
        } else {
            self.handle_failure_response(&uid, false, ctx);
        }
    }

    fn handle_failure_response(
        &self,
        uid: &ObjectUid,
        unique_key_creation_conflict: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        let mut hashed_sqlite_id = None;
        if let Some((sync_id, object_type)) =
            CloudModel::handle(ctx).update(ctx, |cloud_model, ctx| {
                if let Some(object) = cloud_model.get_mut_by_uid(uid) {
                    if unique_key_creation_conflict && object.should_clear_on_unique_key_conflict()
                    {
                        return Some((object.sync_id(), object.object_type()));
                    } else {
                        object.decrement_in_flight_request_count(CloudObjectSyncStatus::Errored);
                        hashed_sqlite_id = Some(object.hashed_sqlite_id());
                    }
                }
                ctx.notify();
                None
            })
        {
            CloudModel::handle(ctx).update(ctx, |cloud_model, ctx| {
                log::info!("Removing object {sync_id:?} after unique key conflict");
                cloud_model.delete_object(sync_id, ctx);
                self.save_to_db([ModelEvent::DeleteObjects {
                    ids: vec![(sync_id, object_type.into())],
                }]);
                ctx.notify();
            });
        }

        if let Some(hashed_sqlite_id) = hashed_sqlite_id {
            self.save_to_db([ModelEvent::IncrementRetryCount(hashed_sqlite_id.to_owned())]);
        }
    }

    fn handle_conflicting_object(
        &self,
        conflicting_object: &Arc<ServerCloudObject>,
        uid: &ObjectUid,
        ctx: &mut ModelContext<Self>,
    ) {
        match conflicting_object.as_ref() {
            ServerCloudObject::Notebook(server_notebook) => {
                // Update in-memory model with the fact that it was rejected. We don't update sqlite
                // since we don't want to wipe away the user's content.
                CloudModel::handle(ctx).update(ctx, |cloud_model, ctx| {
                    if let Some(notebook) = cloud_model.get_notebook_mut(&server_notebook.id) {
                        notebook.set_conflicting_object(Arc::new(server_notebook.clone()));

                        // Setting the in-memory model state of the object to in conflict since all further sync
                        // will be rejected until the conflict is cleared. Note that we don't want to clear the pending status
                        // in the database as on the next app restart we want to fetch the up-to-date revision of the object
                        // for refresh in initial load.
                        notebook
                            .set_pending_content_changes_status(CloudObjectSyncStatus::InConflict);

                        ctx.notify();
                    }
                });
            }
            ServerCloudObject::Workflow(workflow) => {
                // we don't have a good UX right now for resolving conflicts, so if the
                // server tells us that a workflow is in conflict, just reset this client's
                // state to whatever the server returned as the source of truth.
                CloudModel::handle(ctx).update(ctx, |cloud_model, ctx| {
                    cloud_model.overwrite_workflow(workflow.clone().model.data, workflow.id, ctx);
                    let workflow_metadata = workflow.clone().metadata;
                    cloud_model.set_latest_revision_and_editor(
                        uid,
                        RevisionAndLastEditor {
                            revision: workflow_metadata.revision,
                            last_editor_uid: workflow_metadata.last_editor_uid,
                        },
                        ctx,
                    );
                    if let Some(object) = cloud_model.get_mut_by_uid(uid) {
                        object.decrement_in_flight_request_count(
                            CloudObjectSyncStatus::NoLocalChanges,
                        );
                        ctx.notify();
                    }
                });

                let cloud_model = CloudModel::as_ref(ctx);
                if let Some(workflow) = cloud_model.get_workflow(&workflow.id) {
                    self.save_to_db([ModelEvent::UpsertWorkflow {
                        workflow: workflow.clone(),
                    }]);
                }
            }
            ServerCloudObject::EnvVarCollection(env_var_collection) => {
                CloudModel::handle(ctx).update(ctx, |cloud_model, ctx| {
                    cloud_model.overwrite_env_var_collection(
                        env_var_collection.clone().model.string_model,
                        env_var_collection.id,
                        ctx,
                    );
                    let env_var_collection_metadata = env_var_collection.clone().metadata;
                    cloud_model.set_latest_revision_and_editor(
                        uid,
                        RevisionAndLastEditor {
                            revision: env_var_collection_metadata.revision,
                            last_editor_uid: env_var_collection_metadata.last_editor_uid,
                        },
                        ctx,
                    );
                    if let Some(object) = cloud_model.get_mut_by_uid(uid) {
                        object.decrement_in_flight_request_count(
                            CloudObjectSyncStatus::NoLocalChanges,
                        );
                        ctx.notify();
                    }
                });

                let cloud_model = CloudModel::as_ref(ctx);
                if let Some(env_var_collection) = cloud_model
                    .get_object_of_type::<GenericStringObjectId, CloudEnvVarCollectionModel>(
                        &env_var_collection.id,
                    )
                {
                    self.save_to_db([ModelEvent::UpsertGenericStringObject {
                        object: Box::new(env_var_collection.clone()),
                    }]);
                }
            }
            ServerCloudObject::WorkflowEnum(workflow_enum) => {
                // Workflow enums exhibit the same behavior as notebooks, workflows, and environment variables on conflict:
                // If we detect a conflict, we reset the client state to the enum that the server returned as the source of truth.
                CloudModel::handle(ctx).update(ctx, |cloud_model, ctx| {
                    cloud_model.overwrite_workflow_enum(
                        workflow_enum.clone().model.string_model,
                        workflow_enum.id,
                        ctx,
                    );
                    let workflow_enum_metadata = workflow_enum.clone().metadata;
                    cloud_model.set_latest_revision_and_editor(
                        uid,
                        RevisionAndLastEditor {
                            revision: workflow_enum_metadata.revision,
                            last_editor_uid: workflow_enum_metadata.last_editor_uid,
                        },
                        ctx,
                    );
                    if let Some(object) = cloud_model.get_mut_by_uid(uid) {
                        object.decrement_in_flight_request_count(
                            CloudObjectSyncStatus::NoLocalChanges,
                        );
                        ctx.notify();
                    }
                });

                let cloud_model = CloudModel::as_ref(ctx);
                if let Some(workflow_enum) = cloud_model
                    .get_object_of_type::<GenericStringObjectId, CloudWorkflowEnumModel>(
                        &workflow_enum.id,
                    )
                {
                    self.save_to_db([ModelEvent::UpsertGenericStringObject {
                        object: Box::new(workflow_enum.clone()),
                    }]);
                }
            }
            ServerCloudObject::AIExecutionProfile(server_profile) => {
                // Update in-memory model with the fact that it was rejected. We don't update sqlite
                // since we don't want to wipe away the user's content.
                CloudModel::handle(ctx).update(ctx, |cloud_model, ctx| {
                    if let Some(profile) = cloud_model.get_object_of_type_mut(&server_profile.id) {
                        profile.set_conflicting_object(Arc::new(server_profile.clone()));

                        // Setting the in-memory model state of the object to in conflict since all further sync
                        // will be rejected until the conflict is cleared. Note that we don't want to clear the pending status
                        // in the database as on the next app restart we want to fetch the up-to-date revision of the object
                        // for refresh in initial load.
                        profile
                            .set_pending_content_changes_status(CloudObjectSyncStatus::InConflict);

                        ctx.notify();
                    }
                });
            }
            // folders and preferences are last-write-wins, no need to do anything here
            // TODO: Figure out how to deal with conflicts for AI rules INT-759
            ServerCloudObject::Folder(_)
            | ServerCloudObject::Preference(_)
            | ServerCloudObject::AIFact(_)
            | ServerCloudObject::MCPServer(_)
            | ServerCloudObject::TemplatableMCPServer(_)
            | ServerCloudObject::AmbientAgentEnvironment(_)
            | ServerCloudObject::ScheduledAmbientAgent(_)
            | ServerCloudObject::CloudAgentConfig(_) => {}
        }
    }

    /// Replace an object's data with the conflicting version from the server. If the object does
    /// not have a conflict, this has no effect.
    pub fn replace_object_with_conflict(&mut self, uid: &ObjectUid, ctx: &mut ModelContext<Self>) {
        let cloud_model_handle = CloudModel::handle(ctx);

        // Update the in-memory model first, and check for conflicts.
        let had_conflicts = cloud_model_handle.update(ctx, |cloud_model, ctx| {
            match cloud_model.get_mut_by_uid(uid) {
                Some(object) if object.has_conflicting_changes() => {
                    object.replace_object_with_conflict();
                    ctx.emit(CloudModelEvent::ObjectUpdated {
                        type_and_id: object.cloud_object_type_and_id(),
                        source: UpdateSource::Server,
                    });
                    true
                }
                _ => false,
            }
        });

        // Update SQLite, but only if the in-memory model was updated.
        if had_conflicts {
            self.save_in_memory_object_to_sqlite(cloud_model_handle.as_ref(ctx), uid);
        }
    }

    pub fn update_ai_fact(
        &mut self,
        ai_fact: AIFact,
        ai_fact_id: SyncId,
        revision_ts: Option<Revision>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.update_object(CloudAIFactModel::new(ai_fact), ai_fact_id, revision_ts, ctx);
    }

    #[cfg(not(target_family = "wasm"))]
    pub fn update_templatable_mcp_server(
        &mut self,
        templatable_mcp_server: TemplatableMCPServer,
        templatable_mcp_server_id: SyncId,
        revision_ts: Option<Revision>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.update_object(
            CloudTemplatableMCPServerModel::new(templatable_mcp_server),
            templatable_mcp_server_id,
            revision_ts,
            ctx,
        );
    }

    pub fn update_workflow(
        &mut self,
        workflow: Workflow,
        workflow_id: SyncId,
        revision_ts: Option<Revision>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.update_object(
            CloudWorkflowModel::new(workflow),
            workflow_id,
            revision_ts,
            ctx,
        );
    }

    pub fn update_workflow_enum(
        &mut self,
        workflow_enum: WorkflowEnum,
        workflow_enum_id: SyncId,
        revision_ts: Option<Revision>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.update_object(
            CloudWorkflowEnumModel::new(workflow_enum),
            workflow_enum_id,
            revision_ts,
            ctx,
        );
    }

    pub fn update_env_var_collection(
        &mut self,
        env_var_collection: EnvVarCollection,
        env_var_collection_id: SyncId,
        revision_ts: Option<Revision>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.update_object(
            CloudEnvVarCollectionModel::new(env_var_collection),
            env_var_collection_id,
            revision_ts,
            ctx,
        );
    }

    pub fn update_ambient_agent_environment(
        &mut self,
        environment: AmbientAgentEnvironment,
        environment_id: SyncId,
        revision_ts: Option<Revision>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.update_object(
            CloudAmbientAgentEnvironmentModel::new(environment),
            environment_id,
            revision_ts,
            ctx,
        );
    }

    pub fn update_notebook_data(
        &mut self,
        data: Arc<String>,
        notebook_id: SyncId,
        ctx: &mut ModelContext<Self>,
    ) {
        let cloud_model = CloudModel::as_ref(ctx);
        let revision = cloud_model.current_revision(&notebook_id).cloned();
        if let Some(notebook) = cloud_model.get_notebook(&notebook_id) {
            let new_notebook = CloudNotebookModel {
                title: notebook.model().title.to_owned(),
                data: data.to_string(),
                ai_document_id: notebook.model().ai_document_id,
                conversation_id: notebook.model().conversation_id.clone(),
            };
            self.update_object(new_notebook, notebook_id, revision, ctx);
        } else {
            log::warn!("Expected notebook to be in model with id {notebook_id:?}");
        }
    }

    pub fn update_notebook_title(
        &mut self,
        title: Arc<String>,
        notebook_id: SyncId,
        ctx: &mut ModelContext<Self>,
    ) {
        let cloud_model = CloudModel::as_ref(ctx);
        let revision = cloud_model.current_revision(&notebook_id).cloned();
        if let Some(notebook) = cloud_model.get_notebook(&notebook_id) {
            let new_notebook = CloudNotebookModel {
                title: title.to_string(),
                data: notebook.model().data.to_owned(),
                ai_document_id: notebook.model().ai_document_id,
                conversation_id: notebook.model().conversation_id.clone(),
            };
            self.update_object(new_notebook, notebook_id, revision, ctx);
        } else {
            log::warn!("Expected notebook to be in model with id {notebook_id:?}");
        }
    }

    /// Attempts to move the object identified by `object_id`
    /// to the folder identified by `folder_id`. If the server accepts
    /// the move, we persist the changes in sqlite. Otherwise, we revert
    /// the optimistic in-memory update we made earlier to indicate that the
    /// move failed.
    #[allow(clippy::too_many_arguments)]
    fn move_object_to_folder(
        &mut self,
        server_id: ServerId,
        object_type: ObjectType,
        owner: Owner,
        destination_folder: Option<FolderId>,
        current_folder: Option<SyncId>,
        current_metadata_last_updated_ts: Option<ServerTimestamp>,
        ctx: &mut ModelContext<Self>,
    ) {
        let object_client = self.object_client.clone();

        CloudModel::handle(ctx).update(ctx, |model, _| {
            if let Some(object) = model.get_mut_by_uid(&server_id.uid()) {
                // Currently, folder moves are considered metadata changes.
                object
                    .metadata_mut()
                    .pending_changes_statuses
                    .has_pending_metadata_change = true;
            }
        });

        let future = ctx.spawn_with_retry_on_error(
            move || {
                let object_client = object_client.clone();
                async move {
                    // TODO: We should use the new folder's owner here, and not require one in the
                    // API.
                    object_client
                        .move_object(server_id, destination_folder, owner, object_type)
                        .await
                }
            },
            *ONLINE_ONLY_OPERATION_RETRY_STRATEGY,
            move |me, res, ctx| match res {
                RequestState::RequestSucceeded(_) => {
                    // Mark the change as completed.
                    CloudModel::handle(ctx).update(ctx, |cloud_model, ctx| {
                        if let Some(obj) = cloud_model.get_mut_by_uid(&server_id.uid()) {
                            obj.metadata_mut()
                                .pending_changes_statuses
                                .has_pending_metadata_change = false;
                        }
                        ctx.notify();
                    });
                    // Persist changes in sqlite.
                    me.save_in_memory_object_to_sqlite(CloudModel::as_ref(ctx), &server_id.uid());
                    ctx.emit(UpdateManagerEvent::ObjectOperationComplete {
                        result: ObjectOperationResult {
                            success_type: OperationSuccessType::Success,
                            operation: ObjectOperation::MoveToFolder,
                            client_id: None,
                            server_id: Some(server_id),
                            num_objects: None,
                        },
                    });
                    ctx.notify();
                }
                RequestState::RequestFailedRetryPending(e) => {
                    log::warn!("Failed to move object to folder: {e}. Retrying");
                }
                RequestState::RequestFailed(e) => {
                    log::warn!("Failed to move object to folder: {e}. Not retrying");
                    // Since the move failed, let's return the object to its original location.
                    // TODO: technically the HTTP request could have failed (e.g. network blip)
                    // but it was actually processed by the server. To remedy this,
                    // we could query the object at this point to get the latest server state.
                    CloudModel::handle(ctx).update(ctx, |cloud_model, ctx| {
                        if let Some(obj) = cloud_model.get_mut_by_uid(&server_id.uid()) {
                            obj.metadata_mut()
                                .pending_changes_statuses
                                .has_pending_metadata_change = false;

                            // Only revert the move if the metadata hasn't changed since we started the move.
                            // If it has (e.g. from an RTC message), that message would have updated the
                            // metadata to the latest server state, so we should not do any further updates here.
                            // Otherwise, let's revert the change we did.
                            let metadata_ts_unchanged = obj.metadata().metadata_last_updated_ts
                                == current_metadata_last_updated_ts;
                            if metadata_ts_unchanged {
                                cloud_model.update_object_location(
                                    &server_id.uid(),
                                    None,
                                    current_folder,
                                    ctx,
                                );
                            }
                            ctx.notify();
                        }
                    });

                    // Show an error toast to relay the failure to the user.
                    ctx.emit(UpdateManagerEvent::ObjectOperationComplete {
                        result: ObjectOperationResult {
                            success_type: OperationSuccessType::Failure,
                            operation: ObjectOperation::MoveToFolder,
                            client_id: None,
                            server_id: Some(server_id),
                            num_objects: None,
                        },
                    });
                    ctx.notify();
                }
            },
        );
        self.spawned_futures.push(future.future_id());
    }

    fn move_object_to_drive_failed(
        server_id: ServerId,
        current_folder: Option<SyncId>,
        current_owner: Owner,
        current_permissions_last_updated_ts: Option<ServerTimestamp>,
        ctx: &mut ModelContext<UpdateManager>,
    ) {
        // Since the move failed, let's return the object to its original location.
        // TODO: technically the HTTP request could have failed (e.g. network blip)
        // but it was actually processed by the server. To remedy this,
        // we could query the object at this point to get the latest server state.
        CloudModel::handle(ctx).update(ctx, |cloud_model, ctx| {
            if let Some(obj) = cloud_model.get_mut_by_uid(&server_id.uid()) {
                obj.metadata_mut()
                    .pending_changes_statuses
                    .has_pending_permissions_change = false;

                // Only revert the move if the metadata hasn't changed since we started the move.
                // If it has (e.g. from an RTC message), that message would have updated the
                // metadata to the latest server state, so we should not do any further updates here.
                // Otherwise, let's revert the change we did.
                let permissions_ts_unchanged = obj.permissions().permissions_last_updated_ts
                    == current_permissions_last_updated_ts;
                if permissions_ts_unchanged {
                    // If the folder is still set to root, let's revert those too
                    // because a space change could have also included a folder change
                    // (e.g. personal folder A -> team space root).
                    cloud_model.update_object_location(
                        &server_id.uid(),
                        Some(current_owner),
                        current_folder,
                        ctx,
                    );
                }
                ctx.notify();
            }
        });

        // Show an error toast to relay the failure to the user.
        ctx.emit(UpdateManagerEvent::ObjectOperationComplete {
            result: ObjectOperationResult {
                success_type: OperationSuccessType::Failure,
                operation: ObjectOperation::MoveToDrive,
                client_id: None,
                server_id: Some(server_id),
                num_objects: None,
            },
        });
        ctx.notify();
    }

    /// Attempts to move the object identified by `object_id`
    /// to the root of the drive identified by `destination_owner`.
    /// If the server accepts  the move, we persist the changes in sqlite.
    /// Otherwise, we revert the optimistic in-memory update we made earlier
    /// to indicate that the move failed.
    #[allow(clippy::too_many_arguments)]
    fn move_object_to_drive(
        &mut self,
        server_id: ServerId,
        object_type: ObjectType,
        destination_owner: Owner,
        current_folder: Option<SyncId>,
        current_owner: Owner,
        current_permissions_last_updated_ts: Option<ServerTimestamp>,
        ctx: &mut ModelContext<Self>,
    ) {
        let object_client = self.object_client.clone();

        // If the moved object is a workflow, we also have to move its the workflow enums to the new space.
        // We do this before moving the workflow to avoid a potential failure state where we've moved a workflow
        // that still references enums in the old space.
        let mut original_workflow = None;
        if object_type == ObjectType::Workflow {
            original_workflow =
                self.copy_workflow_enums_to_drive(server_id, destination_owner, ctx);
        }

        CloudModel::handle(ctx).update(ctx, |model, _| {
            if let Some(object) = model.get_mut_by_uid(&server_id.uid()) {
                object
                    .metadata_mut()
                    .pending_changes_statuses
                    .has_pending_permissions_change = true;
            }
        });

        let future = ctx.spawn_with_retry_on_error(
            move || {
                let object_client = object_client.clone();
                async move {
                    // TODO: to avoid matches like this, we should introduce a `transfer_object_owner` API.
                    match object_type {
                        ObjectType::Notebook => {
                            object_client
                                .transfer_notebook_owner(
                                    NotebookId::from(server_id),
                                    destination_owner,
                                )
                                .await
                        }
                        ObjectType::Workflow => {
                            object_client
                                .transfer_workflow_owner(
                                    WorkflowId::from(server_id),
                                    destination_owner,
                                )
                                .await
                        }
                        ObjectType::GenericStringObject(GenericStringObjectFormat::Json(JsonObjectType::EnvVarCollection)) => {
                            object_client
                                .transfer_generic_string_object_owner(
                                    GenericStringObjectId::from(server_id),
                                    destination_owner,
                                )
                                .await
                        }
                        ObjectType::Folder => {
                            log::info!("Moving a folder to a new space is not supported yet.");
                            Ok(false)
                        }
                        ObjectType::GenericStringObject(GenericStringObjectFormat::Json(
                            JsonObjectType::TemplatableMCPServer,
                        )) => {
                            object_client
                                .transfer_generic_string_object_owner(
                                    GenericStringObjectId::from(server_id),
                                    destination_owner,
                                )
                                .await
                        }
                        ObjectType::GenericStringObject(GenericStringObjectFormat::Json(
                            JsonObjectType::CloudEnvironment,
                        )) => {
                            object_client
                                .transfer_generic_string_object_owner(
                                    GenericStringObjectId::from(server_id),
                                    destination_owner,
                                )
                                .await
                        }
                        ObjectType::GenericStringObject(_) => {
                            log::info!("Moving a generic string object to a new space is not supported yet.");
                            Ok(false)
                        }
                    }
                }
            },
            *ONLINE_ONLY_OPERATION_RETRY_STRATEGY,
            move |me, res, ctx| match res {
                RequestState::RequestSucceeded(success) => {
                    if success {
                        // Mark the change as completed.
                        CloudModel::handle(ctx).update(ctx, |cloud_model, ctx| {
                            if let Some(obj) = cloud_model.get_mut_by_uid(&server_id.uid()) {
                                obj.metadata_mut()
                                    .pending_changes_statuses
                                    .has_pending_permissions_change = false;
                            }
                            ctx.notify();
                        });
                        // Persist changes in sqlite.
                        me.save_in_memory_object_to_sqlite(
                            CloudModel::as_ref(ctx),
                            &server_id.uid(),
                        );
                        ctx.emit(UpdateManagerEvent::ObjectOperationComplete {
                            result: ObjectOperationResult {
                                success_type: OperationSuccessType::Success,
                                operation: ObjectOperation::MoveToDrive,
                                client_id: None,
                                server_id: Some(server_id),
                                num_objects: None,
                            },
                        });
                        ctx.notify();

                    } else {
                        // If the move fails, revert the workflow to use the old enums
                        if let Some(workflow) = original_workflow.take() {
                            me.revert_workflow_on_failed_move(server_id, workflow, ctx);
                        }

                        Self::move_object_to_drive_failed(
                            server_id,
                            current_folder,
                            current_owner,
                            current_permissions_last_updated_ts,
                            ctx,
                        );
                    }
                }
                RequestState::RequestFailedRetryPending(e) => {
                    log::warn!("Failed to move object to space: {e}. Retrying");
                }
                RequestState::RequestFailed(e) => {
                    log::warn!("Failed to move object to space: {e}. Not retrying");
                    // If the move fails, revert the workflow to use the old enums
                    if let Some(workflow) = original_workflow.take() {
                        me.revert_workflow_on_failed_move(server_id, workflow, ctx);
                    }

                    Self::move_object_to_drive_failed(
                        server_id,
                        current_folder,
                        current_owner,
                        current_permissions_last_updated_ts,
                        ctx,
                    );
                }
            },
        );
        self.spawned_futures.push(future.future_id());
    }

    /// Leaves a shared object, removing all of the current user's ACLs on it.
    pub fn leave_object(&mut self, server_id: ServerId, ctx: &mut ModelContext<Self>) {
        let uid = server_id.uid();

        // If there's a pending online-only operation for this object, don't leave it.
        if CloudModel::as_ref(ctx)
            .get_by_uid(&uid)
            .is_none_or(|object| object.metadata().has_pending_online_only_change())
        {
            return;
        }

        let object_client = self.object_client.clone();

        // Make the request.
        let future = ctx.spawn_with_retry_on_error(
            move || {
                let object_client = object_client.clone();
                async move { object_client.leave_object(server_id).await }
            },
            *ONLINE_ONLY_OPERATION_RETRY_STRATEGY,
            move |me, res, ctx| match res {
                RequestState::RequestSucceeded(ObjectDeleteResult::Success { .. }) => {
                    // Remove the object and contents.
                    let deleted_objects =
                        CloudModel::handle(ctx).update(ctx, |cloud_model, ctx| {
                            cloud_model.delete_object_and_descendants(server_id.uid(), ctx)
                        });

                    // Show a confirmation toast.
                    ctx.emit(UpdateManagerEvent::ObjectOperationComplete {
                        result: ObjectOperationResult {
                            success_type: OperationSuccessType::Success,
                            operation: ObjectOperation::Leave,
                            client_id: None,
                            server_id: Some(server_id),
                            num_objects: Some(deleted_objects.len() as i32),
                        },
                    });

                    // Delete object actions as well.
                    ObjectActions::handle(ctx).update(ctx, |object_actions, ctx| {
                        for (id, _) in deleted_objects.iter() {
                            object_actions.delete_actions_for_object(&id.uid(), ctx);
                        }
                    });

                    // Delete objects and their actions from SQLite.
                    me.save_to_db([ModelEvent::DeleteObjects {
                        ids: deleted_objects,
                    }]);
                }
                RequestState::RequestFailedRetryPending(e) => {
                    log::warn!("Failed to leave object: {e}. Retrying.");
                }
                RequestState::RequestFailed(e) => {
                    log::warn!("Failed to leave object: {e}. Not retrying.");
                    ctx.emit(UpdateManagerEvent::ObjectOperationComplete {
                        result: ObjectOperationResult {
                            success_type: OperationSuccessType::Failure,
                            operation: ObjectOperation::Leave,
                            client_id: None,
                            server_id: Some(server_id),
                            num_objects: None,
                        },
                    })
                }
                RequestState::RequestSucceeded(ObjectDeleteResult::Failure) => {
                    ctx.emit(UpdateManagerEvent::ObjectOperationComplete {
                        result: ObjectOperationResult {
                            success_type: OperationSuccessType::Failure,
                            operation: ObjectOperation::Leave,
                            client_id: None,
                            server_id: Some(server_id),
                            num_objects: None,
                        },
                    })
                }
            },
        );
        self.spawned_futures.push(future.future_id());
    }

    /// Sets or removes link sharing permissions for a cloud object.
    ///
    /// This function updates the link sharing permissions of a cloud object identified by `server_id`.
    /// It can either set a new access level or remove the existing permissions (if `access_level` is `None`).
    pub fn set_object_link_permissions(
        &mut self,
        server_id: ServerId,
        access_level: Option<SharingAccessLevel>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.update_permissions_pessimistic(
            server_id,
            ctx,
            move |object_client| async move {
                if let Some(access_level) = access_level {
                    object_client
                        .set_object_link_permissions(server_id, access_level)
                        .await
                } else {
                    object_client
                        .remove_object_link_permissions(server_id)
                        .await
                }
            },
            move |me, _, ctx| {
                let uid = server_id.uid();
                let cloud_model = CloudModel::handle(ctx);
                // Mark the change as completed.
                cloud_model.update(ctx, |cloud_model, ctx| {
                    if let Some(obj) = cloud_model.get_mut_by_uid(&uid) {
                        obj.metadata_mut()
                            .pending_changes_statuses
                            .has_pending_permissions_change = false;
                        obj.permissions_mut().anyone_with_link =
                            access_level.map(|access_level| CloudLinkSharing {
                                access_level,
                                source: None,
                            });
                    }
                    ctx.notify();
                });
                // Persist changes in sqlite.
                me.save_in_memory_object_to_sqlite(cloud_model.as_ref(ctx), &uid);
            },
        );
    }

    /// Add guests to an object.
    pub fn add_object_guests(
        &mut self,
        server_id: ServerId,
        guest_emails: Vec<String>,
        access_level: AccessLevel,
        ctx: &mut ModelContext<Self>,
    ) {
        self.update_permissions_pessimistic(
            server_id,
            ctx,
            move |object_client| {
                let guest_emails = guest_emails.clone();
                async move {
                    object_client
                        .add_object_guests(server_id, guest_emails, access_level)
                        .await
                }
            },
            move |me, data, ctx| {
                let uid = server_id.uid();
                me.save_permissions_update(&uid, data, ctx);
            },
        );
    }

    /// Update the access level for guest(s) on an object.
    pub fn update_object_guests(
        &mut self,
        server_id: ServerId,
        guest_emails: Vec<String>,
        access_level: AccessLevel,
        ctx: &mut ModelContext<Self>,
    ) {
        self.update_permissions_pessimistic(
            server_id,
            ctx,
            move |object_client| {
                let guest_emails = guest_emails.clone();
                async move {
                    object_client
                        .update_object_guests(server_id, guest_emails, access_level)
                        .await
                }
            },
            move |me, permissions, ctx| {
                let uid = server_id.uid();
                me.save_permissions_update(
                    &uid,
                    ObjectPermissionsUpdateData {
                        permissions,
                        profiles: vec![],
                    },
                    ctx,
                );
            },
        );
    }

    /// Remove a guest from an object.
    pub fn remove_object_guest(
        &mut self,
        server_id: ServerId,
        guest: GuestIdentifier,
        ctx: &mut ModelContext<Self>,
    ) {
        self.update_permissions_pessimistic(
            server_id,
            ctx,
            move |object_client| {
                let guest = guest.clone();
                async move { object_client.remove_object_guest(server_id, guest).await }
            },
            move |me, permissions, ctx| {
                let uid = server_id.uid();
                me.save_permissions_update(
                    &uid,
                    ObjectPermissionsUpdateData {
                        permissions,
                        // Fun fact: Vec guarantees that a zero-capacity Vec will not allocate.
                        // https://doc.rust-lang.org/std/vec/struct.Vec.html#guarantees
                        profiles: vec![],
                    },
                    ctx,
                );
            },
        );
    }

    /// Add guests to an AI conversation.
    pub fn add_ai_conversation_guests(
        &mut self,
        server_id: ServerId,
        conversation_id: AIConversationId,
        guest_emails: Vec<String>,
        access_level: AccessLevel,
        ctx: &mut ModelContext<Self>,
    ) {
        self.update_ai_conversation_permissions(
            conversation_id,
            "add guests",
            move |object_client| {
                let guest_emails = guest_emails.clone();
                async move {
                    object_client
                        .add_object_guests(server_id, guest_emails, access_level)
                        .await
                }
            },
            |data, ctx| {
                // Update UserProfiles with any new profiles returned
                if !data.profiles.is_empty() {
                    UserProfiles::handle(ctx).update(ctx, |user_profiles, _| {
                        user_profiles.insert_profiles(&data.profiles);
                    });
                }
                Some(data.permissions)
            },
            ctx,
        );
    }

    /// Update guest access for an AI conversation.
    pub fn update_ai_conversation_guests(
        &mut self,
        server_id: ServerId,
        conversation_id: AIConversationId,
        guest_emails: Vec<String>,
        access_level: AccessLevel,
        ctx: &mut ModelContext<Self>,
    ) {
        self.update_ai_conversation_permissions(
            conversation_id,
            "update guests",
            move |object_client| {
                let guest_emails = guest_emails.clone();
                async move {
                    object_client
                        .update_object_guests(server_id, guest_emails, access_level)
                        .await
                }
            },
            |permissions, _ctx| Some(permissions),
            ctx,
        );
    }

    /// Remove a guest from an AI conversation.
    pub fn remove_ai_conversation_guest(
        &mut self,
        server_id: ServerId,
        conversation_id: AIConversationId,
        guest: GuestIdentifier,
        ctx: &mut ModelContext<Self>,
    ) {
        self.update_ai_conversation_permissions(
            conversation_id,
            "remove guest",
            move |object_client| {
                let guest = guest.clone();
                async move { object_client.remove_object_guest(server_id, guest).await }
            },
            |permissions, _ctx| Some(permissions),
            ctx,
        );
    }

    /// Set or remove link sharing permissions for an AI conversation.
    pub fn set_ai_conversation_link_permissions(
        &mut self,
        server_id: ServerId,
        conversation_id: AIConversationId,
        access_level: Option<SharingAccessLevel>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.update_ai_conversation_permissions(
            conversation_id,
            "set link permissions",
            move |object_client| async move {
                if let Some(access_level) = access_level {
                    object_client
                        .set_object_link_permissions(server_id, access_level)
                        .await
                        .map(|_| ())
                } else {
                    object_client
                        .remove_object_link_permissions(server_id)
                        .await
                        .map(|_| ())
                }
            },
            move |_result, ctx| {
                // For link permissions, we manually construct the permissions update
                // since the API only returns success/failure.
                // Use the unified helper that checks both in-memory and historical metadata.
                let mut permissions = BlocklistAIHistoryModel::as_ref(ctx)
                    .get_server_conversation_metadata(&conversation_id)
                    .map(|metadata| metadata.permissions.clone())?;
                permissions.anyone_link_sharing =
                    access_level.map(|level| crate::cloud_object::ServerLinkSharing {
                        access_level: level.into(),
                        source: None,
                    });
                Some(permissions)
            },
            ctx,
        );
    }

    /// Helper for updating AI conversation permissions.
    ///
    /// This centralizes the common pattern of:
    /// 1. Making an API request
    /// 2. Processing the result to extract permissions
    /// 3. Updating BlocklistAIHistoryModel with new permissions
    /// 4. Emitting ConversationMetadataUpdated event
    fn update_ai_conversation_permissions<P, S, R, F>(
        &mut self,
        conversation_id: AIConversationId,
        operation_name: &'static str,
        mut update_fn: P,
        mut get_updated_permissions: F,
        ctx: &mut ModelContext<Self>,
    ) where
        P: 'static + FnMut(Arc<dyn ObjectClient>) -> S,
        S: warpui::r#async::Spawnable + Future<Output = anyhow::Result<R>>,
        <S as Future>::Output: warpui::r#async::SpawnableOutput,
        F: 'static + FnMut(R, &mut AppContext) -> Option<ServerPermissions>,
    {
        let object_client = self.object_client.clone();

        ctx.spawn_with_retry_on_error(
            move || {
                let object_client = object_client.clone();
                update_fn(object_client)
            },
            *ONLINE_ONLY_OPERATION_RETRY_STRATEGY,
            move |_me, res, ctx| match res {
                RequestState::RequestSucceeded(data) => {
                    if let Some(permissions) = get_updated_permissions(data, ctx) {
                        // Update BlocklistAIHistoryModel (handles both in-memory and historical)
                        BlocklistAIHistoryModel::handle(ctx).update(ctx, |model, ctx| {
                            // Get current metadata from either in-memory conversation or historical
                            let current_metadata = model
                                .get_server_conversation_metadata(&conversation_id)
                                .cloned();
                            if let Some(mut metadata) = current_metadata {
                                metadata.permissions = permissions;
                                model.set_server_metadata_for_conversation(
                                    conversation_id,
                                    metadata,
                                    ctx,
                                );
                            }
                        });
                    }
                }
                RequestState::RequestFailedRetryPending(error) => {
                    log::warn!("Failed to {operation_name} for AI conversation: {error}. Retrying");
                }
                RequestState::RequestFailed(error) => {
                    log::warn!(
                        "Failed to {operation_name} for AI conversation: {error}. Not retrying"
                    );
                }
            },
        );
    }

    /// Helper for implementing *pessimistic* permission changes.
    ///
    /// The overall flow for a pessimistic permission change is:
    /// 1. Short-circuit if there's a pending online-only operation, as any optimistic changes it
    ///    made could be overwritten by this pessimistic update.
    /// 2. Mark the object as having a pending permission change.
    /// 3. Make an API request using `update_fn`
    /// 4. On success, persist the results using `on_success`.
    /// 5. In all cases, emit a completion event and mark the object as no longer having a pending
    ///    permissions change.
    fn update_permissions_pessimistic<M, P, S>(
        &mut self,
        server_id: ServerId,
        ctx: &mut ModelContext<Self>,
        mut update_fn: P,
        mut on_success: impl FnMut(&mut Self, M, &mut ModelContext<Self>) + 'static,
    ) where
        P: 'static + FnMut(Arc<dyn ObjectClient>) -> S,
        S: warpui::r#async::Spawnable + Future<Output = anyhow::Result<M>>,
        <S as Future>::Output: warpui::r#async::SpawnableOutput,
    {
        let cloud_model = CloudModel::handle(ctx);
        let uid = server_id.uid();

        let has_pending_change = cloud_model.update(ctx, |cloud_model, ctx| {
            match cloud_model.get_mut_by_uid(&uid) {
                Some(object) if object.metadata().has_pending_online_only_change() => true,
                Some(object) => {
                    object
                        .metadata_mut()
                        .pending_changes_statuses
                        .has_pending_permissions_change = true;
                    ctx.notify();
                    false
                }
                None => false,
            }
        });

        if has_pending_change {
            log::debug!(
                "Not making permissions change to {server_id} due to pending online-only change"
            );
            return;
        }

        let object_client = self.object_client.clone();
        let future = ctx.spawn_with_retry_on_error(
            move || {
                let object_client = object_client.clone();
                update_fn(object_client)
            },
            *ONLINE_ONLY_OPERATION_RETRY_STRATEGY,
            move |me, res, ctx| match res {
                RequestState::RequestSucceeded(data) => {
                    on_success(me, data, ctx);

                    // Clear the pending-permission-change flag.
                    cloud_model.update(ctx, |cloud_model, _| {
                        if let Some(object) = cloud_model.get_mut_by_uid(&uid) {
                            object
                                .metadata_mut()
                                .pending_changes_statuses
                                .has_pending_permissions_change = false;
                        }
                    });

                    ctx.emit(UpdateManagerEvent::ObjectOperationComplete {
                        result: ObjectOperationResult {
                            success_type: OperationSuccessType::Success,
                            operation: ObjectOperation::UpdatePermissions,
                            client_id: None,
                            server_id: Some(server_id),
                            num_objects: None,
                        },
                    });
                    ctx.notify();
                }
                RequestState::RequestFailedRetryPending(error) => {
                    log::warn!("Failed permissions change: {error}. Retrying");
                }
                RequestState::RequestFailed(error) => {
                    CloudModel::handle(ctx).update(ctx, |cloud_model, ctx| {
                        if let Some(obj) = cloud_model.get_mut_by_uid(&uid) {
                            // Un-mark the pending permissions change. This isn't persisted to SQLite.
                            obj.metadata_mut()
                                .pending_changes_statuses
                                .has_pending_permissions_change = false;
                            ctx.notify()
                        }
                    });

                    ctx.emit(UpdateManagerEvent::ObjectOperationComplete {
                        result: ObjectOperationResult {
                            success_type: OperationSuccessType::Failure,
                            operation: ObjectOperation::UpdatePermissions,
                            client_id: None,
                            server_id: Some(server_id),
                            num_objects: None,
                        },
                    });
                    log::warn!("Failed permissions change: {error}. Not retrying");
                    ctx.notify();
                }
            },
        );
        self.spawned_futures.push(future.future_id());
    }

    /// Given a workflow_id and a destination drive, make a copy of all referenced workflow enums in the destination drive.
    /// Returns the original workflow object if it was modified (in case a future revert is needed), otherwise returns None.
    fn copy_workflow_enums_to_drive(
        &mut self,
        server_id: ServerId,
        owner: Owner,
        ctx: &mut ModelContext<Self>,
    ) -> Option<Workflow> {
        let workflow_id = SyncId::ServerId(server_id);
        let workflow = CloudModel::as_ref(ctx).get_workflow(&workflow_id);

        if let Some(workflow) = workflow {
            let original_workflow = workflow.model().data.clone();
            let mut workflow_model = original_workflow.clone();

            // Duplicate all enums associated with the workflow
            let enums = workflow_model.get_enum_ids();
            for enum_id in enums.iter() {
                let cloud_model = CloudModel::as_ref(ctx);
                let object: Option<&CloudWorkflowEnum> = cloud_model.get_object_of_type(enum_id);
                let Some(object) = object else {
                    log::error!("Could not find referenced workflow enum to copy over to the new space, skipping");
                    continue;
                };

                let client_id = ClientId::new();

                // Create a duplicate enum in the new space with a new client ID
                self.create_object(
                    object.model().clone(),
                    owner,
                    client_id,
                    CloudObjectEventEntrypoint::Unknown,
                    true,
                    None,
                    // When adding the initiated_by parameter to this function call, InitiatedBy::User was set as a default value.
                    // This can be changed to InitiatedBy::System if this action was automatically kicked off by the system and we do not want a user facing toast.
                    InitiatedBy::User,
                    ctx,
                );

                workflow_model.replace_object_id(*enum_id, SyncId::ClientId(client_id));
            }

            // Update the workflow with the new enum IDs, if there are any
            if !enums.is_empty() {
                self.update_workflow(workflow_model, workflow_id, None, ctx);
                Some(original_workflow)
            } else {
                None
            }
        } else {
            log::error!(
                "Tried to move workflow enums to new space but could not find associated workflow",
            );
            None
        }
    }

    /// If an ownership transfer fails, revert the workflow to reference the pre-transition workflow enums
    fn revert_workflow_on_failed_move(
        &mut self,
        server_id: ServerId,
        original_workflow: Workflow,
        ctx: &mut ModelContext<Self>,
    ) {
        let workflow_id = WorkflowId::from(server_id);
        self.update_workflow(
            original_workflow,
            SyncId::ServerId(workflow_id.into()),
            None,
            ctx,
        );
    }

    // This method moves an object from its current location to a new location.
    // Since moving is an online-only operation, this operation does NOT go through the sync queue.
    pub fn move_object_to_location(
        &mut self,
        object_id: CloudObjectTypeAndId,
        new_location: CloudObjectLocation,
        ctx: &mut ModelContext<Self>,
    ) {
        // If we are moving into the trash, we really mean to trash the object
        if let CloudObjectLocation::Trash = new_location {
            return self.trash_object(object_id, ctx);
        }

        // A move operation does not make sense offline,
        // so early return if we don't have a server ID for whatever reason.
        let uid = object_id.uid();
        let Some(server_id) = object_id.server_id() else {
            return;
        };

        let Some((
            object_current_owner,
            object_current_folder,
            object_type,
            has_pending_online_only_change,
            curr_metadata_ts,
            curr_permissions_ts,
        )) = CloudModel::handle(ctx).read(ctx, |model, _| {
            let object = model.get_by_uid(&uid)?;
            Some((
                object.permissions().owner,
                object.metadata().folder_id,
                object.into(),
                object.metadata().has_pending_online_only_change(),
                object.metadata().metadata_last_updated_ts,
                object.permissions().permissions_last_updated_ts,
            ))
        })
        else {
            return;
        };

        // We disallow stacked online-only changes so early return
        // if there's already one pending for this object.
        if has_pending_online_only_change {
            return;
        }

        // Apply a pending, optimistic update and then try to sync the move with the server.
        // We only update the in-memory data but don't persist anything in sqlite until the server confirms the move.
        // Todo: this logic shouldn't need to match based on Space versus Folder. Once we have moving across spaces in MoveObject,
        // we should simplify this to a unified call to move_object that sends the new space AND the new folder.
        let mut not_supported = false;
        match new_location {
            CloudObjectLocation::Space(destination_space) => {
                match UserWorkspaces::as_ref(ctx).space_to_owner(destination_space, ctx) {
                    Some(destination_owner) => {
                        if destination_owner == object_current_owner {
                            // If the space is staying the same, then the move must be to move to the root of the space.
                            CloudModel::handle(ctx).update(ctx, |model, ctx| {
                                model.update_object_location(&uid, None, None, ctx);
                            });
                            self.move_object_to_folder(
                                server_id,
                                object_type,
                                object_current_owner,
                                None,
                                object_current_folder,
                                curr_metadata_ts,
                                ctx,
                            );
                        } else {
                            CloudModel::handle(ctx).update(ctx, |model, ctx| {
                                model.update_object_location(
                                    &uid,
                                    Some(destination_owner),
                                    None,
                                    ctx,
                                );
                            });
                            self.move_object_to_drive(
                                server_id,
                                object_type,
                                destination_owner,
                                object_current_folder,
                                object_current_owner,
                                curr_permissions_ts,
                                ctx,
                            );
                        }
                    }
                    None => {
                        // We couldn't map the space to a valid owner (most likely, it's the
                        // "shared" space).
                        not_supported = true;
                    }
                }
            }
            CloudObjectLocation::Folder(SyncId::ServerId(destination_folder_id)) => {
                // If we're moving across folders, then the space must be staying the same.
                CloudModel::handle(ctx).update(ctx, |model, ctx| {
                    model.update_object_location(
                        &uid,
                        None,
                        Some(SyncId::ServerId(destination_folder_id)),
                        ctx,
                    );
                });
                self.move_object_to_folder(
                    server_id,
                    object_type,
                    object_current_owner,
                    Some(destination_folder_id.into()),
                    object_current_folder,
                    curr_metadata_ts,
                    ctx,
                );
            }
            _ => {
                not_supported = true;
            }
        }

        // In all other cases, just immediately revert the optimistic update since
        // we won't be trying to move the object and we don't want the object to appear
        // as pending.
        if not_supported {
            CloudModel::handle(ctx).update(ctx, |model, ctx| {
                model.update_object_location(
                    &uid,
                    Some(object_current_owner),
                    object_current_folder,
                    ctx,
                );
            });
        }

        ctx.notify();
    }

    pub fn duplicate_object(
        &mut self,
        cloud_object_type_and_id: &CloudObjectTypeAndId,
        ctx: &mut ModelContext<Self>,
    ) {
        match cloud_object_type_and_id {
            CloudObjectTypeAndId::Notebook(notebook_id) => {
                self.duplicate_object_internal::<NotebookId, CloudNotebookModel>(notebook_id, ctx);
            }
            CloudObjectTypeAndId::Workflow(workflow_id) => {
                self.duplicate_object_internal::<WorkflowId, CloudWorkflowModel>(workflow_id, ctx);
            }
            CloudObjectTypeAndId::GenericStringObject { object_type, id } => {
                if let GenericStringObjectFormat::Json(JsonObjectType::EnvVarCollection) =
                    object_type
                {
                    self.duplicate_object_internal::<GenericStringObjectId, CloudEnvVarCollectionModel>(
                        id, ctx,
                    );
                } else {
                    log::error!("Tried to duplicate an unsupported type: json object");
                    debug_assert!(false, "Tried to duplicate an unsupported type: json object");
                }
            }
            CloudObjectTypeAndId::Folder(_) => {
                // Duplicating folders not currently supported.
                log::error!("Tried to duplicate an unsupported type: folder");
                debug_assert!(false, "Tried to duplicate an unsupported type: folder");
            }
        }
    }

    fn duplicate_object_internal<K, M>(&mut self, id: &SyncId, ctx: &mut ModelContext<Self>)
    where
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
        let (duplicate_model, client_id, owner, initial_folder_id, entrypoint) = {
            let cloud_model = CloudModel::as_ref(ctx);
            let object: GenericCloudObject<K, M> = cloud_model
                .get_object_of_type(id)
                .expect("object should exist in order to be duplicated")
                .clone();
            let client_id = ClientId::new();
            let owner = object.permissions.owner;
            let initial_folder_id = object.metadata.folder_id;
            let entrypoint = CloudObjectEventEntrypoint::Unknown;
            let mut duplicate_model = object.model().clone();
            let duplicate_name =
                self.get_next_duplicate_object_name(&object as &dyn CloudObject, cloud_model, ctx);
            duplicate_model.set_display_name(&duplicate_name);
            (
                duplicate_model,
                client_id,
                owner,
                initial_folder_id,
                entrypoint,
            )
        };
        self.create_object(
            duplicate_model,
            owner,
            client_id,
            entrypoint,
            true,
            initial_folder_id,
            // When adding the initiated_by parameter to this function call, InitiatedBy::User was set as a default value.
            // This can be changed to InitiatedBy::System if this action was automatically kicked off by the system and we do not want a user facing toast.
            InitiatedBy::User,
            ctx,
        );
    }

    pub fn create_ai_fact(
        &mut self,
        ai_fact: AIFact,
        client_id: ClientId,
        owner: Owner,
        ctx: &mut ModelContext<Self>,
    ) {
        self.create_object(
            CloudAIFactModel::new(ai_fact),
            owner,
            client_id,
            Default::default(),
            false,
            None,
            // When adding the initiated_by parameter to this function call, InitiatedBy::User was set as a default value.
            // This can be changed to InitiatedBy::System if this action was automatically kicked off by the system and we do not want a user facing toast.
            InitiatedBy::User,
            ctx,
        );
    }

    #[cfg(not(target_family = "wasm"))]
    pub fn create_templatable_mcp_server(
        &mut self,
        templatable_mcp_server: TemplatableMCPServer,
        client_id: ClientId,
        owner: Owner,
        initiated_by: InitiatedBy,
        ctx: &mut ModelContext<Self>,
    ) {
        self.create_object(
            CloudTemplatableMCPServerModel::new(templatable_mcp_server),
            owner,
            client_id,
            Default::default(),
            false,
            None,
            initiated_by,
            ctx,
        );
    }

    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    pub fn create_ambient_agent_environment(
        &mut self,
        ambient_agent_environment: AmbientAgentEnvironment,
        client_id: ClientId,
        owner: Owner,
        ctx: &mut ModelContext<Self>,
    ) {
        self.create_object(
            CloudAmbientAgentEnvironmentModel::new(ambient_agent_environment),
            owner,
            client_id,
            Default::default(),
            false,
            None,
            // When adding the initiated_by parameter to this function call, InitiatedBy::User was set as a default value.
            // This can be changed to InitiatedBy::System if this action was automatically kicked off by the system and we do not want a user facing toast.
            InitiatedBy::User,
            ctx,
        )
    }

    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    pub fn create_ambient_agent_environment_online(
        &mut self,
        ambient_agent_environment: AmbientAgentEnvironment,
        client_id: ClientId,
        owner: Owner,
        ctx: &mut ModelContext<Self>,
    ) -> impl Future<Output = anyhow::Result<ServerId>> {
        self.create_object_online(
            CloudAmbientAgentEnvironmentModel::new(ambient_agent_environment),
            owner,
            client_id,
            Default::default(),
            false,
            None,
            ctx,
        )
    }

    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    pub fn create_scheduled_ambient_agent_online(
        &mut self,
        scheduled_ambient_agent: ScheduledAmbientAgent,
        client_id: ClientId,
        owner: Owner,
        ctx: &mut ModelContext<Self>,
    ) -> impl Future<Output = anyhow::Result<ServerId>> {
        self.create_object_online(
            CloudScheduledAmbientAgentModel::new(scheduled_ambient_agent),
            owner,
            client_id,
            Default::default(),
            false,
            None,
            ctx,
        )
    }

    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    pub fn update_scheduled_ambient_agent_online(
        &mut self,
        scheduled_ambient_agent: ScheduledAmbientAgent,
        scheduled_ambient_agent_id: SyncId,
        revision_ts: Option<Revision>,
        ctx: &mut ModelContext<Self>,
    ) -> impl Future<Output = anyhow::Result<()>> {
        self.update_object_online(
            CloudScheduledAmbientAgentModel::new(scheduled_ambient_agent),
            scheduled_ambient_agent_id,
            revision_ts,
            ctx,
        )
    }

    #[allow(dead_code)]
    pub fn create_ai_execution_profile(
        &mut self,
        ai_execution_profile: AIExecutionProfile,
        client_id: ClientId,
        owner: Owner,
        ctx: &mut ModelContext<Self>,
    ) {
        self.create_object(
            CloudAIExecutionProfileModel::new(ai_execution_profile),
            owner,
            client_id,
            Default::default(),
            false,
            None,
            // When adding the initiated_by parameter to this function call, InitiatedBy::User was set as a default value.
            // This can be changed to InitiatedBy::System if this action was automatically kicked off by the system and we do not want a user facing toast.
            InitiatedBy::User,
            ctx,
        );
    }

    #[allow(dead_code)]
    pub fn update_ai_execution_profile(
        &mut self,
        ai_execution_profile: AIExecutionProfile,
        ai_execution_profile_id: SyncId,
        revision_ts: Option<Revision>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.update_object(
            CloudAIExecutionProfileModel::new(ai_execution_profile),
            ai_execution_profile_id,
            revision_ts,
            ctx,
        );
    }

    pub fn delete_ai_execution_profile(
        &mut self,
        ai_execution_profile_id: SyncId,
        ctx: &mut ModelContext<Self>,
    ) {
        self.delete_object_by_user(
            CloudObjectTypeAndId::GenericStringObject {
                object_type: GenericStringObjectFormat::Json(JsonObjectType::AIExecutionProfile),
                id: ai_execution_profile_id,
            },
            ctx,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_notebook(
        &mut self,
        client_id: ClientId,
        owner: Owner,
        initial_folder_id: Option<SyncId>,
        model: CloudNotebookModel,
        entrypoint: CloudObjectEventEntrypoint,
        force_expand: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        let count = CloudModel::handle(ctx).read(ctx, |model, ctx| {
            model
                .active_non_welcome_notebooks_in_space(Space::Personal, ctx)
                .count()
        });
        if AuthStateProvider::handle(ctx).read(ctx, |auth_state_provider, _ctx| {
            auth_state_provider
                .get()
                .is_anonymous_user_past_object_limit(ObjectType::Notebook, count + 1)
                .unwrap_or_default()
        }) {
            AuthManager::handle(ctx).update(ctx, |auth_manager: &mut AuthManager, ctx| {
                auth_manager.anonymous_user_hit_drive_object_limit(ctx);
            });
            return;
        };

        self.create_object(
            model,
            owner,
            client_id,
            entrypoint,
            force_expand,
            initial_folder_id,
            // When adding the initiated_by parameter to this function call, InitiatedBy::User was set as a default value.
            // This can be changed to InitiatedBy::System if this action was automatically kicked off by the system and we do not want a user facing toast.
            InitiatedBy::User,
            ctx,
        );
    }

    fn get_next_duplicate_object_name(
        &self,
        original_cloud_object: &dyn CloudObject,
        cloud_model: &CloudModel,
        app: &AppContext,
    ) -> String {
        let original_name = original_cloud_object.display_name();

        // Iterate through items in the same folder as the original object that are of the
        // same type, and populate a hashset with those names.
        let same_type_and_folder_names = cloud_model
            .active_cloud_objects_in_location_without_descendents(
                original_cloud_object.location(cloud_model, app),
                app,
            )
            .filter(|&object| object.object_type() == original_cloud_object.object_type())
            .map(|object| object.display_name())
            .collect::<HashSet<String>>();

        // Start with "{original_object_name} ({original_object_name's count + 1})".
        // Keep incrementing by one if there already exists an object of the same type in
        // the same folder (using the hashset generated above).
        let mut duplicate_name = get_duplicate_object_name(&original_name);
        while same_type_and_folder_names.contains(&duplicate_name) {
            duplicate_name = get_duplicate_object_name(&duplicate_name);
        }
        duplicate_name
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_workflow(
        &mut self,
        workflow: Workflow,
        owner: Owner,
        initial_folder_id: Option<SyncId>,
        client_id: ClientId,
        entrypoint: CloudObjectEventEntrypoint,
        force_expand: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        let count = CloudModel::handle(ctx).read(ctx, |model, ctx| {
            model
                .active_non_welcome_workflows_in_space(Space::Personal, ctx)
                .count()
        });
        if AuthStateProvider::handle(ctx).read(ctx, |auth_state_provider, _ctx| {
            auth_state_provider
                .get()
                .is_anonymous_user_past_object_limit(ObjectType::Workflow, count + 1)
                .unwrap_or_default()
        }) {
            AuthManager::handle(ctx).update(ctx, |auth_manager: &mut AuthManager, ctx| {
                auth_manager.anonymous_user_hit_drive_object_limit(ctx);
            });
            return;
        };

        self.create_object(
            CloudWorkflowModel::new(workflow),
            owner,
            client_id,
            entrypoint,
            force_expand,
            initial_folder_id,
            // When adding the initiated_by parameter to this function call, InitiatedBy::User was set as a default value.
            // This can be changed to InitiatedBy::System if this action was automatically kicked off by the system and we do not want a user facing toast.
            InitiatedBy::User,
            ctx,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_workflow_enum(
        &mut self,
        workflow_enum: WorkflowEnum,
        owner: Owner,
        client_id: ClientId,
        entrypoint: CloudObjectEventEntrypoint,
        force_expand: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        self.create_object(
            CloudWorkflowEnumModel::new(workflow_enum),
            owner,
            client_id,
            entrypoint,
            force_expand,
            None,
            // When adding the initiated_by parameter to this function call, InitiatedBy::User was set as a default value.
            // This can be changed to InitiatedBy::System if this action was automatically kicked off by the system and we do not want a user facing toast.
            InitiatedBy::User,
            ctx,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_env_var_collection(
        &mut self,
        client_id: ClientId,
        owner: Owner,
        initial_folder_id: Option<SyncId>,
        model: CloudEnvVarCollectionModel,
        entrypoint: CloudObjectEventEntrypoint,
        force_expand: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        let count = CloudModel::handle(ctx).read(ctx, |model, ctx| {
            model
                .active_non_welcome_env_var_collections_in_space(Space::Personal, ctx)
                .count()
        });
        let env_var_collection_type = ObjectType::GenericStringObject(
            GenericStringObjectFormat::Json(JsonObjectType::EnvVarCollection),
        );
        if AuthStateProvider::handle(ctx).read(ctx, |auth_state_provider, _ctx| {
            auth_state_provider
                .get()
                .is_anonymous_user_past_object_limit(env_var_collection_type, count + 1)
                .unwrap_or_default()
        }) {
            AuthManager::handle(ctx).update(ctx, |auth_manager: &mut AuthManager, ctx| {
                auth_manager.anonymous_user_hit_drive_object_limit(ctx);
            });
            return;
        };

        self.create_object(
            model,
            owner,
            client_id,
            entrypoint,
            force_expand,
            initial_folder_id,
            // When adding the initiated_by parameter to this function call, InitiatedBy::User was set as a default value.
            // This can be changed to InitiatedBy::System if this action was automatically kicked off by the system and we do not want a user facing toast.
            InitiatedBy::User,
            ctx,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_folder(
        &mut self,
        name: String,
        owner: Owner,
        client_id: ClientId,
        initial_folder_id: Option<SyncId>,
        force_expand: bool,
        initiated_by: InitiatedBy,
        ctx: &mut ModelContext<Self>,
    ) {
        self.create_object(
            // TODO(INT-789): support creating folders as warp packs
            CloudFolderModel::new(&name, false),
            owner,
            client_id,
            Default::default(),
            force_expand,
            initial_folder_id,
            initiated_by,
            ctx,
        );
    }

    /// Bulk creates a list of generic string objects, all in a single
    /// sqllite write and server api call.  More efficient than calling
    /// create_object for each object.
    ///
    /// Note that if the bulk creation request fails, the client will end up retrying
    /// object creation one write and request at a time.
    pub fn bulk_create_generic_string_objects<S, T>(
        &mut self,
        owner: Owner,
        inputs: Vec<GenericStringObjectInput<T, S>>,
        ctx: &mut ModelContext<Self>,
    ) where
        T: StringModel<
                CloudObjectType = GenericCloudObject<
                    GenericStringObjectId,
                    GenericStringModel<T, S>,
                >,
            > + 'static,
        S: Serializer<T> + 'static,
    {
        let mut objects = Vec::new();
        let mut sync_queue_objects = Vec::new();
        for input in inputs {
            let object_id = SyncId::ClientId(input.id);
            let serialized_model = input.model.serialized().into();
            let uniqueness_key = input.model.string_model.uniqueness_key();

            // Update in-memory model.
            CloudModel::handle(ctx).update(ctx, |cloud_model, ctx| {
                let object =
                GenericCloudObject::<GenericStringObjectId, GenericStringModel<T, S>>::new_local(
                    input.model,
                    owner,
                    input.initial_folder_id,
                    input.id,
                );
                cloud_model.create_object(object_id, object, ctx);
            });

            let cloud_model = CloudModel::as_ref(ctx);
            if let Some(object) = cloud_model
                .get_object_of_type::<GenericStringObjectId, GenericStringModel<T, S>>(&object_id)
            {
                objects.push(object.clone());
            }

            sync_queue_objects.push(GenericStringObjectToCreate {
                id: input.id,
                format: T::model_format(),
                serialized_model,
                initial_folder_id: input.initial_folder_id,
                entrypoint: input.entrypoint,
                uniqueness_key,

                // When adding the initiated_by parameter to this function call, InitiatedBy::User was set as a default value.
                // initiated_by values currently do not propagate through the sync queue for bulk create operations, but can be added in the future
                initiated_by: InitiatedBy::User,
            });
        }

        // Update sqlite with a single bulk request
        self.save_to_db(vec![GenericStringModel::<T, S>::bulk_upsert_event(
            &objects,
        )]);

        // Populate sync queue with a single bulk request
        SyncQueue::handle(ctx).update(ctx, |sync_queue, ctx| {
            sync_queue.enqueue(
                QueueItem::BulkCreateGenericStringObjects {
                    owner,
                    objects: sync_queue_objects,
                },
                ctx,
            )
        });
    }

    /// Generic function for creating a new cloud object with a given model.
    #[allow(clippy::too_many_arguments)]
    pub fn create_object<K, M>(
        &mut self,
        model: M,
        owner: Owner,
        client_id: ClientId,
        entrypoint: CloudObjectEventEntrypoint,
        force_expand: bool,
        initial_folder_id: Option<SyncId>,
        initiated_by: InitiatedBy,
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
        let object_id = SyncId::ClientId(client_id);
        let auth_state = AuthStateProvider::as_ref(ctx).get();
        let initial_editor = auth_state.user_id();

        // Update in-memory model.
        CloudModel::handle(ctx).update(ctx, |cloud_model, ctx| {
            let mut object = GenericCloudObject::<K, M>::new_local(
                model.clone(),
                owner,
                initial_folder_id,
                client_id,
            );
            object.metadata.current_editor_uid = initial_editor.map(|uid| uid.as_string());
            cloud_model.create_object(object_id, object, ctx);

            if force_expand {
                cloud_model.force_expand_object_and_ancestors(object_id, ctx);
            }
        });

        // Update sqlite.
        let cloud_model = CloudModel::as_ref(ctx);
        if let Some(object) = cloud_model.get_object_of_type::<K, M>(&object_id) {
            self.save_to_db([object.upsert_event()]);
        }

        // Populate sync queue.
        SyncQueue::handle(ctx).update(ctx, |sync_queue, ctx| {
            let cloud_model = CloudModel::as_ref(ctx);
            if let Some(object) = cloud_model.get_object_of_type::<K, M>(&object_id) {
                if let Some(queue_item) = object.create_object_queue_item(entrypoint, initiated_by)
                {
                    sync_queue.enqueue(queue_item, ctx);
                }
            };
        });
    }

    /// Create a new cloud object as an online-only operation.
    ///
    /// This is intended for creating objects where the caller will await completion and
    /// handle retries, such as the CLI.
    ///
    /// The cloud model and SQLite are only updated on success. This is to prevent the
    /// sync queue from clashing with caller-managed retries and potentially creating
    /// duplicates of the object.
    #[allow(clippy::too_many_arguments)]
    fn create_object_online<K, M>(
        &mut self,
        model: M,
        owner: Owner,
        client_id: ClientId,
        entrypoint: CloudObjectEventEntrypoint,
        force_expand: bool,
        initial_folder_id: Option<SyncId>,
        ctx: &mut ModelContext<Self>,
    ) -> impl Future<Output = anyhow::Result<ServerId>>
    where
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
        let (tx, rx) = oneshot::channel();
        let completion = async move { rx.await? };

        let initial_server_folder_id = match initial_folder_id {
            Some(SyncId::ServerId(id)) => Some(FolderId::from(id)),
            Some(SyncId::ClientId(_)) => {
                let _ = tx.send(Err(anyhow::anyhow!("Folder does not exist on the server")));
                return completion;
            }
            None => None,
        };

        let object_client = self.object_client.clone();
        let serialized_model = model.serialized();
        let handle = ctx.spawn(
            async move {
                M::send_create_request(
                    object_client,
                    CreateObjectRequest {
                        serialized_model: Some(serialized_model),
                        // TODO: Need a generic way to access this on cloud object models.
                        title: None,
                        owner,
                        client_id,
                        initial_folder_id: initial_server_folder_id,
                        entrypoint,
                    },
                )
                .await
            },
            move |me, result, ctx| match result {
                Ok(CreateCloudObjectResult::Success {
                    created_cloud_object,
                }) => {
                    let server_id = created_cloud_object.server_id_and_type.id;

                    // On success, and only on success, update the in-memory model and SQLite.
                    let upsert_event = CloudModel::handle(ctx).update(ctx, |cloud_model, ctx| {
                        // Because we don't fetch the full object from the server on creation, we
                        // instead create a local object and populate the server metadata. This
                        // mirrors how we handle SyncQueueEvent::ObjectCreationSuccessful, but
                        // since this is a fresh object, there are no dependencies or existing
                        // actions to modify.
                        let mut object = GenericCloudObject::<K, M>::new_local(
                            model.clone(),
                            owner,
                            initial_folder_id,
                            client_id,
                        );
                        object.set_pending_content_changes_status(
                            CloudObjectSyncStatus::NoLocalChanges,
                        );
                        object.set_server_id(server_id);
                        let object_id = SyncId::ServerId(server_id);
                        cloud_model.create_object(object_id, object, ctx);
                        let server_uid = server_id.uid();
                        cloud_model.set_latest_revision_and_editor(
                            &server_uid,
                            created_cloud_object.revision_and_editor,
                            ctx,
                        );
                        cloud_model.update_object_metadata_last_updated_ts(
                            &server_uid,
                            created_cloud_object.metadata_ts,
                            ctx,
                        );

                        if force_expand {
                            cloud_model.force_expand_object_and_ancestors(object_id, ctx);
                        }

                        cloud_model
                            .get_object_of_type::<K, M>(&object_id)
                            .map(|obj| obj.upsert_event())
                    });

                    // Save the object to SQLite.
                    if let Some(upsert_event) = upsert_event {
                        me.save_to_db([upsert_event]);
                    }

                    // Notify the caller.
                    let _ = tx.send(Ok(server_id));
                }
                Ok(CreateCloudObjectResult::UserFacingError(error)) => {
                    let _ = tx.send(Err(anyhow::anyhow!(error)));
                }
                Ok(CreateCloudObjectResult::GenericStringObjectUniqueKeyConflict) => {
                    let _ = tx.send(Err(anyhow::anyhow!("Unique key conflict")));
                }
                Err(err) => {
                    let _ = tx.send(Err(err));
                }
            },
        );
        self.spawned_futures.push(handle.future_id());
        completion
    }

    /// Update an existing cloud object as an online-only operation.
    ///
    /// This is intended for updating objects where the caller will await completion and
    /// handle retries, such as the CLI.
    ///
    /// The cloud model and SQLite are only updated on success. This is to prevent the
    /// sync queue from clashing with caller-managed retries.
    pub fn update_object_online<K, M>(
        &mut self,
        model: M,
        object_id: SyncId,
        revision_ts: Option<Revision>,
        ctx: &mut ModelContext<Self>,
    ) -> impl Future<Output = anyhow::Result<()>>
    where
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
        let (tx, rx) = oneshot::channel();
        let completion = async move { rx.await? };

        let server_id = match object_id {
            SyncId::ServerId(id) => id,
            SyncId::ClientId(_) => {
                let _ = tx.send(Err(anyhow::anyhow!("Object does not exist on the server")));
                return completion;
            }
        };

        if let Err(err) = CloudModel::handle(ctx).update(ctx, |cloud_model, _| {
            match cloud_model.get_object_of_type_mut::<K, M>(&object_id) {
                Some(object) => {
                    if object.has_conflicting_changes()
                        || object.metadata.has_pending_content_changes()
                        || object.metadata.has_pending_online_only_change()
                    {
                        anyhow::bail!("Object has pending changes");
                    }

                    // Because the content change is not persisted in SQLite, we do not increment
                    // the in-flight request counter. Since the request counter is persisted, if
                    // we increment it and the request fails, the object can be stuck in a pending
                    // state despite not having any changes to sync.

                    Ok(())
                }
                None => {
                    anyhow::bail!("Object is not synced");
                }
            }
        }) {
            let _ = tx.send(Err(err));
            return completion;
        }

        let object_client = self.object_client.clone();
        let model_to_save = model.clone();
        let handle = ctx.spawn(
            async move {
                model
                    .send_update_request(object_client, server_id, revision_ts)
                    .await
            },
            move |me, result, ctx| {
                match result {
                    Ok(UpdateCloudObjectResult::Success {
                        revision_and_editor,
                    }) => {
                        // On success, and only on success, update the in-memory model and SQLite.
                        let upsert_event =
                            CloudModel::handle(ctx).update(ctx, |cloud_model, ctx| {
                                cloud_model.update_object_from_edit(model_to_save, object_id, ctx);
                                let server_uid = server_id.uid();
                                cloud_model.set_latest_revision_and_editor(
                                    &server_uid,
                                    revision_and_editor.clone(),
                                    ctx,
                                );
                                cloud_model
                                    .check_and_maybe_clear_current_conflict(&server_uid, ctx);

                                cloud_model
                                    .get_by_uid(&server_uid)
                                    .map(|object| object.upsert_event())
                            });

                        // Save the object to SQLite.
                        if let Some(upsert_event) = upsert_event {
                            me.save_to_db([upsert_event]);
                        }

                        // Notify the caller.
                        let _ = tx.send(Ok(()));
                    }
                    Ok(UpdateCloudObjectResult::Rejected { .. }) => {
                        // We don't need to do anything with the conflicting object, since the
                        // original edit wasn't saved to SQLite.
                        let _ = tx.send(Err(anyhow::anyhow!(
                            "Update rejected: object was modified by another client."
                        )));
                    }
                    Err(err) => {
                        let _ = tx.send(Err(err));
                    }
                };
            },
        );
        self.spawned_futures.push(handle.future_id());
        completion
    }

    /// Generic function for updating a cloud object with a new model.
    pub fn update_object<K, M>(
        &mut self,
        model: M,
        object_id: SyncId,
        revision_ts: Option<Revision>,
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
        // Update in-memory model.
        CloudModel::handle(ctx).update(ctx, |cloud_model, ctx| {
            cloud_model.update_object_from_edit(model.clone(), object_id, ctx);
            if let Some(object) = cloud_model.get_mut_by_uid(&object_id.uid()) {
                object.increment_in_flight_request_count();
                ctx.notify();
            }
        });

        // Update sqlite.
        let cloud_model = CloudModel::as_ref(ctx);
        if let Some(object) = cloud_model.get_object_of_type::<K, M>(&object_id) {
            self.save_to_db([object.upsert_event()]);
        };

        // Populate sync queue.
        SyncQueue::handle(ctx).update(ctx, |sync_queue, ctx| {
            let cloud_model = CloudModel::as_ref(ctx);
            if let Some(object) = cloud_model.get_object_of_type::<K, M>(&object_id) {
                sync_queue.enqueue(object.update_object_queue_item(revision_ts), ctx);
            };
        });
    }

    // Takes a generic SyncId and records the action.
    pub fn record_object_action(
        &mut self,
        id_and_type: CloudObjectTypeAndId,
        action_type: ObjectActionType,
        data: Option<String>,
        ctx: &mut ModelContext<Self>,
    ) {
        // Take the action timestamp from the client.
        let action_timestamp = Utc::now();

        // Update in-memory model.
        let object_action = ObjectActions::handle(ctx).update(ctx, |object_actions_model, ctx| {
            object_actions_model.insert_action(
                id_and_type.uid(),
                id_and_type.sqlite_uid_hash(),
                action_type.clone(),
                data.clone(),
                action_timestamp,
                ctx,
            )
        });

        // Update sqlite.
        self.save_to_db([ModelEvent::InsertObjectAction { object_action }]);

        // Populate sync queue.
        SyncQueue::handle(ctx).update(ctx, |sync_queue, ctx| {
            sync_queue.enqueue(
                QueueItem::RecordObjectAction {
                    id_and_type,
                    action_type,
                    data,
                    action_timestamp,
                },
                ctx,
            );
        });
    }

    /// After a call to RecordObjectAction returns, we remove whichever pending action caused the call from the model.
    fn remove_pending_object_action(
        &mut self,
        uid: &ObjectUid,
        action_timestamp: &DateTime<Utc>,
        ctx: &mut ModelContext<Self>,
    ) {
        ObjectActions::handle(ctx).update(ctx, |object_actions_model, ctx| {
            object_actions_model.remove_pending_action(uid, action_timestamp, ctx);
        });
    }

    fn maybe_overwrite_object_action_history(
        &mut self,
        history: &ObjectActionHistory,
        ctx: &mut ModelContext<Self>,
    ) {
        ObjectActions::handle(ctx).update(ctx, |object_actions_model, ctx| {
            // Accept this action history if we don't have any actions for this object OR the server's latest action
            // for this object is at least as recent as our latest synced action for this object
            let latest_processed_at_ts =
                object_actions_model.get_latest_processed_at_ts(&history.uid);
            if latest_processed_at_ts
                .is_none_or(|client_ts| client_ts <= history.latest_processed_at_timestamp)
            {
                // Overwrite the history for this object.
                object_actions_model.overwrite_action_history_for_object(
                    &history.uid,
                    history.actions.clone(),
                    ctx,
                );
            }
        });
    }

    /// Overwrites the actions in SQLite for a specified set of objects with the actions that
    /// are currently in the ObjectActions singleton model.
    fn sync_actions_for_objects_to_sqlite(
        &mut self,
        object_uids: Vec<&ObjectUid>,
        ctx: &mut ModelContext<Self>,
    ) {
        // Retrieve the objects from the ObjectActions model
        let actions = ObjectActions::handle(ctx).read(ctx, |object_actions_model, _ctx| {
            object_actions_model.get_actions_for_objects(object_uids)
        });

        // Overwrite the actions for those objects in sqlite
        let actions_to_sync: Vec<ObjectAction> = actions.values().flatten().cloned().collect();
        self.save_to_db([ModelEvent::SyncObjectActions { actions_to_sync }]);
    }

    /// Sets the notebooks current editor in memory. SQLite is not updated until we receive
    /// server confirmation.
    fn set_notebook_current_editor(
        &self,
        notebook_id: &SyncId,
        editor_uid: Option<String>,
        ctx: &mut ModelContext<Self>,
    ) {
        CloudModel::handle(ctx).update(ctx, |cloud_model, ctx| {
            if let Some(notebook) = cloud_model.get_notebook_mut(notebook_id) {
                notebook.metadata.set_current_editor(editor_uid);
                ctx.notify();
            }
        });
    }

    pub fn grab_notebook_edit_access(
        &mut self,
        notebook_id: SyncId,
        optimistically_grant_access: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        // If the object isn't known to the server yet, we should not proceed
        let SyncId::ServerId(server_id) = notebook_id else {
            return;
        };

        let auth_state = AuthStateProvider::as_ref(ctx).get();
        let user_uid = auth_state.user_id().unwrap_or_default();
        if optimistically_grant_access {
            self.set_notebook_current_editor(&notebook_id, Some(user_uid.as_string()), ctx);
        }
        let cloud_object_client = self.object_client.clone();
        // Make the request.
        let future = ctx.spawn_with_retry_on_error(
            move || {
                let cloud_object_client = cloud_object_client.clone();
                async move { cloud_object_client.grab_notebook_edit_access(server_id.into()).await }
            },
            *ONLINE_ONLY_OPERATION_RETRY_STRATEGY,
            move |me, res, ctx| match res {
                RequestState::RequestSucceeded(metadata) => {
                    // First, update the local view of metadata.
                    me.store_metadata_update(server_id, metadata, ctx, |_| {});

                    // If we successfully took access from another user, update the in memory editor
                    // and emit an event so we know to switch into edit mode.
                    if !optimistically_grant_access {
                        me.set_notebook_current_editor(
                            &notebook_id,
                            Some(user_uid.as_string()),
                            ctx,
                        );
                        ctx.emit(UpdateManagerEvent::ObjectOperationComplete {
                            result: ObjectOperationResult {
                                success_type: OperationSuccessType::Success,
                                operation: ObjectOperation::TakeEditAccess,
                                client_id: None,
                                server_id: Some(server_id),
                                num_objects: None,
                            },
                        });
                    }
                }
                RequestState::RequestFailedRetryPending(e) => {
                    log::warn!("Failed to grab edit access: {e}. Retrying");
                }
                RequestState::RequestFailed(e) => {
                    // If we are trying to take access, notify the user that the operation failed. If nobody else was
                    // editing, then we optimistically allow the user to proceed and do nothing here.
                    if !optimistically_grant_access {
                        log::warn!("Failed to grab edit access on server: {e}. Not retrying. Edit access not granted on client.");
                        ctx.emit(UpdateManagerEvent::ObjectOperationComplete {
                            result: ObjectOperationResult {
                                success_type: OperationSuccessType::Failure,
                                operation: ObjectOperation::TakeEditAccess,
                                client_id: None,
                                server_id: Some(server_id),
                                num_objects: None,
                            },
                        });
                    } else {
                        log::warn!("Failed to grab edit access on server: {e}. Not retrying. Edit access still granted on client.");
                    }
                    ctx.notify();
                }
            },
        );
        self.spawned_futures.push(future.future_id());
    }

    /// Optimistically gives up edit access for a notebook and sends a request to the server
    /// to update the notebooks current editor. We current do not have a retry protocol
    /// for this request and intentionall do nothing on error. For more info see:
    /// https://docs.google.com/document/d/1KgDFLApPg1uDVP-vOwhZzL1kRIviS8mMECIZg2VCKLY/edit
    pub fn give_up_notebook_edit_access(
        &mut self,
        notebook_id: SyncId,
        ctx: &mut ModelContext<Self>,
    ) {
        // If the object isn't known to the server yet, we should not proceed
        let SyncId::ServerId(server_id) = notebook_id else {
            return;
        };

        let current_editor = CloudViewModel::as_ref(ctx)
            .object_current_editor(&notebook_id.uid(), ctx)
            .unwrap_or(Editor::no_editor());

        // Only give up access if the current user has edit access
        if matches!(current_editor.state, EditorState::CurrentUser) {
            self.set_notebook_current_editor(&notebook_id, None, ctx);
            let object_client = self.object_client.clone();
            // Make the request.
            let future = ctx.spawn_with_retry_on_error(
                move || {
                    let object_client = object_client.clone();
                    async move {
                        object_client
                            .give_up_notebook_edit_access(server_id.into())
                            .await
                    }
                },
                *ONLINE_ONLY_OPERATION_RETRY_STRATEGY,
                move |me, res, ctx| match res {
                    RequestState::RequestSucceeded(new_metadata) => {
                        // If the request was successful, ensure we have the most up to date metadata
                        me.store_metadata_update(server_id, new_metadata, ctx, |_| {});
                    }
                    RequestState::RequestFailedRetryPending(e) => {
                        log::warn!("Failed to give up edit access: {e}. Retrying");
                    }
                    RequestState::RequestFailed(e) => {
                        log::warn!("Failed to give up edit access: {e}. Not retrying");
                    }
                },
            );
            self.spawned_futures.push(future.future_id());
        }
    }

    /// Optimistically marks the object as trashed, updates the metadata sync status to pending, and returns both
    /// the metadata timestamp and the newly-set trashed timestamp. We need to check the metadata timestamp
    /// in the case where we need to revert this (i.e. if there was a rtc message in the meantime, we shouldn't
    /// overwrite the values and don't need to).
    // TODO: we currently set trashed_ts here with the client's clock, but we should revise this metadata flow
    // to get the timestamp from the server instead.
    fn mark_object_trashed_and_return_timestamps(
        &self,
        uid: &ObjectUid,
        ctx: &mut ModelContext<Self>,
    ) -> (Option<ServerTimestamp>, Option<ServerTimestamp>) {
        let timestamp = ServerTimestamp::new(Utc::now());
        CloudModel::handle(ctx).update(ctx, |cloud_model, ctx| {
            if let Some(object) = cloud_model.get_mut_by_uid(uid) {
                // Here, we write a timestamp to the trashed_ts field. The client will eventually update to
                // the canonical version of the timestamp once it receives an rtc message from the server.

                object.metadata_mut().trashed_ts = Some(timestamp);
                object
                    .metadata_mut()
                    .pending_changes_statuses
                    .has_pending_metadata_change = true;
                ctx.emit(CloudModelEvent::ObjectTrashed {
                    type_and_id: object.cloud_object_type_and_id(),
                    source: UpdateSource::Local,
                });
                ctx.notify();
                (
                    object.metadata().metadata_last_updated_ts,
                    object.metadata().trashed_ts,
                )
            } else {
                (None, None)
            }
        })
    }

    pub fn trash_object(&mut self, id: CloudObjectTypeAndId, ctx: &mut ModelContext<Self>) {
        // // If the object isn't known to the server yet, we can't trash it.
        let Some(server_id) = id.server_id() else {
            return;
        };

        let hashed_id = id.uid();
        // If there's a pending online-only operation for this object, don't trash it.
        let Some(has_pending_online_only_operation) =
            CloudModel::handle(ctx).read(ctx, |model, _| {
                model
                    .get_by_uid(&hashed_id)
                    .map(|object| object.metadata().has_pending_online_only_change())
            })
        else {
            return;
        };

        if has_pending_online_only_operation {
            return;
        }

        let (metadata_ts, _trashed_ts) =
            self.mark_object_trashed_and_return_timestamps(&hashed_id, ctx);

        let object_client = self.object_client.clone();

        // Make the request.
        let future = ctx.spawn_with_retry_on_error(
            move || {
                let object_client = object_client.clone();
                async move { object_client.trash_object(server_id).await }
            },
            *ONLINE_ONLY_OPERATION_RETRY_STRATEGY,
            move |me, res, ctx| match res {
                RequestState::RequestSucceeded(_) => {
                    // Mark change as completed.
                    CloudModel::handle(ctx).update(ctx, |cloud_model, _| {
                        if let Some(object) = cloud_model.get_mut_by_uid(&hashed_id) {
                            object
                                .metadata_mut()
                                .pending_changes_statuses
                                .has_pending_metadata_change = false;
                        }

                        // Persist changes in sqlite.
                        let hashed_sqlite_id =
                            server_id.sqlite_type_and_uid_hash(id.object_id_type());
                        me.save_in_memory_object_metadata_to_sqlite(
                            cloud_model,
                            &hashed_id,
                            &hashed_sqlite_id,
                        );
                    });

                    ctx.emit(UpdateManagerEvent::ObjectOperationComplete {
                        result: ObjectOperationResult {
                            success_type: OperationSuccessType::Success,
                            operation: ObjectOperation::Trash,
                            client_id: None,
                            server_id: Some(ServerId::from_string_lossy(&hashed_id)),
                            num_objects: None,
                        },
                    });
                    ctx.notify();
                }
                RequestState::RequestFailedRetryPending(e) => {
                    log::warn!("Failed to trash object: {e}. Retrying");
                }
                RequestState::RequestFailed(e) => {
                    log::warn!("Failed to trash object: {e}. Not retrying");
                    // Since the trashing operation failed, let's return the object to its previous state.
                    CloudModel::handle(ctx).update(ctx, |cloud_model, ctx| {
                        if let Some(obj) = cloud_model.get_mut_by_uid(&hashed_id) {
                            // Only revert the operation if the metadata hasn't changed in the meantime.
                            // If it has (e.g. from an RTC message), that message would have updated the metadata to the latest
                            // server state, so we shouldn't do any further updates here. Otherwise, revert the change we did.
                            let metadata_ts_unchanged =
                                obj.metadata().metadata_last_updated_ts == metadata_ts;
                            if metadata_ts_unchanged {
                                obj.metadata_mut().trashed_ts = None;
                            }

                            obj.metadata_mut()
                                .pending_changes_statuses
                                .has_pending_metadata_change = false;

                            ctx.emit(CloudModelEvent::ObjectUntrashed {
                                type_and_id: obj.cloud_object_type_and_id(),
                                source: UpdateSource::Local,
                            });
                            ctx.notify();
                        }
                    });

                    // Show an error toast to relay the failure to the user.
                    ctx.emit(UpdateManagerEvent::ObjectOperationComplete {
                        result: ObjectOperationResult {
                            success_type: OperationSuccessType::Failure,
                            operation: ObjectOperation::Trash,
                            client_id: None,
                            server_id: Some(ServerId::from_string_lossy(&hashed_id)),
                            num_objects: None,
                        },
                    });
                    ctx.notify();
                }
            },
        );

        self.spawned_futures.push(future.future_id());
    }

    pub fn untrash_object(&mut self, id: CloudObjectTypeAndId, ctx: &mut ModelContext<Self>) {
        // If the object isn't known to the server yet, we can't untrash it.
        let Some(server_id) = id.server_id() else {
            return;
        };

        let hashed_id = id.uid();
        // If there's a pending online-only operation for this object, don't untrash it.
        let Some(has_pending_online_only_operation) =
            CloudModel::handle(ctx).read(ctx, |model, _| {
                model
                    .get_by_uid(&hashed_id)
                    .map(|object| object.metadata().has_pending_online_only_change())
            })
        else {
            return;
        };

        if has_pending_online_only_operation {
            return;
        }

        CloudModel::handle(ctx).update(ctx, |cloud_model, _| {
            if let Some(object) = cloud_model.get_mut_by_uid(&hashed_id) {
                object
                    .metadata_mut()
                    .pending_changes_statuses
                    .pending_untrash = true;
            }
        });

        let object_client = self.object_client.clone();

        // Make the request.
        let future = ctx.spawn_with_retry_on_error(
            move || {
                let object_client = object_client.clone();
                async move { object_client.untrash_object(server_id).await }
            },
            *ONLINE_ONLY_OPERATION_RETRY_STRATEGY,
            move |me, res, ctx| match res {
                RequestState::RequestSucceeded(untrash_result) => {
                    // Mark change as completed.
                    match untrash_result {
                        ObjectMetadataUpdateResult::Failure => {
                            // Mark item as no longer pending.
                            CloudModel::handle(ctx).update(ctx, |cloud_model, _| {
                                if let Some(object) = cloud_model.get_mut_by_uid(&hashed_id) {
                                    object
                                        .metadata_mut()
                                        .pending_changes_statuses
                                        .pending_untrash = false;
                                }
                            });

                            // Show an error toast to relay the failure to the user.
                            ctx.emit(UpdateManagerEvent::ObjectOperationComplete {
                                result: ObjectOperationResult {
                                    success_type: OperationSuccessType::Failure,
                                    operation: ObjectOperation::Untrash,
                                    client_id: None,
                                    server_id: Some(ServerId::from_string_lossy(&hashed_id)),
                                    num_objects: None,
                                },
                            });
                        }
                        ObjectMetadataUpdateResult::Success { metadata } => {
                            me.store_metadata_update(server_id, *metadata, ctx, |object| {
                                object
                                    .metadata_mut()
                                    .pending_changes_statuses
                                    .pending_untrash = false;
                            });

                            // When untrashing an object, we do not optimistically clear its
                            // trashed_ts. Instead, on success, it'll be cleared when the
                            // store_metadata_update call above applies the new metadata from the
                            // server. Once that's done, we can emit an event so callers re-check
                            // trashed_ts.
                            CloudModel::handle(ctx).update(ctx, |cloud_model, ctx| {
                                if let Some(object) = cloud_model.get_by_uid(&hashed_id) {
                                    ctx.emit(CloudModelEvent::ObjectUntrashed {
                                        type_and_id: object.cloud_object_type_and_id(),
                                        source: UpdateSource::Local,
                                    });
                                    ctx.notify();
                                }
                            });

                            ctx.emit(UpdateManagerEvent::ObjectOperationComplete {
                                result: ObjectOperationResult {
                                    success_type: OperationSuccessType::Success,
                                    operation: ObjectOperation::Untrash,
                                    client_id: None,
                                    server_id: Some(ServerId::from_string_lossy(&hashed_id)),
                                    num_objects: None,
                                },
                            });
                        }
                    }

                    ctx.notify();
                }
                RequestState::RequestFailedRetryPending(e) => {
                    log::warn!("Failed to restore object: {e}. Retrying");
                }
                RequestState::RequestFailed(e) => {
                    log::warn!("Failed to restore object: {e}. Not retrying");

                    // Mark item as no longer pending.
                    CloudModel::handle(ctx).update(ctx, |cloud_model, _| {
                        if let Some(object) = cloud_model.get_mut_by_uid(&hashed_id) {
                            object
                                .metadata_mut()
                                .pending_changes_statuses
                                .pending_untrash = false;
                        }
                    });

                    // Show an error toast to relay the failure to the user.
                    ctx.emit(UpdateManagerEvent::ObjectOperationComplete {
                        result: ObjectOperationResult {
                            success_type: OperationSuccessType::Failure,
                            operation: ObjectOperation::Untrash,
                            client_id: None,
                            server_id: Some(ServerId::from_string_lossy(&hashed_id)),
                            num_objects: None,
                        },
                    });

                    ctx.notify();
                }
            },
        );

        self.spawned_futures.push(future.future_id());
    }

    pub fn delete_object_by_user(
        &mut self,
        id: CloudObjectTypeAndId,
        ctx: &mut ModelContext<Self>,
    ) {
        self.delete_object_with_initiated_by(id, InitiatedBy::User, ctx);
    }

    pub fn delete_object_with_initiated_by(
        &mut self,
        id: CloudObjectTypeAndId,
        initiated_by: InitiatedBy,
        ctx: &mut ModelContext<Self>,
    ) {
        // If the object isn't known to the server yet, we can't delete it.
        let Some(server_id) = id.server_id() else {
            return;
        };

        let uid = id.uid();
        // If there's a pending online-only operation for this object, don't delete it.
        let Some((has_pending_online_only_operation, has_pending_delete)) = CloudModel::handle(ctx)
            .read(ctx, |model, _| {
                model.get_by_uid(&uid).map(|object| {
                    (
                        object.metadata().has_pending_online_only_change(),
                        object.metadata().pending_changes_statuses.pending_delete,
                    )
                })
            })
        else {
            return;
        };

        if has_pending_online_only_operation || has_pending_delete {
            return;
        }

        let object_client = self.object_client.clone();

        CloudModel::handle(ctx).update(ctx, |cloud_model, _| {
            if let Some(object) = cloud_model.get_mut_by_uid(&uid) {
                // Mark the object as pending deletion.
                object
                    .metadata_mut()
                    .pending_changes_statuses
                    .pending_delete = true;
            }
        });

        // Make the request.
        let future = ctx.spawn_with_retry_on_error(
            move || {
                let object_client = object_client.clone();
                async move { object_client.delete_object(server_id).await }
            },
            *ONLINE_ONLY_OPERATION_RETRY_STRATEGY,
            move |me, res, ctx| match res {
                RequestState::RequestSucceeded(delete_result) => {
                    match delete_result {
                        ObjectDeleteResult::Success { deleted_ids } => {
                            let num_deleted_objects = me.on_object_delete_success(deleted_ids, ctx);
                            ctx.emit(UpdateManagerEvent::ObjectOperationComplete {
                                result: ObjectOperationResult {
                                    success_type: OperationSuccessType::Success,
                                    operation: ObjectOperation::Delete { initiated_by },
                                    client_id: None,
                                    server_id: Some(ServerId::from_string_lossy(&uid)),
                                    num_objects: Some(num_deleted_objects),
                                },
                            });
                        }
                        ObjectDeleteResult::Failure => {
                            ctx.emit(UpdateManagerEvent::ObjectOperationComplete {
                                result: ObjectOperationResult {
                                    success_type: OperationSuccessType::Failure,
                                    operation: ObjectOperation::Delete { initiated_by },
                                    client_id: None,
                                    server_id: Some(ServerId::from_string_lossy(&uid)),
                                    num_objects: None,
                                },
                            });
                        }
                    }

                    ctx.notify();
                }
                RequestState::RequestFailedRetryPending(e) => {
                    log::warn!("Failed to delete object: {e}. Retrying");
                }
                RequestState::RequestFailed(e) => {
                    log::warn!("Failed to delete object: {e}. Not retrying");

                    // Show an error toast to relay the failure to the user.
                    ctx.emit(UpdateManagerEvent::ObjectOperationComplete {
                        result: ObjectOperationResult {
                            success_type: OperationSuccessType::Failure,
                            operation: ObjectOperation::Delete { initiated_by },
                            client_id: None,
                            server_id: Some(ServerId::from_string_lossy(&uid)),
                            num_objects: None,
                        },
                    });

                    // Reset the delete bit since the request failed.
                    CloudModel::handle(ctx).update(ctx, |cloud_model, _| {
                        if let Some(object) = cloud_model.get_mut_by_uid(&uid) {
                            // Mark the object as pending deletion.
                            object
                                .metadata_mut()
                                .pending_changes_statuses
                                .pending_delete = false;
                        }
                    });

                    ctx.notify();
                }
            },
        );

        self.spawned_futures.push(future.future_id());
    }

    pub fn empty_trash(&mut self, space: Space, ctx: &mut ModelContext<Self>) {
        let object_client = self.object_client.clone();

        let owner = match UserWorkspaces::as_ref(ctx).space_to_owner(space, ctx) {
            Some(owner) => owner,
            None => {
                // TODO: For the Shared space, this should delete every object that's shared with the user
                // and trashed.
                log::warn!("Tried to empty trash in unsupported space {space:?}");
                return;
            }
        };

        // Make the request.
        let future = ctx.spawn_with_retry_on_error(
            move || {
                let object_client = object_client.clone();
                async move { object_client.empty_trash(owner).await }
            },
            *ONLINE_ONLY_OPERATION_RETRY_STRATEGY,
            move |me, res, ctx| match res {
                RequestState::RequestSucceeded(delete_result) => {
                    match delete_result {
                        ObjectDeleteResult::Success { deleted_ids } => {
                            let num_deleted_objects = me.on_object_delete_success(deleted_ids, ctx);

                            if num_deleted_objects == 0 {
                                // Show rejection toast that states there are no objects in the Trash
                                ctx.emit(UpdateManagerEvent::ObjectOperationComplete {
                                    result: ObjectOperationResult {
                                        success_type: OperationSuccessType::Rejection,
                                        operation: ObjectOperation::EmptyTrash,
                                        client_id: None,
                                        server_id: None,
                                        num_objects: Some(num_deleted_objects),
                                    },
                                });
                            } else {
                                // Show success confirmation toast
                                ctx.emit(UpdateManagerEvent::ObjectOperationComplete {
                                    result: ObjectOperationResult {
                                        success_type: OperationSuccessType::Success,
                                        operation: ObjectOperation::EmptyTrash,
                                        client_id: None,
                                        server_id: None,
                                        num_objects: Some(num_deleted_objects),
                                    },
                                });
                            }
                        }
                        ObjectDeleteResult::Failure => {
                            // Show an error toast to relay the failure to the user.
                            ctx.emit(UpdateManagerEvent::ObjectOperationComplete {
                                result: ObjectOperationResult {
                                    success_type: OperationSuccessType::Failure,
                                    operation: ObjectOperation::EmptyTrash,
                                    client_id: None,
                                    server_id: None,
                                    num_objects: Some(0),
                                },
                            });
                        }
                    }

                    ctx.notify();
                }
                RequestState::RequestFailedRetryPending(e) => {
                    log::warn!("Failed to empty trash: {e}. Retrying");
                }
                RequestState::RequestFailed(e) => {
                    log::warn!("Failed to empty trash: {e}. Not retrying");

                    // Show an error toast to relay the failure to the user.
                    ctx.emit(UpdateManagerEvent::ObjectOperationComplete {
                        result: ObjectOperationResult {
                            success_type: OperationSuccessType::Failure,
                            operation: ObjectOperation::EmptyTrash,
                            client_id: None,
                            server_id: None,
                            num_objects: Some(0),
                        },
                    });
                    ctx.notify();
                }
            },
        );

        self.spawned_futures.push(future.future_id());
    }

    pub fn on_object_delete_success(
        &mut self,
        deleted_ids: Vec<SyncId>,
        ctx: &mut ModelContext<'_, UpdateManager>,
    ) -> i32 {
        let cloud_model_handle = CloudModel::handle(ctx);
        let all_object_uids: Vec<ObjectUid> = deleted_ids.iter().map(|&id| id.uid()).collect();

        // This variable counts the number of objects deleted client-side in each Empty Trash action,
        // because the server returns everything in the db, including objects that have already been marked for deletion
        let mut num_deleted_objects = 0;
        let mut sync_ids_and_types: Vec<(SyncId, ObjectIdType)> = Vec::new();
        cloud_model_handle.update(ctx, |cloud_model, ctx| {
            (sync_ids_and_types, num_deleted_objects) =
                cloud_model.delete_objects_by_id(all_object_uids.clone(), ctx);
        });

        // Deleted the actions associated with these objects too.
        ObjectActions::handle(ctx).update(ctx, |object_actions, ctx| {
            for uid in all_object_uids.clone() {
                object_actions.delete_actions_for_object(&uid, ctx);
            }
        });

        // Return early if empty
        if num_deleted_objects == 0 {
            return num_deleted_objects;
        }

        // Delete objects from sqlite. This will also delete their actions.
        self.save_to_db([ModelEvent::DeleteObjects {
            ids: sync_ids_and_types,
        }]);

        num_deleted_objects
    }

    pub fn rename_folder(
        &mut self,
        folder_id: SyncId,
        new_name: String,
        ctx: &mut ModelContext<Self>,
    ) {
        let cloud_model = CloudModel::as_ref(ctx);
        let revision = cloud_model.current_revision(&folder_id).cloned();
        if let Some(folder) = cloud_model.get_folder(&folder_id) {
            let new_folder = CloudFolderModel {
                name: new_name,
                is_open: folder.model().is_open,
                is_warp_pack: folder.model().is_warp_pack,
            };
            self.update_object(new_folder, folder_id, revision, ctx);
        } else {
            log::warn!("Attempted to rename folder that doesn't exist with id: {folder_id:?}");
        }
    }

    /// Persist updated metadata returned by a non-content update API. Because this metadata comes
    /// from an online-only API call, we assume it is always more up-to-date than the local
    /// metadata.
    ///
    /// The caller is responsible for clearing operation-specific pending state via the `update`
    /// function.
    ///
    /// See https://docs.google.com/document/d/1fLfSJu53DAlxeznRUaE3WjqJ2W3qbVIxCOisKdW-yBE/edit
    fn store_metadata_update(
        &mut self,
        server_id: ServerId,
        new_metadata: ServerMetadata,
        ctx: &mut ModelContext<Self>,
        update: impl FnOnce(&mut dyn CloudObject),
    ) {
        let cloud_model_handle = CloudModel::handle(ctx);

        // Update the in-memory metadata.
        let mut hashed_sqlite_id = None;
        cloud_model_handle.update(ctx, |cloud_model, _| {
            if let Some(object) = cloud_model.get_mut_by_uid(&server_id.uid()) {
                object
                    .metadata_mut()
                    .update_from_new_metadata_ts(new_metadata);
                update(object.as_mut());

                hashed_sqlite_id =
                    Some(server_id.sqlite_type_and_uid_hash(object.object_type().into()));
            }
        });

        // If we updated in memory, persist to SQLite.
        if let Some(hashed_sqlite_id) = hashed_sqlite_id {
            self.save_in_memory_object_metadata_to_sqlite(
                cloud_model_handle.as_ref(ctx),
                &server_id.uid(),
                &hashed_sqlite_id,
            );
        }
    }
}

/// Return the newly duplicated object's name based on the original object's name. E.g.:
/// - "my object name" -> "my object name (1)"
pub fn get_duplicate_object_name(original_name: &str) -> String {
    match DUPLICATE_OBJECT_NAME_REGEX
        .captures(original_name)
        .and_then(|caps| caps.get(1))
        .and_then(|num| num.as_str().parse::<usize>().ok())
    {
        Some(num) => {
            let new_num = num.saturating_add(1);

            // edge case check for when the duplicate number is usize::MAX
            if new_num == usize::MAX {
                format!("{original_name} (1)")
            } else {
                DUPLICATE_OBJECT_NAME_REGEX
                    .replace(original_name, format!(" ({new_num})"))
                    .to_string()
            }
        }
        None => format!("{original_name} (1)"),
    }
}

impl Entity for UpdateManager {
    type Event = UpdateManagerEvent;
}

impl SingletonEntity for UpdateManager {}

#[cfg(test)]
#[path = "update_manager_tests.rs"]
mod tests;
