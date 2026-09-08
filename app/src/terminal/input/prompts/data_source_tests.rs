use std::sync::Arc;

use chrono::Utc;
use cloud_object_client::MockObjectClient;
use itertools::Itertools;
use settings::manager::SettingsManager;
use warpui::{App, SingletonEntity};

use super::*;
use crate::auth::AuthStateProvider;
use crate::cloud_object::model::view::CloudViewModel;
use crate::cloud_object::{
    Owner, Revision, ServerMetadata, ServerPermissions, ServerWorkflow, Space,
};
use crate::network::NetworkStatus;
use crate::notebooks::manager::NotebookManager;
use crate::search::data_source::Query;
use crate::server::cloud_objects::update_manager::UpdateManager;
use crate::server::ids::ServerId;
use crate::server::server_api::ServerApiProvider;
use crate::server::server_api::team::MockTeamClient;
use crate::server::server_api::workspace::MockWorkspaceClient;
use crate::server::sync_queue::SyncQueue;
use crate::settings::AISettings;
use crate::system::SystemStats;
use crate::workflows::workflow::Workflow;
use crate::workflows::{CloudWorkflowModel, WorkflowId};
use crate::workspaces::team::{Team, TeamVisibility};
use crate::workspaces::team_tester::TeamTesterStatus;
use crate::workspaces::user_profiles::UserProfiles;
use crate::workspaces::workspace::Workspace;

const IN_WINDOW_TEAM_PROMPT_ID: i64 = 1;
const OTHER_TEAM_PROMPT_ID: i64 = 2;
const PERSONAL_PROMPT_ID: i64 = 3;

