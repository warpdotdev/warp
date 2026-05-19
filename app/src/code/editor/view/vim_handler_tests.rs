use crate::auth::AuthStateProvider;
use crate::cloud_object::model::persistence::CloudModel;
use crate::code::editor::view::CodeEditorRenderOptions;
use crate::notebooks::editor::keys::NotebookKeybindings;
use crate::workspace::ActiveSession;
use crate::{
    code::editor::view::{CodeEditorView, CodeEditorViewAction},
    server::server_api::{team::MockTeamClient, workspace::MockWorkspaceClient},
    settings::AppEditorSettings,
    settings_view::keybindings::KeybindingChangedNotifier,
    test_util::settings::initialize_settings_for_tests,
    vim_registers::VimRegisters,
    workspace::sync_inputs::SyncedInputState,
    workspaces::user_workspaces::UserWorkspaces,
};
use pathfinder_geometry::vector::Vector2F;
use std::sync::Arc;
use unindent::Unindent;
use vim::vim::{MotionType, VimMode};
use warp_core::{features::FeatureFlag, settings::Setting, ui::appearance::Appearance};
use warp_editor::model::CoreEditorModel;
use warp_editor::render::model::viewport::SizeInfo;
use warp_editor::{
    content::buffer::{InitialBufferState, ToBufferCharOffset, ToBufferPoint},
    render::element::VerticalExpansionBehavior,
};
use warp_util::user_input::UserInput;
use warpui::text::point::Point;
use warpui::units::IntoPixels;
use warpui::{
    keymap::Keystroke, platform::WindowStyle, App, SingletonEntity, TypedActionView, UpdateModel,
    ViewHandle,
};

// Await render/layout completion for a CodeEditorView in tests.
async fn layout_editor_view(app: &mut App, editor: &ViewHandle<CodeEditorView>) {
    app.read(|ctx| {
        let model = editor.as_ref(ctx).model.as_ref(ctx);
        model.render_state().as_ref(ctx).layout_complete()
    })
    .await;
}

/// Helper function to initialize all required singleton models for CodeEditorView tests.
fn initialize_code_editor_app(app: &mut App) {
    initialize_settings_for_tests(app);

    // Add required singleton models for CodeEditorView
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(|_| Appearance::mock());
    app.add_singleton_model(|_| SyncedInputState::mock());
    app.add_singleton_model(|_| VimRegisters::new());
    app.add_singleton_model(|_| KeybindingChangedNotifier::mock());
    #[cfg(feature = "voice_input")]
    app.add_singleton_model(voice_input::VoiceInput::new);

    // Add mocks required by rich text editor (used in the CommentEditor)
    app.add_singleton_model(CloudModel::mock);
    app.add_singleton_model(|_| ActiveSession::default());
    app.add_singleton_model(NotebookKeybindings::new);

    // Add UserWorkspaces mock (required by CodeEditorView)
    let team_client_mock = Arc::new(MockTeamClient::new());
    let workspace_client_mock = Arc::new(MockWorkspaceClient::new());
    app.add_singleton_model(|ctx| {
        UserWorkspaces::mock(
            team_client_mock.clone(),
            workspace_client_mock.clone(),
            vec![],
            ctx,
        )
    });

    // Enable vim mode in editor settings
    app.update_model(
        &AppEditorSettings::handle(app),
        |settings: &mut AppEditorSettings, ctx| {
            settings.vim_mode.set_value(true, ctx).unwrap();
        },
    );
}

/// Helper function for creating a code editor with buffer text.
fn add_code_editor(buffer_content: &str, app: &mut App) -> ViewHandle<CodeEditorView> {
    let (_, editor) = app.add_window(WindowStyle::NotStealFocus, move |ctx| {
        let mut editor = CodeEditorView::new(
            None,
            None,
            CodeEditorRenderOptions::new(VerticalExpansionBehavior::GrowToMaxHeight),
            ctx,
        );

        let base_text = buffer_content.unindent();
        editor.reset(InitialBufferState::plain_text(base_text.as_str()), ctx);

        editor.handle_action(&CodeEditorViewAction::CursorAtBufferStart, ctx);

        editor
    });
    editor
}

/// Helper function for simulating vim user input.
fn vim_user_insert(editor: &ViewHandle<CodeEditorView>, text: &str, app: &mut App) {
    editor.update(app, |view, ctx| {
        view.handle_action(
            &CodeEditorViewAction::VimUserTyped(UserInput::new(text)),
            ctx,
        );
    });
}

/// Helper function to get buffer text from CodeEditorView.
fn buffer_text(editor: &ViewHandle<CodeEditorView>, app: &App) -> String {
    editor.read(app, |view, ctx| view.text(ctx).into_string())
}

/// Helper function to get current cursor position (head of first selection).
fn cursor_position(editor: &ViewHandle<CodeEditorView>, app: &App) -> (usize, usize) {
    editor.read(app, |view, ctx| {
        let buffer_selection = view.model.as_ref(ctx).buffer_selection_model().as_ref(ctx);
        let buffer = view.model.as_ref(ctx).buffer().as_ref(ctx);
        let head_point = buffer_selection
            .first_selection_head()
            .to_buffer_point(buffer);
        (head_point.row as usize, head_point.column as usize)
    })
}

/// Helper function to get vim mode from CodeEditorView.
fn vim_mode(editor: &ViewHandle<CodeEditorView>, app: &App) -> Option<VimMode> {
    editor.read(app, |view, ctx| view.vim_mode(ctx))
}

/// Helper to set the cursor to a specific (row, col). Rows are 1-based to match test expectations.
fn set_cursor_position(editor: &ViewHandle<CodeEditorView>, row: usize, col: usize, app: &mut App) {
    editor.update(app, |view, ctx| {
        view.model.update(ctx, |model, ctx| {
            let buffer = model.buffer().as_ref(ctx);
            let point = Point::new(row as u32, col as u32);
            let offset = point.to_buffer_char_offset(buffer);
            model
                .selection()
                .update(ctx, |sel, ctx| sel.set_cursor(offset, ctx));
        });
    });
}

/// Set the viewport to exactly `lines` rows tall. Returns the line height in pixels.
fn set_viewport_lines(editor: &ViewHandle<CodeEditorView>, lines: usize, app: &mut App) -> f32 {
    let (line_height, render_state) = editor.read(app, |view, ctx| {
        let model = view.model.as_ref(ctx);
        (model.line_height(ctx), model.render_state().clone())
    });
    render_state.update(app, |render_state, ctx| {
        render_state.set_viewport_size(
            SizeInfo {
                viewport_size: Vector2F::new(800.0, line_height * lines as f32),
                needs_layout: false,
            },
            ctx,
        );
    });
    line_height
}

/// Read the current vertical scroll position.
fn scroll_top(editor: &ViewHandle<CodeEditorView>, app: &App) -> f32 {
    editor.read(app, |view, ctx| {
        view.model
            .as_ref(ctx)
            .render_state()
            .as_ref(ctx)
            .viewport()
            .scroll_top()
            .as_f32()
    })
}

