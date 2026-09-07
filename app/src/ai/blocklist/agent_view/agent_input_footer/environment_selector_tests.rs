use std::sync::Arc;

use warpui::platform::WindowStyle;
use warpui::{App, SingletonEntity as _};

use super::{EnvironmentSelector, EnvironmentSelectorTarget};
use crate::ai::ambient_agents::telemetry::HandoffEntryPoint;
use crate::ai::cloud_environments::{
    AmbientAgentEnvironment, CloudAmbientAgentEnvironment, CloudAmbientAgentEnvironmentModel,
    CloudEnvironmentCatalog,
};
use crate::appearance::Appearance;
use crate::auth::AuthStateProvider;
use crate::cloud_object::model::persistence::CloudModel;
use crate::cloud_object::{CloudObjectMetadata, CloudObjectPermissions, Owner};
use crate::server::ids::{ServerId, SyncId};
use crate::settings_view::keybindings::KeybindingChangedNotifier;
use crate::terminal::input::{HandoffComposeState, MenuPositioning};
use crate::test_util::settings::initialize_settings_for_tests;
use crate::workspaces::user_workspaces::UserWorkspaces;

fn environment(id: SyncId, name: &str, owner: Owner) -> CloudAmbientAgentEnvironment {
    let environment = AmbientAgentEnvironment::new(
        name.to_owned(),
        None,
        Vec::new(),
        "ubuntu:latest".to_owned(),
        Vec::new(),
    );
    let mut permissions = CloudObjectPermissions::mock_personal();
    permissions.owner = owner;
    CloudAmbientAgentEnvironment::new(
        id,
        CloudAmbientAgentEnvironmentModel::new(environment),
        CloudObjectMetadata::mock(),
        permissions,
    )
}

fn initialize_app(app: &mut App, environments: Vec<CloudAmbientAgentEnvironment>) {
    initialize_settings_for_tests(app);
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(|_| Appearance::mock());
    app.add_singleton_model(|_| KeybindingChangedNotifier::new());
    app.add_singleton_model(UserWorkspaces::default_mock);
    let cloud_model = app.add_singleton_model(CloudModel::mock);
    cloud_model.update(app, |model, ctx| {
        for environment in environments {
            model.create_object(environment.id, environment, ctx);
        }
    });
    app.add_singleton_model(CloudEnvironmentCatalog::new);
}

fn add_selector(
    app: &mut App,
) -> (
    warpui::WindowId,
    warpui::ViewHandle<EnvironmentSelector>,
    warpui::ModelHandle<HandoffComposeState>,
) {
    let state = app.add_model(|_| HandoffComposeState::default());
    state.update(app, |state, ctx| {
        state.activate(HandoffEntryPoint::Ampersand, ctx);
    });
    let state_for_view = state.clone();
    let (window_id, selector) = app.add_window(WindowStyle::NotStealFocus, move |ctx| {
        EnvironmentSelector::new(
            Arc::new(MenuPositioning::AboveInputBox),
            EnvironmentSelectorTarget::Handoff(state_for_view),
            ctx,
        )
    });
    (window_id, selector, state)
}

fn visible_ids(app: &mut App, selector: &warpui::ViewHandle<EnvironmentSelector>) -> Vec<SyncId> {
    selector.update(app, |selector, ctx| selector.visible_environment_ids(ctx))
}

#[test]
fn selector_includes_personal_and_current_team_environments() {
    let team_a = ServerId::from(101);
    let team_b = ServerId::from(202);
    let personal_id = ServerId::from(1);
    let team_a_id = ServerId::from(2);
    let team_b_id = ServerId::from(3);
    App::test((), |mut app| async move {
        initialize_app(
            &mut app,
            vec![
                environment(
                    SyncId::ServerId(personal_id),
                    "Personal",
                    Owner::mock_current_user(),
                ),
                environment(
                    SyncId::ServerId(team_a_id),
                    "Team A",
                    Owner::Team { team_uid: team_a },
                ),
                environment(
                    SyncId::ServerId(team_b_id),
                    "Team B",
                    Owner::Team { team_uid: team_b },
                ),
            ],
        );
        let (window_id, selector, _) = add_selector(&mut app);
        UserWorkspaces::handle(&app).update(&mut app, |workspaces, ctx| {
            workspaces.set_team_for_window(window_id, team_a, ctx);
        });

        assert_eq!(
            visible_ids(&mut app, &selector),
            [SyncId::ServerId(personal_id), SyncId::ServerId(team_a_id)]
        );
        assert_eq!(
            app.read(|ctx| CloudEnvironmentCatalog::as_ref(ctx).environments().len()),
            3,
            "window filtering must not mutate the shared catalog"
        );
    });
}

