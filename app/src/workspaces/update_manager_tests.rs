use std::sync::Mutex;

use chrono::Utc;
use cloud_object_client::MockObjectClient;
use itertools::Itertools;
use warpui::{AddSingletonModel, App, ModelHandle};

use super::*;
use crate::auth::AuthManager;
use crate::cloud_object::model::actions::ObjectActions;
use crate::cloud_object::model::persistence::CloudModel;
use crate::cloud_object::{Owner, Revision, ServerMetadata, ServerPermissions, ServerWorkflow};
use crate::server::cloud_objects::update_manager::InitialLoadResponse;
use crate::server::ids::SyncId;
use crate::server::server_api::team::{LeaveTeamUserFacingError, MockTeamClient};
use crate::server::server_api::workspace::{MockWorkspaceClient, WorkspaceClient};
use crate::server::sync_queue::SyncQueue;
use crate::server::telemetry::context_provider::AppTelemetryContextProvider;
use crate::settings::PrivacySettings;
use crate::system::SystemStats;
use crate::workflows::workflow::Workflow;
use crate::workflows::{CloudWorkflow, CloudWorkflowModel, WorkflowId};
use crate::workspaces::team::Team;
use crate::workspaces::user_profiles::UserProfiles;
use crate::workspaces::workspace::{Workspace, WorkspaceUid};

fn initialize_app(
    team_client: Arc<dyn TeamClient>,
    workspace_client: Arc<dyn WorkspaceClient>,
    workspaces: Vec<Workspace>,
    app: &mut App,
) {
    app.add_singleton_model(|_| NetworkStatus::new());
    app.add_singleton_model(|_| SystemStats::new());
    app.add_singleton_model(TeamTesterStatus::new);
    app.add_singleton_model(|ctx| {
        UserWorkspaces::mock(
            team_client.clone(),
            workspace_client.clone(),
            workspaces,
            ctx,
        )
    });
    app.add_singleton_model(SyncQueue::mock);
    app.add_singleton_model(CloudModel::mock);
    app.add_singleton_model(|_| ObjectActions::new(vec![]));
    app.add_singleton_model(PrivacySettings::mock);
    app.add_singleton_model(|_| UserProfiles::new(vec![]));
    app.add_singleton_model(|_| ServerApiProvider::new_for_test());
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(AppTelemetryContextProvider::new_context_provider);
    app.add_singleton_model(AuthManager::new_for_test);
}

fn mock_workflow(id: WorkflowId, owner: Owner) -> CloudWorkflow {
    CloudWorkflow::new_from_server(mock_server_workflow(id, owner))
}

fn mock_server_workflow(id: WorkflowId, owner: Owner) -> ServerWorkflow {
    ServerWorkflow::new(
        SyncId::ServerId(id.into()),
        CloudWorkflowModel::new(Workflow::new("Test Workflow", "echo hello")),
        ServerMetadata {
            uid: id.into(),
            revision: Revision::now(),
            metadata_last_updated_ts: Utc::now().into(),
            trashed_ts: None,
            folder_id: None,
            is_welcome_object: false,
            creator_uid: None,
            last_editor_uid: None,
            current_editor_uid: None,
        },
        ServerPermissions {
            space: owner,
            permissions_last_updated_ts: Utc::now().into(),
            anyone_link_sharing: None,
            guests: vec![],
        },
    )
}

