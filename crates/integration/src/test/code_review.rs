use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use command::blocking::Command;
use warp::features::FeatureFlag;
use warp::integration_testing::code_review::{
    assert_code_review_anchor, assert_code_review_line_text, assert_code_review_loaded,
    assert_code_review_scroll_region, scroll_code_review_to_deleted_range,
    scroll_code_review_to_footer, scroll_code_review_to_header, scroll_code_review_to_line,
    ScrollRegion,
};
use warp::integration_testing::terminal::wait_until_bootstrapped_single_pane_for_tab;
use warp::integration_testing::view_getters::{single_terminal_view_for_tab, workspace_view};
use warp::workspace::WorkspaceAction;
use warpui_core::integration::{AssertionCallback, TestStep};
use warpui_core::{async_assert, App, WindowId};

use super::new_builder;
use crate::util::write_all_rc_files_for_test;
use crate::Builder;

const TEST_FILE_NAME: &str = "scroll_target.txt";
const TARGET_LINE_NUMBER: usize = 70;
const INSERT_ABOVE_LINE_NUMBER: usize = 15;
const INSERT_BELOW_LINE_NUMBER: usize = 250;
const INSERTED_LINE_COUNT: usize = 10;
const TOTAL_LINE_COUNT: usize = 400;

fn base_line_text(line_number: usize) -> String {
    format!("line {line_number:03}")
}

fn modified_line_text(line_number: usize) -> String {
    format!("line {line_number:03} modified")
}

fn initial_committed_contents() -> String {
    (1..=TOTAL_LINE_COUNT)
        .map(|line_number| format!("{}\n", base_line_text(line_number)))
        .collect()
}

fn initial_diff_contents() -> String {
    (1..=TOTAL_LINE_COUNT)
        .map(|line_number| {
            let line_text =
                if (10..=80).contains(&line_number) || (200..=300).contains(&line_number) {
                    modified_line_text(line_number)
                } else {
                    base_line_text(line_number)
                };
            format!("{line_text}\n")
        })
        .collect()
}

fn inserted_lines(prefix: &str) -> Vec<String> {
    (1..=INSERTED_LINE_COUNT)
        .map(|index| format!("{prefix} inserted {index:02}"))
        .collect()
}

fn insert_lines(path: &Path, before_line_number: usize, new_lines: &[String]) {
    let contents = fs::read_to_string(path).expect("should read test file");
    let mut lines: Vec<String> = contents.lines().map(ToOwned::to_owned).collect();
    let insert_index = before_line_number.saturating_sub(1);
    lines.splice(insert_index..insert_index, new_lines.iter().cloned());
    fs::write(path, format!("{}\n", lines.join("\n"))).expect("should rewrite test file");
}

fn run_git(test_dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(test_dir)
        .status()
        .expect("git command should run");
    assert!(status.success(), "git {:?} should succeed", args);
}

fn open_code_review_panel(app: &mut App, window_id: WindowId) {
    let workspace = workspace_view(app, window_id);
    app.update(|ctx| {
        ctx.dispatch_typed_action_for_view(
            window_id,
            workspace.id(),
            &WorkspaceAction::ToggleRightPanel,
        );
    });
}

fn assert_repo_detected() -> AssertionCallback {
    Box::new(|app, window_id| {
        let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
        terminal_view.read(app, |terminal_view, _ctx| {
            async_assert!(
                terminal_view.current_repo_path().is_some(),
                "expected the active terminal to detect a git repository"
            )
        })
    })
}

fn scroll_code_review_to_target_line() -> TestStep {
    scroll_code_review_to_line(TEST_FILE_NAME, TARGET_LINE_NUMBER)
        .set_timeout(Duration::from_secs(10))
        .set_retries(2)
        .add_assertion(assert_code_review_anchor(
            TEST_FILE_NAME,
            modified_line_text(TARGET_LINE_NUMBER),
            Some(TARGET_LINE_NUMBER),
        ))
        // Allow the scroll debounce (150ms) to fire so that the stored
        // scroll context is captured before the next step mutates the file.
        .set_post_step_pause(Duration::from_millis(250))
}

