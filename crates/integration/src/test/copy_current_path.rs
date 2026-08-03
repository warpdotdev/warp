use regex::Regex;
use warp::integration_testing::clipboard::assert_clipboard_contains_string;
use warp::integration_testing::command_palette::open_command_palette_and_run_action;
use warp::integration_testing::step::new_step_with_default_assertions;
use warp::integration_testing::tab::assert_pane_title;
use warp::integration_testing::terminal::wait_until_bootstrapped_single_pane_for_tab;
use warp::integration_testing::view_getters::{pane_group_view, workspace_view};
use warp::workspace::WorkspaceAction;
use warpui_core::integration::{AssertionOutcome, TestStep};
use warpui_core::{App, async_assert_eq};

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

/// Running the "Copy current path" command-palette action while a regular terminal is focused
/// copies the focused session's working directory to the clipboard.
///
/// The file-viewer branch of `PaneGroup::path_from_focused_pane` (copying the open file's
/// `display_path()`) shares this same dispatch path and reuses the accessors covered by the
/// file-viewer context-menu unit tests, so it is not re-exercised end-to-end here.
pub fn test_copy_current_path_copies_terminal_pwd() -> Builder {
    new_builder()
        .with_setup(|utils| {
            let test_dir = utils.test_dir();
            let dir_string = test_dir
                .to_str()
                .expect("Should be able to convert test dir to str");
            // Start the shell in a known directory so the focused session has a stable pwd.
            write_all_rc_files_for_test(&test_dir, format!("cd {dir_string}"));
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_steps(open_command_palette_and_run_action("Copy current path"))
        .with_step(
            TestStep::new("Clipboard contains the focused terminal pwd").add_assertion(
                |app, window_id| {
                    let pane_group = pane_group_view(app, window_id, 0);
                    let expected = pane_group.read(app, |pane_group, ctx| {
                        pane_group
                            .focused_session_view(ctx)
                            .and_then(|terminal_view| terminal_view.as_ref(ctx).pwd())
                    });
                    let Some(expected) = expected else {
                        return AssertionOutcome::failure(
                            "focused terminal pwd not yet available".to_string(),
                        );
                    };
                    let mut assert_clipboard = assert_clipboard_contains_string(expected);
                    assert_clipboard(app, window_id)
                },
            ),
        )
}

/// Running "Copy current path" while a code editor pane is focused copies the active tab's
/// file path. Regression coverage for issue #14518.
pub fn test_copy_current_path_copies_code_editor_file_path() -> Builder {
    new_builder()
        .with_setup(|utils| {
            let test_dir = utils.test_dir();
            let dir_string = test_dir
                .to_str()
                .expect("Should be able to convert test dir to str");
            write_all_rc_files_for_test(&test_dir, format!("cd {dir_string}"));

            std::fs::write(test_dir.join("copy_path_test.txt"), "path copy regression")
                .expect("Failed to create test file");
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Open file tree panel")
                .with_action(|app, _, _| open_file_tree_panel(app)),
        )
        .with_step(
            new_step_with_default_assertions("Click on copy_path_test.txt in file tree")
                .with_click_on_saved_position("file_tree_item:copy_path_test.txt")
                .add_assertion(|app, window_id| {
                    let pane_group = pane_group_view(app, window_id, 0);
                    pane_group.read(app, |pane_group, _ctx| {
                        async_assert_eq!(
                            pane_group.pane_count(),
                            2,
                            "Expected 2 panes after opening file"
                        )
                    })
                }),
        )
        .with_step(
            new_step_with_default_assertions("Verify file opened in editor").add_assertion(
                assert_pane_title(0, 1, Regex::new(r"copy_path_test\.txt$").unwrap()),
            ),
        )
        .with_steps(open_command_palette_and_run_action("Copy current path"))
        .with_step(
            TestStep::new("Clipboard contains the focused code editor file path").add_assertion(
                |app, window_id| {
                    let pane_group = pane_group_view(app, window_id, 0);
                    // Read the expected path from the active code tab location, not from
                    // `path_from_focused_pane`, so a regression there fails this assertion.
                    let expected = pane_group.read(app, |pane_group, ctx| {
                        pane_group
                            .code_views(ctx)
                            .into_iter()
                            .find_map(|code_view| {
                                let code_view = code_view.as_ref(ctx);
                                code_view
                                    .tab_at(code_view.active_tab_index())
                                    .and_then(|tab| tab.location())
                                    .map(|path| path.display_path())
                            })
                    });
                    let Some(expected) = expected else {
                        return AssertionOutcome::failure(
                            "code editor active tab path not yet available".to_string(),
                        );
                    };
                    if !expected.ends_with("copy_path_test.txt") {
                        return AssertionOutcome::failure(format!(
                            "expected code editor path ending in copy_path_test.txt, got {expected}"
                        ));
                    }
                    let mut assert_clipboard = assert_clipboard_contains_string(expected);
                    assert_clipboard(app, window_id)
                },
            ),
        )
}
