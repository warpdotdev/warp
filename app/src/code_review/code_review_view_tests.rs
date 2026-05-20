use super::*;
use crate::ai::persisted_workspace::PersistedWorkspace;
use crate::ai::request_usage_model::AIRequestUsageModel;
use crate::auth::AuthStateProvider;
use crate::cloud_object::model::persistence::CloudModel;
use crate::code::editor::view::{CodeEditorRenderOptions, CodeEditorView};
use crate::code::local_code_editor::LocalCodeEditorView;
use crate::code_review::comments::{
    attach_pending_imported_comments, AttachedReviewComment, AttachedReviewCommentTarget,
    CommentId, CommentOrigin, LineDiffContent, PendingImportedReviewComment,
    PendingImportedReviewCommentTarget,
};
use crate::code_review::diff_size_limits::DiffSize;
use crate::code_review::diff_state::{DiffStateModel, FileDiff, GitFileStatus};
use crate::code_review::editor_state::CodeReviewEditorState;
use crate::code_review::GlobalCodeReviewModel;
use crate::pane_group::WorkingDirectoriesModel;
use crate::server::server_api::{
    team::MockTeamClient, workspace::MockWorkspaceClient, ServerApiProvider,
};
use crate::server::telemetry::context_provider::AppTelemetryContextProvider;
use crate::settings_view::keybindings::KeybindingChangedNotifier;
use crate::terminal::local_shell::LocalShellState;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::vim_registers::VimRegisters;
use crate::workspace::sync_inputs::SyncedInputState;
use crate::workspace::ActiveSession;
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::NotebookKeybindings;
use ai::agent::action::InsertReviewComment;
use chrono::Local;
use lsp::LspManagerModel;
use repo_metadata::repositories::DetectedRepositories;
use std::path::PathBuf;
use std::sync::Arc;
use warp_core::features::FeatureFlag;
use warp_core::ui::appearance::Appearance;
use warp_editor::content::buffer::InitialBufferState;
use warp_editor::render::element::VerticalExpansionBehavior;
use warp_editor::render::model::LineCount;
use warpui::elements::{Empty, MouseStateHandle};
use warpui::platform::WindowStyle;
use warpui::{App, ViewHandle};

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
    app.add_singleton_model(|_| DetectedRepositories::default());
    app.add_singleton_model(|_| LspManagerModel::new());
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
            absolute_file_path: file_path.into(),
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

