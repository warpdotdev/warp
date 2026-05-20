//! Unit tests for [`super::QueuedQueryModel`].
//!
//! Covers FIFO ordering, append from each origin, edit semantics, per-conversation isolation,
//! clear, reorder semantics, and Cloud Mode immutability.
use super::{
    AutofireAction, QueuedQuery, QueuedQueryEvent, QueuedQueryId, QueuedQueryModel,
    QueuedQueryOrigin,
};
use crate::ai::agent::conversation::AIConversationId;
use std::cell::RefCell;
use std::rc::Rc;
use warpui::App;

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

fn cloud_query(text: &str) -> QueuedQuery {
    QueuedQuery::new(text.to_owned(), QueuedQueryOrigin::InitialCloudMode)
}

fn append_user(
    model: &warpui::ModelHandle<QueuedQueryModel>,
    app: &mut App,
    conversation_id: AIConversationId,
    text: &str,
) -> QueuedQueryId {
    model.update(app, |model, ctx| {
        model.append(conversation_id, user_query(text), ctx)
    })
}

#[test]
fn append_preserves_fifo_order_within_a_conversation() {
    with_model(|mut app, model, _events| {
        let conv = AIConversationId::new();
        let id_a = append_user(&model, &mut app, conv, "first");
        let id_b = append_user(&model, &mut app, conv, "second");
        let id_c = append_user(&model, &mut app, conv, "third");

        model.read(&app, |model, _| {
            let queue = model.queue_for(conv);
            assert_eq!(queue.len(), 3);
            assert_eq!(queue[0].id(), id_a);
            assert_eq!(queue[0].text(), "first");
            assert_eq!(queue[1].id(), id_b);
            assert_eq!(queue[1].text(), "second");
            assert_eq!(queue[2].id(), id_c);
            assert_eq!(queue[2].text(), "third");
            assert_eq!(model.first_text(conv), Some("first"));
        });
    });
}

