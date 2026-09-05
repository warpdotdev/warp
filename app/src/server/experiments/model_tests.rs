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