#[test]
fn test_leaving_team_removes_objects() {
    App::test((), |mut app| async move {
        let workspace_uid: WorkspaceUid = WorkspaceUid::from(ServerId::from(987));
        let team_uid: ServerId = ServerId::from(123);
        let team_workflow_id = WorkflowId::from(1);
        let personal_workflow_id = WorkflowId::from(2);
        let shared_workflow_id = WorkflowId::from(3);
        let shared_workflow = mock_server_workflow(shared_workflow_id, Owner::Team { team_uid });

        let mut team_client = MockTeamClient::new();
        team_client.expect_workspaces_metadata().returning(|| {
            Ok(WorkspacesMetadataWithPricing {
                metadata: WorkspacesMetadataResponse {
                    workspaces: vec![],
                    joinable_teams: vec![],
                    experiments: None,
                    feature_model_choices: None,
                },
                pricing_info: None,
            })
        });

        let workspace_client = MockWorkspaceClient::new();
        let team_client = Arc::new(team_client);
        let workspace_client = Arc::new(workspace_client);
        initialize_app(
            team_client.clone(),
            workspace_client.clone(),
            vec![Workspace::from_local_cache(
                workspace_uid,
                "Test Workspace".to_owned(),
                Some(vec![Team::from_local_cache(
                    team_uid,
                    "Test Team".to_owned(),
                    None,
                    None,
                    None,
                )]),
            )],
            &mut app,
        );

        // Add the initial Warp Drive objects.
        CloudModel::handle(&app).update(&mut app, |cloud_model, _| {
            cloud_model.add_object(
                SyncId::ServerId(team_workflow_id.into()),
                mock_workflow(team_workflow_id, Owner::Team { team_uid }),
            );

            cloud_model.add_object(
                SyncId::ServerId(shared_workflow_id.into()),
                CloudWorkflow::new_from_server(shared_workflow.clone()),
            );

            cloud_model.add_object(
                SyncId::ServerId(personal_workflow_id.into()),
                mock_workflow(personal_workflow_id, Owner::mock_current_user()),
            );
        });

        let mut cloud_server_api = MockObjectClient::new();
        cloud_server_api
            .expect_fetch_changed_objects()
            .returning(move |_, _| {
                Ok(InitialLoadResponse {
                    updated_workflows: vec![shared_workflow.clone()],
                    ..Default::default()
                })
            });

        let team_update_manager =
            app.add_singleton_model(|ctx| TeamUpdateManager::new(team_client, None, ctx));

        let cloud_update_manager = app
            .add_singleton_model(|ctx| UpdateManager::new(None, Arc::new(cloud_server_api), ctx));

        // Simulate leaving the team.
        team_update_manager.update(&mut app, |team_manager, ctx| {
            team_manager.on_team_left(
                team_uid,
                Ok(WorkspacesMetadataWithPricing {
                    metadata: WorkspacesMetadataResponse {
                        workspaces: vec![],
                        joinable_teams: vec![],
                        experiments: None,
                        feature_model_choices: None,
                    },
                    pricing_info: None,
                }),
                ctx,
            );
        });

        // Both team-owned objects should be removed.
        CloudModel::handle(&app).read(&app, |cloud_model, _| {
            assert_eq!(
                cloud_model
                    .cloud_objects()
                    .map(|obj| obj.uid())
                    .collect_vec(),
                vec![personal_workflow_id.to_string()]
            );
        });

        // This should also trigger a refresh.
        cloud_update_manager
            .update(&mut app, |update_manager, ctx| {
                ctx.await_spawned_future(update_manager.spawned_futures()[0])
            })
            .await;

        // The refresh will then re-add the shared workflow.
        CloudModel::handle(&app).read(&app, |cloud_model, _| {
            let mut objects = cloud_model
                .cloud_objects()
                .map(|obj| obj.uid())
                .collect_vec();
            objects.sort();
            assert_eq!(
                objects,
                vec![
                    personal_workflow_id.to_string(),
                    shared_workflow_id.to_string()
                ]
            );
        });
    });
}