#[test]
fn selectors_in_different_windows_keep_independent_team_scopes() {
    let team_a = ServerId::from(101);
    let team_b = ServerId::from(202);
    let team_a_id = ServerId::from(2);
    let team_b_id = ServerId::from(3);
    App::test((), |mut app| async move {
        initialize_app(
            &mut app,
            vec![
                environment(
                    SyncId::ServerId(team_a_id),
                    "Team A",
                    Owner::Team { team_uid: team_a },
                ),
                environment(
                    SyncId::ServerId(team_b_id),
                    "Team B",
                    Owner::Team { team_uid: team_b },
                ),
            ],
        );
        let (window_a, selector_a, _) = add_selector(&mut app);
        let (window_b, selector_b, _) = add_selector(&mut app);
        UserWorkspaces::handle(&app).update(&mut app, |workspaces, ctx| {
            workspaces.set_team_for_window(window_a, team_a, ctx);
            workspaces.set_team_for_window(window_b, team_b, ctx);
        });

        assert_eq!(
            visible_ids(&mut app, &selector_a),
            [SyncId::ServerId(team_a_id)]
        );
        assert_eq!(
            visible_ids(&mut app, &selector_b),
            [SyncId::ServerId(team_b_id)]
        );
    });
}

#[test]
fn team_switch_clears_an_invisible_selection_and_redefaults() {
    let team_a = ServerId::from(101);
    let team_b = ServerId::from(202);
    let team_a_id = SyncId::ServerId(ServerId::from(2));
    let team_b_id = SyncId::ServerId(ServerId::from(3));
    App::test((), |mut app| async move {
        initialize_app(
            &mut app,
            vec![
                environment(team_a_id, "Team A", Owner::Team { team_uid: team_a }),
                environment(team_b_id, "Team B", Owner::Team { team_uid: team_b }),
            ],
        );
        let (window_id, _, state) = add_selector(&mut app);
        UserWorkspaces::handle(&app).update(&mut app, |workspaces, ctx| {
            workspaces.set_team_for_window(window_id, team_a, ctx);
        });
        state.update(&mut app, |state, ctx| {
            state.set_environment_id(Some(team_a_id), true, ctx);
        });

        UserWorkspaces::handle(&app).update(&mut app, |workspaces, ctx| {
            workspaces.switch_window_to_team(window_id, team_b, ctx);
        });

        assert_eq!(
            state.read(&app, |state, _| state.selected_environment_id().copied()),
            Some(team_b_id)
        );
    });
}

#[test]
fn out_of_scope_persisted_environment_is_ignored_for_default() {
    let team_a = ServerId::from(101);
    let team_b = ServerId::from(202);
    let team_a_id = SyncId::ServerId(ServerId::from(2));
    let team_b_id = SyncId::ServerId(ServerId::from(3));
    App::test((), |mut app| async move {
        initialize_app(
            &mut app,
            vec![
                environment(team_a_id, "Team A", Owner::Team { team_uid: team_a }),
                environment(team_b_id, "Team B", Owner::Team { team_uid: team_b }),
            ],
        );
        CloudEnvironmentCatalog::handle(&app).update(&mut app, |catalog, ctx| {
            catalog.persist_selection(team_b_id, ctx);
        });
        let (window_id, _, state) = add_selector(&mut app);
        UserWorkspaces::handle(&app).update(&mut app, |workspaces, ctx| {
            workspaces.set_team_for_window(window_id, team_a, ctx);
        });

        assert_eq!(
            state.read(&app, |state, _| state.selected_environment_id().copied()),
            Some(team_a_id)
        );
    });
}
