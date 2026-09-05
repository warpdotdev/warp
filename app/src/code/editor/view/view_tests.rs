use std::sync::Arc;

use string_offset::CharOffset;
use warp_core::ui::appearance::Appearance;
use warp_editor::render::element::VerticalExpansionBehavior;
use warp_util::user_input::UserInput;
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

#[test]
fn test_select_to_buffer_boundaries() {
    App::test((), |mut app| async move {
        let (_window, editor_view) = initialize_editor(&mut app);

        editor_view.update(&mut app, |view, ctx| {
            view.handle_action(
                &CodeEditorViewAction::UserTyped(UserInput::new("first\nsecond")),
                ctx,
            );
            let max_offset = view.model.as_ref(ctx).max_character_offset(ctx);

            view.handle_action(&CodeEditorViewAction::SelectToBufferStart, ctx);
            let selection = view
                .model
                .as_ref(ctx)
                .buffer_selection_model()
                .as_ref(ctx)
                .selection_offsets()[0];
            assert_eq!(selection.head, CharOffset::from(1));
            assert_eq!(selection.tail, max_offset);

            view.handle_action(&CodeEditorViewAction::CursorAtBufferStart, ctx);
            view.handle_action(&CodeEditorViewAction::SelectToBufferEnd, ctx);
            let selection = view
                .model
                .as_ref(ctx)
                .buffer_selection_model()
                .as_ref(ctx)
                .selection_offsets()[0];
            assert_eq!(selection.head, max_offset);
            assert_eq!(selection.tail, CharOffset::from(1));
        });
    });
}

#[test]
fn test_select_to_buffer_boundary_preserves_reverse_selection_anchor() {
    App::test((), |mut app| async move {
        let (_window, editor_view) = initialize_editor(&mut app);

        editor_view.update(&mut app, |view, ctx| {
            view.handle_action(
                &CodeEditorViewAction::UserTyped(UserInput::new("first\nsecond")),
                ctx,
            );
            let max_offset = view.model.as_ref(ctx).max_character_offset(ctx);

            view.handle_action(&CodeEditorViewAction::SelectToBufferStart, ctx);
            view.handle_action(&CodeEditorViewAction::SelectRight, ctx);
            view.handle_action(&CodeEditorViewAction::SelectToBufferStart, ctx);

            let selection = view
                .model
                .as_ref(ctx)
                .buffer_selection_model()
                .as_ref(ctx)
                .selection_offsets()[0];
            assert_eq!(selection.head, CharOffset::from(1));
            assert_eq!(selection.tail, max_offset);
        });
    });
}

#[test]
fn test_select_to_buffer_boundaries_in_empty_editor() {
    App::test((), |mut app| async move {
        let (_window, editor_view) = initialize_editor(&mut app);

        editor_view.update(&mut app, |view, ctx| {
            view.handle_action(&CodeEditorViewAction::SelectToBufferStart, ctx);
            view.handle_action(&CodeEditorViewAction::SelectToBufferEnd, ctx);

            let selection = view
                .model
                .as_ref(ctx)
                .buffer_selection_model()
                .as_ref(ctx)
                .selection_offsets()[0];
            assert_eq!(selection.head, CharOffset::from(1));
            assert_eq!(selection.tail, CharOffset::from(1));
        });
    });
}
