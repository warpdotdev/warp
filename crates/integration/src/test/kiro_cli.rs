use std::time::Duration;

use warp::features::FeatureFlag;
use warp::integration_testing::input::{
    active_cli_agent_is, cli_agent_rich_input_is_open, open_cli_agent_rich_input_for_agent,
    rich_input_placeholder_exists,
};
use warp::integration_testing::terminal::{
    clear_blocklist_to_remove_bootstrapped_blocks, wait_until_bootstrapped_single_pane_for_tab,
};
use warp::terminal::CLIAgent;
use warpui_core::integration::TestStep;

use super::new_builder;
use crate::Builder;

pub fn test_kiro_cli_rich_input_shows_kiro_branding() -> Builder {
    FeatureFlag::CLIAgentRichInput.set_enabled(true);

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
}
