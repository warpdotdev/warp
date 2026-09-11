use uuid::Uuid;
use warpui::App;

use super::*;
use crate::ai::agent::{AIAgentContext, AIAgentInput};
use crate::ai::blocklist::QueuedQueryOrigin;
use crate::test_util::terminal::{add_window_with_terminal, initialize_app_for_terminal_view};

/// The text of every `UserQuery` input across `id`'s root-task exchanges, in the order they
/// were appended to history.
fn user_queries_in_order(history: &BlocklistAIHistoryModel, id: AIConversationId) -> Vec<String> {
    history
        .conversation(&id)
        .unwrap()
        .root_task_exchanges()
        .flat_map(|exchange| exchange.input.iter())
        .filter_map(|input| match input {
            AIAgentInput::UserQuery { query, .. } => Some(query.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn startup_injections_queued_before_the_initial_prompt_are_dispatched_one_at_a_time() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let terminal = add_window_with_terminal(&mut app, None);
        let controller = terminal.read(&app, |terminal, _| terminal.ai_controller().clone());
        let participant = ParticipantId::new();
        let id = controller.update(&mut app, |controller, ctx| {
            let id = controller.bind_native_prompt_conversation(None, ctx);
            controller.execute_warp_agent_prompt_from_shared_session_injection(
                "prompt2".into(),
                None,
                vec![],
                participant.clone(),
                ctx,
            );
            controller.execute_warp_agent_prompt_from_shared_session_injection(
                "prompt3".into(),
                None,
                vec![],
                participant.clone(),
                ctx,
            );
            assert_eq!(
                QueuedQueryModel::as_ref(ctx)
                    .queue(id)
                    .iter()
                    .map(QueuedQuery::text)
                    .collect::<Vec<_>>(),
                vec!["prompt2", "prompt3"],
                "both should be held behind the not-yet-sent initial prompt"
            );
            id
        });
        terminal.update(&mut app, |terminal, ctx| {
            terminal.enter_agent_view(None, Some(id), AgentViewEntryOrigin::Cli, ctx);
        });

        // Mirrors what `AgentDriver::execute_run` does: send the initial prompt, then dispatch
        // the head of the backlog.
        controller.update(&mut app, |controller, ctx| {
            controller.send_user_query_in_conversation("prompt1".into(), id, None, ctx);
            controller.dispatch_queued_warp_agent_prompt(id, None, ctx);
        });

        QueuedQueryModel::handle(&app).read(&app, |queue, _| {
            assert_eq!(
                queue
                    .queue(id)
                    .iter()
                    .map(QueuedQuery::text)
                    .collect::<Vec<_>>(),
                vec!["prompt3"],
                "only the head row (prompt2) should have been dispatched; prompt3 waits for the \
                 next natural boundary or the conversation going idle again"
            );
        });
        BlocklistAIHistoryModel::handle(&app).read(&app, |history, _| {
            assert_eq!(
                user_queries_in_order(history, id),
                vec!["prompt1".to_owned(), "prompt2".to_owned()],
                "prompt1 and prompt2 should both have been sent, each as its own exchange"
            );
        });
        controller.read(&app, |controller, ctx| {
            let streams = controller
                .in_flight_response_streams
                .stream_ids_for_conversation(id, ctx);
            assert_eq!(
                streams.len(),
                1,
                "prompt2's turn should be active; prompt1's was interrupted the same way a \
                 rapid live follow-up interrupts a prior turn"
            );
        });
    });
}

#[test]
fn live_injection_while_a_turn_is_active_is_queued_instead_of_interrupting_it() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let terminal = add_window_with_terminal(&mut app, None);
        let controller = terminal.read(&app, |terminal, _| terminal.ai_controller().clone());
        let id = controller.update(&mut app, |controller, ctx| {
            controller.bind_native_prompt_conversation(None, ctx)
        });
        terminal.update(&mut app, |terminal, ctx| {
            terminal.enter_agent_view(None, Some(id), AgentViewEntryOrigin::Cli, ctx);
        });
        let initial_streams = controller.update(&mut app, |controller, ctx| {
            controller.send_user_query_in_conversation("prompt1".into(), id, None, ctx);
            controller.dispatch_queued_warp_agent_prompt(id, None, ctx);
            assert!(!QueuedQueryModel::as_ref(ctx).has_queue(id));
            controller
                .in_flight_response_streams
                .stream_ids_for_conversation(id, ctx)
        });
        assert_eq!(initial_streams.len(), 1);

        // A stream is already active for the conversation, so this must be queued rather than
        // dispatched immediately -- dispatching now would interrupt prompt1's turn before it
        // produced any output, silently dropping it.
        controller.update(&mut app, |controller, ctx| {
            controller.execute_warp_agent_prompt_from_shared_session_injection(
                "prompt2".into(),
                None,
                vec![],
                ParticipantId::new(),
                ctx,
            );
        });
        controller.read(&app, |controller, ctx| {
            assert_eq!(
                QueuedQueryModel::as_ref(ctx)
                    .queue(id)
                    .iter()
                    .map(QueuedQuery::text)
                    .collect::<Vec<_>>(),
                vec!["prompt2"],
                "prompt2 should be queued, not dispatched, while prompt1's turn is active"
            );
            let after_streams = controller
                .in_flight_response_streams
                .stream_ids_for_conversation(id, ctx);
            assert_eq!(
                after_streams, initial_streams,
                "prompt1's stream should be untouched"
            );
        });
        BlocklistAIHistoryModel::handle(&app).read(&app, |history, _| {
            assert_eq!(
                user_queries_in_order(history, id),
                vec!["prompt1".to_owned()],
                "prompt2 should not have been sent yet"
            );
        });
    });
}

