use crate::cloud_object::model::actions::{
    ObjectAction, ObjectActionHistory, ObjectActionSubtype, ObjectActionType,
};
use crate::cloud_object::model::generic_string_model::GenericStringObjectId;
use crate::cloud_object::{
    CloudModelType, CloudObjectEventEntrypoint, CreateCloudObjectResult, CreatedCloudObject,
    GenericStringObjectFormat, JsonObjectType, ObjectIdType, ObjectType, Owner, Revision,
    RevisionAndLastEditor, ServerCreationInfo, UpdateCloudObjectResult,
};

use crate::drive::CloudObjectTypeAndId;
use crate::notebooks::{CloudNotebookModel, NotebookId};
use crate::server::cloud_objects::update_manager::InitiatedBy;
use crate::server::server_api::auth::UserAuthenticationError;
use crate::server::server_api::ServerApiProvider;
use crate::system::SystemStats;
use crate::workflows::workflow::{Argument, ArgumentType, Workflow};
use crate::workflows::CloudWorkflowModel;
use std::collections::HashSet;
use std::ops::Index;

use crate::server::ids::{ClientId, HashableId, ServerId, ServerIdAndType, SyncId};
use crate::server::sync_queue::{CreationFailureReason, QueueItemId, SyncQueueEvent};
use crate::{NetworkStatus, QueueItem, SyncQueue};
use anyhow::anyhow;
use chrono::{DateTime, Duration, Utc};
use firebase::FirebaseError;
use itertools::Itertools;
use std::sync::Arc;
use warp_server_client::cloud_object::ServerPermissions;
use warpui::{r#async::Timer, App, Entity, ModelHandle, SingletonEntity};

#[cfg(test)]
use crate::server::server_api::object::MockObjectClient;

#[derive(Default)]
struct Events(Vec<SyncQueueEvent>);

impl Entity for Events {
    type Event = ();
}

impl Index<usize> for Events {
    type Output = SyncQueueEvent;

    fn index(&self, index: usize) -> &Self::Output {
        &self.0[index]
    }
}

fn initialize_app(app: &mut App) {
    app.add_singleton_model(|_| ServerApiProvider::new_for_test());
    app.add_singleton_model(|_| NetworkStatus::new());
    app.add_singleton_model(|_| SystemStats::new());
}

fn create_sync_queue(
    app: &mut App,
    queue_items: Vec<QueueItem>,
    cloud_objects_client_mock: MockObjectClient,
    immediately_start_dequeueing: bool,
) -> ModelHandle<SyncQueue> {
    let sync_queue = app.add_singleton_model(|ctx| {
        SyncQueue::new(queue_items, Arc::new(cloud_objects_client_mock), ctx)
    });

    if immediately_start_dequeueing {
        SyncQueue::handle(app).update(app, |sync_queue, ctx| {
            sync_queue.start_dequeueing(ctx);
        });
    }

    sync_queue
}

#[test]
fn test_create_notebook() {
    App::test((), |mut app| async move {
        let notebook_id = ClientId::default();
        let notebook_server_id = ServerId::from(1);
        let notebook_ts = DateTime::<Utc>::default();
        let notebook = CloudNotebookModel {
            title: "Shared Notebook".to_string(),
            data: "Hello".to_string(),
            ai_document_id: None,
            conversation_id: None,
        };

        let mut cloud_objects_client_mock = MockObjectClient::new();
        cloud_objects_client_mock
            .expect_create_notebook()
            .times(1)
            .return_once(move |_| {
                Ok(CreateCloudObjectResult::Success {
                    created_cloud_object: CreatedCloudObject {
                        client_id: notebook_id,
                        revision_and_editor: RevisionAndLastEditor {
                            revision: notebook_ts.into(),
                            last_editor_uid: None,
                        },
                        metadata_ts: notebook_ts.into(),
                        server_id_and_type: ServerIdAndType {
                            id: notebook_server_id,
                            id_type: ObjectIdType::Notebook,
                        },
                        creator_uid: None,
                        permissions: ServerPermissions::mock_personal(),
                    },
                })
            });

        initialize_app(&mut app);
        let sync_queue = create_sync_queue(&mut app, vec![], cloud_objects_client_mock, true);

        let sync_queue_events = app.add_model(|_ctx| Events::default());

        sync_queue_events.update(&mut app, |_, ctx| {
            ctx.subscribe_to_model(&sync_queue, |me, event, _ctx| me.0.push(event.clone()))
        });

        // Enqueue a single item and wait for the response to complete.
        sync_queue
            .update(&mut app, |sync_queue, ctx| {
                sync_queue.enqueue(
                    QueueItem::CreateObject {
                        object_type: ObjectType::Notebook,
                        owner: Owner::mock_current_user(),
                        id: notebook_id,
                        title: None,
                        serialized_model: Some(Arc::new(notebook.serialized())),
                        initial_folder_id: None,
                        entrypoint: Default::default(),
                        initiated_by: InitiatedBy::User,
                    },
                    ctx,
                );

                ctx.await_spawned_future(sync_queue.spawned_futures[0])
            })
            .await;

        sync_queue_events.update(&mut app, |sync_queue_events, _ctx| {
            assert_eq!(sync_queue_events.0.len(), 1);
            assert_eq!(
                sync_queue_events[0],
                SyncQueueEvent::ObjectCreationSuccessful {
                    server_creation_info: ServerCreationInfo {
                        server_id_and_type: ServerIdAndType {
                            id: notebook_server_id,
                            id_type: ObjectIdType::Notebook
                        },
                        creator_uid: None,
                        permissions: ServerPermissions::mock_personal(),
                    },
                    client_id: notebook_id,
                    revision_and_editor: RevisionAndLastEditor {
                        revision: notebook_ts.into(),
                        last_editor_uid: None
                    },
                    metadata_ts: notebook_ts.into(),
                    initiated_by: InitiatedBy::User
                }
            );
        })
    });
}

#[test]
fn test_generic_string_object_unique_key_failure() {
    App::test((), |mut app| async move {
        let owner = Owner::mock_current_user();
        let gso_id = ClientId::default();
        let gso_json = "{\"storage_key\":\"somepref\",\"value\":true,\"platform\":\"Global\"}";
        let workflow_id = ClientId::default();
        let workflow_server_id = ServerId::from(1);
        let workflow_ts = DateTime::<Utc>::default();
        let workflow_data = Workflow::new("my workflow", "echo hi");

        // This pattern is used in a couple places to control the order of async operations.
        // Basically, we wait to receive a message on a channel before certain mocks can continue,
        // so that we have time to assert intermediate states, at which point we fire a message.
        let (tx, rx) = std::sync::mpsc::channel();

        initialize_app(&mut app);

        let mut cloud_objects_client_mock = MockObjectClient::new();
        cloud_objects_client_mock
            .expect_create_generic_string_object()
            .times(1)
            .return_once(move |_, _, _| {
                Ok(CreateCloudObjectResult::GenericStringObjectUniqueKeyConflict)
            });
        cloud_objects_client_mock
            .expect_create_workflow()
            .times(1)
            .return_once(move |_| {
                rx.recv().unwrap();
                Ok(CreateCloudObjectResult::Success {
                    created_cloud_object: CreatedCloudObject {
                        client_id: workflow_id,
                        revision_and_editor: RevisionAndLastEditor {
                            revision: workflow_ts.into(),
                            last_editor_uid: None,
                        },
                        metadata_ts: workflow_ts.into(),
                        server_id_and_type: ServerIdAndType {
                            id: workflow_server_id,
                            id_type: ObjectIdType::Workflow,
                        },
                        creator_uid: None,
                        permissions: ServerPermissions::mock_personal(),
                    },
                })
            });

        let sync_queue = create_sync_queue(
            &mut app,
            vec![
                QueueItem::CreateObject {
                    object_type: ObjectType::GenericStringObject(GenericStringObjectFormat::Json(
                        JsonObjectType::Preference,
                    )),
                    owner,
                    id: gso_id,
                    title: None,
                    serialized_model: Some(Arc::new(gso_json.to_string().into())),
                    initial_folder_id: None,
                    entrypoint: CloudObjectEventEntrypoint::Unknown,
                    initiated_by: InitiatedBy::User,
                },
                QueueItem::CreateWorkflow {
                    object_type: ObjectType::Workflow,
                    owner,
                    id: workflow_id,
                    model: Arc::new(CloudWorkflowModel {
                        data: workflow_data,
                    }),
                    initial_folder_id: None,
                    entrypoint: CloudObjectEventEntrypoint::Unknown,
                    initiated_by: InitiatedBy::User,
                },
            ],
            cloud_objects_client_mock,
            false,
        );

        let sync_queue_events = app.add_model(|_ctx| Events::default());
        sync_queue_events.update(&mut app, |_, ctx| {
            ctx.subscribe_to_model(&sync_queue, |me, event, _ctx| me.0.push(event.clone()))
        });

        // Await the first future.
        sync_queue
            .update(&mut app, |queue, ctx| {
                queue.start_dequeueing(ctx);
                ctx.await_spawned_future(queue.spawned_futures[0])
            })
            .await;

        app.read(|ctx| {
            // The GSO failure should not stop the queue.
            assert!(sync_queue.as_ref(ctx).is_dequeueing());

            let events = sync_queue_events.as_ref(ctx);
            assert_eq!(events.0.len(), 1);
            assert_eq!(
                &events.0,
                &[SyncQueueEvent::ObjectCreationFailure {
                    reason: CreationFailureReason::UniqueKeyConflict {
                        id: gso_id.to_hash(),
                        initiated_by: InitiatedBy::User
                    },
                },]
            );
        });

        // Allow the workflow to be created.
        tx.send(()).unwrap();
        sync_queue
            .update(&mut app, |queue, ctx| {
                assert!(queue.is_dequeueing());
                ctx.await_spawned_future(queue.spawned_futures[1])
            })
            .await;

        app.read(|ctx| {
            // The queue should still be dequeueing.
            assert!(sync_queue.as_ref(ctx).is_dequeueing());

            let events = sync_queue_events.as_ref(ctx);
            assert_eq!(events.0.len(), 2);
            assert_eq!(
                &events.0,
                &[
                    SyncQueueEvent::ObjectCreationFailure {
                        reason: CreationFailureReason::UniqueKeyConflict {
                            id: gso_id.to_hash(),
                            initiated_by: InitiatedBy::User
                        },
                    },
                    SyncQueueEvent::ObjectCreationSuccessful {
                        server_creation_info: ServerCreationInfo {
                            server_id_and_type: ServerIdAndType {
                                id: workflow_server_id,
                                id_type: ObjectIdType::Workflow
                            },
                            creator_uid: None,
                            permissions: ServerPermissions::mock_personal(),
                        },
                        client_id: workflow_id,
                        revision_and_editor: RevisionAndLastEditor {
                            revision: workflow_ts.into(),
                            last_editor_uid: None
                        },
                        metadata_ts: workflow_ts.into(),
                        initiated_by: InitiatedBy::User
                    }
                ]
            );
        });
    });
}

#[test]
fn test_dequeue_after_transient_failure() {
    App::test((), |mut app| async move {
        let owner = Owner::mock_current_user();
        let success_notebook_id = ClientId::default();
        let success_notebook_server_id = ServerId::from(1);
        let success_notebook_ts = DateTime::<Utc>::default();
        let success_notebook = CloudNotebookModel {
            title: "Successful Notebook".to_string(),
            data: "Hello :)".to_string(),
            ai_document_id: None,
            conversation_id: None,
        };
        let failure_notebook_id = ClientId::default();
        let failure_notebook = CloudNotebookModel {
            title: "Failed Notebook".to_string(),
            data: "Hello :(".to_string(),
            ai_document_id: None,
            conversation_id: None,
        };

        let (tx, rx) = std::sync::mpsc::channel();

        let failure_attempts = 1 + super::DEFAULT_RETRY_OPTION.remaining_retries();
        let mut cloud_objects_client_mock = MockObjectClient::new();
        cloud_objects_client_mock
            .expect_create_notebook()
            .times(failure_attempts)
            .returning(move |_| Err(anyhow!("Transient network error!!!!")));
        cloud_objects_client_mock
            .expect_create_notebook()
            .times(1)
            .return_once(move |_| {
                rx.recv().unwrap();
                Ok(CreateCloudObjectResult::Success {
                    created_cloud_object: CreatedCloudObject {
                        client_id: success_notebook_id,
                        revision_and_editor: RevisionAndLastEditor {
                            revision: success_notebook_ts.into(),
                            last_editor_uid: None,
                        },
                        metadata_ts: success_notebook_ts.into(),
                        server_id_and_type: ServerIdAndType {
                            id: success_notebook_server_id,
                            id_type: ObjectIdType::Notebook,
                        },
                        creator_uid: None,
                        permissions: ServerPermissions::mock_personal(),
                    },
                })
            });

        initialize_app(&mut app);

        let sync_queue = create_sync_queue(
            &mut app,
            vec![
                QueueItem::CreateObject {
                    object_type: ObjectType::Notebook,
                    owner,
                    id: failure_notebook_id,
                    title: None,
                    serialized_model: Some(Arc::new(failure_notebook.serialized())),
                    initial_folder_id: None,
                    entrypoint: CloudObjectEventEntrypoint::Unknown,
                    initiated_by: InitiatedBy::User,
                },
                QueueItem::CreateObject {
                    object_type: ObjectType::Notebook,
                    owner,
                    id: success_notebook_id,
                    title: None,
                    serialized_model: Some(Arc::new(success_notebook.serialized())),
                    initial_folder_id: None,
                    entrypoint: CloudObjectEventEntrypoint::Unknown,
                    initiated_by: InitiatedBy::User,
                },
            ],
            cloud_objects_client_mock,
            false,
        );

        let sync_queue_events = app.add_model(|_ctx| Events::default());
        sync_queue_events.update(&mut app, |_, ctx| {
            ctx.subscribe_to_model(&sync_queue, |me, event, _ctx| me.0.push(event.clone()))
        });

        sync_queue
            .update(&mut app, |queue, ctx| {
                queue.start_dequeueing(ctx);
                ctx.await_spawned_future(queue.spawned_futures[0])
            })
            .await;

        // Wait for the first notebook's creation to fail.
        // The failures are retried, but their futures are spawned on background threads,
        // so we can't access them. Instead, we wait for a SyncQueue event to appear in the model.
        let mut timeout = Timer::after(std::time::Duration::from_secs(20));
        let mut has_event = false;
        while !has_event {
            if futures::poll!(&mut timeout).is_ready() {
                panic!("Timed out waiting for failure");
            }

            Timer::after(std::time::Duration::from_millis(500)).await;
            sync_queue_events.read(&app, |events, _ctx| {
                has_event = !events.0.is_empty();
            });
        }

        sync_queue_events.read(&app, |events, _| {
            assert_eq!(
                &events.0[0],
                &SyncQueueEvent::ObjectCreationFailure {
                    reason: CreationFailureReason::Other {
                        id: failure_notebook_id.to_hash(),
                        initiated_by: InitiatedBy::User
                    },
                }
            );
        });

        // Allow the second notebook to be created.
        tx.send(()).unwrap();
        sync_queue
            .update(&mut app, |queue, ctx| {
                assert!(queue.is_dequeueing());
                ctx.await_spawned_future(queue.spawned_futures[1])
            })
            .await;

        sync_queue_events.read(&app, |events, _| {
            assert_eq!(events.0.len(), 2);
            assert_eq!(
                &events.0,
                &[
                    SyncQueueEvent::ObjectCreationFailure {
                        reason: CreationFailureReason::Other {
                            id: failure_notebook_id.to_hash(),
                            initiated_by: InitiatedBy::User
                        },
                    },
                    SyncQueueEvent::ObjectCreationSuccessful {
                        server_creation_info: ServerCreationInfo {
                            server_id_and_type: ServerIdAndType {
                                id: success_notebook_server_id,
                                id_type: ObjectIdType::Notebook
                            },
                            creator_uid: None,
                            permissions: ServerPermissions::mock_personal(),
                        },
                        client_id: success_notebook_id,
                        revision_and_editor: RevisionAndLastEditor {
                            revision: success_notebook_ts.into(),
                            last_editor_uid: None
                        },
                        metadata_ts: success_notebook_ts.into(),
                        initiated_by: InitiatedBy::User
                    }
                ]
            );
        });
    });
}

#[test]
fn test_no_dequeue_after_intransient_failure() {
    App::test((), |mut app| async move {
        let owner = Owner::mock_current_user();
        let failure_notebook_id = ClientId::default();
        let failure_notebook = CloudNotebookModel {
            title: "Failed Notebook".to_string(),
            data: "Hello :(".to_string(),
            ai_document_id: None,
            conversation_id: None,
        };
        let second_notebook = CloudNotebookModel {
            title: "Second Notebook".to_string(),
            data: "I'd like to be created! But I won't be :(".to_string(),
            ai_document_id: None,
            conversation_id: None,
        };
        let second_notebook_id = ClientId::default();
        let second_notebook_item = QueueItem::CreateObject {
            object_type: ObjectType::Notebook,
            owner,
            id: second_notebook_id,
            title: None,
            serialized_model: Some(Arc::new(second_notebook.serialized())),
            initial_folder_id: None,
            entrypoint: CloudObjectEventEntrypoint::Unknown,
            initiated_by: InitiatedBy::User,
        };

        let failure_attempts = 1 + super::DEFAULT_RETRY_OPTION.remaining_retries();
        let mut cloud_objects_client_mock = MockObjectClient::new();
        cloud_objects_client_mock
            .expect_create_notebook()
            // Note that even though we don't dequeue future items because we know they'll fail,
            // we're still stuck retrying the initial item.
            .times(failure_attempts)
            .returning(move |_| {
                // This is one of the types of errors that won't cause us to keep dequeueing;
                // if Firebase rejects the user once, they'll likely reject requests for other queue items.
                Err(UserAuthenticationError::DeniedAccessToken(FirebaseError {
                    code: 401,
                    message: "Unauthenticated".to_string(),
                })
                .into())
            });

        initialize_app(&mut app);

        let sync_queue = create_sync_queue(
            &mut app,
            vec![
                QueueItem::CreateObject {
                    object_type: ObjectType::Notebook,
                    owner,
                    id: failure_notebook_id,
                    title: None,
                    serialized_model: Some(Arc::new(failure_notebook.serialized())),
                    initial_folder_id: None,
                    entrypoint: CloudObjectEventEntrypoint::Unknown,
                    initiated_by: InitiatedBy::User,
                },
                second_notebook_item.clone(),
            ],
            cloud_objects_client_mock,
            false,
        );

        let sync_queue_events = app.add_model(|_ctx| Events::default());
        sync_queue_events.update(&mut app, |_, ctx| {
            ctx.subscribe_to_model(&sync_queue, |me, event, _ctx| me.0.push(event.clone()))
        });

        sync_queue
            .update(&mut app, |queue, ctx| {
                queue.start_dequeueing(ctx);
                ctx.await_spawned_future(queue.spawned_futures[0])
            })
            .await;

        // Wait for the first notebook's creation to fail.
        // The failures are retried, but their futures are spawned on background threads,
        // so we can't access them. Instead, we wait for a SyncQueue event to appear in the model.
        let mut timeout = Timer::after(std::time::Duration::from_secs(20));
        let mut has_event = false;
        while !has_event {
            if futures::poll!(&mut timeout).is_ready() {
                panic!("Timed out waiting for failure");
            }

            Timer::after(std::time::Duration::from_millis(500)).await;
            sync_queue_events.read(&app, |events, _ctx| {
                has_event = !events.0.is_empty();
            });
        }

        sync_queue_events.read(&app, |events, _| {
            assert_eq!(
                &events.0[0],
                &SyncQueueEvent::ObjectCreationFailure {
                    reason: CreationFailureReason::Other {
                        id: failure_notebook_id.to_hash(),
                        initiated_by: InitiatedBy::User
                    },
                }
            );
        });

        // The second notebook should not be dequeued.
        sync_queue.read(&app, |queue, _ctx| {
            // The queue will still be in a "dequeueing" state, so it processes new items, but it
            // will not process the existing item.
            assert_eq!(queue.spawned_futures.len(), 1);
            let items = queue.queue().iter().map(|(_, item)| item).collect_vec();
            assert_eq!(items, &[&second_notebook_item]);
        });
    });
}

#[test]
fn test_create_and_update_notebook() {
    App::test((), |mut app| async move {
        let owner = Owner::mock_current_user();
        let notebook_id = ClientId::default();
        let notebook_server_id = ServerId::from(1);
        let notebook_ts = DateTime::<Utc>::default();
        let notebook_revision_after_create = Revision::from(notebook_ts);
        let notebook_revision_after_update =
            Revision::from(notebook_ts + chrono::Duration::minutes(100));
        let notebook_title = "My Notebook".to_string();
        let notebook_data = "Hello".to_string();
        let notebook = CloudNotebookModel {
            title: notebook_title.clone(),
            data: notebook_data.clone(),
            ai_document_id: None,
            conversation_id: None,
        };

        let (tx, rx) = std::sync::mpsc::channel();

        let mut cloud_objects_client_mock = MockObjectClient::new();
        let notebook_revision_after_create_clone = notebook_revision_after_create.clone();
        let notebook_revision_after_update_clone = notebook_revision_after_update.clone();
        cloud_objects_client_mock
            .expect_create_notebook()
            .times(1)
            .return_once(move |_| {
                Ok(CreateCloudObjectResult::Success {
                    created_cloud_object: CreatedCloudObject {
                        client_id: notebook_id,
                        revision_and_editor: RevisionAndLastEditor {
                            revision: notebook_revision_after_create_clone,
                            last_editor_uid: Some("34jkaosdfj".to_string()),
                        },
                        metadata_ts: notebook_ts.into(),
                        server_id_and_type: ServerIdAndType {
                            id: notebook_server_id,
                            id_type: ObjectIdType::Notebook,
                        },
                        creator_uid: Some("34jkaosdfj".to_string()),
                        permissions: ServerPermissions::mock_personal(),
                    },
                })
            });
        cloud_objects_client_mock
            .expect_update_notebook()
            .times(1)
            .return_once(move |_, _, _, _| {
                rx.recv().unwrap();
                Ok(UpdateCloudObjectResult::Success {
                    revision_and_editor: RevisionAndLastEditor {
                        revision: notebook_revision_after_update_clone,
                        last_editor_uid: Some("34jkaosdfj".to_string()),
                    },
                })
            });

        initialize_app(&mut app);

        let sync_queue = create_sync_queue(
            &mut app,
            vec![
                QueueItem::CreateObject {
                    object_type: ObjectType::Notebook,
                    owner,
                    id: notebook_id,
                    title: None,
                    serialized_model: Some(Arc::new(notebook.serialized())),
                    initial_folder_id: None,
                    entrypoint: CloudObjectEventEntrypoint::Unknown,
                    initiated_by: InitiatedBy::User,
                },
                QueueItem::UpdateNotebook {
                    model: CloudNotebookModel {
                        title: notebook_title,
                        data: notebook_data,
                        ai_document_id: None,
                        conversation_id: None,
                    }
                    .into(),
                    id: SyncId::ClientId(notebook_id),
                    revision: Some(notebook_revision_after_create),
                },
            ],
            cloud_objects_client_mock,
            true,
        );
        let events = app.add_model(|_ctx| Events::default());

        events.update(&mut app, |_, ctx| {
            ctx.subscribe_to_model(&sync_queue, |me, event, _ctx| me.0.push(event.clone()))
        });

        sync_queue
            .update(&mut app, |queue, ctx| {
                queue.start_dequeueing(ctx);
                ctx.await_spawned_future(queue.spawned_futures[0])
            })
            .await;

        // Assert we have received one event and that it corresponds to a notebook creation.
        events.read(&app, |events, _ctx| {
            assert_eq!(events.0.len(), 1);
            assert_eq!(
                events[0],
                SyncQueueEvent::ObjectCreationSuccessful {
                    client_id: notebook_id,
                    revision_and_editor: RevisionAndLastEditor {
                        revision: notebook_ts.into(),
                        last_editor_uid: Some("34jkaosdfj".to_string()),
                    },
                    metadata_ts: notebook_ts.into(),
                    server_creation_info: ServerCreationInfo {
                        server_id_and_type: ServerIdAndType {
                            id: notebook_server_id,
                            id_type: ObjectIdType::Notebook,
                        },
                        creator_uid: Some("34jkaosdfj".to_string()),
                        permissions: ServerPermissions::mock_personal(),
                    },
                    initiated_by: InitiatedBy::User
                }
            );
        });

        tx.send(()).unwrap();
        sync_queue
            .update(&mut app, |item, ctx| {
                ctx.await_spawned_future(item.spawned_futures[1])
            })
            .await;

        events.update(&mut app, |sync_queue_events, _ctx| {
            assert_eq!(sync_queue_events.0.len(), 2);
            assert_eq!(
                sync_queue_events[1],
                SyncQueueEvent::ObjectUpdateSuccessful {
                    server_id: notebook_server_id,
                    revision_and_editor: RevisionAndLastEditor {
                        revision: notebook_revision_after_update,
                        last_editor_uid: Some("34jkaosdfj".to_string()),
                    },
                }
            );
        });

        sync_queue.read(&app, |sync_queue, _ctx| {
            // There should be no more items in the queue.
            assert!(sync_queue.queue.is_empty());
        });
    });
}

// regression test for https://linear.app/warpdotdev/issue/CLD-571
#[test]
fn test_initial_queue_items_processed_properly() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let cloud_objects_client_mock = MockObjectClient::new();
        // this is how the sync queue will look when we have 3 pending items from sqlite, but the
        // call to initial load failed, so nothing called `start_dequeueing()`
        let sync_queue = create_sync_queue(
            &mut app,
            vec![
                QueueItem::CreateObject {
                    object_type: ObjectType::Folder,
                    serialized_model: Some(Arc::new("folder 1".to_string().into())),
                    owner: Owner::mock_current_user(),
                    id: ClientId::new(),
                    initial_folder_id: None,
                    entrypoint: Default::default(),
                    title: None,
                    initiated_by: InitiatedBy::User,
                },
                QueueItem::CreateObject {
                    object_type: ObjectType::Folder,
                    serialized_model: Some(Arc::new("folder 2".to_string().into())),
                    owner: Owner::mock_current_user(),
                    id: ClientId::new(),
                    initial_folder_id: None,
                    entrypoint: Default::default(),
                    title: None,
                    initiated_by: InitiatedBy::User,
                },
                QueueItem::CreateObject {
                    object_type: ObjectType::Folder,
                    serialized_model: Some(Arc::new("folder 3".to_string().into())),
                    owner: Owner::mock_current_user(),
                    id: ClientId::new(),
                    initial_folder_id: None,
                    entrypoint: Default::default(),
                    title: None,
                    initiated_by: InitiatedBy::User,
                },
            ],
            cloud_objects_client_mock,
            false,
        );

        sync_queue.update(&mut app, |sync_queue, ctx| {
            sync_queue.enqueue(
                QueueItem::CreateObject {
                    object_type: ObjectType::Folder,
                    serialized_model: Some(Arc::new("folder 4".to_string().into())),
                    id: ClientId::new(),
                    owner: Owner::mock_current_user(),
                    title: None,
                    initial_folder_id: None,
                    entrypoint: Default::default(),
                    initiated_by: InitiatedBy::User,
                },
                ctx,
            );
        });

        sync_queue.update(&mut app, |sync_queue, _ctx| {
            // enqueueing a 4th item shouldn't dequeue any items
            assert_eq!(sync_queue.spawned_futures().len(), 0);
            assert_eq!(sync_queue.queue().len(), 4);
        });
    })
}

