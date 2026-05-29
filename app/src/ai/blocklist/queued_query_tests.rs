//! Unit tests for [`super::QueuedQueryModel`].
//!
//! Covers FIFO ordering, append from each origin, edit semantics, reorder semantics, the
//! per-conversation auto-queue toggle, and history-driven cleanup.
use std::cell::RefCell;
use std::rc::Rc;

use warpui::{App, SingletonEntity};

use super::{
    AutofireAction, QueuedQuery, QueuedQueryEvent, QueuedQueryId, QueuedQueryModel,
    QueuedQueryOrigin,
};
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::ImageContext;
use crate::ai::blocklist::{BlocklistAIHistoryModel, PendingAttachment};
use crate::test_util::settings::initialize_history_persistence_for_tests;

/// Helper to drive the singleton `QueuedQueryModel` (plus its required `BlocklistAIHistoryModel`
/// singleton) inside a test app and capture emitted events.
fn with_model<F>(test: F)
where
    F: FnOnce(App, warpui::ModelHandle<QueuedQueryModel>, Rc<RefCell<Vec<QueuedQueryEvent>>>)
        + 'static,
{
    App::test((), |mut app| async move {
        // Initializes settings (incl. `PrivatePreferences`) and registers
        // `GlobalResourceHandlesProvider`. The provider is required because
        // `BlocklistAIHistoryModel::delete_conversation` reads the global
        // model-event sender to enqueue a sqlite delete.
        initialize_history_persistence_for_tests(&mut app);
        app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let model = app.add_singleton_model(QueuedQueryModel::new);
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

fn initial_cloud_mode_query(text: &str) -> QueuedQuery {
    QueuedQuery::new(text.to_owned(), QueuedQueryOrigin::InitialCloudMode)
}

fn image_attachment(file_name: &str) -> PendingAttachment {
    PendingAttachment::Image(ImageContext {
        data: String::new(),
        mime_type: "image/png".to_owned(),
        file_name: file_name.to_owned(),
        is_figma: false,
    })
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
fn initial_cloud_mode_head_rejects_user_mutations_and_autofire() {
    with_model(|mut app, model, _events| {
        let conv = AIConversationId::new();
        let initial_id = model.update(&mut app, |model, ctx| {
            model.append(conv, initial_cloud_mode_query("initial"), ctx)
        });
        let followup_id = append_user(&model, &mut app, conv, "follow up");

        let removed = model.update(&mut app, |model, ctx| {
            model.remove_by_id(conv, initial_id, ctx)
        });
        assert!(removed.is_none());

        model.update(&mut app, |model, ctx| {
            model.enter_edit_mode(conv, initial_id, ctx);
            model.reorder(conv, initial_id, 1, ctx);
            model.reorder(conv, followup_id, 0, ctx);
        });

        let action = model.read(&app, |model, _| model.peek_autofire(conv));
        assert!(action.is_none());

        model.read(&app, |model, _| {
            let queue = model.queue(conv);
            assert_eq!(queue.len(), 2);
            assert_eq!(queue[0].id(), initial_id);
            assert_eq!(queue[0].origin(), QueuedQueryOrigin::InitialCloudMode);
            assert_eq!(queue[1].id(), followup_id);
            assert_eq!(model.editing_row(conv), None);
        });
    });
}

#[test]
fn pop_front_no_ops_when_head_is_locked() {
    // The Error/Cancelled drain path calls `pop_front` to restore a row to the editor. A locked
    // initial Cloud Mode head must not be popped even if a status-transition arrives before the
    // ambient-agent cleanup events fire `remove_initial_cloud_mode_row`.
    with_model(|mut app, model, _events| {
        let conv = AIConversationId::new();
        let initial_id = model.update(&mut app, |model, ctx| {
            model.append(conv, initial_cloud_mode_query("locked initial"), ctx)
        });
        let followup_id = append_user(&model, &mut app, conv, "follow up");

        let popped = model.update(&mut app, |model, ctx| model.pop_front(conv, ctx));
        assert!(popped.is_none());

        model.read(&app, |model, _| {
            let queue = model.queue(conv);
            assert_eq!(queue.len(), 2);
            assert_eq!(queue[0].id(), initial_id);
            assert_eq!(queue[1].id(), followup_id);
        });
    });
}

#[test]
fn remove_initial_cloud_mode_row_only_removes_the_locked_head() {
    with_model(|mut app, model, events| {
        let conv = AIConversationId::new();
        let initial_id = model.update(&mut app, |model, ctx| {
            model.append(conv, initial_cloud_mode_query("initial"), ctx)
        });
        append_user(&model, &mut app, conv, "follow up");
        events.borrow_mut().clear();

        let removed = model.update(&mut app, |model, ctx| {
            model.remove_initial_cloud_mode_row(conv, ctx)
        });
        assert_eq!(
            removed.map(|query| query.text().to_owned()),
            Some("initial".to_owned())
        );

        let removed_again = model.update(&mut app, |model, ctx| {
            model.remove_initial_cloud_mode_row(conv, ctx)
        });
        assert!(removed_again.is_none());

        let action = model.read(&app, |model, _| model.peek_autofire(conv));
        match action {
            Some(AutofireAction::Submit { text, .. }) => assert_eq!(text, "follow up"),
            other => panic!("expected Submit, got {other:?}"),
        }

        let evts = events.borrow();
        assert!(matches!(
            evts.first(),
            Some(QueuedQueryEvent::Removed { query_id, .. }) if *query_id == initial_id
        ));
    });
}

#[test]
fn append_preserves_fifo_order() {
    with_model(|mut app, model, _events| {
        let conv = AIConversationId::new();
        let id_a = append_user(&model, &mut app, conv, "first");
        let id_b = append_user(&model, &mut app, conv, "second");
        let id_c = append_user(&model, &mut app, conv, "third");

        model.read(&app, |model, _| {
            let queue = model.queue(conv);
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
        let conv = AIConversationId::new();
        let origins = [
            QueuedQueryOrigin::QueueSlashCommand,
            QueuedQueryOrigin::AutoQueueToggle,
        ];
        for (i, origin) in origins.iter().enumerate() {
            let text = format!("p{i}");
            model.update(&mut app, |m, ctx| {
                m.append(conv, QueuedQuery::new(text, *origin), ctx)
            });
        }
        model.read(&app, |model, _| {
            let queue = model.queue(conv);
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
        let conv = AIConversationId::new();
        model.read(&app, |model, _| {
            assert!(!model.is_queue_next_prompt_enabled(conv));
        });

        model.update(&mut app, |model, ctx| {
            model.toggle_queue_next_prompt(conv, ctx);
        });

        model.read(&app, |model, _| {
            assert!(model.is_queue_next_prompt_enabled(conv));
        });

        let evts = events.borrow();
        assert!(matches!(
            evts.as_slice(),
            [QueuedQueryEvent::QueueNextPromptToggled { conversation_id }] if *conversation_id == conv
        ));
    });
}

#[test]
fn toggle_state_is_isolated_per_conversation() {
    // Toggling for conversation A must not affect conversation B's toggle state.
    with_model(|mut app, model, _events| {
        let conv_a = AIConversationId::new();
        let conv_b = AIConversationId::new();

        model.update(&mut app, |m, ctx| m.toggle_queue_next_prompt(conv_a, ctx));
        model.read(&app, |m, _| {
            assert!(m.is_queue_next_prompt_enabled(conv_a));
            assert!(!m.is_queue_next_prompt_enabled(conv_b));
        });
    });
}

#[test]
fn append_state_is_isolated_per_conversation() {
    // Appending to one conversation's queue must not show up in another's.
    with_model(|mut app, model, _events| {
        let conv_a = AIConversationId::new();
        let conv_b = AIConversationId::new();

        append_user(&model, &mut app, conv_a, "a-first");
        append_user(&model, &mut app, conv_b, "b-first");
        append_user(&model, &mut app, conv_a, "a-second");

        model.read(&app, |m, _| {
            let a = m.queue(conv_a);
            assert_eq!(a.len(), 2);
            assert_eq!(a[0].text(), "a-first");
            assert_eq!(a[1].text(), "a-second");
            let b = m.queue(conv_b);
            assert_eq!(b.len(), 1);
            assert_eq!(b[0].text(), "b-first");
        });
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
            assert_eq!(model.queue(conv).len(), 1);
        });

        let evts = events.borrow();
        assert!(matches!(
            evts.as_slice(),
            [QueuedQueryEvent::Removed { conversation_id, query_id }]
                if *conversation_id == conv && *query_id == id_a
        ));
    });
}

#[test]
fn peek_autofire_leaves_row_until_remove_fired_row_drops_it() {
    with_model(|mut app, model, _events| {
        let conv = AIConversationId::new();
        let first_id = append_user(&model, &mut app, conv, "first");
        append_user(&model, &mut app, conv, "second");

        // peek_autofire reports the head's Submit action WITHOUT removing the row, so the send
        // path can still read its attachments by id.
        let action = model.read(&app, |model, _| model.peek_autofire(conv));
        match action {
            Some(AutofireAction::Submit { query_id, text }) => {
                assert_eq!(query_id, first_id);
                assert_eq!(text, "first");
            }
            other => panic!("expected Submit, got {other:?}"),
        }
        model.read(&app, |model, _| assert_eq!(model.queue(conv).len(), 2));

        // remove_fired_row drops the fired head once the synchronous send completes.
        model.update(&mut app, |model, ctx| {
            model.remove_fired_row(conv, first_id, ctx)
        });
        model.read(&app, |model, _| {
            let queue = model.queue(conv);
            assert_eq!(queue.len(), 1);
            assert_eq!(queue[0].text(), "second");
        });
    });
}

#[test]
fn peek_autofire_returns_pop_from_edit_mode_with_committed_text_and_attachments() {
    // Per spec: when the first row is in edit mode, peek_autofire's PopFromEditMode action
    // carries the row's last-committed text and its stored attachments (NOT any uncommitted
    // live-editor buffer text). peek leaves edit state intact; remove_fired_row clears it.
    with_model(|mut app, model, _events| {
        let conv = AIConversationId::new();
        let id_a = model.update(&mut app, |m, ctx| {
            m.append(
                conv,
                QueuedQuery::new_with_attachments(
                    "first".to_owned(),
                    QueuedQueryOrigin::QueueSlashCommand,
                    vec![image_attachment("a.png")],
                ),
                ctx,
            )
        });
        append_user(&model, &mut app, conv, "second");
        model.update(&mut app, |m, ctx| m.enter_edit_mode(conv, id_a, ctx));

        let action = model.read(&app, |m, _| m.peek_autofire(conv));
        match action {
            Some(AutofireAction::PopFromEditMode {
                query_id,
                text,
                attachments,
            }) => {
                assert_eq!(query_id, id_a);
                assert_eq!(text, "first");
                assert_eq!(attachments.len(), 1);
                assert_eq!(attachments[0].file_name(), "a.png");
            }
            other => panic!("expected PopFromEditMode, got {other:?}"),
        }
        // peek does not mutate: the row is still in edit mode.
        model.read(&app, |m, _| assert_eq!(m.editing_row(conv), Some(id_a)));

        model.update(&mut app, |m, ctx| m.remove_fired_row(conv, id_a, ctx));
        model.read(&app, |m, _| {
            assert_eq!(m.editing_row(conv), None);
            assert_eq!(m.queue(conv).len(), 1);
        });
    });
}

#[test]
fn first_row_is_in_edit_mode_only_when_the_head_row_is_being_edited() {
    with_model(|mut app, model, _events| {
        let conv = AIConversationId::new();
        let id_a = append_user(&model, &mut app, conv, "first");
        let id_b = append_user(&model, &mut app, conv, "second");

        model.update(&mut app, |m, ctx| m.enter_edit_mode(conv, id_b, ctx));
        model.read(&app, |m, _| {
            assert!(!m.first_row_is_in_edit_mode(conv));
        });

        model.update(&mut app, |m, ctx| m.enter_edit_mode(conv, id_a, ctx));
        model.read(&app, |m, _| {
            assert!(m.first_row_is_in_edit_mode(conv));
        });
    });
}

#[test]
fn enter_edit_mode_locks_to_one_row_at_a_time() {
    // Entering edit mode on one row cancels the prior edit state.
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
            m.commit_edit(conv, "first updated".to_owned(), ctx)
        });

        model.read(&app, |m, _| {
            let queue = m.queue(conv);
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

        model.update(&mut app, |m, ctx| m.commit_edit(conv, String::new(), ctx));

        model.read(&app, |m, _| {
            let queue = m.queue(conv);
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

        model.update(&mut app, |m, ctx| m.cancel_edit(conv, ctx));

        model.read(&app, |m, _| {
            let queue = m.queue(conv);
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
        assert_eq!(
            removed.map(|r| r.text().to_owned()),
            Some("first".to_owned())
        );
        model.read(&app, |m, _| {
            let queue = m.queue(conv);
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
            let queue = m.queue(conv);
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
            let ids: Vec<_> = m.queue(conv).iter().map(|q| q.id()).collect();
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
            let queue = m.queue(conv);
            assert_eq!(queue[0].id(), id_b);
            assert_eq!(queue[1].id(), id_a);
        });
    });
}

#[test]
fn delete_conversation_drops_only_that_conversation_state() {
    // Removing one conversation from history should drop its queue + toggle but leave others.
    with_model(|mut app, model, _events| {
        let history = BlocklistAIHistoryModel::handle(&app);
        let terminal_view_id = warpui::EntityId::new();
        let conv_a = history.update(&mut app, |h, ctx| {
            h.start_new_conversation(terminal_view_id, false, false, false, ctx)
        });
        let conv_b = history.update(&mut app, |h, ctx| {
            h.start_new_conversation(terminal_view_id, false, false, false, ctx)
        });
        append_user(&model, &mut app, conv_a, "a1");
        append_user(&model, &mut app, conv_b, "b1");
        model.update(&mut app, |m, ctx| m.toggle_queue_next_prompt(conv_a, ctx));

        history.update(&mut app, |h, ctx| {
            h.delete_conversation(conv_a, Some(terminal_view_id), ctx);
        });

        model.read(&app, |m, _| {
            assert!(!m.has_queue(conv_a));
            assert!(!m.is_queue_next_prompt_enabled(conv_a));
            let b = m.queue(conv_b);
            assert_eq!(b.len(), 1);
            assert_eq!(b[0].text(), "b1");
        });
    });
}

#[test]
fn clear_conversations_in_terminal_view_drops_every_listed_conversation() {
    // ClearedConversationsInTerminalView with multiple ids must drop each listed conversation's queue.
    with_model(|mut app, model, _events| {
        let history = BlocklistAIHistoryModel::handle(&app);
        let terminal_view_id = warpui::EntityId::new();
        let conv_a = history.update(&mut app, |h, ctx| {
            h.start_new_conversation(terminal_view_id, false, false, false, ctx)
        });
        let conv_b = history.update(&mut app, |h, ctx| {
            h.start_new_conversation(terminal_view_id, false, false, false, ctx)
        });
        append_user(&model, &mut app, conv_a, "a1");
        append_user(&model, &mut app, conv_b, "b1");

        history.update(&mut app, |h, ctx| {
            h.clear_conversations_in_terminal_view(terminal_view_id, ctx)
        });

        model.read(&app, |m, _| {
            assert!(!m.has_queue(conv_a));
            assert!(!m.has_queue(conv_b));
        });
    });
}
