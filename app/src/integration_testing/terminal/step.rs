use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use warpui::integration::{AssertionOutcome, TestStep};
use warpui::{async_assert, Event, SingletonEntity};

use super::util::{current_shell_starter_and_version, nonce, ExpectedExitStatus, ExpectedOutput};
use super::{
    assert_active_block_output_for_single_terminal_in_tab, assert_active_block_received_precmd,
    assert_alt_grid_active, assert_command_executed,
    assert_long_running_block_executing_for_single_terminal_in_tab, assert_terminal_bootstrapped,
    validate_block_output, PYTHON_PROMPT_READY,
};
use crate::cmd_or_ctrl_shift;
use crate::integration_testing::block::assert_num_blocks_in_model;
use crate::integration_testing::command_palette::open_command_palette_and_run_action;
use crate::integration_testing::step::{
    assert_no_pending_model_events, new_step_with_default_assertions,
    new_step_with_default_assertions_for_pane,
};
use crate::integration_testing::terminal::{
    assert_active_block_input_is_empty, assert_context_menu_is_open,
    assert_long_running_block_executing, validate_block_output_on_finished_block,
};
use crate::integration_testing::view_getters::{
    single_input_view_for_tab, single_terminal_view_for_tab, terminal_view,
};
use crate::settings::PrivacySettings;
use crate::terminal::input::InputSuggestionsMode;
use crate::terminal::model::terminal_model::BlockIndex;
use crate::terminal::shell::ShellType;

pub fn wait_until_bootstrapped_single_pane_for_tab(tab_index: usize) -> TestStep {
    wait_until_bootstrapped_pane(tab_index, 0)
}

pub fn initialize_secret_regexes() -> TestStep {
    new_step_with_default_assertions("Initialize default secret regexes").with_action(
        move |app, _, _| {
            let privacy_settings = PrivacySettings::handle(app);
            privacy_settings.update(app, |me, ctx| {
                me.initialize_default_regexes_once(ctx);
            });
        },
    )
}

pub fn wait_until_bootstrapped_pane(tab_index: usize, pane_index: usize) -> TestStep {
    new_step_with_default_assertions("Wait for bootstrapping")
        .add_named_assertion(
            "waiting for bootstrapping",
            assert_terminal_bootstrapped(tab_index, pane_index),
        )
        .set_timeout(Duration::from_secs(20))
        .set_on_failure_handler("bootstrapping failed, bail on the test", move |_, _| {
            let (starter, version) = current_shell_starter_and_version();
            if matches!(&starter.shell_type(), &ShellType::Bash) && version.starts_with('3') {
                // There's a bug in older versions of bash that causes bootstrapping
                // to occasionally fail.
                AssertionOutcome::PreconditionFailed("bash flaked on startup".to_owned())
            } else {
                AssertionOutcome::failure("failed to bootstrap".to_owned())
            }
        })
}

pub fn open_context_menu_for_selected_block() -> Vec<TestStep> {
    let mut steps = open_command_palette_and_run_action("Open Block Context Menu");
    let last = steps.pop().expect("steps should not be empty");
    steps.push(last.add_assertion(assert_context_menu_is_open(true)));
    steps
}

/// Runs the completer with the given input text, waiting up to 2 seconds for the completer to return
/// with results.
pub fn run_completer(tab_index: usize, input_text: impl Into<String>) -> TestStep {
    let input_text = input_text.into();
    new_step_with_default_assertions(&format!("Type {} and hit tab", &input_text))
        .with_typed_characters(&[&input_text])
        .with_keystrokes(&["tab"])
        .set_timeout(Duration::from_secs(2))
        .add_assertion(move |app, window_id| {
            let input_view = single_input_view_for_tab(app, window_id, tab_index);

            input_view.read(app, |input, ctx| {
                let buffer_text = input.buffer_text(ctx);
                // There are 2 possible outcomes that can signify the completer has finished:
                // 1: TabCompletion mode is now active.
                // 2: InputSuggestionsMode is `Closed`, but the buffer text has changed. This is the
                // case when there is a single completion result that we insert directly into the
                // buffer.
                async_assert!(
                    matches!(
                        input.suggestions_mode_model().as_ref(ctx).mode(),
                        InputSuggestionsMode::CompletionSuggestions { .. }
                    ) || (buffer_text != input_text
                        && matches!(
                            input.suggestions_mode_model().as_ref(ctx).mode(),
                            InputSuggestionsMode::Closed
                        )),
                    "Completions did not finish"
                )
            })
        })
}

