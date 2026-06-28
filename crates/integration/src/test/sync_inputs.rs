use warp::cmd_or_ctrl_shift;
use warp::integration_testing::step::new_step_with_default_assertions;
use warp::integration_testing::terminal::util::ExpectedExitStatus;
use warp::integration_testing::terminal::{
    assert_active_block_output, assert_command_executed, assert_long_running_block_executing,
    assert_no_block_executing, execute_command, run_alt_grid_program, wait_until_bootstrapped_pane,
    wait_until_bootstrapped_single_pane_for_tab,
};
use warp::integration_testing::view_getters::{terminal_view, workspace_view};
use warp::workspace::WorkspaceAction;
use warpui_core::integration::TestStep;
use warpui_core::{async_assert, async_assert_eq};

use super::{new_builder, Builder};
use crate::util::{get_input_buffer, skip_if_powershell_core_2303};

pub fn test_input_syncing_is_off_by_default() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("create one additional pane")
                .with_keystrokes(&[cmd_or_ctrl_shift("d")]),
        )
        .with_step(wait_until_bootstrapped_pane(0, 1))
        .with_step(
            new_step_with_default_assertions(
                "type something into pane 2 and check both pane contents",
            )
            .with_keystrokes(&["b"])
            .add_named_assertion("Check that pane 1 is still empty", |app, window_id| {
                let input1 = get_input_buffer(app, window_id, 0, 0);

                async_assert!(
                    input1.is_empty(),
                    "pane 1 should be empty but it contains {}",
                    input1
                )
            })
            .add_named_assertion(
                "Check that pane 2 has the correct contents",
                |app, window_id| {
                    let input2 = get_input_buffer(app, window_id, 0, 1);

                    async_assert!(
                        input2 == "b",
                        "pane 2 should contain 'b' but it contains {}",
                        input2
                    )
                },
            ),
        )
}

pub fn test_can_sync_input_editor_text_in_tab() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("create one additional pane")
                .with_keystrokes(&[cmd_or_ctrl_shift("d")]),
        )
        .with_step(wait_until_bootstrapped_pane(0, 1))
        .with_step(
            new_step_with_default_assertions("turn syncing on in tab").with_action(
                move |app, _, _| {
                    let window_id =
                        app.read(|ctx| ctx.windows().active_window().expect("no active window"));
                    let workspace_view_id = workspace_view(app, window_id).id();

                    app.dispatch_typed_action(
                        window_id,
                        &[workspace_view_id],
                        &WorkspaceAction::ToggleSyncTerminalInputsInTab,
                    );
                },
            ),
        )
        .with_step(
            new_step_with_default_assertions(
                "type something into pane 2 and check pane 1 and 2 contents",
            )
            .with_keystrokes(&["b"])
            .add_named_assertion(
                "check that pane 1 and 2 have the same contents",
                |app, window_id| {
                    let input1 = get_input_buffer(app, window_id, 0, 0);
                    let input2 = get_input_buffer(app, window_id, 0, 1);

                    async_assert!(
                        input1 == "b" && input2 == "b",
                        "Both panes should contain 'b', pane 1: '{input1}', pane 2: '{input2}'"
                    )
                },
            ),
        )
}

pub fn test_can_run_command_in_synced_panes_in_tab() -> Builder {
    let command = "echo typedInPane2";
    let expected_output = "typedInPane2";

    new_builder()
        // TODO(CORE-2732): Flakey on Powershell
        .set_should_run_test(skip_if_powershell_core_2303)
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("create one additional pane")
                .with_keystrokes(&[cmd_or_ctrl_shift("d")]),
        )
        .with_step(wait_until_bootstrapped_pane(0, 1))
        .with_step(
            new_step_with_default_assertions("turn syncing on in tab").with_action(
                move |app, _, _| {
                    let window_id =
                        app.read(|ctx| ctx.windows().active_window().expect("no active window"));
                    let workspace_view_id = workspace_view(app, window_id).id();

                    app.dispatch_typed_action(
                        window_id,
                        &[workspace_view_id],
                        &WorkspaceAction::ToggleSyncTerminalInputsInTab,
                    );
                },
            ),
        )
        .with_step(
            execute_command(
                0,
                0,
                command.to_owned(),
                ExpectedExitStatus::Success,
                expected_output,
            )
            .add_named_assertion(
                "assert that the same command ran in pane 0",
                assert_command_executed(0, 1, command.to_owned()),
            ),
        )
}