#[test]
fn test_record_object_action() {
    App::test((), |mut app| async move {
        let timestamp = Utc::now();
        let timestamp_old = timestamp - Duration::minutes(10);
        let timestamp_older = timestamp - Duration::minutes(20);
        let timestamp_oldest = timestamp - Duration::minutes(30);
        let workflow_id = "0000watermelonchestnut".to_string();
        let workflow_id_clone = workflow_id.clone();
        let hashed_sqlite_id = SyncId::ServerId(ServerId::from_string_lossy(&workflow_id))
            .sqlite_uid_hash(ObjectIdType::Workflow);
        let hashed_sqlite_id_clone = hashed_sqlite_id.clone();

        let mut cloud_objects_client_mock = MockObjectClient::new();
        cloud_objects_client_mock
            .expect_record_object_action()
            .times(1)
            .returning(move |_, _, _, _| {
                Ok(ObjectActionHistory {
                    uid: workflow_id_clone.clone(),
                    hashed_sqlite_id: hashed_sqlite_id_clone.clone(),
                    latest_processed_at_timestamp: timestamp,
                    actions: vec![
                        // One action that occurred just now
                        ObjectAction {
                            action_type: ObjectActionType::Execute,
                            uid: workflow_id_clone.clone(),
                            hashed_sqlite_id: hashed_sqlite_id_clone.clone(),
                            action_subtype: ObjectActionSubtype::SingleAction {
                                timestamp,
                                processed_at_timestamp: Some(timestamp),
                                data: None,
                                pending: false,
                            },
                        },
                        // One action that occurred a lil bit ago
                        ObjectAction {
                            action_type: ObjectActionType::Execute,
                            uid: workflow_id_clone.clone(),
                            hashed_sqlite_id: hashed_sqlite_id_clone.clone(),
                            action_subtype: ObjectActionSubtype::SingleAction {
                                timestamp: timestamp_old,
                                processed_at_timestamp: Some(timestamp_old),
                                data: None,
                                pending: false,
                            },
                        },
                        // A bundle of 10 actions from a lil while ago
                        ObjectAction {
                            action_type: ObjectActionType::Execute,
                            uid: workflow_id_clone.clone(),
                            hashed_sqlite_id: hashed_sqlite_id_clone.clone(),
                            action_subtype: ObjectActionSubtype::BundledActions {
                                latest_timestamp: timestamp_older,
                                oldest_timestamp: timestamp_oldest,
                                count: 10,
                                latest_processed_at_timestamp: timestamp_older,
                            },
                        },
                    ],
                })
            });

        initialize_app(&mut app);
        let sync_queue = create_sync_queue(&mut app, vec![], cloud_objects_client_mock, true);
        let sync_queue_events = app.add_model(|_ctx| Events::default());

        sync_queue_events.update(&mut app, |_, ctx| {
            ctx.subscribe_to_model(&sync_queue, |me, event, _ctx| me.0.push(event.clone()))
        });

        // Enqueue a single item and wait for the response to complete.
        sync_queue
            .update(&mut app, |item, ctx| {
                item.enqueue(
                    QueueItem::RecordObjectAction {
                        id_and_type: CloudObjectTypeAndId::Workflow(SyncId::ServerId(
                            ServerId::from_string_lossy(&workflow_id),
                        )),
                        action_type: ObjectActionType::Execute,
                        action_timestamp: timestamp,
                        data: None,
                    },
                    ctx,
                );

                ctx.await_spawned_future(item.spawned_futures[0])
            })
            .await;

        let uid = SyncId::ServerId(ServerId::from_string_lossy(&workflow_id)).uid();
        sync_queue_events.update(&mut app, |sync_queue_events, _ctx| {
            assert_eq!(sync_queue_events.0.len(), 1);
            assert_eq!(
                sync_queue_events[0],
                SyncQueueEvent::ReportObjectActionSucceeded {
                    uid: uid.clone(),
                    action_timestamp: timestamp,
                    action_history: ObjectActionHistory {
                        uid: uid.clone(),
                        hashed_sqlite_id: hashed_sqlite_id.clone(),
                        latest_processed_at_timestamp: timestamp,
                        actions: vec![
                            ObjectAction {
                                action_type: ObjectActionType::Execute,
                                uid: uid.clone(),
                                hashed_sqlite_id: hashed_sqlite_id.clone(),
                                action_subtype: ObjectActionSubtype::SingleAction {
                                    timestamp,
                                    processed_at_timestamp: Some(timestamp),
                                    data: None,
                                    pending: false
                                }
                            },
                            ObjectAction {
                                action_type: ObjectActionType::Execute,
                                uid: uid.clone(),
                                hashed_sqlite_id: hashed_sqlite_id.clone(),
                                action_subtype: ObjectActionSubtype::SingleAction {
                                    timestamp: timestamp_old,
                                    processed_at_timestamp: Some(timestamp_old),
                                    data: None,
                                    pending: false
                                }
                            },
                            ObjectAction {
                                action_type: ObjectActionType::Execute,
                                uid: uid.clone(),
                                hashed_sqlite_id: hashed_sqlite_id.clone(),
                                action_subtype: ObjectActionSubtype::BundledActions {
                                    latest_timestamp: timestamp_older,
                                    oldest_timestamp: timestamp_oldest,
                                    count: 10,
                                    latest_processed_at_timestamp: timestamp_older,
                                }
                            }
                        ]
                    }
                }
            );
        })
    });
}

