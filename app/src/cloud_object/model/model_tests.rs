use chrono::Utc;
use lazy_static::lazy_static;
use settings::{RespectUserSyncSetting, SyncToCloud};
use warpui::{App, ModelHandle};

use crate::auth::auth_manager::AuthManager;
use crate::auth::user::TEST_USER_UID;
use crate::auth::AuthStateProvider;
use crate::auth::UserUid;
use crate::cloud_object::model::actions::ObjectActions;
use crate::cloud_object::model::generic_string_model::GenericStringModel;
use crate::cloud_object::model::view::CloudViewModel;
use crate::cloud_object::model::view::EditorState;
use crate::cloud_object::model::view::UpdateTimestamp;
use crate::cloud_object::model::view::EDITOR_TIMEOUT_DURATION_MINUTES;
use crate::cloud_object::CloudObjectMetadata;
use crate::cloud_object::CloudObjectPermissions;
use crate::cloud_object::CloudObjectStatuses;
use crate::cloud_object::CloudObjectSyncStatus;
use crate::cloud_object::NumInFlightRequests;
use crate::cloud_object::ObjectIdType;
use crate::cloud_object::Owner;
use crate::cloud_object::ServerMetadata;
use crate::cloud_object::ServerPermissions;
use crate::drive::folders::CloudFolderModel;
use crate::drive::folders::FolderId;
use crate::drive::DriveIndexVariant;
use crate::features::FeatureFlag;
use crate::notebooks::CloudNotebookModel;
use crate::notebooks::NotebookId;
use crate::server::cloud_objects::listener::ObjectUpdateMessage;
use crate::server::cloud_objects::update_manager::InitialLoadResponse;
use crate::server::ids::ServerId;
use crate::server::ids::ServerIdAndType;
use crate::server::server_api::object::ObjectClient;
use crate::server::server_api::ServerApiProvider;
use crate::server::sync_queue::SyncQueue;
use crate::server::telemetry::context_provider::AppTelemetryContextProvider;
use crate::settings::init_and_register_user_preferences;
use crate::settings::Preference;
use crate::system::SystemStats;
use crate::workspaces::team::Team;
use crate::workspaces::team_tester::TeamTesterStatus;
use crate::workspaces::user_profiles::UserProfiles;
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::workspaces::workspace::Workspace;

use crate::workflows::CloudWorkflowModel;
use crate::workspaces::workspace::WorkspaceUid;
use crate::NetworkStatus;
use crate::UpdateManager;
use mockall::Sequence;
use rand::Rng;
use std::sync::Arc;
use std::time::Duration;

use super::*;

#[cfg(test)]
use crate::server::server_api::object::MockObjectClient;
#[cfg(test)]
use crate::server::server_api::team::MockTeamClient;
#[cfg(test)]
use crate::server::server_api::workspace::MockWorkspaceClient;

fn create_cloud_model(
    app: &mut App,
    objects: Vec<Box<dyn CloudObject>>,
) -> ModelHandle<CloudModel> {
    // Make sure to register the CloudModel singleton - some CloudObject methods
    // find it and other dependencies via the AppContext.
    app.add_singleton_model(|_ctx| CloudModel::new(None, objects, None))
}

lazy_static! {
    /// Mock the user being on _a_ team in tests, so that the team drive is available.
    /// Otherwise, any team objects will appear shared.
    static ref TEST_TEAM: Team = Team::from_local_cache(
        ServerId::from(1),
        "Test Team".to_string(),
        None,
        None,
        None,
    );

    static ref TEST_WORKSPACE: Workspace = Workspace::from_local_cache(
        WorkspaceUid::from(ServerId::from(1)),
        "Test Workspace".to_string(),
        Some(vec![TEST_TEAM.clone()]),
    );
}

fn initialize_app(
    app: &mut App,
    cached_objects: Vec<Box<dyn CloudObject>>,
    cloud_object_server_api_mock: Arc<impl ObjectClient>,
) {
    let team_client_mock = Arc::new(MockTeamClient::new());
    let workspace_client_mock = Arc::new(MockWorkspaceClient::new());

    // Add the necessary singleton models to the App
    app.add_singleton_model(|_| NetworkStatus::new());
    app.add_singleton_model(|_| SystemStats::new());
    app.add_singleton_model(|_| ServerApiProvider::new_for_test());
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(AppTelemetryContextProvider::new_context_provider);
    app.add_singleton_model(AuthManager::new_for_test);
    app.add_singleton_model(|ctx| {
        UserWorkspaces::mock(
            team_client_mock.clone(),
            workspace_client_mock.clone(),
            vec![TEST_WORKSPACE.clone()],
            ctx,
        )
    });
    app.add_singleton_model(TeamTesterStatus::new);
    app.add_singleton_model(SyncQueue::mock);
    app.add_singleton_model(|_ctx| CloudModel::new(None, cached_objects, None));
    app.add_singleton_model(|ctx| UpdateManager::new(None, cloud_object_server_api_mock, ctx));
    app.add_singleton_model(|_| UserProfiles::new(Vec::new()));
    app.add_singleton_model(CloudViewModel::new);
    app.add_singleton_model(|_| ObjectActions::new(Vec::new()));

    // The start of polling is normally triggered by authentication completion, but
    // we need to do it manually for tests.
    TeamTesterStatus::handle(app).update(app, |team_tester, ctx| {
        team_tester.initiate_data_pollers(false, ctx);
    });
}

fn mock_random_workflows(start_id: i64, owner: Owner) -> Vec<ServerWorkflow> {
    let mut rng = rand::thread_rng();
    // pick how many workflows to generate at random
    let number_of_workflows = rng.gen_range(1..10);
    mock_server_workflows(start_id, owner, number_of_workflows)
}

fn mock_server_metadata() -> ServerMetadata {
    ServerMetadata {
        uid: ServerId::default(),
        revision: Revision::now(),
        metadata_last_updated_ts: Utc::now().into(),
        trashed_ts: None,
        folder_id: None,
        is_welcome_object: false,
        creator_uid: None,
        last_editor_uid: None,
        current_editor_uid: None,
    }
}

fn mock_server_permissions(owner: Owner) -> ServerPermissions {
    ServerPermissions {
        space: owner,
        guests: Vec::new(),
        permissions_last_updated_ts: Utc::now().into(),
        anyone_link_sharing: None,
    }
}

fn mock_permissions() -> CloudObjectPermissions {
    CloudObjectPermissions {
        owner: Owner::mock_current_user(),
        guests: Vec::new(),
        permissions_last_updated_ts: None,
        anyone_with_link: None,
    }
}

fn mock_server_workflows(
    start_id: i64,
    owner: Owner,
    number_of_workflows: i64,
) -> Vec<ServerWorkflow> {
    (0..number_of_workflows)
        .map(|idx| ServerWorkflow {
            id: SyncId::ServerId((start_id + idx).into()),
            metadata: mock_server_metadata(),
            permissions: mock_server_permissions(owner),
            model: CloudWorkflowModel::new(Workflow::new(
                format!("w{}", start_id + idx),
                format!("c{}", start_id + idx),
            )),
        })
        .collect()
}

fn mock_random_folders(start_id: i64, owner: Owner) -> Vec<ServerFolder> {
    let mut rng = rand::thread_rng();
    // pick how many folders to generate at random
    let number_of_workflows = rng.gen_range(1..10);
    mock_server_folders(start_id, owner, number_of_workflows)
}

fn mock_server_folders(start_id: i64, owner: Owner, number_of_folders: i64) -> Vec<ServerFolder> {
    (0..number_of_folders)
        .map(|idx| ServerFolder {
            id: SyncId::ServerId((start_id + idx).into()),
            model: CloudFolderModel::new(&format!("f{}", start_id + idx), false),
            metadata: mock_server_metadata(),
            permissions: mock_server_permissions(owner),
        })
        .collect()
}

fn mock_server_notebooks() -> Vec<ServerNotebook> {
    let owner = Owner::mock_current_user();
    vec![
        ServerNotebook {
            id: SyncId::ServerId(1.into()),
            model: CloudNotebookModel {
                title: "t1".to_string(),
                data: "d1".to_string(),
                ai_document_id: None,
                conversation_id: None,
            },
            metadata: mock_server_metadata(),
            permissions: mock_server_permissions(owner),
        },
        ServerNotebook {
            id: SyncId::ServerId(2.into()),
            model: CloudNotebookModel {
                title: "t2".to_string(),
                data: "d2".to_string(),
                ai_document_id: None,
                conversation_id: None,
            },
            metadata: mock_server_metadata(),
            permissions: mock_server_permissions(owner),
        },
        ServerNotebook {
            id: SyncId::ServerId(3.into()),
            model: CloudNotebookModel {
                title: "t3".to_string(),
                data: "d3".to_string(),
                ai_document_id: None,
                conversation_id: None,
            },
            metadata: mock_server_metadata(),
            permissions: mock_server_permissions(owner),
        },
        ServerNotebook {
            id: SyncId::ServerId(4.into()),
            model: CloudNotebookModel {
                title: "t4".to_string(),
                data: "d4".to_string(),
                ai_document_id: None,
                conversation_id: None,
            },
            metadata: mock_server_metadata(),
            permissions: mock_server_permissions(owner),
        },
    ]
}

