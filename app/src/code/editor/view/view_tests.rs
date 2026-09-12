use std::sync::Arc;

use warp_core::ui::appearance::Appearance;
use warp_editor::content::buffer::InitialBufferState;
use warp_editor::render::element::VerticalExpansionBehavior;
use warp_util::user_input::UserInput;
use warpui::clipboard::ClipboardContent;
use warpui::elements::ScrollbarWidth;
use warpui::elements::new_scrollable::ScrollableAppearance;
use warpui::platform::WindowStyle;
use warpui::{App, TypedActionView, ViewHandle, WindowId};

use super::{CodeEditorRenderOptions, CodeEditorView, CodeEditorViewAction};
use crate::AuthStateProvider;
use crate::cloud_object::model::persistence::CloudModel;
use crate::editor::InteractionState;
use crate::notebooks::editor::keys::NotebookKeybindings;
use crate::server::server_api::team::MockTeamClient;
use crate::server::server_api::workspace::MockWorkspaceClient;
use crate::settings_view::keybindings::KeybindingChangedNotifier;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::vim_registers::VimRegisters;
use crate::workspace::ActiveSession;
use crate::workspace::sync_inputs::SyncedInputState;
use crate::workspaces::user_workspaces::UserWorkspaces;

fn initialize_editor(app: &mut App) -> (WindowId, ViewHandle<CodeEditorView>) {
    initialize_editor_singletons(app);

    let (window, editor_view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
        CodeEditorView::new(
            None,
            None,
            CodeEditorRenderOptions::new(VerticalExpansionBehavior::GrowToMaxHeight),
            ctx,
        )
        .with_horizontal_scrollbar_appearance(ScrollableAppearance::new(ScrollbarWidth::Auto, true))
    });

    (window, editor_view)
}

/// Adds an editor that owns its copy shortcut outright, seeded with `buffer_content`
/// and the cursor at the start of the buffer.
fn initialize_editor_copying_the_cursor_line(
    app: &mut App,
    buffer_content: &str,
) -> ViewHandle<CodeEditorView> {
    initialize_editor_singletons(app);

    let buffer_content = buffer_content.to_string();
    app.add_window(WindowStyle::NotStealFocus, move |ctx| {
        let mut editor = CodeEditorView::new(
            None,
            None,
            CodeEditorRenderOptions::new(VerticalExpansionBehavior::GrowToMaxHeight),
            ctx,
        )
        .with_copy_line_when_selection_is_empty();
        editor.reset(InitialBufferState::plain_text(&buffer_content), ctx);
        editor.handle_action(&CodeEditorViewAction::CursorAtBufferStart, ctx);
        editor
    })
    .1
}

/// Registers the singleton models that a [`CodeEditorView`] depends on.
fn initialize_editor_singletons(app: &mut App) {
    initialize_settings_for_tests(app);

    // Add all required singleton models for EditorView dependencies
    app.add_singleton_model(|_| Appearance::mock());
    app.add_singleton_model(|_| SyncedInputState::mock());
    app.add_singleton_model(|_| VimRegisters::new());
    app.add_singleton_model(|_| KeybindingChangedNotifier::mock());
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());

    // Add mocks required by rich text editor (used in CommentEditor)
    app.add_singleton_model(CloudModel::mock);
    app.add_singleton_model(|_| ActiveSession::default());
    app.add_singleton_model(NotebookKeybindings::new);

    // Add UserWorkspaces mock (required by EditorView)
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
}

#[test]
fn test_interaction_state_prevents_editing() {
    App::test((), |mut app| async move {
        let (_window, editor_view) = initialize_editor(&mut app);

        let text = editor_view.update(&mut app, |view, ctx| {
            view.handle_action(&CodeEditorViewAction::UserTyped(UserInput::new("abc")), ctx);
            view.text(ctx)
        });

        assert_eq!(text.as_str(), "abc");

        // Set to be only selectable
        editor_view.update(&mut app, |view, ctx| {
            view.set_interaction_state(InteractionState::Selectable, ctx);
        });

        let text = editor_view.update(&mut app, |view, ctx| {
            view.handle_action(&CodeEditorViewAction::UserTyped(UserInput::new("def")), ctx);
            view.text(ctx)
        });

        assert_eq!(text.as_str(), "abc");
    });
}

#[test]
fn copies_the_cursor_line_when_the_selection_is_empty() {
    App::test((), |mut app| async move {
        let editor_view = initialize_editor_copying_the_cursor_line(&mut app, "alpha\nbeta");

        editor_view.update(&mut app, |view, ctx| {
            view.handle_action(&CodeEditorViewAction::Copy, ctx);
        });

        assert_eq!(
            app.update(|ctx| ctx.clipboard().read().plain_text),
            "alpha\n",
            "an empty-selection copy should take the cursor's line and its newline"
        );
    });
}

