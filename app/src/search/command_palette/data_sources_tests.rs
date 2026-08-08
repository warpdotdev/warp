use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use cloud_object_client::MockObjectClient;
use settings::manager::SettingsManager;
use warpui::{App, SingletonEntity, WindowId};

use super::*;
use crate::auth::AuthStateProvider;
use crate::cloud_object::model::persistence::{CloudModel, UpdateSource};
use crate::cloud_object::model::view::CloudViewModel;
use crate::cloud_object::{
    Owner, Revision, ServerMetadata, ServerNotebook, ServerPermissions, ServerWorkflow,
};
use crate::features::FeatureFlag;
use crate::network::NetworkStatus;
use crate::notebooks::manager::NotebookManager;
use crate::notebooks::{CloudNotebookModel, NotebookId};
use crate::search::data_source::Query;
use crate::server::cloud_objects::update_manager::UpdateManager;
use crate::server::ids::SyncId::{self};
use crate::server::ids::{ObjectUid, ServerId};
use crate::server::server_api::ServerApiProvider;
use crate::server::server_api::team::MockTeamClient;
use crate::server::server_api::workspace::MockWorkspaceClient;
use crate::server::sync_queue::SyncQueue;
use crate::settings::AISettings;
use crate::system::SystemStats;
use crate::workflows::workflow::Workflow;
use crate::workflows::{CloudWorkflowModel, WorkflowId};
use crate::workspaces::team::Team;
use crate::workspaces::team_tester::TeamTesterStatus;
use crate::workspaces::user_profiles::UserProfiles;
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::workspaces::workspace::Workspace;

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
    mock_named_server_workflow(id, owner, format!("foo{id}"), format!("bar{id}"))
}

fn mock_named_server_workflow(
    id: WorkflowId,
    owner: Owner,
    name: impl Into<String>,
    command: impl Into<String>,
) -> ServerWorkflow {
    ServerWorkflow::new(
        SyncId::ServerId(id.into()),
        CloudWorkflowModel::new(Workflow::new(name, command)),
        mock_server_metadata(),
        mock_server_permissions(owner),
    )
}

fn team_for_test(uid: i64, name: &str) -> Team {
    Team {
        uid: uid.into(),
        name: name.to_owned(),
        color: None,
        invite_code: None,
        members: vec![],
        pending_email_invites: vec![],
        invite_link_domain_restrictions: vec![],
        billing_metadata: Default::default(),
        stripe_customer_id: None,
        settings: Default::default(),
        is_eligible_for_discovery: false,
        has_billing_history: false,
    }
}

fn workspace_for_test(teams: Vec<Team>) -> Workspace {
    Workspace {
        uid: "workspace_uid123456789".to_string().into(),
        name: "test".to_string(),
        stripe_customer_id: None,
        teams,
        billing_metadata: Default::default(),
        bonus_grants_purchased_this_month: Default::default(),
        billing_cycle_usage: None,
        has_billing_history: false,
        settings: Default::default(),
        invite_code: None,
        invite_link_domain_restrictions: vec![],
        pending_email_invites: vec![],
        is_eligible_for_discovery: false,
        members: vec![],
        total_requests_used_since_last_refresh: 0,
    }
}

fn mock_server_notebook(id: NotebookId, owner: Owner) -> ServerNotebook {
    ServerNotebook::new(
        SyncId::ServerId(id.into()),
        CloudNotebookModel {
            title: format!("foo{id}"),
            data: format!("bar{id}"),
            ai_document_id: None,
            conversation_id: None,
        },
        mock_server_metadata(),
        mock_server_permissions(owner),
    )
}

