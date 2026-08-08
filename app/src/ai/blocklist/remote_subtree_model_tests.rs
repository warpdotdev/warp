use std::collections::HashMap;

use chrono::Utc;
use warpui::{App, EntityId};

use super::*;
use crate::ai::agent::conversation::ConversationStatus;
use crate::ai::ambient_agents::AgentConfigSnapshot;
use crate::test_util::settings::initialize_history_persistence_for_tests;

fn make_child_task(task_id: AmbientAgentTaskId, state: AmbientAgentTaskState) -> AmbientAgentTask {
    AmbientAgentTask {
        task_id,
        parent_run_id: None,
        title: "Investigate flaky test".to_string(),
        state,
        prompt: "prompt".to_string(),
        created_at: Utc::now(),
        started_at: Some(Utc::now()),
        updated_at: Utc::now(),
        run_time: None,
        status_message: None,
        source: None,
        execution_location: None,
        session_id: None,
        session_link: None,
        creator: None,
        executor: None,
        conversation_id: None,
        request_usage: None,
        agent_config_snapshot: Some(AgentConfigSnapshot {
            name: Some("grandchild".to_string()),
            ..Default::default()
        }),
        artifacts: vec![],
        is_sandbox_running: false,
        last_event_sequence: None,
        children: vec![],
    }
}

#[test]
fn watch_ignores_non_remote_conversations() {
    App::test((), |mut app| async move {
        initialize_history_persistence_for_tests(&mut app);
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let terminal_view_id = EntityId::new();
        let conversation_id = history_model.update(&mut app, |history, ctx| {
            history.start_new_conversation(terminal_view_id, false, false, false, ctx)
        });

        let model = app.add_singleton_model(RemoteSubtreeModel::new);
        model.update(&mut app, |me, ctx| {
            me.watch(conversation_id, ctx);
        });
        model.read(&app, |me, _| {
            assert!(
                me.entries.is_empty(),
                "a local (non-remote) conversation must not be watched"
            );
        });
    });
}

#[test]
fn dormancy_requires_a_final_sweep_after_the_node_terminates() {
    App::test((), |mut app| async move {
        initialize_history_persistence_for_tests(&mut app);
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let terminal_view_id = EntityId::new();
        let mid_task_id: AmbientAgentTaskId =
            "550e8400-e29b-41d4-a716-446655440610".parse().unwrap();

        // A watched node that terminated quickly (before any poll fired).
        let mid_id = history_model.update(&mut app, |history, ctx| {
            let id = history.start_new_conversation(terminal_view_id, false, false, false, ctx);
            history.update_conversation_status(
                terminal_view_id,
                id,
                ConversationStatus::Success,
                ctx,
            );
            id
        });

        let entry = RemoteSubtreeEntry {
            task_id: mid_task_id,
            children: HashMap::new(),
            fetch_in_flight: false,
            final_sweep_done: false,
        };
        history_model.read(&app, |history, _| {
            assert!(
                subtree_may_still_change(&mid_id, &entry, history),
                "a terminal node must keep polling until a post-terminal fetch completed",
            );
        });

        let entry = RemoteSubtreeEntry {
            final_sweep_done: true,
            ..entry
        };
        history_model.read(&app, |history, _| {
            assert!(
                !subtree_may_still_change(&mid_id, &entry, history),
                "after the final sweep a fully terminal subtree goes dormant",
            );
        });
    });
}

