use regex::Regex;
use warp::integration_testing::goto_line::{
    assert_code_editor_line_numbers, assert_cursor_at_line, assert_cursor_at_line_and_column,
    assert_goto_line_dialog_is_open, goto_line_confirm, open_goto_line_dialog,
    set_code_editor_line_number_mode,
};
use warp::integration_testing::step::new_step_with_default_assertions;
use warp::integration_testing::tab::assert_pane_title;
use warp::integration_testing::terminal::wait_until_bootstrapped_single_pane_for_tab;
use warp::integration_testing::view_getters::{pane_group_view, workspace_view};
use warp::settings::CodeEditorLineNumberMode;
use warp::workspace::WorkspaceAction;
use warpui_core::{async_assert_eq, App};

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

fn create_multiline_test_file_content() -> String {
    (1..=20)
        .map(|i| format!("line {i} content"))
        .collect::<Vec<_>>()
        .join("\n")
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
                test_dir.join("goto_test.txt"),
                create_multiline_test_file_content(),
            )
            .expect("Failed to create test file");
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Open file tree panel")
                .with_action(|app, _, _| open_file_tree_panel(app)),
        )
        .with_step(
            new_step_with_default_assertions("Click on goto_test.txt in file tree")
                .with_click_on_saved_position("file_tree_item:goto_test.txt")
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
                assert_pane_title(0, 1, Regex::new(r"goto_test\.txt$").unwrap()),
            ),
        )
}

pub fn test_goto_line_dialog_open_close() -> Builder {
    file_open_steps(new_builder())
        .with_step(
            new_step_with_default_assertions("Open Go to Line dialog")
                .with_action(|app, window_id, _| open_goto_line_dialog(app, window_id))
                .add_assertion(assert_goto_line_dialog_is_open(true)),
        )
        .with_step(
            new_step_with_default_assertions("Close Go to Line dialog with escape")
                .with_keystrokes(&["escape"])
                .add_assertion(assert_goto_line_dialog_is_open(false)),
        )
}

pub fn test_goto_line_jumps_to_line() -> Builder {
    file_open_steps(new_builder())
        .with_step(
            new_step_with_default_assertions("Open Go to Line dialog")
                .with_action(|app, window_id, _| open_goto_line_dialog(app, window_id))
                .add_assertion(assert_goto_line_dialog_is_open(true)),
        )
        .with_step(
            new_step_with_default_assertions("Type line number and confirm")
                .with_typed_characters(&["10"])
                .with_keystrokes(&["enter"]),
        )
        .with_step(
            new_step_with_default_assertions("Verify cursor at line 10")
                .add_assertion(assert_goto_line_dialog_is_open(false))
                .add_assertion(assert_cursor_at_line(10)),
        )
}

pub fn test_goto_line_with_column() -> Builder {
    file_open_steps(new_builder())
        .with_step(
            new_step_with_default_assertions("Go to line 5, column 3")
                .with_action(|app, window_id, _| goto_line_confirm(app, window_id, "5:3")),
        )
        .with_step(
            new_step_with_default_assertions("Verify cursor at line 5, column 3")
                .add_assertion(assert_goto_line_dialog_is_open(false))
                .add_assertion(assert_cursor_at_line_and_column(5, 3)),
        )
}

pub fn test_goto_line_clamps_out_of_range() -> Builder {
    file_open_steps(new_builder())
        .with_step(
            new_step_with_default_assertions("Go to line 999 (beyond file)")
                .with_action(|app, window_id, _| goto_line_confirm(app, window_id, "999")),
        )
        .with_step(
            new_step_with_default_assertions("Verify cursor clamped to last line")
                .add_assertion(assert_goto_line_dialog_is_open(false))
                .add_assertion(assert_cursor_at_line(20)),
        )
}

pub fn test_code_editor_line_numbers_default_to_absolute() -> Builder {
    file_open_steps(new_builder()).with_step(
        new_step_with_default_assertions("Verify code editor line numbers are absolute by default")
            .add_assertion(assert_code_editor_line_numbers(vec![
                (1, 1),
                (10, 10),
                (20, 20),
            ])),
    )
}

pub fn test_code_editor_relative_line_numbers_follow_cursor() -> Builder {
    file_open_steps(new_builder())
        .with_step(
            new_step_with_default_assertions("Enable relative line numbers and move to line 10")
                .with_action(|app, window_id, _| {
                    set_code_editor_line_number_mode(app, CodeEditorLineNumberMode::Relative);
                    goto_line_confirm(app, window_id, "10");
                })
                .add_assertion(assert_goto_line_dialog_is_open(false))
                .add_assertion(assert_cursor_at_line(10))
                .add_assertion(assert_code_editor_line_numbers(vec![
                    (5, 5),
                    (9, 1),
                    (10, 10),
                    (11, 1),
                    (20, 10),
                ])),
        )
        .with_step(
            new_step_with_default_assertions("Verify relative line numbers update on cursor move")
                .with_action(|app, window_id, _| goto_line_confirm(app, window_id, "12"))
                .add_assertion(assert_cursor_at_line(12))
                .add_assertion(assert_code_editor_line_numbers(vec![
                    (5, 7),
                    (10, 2),
                    (12, 12),
                    (13, 1),
                    (20, 8),
                ])),
        )
}
