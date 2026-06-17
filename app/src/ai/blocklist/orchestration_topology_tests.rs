use warpui::{App, EntityId, ModelHandle};

use super::*;
use crate::ai::agent::conversation::{AIConversationId, ConversationStatus};
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::test_util::settings::initialize_history_persistence_for_tests;
#[test]
fn pill_order_keys_prioritize_attention_then_in_progress_then_done() {
    let blocked = ConversationStatus::Blocked {
        blocked_action: String::new(),
    };
    let blocked_key = pill_status_sort_key(Some(&blocked));
    let error_key = pill_status_sort_key(Some(&ConversationStatus::Error));
    let in_progress_key = pill_status_sort_key(Some(&ConversationStatus::InProgress));
    let cancelled_key = pill_status_sort_key(Some(&ConversationStatus::Cancelled));
    let success_key = pill_status_sort_key(Some(&ConversationStatus::Success));

    assert!(blocked_key < error_key);
    assert!(error_key < in_progress_key);
    assert!(in_progress_key < cancelled_key);
    assert_eq!(cancelled_key, success_key);
    assert_eq!(pill_status_sort_key(None), in_progress_key);
}

#[test]
fn pill_order_keys_sort_done_conversations_by_most_recent_first() {
    let older = pill_secondary_sort_key(DONE_STATUS_KEY, Some(1_000));
    let newer = pill_secondary_sort_key(DONE_STATUS_KEY, Some(2_000));
    let unknown = pill_secondary_sort_key(DONE_STATUS_KEY, None);

    assert!(newer < older);
    assert!(older < unknown);
}

#[test]
fn descendant_conversation_ids_in_spawn_order_flattens_nested_children_preorder() {
    App::test((), |mut app| async move {
        initialize_history_persistence_for_tests(&mut app);
        let terminal_view_id = EntityId::new();
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());

        let orchestrator_id = history_model.update(&mut app, |history_model, ctx| {
            history_model.start_new_conversation(terminal_view_id, false, false, false, ctx)
        });
        let child_a = history_model.update(&mut app, |history_model, ctx| {
            history_model.start_new_child_conversation(
                terminal_view_id,
                "oz-env-check".to_string(),
                orchestrator_id,
                None,
                ctx,
            )
        });
        let child_b = history_model.update(&mut app, |history_model, ctx| {
            history_model.start_new_child_conversation(
                terminal_view_id,
                "sibling-agent".to_string(),
                orchestrator_id,
                None,
                ctx,
            )
        });
        let grandchild_a1 = history_model.update(&mut app, |history_model, ctx| {
            history_model.start_new_child_conversation(
                terminal_view_id,
                "codex-child".to_string(),
                child_a,
                None,
                ctx,
            )
        });
        let grandchild_a2 = history_model.update(&mut app, |history_model, ctx| {
            history_model.start_new_child_conversation(
                terminal_view_id,
                "follow-up-child".to_string(),
                child_a,
                None,
                ctx,
            )
        });
        let grandchild_b1 = history_model.update(&mut app, |history_model, ctx| {
            history_model.start_new_child_conversation(
                terminal_view_id,
                "sibling-grandchild".to_string(),
                child_b,
                None,
                ctx,
            )
        });

        history_model.read(&app, |history_model, _| {
            assert_eq!(
                descendant_conversation_ids_in_spawn_order(history_model, orchestrator_id),
                vec![
                    child_a,
                    grandchild_a1,
                    grandchild_a2,
                    child_b,
                    grandchild_b1
                ],
            );
        });
    });
}

