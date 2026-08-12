use regex::Regex;
use warp::features::FeatureFlag;
use warp::integration_testing::code_editor_find::{
    FIND_QUERY_SAVE_POSITION_ID, assert_find_query_editable, assert_find_query_focused,
    assert_find_query_text, assert_main_editor_focused, open_code_editor_find,
    set_vim_mode_enabled,
};
use warp::integration_testing::step::new_step_with_default_assertions;
use warp::integration_testing::tab::assert_pane_title;
use warp::integration_testing::terminal::wait_until_bootstrapped_single_pane_for_tab;
use warp::integration_testing::view_getters::{pane_group_view, workspace_view};
use warp::workspace::WorkspaceAction;
use warpui_core::async_assert_eq;

use super::{Builder, new_builder};
use crate::util::write_all_rc_files_for_test;

fn open_file_tree_panel(app: &mut warpui_core::App) {
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

fn file_open_steps(builder: Builder) -> Builder {
    builder
        .with_setup(|utils| {
            let test_dir = utils.test_dir();
            let dir_string = test_dir
                .to_str()
                .expect("Should be able to convert test dir to str");
            write_all_rc_files_for_test(&test_dir, format!("cd {dir_string}"));

            std::fs::write(
                test_dir.join("code_editor_find_test.txt"),
                "hello world\nsecond line\nthird line\n",
            )
            .expect("Failed to create test file");
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Open file tree panel")
                .with_action(|app, _, _| open_file_tree_panel(app)),
        )
        .with_step(
            new_step_with_default_assertions("Click on code_editor_find_test.txt in file tree")
                .with_click_on_saved_position("file_tree_item:code_editor_find_test.txt")
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
                assert_pane_title(0, 1, Regex::new(r"code_editor_find_test\.txt$").unwrap()),
            ),
        )
}

/// Regression test for a bug where, after pressing Enter in the find query field while Vim mode
/// was enabled (which intentionally moves focus back to the main editor and disables the query
/// field), clicking on the query field again had no effect: the field's own mouse handling
/// declines events while non-editable, and nothing above it caught the click. Verifies that a
/// click on the query field always re-enables and re-focuses it.
pub fn test_code_editor_find_click_refocuses_query_after_vim_enter() -> Builder {
    FeatureFlag::VimCodeEditor.set_enabled(true);

    file_open_steps(new_builder())
        .with_step(
            new_step_with_default_assertions("Enable Vim mode")
                .with_action(|app, _, _| set_vim_mode_enabled(app, true)),
        )
        .with_step(
            new_step_with_default_assertions("Open the code editor find bar")
                .with_action(|app, window_id, _| open_code_editor_find(app, window_id))
                .add_assertion(assert_find_query_focused(true))
                .add_assertion(assert_find_query_editable(true)),
        )
        .with_step(
            new_step_with_default_assertions("Type a search query")
                .with_typed_characters(&["hello"])
                .add_assertion(assert_find_query_text("hello".to_string())),
        )
        .with_step(
            new_step_with_default_assertions(
                "Press enter: Vim mode should disable the query field and focus the main editor",
            )
            .with_keystrokes(&["enter"])
            .add_assertion(assert_main_editor_focused(true))
            .add_assertion(assert_find_query_editable(false)),
        )
        .with_step(
            new_step_with_default_assertions(
                "Click the query field: it should become focused and editable again",
            )
            .with_click_on_saved_position(FIND_QUERY_SAVE_POSITION_ID)
            .add_assertion(assert_find_query_focused(true))
            .add_assertion(assert_find_query_editable(true)),
        )
        .with_step(
            // The query field selects all of its text on focus (matching its behavior when
            // reopened via cmd-f), so typing now replaces "hello" with the new characters.
            new_step_with_default_assertions("Typing should now update the query")
                .with_typed_characters(&["world"])
                .add_assertion(assert_find_query_text("world".to_string())),
        )
}
