use warp_cli::scope::{ObjectScope, TeamSelection};
use warpui::App;

use super::{resolve_schedule_team_scope, schedule_is_visible_to_scope};
use crate::ai::ambient_agents::scheduled::{
    CloudScheduledAmbientAgent, CloudScheduledAmbientAgentModel, ScheduledAmbientAgent,
};
use crate::cloud_object::{CloudObjectMetadata, CloudObjectPermissions, Owner};
use crate::server::ids::{ServerId, SyncId};
use crate::settings::PrivacySettings;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::workspaces::user_workspaces::{
    TeamContextForOperation, TeamScope, TeamlessScopeForTest, UserWorkspaces,
};

fn schedule_with_owner(id: i64, owner: Owner) -> CloudScheduledAmbientAgent {
    let mut permissions = CloudObjectPermissions::mock_personal();
    permissions.owner = owner;
    CloudScheduledAmbientAgent::new(
        SyncId::ServerId(ServerId::from(id)),
        CloudScheduledAmbientAgentModel::new(ScheduledAmbientAgent::new(
            "Schedule".to_string(),
            "0 9 * * 1".to_string(),
            true,
            "Prompt".to_string(),
        )),
        CloudObjectMetadata::mock(),
        permissions,
    )
}

#[test]
fn schedule_scope_resolves_team_personal_and_ambiguity() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        app.add_singleton_model(PrivacySettings::mock);
        let user_workspaces = app.add_singleton_model(UserWorkspaces::default_mock);
        user_workspaces.update(&mut app, |user_workspaces, ctx| {
            user_workspaces.setup_test_workspace(ctx);
            user_workspaces.update_current_workspace(
                |workspace| {
                    let mut second_team = workspace.teams[0].clone();
                    second_team.uid = ServerId::from(456);
                    second_team.name = "Second team".to_string();
                    workspace.teams.push(second_team);
                },
                ctx,
            );
        });
        let selected_team_uid = user_workspaces.read(&app, |user_workspaces, _| {
            user_workspaces.current_workspace().unwrap().teams[0].uid
        });

        app.read(|ctx| {
            let team_scope = resolve_schedule_team_scope(
                &ObjectScope {
                    team_selection: TeamSelection {
                        team: Some(Some(selected_team_uid.to_string())),
                    },
                    personal: false,
                },
                ctx,
            )
            .expect("an explicit member team should resolve");
            assert_eq!(team_scope.team_uid(), Some(selected_team_uid));

            let personal_scope = resolve_schedule_team_scope(
                &ObjectScope {
                    team_selection: TeamSelection { team: None },
                    personal: true,
                },
                ctx,
            )
            .expect("explicit personal scope should not require a sole team");
            assert_eq!(personal_scope.team_uid(), None);

            let ambiguous_scope = resolve_schedule_team_scope(
                &ObjectScope {
                    team_selection: TeamSelection { team: None },
                    personal: false,
                },
                ctx,
            );
            assert!(ambiguous_scope.is_err());
        });
    });
}

#[test]
fn schedule_scope_is_teamless_without_teams() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        app.add_singleton_model(PrivacySettings::mock);
        app.add_singleton_model(UserWorkspaces::default_mock);

        app.read(|ctx| {
            let scope = resolve_schedule_team_scope(
                &ObjectScope {
                    team_selection: TeamSelection { team: None },
                    personal: false,
                },
                ctx,
            )
            .expect("implicit scope should be teamless when the user has no teams");
            assert_eq!(scope.team_uid(), None);
        });
    });
}

#[test]
fn schedule_scope_includes_personal_and_matching_team_schedules() {
    let selected_team_uid = ServerId::from(123);
    let selected_scope = TeamContextForOperation::new_for_test(selected_team_uid);
    let personal_schedule = schedule_with_owner(1, Owner::mock_current_user());
    let selected_team_schedule = schedule_with_owner(
        2,
        Owner::Team {
            team_uid: selected_team_uid,
        },
    );
    let other_team_schedule = schedule_with_owner(
        3,
        Owner::Team {
            team_uid: ServerId::from(456),
        },
    );

    assert!(schedule_is_visible_to_scope(
        &personal_schedule,
        &selected_scope
    ));
    assert!(schedule_is_visible_to_scope(
        &selected_team_schedule,
        &selected_scope
    ));
    assert!(!schedule_is_visible_to_scope(
        &other_team_schedule,
        &selected_scope
    ));
}

#[test]
fn teamless_schedule_scope_includes_only_personal_schedules() {
    let personal_schedule = schedule_with_owner(1, Owner::mock_current_user());
    let team_schedule = schedule_with_owner(
        2,
        Owner::Team {
            team_uid: ServerId::from(123),
        },
    );

    assert!(schedule_is_visible_to_scope(
        &personal_schedule,
        &TeamlessScopeForTest
    ));
    assert!(!schedule_is_visible_to_scope(
        &team_schedule,
        &TeamlessScopeForTest
    ));
}
