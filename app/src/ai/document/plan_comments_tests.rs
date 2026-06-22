use warpui::App;

use super::{PlanComment, PlanCommentBatch};

#[test]
fn upsert_replaces_existing_comment_in_place() {
    App::test((), |mut app| async move {
        let model = app.add_model(|_| PlanCommentBatch::default());

        let mut comment = PlanComment::new_general("first".to_string());
        let id = comment.id;

        model.update(&mut app, |batch, ctx| {
            batch.upsert_comment(comment.clone(), ctx);
        });

        model.read(&app, |batch, _| {
            assert_eq!(batch.comments().len(), 1);
            assert_eq!(batch.comments()[0].id, id);
            assert_eq!(batch.comments()[0].body, "first");
        });

        comment.body = "updated".to_string();
        model.update(&mut app, |batch, ctx| {
            batch.upsert_comment(comment.clone(), ctx);
        });

        model.read(&app, |batch, _| {
            assert_eq!(batch.comments().len(), 1);
            assert_eq!(batch.comments()[0].id, id);
            assert_eq!(batch.comments()[0].body, "updated");
        });
    });
}

#[test]
fn delete_and_clear_mutations_work() {
    App::test((), |mut app| async move {
        let model = app.add_model(|_| PlanCommentBatch::default());

        let comment_a = PlanComment::new_general("a".to_string());
        let comment_b = PlanComment::new_general("b".to_string());
        let id_a = comment_a.id;

        model.update(&mut app, |batch, ctx| {
            batch.upsert_comment(comment_a.clone(), ctx);
            batch.upsert_comment(comment_b.clone(), ctx);
        });

        model.update(&mut app, |batch, ctx| {
            batch.delete_comment(id_a, ctx);
        });

        model.read(&app, |batch, _| {
            assert_eq!(batch.comments().len(), 1);
            assert_eq!(batch.comments()[0].body, "b");
        });

        model.update(&mut app, |batch, ctx| {
            batch.clear_all(ctx);
        });

        model.read(&app, |batch, _| {
            assert!(batch.comments().is_empty());
        });
    });
}

#[test]
fn active_comments_excludes_outdated() {
    App::test((), |mut app| async move {
        let model = app.add_model(|_| PlanCommentBatch::default());

        let comment_a = PlanComment::new_general("a".to_string());
        let comment_b = PlanComment::new_general("b".to_string());
        let id_b = comment_b.id;

        model.update(&mut app, |batch, ctx| {
            batch.upsert_comment(comment_a.clone(), ctx);
            batch.upsert_comment(comment_b.clone(), ctx);
        });

        model.update(&mut app, |batch, _| {
            assert!(batch.set_outdated(id_b, true));
            // Setting the same value again is a no-op.
            assert!(!batch.set_outdated(id_b, true));
        });

        model.read(&app, |batch, _| {
            let active: Vec<_> = batch
                .comments()
                .iter()
                .filter(|c| !c.outdated)
                .map(|c| c.body.as_str())
                .collect();
            assert_eq!(active, vec!["a"]);
        });
    });
}

#[test]
fn general_comment_has_no_quoted_text_or_range() {
    let comment = PlanComment::new_general("body".to_string());
    assert_eq!(comment.quoted_text(), None);
}
