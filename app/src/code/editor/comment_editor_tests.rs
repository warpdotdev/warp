use std::sync::Arc;

use repo_metadata::RepoMetadataModel;
use repo_metadata::repositories::DetectedRepositories;
use warpui::platform::WindowStyle;
use warpui::presenter::ChildView;
use warpui::{App, Element, Entity, TypedActionView, View, ViewHandle, WindowId};

use super::{create_editable_comment_markdown_editor, create_readonly_comment_markdown_editor};
use crate::appearance::Appearance;
use crate::auth::AuthStateProvider;
use crate::cloud_object::model::persistence::CloudModel;
use crate::notebooks::editor::keys::NotebookKeybindings;
use crate::notebooks::editor::view::RichTextEditorView;
use crate::notebooks::link::{NotebookLinks, SessionSource};
use crate::search::files::model::FileSearchModel;
use crate::server::server_api::team::MockTeamClient;
use crate::server::server_api::workspace::MockWorkspaceClient;
use crate::settings_view::keybindings::KeybindingChangedNotifier;
use crate::terminal::keys::TerminalKeybindings;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::workspace::ActiveSession;
use crate::{GlobalResourceHandles, GlobalResourceHandlesProvider, UserWorkspaces};

struct TestView {
    editor: ViewHandle<RichTextEditorView>,
}

enum CommentEditorMode {
    Editable,
    Readonly,
}

impl Entity for TestView {
    type Event = ();
}

impl View for TestView {
    fn ui_name() -> &'static str {
        "CommentEditorTestView"
    }

    fn render(&self, _app: &warpui::AppContext) -> Box<dyn warpui::Element> {
        ChildView::new(&self.editor).finish()
    }
}

impl TypedActionView for TestView {
    type Action = ();
}

fn initialize_editor(
    app: &mut App,
    mode: CommentEditorMode,
) -> (
    WindowId,
    ViewHandle<RichTextEditorView>,
    ViewHandle<TestView>,
) {
    initialize_settings_for_tests(app);

    let global_resources = GlobalResourceHandles::mock(app);
    app.add_singleton_model(|_| GlobalResourceHandlesProvider::new(global_resources));
    app.add_singleton_model(|_| Appearance::mock());
    app.add_singleton_model(|_| ActiveSession::default());
    app.add_singleton_model(|_| KeybindingChangedNotifier::new());
    app.add_singleton_model(|_| DetectedRepositories::default());
    app.add_singleton_model(RepoMetadataModel::new);
    app.add_singleton_model(FileSearchModel::new);
    app.add_singleton_model(NotebookKeybindings::new);
    app.add_singleton_model(TerminalKeybindings::new);
    app.add_singleton_model(CloudModel::mock);
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());

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

    let (window, test_view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
        let window_id = ctx.window_id();
        let _links = ctx.add_model(|ctx| NotebookLinks::new(SessionSource::Active(window_id), ctx));
        let editor = match mode {
            CommentEditorMode::Editable => create_editable_comment_markdown_editor(None, ctx),
            CommentEditorMode::Readonly => create_readonly_comment_markdown_editor(
                "```rust\nfn main() {}\n```",
                false,
                None,
                ctx,
            ),
        };
        TestView { editor }
    });

    let editor_view = app.read(|ctx| test_view.as_ref(ctx).editor.clone());
    (window, editor_view, test_view)
}

#[test]
fn test_editable_comment_editor_keeps_final_trailing_newline_for_non_empty_code_blocks() {
    App::test((), |mut app| async move {
        let (_window, editor_view, _test_view) =
            initialize_editor(&mut app, CommentEditorMode::Editable);
        let render_model = editor_view.read(&app, |editor, ctx| {
            editor.model().as_ref(ctx).render_state().clone()
        });

        let pending_layout =
            render_model.read(&app, |render_state, _| render_state.layout_complete());
        pending_layout.await;
        assert_eq!(
            render_model.read(&app, |render_state, _| render_state.blocks()),
            1
        );

        editor_view.update(&mut app, |editor, ctx| {
            editor.reset_with_markdown("```rust\nfn main() {}\n```", ctx);
        });

        let pending_layout =
            render_model.read(&app, |render_state, _| render_state.layout_complete());
        pending_layout.await;
        assert_eq!(
            render_model.read(&app, |render_state, _| render_state.blocks()),
            2
        );
    });
}

#[test]
fn test_readonly_comment_editor_hides_final_trailing_newline_for_non_empty_code_blocks() {
    App::test((), |mut app| async move {
        let (_window, editor_view, _test_view) =
            initialize_editor(&mut app, CommentEditorMode::Readonly);
        let render_model = editor_view.read(&app, |editor, ctx| {
            editor.model().as_ref(ctx).render_state().clone()
        });

        let pending_layout =
            render_model.read(&app, |render_state, _| render_state.layout_complete());
        pending_layout.await;
        assert_eq!(
            render_model.read(&app, |render_state, _| render_state.blocks()),
            1
        );
    });
}
