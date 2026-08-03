use warpui::App;

use super::runner_controls_enabled;
use crate::features::FeatureFlag;
use crate::server::experiments::{ServerExperiment, ServerExperiments};
use crate::{GlobalResourceHandles, GlobalResourceHandlesProvider};

fn initialize_app(app: &mut App) {
    app.update(crate::settings::init_and_register_user_preferences);

    let global_resources = GlobalResourceHandles::mock(app);
    app.add_singleton_model(|_| GlobalResourceHandlesProvider::new(global_resources));
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
