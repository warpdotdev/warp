use std::fs;
use std::path::Path;
use std::time::Duration;

use command::blocking::Command;
use warp::features::FeatureFlag;
use warp::integration_testing::git_graph::{assert_git_graph_loaded, open_git_graph_panel};
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
        .with_setup(|utils| {
            let test_dir = utils.test_dir();
            let repo_dir = test_dir.join("repo");
            fs::create_dir_all(&repo_dir).expect("should create repo subdirectory");
            let repo_dir_string = repo_dir
                .to_str()
                .expect("repo directory should be valid utf-8");

            write_all_rc_files_for_test(&test_dir, format!("cd {repo_dir_string}"));

            run_git(&repo_dir, &["init", "-b", "main"]);
            run_git(&repo_dir, &["config", "user.email", "test@example.com"]);
            run_git(&repo_dir, &["config", "user.name", "Warp Integration Test"]);

            fs::write(repo_dir.join("first.txt"), "first\n").expect("should write first file");
            run_git(&repo_dir, &["add", "."]);
            run_git(&repo_dir, &["commit", "-m", "first commit"]);

            fs::write(repo_dir.join("second.txt"), "second\n").expect("should write second file");
            run_git(&repo_dir, &["add", "."]);
            run_git(&repo_dir, &["commit", "-m", "second commit"]);
        })
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