#[test]
fn test_code_editor_vim_basic_mode_switching() {
    let _feature_flag_guard = FeatureFlag::VimCodeEditor.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_code_editor_app(&mut app);

        let editor = add_code_editor("hello", &mut app);
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Normal));

        vim_user_insert(&editor, "i", &mut app);
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Insert));

        editor.update(&mut app, |view, ctx| {
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), ctx);
        });
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Normal));
    });
}

#[test]
fn test_vim_number_repeat_action() {
    let _feature_flag_guard = FeatureFlag::VimCodeEditor.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_code_editor_app(&mut app);

        let editor = add_code_editor(
            "line 1
            line 2
            line 3
            line 4",
            &mut app,
        );

        assert_eq!(cursor_position(&editor, &app), (1, 0));
        vim_user_insert(&editor, "3dd", &mut app);
        assert_eq!(buffer_text(&editor, &app), "line 4",);
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Normal));
        assert_eq!(cursor_position(&editor, &app), (1, 0));
    });
}

#[test]
fn test_vim_number_repeat_word_motion() {
    let _feature_flag_guard = FeatureFlag::VimCodeEditor.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_code_editor_app(&mut app);

        let editor = add_code_editor(
            "abc de f
            gh ijkl m nop"
                .unindent()
                .as_str(),
            &mut app,
        );

        vim_user_insert(&editor, "3w", &mut app);
        assert_eq!(cursor_position(&editor, &app), (2, 0));

        // Try to go past the end of the buffer.
        vim_user_insert(&editor, "4w", &mut app);
        assert_eq!(cursor_position(&editor, &app), (2, 12));

        vim_user_insert(&editor, "2b", &mut app);
        assert_eq!(cursor_position(&editor, &app), (2, 8));

        vim_user_insert(&editor, "2b2e", &mut app);
        assert_eq!(cursor_position(&editor, &app), (2, 6));
    });
}

#[test]
fn test_vim_number_repeat_line_motion() {
    let _feature_flag_guard = FeatureFlag::VimCodeEditor.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_code_editor_app(&mut app);

        let editor = add_code_editor(
            "aa
            bbbbb
            ccc
            dddd"
                .unindent()
                .as_str(),
            &mut app,
        );

        // Ensure layout is completed before vertical/line motions like count + '$' or '^'.
        layout_editor_view(&mut app, &editor).await;

        // Start at buffer start (helper already positions at start). Apply "2$".
        // Expect to end on line 2, column 4 (1-based row indexing in CodeEditorView tests).
        vim_user_insert(&editor, "2$", &mut app);
        assert_eq!(cursor_position(&editor, &app), (2, 4));

        // Apply "3$" from there – should move to end of last line (line 4), column 3.
        vim_user_insert(&editor, "3$", &mut app);
        assert_eq!(cursor_position(&editor, &app), (4, 3));

        // "^" ignores the repetition count – should move to first non-blank in current line.
        vim_user_insert(&editor, "3^", &mut app);
        assert_eq!(cursor_position(&editor, &app), (4, 0));
    });
}

#[test]
fn test_vim_number_repeat_character_motion() {
    let _feature_flag_guard = FeatureFlag::VimCodeEditor.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_code_editor_app(&mut app);

        let editor = add_code_editor(
            "aa
            bbbbb
            ccc
            dddd"
                .unindent()
                .as_str(),
            &mut app,
        );

        // Ensure layout so vertical motions (j/k) use real geometry for goal columns.
        layout_editor_view(&mut app, &editor).await;

        // #j: from (1,0) go down 3 lines -> (4,0)
        vim_user_insert(&editor, "3j", &mut app);
        assert_eq!(cursor_position(&editor, &app), (4, 0));

        // #l: move right 3 -> (4,3)
        vim_user_insert(&editor, "3l", &mut app);
        assert_eq!(cursor_position(&editor, &app), (4, 3));

        // #k: move up 2 -> (2,3) (line 2 has enough width)
        vim_user_insert(&editor, "2k", &mut app);
        assert_eq!(cursor_position(&editor, &app), (2, 3));

        // #h (attempt to go past start): from col 3 left 4 -> clamp to 0 -> (2,0)
        vim_user_insert(&editor, "4h", &mut app);
        assert_eq!(cursor_position(&editor, &app), (2, 0));
    });
}

#[test]
fn test_vim_number_repeat_op_word_motion() {
    let _feature_flag_guard = FeatureFlag::VimCodeEditor.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_code_editor_app(&mut app);

        let editor = add_code_editor(
            "line 1
            line 2
            line 3
            line 4",
            &mut app,
        );

        set_cursor_position(&editor, 2, 0, &mut app);
        vim_user_insert(&editor, "d3w", &mut app);
        assert_eq!(
            buffer_text(&editor, &app),
            "line 1
            3
            line 4"
                .unindent()
                .as_str(),
        );
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Normal));

        // Change two words starting at start of the first line
        set_cursor_position(&editor, 1, 0, &mut app);
        vim_user_insert(&editor, "c2w", &mut app);
        assert_eq!(buffer_text(&editor, &app), "\n3\nline 4");
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Insert));

        // Insert replacement, return to Normal
        vim_user_insert(&editor, "replacement", &mut app);
        editor.update(&mut app, |view, ctx| {
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), ctx);
        });
        assert_eq!(
            buffer_text(&editor, &app),
            "replacement
             3
             line 4"
                .unindent()
                .as_str(),
        );
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Normal));

        // Attempt to delete more words than remain after the cursor
        set_cursor_position(&editor, 3, 0, &mut app);
        vim_user_insert(&editor, "d3w", &mut app);
        assert_eq!(buffer_text(&editor, &app), "replacement\n3\n");
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Normal));
    });
}

#[test]
fn test_vim_number_repeat_op_line_object() {
    let _feature_flag_guard = FeatureFlag::VimCodeEditor.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_code_editor_app(&mut app);

        let editor = add_code_editor(
            "line 1
            line 2
            line 3 is longer
            line 4 is long
            line 5
            line 6
            line 7 too",
            &mut app,
        );

        // Begin on line 2
        set_cursor_position(&editor, 2, 0, &mut app);
        // Delete 2 lines starting at current line (use canonical Vim count form for CodeEditorView)
        vim_user_insert(&editor, "d2d", &mut app);
        assert_eq!(
            buffer_text(&editor, &app),
            "line 1
            line 4 is long
            line 5
            line 6
            line 7 too"
                .unindent()
                .as_str(),
        );
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Normal));

        // Change 3 lines starting at current line
        vim_user_insert(&editor, "c3c", &mut app);
        assert_eq!(
            buffer_text(&editor, &app),
            "line 1

            line 7 too"
                .unindent()
                .as_str(),
        );
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Insert));

        // Insert replacement text; remain in insert mode
        vim_user_insert(&editor, "replacement", &mut app);
        assert_eq!(cursor_position(&editor, &app), (2, 11));
        assert_eq!(
            buffer_text(&editor, &app),
            "line 1
            replacement
            line 7 too"
                .unindent()
                .as_str(),
        );
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Insert));
    });
}

