use std::path::PathBuf;
use std::sync::Arc;

use repo_metadata::repositories::DetectedRepositories;
use string_offset::CharOffset;
use warp_core::ui::appearance::Appearance;
use warp_editor::content::buffer::InitialBufferState;
use warp_editor::render::element::VerticalExpansionBehavior;
use warpui::elements::Empty;
use warpui::platform::WindowStyle;
use warpui::{App, Element as _, ModelHandle, SingletonEntity, ViewHandle};

use super::*;
use crate::ai::request_usage_model::AIRequestUsageModel;
use crate::auth::AuthStateProvider;
use crate::cloud_object::model::persistence::CloudModel;
use crate::code::buffer_location::LocalOrRemotePath;
use crate::code::editor::view::{CodeEditorRenderOptions, CodeEditorView};
use crate::code::local_code_editor::LocalCodeEditorView;
use crate::code_review::code_review_view::CodeReviewView;
use crate::code_review::diff_state::DiffStateModel;
use crate::code_review::GlobalCodeReviewModel;
use crate::pane_group::WorkingDirectoriesModel;
use crate::server::server_api::team::MockTeamClient;
use crate::server::server_api::workspace::MockWorkspaceClient;
use crate::server::server_api::ServerApiProvider;
use crate::server::telemetry::context_provider::AppTelemetryContextProvider;
use crate::settings_view::keybindings::KeybindingChangedNotifier;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::vim_registers::VimRegisters;
use crate::workspace::sync_inputs::SyncedInputState;
use crate::workspace::ActiveSession;
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::NotebookKeybindings;

#[derive(Default)]
struct TestView;

impl warpui::Entity for TestView {
    type Event = ();
}

impl warpui::View for TestView {
    fn render(&self, _: &warpui::AppContext) -> Box<dyn warpui::Element> {
        Empty::new().finish()
    }

    fn ui_name() -> &'static str {
        "TestView"
    }
}

impl warpui::TypedActionView for TestView {
    type Action = ();
}

#[test]
fn test_search_across_multiple_editors() {
    App::test((), |mut app| async move {
        initialize_test_app(&mut app);

        let editor1 =
            create_editor_with_content(&mut app, "hello world\ntest content\nhello again");
        let editor2 = create_editor_with_content(&mut app, "no matches here\njust some text");
        let editor3 = create_editor_with_content(&mut app, "hello from editor 3\nmore hello");

        let editor1_id = editor1.id();
        let editor2_id = editor2.id();
        let editor3_id = editor3.id();

        let editor_handles = vec![editor1, editor2, editor3];

        let model = create_find_model_with_query(&mut app, "hello", false, false);

        run_search_and_wait(&mut app, &model, editor_handles.into_iter()).await;

        app.read(|ctx| {
            let m = model.as_ref(ctx);
            let results = m.results.as_ref().expect("Should have results");

            assert_eq!(results.len(), 4, "Should find 4 matches for 'hello'");

            let editor1_matches: Vec<_> = results
                .iter()
                .filter(|m| m.editor_id == editor1_id)
                .collect();
            assert_eq!(editor1_matches.len(), 2);

            let editor2_matches: Vec<_> = results
                .iter()
                .filter(|m| m.editor_id == editor2_id)
                .collect();
            assert_eq!(editor2_matches.len(), 0);

            let editor3_matches: Vec<_> = results
                .iter()
                .filter(|m| m.editor_id == editor3_id)
                .collect();
            assert_eq!(editor3_matches.len(), 2);
        });
    });
}

#[test]
fn test_case_sensitive_search() {
    App::test((), |mut app| async move {
        initialize_test_app(&mut app);

        let editor1 = create_editor_with_content(&mut app, "Hello HELLO hello");
        let editor_handles = vec![editor1.clone()];

        let model = create_find_model_with_query(&mut app, "hello", false, false);

        run_search_and_wait(&mut app, &model, editor_handles.iter().cloned()).await;

        app.read(|ctx| {
            let results = model.as_ref(ctx).results.as_ref().unwrap();
            assert_eq!(results.len(), 3, "Case insensitive should find all 3");
        });

        let model2 = create_find_model_with_query(&mut app, "hello", false, true);

        run_search_and_wait(&mut app, &model2, editor_handles.into_iter()).await;

        app.read(|ctx| {
            let results = model2.as_ref(ctx).results.as_ref().unwrap();
            assert_eq!(results.len(), 1, "Case sensitive should find only 1");
        });
    });
}

#[test]
fn test_regex_search() {
    App::test((), |mut app| async move {
        initialize_test_app(&mut app);

        let editor1 = create_editor_with_content(&mut app, "test123 test456 test789");
        let editor_handles = vec![editor1];

        let model = create_find_model_with_query(&mut app, r"test\d+", true, false);

        run_search_and_wait(&mut app, &model, editor_handles.into_iter()).await;

        app.read(|ctx| {
            let results = model.as_ref(ctx).results.as_ref().unwrap();
            assert_eq!(results.len(), 3, "Regex should find 3 matches");
        });
    });
}

