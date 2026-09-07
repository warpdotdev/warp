use ai::agent::action::RunAgentsExecutionMode;
use warpui::elements::Empty;
use warpui::platform::WindowStyle;
use warpui::{
    App, AppContext, Element as _, Entity, SingletonEntity as _, TypedActionView, View,
    ViewContext, ViewHandle,
};

use super::{
    OrchestrationConfigState, OrchestrationPickerHandles, apply_execution_mode_change,
    resolve_default_environment_id, runner_controls_enabled,
};
use crate::ai::blocklist::inline_action::run_agents_card_view::RunAgentsCardViewAction;
use crate::ai::cloud_environments::{
    AmbientAgentEnvironment, CloudAmbientAgentEnvironment, CloudAmbientAgentEnvironmentModel,
    CloudEnvironmentCatalog,
};
use crate::cloud_object::model::persistence::CloudModel;
use crate::cloud_object::{CloudObjectMetadata, CloudObjectPermissions, Owner};
use crate::features::FeatureFlag;
use crate::server::experiments::{ServerExperiment, ServerExperiments};
use crate::server::ids::{ServerId, SyncId};
use crate::test_util::settings::initialize_settings_for_tests;
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::{GlobalResourceHandles, GlobalResourceHandlesProvider};

fn initialize_app(app: &mut App) {
    initialize_settings_for_tests(app);

    let global_resources = GlobalResourceHandles::mock(app);
    app.add_singleton_model(|_| GlobalResourceHandlesProvider::new(global_resources));
    app.add_singleton_model(UserWorkspaces::default_mock);
}

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

fn install_environment_catalog(app: &mut App, environments: Vec<CloudAmbientAgentEnvironment>) {
    let cloud_model = app.add_singleton_model(CloudModel::mock);
    cloud_model.update(app, |model, ctx| {
        for environment in environments {
            model.create_object(environment.id, environment, ctx);
        }
    });
    app.add_singleton_model(CloudEnvironmentCatalog::new);
}

struct EnvironmentDefaultView;

impl Entity for EnvironmentDefaultView {
    type Event = ();
}

impl View for EnvironmentDefaultView {
    fn ui_name() -> &'static str {
        "EnvironmentDefaultView"
    }

    fn render(&self, _app: &AppContext) -> Box<dyn warpui::Element> {
        Empty::new().finish()
    }
}

impl TypedActionView for EnvironmentDefaultView {
    type Action = ();

    fn handle_action(&mut self, _action: &Self::Action, _ctx: &mut ViewContext<Self>) {}
}

fn add_environment_default_view(
    app: &mut App,
) -> (warpui::WindowId, ViewHandle<EnvironmentDefaultView>) {
    app.add_window(WindowStyle::NotStealFocus, |_ctx| EnvironmentDefaultView)
}

fn resolved_environment_id(
    app: &mut App,
    view: &ViewHandle<EnvironmentDefaultView>,
) -> Option<String> {
    view.update(app, |_view, ctx| resolve_default_environment_id(ctx))
}

#[test]
fn runner_controls_require_both_feature_flag_and_experiment_arm() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let experiments =
            app.add_singleton_model(|ctx| ServerExperiments::new_from_cache(vec![], ctx));

        {
            let _cloud_agent_runners = FeatureFlag::CloudAgentRunners.override_enabled(false);
            experiments.update(&mut app, |experiments, ctx| {
                experiments.apply_latest_state(vec![], ctx);
            });
            app.read(|ctx| assert!(!runner_controls_enabled(ctx)));
        }

        {
            let _cloud_agent_runners = FeatureFlag::CloudAgentRunners.override_enabled(false);
            experiments.update(&mut app, |experiments, ctx| {
                experiments.apply_latest_state(vec![ServerExperiment::MacosRunnersExperiment], ctx);
            });
            app.read(|ctx| assert!(!runner_controls_enabled(ctx)));
        }

        {
            let _cloud_agent_runners = FeatureFlag::CloudAgentRunners.override_enabled(true);
            experiments.update(&mut app, |experiments, ctx| {
                experiments.apply_latest_state(vec![], ctx);
            });
            app.read(|ctx| assert!(!runner_controls_enabled(ctx)));
        }

        {
            let _cloud_agent_runners = FeatureFlag::CloudAgentRunners.override_enabled(true);
            experiments.update(&mut app, |experiments, ctx| {
                experiments.apply_latest_state(vec![ServerExperiment::MacosRunnersExperiment], ctx);
            });
            app.read(|ctx| assert!(runner_controls_enabled(ctx)));
        }
    });
}