#[test]
fn test_vim_number_repeat_op_line_motions() {
    let _feature_flag_guard = FeatureFlag::VimCodeEditor.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_code_editor_app(&mut app);

        let editor = add_code_editor(
            "The quick brown fox
            jumped over
            the lazy dog.",
            &mut app,
        );

        // Start at (1, 10) – after "The quick "
        set_cursor_position(&editor, 1, 10, &mut app);
        // Change through end-of-line twice (across line break)
        vim_user_insert(&editor, "c2$", &mut app);
        assert_eq!(buffer_text(&editor, &app), "The quick \nthe lazy dog.");
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Insert));

        // Insert replacement lines, then return to Normal
        vim_user_insert(&editor, "bear\nhigh-fived", &mut app);
        editor.update(&mut app, |view, ctx| {
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), ctx);
        });
        assert_eq!(cursor_position(&editor, &app), (2, 9)); // TODO is this ok
        assert_eq!(
            buffer_text(&editor, &app),
            "The quick bear
            high-fived
            the lazy dog."
                .unindent()
                .as_str(),
        );
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Normal));

        // Now delete through end-of-line twice from (1,9)
        set_cursor_position(&editor, 1, 9, &mut app);
        vim_user_insert(&editor, "d2$", &mut app);
        assert_eq!(
            buffer_text(&editor, &app),
            "The quick
            the lazy dog."
                .unindent()
                .as_str(),
        );
        assert_eq!(cursor_position(&editor, &app), (1, 8));
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Normal));
    });
}

#[test]
fn test_vim_number_repeat_character_motions_right() {
    let _feature_flag_guard = FeatureFlag::VimCodeEditor.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_code_editor_app(&mut app);

        let editor = add_code_editor(
            "The quick brown fox
            jumped",
            &mut app,
        );

        // Replace to the right (5 characters)
        set_cursor_position(&editor, 1, 10, &mut app);
        vim_user_insert(&editor, "c5l", &mut app);
        assert_eq!(buffer_text(&editor, &app), "The quick  fox\njumped");
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Insert));

        // Insert replacement text, escape back to Normal, then move right once
        vim_user_insert(&editor, "purple", &mut app);
        editor.update(&mut app, |view, ctx| {
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), ctx);
        });
        vim_user_insert(&editor, "l", &mut app);
        assert_eq!(cursor_position(&editor, &app), (1, 16));
        assert_eq!(buffer_text(&editor, &app), "The quick purple fox\njumped");
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Normal));

        // Delete to the right (attempting to go past end of line)
        vim_user_insert(&editor, "d5l", &mut app);
        assert_eq!(buffer_text(&editor, &app), "The quick purple\njumped");
        assert_eq!(cursor_position(&editor, &app), (1, 15));
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Normal));
    });
}

#[test]
fn test_vim_number_repeat_character_motions_left() {
    let _feature_flag_guard = FeatureFlag::VimCodeEditor.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_code_editor_app(&mut app);

        let editor = add_code_editor(
            "The quick
            brown fox jumped",
            &mut app,
        );

        // Replace to the left (3 characters)
        set_cursor_position(&editor, 2, 9, &mut app);
        vim_user_insert(&editor, "c3h", &mut app);
        assert_eq!(buffer_text(&editor, &app), "The quick\nbrown  jumped");
        assert_eq!(cursor_position(&editor, &app), (2, 6));
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Insert));

        // Insert replacement and escape
        vim_user_insert(&editor, "bear", &mut app);
        editor.update(&mut app, |view, ctx| {
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), ctx);
        });
        assert_eq!(buffer_text(&editor, &app), "The quick\nbrown bear jumped");
        assert_eq!(cursor_position(&editor, &app), (2, 9));
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Normal));

        // Delete to the left, attempting to delete past line start
        vim_user_insert(&editor, "d12h", &mut app);
        assert_eq!(buffer_text(&editor, &app), "The quick\nr jumped");
        assert_eq!(cursor_position(&editor, &app), (2, 0));
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Normal));
    });
}

#[test]
fn test_vim_number_repeat_op_combination() {
    let _feature_flag_guard = FeatureFlag::VimCodeEditor.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_code_editor_app(&mut app);

        let editor = add_code_editor(
            "line 1
            line 2
            line 3
            line 4
            line 5",
            &mut app,
        );

        // Start on line 2 and delete 2 * (3 words) => removes lines 2..4
        set_cursor_position(&editor, 2, 0, &mut app);
        vim_user_insert(&editor, "2d3w", &mut app);
        assert_eq!(
            buffer_text(&editor, &app),
            "line 1
            line 5"
                .unindent()
                .as_str(),
        );

        // Reset buffer and try 3d2w
        editor.update(&mut app, |view, ctx| {
            view.reset(
                InitialBufferState::plain_text(
                    "line 1\nline 2\nline 3\nline 4\nline 5".unindent().as_str(),
                ),
                ctx,
            );
            view.handle_action(&CodeEditorViewAction::CursorAtBufferStart, ctx);
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), ctx);
        });
        set_cursor_position(&editor, 2, 0, &mut app);
        vim_user_insert(&editor, "3d2w", &mut app);
        assert_eq!(
            buffer_text(&editor, &app),
            "line 1
            line 5"
                .unindent()
                .as_str(),
        );
    });
}

#[test]
fn test_vim_number_repeat_yank_paste_linewise() {
    let _feature_flag_guard = FeatureFlag::VimCodeEditor.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_code_editor_app(&mut app);

        let editor = add_code_editor(
            "aaa
            b b b
            cc"
            .unindent()
            .as_str(),
            &mut app,
        );

        // Yank 2 lines from line 2
        set_cursor_position(&editor, 2, 0, &mut app);
        vim_user_insert(&editor, "y2y", &mut app);
        assert_eq!(
            buffer_text(&editor, &app),
            "aaa
            b b b
            cc"
            .unindent()
            .as_str()
        );

        // Paste 3 times before cursor
        vim_user_insert(&editor, "3P", &mut app);
        assert_eq!(
            buffer_text(&editor, &app),
            "aaa
            b b b
            cc
            b b b
            cc
            b b b
            cc
            b b b
            cc"
            .unindent()
            .as_str()
        );
    });
}

#[test]
fn test_vim_number_repeat_yank_paste_charwise() {
    let _feature_flag_guard = FeatureFlag::VimCodeEditor.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_code_editor_app(&mut app);

        let editor = add_code_editor("a bc d efg h", &mut app);

        set_cursor_position(&editor, 1, 2, &mut app);
        vim_user_insert(&editor, "y2w", &mut app);
        assert_eq!(buffer_text(&editor, &app), "a bc d efg h");

        vim_user_insert(&editor, "3P", &mut app);
        assert_eq!(buffer_text(&editor, &app), "a bc d bc d bc d bc d efg h");
        assert_eq!(cursor_position(&editor, &app), (1, 16));
    });
}

