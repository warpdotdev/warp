//! Unit tests for [`super::QueuedQueryModel`].
//!
//! Covers FIFO ordering, append from each origin, edit semantics, clear, and reorder semantics.
use std::cell::RefCell;
use std::rc::Rc;

use warpui::App;

use super::{
    AutofireAction, QueuedQuery, QueuedQueryEvent, QueuedQueryId, QueuedQueryModel,
    QueuedQueryOrigin,
};

/// Helper to drive a `QueuedQueryModel` inside a test app and capture emitted events.
fn with_model<F>(test: F)
where
    F: FnOnce(App, warpui::ModelHandle<QueuedQueryModel>, Rc<RefCell<Vec<QueuedQueryEvent>>>)
        + 'static,
{
    App::test((), |mut app| async move {
        let model = app.add_model(|_| QueuedQueryModel::new());
        let events: Rc<RefCell<Vec<QueuedQueryEvent>>> = Rc::new(RefCell::new(Vec::new()));
        let events_clone = events.clone();
        app.update(|ctx| {
            ctx.subscribe_to_model(&model, move |_, event: &QueuedQueryEvent, _| {
                events_clone.borrow_mut().push(event.clone());
            });
        });
        test(app, model, events);
    });
}

fn user_query(text: &str) -> QueuedQuery {
    QueuedQuery::new(text.to_owned(), QueuedQueryOrigin::QueueSlashCommand)
}

fn append_user(
    model: &warpui::ModelHandle<QueuedQueryModel>,
    app: &mut App,
    text: &str,
) -> QueuedQueryId {
    model.update(app, |model, ctx| model.append(user_query(text), ctx))
}

#[test]
fn append_preserves_fifo_order() {
    with_model(|mut app, model, _events| {
        let id_a = append_user(&model, &mut app, "first");
        let id_b = append_user(&model, &mut app, "second");
        let id_c = append_user(&model, &mut app, "third");

        model.read(&app, |model, _| {
            let queue = model.queue();
            assert_eq!(queue.len(), 3);
            assert_eq!(queue[0].id(), id_a);
            assert_eq!(queue[0].text(), "first");
            assert_eq!(queue[1].id(), id_b);
            assert_eq!(queue[1].text(), "second");
            assert_eq!(queue[2].id(), id_c);
            assert_eq!(queue[2].text(), "third");
        });
    });
}

#[test]
fn append_from_each_user_origin_lands_in_the_queue() {
    // /queue and the auto-queue toggle both land in the queue.
    with_model(|mut app, model, _events| {
        let origins = [
            QueuedQueryOrigin::QueueSlashCommand,
            QueuedQueryOrigin::AutoQueueToggle,
        ];
        for (i, origin) in origins.iter().enumerate() {
            let text = format!("p{i}");
            model.update(&mut app, |m, ctx| {
                m.append(QueuedQuery::new(text, *origin), ctx)
            });
        }
        model.read(&app, |model, _| {
            let queue = model.queue();
            assert_eq!(queue.len(), 2);
            for (i, origin) in origins.iter().enumerate() {
                assert_eq!(queue[i].origin(), *origin);
            }
        });
    });
}

#[test]
fn queue_next_prompt_toggle_defaults_false_and_emits_event() {
    with_model(|mut app, model, events| {
        model.read(&app, |model, _| {
            assert!(!model.is_queue_next_prompt_enabled());
        });

        model.update(&mut app, |model, ctx| {
            model.toggle_queue_next_prompt(ctx);
        });

        model.read(&app, |model, _| {
            assert!(model.is_queue_next_prompt_enabled());
        });

        let evts = events.borrow();
        assert!(matches!(
            evts.as_slice(),
            [QueuedQueryEvent::QueueNextPromptToggled]
        ));
    });
}

#[test]
fn pop_front_removes_head_and_emits_removed() {
    with_model(|mut app, model, events| {
        let id_a = append_user(&model, &mut app, "first");
        let _id_b = append_user(&model, &mut app, "second");
        events.borrow_mut().clear();

        let popped = model.update(&mut app, |m, ctx| m.pop_front(ctx));
        let popped = popped.expect("queue had a head");
        assert_eq!(popped.id(), id_a);
        assert_eq!(popped.text(), "first");

        model.read(&app, |model, _| {
            assert_eq!(model.queue().len(), 1);
        });

        let evts = events.borrow();
        assert!(matches!(
            evts.as_slice(),
            [QueuedQueryEvent::Removed { query_id }] if *query_id == id_a
        ));
    });
}

