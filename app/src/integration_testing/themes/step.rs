use warpui::async_assert;
use warpui::integration::TestStep;

use crate::integration_testing::command_palette::open_command_palette_and_run_action;
use crate::integration_testing::view_getters::workspace_view;

pub fn open_theme_picker() -> Vec<TestStep> {
    let mut steps = open_command_palette_and_run_action("Open Theme Picker");
    let last = steps.pop().expect("steps should not be empty");
    steps.push(last.add_assertion(|app, window_id| {
        let workspace = workspace_view(app, window_id);
        workspace.read(app, |workspace, _| {
            async_assert!(
                workspace.is_theme_chooser_open(),
                "Theme chooser should be open"
            )
        })
    }));
    steps
}
