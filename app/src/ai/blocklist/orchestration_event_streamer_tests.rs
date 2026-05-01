use super::*;
use crate::ai::agent_events::{
    agent_event_backoff, agent_event_failures_exceeded_threshold,
    DEFAULT_AGENT_EVENT_RECONNECT_BACKOFF_STEPS,
};

#[test]
fn sse_backoff_escalates_then_caps() {
    assert_eq!(
        agent_event_backoff(1, DEFAULT_AGENT_EVENT_RECONNECT_BACKOFF_STEPS),
        Duration::from_secs(1)
    );
    assert_eq!(
        agent_event_backoff(2, DEFAULT_AGENT_EVENT_RECONNECT_BACKOFF_STEPS),
        Duration::from_secs(2)
    );
    assert_eq!(
        agent_event_backoff(3, DEFAULT_AGENT_EVENT_RECONNECT_BACKOFF_STEPS),
        Duration::from_secs(5)
    );
    assert_eq!(
        agent_event_backoff(4, DEFAULT_AGENT_EVENT_RECONNECT_BACKOFF_STEPS),
        Duration::from_secs(10)
    );
    // Caps at 10s for any higher failure count.
    assert_eq!(
        agent_event_backoff(5, DEFAULT_AGENT_EVENT_RECONNECT_BACKOFF_STEPS),
        Duration::from_secs(10)
    );
    assert_eq!(
        agent_event_backoff(100, DEFAULT_AGENT_EVENT_RECONNECT_BACKOFF_STEPS),
        Duration::from_secs(10)
    );
}

#[test]
fn sse_backoff_zero_failures_uses_first_step() {
    // Defensive: 0 failures should still return a valid backoff.
    assert_eq!(
        agent_event_backoff(0, DEFAULT_AGENT_EVENT_RECONNECT_BACKOFF_STEPS),
        Duration::from_secs(1)
    );
}

#[test]
fn threshold_not_exceeded_below_limit() {
    assert!(!agent_event_failures_exceeded_threshold(0, 5));
    assert!(!agent_event_failures_exceeded_threshold(1, 5));
    assert!(!agent_event_failures_exceeded_threshold(4, 5));
}

#[test]
fn threshold_exceeded_at_and_above_limit() {
    assert!(agent_event_failures_exceeded_threshold(5, 5));
    assert!(agent_event_failures_exceeded_threshold(6, 5));
    assert!(agent_event_failures_exceeded_threshold(100, 5));
}

fn make_run_event(event_type: &str, run_id: &str, ref_id: Option<&str>) -> AgentRunEvent {
    AgentRunEvent {
        event_type: event_type.to_string(),
        run_id: run_id.to_string(),
        ref_id: ref_id.map(|s| s.to_string()),
        execution_id: None,
        occurred_at: "2026-01-01T00:00:00Z".to_string(),
        sequence: 1,
    }
}

#[test]
fn convert_lifecycle_events_includes_run_blocked() {
    let events = vec![make_run_event("run_blocked", "child-run", None)];
    let result = convert_lifecycle_events(&events, "self-run");
    assert_eq!(result.len(), 1);
    let event = &result[0];
    let Some(api::agent_event::Event::LifecycleEvent(lifecycle)) = &event.event else {
        panic!("expected lifecycle event");
    };
    let Some(api::agent_event::lifecycle_event::Detail::Blocked(blocked)) = &lifecycle.detail
    else {
        panic!("expected blocked detail");
    };
    assert!(blocked.blocked_action.is_empty());
}

#[test]
fn convert_lifecycle_events_filters_self_run_blocked() {
    let events = vec![make_run_event("run_blocked", "self-run", None)];
    let result = convert_lifecycle_events(&events, "self-run");
    assert!(result.is_empty());
}

