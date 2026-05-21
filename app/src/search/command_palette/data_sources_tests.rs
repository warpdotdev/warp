use std::sync::Arc;

use chrono::Utc;
use settings::manager::SettingsManager;
use warpui::{App, SingletonEntity};

use super::*;
use crate::auth::AuthStateProvider;
use crate::cloud_object::Owner;
use crate::notebooks::manager::NotebookManager;
use crate::notebooks::CloudNotebookModel;
use crate::server::ids::ServerId;
use crate::server::ids::SyncId::{self};
use crate::settings::AISettings;
use crate::workflows::workflow::Workflow;
use crate::workflows::CloudWorkflowModel;
use crate::{
    cloud_object::{
        model::{persistence::CloudModel, view::CloudViewModel},
        Revision, ServerMetadata, ServerNotebook, ServerPermissions, ServerWorkflow,
    },
    network::NetworkStatus,
    notebooks::NotebookId,
    search::data_source::Query,
    server::{
        cloud_objects::update_manager::UpdateManager, server_api::ServerApiProvider,
        sync_queue::SyncQueue,
    },
    system::SystemStats,
    workflows::WorkflowId,
    workspaces::{
        team_tester::TeamTesterStatus, user_profiles::UserProfiles, user_workspaces::UserWorkspaces,
    },
};

#[cfg(test)]
use crate::server::server_api::object::MockObjectClient;
#[cfg(test)]
use crate::server::server_api::team::MockTeamClient;
#[cfg(test)]
use crate::server::server_api::workspace::MockWorkspaceClient;

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
        anyone_link_sharing: None,
        permissions_last_updated_ts: Utc::now().into(),
    }
}

fn mock_server_workflow(id: WorkflowId, owner: Owner) -> ServerWorkflow {
    ServerWorkflow {
        id: SyncId::ServerId(id.into()),
        metadata: mock_server_metadata(),
        permissions: mock_server_permissions(owner),
        model: CloudWorkflowModel::new(Workflow::new(format!("foo{id}"), format!("bar{id}"))),
    }
}

fn mock_server_notebook(id: NotebookId, owner: Owner) -> ServerNotebook {
    ServerNotebook {
        id: SyncId::ServerId(id.into()),
        metadata: mock_server_metadata(),
        permissions: mock_server_permissions(owner),
        model: CloudNotebookModel {
            title: format!("foo{id}"),
            data: format!("bar{id}"),
            ai_document_id: None,
            conversation_id: None,
        },
    }
}

fn initialize_app(app: &mut App) {
    // Add the necessary singleton models to the App
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

#[test]
fn test_drive_data_source_correctly_filters_drive_filter() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        // Initialize CloudModel
        CloudModel::handle(&app).update(&mut app, |model, ctx| {
            model.upsert_from_server_notebook(
                mock_server_notebook(1.into(), Owner::mock_current_user()),
                ctx,
            );
            model.upsert_from_server_workflow(
                mock_server_workflow(2.into(), Owner::mock_current_user()),
                ctx,
            )
        });

        let mixer = app.add_model(|_| CommandPaletteMixer::new());
        let data_source_handle = app.add_model(warp_drive::DataSource::new);
        mixer.update(&mut app, |mixer, ctx| {
            // Add the drive data source with the relevant filters
            mixer.add_sync_source(
                data_source_handle,
                [
                    QueryFilter::Drive,
                    QueryFilter::Notebooks,
                    QueryFilter::Workflows,
                ],
            );

            // Run the query with the drive filter
            mixer.run_query(
                Query {
                    filters: HashSet::from([QueryFilter::Drive]),
                    text: "foo".into(),
                },
                ctx,
            );
        });

        app.read(|app| {
            let results = mixer.as_ref(app).results();

            // Expect both of the results to be included
            assert_eq!(results.len(), 2);
        });
    })
}