#[test]
fn append_from_each_user_origin_lands_in_the_queue() {
    // /queue, auto-queue toggle, /compact-and, and /fork-and-compact all land in the queue.
    with_model(|mut app, model, _events| {
        let conv = AIConversationId::new();
        let origins = [
            QueuedQueryOrigin::QueueSlashCommand,
            QueuedQueryOrigin::AutoQueueToggle,
            QueuedQueryOrigin::CompactAnd,
            QueuedQueryOrigin::ForkAndCompact,
        ];
        for (i, origin) in origins.iter().enumerate() {
            let text = format!("p{i}");
            model.update(&mut app, |m, ctx| {
                m.append(conv, QueuedQuery::new(text, *origin), ctx)
            });
        }
        model.read(&app, |model, _| {
            let queue = model.queue_for(conv);
            assert_eq!(queue.len(), 4);
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
        let conv = AIConversationId::new();
        let id_a = append_user(&model, &mut app, conv, "first");
        let _id_b = append_user(&model, &mut app, conv, "second");
        events.borrow_mut().clear();

        let popped = model.update(&mut app, |m, ctx| m.pop_front(conv, ctx));
        let popped = popped.expect("queue had a head");
        assert_eq!(popped.id(), id_a);
        assert_eq!(popped.text(), "first");

        model.read(&app, |model, _| {
            assert_eq!(model.queue_for(conv).len(), 1);
        });

        let evts = events.borrow();
        assert!(matches!(
            evts.as_slice(),
            [QueuedQueryEvent::Removed { query_id, .. }] if *query_id == id_a
        ));
    });
}

#[test]
fn pop_front_user_managed_skips_cloud_mode_head() {
    with_model(|mut app, model, _events| {
        let conv = AIConversationId::new();
        model.update(&mut app, |m, ctx| {
            m.append(conv, cloud_query("cloud"), ctx);
            m.append(conv, user_query("user"), ctx);
        });

        let popped = model.update(&mut app, |m, ctx| m.pop_front_user_managed(conv, ctx));
        assert!(popped.is_none());

        model.read(&app, |model, _| {
            let queue = model.queue_for(conv);
            assert_eq!(queue.len(), 2);
            assert_eq!(queue[0].text(), "cloud");
            assert_eq!(queue[1].text(), "user");
        });
    });
}
#[test]
fn pop_for_autofire_skips_cloud_mode_head() {
    with_model(|mut app, model, _events| {
        let conv = AIConversationId::new();
        // Cloud Mode row at head; user-managed row behind it.
        model.update(&mut app, |m, ctx| {
            m.append(conv, cloud_query("cloud"), ctx);
            m.append(conv, user_query("user"), ctx);
        });

        let action = model.update(&mut app, |m, ctx| m.pop_for_autofire(conv, None, ctx));
        assert!(action.is_none(), "Cloud Mode head must not auto-fire");

        // The Cloud Mode row is still present; the queue is untouched.
        model.read(&app, |model, _| {
            let queue = model.queue_for(conv);
            assert_eq!(queue.len(), 2);
            assert_eq!(queue[0].origin(), QueuedQueryOrigin::InitialCloudMode);
        });
    });
}

#[test]
fn pop_for_autofire_returns_submit_for_user_managed_head() {
    with_model(|mut app, model, _events| {
        let conv = AIConversationId::new();
        append_user(&model, &mut app, conv, "first");
        append_user(&model, &mut app, conv, "second");

        let action = model.update(&mut app, |m, ctx| m.pop_for_autofire(conv, None, ctx));
        match action {
            Some(AutofireAction::Submit { text }) => assert_eq!(text, "first"),
            other => panic!("expected Submit, got {other:?}"),
        }

        model.read(&app, |model, _| {
            assert_eq!(model.queue_for(conv).len(), 1);
        });
    });
}

#[test]
fn pop_for_autofire_uses_edit_text_override_when_first_row_is_in_edit_mode() {
    // Edit-mode autofire uses the live edit text.
    with_model(|mut app, model, _events| {
        let conv = AIConversationId::new();
        let id_a = append_user(&model, &mut app, conv, "first");
        append_user(&model, &mut app, conv, "second");
        model.update(&mut app, |m, ctx| m.enter_edit_mode(conv, id_a, ctx));

        let action = model.update(&mut app, |m, ctx| {
            m.pop_for_autofire(conv, Some("edited".to_owned()), ctx)
        });
        match action {
            Some(AutofireAction::PopFromEditMode { text }) => assert_eq!(text, "edited"),
            other => panic!("expected PopFromEditMode, got {other:?}"),
        }
        // Edit mode is cleared after pop.
        model.read(&app, |model, _| {
            assert_eq!(model.editing_row(conv), None);
        });
    });
}

#[test]
fn enter_edit_mode_locks_to_one_row_at_a_time() {
    // Entering edit mode on one row replaces the prior edit state.
    with_model(|mut app, model, _events| {
        let conv = AIConversationId::new();
        let id_a = append_user(&model, &mut app, conv, "first");
        let id_b = append_user(&model, &mut app, conv, "second");

        model.update(&mut app, |m, ctx| m.enter_edit_mode(conv, id_a, ctx));
        model.read(&app, |m, _| assert_eq!(m.editing_row(conv), Some(id_a)));

        // Entering edit mode on a different row replaces the prior edit.
        model.update(&mut app, |m, ctx| m.enter_edit_mode(conv, id_b, ctx));
        model.read(&app, |m, _| assert_eq!(m.editing_row(conv), Some(id_b)));
    });
}

#[test]
fn commit_edit_with_text_replaces_row_and_clears_edit_state() {
    // Non-empty edits replace the queued row's text.
    with_model(|mut app, model, _events| {
        let conv = AIConversationId::new();
        let id_a = append_user(&model, &mut app, conv, "first");
        model.update(&mut app, |m, ctx| m.enter_edit_mode(conv, id_a, ctx));

        model.update(&mut app, |m, ctx| {
            m.commit_edit("first updated".to_owned(), ctx)
        });

        model.read(&app, |m, _| {
            let queue = m.queue_for(conv);
            assert_eq!(queue.len(), 1);
            assert_eq!(queue[0].id(), id_a);
            assert_eq!(queue[0].text(), "first updated");
            assert_eq!(m.editing_row(conv), None);
        });
    });
}

#[test]
fn commit_edit_with_empty_text_restores_original_text() {
    // Empty edits restore the original text.
    with_model(|mut app, model, _events| {
        let conv = AIConversationId::new();
        let id_a = append_user(&model, &mut app, conv, "first");
        append_user(&model, &mut app, conv, "second");
        model.update(&mut app, |m, ctx| m.enter_edit_mode(conv, id_a, ctx));

        model.update(&mut app, |m, ctx| m.commit_edit(String::new(), ctx));

        model.read(&app, |m, _| {
            let queue = m.queue_for(conv);
            assert_eq!(queue.len(), 2);
            assert_eq!(queue[0].id(), id_a);
            assert_eq!(queue[0].text(), "first");
            assert_eq!(queue[1].text(), "second");
            assert_eq!(m.editing_row(conv), None);
        });
    });
}

#[test]
fn cancel_edit_leaves_row_unchanged_and_clears_edit_state() {
    // Canceling an edit leaves the row unchanged.
    with_model(|mut app, model, _events| {
        let conv = AIConversationId::new();
        let id_a = append_user(&model, &mut app, conv, "first");
        model.update(&mut app, |m, ctx| m.enter_edit_mode(conv, id_a, ctx));

        model.update(&mut app, |m, ctx| m.cancel_edit(ctx));

        model.read(&app, |m, _| {
            let queue = m.queue_for(conv);
            assert_eq!(queue.len(), 1);
            assert_eq!(queue[0].text(), "first");
            assert_eq!(m.editing_row(conv), None);
        });
    });
}

#[test]
fn remove_by_id_removes_only_the_targeted_row() {
    with_model(|mut app, model, _events| {
        let conv = AIConversationId::new();
        let id_a = append_user(&model, &mut app, conv, "first");
        let _id_b = append_user(&model, &mut app, conv, "second");
        let _id_c = append_user(&model, &mut app, conv, "third");

        let removed = model.update(&mut app, |m, ctx| m.remove_by_id(conv, id_a, ctx));
        assert_eq!(removed.map(|r| r.into_text()), Some("first".to_owned()));
        model.read(&app, |m, _| {
            let queue = m.queue_for(conv);
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
        let conv = AIConversationId::new();
        let id_a = append_user(&model, &mut app, conv, "a");
        let id_b = append_user(&model, &mut app, conv, "b");
        let id_c = append_user(&model, &mut app, conv, "c");

        // Move a (index 0) to the end (post-removal index 2).
        model.update(&mut app, |m, ctx| m.reorder(conv, id_a, 2, ctx));

        model.read(&app, |m, _| {
            let queue = m.queue_for(conv);
            assert_eq!(queue[0].id(), id_b);
            assert_eq!(queue[1].id(), id_c);
            assert_eq!(queue[2].id(), id_a);
        });
    });
}

#[test]
fn reorder_preserves_every_row_when_moving_last_to_front() {
    with_model(|mut app, model, _events| {
        let conv = AIConversationId::new();
        let id_a = append_user(&model, &mut app, conv, "a");
        let id_b = append_user(&model, &mut app, conv, "b");
        let id_c = append_user(&model, &mut app, conv, "c");
        let id_d = append_user(&model, &mut app, conv, "d");

        model.update(&mut app, |m, ctx| m.reorder(conv, id_d, 0, ctx));

        model.read(&app, |m, _| {
            let ids: Vec<_> = m.queue_for(conv).iter().map(|q| q.id()).collect();
            assert_eq!(ids, vec![id_d, id_a, id_b, id_c]);
        });
    });
}
#[test]
fn reorder_clamps_target_index_to_queue_len() {
    with_model(|mut app, model, _events| {
        let conv = AIConversationId::new();
        let id_a = append_user(&model, &mut app, conv, "a");
        let id_b = append_user(&model, &mut app, conv, "b");

        // Target index >= len after removal should clamp to the end.
        model.update(&mut app, |m, ctx| m.reorder(conv, id_a, 99, ctx));
        model.read(&app, |m, _| {
            let queue = m.queue_for(conv);
            assert_eq!(queue[0].id(), id_b);
            assert_eq!(queue[1].id(), id_a);
        });
    });
}

#[test]
fn cloud_mode_rows_reject_reorder_and_replace_text() {
    // The harness owns Cloud Mode row state.
    with_model(|mut app, model, _events| {
        let conv = AIConversationId::new();
        let cloud_id = model.update(&mut app, |m, ctx| m.append(conv, cloud_query("cloud"), ctx));
        let user_id = append_user(&model, &mut app, conv, "user");

        // reorder is a no-op when the source is a Cloud Mode row.
        model.update(&mut app, |m, ctx| m.reorder(conv, cloud_id, 1, ctx));
        model.read(&app, |m, _| {
            let queue = m.queue_for(conv);
            assert_eq!(queue[0].id(), cloud_id);
            assert_eq!(queue[1].id(), user_id);
        });

        // replace_text_by_id is a no-op for Cloud Mode rows.
        model.update(&mut app, |m, ctx| {
            m.replace_text_by_id(conv, cloud_id, "tampered".to_owned(), ctx);
        });
        model.read(&app, |m, _| {
            assert_eq!(m.queue_for(conv)[0].text(), "cloud");
        });
    });
}

#[test]
fn user_managed_rows_cannot_reorder_above_cloud_mode_prefix() {
    with_model(|mut app, model, _events| {
        let conv = AIConversationId::new();
        let cloud_id = model.update(&mut app, |m, ctx| m.append(conv, cloud_query("cloud"), ctx));
        let user_id_a = append_user(&model, &mut app, conv, "a");
        let user_id_b = append_user(&model, &mut app, conv, "b");

        model.update(&mut app, |m, ctx| m.reorder(conv, user_id_b, 0, ctx));

        model.read(&app, |m, _| {
            let ids: Vec<_> = m.queue_for(conv).iter().map(|q| q.id()).collect();
            assert_eq!(ids, vec![cloud_id, user_id_b, user_id_a]);
        });
    });
}

#[test]
fn cloud_mode_rows_reject_enter_edit_mode() {
    with_model(|mut app, model, _events| {
        let conv = AIConversationId::new();
        let cloud_id = model.update(&mut app, |m, ctx| m.append(conv, cloud_query("cloud"), ctx));
        model.update(&mut app, |m, ctx| m.enter_edit_mode(conv, cloud_id, ctx));
        model.read(&app, |m, _| assert_eq!(m.editing_row(conv), None));
    });
}

#[test]
fn queues_are_per_conversation_isolated() {
    // Queue state is isolated per conversation.
    with_model(|mut app, model, _events| {
        let conv_a = AIConversationId::new();
        let conv_b = AIConversationId::new();
        append_user(&model, &mut app, conv_a, "a1");
        append_user(&model, &mut app, conv_a, "a2");
        append_user(&model, &mut app, conv_b, "b1");

        model.read(&app, |m, _| {
            assert_eq!(m.queue_for(conv_a).len(), 2);
            assert_eq!(m.queue_for(conv_b).len(), 1);
            assert!(m.has_queue(conv_a));
            assert!(m.has_queue(conv_b));
        });
    });
}

#[test]
fn clear_for_conversation_clears_queue_edit_and_collapse_state() {
    // Clearing a conversation clears its queue, edit state, and collapse state.
    with_model(|mut app, model, _events| {
        let conv_a = AIConversationId::new();
        let conv_b = AIConversationId::new();
        let id_a = append_user(&model, &mut app, conv_a, "a1");
        append_user(&model, &mut app, conv_b, "b1");
        model.update(&mut app, |m, ctx| {
            m.enter_edit_mode(conv_a, id_a, ctx);
            m.set_collapsed(conv_a, true, ctx);
        });

        model.update(&mut app, |m, ctx| m.clear_for_conversation(conv_a, ctx));

        model.read(&app, |m, _| {
            assert!(!m.has_queue(conv_a));
            assert_eq!(m.editing_row(conv_a), None);
            assert!(!m.is_collapsed(conv_a));
            // Other conversations are untouched.
            assert_eq!(m.queue_for(conv_b).len(), 1);
        });
    });
}

#[test]
fn removing_last_row_resets_collapse_state() {
    with_model(|mut app, model, _events| {
        let conv = AIConversationId::new();
        let id = append_user(&model, &mut app, conv, "only");
        model.update(&mut app, |m, ctx| m.set_collapsed(conv, true, ctx));

        model.update(&mut app, |m, ctx| m.remove_by_id(conv, id, ctx));

        model.read(&app, |m, _| {
            assert!(!m.has_queue(conv));
            assert!(!m.is_collapsed(conv));
        });
    });
}

#[test]
fn clear_all_wipes_every_conversation() {
    // Agent-view exit clears all queues.
    with_model(|mut app, model, _events| {
        let conv_a = AIConversationId::new();
        let conv_b = AIConversationId::new();
        append_user(&model, &mut app, conv_a, "a1");
        append_user(&model, &mut app, conv_b, "b1");
        model.update(&mut app, |m, ctx| m.set_collapsed(conv_a, true, ctx));

        model.update(&mut app, |m, ctx| m.clear_all(ctx));

        model.read(&app, |m, _| {
            assert!(!m.has_queue(conv_a));
            assert!(!m.has_queue(conv_b));
            assert!(!m.is_collapsed(conv_a));
        });
    });
}

#[test]
fn collapse_state_is_per_conversation() {
    with_model(|mut app, model, _events| {
        let conv_a = AIConversationId::new();
        let conv_b = AIConversationId::new();
        model.update(&mut app, |m, ctx| {
            m.set_collapsed(conv_a, true, ctx);
        });
        model.read(&app, |m, _| {
            assert!(m.is_collapsed(conv_a));
            assert!(!m.is_collapsed(conv_b));
        });
    });
}