fn mock_server_prompt(id: WorkflowId, owner: Owner, name: &str) -> ServerWorkflow {
    ServerWorkflow::new(
        SyncId::ServerId(id.into()),
        CloudWorkflowModel::new(Workflow::AgentMode {
            name: name.to_owned(),
            query: format!("do {name}"),
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
            space: owner,
            guests: Vec::new(),
            anyone_link_sharing: None,
            permissions_last_updated_ts: Utc::now().into(),
        },
    )
}

fn team_for_test(uid: i64, name: &str) -> Team {
    Team {
        uid: uid.into(),
        name: name.to_owned(),
        color: None,
        invite_link: None,
        members: vec![],
        pending_email_invites: vec![],
        invite_link_domain_restrictions: vec![],
        billing_metadata: Default::default(),
        stripe_customer_id: None,
        settings: Default::default(),
        feature_model_choice: Default::default(),
        is_eligible_for_discovery: false,
        has_billing_history: false,
        visibility: TeamVisibility::Open,
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
        feature_model_choice: Default::default(),
        invite_link_domain_restrictions: vec![],
        pending_email_invites: vec![],
        is_eligible_for_discovery: false,
        members: vec![],
        total_requests_used_since_last_refresh: 0,
    }
}

fn initialize_app(app: &mut App, workspaces: Vec<Workspace>) {
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

/// Seeds one prompt in the window's team space, one in another team's space and one personal
/// prompt. Every name starts with "a" so only the window scoping can narrow a prefix search.
fn seed_prompts(app: &mut App, in_window_team: &Team, other_team: &Team) {
    CloudModel::handle(app).update(app, |model, ctx| {
        model.upsert_from_server_workflow(
            mock_server_prompt(
                IN_WINDOW_TEAM_PROMPT_ID.into(),
                Owner::Team {
                    team_uid: in_window_team.uid,
                },
                "align the release notes",
            ),
            ctx,
        );
        model.upsert_from_server_workflow(
            mock_server_prompt(
                OTHER_TEAM_PROMPT_ID.into(),
                Owner::Team {
                    team_uid: other_team.uid,
                },
                "audit the other team's billing",
            ),
            ctx,
        );
        model.upsert_from_server_workflow(
            mock_server_prompt(
                PERSONAL_PROMPT_ID.into(),
                Owner::mock_current_user(),
                "annotate my scratch notes",
            ),
            ctx,
        );
    });
}

fn prompt_uid(id: i64) -> String {
    SyncId::ServerId(WorkflowId::from(id).into()).uid()
}

fn prompt_ids_for_query(
    data_source: &ModelHandle<PromptsMenuDataSource>,
    query: &str,
    app: &App,
) -> Vec<String> {
    app.read(|app| {
        data_source
            .as_ref(app)
            .run_query(&Query::from(query), app)
            .expect("prompts menu query should succeed")
            .iter()
            .map(|result| result.accept_result().id.uid())
            .sorted()
            .collect()
    })
}

/// The `#` menu's empty-query path reads the cloud model directly rather than going through the
/// Warp Drive data source, so it needs its own window scoping.
#[test]
fn test_prompts_menu_empty_query_only_returns_prompts_in_the_window() {
    let in_window_team = team_for_test(123, "selected");
    let other_team = team_for_test(456, "other");
    let workspace = workspace_for_test(vec![in_window_team.clone(), other_team.clone()]);

    App::test((), |mut app| async move {
        initialize_app(&mut app, vec![workspace]);
        seed_prompts(&mut app, &in_window_team, &other_team);

        let window_id = WindowId::new();
        UserWorkspaces::handle(&app).update(&mut app, |user_workspaces, ctx| {
            user_workspaces.set_team_for_window(window_id, in_window_team.uid, ctx);
        });

        let data_source = app.add_model(|ctx| PromptsMenuDataSource::new(window_id, ctx));

        assert_eq!(
            prompt_ids_for_query(&data_source, "", &app),
            vec![
                prompt_uid(IN_WINDOW_TEAM_PROMPT_ID),
                prompt_uid(PERSONAL_PROMPT_ID)
            ]
            .into_iter()
            .sorted()
            .collect::<Vec<_>>()
        );
    })
}

/// The single-character path takes a separate prefix match that also reads the cloud model.
#[test]
fn test_prompts_menu_single_character_query_only_returns_prompts_in_the_window() {
    let in_window_team = team_for_test(123, "selected");
    let other_team = team_for_test(456, "other");
    let workspace = workspace_for_test(vec![in_window_team.clone(), other_team.clone()]);

    App::test((), |mut app| async move {
        initialize_app(&mut app, vec![workspace]);
        seed_prompts(&mut app, &in_window_team, &other_team);

        let window_id = WindowId::new();
        UserWorkspaces::handle(&app).update(&mut app, |user_workspaces, ctx| {
            user_workspaces.set_team_for_window(window_id, in_window_team.uid, ctx);
        });

        let data_source = app.add_model(|ctx| PromptsMenuDataSource::new(window_id, ctx));

        assert_eq!(
            prompt_ids_for_query(&data_source, "a", &app),
            vec![
                prompt_uid(IN_WINDOW_TEAM_PROMPT_ID),
                prompt_uid(PERSONAL_PROMPT_ID)
            ]
            .into_iter()
            .sorted()
            .collect::<Vec<_>>()
        );
    })
}

/// The normal search path delegates to the window-scoped Warp Drive data source.
#[test]
fn test_prompts_menu_search_query_only_returns_prompts_in_the_window() {
    let in_window_team = team_for_test(123, "selected");
    let other_team = team_for_test(456, "other");
    let workspace = workspace_for_test(vec![in_window_team.clone(), other_team.clone()]);

    App::test((), |mut app| async move {
        initialize_app(&mut app, vec![workspace]);
        CloudModel::handle(&app).update(&mut app, |model, ctx| {
            model.upsert_from_server_workflow(
                mock_server_prompt(
                    IN_WINDOW_TEAM_PROMPT_ID.into(),
                    Owner::Team {
                        team_uid: in_window_team.uid,
                    },
                    "deployment runbook",
                ),
                ctx,
            );
            model.upsert_from_server_workflow(
                mock_server_prompt(
                    OTHER_TEAM_PROMPT_ID.into(),
                    Owner::Team {
                        team_uid: other_team.uid,
                    },
                    "deployment checklist",
                ),
                ctx,
            );
        });

        let window_id = WindowId::new();
        UserWorkspaces::handle(&app).update(&mut app, |user_workspaces, ctx| {
            user_workspaces.set_team_for_window(window_id, in_window_team.uid, ctx);
        });

        let data_source = app.add_model(|ctx| PromptsMenuDataSource::new(window_id, ctx));

        assert_eq!(
            prompt_ids_for_query(&data_source, "deployment", &app),
            vec![prompt_uid(IN_WINDOW_TEAM_PROMPT_ID)]
        );
    })
}

/// A window with no team still sees the user's personal prompts.
#[test]
fn test_prompts_menu_teamless_window_returns_personal_prompts() {
    let in_window_team = team_for_test(123, "selected");
    let other_team = team_for_test(456, "other");

    App::test((), |mut app| async move {
        initialize_app(&mut app, vec![]);
        seed_prompts(&mut app, &in_window_team, &other_team);

        let window_id = WindowId::new();
        let data_source = app.add_model(|ctx| PromptsMenuDataSource::new(window_id, ctx));

        app.read(|app| {
            assert_eq!(
                UserWorkspaces::as_ref(app).spaces_for_window(window_id, app),
                vec![Space::Personal]
            );
        });
        assert_eq!(
            prompt_ids_for_query(&data_source, "", &app),
            vec![prompt_uid(PERSONAL_PROMPT_ID)]
        );
    })
}