#[test]
fn test_vim_delete_lines_d0() {
    let _feature_flag_guard = FeatureFlag::VimCodeEditor.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_code_editor_app(&mut app);

        let editor = add_code_editor("abcd\n  efgh\nijkl", &mut app);

        // delete to start of line (including non-whitespace)
        set_cursor_position(&editor, 2, 4, &mut app);
        vim_user_insert(&editor, "d0", &mut app);
        assert_eq!(buffer_text(&editor, &app), "abcd\ngh\nijkl");
        assert_eq!(cursor_position(&editor, &app), (2, 0));
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Normal));
    });
}

#[test]
fn test_vim_delete_lines_d_caret() {
    let _feature_flag_guard = FeatureFlag::VimCodeEditor.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_code_editor_app(&mut app);

        let editor = add_code_editor("abcd\n  efgh\nijkl", &mut app);

        // delete to first non-whitespace
        set_cursor_position(&editor, 2, 4, &mut app);
        vim_user_insert(&editor, "d^", &mut app);
        assert_eq!(buffer_text(&editor, &app), "abcd\n  gh\nijkl");
        assert_eq!(cursor_position(&editor, &app), (2, 2));
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Normal));
    });
}

#[test]
fn test_vim_delete_lines_d_dollar() {
    let _feature_flag_guard = FeatureFlag::VimCodeEditor.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_code_editor_app(&mut app);

        let editor = add_code_editor("abcd\n  efgh\nijkl", &mut app);

        // delete to end of line
        set_cursor_position(&editor, 2, 4, &mut app);
        vim_user_insert(&editor, "d$", &mut app);
        assert_eq!(buffer_text(&editor, &app), "abcd\n  ef\nijkl");
        assert_eq!(cursor_position(&editor, &app), (2, 3));
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Normal));

        // delete to end of line using D alias
        set_cursor_position(&editor, 3, 2, &mut app);
        vim_user_insert(&editor, "D", &mut app);
        assert_eq!(buffer_text(&editor, &app), "abcd\n  ef\nij");
        assert_eq!(cursor_position(&editor, &app), (3, 1));
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Normal));
    });
}

#[test]
fn test_vim_replace_simple() {
    let _feature_flag_guard = FeatureFlag::VimCodeEditor.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_code_editor_app(&mut app);
        let editor = add_code_editor("abcdef", &mut app);

        // Replace first character
        vim_user_insert(&editor, "r", &mut app);
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Replace));
        assert_eq!(cursor_position(&editor, &app), (1, 0));
        assert_eq!(buffer_text(&editor, &app), "abcdef");

        vim_user_insert(&editor, "g", &mut app);
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Normal));
        assert_eq!(cursor_position(&editor, &app), (1, 0));
        assert_eq!(buffer_text(&editor, &app), "gbcdef");

        // Replace last character
        set_cursor_position(&editor, 1, 5, &mut app);
        vim_user_insert(&editor, "r", &mut app);
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Replace));
        assert_eq!(cursor_position(&editor, &app), (1, 5));
        assert_eq!(buffer_text(&editor, &app), "gbcdef");

        vim_user_insert(&editor, "h", &mut app);
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Normal));
        assert_eq!(cursor_position(&editor, &app), (1, 5));
        assert_eq!(buffer_text(&editor, &app), "gbcdeh");
    });
}

#[test]
fn test_vim_replace_number_repeat() {
    let _feature_flag_guard = FeatureFlag::VimCodeEditor.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_code_editor_app(&mut app);
        let editor = add_code_editor("abcdef\nghijk", &mut app);

        // Replace first three characters with 'l'
        vim_user_insert(&editor, "3r", &mut app);
        assert_eq!(buffer_text(&editor, &app), "abcdef\nghijk");
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Replace));
        assert_eq!(cursor_position(&editor, &app), (1, 0));

        vim_user_insert(&editor, "l", &mut app);
        assert_eq!(buffer_text(&editor, &app), "llldef\nghijk");
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Normal));
        assert_eq!(cursor_position(&editor, &app), (1, 2));
    });
}

#[test]
fn test_vim_replace_number_repeat_end_of_line() {
    // If count > remaining chars, cancel
    let _feature_flag_guard = FeatureFlag::VimCodeEditor.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_code_editor_app(&mut app);
        let editor = add_code_editor("abcdef\nghijk", &mut app);

        set_cursor_position(&editor, 1, 4, &mut app);
        vim_user_insert(&editor, "3r", &mut app);
        assert_eq!(buffer_text(&editor, &app), "abcdef\nghijk");
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Replace));
        assert_eq!(cursor_position(&editor, &app), (1, 4));

        vim_user_insert(&editor, "m", &mut app);
        // Cancelled; no change
        assert_eq!(buffer_text(&editor, &app), "abcdef\nghijk");
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Normal));
        assert_eq!(cursor_position(&editor, &app), (1, 4));
    });
}

#[test]
fn test_vim_delete_char_x() {
    let _feature_flag_guard = FeatureFlag::VimCodeEditor.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_code_editor_app(&mut app);
        let editor = add_code_editor("abcde", &mut app);

        // delete first character in the buffer
        set_cursor_position(&editor, 1, 0, &mut app);
        vim_user_insert(&editor, "x", &mut app);
        assert_eq!(buffer_text(&editor, &app), "bcde");
        assert_eq!(cursor_position(&editor, &app), (1, 0));

        // delete last character in the buffer
        set_cursor_position(&editor, 1, 3, &mut app);
        vim_user_insert(&editor, "x", &mut app);
        assert_eq!(buffer_text(&editor, &app), "bcd");
        // cursor moves back to cover last char
        assert_eq!(cursor_position(&editor, &app), (1, 2));

        // delete last character again
        vim_user_insert(&editor, "x", &mut app);
        assert_eq!(buffer_text(&editor, &app), "bc");
        assert_eq!(cursor_position(&editor, &app), (1, 1));
    });
}

#[test]
fn test_vim_delete_char_motion_sideways() {
    let _feature_flag_guard = FeatureFlag::VimCodeEditor.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_code_editor_app(&mut app);
        let editor = add_code_editor("echo hello", &mut app);

        vim_user_insert(&editor, "w", &mut app);
        vim_user_insert(&editor, "dl", &mut app);
        assert_eq!(buffer_text(&editor, &app), "echo ello");
        assert_eq!(cursor_position(&editor, &app), (1, 5));

        vim_user_insert(&editor, "dh", &mut app);
        assert_eq!(buffer_text(&editor, &app), "echoello");
        assert_eq!(cursor_position(&editor, &app), (1, 4));

        vim_user_insert(&editor, "$", &mut app);
        assert_eq!(cursor_position(&editor, &app), (1, 7));

        vim_user_insert(&editor, "dl", &mut app);
        assert_eq!(buffer_text(&editor, &app), "echoell");
        assert_eq!(cursor_position(&editor, &app), (1, 6));
    });
}

