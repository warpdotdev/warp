use warp::integration_testing::step::new_step_with_default_assertions;
use warp::integration_testing::terminal::util::current_shell_starter_and_version;
use warp::integration_testing::terminal::{
    assert_active_block_output_for_single_terminal_in_tab, assert_input_editor_contents,
    assert_long_running_block_executing_for_single_terminal_in_tab,
    assert_no_visible_background_blocks, wait_until_bootstrapped_single_pane_for_tab,
};
use warp::integration_testing::view_getters::single_terminal_view_for_tab;
use warp::terminal::model::block::TranscriptScope;
use warp::terminal::model::terminal_model::BlockIndex;
use warp::terminal::shell::{Shell, ShellType};
use warpui_core::integration::{AssertionCallback, AssertionOutcome, TestStep};
use warpui_core::{async_assert, async_assert_eq};

use super::{new_builder, Builder};
use crate::util::skip_if_powershell_core_2303;

pub fn test_typeahead() -> Builder {
    new_builder()
        // TODO(CORE-2732): Flakey on Powershell (Linux)
        .set_should_run_test(skip_if_powershell_core_2303)
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            TestStep::new("Execute sleep 4")
                .with_typed_characters(&["sleep 4"])
                .with_keystrokes(&["enter"])
                .add_assertion(
                    assert_long_running_block_executing_for_single_terminal_in_tab(true, 0),
                ),
        )
        .with_step(
            TestStep::new("Enter text to long running command")
                .with_input_string("foo", None)
                .add_assertion(require_long_running_block_executing(0))
                .add_assertion(assert_active_block_output_for_single_terminal_in_tab(
                    "foo", 0,
                )),
        )
        .with_step(
            new_step_with_default_assertions("Input box should have typeahead text")
                .add_assertion(assert_input_editor_contents(0, "foo"))
                .add_named_assertion(
                    "No typeahead duplicated in background block",
                    assert_no_visible_background_blocks(0, 0),
                ),
        )
}

/// Checks a typeahead command has the expected value.
///
/// There's a race condition sending the ESC-i keybinding to
/// report input. In real user input, it's unlikely, but it
/// happens in integration tests because of how quickly the
/// command is entered.
macro_rules! check_command {
    ($command:expr, $expected:expr) => {
        let command = $command;
        if command.contains("^[i") {
            return AssertionOutcome::PreconditionFailed(format!(
                "Flake: input reporting keybinding sent too early on `{command}`"
            ));
        } else {
            assert_eq!(command, $expected);
        }
    };
}

/// Tests that the shell reports its input buffer to the Warp typeahead model after
/// a long-running command completes.
pub fn test_input_reporting_posix_shells() -> Builder {
    // When the shell can report its input buffer, we can handle typeahead with
    // line editing. When matching user input ourselves (only on pre-4.0 bash),
    // we do not support line edits.
    let (starter, version) = current_shell_starter_and_version();
    let shell = Shell::new(
        starter.shell_type(),
        Some(version),
        None,
        Default::default(),
        None,
    );
    let supports_line_editing = shell.input_reporting_sequence().is_some();

    let mut input_step = TestStep::new("Enter text to long-running command")
        .with_input_string("true", Some(&["enter"]))
        // Test behavior when one of the typeahead commands is itself long-running.
        .with_input_string("sleep 1", Some(&["enter"]))
        .add_assertion(require_long_running_block_executing(0));

    if supports_line_editing {
        // Test that we correctly handle line edits on both submitted lines and typeahead.
        input_step = input_step
            .with_keystrokes(&["p", "w", "f", "backspace", "d", "enter"])
            .with_keystrokes(&["l", "s", " ", "-", "a", "backspace", "l"]);
    } else {
        input_step = input_step
            .with_input_string("pwd", Some(&["enter"]))
            // This is the input we expect as typeahead.
            .with_input_string("ls -l", None);
    }

    new_builder()
        .set_should_run_test(move || starter.shell_type() != ShellType::PowerShell)
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            TestStep::new("Execute sleep")
                .with_typed_characters(&["sleep 3"])
                .with_keystrokes(&["enter"])
                .add_assertion(
                    assert_long_running_block_executing_for_single_terminal_in_tab(true, 0),
                ),
        )
        .with_step(input_step)
        .with_step(
            new_step_with_default_assertions("Input should be reported to the terminal")
                .add_named_assertion(
                    "Typeahead is in input editor",
                    assert_input_editor_contents(0, "ls -l"),
                )
                .add_named_assertion("Intermediate commands ran", |app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |view, _| {
                        let model = view.model.lock();
                        let blocks = model.block_list();

                        let start_index = blocks
                            .first_non_hidden_block_by_index()
                            .expect("Block should exist");

                        let sleep_3_block =
                            blocks.block_at(start_index).expect("Block should exist");
                        check_command!(sleep_3_block.command_to_string(), "sleep 3");

                        let true_block = blocks
                            .block_at(start_index + BlockIndex::from(1))
                            .expect("Block should exist");
                        assert!(!true_block.is_background());
                        check_command!(true_block.command_to_string(), "true");

                        let sleep_1_block = blocks
                            .block_at(start_index + BlockIndex::from(2))
                            .expect("Block should exist");
                        assert!(!sleep_1_block.is_background());
                        check_command!(sleep_1_block.command_to_string(), "sleep 1");

                        let pwd_block = blocks
                            .block_at(start_index + BlockIndex::from(3))
                            .expect("Block should exist");
                        assert!(!pwd_block.is_background());
                        check_command!(pwd_block.command_to_string(), "pwd");

                        // On shells that support input reporting, there will be
                        // an empty block that formerly held echoed typeahead. On
                        // shells using input matching, the typeahead block is never
                        // created.
                        let next_block = blocks
                            .block_at(start_index + BlockIndex::from(4))
                            .expect("Block should exist");
                        if next_block.is_background() {
                            async_assert!(next_block.is_empty(&TranscriptScope::Terminal))
                        } else {
                            async_assert_eq!(next_block.index(), blocks.active_block_index())
                        }
                    })
                }),
        )
}

