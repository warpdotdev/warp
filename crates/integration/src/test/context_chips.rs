use std::path::Path;
use std::time::Duration;

use command::blocking::Command;

use warp::integration_testing::context_chips::{
    assert_git_operation_state_chip_value, enable_git_operation_state_chip,
    open_git_operation_state_chip_menu,
};
use warp::integration_testing::step::new_step_with_default_assertions;
use warp::integration_testing::terminal::{
    assert_command_executed_for_single_terminal_in_tab, wait_until_bootstrapped_single_pane_for_tab,
};
use warpui_core::integration::TestStep;

use super::new_builder;
use crate::Builder;

/// Runs `git` with `args` in `dir`, panicking if it doesn't exit successfully.
/// Used for fixture setup steps that must succeed.
fn run_git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .status()
        .unwrap_or_else(|e| panic!("failed to run git {args:?} in {}: {e}", dir.display()));
    assert!(status.success(), "git {args:?} failed in {}", dir.display());
}

/// Runs `git` with `args` in `dir`, ignoring its exit status. Used for the
/// final setup command that is expected to fail (e.g. a conflicting rebase).
fn run_git_allow_failure(dir: &Path, args: &[&str]) {
    let _ = Command::new("git").args(args).current_dir(dir).status();
}

/// Builds a real repository directly on disk, before the app boots, with a
/// conflicting rebase in progress: two branches each change the same line of
/// the same file and diverge.
///
/// This runs host-side via `with_setup` (rather than by typing `git`
/// commands into the live terminal after bootstrap) so the fixture lands in
/// exactly the directory the harness reports as the block's working
/// directory, sidestepping a harness-only gap where a real interactive `cd`
/// after bootstrap isn't reliably reflected in that reported directory.
fn setup_conflicting_rebase(dir: &Path) {
    run_git(dir, &["init", "-q", "-b", "main"]);
    run_git(dir, &["config", "user.email", "test@test.com"]);
    run_git(dir, &["config", "user.name", "Git TestUser"]);
    std::fs::write(dir.join("file.txt"), "base\n").expect("should write fixture file");
    run_git(dir, &["add", "file.txt"]);
    run_git(dir, &["commit", "-q", "-m", "base"]);
    run_git(dir, &["checkout", "-q", "-b", "feature"]);
    std::fs::write(dir.join("file.txt"), "feature-change\n").expect("should write fixture file");
    run_git(dir, &["commit", "-q", "-am", "feature-change"]);
    run_git(dir, &["checkout", "-q", "main"]);
    std::fs::write(dir.join("file.txt"), "main-change\n").expect("should write fixture file");
    run_git(dir, &["commit", "-q", "-am", "main-change"]);
    run_git(dir, &["checkout", "-q", "feature"]);
    run_git_allow_failure(dir, &["rebase", "main"]);
}

/// Builds a real repository directly on disk, before the app boots, with a
/// bisect session in progress (see [`setup_conflicting_rebase`] for why this
/// runs host-side rather than via typed terminal commands).
fn setup_bisect_session(dir: &Path) {
    run_git(dir, &["init", "-q", "-b", "main"]);
    run_git(dir, &["config", "user.email", "test@test.com"]);
    run_git(dir, &["config", "user.name", "Git TestUser"]);
    run_git(dir, &["commit", "-q", "--allow-empty", "-m", "c1"]);
    run_git(dir, &["commit", "-q", "--allow-empty", "-m", "c2"]);
    run_git(dir, &["commit", "-q", "--allow-empty", "-m", "c3"]);
    run_git(dir, &["bisect", "start"]);
    run_git(dir, &["bisect", "bad"]);
    run_git(dir, &["bisect", "good", "HEAD~2"]);
}