#[test]
fn local_to_cloud_uses_window_scoped_environment_default() {
    let team_a = ServerId::from(101);
    let team_b = ServerId::from(202);
    let team_a_id = SyncId::ServerId(ServerId::from(1));
    let team_b_id = SyncId::ServerId(ServerId::from(2));
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        install_environment_catalog(
            &mut app,
            vec![
                environment(team_a_id, "A Team A", Owner::Team { team_uid: team_a }),
                environment(team_b_id, "B Team B", Owner::Team { team_uid: team_b }),
            ],
        );
        CloudEnvironmentCatalog::handle(&app).update(&mut app, |catalog, ctx| {
            catalog.persist_selection(team_a_id, ctx);
        });
        let (window_id, view) = add_environment_default_view(&mut app);
        UserWorkspaces::handle(&app).update(&mut app, |workspaces, ctx| {
            workspaces.set_team_for_window(window_id, team_b, ctx);
        });
        let mut state = OrchestrationConfigState::from_run_agents_fields(
            None,
            Some("claude"),
            &RunAgentsExecutionMode::Local,
        );
        let handles = OrchestrationPickerHandles::<RunAgentsCardViewAction>::default();

        view.update(&mut app, |_view, ctx| {
            apply_execution_mode_change(&mut state, &handles, true, None, ctx);
        });

        let RunAgentsExecutionMode::Remote { environment_id, .. } = state.execution_mode else {
            panic!("expected Remote after mode change");
        };
        assert_eq!(environment_id, team_b_id.uid());
        assert_ne!(environment_id, team_a_id.uid());
    });
}

#[test]
fn environment_defaults_are_independent_per_window_and_include_personal() {
    let team_a = ServerId::from(101);
    let team_b = ServerId::from(202);
    let team_a_id = SyncId::ServerId(ServerId::from(1));
    let team_b_id = SyncId::ServerId(ServerId::from(2));
    let personal_id = SyncId::ServerId(ServerId::from(3));
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        install_environment_catalog(
            &mut app,
            vec![
                environment(team_a_id, "A Team A", Owner::Team { team_uid: team_a }),
                environment(team_b_id, "B Team B", Owner::Team { team_uid: team_b }),
                environment(personal_id, "Z Personal", Owner::mock_current_user()),
            ],
        );
        let (window_a, view_a) = add_environment_default_view(&mut app);
        let (window_b, view_b) = add_environment_default_view(&mut app);
        UserWorkspaces::handle(&app).update(&mut app, |workspaces, ctx| {
            workspaces.set_team_for_window(window_a, team_a, ctx);
            workspaces.set_team_for_window(window_b, team_b, ctx);
        });

        assert_eq!(
            resolved_environment_id(&mut app, &view_a),
            Some(team_a_id.uid())
        );
        assert_eq!(
            resolved_environment_id(&mut app, &view_b),
            Some(team_b_id.uid())
        );
        assert_eq!(
            app.read(|ctx| CloudEnvironmentCatalog::as_ref(ctx).environments().len()),
            3
        );

        CloudEnvironmentCatalog::handle(&app).update(&mut app, |catalog, ctx| {
            catalog.persist_selection(personal_id, ctx);
        });
        assert_eq!(
            resolved_environment_id(&mut app, &view_a),
            Some(personal_id.uid())
        );
        assert_eq!(
            resolved_environment_id(&mut app, &view_b),
            Some(personal_id.uid())
        );
    });
}

#[test]
fn environment_default_uses_live_team_and_ignores_invisible_persisted_selection() {
    let team_a = ServerId::from(101);
    let team_b = ServerId::from(202);
    let team_a_id = SyncId::ServerId(ServerId::from(1));
    let team_b_id = SyncId::ServerId(ServerId::from(2));
    let personal_id = SyncId::ServerId(ServerId::from(3));
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        install_environment_catalog(
            &mut app,
            vec![
                environment(team_a_id, "A Team A", Owner::Team { team_uid: team_a }),
                environment(team_b_id, "B Team B", Owner::Team { team_uid: team_b }),
                environment(personal_id, "Z Personal", Owner::mock_current_user()),
            ],
        );
        CloudEnvironmentCatalog::handle(&app).update(&mut app, |catalog, ctx| {
            catalog.persist_selection(team_a_id, ctx);
        });
        let (window_id, view) = add_environment_default_view(&mut app);
        UserWorkspaces::handle(&app).update(&mut app, |workspaces, ctx| {
            workspaces.set_team_for_window(window_id, team_a, ctx);
        });
        assert_eq!(
            resolved_environment_id(&mut app, &view),
            Some(team_a_id.uid())
        );

        UserWorkspaces::handle(&app).update(&mut app, |workspaces, ctx| {
            workspaces.switch_window_to_team(window_id, team_b, ctx);
        });
        assert_eq!(
            resolved_environment_id(&mut app, &view),
            Some(team_b_id.uid())
        );
    });
}