fn make_team_update_manager_test_app(
    workspace_uid: WorkspaceUid,
    app: &mut App,
) -> ModelHandle<TeamUpdateManager> {
    let mut team_client = MockTeamClient::new();
    team_client.expect_workspaces_metadata().returning(|| {
        Ok(WorkspacesMetadataWithPricing {
            metadata: WorkspacesMetadataResponse {
                workspaces: vec![],
                joinable_teams: vec![],
                experiments: None,
                feature_model_choices: None,
            },
            pricing_info: None,
        })
    });
    let team_client = Arc::new(team_client);
    let workspace_client = Arc::new(MockWorkspaceClient::new());
    initialize_app(
        team_client.clone(),
        workspace_client,
        vec![Workspace::from_local_cache(
            workspace_uid,
            "Test Workspace".to_owned(),
            None,
        )],
        app,
    );
    let mut cloud_server_api = MockObjectClient::new();
    cloud_server_api
        .expect_fetch_changed_objects()
        .returning(|_, _| {
            Ok(InitialLoadResponse {
                ..Default::default()
            })
        });
    let team_update_manager =
        app.add_singleton_model(|ctx| TeamUpdateManager::new(team_client, None, ctx));
    let _cloud_update_manager =
        app.add_singleton_model(|ctx| UpdateManager::new(None, Arc::new(cloud_server_api), ctx));
    team_update_manager
}

/// When the server returns a plain transport/internal error (no `LeaveTeamUserFacingError`
/// in the chain), `on_team_left` must emit the generic fallback toast instead of
/// leaking the raw error string to the user (REV-1795).
#[test]
fn test_leave_team_transport_error_emits_fallback_message() {
    App::test((), |mut app| async move {
        let team_uid: ServerId = ServerId::from(123);
        let workspace_uid: WorkspaceUid = WorkspaceUid::from(ServerId::from(987));

        let team_update_manager = make_team_update_manager_test_app(workspace_uid, &mut app);

        // Subscribe to LeaveError events and capture the message.
        let captured_error_msg: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let captured_clone = captured_error_msg.clone();
        app.update(|ctx| {
            ctx.subscribe_to_model(&team_update_manager, move |_, event, _| {
                if let TeamUpdateManagerEvent::LeaveError(msg) = event {
                    *captured_clone.lock().unwrap() = Some(msg.clone());
                }
            });
        });

        // Simulate a plain transport/DB failure — no LeaveTeamUserFacingError in the chain.
        team_update_manager.update(&mut app, |team_manager, ctx| {
            team_manager.on_team_left(
                team_uid,
                Err(anyhow::anyhow!("Server returned an error")),
                ctx,
            );
        });

        // The generic fallback must be shown; the raw error string must NOT reach the user.
        let error_msg = captured_error_msg.lock().unwrap();
        assert_eq!(
            error_msg.as_deref(),
            Some("Failed to leave team. Please try again."),
            "a transport/internal error must fall back to the generic message"
        );
    });
}

/// When the server returns a `LeaveTeamUserFacingError`, `on_team_left` must surface
/// that message verbatim in the toast — nothing else reaches the user (REV-1795).
#[test]
fn test_leave_team_user_facing_error_emits_verbatim_message() {
    App::test((), |mut app| async move {
        let team_uid: ServerId = ServerId::from(123);
        let workspace_uid: WorkspaceUid = WorkspaceUid::from(ServerId::from(987));

        let team_update_manager = make_team_update_manager_test_app(workspace_uid, &mut app);

        // Subscribe to LeaveError events and capture the message.
        let captured_error_msg: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let captured_clone = captured_error_msg.clone();
        app.update(|ctx| {
            ctx.subscribe_to_model(&team_update_manager, move |_, event, _| {
                if let TeamUpdateManagerEvent::LeaveError(msg) = event {
                    *captured_clone.lock().unwrap() = Some(msg.clone());
                }
            });
        });

        // Simulate a server-side business-rule rejection carrying a typed user-facing message.
        let user_msg = "Cannot delete workspace with an active paid subscription".to_string();
        team_update_manager.update(&mut app, |team_manager, ctx| {
            team_manager.on_team_left(
                team_uid,
                Err(anyhow::Error::new(LeaveTeamUserFacingError(
                    user_msg.clone(),
                ))),
                ctx,
            );
        });

        // The typed message must reach the toast verbatim — no generic fallback.
        let error_msg = captured_error_msg.lock().unwrap();
        assert_eq!(
            error_msg.as_deref(),
            Some(user_msg.as_str()),
            "a typed user-facing error must be surfaced verbatim"
        );
    });
}