fn mock_cloud_folder(id: SyncId, name: String, folder_id: Option<SyncId>) -> CloudFolder {
    CloudFolder::new(
        id,
        CloudFolderModel {
            name,
            is_open: true,
            is_warp_pack: false,
        },
        CloudObjectMetadata {
            pending_changes_statuses: CloudObjectStatuses {
                content_sync_status: CloudObjectSyncStatus::NoLocalChanges,
                has_pending_metadata_change: false,
                has_pending_permissions_change: false,
                pending_untrash: false,
                pending_delete: false,
            },
            folder_id,
            revision: Default::default(),
            metadata_last_updated_ts: Default::default(),
            current_editor_uid: Default::default(),
            trashed_ts: Default::default(),
            is_welcome_object: false,
            creator_uid: None,
            last_editor_uid: None,
            last_task_run_ts: None,
        },
        mock_permissions(),
    )
}

fn mock_cloud_notebook(id: SyncId, title: String, folder_id: Option<SyncId>) -> CloudNotebook {
    CloudNotebook::new(
        id,
        CloudNotebookModel {
            title,
            data: "test".into(),
            ai_document_id: None,
            conversation_id: None,
        },
        CloudObjectMetadata {
            pending_changes_statuses: CloudObjectStatuses {
                content_sync_status: CloudObjectSyncStatus::NoLocalChanges,
                has_pending_metadata_change: false,
                has_pending_permissions_change: false,
                pending_untrash: false,
                pending_delete: false,
            },
            folder_id,
            revision: Default::default(),
            metadata_last_updated_ts: Default::default(),
            current_editor_uid: Default::default(),
            trashed_ts: Default::default(),
            is_welcome_object: false,
            creator_uid: None,
            last_editor_uid: None,
            last_task_run_ts: None,
        },
        mock_permissions(),
    )
}

fn mock_trashed_cloud_folder(id: SyncId, name: String, folder_id: Option<SyncId>) -> CloudFolder {
    let mut folder = mock_cloud_folder(id, name, folder_id);
    folder.metadata.trashed_ts = Some(ServerTimestamp::from_unix_timestamp_micros(10).unwrap());
    folder
}

fn folder_from_cloud_model(model: &CloudModel, id: SyncId) -> &CloudFolder {
    model.get_folder_by_uid(&id.uid()).expect("is a folder")
}

/// Mock receiving an RTC update. These tests update objects by mocking RTC messages so that they
/// don't need to mock the server API for updates. The unit tests for [`UpdateManager`] ensure that
/// updates from both RTC and client actions emit the same events.
fn receive_rtc_update(message: ObjectUpdateMessage, app: &mut App) {
    UpdateManager::handle(app).update(app, |update_manager, ctx| {
        update_manager.received_message_from_server(message, ctx)
    });
}

fn move_object(id: ServerId, folder_id: Option<FolderId>, app: &mut App) {
    let message = CloudModel::handle(app).read(app, |cloud_model, _| {
        let object = cloud_model
            .get_by_uid(&id.uid())
            .expect("Expected object to exist in cloud model");

        let metadata = ServerMetadata {
            uid: id,
            revision: object
                .metadata()
                .revision
                .clone()
                .expect("Revision is required"),
            current_editor_uid: object.metadata().current_editor_uid.clone(),
            metadata_last_updated_ts: (object
                .metadata()
                .metadata_last_updated_ts
                .expect("Metadata TS is required")
                .utc()
                + chrono::Duration::seconds(1))
            .into(),
            trashed_ts: object.metadata().trashed_ts,
            folder_id,
            is_welcome_object: object.metadata().is_welcome_object,
            creator_uid: object.metadata().creator_uid.clone(),
            last_editor_uid: object.metadata().last_editor_uid.clone(),
        };

        ObjectUpdateMessage::ObjectMetadataChanged { metadata }
    });

    receive_rtc_update(message, app);
}

#[test]
fn test_update_with_deleted_objects() {
    let workflows = mock_server_workflows(
        5,
        Owner::Team {
            team_uid: ServerId::from(1),
        },
        3,
    );
    let notebooks = mock_server_notebooks();

    App::test((), |mut app| async move {
        let cloud_model = create_cloud_model(
            &mut app,
            workflows
                .iter()
                .map(|workflow| CloudWorkflow::new_from_server(workflow.clone()))
                .map(|o| Box::new(o) as Box<dyn CloudObject>)
                .collect(),
        );
        cloud_model.update(&mut app, |model, ctx| {
            for notebook in notebooks.clone() {
                model.upsert_from_server_notebook(notebook, ctx);
            }
        });

        // Validate there's some notebooks and workflows in memory
        cloud_model.read(&app, |cloud_model, _| {
            assert_eq!(
                3,
                cloud_model.get_all_active_and_inactive_workflows().count()
            );
            assert_eq!(
                4,
                cloud_model.get_all_active_and_inactive_notebooks().count()
            );
            assert_eq!(7, cloud_model.as_cloud_objects().count());
        });

        // Apply the "update from server"
        cloud_model.update(&mut app, |cloud_model, ctx| {
            // Set 3rd notebook to have pending changes. This should keep it in memory,
            // even though it's not returned from the server.
            let notebook_id: SyncId = SyncId::ServerId(3.into());
            if let Some(object) = cloud_model.get_notebook_mut(&notebook_id) {
                object.set_pending_content_changes_status(CloudObjectSyncStatus::InFlight(
                    NumInFlightRequests(1),
                ));
            }
            cloud_model.update_objects(notebooks.into_iter().take(2), ctx);
            cloud_model.update_objects(workflows.into_iter().take(2), ctx);
        });

        cloud_model.read(&app, |cloud_model, _| {
            // expected: 3rd workflow was removed on the server, and so we don't want it in
            // memory
            assert_eq!(
                2,
                cloud_model.get_all_active_and_inactive_workflows().count()
            );
            // expected: 3rd notebook has local changes, so we want to keep it, but 4th
            // doesn't and also wasn't returned from the server, so we want to remove it.
            assert_eq!(
                3,
                cloud_model.get_all_active_and_inactive_notebooks().count()
            );
            assert_eq!(5, cloud_model.as_cloud_objects().count());
        });
    })
}

#[test]
fn test_update_object_server_id_for_notebook() {
    let client_id = ClientId::new();
    let server_id: NotebookId = 1.into();
    let notebooks: Vec<Box<dyn CloudObject>> = vec![Box::new(CloudNotebook::new(
        SyncId::ClientId(client_id),
        CloudNotebookModel {
            title: "t1".to_string(),
            data: "d1".to_string(),
            ai_document_id: None,
            conversation_id: None,
        },
        CloudObjectMetadata {
            pending_changes_statuses: CloudObjectStatuses {
                content_sync_status: CloudObjectSyncStatus::NoLocalChanges,
                has_pending_metadata_change: false,
                has_pending_permissions_change: false,
                pending_untrash: false,
                pending_delete: false,
            },
            folder_id: Default::default(),
            revision: Default::default(),
            metadata_last_updated_ts: Default::default(),
            current_editor_uid: Default::default(),
            trashed_ts: Default::default(),
            is_welcome_object: false,
            creator_uid: None,
            last_editor_uid: None,
            last_task_run_ts: None,
        },
        mock_permissions(),
    ))];

    App::test((), |mut app| async move {
        let cloud_model = create_cloud_model(&mut app, notebooks);
        cloud_model.update(&mut app, |model, ctx| {
            model.update_object_after_server_creation(
                client_id,
                ServerCreationInfo {
                    creator_uid: None,
                    permissions: ServerPermissions::mock_personal(),
                    server_id_and_type: ServerIdAndType {
                        id: server_id.to_server_id(),
                        id_type: ObjectIdType::Notebook,
                    },
                },
                ctx,
            )
        });

        cloud_model.read(&app, |model, _| {
            let notebook = model
                .get_notebook(&SyncId::ServerId(server_id.into()))
                .unwrap();
            assert_eq!(notebook.id, SyncId::ServerId(server_id.into()));
        });
    })
}