/// Executes a given command and verifies it is executing.
pub fn execute_long_running_command(tab_idx: usize, command: String) -> TestStep {
    execute_long_running_command_for_pane(tab_idx, 0 /* pane_idx */, command)
}

/// Executes a given command for a specific pane and tab and verifies it is executing.
pub fn execute_long_running_command_for_pane(
    tab_idx: usize,
    pane_idx: usize,
    command: impl AsRef<str>,
) -> TestStep {
    let command = command.as_ref();
    TestStep::new(&format!("Run '{command}' and verify block is running"))
        .add_named_assertion("no pending model events", assert_no_pending_model_events())
        .with_typed_characters(&[command])
        .with_keystrokes(&["enter"])
        .set_timeout(Duration::from_secs(10))
        .add_named_assertion(
            format!("assert '{command}' is running"),
            assert_long_running_block_executing(
                true, /* output_grid_active */
                tab_idx, pane_idx,
            ),
        )
}

/// Executes a python3 interpreter and leaves it running in the active block.
pub fn execute_python_interpreter_in_tab(tab_idx: usize) -> TestStep {
    TestStep::new("Run python3 interpreter")
        .add_named_assertion("no pending model events", assert_no_pending_model_events())
        .with_typed_characters(&["python3"])
        .with_keystrokes(&["enter"])
        .add_assertion(assert_active_block_output_for_single_terminal_in_tab(
            &*PYTHON_PROMPT_READY,
            0,
        ))
        .add_named_assertion(
            "assert python3 is running",
            assert_long_running_block_executing_for_single_terminal_in_tab(
                true, /* output_grid_active */
                tab_idx,
            ),
        )
}

/// Runs an alt-grid program followed by a series of steps and then
/// runs a command to exit the alt grid and asserts it's no longer active.
///
/// The terminal view at tab_index, pane_index is expected to be focused to run the program (this
/// step asserts the alt screen is active on the corresponding TerminalView).
pub fn run_alt_grid_program(
    command: &str,
    tab_index: usize,
    pane_index: usize,
    exit_step: TestStep,
    steps_before_exiting: Vec<TestStep>,
) -> Vec<TestStep> {
    let mut steps = vec![];

    let run_step = TestStep::new(&format!("Run '{command}' and then exit with exit step"))
        .add_named_assertion("no pending model events", assert_no_pending_model_events())
        .with_typed_characters(&[command])
        .with_keystrokes(&["enter"])
        .set_timeout(Duration::from_secs(10))
        .add_named_assertion(
            "alt grid should be active",
            assert_alt_grid_active(tab_index, pane_index, true),
        );

    steps.push(run_step);
    steps.extend(steps_before_exiting);
    steps.push(exit_step);
    steps.push(
        new_step_with_default_assertions("return to block list").add_named_assertion(
            "alt grid should not be active",
            assert_alt_grid_active(tab_index, pane_index, false),
        ),
    );

    steps
}

/// Executes a given command and verifies it's executed.
/// Asserts the exit code of the command is the same as the expected exit code.
/// #Panics if the execution failed for some reason.
pub fn execute_command_for_single_terminal_in_tab(
    tab_idx: usize,
    command: String,
    expected_exit_code: ExpectedExitStatus,
    expected_output: impl ExpectedOutput + 'static,
) -> TestStep {
    execute_command(tab_idx, 0, command, expected_exit_code, expected_output)
}

pub fn execute_command_successfully(command: &str) -> TestStep {
    execute_command(0, 0, command.to_owned(), ExpectedExitStatus::Success, ())
}

pub fn assert_execute_command_successfully(
    command: &str,
    expected_output: impl ExpectedOutput + 'static,
) -> TestStep {
    execute_command(
        0,
        0,
        command.to_owned(),
        ExpectedExitStatus::Success,
        expected_output,
    )
}