#[test]
fn dispatch_queued_warp_agent_prompt_with_an_explicit_id_targets_that_row_not_the_head() {
    // Regression test: "Send now" on a specific queued row must dispatch that exact row, even
    // when it isn't the head, rather than silently substituting whatever's at the head.
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let terminal = add_window_with_terminal(&mut app, None);
        let controller = terminal.read(&app, |terminal, _| terminal.ai_controller().clone());
        let participant = ParticipantId::new();
        let (id, second_row_id) = controller.update(&mut app, |controller, ctx| {
            let id = controller.bind_native_prompt_conversation(None, ctx);
            controller.execute_warp_agent_prompt_from_shared_session_injection(
                "prompt2".into(),
                None,
                vec![],
                participant.clone(),
                ctx,
            );
            controller.execute_warp_agent_prompt_from_shared_session_injection(
                "prompt3".into(),
                None,
                vec![],
                participant.clone(),
                ctx,
            );
            let second_row_id = QueuedQueryModel::as_ref(ctx).queue(id)[1].id();
            (id, second_row_id)
        });
        terminal.update(&mut app, |terminal, ctx| {
            terminal.enter_agent_view(None, Some(id), AgentViewEntryOrigin::Cli, ctx);
        });

        controller.update(&mut app, |controller, ctx| {
            controller.dispatch_queued_warp_agent_prompt(id, Some(second_row_id), ctx);
        });

        QueuedQueryModel::handle(&app).read(&app, |queue, _| {
            assert_eq!(
                queue
                    .queue(id)
                    .iter()
                    .map(QueuedQuery::text)
                    .collect::<Vec<_>>(),
                vec!["prompt2"],
                "the targeted row (prompt3) should be dispatched and removed; prompt2 -- the \
                 head -- should be untouched"
            );
        });
        BlocklistAIHistoryModel::handle(&app).read(&app, |history, _| {
            assert_eq!(
                user_queries_in_order(history, id),
                vec!["prompt3".to_owned()],
                "the explicitly targeted row should have been sent"
            );
        });
    });
}

#[test]
fn dispatch_queued_warp_agent_prompt_respects_fifo_order_across_mixed_row_kinds() {
    // Regression test: the automatic (head-only) dispatch path must not special-case shared-
    // session rows -- a local prompt sitting ahead of a shared-session row in the queue must
    // still be the one dispatched, not silently skipped.
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let terminal = add_window_with_terminal(&mut app, None);
        let controller = terminal.read(&app, |terminal, _| terminal.ai_controller().clone());
        let id = controller.update(&mut app, |controller, ctx| {
            let id = controller.bind_native_prompt_conversation(None, ctx);
            QueuedQueryModel::handle(ctx).update(ctx, |queue, ctx| {
                queue.append(
                    id,
                    QueuedQuery::new("local prompt".into(), QueuedQueryOrigin::QueueSlashCommand),
                    ctx,
                );
            });
            controller.execute_warp_agent_prompt_from_shared_session_injection(
                "shared prompt".into(),
                None,
                vec![],
                ParticipantId::new(),
                ctx,
            );
            id
        });
        terminal.update(&mut app, |terminal, ctx| {
            terminal.enter_agent_view(None, Some(id), AgentViewEntryOrigin::Cli, ctx);
        });

        controller.update(&mut app, |controller, ctx| {
            controller.send_user_query_in_conversation("initial".into(), id, None, ctx);
            controller.dispatch_queued_warp_agent_prompt(id, None, ctx);
        });

        QueuedQueryModel::handle(&app).read(&app, |queue, _| {
            assert_eq!(
                queue
                    .queue(id)
                    .iter()
                    .map(QueuedQuery::text)
                    .collect::<Vec<_>>(),
                vec!["shared prompt"],
                "the local prompt at the head should have been dispatched, in FIFO order; the \
                 shared-session row behind it should still be waiting"
            );
        });
        BlocklistAIHistoryModel::handle(&app).read(&app, |history, _| {
            assert_eq!(
                user_queries_in_order(history, id),
                vec!["initial".to_owned(), "local prompt".to_owned()],
            );
        });
    });
}

