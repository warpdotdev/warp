use std::time::Duration;

use warpui::integration::{AssertionCallback, TestStep};
use warpui::{async_assert, async_assert_eq};

use crate::integration_testing::step::new_step_with_default_assertions;
use crate::integration_testing::terminal::util::ExpectedExitStatus;
use crate::integration_testing::terminal::{
    assert_long_running_block_executing_for_single_terminal_in_tab,
    execute_command_for_single_terminal_in_tab, execute_long_running_command,
};
use crate::integration_testing::view_getters::single_terminal_view_for_tab;
use crate::terminal::model::terminal_model::BlockIndex;

/// This test case covers the creates the following output grid:
/// -----------
/// line 0 even
/// line 1 odd
/// line 2 even
/// -----------
///
/// And applies the filter query "odd" so that the resulting output grid becomes:
/// ----------
/// line 1 odd
/// ----------
pub struct SimpleTestCase;

impl SimpleTestCase {
    const OUTPUT_LINE_1: &'static str = "line 0 even";
    const OUTPUT_LINE_2: &'static str = "line 1 odd";
    const OUTPUT_LINE_3: &'static str = "line 2 even";
    const OUTPUT_LINES: &'static str = "line 0 even\nline 1 odd\nline 2 even";
    const FILTER_QUERY: &'static str = "odd";

    pub fn execute_command() -> TestStep {
        execute_command_for_single_terminal_in_tab(
            0,
            format!(
                "echo \"{}\"; echo \"{}\"; echo \"{}\";",
                Self::OUTPUT_LINE_1,
                Self::OUTPUT_LINE_2,
                Self::OUTPUT_LINE_3
            ),
            ExpectedExitStatus::Success,
            Self::OUTPUT_LINES,
        )
    }

    pub fn perform_filter_query() -> TestStep {
        new_step_with_default_assertions("Perform filter query")
            .with_typed_characters(&[Self::FILTER_QUERY])
            .add_named_assertion(
                "Assert that 1 line is left after filtering",
                Self::assert_filter_is_applied(),
            )
    }

    pub fn assert_filter_is_applied() -> AssertionCallback {
        Box::new(|app, window_id| {
            let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
            terminal_view.read(app, |view, _ctx| {
                let model = view.model.lock();
                let displayed_output_rows = model
                    .block_list()
                    .last_non_hidden_block()
                    .expect("No last non-hidden block found.")
                    .displayed_output_rows()
                    .expect("No displayed output rows found.")
                    .collect::<Vec<_>>();
                async_assert_eq!(displayed_output_rows, vec![1])
            })
        })
    }
}

/// This test case covers the creates the following output grid.
/// Note that 123-456-7890 is the secret.
/// -----------
/// line 0 even
/// line 1 (phone number is 123-456-7890) odd
/// line 2 even
/// -----------
///
/// And applies the filter query "odd" so that the resulting output grid becomes:
/// ----------
/// line 1 (phone number is 123-456-7890) odd
/// ----------
pub struct SecretTestCase;

impl SecretTestCase {
    const OUTPUT_LINE_1: &'static str = "line 0 even";
    const OUTPUT_LINE_2: &'static str = "line 1 (phone number is 123-456-7890) odd";
    const OUTPUT_LINE_3: &'static str = "line 2 even";
    const OUTPUT_LINES: &'static str =
        "line 0 even\nline 1 (phone number is 123-456-7890) odd\nline 2 even";
    const FILTER_QUERY: &'static str = "odd";

    pub fn execute_command() -> TestStep {
        execute_command_for_single_terminal_in_tab(
            0,
            format!(
                "echo \"{}\"; echo \"{}\"; echo \"{}\";",
                Self::OUTPUT_LINE_1,
                Self::OUTPUT_LINE_2,
                Self::OUTPUT_LINE_3
            ),
            ExpectedExitStatus::Success,
            Self::OUTPUT_LINES,
        )
    }

    pub fn perform_filter_query() -> TestStep {
        new_step_with_default_assertions("Perform filter query")
            .with_typed_characters(&[Self::FILTER_QUERY])
            .add_named_assertion(
                "Assert that 1 line are left after filtering",
                Self::assert_filter_is_applied(),
            )
    }

