#![allow(deprecated)]
use super::*;
use crate::ai::blocklist::BlocklistAIHistoryModel;
use std::collections::HashSet;
use warp_core::features::FeatureFlag;
use warp_multi_agent_api as api;
use warpui::{App, EntityId};
// Helper for constructing lifecycle pending events with minimal boilerplate.
// Tests use this to focus on queue/coalescing behavior rather than payload setup.

fn lifecycle_pending_event(
    event_id: &str,
    sender_agent_id: &str,
    event_type: api::LifecycleEventType,
    attempt_count: i32,
) -> PendingEvent {
    let detail = match event_type {
        api::LifecycleEventType::Started => {
            Some(api::agent_event::lifecycle_event::Detail::Started(()))
        }
        api::LifecycleEventType::Idle => Some(api::agent_event::lifecycle_event::Detail::Idle(())),
        api::LifecycleEventType::Restarted => {
            Some(api::agent_event::lifecycle_event::Detail::Restarted(()))
        }
        api::LifecycleEventType::InProgress => {
            Some(api::agent_event::lifecycle_event::Detail::InProgress(()))
        }
        api::LifecycleEventType::Succeeded => {
            Some(api::agent_event::lifecycle_event::Detail::Succeeded(()))
        }
        api::LifecycleEventType::Failed => Some(api::agent_event::lifecycle_event::Detail::Failed(
            api::agent_event::lifecycle_event::Failed {
                reason: String::new(),
                error_message: String::new(),
            },
        )),
        api::LifecycleEventType::Cancelled => {
            Some(api::agent_event::lifecycle_event::Detail::Cancelled(()))
        }
        api::LifecycleEventType::Blocked => {
            Some(api::agent_event::lifecycle_event::Detail::Blocked(
                api::agent_event::lifecycle_event::Blocked {
                    blocked_action: "run command".to_string(),
                },
            ))
        }
        api::LifecycleEventType::Errored => Some(
            api::agent_event::lifecycle_event::Detail::Errored(Default::default()),
        ),
        api::LifecycleEventType::Unspecified => None,
    };
    PendingEvent {
        event_id: event_id.to_string(),
        source_agent_id: sender_agent_id.to_string(),
        attempt_count,
        detail: PendingEventDetail::Lifecycle {
            event: api::AgentEvent {
                event_id: event_id.to_string(),
                occurred_at: None,
                event: Some(api::agent_event::Event::LifecycleEvent(
                    api::agent_event::LifecycleEvent {
                        sender_agent_id: sender_agent_id.to_string(),
                        detail,
                    },
                )),
            },
        },
    }
}

fn message_pending_event(event_id: &str) -> PendingEvent {
    PendingEvent {
        event_id: event_id.to_string(),
        source_agent_id: "sender".to_string(),
        attempt_count: 0,
        detail: PendingEventDetail::Message {
            sequence: 0,
            message_id: "message-1".to_string(),
            addresses: vec!["target".to_string()],
            subject: "subject".to_string(),
            message_body: "body".to_string(),
            occurred_at: "2026-01-01T00:00:00Z".to_string(),
        },
    }
}

#[test]
fn test_is_subscribed_defaults_to_all_when_subscription_omitted() {
    assert!(is_subscribed(None, LifecycleEventType::Started));
    assert!(is_subscribed(None, LifecycleEventType::Idle));
    assert!(is_subscribed(None, LifecycleEventType::Restarted));
    assert!(is_subscribed(None, LifecycleEventType::Errored));
    assert!(is_subscribed(None, LifecycleEventType::Cancelled));
    assert!(is_subscribed(None, LifecycleEventType::Blocked));
}

#[test]
fn test_is_subscribed_filters_unsubscribed_event_types() {
    let subscription = [LifecycleEventType::Started, LifecycleEventType::Idle];
    assert!(is_subscribed(
        Some(&subscription),
        LifecycleEventType::Started
    ));
    assert!(!is_subscribed(
        Some(&subscription),
        LifecycleEventType::Errored
    ));
}