#[test]
fn test_sync_queue_dependency_successes() {
    // Create a client ID for a workflow
    let workflow_client_id = ClientId::new();

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let cloud_objects_client_mock = MockObjectClient::new();
        let sync_queue = create_sync_queue(&mut app, vec![], cloud_objects_client_mock, false);

        sync_queue.update(&mut app, |sync_queue, ctx| {
            // Enqueue the workflow create request
            let create_id = sync_queue.enqueue(
                QueueItem::CreateWorkflow {
                    object_type: ObjectType::Workflow,
                    owner: Owner::mock_current_user(),
                    id: workflow_client_id,
                    model: Arc::new(CloudWorkflowModel {
                        data: Workflow::new("test".to_string(), "no".to_string()),
                    }),
                    initial_folder_id: None,
                    entrypoint: Default::default(),
                    initiated_by: InitiatedBy::User,
                },
                ctx,
            );

            // Enqueue an unrelated request
            sync_queue.enqueue(
                QueueItem::CreateObject {
                    object_type: ObjectType::Notebook,
                    owner: Owner::mock_current_user(),
                    id: ClientId::new(),
                    title: None,
                    serialized_model: None,
                    initial_folder_id: None,
                    entrypoint: Default::default(),
                    initiated_by: InitiatedBy::User,
                },
                ctx,
            );

            // Enqueue an update to the workflow
            let update_id = sync_queue.enqueue(
                QueueItem::UpdateWorkflow {
                    model: CloudWorkflowModel {
                        data: Workflow::new("hi".to_string(), "no".to_string()),
                    }
                    .into(),
                    id: SyncId::ClientId(workflow_client_id),
                    revision: None,
                },
                ctx,
            );

            // Assert initial state of the queue dependencies
            assert_eq!(
                sync_queue.queue_dependencies().get(&create_id).unwrap(),
                &HashSet::<QueueItemId>::new()
            );
            assert_eq!(
                sync_queue.queue_dependencies().get(&update_id).unwrap(),
                &HashSet::<QueueItemId>::from([create_id])
            );

            // Simulate success of the create request
            sync_queue.handle_dependency_success(&create_id);

            // We should no longer store a dependency on the update request
            assert_eq!(
                sync_queue
                    .queue_dependencies()
                    .get(&update_id)
                    .unwrap()
                    .len(),
                0
            )
        });
    })
}