#[test]
fn test_create_json_object() {
    let client_id = ClientId::default();
    let id = SyncId::ClientId(client_id);
    let json_object: Box<dyn CloudObject> = Box::new(CloudPreference::new(
        id,
        GenericStringModel::new(
            Preference::new(
                "test_storage_key".to_owned(),
                "{\"test_key\": \"test_value\"}",
                SyncToCloud::Globally(RespectUserSyncSetting::Yes),
            )
            .expect("error creating preference"),
        ),
        CloudObjectMetadata {
            pending_changes_statuses: CloudObjectStatuses {
                content_sync_status: CloudObjectSyncStatus::NoLocalChanges,
                has_pending_metadata_change: false,
                has_pending_permissions_change: false,
                pending_untrash: false,
                pending_delete: false,
            },
            folder_id: Default::default(),
            revision: Default::default(),
            metadata_last_updated_ts: Default::default(),
            current_editor_uid: Default::default(),
            trashed_ts: Default::default(),
            is_welcome_object: false,
            creator_uid: None,
            last_editor_uid: None,
            last_task_run_ts: None,
        },
        mock_permissions(),
    ));

    App::test((), |mut app| async move {
        let cloud_model = create_cloud_model(&mut app, vec![json_object]);
        cloud_model.read(&app, |model, _| {
            let json_object: &CloudPreference =
                model.get_object_of_type(&id).expect("model should exist");
            assert_eq!(
                json_object.model().string_model.storage_key,
                "test_storage_key".to_owned()
            );
        });
    })
}

#[test]
fn test_update_object_server_id_for_workflow() {
    let client_id = ClientId::new();
    let server_id: ServerId = 1.into();
    let workflows: Vec<Box<dyn CloudObject>> = vec![Box::new(CloudWorkflow::new(
        SyncId::ServerId(1.into()),
        CloudWorkflowModel::new(Workflow::new("w1", "c1")),
        CloudObjectMetadata {
            pending_changes_statuses: CloudObjectStatuses {
                content_sync_status: CloudObjectSyncStatus::NoLocalChanges,
                has_pending_metadata_change: false,
                has_pending_permissions_change: false,
                pending_untrash: false,
                pending_delete: false,
            },
            folder_id: Default::default(),
            revision: Default::default(),
            metadata_last_updated_ts: Default::default(),
            current_editor_uid: Default::default(),
            trashed_ts: Default::default(),
            is_welcome_object: false,
            creator_uid: None,
            last_editor_uid: None,
            last_task_run_ts: None,
        },
        mock_permissions(),
    ))];
    App::test((), |mut app| async move {
        let cloud_model = create_cloud_model(&mut app, workflows);
        cloud_model.update(&mut app, |model, ctx| {
            model.update_object_after_server_creation(
                client_id,
                ServerCreationInfo {
                    creator_uid: None,
                    permissions: ServerPermissions::mock_personal(),
                    server_id_and_type: ServerIdAndType {
                        id: server_id,
                        id_type: ObjectIdType::Workflow,
                    },
                },
                ctx,
            )
        });

        cloud_model.read(&app, |model, _| {
            let workflow = model.get_workflow(&SyncId::ServerId(server_id)).unwrap();
            assert_eq!(workflow.id, SyncId::ServerId(server_id));
        });
    })
}

#[test]
fn test_update_object_server_id_for_folder() {
    let client_id = ClientId::new();
    let server_id: FolderId = 1.into();
    let folders: Vec<Box<dyn CloudObject>> = vec![Box::new(CloudFolder::new(
        SyncId::ServerId(1.into()),
        CloudFolderModel::new("test", false),
        CloudObjectMetadata {
            pending_changes_statuses: CloudObjectStatuses {
                content_sync_status: CloudObjectSyncStatus::NoLocalChanges,
                has_pending_metadata_change: false,
                has_pending_permissions_change: false,
                pending_untrash: false,
                pending_delete: false,
            },
            folder_id: Default::default(),
            revision: Default::default(),
            metadata_last_updated_ts: Default::default(),
            current_editor_uid: Default::default(),
            trashed_ts: Default::default(),
            is_welcome_object: false,
            creator_uid: None,
            last_editor_uid: None,
            last_task_run_ts: None,
        },
        mock_permissions(),
    ))];
    App::test((), |mut app| async move {
        let cloud_model = create_cloud_model(&mut app, folders);
        cloud_model.update(&mut app, |model, ctx| {
            model.update_object_after_server_creation(
                client_id,
                ServerCreationInfo {
                    creator_uid: None,
                    permissions: ServerPermissions::mock_personal(),
                    server_id_and_type: ServerIdAndType {
                        id: server_id.to_server_id(),
                        id_type: ObjectIdType::Folder,
                    },
                },
                ctx,
            )
        });

        cloud_model.read(&app, |model, _| {
            let folder = model
                .get_folder_by_uid(&SyncId::ServerId(server_id.into()).uid())
                .unwrap();
            assert_eq!(folder.id, SyncId::ServerId(server_id.into()));
        });
    })
}

fn base_mock_cloud_object_server_api() -> MockObjectClient {
    MockObjectClient::new()
}

fn check_cloud_folders(app: &mut App, number_of_folders: usize) {
    CloudModel::handle(app).read(app, |model, _| {
        assert_eq!(
            number_of_folders,
            model.get_all_active_and_inactive_folders().count(),
            "we expected {} folders, and received {}",
            number_of_folders,
            model.get_all_active_and_inactive_folders().count()
        );
    });
}

fn check_cloud_workflows(app: &mut App, number_of_workflows: usize) {
    CloudModel::handle(app).read(app, |model, _| {
        assert_eq!(
            number_of_workflows,
            model.get_all_active_and_inactive_workflows().count(),
            "we expected {} workflows, and received {}",
            number_of_workflows,
            model.get_all_active_and_inactive_workflows().count()
        );
    });
}

fn check_cloud_notebooks(app: &mut App, number_of_notebooks: usize) {
    CloudModel::handle(app).read(app, |model, _| {
        assert_eq!(
            number_of_notebooks,
            model.get_all_active_and_inactive_notebooks().count(),
            "we expected {} notebooks, and received {}",
            number_of_notebooks,
            model.get_all_active_and_inactive_notebooks().count()
        );
    });
}

#[test]
/// This test validates the behavior when the CloudModel has been initiated with empty values (ie.
/// empty sqlite / local cache). We check that after initialization, we fetch the values and
/// successfully update CloudModel.
// TODO followup with a test equivalent for when the user is not team tester but still has teams?
fn test_load_cloud_objects_on_initial_load_with_empty_cache() {
    let _flag = FeatureFlag::KnowledgeSidebar.override_enabled(true);

    let personal_workflows = mock_random_workflows(100, Owner::mock_current_user());
    let personal_workflows_len = personal_workflows.len();
    let team_workflows = mock_random_workflows(
        200,
        Owner::Team {
            team_uid: ServerId::from(1),
        },
    );
    let team_workflows_len = team_workflows.len();
    let personal_folders = mock_random_folders(300, Owner::mock_current_user());
    let personal_folders_len = personal_folders.len();
    let team_folders = mock_random_folders(
        400,
        Owner::Team {
            team_uid: ServerId::from(1),
        },
    );
    let team_folders_len = team_folders.len();
    let notebooks = mock_server_notebooks();
    let notebooks_len = notebooks.len();
    let all_workflows = [personal_workflows, team_workflows].concat();
    let all_folders = [personal_folders, team_folders].concat();

    App::test((), |mut app| async move {
        // Setup the app and APIs
        let mut cloud_object_server_api_mock = base_mock_cloud_object_server_api();
        cloud_object_server_api_mock
            .expect_fetch_changed_objects()
            .times(1)
            // since we don't have anything cached, at startup we just send empty list here
            .withf(|objects_to_update, _| {
                objects_to_update.notebooks.is_empty()
                    && objects_to_update.workflows.is_empty()
                    && objects_to_update.folders.is_empty()
                    && objects_to_update.generic_string_objects.is_empty()
            })
            .return_once(move |_, _| {
                Ok(InitialLoadResponse {
                    updated_notebooks: notebooks,
                    updated_workflows: all_workflows,
                    updated_folders: all_folders,
                    deleted_notebooks: vec![],
                    deleted_workflows: vec![],
                    deleted_folders: vec![],
                    user_profiles: vec![],
                    updated_generic_string_objects: Default::default(),
                    deleted_generic_string_objects: Default::default(),
                    action_histories: Default::default(),
                    mcp_gallery: Default::default(),
                })
            });

        // No workflows or notebooks (or other objects) loaded from sqlite passed to CloudModel
        initialize_app(&mut app, Vec::new(), Arc::new(cloud_object_server_api_mock));

        // Spend time waiting for the initial load to finish etc.
        warpui::r#async::Timer::after(Duration::from_secs(1)).await;

        // TODO: @ianhodge - update tests once cloud model APIs are added
        // Now CloudModel should include all objects that were fetched via initial load
        check_cloud_workflows(&mut app, personal_workflows_len + team_workflows_len);
        check_cloud_folders(&mut app, personal_folders_len + team_folders_len);
        check_cloud_notebooks(&mut app, notebooks_len);
    })
}