#[test]
fn test_drive_data_source_correctly_filters_no_filter() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        // Initialize CloudModel
        CloudModel::handle(&app).update(&mut app, |model, ctx| {
            model.upsert_from_server_notebook(
                mock_server_notebook(1.into(), Owner::mock_current_user()),
                ctx,
            );
            model.upsert_from_server_workflow(
                mock_server_workflow(2.into(), Owner::mock_current_user()),
                ctx,
            )
        });
        let mixer = app.add_model(|_| CommandPaletteMixer::new());
        let data_source_handle = app.add_model(warp_drive::DataSource::new);
        mixer.update(&mut app, |mixer, ctx| {
            // Add the drive data source with the relevant filters
            mixer.add_sync_source(
                data_source_handle,
                [
                    QueryFilter::Drive,
                    QueryFilter::Notebooks,
                    QueryFilter::Workflows,
                ],
            );

            // Run the query with no filter
            mixer.run_query(
                Query {
                    filters: HashSet::new(),
                    text: "foo".into(),
                },
                ctx,
            );
        });

        app.read(|app| {
            let results = mixer.as_ref(app).results();

            // Expect both of the results to be included
            assert_eq!(results.len(), 2);
        });
    })
}

#[test]
fn test_drive_data_source_correctly_filters_workflow_filter() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        // Initialize CloudModel
        CloudModel::handle(&app).update(&mut app, |model, ctx| {
            model.upsert_from_server_notebook(
                mock_server_notebook(1.into(), Owner::mock_current_user()),
                ctx,
            );
            model.upsert_from_server_workflow(
                mock_server_workflow(2.into(), Owner::mock_current_user()),
                ctx,
            )
        });
        let mixer = app.add_model(|_| CommandPaletteMixer::new());
        let data_source_handle = app.add_model(warp_drive::DataSource::new);
        mixer.update(&mut app, |mixer, ctx| {
            // Add the drive data source with the relevant filters
            mixer.add_sync_source(
                data_source_handle,
                [
                    QueryFilter::Drive,
                    QueryFilter::Notebooks,
                    QueryFilter::Workflows,
                ],
            );

            // Run the query with no filter
            mixer.run_query(
                Query {
                    filters: HashSet::from([QueryFilter::Workflows]),
                    text: "foo".into(),
                },
                ctx,
            );
        });

        app.read(|app| {
            let results = mixer.as_ref(app).results();

            // Expect only the workflow result to be included
            assert_eq!(results.len(), 1);

            assert!(results[0].accessibility_label().starts_with("Workflow:"));
        });
    })
}

#[test]
fn test_drive_data_source_correctly_filters_notebook_filter() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        // Initialize CloudModel
        CloudModel::handle(&app).update(&mut app, |model, ctx| {
            model.upsert_from_server_notebook(
                mock_server_notebook(1.into(), Owner::mock_current_user()),
                ctx,
            );
            model.upsert_from_server_workflow(
                mock_server_workflow(2.into(), Owner::mock_current_user()),
                ctx,
            )
        });
        let mixer = app.add_model(|_| CommandPaletteMixer::new());
        let data_source_handle = app.add_model(warp_drive::DataSource::new);
        mixer.update(&mut app, |mixer, ctx| {
            // Add the drive data source with the relevant filters
            mixer.add_sync_source(
                data_source_handle,
                [
                    QueryFilter::Drive,
                    QueryFilter::Notebooks,
                    QueryFilter::Workflows,
                ],
            );

            // Run the query with no filter
            mixer.run_query(
                Query {
                    filters: HashSet::from([QueryFilter::Notebooks]),
                    text: "foo".into(),
                },
                ctx,
            );
        });

        app.read(|app| {
            let results = mixer.as_ref(app).results();

            // Expect only the workflow result to be included
            assert_eq!(results.len(), 1);

            assert!(results[0].accessibility_label().starts_with("Notebook:"));
        });
    })
}
