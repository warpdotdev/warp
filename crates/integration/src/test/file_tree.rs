use regex::Regex;
use warp::integration_testing::step::new_step_with_default_assertions;
use warp::integration_testing::tab::assert_pane_title;
use warp::integration_testing::terminal::wait_until_bootstrapped_single_pane_for_tab;
use warp::integration_testing::view_getters::{pane_group_view, workspace_view};
use warp::workspace::WorkspaceAction;
use warpui_core::integration::TestStep;
use warpui_core::{App, async_assert, async_assert_eq};

use super::{Builder, new_builder};
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

                        if let Some(title) = title
                            && config_regex.is_match(&title)
                        {
                            found = true;
                            break;
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