#[test]
fn copies_the_last_line_without_a_trailing_newline() {
    App::test((), |mut app| async move {
        let editor_view = initialize_editor_copying_the_cursor_line(&mut app, "alpha\nbeta");

        editor_view.update(&mut app, |view, ctx| {
            view.handle_action(&CodeEditorViewAction::CursorAtBufferEnd, ctx);
            view.handle_action(&CodeEditorViewAction::Copy, ctx);
        });

        assert_eq!(app.update(|ctx| ctx.clipboard().read().plain_text), "beta");
    });
}

#[test]
fn copies_only_the_selection_when_one_exists() {
    App::test((), |mut app| async move {
        let editor_view = initialize_editor_copying_the_cursor_line(&mut app, "alpha\nbeta");

        editor_view.update(&mut app, |view, ctx| {
            view.handle_action(&CodeEditorViewAction::CursorAtBufferEnd, ctx);
            view.handle_action(&CodeEditorViewAction::SelectLeft, ctx);
            view.handle_action(&CodeEditorViewAction::SelectLeft, ctx);
            view.handle_action(&CodeEditorViewAction::Copy, ctx);
        });

        assert_eq!(app.update(|ctx| ctx.clipboard().read().plain_text), "ta");
    });
}

#[test]
fn copying_an_empty_document_leaves_the_clipboard_untouched() {
    App::test((), |mut app| async move {
        let editor_view = initialize_editor_copying_the_cursor_line(&mut app, "");
        app.update(|ctx| {
            ctx.clipboard()
                .write(ClipboardContent::plain_text("kept".to_string()))
        });

        editor_view.update(&mut app, |view, ctx| {
            view.handle_action(&CodeEditorViewAction::Copy, ctx);
        });

        assert_eq!(
            app.update(|ctx| ctx.clipboard().read().plain_text),
            "kept",
            "an empty document has no line to copy, so the clipboard must survive"
        );
    });
}

#[test]
fn pastes_a_copied_line_above_the_cursor_line() {
    App::test((), |mut app| async move {
        let editor_view = initialize_editor_copying_the_cursor_line(&mut app, "alpha\nbeta");

        let text = editor_view.update(&mut app, |view, ctx| {
            view.handle_action(&CodeEditorViewAction::Copy, ctx);
            view.handle_action(&CodeEditorViewAction::CursorAtBufferEnd, ctx);
            view.handle_action(&CodeEditorViewAction::Paste, ctx);
            view.text(ctx)
        });

        assert_eq!(text.as_str(), "alpha\nalpha\nbeta");
    });
}

#[test]
fn pastes_a_copied_line_whole_when_the_cursor_sits_mid_line() {
    App::test((), |mut app| async move {
        let editor_view =
            initialize_editor_copying_the_cursor_line(&mut app, "    alpha\nbeta\ngamma");

        let text = editor_view.update(&mut app, |view, ctx| {
            view.handle_action(&CodeEditorViewAction::Copy, ctx);
            view.handle_action(&CodeEditorViewAction::MoveToLineEnd, ctx);
            view.handle_action(&CodeEditorViewAction::Paste, ctx);
            view.text(ctx)
        });

        assert_eq!(
            text.as_str(),
            "    alpha\n    alpha\nbeta\ngamma",
            "a line-wise paste must not split the line the caret happens to sit in"
        );
    });
}

#[test]
fn an_editor_that_delegates_empty_copies_pastes_at_the_caret() {
    App::test((), |mut app| async move {
        let (_window, editor_view) = initialize_editor(&mut app);
        app.update(|ctx| {
            ctx.clipboard()
                .write(ClipboardContent::plain_text("X".to_string()))
        });

        let text = editor_view.update(&mut app, |view, ctx| {
            view.handle_action(&CodeEditorViewAction::UserTyped(UserInput::new("ab")), ctx);
            view.handle_action(&CodeEditorViewAction::MoveLeft, ctx);
            view.handle_action(&CodeEditorViewAction::Paste, ctx);
            view.text(ctx)
        });

        assert_eq!(
            text.as_str(),
            "aXb",
            "an editor that has not opted in keeps the char-wise paste at the caret"
        );
    });
}

#[test]
fn an_editor_that_delegates_empty_copies_does_not_take_the_cursor_line() {
    App::test((), |mut app| async move {
        let (_window, editor_view) = initialize_editor(&mut app);

        editor_view.update(&mut app, |view, ctx| {
            view.handle_action(
                &CodeEditorViewAction::UserTyped(UserInput::new("alpha")),
                ctx,
            );
            view.handle_action(&CodeEditorViewAction::Copy, ctx);
        });

        assert_eq!(
            app.update(|ctx| ctx.clipboard().read().plain_text),
            "",
            "an embedded editor hands an empty-selection copy to its parent view instead"
        );
    });
}