fn mutate_test_file(before_line_number: usize, prefix: &'static str) -> TestStep {
    TestStep::new(&format!(
        "Insert {INSERTED_LINE_COUNT} lines at {before_line_number}"
    ))
    .with_action(move |app, window_id, _| {
        let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
        let cwd = terminal_view
            .read(app, |terminal_view, _ctx| terminal_view.pwd())
            .expect("terminal should expose current working directory");
        let file_path = PathBuf::from(cwd).join(TEST_FILE_NAME);
        let new_lines = inserted_lines(prefix);
        insert_lines(&file_path, before_line_number, &new_lines);
    })
    .set_post_step_pause(Duration::from_millis(250))
}

fn code_review_scroll_anchor_builder(
    insertion_line_number: usize,
    insertion_prefix: &'static str,
) -> Builder {
    FeatureFlag::CodeReviewScrollPreservation.set_enabled(true);
    FeatureFlag::IncrementalAutoReload.set_enabled(true);
    let inserted_line_text = inserted_lines(insertion_prefix)
        .into_iter()
        .next()
        .expect("inserted lines should not be empty");
    new_builder()
        .use_tmp_filesystem_for_test_root_directory()
        .with_setup(|utils| {
            let test_dir = utils.test_dir();
            let repo_dir = test_dir.join("repo");
            fs::create_dir_all(&repo_dir).expect("should create repo subdirectory");
            let repo_dir_string = repo_dir
                .to_str()
                .expect("repo directory should be valid utf-8");

            write_all_rc_files_for_test(&test_dir, format!("cd {repo_dir_string}"));

            fs::write(repo_dir.join(TEST_FILE_NAME), initial_committed_contents())
                .expect("should write initial committed contents");
            run_git(&repo_dir, &["init", "-b", "main"]);
            run_git(&repo_dir, &["config", "user.email", "test@example.com"]);
            run_git(&repo_dir, &["config", "user.name", "Warp Integration Test"]);
            run_git(&repo_dir, &["add", TEST_FILE_NAME]);
            run_git(&repo_dir, &["commit", "-m", "Initial commit"]);

            fs::write(repo_dir.join(TEST_FILE_NAME), initial_diff_contents())
                .expect("should write initial diff contents");
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            TestStep::new("Wait for the terminal to detect the git repository")
                .set_timeout(Duration::from_secs(20))
                .add_assertion(assert_repo_detected()),
        )
        .with_step(
            TestStep::new("Open the code review panel")
                .with_action(|app, window_id, _| open_code_review_panel(app, window_id)),
        )
        .with_step(
            TestStep::new("Wait for the code review panel to load file diffs")
                .set_timeout(Duration::from_secs(20))
                .add_assertion(assert_code_review_loaded()),
        )
        .with_step(scroll_code_review_to_target_line())
        .with_step(mutate_test_file(insertion_line_number, insertion_prefix))
        .with_step(
            TestStep::new("Wait for code review to reflect the inserted lines")
                .set_timeout(Duration::from_secs(20))
                .add_assertion(assert_code_review_line_text(
                    TEST_FILE_NAME,
                    insertion_line_number,
                    inserted_line_text,
                )),
        )
        .with_step(
            TestStep::new("Wait for code review to preserve the visible anchor text")
                .set_timeout(Duration::from_secs(20))
                .add_assertion(assert_code_review_anchor(
                    TEST_FILE_NAME,
                    modified_line_text(TARGET_LINE_NUMBER),
                    None,
                )),
        )
}

pub fn test_code_review_scroll_anchor_preserved_when_inserting_above() -> Builder {
    code_review_scroll_anchor_builder(INSERT_ABOVE_LINE_NUMBER, "above")
}

pub fn test_code_review_scroll_anchor_unchanged_when_inserting_below() -> Builder {
    code_review_scroll_anchor_builder(INSERT_BELOW_LINE_NUMBER, "below")
}

// --- Multi-file test ---
// Tests that scroll preservation works when scrolled to the second file in the
// code review list. This exercises the adjustment callback returning an
// item-relative offset (not absolute), which only matters for index > 0.

const SECOND_FILE_NAME: &str = "second_file.txt";
const FIRST_FILE_NAME: &str = "first_file.txt";
const MULTI_FILE_TARGET_LINE: usize = 70;
const MULTI_FILE_INSERT_LINE: usize = 15;

fn multi_file_base_line(file_prefix: &str, line_number: usize) -> String {
    format!("{file_prefix} line {line_number:03}")
}

fn multi_file_modified_line(file_prefix: &str, line_number: usize) -> String {
    format!("{file_prefix} line {line_number:03} modified")
}

fn multi_file_committed_contents(file_prefix: &str) -> String {
    (1..=TOTAL_LINE_COUNT)
        .map(|n| format!("{}\n", multi_file_base_line(file_prefix, n)))
        .collect()
}

fn multi_file_diff_contents(file_prefix: &str) -> String {
    (1..=TOTAL_LINE_COUNT)
        .map(|n| {
            let text = if (10..=80).contains(&n) || (200..=300).contains(&n) {
                multi_file_modified_line(file_prefix, n)
            } else {
                multi_file_base_line(file_prefix, n)
            };
            format!("{text}\n")
        })
        .collect()
}

fn mutate_named_file(
    file_name: &'static str,
    before_line_number: usize,
    prefix: &'static str,
) -> TestStep {
    TestStep::new(&format!(
        "Insert {INSERTED_LINE_COUNT} lines at {before_line_number} in {file_name}"
    ))
    .with_action(move |app, window_id, _| {
        let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
        let cwd = terminal_view
            .read(app, |terminal_view, _ctx| terminal_view.pwd())
            .expect("terminal should expose current working directory");
        let file_path = PathBuf::from(cwd).join(file_name);
        let new_lines = inserted_lines(prefix);
        insert_lines(&file_path, before_line_number, &new_lines);
    })
    .set_post_step_pause(Duration::from_millis(250))
}

// --- Deleted range test ---
// Tests that scroll preservation works when scrolled to a deleted (removed) line
// region. This exercises the RemovedLine variant of RelocatableScrollContext.

const DELETED_RANGE_START: usize = 61;
const DELETED_RANGE_END: usize = 80;
/// Current buffer line just before the deleted range. The temporary blocks
/// for deleted lines 61-80 appear immediately after this line in the diff.
const DELETED_RANGE_NEAR_LINE: usize = 60;

fn deleted_range_diff_contents() -> String {
    (1..=TOTAL_LINE_COUNT)
        .filter(|&n| !(DELETED_RANGE_START..=DELETED_RANGE_END).contains(&n))
        .map(|n| {
            let text = if (200..=300).contains(&n) {
                modified_line_text(n)
            } else {
                base_line_text(n)
            };
            format!("{text}\n")
        })
        .collect()
}

pub fn test_code_review_scroll_preserved_deleted_range() -> Builder {
    FeatureFlag::CodeReviewScrollPreservation.set_enabled(true);
    FeatureFlag::IncrementalAutoReload.set_enabled(true);

    let inserted_line_text = inserted_lines("above")
        .into_iter()
        .next()
        .expect("inserted lines should not be empty");

    new_builder()
        .use_tmp_filesystem_for_test_root_directory()
        .with_setup(|utils| {
            let test_dir = utils.test_dir();
            let repo_dir = test_dir.join("repo");
            fs::create_dir_all(&repo_dir).expect("should create repo subdirectory");
            let repo_dir_string = repo_dir
                .to_str()
                .expect("repo directory should be valid utf-8");

            write_all_rc_files_for_test(&test_dir, format!("cd {repo_dir_string}"));

            fs::write(repo_dir.join(TEST_FILE_NAME), initial_committed_contents())
                .expect("should write initial committed contents");
            run_git(&repo_dir, &["init", "-b", "main"]);
            run_git(&repo_dir, &["config", "user.email", "test@example.com"]);
            run_git(&repo_dir, &["config", "user.name", "Warp Integration Test"]);
            run_git(&repo_dir, &["add", TEST_FILE_NAME]);
            run_git(&repo_dir, &["commit", "-m", "Initial commit"]);

            // Write diff contents that DELETE lines 61-80
            fs::write(repo_dir.join(TEST_FILE_NAME), deleted_range_diff_contents())
                .expect("should write deleted range diff contents");
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            TestStep::new("Wait for the terminal to detect the git repository")
                .set_timeout(Duration::from_secs(20))
                .add_assertion(assert_repo_detected()),
        )
        .with_step(
            TestStep::new("Open the code review panel")
                .with_action(|app, window_id, _| open_code_review_panel(app, window_id)),
        )
        .with_step(
            TestStep::new("Wait for the code review panel to load file diffs")
                .set_timeout(Duration::from_secs(20))
                .add_assertion(assert_code_review_loaded()),
        )
        .with_step(
            scroll_code_review_to_deleted_range(TEST_FILE_NAME, DELETED_RANGE_NEAR_LINE)
                .set_timeout(Duration::from_secs(10))
                .set_retries(2)
                .add_assertion(assert_code_review_scroll_region(ScrollRegion::RemovedLine))
                .set_post_step_pause(Duration::from_millis(250)),
        )
        .with_step(mutate_test_file(INSERT_ABOVE_LINE_NUMBER, "above"))
        .with_step(
            TestStep::new("Wait for code review to reflect the inserted lines")
                .set_timeout(Duration::from_secs(20))
                .add_assertion(assert_code_review_line_text(
                    TEST_FILE_NAME,
                    INSERT_ABOVE_LINE_NUMBER,
                    inserted_line_text,
                ))
                // Allow time for the asynchronous diff recomputation to complete.
                // Without this, the assertion below may pass against the stale
                // (pre-recompute) layout where temporary blocks haven't moved.
                .set_post_step_pause(Duration::from_millis(1000)),
        )
        .with_step(
            TestStep::new("Assert scroll is still in the deleted range after preservation")
                .set_timeout(Duration::from_secs(20))
                .add_assertion(assert_code_review_scroll_region(ScrollRegion::RemovedLine)),
        )
}

// --- Header range test ---
// Tests that scroll preservation works when scrolled to the file header region.
// This exercises the Header variant of RelocatableScrollContext.

pub fn test_code_review_scroll_preserved_header_range() -> Builder {
    FeatureFlag::CodeReviewScrollPreservation.set_enabled(true);
    FeatureFlag::IncrementalAutoReload.set_enabled(true);

    let inserted_line_text = inserted_lines("above")
        .into_iter()
        .next()
        .expect("inserted lines should not be empty");

    new_builder()
        .use_tmp_filesystem_for_test_root_directory()
        .with_setup(|utils| {
            let test_dir = utils.test_dir();
            let repo_dir = test_dir.join("repo");
            fs::create_dir_all(&repo_dir).expect("should create repo subdirectory");
            let repo_dir_string = repo_dir
                .to_str()
                .expect("repo directory should be valid utf-8");

            write_all_rc_files_for_test(&test_dir, format!("cd {repo_dir_string}"));

            fs::write(repo_dir.join(TEST_FILE_NAME), initial_committed_contents())
                .expect("should write initial committed contents");
            run_git(&repo_dir, &["init", "-b", "main"]);
            run_git(&repo_dir, &["config", "user.email", "test@example.com"]);
            run_git(&repo_dir, &["config", "user.name", "Warp Integration Test"]);
            run_git(&repo_dir, &["add", TEST_FILE_NAME]);
            run_git(&repo_dir, &["commit", "-m", "Initial commit"]);

            fs::write(repo_dir.join(TEST_FILE_NAME), initial_diff_contents())
                .expect("should write initial diff contents");
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            TestStep::new("Wait for the terminal to detect the git repository")
                .set_timeout(Duration::from_secs(20))
                .add_assertion(assert_repo_detected()),
        )
        .with_step(
            TestStep::new("Open the code review panel")
                .with_action(|app, window_id, _| open_code_review_panel(app, window_id)),
        )
        .with_step(
            TestStep::new("Wait for the code review panel to load file diffs")
                .set_timeout(Duration::from_secs(20))
                .add_assertion(assert_code_review_loaded()),
        )
        .with_step(
            scroll_code_review_to_header(TEST_FILE_NAME)
                .set_timeout(Duration::from_secs(10))
                .set_retries(2)
                .add_assertion(assert_code_review_scroll_region(ScrollRegion::Header))
                .set_post_step_pause(Duration::from_millis(250)),
        )
        .with_step(mutate_test_file(INSERT_ABOVE_LINE_NUMBER, "above"))
        .with_step(
            TestStep::new("Wait for code review to reflect the inserted lines")
                .set_timeout(Duration::from_secs(20))
                .add_assertion(assert_code_review_line_text(
                    TEST_FILE_NAME,
                    INSERT_ABOVE_LINE_NUMBER,
                    inserted_line_text,
                )),
        )
        .with_step(
            TestStep::new("Assert scroll is still in the header region after preservation")
                .set_timeout(Duration::from_secs(20))
                .add_assertion(assert_code_review_scroll_region(ScrollRegion::Header)),
        )
}

// --- Footer range test ---
// Tests that scroll preservation works when scrolled past the editor content
// into the footer region. This exercises the Footer variant of RelocatableScrollContext.
//
// The footer region of a file is only reachable when there is a sufficiently
// tall file below it in the list; otherwise the list's max-scroll clamping
// prevents scrolling past the editor content. This test reuses the multi-file
// helpers (FIRST_FILE_NAME / SECOND_FILE_NAME) so that first_file.txt (index 0)
// has second_file.txt below it, making the footer reachable.

pub fn test_code_review_scroll_preserved_footer_range() -> Builder {
    FeatureFlag::CodeReviewScrollPreservation.set_enabled(true);
    FeatureFlag::IncrementalAutoReload.set_enabled(true);

    let inserted_line_text = inserted_lines("first")
        .into_iter()
        .next()
        .expect("inserted lines should not be empty");

    new_builder()
        .use_tmp_filesystem_for_test_root_directory()
        .with_setup(|utils| {
            let test_dir = utils.test_dir();
            let repo_dir = test_dir.join("repo");
            fs::create_dir_all(&repo_dir).expect("should create repo subdirectory");
            let repo_dir_string = repo_dir
                .to_str()
                .expect("repo directory should be valid utf-8");

            write_all_rc_files_for_test(&test_dir, format!("cd {repo_dir_string}"));

            // Two files: first_file.txt at index 0, second_file.txt at index 1.
            // We scroll to the footer of the first file; the second file
            // provides enough total list height so the footer is reachable.
            fs::write(
                repo_dir.join(FIRST_FILE_NAME),
                multi_file_committed_contents("first"),
            )
            .expect("should write first file committed contents");
            fs::write(
                repo_dir.join(SECOND_FILE_NAME),
                multi_file_committed_contents("second"),
            )
            .expect("should write second file committed contents");

            run_git(&repo_dir, &["init", "-b", "main"]);
            run_git(&repo_dir, &["config", "user.email", "test@example.com"]);
            run_git(&repo_dir, &["config", "user.name", "Warp Integration Test"]);
            run_git(&repo_dir, &["add", FIRST_FILE_NAME, SECOND_FILE_NAME]);
            run_git(&repo_dir, &["commit", "-m", "Initial commit"]);

            fs::write(
                repo_dir.join(FIRST_FILE_NAME),
                multi_file_diff_contents("first"),
            )
            .expect("should write first file diff contents");
            fs::write(
                repo_dir.join(SECOND_FILE_NAME),
                multi_file_diff_contents("second"),
            )
            .expect("should write second file diff contents");
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            TestStep::new("Wait for the terminal to detect the git repository")
                .set_timeout(Duration::from_secs(20))
                .add_assertion(assert_repo_detected()),
        )
        .with_step(
            TestStep::new("Open the code review panel")
                .with_action(|app, window_id, _| open_code_review_panel(app, window_id)),
        )
        .with_step(
            TestStep::new("Wait for the code review panel to load file diffs")
                .set_timeout(Duration::from_secs(20))
                .add_assertion(assert_code_review_loaded()),
        )
        .with_step(
            scroll_code_review_to_footer(FIRST_FILE_NAME)
                .set_timeout(Duration::from_secs(10))
                .set_retries(2)
                .add_assertion(assert_code_review_scroll_region(ScrollRegion::Footer))
                .set_post_step_pause(Duration::from_millis(250)),
        )
        .with_step(mutate_named_file(
            FIRST_FILE_NAME,
            MULTI_FILE_INSERT_LINE,
            "first",
        ))
        .with_step(
            TestStep::new("Wait for code review to reflect the inserted lines")
                .set_timeout(Duration::from_secs(20))
                .add_assertion(assert_code_review_line_text(
                    FIRST_FILE_NAME,
                    MULTI_FILE_INSERT_LINE,
                    inserted_line_text,
                )),
        )
        .with_step(
            TestStep::new("Assert scroll is still in the footer region after preservation")
                .set_timeout(Duration::from_secs(20))
                .add_assertion(assert_code_review_scroll_region(ScrollRegion::Footer)),
        )
}

pub fn test_code_review_scroll_preserved_second_file() -> Builder {
    FeatureFlag::CodeReviewScrollPreservation.set_enabled(true);
    FeatureFlag::IncrementalAutoReload.set_enabled(true);

    let inserted_line_text = inserted_lines("second")
        .into_iter()
        .next()
        .expect("inserted lines should not be empty");

    new_builder()
        .use_tmp_filesystem_for_test_root_directory()
        .with_setup(|utils| {
            let test_dir = utils.test_dir();
            let repo_dir = test_dir.join("repo");
            fs::create_dir_all(&repo_dir).expect("should create repo subdirectory");
            let repo_dir_string = repo_dir
                .to_str()
                .expect("repo directory should be valid utf-8");

            write_all_rc_files_for_test(&test_dir, format!("cd {repo_dir_string}"));

            // Create and commit two files. File names sort alphabetically so
            // first_file.txt appears at index 0 and second_file.txt at index 1.
            fs::write(
                repo_dir.join(FIRST_FILE_NAME),
                multi_file_committed_contents("first"),
            )
            .expect("should write first file committed contents");
            fs::write(
                repo_dir.join(SECOND_FILE_NAME),
                multi_file_committed_contents("second"),
            )
            .expect("should write second file committed contents");

            run_git(&repo_dir, &["init", "-b", "main"]);
            run_git(&repo_dir, &["config", "user.email", "test@example.com"]);
            run_git(&repo_dir, &["config", "user.name", "Warp Integration Test"]);
            run_git(&repo_dir, &["add", FIRST_FILE_NAME, SECOND_FILE_NAME]);
            run_git(&repo_dir, &["commit", "-m", "Initial commit"]);

            // Write modified versions to create diffs in both files
            fs::write(
                repo_dir.join(FIRST_FILE_NAME),
                multi_file_diff_contents("first"),
            )
            .expect("should write first file diff contents");
            fs::write(
                repo_dir.join(SECOND_FILE_NAME),
                multi_file_diff_contents("second"),
            )
            .expect("should write second file diff contents");
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            TestStep::new("Wait for the terminal to detect the git repository")
                .set_timeout(Duration::from_secs(20))
                .add_assertion(assert_repo_detected()),
        )
        .with_step(
            TestStep::new("Open the code review panel")
                .with_action(|app, window_id, _| open_code_review_panel(app, window_id)),
        )
        .with_step(
            TestStep::new("Wait for the code review panel to load file diffs")
                .set_timeout(Duration::from_secs(20))
                .add_assertion(assert_code_review_loaded()),
        )
        // Scroll to a target line in the SECOND file (index 1)
        .with_step(
            scroll_code_review_to_line(SECOND_FILE_NAME, MULTI_FILE_TARGET_LINE)
                .set_timeout(Duration::from_secs(10))
                .set_retries(2)
                .add_assertion(assert_code_review_anchor(
                    SECOND_FILE_NAME,
                    multi_file_modified_line("second", MULTI_FILE_TARGET_LINE),
                    Some(MULTI_FILE_TARGET_LINE),
                ))
                .set_post_step_pause(Duration::from_millis(250)),
        )
        // Insert lines above the target in the second file
        .with_step(mutate_named_file(
            SECOND_FILE_NAME,
            MULTI_FILE_INSERT_LINE,
            "second",
        ))
        .with_step(
            TestStep::new("Wait for code review to reflect the inserted lines in second file")
                .set_timeout(Duration::from_secs(20))
                .add_assertion(assert_code_review_line_text(
                    SECOND_FILE_NAME,
                    MULTI_FILE_INSERT_LINE,
                    inserted_line_text,
                )),
        )
        .with_step(
            TestStep::new("Wait for code review to preserve the visible anchor in second file")
                .set_timeout(Duration::from_secs(20))
                .add_assertion(assert_code_review_anchor(
                    SECOND_FILE_NAME,
                    multi_file_modified_line("second", MULTI_FILE_TARGET_LINE),
                    None,
                )),
        )
}