/// Manual, real-display test proving the (opt-in) Git Operation State chip
/// detects a real conflicting rebase, renders its dropdown menu, and running
/// "Abort" from that menu executes the real `git rebase --abort` command and
/// clears the chip. Run with:
///
/// ```sh
/// WARPUI_USE_REAL_DISPLAY_IN_INTEGRATION_TESTS=1 \
///   cargo run -p integration --bin integration -- test_git_operation_state_chip_rebase_menu
/// ```
pub fn test_git_operation_state_chip_rebase_menu() -> Builder {
    new_builder()
        .with_real_display()
        .use_tmp_filesystem_for_test_root_directory()
        .with_setup(|utils| setup_conflicting_rebase(&utils.test_dir()))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(enable_git_operation_state_chip())
        .with_step(
            new_step_with_default_assertions("Git Operation State chip shows the rebase conflict")
                .set_timeout(Duration::from_secs(20))
                .add_assertion(assert_git_operation_state_chip_value(
                    0,
                    Some("rebase-interactive"),
                )),
        )
        .with_step(TestStep::new("Start recording").with_start_recording())
        .with_step(
            TestStep::new("Screenshot the rebase chip")
                .with_take_screenshot("git_operation_state_rebase_chip.png"),
        )
        .with_step(open_git_operation_state_chip_menu())
        .with_step(
            TestStep::new("Screenshot the open rebase menu")
                .with_take_screenshot("git_operation_state_rebase_menu.png"),
        )
        .with_step(
            // The menu opens with "Continue" (the first item) already selected,
            // so two "down"s move the selection to "Skip" then "Abort".
            new_step_with_default_assertions("Select Abort from the menu")
                .with_keystrokes(&["down", "down", "enter"])
                .set_timeout(Duration::from_secs(10))
                .add_assertion(assert_command_executed_for_single_terminal_in_tab(
                    0,
                    "git rebase --abort".to_string(),
                )),
        )
        .with_step(TestStep::new("Stop recording").with_stop_recording())
        .with_step(
            new_step_with_default_assertions("Git Operation State chip clears after the abort")
                .set_timeout(Duration::from_secs(20))
                .add_assertion(assert_git_operation_state_chip_value(0, None)),
        )
}

/// Manual, real-display test proving the (opt-in) Git Operation State chip
/// detects a real bisect session, renders its dropdown menu, and running
/// "Reset" from that menu executes the real `git bisect reset` command and
/// clears the chip. Run with:
///
/// ```sh
/// WARPUI_USE_REAL_DISPLAY_IN_INTEGRATION_TESTS=1 \
///   cargo run -p integration --bin integration -- test_git_operation_state_chip_bisect_menu
/// ```
pub fn test_git_operation_state_chip_bisect_menu() -> Builder {
    new_builder()
        .with_real_display()
        .use_tmp_filesystem_for_test_root_directory()
        .with_setup(|utils| setup_bisect_session(&utils.test_dir()))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(enable_git_operation_state_chip())
        .with_step(
            new_step_with_default_assertions("Git Operation State chip shows the bisect session")
                .set_timeout(Duration::from_secs(20))
                .add_assertion(assert_git_operation_state_chip_value(0, Some("bisect"))),
        )
        .with_step(TestStep::new("Start recording").with_start_recording())
        .with_step(
            TestStep::new("Screenshot the bisect chip")
                .with_take_screenshot("git_operation_state_bisect_chip.png"),
        )
        .with_step(open_git_operation_state_chip_menu())
        .with_step(
            TestStep::new("Screenshot the open bisect menu")
                .with_take_screenshot("git_operation_state_bisect_menu.png"),
        )
        .with_step(
            // The menu opens with "Good" (the first item) already selected, so
            // three "down"s move the selection to "Bad", "Skip", then "Reset".
            new_step_with_default_assertions("Select Reset from the menu")
                .with_keystrokes(&["down", "down", "down", "enter"])
                .set_timeout(Duration::from_secs(10))
                .add_assertion(assert_command_executed_for_single_terminal_in_tab(
                    0,
                    "git bisect reset".to_string(),
                )),
        )
        .with_step(TestStep::new("Stop recording").with_stop_recording())
        .with_step(
            new_step_with_default_assertions("Git Operation State chip clears after the reset")
                .set_timeout(Duration::from_secs(20))
                .add_assertion(assert_git_operation_state_chip_value(0, None)),
        )
}
