use warpui::{App, EntityId, ModelHandle};

use super::*;
use crate::ai::agent::conversation::{AIConversationId, ConversationStatus};
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::test_util::settings::initialize_history_persistence_for_tests;

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