/// Creates an attached review comment with a File target
fn create_file_comment(
    file_path: impl Into<PathBuf>,
    comment_content: &str,
) -> AttachedReviewComment {
    AttachedReviewComment {
        id: CommentId::new(),
        content: comment_content.to_string(),
        target: AttachedReviewCommentTarget::File {
            absolute_file_path: file_path.into(),
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
    #[allow(dead_code)]
    window_id: warpui::WindowId,
    state: LoadedState,
    code_review_view: ViewHandle<CodeReviewView>,
}

impl TestContext {
    /// Initialize common test state with a single file editor
    fn new(app: &mut App, file_path: PathBuf, editor_content: &str) -> Self {
        initialize_test_app(app);

        let editor = create_editor_with_content(app, editor_content);
        let repo_path = PathBuf::from("/repo");

        let (window_id, _) = app.add_window(WindowStyle::NotStealFocus, |_| TestView);
        let state = create_loaded_state_with_editors(app, window_id, vec![(file_path, editor)]);

        let diff_state_model = app.add_model(DiffStateModel::new_for_test);

        let working_directories_model = app.add_model(|_| WorkingDirectoriesModel::new());
        let code_review_comment_batch =
            working_directories_model.update(app, |working_directories, ctx| {
                working_directories.get_or_create_code_review_comments(repo_path.as_path(), ctx)
            });

        let code_review_view = app.add_view(window_id, |ctx| {
            CodeReviewView::new(
                Some(repo_path.clone()),
                diff_state_model,
                code_review_comment_batch,
                None,
                ctx,
            )
        });

        Self {
            repo_path,
            window_id,
            state,
            code_review_view,
        }
    }
}

/// Creates a minimal LoadedState with file states containing editors.
/// Must be called within an App context.
fn create_loaded_state_with_editors(
    app: &mut App,
    window_id: warpui::WindowId,
    file_editors: Vec<(PathBuf, ViewHandle<LocalCodeEditorView>)>,
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
    }
}

#[test]
fn test_relocate_comments_empty_input() {
    App::test((), |mut app| async move {
        let ctx = TestContext::new(
            &mut app,
            PathBuf::from("test.txt"),
            "line 1\nline 2\nline 3",
        );

        ctx.code_review_view.update(&mut app, |_view, view_ctx| {
            let RelocateCommentsResult {
                comments: relocated,
                fallback_count: fallbacks,
            } = CodeReviewView::relocate_comments(vec![], &ctx.state, &ctx.repo_path, view_ctx);

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
        let ctx = TestContext::new(
            &mut app,
            PathBuf::from("test.txt"),
            "line 1\nline 2\nline 3",
        );

        let general_comment = create_general_comment("This is a general comment");
        let original_id = general_comment.id;

        ctx.code_review_view.update(&mut app, |_view, view_ctx| {
            let RelocateCommentsResult {
                comments: relocated,
                fallback_count: fallbacks,
            } = CodeReviewView::relocate_comments(
                vec![general_comment],
                &ctx.state,
                &ctx.repo_path,
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
        let file_path = PathBuf::from("test.txt");
        let ctx = TestContext::new(&mut app, file_path.clone(), "line 1\nline 2\nline 3");

        let file_comment =
            create_file_comment(ctx.repo_path.join(&file_path), "This is a file comment");
        let original_id = file_comment.id;

        ctx.code_review_view.update(&mut app, |_view, view_ctx| {
            let RelocateCommentsResult {
                comments: relocated,
                fallback_count: fallbacks,
            } = CodeReviewView::relocate_comments(
                vec![file_comment],
                &ctx.state,
                &ctx.repo_path,
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
        let _flag_override = FeatureFlag::PRCommentsSlashCommand.override_enabled(true);

        // Editor is for "test.txt" but comment is for "other.txt"
        let ctx = TestContext::new(
            &mut app,
            PathBuf::from("test.txt"),
            "line 1\nline 2\nline 3",
        );

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
                &ctx.repo_path,
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
        let file_path = PathBuf::from("test.txt");
        let ctx = TestContext::new(&mut app, file_path.clone(), "line 1\nline 2\nline 3");

        let general_comment = create_general_comment("General comment");
        let file_comment = create_file_comment(ctx.repo_path.join(&file_path), "File comment");
        let line_comment = create_line_comment("/repo/test.txt", 1, "line 1", "Line comment");

        let general_id = general_comment.id;
        let file_id = file_comment.id;
        let line_id = line_comment.id;

        ctx.code_review_view.update(&mut app, |_view, view_ctx| {
            let comments = vec![general_comment, file_comment, line_comment];
            let RelocateCommentsResult {
                comments: relocated,
                fallback_count: _,
            } = CodeReviewView::relocate_comments(comments, &ctx.state, &ctx.repo_path, view_ctx);

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
        let file_path = PathBuf::from("test.txt");
        let ctx = TestContext::new(&mut app, file_path.clone(), "line 1\nline 2\nline 3");

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
                &ctx.repo_path,
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

    let attached = attach_pending_imported_comments(vec![pending], repo_path.as_path());

    assert_eq!(attached.len(), 1);
    assert_eq!(attached[0].content, "**@alice**:\nHello world");

    match &attached[0].target {
        AttachedReviewCommentTarget::Line {
            absolute_file_path, ..
        } => {
            assert_eq!(*absolute_file_path, repo_path.join("test.txt"));
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

    let attached = attach_pending_imported_comments(
        vec![reply_late, root, reply_nested, reply_early],
        repo_path.as_path(),
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
            assert_eq!(*absolute_file_path, repo_path.join("test.txt"));
        }
        _ => panic!("expected root line target to be preserved"),
    }
}

#[test]
fn test_relocate_comments_file_comment_no_matching_editor_marked_outdated() {
    App::test((), |mut app| async move {
        let _flag_override = FeatureFlag::PRCommentsSlashCommand.override_enabled(true);

        // Editor is for "test.txt" but comment is for "other.txt"
        let ctx = TestContext::new(
            &mut app,
            PathBuf::from("test.txt"),
            "line 1\nline 2\nline 3",
        );

        let file_comment = create_file_comment("/repo/other.txt", "Comment on other file");
        let original_id = file_comment.id;

        ctx.code_review_view.update(&mut app, |_view, view_ctx| {
            let RelocateCommentsResult {
                comments: relocated,
                fallback_count: fallbacks,
            } = CodeReviewView::relocate_comments(
                vec![file_comment],
                &ctx.state,
                &ctx.repo_path,
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
        let _flag_override = FeatureFlag::PRCommentsSlashCommand.override_enabled(true);

        // Editor has "line 1\nline 3" (line 2 was removed)
        // Comment was attached to "line 2" which no longer exists
        let file_path = PathBuf::from("test.txt");
        let ctx = TestContext::new(&mut app, file_path.clone(), "line 1\nline 3");

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
                &ctx.repo_path,
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

#[test]
fn test_setup_dropdown_with_branches_includes_all_items() {
    App::test((), |mut app| async move {
        let ctx = TestContext::new(
            &mut app,
            PathBuf::from("test.txt"),
            "line 1\nline 2\nline 3",
        );

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
        let ctx = TestContext::new(
            &mut app,
            PathBuf::from("test.txt"),
            "line 1\nline 2\nline 3",
        );

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
        let ctx = TestContext::new(
            &mut app,
            PathBuf::from("test.txt"),
            "line 1\nline 2\nline 3",
        );
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
                view.repo_path(),
                Some(&repo_path),
                "Repo path should be preserved after on_open (set at construction)"
            );
        });
    });
}

#[test]
fn test_handle_edit_comment_scrolls_with_buffer() {
    App::test((), |mut app| async move {
        let file_path = PathBuf::from("test.txt");
        let content = (0..100)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let ctx = TestContext::new(&mut app, file_path.clone(), &content);

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
        let _flag_override = FeatureFlag::PRCommentsSlashCommand.override_enabled(true);

        let file_path = PathBuf::from("test.txt");
        let ctx = TestContext::new(&mut app, file_path.clone(), "line 1\nline 2\nline 3");

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
                &ctx.repo_path,
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