#[test]
fn convert_lifecycle_events_maps_run_restarted() {
    let events = vec![make_run_event("run_restarted", "child-run", None)];
    let result = convert_lifecycle_events(&events, "self-run");
    assert_eq!(result.len(), 1);
    let event = &result[0];
    let Some(api::agent_event::Event::LifecycleEvent(lifecycle)) = &event.event else {
        panic!("expected lifecycle event");
    };
    assert!(matches!(
        lifecycle.detail,
        Some(api::agent_event::lifecycle_event::Detail::InProgress(..))
    ));
}

#[test]
fn ai_conversation_new_restored_preserves_last_event_sequence() {
    // Guards against regressions that drop the field when wiring the restore
    // path: a conversation restored with `last_event_sequence: Some(N)`
    // should expose it via `conversation.last_event_sequence()`.
    use crate::ai::agent::conversation::{AIConversation, AIConversationId};
    use crate::persistence::model::AgentConversationData;

    let task = api::Task {
        id: "root".to_string(),
        messages: vec![api::Message {
            id: "m1".to_string(),
            task_id: "root".to_string(),
            server_message_data: String::new(),
            citations: vec![],
            message: Some(api::message::Message::AgentOutput(
                api::message::AgentOutput {
                    text: "hi".to_string(),
                },
            )),
            request_id: String::new(),
            timestamp: None,
        }],
        dependencies: None,
        description: String::new(),
        summary: String::new(),
        server_data: String::new(),
    };
    let data = AgentConversationData {
        server_conversation_token: None,
        conversation_usage_metadata: None,
        reverted_action_ids: None,
        forked_from_server_conversation_token: None,
        artifacts_json: None,
        parent_agent_id: None,
        agent_name: None,
        parent_conversation_id: None,
        run_id: None,
        autoexecute_override: None,
        last_event_sequence: Some(42),
    };
    let conversation =
        AIConversation::new_restored(AIConversationId::new(), vec![task], Some(data))
            .expect("should restore");
    assert_eq!(conversation.last_event_sequence(), Some(42));
}

// ---- Helpers for App-based poller tests ----

fn make_ambient_task_with_children(
    children: Vec<String>,
) -> crate::ai::ambient_agents::AmbientAgentTask {
    let mut task = make_ambient_task_with_event_seq(None);
    task.children = children;
    task
}

fn make_ambient_task_with_event_seq(
    last_event_sequence: Option<i64>,
) -> crate::ai::ambient_agents::AmbientAgentTask {
    use chrono::Utc;
    crate::ai::ambient_agents::AmbientAgentTask {
        task_id: "550e8400-e29b-41d4-a716-446655440000".parse().unwrap(),
        parent_run_id: None,
        title: "test".to_string(),
        state: crate::ai::ambient_agents::AmbientAgentTaskState::Succeeded,
        prompt: "prompt".to_string(),
        created_at: Utc::now(),
        started_at: Some(Utc::now()),
        updated_at: Utc::now(),
        status_message: None,
        source: None,
        session_id: None,
        session_link: None,
        creator: None,
        conversation_id: None,
        request_usage: None,
        agent_config_snapshot: None,
        artifacts: vec![],
        is_sandbox_running: false,
        last_event_sequence,
        children: vec![],
    }
}

