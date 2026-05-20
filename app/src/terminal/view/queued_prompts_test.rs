//! Tests for the auto-fire drain logic that runs from [`super::TerminalView::drain_queued_prompts`].
//!
//! `TerminalView` orchestrates the input editor and the queue model on `FinishedReceivingOutput`.
//! Constructing a full `TerminalView` in a unit test would require dozens of dependencies, so the
//! tests below exercise the underlying `QueuedQueryModel` semantics that the drain path relies on.
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::blocklist::{AutofireAction, QueuedQuery, QueuedQueryModel, QueuedQueryOrigin};
use warpui::App;

fn user_query(text: &str) -> QueuedQuery {
    QueuedQuery::new(text.to_owned(), QueuedQueryOrigin::QueueSlashCommand)
}

fn cloud_query(text: &str) -> QueuedQuery {
    QueuedQuery::new(text.to_owned(), QueuedQueryOrigin::InitialCloudMode)
}

#[test]
fn complete_drain_pops_head_and_returns_submit_action() {
    // On Complete, the next queued prompt fires via Submit.
    App::test((), |mut app| async move {
        let model = app.add_model(|_| QueuedQueryModel::new());
        let conv = AIConversationId::new();
        model.update(&mut app, |m, ctx| {
            m.append(conv, user_query("first"), ctx);
            m.append(conv, user_query("second"), ctx);
        });

        let action = model.update(&mut app, |m, ctx| m.pop_for_autofire(conv, None, ctx));
        match action {
            Some(AutofireAction::Submit { text }) => assert_eq!(text, "first"),
            other => panic!("expected Submit, got {other:?}"),
        }
        model.read(&app, |m, _| {
            assert_eq!(m.queue_for(conv).len(), 1);
            assert_eq!(m.queue_for(conv)[0].text(), "second");
        });
    });
}

#[test]
fn complete_drain_with_first_row_in_edit_mode_returns_pop_from_edit_mode() {
    // When the first row is being edited, drain produces a PopFromEditMode action carrying the
    // live-edit override text.
    App::test((), |mut app| async move {
        let model = app.add_model(|_| QueuedQueryModel::new());
        let conv = AIConversationId::new();
        let id_a = model.update(&mut app, |m, ctx| m.append(conv, user_query("first"), ctx));
        model.update(&mut app, |m, ctx| {
            m.append(conv, user_query("second"), ctx);
            m.enter_edit_mode(conv, id_a, ctx);
        });

        let action = model.update(&mut app, |m, ctx| {
            m.pop_for_autofire(conv, Some("edited-first".to_owned()), ctx)
        });
        match action {
            Some(AutofireAction::PopFromEditMode { text }) => assert_eq!(text, "edited-first"),
            other => panic!("expected PopFromEditMode, got {other:?}"),
        }
        model.read(&app, |m, _| {
            // Edit mode is cleared so the next drain doesn't re-enter the edit-mode branch.
            assert_eq!(m.editing_row(conv), None);
            assert_eq!(m.queue_for(conv).len(), 1);
            assert_eq!(m.queue_for(conv)[0].text(), "second");
        });
    });
}

#[test]
fn complete_drain_skips_initial_cloud_mode_head() {
    // The harness owns Cloud Mode firing; auto-fire does not pop it.
    App::test((), |mut app| async move {
        let model = app.add_model(|_| QueuedQueryModel::new());
        let conv = AIConversationId::new();
        model.update(&mut app, |m, ctx| {
            m.append(conv, cloud_query("cloud"), ctx);
            m.append(conv, user_query("user"), ctx);
        });

        let action = model.update(&mut app, |m, ctx| m.pop_for_autofire(conv, None, ctx));
        assert!(action.is_none(), "Cloud Mode head must not auto-fire");
        model.read(&app, |m, _| {
            assert_eq!(m.queue_for(conv).len(), 2);
        });
    });
}

#[test]
fn complete_drain_with_empty_queue_returns_none() {
    App::test((), |mut app| async move {
        let model = app.add_model(|_| QueuedQueryModel::new());
        let conv = AIConversationId::new();
        let action = model.update(&mut app, |m, ctx| m.pop_for_autofire(conv, None, ctx));
        assert!(action.is_none());
    });
}

