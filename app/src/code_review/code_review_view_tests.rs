use std::path::{Path, PathBuf};
use std::sync::Arc;

use ai::agent::action::InsertReviewComment;
use chrono::Local;
use lsp::LspManagerModel;
use remote_server::HostId;
use repo_metadata::RepoMetadataModel;
use repo_metadata::repositories::DetectedRepositories;
use repo_metadata::watcher::DirectoryWatcher;
use warp_core::ui::appearance::Appearance;
use warp_editor::content::buffer::InitialBufferState;
use warp_editor::render::element::VerticalExpansionBehavior;
use warp_editor::render::model::LineCount;
use warp_files::FileModel;
use warp_util::remote_path::RemotePath;
use warp_util::standardized_path::StandardizedPath;
use warpui::elements::{Empty, MouseStateHandle};
use warpui::platform::WindowStyle;
use warpui::{App, ViewHandle};

use super::*;
use crate::NotebookKeybindings;
use crate::ai::persisted_workspace::PersistedWorkspace;
use crate::ai::request_usage_model::AIRequestUsageModel;
use crate::auth::AuthStateProvider;
use crate::cloud_object::model::persistence::CloudModel;
use crate::code::buffer_location::LocalOrRemotePath;
use crate::code::editor::view::{CodeEditorRenderOptions, CodeEditorView};
use crate::code::global_buffer_model::GlobalBufferModel;
use crate::code::local_code_editor::LocalCodeEditorView;
use crate::code_review::GlobalCodeReviewModel;
use crate::code_review::comments::{
    AttachedReviewComment, AttachedReviewCommentTarget, CommentId, CommentOrigin,
    ImportedCommentDetails, LineDiffContent, PendingImportedReviewComment,
    PendingImportedReviewCommentTarget, attach_pending_imported_comments,
};
use crate::code_review::diff_size_limits::DiffSize;
use crate::code_review::diff_state::{
    DiffHunk, DiffLine, DiffLineType, DiffStateModel, FileDiff, GitFileStatus, LocalDiffStateModel,
};
use crate::code_review::editor_state::CodeReviewEditorState;
use crate::code_review::git_repo_model::GitRepoModels;
use crate::pane_group::WorkingDirectoriesModel;
use crate::server::server_api::ServerApiProvider;
use crate::server::server_api::team::MockTeamClient;
use crate::server::server_api::workspace::MockWorkspaceClient;
use crate::server::telemetry::context_provider::AppTelemetryContextProvider;
use crate::settings_view::keybindings::KeybindingChangedNotifier;
use crate::terminal::local_shell::LocalShellState;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::vim_registers::VimRegisters;
use crate::workspace::ActiveSession;
use crate::workspace::sync_inputs::SyncedInputState;
use crate::workspaces::user_workspaces::UserWorkspaces;

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

/// Initialize required singletons for testing
fn initialize_test_app(app: &mut App) {
    initialize_settings_for_tests(app);
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(AppTelemetryContextProvider::new_context_provider);
    app.add_singleton_model(|_| Appearance::mock());
    app.add_singleton_model(|_| SyncedInputState::mock());
    app.add_singleton_model(|_| VimRegisters::new());
    app.add_singleton_model(|_| KeybindingChangedNotifier::mock());
    app.add_singleton_model(|_| LspManagerModel::new());
    app.add_singleton_model(|_| DetectedRepositories::default());
    app.add_singleton_model(|_| GitRepoModels::new());
    app.add_singleton_model(|_| LocalShellState::NotLoaded);
    app.add_singleton_model(PersistedWorkspace::new_for_test);
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
    app.add_singleton_model(|_| ServerApiProvider::new_for_test());
    app.add_singleton_model(|ctx| {
        AIRequestUsageModel::new_for_test(ServerApiProvider::as_ref(ctx).get_ai_client(), ctx)
    });
}

/// Creates a LocalCodeEditorView with the given content
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

/// Creates a LocalCodeEditorView with base and current content for diff testing
#[allow(dead_code)]
fn create_editor_with_diff(
    app: &mut App,
    base_content: &str,
    current_content: &str,
) -> ViewHandle<LocalCodeEditorView> {
    let current = current_content.to_string();
    let base = base_content.to_string();
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
            editor.reset(InitialBufferState::plain_text(&current), ctx);
            editor.set_base(&base, true, ctx);
        });

        LocalCodeEditorView::new(code_editor_view, None, false, None, ctx)
    });

    local_editor
}

/// Creates an attached review comment with a Line target
fn create_line_comment(
    file_path: impl Into<PathBuf>,
    line_number: usize,
    line_text: &str,
    comment_content: &str,
) -> AttachedReviewComment {
    let line_count = LineCount::from(line_number);
    AttachedReviewComment {
        id: CommentId::new(),
        content: comment_content.to_string(),
        target: AttachedReviewCommentTarget::Line {
            absolute_file_path: LocalOrRemotePath::Local(file_path.into()),
            line: EditorLineLocation::Current {
                line_number: line_count,
                line_range: line_count..LineCount::from(line_number + 1),
            },
            content: LineDiffContent {
                content: format!("+{line_text}"),
                lines_added: LineCount::from(1),
                lines_removed: LineCount::from(0),
            },
        },
        last_update_time: Local::now(),
        base: None,
        head: None,
        outdated: false,
        origin: CommentOrigin::Native,
    }
}

/// Creates an imported (from GitHub) line comment whose stored `content` is the
/// raw unified-diff line, including its one-char marker. `raw_diff_content`
/// should therefore carry the leading `+`/`-`/space (e.g. `" line 2"` for a
/// context line, `"+added"` for an addition).
fn create_imported_line_comment(
    file_path: impl Into<PathBuf>,
    line_number: usize,
    raw_diff_content: &str,
    comment_content: &str,
) -> AttachedReviewComment {
    let line_count = LineCount::from(line_number);
    AttachedReviewComment {
        id: CommentId::new(),
        content: comment_content.to_string(),
        target: AttachedReviewCommentTarget::Line {
            absolute_file_path: LocalOrRemotePath::Local(file_path.into()),
            line: EditorLineLocation::Current {
                line_number: line_count,
                line_range: line_count..LineCount::from(line_number + 1),
            },
            content: LineDiffContent {
                content: raw_diff_content.to_string(),
                lines_added: LineCount::from(0),
                lines_removed: LineCount::from(0),
            },
        },
        last_update_time: Local::now(),
        base: None,
        head: None,
        outdated: false,
        origin: CommentOrigin::ImportedFromGitHub(ImportedCommentDetails {
            author: "reviewer".to_string(),
            github_comment_id: "1".to_string(),
            github_parent_id: None,
            html_url: None,
        }),
    }
}

/// Creates an attached review comment with a File target
fn create_file_comment(
    file_path: impl Into<PathBuf>,
    comment_content: &str,
) -> AttachedReviewComment {
    AttachedReviewComment {
        id: CommentId::new(),
        content: comment_content.to_string(),
        target: AttachedReviewCommentTarget::File {
            absolute_file_path: LocalOrRemotePath::Local(file_path.into()),
        },
        last_update_time: Local::now(),
        base: None,
        head: None,
        outdated: false,
        origin: CommentOrigin::Native,
    }
}

