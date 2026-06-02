use std::time::Duration;

use warp::features::FeatureFlag;
use warp::integration_testing::clipboard::write_to_clipboard;
use warp::integration_testing::input::{
    assert_autosuggestion_state, input_contains_string, input_is_empty,
    latest_buffer_operations_are_empty, tab_completions_menu_is_open, AutosuggestionState,
};
use warp::integration_testing::step::new_step_with_default_assertions;
use warp::integration_testing::terminal::util::{
    current_shell_starter_and_version, ExpectedExitStatus,
};
use warp::integration_testing::terminal::{
    execute_command_for_single_terminal_in_tab, wait_until_bootstrapped_single_pane_for_tab,
};
use warp::integration_testing::view_getters::{
    single_input_view_for_tab, single_terminal_view_for_tab,
};
use warp::terminal::shell::ShellType;
use warpui_core::integration::TestStep;
use warpui_core::{async_assert_eq, Event};

use super::new_builder;
use crate::Builder;

/// Ensures that tab completions are hidden when the completions menu is opened
/// but re-appear when the menu is closed.
pub fn test_autosuggestions_are_hidden_when_opening_tab_completions() -> Builder {
    FeatureFlag::RemoveAutosuggestionDuringTabCompletions.set_enabled(true);

    new_builder()
        // Ensure that $HOME contains a directory as a tab-completion candidate.
        .with_setup(|utils| {
            let dir = utils.test_dir();
            std::fs::create_dir(dir.join("foo")).expect("must be able to create dirs for test");
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        // Execute a command so that we can generate autosuggestions.
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "cd .".into(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(
            new_step_with_default_assertions("Insert 'cd' into input")
                .with_typed_characters(&["cd "])
                .add_named_assertion(
                    "Ensure cd is in input",
                    input_contains_string(0, String::from("cd ")),
                )
                .add_named_assertion(
                    "Ensure autosuggestion is present",
                    assert_autosuggestion_state(
                        0,
                        AutosuggestionState::ActiveWithText(String::from(".")),
                    ),
                ),
        )
        .with_step(
            new_step_with_default_assertions("Open tab completions menu")
                .with_keystrokes(&["tab"])
                .add_named_assertion(
                    "Ensure tab completions menu is open",
                    tab_completions_menu_is_open(0, true),
                )
                .add_named_assertion(
                    "Ensure autosuggestion is closed",
                    assert_autosuggestion_state(0, AutosuggestionState::Closed),
                ),
        )
        .with_step(
            new_step_with_default_assertions("Close tab completions menu")
                .with_keystrokes(&["escape"])
                .add_named_assertion(
                    "Ensure tab completions menu is closed",
                    tab_completions_menu_is_open(0, false),
                )
                .add_named_assertion(
                    "Ensure autosuggestion is closed",
                    assert_autosuggestion_state(
                        0,
                        AutosuggestionState::ActiveWithText(String::from(".")),
                    ),
                ),
        )
}

pub fn test_latest_buffer_operations() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        // Execute a command so that we can generate autosuggestions.
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "cd .".into(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(
            new_step_with_default_assertions("Check initial state").add_named_assertion(
                "Ensure the latest buffer operations start off empty",
                latest_buffer_operations_are_empty(0, true),
            ),
        )
        .with_step(
            new_step_with_default_assertions("Write into the input")
                .with_typed_characters(&["echo 'foo'"])
                .add_named_assertion(
                    "Ensure the input was written to",
                    input_contains_string(0, String::from("echo 'foo'")),
                )
                .add_named_assertion(
                    "Ensure the latest buffer operations are non-empty",
                    latest_buffer_operations_are_empty(0, false),
                ),
        )
        .with_step(
            new_step_with_default_assertions("Run the command with the current buffer text")
                .with_keystrokes(&["enter"])
                .add_named_assertion("Ensure the input is empty", input_is_empty(0))
                .add_named_assertion(
                    "Ensure the latest buffer operations are empty",
                    latest_buffer_operations_are_empty(0, true),
                ),
        )
}

pub fn test_middle_click_paste() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(write_to_clipboard(String::from("abc")).add_named_assertion(
            "Ensure the input is empty to start",
            input_contains_string(0, String::from("")),
        ))
        .with_step(
            TestStep::new("Middle click in the input editor")
                .with_event_fn(|app, window_id| {
                    let input_view = single_input_view_for_tab(app, window_id, 0);
                    input_view.update(app, |view, ctx| {
                        let mut position = ctx
                            .element_position_by_id(view.prompt_save_position_id())
                            .expect("prompt should have a position")
                            .origin();
                        // Move the position slightly so it's clearly over the editor.
                        position.set_x(position.x() + 10.);
                        position.set_y(position.y() + 5.);
                        Event::MiddleMouseDown {
                            position,
                            cmd: false,
                            shift: false,
                            click_count: 1,
                        }
                    })
                })
                .add_named_assertion(
                    "Ensure the text is pasted once",
                    input_contains_string(0, String::from("abc")),
                ),
        )
        .with_step(
            TestStep::new("Middle click on the prompt area")
                .with_event_fn(|app, window_id| {
                    let input_view = single_input_view_for_tab(app, window_id, 0);
                    input_view.update(app, |view, ctx| {
                        let mut position = ctx
                            .element_position_by_id(view.prompt_save_position_id())
                            .expect("prompt should have a position")
                            .origin();
                        // Move the position slightly so it's clearly over the prompt.
                        position.set_x(position.x() + 10.);
                        position.set_y(position.y() + 5.);
                        Event::MiddleMouseDown {
                            position,
                            cmd: false,
                            shift: false,
                            click_count: 1,
                        }
                    })
                })
                .add_named_assertion(
                    "Ensure the text is pasted again",
                    input_contains_string(0, String::from("abcabc")),
                ),
        )
}

/// Checks that the git branch prompt chip value is correctly populated.
pub fn test_git_prompt_chips() -> Builder {
    // Note that we can't use the OUT_DIR for the temp directory
    // here because that would put us in the warp repo. We need to
    // be in a place in the filesystem that's not already a git repo.
    new_builder()
        .set_should_run_test(|| {
            // TODO(alokedesai): Re-enable for Powershell once the cause of the flakiness has been
            // resolved.
            let (starter, _) = current_shell_starter_and_version();
            starter.shell_type() != ShellType::PowerShell
        })
        .use_tmp_filesystem_for_test_root_directory()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "git init -b main; git config user.email \"test@test.com\"; git config user.name \"Git TestUser\"".into(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "touch file".into(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(
            new_step_with_default_assertions("Git branch chip should be populated").set_timeout(Duration::from_secs(15)).add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |terminal_view, ctx| {
                        terminal_view.input().read(ctx, |input_view, ctx| {
                            let git_branch = input_view.prompt_render_helper.git_branch(ctx);
                            async_assert_eq!(git_branch, Some("main".to_string()))
                        })
                    })
            }),
        )
}