#[test]
fn test_loading_all_cloud_objects_after_switching_from_offline() {
    let _flag = FeatureFlag::KnowledgeSidebar.override_enabled(true);

    let personal_workflows = mock_random_workflows(100, Owner::mock_current_user());
    let personal_workflows_len = personal_workflows.len();
    let team_workflows = mock_random_workflows(
        200,
        Owner::Team {
            team_uid: ServerId::from(1),
        },
    );
    let team_workflows_len = team_workflows.len();
    let personal_folders = mock_random_folders(300, Owner::mock_current_user());
    let personal_folders_len = personal_folders.len();
    let team_folders = mock_random_folders(
        400,
        Owner::Team {
            team_uid: ServerId::from(1),
        },
    );
    let team_folders_len = team_folders.len();
    let notebooks = mock_server_notebooks();
    let notebooks_len = notebooks.len();

    App::test((), |mut app| async move {
        // Sequences used for ordering requests (so first call will return something different than
        // next etc.)
        let mut cloud_objects_sequence = Sequence::new();

        // Setup the app and APIs
        let mut cloud_object_server_api_mock = base_mock_cloud_object_server_api();

        // Update manager also calls for update based on the current in memory state
        // We only expect it once with the given set of arguments (empty vector) and it'll return
        // personal workflows
        cloud_object_server_api_mock
            .expect_fetch_changed_objects()
            .times(1)
            // since we don't have anything cached, at startup we just send empty list here
            .withf(|objects_to_update, _| {
                objects_to_update.notebooks.is_empty()
                    && objects_to_update.workflows.is_empty()
                    && objects_to_update.folders.is_empty()
                    && objects_to_update.generic_string_objects.is_empty()
            })
            .in_sequence(&mut cloud_objects_sequence)
            .return_once(move |_, _| {
                Ok(InitialLoadResponse {
                    updated_notebooks: vec![],
                    updated_workflows: personal_workflows,
                    updated_folders: personal_folders,
                    deleted_notebooks: vec![],
                    deleted_workflows: vec![],
                    deleted_folders: vec![],
                    user_profiles: vec![],
                    updated_generic_string_objects: Default::default(),
                    deleted_generic_string_objects: Default::default(),
                    action_histories: Default::default(),
                    mcp_gallery: Default::default(),
                })
            });

        // Second call will return objects from the team (as a list of updated / new to user objects)
        // It'll also be called only once, and with personal_workflows in the input
        // We expect this call to happen _after_ the network status change (offline -> online).
        cloud_object_server_api_mock
            .expect_fetch_changed_objects()
            .times(1)
            // verify that the list of objects passed equals the number of personal workflows we
            // already have
            .withf(move |objects_to_update, _| {
                objects_to_update.workflows.len() == personal_workflows_len
                    && objects_to_update.folders.len() == personal_folders_len
            })
            .in_sequence(&mut cloud_objects_sequence)
            .returning(move |_, _| {
                Ok(InitialLoadResponse {
                    updated_notebooks: notebooks.clone(),
                    updated_workflows: team_workflows.clone(),
                    updated_folders: team_folders.clone(),
                    deleted_notebooks: vec![],
                    deleted_workflows: vec![],
                    deleted_folders: vec![],
                    user_profiles: vec![],
                    updated_generic_string_objects: Default::default(),
                    deleted_generic_string_objects: Default::default(),
                    action_histories: Default::default(),
                    mcp_gallery: Default::default(),
                })
            });

        // No workflows or notebooks (or other objects) loaded from sqlite passed to CloudModel
        initialize_app(&mut app, Vec::new(), Arc::new(cloud_object_server_api_mock));
        check_cloud_workflows(&mut app, 0);
        check_cloud_notebooks(&mut app, 0);
        check_cloud_folders(&mut app, 0);

        // Spend time waiting for the initial load to finish etc.
        warpui::r#async::Timer::after(Duration::from_secs(1)).await;

        // Now CloudModel should include all objects that were fetched via initial load (in this
        // case: personal only)
        check_cloud_workflows(&mut app, personal_workflows_len);
        check_cloud_folders(&mut app, personal_folders_len);
        check_cloud_notebooks(&mut app, 0);

        // Lets go offline
        NetworkStatus::handle(&app).update(&mut app, |network_status, ctx| {
            network_status.reachability_changed(false, ctx);
        });

        // Lets go back online
        NetworkStatus::handle(&app).update(&mut app, |network_status, ctx| {
            network_status.reachability_changed(true, ctx);
        });

        // Spend time waiting for the load to finish etc.
        warpui::r#async::Timer::after(Duration::from_secs(1)).await;

        // Now CloudModel should include all objects that were fetched via initial load
        check_cloud_workflows(&mut app, personal_workflows_len + team_workflows_len);
        check_cloud_folders(&mut app, personal_folders_len + team_folders_len);
        check_cloud_notebooks(&mut app, notebooks_len);
    })
}

#[test]
fn test_force_refresh_only_happens_once() {
    let _flag = FeatureFlag::KnowledgeSidebar.override_enabled(true);

    App::test((), |mut app| async move {
        let mut cloud_objects_sequence = Sequence::new();
        let mut cloud_object_server_api_mock = base_mock_cloud_object_server_api();

        // We expect force_refresh to be true on this request.
        cloud_object_server_api_mock
            .expect_fetch_changed_objects()
            .times(1)
            .withf(|objects_to_update, force_refresh| {
                objects_to_update.notebooks.is_empty()
                    && objects_to_update.workflows.is_empty()
                    && objects_to_update.folders.is_empty()
                    && objects_to_update.generic_string_objects.is_empty()
                    && *force_refresh
            })
            .in_sequence(&mut cloud_objects_sequence)
            .return_once(move |_, _| {
                Ok(InitialLoadResponse {
                    updated_notebooks: vec![],
                    updated_workflows: vec![],
                    updated_folders: vec![],
                    deleted_notebooks: vec![],
                    deleted_workflows: vec![],
                    deleted_folders: vec![],
                    user_profiles: vec![],
                    updated_generic_string_objects: Default::default(),
                    deleted_generic_string_objects: Default::default(),
                    action_histories: Default::default(),
                    mcp_gallery: Default::default(),
                })
            });

        // Initialize app with pending refresh = true!
        initialize_app(&mut app, Vec::new(), Arc::new(cloud_object_server_api_mock));

        // Spend time waiting for the initial load to finish etc.
        warpui::r#async::Timer::after(Duration::from_secs(1)).await;

        // Check that pending refresh is now false on CloudModel
        CloudModel::handle(&app).read(&app, |model, _ctx| {
            assert!(!model.cloud_objects_force_refresh_pending())
        });
    })
}

#[test]
fn test_force_refresh_correctly_resets_timestamp() {
    let _flag = FeatureFlag::KnowledgeSidebar.override_enabled(true);

    App::test((), |mut app| async move {
        let mut cloud_objects_sequence = Sequence::new();
        let mut cloud_object_server_api_mock = base_mock_cloud_object_server_api();

        // We expect force_refresh to be true on this request.
        cloud_object_server_api_mock
            .expect_fetch_changed_objects()
            .times(1)
            .withf(|objects_to_update, force_refresh| {
                objects_to_update.notebooks.is_empty()
                    && objects_to_update.workflows.is_empty()
                    && objects_to_update.folders.is_empty()
                    && objects_to_update.generic_string_objects.is_empty()
                    && *force_refresh
            })
            .in_sequence(&mut cloud_objects_sequence)
            .return_once(move |_, _| {
                Ok(InitialLoadResponse {
                    updated_notebooks: vec![],
                    updated_workflows: vec![],
                    updated_folders: vec![],
                    deleted_notebooks: vec![],
                    deleted_workflows: vec![],
                    deleted_folders: vec![],
                    user_profiles: vec![],
                    updated_generic_string_objects: Default::default(),
                    deleted_generic_string_objects: Default::default(),
                    action_histories: Default::default(),
                    mcp_gallery: Default::default(),
                })
            });

        // Initialize app with pending refresh = true!
        initialize_app(&mut app, Vec::new(), Arc::new(cloud_object_server_api_mock));

        // Spend time waiting for the initial load to finish etc.
        warpui::r#async::Timer::after(Duration::from_secs(1)).await;

        // Check that pending refresh is within the acceptable hour range.
        CloudModel::handle(&app).read(&app, |model, _ctx| {
            let time_option = model.time_of_next_force_refresh;
            assert!(time_option.is_some());
            let time = time_option.unwrap();
            assert!(
                time <= (Utc::now()
                    + chrono::Duration::minutes(MAX_MINUTES_UNTIL_NEXT_FORCE_REFRESH))
            );
            assert!(
                time >= (Utc::now()
                    + chrono::Duration::minutes(MIN_MINUTES_UNTIL_NEXT_FORCE_REFRESH))
            );
        });
    })
}