#[test]
fn test_is_subscribed_with_explicit_empty_subscription_disables_all_events() {
    assert!(!is_subscribed(Some(&[]), LifecycleEventType::Started));
    assert!(!is_subscribed(Some(&[]), LifecycleEventType::Idle));
    assert!(!is_subscribed(Some(&[]), LifecycleEventType::Restarted));
    assert!(!is_subscribed(Some(&[]), LifecycleEventType::Errored));
    assert!(!is_subscribed(Some(&[]), LifecycleEventType::Cancelled));
    assert!(!is_subscribed(Some(&[]), LifecycleEventType::Blocked));
}

#[test]
fn test_coalesce_lifecycle_events_removes_supersedable_events_for_same_child() {
    let mut queue = vec![
        lifecycle_pending_event(
            "succeeded-a",
            "child-a",
            api::LifecycleEventType::Succeeded,
            0,
        ),
        lifecycle_pending_event(
            "in-progress-a",
            "child-a",
            api::LifecycleEventType::InProgress,
            0,
        ),
        lifecycle_pending_event("errored-a", "child-a", api::LifecycleEventType::Errored, 0),
        lifecycle_pending_event(
            "succeeded-b",
            "child-b",
            api::LifecycleEventType::Succeeded,
            0,
        ),
    ];

    let new_event = lifecycle_pending_event(
        "in-progress-a-new",
        "child-a",
        api::LifecycleEventType::InProgress,
        0,
    );

    let removed = coalesce_lifecycle_events(&mut queue, &new_event);
    // succeeded/in_progress transitions for the same sender are supersedable and should
    // be removed when a newer supersedable event arrives.

    assert_eq!(
        removed,
        vec!["succeeded-a".to_string(), "in-progress-a".to_string()]
    );
    assert_eq!(queue.len(), 2);
    assert_eq!(queue[0].event_id, "errored-a");
    assert_eq!(queue[1].event_id, "succeeded-b");
}

#[test]
fn test_coalesce_lifecycle_events_does_not_coalesce_for_non_supersedable_new_event() {
    let mut queue = vec![lifecycle_pending_event(
        "succeeded-a",
        "child-a",
        api::LifecycleEventType::Succeeded,
        0,
    )];
    let new_event =
        lifecycle_pending_event("errored-a", "child-a", api::LifecycleEventType::Errored, 0);

    let removed = coalesce_lifecycle_events(&mut queue, &new_event);

    assert!(removed.is_empty());
    assert_eq!(queue.len(), 1);
    assert_eq!(queue[0].event_id, "succeeded-a");
}

#[test]
fn test_enforce_lifecycle_queue_cap_drops_oldest_coalescable_events() {
    let mut queue = vec![
        message_pending_event("message"),
        lifecycle_pending_event(
            "cancelled",
            "child-a",
            api::LifecycleEventType::Cancelled,
            0,
        ),
        lifecycle_pending_event(
            "succeeded",
            "child-a",
            api::LifecycleEventType::Succeeded,
            0,
        ),
        lifecycle_pending_event(
            "in-progress",
            "child-b",
            api::LifecycleEventType::InProgress,
            0,
        ),
        lifecycle_pending_event("errored", "child-c", api::LifecycleEventType::Errored, 0),
    ];

    let dropped = enforce_lifecycle_queue_cap(&mut queue, 2);

    assert_eq!(
        dropped,
        vec!["succeeded".to_string(), "in-progress".to_string()]
    );
    assert_eq!(count_pending_lifecycle_events(&queue), 2);
    assert_eq!(queue.len(), 3);
    assert_eq!(queue[0].event_id, "message");
    assert_eq!(queue[1].event_id, "cancelled");
    assert_eq!(queue[2].event_id, "errored");
}

#[test]
fn test_enforce_lifecycle_queue_cap_keeps_critical_events_even_when_over_limit() {
    let mut queue = vec![
        lifecycle_pending_event(
            "cancelled-1",
            "child-a",
            api::LifecycleEventType::Cancelled,
            0,
        ),
        lifecycle_pending_event("errored-1", "child-a", api::LifecycleEventType::Errored, 0),
        lifecycle_pending_event("blocked-1", "child-b", api::LifecycleEventType::Blocked, 0),
    ];

    let dropped = enforce_lifecycle_queue_cap(&mut queue, 1);

    assert!(dropped.is_empty());
    assert_eq!(count_pending_lifecycle_events(&queue), 3);
}