/// Creates an attached review comment with a General target
fn create_general_comment(comment_content: &str) -> AttachedReviewComment {
    AttachedReviewComment {
        id: CommentId::new(),
        content: comment_content.to_string(),
        target: AttachedReviewCommentTarget::General,
        last_update_time: Local::now(),
        base: None,
        head: None,
        outdated: false,
        origin: CommentOrigin::Native,
    }
}

fn make_pending_comment(
    id: &str,
    author: &str,
    body: &str,
    parent_id: Option<&str>,
    timestamp: &str,
    target: PendingImportedReviewCommentTarget,
) -> PendingImportedReviewComment {
    let mut pending = PendingImportedReviewComment::try_from(InsertReviewComment {
        comment_id: id.to_string(),
        author: author.to_string(),
        comment_body: body.to_string(),
        parent_comment_id: parent_id.map(|s| s.to_string()),
        last_modified_timestamp: timestamp.to_string(),
        comment_location: None,
        html_url: None,
    })
    .expect("valid pending import conversion");

    // Override the location target since we intentionally use `comment_location: None` above.
    pending.target = target;

    pending
}

use crate::view_components::action_button::{ActionButton, NakedTheme};

/// Test context that holds all common test state
struct TestContext {
    repo_path: PathBuf,
    repo_location: LocalOrRemotePath,
    #[allow(dead_code)]
    window_id: warpui::WindowId,
    state: LoadedState,
    code_review_view: ViewHandle<CodeReviewView>,
}

impl TestContext {
    /// Initialize common test state with a single file editor
    fn new(app: &mut App, file_path: impl Into<String>, editor_content: &str) -> Self {
        Self::new_at_repo(
            app,
            PathBuf::from("/repo"),
            file_path.into(),
            editor_content,
        )
    }

    fn new_at_repo(
        app: &mut App,
        repo_path: PathBuf,
        file_path: String,
        editor_content: &str,
    ) -> Self {
        initialize_test_app(app);

        let editor = create_editor_with_content(app, editor_content);

        let (window_id, _) = app.add_window(WindowStyle::NotStealFocus, |_| TestView);
        let state = create_loaded_state_with_editors(app, window_id, vec![(file_path, editor)]);

        let diff_state_model = app.add_model(DiffStateModel::new_for_test);

        let working_directories_model = app.add_model(|_| WorkingDirectoriesModel::new());
        let repo_key = LocalOrRemotePath::Local(repo_path.clone());
        let code_review_comment_batch =
            working_directories_model.update(app, |working_directories, ctx| {
                working_directories.get_or_create_code_review_comments(&repo_key, ctx)
            });

        let code_review_view = app.add_view(window_id, |ctx| {
            CodeReviewView::new(
                Some(repo_key.clone()),
                diff_state_model,
                code_review_comment_batch,
                None,
                ctx,
            )
        });

        Self {
            repo_path: repo_path.clone(),
            repo_location: LocalOrRemotePath::Local(repo_path),
            window_id,
            state,
            code_review_view,
        }
    }
}

fn initialize_file_loading_models(app: &mut App) {
    app.add_singleton_model(DirectoryWatcher::new);
    app.add_singleton_model(RepoMetadataModel::new);
    app.add_singleton_model(FileModel::new);
    app.add_singleton_model(GlobalBufferModel::new);
}

async fn create_modified_repo(
    file_path: &str,
    base_content: &str,
    current_content: &str,
) -> tempfile::TempDir {
    let repo = tempfile::tempdir().expect("create temp repo");
    let repo_path = repo.path();
    warp_util::git::run_git_command(repo_path, &["init", "-b", "main"])
        .await
        .expect("initialize git repository");
    warp_util::git::run_git_command(repo_path, &["config", "user.email", "test@test.com"])
        .await
        .expect("configure git email");
    warp_util::git::run_git_command(repo_path, &["config", "user.name", "Test"])
        .await
        .expect("configure git name");
    std::fs::write(repo_path.join(file_path), base_content).expect("write base content");
    warp_util::git::run_git_command(repo_path, &["add", file_path])
        .await
        .expect("stage base content");
    warp_util::git::run_git_command(repo_path, &["commit", "-m", "initial"])
        .await
        .expect("commit base content");
    std::fs::write(repo_path.join(file_path), current_content).expect("write current content");
    repo
}

async fn retrieve_collapsed_file(repo_path: &Path, file_path: &str) -> FileDiffAndContent {
    let (_, file) = LocalDiffStateModel::retrieve_diff_state(
        repo_path,
        &repo_path.join(file_path),
        &DiffMode::Head,
        None,
    )
    .await
    .expect("retrieve file diff");
    let mut file = Arc::try_unwrap(file.expect("file should be part of diff"))
        .expect("test should own the only diff reference");
    file.file_diff.is_autogenerated = true;
    file
}