fn initialize_app(app: &mut App, workspaces: Vec<Workspace>) {
    // Add the necessary singleton models to the App
    app.add_singleton_model(|_| NetworkStatus::new());
    app.add_singleton_model(|_| SystemStats::new());
    let mock_team_client = Arc::new(MockTeamClient::new());
    let mock_workspace_client = Arc::new(MockWorkspaceClient::new());
    app.add_singleton_model(|ctx| {
        UserWorkspaces::mock(
            mock_team_client.clone(),
            mock_workspace_client.clone(),
            workspaces,
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
        initialize_app(&mut app, vec![]);
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
        let data_source_handle =
            app.add_model(|ctx| warp_drive::DataSource::new(WindowId::new(), ctx));
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
        initialize_app(&mut app, vec![]);
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
        let data_source_handle =
            app.add_model(|ctx| warp_drive::DataSource::new(WindowId::new(), ctx));
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
        initialize_app(&mut app, vec![]);
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
        let data_source_handle =
            app.add_model(|ctx| warp_drive::DataSource::new(WindowId::new(), ctx));
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
        initialize_app(&mut app, vec![]);
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
        let data_source_handle =
            app.add_model(|ctx| warp_drive::DataSource::new(WindowId::new(), ctx));
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

/// The full-text index is written on the background executor.
const INDEX_SETTLE: Duration = Duration::from_millis(750);

fn workflow_labels(
    mixer: &ModelHandle<CommandPaletteMixer>,
    query: &str,
    app: &mut App,
) -> Vec<String> {
    mixer.update(app, |mixer, ctx| {
        mixer.run_query(
            Query {
                filters: HashSet::from([QueryFilter::Workflows]),
                text: query.into(),
            },
            ctx,
        );
    });
    app.read(|app| {
        let mut labels = mixer
            .as_ref(app)
            .results()
            .iter()
            .map(|result| result.accessibility_label())
            .collect::<Vec<_>>();
        labels.sort();
        labels
    })
}

fn workflow_label(name: &str) -> String {
    format!("Workflow: {name}")
}

fn prompt_or_workflow_uid(id: i64) -> ObjectUid {
    SyncId::ServerId(WorkflowId::from(id).into()).uid()
}

#[test]
fn test_drive_data_source_only_returns_objects_visible_in_the_window() {
    let selected_team = team_for_test(123, "selected");
    let other_team = team_for_test(456, "other");
    let workspace = workspace_for_test(vec![selected_team.clone(), other_team.clone()]);

    App::test((), |mut app| async move {
        initialize_app(&mut app, vec![workspace]);
        CloudModel::handle(&app).update(&mut app, |model, ctx| {
            model.upsert_from_server_workflow(
                mock_named_server_workflow(
                    1.into(),
                    Owner::Team {
                        team_uid: selected_team.uid,
                    },
                    "selected team workflow",
                    "echo selected",
                ),
                ctx,
            );
            model.upsert_from_server_workflow(
                mock_named_server_workflow(
                    2.into(),
                    Owner::Team {
                        team_uid: other_team.uid,
                    },
                    "other team workflow",
                    "echo other",
                ),
                ctx,
            );
            model.upsert_from_server_workflow(
                mock_named_server_workflow(
                    3.into(),
                    Owner::mock_current_user(),
                    "personal workflow",
                    "echo personal",
                ),
                ctx,
            );
        });

        let window_id = WindowId::new();
        UserWorkspaces::handle(&app).update(&mut app, |user_workspaces, ctx| {
            user_workspaces.set_team_for_window(window_id, selected_team.uid, ctx);
        });

        let mixer = app.add_model(|_| CommandPaletteMixer::new());
        let data_source_handle = app.add_model(|ctx| warp_drive::DataSource::new(window_id, ctx));
        mixer.update(&mut app, |mixer, _| {
            mixer.add_sync_source(data_source_handle, [QueryFilter::Workflows]);
        });

        assert_eq!(
            workflow_labels(&mixer, "workflow", &mut app),
            vec![
                workflow_label("personal workflow"),
                workflow_label("selected team workflow"),
            ]
        );
    })
}

/// Enough out-of-window workflows to more than fill the full-text searcher's result cap. They must
/// never reach the ranker: they are not in this window's corpus at all.
const CROWDING_WORKFLOW_COUNT: i64 = 25;

#[test]
fn test_full_text_drive_data_source_finds_in_window_objects_outranked_by_another_team() {
    let _flag = FeatureFlag::UseTantivySearch.override_enabled(true);
    let selected_team = team_for_test(123, "selected");
    let other_team = team_for_test(456, "other");
    let workspace = workspace_for_test(vec![selected_team.clone(), other_team.clone()]);

    App::test((), |mut app| async move {
        initialize_app(&mut app, vec![workspace]);

        let window_id = WindowId::new();
        UserWorkspaces::handle(&app).update(&mut app, |user_workspaces, ctx| {
            user_workspaces.set_team_for_window(window_id, selected_team.uid, ctx);
        });

        CloudModel::handle(&app).update(&mut app, |model, ctx| {
            // The other team's workflows match "deploy" in both name and content, so every one of
            // them would outrank the in-window workflow if they shared a corpus.
            for id in 1..=CROWDING_WORKFLOW_COUNT {
                model.upsert_from_server_workflow(
                    mock_named_server_workflow(
                        id.into(),
                        Owner::Team {
                            team_uid: other_team.uid,
                        },
                        "deploy",
                        "deploy deploy deploy",
                    ),
                    ctx,
                );
            }
            model.upsert_from_server_workflow(
                mock_named_server_workflow(
                    (CROWDING_WORKFLOW_COUNT + 1).into(),
                    Owner::Team {
                        team_uid: selected_team.uid,
                    },
                    "release notes generator",
                    "a long command that only mentions deploy once, near the very end of the text",
                ),
                ctx,
            );
        });

        let mixer = app.add_model(|_| CommandPaletteMixer::new());
        let data_source_handle = app.add_model(|ctx| warp_drive::DataSource::new(window_id, ctx));
        std::thread::sleep(INDEX_SETTLE);
        mixer.update(&mut app, |mixer, _| {
            mixer.add_sync_source(data_source_handle, [QueryFilter::Workflows]);
        });

        assert_eq!(
            workflow_labels(&mixer, "deploy", &mut app),
            vec![workflow_label("release notes generator")]
        );
    })
}

#[test]
fn test_full_text_drive_data_source_indexes_an_object_that_moves_into_the_window() {
    let _flag = FeatureFlag::UseTantivySearch.override_enabled(true);
    let selected_team = team_for_test(123, "selected");
    let other_team = team_for_test(456, "other");
    let workspace = workspace_for_test(vec![selected_team.clone(), other_team.clone()]);

    App::test((), |mut app| async move {
        initialize_app(&mut app, vec![workspace]);

        let window_id = WindowId::new();
        UserWorkspaces::handle(&app).update(&mut app, |user_workspaces, ctx| {
            user_workspaces.set_team_for_window(window_id, selected_team.uid, ctx);
        });

        CloudModel::handle(&app).update(&mut app, |model, ctx| {
            model.upsert_from_server_workflow(
                mock_named_server_workflow(
                    1.into(),
                    Owner::Team {
                        team_uid: other_team.uid,
                    },
                    "migrating workflow",
                    "echo migrating",
                ),
                ctx,
            );
        });

        let mixer = app.add_model(|_| CommandPaletteMixer::new());
        let data_source_handle = app.add_model(|ctx| warp_drive::DataSource::new(window_id, ctx));
        std::thread::sleep(INDEX_SETTLE);
        mixer.update(&mut app, |mixer, _| {
            mixer.add_sync_source(data_source_handle, [QueryFilter::Workflows]);
        });

        assert!(
            workflow_labels(&mixer, "migrating", &mut app).is_empty(),
            "another team's workflow should not be in this window's corpus"
        );

        // The server reassigns the workflow to this window's team.
        CloudModel::handle(&app).update(&mut app, |model, ctx| {
            model.update_object_permissions(
                &prompt_or_workflow_uid(1),
                mock_server_permissions(Owner::Team {
                    team_uid: selected_team.uid,
                }),
                UpdateSource::Server,
                ctx,
            );
        });
        std::thread::sleep(INDEX_SETTLE);

        assert_eq!(
            workflow_labels(&mixer, "migrating", &mut app),
            vec![workflow_label("migrating workflow")]
        );
    })
}

#[test]
fn test_full_text_drive_data_source_removes_an_object_that_moves_out_of_the_window() {
    let _flag = FeatureFlag::UseTantivySearch.override_enabled(true);
    let selected_team = team_for_test(123, "selected");
    let other_team = team_for_test(456, "other");
    let workspace = workspace_for_test(vec![selected_team.clone(), other_team.clone()]);

    App::test((), |mut app| async move {
        initialize_app(&mut app, vec![workspace]);

        let window_id = WindowId::new();
        UserWorkspaces::handle(&app).update(&mut app, |user_workspaces, ctx| {
            user_workspaces.set_team_for_window(window_id, selected_team.uid, ctx);
        });

        CloudModel::handle(&app).update(&mut app, |model, ctx| {
            model.upsert_from_server_workflow(
                mock_named_server_workflow(
                    1.into(),
                    Owner::Team {
                        team_uid: selected_team.uid,
                    },
                    "departing workflow",
                    "echo departing",
                ),
                ctx,
            );
        });

        let mixer = app.add_model(|_| CommandPaletteMixer::new());
        let data_source_handle = app.add_model(|ctx| warp_drive::DataSource::new(window_id, ctx));
        std::thread::sleep(INDEX_SETTLE);
        mixer.update(&mut app, |mixer, _| {
            mixer.add_sync_source(data_source_handle, [QueryFilter::Workflows]);
        });

        assert_eq!(
            workflow_labels(&mixer, "departing", &mut app),
            vec![workflow_label("departing workflow")]
        );

        // The user moves the workflow into the other team's drive.
        CloudModel::handle(&app).update(&mut app, |model, ctx| {
            model.update_object_location(
                &prompt_or_workflow_uid(1),
                Some(Owner::Team {
                    team_uid: other_team.uid,
                }),
                None,
                ctx,
            );
        });
        std::thread::sleep(INDEX_SETTLE);

        assert!(
            workflow_labels(&mixer, "departing", &mut app).is_empty(),
            "a workflow that leaves the window's spaces should be removed from its index"
        );
    })
}

#[test]
fn test_full_text_drive_data_source_rebuilds_when_the_windows_team_changes() {
    let _flag = FeatureFlag::UseTantivySearch.override_enabled(true);
    let first_team = team_for_test(123, "first");
    let second_team = team_for_test(456, "second");
    let workspace = workspace_for_test(vec![first_team.clone(), second_team.clone()]);

    App::test((), |mut app| async move {
        initialize_app(&mut app, vec![workspace]);

        CloudModel::handle(&app).update(&mut app, |model, ctx| {
            model.upsert_from_server_workflow(
                mock_named_server_workflow(
                    1.into(),
                    Owner::Team {
                        team_uid: second_team.uid,
                    },
                    "second team workflow",
                    "echo second",
                ),
                ctx,
            );
        });

        // The window has no team yet, so the second team's workflow is out of scope.
        let window_id = WindowId::new();
        let mixer = app.add_model(|_| CommandPaletteMixer::new());
        let data_source_handle = app.add_model(|ctx| warp_drive::DataSource::new(window_id, ctx));
        std::thread::sleep(INDEX_SETTLE);
        mixer.update(&mut app, |mixer, _| {
            mixer.add_sync_source(data_source_handle, [QueryFilter::Workflows]);
        });

        assert!(workflow_labels(&mixer, "second", &mut app).is_empty());

        UserWorkspaces::handle(&app).update(&mut app, |user_workspaces, ctx| {
            user_workspaces.set_team_for_window(window_id, second_team.uid, ctx);
        });
        std::thread::sleep(INDEX_SETTLE);

        assert_eq!(
            workflow_labels(&mixer, "second", &mut app),
            vec![workflow_label("second team workflow")]
        );
    })
}

#[test]
fn test_drive_data_sources_for_different_windows_stay_independent() {
    let first_team = team_for_test(123, "first");
    let second_team = team_for_test(456, "second");
    let workspace = workspace_for_test(vec![first_team.clone(), second_team.clone()]);

    App::test((), |mut app| async move {
        initialize_app(&mut app, vec![workspace]);
        CloudModel::handle(&app).update(&mut app, |model, ctx| {
            model.upsert_from_server_workflow(
                mock_named_server_workflow(
                    1.into(),
                    Owner::Team {
                        team_uid: first_team.uid,
                    },
                    "first team workflow",
                    "echo first",
                ),
                ctx,
            );
            model.upsert_from_server_workflow(
                mock_named_server_workflow(
                    2.into(),
                    Owner::Team {
                        team_uid: second_team.uid,
                    },
                    "second team workflow",
                    "echo second",
                ),
                ctx,
            );
        });

        let first_window_id = WindowId::new();
        let second_window_id = WindowId::new();
        UserWorkspaces::handle(&app).update(&mut app, |user_workspaces, ctx| {
            user_workspaces.set_team_for_window(first_window_id, first_team.uid, ctx);
            user_workspaces.set_team_for_window(second_window_id, second_team.uid, ctx);
        });

        let first_mixer = app.add_model(|_| CommandPaletteMixer::new());
        let first_data_source =
            app.add_model(|ctx| warp_drive::DataSource::new(first_window_id, ctx));
        first_mixer.update(&mut app, |mixer, _| {
            mixer.add_sync_source(first_data_source, [QueryFilter::Workflows]);
        });

        let second_mixer = app.add_model(|_| CommandPaletteMixer::new());
        let second_data_source =
            app.add_model(|ctx| warp_drive::DataSource::new(second_window_id, ctx));
        second_mixer.update(&mut app, |mixer, _| {
            mixer.add_sync_source(second_data_source, [QueryFilter::Workflows]);
        });

        assert_eq!(
            workflow_labels(&first_mixer, "workflow", &mut app),
            vec![workflow_label("first team workflow")]
        );
        assert_eq!(
            workflow_labels(&second_mixer, "workflow", &mut app),
            vec![workflow_label("second team workflow")]
        );
    })
}
