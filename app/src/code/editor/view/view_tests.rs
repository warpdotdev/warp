use std::sync::Arc;
use warp_core::ui::appearance::Appearance;
use warp_editor::{
    content::buffer::{InitialBufferState, ToBufferPoint},
    model::CoreEditorModel,
    render::{element::VerticalExpansionBehavior, model::viewport::SizeInfo},
};
use warpui::{
    elements::{new_scrollable::ScrollableAppearance, ScrollbarWidth},
    geometry::vector::vec2f,
    platform::WindowStyle,
    text::point::Point,
    App, TypedActionView, ViewHandle, WindowId,
};

use crate::{
    cloud_object::model::persistence::CloudModel,
    editor::InteractionState,
    notebooks::editor::keys::NotebookKeybindings,
    server::server_api::{team::MockTeamClient, workspace::MockWorkspaceClient},
    settings_view::keybindings::KeybindingChangedNotifier,
    test_util::settings::initialize_settings_for_tests,
    vim_registers::VimRegisters,
    workspace::{sync_inputs::SyncedInputState, ActiveSession},
    workspaces::user_workspaces::UserWorkspaces,
    AuthStateProvider,
};

use super::{CodeEditorRenderOptions, CodeEditorView, CodeEditorViewAction};
use warp_util::user_input::UserInput;

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

async fn layout_editor_view(app: &mut App, editor: &ViewHandle<CodeEditorView>) {
    app.read(|ctx| {
        let model = editor.as_ref(ctx).model.as_ref(ctx);
        model.render_state().as_ref(ctx).layout_complete()
    })
    .await;
}

fn cursor_position(editor: &ViewHandle<CodeEditorView>, app: &App) -> Point {
    editor.read(app, |view, ctx| {
        let selection = view.model.as_ref(ctx).buffer_selection_model().as_ref(ctx);
        let buffer = view.model.as_ref(ctx).buffer().as_ref(ctx);
        selection.first_selection_head().to_buffer_point(buffer)
    })
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
fn test_page_up_and_page_down_move_cursor_by_viewport() {
    App::test((), |mut app| async move {
        let (_window, editor_view) = initialize_editor(&mut app);

        editor_view.update(&mut app, |view, ctx| {
            view.reset(
                InitialBufferState::plain_text(
                    "line 1\nline 2\nline 3\nline 4\nline 5\nline 6\nline 7",
                ),
                ctx,
            );
            view.handle_action(&CodeEditorViewAction::CursorAtBufferStart, ctx);
        });
        layout_editor_view(&mut app, &editor_view).await;

        editor_view.update(&mut app, |view, ctx| {
            view.model.update(ctx, |model, ctx| {
                model.render_state().update(ctx, |render_state, ctx| {
                    let line_height = render_state.styles().base_line_height().as_f32();
                    render_state.set_viewport_size(
                        SizeInfo {
                            viewport_size: vec2f(400., line_height * 3.),
                            needs_layout: false,
                        },
                        ctx,
                    );
                });
            });
        });

        editor_view.update(&mut app, |view, ctx| {
            view.handle_action(&CodeEditorViewAction::PageDown, ctx);
        });
        assert_eq!(cursor_position(&editor_view, &app), Point::new(4, 0));

        editor_view.update(&mut app, |view, ctx| {
            view.handle_action(&CodeEditorViewAction::PageUp, ctx);
        });
        assert_eq!(cursor_position(&editor_view, &app), Point::new(1, 0));
    });
}