#[test]
fn test_increment_attempt_and_partition_by_retry_limit() {
    let attempted = vec![
        lifecycle_pending_event("retryable", "child-a", api::LifecycleEventType::Started, 0),
        lifecycle_pending_event(
            "exhausted-at-limit",
            "child-b",
            api::LifecycleEventType::Idle,
            2,
        ),
        lifecycle_pending_event(
            "already-exhausted",
            "child-c",
            api::LifecycleEventType::Errored,
            3,
        ),
    ];

    let (retryable, exhausted) = increment_attempt_and_partition_by_retry_limit(attempted, 3);

    assert_eq!(retryable.len(), 1);
    assert_eq!(retryable[0].event_id, "retryable");
    assert_eq!(retryable[0].attempt_count, 1);

    assert_eq!(exhausted.len(), 2);
    assert_eq!(exhausted[0].event_id, "exhausted-at-limit");
    assert_eq!(exhausted[0].attempt_count, 3);
    assert_eq!(exhausted[1].event_id, "already-exhausted");
    assert_eq!(exhausted[1].attempt_count, 4);
}

#[test]
fn test_did_event_round_trip_through_server_matches_message_event_by_message_id() {
    let pending = PendingEvent {
        event_id: "event-1".to_string(),
        source_agent_id: "sender".to_string(),
        attempt_count: 0,
        detail: PendingEventDetail::Message {
            sequence: 0,
            message_id: "message-1".to_string(),
            addresses: vec!["target".to_string()],
            subject: "subject".to_string(),
            message_body: "body".to_string(),
            occurred_at: "2026-01-01T00:00:00Z".to_string(),
        },
    };

    let echoed_message_ids = HashSet::from(["message-1"]);
    let echoed_lifecycle_event_ids = HashSet::new();
    assert!(did_event_round_trip_through_server(
        &pending,
        &echoed_message_ids,
        &echoed_lifecycle_event_ids
    ));
}

#[test]
fn test_did_event_round_trip_through_server_matches_lifecycle_event_by_event_id() {
    let pending = lifecycle_pending_event(
        "lifecycle-event-1",
        "child-a",
        api::LifecycleEventType::Idle,
        0,
    );

    let echoed_message_ids = HashSet::new();
    let echoed_lifecycle_event_ids = HashSet::from(["lifecycle-event-1"]);
    assert!(did_event_round_trip_through_server(
        &pending,
        &echoed_message_ids,
        &echoed_lifecycle_event_ids
    ));
}

#[test]
fn test_did_event_round_trip_through_server_does_not_match_unrelated_echo() {
    let pending = message_pending_event("event-1");
    let echoed_message_ids = HashSet::from(["different-message-id"]);
    let echoed_lifecycle_event_ids = HashSet::from(["different-event-id"]);
    assert!(!did_event_round_trip_through_server(
        &pending,
        &echoed_message_ids,
        &echoed_lifecycle_event_ids
    ));
}

#[test]
fn test_lifecycle_event_type_from_proto_includes_cancelled_and_blocked() {
    let cancelled = lifecycle_pending_event(
        "cancelled-1",
        "child-a",
        api::LifecycleEventType::Cancelled,
        0,
    );
    let blocked =
        lifecycle_pending_event("blocked-1", "child-a", api::LifecycleEventType::Blocked, 0);

    let PendingEventDetail::Lifecycle {
        event: cancelled_event,
    } = &cancelled.detail
    else {
        panic!("expected lifecycle event");
    };
    let PendingEventDetail::Lifecycle {
        event: blocked_event,
    } = &blocked.detail
    else {
        panic!("expected lifecycle event");
    };

    let Some(api::agent_event::Event::LifecycleEvent(cancelled_lifecycle)) = &cancelled_event.event
    else {
        panic!("expected lifecycle event payload");
    };
    let Some(api::agent_event::Event::LifecycleEvent(blocked_lifecycle)) = &blocked_event.event
    else {
        panic!("expected lifecycle event payload");
    };

    assert_eq!(
        lifecycle_event_type_from_proto(cancelled_lifecycle),
        api::LifecycleEventType::Cancelled
    );
    assert_eq!(
        lifecycle_event_type_from_proto(blocked_lifecycle),
        api::LifecycleEventType::Blocked
    );
}

