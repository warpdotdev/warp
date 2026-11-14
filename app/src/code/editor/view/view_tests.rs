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

// Regression coverage for lazily-created sub-views: constructing a `CodeEditorView` (as happens
// for every historical code block restored into an AI conversation) should not eagerly build the
// diff nav bar or go-to-line dialog, since most instances never use them. See APP-5770.
#[test]
fn test_nav_bar_not_created_until_diff_nav_toggled_on() {
    App::test((), |mut app| async move {
        let (_window, editor_view) = initialize_editor(&mut app);

        let nav_bar_exists_initially =
            editor_view.update(&mut app, |view, _ctx| view.nav_bar.is_some());
        assert!(
            !nav_bar_exists_initially,
            "nav bar should not be created until diff navigation is toggled on"
        );

        editor_view.update(&mut app, |view, ctx| {
            view.handle_action(&CodeEditorViewAction::ToggleDiffNav(None), ctx);
        });

        let nav_bar_exists_after_toggle =
            editor_view.update(&mut app, |view, _ctx| view.nav_bar.is_some());
        assert!(
            nav_bar_exists_after_toggle,
            "nav bar should be created once diff navigation is toggled on"
        );
    });
}

#[test]
fn test_goto_line_dialog_not_created_until_shown() {
    App::test((), |mut app| async move {
        let (_window, editor_view) = initialize_editor(&mut app);

        let dialog_exists_initially =
            editor_view.update(&mut app, |view, _ctx| view.goto_line_dialog.is_some());
        assert!(
            !dialog_exists_initially,
            "go-to-line dialog should not be created until it is shown"
        );

        editor_view.update(&mut app, |view, ctx| {
            view.handle_action(&CodeEditorViewAction::ShowGoToLine, ctx);
        });

        let dialog_open_after_show = editor_view.update(&mut app, |view, ctx| {
            view.goto_line_dialog
                .as_ref()
                .is_some_and(|dialog| dialog.as_ref(ctx).is_open())
        });
        assert!(
            dialog_open_after_show,
            "go-to-line dialog should be created and open once shown"
        );
    });
}