#[test]
fn bulk_surface_clear_unwatches_cleared_conversations() {
    App::test((), |mut app| async move {
        initialize_history_persistence_for_tests(&mut app);
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let terminal_view_id = EntityId::new();
        let cleared_task_id: AmbientAgentTaskId =
            "550e8400-e29b-41d4-a716-446655440620".parse().unwrap();
        let surviving_task_id: AmbientAgentTaskId =
            "550e8400-e29b-41d4-a716-446655440621".parse().unwrap();

        let (cleared_id, surviving_id) = history_model.update(&mut app, |history, ctx| {
            let cleared =
                history.start_new_conversation(terminal_view_id, false, false, false, ctx);
            let surviving =
                history.start_new_conversation(EntityId::new(), false, false, false, ctx);
            (cleared, surviving)
        });

        let model = app.add_singleton_model(RemoteSubtreeModel::new);
        model.update(&mut app, |me, _| {
            me.entries.insert(
                cleared_id,
                RemoteSubtreeEntry {
                    task_id: cleared_task_id,
                    children: HashMap::new(),
                    fetch_in_flight: false,
                    final_sweep_done: false,
                },
            );
            // An unrelated watched node referencing the cleared conversation as a child.
            let children = HashMap::from([(
                cleared_task_id,
                RemoteSubtreeChild {
                    conversation_id: cleared_id,
                    last_state: None,
                },
            )]);
            me.entries.insert(
                surviving_id,
                RemoteSubtreeEntry {
                    task_id: surviving_task_id,
                    children,
                    fetch_in_flight: false,
                    final_sweep_done: false,
                },
            );
        });

        model.update(&mut app, |me, _| {
            me.handle_history_event(
                &BlocklistAIHistoryEvent::ClearedConversationsForTerminalSurface {
                    terminal_surface_id: terminal_view_id,
                    active_conversation_id: None,
                    cleared_conversation_ids: vec![cleared_id],
                },
            );
        });

        model.read(&app, |me, _| {
            assert!(
                !me.entries.contains_key(&cleared_id),
                "cleared conversations must stop being watched"
            );
            let surviving = me
                .entries
                .get(&surviving_id)
                .expect("unrelated entry survives the clear");
            assert!(
                surviving.children.is_empty(),
                "child references to cleared conversations must be pruned"
            );
        });
    });
}

#[test]
fn register_or_update_child_materializes_placeholder_and_updates_status() {
    App::test((), |mut app| async move {
        initialize_history_persistence_for_tests(&mut app);
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let terminal_view_id = EntityId::new();
        let mid_task_id: AmbientAgentTaskId =
            "550e8400-e29b-41d4-a716-446655440600".parse().unwrap();
        let child_task_id: AmbientAgentTaskId =
            "550e8400-e29b-41d4-a716-446655440601".parse().unwrap();

        // The watched node: a remote-child placeholder hosted on a surface.
        let mid_id = history_model.update(&mut app, |history, ctx| {
            let id = history.start_new_conversation(terminal_view_id, false, false, false, ctx);
            history.mark_conversation_as_remote_child(id, ctx);
            history.assign_run_id_for_conversation(
                id,
                mid_task_id.to_string(),
                Some(mid_task_id),
                terminal_view_id,
                ctx,
            );
            id
        });

        let model = app.add_singleton_model(RemoteSubtreeModel::new);
        model.update(&mut app, |me, _| {
            me.entries.insert(
                mid_id,
                RemoteSubtreeEntry {
                    task_id: mid_task_id,
                    children: HashMap::new(),
                    fetch_in_flight: false,
                    final_sweep_done: false,
                },
            );
        });

        model.update(&mut app, |me, ctx| {
            me.register_or_update_child(
                mid_id,
                make_child_task(child_task_id, AmbientAgentTaskState::InProgress),
                ctx,
            );
        });

        let grandchild_id = history_model.read(&app, |history, _| {
            let children = history.child_conversation_ids_of(&mid_id);
            assert_eq!(children.len(), 1, "placeholder must join the topology");
            let grandchild_id = children[0];
            let grandchild = history
                .conversation(&grandchild_id)
                .expect("placeholder conversation exists");
            assert!(grandchild.is_remote_child());
            assert_eq!(grandchild.agent_name(), Some("grandchild"));
            assert_eq!(grandchild.status(), &ConversationStatus::InProgress);
            assert_eq!(grandchild.task_id(), Some(child_task_id));
            assert_eq!(
                history.conversation_id_for_agent_id(&child_task_id.to_string()),
                Some(grandchild_id),
                "run-id index must resolve the placeholder"
            );
            grandchild_id
        });

        // A refresh with a new server state updates the same placeholder
        // instead of duplicating it.
        model.update(&mut app, |me, ctx| {
            me.register_or_update_child(
                mid_id,
                make_child_task(child_task_id, AmbientAgentTaskState::Succeeded),
                ctx,
            );
        });
        history_model.read(&app, |history, _| {
            assert_eq!(history.child_conversation_ids_of(&mid_id).len(), 1);
            assert_eq!(
                history
                    .conversation(&grandchild_id)
                    .expect("placeholder conversation exists")
                    .status(),
                &ConversationStatus::Success,
            );
        });
    });
}