/// Initialize required singletons for testing LocalCodeEditorView
fn initialize_test_app(app: &mut App) {
    initialize_settings_for_tests(app);
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(AppTelemetryContextProvider::new_context_provider);
    app.add_singleton_model(|_| Appearance::mock());
    app.add_singleton_model(|_| SyncedInputState::mock());
    app.add_singleton_model(|_| VimRegisters::new());
    app.add_singleton_model(|_| KeybindingChangedNotifier::mock());
    app.add_singleton_model(|_| DetectedRepositories::default());
    app.add_singleton_model(|_| GlobalCodeReviewModel);
    app.add_singleton_model(|ctx| {
        UserWorkspaces::mock(
            Arc::new(MockTeamClient::new()),
            Arc::new(MockWorkspaceClient::new()),
            vec![],
            ctx,
        )
    });

    // Add mocks required by rich text editor (used in the CommentEditor)
    app.add_singleton_model(CloudModel::mock);
    app.add_singleton_model(|_| ActiveSession::default());
    app.add_singleton_model(NotebookKeybindings::new);

    // CodeReviewView reads AI usage/availability when comments are populated
    // (e.g. to compute the comment tray's "Send to Agent" button state), so
    // register the same AI singletons the other code_review tests use.
    app.add_singleton_model(|_| ServerApiProvider::new_for_test());
    app.add_singleton_model(|ctx| {
        AIRequestUsageModel::new_for_test(ServerApiProvider::as_ref(ctx).get_ai_client(), ctx)
    });
}

fn create_find_model_with_query(
    app: &mut App,
    query_text: &str,
    regex: bool,
    case_sensitive: bool,
) -> ModelHandle<CodeReviewFindModel> {
    let (window_id, _) = app.add_window(WindowStyle::NotStealFocus, |_| TestView);

    let diff_state_model = app.add_model(DiffStateModel::new_for_test);
    let repo_path = PathBuf::from("/tmp/test");
    let working_directories_model = app.add_model(|_| WorkingDirectoriesModel::new());
    let repo_key = LocalOrRemotePath::Local(repo_path);
    let code_review_comment_batch =
        working_directories_model.update(app, |working_directories, ctx| {
            working_directories.get_or_create_code_review_comments(&repo_key, ctx)
        });

    let code_review_view = app.add_view(window_id, |ctx| {
        CodeReviewView::new(
            Some(repo_key),
            diff_state_model,
            code_review_comment_batch,
            None,
            ctx,
        )
    });

    let weak_handle = code_review_view.downgrade();

    app.add_model(|ctx| {
        let mut m = CodeReviewFindModel::new(weak_handle, ctx);
        m.query_text = query_text.to_string();
        m.regex = regex;
        m.case_sensitive = case_sensitive;
        m
    })
}

fn create_editor_with_content(app: &mut App, content: &str) -> ViewHandle<LocalCodeEditorView> {
    let content = content.to_string();
    let (_, local_editor) = app.add_window(WindowStyle::NotStealFocus, move |ctx| {
        let code_editor_view = ctx.add_typed_action_view(|ctx| {
            CodeEditorView::new(
                None,
                None,
                CodeEditorRenderOptions::new(VerticalExpansionBehavior::GrowToMaxHeight),
                ctx,
            )
        });

        code_editor_view.update(ctx, |editor, ctx| {
            editor.reset(InitialBufferState::plain_text(&content), ctx);
        });

        LocalCodeEditorView::new(code_editor_view, None, false, None, ctx)
    });

    local_editor
}

async fn run_search_and_wait(
    app: &mut App,
    model: &ModelHandle<CodeReviewFindModel>,
    editors: impl Iterator<Item = ViewHandle<LocalCodeEditorView>>,
) {
    let search_future = model.update(app, |m, ctx| {
        m.run_search(editors, ctx);
        let future_id = m.search_handle.as_ref().unwrap().future_id();
        ctx.await_spawned_future(future_id)
    });
    search_future.await;
}

#[test]
fn test_clear_selection_when_editor_removed() {
    App::test((), |mut app| async move {
        initialize_test_app(&mut app);

        let editor1 = create_editor_with_content(&mut app, "hello world");
        let editor2 = create_editor_with_content(&mut app, "another file");

        let model = create_find_model_with_query(&mut app, "hello", false, false);

        // Run initial search with both editors
        run_search_and_wait(
            &mut app,
            &model,
            vec![editor1.clone(), editor2.clone()].into_iter(),
        )
        .await;

        // Select the match in editor1
        let selected_result = editor1.update(&mut app, |local_editor, ctx| {
            local_editor.editor().update(ctx, |editor, ctx| {
                editor.searcher.update(ctx, |searcher, ctx| {
                    searcher.select_match_at_offset(CharOffset::from(0), 0, ctx)
                })
            })
        });

        model.update(&mut app, |m, _ctx| {
            m.selected_match = Some(MultiEditorSelectedResult {
                editor_id: editor1.id(),
                selected_result,
            });
        });

        app.read(|ctx| {
            let m = model.as_ref(ctx);
            assert_eq!(m.focused_match_index(), Some(0));
        });

        // Run search again but without editor1 (simulating collapsed file)
        run_search_and_wait(&mut app, &model, vec![editor2].into_iter()).await;

        // Selection should be cleared since the editor is no longer in the view
        app.read(|ctx| {
            let m = model.as_ref(ctx);
            assert_eq!(m.match_count(), 0, "Should find no matches in editor2");
            assert_eq!(
                m.focused_match_index(),
                None,
                "Selection should be cleared when editor is removed"
            );
        });
    });
}