#[test]
fn adjacent_orchestration_child_navigation_uses_pinned_first_order() {
    App::test((), |mut app| async move {
        initialize_history_persistence_for_tests(&mut app);
        let terminal_view_id = EntityId::new();
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());

        let orchestrator_id = history_model.update(&mut app, |history_model, ctx| {
            history_model.start_new_conversation(terminal_view_id, false, false, false, ctx)
        });
        let child_a = history_model.update(&mut app, |history_model, ctx| {
            history_model.start_new_child_conversation(
                terminal_view_id,
                "child-a".to_string(),
                orchestrator_id,
                None,
                ctx,
            )
        });
        let child_b = history_model.update(&mut app, |history_model, ctx| {
            history_model.start_new_child_conversation(
                terminal_view_id,
                "child-b".to_string(),
                orchestrator_id,
                None,
                ctx,
            )
        });
        let child_c = history_model.update(&mut app, |history_model, ctx| {
            history_model.start_new_child_conversation(
                terminal_view_id,
                "child-c".to_string(),
                orchestrator_id,
                None,
                ctx,
            )
        });
        history_model.update(&mut app, |history_model, ctx| {
            history_model.set_conversation_pinned(child_b, true, ctx);
            history_model.set_conversation_pinned(child_c, true, ctx);
        });

        history_model.read(&app, |history_model, _| {
            assert_eq!(
                adjacent_orchestration_child_conversation_id(
                    history_model,
                    orchestrator_id,
                    OrchestrationNavigationDirection::Next,
                ),
                Some(child_b),
            );
            assert_eq!(
                adjacent_orchestration_child_conversation_id(
                    history_model,
                    child_b,
                    OrchestrationNavigationDirection::Next,
                ),
                Some(child_c),
            );
            assert_eq!(
                adjacent_orchestration_child_conversation_id(
                    history_model,
                    child_c,
                    OrchestrationNavigationDirection::Next,
                ),
                Some(child_a),
            );
            assert_eq!(
                adjacent_orchestration_child_conversation_id(
                    history_model,
                    child_a,
                    OrchestrationNavigationDirection::Next,
                ),
                Some(orchestrator_id),
            );
        });
    });
}

#[test]
fn orchestration_aware_status_uses_aggregated_status_for_known_parent() {
    App::test((), |mut app| async move {
        initialize_history_persistence_for_tests(&mut app);
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let (terminal_view_id, orchestrator_id, child_a, child_b) =
            build_orchestrator_with_two_children(&mut app, &history_model);

        history_model.update(&mut app, |history_model, ctx| {
            history_model.update_conversation_status(
                terminal_view_id,
                orchestrator_id,
                ConversationStatus::Success,
                ctx,
            );
            history_model.update_conversation_status(
                terminal_view_id,
                child_a,
                ConversationStatus::InProgress,
                ctx,
            );
            history_model.update_conversation_status(
                terminal_view_id,
                child_b,
                ConversationStatus::Success,
                ctx,
            );
        });

        history_model.read(&app, |history_model, _| {
            let orchestrator = history_model
                .conversation(&orchestrator_id)
                .expect("orchestrator conversation exists");
            assert_eq!(
                orchestration_aware_conversation_status(history_model, orchestrator),
                ConversationStatus::InProgress,
            );
        });
    });
}

#[test]
fn orchestration_aware_status_uses_direct_status_for_non_parent() {
    App::test((), |mut app| async move {
        initialize_history_persistence_for_tests(&mut app);
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let terminal_view_id = EntityId::new();
        let conversation_id = history_model.update(&mut app, |history_model, ctx| {
            let conversation_id =
                history_model.start_new_conversation(terminal_view_id, false, false, false, ctx);
            history_model.update_conversation_status(
                terminal_view_id,
                conversation_id,
                ConversationStatus::Error,
                ctx,
            );
            conversation_id
        });

        history_model.read(&app, |history_model, _| {
            let conversation = history_model
                .conversation(&conversation_id)
                .expect("conversation exists");
            assert_eq!(
                orchestration_aware_conversation_status(history_model, conversation),
                ConversationStatus::Error,
            );
        });
    });
}
#[test]
fn descendant_conversation_ids_in_spawn_order_returns_empty_without_children() {
    App::test((), |mut app| async move {
        initialize_history_persistence_for_tests(&mut app);
        let terminal_view_id = EntityId::new();
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());

        let orchestrator_id = history_model.update(&mut app, |history_model, ctx| {
            history_model.start_new_conversation(terminal_view_id, false, false, false, ctx)
        });

        history_model.read(&app, |history_model, _| {
            assert!(
                descendant_conversation_ids_in_spawn_order(history_model, orchestrator_id)
                    .is_empty()
            );
        });
    });
}