#[test]
fn finish_restore_fetch_uses_server_cursor_when_sqlite_is_absent() {
    use crate::ai::agent::conversation::AIConversation;
    use crate::server::server_api::ai::MockAIClient;
    use crate::server::server_api::ServerApiProvider;
    use std::sync::Arc;
    use warpui::App;

    App::test((), |mut app| async move {
        let _v2_guard = FeatureFlag::OrchestrationV2.override_enabled(true);

        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        // Restore a conversation with no SQLite cursor (`last_event_sequence:
        // None`). After the server fetch completes with `Some(42)` we expect
        // the in-memory cursor to be 42 (max(0, 42)).
        let conversation = AIConversation::new(false);
        let conversation_id = conversation.id();
        let terminal_view_id = warpui::EntityId::new();
        history_model.update(&mut app, |model, ctx| {
            model.restore_conversations(terminal_view_id, vec![conversation], ctx);
        });

        let mock = MockAIClient::new();
        let ai_client: Arc<dyn AIClient> = Arc::new(mock);
        let server_api = ServerApiProvider::new_for_test().get();

        let poller = app.add_singleton_model(|ctx| {
            OrchestrationEventStreamer::new_with_clients_for_test(ai_client, server_api, ctx)
        });

        // Seed a stream entry as on_restored_conversations would before
        // spawning the async fetch. Without this the guard that detects
        // mid-flight conversation deletion would fire and return early.
        poller.update(&mut app, |me, _| {
            me.streams.entry(conversation_id).or_default();
        });

        let task_id: crate::ai::ambient_agents::AmbientAgentTaskId =
            "550e8400-e29b-41d4-a716-446655440000".parse().unwrap();
        poller.update(&mut app, |me, ctx| {
            me.finish_restore_fetch(
                conversation_id,
                task_id,
                /* sqlite_cursor */ 0,
                Ok(make_ambient_task_with_event_seq(Some(42))),
                ctx,
            );
        });

        poller.read(&app, |me, _| {
            assert_eq!(
                me.streams.get(&conversation_id).map(|s| s.event_cursor),
                Some(42)
            );
        });
    });
}

#[test]
fn handle_event_batch_persists_max_seq_to_history_model() {
    use crate::ai::agent::conversation::{AIConversation, AIConversationId};
    use crate::persistence::ModelEvent;
    use crate::server::server_api::ai::MockAIClient;
    use crate::server::server_api::ServerApiProvider;
    use crate::test_util::settings::initialize_settings_for_tests;
    use crate::{GlobalResourceHandles, GlobalResourceHandlesProvider};
    use std::sync::Arc;
    use warpui::App;

    App::test((), |mut app| async move {
        let _v2_guard = FeatureFlag::OrchestrationV2.override_enabled(true);

        // `update_event_sequence` calls `write_updated_conversation_state`,
        // which reads `GeneralSettings`, `AppExecutionMode`, and the global
        // resource sender. Wire all of these up so the SQLite write can run.
        initialize_settings_for_tests(&mut app);
        let (sender, receiver) = std::sync::mpsc::sync_channel::<ModelEvent>(4);
        let mut global_resource_handles = GlobalResourceHandles::mock(&mut app);
        global_resource_handles.model_event_sender = Some(sender);
        app.add_singleton_model(|_| GlobalResourceHandlesProvider::new(global_resource_handles));

        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        let mut conversation = AIConversation::new(false);
        conversation.set_run_id("550e8400-e29b-41d4-a716-446655440200".to_string());
        let conversation_id: AIConversationId = conversation.id();
        let terminal_view_id = warpui::EntityId::new();
        history_model.update(&mut app, |model, ctx| {
            model.restore_conversations(terminal_view_id, vec![conversation], ctx);
        });

        let mut mock = MockAIClient::new();
        // The fire-and-forget server PATCH should be issued; permissive Ok.
        mock.expect_update_event_sequence_on_server()
            .returning(|_, _| Ok(()));
        let ai_client: Arc<dyn AIClient> = Arc::new(mock);
        let server_api = ServerApiProvider::new_for_test().get();

        let poller = app.add_singleton_model(|ctx| {
            OrchestrationEventStreamer::new_with_clients_for_test(ai_client, server_api, ctx)
        });

        // Build a poll batch with max sequence = 42. Use an unrecognized
        // event_type so `convert_lifecycle_events` returns empty and the
        // function early-exits before touching `OrchestrationEventService`
        // (which we did not register in this test App).
        let events = vec![
            AgentRunEvent {
                event_type: "unrecognized_event_type".to_string(),
                run_id: "some-other-run".to_string(),
                ref_id: None,
                execution_id: None,
                occurred_at: "2026-01-01T00:00:00Z".to_string(),
                sequence: 17,
            },
            AgentRunEvent {
                event_type: "unrecognized_event_type".to_string(),
                run_id: "some-other-run".to_string(),
                ref_id: None,
                execution_id: None,
                occurred_at: "2026-01-01T00:00:00Z".to_string(),
                sequence: 42,
            },
        ];

        poller.update(&mut app, |me, ctx| {
            me.handle_event_batch(
                conversation_id,
                /* self_run_id */ "some-other-run",
                /* previous_cursor */ 0,
                events,
                /* messages */ vec![],
                ctx,
            );
        });

        history_model.read(&app, |model, _| {
            let last_seq = model
                .conversation(&conversation_id)
                .and_then(|c| c.last_event_sequence());
            assert_eq!(
                last_seq,
                Some(42),
                "BlocklistAIHistoryModel.update_event_sequence must be called with max_seq"
            );
        });

        // Drain at least one persistence event to confirm the SQLite write
        // path was triggered (sanity check for the side effect, not the
        // primary assertion).
        let _ = receiver.recv_timeout(std::time::Duration::from_secs(1));
    });
}