#[test]
fn test_collapse_all_in_location() {
    /*
       the folder structure looks like:

       test1
        ↳ test 4
         ↳ test 5
       test 2
        ↳ test 6
         ↳ test 7
       test 3

    */
    let folder_1_id: SyncId = SyncId::ServerId(1.into());
    let folder_2_id: SyncId = SyncId::ServerId(2.into());
    let folder_3_id: SyncId = SyncId::ServerId(3.into());
    let folder_4_id: SyncId = SyncId::ServerId(4.into());
    let folder_5_id: SyncId = SyncId::ServerId(5.into());
    let folder_6_id: SyncId = SyncId::ServerId(6.into());
    let folder_7_id: SyncId = SyncId::ServerId(7.into());

    let folders = vec![
        mock_cloud_folder(folder_1_id, "test1".to_string(), None),
        mock_cloud_folder(folder_2_id, "test2".to_string(), None),
        mock_cloud_folder(folder_3_id, "test3".to_string(), None),
        mock_cloud_folder(folder_4_id, "test4".to_string(), Some(folder_1_id)),
        mock_cloud_folder(folder_5_id, "test5".to_string(), Some(folder_4_id)),
        mock_cloud_folder(folder_6_id, "test6".to_string(), Some(folder_2_id)),
        mock_cloud_folder(folder_7_id, "test7".to_string(), Some(folder_6_id)),
    ]
    .into_iter()
    .map(|o| Box::new(o) as Box<dyn CloudObject>)
    .collect();

    App::test((), |mut app| async move {
        app.add_singleton_model(UserWorkspaces::default_mock);
        let cloud_model = create_cloud_model(&mut app, folders);

        cloud_model.update(&mut app, |model, ctx| {
            // first, collapse all folders in folder 1
            model.collapse_all_in_location(
                CloudObjectLocation::Folder(folder_1_id),
                DriveIndexVariant::MainIndex,
                ctx,
            );

            // folders 1, 4, and 5 should be collapsed
            let folder_1 = folder_from_cloud_model(model, folder_1_id);
            let folder_4 = folder_from_cloud_model(model, folder_4_id);
            let folder_5 = folder_from_cloud_model(model, folder_5_id);
            assert!(!folder_1.model.is_open);
            assert!(!folder_4.model.is_open);
            assert!(!folder_5.model.is_open);
            // but the others are still open
            let folder_2 = folder_from_cloud_model(model, folder_2_id);
            let folder_3 = folder_from_cloud_model(model, folder_3_id);
            let folder_6 = folder_from_cloud_model(model, folder_6_id);
            let folder_7 = folder_from_cloud_model(model, folder_7_id);
            assert!(folder_2.model.is_open);
            assert!(folder_3.model.is_open);
            assert!(folder_6.model.is_open);
            assert!(folder_7.model.is_open);

            model.collapse_all_in_location(
                CloudObjectLocation::Space(Default::default()),
                DriveIndexVariant::MainIndex,
                ctx,
            );
            // now all folders in this space are collapsed
            let folder_1 = folder_from_cloud_model(model, folder_1_id);
            let folder_2 = folder_from_cloud_model(model, folder_2_id);
            let folder_3 = folder_from_cloud_model(model, folder_3_id);
            let folder_4 = folder_from_cloud_model(model, folder_4_id);
            let folder_5 = folder_from_cloud_model(model, folder_5_id);
            let folder_6 = folder_from_cloud_model(model, folder_6_id);
            let folder_7 = folder_from_cloud_model(model, folder_7_id);
            assert!(!folder_1.model.is_open);
            assert!(!folder_2.model.is_open);
            assert!(!folder_3.model.is_open);
            assert!(!folder_4.model.is_open);
            assert!(!folder_5.model.is_open);
            assert!(!folder_6.model.is_open);
            assert!(!folder_7.model.is_open);
        });
    })
}

#[test]
fn test_collapse_all_in_trash() {
    /*
       the folder structure looks like:

       test1 -- trashed by user
        ↳ test 4
         ↳ test 5 -- trashed by user
       test 2 -- trashed by user
        ↳ test 6
         ↳ test 7
       test 3 -- trashed by user

       the structure in the trash index looks like:

       test1 -- trashed by user
        ↳ test 4
       test 5 -- trashed by user
       test 2 -- trashed by user
        ↳ test 6
         ↳ test 7
       test 3 -- trashed by user

    */
    let folder_1_id: SyncId = SyncId::ServerId(1.into());
    let folder_2_id: SyncId = SyncId::ServerId(2.into());
    let folder_3_id: SyncId = SyncId::ServerId(3.into());
    let folder_4_id: SyncId = SyncId::ServerId(4.into());
    let folder_5_id: SyncId = SyncId::ServerId(5.into());
    let folder_6_id: SyncId = SyncId::ServerId(6.into());
    let folder_7_id: SyncId = SyncId::ServerId(7.into());

    let folders = vec![
        mock_trashed_cloud_folder(folder_1_id, "test1".to_string(), None),
        mock_trashed_cloud_folder(folder_2_id, "test2".to_string(), None),
        mock_trashed_cloud_folder(folder_3_id, "test3".to_string(), None),
        mock_cloud_folder(folder_4_id, "test4".to_string(), Some(folder_1_id)),
        mock_trashed_cloud_folder(folder_5_id, "test5".to_string(), Some(folder_4_id)),
        mock_cloud_folder(folder_6_id, "test6".to_string(), Some(folder_2_id)),
        mock_cloud_folder(folder_7_id, "test7".to_string(), Some(folder_6_id)),
    ]
    .into_iter()
    .map(|o| Box::new(o) as Box<dyn CloudObject>)
    .collect();

    App::test((), |mut app| async move {
        app.add_singleton_model(UserWorkspaces::default_mock);
        let cloud_model = create_cloud_model(&mut app, folders);

        cloud_model.update(&mut app, |model, ctx| {
            // first, collapse all folders in folder 1
            model.collapse_all_in_location(
                CloudObjectLocation::Folder(folder_1_id),
                DriveIndexVariant::Trash,
                ctx,
            );

            // folders 1, 4 should be collapsed
            let folder_1 = folder_from_cloud_model(model, folder_1_id);
            let folder_4 = folder_from_cloud_model(model, folder_4_id);
            assert!(!folder_1.model.is_open);
            assert!(!folder_4.model.is_open);
            // but the others, including folder 5, are still open
            let folder_2 = folder_from_cloud_model(model, folder_2_id);
            let folder_3 = folder_from_cloud_model(model, folder_3_id);
            let folder_5 = folder_from_cloud_model(model, folder_5_id);
            let folder_6 = folder_from_cloud_model(model, folder_6_id);
            let folder_7 = folder_from_cloud_model(model, folder_7_id);
            assert!(folder_2.model.is_open);
            assert!(folder_3.model.is_open);
            assert!(folder_5.model.is_open);
            assert!(folder_6.model.is_open);
            assert!(folder_7.model.is_open);

            model.collapse_all_in_location(
                CloudObjectLocation::Space(Default::default()),
                DriveIndexVariant::Trash,
                ctx,
            );
            // now all folders in this space are collapsed
            let folder_1 = folder_from_cloud_model(model, folder_1_id);
            let folder_2 = folder_from_cloud_model(model, folder_2_id);
            let folder_3 = folder_from_cloud_model(model, folder_3_id);
            let folder_4 = folder_from_cloud_model(model, folder_4_id);
            let folder_5 = folder_from_cloud_model(model, folder_5_id);
            let folder_6 = folder_from_cloud_model(model, folder_6_id);
            let folder_7 = folder_from_cloud_model(model, folder_7_id);
            assert!(!folder_1.model.is_open);
            assert!(!folder_2.model.is_open);
            assert!(!folder_3.model.is_open);
            assert!(!folder_4.model.is_open);
            assert!(!folder_5.model.is_open);
            assert!(!folder_6.model.is_open);
            assert!(!folder_7.model.is_open);
        });
    })
}

#[test]
fn test_object_editor_timeout() {
    App::test((), |mut app| async move {
        // Setup the app and APIs
        let cloud_object_server_api_mock = base_mock_cloud_object_server_api();
        initialize_app(&mut app, Vec::new(), Arc::new(cloud_object_server_api_mock));
        let notebook_id: SyncId = SyncId::ServerId(1.into());
        let cloud_notebook = mock_cloud_notebook(notebook_id, "test1".into(), None);

        CloudModel::handle(&app).update(&mut app, |model, _ctx| {
            // Add a notebook to CloudModel
            model.add_object(notebook_id, cloud_notebook.clone());

            let notebook = model
                .get_notebook_mut(&notebook_id)
                .expect("notebook should exist");

            // Set the editor to be somebody else.
            notebook.metadata.current_editor_uid = Some("ian@warp.dev".to_string());
        });

        let current_editor = CloudViewModel::handle(&app).read(&app, |view_model, ctx| {
            view_model
                .object_current_editor(&notebook_id.uid(), ctx)
                .expect("expect editor to be set")
        });
        // Assert that the current editor is an active other user
        assert_eq!(current_editor.state, EditorState::OtherUserActive);

        CloudModel::handle(&app).update(&mut app, |model, _ctx| {
            let notebook = model
                .get_notebook_mut(&notebook_id)
                .expect("notebook should exist");

            // Set the notebook timesteps to be more than the timeout
            let timeout_timestamp = Utc::now()
                - chrono::Duration::minutes(EDITOR_TIMEOUT_DURATION_MINUTES)
                - chrono::Duration::seconds(1);
            notebook.metadata.revision = Some(Revision::from(timeout_timestamp));
            notebook.metadata.metadata_last_updated_ts = Some(timeout_timestamp.into());
        });

        let current_editor = CloudViewModel::handle(&app).read(&app, |view_model, ctx| {
            view_model
                .object_current_editor(&notebook_id.uid(), ctx)
                .expect("expect editor to be set")
        });
        // Assert that the current editor is an idle other user
        assert_eq!(current_editor.state, EditorState::OtherUserIdle);
    });
}