/// Creates an event function that saves whether AI mode is active, switches to terminal
/// input mode if needed, and returns a `TypedCharacters` event for the given command.
fn switch_to_terminal_mode_and_type_command(
    tab_idx: usize,
    pane_idx: usize,
    was_ai_mode: Arc<AtomicBool>,
    command: String,
) -> impl Fn(&mut warpui::App, warpui::WindowId) -> Event + 'static {
    move |app, window_id| {
        let tv = terminal_view(app, window_id, tab_idx, pane_idx);
        let is_ai = tv.read(app, |view, ctx| {
            view.input()
                .read(ctx, |input, ctx| input.input_type(ctx).is_ai())
        });
        was_ai_mode.store(is_ai, Ordering::SeqCst);
        if is_ai {
            tv.update(app, |view, ctx| {
                view.input().update(ctx, |input, ctx| {
                    input.set_input_mode_terminal(false, ctx);
                });
            });
        }
        Event::TypedCharacters {
            chars: command.clone(),
        }
    }
}

/// Restores AI input mode if it was previously active (as recorded in `was_ai_mode`).
///
/// This is an action (not an assertion) so it always runs before assertions,
/// ensuring the input mode is restored even if a subsequent assertion fails.
fn restore_ai_mode_if_needed(
    tab_idx: usize,
    pane_idx: usize,
    was_ai_mode: Arc<AtomicBool>,
) -> impl Fn(&mut warpui::App, warpui::WindowId) + 'static {
    move |app, window_id| {
        if was_ai_mode.load(Ordering::SeqCst) {
            let tv = terminal_view(app, window_id, tab_idx, pane_idx);
            tv.update(app, |view, ctx| {
                view.input().update(ctx, |input, ctx| {
                    input.set_input_mode_agent(false, ctx);
                });
            });
        }
    }
}

/// Shared implementation for executing a shell command in the terminal.
///
/// If the input is currently in AI mode, this automatically switches to terminal input
/// mode before typing the command, and restores AI mode after the command completes.
/// This allows callers to run shell commands without manually toggling input mode.
fn execute_command_step(
    tab_idx: usize,
    pane_idx: usize,
    command: String,
    validate_output_fn: impl FnMut(&mut warpui::App, warpui::WindowId) -> AssertionOutcome + 'static,
) -> TestStep {
    let was_ai_mode = Arc::new(AtomicBool::new(false));
    let was_ai_mode_for_restore = was_ai_mode.clone();
    let command_for_event = command.clone();

    new_step_with_default_assertions_for_pane(
        &format!("Run '{command}' and verify block exists"),
        tab_idx,
        pane_idx,
    )
    .with_event_fn(switch_to_terminal_mode_and_type_command(
        tab_idx,
        pane_idx,
        was_ai_mode,
        command_for_event,
    ))
    .with_keystrokes(&["enter"])
    .set_timeout(Duration::from_secs(10))
    .with_action({
        let restore = restore_ai_mode_if_needed(tab_idx, pane_idx, was_ai_mode_for_restore);
        move |app, window_id, _| restore(app, window_id)
    })
    .add_named_assertion(
        format!("assert '{command}' ran"),
        assert_command_executed(tab_idx, pane_idx, command),
    )
    .add_named_assertion("assert command output", validate_output_fn)
    .add_named_assertion(
        "wait for precmd so we have metadata for the next block",
        assert_active_block_received_precmd(tab_idx, pane_idx),
    )
}

/// Executes a given command and verifies it ran successfully.
/// Asserts the exit code matches `expected_exit_code` and validates the output.
///
/// If the input is in AI mode, this automatically switches to terminal input mode
/// before running the command and restores AI mode afterward.
pub fn execute_command(
    tab_idx: usize,
    pane_idx: usize,
    command: String,
    expected_exit_code: ExpectedExitStatus,
    expected_output: impl ExpectedOutput + 'static,
) -> TestStep {
    execute_command_step(tab_idx, pane_idx, command, move |app, window_id| {
        validate_block_output_on_finished_block(&expected_output, tab_idx, pane_idx, window_id, app)
    })
    .add_named_assertion("assert exit code", move |app, window_id| {
        let terminal_view = terminal_view(app, window_id, tab_idx, pane_idx);
        terminal_view.read(app, |view, _ctx| {
            let model = view.model.lock();
            // After the last test step, there should always be a block here, but for
            // some reason, it sometimes doesn't exist.
            let last_block = model
                .block_list()
                .last_non_hidden_block()
                .expect("Block should exist");
            match expected_exit_code {
                ExpectedExitStatus::Success => {
                    if last_block.exit_code().value() != 0 {
                        return AssertionOutcome::immediate_failure(format!(
                            "Expected exit code 0, but got {}. Block output:\n{}\n",
                            last_block.exit_code().value(),
                            last_block
                                .output_grid()
                                .contents_to_string_with_secrets_unobfuscated(
                                    false, /*include_escape_sequences*/
                                    None,  /*max_rows*/
                                )
                        ));
                    }
                }
                ExpectedExitStatus::Failure => {
                    if last_block.exit_code().value() == 0 {
                        return AssertionOutcome::immediate_failure(format!(
                            "Expected non-zero exit code, but got 0. Block output:\n{}\n",
                            last_block
                                .output_grid()
                                .contents_to_string_with_secrets_unobfuscated(
                                    false, /*include_escape_sequences*/
                                    None,  /*max_rows*/
                                )
                        ));
                    }
                }
                ExpectedExitStatus::ExactCode(code) => {
                    if last_block.exit_code() != code {
                        return AssertionOutcome::immediate_failure(format!(
                            "Expected exit code {}, but got {}",
                            code.value(),
                            last_block.exit_code().value()
                        ));
                    }
                }
                ExpectedExitStatus::Any => (),
            };
            AssertionOutcome::Success
        })
    })
    .add_named_assertion(
        "check that input is empty",
        assert_active_block_input_is_empty(tab_idx, pane_idx),
    )
}