async fn wait_for_deferred_load(app: &mut App, view: &ViewHandle<CodeReviewView>, file_path: &str) {
    for _ in 0..5000 {
        let finished = view.read(app, |view, ctx| {
            let CodeReviewViewState::Loaded(state) = view.state() else {
                return false;
            };
            let index = state
                .file_states
                .get_index_of(file_path)
                .expect("file should remain in loaded state");
            let _ = view.render_diff_at_index(index, ScrollOffset::default(), ctx);
            let file = &state.file_states[file_path];
            file.editor_state
                .as_ref()
                .is_some_and(CodeReviewEditorState::is_loaded)
                || matches!(file.deferred_editor_load, DeferredEditorLoad::Failed(_))
        });
        if finished {
            return;
        }
        futures_lite::future::yield_now().await;
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    panic!("timed out waiting for deferred editor load");
}

/// Creates a minimal LoadedState with file states containing editors.
/// Must be called within an App context.
fn create_loaded_state_with_editors(
    app: &mut App,
    window_id: warpui::WindowId,
    file_editors: Vec<(String, ViewHandle<LocalCodeEditorView>)>,
) -> LoadedState {
    let file_states = file_editors
        .into_iter()
        .map(|(file_path, editor)| {
            let chevron_button = app.add_view(window_id, |_| ActionButton::new("", NakedTheme));
            let open_in_tab_button = app.add_view(window_id, |_| ActionButton::new("", NakedTheme));
            let discard_button = app.add_view(window_id, |_| ActionButton::new("", NakedTheme));
            let add_context_button = app.add_view(window_id, |_| ActionButton::new("", NakedTheme));
            let copy_path_button = app.add_view(window_id, |_| ActionButton::new("", NakedTheme));

            let state = FileState {
                file_diff: FileDiff {
                    file_path: file_path.clone(),
                    status: GitFileStatus::Modified,
                    hunks: Arc::new(vec![]),
                    is_binary: false,
                    is_autogenerated: false,
                    max_line_number: 0,
                    has_hidden_bidi_chars: false,
                    size: DiffSize::Normal,
                },
                editor_state: Some(CodeReviewEditorState::new_loaded(editor)),
                deferred_editor_load: DeferredEditorLoad::NotDeferred,
                is_expanded: true,
                sidebar_mouse_state: MouseStateHandle::default(),
                header_mouse_state: MouseStateHandle::default(),
                chevron_button,
                open_in_tab_button,
                discard_button,
                add_context_button,
                copy_path_button,
            };
            (file_path, state)
        })
        .collect();

    LoadedState {
        file_states,
        total_additions: 0,
        total_deletions: 0,
        files_changed: 0,
        initial_editors_loading: false,
    }
}

fn code_review_file(file_path: &str, content: &str, is_autogenerated: bool) -> FileDiffAndContent {
    let content_line_count = content.lines().count();
    let deleted_lines = content
        .lines()
        .enumerate()
        .map(|(index, text)| DiffLine {
            line_type: DiffLineType::Delete,
            old_line_number: Some(index + 1),
            new_line_number: None,
            text: text.to_owned(),
            no_trailing_newline: index + 1 == content_line_count && !content.ends_with('\n'),
        })
        .collect::<Vec<_>>();
    let line_count = deleted_lines.len();
    FileDiffAndContent {
        file_diff: FileDiff {
            file_path: file_path.to_string(),
            status: GitFileStatus::Deleted,
            hunks: Arc::new(vec![DiffHunk {
                old_start_line: usize::from(line_count > 0),
                old_line_count: line_count,
                new_start_line: 0,
                new_line_count: 0,
                lines: deleted_lines,
                unified_diff_start: 0,
                unified_diff_end: 0,
            }]),
            is_binary: false,
            is_autogenerated,
            max_line_number: line_count,
            has_hidden_bidi_chars: false,
            size: DiffSize::Normal,
        },
        content_at_head: Some(content.to_string()),
    }
}

fn install_diff_state(
    view: &mut CodeReviewView,
    diff_data: Arc<GitDiffWithBaseContent>,
    ctx: &mut ViewContext<CodeReviewView>,
) {
    let file_states_vec = view.build_view_state_for_file_diffs(diff_data.files.iter(), ctx);
    let initial_editors_loading = file_states_vec.iter().any(|file| {
        file.is_expanded
            && file
                .editor_state
                .as_ref()
                .is_some_and(|editor| !editor.is_loaded())
    });
    let file_states = file_states_vec
        .into_iter()
        .map(|state| (state.file_diff.file_path.clone(), state))
        .collect();
    let files_changed = diff_data.files.len();

    view.active_repo.as_mut().unwrap().state = CodeReviewViewState::Loaded(LoadedState {
        file_states,
        total_additions: 0,
        total_deletions: 0,
        files_changed,
        initial_editors_loading,
    });
}

#[test]
fn test_collapsed_file_does_not_construct_editor() {
    App::test((), |mut app| async move {
        let test = TestContext::new(&mut app, "existing.txt", "existing");
        let diff_data = Arc::new(GitDiffWithBaseContent {
            files: vec![code_review_file("generated.rs", "fn generated() {}", true)],
            total_additions: 0,
            total_deletions: 0,
            files_changed: 1,
        });

        test.code_review_view.update(&mut app, |view, ctx| {
            install_diff_state(view, diff_data, ctx);

            let CodeReviewViewState::Loaded(state) = view.state() else {
                panic!("expected loaded state");
            };
            let file = &state.file_states["generated.rs"];
            assert!(!file.is_expanded);
            assert!(file.editor_state.is_none());
            assert_eq!(file.deferred_editor_load, DeferredEditorLoad::Ready);
            assert!(view.all_editors_loaded());
        });
    });
}

#[test]
fn test_remote_collapsed_file_constructs_editor_eagerly() {
    App::test((), |mut app| async move {
        let test = TestContext::new(&mut app, "existing.txt", "existing");
        let diff_data = Arc::new(GitDiffWithBaseContent {
            files: vec![code_review_file(
                "generated.rs",
                "fn generated() {}\n",
                true,
            )],
            total_additions: 0,
            total_deletions: 0,
            files_changed: 1,
        });

        test.code_review_view.update(&mut app, |view, ctx| {
            view.active_repo.as_mut().unwrap().repo_path =
                LocalOrRemotePath::Remote(RemotePath::new(
                    HostId::new("test-host".to_string()),
                    StandardizedPath::try_new("/repo").expect("valid remote repo path"),
                ));
            assert!(!view.can_defer_editor_construction());
            install_diff_state(view, diff_data, ctx);

            let CodeReviewViewState::Loaded(state) = view.state() else {
                panic!("expected loaded state");
            };
            let file = &state.file_states["generated.rs"];
            assert!(!file.is_expanded);
            assert!(file.editor_state.is_some());
            assert_eq!(file.deferred_editor_load, DeferredEditorLoad::NotDeferred);
        });
    });
}

#[test]
fn test_collapsed_file_does_not_retain_diff_snapshot() {
    App::test((), |mut app| async move {
        let test = TestContext::new(&mut app, "existing.txt", "existing");
        let diff_data = Arc::new(GitDiffWithBaseContent {
            files: vec![code_review_file(
                "generated.rs",
                &"large base content\n".repeat(1024),
                true,
            )],
            total_additions: 0,
            total_deletions: 0,
            files_changed: 1,
        });
        let weak_diff_data = Arc::downgrade(&diff_data);

        test.code_review_view.update(&mut app, |view, ctx| {
            install_diff_state(view, diff_data, ctx);
            assert!(weak_diff_data.upgrade().is_none());
        });
    });
}

#[test]
fn test_comment_for_deferred_file_is_not_marked_outdated() {
    App::test((), |mut app| async move {
        let test = TestContext::new(&mut app, "existing.txt", "existing");
        let diff_data = Arc::new(GitDiffWithBaseContent {
            files: vec![code_review_file("generated.rs", "fn generated() {}", true)],
            total_additions: 0,
            total_deletions: 0,
            files_changed: 1,
        });
        let comment = create_line_comment(
            "/repo/generated.rs",
            0,
            "fn generated() {}",
            "Review generated code",
        );

        test.code_review_view.update(&mut app, |view, ctx| {
            install_diff_state(view, diff_data, ctx);
            let CodeReviewViewState::Loaded(state) = view.state() else {
                panic!("expected loaded state");
            };

            let result = CodeReviewView::relocate_comments(
                vec![comment],
                state,
                &LocalOrRemotePath::Local(PathBuf::from("/repo")),
                ctx,
            );

            assert_eq!(result.comments.len(), 1);
            assert!(!result.comments[0].outdated);
            assert_eq!(result.fallback_count, 0);
        });
    });
}

#[test]
fn test_expanding_collapsed_file_constructs_and_renders_loaded_editor() {
    App::test((), |mut app| async move {
        let repo =
            create_modified_repo("generated.rs", "fn base() {}\n", "fn generated() {}\n").await;
        let test = TestContext::new_at_repo(
            &mut app,
            repo.path().to_path_buf(),
            "existing.txt".to_string(),
            "existing",
        );
        initialize_file_loading_models(&mut app);
        let file = retrieve_collapsed_file(repo.path(), "generated.rs").await;
        let diff_data = Arc::new(GitDiffWithBaseContent {
            files: vec![file],
            total_additions: 0,
            total_deletions: 0,
            files_changed: 1,
        });

        test.code_review_view.update(&mut app, |view, ctx| {
            install_diff_state(view, diff_data, ctx);

            view.handle_action(
                &CodeReviewAction::ToggleFileExpanded("generated.rs".to_string()),
                ctx,
            );

            let CodeReviewViewState::Loaded(state) = view.state() else {
                panic!("expected loaded state");
            };
            let file = &state.file_states["generated.rs"];
            assert!(file.is_expanded);
            assert!(file.editor_state.is_none());
            assert!(matches!(
                file.deferred_editor_load,
                DeferredEditorLoad::Loading(_)
            ));
            assert_eq!(
                view.active_repo
                    .as_ref()
                    .and_then(|repo| repo.file_expanded.get("generated.rs")),
                Some(&true)
            );
            let _rendered_diff = view.render_diff_at_index(0, ScrollOffset::default(), ctx);
        });

        wait_for_deferred_load(&mut app, &test.code_review_view, "generated.rs").await;

        test.code_review_view.read(&app, |view, ctx| {
            let CodeReviewViewState::Loaded(state) = view.state() else {
                panic!("expected loaded state");
            };
            let file = &state.file_states["generated.rs"];
            let editor_state = file
                .editor_state
                .as_ref()
                .expect("editor should be created");
            assert!(editor_state.is_loaded());
            assert_eq!(file.deferred_editor_load, DeferredEditorLoad::NotDeferred);
            assert_eq!(
                editor_state
                    .editor()
                    .as_ref(ctx)
                    .editor()
                    .as_ref(ctx)
                    .text(ctx)
                    .into_string(),
                "fn generated() {}\n"
            );
            let base = editor_state
                .editor()
                .as_ref(ctx)
                .editor()
                .as_ref(ctx)
                .model
                .as_ref(ctx)
                .diff()
                .as_ref(ctx)
                .base()
                .expect("editor should have an authoritative base");
            assert_eq!(base.as_str(), "fn base() {}\n");
            assert_eq!(view.editor_handles().count(), 1);
        });
    });
}

#[test]
fn test_auto_expanded_file_constructs_editor_eagerly() {
    App::test((), |mut app| async move {
        let test = TestContext::new(&mut app, "existing.txt", "existing");
        let diff_data = Arc::new(GitDiffWithBaseContent {
            files: vec![code_review_file("source.rs", "fn source() {}\n", false)],
            total_additions: 0,
            total_deletions: 0,
            files_changed: 1,
        });

        test.code_review_view.update(&mut app, |view, ctx| {
            install_diff_state(view, diff_data, ctx);

            let CodeReviewViewState::Loaded(state) = view.state() else {
                panic!("expected loaded state");
            };
            let file = &state.file_states["source.rs"];
            assert!(file.is_expanded);
            assert!(
                file.editor_state
                    .as_ref()
                    .is_some_and(|state| state.is_loaded())
            );
            assert_eq!(file.deferred_editor_load, DeferredEditorLoad::NotDeferred);
        });
    });
}

#[test]
fn test_comment_actions_wait_for_authoritative_deferred_editor_load() {
    App::test((), |mut app| async move {
        let repo = create_modified_repo("generated.rs", "first\nbase\n", "first\nsecond\n").await;
        let test = TestContext::new_at_repo(
            &mut app,
            repo.path().to_path_buf(),
            "existing.txt".to_string(),
            "existing",
        );
        initialize_file_loading_models(&mut app);
        let file = retrieve_collapsed_file(repo.path(), "generated.rs").await;
        let diff_data = Arc::new(GitDiffWithBaseContent {
            files: vec![file],
            total_additions: 0,
            total_deletions: 0,
            files_changed: 1,
        });
        let comment = create_line_comment(
            repo.path().join("generated.rs"),
            1,
            "second",
            "Review generated code",
        );
        let comment_id = comment.id;

        test.code_review_view.update(&mut app, |view, ctx| {
            install_diff_state(view, diff_data, ctx);
            view.active_comment_model
                .clone()
                .unwrap()
                .update(ctx, |batch, ctx| batch.upsert_comment(comment, ctx));

            view.handle_jump_to_comment_location(&comment_id, ctx);
            view.handle_edit_comment(&comment_id, ctx);

            let CodeReviewViewState::Loaded(state) = view.state() else {
                panic!("expected loaded state");
            };
            let file = &state.file_states["generated.rs"];
            assert!(file.is_expanded);
            assert!(file.editor_state.is_none());
            assert!(matches!(
                file.deferred_editor_load,
                DeferredEditorLoad::Loading(_)
            ));
            assert_eq!(view.pending_jump_to_comment, Some(comment_id));
            assert_eq!(view.pending_edit_comment, Some(comment_id));
        });

        wait_for_deferred_load(&mut app, &test.code_review_view, "generated.rs").await;

        test.code_review_view.read(&app, |view, _| {
            assert!(view.pending_jump_to_comment.is_none());
            assert!(view.pending_edit_comment.is_none());
            assert_eq!(view.viewported_list_state.get_scroll_index(), 0);
            assert!(
                !view
                    .active_repo
                    .as_ref()
                    .unwrap()
                    .file_expanded
                    .contains_key("generated.rs")
            );
        });
    });
}

#[test]
fn test_deferred_editor_load_keeps_loaded_sibling_visible() {
    App::test((), |mut app| async move {
        let repo = create_modified_repo("generated.rs", "base\n", "generated\n").await;
        let test = TestContext::new_at_repo(
            &mut app,
            repo.path().to_path_buf(),
            "existing.txt".to_string(),
            "existing",
        );
        initialize_file_loading_models(&mut app);

        let loaded_file = code_review_file("source.rs", "source\n", false);
        let deferred_file = retrieve_collapsed_file(repo.path(), "generated.rs").await;
        let diff_data = Arc::new(GitDiffWithBaseContent {
            files: vec![loaded_file, deferred_file],
            total_additions: 0,
            total_deletions: 0,
            files_changed: 2,
        });

        test.code_review_view.update(&mut app, |view, ctx| {
            install_diff_state(view, diff_data, ctx);

            let CodeReviewViewState::Loaded(state) = view.state() else {
                panic!("expected loaded state");
            };
            assert!(!state.initial_editors_loading);
            assert!(
                state.file_states["source.rs"]
                    .editor_state
                    .as_ref()
                    .is_some_and(CodeReviewEditorState::is_loaded)
            );

            view.set_file_expanded(1, true, true, ctx);

            let CodeReviewViewState::Loaded(state) = view.state() else {
                panic!("expected loaded state");
            };
            assert!(!state.initial_editors_loading);
            assert!(
                state.file_states["source.rs"]
                    .editor_state
                    .as_ref()
                    .is_some_and(CodeReviewEditorState::is_loaded)
            );
            assert!(state.file_states["generated.rs"].editor_state.is_none());
            assert!(matches!(
                state.file_states["generated.rs"].deferred_editor_load,
                DeferredEditorLoad::Loading(_)
            ));
            let _loaded_panel = view.render(ctx);
            let _loaded_sibling = view.render_diff_at_index(0, ScrollOffset::default(), ctx);
        });

        wait_for_deferred_load(&mut app, &test.code_review_view, "generated.rs").await;

        test.code_review_view.read(&app, |view, ctx| {
            let CodeReviewViewState::Loaded(state) = view.state() else {
                panic!("expected loaded state");
            };
            assert!(
                state.file_states["source.rs"]
                    .editor_state
                    .as_ref()
                    .is_some_and(CodeReviewEditorState::is_loaded)
            );
            assert!(
                state.file_states["generated.rs"]
                    .editor_state
                    .as_ref()
                    .is_some_and(CodeReviewEditorState::is_loaded)
            );
            let _loaded_sibling = view.render_diff_at_index(0, ScrollOffset::default(), ctx);
        });
    });
}

#[test]
fn test_deferred_load_uses_fresh_diff_with_preloaded_global_buffer() {
    App::test((), |mut app| async move {
        let repo = create_modified_repo("generated.rs", "base\n", "first edit\n").await;
        let test = TestContext::new_at_repo(
            &mut app,
            repo.path().to_path_buf(),
            "existing.txt".to_string(),
            "existing",
        );
        initialize_file_loading_models(&mut app);
        let stale_file = retrieve_collapsed_file(repo.path(), "generated.rs").await;
        let diff_data = Arc::new(GitDiffWithBaseContent {
            files: vec![stale_file],
            total_additions: 0,
            total_deletions: 0,
            files_changed: 1,
        });
        test.code_review_view.update(&mut app, |view, ctx| {
            install_diff_state(view, diff_data, ctx);
        });

        std::fs::write(repo.path().join("generated.rs"), "second edit\n")
            .expect("write newer collapsed-file content");
        let buffer_state = GlobalBufferModel::handle(&app).update(&mut app, |model, ctx| {
            model.open(
                LocalOrRemotePath::Local(repo.path().join("generated.rs")),
                ctx,
            )
        });
        let version = ContentVersion::new();
        GlobalBufferModel::handle(&app).update(&mut app, |model, ctx| {
            model.populate_buffer_with_read_content(
                buffer_state.file_id,
                "second edit\n",
                version,
                version,
                true,
                ctx,
            );
        });
        assert!(
            GlobalBufferModel::handle(&app)
                .read(&app, |model, _| model.buffer_loaded(buffer_state.file_id))
        );

        test.code_review_view.update(&mut app, |view, ctx| {
            view.handle_action(
                &CodeReviewAction::ToggleFileExpanded("generated.rs".to_string()),
                ctx,
            );
        });
        wait_for_deferred_load(&mut app, &test.code_review_view, "generated.rs").await;

        test.code_review_view.read(&app, |view, ctx| {
            let CodeReviewViewState::Loaded(state) = view.state() else {
                panic!("expected loaded state");
            };
            let file = &state.file_states["generated.rs"];
            assert!(
                file.file_diff
                    .hunks
                    .iter()
                    .flat_map(|hunk| &hunk.lines)
                    .any(|line| line.line_type == DiffLineType::Add && line.text == "second edit")
            );
            let editor = file
                .editor_state
                .as_ref()
                .expect("editor should load")
                .editor();
            assert_eq!(
                editor.as_ref(ctx).editor().as_ref(ctx).text(ctx).as_str(),
                "second edit\n"
            );
            let base = editor
                .as_ref(ctx)
                .editor()
                .as_ref(ctx)
                .model
                .as_ref(ctx)
                .diff()
                .as_ref(ctx)
                .base()
                .expect("editor should have an authoritative base");
            assert_eq!(base.as_str(), "base\n");
        });
        drop(buffer_state);
    });
}

#[test]
fn test_deferred_load_failure_remains_retryable_without_editor() {
    App::test((), |mut app| async move {
        let repo = create_modified_repo("tracked.rs", "base\n", "changed\n").await;
        let test = TestContext::new_at_repo(
            &mut app,
            repo.path().to_path_buf(),
            "existing.txt".to_string(),
            "existing",
        );
        let diff_data = Arc::new(GitDiffWithBaseContent {
            files: vec![code_review_file("missing.rs", "stale base\n", true)],
            total_additions: 0,
            total_deletions: 0,
            files_changed: 1,
        });
        test.code_review_view.update(&mut app, |view, ctx| {
            install_diff_state(view, diff_data, ctx);
            view.handle_action(
                &CodeReviewAction::ToggleFileExpanded("missing.rs".to_string()),
                ctx,
            );
        });

        wait_for_deferred_load(&mut app, &test.code_review_view, "missing.rs").await;

        test.code_review_view.read(&app, |view, ctx| {
            let CodeReviewViewState::Loaded(state) = view.state() else {
                panic!("expected loaded state");
            };
            let file = &state.file_states["missing.rs"];
            assert!(file.editor_state.is_none());
            assert!(matches!(
                file.deferred_editor_load,
                DeferredEditorLoad::Failed(_)
            ));
            assert!(!view.all_editors_loaded());
            let _error_state = view.render_diff_at_index(0, ScrollOffset::default(), ctx);
        });

        test.code_review_view.update(&mut app, |view, ctx| {
            view.set_file_expanded(0, false, true, ctx);
            let CodeReviewViewState::Loaded(state) = view.state() else {
                panic!("expected loaded state");
            };
            assert_eq!(
                state.file_states["missing.rs"].deferred_editor_load,
                DeferredEditorLoad::Ready
            );
        });
    });
}

#[test]
fn test_relocate_comments_empty_input() {
    App::test((), |mut app| async move {
        let ctx = TestContext::new(&mut app, "test.txt", "line 1\nline 2\nline 3");

        ctx.code_review_view.update(&mut app, |_view, view_ctx| {
            let RelocateCommentsResult {
                comments: relocated,
                fallback_count: fallbacks,
            } = CodeReviewView::relocate_comments(vec![], &ctx.state, &ctx.repo_location, view_ctx);

            assert!(
                relocated.is_empty(),
                "Empty input should return empty output"
            );
            assert_eq!(fallbacks, 0, "Empty input should have no fallbacks");
        });
    });
}

#[test]
fn test_relocate_comments_general_comment_passes_through() {
    App::test((), |mut app| async move {
        let ctx = TestContext::new(&mut app, "test.txt", "line 1\nline 2\nline 3");

        let general_comment = create_general_comment("This is a general comment");
        let original_id = general_comment.id;

        ctx.code_review_view.update(&mut app, |_view, view_ctx| {
            let RelocateCommentsResult {
                comments: relocated,
                fallback_count: fallbacks,
            } = CodeReviewView::relocate_comments(
                vec![general_comment],
                &ctx.state,
                &ctx.repo_location,
                view_ctx,
            );

            assert_eq!(relocated.len(), 1, "Should return the comment");
            assert_eq!(relocated[0].id, original_id, "Should preserve comment ID");
            assert!(
                matches!(relocated[0].target, AttachedReviewCommentTarget::General),
                "General comment should remain General"
            );
            assert_eq!(
                fallbacks, 0,
                "General comments should not count as fallbacks"
            );
        });
    });
}

#[test]
fn test_relocate_comments_file_comment_passes_through() {
    App::test((), |mut app| async move {
        let file_path = "test.txt";
        let ctx = TestContext::new(&mut app, file_path, "line 1\nline 2\nline 3");

        let file_comment =
            create_file_comment(ctx.repo_path.join(file_path), "This is a file comment");
        let original_id = file_comment.id;

        ctx.code_review_view.update(&mut app, |_view, view_ctx| {
            let RelocateCommentsResult {
                comments: relocated,
                fallback_count: fallbacks,
            } = CodeReviewView::relocate_comments(
                vec![file_comment],
                &ctx.state,
                &ctx.repo_location,
                view_ctx,
            );

            assert_eq!(relocated.len(), 1, "Should return the comment");
            assert_eq!(relocated[0].id, original_id, "Should preserve comment ID");
            assert!(
                matches!(
                    relocated[0].target,
                    AttachedReviewCommentTarget::File { .. }
                ),
                "File comment should remain File"
            );
            assert_eq!(fallbacks, 0, "File comments should not count as fallbacks");
        });
    });
}

#[test]
fn test_relocate_comments_line_comment_no_matching_editor_marked_outdated() {
    App::test((), |mut app| async move {
        // Editor is for "test.txt" but comment is for "other.txt"
        let ctx = TestContext::new(&mut app, "test.txt", "line 1\nline 2\nline 3");

        let line_comment =
            create_line_comment("/repo/other.txt", 1, "line 1", "Comment on other file");
        let original_id = line_comment.id;

        ctx.code_review_view.update(&mut app, |_view, view_ctx| {
            let RelocateCommentsResult {
                comments: relocated,
                fallback_count: fallbacks,
            } = CodeReviewView::relocate_comments(
                vec![line_comment],
                &ctx.state,
                &ctx.repo_location,
                view_ctx,
            );

            assert_eq!(
                relocated.len(),
                1,
                "Comment with no matching editor should be kept but marked outdated"
            );
            assert_eq!(relocated[0].id, original_id, "Should preserve comment ID");
            assert!(
                relocated[0].outdated,
                "Comment should be marked as outdated"
            );
            assert_eq!(
                fallbacks, 0,
                "Outdated comments should not count as fallbacks"
            );
        });
    });
}

#[test]
fn test_relocate_comments_multiple_comment_types() {
    App::test((), |mut app| async move {
        let file_path = "test.txt";
        let ctx = TestContext::new(&mut app, file_path, "line 1\nline 2\nline 3");

        let general_comment = create_general_comment("General comment");
        let file_comment = create_file_comment(ctx.repo_path.join(file_path), "File comment");
        let line_comment = create_line_comment("/repo/test.txt", 1, "line 1", "Line comment");

        let general_id = general_comment.id;
        let file_id = file_comment.id;
        let line_id = line_comment.id;

        ctx.code_review_view.update(&mut app, |_view, view_ctx| {
            let comments = vec![general_comment, file_comment, line_comment];
            let RelocateCommentsResult {
                comments: relocated,
                fallback_count: _,
            } = CodeReviewView::relocate_comments(
                comments,
                &ctx.state,
                &ctx.repo_location,
                view_ctx,
            );

            assert_eq!(
                relocated.len(),
                3,
                "Should return all comments (general, file, and line)"
            );

            // Find each comment by ID
            let relocated_general = relocated.iter().find(|c| c.id == general_id).unwrap();
            let relocated_file = relocated.iter().find(|c| c.id == file_id).unwrap();
            let relocated_line = relocated.iter().find(|c| c.id == line_id).unwrap();

            assert!(matches!(
                relocated_general.target,
                AttachedReviewCommentTarget::General
            ));
            assert!(matches!(
                relocated_file.target,
                AttachedReviewCommentTarget::File { .. }
            ));
            assert!(matches!(
                relocated_line.target,
                AttachedReviewCommentTarget::Line { .. }
            ));
        });
    });
}

#[test]
fn test_relocate_comments_line_comment_with_absolute_path() {
    App::test((), |mut app| async move {
        let ctx = TestContext::new(&mut app, "test.txt", "line 1\nline 2\nline 3");

        // Comment with absolute path matching the editor's file
        let line_comment = create_line_comment("/repo/test.txt", 1, "line 1", "Line comment");
        let original_id = line_comment.id;

        ctx.code_review_view.update(&mut app, |_view, view_ctx| {
            let RelocateCommentsResult {
                comments: relocated,
                fallback_count: _,
            } = CodeReviewView::relocate_comments(
                vec![line_comment],
                &ctx.state,
                &ctx.repo_location,
                view_ctx,
            );

            assert_eq!(
                relocated.len(),
                1,
                "Comment with absolute path should be relocated"
            );
            assert_eq!(relocated[0].id, original_id, "Should preserve comment ID");
            assert!(
                matches!(
                    relocated[0].target,
                    AttachedReviewCommentTarget::Line { .. }
                ),
                "Line comment should remain Line"
            );
        });
    });
}

#[test]
fn test_attach_pending_imported_comment_formats_body_and_uses_absolute_path() {
    let repo_path = PathBuf::from("/repo");

    let pending = make_pending_comment(
        "1",
        "alice",
        "Hello world",
        None,
        "2024-01-01T00:00:00Z",
        PendingImportedReviewCommentTarget::Line {
            relative_file_path: PathBuf::from("test.txt"),
            line: EditorLineLocation::Current {
                line_number: LineCount::from(1),
                line_range: LineCount::from(1)..LineCount::from(2),
            },
            diff_content: LineDiffContent {
                content: "+line 1".to_string(),
                lines_added: LineCount::from(1),
                lines_removed: LineCount::from(0),
            },
        },
    );

    let repo_location = LocalOrRemotePath::Local(repo_path.clone());
    let attached = attach_pending_imported_comments(vec![pending], &repo_location);

    assert_eq!(attached.len(), 1);
    assert_eq!(attached[0].content, "**@alice**:\nHello world");

    match &attached[0].target {
        AttachedReviewCommentTarget::Line {
            absolute_file_path, ..
        } => {
            assert_eq!(
                *absolute_file_path,
                LocalOrRemotePath::Local(repo_path.join("test.txt")),
            );
        }
        _ => panic!("expected line comment target"),
    }

    match &attached[0].origin {
        CommentOrigin::ImportedFromGitHub(details) => {
            assert_eq!(details.author, "alice");
            assert_eq!(details.github_comment_id, "1");
            assert!(details.github_parent_id.is_none());
        }
        _ => panic!("expected imported origin"),
    }
}

#[test]
fn test_attach_pending_imported_thread_flattens_depth_first_sorted_by_timestamp() {
    let repo_path = PathBuf::from("/repo");

    let root = make_pending_comment(
        "1",
        "alice",
        "Root",
        None,
        "2024-01-01T00:00:00Z",
        PendingImportedReviewCommentTarget::Line {
            relative_file_path: PathBuf::from("test.txt"),
            line: EditorLineLocation::Current {
                line_number: LineCount::from(1),
                line_range: LineCount::from(1)..LineCount::from(2),
            },
            diff_content: LineDiffContent {
                content: "+line 1".to_string(),
                lines_added: LineCount::from(1),
                lines_removed: LineCount::from(0),
            },
        },
    );

    // Earlier reply to the root.
    let reply_early = make_pending_comment(
        "4",
        "dana",
        "Reply early",
        Some("1"),
        "2024-01-01T00:30:00Z",
        PendingImportedReviewCommentTarget::General,
    );

    // Later reply to the root.
    let reply_late = make_pending_comment(
        "2",
        "bob",
        "Reply later",
        Some("1"),
        "2024-01-01T01:00:00Z",
        PendingImportedReviewCommentTarget::General,
    );

    // Reply to the later reply.
    let reply_nested = make_pending_comment(
        "3",
        "charlie",
        "Nested reply",
        Some("2"),
        "2024-01-01T02:00:00Z",
        PendingImportedReviewCommentTarget::General,
    );

    let latest_timestamp = reply_nested.last_update_time;

    let repo_location = LocalOrRemotePath::Local(repo_path.clone());
    let attached = attach_pending_imported_comments(
        vec![reply_late, root, reply_nested, reply_early],
        &repo_location,
    );

    assert_eq!(attached.len(), 1);
    assert_eq!(
        attached[0].content,
        "**@alice**:\nRoot\n---\n**@dana**:\nReply early\n---\n**@bob**:\nReply later\n---\n**@charlie**:\nNested reply"
    );
    assert_eq!(attached[0].last_update_time, latest_timestamp);

    match &attached[0].target {
        AttachedReviewCommentTarget::Line {
            absolute_file_path, ..
        } => {
            assert_eq!(
                *absolute_file_path,
                LocalOrRemotePath::Local(repo_path.join("test.txt")),
            );
        }
        _ => panic!("expected root line target to be preserved"),
    }
}

#[test]
fn test_relocate_comments_file_comment_no_matching_editor_marked_outdated() {
    App::test((), |mut app| async move {
        // Editor is for "test.txt" but comment is for "other.txt"
        let ctx = TestContext::new(&mut app, "test.txt", "line 1\nline 2\nline 3");

        let file_comment = create_file_comment("/repo/other.txt", "Comment on other file");
        let original_id = file_comment.id;

        ctx.code_review_view.update(&mut app, |_view, view_ctx| {
            let RelocateCommentsResult {
                comments: relocated,
                fallback_count: fallbacks,
            } = CodeReviewView::relocate_comments(
                vec![file_comment],
                &ctx.state,
                &ctx.repo_location,
                view_ctx,
            );

            assert_eq!(
                relocated.len(),
                1,
                "File comment with no matching editor should be kept but marked outdated"
            );
            assert_eq!(relocated[0].id, original_id, "Should preserve comment ID");
            assert!(
                relocated[0].outdated,
                "Comment should be marked as outdated"
            );
            assert_eq!(
                fallbacks, 0,
                "Outdated file comments should not count as fallbacks"
            );
        });
    });
}

#[test]
fn test_relocate_comments_line_removed_marked_outdated() {
    App::test((), |mut app| async move {
        // Editor has "line 1\nline 3" (line 2 was removed)
        // Comment was attached to "line 2" which no longer exists
        let ctx = TestContext::new(&mut app, "test.txt", "line 1\nline 3");

        // Create a comment that was attached to "line 2" at line index 1
        let line_comment =
            create_line_comment("/repo/test.txt", 1, "line 2", "Comment on removed line");
        let original_id = line_comment.id;

        ctx.code_review_view.update(&mut app, |_view, view_ctx| {
            let RelocateCommentsResult {
                comments: relocated,
                fallback_count: fallbacks,
            } = CodeReviewView::relocate_comments(
                vec![line_comment],
                &ctx.state,
                &ctx.repo_location,
                view_ctx,
            );

            assert_eq!(
                relocated.len(),
                1,
                "Comment should be kept even when line content is removed"
            );
            assert_eq!(relocated[0].id, original_id, "Should preserve comment ID");
            assert!(
                relocated[0].outdated,
                "Comment should be marked as outdated when line content cannot be found"
            );
            assert_eq!(
                fallbacks, 1,
                "Should count as a fallback when line content cannot be matched"
            );
        });
    });
}

/// Regression test for the reported bug: an imported GitHub PR comment placed on
/// a **context** (unchanged) diff line must NOT be marked `outdated` when the
/// line still exists verbatim in the editor.
///
/// Imported context diff lines are stored with the unified-diff leading-space
/// marker (`" line 2"`). Before the fix, `original_text()` (which strips only
/// `+`/`-`) was used for matching, so `" line 2"` never matched the editor's
/// `"line 2"` and the comment fell back to outdated. The fix routes imported
/// comments through `imported_original_text()`, which also strips the space.
#[test]
fn test_imported_context_line_comment_relocates_and_not_outdated() {
    App::test((), |mut app| async move {
        let ctx = TestContext::new(&mut app, "test.txt", "line 1\nline 2\nline 3");

        // Imported comment on line 2 as a CONTEXT line (leading-space marker).
        let comment = create_imported_line_comment(
            "/repo/test.txt",
            1,
            " line 2",
            "Comment on an unchanged (context) line",
        );

        ctx.code_review_view.update(&mut app, |_view, view_ctx| {
            let RelocateCommentsResult {
                comments: relocated,
                fallback_count: fallbacks,
            } = CodeReviewView::relocate_comments(
                vec![comment],
                &ctx.state,
                &ctx.repo_location,
                view_ctx,
            );

            assert_eq!(relocated.len(), 1, "Comment should be relocated");
            assert!(
                !relocated[0].outdated,
                "Imported context-line comment should NOT be outdated when the line still exists"
            );
            assert_eq!(
                fallbacks, 0,
                "Should have no fallbacks when content matches"
            );
        });
    });
}

/// An imported context-line comment whose line genuinely no longer exists must
/// still fall back to `outdated` — the fix must not defeat real outdated detection.
#[test]
fn test_imported_context_line_comment_removed_marked_outdated() {
    App::test((), |mut app| async move {
        let ctx = TestContext::new(&mut app, "test.txt", "line 1\nline 3");

        // Imported context comment on a line (" old line 2") that no longer exists.
        let comment = create_imported_line_comment(
            "/repo/test.txt",
            1,
            " old line 2",
            "Comment on a since-removed context line",
        );

        ctx.code_review_view.update(&mut app, |_view, view_ctx| {
            let RelocateCommentsResult {
                comments: relocated,
                fallback_count: fallbacks,
            } = CodeReviewView::relocate_comments(
                vec![comment],
                &ctx.state,
                &ctx.repo_location,
                view_ctx,
            );

            assert_eq!(relocated.len(), 1, "Comment should be kept");
            assert!(
                relocated[0].outdated,
                "Imported comment should be outdated when its line no longer exists"
            );
            assert_eq!(
                fallbacks, 1,
                "Should count as a fallback when content is gone"
            );
        });
    });
}

/// Guards against a regression from the fix: a NATIVE comment whose content is a
/// genuinely indented source line (leading whitespace is significant, not a diff
/// marker) must keep using `original_text()` and still match / not be outdated.
#[test]
fn test_native_indented_context_comment_not_outdated() {
    App::test((), |mut app| async move {
        let ctx = TestContext::new(&mut app, "test.txt", "fn f() {\n    let x = 1;\n}");

        // Native comment on the indented line; content is the raw line (no marker).
        let line_count = LineCount::from(1);
        let comment = AttachedReviewComment {
            id: CommentId::new(),
            content: "Comment on an indented native line".to_string(),
            target: AttachedReviewCommentTarget::Line {
                absolute_file_path: LocalOrRemotePath::Local(PathBuf::from("/repo/test.txt")),
                line: EditorLineLocation::Current {
                    line_number: line_count,
                    line_range: line_count..LineCount::from(2),
                },
                content: LineDiffContent {
                    content: "    let x = 1;".to_string(),
                    lines_added: LineCount::from(0),
                    lines_removed: LineCount::from(0),
                },
            },
            last_update_time: Local::now(),
            base: None,
            head: None,
            outdated: false,
            origin: CommentOrigin::Native,
        };

        ctx.code_review_view.update(&mut app, |_view, view_ctx| {
            let RelocateCommentsResult {
                comments: relocated,
                fallback_count: fallbacks,
            } = CodeReviewView::relocate_comments(
                vec![comment],
                &ctx.state,
                &ctx.repo_location,
                view_ctx,
            );

            assert_eq!(relocated.len(), 1, "Comment should be relocated");
            assert!(
                !relocated[0].outdated,
                "Native indented-line comment should NOT be outdated (leading whitespace is significant)"
            );
            assert_eq!(fallbacks, 0, "Should have no fallbacks for native indented match");
        });
    });
}

#[test]
fn test_setup_dropdown_with_branches_includes_all_items() {
    App::test((), |mut app| async move {
        let ctx = TestContext::new(&mut app, "test.txt", "line 1\nline 2\nline 3");

        // Populate branches and compute targets via the selector's build method.
        let target_count = ctx.code_review_view.update(&mut app, |view, view_ctx| {
            if let Some(repo) = view.active_repo.as_mut() {
                repo.available_branches = vec![
                    BranchEntry {
                        name: "main".to_string(),
                        is_main: true,
                    },
                    BranchEntry {
                        name: "feature-1".to_string(),
                        is_main: false,
                    },
                    BranchEntry {
                        name: "feature-2".to_string(),
                        is_main: false,
                    },
                ];
            }
            view.build_diff_targets(view_ctx).len()
        });

        // Verify the selector surfaces all expected items:
        // 1. "Uncommitted changes" (always first)
        // 2. "main" (main branch)
        // 3. "feature-1"
        // 4. "feature-2"
        assert_eq!(
            target_count, 4,
            "Diff selector should have 4 targets: Uncommitted changes + main + 2 feature branches"
        );
    });
}

#[test]
fn test_setup_dropdown_without_branches_only_has_uncommitted_changes() {
    App::test((), |mut app| async move {
        let ctx = TestContext::new(&mut app, "test.txt", "line 1\nline 2\nline 3");

        // Ensure branches are empty (simulates the bug state) and count targets.
        let target_count = ctx.code_review_view.update(&mut app, |view, view_ctx| {
            if let Some(repo) = view.active_repo.as_mut() {
                repo.available_branches = vec![];
            }
            view.build_diff_targets(view_ctx).len()
        });

        assert_eq!(
            target_count, 1,
            "Diff selector should only have 'Uncommitted changes' when no branches are available"
        );
    });
}

#[test]
fn test_on_close_then_on_open_reinitializes_repo_state() {
    App::test((), |mut app| async move {
        let ctx = TestContext::new(&mut app, "test.txt", "line 1\nline 2\nline 3");
        let repo_path = ctx.repo_path.clone();

        // Populate branches to simulate a working state
        let target_count_before = ctx.code_review_view.update(&mut app, |view, view_ctx| {
            if let Some(repo) = view.active_repo.as_mut() {
                repo.available_branches = vec![
                    BranchEntry {
                        name: "main".to_string(),
                        is_main: true,
                    },
                    BranchEntry {
                        name: "feature-1".to_string(),
                        is_main: false,
                    },
                ];
            }
            view.build_diff_targets(view_ctx).len()
        });
        assert_eq!(target_count_before, 3, "Should have 3 targets before close");

        // Close the view
        ctx.code_review_view.update(&mut app, |view, view_ctx| {
            view.on_close(view_ctx);
            assert!(!view.is_open, "View should be closed after on_close");
        });

        // Re-open the view
        ctx.code_review_view.update(&mut app, |view, view_ctx| {
            view.on_open(view_ctx);

            assert!(view.is_open, "View should be open after on_open");
            assert_eq!(
                view.repo_path().and_then(LocalOrRemotePath::to_local_path),
                Some(repo_path.as_path()),
                "Repo path should be preserved after on_open (set at construction)"
            );
        });
    });
}

#[test]
fn test_handle_edit_comment_scrolls_with_buffer() {
    App::test((), |mut app| async move {
        let content = (0..100)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let ctx = TestContext::new(&mut app, "test.txt", &content);

        // Create a line comment targeting this file
        let line_comment = create_line_comment("/repo/test.txt", 5, "line 5", "Review comment");
        let comment_id = line_comment.id;

        ctx.code_review_view.update(&mut app, |view, view_ctx| {
            // Inject the loaded state into the view's active repo
            if let Some(repo) = view.active_repo.as_mut() {
                repo.state = CodeReviewViewState::Loaded(ctx.state);
            }

            // Add the comment to the active comment model so get_comment_by_id can find it
            if let Some(model) = view.active_comment_model.clone() {
                model.update(view_ctx, |batch, ctx| {
                    batch.upsert_comment(line_comment, ctx);
                });
            }

            // Record scroll offset before the edit-comment scroll
            let offset_before = view.viewported_list_state.get_scroll_offset();

            // Call handle_edit_comment — should call scroll_to_line with COMMENT_EDITOR_SCROLL_BUFFER
            view.handle_edit_comment(&comment_id, view_ctx);

            // handle_edit_comment scrolls to the comment line. The scroll offset should
            // include COMMENT_EDITOR_SCROLL_BUFFER (200px) to account for the comment
            // editor that opens below the line.
            // Before the buffer fix, scroll_to_line passed buffer=0.0, so the offset
            // would be smaller. After the fix, it passes COMMENT_EDITOR_SCROLL_BUFFER.
            let offset_after = view.viewported_list_state.get_scroll_offset();
            let scroll_delta = offset_after - offset_before;

            // The scroll delta should include the COMMENT_EDITOR_SCROLL_BUFFER.
            // Without the buffer fix, scroll_delta would be smaller by 200px.
            assert!(
                scroll_delta >= Pixels::new(COMMENT_EDITOR_SCROLL_BUFFER),
                "Scroll delta ({scroll_delta:?}) should be >= COMMENT_EDITOR_SCROLL_BUFFER ({COMMENT_EDITOR_SCROLL_BUFFER}px) to account for the comment editor"
            );
        });
    });
}

#[test]
fn test_active_comments_not_marked_outdated() {
    App::test((), |mut app| async move {
        let ctx = TestContext::new(&mut app, "test.txt", "line 1\nline 2\nline 3");

        // Comment attached to "line 2" which exists in the editor
        let line_comment =
            create_line_comment("/repo/test.txt", 1, "line 2", "Comment on existing line");
        let original_id = line_comment.id;

        ctx.code_review_view.update(&mut app, |_view, view_ctx| {
            let RelocateCommentsResult {
                comments: relocated,
                fallback_count: fallbacks,
            } = CodeReviewView::relocate_comments(
                vec![line_comment],
                &ctx.state,
                &ctx.repo_location,
                view_ctx,
            );

            assert_eq!(relocated.len(), 1, "Comment should be relocated");
            assert_eq!(relocated[0].id, original_id, "Should preserve comment ID");
            assert!(
                !relocated[0].outdated,
                "Comment should NOT be marked as outdated when line content is found"
            );
            assert_eq!(
                fallbacks, 0,
                "Should have no fallbacks when content matches"
            );
        });
    });
}