#[test]
fn route_native_startup_injection_rejects_a_prompt_targeting_a_different_conversation() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        let other_token_str = "550e8400-e29b-41d4-a716-446655440b00";
        let other_id = terminal.update(&mut app, |terminal, ctx| {
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                let other_id =
                    history.start_new_conversation(terminal.id(), false, false, false, ctx);
                history.set_server_conversation_token_for_conversation(
                    other_id,
                    other_token_str.to_owned(),
                );
                other_id
            })
        });
        let other_token =
            ServerConversationToken::from_uuid(Uuid::parse_str(other_token_str).unwrap());

        terminal.update(&mut app, |terminal, ctx| {
            terminal.ai_controller().update(ctx, |controller, ctx| {
                let id = controller.bind_native_prompt_conversation(None, ctx);
                controller.execute_warp_agent_prompt_from_shared_session_injection(
                    "wrong target".into(),
                    Some(other_token),
                    vec![],
                    ParticipantId::new(),
                    ctx,
                );
                assert!(!QueuedQueryModel::as_ref(ctx).has_queue(id));
            });
        });
        BlocklistAIHistoryModel::handle(&app).read(&app, |history, _| {
            assert_eq!(
                history.conversation(&other_id).unwrap().exchange_count(),
                0,
                "the wrongly-targeted conversation must not have received the prompt"
            );
        });
    });
}

#[test]
fn drained_injection_stages_attachments_and_attributes_the_exchange_to_its_participant() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let terminal = add_window_with_terminal(&mut app, None);
        let controller = terminal.read(&app, |terminal, _| terminal.ai_controller().clone());
        let participant = ParticipantId::new();
        let id = controller.update(&mut app, |controller, ctx| {
            let id = controller.bind_native_prompt_conversation(None, ctx);
            controller.execute_warp_agent_prompt_from_shared_session_injection(
                "prompt2".into(),
                None,
                vec![AgentAttachment::PlainText {
                    content: "remote context".into(),
                }],
                participant.clone(),
                ctx,
            );
            id
        });
        terminal.update(&mut app, |terminal, ctx| {
            terminal.enter_agent_view(None, Some(id), AgentViewEntryOrigin::Cli, ctx);
        });
        controller.update(&mut app, |controller, ctx| {
            controller.send_user_query_in_conversation("prompt1".into(), id, None, ctx);
            controller.dispatch_queued_warp_agent_prompt(id, None, ctx);
        });

        BlocklistAIHistoryModel::handle(&app).read(&app, |history, _| {
            let conversation = history.conversation(&id).unwrap();
            let exchange = conversation
                .root_task_exchanges()
                .find(|exchange| {
                    exchange.input.iter().any(|input| {
                        matches!(input, AIAgentInput::UserQuery { query, .. } if query == "prompt2")
                    })
                })
                .expect("prompt2's exchange should exist");
            assert_eq!(exchange.response_initiator, Some(participant));
            let context = exchange
                .input
                .iter()
                .find_map(|input| match input {
                    AIAgentInput::UserQuery { context, .. } => Some(context),
                    _ => None,
                })
                .unwrap();
            let selected: Vec<_> = context
                .iter()
                .filter_map(|context| match context {
                    AIAgentContext::SelectedText(text) => Some(text.as_str()),
                    _ => None,
                })
                .collect();
            assert_eq!(selected, vec!["remote context"]);
        });
    });
}