#[test]
fn finish_restore_fetch_no_ops_when_conversation_deleted_mid_flight() {
    // If the conversation is removed while the async fetch is in-flight, the
    // RemoveConversation handler removes the streams entry. finish_restore_fetch
    // uses the missing entry as a sentinel and must not re-populate
    // streamer state for the deleted conversation.
    use crate::ai::agent::conversation::AIConversation;
    use crate::server::server_api::ai::MockAIClient;
    use crate::server::server_api::ServerApiProvider;
    use std::sync::Arc;
    use warpui::App;

    App::test((), |mut app| async move {
        let _v2_guard = FeatureFlag::OrchestrationV2.override_enabled(true);

        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        let mut conversation = AIConversation::new(false);
        conversation.set_run_id("550e8400-e29b-41d4-a716-446655440300".to_string());
        let conversation_id = conversation.id();
        let terminal_view_id = warpui::EntityId::new();
        history_model.update(&mut app, |model, ctx| {
            model.restore_conversations(terminal_view_id, vec![conversation], ctx);
        });

        let mock = MockAIClient::new();
        let ai_client: Arc<dyn AIClient> = Arc::new(mock);
        let server_api = ServerApiProvider::new_for_test().get();

        let poller = app.add_singleton_model(|ctx| {
            OrchestrationEventStreamer::new_with_clients_for_test(ai_client, server_api, ctx)
        });

        // Seed a stream entry as on_restored_conversations would.
        poller.update(&mut app, |me, _| {
            me.streams.entry(conversation_id).or_default();
        });

        // Simulate the RemoveConversation handler firing while the fetch is
        // in-flight: it drops the conversation's streamer state.
        poller.update(&mut app, |me, _| {
            me.streams.remove(&conversation_id);
        });

        // The in-flight fetch now completes — with children.
        let task_id: crate::ai::ambient_agents::AmbientAgentTaskId =
            "550e8400-e29b-41d4-a716-446655440000".parse().unwrap();
        poller.update(&mut app, |me, ctx| {
            me.finish_restore_fetch(
                conversation_id,
                task_id,
                /* sqlite_cursor */ 0,
                Ok(make_ambient_task_with_children(vec![
                    "child-run-1".to_string()
                ])),
                ctx,
            );
        });

        poller.read(&app, |me, _| {
            assert!(
                !me.streams.contains_key(&conversation_id),
                "streamer state must not be repopulated for a deleted conversation"
            );
        });
    });
}