#[test]
fn adjacent_orchestration_child_navigation_enters_child_list_from_orchestrator() {
    App::test((), |mut app| async move {
        initialize_history_persistence_for_tests(&mut app);
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let (_, orchestrator_id, child_a, child_b) =
            build_orchestrator_with_two_children(&mut app, &history_model);

        history_model.read(&app, |history_model, _| {
            assert_eq!(
                adjacent_orchestration_child_conversation_id(
                    history_model,
                    orchestrator_id,
                    OrchestrationNavigationDirection::Next,
                ),
                Some(child_a),
            );
            assert_eq!(
                adjacent_orchestration_child_conversation_id(
                    history_model,
                    orchestrator_id,
                    OrchestrationNavigationDirection::Previous,
                ),
                Some(child_b),
            );
        });
    });
}

#[test]
fn adjacent_orchestration_child_navigation_wraps_within_child_list() {
    App::test((), |mut app| async move {
        initialize_history_persistence_for_tests(&mut app);
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let (_, orchestrator_id, child_a, child_b) =
            build_orchestrator_with_two_children(&mut app, &history_model);

        history_model.read(&app, |history_model, _| {
            assert_eq!(
                adjacent_orchestration_child_conversation_id(
                    history_model,
                    child_a,
                    OrchestrationNavigationDirection::Previous,
                ),
                Some(orchestrator_id),
            );
            assert_eq!(
                adjacent_orchestration_child_conversation_id(
                    history_model,
                    child_b,
                    OrchestrationNavigationDirection::Next,
                ),
                Some(orchestrator_id),
            );
        });
    });
}

#[test]
fn adjacent_orchestration_child_navigation_noops_for_single_child() {
    App::test((), |mut app| async move {
        initialize_history_persistence_for_tests(&mut app);
        let terminal_view_id = EntityId::new();
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());

        let orchestrator_id = history_model.update(&mut app, |history_model, ctx| {
            history_model.start_new_conversation(terminal_view_id, false, false, false, ctx)
        });
        let child_id = history_model.update(&mut app, |history_model, ctx| {
            history_model.start_new_child_conversation(
                terminal_view_id,
                "child".to_string(),
                orchestrator_id,
                None,
                ctx,
            )
        });

        history_model.read(&app, |history_model, _| {
            assert_eq!(
                adjacent_orchestration_child_conversation_id(
                    history_model,
                    orchestrator_id,
                    OrchestrationNavigationDirection::Next,
                ),
                Some(child_id),
            );
            assert_eq!(
                adjacent_orchestration_child_conversation_id(
                    history_model,
                    child_id,
                    OrchestrationNavigationDirection::Next,
                ),
                Some(orchestrator_id),
            );
            assert_eq!(
                adjacent_orchestration_child_conversation_id(
                    history_model,
                    child_id,
                    OrchestrationNavigationDirection::Previous,
                ),
                Some(orchestrator_id),
            );
        });
    });
}

/// Convenience: build an orchestrator with two children for status-aggregation
/// tests so individual cases stay focused on the precedence logic.
fn build_orchestrator_with_two_children(
    app: &mut App,
    history_model: &ModelHandle<BlocklistAIHistoryModel>,
) -> (
    EntityId,
    AIConversationId,
    AIConversationId,
    AIConversationId,
) {
    let terminal_view_id = EntityId::new();
    let orchestrator_id = history_model.update(app, |history_model, ctx| {
        history_model.start_new_conversation(terminal_view_id, false, false, false, ctx)
    });
    let child_a = history_model.update(app, |history_model, ctx| {
        history_model.start_new_child_conversation(
            terminal_view_id,
            "child-a".to_string(),
            orchestrator_id,
            None,
            ctx,
        )
    });
    let child_b = history_model.update(app, |history_model, ctx| {
        history_model.start_new_child_conversation(
            terminal_view_id,
            "child-b".to_string(),
            orchestrator_id,
            None,
            ctx,
        )
    });
    (terminal_view_id, orchestrator_id, child_a, child_b)
}

