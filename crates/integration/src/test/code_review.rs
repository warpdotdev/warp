use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use command::blocking::Command;
use warp::features::FeatureFlag;
use warp::integration_testing::code_review::{
    assert_code_review_anchor, assert_code_review_image_preview, assert_code_review_line_text,
    assert_code_review_loaded, assert_code_review_scroll_region, assert_min_hidden_sections,
    expand_first_hidden_section_and_assert_full_reveal, scroll_code_review_to_deleted_range,
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

// --- Hidden-section double-click expansion (GH11622) ---
// The fixture modifies lines 10-80 and 200-300 of a 400-line file, leaving
// large unchanged stretches that the diff editor collapses into hidden
// sections. Fully expanding a section (the action a bar double-click performs,
// via ExpansionType::Both over the section's full range) reveals the whole
// section in one transition and removes it from the hidden set, whereas a
// chunked gutter reveal would only shrink it. The expansion is size-agnostic,
// so this also covers the small-section case (product invariant #8).

pub fn test_code_review_double_click_fully_expands_hidden_section() -> Builder {
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
            TestStep::new("Wait for the diff to collapse unchanged context into hidden sections")
                .set_timeout(Duration::from_secs(20))
                .add_assertion(assert_min_hidden_sections(TEST_FILE_NAME, 1)),
        )
        .with_step(expand_first_hidden_section_and_assert_full_reveal(
            TEST_FILE_NAME,
        ))
}

// ── Image previews (specs/GH12093) ─────────────────────────────────────

const MODIFIED_IMAGE_FILE_NAME: &str = "logo.png";
const DELETED_IMAGE_FILE_NAME: &str = "removed.png";
const ADDED_IMAGE_FILE_NAME: &str = "added.png";
/// An image-looking extension over non-image binary bytes — must keep the
/// binary placeholder (misclassification guard, product spec §8).
const NON_IMAGE_FILE_NAME: &str = "corrupt.png";

/// 1×1 red PNG.
const BASE_PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0xF8, 0xCF, 0xC0, 0xF0,
    0x1F, 0x00, 0x05, 0x00, 0x01, 0xFF, 0x89, 0x99, 0x3D, 0x1D, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45,
    0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
];
/// 2×2 blue PNG.
const MODIFIED_PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x02, 0x08, 0x06, 0x00, 0x00, 0x00, 0x72, 0xB6, 0x0D,
    0x24, 0x00, 0x00, 0x00, 0x10, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x60, 0x60, 0xF8, 0xFF,
    0x1F, 0x82, 0xA1, 0x0C, 0x00, 0x3F, 0xD2, 0x07, 0xF9, 0xB4, 0x12, 0x4F, 0xCD, 0x00, 0x00, 0x00,
    0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
];

fn code_review_image_preview_builder(flag_enabled: bool) -> Builder {
    FeatureFlag::ImagePreviewInCodeReview.set_enabled(flag_enabled);
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

            fs::write(repo_dir.join(MODIFIED_IMAGE_FILE_NAME), BASE_PNG)
                .expect("should write committed image");
            fs::write(repo_dir.join(DELETED_IMAGE_FILE_NAME), BASE_PNG)
                .expect("should write to-be-deleted image");
            run_git(&repo_dir, &["init", "-b", "main"]);
            run_git(&repo_dir, &["config", "user.email", "test@example.com"]);
            run_git(&repo_dir, &["config", "user.name", "Warp Integration Test"]);
            run_git(&repo_dir, &["add", "."]);
            run_git(&repo_dir, &["commit", "-m", "Initial commit"]);

            // Modified, deleted, added (untracked), and non-image binary
            // (untracked) — one file per product-spec status case.
            fs::write(repo_dir.join(MODIFIED_IMAGE_FILE_NAME), MODIFIED_PNG)
                .expect("should overwrite committed image");
            fs::remove_file(repo_dir.join(DELETED_IMAGE_FILE_NAME))
                .expect("should delete committed image");
            fs::write(repo_dir.join(ADDED_IMAGE_FILE_NAME), MODIFIED_PNG)
                .expect("should write untracked image");
            fs::write(
                repo_dir.join(NON_IMAGE_FILE_NAME),
                b"not an image\x00\xFF\xFE",
            )
            .expect("should write non-image binary file");
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
}

pub fn test_code_review_image_preview() -> Builder {
    code_review_image_preview_builder(true).with_step(
        TestStep::new("Assert per-status image previews")
            .set_timeout(Duration::from_secs(20))
            .add_named_assertion(
                "modified image previews old and new sides",
                |app, window_id| {
                    assert_code_review_image_preview(MODIFIED_IMAGE_FILE_NAME, Some((true, true)))(
                        app, window_id,
                    )
                },
            )
            .add_named_assertion("deleted image previews old side only", |app, window_id| {
                assert_code_review_image_preview(DELETED_IMAGE_FILE_NAME, Some((true, false)))(
                    app, window_id,
                )
            })
            .add_named_assertion("added image previews new side only", |app, window_id| {
                assert_code_review_image_preview(ADDED_IMAGE_FILE_NAME, Some((false, true)))(
                    app, window_id,
                )
            })
            .add_named_assertion(
                "non-image .png keeps the binary placeholder",
                |app, window_id| {
                    assert_code_review_image_preview(NON_IMAGE_FILE_NAME, None)(app, window_id)
                },
            ),
    )
}

pub fn test_code_review_image_preview_flag_off() -> Builder {
    code_review_image_preview_builder(false).with_step(
        TestStep::new("Assert no image previews with the feature flag off")
            .set_timeout(Duration::from_secs(20))
            .add_named_assertion(
                "modified image keeps the binary placeholder",
                |app, window_id| {
                    assert_code_review_image_preview(MODIFIED_IMAGE_FILE_NAME, None)(app, window_id)
                },
            ),
    )
}
