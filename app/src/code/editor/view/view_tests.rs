use std::sync::Arc;

use string_offset::CharOffset;
use vec1::vec1;
use warp_core::ui::appearance::Appearance;
use warp_editor::content::buffer::{InitialBufferState, SelectionOffsets};
use warp_editor::content::text::LineCount;
use warp_editor::model::CoreEditorModel;
use warp_editor::render::element::VerticalExpansionBehavior;
use warp_util::content_version::ContentVersion;
use warp_util::user_input::UserInput;
use warpui::elements::ScrollbarWidth;
use warpui::elements::new_scrollable::ScrollableAppearance;
use warpui::keymap::Keystroke;
use warpui::platform::WindowStyle;
use warpui::{App, TypedActionView, ViewHandle, WindowId};

use super::{CodeEditorRenderOptions, CodeEditorView, CodeEditorViewAction, init};
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
fn fold_shortcuts_dispatch_in_focused_code_editor() {
    App::test((), |mut app| async move {
        app.update(init);
        let (window_id, editor_view) = initialize_editor(&mut app);

        editor_view.update(&mut app, |view, ctx| {
            view.reset(
                InitialBufferState::plain_text("fn main() {\n    let x = 1;\n}\n")
                    .with_version(ContentVersion::new()),
                ctx,
            );
            view.model.update(ctx, |model, ctx| {
                model.cursor_at(CharOffset::from(18), ctx);
            });
            view.focus(ctx);
        });

        let fold = if cfg!(target_os = "macos") {
            "alt-cmd-["
        } else {
            "alt-ctrl-["
        };
        assert!(
            app.dispatch_keystroke(
                window_id,
                &[editor_view.id()],
                &Keystroke::parse(fold).expect("valid fold shortcut"),
                false,
            )
            .expect("fold shortcut should dispatch")
        );
        editor_view.read(&app, |view, ctx| {
            assert!(!view.model.as_ref(ctx).hidden_ranges(ctx).is_empty());
        });

        let unfold = if cfg!(target_os = "macos") {
            "alt-cmd-]"
        } else {
            "alt-ctrl-]"
        };
        assert!(
            app.dispatch_keystroke(
                window_id,
                &[editor_view.id()],
                &Keystroke::parse(unfold).expect("valid unfold shortcut"),
                false,
            )
            .expect("unfold shortcut should dispatch")
        );
        editor_view.read(&app, |view, ctx| {
            assert!(view.model.as_ref(ctx).hidden_ranges(ctx).is_empty());
        });

        editor_view.update(&mut app, |view, ctx| {
            view.model.update(ctx, |model, ctx| {
                let (start, end) = {
                    let buffer = model.buffer().as_ref(ctx);
                    (
                        buffer.line_start(LineCount::from(1)),
                        buffer.line_start(LineCount::from(4)),
                    )
                };
                model.buffer_selection_model().update(ctx, |selections, _| {
                    selections.set_selection_offsets(vec1![SelectionOffsets {
                        head: end,
                        tail: start,
                    }]);
                });
            });
        });

        let fold_selection = if cfg!(target_os = "macos") {
            "alt-cmd-f"
        } else {
            "alt-ctrl-f"
        };
        assert!(
            app.dispatch_keystroke(
                window_id,
                &[editor_view.id()],
                &Keystroke::parse(fold_selection).expect("valid fold selection shortcut"),
                false,
            )
            .expect("fold selection shortcut should dispatch")
        );
        editor_view.read(&app, |view, ctx| {
            assert!(!view.model.as_ref(ctx).hidden_ranges(ctx).is_empty());
        });
    });
}