#[test]
fn pop_for_autofire_returns_submit_for_user_managed_head() {
    with_model(|mut app, model, _events| {
        append_user(&model, &mut app, "first");
        append_user(&model, &mut app, "second");

        let action = model.update(&mut app, |m, ctx| m.pop_for_autofire(None, ctx));
        match action {
            Some(AutofireAction::Submit { text }) => assert_eq!(text, "first"),
            other => panic!("expected Submit, got {other:?}"),
        }

        model.read(&app, |model, _| {
            assert_eq!(model.queue().len(), 1);
        });
    });
}

#[test]
fn pop_for_autofire_uses_edit_text_override_when_first_row_is_in_edit_mode() {
    // Edit-mode autofire uses the live edit text.
    with_model(|mut app, model, _events| {
        let id_a = append_user(&model, &mut app, "first");
        append_user(&model, &mut app, "second");
        model.update(&mut app, |m, ctx| m.enter_edit_mode(id_a, ctx));

        let action = model.update(&mut app, |m, ctx| {
            m.pop_for_autofire(Some("edited".to_owned()), ctx)
        });
        match action {
            Some(AutofireAction::PopFromEditMode { text }) => assert_eq!(text, "edited"),
            other => panic!("expected PopFromEditMode, got {other:?}"),
        }
        // Edit mode is cleared after pop.
        model.read(&app, |model, _| {
            assert_eq!(model.editing_row(), None);
        });
    });
}

#[test]
fn first_row_is_in_edit_mode_only_when_the_head_row_is_being_edited() {
    with_model(|mut app, model, _events| {
        let id_a = append_user(&model, &mut app, "first");
        let id_b = append_user(&model, &mut app, "second");

        model.update(&mut app, |m, ctx| m.enter_edit_mode(id_b, ctx));
        model.read(&app, |m, _| {
            assert!(!m.first_row_is_in_edit_mode());
        });

        model.update(&mut app, |m, ctx| m.enter_edit_mode(id_a, ctx));
        model.read(&app, |m, _| {
            assert!(m.first_row_is_in_edit_mode());
        });
    });
}

#[test]
fn enter_edit_mode_locks_to_one_row_at_a_time() {
    // Entering edit mode on one row replaces the prior edit state.
    with_model(|mut app, model, _events| {
        let id_a = append_user(&model, &mut app, "first");
        let id_b = append_user(&model, &mut app, "second");

        model.update(&mut app, |m, ctx| m.enter_edit_mode(id_a, ctx));
        model.read(&app, |m, _| assert_eq!(m.editing_row(), Some(id_a)));

        // Entering edit mode on a different row replaces the prior edit.
        model.update(&mut app, |m, ctx| m.enter_edit_mode(id_b, ctx));
        model.read(&app, |m, _| assert_eq!(m.editing_row(), Some(id_b)));
    });
}

#[test]
fn commit_edit_with_text_replaces_row_and_clears_edit_state() {
    // Non-empty edits replace the queued row's text.
    with_model(|mut app, model, _events| {
        let id_a = append_user(&model, &mut app, "first");
        model.update(&mut app, |m, ctx| m.enter_edit_mode(id_a, ctx));

        model.update(&mut app, |m, ctx| {
            m.commit_edit("first updated".to_owned(), ctx)
        });

        model.read(&app, |m, _| {
            let queue = m.queue();
            assert_eq!(queue.len(), 1);
            assert_eq!(queue[0].id(), id_a);
            assert_eq!(queue[0].text(), "first updated");
            assert_eq!(m.editing_row(), None);
        });
    });
}

#[test]
fn commit_edit_with_empty_text_restores_original_text() {
    // Empty edits restore the original text.
    with_model(|mut app, model, _events| {
        let id_a = append_user(&model, &mut app, "first");
        append_user(&model, &mut app, "second");
        model.update(&mut app, |m, ctx| m.enter_edit_mode(id_a, ctx));

        model.update(&mut app, |m, ctx| m.commit_edit(String::new(), ctx));

        model.read(&app, |m, _| {
            let queue = m.queue();
            assert_eq!(queue.len(), 2);
            assert_eq!(queue[0].id(), id_a);
            assert_eq!(queue[0].text(), "first");
            assert_eq!(queue[1].text(), "second");
            assert_eq!(m.editing_row(), None);
        });
    });
}

#[test]
fn cancel_edit_leaves_row_unchanged_and_clears_edit_state() {
    // Canceling an edit leaves the row unchanged.
    with_model(|mut app, model, _events| {
        let id_a = append_user(&model, &mut app, "first");
        model.update(&mut app, |m, ctx| m.enter_edit_mode(id_a, ctx));

        model.update(&mut app, |m, ctx| m.cancel_edit(ctx));

        model.read(&app, |m, _| {
            let queue = m.queue();
            assert_eq!(queue.len(), 1);
            assert_eq!(queue[0].text(), "first");
            assert_eq!(m.editing_row(), None);
        });
    });
}

