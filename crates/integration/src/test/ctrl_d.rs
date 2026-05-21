//! Integration tests for CTRL-D / EOT behaviour.

use warp::{
    integration_testing::{
        step::new_step_with_default_assertions,
        terminal::{
            assert_active_block_command_for_single_terminal_in_tab, assert_bootstrapping_stage,
            assert_no_block_executing, assert_terminal_bootstrapped,
            execute_python_interpreter_in_tab, util::current_shell_starter_and_version,
            wait_until_bootstrapped_single_pane_for_tab, PYTHON_PROMPT_READY,
        },
        view_getters::assert_no_views_of_type,
    },
    pane_group::PaneGroup,
    terminal::{model::bootstrap::BootstrapStage, shell::ShellType, TerminalView},
    workspace::Workspace,
};
use warpui::integration::TestStep;

use super::{new_builder, Builder};
use crate::util::write_all_rc_files_for_test;

/// Verifies that ctrl-d correctly sends EOT to long-running commands.
pub fn test_ctrl_d_eot() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(execute_python_interpreter_in_tab(0))
        .with_step(
            new_step_with_default_assertions("Check ctrl-d terminates the command")
                .with_keystrokes(&["ctrl-d"])
                .add_assertion(assert_no_block_executing(0, 0)),
        )
}

// TODO(zheng) Add ctrl-d to exit the window too, after stopping this from beachballing.
pub fn test_ctrl_d_exit() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            TestStep::new(
                "ctrl-d should exit the shell and the corresponding pane / tab should be closed",
            )
            .with_keystrokes(&["ctrl-d"])
            .add_assertion(assert_no_views_of_type::<TerminalView>())
            .add_assertion(assert_no_views_of_type::<PaneGroup>())
            .add_assertion(assert_no_views_of_type::<Workspace>()),
        )
}

/// Tests that CTRL-D will complete a blocking read during bootstrapping
/// (e.g. omz update) by writing EOT to the PTY (which will result in an EOF condition).
pub fn test_ctrl_d_handled_by_read_during_bootstrapping() -> Builder {
    new_builder()
        .set_should_run_test(|| {
            let (starter, _) = current_shell_starter_and_version();
            // We need to fix https://github.com/warpdotdev/Warp/issues/1869 before
            // this test works on fish.
            !matches!(starter.shell_type(), ShellType::Fish)
        })
        .with_setup(|utils| {
            let dir = utils.test_dir();
            write_all_rc_files_for_test(dir, "python3");
        })
        .with_step(
            TestStep::new("Make sure shell is still bootstrapping")
                .add_assertion(assert_bootstrapping_stage(0, 0, BootstrapStage::ScriptExecution))
                // When the client's RC files produce output, they appear in the command grid of the
                // active block.
                .add_assertion(assert_active_block_command_for_single_terminal_in_tab(
                    &*PYTHON_PROMPT_READY,
                    0,
                ))
        )
        .with_step(
            TestStep::new("ctrl-d should write EOT to PTY which should signal EOF to python and bootstrapping should finish")
                .with_keystrokes(&["ctrl-d"])
                .add_assertion(assert_terminal_bootstrapped(0, 0)),
        )
}

/// Tests that entering CTRL-D while the PTY is bootstrapping
/// will exit the shell once bootstrapping is done
/// (assuming nothing else consumed EOT during bootstrapping).
pub fn test_ctrl_d_during_bootstrapping_exits_shell_upon_completion() -> Builder {
    let test = new_builder()
        .with_setup(|utils| {
            let dir = utils.test_dir();
            write_all_rc_files_for_test(dir, "sleep 2");
        })
        .with_step(
            TestStep::new("Make sure shell is still bootstrapping").add_assertion(
                assert_bootstrapping_stage(0, 0, BootstrapStage::ScriptExecution),
            ),
        );

    let final_test_step =
        TestStep::new("ctrl-d should write EOT to PTY which should exit the shell process")
            .with_keystrokes(&["ctrl-d"]);

    // On MacOS, the common behaviour for writing ctrl-d while bootstrapping
    // is to exit upon completion.
    // However, this differs on Linux where the common behaviour is to
    // ignore ctrl-d (and the corresponding EOF) while bootstrapping.
    let final_assertion = if cfg!(any(target_os = "linux", target_os = "freebsd")) {
        assert_terminal_bootstrapped(0, 0)
    } else {
        // TODO: figure out what the right behaviour is on windows.
        assert_no_views_of_type::<TerminalView>()
    };

    test.with_step(final_test_step.add_assertion(final_assertion))
}
