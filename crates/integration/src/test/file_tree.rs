use std::path::Path;
use std::time::Duration;

use command::blocking::Command;
use regex::Regex;
use repo_metadata::file_tree_store::FileTreeEntryState;
use repo_metadata::local_model::IndexedRepoState;
use repo_metadata::{RepoMetadataModel, RepositoryIdentifier};
use warp::features::FeatureFlag;
use warp::integration_testing::step::new_step_with_default_assertions;
use warp::integration_testing::tab::assert_pane_title;
use warp::integration_testing::terminal::wait_until_bootstrapped_single_pane_for_tab;
use warp::integration_testing::view_getters::{pane_group_view, workspace_view};
use warp::workspace::WorkspaceAction;
use warpui_core::integration::{AssertionCallback, TestStep};
use warpui_core::{async_assert, async_assert_eq, App, SingletonEntity};

use super::{new_builder, Builder};
use crate::util::write_all_rc_files_for_test;

fn open_file_tree_panel(app: &mut App) {
    let window_id = app.read(|ctx| {
        ctx.windows()
            .active_window()
            .expect("should have active window")
    });
    let workspace = workspace_view(app, window_id);
    app.update(|ctx| {
        ctx.dispatch_typed_action_for_view(
            window_id,
            workspace.id(),
            &WorkspaceAction::ToggleProjectExplorer,
        );
    });
}

