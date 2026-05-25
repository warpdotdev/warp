use std::path::PathBuf;

use chrono::Local;
use warp_editor::render::model::LineCount;
use warpui::App;

use crate::code::buffer_location::LocalOrRemotePath;
use crate::code::editor::line::EditorLineLocation;
use crate::code_review::comments::{
    AttachedReviewComment, AttachedReviewCommentTarget, CommentOrigin, LineDiffContent,
    ReviewCommentBatch,
};

fn line_comment(file_path: &str, line_number: usize, content: &str) -> AttachedReviewComment {
    AttachedReviewComment {
        id: Default::default(),
        content: content.to_string(),
        target: AttachedReviewCommentTarget::Line {
            absolute_file_path: LocalOrRemotePath::Local(PathBuf::from(file_path)),
            line: EditorLineLocation::Current {
                line_number: LineCount::from(line_number),
                line_range: LineCount::from(line_number)..LineCount::from(line_number + 1),
            },
            content: LineDiffContent {
                content: "+line\n".to_string(),
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

#[test]
fn upsert_replaces_existing_comment_in_place() {
    App::test((), |mut app| async move {
        let model = app.add_model(|_| ReviewCommentBatch::default());

        let mut comment = line_comment("/repo/src/lib.rs", 10, "first");
        let id = comment.id;

        model.update(&mut app, |batch, ctx| {
            batch.upsert_comment(comment.clone(), ctx);
        });

        model.read(&app, |batch, _| {
            assert_eq!(batch.comments.len(), 1);
            assert_eq!(batch.comments[0].id, id);
            assert_eq!(batch.comments[0].content, "first");
        });

        comment.content = "updated".to_string();
        model.update(&mut app, |batch, ctx| {
            batch.upsert_comment(comment.clone(), ctx);
        });

        model.read(&app, |batch, _| {
            assert_eq!(batch.comments.len(), 1);
            assert_eq!(batch.comments[0].id, id);
            assert_eq!(batch.comments[0].content, "updated");
        });
    });
}

#[test]
fn delete_and_clear_mutations_work() {
    App::test((), |mut app| async move {
        let model = app.add_model(|_| ReviewCommentBatch::default());

        let comment_a = line_comment("/repo/src/lib.rs", 1, "a");
        let comment_b = line_comment("/repo/src/lib.rs", 2, "b");
        let id_a = comment_a.id;

        model.update(&mut app, |batch, ctx| {
            batch.upsert_comment(comment_a.clone(), ctx);
            batch.upsert_comment(comment_b.clone(), ctx);
        });

        model.update(&mut app, |batch, ctx| {
            batch.delete_comment(id_a, ctx);
        });

        model.read(&app, |batch, _| {
            assert_eq!(batch.comments.len(), 1);
            assert_eq!(batch.comments[0].content, "b");
        });

        model.update(&mut app, |batch, ctx| {
            batch.clear_all(ctx);
        });

        model.read(&app, |batch, _| {
            assert!(batch.comments.is_empty());
        });
    });
}

#[test]
fn file_and_line_queries_filter_by_suffix() {
    App::test((), |mut app| async move {
        let model = app.add_model(|_| ReviewCommentBatch::default());

        let comment_a = line_comment("/repo/src/lib.rs", 3, "a");
        let comment_b = line_comment("/repo/src/main.rs", 5, "b");

        model.update(&mut app, |batch, ctx| {
            batch.upsert_comment(comment_a.clone(), ctx);
            batch.upsert_comment(comment_b.clone(), ctx);
        });

        model.read(&app, |batch, _| {
            let file_path = LocalOrRemotePath::Local(PathBuf::from("/repo/src/lib.rs"));
            let file_comments: Vec<_> = batch.file_comments(&file_path).collect();
            assert_eq!(file_comments.len(), 1);
            assert_eq!(file_comments[0].content, "a");
            let line_numbers: Vec<_> = batch.comment_line_numbers_for_file(&file_path).collect();
            assert_eq!(line_numbers, vec![LineCount::from(3)]);
        });
    });
}

#[test]
fn editor_comments_for_file_includes_only_line_comments() {
    App::test((), |mut app| async move {
        let model = app.add_model(|_| ReviewCommentBatch::default());

        let comment_a = line_comment("/repo/src/lib.rs", 7, "a");
        let general = AttachedReviewComment {
            id: Default::default(),
            content: "general".to_string(),
            target: AttachedReviewCommentTarget::General,
            last_update_time: Local::now(),
            base: None,
            head: None,
            outdated: false,
            origin: CommentOrigin::Native,
        };

        model.update(&mut app, |batch, ctx| {
            batch.upsert_comment(comment_a.clone(), ctx);
            batch.upsert_comment(general.clone(), ctx);
        });

        model.read(&app, |batch, _| {
            let editor_comments = batch.editor_comments_for_file(&LocalOrRemotePath::Local(
                PathBuf::from("/repo/src/lib.rs"),
            ));
            assert_eq!(editor_comments.len(), 1);
            assert_eq!(editor_comments[0].id, comment_a.id);
            assert_eq!(editor_comments[0].comment_content, "a");
        });
    });
}