#[test]
fn startup_injections_reuse_the_restored_conversation() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |terminal, ctx| {
            let restored = BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                history.start_new_conversation(terminal.id(), false, false, false, ctx)
            });
            terminal.ai_controller().update(ctx, |controller, ctx| {
                assert_eq!(
                    controller.bind_native_prompt_conversation(Some(restored), ctx),
                    restored
                );
                assert_eq!(
                    controller.bind_native_prompt_conversation(None, ctx),
                    restored
                );
                controller.execute_warp_agent_prompt_from_shared_session_injection(
                    "followup".into(),
                    None,
                    vec![],
                    ParticipantId::new(),
                    ctx,
                );
                let row = &QueuedQueryModel::as_ref(ctx).queue(restored)[0];
                assert_eq!(row.text(), "followup");
                assert_eq!(row.origin(), QueuedQueryOrigin::SharedSessionInjection);
                assert_eq!(
                    BlocklistAIHistoryModel::as_ref(ctx)
                        .conversation(&restored)
                        .unwrap()
                        .exchange_count(),
                    0
                );
            });
        });
    });
}

#[test]
fn native_initial_prompt_uses_its_bound_conversation_without_agent_view() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(false);
        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |terminal, ctx| {
            terminal.ai_controller().update(ctx, |controller, ctx| {
                let id = controller.bind_native_prompt_conversation(None, ctx);
                controller.execute_warp_agent_prompt_from_shared_session_injection(
                    "followup".into(),
                    None,
                    vec![],
                    ParticipantId::new(),
                    ctx,
                );
                controller.send_ai_input_with_context(
                    |context| AIAgentInput::StartFromAmbientRunPrompt {
                        ambient_run_id: "550e8400-e29b-41d4-a716-446655440a00".into(),
                        context,
                        runtime_skill: None,
                        attachments_dir: None,
                    },
                    ctx,
                );
                assert_eq!(
                    BlocklistAIHistoryModel::as_ref(ctx)
                        .conversation(&id)
                        .unwrap()
                        .exchange_count(),
                    1
                );
                // The initial send finishes the setup barrier immediately (it does not wait for
                // this turn to complete); the queued follow-up is left untouched until something
                // explicitly drains it (`AgentDriver::execute_run` does so right after this send
                // in production).
                assert!(!QueuedQueryModel::as_ref(ctx).is_dispatch_blocked(id));
                assert_eq!(QueuedQueryModel::as_ref(ctx).queue(id).len(), 1);
            });
        });
    });
}

#[test]
fn unbind_native_prompt_conversation_releases_the_setup_barrier() {
    // Regression test: unbinding while setup never finished must not leave the conversation
    // permanently dispatch-blocked, since nothing else would ever release that barrier.
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |terminal, ctx| {
            terminal.ai_controller().update(ctx, |controller, ctx| {
                let id = controller.bind_native_prompt_conversation(None, ctx);
                assert!(QueuedQueryModel::as_ref(ctx).is_dispatch_blocked(id));

                controller.unbind_native_prompt_conversation(ctx);

                assert!(!QueuedQueryModel::as_ref(ctx).is_dispatch_blocked(id));
                assert!(!QueuedQueryModel::as_ref(ctx).has_pending_native_injections(id));
            });
        });
    });
}

#[test]
fn unbind_native_prompt_conversation_drops_any_prompts_still_queued() {
    // Covers both a shared-session-injected row and a plain local row (e.g. from `/queue`
    // against this same conversation while it's native-bound) -- unbinding must clear the
    // whole queue, not just the shared-session-injected rows within it.
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |terminal, ctx| {
            terminal.ai_controller().update(ctx, |controller, ctx| {
                let id = controller.bind_native_prompt_conversation(None, ctx);
                controller.execute_warp_agent_prompt_from_shared_session_injection(
                    "pending".into(),
                    None,
                    vec![],
                    ParticipantId::new(),
                    ctx,
                );
                QueuedQueryModel::handle(ctx).update(ctx, |queue, ctx| {
                    queue.append(
                        id,
                        QueuedQuery::new(
                            "local pending".into(),
                            QueuedQueryOrigin::QueueSlashCommand,
                        ),
                        ctx,
                    );
                });
                assert_eq!(QueuedQueryModel::as_ref(ctx).queue(id).len(), 2);

                controller.unbind_native_prompt_conversation(ctx);

                assert_eq!(controller.native_prompt_conversation_id(), None);
                assert!(!QueuedQueryModel::as_ref(ctx).has_queue(id));
                assert_eq!(
                    BlocklistAIHistoryModel::as_ref(ctx)
                        .conversation(&id)
                        .unwrap()
                        .exchange_count(),
                    0
                );
            });
        });
    });
}