#[test]
fn finish_restore_fetch_err_does_not_resurrect_deleted_conversation() {
    // Mirror image of `finish_restore_fetch_no_ops_when_conversation_deleted_mid_flight`
    // but for the Err arm: a transient fetch failure on a conversation that
    // was just removed must not resurrect a streams entry (which would then
    // defeat the deletion sentinel inside the retry timer and cause an
    // indefinite retry loop).
    use crate::ai::agent::conversation::AIConversation;
    use crate::server::server_api::ai::MockAIClient;
    use crate::server::server_api::ServerApiProvider;
    use std::sync::Arc;
    use warpui::App;

    App::test((), |mut app| async move {
        let _v2_guard = FeatureFlag::OrchestrationV2.override_enabled(true);

        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        let mut conversation = AIConversation::new(false);
        conversation.set_run_id("550e8400-e29b-41d4-a716-446655440500".to_string());
        let conversation_id = conversation.id();
        let terminal_view_id = warpui::EntityId::new();
        history_model.update(&mut app, |model, ctx| {
            model.restore_conversations(terminal_view_id, vec![conversation], ctx);
        });

        let mock = MockAIClient::new();
        let ai_client: Arc<dyn AIClient> = Arc::new(mock);
        let server_api = ServerApiProvider::new_for_test().get();

        let poller = app.add_singleton_model(|ctx| {
            OrchestrationEventStreamer::new_with_clients_for_test(ai_client, server_api, ctx)
        });

        // Seed the entry as on_restored_conversations would, then drop it
        // (simulates the RemoveConversation handler firing while the fetch
        // is in-flight).
        poller.update(&mut app, |me, _| {
            me.streams.entry(conversation_id).or_default();
            me.streams.remove(&conversation_id);
        });

        // The in-flight fetch now completes with an error.
        let task_id: crate::ai::ambient_agents::AmbientAgentTaskId =
            "550e8400-e29b-41d4-a716-446655440000".parse().unwrap();
        poller.update(&mut app, |me, ctx| {
            me.finish_restore_fetch(
                conversation_id,
                task_id,
                /* sqlite_cursor */ 0,
                Err(anyhow::anyhow!("transient network failure")),
                ctx,
            );
        });

        poller.read(&app, |me, _| {
            assert!(
                !me.streams.contains_key(&conversation_id),
                "Err retry must not resurrect a streams entry for a deleted conversation"
            );
        });
    });
}

#[test]
fn on_conversation_removed_prunes_stale_child_run_id_from_parent() {
    // Regression for the case where a child conversation is deleted: the
    // parent's `watched_run_ids` set must be pruned of that child's run_id
    // so subsequent SSE reconnects do not include the dead run_id in the
    // filter. Previously the streamer looked up the run_id from the history
    // model after the removal, which always returned `None` because the
    // history model emits `RemoveConversation` after dropping the record.
    use crate::ai::agent::conversation::AIConversation;
    use crate::server::server_api::ai::MockAIClient;
    use crate::server::server_api::ServerApiProvider;
    use std::sync::Arc;
    use warpui::App;

    App::test((), |mut app| async move {
        let _v2_guard = FeatureFlag::OrchestrationV2.override_enabled(true);

        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        let parent_id = AIConversation::new(false).id();
        let mut child_conversation = AIConversation::new(false);
        let child_run_id = "550e8400-e29b-41d4-a716-446655440600".to_string();
        child_conversation.set_run_id(child_run_id.clone());
        let child_id = child_conversation.id();
        let terminal_view_id = warpui::EntityId::new();
        history_model.update(&mut app, |model, ctx| {
            model.restore_conversations(terminal_view_id, vec![child_conversation], ctx);
        });

        let mock = MockAIClient::new();
        let ai_client: Arc<dyn AIClient> = Arc::new(mock);
        let server_api = ServerApiProvider::new_for_test().get();

        let poller = app.add_singleton_model(|ctx| {
            OrchestrationEventStreamer::new_with_clients_for_test(ai_client, server_api, ctx)
        });

        // Seed the parent's watched set with the child's run_id, as
        // `register_watched_run_id` would have done after the child got
        // its server token.
        poller.update(&mut app, |me, _| {
            me.streams
                .entry(parent_id)
                .or_default()
                .watched_run_ids
                .insert(child_run_id.clone());
        });

        // Now invoke the removal handler with the run_id (mirroring the
        // event payload that history_model emits with the captured run_id).
        poller.update(&mut app, |me, ctx| {
            me.on_conversation_removed(child_id, Some(child_run_id.clone()), ctx);
        });

        poller.read(&app, |me, _| {
            assert!(
                me.streams
                    .get(&parent_id)
                    .is_some_and(|s| !s.watched_run_ids.contains(&child_run_id)),
                "parent's watched_run_ids must be pruned of the removed child's run_id"
            );
        });
    });
}