#[test]
fn aggregated_status_is_in_progress_when_any_descendant_is_running() {
    App::test((), |mut app| async move {
        initialize_history_persistence_for_tests(&mut app);
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let (terminal_view_id, orchestrator_id, child_a, child_b) =
            build_orchestrator_with_two_children(&mut app, &history_model);

        // Orchestrator's own turn already finished, but one child is still
        // running and another has errored. The aggregated status should
        // privilege the running child so the pill stays "in progress".
        history_model.update(&mut app, |history_model, ctx| {
            history_model.update_conversation_status(
                terminal_view_id,
                orchestrator_id,
                ConversationStatus::Success,
                ctx,
            );
            history_model.update_conversation_status(
                terminal_view_id,
                child_a,
                ConversationStatus::InProgress,
                ctx,
            );
            history_model.update_conversation_status(
                terminal_view_id,
                child_b,
                ConversationStatus::Error,
                ctx,
            );
        });

        history_model.read(&app, |history_model, _| {
            assert_eq!(
                aggregated_orchestrator_status(history_model, orchestrator_id),
                ConversationStatus::InProgress,
            );
        });
    });
}

#[test]
fn aggregated_status_prefers_blocked_over_terminal_states() {
    App::test((), |mut app| async move {
        initialize_history_persistence_for_tests(&mut app);
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let (terminal_view_id, orchestrator_id, child_a, child_b) =
            build_orchestrator_with_two_children(&mut app, &history_model);

        // Nothing is running, but one child is blocked waiting on user input.
        // The aggregated status should surface the blocked state so the user
        // notices attention is needed somewhere in the tree.
        history_model.update(&mut app, |history_model, ctx| {
            history_model.update_conversation_status(
                terminal_view_id,
                orchestrator_id,
                ConversationStatus::Success,
                ctx,
            );
            history_model.update_conversation_status(
                terminal_view_id,
                child_a,
                ConversationStatus::Blocked {
                    blocked_action: "approve_command".to_string(),
                },
                ctx,
            );
            history_model.update_conversation_status(
                terminal_view_id,
                child_b,
                ConversationStatus::Error,
                ctx,
            );
        });

        history_model.read(&app, |history_model, _| {
            assert_eq!(
                aggregated_orchestrator_status(history_model, orchestrator_id),
                ConversationStatus::Blocked {
                    blocked_action: "approve_command".to_string(),
                },
            );
        });
    });
}

#[test]
fn aggregated_status_falls_back_to_worst_terminal_outcome() {
    App::test((), |mut app| async move {
        initialize_history_persistence_for_tests(&mut app);
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let (terminal_view_id, orchestrator_id, child_a, child_b) =
            build_orchestrator_with_two_children(&mut app, &history_model);

        // Nothing in progress or blocked, but one child errored: Error wins
        // over both Cancelled and Success.
        history_model.update(&mut app, |history_model, ctx| {
            history_model.update_conversation_status(
                terminal_view_id,
                orchestrator_id,
                ConversationStatus::Success,
                ctx,
            );
            history_model.update_conversation_status(
                terminal_view_id,
                child_a,
                ConversationStatus::Error,
                ctx,
            );
            history_model.update_conversation_status(
                terminal_view_id,
                child_b,
                ConversationStatus::Cancelled,
                ctx,
            );
        });

        history_model.read(&app, |history_model, _| {
            assert_eq!(
                aggregated_orchestrator_status(history_model, orchestrator_id),
                ConversationStatus::Error,
            );
        });
    });
}