#[test]
fn test_has_pending_events_tracks_any_event_kind() {
    let conversation_id = crate::ai::agent::conversation::AIConversationId::new();
    let mut service = OrchestrationEventService::new_without_subscriptions();
    assert!(!service.has_pending_events(conversation_id));
    service.pending_events.insert(
        conversation_id,
        vec![
            lifecycle_pending_event(
                "lifecycle-1",
                "child-a",
                api::LifecycleEventType::InProgress,
                0,
            ),
            message_pending_event("message-event-1"),
            lifecycle_pending_event(
                "lifecycle-2",
                "child-b",
                api::LifecycleEventType::Succeeded,
                0,
            ),
            message_pending_event("message-event-2"),
        ],
    );
    assert!(service.has_pending_events(conversation_id));
    service.pending_events.remove(&conversation_id);
    assert!(!service.has_pending_events(conversation_id));
}

#[test]
fn test_emit_child_killed_enqueues_cancelled_event_for_parent() {
    App::test((), |mut app| async move {
        let _orchestration_v2 = FeatureFlag::OrchestrationV2.override_enabled(true);
        let terminal_view_id = EntityId::new();
        let parent_run_id = uuid::Uuid::new_v4().to_string();
        let child_run_id = uuid::Uuid::new_v4().to_string();
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let service = app.add_model(|_| OrchestrationEventService::new_without_subscriptions());

        let (parent_conversation_id, child_conversation_id) =
            history_model.update(&mut app, |history_model, ctx| {
                let parent_conversation_id = history_model.start_new_conversation(
                    terminal_view_id,
                    false,
                    false,
                    false,
                    ctx,
                );
                history_model.assign_run_id_for_conversation(
                    parent_conversation_id,
                    parent_run_id,
                    None,
                    terminal_view_id,
                    ctx,
                );
                let child_conversation_id = history_model.start_new_child_conversation(
                    terminal_view_id,
                    "child".to_string(),
                    parent_conversation_id,
                    None,
                    ctx,
                );
                history_model.assign_run_id_for_conversation(
                    child_conversation_id,
                    child_run_id.clone(),
                    None,
                    terminal_view_id,
                    ctx,
                );
                (parent_conversation_id, child_conversation_id)
            });

        service.update(&mut app, |service, ctx| {
            assert!(matches!(
                service.emit_child_killed(child_conversation_id, ctx),
                SendEventResult::LifecycleSent
            ));

            let (inputs, _) = service
                .drain_events_for_request(parent_conversation_id, ctx)
                .expect("parent should have a pending lifecycle event");
            assert_eq!(inputs.len(), 1);
            let AIAgentInput::EventsFromAgents { events } = &inputs[0] else {
                panic!("expected lifecycle events input");
            };
            assert_eq!(events.len(), 1);

            let Some(api::agent_event::Event::LifecycleEvent(lifecycle_event)) = &events[0].event
            else {
                panic!("expected lifecycle event");
            };
            assert_eq!(lifecycle_event.sender_agent_id, child_run_id);
            assert_eq!(
                lifecycle_event_type_from_proto(lifecycle_event),
                api::LifecycleEventType::Cancelled
            );
        });
    });
}

