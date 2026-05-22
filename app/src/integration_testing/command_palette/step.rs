use warpui::integration::{AssertionOutcome, TestStep};
use warpui::{App, WindowId};

use crate::integration_testing::command_palette::assertions::{
    assert_command_palette_has_results, assert_command_palette_is_closed,
    assert_command_palette_is_open,
};
use crate::util::bindings::cmd_or_ctrl_shift;

/// Extension trait for `Vec<TestStep>` that allows chaining assertions onto the last step.
pub trait TestStepsExt {
    fn add_assertion<F>(self, callback: F) -> Self
    where
        F: FnMut(&mut App, WindowId) -> AssertionOutcome + 'static;

    fn add_named_assertion<N, F>(self, name: N, callback: F) -> Self
    where
        N: Into<String>,
        F: FnMut(&mut App, WindowId) -> AssertionOutcome + 'static;
}

impl TestStepsExt for Vec<TestStep> {
    fn add_assertion<F>(mut self, callback: F) -> Self
    where
        F: FnMut(&mut App, WindowId) -> AssertionOutcome + 'static,
    {
        let last = self.pop().expect("steps should not be empty");
        self.push(last.add_assertion(callback));
        self
    }

    fn add_named_assertion<N, F>(mut self, name: N, callback: F) -> Self
    where
        N: Into<String>,
        F: FnMut(&mut App, WindowId) -> AssertionOutcome + 'static,
    {
        let last = self.pop().expect("steps should not be empty");
        self.push(last.add_named_assertion(name, callback));
        self
    }
}

pub fn open_command_palette() -> TestStep {
    TestStep::new("Open Command Palette")
        .with_keystrokes(&[cmd_or_ctrl_shift("p")])
        .add_assertion(assert_command_palette_is_open())
}

/// Test steps to run an `action` within the command palette.
///
/// Returns two steps: the first opens the palette and types the action text, waiting for
/// search results to appear (needed because async data sources like file search may delay
/// result delivery). The second presses Enter to execute the selected action.
pub fn open_command_palette_and_run_action(action: &str) -> Vec<TestStep> {
    vec![
        TestStep::new(format!("Type {action} in command palette").as_str())
            .with_keystrokes(&[cmd_or_ctrl_shift("p")])
            .with_typed_characters(&[action])
            .add_assertion(assert_command_palette_has_results()),
        TestStep::new(format!("Run {action} in command palette").as_str())
            .with_keystrokes(&["enter"]),
    ]
}

/// Test step to close the command palette.
pub fn close_command_palette() -> TestStep {
    TestStep::new("Close command Palette")
        .with_keystrokes(&["escape"])
        .add_assertion(assert_command_palette_is_closed())
}