#[test]
fn test_vim_delete_char_space() {
    let _feature_flag_guard = FeatureFlag::VimCodeEditor.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_code_editor_app(&mut app);
        let editor = add_code_editor(
            "echo hi
            echo there",
            &mut app,
        );

        // Deleting forward at the start of the buffer works (d<space>)
        vim_user_insert(&editor, "d ", &mut app);
        assert_eq!(buffer_text(&editor, &app), "cho hi\necho there");
        assert_eq!(cursor_position(&editor, &app), (1, 0));

        // Move to the end of the line.
        vim_user_insert(&editor, "$", &mut app);

        // Deleting forward at the end of the line deletes the last character.
        vim_user_insert(&editor, "d ", &mut app);
        assert_eq!(buffer_text(&editor, &app), "cho h\necho there");
        assert_eq!(cursor_position(&editor, &app), (1, 4));

        // Deleting forward wraps around to the next line with count
        vim_user_insert(&editor, "3d ", &mut app);
        assert_eq!(buffer_text(&editor, &app), "cho cho there");
        assert_eq!(cursor_position(&editor, &app), (1, 4));
    });
}

#[test]
fn test_vim_delete_char_backspace() {
    let _feature_flag_guard = FeatureFlag::VimCodeEditor.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_code_editor_app(&mut app);
        let editor = add_code_editor(
            "echo hi
            echo there",
            &mut app,
        );

        // Deleting backward at the start of the buffer does nothing.
        vim_user_insert(&editor, "d", &mut app);
        editor.update(&mut app, |view, ctx| {
            view.vim_keystroke(&Keystroke::parse("backspace").unwrap(), ctx);
        });
        assert_eq!(buffer_text(&editor, &app), "echo hi\necho there");
        assert_eq!(cursor_position(&editor, &app), (1, 0));

        // Move to the end of the first line.
        vim_user_insert(&editor, "$", &mut app);

        // Deleting backward at the end of the line removes the second-to-last character.
        vim_user_insert(&editor, "d", &mut app);
        editor.update(&mut app, |view, ctx| {
            view.vim_keystroke(&Keystroke::parse("backspace").unwrap(), ctx);
        });
        assert_eq!(buffer_text(&editor, &app), "echo i\necho there");
        assert_eq!(cursor_position(&editor, &app), (1, 5));

        // Move to the next line.
        vim_user_insert(&editor, "w", &mut app);

        // Deleting backward wraps around to the previous line with count
        vim_user_insert(&editor, "3d", &mut app);
        editor.update(&mut app, |view, ctx| {
            view.vim_keystroke(&Keystroke::parse("backspace").unwrap(), ctx);
        });
        assert_eq!(buffer_text(&editor, &app), "echoecho there");
        assert_eq!(cursor_position(&editor, &app), (1, 4));
    });
}

#[test]
fn test_vim_delete_word_dw() {
    let _feature_flag_guard = FeatureFlag::VimCodeEditor.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_code_editor_app(&mut app);
        let editor = add_code_editor(
            "abc de.f g-hi-j
            kL",
            &mut app,
        );

        // dw
        vim_user_insert(&editor, "dw", &mut app);
        assert_eq!(buffer_text(&editor, &app), "de.f g-hi-j\nkL".unindent());

        // dW
        vim_user_insert(&editor, "dW", &mut app);
        assert_eq!(buffer_text(&editor, &app), "g-hi-j\nkL".unindent());

        // dW again does not cross a line break
        vim_user_insert(&editor, "dW", &mut app);
        assert_eq!(buffer_text(&editor, &app), "\nkL");
    });
}

#[test]
fn test_vim_delete_word_de() {
    let _feature_flag_guard = FeatureFlag::VimCodeEditor.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_code_editor_app(&mut app);
        let editor = add_code_editor(
            "g-hi-j kL
            mNop.qr st u v wX/yZ",
            &mut app,
        );

        // dE
        vim_user_insert(&editor, "dE", &mut app);
        assert_eq!(
            buffer_text(&editor, &app),
            " kL\nmNop.qr st u v wX/yZ".unindent()
        );

        // de
        vim_user_insert(&editor, "de", &mut app);
        assert_eq!(buffer_text(&editor, &app), "\nmNop.qr st u v wX/yZ");

        // dE across line break
        vim_user_insert(&editor, "dE", &mut app);
        assert_eq!(buffer_text(&editor, &app), " st u v wX/yZ");
    });
}

#[test]
fn test_vim_delete_word_db() {
    let _feature_flag_guard = FeatureFlag::VimCodeEditor.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_code_editor_app(&mut app);
        let editor = add_code_editor("mNop.qr st u v wX/yZ", &mut app);

        // Move cursor to end of line (simulate Vim quirk of block cursor)
        editor.update(&mut app, |view, ctx| {
            view.handle_action(&CodeEditorViewAction::MoveToLineEnd, ctx);
            view.handle_action(&CodeEditorViewAction::MoveLeft, ctx);
        });
        assert_eq!(cursor_position(&editor, &app), (1, 19));

        // dB
        vim_user_insert(&editor, "dB", &mut app);
        assert_eq!(buffer_text(&editor, &app), "mNop.qr st u v Z");

        // db
        vim_user_insert(&editor, "db", &mut app);
        assert_eq!(buffer_text(&editor, &app), "mNop.qr st u Z");
        assert_eq!(cursor_position(&editor, &app), (1, 13));

        // dw to remove the last Z
        vim_user_insert(&editor, "dw", &mut app);
        assert_eq!(buffer_text(&editor, &app), "mNop.qr st u ");
        assert_eq!(cursor_position(&editor, &app), (1, 12));

        // db twice more
        vim_user_insert(&editor, "db", &mut app);
        assert_eq!(buffer_text(&editor, &app), "mNop.qr st  ");
        assert_eq!(cursor_position(&editor, &app), (1, 11));

        vim_user_insert(&editor, "db", &mut app);
        assert_eq!(buffer_text(&editor, &app), "mNop.qr  ");
        assert_eq!(cursor_position(&editor, &app), (1, 8));

        // dB to remove symbol
        vim_user_insert(&editor, "dB", &mut app);
        assert_eq!(buffer_text(&editor, &app), " ");
        assert_eq!(cursor_position(&editor, &app), (1, 0));
    });
}

#[test]
fn test_vim_delete_word_dge() {
    let _feature_flag_guard = FeatureFlag::VimCodeEditor.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_code_editor_app(&mut app);
        let editor = add_code_editor("echo hello-hi warp-dev", &mut app);

        set_cursor_position(&editor, 1, 20, &mut app);
        vim_user_insert(&editor, "dge", &mut app);
        assert_eq!(buffer_text(&editor, &app), "echo hello-hi warpev");
        assert_eq!(cursor_position(&editor, &app), (1, 18));

        vim_user_insert(&editor, "dge", &mut app);
        assert_eq!(buffer_text(&editor, &app), "echo hello-hev");
        assert_eq!(cursor_position(&editor, &app), (1, 12));

        vim_user_insert(&editor, "dgE", &mut app);
        assert_eq!(buffer_text(&editor, &app), "echev");
        assert_eq!(cursor_position(&editor, &app), (1, 3));
    });
}

