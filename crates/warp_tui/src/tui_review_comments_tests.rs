use std::path::PathBuf;
use std::sync::Arc;

use ai::agent::action::{
    CommentSide, InsertReviewComment, InsertedCommentLine, InsertedCommentLocation,
};
use ai::agent::action_result::InsertReviewCommentsResult;
use warp::tui_export::{
    AIActionStatus, AIAgentAction, AIAgentActionId, AIAgentActionResult, AIAgentActionResultType,
    AIAgentActionType, Appearance, TaskId,
};
use warpui_core::App;
use warpui_core::elements::tui::{TuiBufferExt, TuiRect};
use warpui_core::presenter::tui::TuiPresenter;

use super::{format_comment_target, render_review_comments_tool_call};

fn comment(
    id: &str,
    author: &str,
    body: &str,
    parent_id: Option<&str>,
    location: Option<InsertedCommentLocation>,
) -> InsertReviewComment {
    InsertReviewComment {
        comment_id: id.to_owned(),
        author: author.to_owned(),
        last_modified_timestamp: format!("2024-01-01T00:00:0{id}Z"),
        comment_body: body.to_owned(),
        parent_comment_id: parent_id.map(str::to_owned),
        comment_location: location,
        html_url: Some(format!("https://github.com/warp/warp/pull/1#comment-{id}")),
    }
}

fn action(comments: Vec<InsertReviewComment>) -> AIAgentAction {
    AIAgentAction {
        id: AIAgentActionId::from("comments-action".to_owned()),
        task_id: TaskId::new("task-1".to_owned()),
        action: AIAgentActionType::InsertCodeReviewComments {
            repo_path: PathBuf::from("/repo"),
            comments,
            base_branch: Some("master".to_owned()),
        },
        requires_result: true,
    }
}

fn finished_status(action: &AIAgentAction, result: InsertReviewCommentsResult) -> AIActionStatus {
    AIActionStatus::Finished(Arc::new(AIAgentActionResult {
        id: action.id.clone(),
        task_id: action.task_id.clone(),
        result: AIAgentActionResultType::InsertReviewComments(result),
    }))
}

#[test]
fn formats_pull_request_file_and_line_targets() {
    assert_eq!(
        format_comment_target(&comment("1", "alice", "body", None, None)),
        "Pull request"
    );
    assert_eq!(
        format_comment_target(&comment(
            "1",
            "alice",
            "body",
            None,
            Some(InsertedCommentLocation {
                relative_file_path: "src/main.rs".to_owned(),
                line: None,
            }),
        )),
        "src/main.rs"
    );
    assert_eq!(
        format_comment_target(&comment(
            "1",
            "alice",
            "body",
            None,
            Some(InsertedCommentLocation {
                relative_file_path: "src/main.rs".to_owned(),
                line: Some(InsertedCommentLine {
                    comment_line_range: 10..12,
                    diff_hunk_line_range: 8..14,
                    diff_hunk_text: String::new(),
                    side: Some(CommentSide::Right),
                }),
            }),
        )),
        "src/main.rs:10-12 (new)"
    );
    assert_eq!(
        format_comment_target(&comment(
            "1",
            "alice",
            "body",
            None,
            Some(InsertedCommentLocation {
                relative_file_path: "src/main.rs".to_owned(),
                line: Some(InsertedCommentLine {
                    comment_line_range: 15..15,
                    diff_hunk_line_range: 15..15,
                    diff_hunk_text: String::new(),
                    side: Some(CommentSide::Left),
                }),
            }),
        )),
        "src/main.rs:15 (old)"
    );
}

#[test]
fn renders_shared_markdown_thread_only_after_success() {
    App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let action = action(vec![
            comment("1", "alice", "**Root**", None, None),
            comment("2", "bob", "Reply", Some("1"), None),
        ]);
        app.read(|ctx| {
            let mut presenter = TuiPresenter::new();
            let frame = presenter.present_element(
                render_review_comments_tool_call(
                    &action,
                    Some(&finished_status(
                        &action,
                        InsertReviewCommentsResult::Success {
                            repo_path: "/repo".to_owned(),
                        },
                    )),
                    false,
                    ctx,
                )
                .expect("review comment action should render"),
                TuiRect::new(0, 0, 80, 20),
                ctx,
            );
            let lines = frame
                .buffer
                .to_lines()
                .into_iter()
                .map(|line| line.trim_end().to_owned())
                .filter(|line| !line.is_empty())
                .collect::<Vec<_>>();
            assert_eq!(
                lines,
                vec![
                    "✓ Inserted 2 review comments",
                    "  Pull request",
                    "  @alice:",
                    "  Root",
                    "  ──────────────────────────────────────────────────────────────────────────────",
                    "  @bob:",
                    "  Reply",
                    "  https://github.com/warp/warp/pull/1#comment-1",
                ]
            );

            let mut presenter = TuiPresenter::new();
            let pending = presenter.present_element(
                render_review_comments_tool_call(&action, None, false, ctx)
                    .expect("review comment action should render"),
                TuiRect::new(0, 0, 80, 4),
                ctx,
            );
            assert_eq!(
                pending.buffer.to_lines()[0].trim_end(),
                "○ Insert 2 review comments"
            );

            for (result, expected) in [
                (
                    InsertReviewCommentsResult::Error {
                        repo_path: "/repo".to_owned(),
                        message: "failed".to_owned(),
                    },
                    "× Failed to insert review comments",
                ),
                (
                    InsertReviewCommentsResult::Cancelled,
                    "■ Insert review comments cancelled",
                ),
            ] {
                let mut presenter = TuiPresenter::new();
                let status = finished_status(&action, result);
                let frame = presenter.present_element(
                    render_review_comments_tool_call(&action, Some(&status), false, ctx)
                        .expect("review comment action should render"),
                    TuiRect::new(0, 0, 80, 4),
                    ctx,
                );
                assert_eq!(frame.buffer.to_lines()[0].trim_end(), expected);
                assert!(
                    frame
                        .buffer
                        .to_lines()
                        .iter()
                        .skip(1)
                        .all(|line| line.trim().is_empty())
                );
            }
        });
    });
}