/// Executes a given command and validates its output, without asserting the exit code.
///
/// If the input is in AI mode, this automatically switches to terminal input mode
/// before running the command and restores AI mode afterward.
pub fn execute_command_without_expected_exit_code(
    tab_idx: usize,
    pane_idx: usize,
    command: String,
    expected_output: impl ExpectedOutput + 'static,
) -> TestStep {
    execute_command_step(tab_idx, pane_idx, command, move |app, window_id| {
        validate_block_output(&expected_output, tab_idx, pane_idx, window_id, app)
    })
}

// Executes an echo with a random nonce and verifies it's executed.
// The purpose of the nonce is to distinguish between distinct command executions.
pub fn execute_echo(tab_idx: usize) -> TestStep {
    let rand = nonce();
    let command = format!("echo {rand}");
    execute_command_for_single_terminal_in_tab(tab_idx, command, ExpectedExitStatus::Success, rand)
}

// Executes an echo with the specified string.
pub fn execute_echo_str(tab_idx: usize, str: &str) -> TestStep {
    let command = format!("echo \"{str}\"");
    execute_command_for_single_terminal_in_tab(
        tab_idx,
        command,
        ExpectedExitStatus::Success,
        str.to_owned(),
    )
}

/// Runs a performance test on a given tab idx.
/// # Arguments
/// * `tab_idx` - id number of the tab the step should be executed on;
/// * `test_file` is a path to the bash file that will execute the test, and needs to be available
/// for the test itself (use MockUserData structure to ensure it);
/// * `repetitions` denotes how many times a test should be repeated;
/// #Panics if the execution failed for some reason.
pub fn performance_test(tab_idx: usize, test_file: &str, repetitions: usize) -> TestStep {
    execute_command_for_single_terminal_in_tab(
        tab_idx,
        format!("multitime -n {repetitions} bash {test_file}"),
        ExpectedExitStatus::Success,
        (),
    )
}

/// Clears the blocklist so that when we create a new block, its block index is 0.
/// Otherwise, its index within the blocklist will be dependent on the shell bootstrapped.
///
/// NOTE: Call this step after bootstrapping and before running any commands
/// to ensure that the next block created has `BlockIndex::zero()`. Also, this function
/// assumes that there is only one terminal view in tab 0.
pub fn clear_blocklist_to_remove_bootstrapped_blocks() -> TestStep {
    new_step_with_default_assertions("Clear blocklist")
        .with_keystrokes(&[cmd_or_ctrl_shift("k")])
        .set_timeout(Duration::from_secs(10))
        .add_assertion(assert_num_blocks_in_model(1))
}

pub fn hover_over_block_zero() -> TestStep {
    new_step_with_default_assertions("Hover over the recently created block")
        .with_hover_over_saved_position("block_index:0")
        .add_assertion(|app, window_id| {
            let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
            terminal_view.read(app, |view, _ctx| {
                assert_eq!(
                    Some(BlockIndex::from(0)),
                    view.hovered_block_index(),
                    "Expected first block to be hovered over, but got block index {:?}",
                    view.hovered_block_index()
                );
            });
            AssertionOutcome::Success
        })
}