#[test]
fn finish_restore_fetch_reconnects_sse_when_children_added_to_open_connection() {
    // When a status transition races with the restore fetch and opens SSE
    // before children are known, finish_restore_fetch must reconnect SSE
    // with the updated run_id set rather than leaving children unwatched.
    use crate::ai::agent::conversation::{AIConversation, ConversationStatus};
    use crate::server::server_api::ai::MockAIClient;
    use crate::server::server_api::ServerApiProvider;
    use std::sync::Arc;
    use warpui::App;

    App::test((), |mut app| async move {
        let _v2_guard = FeatureFlag::OrchestrationV2.override_enabled(true);

        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        let own_run_id = "550e8400-e29b-41d4-a716-446655440400";
        let mut conversation = AIConversation::new(false);
        conversation.set_run_id(own_run_id.to_string());
        let conversation_id = conversation.id();
        let terminal_view_id = warpui::EntityId::new();
        history_model.update(&mut app, |model, ctx| {
            model.restore_conversations(terminal_view_id, vec![conversation], ctx);
            model.update_conversation_status(
                terminal_view_id,
                conversation_id,
                ConversationStatus::InProgress,
                ctx,
            );
        });

        let mock = MockAIClient::new();
        let ai_client: Arc<dyn AIClient> = Arc::new(mock);
        let server_api = ServerApiProvider::new_for_test().get();

        let poller = app.add_singleton_model(|ctx| {
            OrchestrationEventStreamer::new_with_clients_for_test(ai_client, server_api, ctx)
        });

        // Seed the state on_restored_conversations would have set up, then
        // inject a fake open SSE connection (generation 0) simulating the
        // race: a consumer registered before the restore fetch completed.
        // The dummy `EntityId` stands in for any local consumer (e.g. an
        // open agent view); without it the eligibility predicate would
        // bail and reconnect_sse would tear the connection down instead
        // of opening a new one.
        let (_, rx) = futures::channel::mpsc::unbounded::<SseStreamItem>();
        let consumer_id = warpui::EntityId::new();
        poller.update(&mut app, |me, _| {
            let stream = me.streams.entry(conversation_id).or_default();
            stream.event_cursor = 0;
            stream.watched_run_ids.insert(own_run_id.to_string());
            stream.consumers.insert(consumer_id);
            stream.sse_connection = Some(SseConnectionState {
                event_receiver: rx,
                generation: 0,
            });
            me.next_sse_generation = 1;
        });

        // The restore fetch returns with a child run_id.
        let task_id: crate::ai::ambient_agents::AmbientAgentTaskId =
            "550e8400-e29b-41d4-a716-446655440000".parse().unwrap();
        poller.update(&mut app, |me, ctx| {
            me.finish_restore_fetch(
                conversation_id,
                task_id,
                /* sqlite_cursor */ 0,
                Ok(make_ambient_task_with_children(vec![
                    "child-run-1".to_string()
                ])),
                ctx,
            );
        });

        poller.read(&app, |me, _| {
            assert!(
                me.streams
                    .get(&conversation_id)
                    .is_some_and(|s| s.watched_run_ids.contains("child-run-1")),
                "child run_id must be in watched set"
            );
            // The old generation-0 connection must have been replaced by a
            // new one with a higher generation, proving SSE was reconnected.
            let gen = me
                .streams
                .get(&conversation_id)
                .and_then(|s| s.sse_connection.as_ref())
                .map(|c| c.generation);
            assert!(
                gen.is_some_and(|g| g > 0),
                "SSE must be reconnected (new generation) after children are discovered; got gen={gen:?}"
            );
        });
    });
}
