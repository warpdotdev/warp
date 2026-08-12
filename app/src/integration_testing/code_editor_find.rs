use settings::Setting as _;
use warpui::integration::AssertionCallback;
use warpui::{App, SingletonEntity, ViewHandle, WindowId, async_assert_eq};

use crate::code::editor::find::view::CodeEditorFind;
pub use crate::code::editor::find::view::FIND_QUERY_SAVE_POSITION_ID;
use crate::code::editor::view::CodeEditorView;
use crate::settings::AppEditorSettings;

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

fn code_editor_find_view(app: &App, window_id: WindowId) -> ViewHandle<CodeEditorFind> {
    let editor = file_code_editor_view(app, window_id);
    editor
        .read(app, |editor, _ctx| editor.find_bar_for_test())
        .expect("code editor should have a find bar")
}

/// Opens the find bar for the code editor under test, bypassing the `cmd-f` keybinding (which
/// requires the `CodeFindReplace` feature flag and the editor's `FindBarAvailable` context).
pub fn open_code_editor_find(app: &mut App, window_id: WindowId) {
    let editor = file_code_editor_view(app, window_id);
    editor.update(app, |view, ctx| {
        view.open_find_bar_for_test(ctx);
    });
}

/// Enables or disables the Vim mode user setting.
pub fn set_vim_mode_enabled(app: &mut App, enabled: bool) {
    app.update(|ctx| {
        AppEditorSettings::handle(ctx).update(ctx, |settings, ctx| {
            settings
                .vim_mode
                .set_value(enabled, ctx)
                .expect("failed to serialize VimModeEnabled");
            ctx.notify();
        });
    });
}

pub fn assert_main_editor_focused(expected: bool) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let editor = file_code_editor_view(app, window_id);
        let is_focused = editor.read(app, |_editor, ctx| editor.is_focused(ctx));
        async_assert_eq!(
            is_focused,
            expected,
            "Expected main code editor focused={expected}, got {is_focused}"
        )
    })
}

pub fn assert_find_query_editable(expected: bool) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let find_view = code_editor_find_view(app, window_id);
        let is_editable = find_view.read(app, |find, ctx| find.is_find_input_editable(ctx));
        async_assert_eq!(
            is_editable,
            expected,
            "Expected find query editor editable={expected}, got {is_editable}"
        )
    })
}

pub fn assert_find_query_focused(expected: bool) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let find_view = code_editor_find_view(app, window_id);
        let is_focused = find_view.read(app, |find, ctx| find.is_find_input_focused(ctx));
        async_assert_eq!(
            is_focused,
            expected,
            "Expected find query editor focused={expected}, got {is_focused}"
        )
    })
}

pub fn assert_find_query_text(expected: String) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let find_view = code_editor_find_view(app, window_id);
        let actual = find_view.read(app, |find, ctx| find.find_query_text(ctx));
        async_assert_eq!(
            actual,
            expected,
            "Expected find query text {expected:?}, got {actual:?}"
        )
    })
}
