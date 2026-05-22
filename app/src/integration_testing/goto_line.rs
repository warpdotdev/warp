use settings::Setting as _;
use warp_editor::content::buffer::ToBufferPoint;
use warpui::integration::AssertionCallback;
use warpui::{async_assert, async_assert_eq, App, SingletonEntity, ViewHandle, WindowId};

use crate::code::editor::goto_line::view::GoToLineView;
use crate::code::editor::view::CodeEditorView;
use crate::settings::{AppEditorSettings, CodeEditorLineNumberMode};

fn file_code_editor_view(app: &App, window_id: WindowId) -> ViewHandle<CodeEditorView> {
    let views = app
        .views_of_type::<CodeEditorView>(window_id)
        .expect("should have CodeEditorView");
    views
        .iter()
        .find(|v| {
            v.read(app, |editor, ctx| {
                editor.model.as_ref(ctx).line_count(ctx) > 1
            })
        })
        .cloned()
        .unwrap_or_else(|| {
            views
                .first()
                .expect("should have at least one CodeEditorView")
                .clone()
        })
}

pub fn open_goto_line_dialog(app: &mut App, window_id: WindowId) {
    let editor = file_code_editor_view(app, window_id);
    editor.update(app, |view, ctx| {
        view.open_goto_line_for_test(ctx);
    });
}

pub fn goto_line_confirm(app: &mut App, window_id: WindowId, input: &str) {
    let editor = file_code_editor_view(app, window_id);
    let input_owned = input.to_string();
    editor.update(app, |view, ctx| {
        view.goto_line_confirm_for_test(&input_owned, ctx);
    });
}
pub fn set_code_editor_line_number_mode(app: &mut App, mode: CodeEditorLineNumberMode) {
    app.update(|ctx| {
        AppEditorSettings::handle(ctx).update(ctx, |settings, ctx| {
            settings
                .code_editor_line_number_mode
                .set_value(mode, ctx)
                .expect("failed to serialize CodeEditorLineNumberModeSetting");
            ctx.notify();
        });
    });
}

/// Asserts code editor line numbers with `(logical_line_number, expected_displayed_line_number)` pairs.
pub fn assert_code_editor_line_numbers(expected: Vec<(usize, usize)>) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let editor = file_code_editor_view(app, window_id);
        let actual = editor.read(app, |editor, ctx| {
            expected
                .iter()
                .map(|(line, _)| {
                    (
                        *line,
                        editor
                            .displayed_line_number_for_test(*line, ctx)
                            .expect("line numbers should be enabled for file code editor"),
                    )
                })
                .collect::<Vec<_>>()
        });

        async_assert_eq!(
            actual,
            expected,
            "Expected code editor line numbers to match"
        )
    })
}

pub fn assert_goto_line_dialog_is_open(expected: bool) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let views = app.views_of_type::<GoToLineView>(window_id);
        let Some(views) = views else {
            return async_assert!(
                !expected,
                "No GoToLineView found but expected open={expected}"
            );
        };
        let is_open = views
            .iter()
            .any(|v| v.read(app, |view, _ctx| view.is_open()));
        async_assert_eq!(
            is_open,
            expected,
            "Expected GoToLineView is_open={expected}, got {is_open}"
        )
    })
}

pub fn assert_cursor_at_line(expected_line: usize) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let editor = file_code_editor_view(app, window_id);
        let (cursor_row, raw_row) = editor.read(app, |editor, ctx| {
            let selection_model = editor.model.as_ref(ctx).buffer_selection_model();
            let head = selection_model.as_ref(ctx).first_selection_head();
            let buffer = editor.model.as_ref(ctx).buffer().as_ref(ctx);
            let point = head.to_buffer_point(buffer);
            (point.row as usize, point.row)
        });
        async_assert_eq!(
            cursor_row,
            expected_line,
            "Expected cursor at line {expected_line}, got raw_row={raw_row} (cursor_row={cursor_row})"
        )
    })
}

pub fn assert_cursor_at_line_and_column(
    expected_line: usize,
    expected_column: usize,
) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let editor = file_code_editor_view(app, window_id);
        let (cursor_row, cursor_col) = editor.read(app, |editor, ctx| {
            let selection_model = editor.model.as_ref(ctx).buffer_selection_model();
            let head = selection_model.as_ref(ctx).first_selection_head();
            let buffer = editor.model.as_ref(ctx).buffer().as_ref(ctx);
            let point = head.to_buffer_point(buffer);
            (point.row as usize, point.column as usize)
        });
        let line_match = cursor_row == expected_line;
        let col_match = cursor_col == expected_column;
        async_assert!(
            line_match && col_match,
            "Expected cursor at line {expected_line} col {expected_column}, got line {cursor_row} col {cursor_col}",
        )
    })
}