#[test]
fn test_breadcrumbs() {
    let folder_1_id: SyncId = SyncId::ServerId(1.into());
    let folder_2_id: SyncId = SyncId::ServerId(2.into());
    let folder_3_id: SyncId = SyncId::ServerId(3.into());

    let folders = vec![
        mock_cloud_folder(folder_1_id, "test1".to_string(), None),
        mock_cloud_folder(folder_2_id, "test2".to_string(), Some(folder_1_id)),
        mock_cloud_folder(folder_3_id, "test3".to_string(), Some(folder_2_id)),
    ]
    .into_iter()
    .map(|f| Box::new(f) as Box<dyn CloudObject>)
    .collect::<Vec<_>>();

    App::test((), |mut app| async move {
        let cloud_object_server_api_mock = base_mock_cloud_object_server_api();
        initialize_app(
            &mut app,
            folders.clone(),
            Arc::new(cloud_object_server_api_mock),
        );

        CloudModel::handle(&app).read(&app, |_, ctx| {
            assert_eq!("Personal".to_string(), folders[0].breadcrumbs(ctx));
            assert_eq!("Personal / test1".to_string(), folders[1].breadcrumbs(ctx));
            assert_eq!(
                "Personal / test1 / test2".to_string(),
                folders[2].breadcrumbs(ctx)
            );
        });
    });
}

/// Asserts that the object with the given ID has the expected sorting timestamp.
#[track_caller]
fn assert_sorting_timestamp(id: ServerId, expected_ts: impl Into<ServerTimestamp>, app: &App) {
    let sorting_timestamp = app.read(|ctx| {
        let object = CloudModel::as_ref(ctx).get_by_uid(&id.uid())?;
        CloudViewModel::as_ref(ctx).object_sorting_timestamp(object, UpdateTimestamp::Revision, ctx)
    });
    assert_eq!(
        sorting_timestamp,
        Some(expected_ts.into()),
        "Unexpected timestamp for {}",
        id.uid()
    );
}

/// Test that, if an object is updated, we recalculate its ancestors' sorting timestamps too. This
/// way, the folders containing the updated object move to the top of the Warp Drive index if it's
/// sorted by last updated.
#[test]
fn test_update_folder_timestamp_from_child_update() {
    App::test((), |mut app| async move {
        initialize_app(
            &mut app,
            Vec::new(),
            Arc::new(base_mock_cloud_object_server_api()),
        );

        let folder_id: ServerId = 123.into();
        let parent_folder_id: ServerId = 456.into();
        let notebook_id: ServerId = 789.into();
        let initial_ts = Utc::now();

        CloudModel::handle(&app).update(&mut app, |cloud_model, _ctx| {
            let mut folder = mock_cloud_folder(
                folder_id.into(),
                "Folder".to_string(),
                Some(parent_folder_id.into()),
            );
            folder.metadata.revision = Some(initial_ts.into());

            let mut parent_folder =
                mock_cloud_folder(parent_folder_id.into(), "Parent Folder".to_string(), None);
            parent_folder.metadata.revision = Some(initial_ts.into());

            let mut notebook = mock_cloud_notebook(
                notebook_id.into(),
                "Test Notebook".to_string(),
                Some(folder_id.into()),
            );
            notebook.metadata.revision = Some(initial_ts.into());

            cloud_model.add_object(folder_id.into(), folder);
            cloud_model.add_object(parent_folder_id.into(), parent_folder);
            cloud_model.add_object(notebook_id.into(), notebook);
        });

        // At first, all 3 objects should have the initial sorting timestamp.
        assert_sorting_timestamp(folder_id, initial_ts, &app);
        assert_sorting_timestamp(parent_folder_id, initial_ts, &app);
        assert_sorting_timestamp(notebook_id, initial_ts, &app);

        // After updating the notebook, all 3 timestamps should change.
        let new_ts = initial_ts + chrono::Duration::seconds(5);
        receive_rtc_update(
            ObjectUpdateMessage::ObjectContentChanged {
                server_object: Box::new(ServerCloudObject::Notebook(ServerNotebook {
                    id: SyncId::ServerId(notebook_id),
                    model: CloudNotebookModel {
                        title: "Test Notebook".to_string(),
                        data: "test2".into(),
                        ai_document_id: None,
                        conversation_id: None,
                    },
                    metadata: ServerMetadata {
                        uid: notebook_id,
                        revision: new_ts.into(),
                        metadata_last_updated_ts: new_ts.into(),
                        trashed_ts: None,
                        folder_id: Some(folder_id.into()),
                        is_welcome_object: false,
                        creator_uid: None,
                        last_editor_uid: None,
                        current_editor_uid: None,
                    },
                    permissions: mock_server_permissions(Owner::mock_current_user()),
                })),
                last_editor: None,
            },
            &mut app,
        );

        assert_sorting_timestamp(folder_id, new_ts, &app);
        assert_sorting_timestamp(parent_folder_id, new_ts, &app);
        assert_sorting_timestamp(notebook_id, new_ts, &app);
    });
}

/// Tests that, if an object is moved from one folder to another, we recalculate the sorting
/// timestamps of both. If the object was the most-recently-updated in its old folder, the old
/// folder's sorting timestamp will decrease. If it's the most-recently-updated object in the new
/// folder, the new folder's sorting timestamp will increase.
#[test]
fn test_update_folder_timestamp_from_object_move() {
    App::test((), |mut app| async move {
        initialize_app(
            &mut app,
            Vec::new(),
            Arc::new(base_mock_cloud_object_server_api()),
        );

        let folder_a_id: ServerId = 123.into();
        let folder_b_id: ServerId = 456.into();
        let notebook_id: ServerId = 789.into();

        let t1 = Utc::now();
        let t2 = t1 + chrono::Duration::seconds(5);
        CloudModel::handle(&app).update(&mut app, |cloud_model, _ctx| {
            let mut folder_a = mock_cloud_folder(folder_a_id.into(), "Folder A".to_string(), None);
            folder_a.metadata.revision = Some(t1.into());

            let mut folder_b = mock_cloud_folder(folder_b_id.into(), "Folder B".to_string(), None);
            folder_b.metadata.revision = Some(t1.into());

            let mut notebook = mock_cloud_notebook(
                notebook_id.into(),
                "Test Notebook".to_string(),
                Some(folder_a_id.into()),
            );
            notebook.metadata.revision = Some(t2.into());
            notebook.metadata.metadata_last_updated_ts = Some(t2.into());

            cloud_model.add_object(folder_a_id.into(), folder_a);
            cloud_model.add_object(folder_b_id.into(), folder_b);
            cloud_model.add_object(notebook_id.into(), notebook);
        });

        // At first, folder A and the notebook sort by the notebook's timestamp, and folder B sorts
        // by its older timestamp.
        assert_sorting_timestamp(folder_a_id, t2, &app);
        assert_sorting_timestamp(folder_b_id, t1, &app);
        assert_sorting_timestamp(notebook_id, t2, &app);

        // Move the workflow to folder B, so it now has the newer sort timestamp.
        move_object(notebook_id, Some(folder_b_id.into()), &mut app);

        assert_sorting_timestamp(folder_a_id, t1, &app);
        assert_sorting_timestamp(folder_b_id, t2, &app);

        // Move the workflow into the root, so both folders have the older sort timestamp.
        move_object(notebook_id, None, &mut app);
        assert_sorting_timestamp(folder_a_id, t1, &app);
        assert_sorting_timestamp(folder_b_id, t1, &app);
    });
}