#[test]
fn remove_by_id_removes_only_the_targeted_row() {
    with_model(|mut app, model, _events| {
        let id_a = append_user(&model, &mut app, "first");
        let _id_b = append_user(&model, &mut app, "second");
        let _id_c = append_user(&model, &mut app, "third");

        let removed = model.update(&mut app, |m, ctx| m.remove_by_id(id_a, ctx));
        assert_eq!(
            removed.map(|r| r.text().to_owned()),
            Some("first".to_owned())
        );
        model.read(&app, |m, _| {
            let queue = m.queue();
            assert_eq!(queue.len(), 2);
            assert_eq!(queue[0].text(), "second");
            assert_eq!(queue[1].text(), "third");
        });
    });
}

#[test]
fn reorder_moves_user_managed_rows_to_target_index() {
    // Reordering moves user-managed rows to the requested target index.
    with_model(|mut app, model, _events| {
        let id_a = append_user(&model, &mut app, "a");
        let id_b = append_user(&model, &mut app, "b");
        let id_c = append_user(&model, &mut app, "c");

        // Move a (index 0) to the end (post-removal index 2).
        model.update(&mut app, |m, ctx| m.reorder(id_a, 2, ctx));

        model.read(&app, |m, _| {
            let queue = m.queue();
            assert_eq!(queue[0].id(), id_b);
            assert_eq!(queue[1].id(), id_c);
            assert_eq!(queue[2].id(), id_a);
        });
    });
}

#[test]
fn reorder_preserves_every_row_when_moving_last_to_front() {
    with_model(|mut app, model, _events| {
        let id_a = append_user(&model, &mut app, "a");
        let id_b = append_user(&model, &mut app, "b");
        let id_c = append_user(&model, &mut app, "c");
        let id_d = append_user(&model, &mut app, "d");

        model.update(&mut app, |m, ctx| m.reorder(id_d, 0, ctx));

        model.read(&app, |m, _| {
            let ids: Vec<_> = m.queue().iter().map(|q| q.id()).collect();
            assert_eq!(ids, vec![id_d, id_a, id_b, id_c]);
        });
    });
}

#[test]
fn reorder_clamps_target_index_to_queue_len() {
    with_model(|mut app, model, _events| {
        let id_a = append_user(&model, &mut app, "a");
        let id_b = append_user(&model, &mut app, "b");

        // Target index >= len after removal should clamp to the end.
        model.update(&mut app, |m, ctx| m.reorder(id_a, 99, ctx));
        model.read(&app, |m, _| {
            let queue = m.queue();
            assert_eq!(queue[0].id(), id_b);
            assert_eq!(queue[1].id(), id_a);
        });
    });
}

#[test]
fn removing_last_row_resets_collapse_state() {
    with_model(|mut app, model, _events| {
        let id = append_user(&model, &mut app, "only");
        model.update(&mut app, |m, ctx| m.set_collapsed(true, ctx));

        model.update(&mut app, |m, ctx| m.remove_by_id(id, ctx));

        model.read(&app, |m, _| {
            assert!(!m.has_queue());
            assert!(!m.is_collapsed());
        });
    });
}

#[test]
fn clear_all_wipes_queue_edit_and_collapse_state() {
    // Agent-view exit clears all queue state.
    with_model(|mut app, model, _events| {
        let id_a = append_user(&model, &mut app, "a1");
        append_user(&model, &mut app, "a2");
        model.update(&mut app, |m, ctx| {
            m.enter_edit_mode(id_a, ctx);
            m.set_collapsed(true, ctx);
        });

        model.update(&mut app, |m, ctx| m.clear_all(ctx));

        model.read(&app, |m, _| {
            assert!(!m.has_queue());
            assert_eq!(m.editing_row(), None);
            assert!(!m.is_collapsed());
        });
    });
}

#[test]
fn set_collapsed_toggles_and_emits_event() {
    with_model(|mut app, model, events| {
        model.read(&app, |m, _| assert!(!m.is_collapsed()));

        model.update(&mut app, |m, ctx| m.set_collapsed(true, ctx));
        model.read(&app, |m, _| assert!(m.is_collapsed()));

        // Idempotent — re-setting the same value does not emit again.
        model.update(&mut app, |m, ctx| m.set_collapsed(true, ctx));

        let collapse_events = events
            .borrow()
            .iter()
            .filter(|e| matches!(e, QueuedQueryEvent::CollapseToggled { .. }))
            .count();
        assert_eq!(collapse_events, 1);
    });
}
