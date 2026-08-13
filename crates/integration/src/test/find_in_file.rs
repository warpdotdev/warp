use regex::Regex;
use warp::cmd_or_ctrl_shift;
use warp::integration_testing::code_editor_find::{
    assert_find_query_editor_buffer_contains, assert_find_query_editor_is_editable_and_focused,
    assert_find_query_editor_is_selectable_not_editable, enable_vim_mode,
    find_query_editor_position_id, focus_code_editor_for_test,
};
use warp::integration_testing::step::new_step_with_default_assertions;
use warp::integration_testing::tab::assert_pane_title;
use warp::integration_testing::terminal::wait_until_bootstrapped_single_pane_for_tab;
use warp::integration_testing::view_getters::{pane_group_view, workspace_view};
use warp::workspace::WorkspaceAction;
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

/// Writes a small multi-word text file, opens it in the code editor via the file tree, and
/// enables Vim keybindings.
fn find_test_file_open_steps(builder: Builder) -> Builder {
    builder
        .with_setup(|utils| {
            let test_dir = utils.test_dir();
            let dir_string = test_dir
                .to_str()
                .expect("Should be able to convert test dir to str");
            write_all_rc_files_for_test(&test_dir, format!("cd {dir_string}"));

            std::fs::write(
                test_dir.join("find_test.txt"),
                "hello world\nhello universe\n",
            )
            .expect("Failed to create test file");
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Enable Vim keybindings")
                .with_action(|app, _, _| enable_vim_mode(app)),
        )
        .with_step(
            new_step_with_default_assertions("Open file tree panel")
                .with_action(|app, _, _| open_file_tree_panel(app)),
        )
        .with_step(
            new_step_with_default_assertions("Click on find_test.txt in file tree")
                .with_click_on_saved_position("file_tree_item:find_test.txt")
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
                assert_pane_title(0, 1, Regex::new(r"find_test\.txt$").unwrap()),
            ),
        )
        .with_step(
            new_step_with_default_assertions("Focus the code editor")
                .with_action(|app, window_id, _| focus_code_editor_for_test(app, window_id)),
        )
}

/// Regression test (Find in File click-to-focus): with Vim keybindings enabled, typing a query
/// and pressing Enter commits the query and returns focus to the editor, but must leave the
/// query field clickable. Clicking it (a real pointer click dispatched through the rendered
/// element tree, not a direct action call) must restore focus and editability.
pub fn test_vim_enter_leaves_find_query_editor_clickable() -> Builder {
    find_test_file_open_steps(new_builder())
        .with_step(
            new_step_with_default_assertions("Open Find and type a query")
                .with_keystrokes(&[cmd_or_ctrl_shift("f")])
                .with_typed_characters(&["hello"]),
        )
        .with_step(
            new_step_with_default_assertions(
                "Press Enter to commit the query; Vim returns focus to the editor",
            )
            .with_keystrokes(&["enter"])
            .add_assertion(assert_find_query_editor_is_selectable_not_editable()),
        )
        .with_step(
            new_step_with_default_assertions("Click the find query field")
                .with_click_on_saved_position_fn(find_query_editor_position_id)
                .add_assertion(assert_find_query_editor_is_editable_and_focused()),
        )
        .with_step(
            new_step_with_default_assertions(
                // The find editor selects all text on focus (`select_all_on_focus: true`),
                // mirroring the keyboard shortcut, so typing here replaces the query rather
                // than appending to it. Either way, the buffer changing proves the field is
                // genuinely editable, not just reporting itself as such.
                "Type again to prove the field actually accepts input after the click",
            )
            .with_typed_characters(&["!"])
            .add_assertion(assert_find_query_editor_buffer_contains("!")),
        )
}

/// Regression test (Find in File click-to-focus, Vim word search): a `*` word search populates
/// the query field and leaves it non-editable but clickable. Clicking it must restore focus and
/// editability, the same as after a Vim Enter commit.
pub fn test_vim_word_search_leaves_find_query_editor_clickable() -> Builder {
    find_test_file_open_steps(new_builder())
        .with_step(
            new_step_with_default_assertions(
                "Search for the word under the cursor with '*' (cursor starts on \"hello\")",
            )
            .with_typed_characters(&["*"])
            .add_assertion(assert_find_query_editor_is_selectable_not_editable()),
        )
        .with_step(
            new_step_with_default_assertions("Click the find query field")
                .with_click_on_saved_position_fn(find_query_editor_position_id)
                .add_assertion(assert_find_query_editor_is_editable_and_focused()),
        )
        .with_step(
            new_step_with_default_assertions(
                "Type again to prove the field actually accepts input after the click",
            )
            .with_typed_characters(&["!"])
            .add_assertion(assert_find_query_editor_buffer_contains("!")),
        )
}