#[test]
fn test_emit_child_killed_drops_when_no_parent() {
    App::test((), |mut app| async move {
        let _orchestration_v2 = FeatureFlag::OrchestrationV2.override_enabled(true);
        let terminal_view_id = EntityId::new();
        let child_run_id = uuid::Uuid::new_v4().to_string();
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let service = app.add_model(|_| OrchestrationEventService::new_without_subscriptions());

        // Create a standalone conversation (no parent) with a run_id.
        let orphan_conversation_id = history_model.update(&mut app, |history_model, ctx| {
            let id =
                history_model.start_new_conversation(terminal_view_id, false, false, false, ctx);
            history_model.assign_run_id_for_conversation(
                id,
                child_run_id,
                None,
                terminal_view_id,
                ctx,
            );
            history_model.update_conversation_status(
                terminal_view_id,
                id,
                ConversationStatus::InProgress,
                ctx,
            );
            id
        });

        service.update(&mut app, |service, ctx| {
            assert!(matches!(
                service.emit_child_killed(orphan_conversation_id, ctx),
                SendEventResult::LifecycleDropped
            ));
        });
    });
}

#[test]
fn test_emit_child_killed_drops_when_child_already_terminal() {
    App::test((), |mut app| async move {
        let _orchestration_v2 = FeatureFlag::OrchestrationV2.override_enabled(true);
        let terminal_view_id = EntityId::new();
        let parent_run_id = uuid::Uuid::new_v4().to_string();
        let child_run_id = uuid::Uuid::new_v4().to_string();
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let service = app.add_model(|_| OrchestrationEventService::new_without_subscriptions());

        let (parent_conversation_id, child_conversation_id) =
            history_model.update(&mut app, |history_model, ctx| {
                let parent_conversation_id = history_model.start_new_conversation(
                    terminal_view_id,
                    false,
                    false,
                    false,
                    ctx,
                );
                history_model.assign_run_id_for_conversation(
                    parent_conversation_id,
                    parent_run_id,
                    None,
                    terminal_view_id,
                    ctx,
                );
                let child_conversation_id = history_model.start_new_child_conversation(
                    terminal_view_id,
                    "child".to_string(),
                    parent_conversation_id,
                    None,
                    ctx,
                );
                history_model.assign_run_id_for_conversation(
                    child_conversation_id,
                    child_run_id,
                    None,
                    terminal_view_id,
                    ctx,
                );
                history_model.update_conversation_status(
                    terminal_view_id,
                    child_conversation_id,
                    ConversationStatus::Success,
                    ctx,
                );
                (parent_conversation_id, child_conversation_id)
            });

        service.update(&mut app, |service, ctx| {
            assert!(matches!(
                service.emit_child_killed(child_conversation_id, ctx),
                SendEventResult::LifecycleDropped
            ));
            assert!(!service.has_pending_events(parent_conversation_id));
        });
    });
}

#[test]
fn test_restored_v1_child_reregisters_lifecycle_subscription() {
    App::test((), |mut app| async move {
        let _orchestration_v2 = FeatureFlag::OrchestrationV2.override_enabled(false);
        let terminal_view_id = EntityId::new();
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let service = app.add_model(|_| OrchestrationEventService::new_without_subscriptions());

        let (_parent_conversation_id, child_conversation_id) =
            history_model.update(&mut app, |history_model, ctx| {
                let parent_conversation_id = history_model.start_new_conversation(
                    terminal_view_id,
                    false,
                    false,
                    false,
                    ctx,
                );
                history_model.set_server_conversation_token_for_conversation(
                    parent_conversation_id,
                    "parent-token".to_string(),
                );
                let child_conversation_id = history_model.start_new_child_conversation(
                    terminal_view_id,
                    "child".to_string(),
                    parent_conversation_id,
                    None,
                    ctx,
                );
                history_model.set_server_conversation_token_for_conversation(
                    child_conversation_id,
                    "child-token".to_string(),
                );
                (parent_conversation_id, child_conversation_id)
            });

        service.update(&mut app, |service, ctx| {
            service.handle_history_event(
                &BlocklistAIHistoryEvent::RestoredConversations {
                    terminal_view_id,
                    conversation_ids: vec![child_conversation_id],
                },
                ctx,
            );

            let routes = service
                .lifecycle_subscription_routes
                .get(&child_conversation_id)
                .expect("restored V1 child should have a lifecycle subscription");
            assert_eq!(routes.len(), 1);
            assert_eq!(routes[0].target_agent_id, "parent-token");
            assert_eq!(routes[0].subscribed_event_types, None);
        });
    });
}