/// Tests that, if an object is created in a folder, we recalculate its ancestors' sorting
/// timestamp. The new object will likely be the most-recently-updated child of the folder, so the
/// folder's sorting timestamp will increase.
#[test]
fn test_update_folder_timestamp_from_new_child() {
    App::test((), |mut app| async move {
        initialize_app(
            &mut app,
            Vec::new(),
            Arc::new(base_mock_cloud_object_server_api()),
        );

        let folder_id: ServerId = 123.into();
        let parent_folder_id: ServerId = 456.into();
        let notebook_id: ServerId = 789.into();
        let t1 = Utc::now();
        let t2 = t1 + chrono::Duration::seconds(5);

        CloudModel::handle(&app).update(&mut app, |cloud_model, _ctx| {
            let mut folder = mock_cloud_folder(
                folder_id.into(),
                "Folder".to_string(),
                Some(parent_folder_id.into()),
            );
            folder.metadata.revision = Some(t1.into());

            let mut parent_folder =
                mock_cloud_folder(parent_folder_id.into(), "Parent Folder".to_string(), None);
            parent_folder.metadata.revision = Some(t1.into());

            cloud_model.add_object(folder_id.into(), folder);
            cloud_model.add_object(parent_folder_id.into(), parent_folder);
        });

        // At first, only the two folders exist.
        assert_sorting_timestamp(folder_id, t1, &app);
        assert_sorting_timestamp(parent_folder_id, t1, &app);

        // Create a notebook inside the folder.
        receive_rtc_update(
            ObjectUpdateMessage::ObjectContentChanged {
                server_object: Box::new(ServerCloudObject::Notebook(ServerNotebook {
                    id: SyncId::ServerId(notebook_id),
                    model: CloudNotebookModel {
                        title: "Test Notebook".to_string(),
                        data: "test".to_string(),
                        ai_document_id: None,
                        conversation_id: None,
                    },
                    metadata: ServerMetadata {
                        uid: notebook_id,
                        revision: t2.into(),
                        metadata_last_updated_ts: t2.into(),
                        trashed_ts: None,
                        folder_id: Some(folder_id.into()),
                        is_welcome_object: false,
                        creator_uid: None,
                        last_editor_uid: None,
                        current_editor_uid: None,
                    },
                    permissions: mock_server_permissions(Owner::mock_current_user()),
                })),
                last_editor: None,
            },
            &mut app,
        );

        // The notebook timestamp is now the sort timestamp for the folders.
        assert_sorting_timestamp(folder_id, t2, &app);
        assert_sorting_timestamp(parent_folder_id, t2, &app);
    });
}

/// Tests that, if an object is trashed or untrashed, we recalculate its folder's sorting timestamp.
/// Only untrashed children count towards a folder's sorting timestamp, so trashing/untrashing
/// effectively changes the folder's contents.
#[test]
fn test_update_folder_timestamp_from_child_trash() {
    App::test((), |mut app| async move {
        initialize_app(
            &mut app,
            Vec::new(),
            Arc::new(base_mock_cloud_object_server_api()),
        );

        let notebook_id: ServerId = 123.into();
        let folder_id: ServerId = 456.into();

        let t1 = Utc::now();
        let t2 = t1 + chrono::Duration::seconds(1);
        let t3 = t2 + chrono::Duration::seconds(1);
        let t4 = t3 + chrono::Duration::seconds(1);

        CloudModel::handle(&app).update(&mut app, |cloud_model, _ctx| {
            let mut folder = mock_cloud_folder(folder_id.into(), "Folder".to_string(), None);
            folder.metadata.revision = Some(t1.into());

            let mut notebook = mock_cloud_notebook(
                notebook_id.into(),
                "Notebook".to_string(),
                Some(folder_id.into()),
            );
            notebook.metadata.revision = Some(t2.into());
            notebook.metadata.metadata_last_updated_ts = Some(t2.into());

            cloud_model.add_object(folder_id.into(), folder);
            cloud_model.add_object(notebook_id.into(), notebook);
        });

        assert_sorting_timestamp(folder_id, t2, &app);

        // Trash the notebook so that it no longer counts towards the folder's sort timestamp.
        receive_rtc_update(
            ObjectUpdateMessage::ObjectMetadataChanged {
                metadata: ServerMetadata {
                    uid: notebook_id,
                    revision: t2.into(),
                    metadata_last_updated_ts: t3.into(),
                    trashed_ts: Some(t3.into()),
                    folder_id: Some(folder_id.into()),
                    is_welcome_object: false,
                    creator_uid: None,
                    last_editor_uid: None,
                    current_editor_uid: None,
                },
            },
            &mut app,
        );

        assert_sorting_timestamp(folder_id, t1, &app);

        // Untrash the notebook, updating the folder timestamp.
        receive_rtc_update(
            ObjectUpdateMessage::ObjectMetadataChanged {
                metadata: ServerMetadata {
                    uid: notebook_id,
                    revision: t2.into(),
                    metadata_last_updated_ts: t4.into(),
                    trashed_ts: None,
                    folder_id: Some(folder_id.into()),
                    is_welcome_object: false,
                    creator_uid: None,
                    last_editor_uid: None,
                    current_editor_uid: None,
                },
            },
            &mut app,
        );

        assert_sorting_timestamp(folder_id, t2, &app);
    });
}

#[test]
fn test_shared_personal_object() {
    let _guard = FeatureFlag::SharedWithMe.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(
            &mut app,
            Vec::new(),
            Arc::new(base_mock_cloud_object_server_api()),
        );

        let other_user = UserUid::new("other_user");
        let shared_notebook_id = SyncId::ServerId(123.into());
        let shared_notebook = CloudNotebook::new(
            shared_notebook_id,
            CloudNotebookModel {
                title: "Shared Notebook".to_string(),
                data: "Hello".to_string(),
                ai_document_id: None,
                conversation_id: None,
            },
            CloudObjectMetadata::new_from_server(mock_server_metadata()),
            CloudObjectPermissions {
                owner: Owner::User {
                    user_uid: other_user,
                },
                guests: Vec::new(),
                permissions_last_updated_ts: None,
                anyone_with_link: None,
            },
        );

        CloudModel::handle(&app).update(&mut app, |cloud_model, ctx| {
            cloud_model.add_object(shared_notebook_id, shared_notebook);

            let space = cloud_model
                .get_notebook(&shared_notebook_id)
                .expect("Notebook is in CloudModel")
                .space(ctx);
            assert_eq!(space, Space::Shared);
        });
    });
}

#[test]
fn test_unshared_personal_object() {
    let _guard = FeatureFlag::SharedWithMe.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(
            &mut app,
            Vec::new(),
            Arc::new(base_mock_cloud_object_server_api()),
        );

        let shared_notebook_id = SyncId::ServerId(123.into());
        let shared_notebook = CloudNotebook::new(
            shared_notebook_id,
            CloudNotebookModel {
                title: "Shared Notebook".to_string(),
                data: "Hello".to_string(),
                ai_document_id: None,
                conversation_id: None,
            },
            CloudObjectMetadata::new_from_server(mock_server_metadata()),
            CloudObjectPermissions {
                owner: Owner::User {
                    user_uid: UserUid::new(TEST_USER_UID),
                },
                guests: Vec::new(),
                permissions_last_updated_ts: None,
                anyone_with_link: None,
            },
        );

        CloudModel::handle(&app).update(&mut app, |cloud_model, ctx| {
            cloud_model.add_object(shared_notebook_id, shared_notebook);

            let space = cloud_model
                .get_notebook(&shared_notebook_id)
                .expect("Notebook is in CloudModel")
                .space(ctx);
            assert_eq!(space, Space::Personal);
        });
    });
}

#[test]
fn test_shared_team_object() {
    let _guard = FeatureFlag::SharedWithMe.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(
            &mut app,
            Vec::new(),
            Arc::new(base_mock_cloud_object_server_api()),
        );

        // The user is not on this team.
        let team_uid = ServerId::from(456);

        let shared_notebook_id = SyncId::ServerId(123.into());
        let shared_notebook = CloudNotebook::new(
            shared_notebook_id,
            CloudNotebookModel {
                title: "Shared Notebook".to_string(),
                data: "Hello".to_string(),
                ai_document_id: None,
                conversation_id: None,
            },
            CloudObjectMetadata::new_from_server(mock_server_metadata()),
            CloudObjectPermissions {
                owner: Owner::Team { team_uid },
                guests: Vec::new(),
                permissions_last_updated_ts: None,
                anyone_with_link: None,
            },
        );

        CloudModel::handle(&app).update(&mut app, |cloud_model, ctx| {
            cloud_model.add_object(shared_notebook_id, shared_notebook);

            let space = cloud_model
                .get_notebook(&shared_notebook_id)
                .expect("Notebook is in CloudModel")
                .space(ctx);
            assert_eq!(space, Space::Shared);
        });
    });
}

