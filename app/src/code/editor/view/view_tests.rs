use std::sync::Arc;

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
use crate::editor::{EditorAction, InteractionState};
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
fn test_find_input_can_be_refocused_by_click_after_enter() {
    App::test((), |mut app| async move {
        let (window, editor_view) = initialize_editor(&mut app);

        editor_view.update(&mut app, |view, ctx| {
            view.handle_action(
                &CodeEditorViewAction::UserTyped(UserInput::new("foo bar foo")),
                ctx,
            );
            view.handle_action(&CodeEditorViewAction::ShowFindBar, ctx);
        });

        let find_bar = editor_view
            .read(&app, |view, _ctx| view.find_bar.clone())
            .expect("find bar should be available");
        let find_editor = find_bar.read(&app, |find_bar, _ctx| find_bar.find_editor());

        // Type a query and press Enter, mirroring the reported repro steps.
        find_editor.update(&mut app, |editor, ctx| {
            editor.handle_action(&EditorAction::UserInsert(UserInput::new("foo")), ctx);
        });
        find_editor.update(&mut app, |editor, ctx| {
            editor.handle_action(&EditorAction::Enter, ctx);
        });

        // A real mouse click is dropped by `EditorElement::mouse_down` before it can dispatch
        // `EditorAction::Focus` unless the input is still selectable.
        let can_select_after_enter = find_editor.read(&app, |editor, ctx| editor.can_select(ctx));
        assert!(
            can_select_after_enter,
            "find input should remain selectable after Enter so a click can refocus it"
        );

        // Move focus away from the find input, matching the reported flow where the field is no
        // longer focused after Enter.
        editor_view.update(&mut app, |view, ctx| view.focus(ctx));
        assert_ne!(app.focused_view_id(window), Some(find_editor.id()));

        // Simulate clicking on the find input: this is exactly what `EditorElement::mouse_down`
        // dispatches when the click lands inside the input's bounds and it is selectable.
        find_editor.update(&mut app, |editor, ctx| {
            editor.handle_action(&EditorAction::Focus, ctx);
        });

        assert_eq!(
            app.focused_view_id(window),
            Some(find_editor.id()),
            "clicking the find input after pressing Enter should refocus it for editing"
        );
    });
}