#[test]
fn test_vim_delete_word_empty() {
    let _feature_flag_guard = FeatureFlag::VimCodeEditor.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_code_editor_app(&mut app);
        let editor = add_code_editor("", &mut app);

        // try each operation on an empty buffer
        for seq in ["dw", "dW", "de", "dE", "db", "dB", "dge", "dgE"] {
            vim_user_insert(&editor, seq, &mut app);
        }
        assert_eq!(buffer_text(&editor, &app), "");
    });
}

#[test]
fn test_vim_dw_newline_quirks() {
    let _feature_flag_guard = FeatureFlag::VimCodeEditor.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_code_editor_app(&mut app);
        let editor = add_code_editor("echo foo  \necho bar", &mut app);

        // A "w" motion can traverse the newline.
        vim_user_insert(&editor, "eelw", &mut app);
        assert_eq!(cursor_position(&editor, &app), (2, 0));

        // An "e" motion can also traverse the newline.
        vim_user_insert(&editor, "ggeee", &mut app);
        assert_eq!(cursor_position(&editor, &app), (2, 3));

        // An "e" motion with an operator can traverse the newline.
        vim_user_insert(&editor, "gelde", &mut app);
        assert_eq!(buffer_text(&editor, &app), "echo foo bar");

        // However, a "w" motion with an operator cannot traverse the newline.
        editor.update(&mut app, |view, ctx| {
            view.reset(InitialBufferState::plain_text("echo foo  \necho bar"), ctx);
            view.handle_action(&CodeEditorViewAction::CursorAtBufferStart, ctx);
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), ctx);
        });
        vim_user_insert(&editor, "ggeeldw", &mut app);
        assert_eq!(buffer_text(&editor, &app), "echo foo\necho bar");

        vim_user_insert(&editor, "dw", &mut app);
        assert_eq!(buffer_text(&editor, &app), "echo fo\necho bar");

        // Including a count allows it to traverse the newline
        vim_user_insert(&editor, "2dw", &mut app);
        assert_eq!(buffer_text(&editor, &app), "echo fbar");
    });
}

#[test]
fn test_vim_jump_to_end_and_beginning() {
    let _feature_flag_guard = FeatureFlag::VimCodeEditor.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_code_editor_app(&mut app);
        let editor = add_code_editor(
            "abc
            def
            ghi",
            &mut app,
        );

        vim_user_insert(&editor, "G", &mut app);
        assert_eq!(cursor_position(&editor, &app), (3, 0));

        vim_user_insert(&editor, "gg", &mut app);
        assert_eq!(cursor_position(&editor, &app), (1, 0));
    });
}

#[test]
fn test_vim_begin_line_below() {
    let _feature_flag_guard = FeatureFlag::VimCodeEditor.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_code_editor_app(&mut app);
        let editor = add_code_editor(
            "abc
            def
            ghi",
            &mut app,
        );

        set_cursor_position(&editor, 2, 0, &mut app);
        vim_user_insert(&editor, "o", &mut app);
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Insert));
        assert_eq!(cursor_position(&editor, &app), (3, 0));
        assert_eq!(buffer_text(&editor, &app), "abc\ndef\n\nghi".unindent());
    });
}

#[test]
fn test_vim_begin_line_above() {
    let _feature_flag_guard = FeatureFlag::VimCodeEditor.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_code_editor_app(&mut app);
        let editor = add_code_editor(
            "abcdef
                    ghijkl
                    mnopqr",
            &mut app,
        );

        set_cursor_position(&editor, 2, 4, &mut app);
        layout_editor_view(&mut app, &editor).await;

        vim_user_insert(&editor, "O", &mut app);
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Insert));
        assert_eq!(cursor_position(&editor, &app), (2, 0));
        assert_eq!(
            buffer_text(&editor, &app),
            "abcdef\n\nghijkl\nmnopqr".unindent()
        );
    });
}

#[test]
fn test_vim_substitute_char() {
    let _feature_flag_guard = FeatureFlag::VimCodeEditor.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_code_editor_app(&mut app);
        let editor = add_code_editor("abcdef", &mut app);

        // Substitute at start of line
        set_cursor_position(&editor, 1, 0, &mut app);
        vim_user_insert(&editor, "s", &mut app);
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Insert));
        assert_eq!(buffer_text(&editor, &app), "bcdef");
        assert_eq!(cursor_position(&editor, &app), (1, 0));

        vim_user_insert(&editor, "xyz", &mut app);
        assert_eq!(buffer_text(&editor, &app), "xyzbcdef");
        assert_eq!(cursor_position(&editor, &app), (1, 3));

        editor.update(&mut app, |view, ctx| {
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), ctx);
        });
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Normal));
        assert_eq!(cursor_position(&editor, &app), (1, 2));

        // Substitute at end of line
        set_cursor_position(&editor, 1, 7, &mut app);
        vim_user_insert(&editor, "s", &mut app);
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Insert));
        assert_eq!(buffer_text(&editor, &app), "xyzbcde");
        assert_eq!(cursor_position(&editor, &app), (1, 7));

        vim_user_insert(&editor, "jkl", &mut app);
        assert_eq!(buffer_text(&editor, &app), "xyzbcdejkl");
        assert_eq!(cursor_position(&editor, &app), (1, 10));

        editor.update(&mut app, |view, ctx| {
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), ctx);
        });
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Normal));
        assert_eq!(cursor_position(&editor, &app), (1, 9));
    });
}

#[test]
fn test_vim_substitute_line() {
    let _feature_flag_guard = FeatureFlag::VimCodeEditor.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_code_editor_app(&mut app);
        let editor = add_code_editor(
            "aaa
            bbb
            ccc",
            &mut app,
        );

        // substitute first line
        set_cursor_position(&editor, 1, 0, &mut app);
        vim_user_insert(&editor, "S", &mut app);
        assert_eq!(cursor_position(&editor, &app), (1, 0));
        assert_eq!(buffer_text(&editor, &app), "\n\nbbb\nccc".unindent());
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Insert));

        vim_user_insert(&editor, "new text", &mut app);
        assert_eq!(cursor_position(&editor, &app), (1, 8));
        assert_eq!(buffer_text(&editor, &app), "new text\nbbb\nccc".unindent());
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Insert));

        editor.update(&mut app, |view, ctx| {
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), ctx);
        });
        assert_eq!(cursor_position(&editor, &app), (1, 7));
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Normal));

        // substitute last line
        set_cursor_position(&editor, 3, 2, &mut app);
        vim_user_insert(&editor, "S", &mut app);
        assert_eq!(cursor_position(&editor, &app), (3, 0));
        assert_eq!(buffer_text(&editor, &app), "new text\nbbb\n\n".unindent());
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Insert));

        vim_user_insert(&editor, "replacement", &mut app);
        assert_eq!(cursor_position(&editor, &app), (3, 11));
        assert_eq!(
            buffer_text(&editor, &app),
            "new text\nbbb\nreplacement".unindent()
        );
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Insert));

        editor.update(&mut app, |view, ctx| {
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), ctx);
        });
        assert_eq!(cursor_position(&editor, &app), (3, 10));
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Normal));
    });
}