#[test]
fn aggregated_status_is_cancelled_when_no_errors_present() {
    App::test((), |mut app| async move {
        initialize_history_persistence_for_tests(&mut app);
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let (terminal_view_id, orchestrator_id, child_a, child_b) =
            build_orchestrator_with_two_children(&mut app, &history_model);

        history_model.update(&mut app, |history_model, ctx| {
            history_model.update_conversation_status(
                terminal_view_id,
                orchestrator_id,
                ConversationStatus::Success,
                ctx,
            );
            history_model.update_conversation_status(
                terminal_view_id,
                child_a,
                ConversationStatus::Cancelled,
                ctx,
            );
            history_model.update_conversation_status(
                terminal_view_id,
                child_b,
                ConversationStatus::Success,
                ctx,
            );
        });

        history_model.read(&app, |history_model, _| {
            assert_eq!(
                aggregated_orchestrator_status(history_model, orchestrator_id),
                ConversationStatus::Cancelled,
            );
        });
    });
}

#[test]
fn aggregated_status_is_success_when_orchestrator_and_all_descendants_succeeded() {
    App::test((), |mut app| async move {
        initialize_history_persistence_for_tests(&mut app);
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let (terminal_view_id, orchestrator_id, child_a, child_b) =
            build_orchestrator_with_two_children(&mut app, &history_model);

        history_model.update(&mut app, |history_model, ctx| {
            history_model.update_conversation_status(
                terminal_view_id,
                orchestrator_id,
                ConversationStatus::Success,
                ctx,
            );
            history_model.update_conversation_status(
                terminal_view_id,
                child_a,
                ConversationStatus::Success,
                ctx,
            );
            history_model.update_conversation_status(
                terminal_view_id,
                child_b,
                ConversationStatus::Success,
                ctx,
            );
        });

        history_model.read(&app, |history_model, _| {
            assert_eq!(
                aggregated_orchestrator_status(history_model, orchestrator_id),
                ConversationStatus::Success,
            );
        });
    });
}

#[test]
fn aggregated_status_respects_orchestrator_own_in_progress_state() {
    App::test((), |mut app| async move {
        initialize_history_persistence_for_tests(&mut app);
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let (terminal_view_id, orchestrator_id, child_a, child_b) =
            build_orchestrator_with_two_children(&mut app, &history_model);

        // Orchestrator itself is running; descendants are all idle. The
        // aggregation must still report InProgress.
        history_model.update(&mut app, |history_model, ctx| {
            history_model.update_conversation_status(
                terminal_view_id,
                orchestrator_id,
                ConversationStatus::InProgress,
                ctx,
            );
            history_model.update_conversation_status(
                terminal_view_id,
                child_a,
                ConversationStatus::Success,
                ctx,
            );
            history_model.update_conversation_status(
                terminal_view_id,
                child_b,
                ConversationStatus::Success,
                ctx,
            );
        });

        history_model.read(&app, |history_model, _| {
            assert_eq!(
                aggregated_orchestrator_status(history_model, orchestrator_id),
                ConversationStatus::InProgress,
            );
        });
    });
}

// Aggregation precedence: `InProgress > Blocked > WaitingForEvents > Error >
// Cancelled > Success`. Two carve-outs are pinned below: orchestrator
// `WaitingForEvents` outranks descendant `InProgress`, and a terminal
// orchestrator (`Cancelled`/`Error`) outranks a descendant `WaitingForEvents`.

