use std::time::Duration;

use settings::Setting as _;
use warp::features::FeatureFlag;
use warp::integration_testing::input::{
    active_cli_agent_is, cli_agent_rich_input_is_open, open_cli_agent_rich_input_for_agent,
    rich_input_placeholder_exists,
};
use warp::integration_testing::step::new_step_with_default_assertions;
use warp::integration_testing::terminal::{
    clear_blocklist_to_remove_bootstrapped_blocks, wait_until_bootstrapped_single_pane_for_tab,
};
use warp::integration_testing::view_getters::workspace_view;
use warp::terminal::CLIAgent;
use warp::workspace::WorkspaceAction;
use warp::workspace::tab_settings::{TabSettings, VerticalTabsDisplayGranularity};
use warpui_core::integration::TestStep;
use warpui_core::{SingletonEntity, TypedActionView};

use super::new_builder;
use crate::Builder;

pub fn test_kiro_cli_rich_input_shows_kiro_branding() -> Builder {
    FeatureFlag::CLIAgentRichInput.set_enabled(true);
    FeatureFlag::VerticalTabs.set_enabled(true);

    new_builder()
        .with_real_display()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(clear_blocklist_to_remove_bootstrapped_blocks())
        .with_step(open_cli_agent_rich_input_for_agent(0, CLIAgent::Kiro))
        .with_step(
            TestStep::new("Assert Kiro Rich Input state and capture screenshot")
                .set_timeout(Duration::from_secs(20))
                .set_post_step_pause(Duration::from_secs(2))
                .with_take_screenshot("kiro_cli_rich_input.png")
                .add_assertion(cli_agent_rich_input_is_open(0))
                .add_assertion(active_cli_agent_is(0, CLIAgent::Kiro))
                .add_assertion(rich_input_placeholder_exists(0)),
        )
        .with_step(
            new_step_with_default_assertions("Enable and open vertical tabs").with_action(
                |app, window_id, _| {
                    TabSettings::handle(app).update(app, |settings, ctx| {
                        settings
                            .use_vertical_tabs
                            .set_value(true, ctx)
                            .expect("vertical tabs setting should update");
                        settings
                            .vertical_tabs_display_granularity
                            .set_value(VerticalTabsDisplayGranularity::Panes, ctx)
                            .expect("vertical tabs display granularity should update");
                    });

                    let workspace = workspace_view(app, window_id);
                    workspace.update(app, |workspace, ctx| {
                        workspace.handle_action(&WorkspaceAction::OpenVerticalTabsPanel, ctx);
                    });
                },
            ),
        )
        .with_step(
            TestStep::new("Assert Kiro vertical tab branding and capture screenshot")
                .set_timeout(Duration::from_secs(20))
                .set_post_step_pause(Duration::from_secs(2))
                .with_take_screenshot("kiro_cli_vertical_tab.png")
                .add_assertion(|app, window_id| {
                    let presenter = app.presenter(window_id).expect("presenter should exist");
                    let panel_is_rendered = presenter
                        .borrow()
                        .position_cache()
                        .get_position("workspace_view:vertical_tabs_panel")
                        .is_some();
                    warpui_core::async_assert!(
                        panel_is_rendered,
                        "Expected vertical tabs panel to be rendered"
                    )
                })
                .add_assertion(cli_agent_rich_input_is_open(0))
                .add_assertion(active_cli_agent_is(0, CLIAgent::Kiro)),
        )
}
