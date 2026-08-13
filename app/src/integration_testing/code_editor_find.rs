use settings::Setting as _;
use warpui::integration::AssertionCallback;
use warpui::{App, SingletonEntity, ViewHandle, WindowId, async_assert, async_assert_eq};

use crate::code::editor::find::view::CodeEditorFind;
use crate::code::editor::view::CodeEditorView;
use crate::editor::{EditorView, InteractionState};
use crate::settings::AppEditorSettings;

/// Finds the code editor view containing the most lines (i.e. the file most likely opened by the
/// test), falling back to the first code editor view found.
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

fn find_bar_view(app: &App, window_id: WindowId) -> ViewHandle<CodeEditorFind> {
    file_code_editor_view(app, window_id)
        .read(app, |editor, _ctx| editor.find_bar_for_test())
        .expect("code editor should have a find bar")
}

/// Focuses the code editor directly, so that subsequent keystrokes (e.g. the Find shortcut) are
/// dispatched to it regardless of what has focus after opening the file.
pub fn focus_code_editor_for_test(app: &mut App, window_id: WindowId) {
    let editor = file_code_editor_view(app, window_id);
    editor.update(app, |editor, ctx| editor.focus(ctx));
}

/// Returns a handle to the find query editor of the code editor's find bar.
pub fn find_query_editor_view(app: &App, window_id: WindowId) -> ViewHandle<EditorView> {
    find_bar_view(app, window_id).read(app, |find_bar, _ctx| find_bar.find_editor_for_test())
}

/// Returns the saved-position id for the find query editor, so a test can click on it to
/// simulate a real pointer click through the rendered element tree.
pub fn find_query_editor_position_id(app: &mut App, window_id: WindowId) -> String {
    find_bar_view(app, window_id).read(app, |find_bar, _ctx| {
        find_bar.find_editor_position_id_for_test().to_string()
    })
}

/// Enables Vim keybindings in the code editor, for tests that exercise Vim-specific behavior.
pub fn enable_vim_mode(app: &mut App) {
    app.update(|ctx| {
        AppEditorSettings::handle(ctx).update(ctx, |settings, ctx| {
            settings
                .vim_mode
                .set_value(true, ctx)
                .expect("failed to enable vim mode");
        });
    });
}

/// Asserts that the find query editor is both `Editable` and focused, i.e. that a click
/// successfully restored the field's editability (see `EditorAction::Focus`).
pub fn assert_find_query_editor_is_editable_and_focused() -> AssertionCallback {
    Box::new(move |app, window_id| {
        let editor = find_query_editor_view(app, window_id);
        let (interaction_state, is_focused) = editor.read(app, |editor, ctx| {
            (editor.interaction_state(ctx), editor.is_focused())
        });
        async_assert!(
            interaction_state == InteractionState::Editable && is_focused,
            "Expected find query editor to be Editable and focused, got interaction_state={:?} is_focused={}",
            interaction_state,
            is_focused
        )
    })
}

/// Asserts that the find query editor is `Selectable` (clickable, but not yet editable) — the
/// state it should be in immediately after a Vim commit (Enter, or a `*`/`#` word search), before
/// the user clicks back into it.
pub fn assert_find_query_editor_is_selectable_not_editable() -> AssertionCallback {
    Box::new(move |app, window_id| {
        let editor = find_query_editor_view(app, window_id);
        let interaction_state = editor.read(app, |editor, ctx| editor.interaction_state(ctx));
        async_assert_eq!(
            interaction_state,
            InteractionState::Selectable,
            "Expected find query editor to be Selectable (clickable but not editable) after a Vim commit"
        )
    })
}

/// Asserts that the find query editor's buffer contains the given substring, proving that typed
/// input actually reaches the field (not just that it reports itself as editable/focused).
pub fn assert_find_query_editor_buffer_contains(
    expected_substring: &'static str,
) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let editor = find_query_editor_view(app, window_id);
        let text = editor.read(app, |editor, ctx| editor.buffer_text(ctx));
        async_assert!(
            text.contains(expected_substring),
            "Expected find query editor buffer to contain {:?}, got {:?}",
            expected_substring,
            text
        )
    })
}