#[test]
fn aggregated_status_is_waiting_when_orchestrator_yields_and_children_succeeded() {
    App::test((), |mut app| async move {
        initialize_history_persistence_for_tests(&mut app);
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let (terminal_view_id, orchestrator_id, child_a, child_b) =
            build_orchestrator_with_two_children(&mut app, &history_model);

        // Orchestrator yielded via `wait_for_events`; all children finished
        // cleanly. The tree is quiescent but not terminal — the aggregator
        // must report WaitingForEvents so the pill bar reflects that the
        // run is listening for inbound input.
        history_model.update(&mut app, |history_model, ctx| {
            history_model.update_conversation_status(
                terminal_view_id,
                orchestrator_id,
                ConversationStatus::WaitingForEvents,
                ctx,
            );
            history_model.update_conversation_status(
                terminal_view_id,
                child_a,
                ConversationStatus::Success,
                ctx,
            );
            history_model.update_conversation_status(
                terminal_view_id,
                child_b,
                ConversationStatus::Success,
                ctx,
            );
        });

        history_model.read(&app, |history_model, _| {
            assert_eq!(
                aggregated_orchestrator_status(history_model, orchestrator_id),
                ConversationStatus::WaitingForEvents,
            );
        });
    });
}

#[test]
fn aggregated_status_prefers_parent_waiting_over_descendant_in_progress() {
    App::test((), |mut app| async move {
        initialize_history_persistence_for_tests(&mut app);
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let (terminal_view_id, orchestrator_id, child_a, child_b) =
            build_orchestrator_with_two_children(&mut app, &history_model);

        // Parent waiting outranks descendant in-progress.
        history_model.update(&mut app, |history_model, ctx| {
            history_model.update_conversation_status(
                terminal_view_id,
                orchestrator_id,
                ConversationStatus::WaitingForEvents,
                ctx,
            );
            history_model.update_conversation_status(
                terminal_view_id,
                child_a,
                ConversationStatus::InProgress,
                ctx,
            );
            history_model.update_conversation_status(
                terminal_view_id,
                child_b,
                ConversationStatus::Success,
                ctx,
            );
        });

        history_model.read(&app, |history_model, _| {
            assert_eq!(
                aggregated_orchestrator_status(history_model, orchestrator_id),
                ConversationStatus::WaitingForEvents,
            );
        });
    });
}

#[test]
fn aggregated_status_prefers_cancelled_parent_over_descendant_waiting_for_events() {
    App::test((), |mut app| async move {
        initialize_history_persistence_for_tests(&mut app);
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let (terminal_view_id, orchestrator_id, child_a, child_b) =
            build_orchestrator_with_two_children(&mut app, &history_model);

        // Terminal parent beats descendant waiting: a Cancelled orchestrator
        // with a child still listening for events must surface as Cancelled
        // — the run can't resume on its own once the parent is finalized.
        history_model.update(&mut app, |history_model, ctx| {
            history_model.update_conversation_status(
                terminal_view_id,
                orchestrator_id,
                ConversationStatus::Cancelled,
                ctx,
            );
            history_model.update_conversation_status(
                terminal_view_id,
                child_a,
                ConversationStatus::WaitingForEvents,
                ctx,
            );
            history_model.update_conversation_status(
                terminal_view_id,
                child_b,
                ConversationStatus::Success,
                ctx,
            );
        });

        history_model.read(&app, |history_model, _| {
            assert_eq!(
                aggregated_orchestrator_status(history_model, orchestrator_id),
                ConversationStatus::Cancelled,
            );
        });
    });
}

#[test]
fn aggregated_status_prefers_errored_parent_over_descendant_waiting_for_events() {
    App::test((), |mut app| async move {
        initialize_history_persistence_for_tests(&mut app);
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let (terminal_view_id, orchestrator_id, child_a, child_b) =
            build_orchestrator_with_two_children(&mut app, &history_model);

        // Symmetric to the Cancelled case: an Errored parent still wins over
        // a descendant WaitingForEvents.
        history_model.update(&mut app, |history_model, ctx| {
            history_model.update_conversation_status(
                terminal_view_id,
                orchestrator_id,
                ConversationStatus::Error,
                ctx,
            );
            history_model.update_conversation_status(
                terminal_view_id,
                child_a,
                ConversationStatus::WaitingForEvents,
                ctx,
            );
            history_model.update_conversation_status(
                terminal_view_id,
                child_b,
                ConversationStatus::Success,
                ctx,
            );
        });

        history_model.read(&app, |history_model, _| {
            assert_eq!(
                aggregated_orchestrator_status(history_model, orchestrator_id),
                ConversationStatus::Error,
            );
        });
    });
}