#[test]
fn test_sync_queue_dependency_failure() {
    App::test((), |mut app| async move {
        // have a notebook and make the update requests fail
        let client_id = ClientId::new();
        let notebook_data = "new data".to_owned();
        let notebook_title = "new title".to_owned();
        let final_notebook_title = "final title".to_string();

        let revision_after_create = Revision::from(DateTime::<Utc>::default());

        let cloud_objects_client_mock = MockObjectClient::new();

        initialize_app(&mut app);

        let sync_queue = create_sync_queue(&mut app, vec![], cloud_objects_client_mock, false);

        let sync_queue_events = app.add_model(|_ctx| Events::default());

        sync_queue_events.update(&mut app, |_, ctx| {
            ctx.subscribe_to_model(&sync_queue, |me, event, _ctx| me.0.push(event.clone()))
        });

        sync_queue.update(&mut app, |sync_queue, ctx| {
            let create_id = sync_queue.enqueue(
                QueueItem::CreateObject {
                    object_type: ObjectType::Notebook,
                    owner: Owner::mock_current_user(),
                    id: client_id,
                    title: None,
                    serialized_model: None,
                    initial_folder_id: None,
                    entrypoint: Default::default(),
                    initiated_by: InitiatedBy::User,
                },
                ctx,
            );

            let update_id = sync_queue.enqueue(
                QueueItem::UpdateNotebook {
                    model: CloudNotebookModel {
                        title: notebook_title,
                        data: notebook_data,
                        ai_document_id: None,
                        conversation_id: None,
                    }
                    .into(),
                    id: SyncId::ClientId(client_id),
                    revision: Some(revision_after_create.clone()),
                },
                ctx,
            );

            let update_id_2 = sync_queue.enqueue(
                QueueItem::UpdateNotebook {
                    model: CloudNotebookModel {
                        title: final_notebook_title.clone(),
                        data: String::new(),
                        ai_document_id: None,
                        conversation_id: None,
                    }
                    .into(),
                    id: SyncId::ClientId(client_id),
                    revision: Some(revision_after_create.clone()),
                },
                ctx,
            );

            // Assert initial state of the queue dependencies
            assert_eq!(
                sync_queue.queue_dependencies().get(&create_id).unwrap(),
                &HashSet::<QueueItemId>::new()
            );
            assert_eq!(
                sync_queue.queue_dependencies().get(&update_id).unwrap(),
                &HashSet::<QueueItemId>::from([create_id])
            );
            assert_eq!(
                sync_queue.queue_dependencies().get(&update_id_2).unwrap(),
                &HashSet::<QueueItemId>::from([create_id, update_id])
            );

            // Simulate failure of the create request
            sync_queue.remove_id_from_queue(&create_id);
            sync_queue.queue_dependencies.remove(&create_id);
            sync_queue.handle_creation_failure_response(
                client_id.to_string(),
                create_id,
                InitiatedBy::User,
                ctx,
            );

            // We should no longer store any dependencies; they should all have failed and the queue should be empty
            assert!(!sync_queue.queue_dependencies().contains_key(&update_id));
            assert!(!sync_queue.queue_dependencies().contains_key(&update_id_2));
            assert_eq!(sync_queue.queue().len(), 0);
            assert!(sync_queue.queue_dependencies().is_empty());
        });

        sync_queue_events.update(&mut app, |sync_queue_events, _ctx| {
            assert_eq!(sync_queue_events.0.len(), 3); // one creation success, two update failures
            assert_eq!(
                sync_queue_events[0],
                SyncQueueEvent::ObjectUpdateFailure {
                    id: SyncId::ClientId(client_id)
                }
            );
            assert_eq!(
                sync_queue_events[1],
                SyncQueueEvent::ObjectUpdateFailure {
                    id: SyncId::ClientId(client_id)
                },
            );
            assert_eq!(
                sync_queue_events[2],
                SyncQueueEvent::ObjectCreationFailure {
                    reason: CreationFailureReason::Other {
                        id: client_id.to_string(),
                        initiated_by: InitiatedBy::User
                    },
                },
            );
        });
    });
}

