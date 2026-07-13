//! Integration tests for the files palette (cmd-O) search flow, covering both
//! the on-the-fly streaming engine (`FeatureFlag::StreamingFileSearch` on) and
//! the eager repo-index path (flag off).

use std::path::Path;
use std::time::Duration;

use command::blocking::Command;
use warp::features::FeatureFlag;
use warp::integration_testing::command_palette::{
    open_command_palette_and_run_action, TestStepsExt,
};
use warp::integration_testing::step::new_step_with_default_assertions;
use warp::integration_testing::terminal::wait_until_bootstrapped_single_pane_for_tab;
use warp::integration_testing::view_getters::command_palette_view;
use warp::search::command_palette::mixer::CommandPaletteItemAction;
use warp::search::files::model::FileSearchModel;
use warp::search::QueryFilter;
use warpui_core::integration::{AssertionCallback, TestStep};
use warpui_core::windowing::WindowManager;
use warpui_core::{async_assert, SingletonEntity};

use super::{new_builder, Builder};
use crate::util::write_all_rc_files_for_test;

const TRACKED_FILE: &str = "tracked_target.rs";
const UNTRACKED_FILE: &str = "untracked_target.rs";
const IGNORED_FILE: &str = "ignored_target.rs";

fn run_git(repo_dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(repo_dir)
        .status()
        .expect("git command should run");
    assert!(status.success(), "git {args:?} should succeed");
}

fn assert_repo_detected() -> AssertionCallback {
    Box::new(|app, _window_id| {
        app.read(|ctx| {
            // This is the exact precondition the files data source uses to
            // decide between repo-wide search and current-folder search.
            let repo_root = FileSearchModel::as_ref(ctx).repo_root_location(ctx);
            async_assert!(
                repo_root.is_some(),
                "expected the active window's working directory to resolve to a detected git \
                 repository root"
            )
        })
    })
}

/// Asserts that the files palette results contain file entries ending with
/// every path in `expected` and none ending with a path in `forbidden`.
fn assert_files_palette_results(
    expected: &'static [&'static str],
    forbidden: &'static [&'static str],
) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let palette = command_palette_view(app, window_id);
        palette.read(app, |palette, ctx| {
            let file_paths: Vec<String> = palette
                .search_results(ctx)
                .filter_map(|result| match result.accept_result() {
                    CommandPaletteItemAction::OpenFile { path, .. } => Some(path),
                    _ => None,
                })
                .collect();
            let all_expected_present = expected
                .iter()
                .all(|suffix| file_paths.iter().any(|path| path.ends_with(suffix)));
            let no_forbidden_present = forbidden
                .iter()
                .all(|suffix| !file_paths.iter().any(|path| path.ends_with(suffix)));
            async_assert!(
                all_expected_present && no_forbidden_present,
                "expected file results ending with {expected:?} and none ending with \
                 {forbidden:?}, got {file_paths:?}"
            )
        })
    })
}

/// Shared flow: cd into a git repo fixture, open the files palette, type a
/// query, and verify tracked and untracked files appear while ignored files
/// do not.
///
/// The query step runs immediately after the palette opens, so on the
/// streaming path (flag on) it exercises querying while the initial
/// filesystem scan may still be streaming candidates in; the assertion
/// retries until the expected results appear.
fn files_palette_search_builder() -> Builder {
    new_builder()
        .use_tmp_filesystem_for_test_root_directory()
        .with_setup(|utils| {
            let test_dir = utils.test_dir();
            let repo_dir = test_dir.join("repo");
            std::fs::create_dir_all(repo_dir.join("src")).expect("should create repo/src");
            let repo_dir_string = repo_dir
                .to_str()
                .expect("repo directory should be valid utf-8");

            write_all_rc_files_for_test(&test_dir, format!("cd {repo_dir_string}"));

            // Committed content: a nested source file plus a .gitignore.
            std::fs::write(repo_dir.join("src").join(TRACKED_FILE), "fn main() {}\n")
                .expect("should write tracked file");
            std::fs::write(repo_dir.join(".gitignore"), format!("{IGNORED_FILE}\n"))
                .expect("should write .gitignore");
            run_git(&repo_dir, &["init", "-b", "main"]);
            run_git(&repo_dir, &["config", "user.email", "test@example.com"]);
            run_git(&repo_dir, &["config", "user.name", "Warp Integration Test"]);
            run_git(&repo_dir, &["add", "."]);
            run_git(&repo_dir, &["commit", "-m", "Initial commit"]);

            // Untracked (but not ignored) nested file, and an ignored file
            // that must never show up in results.
            std::fs::create_dir_all(repo_dir.join("notes")).expect("should create repo/notes");
            std::fs::write(repo_dir.join("notes").join(UNTRACKED_FILE), "untracked\n")
                .expect("should write untracked file");
            std::fs::write(repo_dir.join(IGNORED_FILE), "ignored\n")
                .expect("should write ignored file");
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            // Headless integration runs never receive window focus events, so
            // the windowing state's active window (which repo lookup for file
            // search keys off) is populated manually, mirroring the unit-test
            // setup for `FileSearchModel`.
            new_step_with_default_assertions("Mark the test window active").with_action(
                |app, window_id, _| {
                    app.update(|ctx| {
                        WindowManager::handle(ctx).update(ctx, |windowing_state, _ctx| {
                            let stage = windowing_state.stage();
                            windowing_state.overwrite_for_test(stage, Some(window_id));
                        });
                    });
                },
            ),
        )
        .with_step(
            TestStep::new("Wait for the working directory's git repository to be detected")
                .set_timeout(Duration::from_secs(20))
                .add_assertion(assert_repo_detected()),
        )
        .with_steps(
            open_command_palette_and_run_action("Toggle Files Palette").add_named_assertion(
                "Files filter is active",
                |app, window_id| {
                    let palette = command_palette_view(app, window_id);
                    palette.read(app, |palette, ctx| {
                        async_assert!(
                            palette.active_query_filter(ctx) == Some(QueryFilter::Files),
                            "expected the files filter to be active after toggling the files \
                             palette"
                        )
                    })
                },
            ),
        )
        .with_step(
            TestStep::new("Type a file query and verify tracked and untracked results")
                .with_typed_characters(&["target"])
                .set_timeout(Duration::from_secs(20))
                .add_assertion(assert_files_palette_results(
                    &[TRACKED_FILE, UNTRACKED_FILE],
                    &[IGNORED_FILE],
                )),
        )
}

/// Files palette search served by the on-the-fly streaming engine.
pub fn test_files_palette_streaming_search() -> Builder {
    FeatureFlag::CommandPaletteFileSearch.set_enabled(true);
    FeatureFlag::StreamingFileSearch.set_enabled(true);
    files_palette_search_builder()
}

/// Files palette search served by the eager repo index (streaming flag off);
/// guards that the flag-off path keeps working unchanged.
pub fn test_files_palette_eager_search() -> Builder {
    FeatureFlag::CommandPaletteFileSearch.set_enabled(true);
    FeatureFlag::StreamingFileSearch.set_enabled(false);
    files_palette_search_builder()
}