#[test]
fn test_vim_visual_selection_with_newlines() {
    let _feature_flag_guard = FeatureFlag::VimCodeEditor.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_code_editor_app(&mut app);
        let editor = add_code_editor(
            "ddd
            aaa
            bbb

            ccc",
            &mut app,
        );

        // select aaa, bbb, and the newline
        set_cursor_position(&editor, 2, 0, &mut app);
        vim_user_insert(&editor, "V", &mut app);
        vim_user_insert(&editor, "2j", &mut app);
        assert_eq!(
            vim_mode(&editor, &app),
            Some(VimMode::Visual(MotionType::Linewise))
        );

        // delete selection
        vim_user_insert(&editor, "d", &mut app);
        assert_eq!(buffer_text(&editor, &app), "ddd\nccc".unindent());

        // select onto a smaller line above
        editor.update(&mut app, |view, ctx| {
            view.reset(InitialBufferState::plain_text("aaa\nbbbbb\nc\nddd"), ctx);
        });
        set_cursor_position(&editor, 2, 3, &mut app);
        vim_user_insert(&editor, "V", &mut app);
        vim_user_insert(&editor, "j", &mut app);

        // delete selection
        vim_user_insert(&editor, "d", &mut app);

        assert_eq!(buffer_text(&editor, &app), "aaa\nddd".unindent());
    });
}

#[test]
fn test_vim_k_at_top_of_file_does_not_panic() {
    let _feature_flag_guard = FeatureFlag::VimCodeEditor.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_code_editor_app(&mut app);
        let editor = add_code_editor(
            "line 1
            line 2
            line 3",
            &mut app,
        );

        layout_editor_view(&mut app, &editor).await;

        // Cursor already at row 1 (top). Pressing `k` should not panic.
        assert_eq!(cursor_position(&editor, &app), (1, 0));
        vim_user_insert(&editor, "k", &mut app);
        assert_eq!(cursor_position(&editor, &app), (1, 0));
    });
}

#[test]
fn test_vim_counted_k_overflow_does_not_panic() {
    let _feature_flag_guard = FeatureFlag::VimCodeEditor.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_code_editor_app(&mut app);
        let editor = add_code_editor(
            "line 1
            line 2
            line 3
            line 4
            line 5
            line 6
            line 7
            line 8
            line 9
            line 10",
            &mut app,
        );

        layout_editor_view(&mut app, &editor).await;

        // Move to line 10 (row 10)
        set_cursor_position(&editor, 10, 0, &mut app);
        assert_eq!(cursor_position(&editor, &app), (10, 0));

        // 100k should clamp to row 1 without panicking
        vim_user_insert(&editor, "100k", &mut app);
        assert_eq!(cursor_position(&editor, &app), (1, 0));
    });
}

#[test]
fn test_vim_dgg_at_first_line_does_not_panic() {
    let _feature_flag_guard = FeatureFlag::VimCodeEditor.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_code_editor_app(&mut app);
        let editor = add_code_editor(
            "aaa
            bbb
            ccc",
            &mut app,
        );

        // dgg from line 1 — vim_select_to_buffer_start produces CharOffset(0)
        assert_eq!(cursor_position(&editor, &app), (1, 0));
        vim_user_insert(&editor, "dgg", &mut app);
        assert_eq!(cursor_position(&editor, &app), (1, 0));
    });
}

#[test]
fn test_vim_gg_at_first_line_does_not_panic() {
    let _feature_flag_guard = FeatureFlag::VimCodeEditor.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_code_editor_app(&mut app);
        let editor = add_code_editor(
            "aaa
            bbb
            ccc",
            &mut app,
        );

        // gg when already at first line — jump_to_line_column(0, ...)
        assert_eq!(cursor_position(&editor, &app), (1, 0));
        vim_user_insert(&editor, "gg", &mut app);
        assert_eq!(cursor_position(&editor, &app), (1, 0));
    });
}

#[test]
fn test_vim_linewise_delete_at_first_line_does_not_panic() {
    let _feature_flag_guard = FeatureFlag::VimCodeEditor.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_code_editor_app(&mut app);
        let editor = add_code_editor(
            "aaa
            bbb
            ccc",
            &mut app,
        );

        // dd on line 1 — vim_extend_selection_linewise with row 0
        assert_eq!(cursor_position(&editor, &app), (1, 0));
        vim_user_insert(&editor, "dd", &mut app);
        assert_eq!(buffer_text(&editor, &app), "bbb\nccc");
    });
}

#[test]
fn test_vim_visual_linewise_delete_first_line_does_not_panic() {
    let _feature_flag_guard = FeatureFlag::VimCodeEditor.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_code_editor_app(&mut app);
        let editor = add_code_editor(
            "aaa
            bbb
            ccc",
            &mut app,
        );

        // V then d on line 1 — visual linewise at row 0
        assert_eq!(cursor_position(&editor, &app), (1, 0));
        vim_user_insert(&editor, "V", &mut app);
        vim_user_insert(&editor, "d", &mut app);
        assert_eq!(buffer_text(&editor, &app), "bbb\nccc");
    });
}

#[test]
fn test_vim_zz_in_normal_mode_preserves_cursor_and_mode() {
    let _feature_flag_guard = FeatureFlag::VimCodeEditor.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_code_editor_app(&mut app);
        let editor = add_code_editor(
            "line 1
            line 2
            line 3
            line 4
            line 5",
            &mut app,
        );

        layout_editor_view(&mut app, &editor).await;

        // Place cursor on line 3, then center it. zz scrolls but should not move
        // the cursor or change the mode.
        set_cursor_position(&editor, 3, 0, &mut app);
        assert_eq!(cursor_position(&editor, &app), (3, 0));
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Normal));

        vim_user_insert(&editor, "zz", &mut app);

        assert_eq!(cursor_position(&editor, &app), (3, 0));
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Normal));
    });
}

#[test]
fn test_vim_zz_in_visual_mode_preserves_cursor_and_mode() {
    let _feature_flag_guard = FeatureFlag::VimCodeEditor.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_code_editor_app(&mut app);
        let editor = add_code_editor(
            "line 1
            line 2
            line 3
            line 4
            line 5",
            &mut app,
        );

        layout_editor_view(&mut app, &editor).await;

        set_cursor_position(&editor, 3, 0, &mut app);
        vim_user_insert(&editor, "v", &mut app);
        assert_eq!(
            vim_mode(&editor, &app),
            Some(VimMode::Visual(MotionType::Charwise))
        );

        vim_user_insert(&editor, "zz", &mut app);

        assert_eq!(cursor_position(&editor, &app), (3, 0));
        assert_eq!(
            vim_mode(&editor, &app),
            Some(VimMode::Visual(MotionType::Charwise))
        );
    });
}