#[test]
fn test_sync_queue_dependency_mixed_ids() {
    // check dependencies across items referencing client + server ID for the same object

    let client_id = ClientId::new();
    let server_id = SyncId::ServerId(NotebookId::from(123).into());
    let revision_after_create = Revision::from(DateTime::<Utc>::default());

    let notebook_data = "new data".to_owned();
    let notebook_title = "new title".to_owned();
    let final_notebook_title = "final title".to_string();

    App::test((), |mut app| async move {
        let cloud_objects_client_mock = MockObjectClient::new();
        initialize_app(&mut app);
        let sync_queue = create_sync_queue(&mut app, vec![], cloud_objects_client_mock, false);
        let sync_queue_events = app.add_model(|_ctx| Events::default());

        sync_queue_events.update(&mut app, |_, ctx| {
            ctx.subscribe_to_model(&sync_queue, |me, event, _ctx| me.0.push(event.clone()))
        });

        sync_queue.update(&mut app, |sync_queue, ctx| {
            let create_id = sync_queue.enqueue(
                QueueItem::CreateObject {
                    object_type: ObjectType::Notebook,
                    owner: Owner::mock_current_user(),
                    id: client_id,
                    title: None,
                    serialized_model: None,
                    initial_folder_id: None,
                    entrypoint: CloudObjectEventEntrypoint::Unknown,
                    initiated_by: InitiatedBy::User,
                },
                ctx,
            );

            let update_id = sync_queue.enqueue(
                QueueItem::UpdateNotebook {
                    model: CloudNotebookModel {
                        title: notebook_title,
                        data: notebook_data,
                        ai_document_id: None,
                        conversation_id: None,
                    }
                    .into(),
                    id: SyncId::ClientId(client_id),
                    revision: Some(revision_after_create.clone()),
                },
                ctx,
            );

            // Assert initial state of the queue dependencies
            assert_eq!(
                sync_queue.queue_dependencies().get(&create_id).unwrap(),
                &HashSet::<QueueItemId>::new()
            );
            assert_eq!(
                sync_queue.queue_dependencies().get(&update_id).unwrap(),
                &HashSet::<QueueItemId>::from([create_id])
            );

            // Simulate success of create request
            sync_queue.remove_id_from_queue(&create_id);
            sync_queue.queue_dependencies.remove(&create_id);
            sync_queue.handle_success_response(
                &server_id.uid(),
                super::ResponseType::Creation {
                    creation_result: super::CreationResponseType::Success {
                        client_id,
                        revision_and_editor: super::RevisionAndLastEditor {
                            revision: revision_after_create.clone(),
                            last_editor_uid: None,
                        },
                        metadata_ts: DateTime::<Utc>::default().into(),
                        server_creation_info: ServerCreationInfo {
                            server_id_and_type: ServerIdAndType {
                                id: server_id.into_server().expect("Expect server id"),
                                id_type: ObjectIdType::Notebook,
                            },
                            creator_uid: Default::default(),
                            permissions: ServerPermissions::mock_personal(),
                        },
                    },
                },
                create_id,
                InitiatedBy::User,
                ctx,
            );

            assert_eq!(
                sync_queue.queue_dependencies().get(&update_id).unwrap(),
                &HashSet::<QueueItemId>::new()
            );

            let update_id_2 = sync_queue.enqueue(
                QueueItem::UpdateNotebook {
                    model: CloudNotebookModel {
                        title: final_notebook_title.clone(),
                        data: String::new(),
                        ai_document_id: None,
                        conversation_id: None,
                    }
                    .into(),
                    id: server_id,
                    revision: Some(revision_after_create.clone()),
                },
                ctx,
            );

            // Even though one request uses client ID and one uses server ID, we should find a dependency here
            assert_eq!(
                sync_queue.queue_dependencies().get(&update_id_2).unwrap(),
                &HashSet::<QueueItemId>::from([update_id])
            );
        });
    });
}

