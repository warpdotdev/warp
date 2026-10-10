use std::sync::Arc;

use warpui::platform::WindowStyle;
use warpui::{AddSingletonModel, App};

use super::super::EditorAction;
use super::VoiceInputState;
use crate::appearance::Appearance;
use crate::auth::AuthStateProvider;
use crate::editor::EditorView;
use crate::server::server_api::team::MockTeamClient;
use crate::server::server_api::workspace::MockWorkspaceClient;
use crate::server::server_api::TranscribeError;
use crate::settings_view::keybindings::KeybindingChangedNotifier;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::vim_registers::VimRegisters;
use crate::workspace::sync_inputs::SyncedInputState;
use crate::workspace::ToastStack;
use crate::workspaces::user_workspaces::UserWorkspaces;

fn initialize_app(app: &mut App) {
    initialize_settings_for_tests(app);

    app.add_singleton_model(|_| Appearance::mock());
    app.add_singleton_model(|_| ToastStack);
    app.add_singleton_model(|_ctx| SyncedInputState::mock());
    app.add_singleton_model(|_ctx| VimRegisters::new());
    app.add_singleton_model(|_| KeybindingChangedNotifier::mock());
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(voice_input::VoiceInput::new);

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
fn test_aborts_voice_input_predicate_for_passive_and_editing_actions() {
    assert!(EditorAction::Paste.aborts_voice_input());
    assert!(EditorAction::CtrlC.aborts_voice_input());
    assert!(
        EditorAction::UnhandledModifierKey(Arc::new("ctrl-k".to_string())).aborts_voice_input()
    );
    assert!(!EditorAction::Focus.aborts_voice_input());
    assert!(!EditorAction::HideXRay.aborts_voice_input());
    assert!(
        !EditorAction::ToggleVoiceInput(voice_input::VoiceInputToggledFrom::Button)
            .aborts_voice_input()
    );
}

#[test]
fn test_handle_action_aborts_active_voice_for_editing_action() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let (_, editor) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            EditorView::new(Default::default(), ctx)
        });

        editor.update(&mut app, |editor, ctx| {
            editor.set_voice_input_state(VoiceInputState::Listening, ctx);
            assert!(editor.is_voice_input_active());

            <EditorView as warpui::TypedActionView>::handle_action(
                editor,
                &EditorAction::UnhandledModifierKey(Arc::new("ctrl-k".to_string())),
                ctx,
            );

            assert!(!editor.is_voice_input_active());
        });
    });
}

#[test]
fn test_handle_action_keeps_active_voice_for_passive_action() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let (_, editor) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            EditorView::new(Default::default(), ctx)
        });

        editor.update(&mut app, |editor, ctx| {
            editor.set_voice_input_state(VoiceInputState::Listening, ctx);
            assert!(editor.is_voice_input_active());

            <EditorView as warpui::TypedActionView>::handle_action(
                editor,
                &EditorAction::Focus,
                ctx,
            );

            assert!(editor.is_voice_input_active());
        });
    });
}

/// When transcription returns blank text (empty or whitespace-only), the editor
/// buffer must not be modified even with an active selection. This guards
/// against replacing a selection with an empty insert.
#[test]
fn test_blank_transcription_does_not_modify_buffer() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let (_, editor) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            EditorView::new(Default::default(), ctx)
        });
        for transcribed_text in ["", "   \n\t "] {
            editor.update(&mut app, |editor, ctx| {
                editor.set_buffer_text("existing content", ctx);
                <EditorView as warpui::TypedActionView>::handle_action(
                    editor,
                    &EditorAction::SelectAll,
                    ctx,
                );
                assert_eq!(editor.selected_text(ctx), "existing content");
                editor.apply_transcribed_voice_input(Ok(transcribed_text.to_string()), ctx);
            });

            editor.read(&app, |editor, ctx| {
                assert_eq!(
                    editor.buffer_text(ctx),
                    "existing content",
                    "Blank transcription result must not modify the editor buffer"
                );
            });
        }
    });
}

/// When transcription returns a non-empty string, the editor buffer must be updated
/// with that text. This is a regression guard to ensure the fix for empty strings
/// did not inadvertently suppress non-empty transcriptions.
#[test]
fn test_non_empty_transcription_inserts_text() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let (_, editor) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            EditorView::new(Default::default(), ctx)
        });

        editor.update(&mut app, |editor, ctx| {
            editor.apply_transcribed_voice_input(Ok("hello world".to_string()), ctx);
        });

        editor.read(&app, |editor, ctx| {
            assert_eq!(
                editor.buffer_text(ctx),
                "hello world",
                "Non-empty transcription result must be inserted into the editor buffer"
            );
        });
    });
}

/// When transcription returns an error, the buffer must remain unchanged.
#[test]
fn test_transcription_error_does_not_modify_buffer() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let (_, editor) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            EditorView::new(Default::default(), ctx)
        });

        editor.update(&mut app, |editor, ctx| {
            editor.set_buffer_text("untouched", ctx);
        });

        editor.update(&mut app, |editor, ctx| {
            editor.apply_transcribed_voice_input(
                Err(TranscribeError::Other(anyhow::anyhow!("network error"))),
                ctx,
            );
        });

        editor.read(&app, |editor, ctx| {
            assert_eq!(
                editor.buffer_text(ctx),
                "untouched",
                "A transcription error must not modify the editor buffer"
            );
        });
    });
}
