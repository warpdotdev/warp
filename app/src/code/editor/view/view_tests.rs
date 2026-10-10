use std::sync::Arc;

use warp_core::ui::appearance::Appearance;
use warp_editor::render::element::VerticalExpansionBehavior;
use warp_editor::render::model::LineCount;
use warp_util::user_input::UserInput;
use warpui::elements::ScrollbarWidth;
use warpui::elements::new_scrollable::ScrollableAppearance;
use warpui::platform::WindowStyle;
use warpui::{App, TypedActionView, ViewHandle, WindowId};

use super::{CodeEditorRenderOptions, CodeEditorView, CodeEditorViewAction};
use crate::AuthStateProvider;
use crate::cloud_object::model::persistence::CloudModel;
use crate::code::editor::EditorReviewComment;
use crate::code::editor::line::EditorLineLocation;
use crate::code_review::comments::{CommentOrigin, LineDiffContent};
use crate::editor::InteractionState;
use crate::features::FeatureFlag;
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

// Regression coverage for APP-5356 ("CodeEditorView eagerly builds a comment composer per diff
// file"). `CodeReviewView` creates one `CodeEditorView` per changed file, so the composer (which
// pulls in a `NotebooksEditorModel`, `NotebookLinks` and a full `RichTextEditorView`) must never
// be built unless a comment is actually opened on that file.

#[test]
fn test_no_pending_comment_constructs_no_composer() {
    App::test((), |mut app| async move {
        let (_window, editor_view) = initialize_editor(&mut app);
        editor_view.read(&app, |view, _ctx| {
            assert!(
                view.active_comment_editor_for_test().is_none(),
                "A freshly constructed editor with no pending comment should not build a composer"
            );
        });
    });
}

#[test]
fn test_new_comment_on_line_constructs_and_focuses_composer() {
    // The composer's `line` is set only via its subscription to the model's one-shot
    // `NewPendingComment` event, so this also proves `ensure_active_comment_editor` is actually
    // called (a forgotten call would leave `active_comment_editor_for_test()` `None` and this
    // test would fail on the `.expect(...)` below).
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::InlineCodeReview.override_enabled(true);
        let (_window, editor_view) = initialize_editor(&mut app);

        let line = EditorLineLocation::Current {
            line_number: LineCount::from(3),
            line_range: LineCount::from(3)..LineCount::from(4),
        };

        editor_view.update(&mut app, |view, ctx| {
            view.handle_action(
                &CodeEditorViewAction::NewCommentOnLine { line: line.clone() },
                ctx,
            );
        });

        let comment_editor = editor_view.read(&app, |view, _ctx| {
            view.active_comment_editor_for_test()
                .cloned()
                .expect("NewCommentOnLine should construct the composer")
        });

        // `CommentEditor::on_focus` delegates focus to its inner text editor, so the window's
        // focused view ends up being that inner editor rather than the composer itself.
        comment_editor.read(&app, |comment_editor, ctx| {
            assert!(
                comment_editor.is_editor_focused_for_test(ctx),
                "NewCommentOnLine should focus the newly-built composer"
            );
            assert_eq!(comment_editor.line_for_test(), Some(&line));
            assert_eq!(comment_editor.comment_text(ctx), "");
        });
    });
}

#[test]
fn test_open_existing_comment_restores_saved_text_and_update_state() {
    // Exercises the same integration point `CodeReviewView` uses when it receives
    // `RequestOpenComment` off the gutter's "reopen saved comment" button.
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::InlineCodeReview.override_enabled(true);
        let (_window, editor_view) = initialize_editor(&mut app);

        let line = EditorLineLocation::Current {
            line_number: LineCount::from(5),
            line_range: LineCount::from(5)..LineCount::from(6),
        };
        let comment = EditorReviewComment::new(
            line.clone(),
            LineDiffContent::default(),
            "original text".to_string(),
        );
        let id = comment.id;

        editor_view.update(&mut app, |view, ctx| {
            view.set_comment_locations(std::iter::once(comment), ctx);
        });

        editor_view.update(&mut app, |view, ctx| {
            view.open_existing_comment(
                &id,
                &line,
                "saved comment text",
                &CommentOrigin::Native,
                ctx,
            );
        });

        let comment_editor = editor_view.read(&app, |view, _ctx| {
            view.active_comment_editor_for_test()
                .cloned()
                .expect("open_existing_comment should construct the composer")
        });

        comment_editor.read(&app, |comment_editor, ctx| {
            assert_eq!(comment_editor.comment_text(ctx), "saved comment text");
            assert_eq!(comment_editor.comment_id_for_test(), Some(id));
            assert!(
                comment_editor.show_remove_button_for_test(),
                "Reopening a saved comment should show the Update/Remove state"
            );
        });
    });
}
