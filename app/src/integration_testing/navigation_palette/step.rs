use warpui::integration::TestStep;
use warpui::{ViewHandle, async_assert};

use crate::integration_testing::command_palette::assert_command_palette_is_open;
use crate::integration_testing::navigation_palette::assert_navigation_mode_enabled_in_command_palette;
use crate::integration_testing::step::new_step_with_default_assertions;
use crate::util::bindings::cmd_or_ctrl_shift;
use crate::workspace::Workspace;

pub fn open_navigation_palette_step() -> TestStep {
    new_step_with_default_assertions("Open Navigation Palette")
        .with_keystrokes(&[cmd_or_ctrl_shift("p")])
        .with_typed_characters(&["s"])
        .with_keystrokes(&["tab"])
        .add_assertion(assert_command_palette_is_open())
        .add_assertion(assert_navigation_mode_enabled_in_command_palette())
}

pub fn navigate_to_other_session_step() -> TestStep {
    new_step_with_default_assertions("Navigate to original tab using Navigation Palette.")
        .with_keystrokes(&["down", "enter"])
        .add_assertion(move |app, window_id| {
            let views: Vec<ViewHandle<Workspace>> =
                app.views_of_type(window_id).expect("No workspace found");
            let workspace = views.first().expect("No workspace in views");
            workspace.read(app, |view, _| {
                async_assert!(
                    !view.is_palette_open(),
                    "Palette should be closed after hitting enter"
                )
            })
        })
}