/// PowerShell has different behavior for typeahead in that it ignores newlines.
pub fn test_input_reporting_powershell() -> Builder {
    new_builder()
        .set_should_run_test(|| {
            let (starter, _) = current_shell_starter_and_version();
            starter.shell_type() == ShellType::PowerShell
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            TestStep::new("Execute sleep")
                .with_typed_characters(&["sleep 3"])
                .with_keystrokes(&["enter"])
                .add_assertion(
                    assert_long_running_block_executing_for_single_terminal_in_tab(true, 0),
                ),
        )
        .with_step(
            TestStep::new("Enter text to long-running command")
                .with_keystrokes(&["enter"])
                .with_input_string("true", Some(&["enter"]))
                .with_input_string("sleep 1", Some(&["enter"]))
                .add_assertion(require_long_running_block_executing(0)),
        )
        .with_step(
            new_step_with_default_assertions("Input should be reported to the terminal")
                .add_named_assertion(
                    "Typeahead is in input editor",
                    assert_input_editor_contents(0, "truesleep 1"),
                ),
        )
}

/// This tests UNIX-specific signal handling.
#[cfg(not(windows))]
pub fn test_background_output() -> Builder {
    use std::fs::OpenOptions;
    use std::io::Write;
    use std::os::unix::prelude::OpenOptionsExt;

    use regex::Regex;
    use warp::integration_testing::block::assert_background_output;
    use warp::integration_testing::terminal::execute_command_for_single_terminal_in_tab;
    use warp::integration_testing::terminal::util::ExpectedExitStatus;

    let (starter, _) = current_shell_starter_and_version();
    let (spawn_command, kill_command) = match starter.shell_type() {
        ShellType::PowerShell => (
            "$process = Start-Process -FilePath './delayed_output.py' -PassThru",
            "kill -SIGUSR1 $process.Id && echo foreground",
        ),
        _ => (
            "./delayed_output.py &",
            "kill -SIGUSR1 %1 && echo foreground",
        ),
    };
    new_builder()
        .with_setup(|utils| {
            let dir = utils.test_dir();
            // Use a Python script because fish can't run functions in the background
            // https://github.com/fish-shell/fish-shell/issues/238
            let script_path = dir.join("delayed_output.py");

            OpenOptions::new()
                .create_new(true)
                .write(true)
                .mode(0o755)
                .open(script_path)
                .expect("could not create script")
                .write_all(
                    br#"#!/usr/bin/env python3
import signal
import time

# Wait for a SIGUSR1 signal, after which we should print out
# more text.
def handler(signo, cur_frame):
  time.sleep(1)
  print("Output 2")
  print("Output 3")
signal.signal(signal.SIGUSR1, handler)

print("Output 1")
time.sleep(100)
"#,
                )
                .expect("could not write Python script");
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            spawn_command.into(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(
            TestStep::new("First line of background output appears")
                .add_assertion(assert_background_output(0, "Output 1\n")),
        )
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            // Send the signal to the background process and produce some output.
            kill_command.into(),
            ExpectedExitStatus::Success,
            "foreground",
        ))
        .with_step(
            TestStep::new("Rest of background output appears in a new block").add_assertion(
                assert_background_output(
                    0,
                    // Use a regex because the "job completed" message format is shell-specific.
                    // Depending on timing, the "Output 2" line could be part of the
                    // block for `true`, so it's optional - we expect the next
                    // line to always be in the background block though.
                    Regex::new("^(Output 2\n)?Output 3\n").expect("Regex is valid"),
                ),
            ),
        )
}

#[cfg(windows)]
// TODO(CORE-2302): enable this test for windows
pub fn test_background_output() -> Builder {
    new_builder()
}

/// Require (as a test precondition) that a long-running block is executing.
/// Use this instead of [`assert_long_running_block_executing`] with `sleep` commands
/// to turn the race condition of the sleep ending too soon into a flake.
fn require_long_running_block_executing(tab_index: usize) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = single_terminal_view_for_tab(app, window_id, tab_index);
        terminal_view.read(app, |view, _ctx| {
            // The running block should have received the text.
            let model = view.model.lock();
            if !model
                .block_list()
                .active_block()
                .is_active_and_long_running()
            {
                // There's an implicit race condition where the sleep call
                // can finish before we get here.
                // The most robust way of handling this is to just treat it as a flake.
                AssertionOutcome::PreconditionFailed(
                    "long-running block no longer executing".to_owned(),
                )
            } else {
                AssertionOutcome::Success
            }
        })
    })
}
