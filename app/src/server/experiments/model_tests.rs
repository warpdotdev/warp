use onboarding::ChooseHowToStartExperimentArm;
use warpui::{App, Entity, SingletonEntity};

use super::{ServerExperiment, ServerExperiments};
use crate::{GlobalResourceHandles, GlobalResourceHandlesProvider};

/// A model for testing purposes only.
///
/// We use it to demonstrate how client-side
/// models can be mutated to reflect server
/// experiment state changes.
pub struct TestModel(pub usize);

impl Entity for TestModel {
    type Event = ();
}
impl SingletonEntity for TestModel {}

fn initialize_app(app: &mut App) {
    app.update(crate::settings::init_and_register_user_preferences);

    let global_resources = GlobalResourceHandles::mock(app);
    app.add_singleton_model(|_| GlobalResourceHandlesProvider::new(global_resources));
}

#[test]
fn test_new_from_cached() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let model = app.add_singleton_model(|_| TestModel(0));
        let cache = vec![ServerExperiment::TestExperiment];
        app.add_singleton_model(|ctx| ServerExperiments::new_from_cache(cache, ctx));

        // The experiment should have been enabled.
        model.read(&app, |model, _| {
            assert_eq!(model.0, 1);
        });
    });
}

#[test]
fn test_apply_latest_state() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let model = app.add_singleton_model(|_| TestModel(0));
        let experiments =
            app.add_singleton_model(|ctx| ServerExperiments::new_from_cache(vec![], ctx));

        // Enable the experiment.
        experiments.update(&mut app, |experiments, ctx| {
            experiments.apply_latest_state(vec![ServerExperiment::TestExperiment], ctx);
        });
        model.read(&app, |model, _| {
            assert_eq!(model.0, 1);
        });

        // Redundant experiment state should be a no-op.
        experiments.update(&mut app, |experiments, ctx| {
            experiments.apply_latest_state(vec![ServerExperiment::TestExperiment], ctx);
        });
        model.read(&app, |model, _| {
            assert_eq!(model.0, 1);
        });
    });
}

/// REV-1939: the "choose how to start" arm resolves control/experiment only
/// when exactly that arm is enabled; neither and both fail closed to
/// unassigned so the safe two-option layout is shown.
#[test]
fn test_choose_how_to_start_experiment_arm_resolution() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let experiments =
            app.add_singleton_model(|ctx| ServerExperiments::new_from_cache(vec![], ctx));

        let arm_for = |app: &mut App, arms: Vec<ServerExperiment>| {
            experiments.update(app, |experiments, ctx| {
                experiments.apply_latest_state(arms, ctx);
            });
            experiments.read(app, |experiments, _| {
                experiments.choose_how_to_start_experiment_arm()
            })
        };

        assert_eq!(
            arm_for(&mut app, vec![]),
            ChooseHowToStartExperimentArm::Unassigned
        );
        assert_eq!(
            arm_for(
                &mut app,
                vec![ServerExperiment::OnboardingChooseHowToStartControl]
            ),
            ChooseHowToStartExperimentArm::Control
        );
        assert_eq!(
            arm_for(
                &mut app,
                vec![ServerExperiment::OnboardingChooseHowToStartExperiment]
            ),
            ChooseHowToStartExperimentArm::Experiment
        );
        // Both arms present is malformed state: fail closed to unassigned.
        assert_eq!(
            arm_for(
                &mut app,
                vec![
                    ServerExperiment::OnboardingChooseHowToStartControl,
                    ServerExperiment::OnboardingChooseHowToStartExperiment,
                ]
            ),
            ChooseHowToStartExperimentArm::Unassigned
        );
    });
}