/// Test that clicking a file in the file tree opens it in Warp's editor.
/// This is a regression test for the bug where files were being opened in
/// external editors instead of Warp's built-in editor.
pub fn test_file_tree_opens_files_in_warp() -> Builder {
    new_builder()
        .with_setup(|utils| {
            let test_dir = utils.test_dir();

            // Change to the test directory
            let dir_string = test_dir
                .to_str()
                .expect("Should be able to convert test dir to str");
            write_all_rc_files_for_test(&test_dir, format!("cd {dir_string}"));

            // Create a test file
            std::fs::write(test_dir.join("test_file.txt"), "Hello from test file!")
                .expect("Failed to create test file");

            // Create a test directory with a file inside
            std::fs::create_dir_all(test_dir.join("test_dir"))
                .expect("Failed to create test directory");
            std::fs::write(
                test_dir.join("test_dir/nested_file.rs"),
                "fn main() {\n    println!(\"Hello, world!\");\n}",
            )
            .expect("Failed to create nested file");
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Open file tree panel")
                .with_action(|app, _, _| open_file_tree_panel(app)),
        )
        // Click on test_file.txt in the file tree
        .with_step(
            new_step_with_default_assertions("Click on test_file.txt in file tree")
                .with_click_on_saved_position("file_tree_item:test_file.txt")
                .add_assertion(|app, window_id| {
                    // Verify that a new pane was opened with the file
                    let pane_group = pane_group_view(app, window_id, 0);
                    pane_group.read(app, |pane_group, _ctx| {
                        async_assert_eq!(
                            pane_group.pane_count(),
                            2,
                            "Expected 2 panes after opening file (terminal + editor)"
                        )
                    })
                }),
        )
        .with_step(
            new_step_with_default_assertions("Verify file opened in Warp editor").add_assertion(
                assert_pane_title(0, 1, Regex::new(r"test_file\.txt$").unwrap()),
            ),
        )
}

/// Test that the "Open in new pane" context menu action works correctly.
pub fn test_file_tree_open_in_new_pane() -> Builder {
    new_builder()
        .with_setup(|utils| {
            let test_dir = utils.test_dir();
            let dir_string = test_dir
                .to_str()
                .expect("Should be able to convert test dir to str");
            write_all_rc_files_for_test(&test_dir, format!("cd {dir_string}"));

            std::fs::write(
                test_dir.join("sample.md"),
                "# Sample Markdown\n\nThis is a test.",
            )
            .expect("Failed to create sample file");
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Open file tree panel")
                .with_action(|app, _, _| open_file_tree_panel(app)),
        )
        .with_step(
            new_step_with_default_assertions(
                "Right-click on sample.md and select 'Open in new pane'",
            )
            .with_right_click_on_saved_position("file_tree_item:sample.md")
            .with_click_on_saved_position("Open in new pane"),
        )
        .with_step(
            new_step_with_default_assertions("Verify file opened in new pane")
                .add_assertion(assert_pane_title(0, 1, Regex::new(r"sample\.md$").unwrap()))
                .add_assertion(|app, window_id| {
                    let pane_group = pane_group_view(app, window_id, 0);
                    pane_group.read(app, |pane_group, _ctx| {
                        async_assert_eq!(
                            pane_group.pane_count(),
                            2,
                            "Expected 2 panes after 'Open in new pane'"
                        )
                    })
                }),
        )
}

/// Test that the "Open in new tab" context menu action works correctly.
pub fn test_file_tree_open_in_new_tab() -> Builder {
    new_builder()
        .with_setup(|utils| {
            let test_dir = utils.test_dir();
            let dir_string = test_dir
                .to_str()
                .expect("Should be able to convert test dir to str");
            write_all_rc_files_for_test(&test_dir, format!("cd {dir_string}"));

            std::fs::write(test_dir.join("config.json"), "{\"key\": \"value\"}")
                .expect("Failed to create config file");
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Open file tree panel")
                .with_action(|app, _, _| open_file_tree_panel(app)),
        )
        .with_step(
            TestStep::new("Right-click on config.json and select 'Open in new tab'")
                .with_right_click_on_saved_position("file_tree_item:config.json")
                .with_click_on_saved_position("Open in new tab"),
        )
        .with_step(
            TestStep::new("Verify file opened in new tab")
                .add_assertion(|app, window_id| {
                    let workspace = workspace_view(app, window_id);
                    let tab_count = workspace.read(app, |workspace, _ctx| workspace.tab_count());
                    async_assert_eq!(tab_count, 2, "Expected 2 tabs after 'Open in new tab'")
                })
                .add_assertion(|app, window_id| {
                    let workspace = workspace_view(app, window_id);
                    let tab_count = workspace.read(app, |workspace, _ctx| workspace.tab_count());
                    let config_regex = Regex::new(r"config\.json$").unwrap();

                    let mut found = false;
                    for tab_index in 0..tab_count {
                        let pane_group = pane_group_view(app, window_id, tab_index);
                        let title = pane_group.read(app, |pane_group, ctx| {
                            pane_group.pane_by_index(0).map(|pane| {
                                pane.pane_configuration().as_ref(ctx).title().to_owned()
                            })
                        });

                        if let Some(title) = title {
                            if config_regex.is_match(&title) {
                                found = true;
                                break;
                            }
                        }
                    }

                    async_assert!(found, "Expected a tab with config.json opened")
                }),
        )
}

/// Test that keyboard navigation (arrow keys + enter) works to open files.
pub fn test_file_tree_keyboard_navigation() -> Builder {
    new_builder()
        .with_setup(|utils| {
            let test_dir = utils.test_dir();
            let dir_string = test_dir
                .to_str()
                .expect("Should be able to convert test dir to str");
            write_all_rc_files_for_test(&test_dir, format!("cd {dir_string}"));

            std::fs::create_dir_all(test_dir.join("src")).expect("Failed to create src directory");
            std::fs::write(test_dir.join("src/file_a.txt"), "File A")
                .expect("Failed to create file A");
            std::fs::write(test_dir.join("src/file_b.txt"), "File B")
                .expect("Failed to create file B");
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Open file tree panel")
                .with_action(|app, _, _| open_file_tree_panel(app)),
        )
        .with_step(
            new_step_with_default_assertions("Focus file tree")
                .with_click_on_saved_position("file_tree_item:src"),
        )
        .with_step(
            new_step_with_default_assertions("Navigate to a file and press Enter")
                .with_keystrokes(&["down", "enter"])
                .add_assertion(|app, window_id| {
                    let pane_group = pane_group_view(app, window_id, 0);
                    pane_group.read(app, |pane_group, _ctx| {
                        async_assert_eq!(
                            pane_group.pane_count(),
                            2,
                            "Expected 2 panes after opening file via keyboard"
                        )
                    })
                }),
        )
}

/// Test that non-text files (like images) do not crash when clicked.
/// They should either open in the system default app or show an error.
pub fn test_file_tree_non_openable_files() -> Builder {
    new_builder()
        .with_setup(|utils| {
            let test_dir = utils.test_dir();
            let dir_string = test_dir
                .to_str()
                .expect("Should be able to convert test dir to str");
            write_all_rc_files_for_test(&test_dir, format!("cd {dir_string}"));

            // Create a binary file that shouldn't be opened in Warp
            std::fs::write(test_dir.join("test.bin"), vec![0u8, 1, 2, 3, 255])
                .expect("Failed to create binary file");
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Open file tree panel")
                .with_action(|app, _, _| open_file_tree_panel(app)),
        )
        .with_step(
            new_step_with_default_assertions("Click on binary file")
                .with_click_on_saved_position("file_tree_item:test.bin")
                .add_assertion(|app, window_id| {
                    // The binary file should NOT open in a new pane in Warp
                    // It should fall back to system default behavior
                    let pane_group = pane_group_view(app, window_id, 0);
                    pane_group.read(app, |pane_group, _ctx| {
                        async_assert_eq!(
                            pane_group.pane_count(),
                            1,
                            "Binary file should not open in Warp, should stay at 1 pane"
                        )
                    })
                }),
        )
}

/// Test that expanding directories and then clicking files inside them works correctly.
pub fn test_file_tree_nested_file_opening() -> Builder {
    new_builder()
        .with_setup(|utils| {
            let test_dir = utils.test_dir();
            let dir_string = test_dir
                .to_str()
                .expect("Should be able to convert test dir to str");
            write_all_rc_files_for_test(&test_dir, format!("cd {dir_string}"));

            // Create nested directory structure
            std::fs::create_dir_all(test_dir.join("src/utils"))
                .expect("Failed to create nested directories");
            std::fs::write(
                test_dir.join("src/utils/helper.js"),
                "export function helper() { return 42; }",
            )
            .expect("Failed to create nested file");
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Open file tree panel")
                .with_action(|app, _, _| open_file_tree_panel(app)),
        )
        .with_step(
            new_step_with_default_assertions("Expand src directory")
                .with_click_on_saved_position("file_tree_item:src"),
        )
        .with_step(
            new_step_with_default_assertions("Expand utils directory")
                .with_click_on_saved_position("file_tree_item:utils"),
        )
        .with_step(
            new_step_with_default_assertions("Click on helper.js")
                .with_click_on_saved_position("file_tree_item:helper.js")
                .add_assertion(assert_pane_title(0, 1, Regex::new(r"helper\.js$").unwrap())),
        )
}

// ── Lazy file tree indexing (FeatureFlag::LazyFileTreeIndexing) ──────────

/// Env var used to hand the fixture repo's canonical path from the test setup
/// to assertion callbacks (repo metadata keys repositories by canonicalized
/// paths).
const LAZY_INDEXING_REPO_ENV_VAR: &str = "WARP_INTEGRATION_FILE_TREE_REPO_DIR";
const IGNORED_DIR_NAME: &str = "ignored_dir";

fn run_git(repo_dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(repo_dir)
        .status()
        .expect("git command should run");
    assert!(status.success(), "git {args:?} should succeed");
}

/// Returns the fixture repo's repository identifier, resolved from the
/// environment variable published by the test setup.
fn fixture_repo_id() -> RepositoryIdentifier {
    let repo_dir = std::env::var(LAZY_INDEXING_REPO_ENV_VAR)
        .expect("fixture repo env var should be set by the test setup");
    RepositoryIdentifier::try_local(Path::new(&repo_dir))
        .expect("fixture repo path should be a valid local repository identifier")
}

fn assert_fixture_repo_eagerly_indexed() -> AssertionCallback {
    Box::new(|app, _window_id| {
        app.read(|ctx| {
            let id = fixture_repo_id();
            let indexed = matches!(
                RepoMetadataModel::as_ref(ctx).repository_state(&id, ctx),
                Some(IndexedRepoState::Indexed(_))
            );
            async_assert!(
                indexed,
                "expected the fixture repository to be detected and eagerly indexed"
            )
        })
    })
}

/// Shared flow for the file tree indexing tests: cd into a git repo fixture,
/// open the file tree pane, and verify the tree's indexing mode, that the
/// gitignored directory is tagged as ignored, that first-level contents
/// render, and that expanding a directory loads its children so a nested file
/// can be opened.
///
/// When `expect_lazy_indexing` is set, the repo's eager index state is removed
/// before the pane opens (repo detection is left intact), so opening the pane
/// must trigger the pane-owned lazy indexing path.
fn file_tree_indexing_builder(expect_lazy_indexing: bool) -> Builder {
    let mut builder = new_builder()
        .with_setup(|utils| {
            let test_dir = utils.test_dir();
            let repo_dir = test_dir.join("repo");
            std::fs::create_dir_all(repo_dir.join("src")).expect("should create repo/src");
            let repo_dir_string = repo_dir
                .to_str()
                .expect("repo directory should be valid utf-8");
            write_all_rc_files_for_test(&test_dir, format!("cd {repo_dir_string}"));

            // Committed content: a nested source file plus a .gitignore that
            // ignores a directory.
            std::fs::write(repo_dir.join("src").join("main.rs"), "fn main() {}\n")
                .expect("should write nested source file");
            std::fs::write(
                repo_dir.join(".gitignore"),
                format!("/{IGNORED_DIR_NAME}/\n"),
            )
            .expect("should write .gitignore");
            run_git(&repo_dir, &["init", "-b", "main"]);
            run_git(&repo_dir, &["config", "user.email", "test@example.com"]);
            run_git(&repo_dir, &["config", "user.name", "Warp Integration Test"]);
            run_git(&repo_dir, &["add", "."]);
            run_git(&repo_dir, &["commit", "-m", "Initial commit"]);

            // A gitignored directory that must be tagged as ignored in the tree.
            std::fs::create_dir_all(repo_dir.join(IGNORED_DIR_NAME))
                .expect("should create ignored directory");
            std::fs::write(
                repo_dir.join(IGNORED_DIR_NAME).join("hidden.txt"),
                "hidden\n",
            )
            .expect("should write ignored file");

            let canonical_repo_dir =
                dunce::canonicalize(&repo_dir).expect("should canonicalize the fixture repo path");
            utils.set_env(
                LAZY_INDEXING_REPO_ENV_VAR,
                Some(canonical_repo_dir.to_string_lossy().to_string()),
            );
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            // The eager index still runs on cd during this stage; wait for it
            // to finish so both variants start from the same deterministic
            // state.
            TestStep::new("Wait for the repo to be detected and eagerly indexed")
                .set_timeout(Duration::from_secs(20))
                .add_assertion(assert_fixture_repo_eagerly_indexed()),
        );

    if expect_lazy_indexing {
        builder = builder.with_step(
            new_step_with_default_assertions(
                "Remove the repo's eager index, simulating on-the-fly mode",
            )
            .with_action(|app, _, _| {
                app.update(|ctx| {
                    let id = fixture_repo_id();
                    RepoMetadataModel::handle(ctx).update(ctx, |model, ctx| {
                        model
                            .remove_repository(&id, ctx)
                            .expect("should remove the fixture repo's index state");
                    });
                });
            })
            .add_named_assertion("eager index state is gone", |app, _window_id| {
                app.read(|ctx| {
                    let id = fixture_repo_id();
                    async_assert!(
                        RepoMetadataModel::as_ref(ctx)
                            .repository_state(&id, ctx)
                            .is_none(),
                        "expected the fixture repo to have no index state after removal"
                    )
                })
            }),
        );
    }

    builder
        .with_step(
            new_step_with_default_assertions("Open file tree panel")
                .with_action(|app, _, _| open_file_tree_panel(app)),
        )
        .with_step(
            TestStep::new("Verify the tree's indexing mode and gitignore tagging")
                .set_timeout(Duration::from_secs(20))
                .add_named_assertion(
                    "indexing mode matches the feature flag",
                    move |app, _window_id| {
                        app.read(|ctx| {
                            let id = fixture_repo_id();
                            let RepositoryIdentifier::Local(repo_root) = &id else {
                                unreachable!("fixture repo id is always local");
                            };
                            let repo_metadata = RepoMetadataModel::as_ref(ctx);
                            let is_lazy = repo_metadata.is_lazy_loaded_path(repo_root, ctx);
                            // The lazy path materializes only the first level
                            // until a directory is expanded; the eager path
                            // serves the fully-built tree.
                            let nested_file = repo_root.join("src/main.rs");
                            let nested_file_materialized = matches!(
                                repo_metadata.repository_state(&id, ctx),
                                Some(IndexedRepoState::Indexed(state))
                                    if state.entry.contains(&nested_file)
                            );
                            async_assert!(
                                is_lazy == expect_lazy_indexing
                                    && nested_file_materialized != expect_lazy_indexing,
                                "expected lazy indexing = {expect_lazy_indexing}; got \
                                 is_lazy_loaded_path = {is_lazy}, nested file materialized \
                                 before expansion = {nested_file_materialized}"
                            )
                        })
                    },
                )
                .add_named_assertion(
                    "gitignored directory is tagged as ignored",
                    |app, _window_id| {
                        app.read(|ctx| {
                            let id = fixture_repo_id();
                            let RepositoryIdentifier::Local(repo_root) = &id else {
                                unreachable!("fixture repo id is always local");
                            };
                            let ignored_dir = repo_root.join(IGNORED_DIR_NAME);
                            let tagged = matches!(
                                RepoMetadataModel::as_ref(ctx).repository_state(&id, ctx),
                                Some(IndexedRepoState::Indexed(state))
                                    if matches!(
                                        state.entry.get(&ignored_dir),
                                        Some(FileTreeEntryState::Directory(dir)) if dir.ignored
                                    )
                            );
                            async_assert!(
                                tagged,
                                "expected {IGNORED_DIR_NAME} to be present in the tree and \
                                 tagged as gitignored"
                            )
                        })
                    },
                ),
        )
        .with_step(
            // Clicking the directory header requires it to be rendered in the
            // tree, which covers the first-level contents; expanding loads its
            // children on demand on the lazy path.
            new_step_with_default_assertions("Expand the src directory")
                .with_click_on_saved_position("file_tree_item:src"),
        )
        .with_step(
            new_step_with_default_assertions("Open the nested file loaded by expansion")
                .with_click_on_saved_position("file_tree_item:main.rs")
                .add_assertion(assert_pane_title(0, 1, Regex::new(r"main\.rs$").unwrap())),
        )
}

/// File tree served by pane-triggered lazy indexing
/// (`FeatureFlag::LazyFileTreeIndexing` on) with the eager index absent.
pub fn test_file_tree_lazy_indexing() -> Builder {
    FeatureFlag::LazyFileTreeIndexing.set_enabled(true);
    file_tree_indexing_builder(true)
}

/// File tree served by the eager repo index (lazy indexing flag off); guards
/// that the flag-off path keeps working unchanged.
pub fn test_file_tree_eager_indexing() -> Builder {
    FeatureFlag::LazyFileTreeIndexing.set_enabled(false);
    file_tree_indexing_builder(false)
}