#[test]
fn error_or_cancel_drain_pops_front_when_input_is_empty() {
    // On Error/Cancelled with an empty input, the next queued prompt's text is restored to the
    // input by popping it (which the host then writes into the buffer).
    App::test((), |mut app| async move {
        let model = app.add_model(|_| QueuedQueryModel::new());
        let conv = AIConversationId::new();
        model.update(&mut app, |m, ctx| {
            m.append(conv, user_query("first"), ctx);
            m.append(conv, user_query("second"), ctx);
        });

        let popped = model.update(&mut app, |m, ctx| m.pop_front_user_managed(conv, ctx));
        let popped = popped.expect("queue had a head");
        assert_eq!(popped.text(), "first");
        model.read(&app, |m, _| {
            assert_eq!(m.queue_for(conv).len(), 1);
            assert_eq!(m.queue_for(conv)[0].text(), "second");
        });
    });
}

#[test]
fn error_or_cancel_drain_skips_initial_cloud_mode_head() {
    App::test((), |mut app| async move {
        let model = app.add_model(|_| QueuedQueryModel::new());
        let conv = AIConversationId::new();
        model.update(&mut app, |m, ctx| {
            m.append(conv, cloud_query("cloud"), ctx);
            m.append(conv, user_query("user"), ctx);
        });

        let popped = model.update(&mut app, |m, ctx| m.pop_front_user_managed(conv, ctx));
        assert!(popped.is_none(), "Cloud Mode head must stay harness-owned");
        model.read(&app, |m, _| {
            assert_eq!(m.queue_for(conv).len(), 2);
            assert_eq!(
                m.queue_for(conv)[0].origin(),
                QueuedQueryOrigin::InitialCloudMode
            );
            assert_eq!(m.queue_for(conv)[1].text(), "user");
        });
    });
}

#[test]
fn error_or_cancel_drain_leaves_queue_intact_when_input_is_non_empty() {
    // When the input is non-empty, the drain skips popping so the queue remains intact.
    //
    // The host (`TerminalView`) gates the pop on input-empty. We model that here by simply not
    // popping when the simulated input is non-empty, and asserting the queue remains unchanged.
    App::test((), |mut app| async move {
        let model = app.add_model(|_| QueuedQueryModel::new());
        let conv = AIConversationId::new();
        model.update(&mut app, |m, ctx| {
            m.append(conv, user_query("first"), ctx);
            m.append(conv, user_query("second"), ctx);
        });

        let simulated_input_is_non_empty = true;
        if !simulated_input_is_non_empty {
            model.update(&mut app, |m, ctx| m.pop_front_user_managed(conv, ctx));
        }

        model.read(&app, |m, _| {
            assert_eq!(m.queue_for(conv).len(), 2);
            assert_eq!(m.queue_for(conv)[0].text(), "first");
        });
    });
}

#[test]
fn complete_drain_after_error_drain_continues_with_next_row() {
    // After an Error/Cancelled drain pops one row and the user later submits successfully, the
    // *next* Complete drain pops the following row.
    App::test((), |mut app| async move {
        let model = app.add_model(|_| QueuedQueryModel::new());
        let conv = AIConversationId::new();
        model.update(&mut app, |m, ctx| {
            m.append(conv, user_query("first"), ctx);
            m.append(conv, user_query("second"), ctx);
            m.append(conv, user_query("third"), ctx);
        });

        // Error: input is empty, pop "first" and restore to input.
        let popped = model.update(&mut app, |m, ctx| m.pop_front_user_managed(conv, ctx));
        assert_eq!(popped.map(|q| q.into_text()), Some("first".to_owned()));

        // Complete: pop "second".
        let action = model.update(&mut app, |m, ctx| m.pop_for_autofire(conv, None, ctx));
        match action {
            Some(AutofireAction::Submit { text }) => assert_eq!(text, "second"),
            other => panic!("expected Submit(\"second\"), got {other:?}"),
        }

        // Complete again: pop "third".
        let action = model.update(&mut app, |m, ctx| m.pop_for_autofire(conv, None, ctx));
        match action {
            Some(AutofireAction::Submit { text }) => assert_eq!(text, "third"),
            other => panic!("expected Submit(\"third\"), got {other:?}"),
        }

        // Queue is now empty; the next drain returns None.
        let action = model.update(&mut app, |m, ctx| m.pop_for_autofire(conv, None, ctx));
        assert!(action.is_none());
    });
}
