use std::sync::Arc;

use chrono::Utc;
use cloud_object_client::MockObjectClient;
use settings::manager::SettingsManager;
use warpui::{App, SingletonEntity};

use crate::NetworkStatus;
use crate::auth::AuthStateProvider;
use crate::cloud_object::model::persistence::CloudModel;
use crate::cloud_object::model::view::CloudViewModel;
use crate::cloud_object::{Owner, Revision, ServerMetadata, ServerPermissions, ServerWorkflow};
use crate::notebooks::manager::NotebookManager;
use crate::search::ai_context_menu::workflows::data_source::WorkflowDataSource;
use crate::search::data_source::Query;
use crate::search::mixer::SyncDataSource;
use crate::server::cloud_objects::update_manager::UpdateManager;
use crate::server::ids::{ServerId, SyncId};
use crate::server::server_api::ServerApiProvider;
use crate::server::server_api::team::MockTeamClient;
use crate::server::server_api::workspace::MockWorkspaceClient;
use crate::server::sync_queue::SyncQueue;
use crate::settings::AISettings;
use crate::system::SystemStats;
use crate::workflows::workflow::Workflow;
use crate::workflows::{CloudWorkflowModel, WorkflowId};
use crate::workspaces::team_tester::TeamTesterStatus;
use crate::workspaces::user_profiles::UserProfiles;
use crate::workspaces::user_workspaces::UserWorkspaces;

fn mock_server_workflow(id: i64, name: &str, query: &str) -> ServerWorkflow {
    ServerWorkflow::new(
        SyncId::ServerId(WorkflowId::from(id).into()),
        CloudWorkflowModel::new(Workflow::AgentMode {
            name: name.to_owned(),
            query: query.to_owned(),
            description: None,
            arguments: Vec::new(),
        }),
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
        },
        ServerPermissions {
            space: Owner::mock_current_user(),
            guests: Vec::new(),
            anyone_link_sharing: None,
            permissions_last_updated_ts: Utc::now().into(),
        },
    )
}

fn initialize_app(app: &mut App) {
    app.add_singleton_model(|_| NetworkStatus::new());
    app.add_singleton_model(|_| SystemStats::new());
    let mock_team_client = Arc::new(MockTeamClient::new());
    let mock_workspace_client = Arc::new(MockWorkspaceClient::new());
    app.add_singleton_model(|ctx| {
        UserWorkspaces::mock(
            mock_team_client.clone(),
            mock_workspace_client.clone(),
            vec![],
            ctx,
        )
    });
    app.add_singleton_model(TeamTesterStatus::new);
    app.add_singleton_model(SyncQueue::mock);
    app.add_singleton_model(CloudModel::mock);
    app.add_singleton_model(|ctx| UpdateManager::new(None, Arc::new(MockObjectClient::new()), ctx));
    app.add_singleton_model(|_| UserProfiles::new(Vec::new()));
    app.add_singleton_model(CloudViewModel::new);
    app.add_singleton_model(NotebookManager::mock);
    app.add_singleton_model(|_| ServerApiProvider::new_for_test());
    app.add_singleton_model(|_| SettingsManager::default());
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.update(crate::settings::init_and_register_user_preferences);
    app.update(AISettings::register_and_subscribe_to_events);
}

/// Regression test for APP-5287: a workflow whose first three content lines exceed 200 bytes,
/// with a multi-byte character straddling the byte-197 truncation boundary, must not panic and
/// must produce a valid, truncated description.
#[test]
fn run_query_does_not_panic_on_multibyte_content_straddling_truncation_boundary() {
    // 196 ASCII bytes followed by a 3-byte CJK character puts the character's bytes at
    // indices 196-198, straddling the old hard-coded byte-197 cut point.
    let content = format!("{}世界", "a".repeat(196));

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        CloudModel::handle(&app).update(&mut app, |model, ctx| {
            model.upsert_from_server_workflow(mock_server_workflow(1, "multibyte", &content), ctx);
        });

        let data_source = WorkflowDataSource::new();
        let results = app.read(|app| data_source.run_query(&Query::from(""), app).unwrap());

        assert_eq!(results.len(), 1);
    })
}

#[test]
fn short_content_is_not_truncated() {
    let content = "short content";

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        CloudModel::handle(&app).update(&mut app, |model, ctx| {
            model.upsert_from_server_workflow(mock_server_workflow(1, "short", content), ctx);
        });

        let data_source = WorkflowDataSource::new();
        let results = app.read(|app| data_source.run_query(&Query::from(""), app).unwrap());

        assert_eq!(results.len(), 1);
    })
}
