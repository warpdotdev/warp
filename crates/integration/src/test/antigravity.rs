use warp::integration_testing::step::{new_step_with_default_assertions, TestStep};
use warp::integration_testing::terminal::util::ExpectedExitStatus;
use warp::integration_testing::terminal::{
    clear_blocklist_to_remove_bootstrapped_blocks, execute_command_for_single_terminal_in_tab,
    wait_until_bootstrapped_single_pane_for_tab,
};

use crate::Builder;

pub fn test_antigravity_agent_ui() -> Builder {
    Builder::new()
        .with_real_display()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(clear_blocklist_to_remove_bootstrapped_blocks())
        .with_step(TestStep::new("Start recording").with_start_recording())
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "agy".to_string(),
            ExpectedExitStatus::Failure, // the command likely fails because 'agy' doesn't exist, but it still triggers the agent mode!
            "".to_string(),
        ))
        // Take screenshot of the CLI footer
        .with_step(
            new_step_with_default_assertions(
                "Take screenshot of the CLI footer appearing with agy",
            )
            .with_take_screenshot("agy_footer.png"),
        )
        .with_step(TestStep::new("Stop recording").with_stop_recording())
}
