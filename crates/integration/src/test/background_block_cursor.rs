use std::time::Duration;

use warp::integration_testing::block::assert_background_output;
use warp::integration_testing::terminal::util::ExpectedExitStatus;
use warp::integration_testing::terminal::{
    execute_command_for_single_terminal_in_tab, wait_until_bootstrapped_single_pane_for_tab,
};
use warp::integration_testing::view_getters::single_terminal_view_for_tab;
use warpui_core::async_assert;
use warpui_core::integration::{AssertionOutcome, TestStep};

use super::{Builder, new_builder};

/// Text printed by the background job below.
const CURSOR_TEST_TEXT: &str = "cursor test output";

/// The grid's stored content for the line printed above, including the
/// newline `print()` adds. This is what `Block::output_to_string()` reports
/// both while the block is running and once it has finished.
const CURSOR_TEST_OUTPUT: &str = "cursor test output\n";

/// Regression test for CORE-3798: a finished background block kept painting a
/// residual cursor, alongside the one in the input editor or the next block.
///
/// Starts a background job, waits for its output to appear, then runs a second
/// command that finishes the background block, capturing the real rendered
/// frame both while the block is still running and after it has finished.
/// This harness has no automated pixel-comparison primitive (see the
/// `gui-integration-test` skill), so the screenshots are the visual evidence
/// for the fix and must be inspected directly; this is why the test requires
/// a real display and is manual-only in CI. Run with:
/// ```sh
/// WARPUI_USE_REAL_DISPLAY_IN_INTEGRATION_TESTS=1 \
///   cargo run -p integration --bin integration -- test_finished_background_block_has_no_cursor
/// ```
///
/// Note: the narrowest manifestation of the bug requires the PTY cursor to
/// land on top of an already-printed character (rather than one column past
/// it) when the block finishes. Repositioning the cursor that way with ANSI
/// sequences after a `print()` tripped an unrelated content-length-tracking
/// quirk for still-active grids, so this test instead leaves the cursor on
/// the blank row after the text — which still exercises the exact predicate
/// this bug lived in (`Block::is_output_cursor_visible`), just not the
/// hardest-to-see single-character variant.
#[cfg(not(windows))]
pub fn test_finished_background_block_has_no_cursor() -> Builder {
    use std::fs::OpenOptions;
    use std::io::Write;
    use std::os::unix::prelude::OpenOptionsExt;

    new_builder()
        .with_real_display()
        .with_setup(|utils| {
            let dir = utils.test_dir();
            // Use a Python script (rather than a shell builtin) because fish can't
            // run functions in the background; see `test_background_output` for
            // the same constraint.
            let script_path = dir.join("background_cursor_output.py");
            OpenOptions::new()
                .create_new(true)
                .write(true)
                .mode(0o755)
                .open(script_path)
                .expect("could not create script")
                .write_all(
                    format!(
                        "#!/usr/bin/env python3\n\
                         import sys\n\
                         import time\n\
                         \n\
                         print({CURSOR_TEST_TEXT:?})\n\
                         sys.stdout.flush()\n\
                         time.sleep(100)\n"
                    )
                    .as_bytes(),
                )
                .expect("could not write Python script");
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "./background_cursor_output.py &".to_string(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(
            TestStep::new("Background block appears and is still running")
                .set_timeout(Duration::from_secs(15))
                .add_named_assertion(
                    "background output matches and block is not finished",
                    assert_background_output(0, CURSOR_TEST_OUTPUT),
                ),
        )
        .with_step(
            TestStep::new("Screenshot while the background block is still running")
                .with_take_screenshot("background_block_running.png"),
        )
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "echo next-command".to_string(),
            ExpectedExitStatus::Success,
            "next-command".to_string(),
        ))
        .with_step(
            TestStep::new("Background block is now finished").add_named_assertion(
                "finished background block still has the expected output",
                |app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |view, _ctx| {
                        let model = view.model.lock();
                        let background_block = model
                            .block_list()
                            .blocks()
                            .iter()
                            .rev()
                            .find(|block| block.is_background());
                        match background_block {
                            Some(block) => {
                                let output = block.output_to_string();
                                // `finish()` truncates trailing rows past the cursor's row, so the
                                // blank row after the printed text (and its newline) disappears
                                // from the string representation once the block finishes.
                                async_assert!(
                                    block.finished() && output == CURSOR_TEST_TEXT,
                                    "expected a finished background block with output {CURSOR_TEST_TEXT:?}, \
                                     got finished={} output={output:?}",
                                    block.finished()
                                )
                            }
                            None => AssertionOutcome::failure("no background block found".into()),
                        }
                    })
                },
            ),
        )
        .with_step(
            TestStep::new("Screenshot after the background block has finished")
                .with_take_screenshot("background_block_finished.png"),
        )
}

#[cfg(windows)]
// TODO(CORE-3798): enable this test for windows; it depends on the same
// background-job-via-Python-script technique as `test_background_output`.
pub fn test_finished_background_block_has_no_cursor() -> Builder {
    new_builder()
}