pub fn test_synced_panes_long_running_commands() -> Builder {
    new_builder()
        // TODO(CORE-2732): Flakey on Powershell
        .set_should_run_test(skip_if_powershell_core_2303)
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("create one additional pane")
                .with_keystrokes(&[cmd_or_ctrl_shift("d")]),
        )
        .with_step(wait_until_bootstrapped_pane(0, 1))
        .with_step(
            new_step_with_default_assertions("turn syncing on in tab").with_action(
                move |app, _, _| {
                    let window_id =
                        app.read(|ctx| ctx.windows().active_window().expect("no active window"));
                    let workspace_view_id = workspace_view(app, window_id).id();

                    app.dispatch_typed_action(
                        window_id,
                        &[workspace_view_id],
                        &WorkspaceAction::ToggleSyncTerminalInputsInTab,
                    );
                },
            ),
        )
        .with_step(
            TestStep::new("Execute sleep 1000000 in both panes")
                .with_typed_characters(&["sleep 1000000"])
                .with_keystrokes(&["enter"])
                .add_named_assertion(
                    "check that sleep 1000000 ran in pane 0",
                    assert_long_running_block_executing(true, 0, 0),
                )
                .add_named_assertion(
                    "check that sleep 1000000 ran in pane 1",
                    assert_long_running_block_executing(true, 0, 1),
                ),
        )
        .with_step(
            TestStep::new("Send text to both panes")
                .with_typed_characters(&["foo"])
                .add_named_assertion(
                    "check that foo was sent to pane 0",
                    assert_active_block_output("foo", 0, 0),
                )
                .add_named_assertion(
                    "check that foo was sent to pane 1",
                    assert_active_block_output("foo", 0, 1),
                ),
        )
        .with_step(
            TestStep::new("Exit sleep 1000000 in both panes")
                .with_keystrokes(&["ctrl-c"])
                .add_named_assertion(
                    "check that no command is running in pane 0",
                    assert_no_block_executing(0, 0),
                )
                .add_named_assertion(
                    "check that no command is running in pane 1",
                    assert_no_block_executing(0, 1),
                ),
        )
}

/// Tests that as you use synced inputs and terminals switch between
/// alt-screens and the block-list, the correct terminal view maintains focus.
pub fn test_synced_inputs_terminal_mode_change_view_focus() -> Builder {
    let mut builder = new_builder()
        // TODO(CORE-2732): Flakey on Powershell
        .set_should_run_test(skip_if_powershell_core_2303)
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0));

    for i in 1..=3 {
        builder = builder
            .with_step(
                new_step_with_default_assertions(format!("create pane {i} in tab 0").as_str())
                    .with_keystrokes(&[cmd_or_ctrl_shift("d")]),
            )
            .with_step(wait_until_bootstrapped_pane(0, i));
    }

    builder = builder.with_step(
        new_step_with_default_assertions("create 2nd tab")
            .with_keystrokes(&[cmd_or_ctrl_shift("t")]),
    );

    for i in 1..=3 {
        builder = builder
            .with_step(
                new_step_with_default_assertions(format!("create pane {i} in tab 1").as_str())
                    .with_keystrokes(&[cmd_or_ctrl_shift("d")]),
            )
            .with_step(wait_until_bootstrapped_pane(1, i));
    }

    builder = builder.with_step(
        new_step_with_default_assertions("turn syncing on across all tabs").with_action(
            move |app, _, _| {
                let window_id =
                    app.read(|ctx| ctx.windows().active_window().expect("no active window"));
                let workspace_view_id = workspace_view(app, window_id).id();

                app.dispatch_typed_action(
                    window_id,
                    &[workspace_view_id],
                    &WorkspaceAction::ToggleSyncAllTerminalInputsInAllTabs,
                );
            },
        ),
    );

    let exit_vim_step = TestStep::new("Close vim")
        .with_keystrokes(&["escape"])
        .with_typed_characters(&[":q!"])
        .with_keystrokes(&["enter"]);

    let vim_steps = run_alt_grid_program(
        "vim",
        1,
        3,
        exit_vim_step,
        vec![
            TestStep::new("While vim is running, check focused terminal").add_named_assertion(
                "tab 1 terminal 3 is focused",
                |app, window_id| {
                    let terminal_view_id = terminal_view(app, window_id, 1, 3).id();
                    app.update(|app_ctx| {
                        async_assert_eq!(
                            app_ctx.check_view_or_child_focused(window_id, &terminal_view_id),
                            true
                        )
                    })
                },
            ),
        ],
    );

    builder = builder.with_steps(vim_steps);

    builder = builder.with_step(
        TestStep::new("check focused terminal after exiting vim").add_named_assertion(
            "tab 1 terminal 3 is focused",
            |app, window_id| {
                let terminal_view_id = terminal_view(app, window_id, 1, 3).id();
                app.update(|app_ctx| {
                    async_assert_eq!(
                        app_ctx.check_view_or_child_focused(window_id, &terminal_view_id),
                        true
                    )
                })
            },
        ),
    );

    builder
}