#[test]
fn aggregated_status_returns_in_progress_when_parent_is_in_progress_too() {
    App::test((), |mut app| async move {
        initialize_history_persistence_for_tests(&mut app);
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let (terminal_view_id, orchestrator_id, child_a, child_b) =
            build_orchestrator_with_two_children(&mut app, &history_model);

        // The parent-waits carve-out only kicks in when the orchestrator
        // itself is `WaitingForEvents`. If the orchestrator is actively
        // in progress alongside its children, `InProgress` wins (this
        // is the original aggregation precedence).
        history_model.update(&mut app, |history_model, ctx| {
            history_model.update_conversation_status(
                terminal_view_id,
                orchestrator_id,
                ConversationStatus::InProgress,
                ctx,
            );
            history_model.update_conversation_status(
                terminal_view_id,
                child_a,
                ConversationStatus::InProgress,
                ctx,
            );
            history_model.update_conversation_status(
                terminal_view_id,
                child_b,
                ConversationStatus::Success,
                ctx,
            );
        });

        history_model.read(&app, |history_model, _| {
            assert_eq!(
                aggregated_orchestrator_status(history_model, orchestrator_id),
                ConversationStatus::InProgress,
            );
        });
    });
}

#[test]
fn aggregated_status_prefers_blocked_over_waiting_for_events() {
    App::test((), |mut app| async move {
        initialize_history_persistence_for_tests(&mut app);
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let (terminal_view_id, orchestrator_id, child_a, child_b) =
            build_orchestrator_with_two_children(&mut app, &history_model);

        // Orchestrator is waiting for events, but a child is blocked on user
        // input. Blocked outranks WaitingForEvents because the user needs to
        // unblock the tree before it can make progress.
        history_model.update(&mut app, |history_model, ctx| {
            history_model.update_conversation_status(
                terminal_view_id,
                orchestrator_id,
                ConversationStatus::WaitingForEvents,
                ctx,
            );
            history_model.update_conversation_status(
                terminal_view_id,
                child_a,
                ConversationStatus::Blocked {
                    blocked_action: "approve_command".to_string(),
                },
                ctx,
            );
            history_model.update_conversation_status(
                terminal_view_id,
                child_b,
                ConversationStatus::Success,
                ctx,
            );
        });

        history_model.read(&app, |history_model, _| {
            assert_eq!(
                aggregated_orchestrator_status(history_model, orchestrator_id),
                ConversationStatus::Blocked {
                    blocked_action: "approve_command".to_string(),
                },
            );
        });
    });
}

#[test]
fn aggregated_status_prefers_waiting_for_events_over_error() {
    App::test((), |mut app| async move {
        initialize_history_persistence_for_tests(&mut app);
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let (terminal_view_id, orchestrator_id, child_a, child_b) =
            build_orchestrator_with_two_children(&mut app, &history_model);

        // Orchestrator is waiting; one child errored. `WaitingForEvents`
        // outranks `Error` because the run is not terminal — it may still
        // resume on its own and the user shouldn't see a terminal
        // "Error" pill while the driver is still alive.
        history_model.update(&mut app, |history_model, ctx| {
            history_model.update_conversation_status(
                terminal_view_id,
                orchestrator_id,
                ConversationStatus::WaitingForEvents,
                ctx,
            );
            history_model.update_conversation_status(
                terminal_view_id,
                child_a,
                ConversationStatus::Error,
                ctx,
            );
            history_model.update_conversation_status(
                terminal_view_id,
                child_b,
                ConversationStatus::Success,
                ctx,
            );
        });

        history_model.read(&app, |history_model, _| {
            assert_eq!(
                aggregated_orchestrator_status(history_model, orchestrator_id),
                ConversationStatus::WaitingForEvents,
            );
        });
    });
}