    pub fn assert_filter_is_applied() -> AssertionCallback {
        Box::new(|app, window_id| {
            let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
            terminal_view.read(app, |view, _ctx| {
                let model = view.model.lock();
                let displayed_output_rows = model
                    .block_list()
                    .last_non_hidden_block()
                    .expect("No last non-hidden block found.")
                    .displayed_output_rows()
                    .expect("No displayed output rows found.")
                    .collect::<Vec<_>>();
                async_assert_eq!(displayed_output_rows, vec![1])
            })
        })
    }
}

/// This test case covers the creates the following output grid.
/// Note that https://google.com is the URL.
/// -----------
/// line 0 even
/// line 1 https://google.com odd
/// line 2 even
/// -----------
///
/// And applies the filter query "odd" so that the resulting output grid becomes:
/// ----------
/// line 1 https://google.com odd
/// ----------
pub struct URLTestCase;

impl URLTestCase {
    const OUTPUT_LINE_1: &'static str = "line 0 even";
    const OUTPUT_LINE_2: &'static str = "line 1 https://google.com odd";
    const OUTPUT_LINE_3: &'static str = "line 2 even";
    const OUTPUT_LINES: &'static str = "line 0 even\nline 1 https://google.com odd\nline 2 even";
    const FILTER_QUERY: &'static str = "odd";

    pub fn execute_command() -> TestStep {
        execute_command_for_single_terminal_in_tab(
            0,
            format!(
                "echo \"{}\"; echo \"{}\"; echo \"{}\";",
                Self::OUTPUT_LINE_1,
                Self::OUTPUT_LINE_2,
                Self::OUTPUT_LINE_3
            ),
            ExpectedExitStatus::Success,
            Self::OUTPUT_LINES,
        )
    }

    pub fn perform_filter_query() -> TestStep {
        new_step_with_default_assertions("Perform filter query")
            .with_typed_characters(&[Self::FILTER_QUERY])
            .add_named_assertion(
                "Assert that 1 line is left after filtering",
                Self::assert_filter_is_applied(),
            )
    }

    pub fn assert_filter_is_applied() -> AssertionCallback {
        Box::new(|app, window_id| {
            let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
            terminal_view.read(app, |view, _ctx| {
                let model = view.model.lock();
                let displayed_output_rows = model
                    .block_list()
                    .last_non_hidden_block()
                    .expect("No last non-hidden block found.")
                    .displayed_output_rows()
                    .expect("No displayed output rows found.")
                    .collect::<Vec<_>>();
                async_assert_eq!(displayed_output_rows, vec![1])
            })
        })
    }
}

/// This test case covers the creates the following output grid in a long running command:
/// -----------
/// line 0 even
/// line 0 even
/// line 1 odd
/// line 1 odd
/// -----------
///
/// And applies the filter query "odd" so that the resulting output grid becomes:
/// ----------
/// line 1 odd
/// line 1 odd
/// ----------
pub struct LongRunningCommandTestCase;

impl LongRunningCommandTestCase {
    const OUTPUT_LINE_1: &'static str = "line 0 even";
    const OUTPUT_LINE_2: &'static str = "line 1 odd";
    const FILTER_QUERY: &'static str = "odd";