#[test]
fn test_unshared_team_object() {
    let _guard = FeatureFlag::SharedWithMe.override_enabled(true);
    App::test((), |mut app| async move {
        app.update(init_and_register_user_preferences);
        initialize_app(
            &mut app,
            Vec::new(),
            Arc::new(base_mock_cloud_object_server_api()),
        );

        // Use the current user's team.
        let team_uid = TEST_TEAM.uid;
        let shared_notebook_id = SyncId::ServerId(123.into());
        let shared_notebook = CloudNotebook::new(
            shared_notebook_id,
            CloudNotebookModel {
                title: "Shared Notebook".to_string(),
                data: "Hello".to_string(),
                ai_document_id: None,
                conversation_id: None,
            },
            CloudObjectMetadata::new_from_server(mock_server_metadata()),
            CloudObjectPermissions {
                owner: Owner::Team { team_uid },
                guests: Vec::new(),
                permissions_last_updated_ts: None,
                anyone_with_link: None,
            },
        );

        CloudModel::handle(&app).update(&mut app, |cloud_model, ctx| {
            cloud_model.add_object(shared_notebook_id, shared_notebook);

            let space = cloud_model
                .get_notebook(&shared_notebook_id)
                .expect("Notebook is in CloudModel")
                .space(ctx);
            assert_eq!(space, Space::Team { team_uid });
        });
    });
}

#[test]
fn test_shared_object_in_unshared_folder() {
    let _guard = FeatureFlag::SharedWithMe.override_enabled(true);
    App::test((), |mut app| async move {
        app.update(init_and_register_user_preferences);
        initialize_app(
            &mut app,
            Vec::new(),
            Arc::new(base_mock_cloud_object_server_api()),
        );

        let other_user = UserUid::new("other_user");
        let unshared_folder_id = SyncId::ServerId(567.into());
        let shared_notebook_id = SyncId::ServerId(123.into());
        let mut shared_notebook = CloudNotebook::new(
            shared_notebook_id,
            CloudNotebookModel {
                title: "Shared Notebook".to_string(),
                data: "Hello".to_string(),
                ai_document_id: None,
                conversation_id: None,
            },
            CloudObjectMetadata::new_from_server(mock_server_metadata()),
            CloudObjectPermissions {
                owner: Owner::User {
                    user_uid: other_user,
                },
                guests: Vec::new(),
                permissions_last_updated_ts: None,
                anyone_with_link: None,
            },
        );
        shared_notebook.metadata_mut().folder_id = Some(unshared_folder_id);

        CloudModel::handle(&app).update(&mut app, |cloud_model, ctx| {
            cloud_model.add_object(shared_notebook_id, shared_notebook);
            let notebook = cloud_model
                .get_notebook(&shared_notebook_id)
                .expect("Notebook is in CloudModel");

            // Check space-based APIs.
            assert_eq!(notebook.space(ctx), Space::Shared);
            assert!(notebook.is_in_space(Space::Shared, ctx));

            // Check location-based APIs.
            assert_eq!(
                notebook.location(cloud_model, ctx),
                CloudObjectLocation::Space(Space::Shared)
            );
            assert!(notebook.metadata.folder_id.is_some());

            // Despite the missing parent folder, the notebook is not trashed.
            assert!(!notebook.is_trashed(cloud_model));

            // Check that iteration APIs include the notebook where it's expected.
            assert!(cloud_model
                .active_cloud_objects_in_space(Space::Shared, ctx)
                .any(|obj| obj.uid() == notebook.uid()));
            assert!(cloud_model
                .active_cloud_objects_in_location_without_descendents(
                    CloudObjectLocation::Space(Space::Shared),
                    ctx
                )
                .any(|obj| obj.uid() == notebook.uid()));
            assert_eq!(
                cloud_model
                    .trashed_cloud_objects_in_space(Space::Shared, ctx)
                    .count(),
                0
            );
            assert_eq!(
                cloud_model
                    .trashed_cloud_objects_in_location_without_descendents(
                        CloudObjectLocation::Space(Space::Shared),
                        ctx
                    )
                    .count(),
                0
            );

            let folder_location = CloudObjectLocation::Folder(unshared_folder_id);
            assert_eq!(
                cloud_model
                    .active_cloud_objects_in_location_without_descendents(folder_location, ctx)
                    .count(),
                0
            );
            assert_eq!(
                cloud_model
                    .trashed_cloud_objects_in_location_without_descendents(folder_location, ctx)
                    .count(),
                0
            );
        });
    });
}

/// Helper: compute active UIDs using the naive (non-memoized) is_trashed approach.
fn naive_active_object_uids(model: &CloudModel) -> HashSet<String> {
    model
        .as_cloud_objects()
        .filter(|obj| !obj.is_trashed(model))
        .map(|obj| obj.uid())
        .collect()
}

#[test]
fn active_object_uids_matches_naive_with_no_trashed_objects() {
    let folder_id = SyncId::ServerId(1.into());
    let objects: Vec<Box<dyn CloudObject>> = vec![
        Box::new(mock_cloud_folder(folder_id, "Folder".into(), None)),
        Box::new(mock_cloud_notebook(
            SyncId::ServerId(2.into()),
            "Notebook".into(),
            Some(folder_id),
        )),
    ];

    App::test((), |mut app| async move {
        let cloud_model = create_cloud_model(&mut app, objects);
        cloud_model.read(&app, |model, _| {
            assert_eq!(model.active_object_uids(), naive_active_object_uids(model));
            assert_eq!(model.active_object_uids().len(), 2);
        });
    });
}

#[test]
fn active_object_uids_matches_naive_with_directly_trashed_object() {
    let trashed_folder_id = SyncId::ServerId(1.into());
    let active_notebook_id = SyncId::ServerId(2.into());
    let objects: Vec<Box<dyn CloudObject>> = vec![
        Box::new(mock_trashed_cloud_folder(
            trashed_folder_id,
            "Trashed Folder".into(),
            None,
        )),
        Box::new(mock_cloud_notebook(
            active_notebook_id,
            "Active Notebook".into(),
            None,
        )),
    ];

    App::test((), |mut app| async move {
        let cloud_model = create_cloud_model(&mut app, objects);
        cloud_model.read(&app, |model, _| {
            let active = model.active_object_uids();
            assert_eq!(active, naive_active_object_uids(model));
            assert_eq!(active.len(), 1);
            assert!(active.contains(&active_notebook_id.uid()));
            assert!(!active.contains(&trashed_folder_id.uid()));
        });
    });
}

#[test]
fn active_object_uids_matches_naive_with_indirectly_trashed_children() {
    // A trashed folder with a non-trashed notebook inside it.
    // The notebook should be considered trashed (indirectly) by both approaches.
    let trashed_folder_id = SyncId::ServerId(1.into());
    let child_notebook_id = SyncId::ServerId(2.into());
    let active_notebook_id = SyncId::ServerId(3.into());
    let objects: Vec<Box<dyn CloudObject>> = vec![
        Box::new(mock_trashed_cloud_folder(
            trashed_folder_id,
            "Trashed Folder".into(),
            None,
        )),
        Box::new(mock_cloud_notebook(
            child_notebook_id,
            "Child in Trashed Folder".into(),
            Some(trashed_folder_id),
        )),
        Box::new(mock_cloud_notebook(
            active_notebook_id,
            "Top-level Notebook".into(),
            None,
        )),
    ];

    App::test((), |mut app| async move {
        let cloud_model = create_cloud_model(&mut app, objects);
        cloud_model.read(&app, |model, _| {
            let active = model.active_object_uids();
            assert_eq!(active, naive_active_object_uids(model));
            assert_eq!(active.len(), 1);
            assert!(active.contains(&active_notebook_id.uid()));
        });
    });
}

#[test]
fn active_object_uids_matches_naive_with_nested_trashed_folder() {
    // folder_a (trashed) -> folder_b (not trashed) -> notebook (not trashed)
    // Both folder_b and notebook should be indirectly trashed.
    let folder_a_id = SyncId::ServerId(1.into());
    let folder_b_id = SyncId::ServerId(2.into());
    let notebook_id = SyncId::ServerId(3.into());
    let active_notebook_id = SyncId::ServerId(4.into());
    let objects: Vec<Box<dyn CloudObject>> = vec![
        Box::new(mock_trashed_cloud_folder(
            folder_a_id,
            "Folder A (trashed)".into(),
            None,
        )),
        Box::new(mock_cloud_folder(
            folder_b_id,
            "Folder B".into(),
            Some(folder_a_id),
        )),
        Box::new(mock_cloud_notebook(
            notebook_id,
            "Deeply nested".into(),
            Some(folder_b_id),
        )),
        Box::new(mock_cloud_notebook(
            active_notebook_id,
            "Active".into(),
            None,
        )),
    ];

    App::test((), |mut app| async move {
        let cloud_model = create_cloud_model(&mut app, objects);
        cloud_model.read(&app, |model, _| {
            let active = model.active_object_uids();
            assert_eq!(active, naive_active_object_uids(model));
            assert_eq!(active.len(), 1);
            assert!(active.contains(&active_notebook_id.uid()));
        });
    });
}

#[test]
fn active_object_uids_matches_naive_with_empty_model() {
    App::test((), |mut app| async move {
        let cloud_model = create_cloud_model(&mut app, vec![]);
        cloud_model.read(&app, |model, _| {
            let active = model.active_object_uids();
            assert_eq!(active, naive_active_object_uids(model));
            assert!(active.is_empty());
        });
    });
}
