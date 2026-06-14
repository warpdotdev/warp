use std::fs;
use std::path::Path;
use std::time::Duration;

use command::blocking::Command;
use warp::features::FeatureFlag;
use warp::integration_testing::git_graph::{
    assert_git_graph_loaded, assert_local_branch_exists, assert_op_error_shown,
    create_branch_at_top, open_git_graph_panel, run_failing_checkout,
};
use warp::integration_testing::terminal::wait_until_bootstrapped_single_pane_for_tab;
use warp::integration_testing::view_getters::single_terminal_view_for_tab;
use warpui::async_assert;
use warpui::integration::{AssertionCallback, TestStep};

use super::new_builder;
use crate::util::write_all_rc_files_for_test;
use crate::Builder;

fn run_git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .status()
        .expect("git command should run");
    assert!(status.success(), "git {:?} should succeed", args);
}

/// Builds, under `<test_dir>/repo`, a git repository with two commits and points
/// the shell's working directory at it. Shared by the Git Graph tests.
fn setup_two_commit_repo(test_dir: &Path) {
    let repo_dir = test_dir.join("repo");
    fs::create_dir_all(&repo_dir).expect("should create repo subdirectory");
    let repo_dir_string = repo_dir
        .to_str()
        .expect("repo directory should be valid utf-8");

    write_all_rc_files_for_test(test_dir, format!("cd {repo_dir_string}"));

    run_git(&repo_dir, &["init", "-b", "main"]);
    run_git(&repo_dir, &["config", "user.email", "test@example.com"]);
    run_git(&repo_dir, &["config", "user.name", "Warp Integration Test"]);

    fs::write(repo_dir.join("first.txt"), "first\n").expect("should write first file");
    run_git(&repo_dir, &["add", "."]);
    run_git(&repo_dir, &["commit", "-m", "first commit"]);

    fs::write(repo_dir.join("second.txt"), "second\n").expect("should write second file");
    run_git(&repo_dir, &["add", "."]);
    run_git(&repo_dir, &["commit", "-m", "second commit"]);
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

/// Builds a small repository with two commits, opens the Git Graph, and asserts
/// it loads and renders the commits. End-to-end coverage of the core happy path:
/// terminal detects the repo → panel opens → commit graph loads.
pub fn test_git_graph_loads_commits() -> Builder {
    FeatureFlag::GitGraph.set_enabled(true);

    new_builder()
        .use_tmp_filesystem_for_test_root_directory()
        .with_setup(|utils| setup_two_commit_repo(&utils.test_dir()))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            TestStep::new("Wait for the terminal to detect the git repository")
                .set_timeout(Duration::from_secs(20))
                .add_assertion(assert_repo_detected()),
        )
        .with_step(open_git_graph_panel())
        .with_step(
            TestStep::new("Wait for the Git Graph to load commits")
                .set_timeout(Duration::from_secs(20))
                .add_assertion(assert_git_graph_loaded()),
        )
}

/// Builds the same two-commit repository, opens the Git Graph, runs the "Create
/// Branch" write op at the newest commit, and asserts the new branch shows up
/// after the reload. End-to-end coverage of the write path
/// (context-menu action → `git branch` → reload → branch list updated).
pub fn test_git_graph_create_branch() -> Builder {
    FeatureFlag::GitGraph.set_enabled(true);
    FeatureFlag::GitGraphWrite.set_enabled(true);

    new_builder()
        .use_tmp_filesystem_for_test_root_directory()
        .with_setup(|utils| setup_two_commit_repo(&utils.test_dir()))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            TestStep::new("Wait for the terminal to detect the git repository")
                .set_timeout(Duration::from_secs(20))
                .add_assertion(assert_repo_detected()),
        )
        .with_step(open_git_graph_panel())
        .with_step(
            TestStep::new("Wait for the Git Graph to load commits")
                .set_timeout(Duration::from_secs(20))
                .add_assertion(assert_git_graph_loaded()),
        )
        .with_step(create_branch_at_top("graph-created-branch"))
        .with_step(
            TestStep::new("Wait for the created branch to appear")
                .set_timeout(Duration::from_secs(20))
                .add_assertion(assert_local_branch_exists("graph-created-branch")),
        )
}

/// Builds the same repository, opens the Git Graph, runs a write op that fails
/// (checking out a non-existent branch), and asserts the error banner surfaces.
/// Because the harness renders the panel while waiting, this is a regression
/// guard for the banner painting without panicking.
pub fn test_git_graph_op_error_banner() -> Builder {
    FeatureFlag::GitGraph.set_enabled(true);
    FeatureFlag::GitGraphWrite.set_enabled(true);

    new_builder()
        .use_tmp_filesystem_for_test_root_directory()
        .with_setup(|utils| setup_two_commit_repo(&utils.test_dir()))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            TestStep::new("Wait for the terminal to detect the git repository")
                .set_timeout(Duration::from_secs(20))
                .add_assertion(assert_repo_detected()),
        )
        .with_step(open_git_graph_panel())
        .with_step(
            TestStep::new("Wait for the Git Graph to load commits")
                .set_timeout(Duration::from_secs(20))
                .add_assertion(assert_git_graph_loaded()),
        )
        .with_step(run_failing_checkout())
        .with_step(
            TestStep::new("Wait for the error banner to surface")
                .set_timeout(Duration::from_secs(20))
                .add_assertion(assert_op_error_shown()),
        )
}