#[test]
fn test_vim_z_followed_by_non_z_clears_pending() {
    let _feature_flag_guard = FeatureFlag::VimCodeEditor.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_code_editor_app(&mut app);
        let editor = add_code_editor(
            "line 1
            line 2
            line 3",
            &mut app,
        );

        layout_editor_view(&mut app, &editor).await;

        // `z` followed by an unrecognized char should clear the pending action.
        // After that, a subsequent `j` should move the cursor down one line as normal.
        set_cursor_position(&editor, 1, 0, &mut app);
        vim_user_insert(&editor, "z", &mut app);
        vim_user_insert(&editor, "x", &mut app);
        assert_eq!(cursor_position(&editor, &app), (1, 0));

        vim_user_insert(&editor, "j", &mut app);
        assert_eq!(cursor_position(&editor, &app), (2, 0));
    });
}

#[test]
fn test_vim_ctrl_d_scrolls_half_page_down() {
    let _feature_flag_guard = FeatureFlag::VimCodeEditor.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_code_editor_app(&mut app);
        let buffer: String = (1..=200).map(|i| format!("line {}\n", i)).collect();
        let editor = add_code_editor(buffer.as_str(), &mut app);

        layout_editor_view(&mut app, &editor).await;

        // 20 visible lines → half page = 10 lines.
        let line_height = set_viewport_lines(&editor, 20, &mut app);
        let half_page = 10;

        set_cursor_position(&editor, 1, 0, &mut app);
        let (start_row, _) = cursor_position(&editor, &app);
        let start_scroll = scroll_top(&editor, &app);

        editor.update(&mut app, |view, ctx| {
            view.vim_keystroke(&Keystroke::parse("ctrl-d").unwrap(), ctx);
        });

        let (after_row, _) = cursor_position(&editor, &app);
        let after_scroll = scroll_top(&editor, &app);
        assert_eq!(after_row, start_row + half_page);
        assert!(
            (after_scroll - start_scroll - half_page as f32 * line_height).abs() < 0.5,
            "scroll_top should advance by half_page * line_height \
             (start={start_scroll}, after={after_scroll}, line_height={line_height})",
        );
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Normal));
    });
}

#[test]
fn test_vim_ctrl_u_scrolls_half_page_up() {
    let _feature_flag_guard = FeatureFlag::VimCodeEditor.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_code_editor_app(&mut app);
        let buffer: String = (1..=200).map(|i| format!("line {}\n", i)).collect();
        let editor = add_code_editor(buffer.as_str(), &mut app);

        layout_editor_view(&mut app, &editor).await;

        // 20 visible lines → half page = 10 lines.
        let line_height = set_viewport_lines(&editor, 20, &mut app);
        let half_page = 10;

        // Start near the bottom and scroll down so we have room to scroll up.
        set_cursor_position(&editor, 100, 0, &mut app);
        let render_state = editor.read(&app, |view, ctx| {
            view.model.as_ref(ctx).render_state().clone()
        });
        render_state.update(&mut app, |render_state, ctx| {
            render_state.scroll(-(50.0 * line_height).into_pixels(), ctx);
        });

        let (start_row, _) = cursor_position(&editor, &app);
        let start_scroll = scroll_top(&editor, &app);

        editor.update(&mut app, |view, ctx| {
            view.vim_keystroke(&Keystroke::parse("ctrl-u").unwrap(), ctx);
        });

        let (after_row, _) = cursor_position(&editor, &app);
        let after_scroll = scroll_top(&editor, &app);
        assert_eq!(after_row, start_row - half_page);
        assert!(
            (start_scroll - after_scroll - half_page as f32 * line_height).abs() < 0.5,
            "scroll_top should retreat by half_page * line_height \
             (start={start_scroll}, after={after_scroll}, line_height={line_height})",
        );
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Normal));
    });
}

#[test]
fn test_vim_ctrl_d_with_count_scrolls_n_lines() {
    let _feature_flag_guard = FeatureFlag::VimCodeEditor.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_code_editor_app(&mut app);
        let buffer: String = (1..=200).map(|i| format!("line {}\n", i)).collect();
        let editor = add_code_editor(buffer.as_str(), &mut app);

        layout_editor_view(&mut app, &editor).await;

        // Viewport half page would be 10, but `5<C-d>` should scroll by 5 lines,
        // not 5 * half_page.
        set_viewport_lines(&editor, 20, &mut app);

        set_cursor_position(&editor, 1, 0, &mut app);
        let (start_row, _) = cursor_position(&editor, &app);

        vim_user_insert(&editor, "5", &mut app);
        editor.update(&mut app, |view, ctx| {
            view.vim_keystroke(&Keystroke::parse("ctrl-d").unwrap(), ctx);
        });

        let (after_row, _) = cursor_position(&editor, &app);
        assert_eq!(after_row, start_row + 5);
    });
}

#[test]
fn test_vim_ctrl_d_consumes_pending_count() {
    let _feature_flag_guard = FeatureFlag::VimCodeEditor.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_code_editor_app(&mut app);
        // Use a buffer big enough that scrolling won't max the cursor at the bottom.
        let buffer: String = (1..=200)
            .map(|i| format!("line {}\n", i))
            .collect::<String>();
        let editor = add_code_editor(buffer.as_str(), &mut app);

        layout_editor_view(&mut app, &editor).await;

        // After `2<C-d>`, the pending count of 2 must be consumed by ctrl-d. A
        // following `j` should move the cursor down exactly 1 line, not 2.
        set_cursor_position(&editor, 1, 0, &mut app);
        vim_user_insert(&editor, "2", &mut app);
        editor.update(&mut app, |view, ctx| {
            view.vim_keystroke(&Keystroke::parse("ctrl-d").unwrap(), ctx);
        });
        let (after_scroll_row, _) = cursor_position(&editor, &app);
        vim_user_insert(&editor, "j", &mut app);
        let (after_j_row, _) = cursor_position(&editor, &app);
        assert_eq!(
            after_j_row,
            after_scroll_row + 1,
            "j after `2<C-d>` should move down 1, not 2 (after_scroll_row={}, after_j_row={})",
            after_scroll_row,
            after_j_row
        );
    });
}

#[test]
fn test_vim_ctrl_d_clears_pending_operator() {
    let _feature_flag_guard = FeatureFlag::VimCodeEditor.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_code_editor_app(&mut app);
        let editor = add_code_editor(
            "alpha bravo charlie
            delta echo foxtrot
            golf hotel india",
            &mut app,
        );

        layout_editor_view(&mut app, &editor).await;

        // After `d<C-d>`, the pending `d` operator must be cleared. A following
        // `w` should move forward by word, not delete a word.
        set_cursor_position(&editor, 1, 0, &mut app);
        let original = buffer_text(&editor, &app);
        vim_user_insert(&editor, "d", &mut app);
        editor.update(&mut app, |view, ctx| {
            view.vim_keystroke(&Keystroke::parse("ctrl-d").unwrap(), ctx);
        });
        vim_user_insert(&editor, "w", &mut app);
        assert_eq!(
            buffer_text(&editor, &app),
            original,
            "w after `d<C-d>` should not delete (pending d should be cleared)"
        );
    });
}