    pub fn enter_input_into_cat() -> Vec<TestStep> {
        let enter_output_line_1_step = TestStep::new("Type in output line 1")
            .add_assertion(assert_long_running_block_executing_for_single_terminal_in_tab(false, 0))
            .with_typed_characters(&[Self::OUTPUT_LINE_1])
            .with_keystrokes(&["enter"])
            .add_named_assertion(
                "Wait for output line has been printed back",
                |app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |view, _ctx| {
                        let model = view.model.lock();
                        let output_grid = model.block_list().active_block().output_grid();
                        // There should be 3 lines present, including "line 0 even" twice and the cursor line.
                        async_assert_eq!(output_grid.len(), 3)
                    })
                },
            );
        let enter_output_line_2_step = TestStep::new("Type in output line 2")
            .add_assertion(assert_long_running_block_executing_for_single_terminal_in_tab(false, 0))
            .with_typed_characters(&[Self::OUTPUT_LINE_2])
            .with_keystrokes(&["enter"])
            .add_named_assertion(
                "Check that output line has been printed back",
                |app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |view, _ctx| {
                        let model = view.model.lock();
                        let output_grid = model.block_list().active_block().output_grid();
                        // There should be 5 lines present, including "line 0 even" twice, "line 1 odd" twice,
                        // and the cursor line.
                        async_assert_eq!(output_grid.len(), 5)
                    })
                },
            );
        vec![
            execute_long_running_command(0, "cat".to_string()),
            enter_output_line_1_step,
            enter_output_line_2_step,
        ]
    }

    pub fn perform_filter_query() -> TestStep {
        TestStep::new("Perform filter query")
            .with_typed_characters(&[Self::FILTER_QUERY])
            .add_named_assertion(
                "Assert that 2 filtered lines and cursor line is left after filtering",
                Self::assert_filter_is_applied(),
            )
    }

    pub fn assert_filter_is_applied() -> AssertionCallback {
        Box::new(|app, window_id| {
            let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
            terminal_view.read(app, |view, _ctx| {
                let model = view.model.lock();
                let displayed_output_rows = model
                    .block_list()
                    .last_non_hidden_block()
                    .expect("No last non-hidden block found.")
                    .displayed_output_rows()
                    .expect("No displayed output rows found.")
                    .collect::<Vec<_>>();
                async_assert_eq!(displayed_output_rows, vec![2, 3, 4])
            })
        })
    }

    pub fn exit_long_running_command() -> TestStep {
        TestStep::new("Check ctrl-c terminates the command")
            .with_click_on_saved_position("block_index:0")
            .with_keystrokes(&["ctrl-c"])
            .set_timeout(Duration::from_secs(10))
            .add_assertion(|app, window_id| {
                let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                terminal_view.read(app, |view, _ctx| {
                    let model = view.model.lock();
                    async_assert!(
                        !model
                            .block_list()
                            .active_block()
                            .is_active_and_long_running(),
                        "Check if the command has terminated"
                    )
                })
            })
    }
}

pub fn open_block_filter_editor() -> TestStep {
    new_step_with_default_assertions("Open block filter editor")
        .with_hover_over_saved_position("block_index:0")
        .with_click_on_saved_position("filter_button_for_block_0")
        .add_named_assertion("Assert that block filter is open", |app, window_id| {
            let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
            terminal_view.read(app, |view, _ctx| {
                async_assert_eq!(
                    view.active_filter_editor_block_index(),
                    Some(BlockIndex::zero())
                )
            })
        })
}

pub fn open_block_filter_editor_for_long_running_command() -> TestStep {
    TestStep::new("Open block filter editor")
        .with_hover_over_saved_position("block_index:0")
        .with_click_on_saved_position("filter_button_for_block_0")
        .add_named_assertion("Assert that block filter is open", |app, window_id| {
            let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
            terminal_view.read(app, |view, _ctx| {
                async_assert_eq!(
                    view.active_filter_editor_block_index(),
                    Some(BlockIndex::zero())
                )
            })
        })
}

pub fn open_block_filter_editor_via_keybinding() -> TestStep {
    new_step_with_default_assertions("Open block filter editor via keybinding")
        .with_keystrokes(&["shift-alt-F"])
        .add_named_assertion("Assert that block filter is open", |app, window_id| {
            let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
            terminal_view.read(app, |view, _ctx| {
                async_assert_eq!(
                    view.active_filter_editor_block_index(),
                    Some(BlockIndex::zero())
                )
            })
        })
}

pub fn open_block_filter_editor_via_keybinding_long_running_command() -> TestStep {
    TestStep::new("Open block filter editor via keybinding")
        .with_keystrokes(&["shift-alt-F"])
        .add_named_assertion("Assert that block filter is open", |app, window_id| {
            let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
            terminal_view.read(app, |view, _ctx| {
                async_assert_eq!(
                    view.active_filter_editor_block_index(),
                    Some(BlockIndex::zero())
                )
            })
        })
}