#[test]
fn test_sync_queue_enum_dependency() {
    let enum_id_1 = ClientId::new();
    let enum_id_2 = ClientId::new();
    let workflow_client_id = ClientId::new();
    let enum_server_id_1 = SyncId::ServerId(GenericStringObjectId::from(123).into());
    let enum_server_id_2 = SyncId::ServerId(GenericStringObjectId::from(456).into());
    let revision_after_create = Revision::from(DateTime::<Utc>::default());

    let workflow = Workflow::new("test".to_string(), "no".to_string()).with_arguments(vec![
        Argument {
            name: "enum".to_string(),
            default_value: None,
            description: None,
            arg_type: ArgumentType::Enum {
                enum_id: SyncId::ClientId(enum_id_1),
            },
        },
        Argument {
            name: "enum".to_string(),
            default_value: None,
            description: None,
            arg_type: ArgumentType::Enum {
                enum_id: SyncId::ClientId(enum_id_2),
            },
        },
    ]);

    let updated_workflow_1 =
        Workflow::new("test".to_string(), "no".to_string()).with_arguments(vec![
            Argument {
                name: "enum".to_string(),
                default_value: None,
                description: None,
                arg_type: ArgumentType::Enum {
                    enum_id: enum_server_id_1,
                },
            },
            Argument {
                name: "enum".to_string(),
                default_value: None,
                description: None,
                arg_type: ArgumentType::Enum {
                    enum_id: SyncId::ClientId(enum_id_2),
                },
            },
        ]);

    let updated_workflow_2 =
        Workflow::new("test".to_string(), "no".to_string()).with_arguments(vec![
            Argument {
                name: "enum".to_string(),
                default_value: None,
                description: None,
                arg_type: ArgumentType::Enum {
                    enum_id: enum_server_id_1,
                },
            },
            Argument {
                name: "enum".to_string(),
                default_value: None,
                description: None,
                arg_type: ArgumentType::Enum {
                    enum_id: enum_server_id_2,
                },
            },
        ]);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let cloud_objects_client_mock = MockObjectClient::new();
        let sync_queue = create_sync_queue(&mut app, vec![], cloud_objects_client_mock, false);

        sync_queue.update(&mut app, |sync_queue, ctx| {
            // Enqueue two enum requests
            let create_enum_1 = sync_queue.enqueue(
                QueueItem::CreateObject {
                    object_type: ObjectType::GenericStringObject(GenericStringObjectFormat::Json(
                        JsonObjectType::WorkflowEnum,
                    )),
                    owner: Owner::mock_current_user(),
                    id: enum_id_1,
                    title: None,
                    serialized_model: None,
                    initial_folder_id: None,
                    entrypoint: Default::default(),
                    initiated_by: InitiatedBy::User
                },
                ctx,
            );

            let create_enum_2 = sync_queue.enqueue(
                QueueItem::CreateObject {
                    object_type: ObjectType::GenericStringObject(GenericStringObjectFormat::Json(
                        JsonObjectType::WorkflowEnum,
                    )),
                    owner: Owner::mock_current_user(),
                    id: enum_id_2,
                    title: None,
                    serialized_model: None,
                    initial_folder_id: None,
                    entrypoint: Default::default(),
                    initiated_by: InitiatedBy::User
                },
                ctx,
            );

            // Simulate success of one enum request
            sync_queue.remove_id_from_queue(&create_enum_1);
            sync_queue.queue_dependencies.remove(&create_enum_1);
            sync_queue.handle_success_response(
                &enum_server_id_1.uid(),
                super::ResponseType::Creation {
                    creation_result: super::CreationResponseType::Success {
                        client_id: enum_id_1,
                        revision_and_editor: super::RevisionAndLastEditor {
                            revision: revision_after_create.clone(),
                            last_editor_uid: None,
                        },
                        metadata_ts: DateTime::<Utc>::default().into(),
                        server_creation_info: ServerCreationInfo {
                            server_id_and_type: ServerIdAndType {
                                id: enum_server_id_1.into_server().expect("Expect server id"),
                                id_type: ObjectIdType::GenericStringObject,
                            },
                            creator_uid: Default::default(),
                            permissions: ServerPermissions::mock_personal(),
                        },
                    },
                },
                create_enum_1,
                InitiatedBy::User,
                ctx,
            );

            // Enqueue the workflow create request
            let create_id = sync_queue.enqueue(
                QueueItem::CreateWorkflow {
                    object_type: ObjectType::Workflow,
                    owner: Owner::mock_current_user(),
                    id: workflow_client_id,
                    model: Arc::new(CloudWorkflowModel { data: workflow }),
                    initial_folder_id: None,
                    entrypoint: Default::default(),
                    initiated_by: InitiatedBy::User
                },
                ctx,
            );

            // Assert initial state of the queue dependencies
            assert_eq!(
                sync_queue.queue_dependencies().get(&create_id).unwrap(),
                &HashSet::<QueueItemId>::from([create_enum_2])
            );

            // Assert that the workflow that was enqueued has the enum_id that is a server id
            assert!(
                sync_queue
                    .queue()
                    .iter()
                    .find_map(|(_, item)| {
                        match item {
                            QueueItem::CreateWorkflow { model, .. } => {
                                Some(model.data == updated_workflow_1)
                            }
                            _ => None,
                        }
                    })
                    .unwrap(),
                "enqueued workflow should have one enum ID replaced by a server ID"
            );

            // Simulate success of another enum
            sync_queue.remove_id_from_queue(&create_enum_2);
            sync_queue.queue_dependencies.remove(&create_enum_2);
            sync_queue.update_dependencies_on_creation(
                &create_enum_2,
                enum_id_2,
                enum_server_id_2.into_server().expect("Expect server id"),
                ObjectType::GenericStringObject(GenericStringObjectFormat::Json(
                    JsonObjectType::WorkflowEnum,
                )),
            );
            sync_queue.handle_success_response(
                &enum_server_id_2.uid(),
                super::ResponseType::Creation {
                    creation_result: super::CreationResponseType::Success {
                        client_id: enum_id_2,
                        revision_and_editor: super::RevisionAndLastEditor {
                            revision: revision_after_create.clone(),
                            last_editor_uid: None,
                        },
                        metadata_ts: DateTime::<Utc>::default().into(),
                        server_creation_info: ServerCreationInfo {
                            server_id_and_type: ServerIdAndType {
                                id: enum_server_id_2.into_server().expect("Expect server id"),
                                id_type: ObjectIdType::GenericStringObject,
                            },
                            creator_uid: Default::default(),
                            permissions: ServerPermissions::mock_personal(),
                        },
                    },
                },
                create_enum_2,
                InitiatedBy::User,
                ctx,
            );

            // Assert updated state of the queue dependencies
            assert_eq!(
                sync_queue.queue_dependencies().get(&create_id).unwrap(),
                &HashSet::<QueueItemId>::new()
            );

            // Assert that the workflow was updated to reference the server ID of the second enum
            assert!(
                sync_queue
                    .queue()
                    .iter()
                    .find_map(|(_, item)| {
                        match item {
                            QueueItem::CreateWorkflow { model, .. } => {
                                Some(model.data == updated_workflow_2)
                            }
                            _ => None,
                        }
                    })
                    .unwrap(),
                "After completing an enum creation, the enqueued workflow should only reference server IDs"
            );
        });
    })
}
