use std::sync::Arc;

use mockall::predicate::eq;
use warp_core::features::FeatureFlag;
use warpui::App;

use super::*;
use crate::ai::agent::conversation::{AIConversation, ConversationStatus};
use crate::ai::agent_events::{
    agent_event_backoff, agent_event_failures_exceeded_threshold, AgentEventConsumerControlFlow,
    DEFAULT_AGENT_EVENT_RECONNECT_BACKOFF_STEPS,
};
use crate::persistence::ModelEvent;
use crate::server::server_api::ai::MockAIClient;
use crate::server::server_api::ServerApiProvider;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::{GlobalResourceHandles, GlobalResourceHandlesProvider};

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
        orchestration_harness_type: None,
        parent_conversation_id: None,
        is_remote_child: false,
        root_task_is_optimistic: None,
        run_id: None,
        autoexecute_override: None,
        last_event_sequence: Some(42),
        pinned: false,
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
        run_time: Some("PT1S".parse().unwrap()),
        status_message: None,
        source: None,
        session_id: None,
        session_link: None,
        creator: None,
        executor: None,
        conversation_id: None,
        request_usage: None,
        agent_config_snapshot: None,
        artifacts: vec![],
        is_sandbox_running: false,
        last_event_sequence,
        children: vec![],
    }
}

fn make_ambient_task_with_task_id(
    task_id: AmbientAgentTaskId,
    last_event_sequence: Option<i64>,
) -> crate::ai::ambient_agents::AmbientAgentTask {
    let mut task = make_ambient_task_with_event_seq(last_event_sequence);
    task.task_id = task_id;
    task
}

fn make_server_metadata_with_harness(
    harness: AIAgentHarness,
) -> crate::ai::agent::conversation::ServerAIConversationMetadata {
    use chrono::Utc;

    use crate::ai::agent::api::ServerConversationToken;
    use crate::cloud_object::{Revision, ServerMetadata, ServerPermissions};
    use crate::persistence::model::ConversationUsageMetadata;
    use crate::server::ids::ServerId;

    crate::ai::agent::conversation::ServerAIConversationMetadata {
        title: "test".to_string(),
        working_directory: None,
        harness,
        usage: ConversationUsageMetadata {
            was_summarized: false,
            context_window_usage: 0.0,
            credits_spent: 0.0,
            platform_credits_spent: 0.0,
            credits_spent_for_last_block: None,
            token_usage: vec![],
            tool_usage_metadata: Default::default(),
        },
        metadata: ServerMetadata {
            uid: ServerId::default(),
            revision: Revision::now(),
            metadata_last_updated_ts: Utc::now().into(),
            trashed_ts: None,
            folder_id: None,
            is_welcome_object: false,
            creator_uid: None,
            last_editor_uid: None,
            current_editor_uid: None,
        },
        permissions: ServerPermissions::mock_personal(),
        creator: None,
        ambient_agent_task_id: None,
        server_conversation_token: ServerConversationToken::new("server-token".to_string()),
        artifacts: vec![],
    }
}

#[test]
fn dormant_local_claude_child_skips_generic_sse_but_allows_wake_listener() {
    use std::sync::Arc;

    use warpui::App;

    use crate::ai::agent::conversation::{AIConversation, ConversationStatus};
    use crate::server::server_api::ai::MockAIClient;
    use crate::server::server_api::ServerApiProvider;

    App::test((), |mut app| async move {
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        let parent_id = AIConversation::new(false, false).id();
        let mut conversation = AIConversation::new(false, false);
        let run_id = "550e8400-e29b-41d4-a716-446655440610".to_string();
        conversation.set_run_id(run_id.clone());
        conversation.set_parent_conversation_id(parent_id);
        conversation.set_server_metadata(make_server_metadata_with_harness(
            AIAgentHarness::ClaudeCode,
        ));
        let conversation_id = conversation.id();
        let terminal_view_id = warpui::EntityId::new();
        history_model.update(&mut app, |model, ctx| {
            model.restore_conversations(terminal_view_id, vec![conversation], ctx);
            model.update_conversation_status(
                terminal_view_id,
                conversation_id,
                ConversationStatus::Success,
                ctx,
            );
        });

        let mock = MockAIClient::new();
        let ai_client: Arc<dyn AIClient> = Arc::new(mock);
        let server_api = ServerApiProvider::new_for_test().get();

        let streamer = app.add_singleton_model(|ctx| {
            OrchestrationEventStreamer::new_with_clients_for_test(ai_client, server_api, ctx)
        });

        streamer.update(&mut app, |me, _| {
            let stream = me.streams.entry(conversation_id).or_default();
            stream.consumers.insert(warpui::EntityId::new());
            stream.watched_run_ids.insert(run_id);
        });

        streamer.read(&app, |me, ctx| {
            assert!(
                !me.is_eligible(conversation_id, ctx),
                "generic SSE must stay closed for dormant local Claude children"
            );
            assert!(
                me.is_dormant_claude_wake_listener_eligible(conversation_id, ctx),
                "wake-only listener should open for dormant local Claude children"
            );
        });
    });
}

#[test]
fn persist_event_cursor_keeps_the_max_sequence_and_updates_history_model() {
    use std::sync::Arc;

    use warpui::App;

    use crate::ai::agent::conversation::{AIConversation, AIConversationId};
    use crate::persistence::ModelEvent;
    use crate::server::server_api::ai::MockAIClient;
    use crate::server::server_api::ServerApiProvider;
    use crate::test_util::settings::initialize_settings_for_tests;
    use crate::{GlobalResourceHandles, GlobalResourceHandlesProvider};

    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        let (sender, receiver) = std::sync::mpsc::sync_channel::<ModelEvent>(4);
        let mut global_resource_handles = GlobalResourceHandles::mock(&mut app);
        global_resource_handles.model_event_sender = Some(sender);
        app.add_singleton_model(|_| GlobalResourceHandlesProvider::new(global_resource_handles));

        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        let run_id = "550e8400-e29b-41d4-a716-446655440201".to_string();
        let mut conversation = AIConversation::new(false, false);
        conversation.set_run_id(run_id.clone());
        let conversation_id: AIConversationId = conversation.id();
        let terminal_view_id = warpui::EntityId::new();
        history_model.update(&mut app, |model, ctx| {
            model.restore_conversations(terminal_view_id, vec![conversation], ctx);
        });

        let mut mock = MockAIClient::new();
        mock.expect_update_event_sequence_on_server()
            .with(eq(run_id.clone()), eq(42))
            .times(1)
            .returning(|_, _| Ok(()));
        let ai_client: Arc<dyn AIClient> = Arc::new(mock);
        let server_api = ServerApiProvider::new_for_test().get();

        let streamer = app.add_singleton_model(|ctx| {
            OrchestrationEventStreamer::new_with_clients_for_test(ai_client, server_api, ctx)
        });

        streamer.update(&mut app, |me, ctx| {
            me.streams.entry(conversation_id).or_default().event_cursor = 42;
            me.persist_event_cursor(conversation_id, 17, ctx);
        });

        streamer.read(&app, |me, _| {
            assert_eq!(
                me.streams
                    .get(&conversation_id)
                    .map(|stream| stream.event_cursor),
                Some(42)
            );
        });
        history_model.read(&app, |model, _| {
            assert_eq!(
                model
                    .conversation(&conversation_id)
                    .and_then(|conversation| conversation.last_event_sequence()),
                Some(42)
            );
        });

        let _ = receiver.recv_timeout(std::time::Duration::from_secs(1));
    });
}

#[test]
fn wake_ready_does_not_advance_cursor_before_wake_preparation() {
    use std::sync::Arc;

    use warpui::App;

    use crate::ai::agent::conversation::AIConversation;
    use crate::ai::agent_events::AgentMessageEventMetadata;
    use crate::server::server_api::ai::{AIClient, MockAIClient};
    use crate::server::server_api::ServerApiProvider;

    App::test((), |mut app| async move {
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        let mut conversation = AIConversation::new(false, false);
        conversation.set_last_event_sequence(17);
        let conversation_id = conversation.id();
        let terminal_view_id = warpui::EntityId::new();
        history_model.update(&mut app, |model, ctx| {
            model.restore_conversations(terminal_view_id, vec![conversation], ctx);
        });

        let mock = MockAIClient::new();
        let ai_client: Arc<dyn AIClient> = Arc::new(mock);
        let server_api = ServerApiProvider::new_for_test().get();

        let streamer = app.add_singleton_model(|ctx| {
            OrchestrationEventStreamer::new_with_clients_for_test(ai_client, server_api, ctx)
        });

        streamer.update(&mut app, |me, ctx| {
            me.streams.entry(conversation_id).or_default().event_cursor = 17;
            me.finish_dormant_claude_wake_listener(
                conversation_id,
                1,
                Ok(Some(AgentMessageEventMetadata {
                    sequence: 42,
                    message_id: "message-123".to_string(),
                    occurred_at: "2026-01-01T00:00:01Z".to_string(),
                })),
                ctx,
            );
        });

        streamer.read(&app, |me, _| {
            assert_eq!(
                me.streams
                    .get(&conversation_id)
                    .map(|stream| stream.event_cursor),
                Some(17)
            );
        });
        history_model.read(&app, |model, _| {
            assert_eq!(
                model
                    .conversation(&conversation_id)
                    .and_then(|conversation| conversation.last_event_sequence()),
                Some(17)
            );
        });
    });
}

#[test]
fn dormant_local_claude_child_uses_task_harness_when_server_metadata_missing() {
    use std::sync::Arc;

    use warp_cli::agent::Harness;
    use warpui::App;

    use crate::ai::agent::conversation::{AIConversation, ConversationStatus};
    use crate::server::server_api::ai::MockAIClient;
    use crate::server::server_api::ServerApiProvider;

    App::test((), |mut app| async move {
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        let parent_id = AIConversation::new(false, false).id();
        let mut conversation = AIConversation::new(false, false);
        let run_id = "550e8400-e29b-41d4-a716-446655440611".to_string();
        conversation.set_run_id(run_id.clone());
        conversation.set_parent_conversation_id(parent_id);
        let conversation_id = conversation.id();
        let terminal_view_id = warpui::EntityId::new();
        history_model.update(&mut app, |model, ctx| {
            model.restore_conversations(terminal_view_id, vec![conversation], ctx);
            model.update_conversation_status(
                terminal_view_id,
                conversation_id,
                ConversationStatus::Success,
                ctx,
            );
        });

        let mock = MockAIClient::new();
        let ai_client: Arc<dyn AIClient> = Arc::new(mock);
        let server_api = ServerApiProvider::new_for_test().get();

        let streamer = app.add_singleton_model(|ctx| {
            OrchestrationEventStreamer::new_with_clients_for_test(ai_client, server_api, ctx)
        });

        streamer.update(&mut app, |me, _| {
            let stream = me.streams.entry(conversation_id).or_default();
            stream.consumers.insert(warpui::EntityId::new());
            stream.watched_run_ids.insert(run_id);
        });

        streamer.read(&app, |me, ctx| {
            assert!(
                me.is_eligible(conversation_id, ctx),
                "generic SSE should remain eligible before the task harness is known"
            );
            assert!(
                !me.is_dormant_claude_wake_listener_eligible(conversation_id, ctx),
                "wake-only listener should wait until the task harness identifies Claude"
            );
        });

        streamer.update(&mut app, |me, _| {
            me.streams
                .get_mut(&conversation_id)
                .expect("stream exists")
                .harness = Some(Harness::Claude);
        });

        streamer.read(&app, |me, ctx| {
            assert!(
                !me.is_eligible(conversation_id, ctx),
                "generic SSE must close after task metadata identifies a dormant local Claude child"
            );
            assert!(
                me.is_dormant_claude_wake_listener_eligible(conversation_id, ctx),
                "wake-only listener should open based on cached task harness even without server metadata"
            );
        });
    });
}
#[tokio::test]
async fn dormant_claude_wake_consumer_stops_on_first_target_event() {
    let mut consumer = DormantClaudeWakeConsumer::new("target-run".to_string());

    let ignored_event = AgentRunEvent {
        event_type: "new_message".to_string(),
        run_id: "other-run".to_string(),
        ref_id: Some("message-1".to_string()),
        execution_id: None,
        occurred_at: "2026-01-01T00:00:00Z".to_string(),
        sequence: 7,
    };
    assert_eq!(
        consumer.on_event(ignored_event).await.unwrap(),
        AgentEventConsumerControlFlow::Continue
    );
    assert_eq!(consumer.wake_message, None);

    let ignored_same_run_lifecycle = AgentRunEvent {
        event_type: "run_restarted".to_string(),
        run_id: "target-run".to_string(),
        ref_id: None,
        execution_id: None,
        occurred_at: "2026-01-01T00:00:00Z".to_string(),
        sequence: 7,
    };
    assert_eq!(
        consumer.on_event(ignored_same_run_lifecycle).await.unwrap(),
        AgentEventConsumerControlFlow::Continue
    );
    assert_eq!(consumer.wake_message, None);

    // The wake consumer uses the default no-op cursor persistence hook; it
    // should not persist SQLite or server cursors while waiting to wake Claude.
    consumer.persist_cursor(7).await.unwrap();

    let target_event = AgentRunEvent {
        event_type: "new_message".to_string(),
        run_id: "target-run".to_string(),
        ref_id: Some("message-2".to_string()),
        execution_id: None,
        occurred_at: "2026-01-01T00:00:01Z".to_string(),
        sequence: 8,
    };
    assert_eq!(
        consumer.on_event(target_event).await.unwrap(),
        AgentEventConsumerControlFlow::Stop
    );
    let wake_message = consumer.wake_message.expect("wake message");
    assert_eq!(wake_message.sequence, 8);
    assert_eq!(wake_message.message_id, "message-2");
    assert_eq!(wake_message.occurred_at, "2026-01-01T00:00:01Z");
}

#[test]
fn restored_conversations_initialize_v2_streaming_state() {
    use std::sync::Arc;

    use warpui::App;

    use crate::ai::agent::conversation::AIConversation;
    use crate::server::server_api::ai::MockAIClient;
    use crate::server::server_api::ServerApiProvider;

    App::test((), |mut app| async move {
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        let mut conversation = AIConversation::new(false, false);
        conversation.set_run_id("550e8400-e29b-41d4-a716-446655440500".to_string());
        let conversation_id = conversation.id();
        let terminal_view_id = warpui::EntityId::new();
        history_model.update(&mut app, |model, ctx| {
            model.restore_conversations(terminal_view_id, vec![conversation], ctx);
        });

        let mock = MockAIClient::new();
        let ai_client: Arc<dyn AIClient> = Arc::new(mock);
        let server_api = ServerApiProvider::new_for_test().get();

        let streamer = app.add_singleton_model(|ctx| {
            OrchestrationEventStreamer::new_with_clients_for_test(ai_client, server_api, ctx)
        });

        streamer.update(&mut app, |me, ctx| {
            me.on_restored_conversations(vec![conversation_id], ctx);
        });

        streamer.read(&app, |me, _| {
            assert!(
                me.streams.contains_key(&conversation_id),
                "restore should initialize stream state"
            );
        });
    });
}

#[test]
fn build_pending_events_preserves_message_payload() {
    let pending = build_pending_events(
        vec![ReceivedMessageInput {
            message_id: "message-123".to_string(),
            sender_agent_id: "sender-agent".to_string(),
            addresses: vec!["recipient-agent".to_string()],
            subject: "subject".to_string(),
            message_body: "body".to_string(),
        }],
        vec![],
    );

    assert_eq!(pending.len(), 1);
    let detail = &pending[0].detail;
    let PendingEventDetail::Message { message_id, .. } = detail else {
        panic!("expected pending message event");
    };
    assert_eq!(message_id, "message-123");
}

#[tokio::test]
async fn sse_forwarding_consumer_skips_message_hydration_when_disabled() {
    use futures::StreamExt;

    let (tx, mut rx) = futures::channel::mpsc::unbounded();
    let mut ai_client = crate::server::server_api::ai::MockAIClient::new();
    ai_client.expect_read_agent_message().times(0);
    let ai_client: Arc<dyn AIClient> = Arc::new(ai_client);
    let hydrator = MessageHydrator::new(ai_client);
    let mut consumer = SseForwardingConsumer {
        tx,
        self_run_id: "child-run".to_string(),
        hydrator,
        hydrate_new_messages: false,
    };
    let event = make_run_event("new_message", "child-run", Some("message-123"));

    consumer.on_event(event).await.unwrap();

    let item = rx.next().await.expect("expected forwarded event");
    assert_eq!(item.event.ref_id.as_deref(), Some("message-123"));
    assert!(item.fetched_message.is_none());
}
#[test]
fn finish_restore_fetch_uses_server_cursor_when_sqlite_is_absent() {
    use std::sync::Arc;

    use warpui::App;

    use crate::ai::agent::conversation::AIConversation;
    use crate::server::server_api::ai::MockAIClient;
    use crate::server::server_api::ServerApiProvider;

    App::test((), |mut app| async move {
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        // Restore a conversation with no SQLite cursor (`last_event_sequence:
        // None`). After the server fetch completes with `Some(42)` we expect
        // the in-memory cursor to be 42 (max(0, 42)).
        let conversation = AIConversation::new(false, false);
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
    use std::sync::Arc;

    use warpui::App;

    use crate::ai::agent::conversation::{AIConversation, AIConversationId};
    use crate::persistence::ModelEvent;
    use crate::server::server_api::ai::MockAIClient;
    use crate::server::server_api::ServerApiProvider;
    use crate::test_util::settings::initialize_settings_for_tests;
    use crate::{GlobalResourceHandles, GlobalResourceHandlesProvider};

    App::test((), |mut app| async move {
        // `update_event_sequence` calls `write_updated_conversation_state`,
        // which reads `GeneralSettings`, `AppExecutionMode`, and the global
        // resource sender. Wire all of these up so the SQLite write can run.
        initialize_settings_for_tests(&mut app);
        let (sender, receiver) = std::sync::mpsc::sync_channel::<ModelEvent>(4);
        let mut global_resource_handles = GlobalResourceHandles::mock(&mut app);
        global_resource_handles.model_event_sender = Some(sender);
        app.add_singleton_model(|_| GlobalResourceHandlesProvider::new(global_resource_handles));

        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        let mut conversation = AIConversation::new(false, false);
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
fn handle_event_batch_drops_events_for_killed_run_ids_after_persisting_cursor() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        let (sender, _receiver) = std::sync::mpsc::sync_channel::<ModelEvent>(4);
        let mut global_resource_handles = GlobalResourceHandles::mock(&mut app);
        global_resource_handles.model_event_sender = Some(sender);
        app.add_singleton_model(|_| GlobalResourceHandlesProvider::new(global_resource_handles));

        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));
        let event_service = app.add_singleton_model(|_| OrchestrationEventService::default());

        let parent_run_id = "550e8400-e29b-41d4-a716-446655440700".to_string();
        let killed_run_id = "550e8400-e29b-41d4-a716-446655440701".to_string();
        let mut parent_conversation = AIConversation::new(false, false);
        parent_conversation.set_run_id(parent_run_id.clone());
        let parent_conversation_id = parent_conversation.id();
        let terminal_view_id = warpui::EntityId::new();
        history_model.update(&mut app, |model, ctx| {
            model.restore_conversations(terminal_view_id, vec![parent_conversation], ctx);
        });

        let mut mock = MockAIClient::new();
        mock.expect_update_event_sequence_on_server()
            .returning(|_, _| Ok(()));
        let ai_client: Arc<dyn AIClient> = Arc::new(mock);
        let server_api = ServerApiProvider::new_for_test().get();

        let streamer = app.add_singleton_model(|ctx| {
            OrchestrationEventStreamer::new_with_clients_for_test(ai_client, server_api, ctx)
        });

        streamer.update(&mut app, |me, ctx| {
            me.streams.entry(parent_conversation_id).or_default();
            me.remember_killed_run_id(killed_run_id.clone());
            me.handle_event_batch(
                parent_conversation_id,
                &parent_run_id,
                0,
                vec![
                    AgentRunEvent {
                        event_type: "new_message".to_string(),
                        run_id: killed_run_id.clone(),
                        ref_id: Some("message-from-killed-child".to_string()),
                        execution_id: None,
                        occurred_at: "2026-01-01T00:00:00Z".to_string(),
                        sequence: 17,
                    },
                    AgentRunEvent {
                        event_type: "run_cancelled".to_string(),
                        run_id: killed_run_id.clone(),
                        ref_id: None,
                        execution_id: None,
                        occurred_at: "2026-01-01T00:00:01Z".to_string(),
                        sequence: 18,
                    },
                    AgentRunEvent {
                        event_type: "new_message".to_string(),
                        run_id: killed_run_id.clone(),
                        ref_id: None,
                        execution_id: None,
                        occurred_at: "2026-01-01T00:00:02Z".to_string(),
                        sequence: 19,
                    },
                ],
                vec![
                    ReceivedMessageInput {
                        message_id: "message-from-killed-child".to_string(),
                        sender_agent_id: killed_run_id.clone(),
                        addresses: vec![parent_run_id.clone()],
                        subject: "late message".to_string(),
                        message_body: "body".to_string(),
                    },
                    ReceivedMessageInput {
                        message_id: "message-from-killed-child-without-ref".to_string(),
                        sender_agent_id: killed_run_id.clone(),
                        addresses: vec![parent_run_id.clone()],
                        subject: "late message without ref".to_string(),
                        message_body: "body".to_string(),
                    },
                ],
                ctx,
            );
        });

        event_service.read(&app, |service, _| {
            assert!(
                !service.has_pending_events(parent_conversation_id),
                "late events from killed run IDs must not be enqueued"
            );
        });
        history_model.read(&app, |model, _| {
            let last_seq = model
                .conversation(&parent_conversation_id)
                .and_then(|conversation| conversation.last_event_sequence());
            assert_eq!(
                last_seq,
                Some(19),
                "cursor must still advance so dropped killed-run events are not replayed"
            );
        });
    });
}

#[test]
fn killed_run_ids_are_bounded() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));
        let mock = MockAIClient::new();
        let ai_client: Arc<dyn AIClient> = Arc::new(mock);
        let server_api = ServerApiProvider::new_for_test().get();
        let streamer = app.add_singleton_model(|ctx| {
            OrchestrationEventStreamer::new_with_clients_for_test(ai_client, server_api, ctx)
        });

        streamer.update(&mut app, |me, _| {
            for index in 0..=MAX_KILLED_RUN_IDS {
                me.remember_killed_run_id(format!("killed-run-{index}"));
            }
        });

        streamer.read(&app, |me, _| {
            assert_eq!(me.killed_run_ids.len(), MAX_KILLED_RUN_IDS);
            assert!(!me.killed_run_ids.contains("killed-run-0"));
            assert!(me.killed_run_ids.contains("killed-run-1"));
            assert!(me
                .killed_run_ids
                .contains(&format!("killed-run-{MAX_KILLED_RUN_IDS}")));
        });
    });
}

#[test]
fn finish_restore_fetch_no_ops_when_conversation_deleted_mid_flight() {
    // If the conversation is removed while the async fetch is in-flight, the
    // RemoveConversation handler removes the streams entry. finish_restore_fetch
    // uses the missing entry as a sentinel and must not re-populate
    // streamer state for the deleted conversation.
    use std::sync::Arc;

    use warpui::App;

    use crate::ai::agent::conversation::AIConversation;
    use crate::server::server_api::ai::MockAIClient;
    use crate::server::server_api::ServerApiProvider;

    App::test((), |mut app| async move {
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        let mut conversation = AIConversation::new(false, false);
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
    use std::sync::Arc;

    use warpui::App;

    use crate::ai::agent::conversation::AIConversation;
    use crate::server::server_api::ai::MockAIClient;
    use crate::server::server_api::ServerApiProvider;

    App::test((), |mut app| async move {
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        let mut conversation = AIConversation::new(false, false);
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
    use std::sync::Arc;

    use warpui::App;

    use crate::ai::agent::conversation::AIConversation;
    use crate::server::server_api::ai::MockAIClient;
    use crate::server::server_api::ServerApiProvider;

    App::test((), |mut app| async move {
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        let parent_id = AIConversation::new(false, false).id();
        let mut child_conversation = AIConversation::new(false, false);
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
fn on_conversation_removed_prunes_killed_child_run_id_from_parent_but_keeps_tombstone() {
    use std::sync::Arc;

    use warpui::App;

    use crate::ai::agent::conversation::AIConversation;
    use crate::server::server_api::ai::MockAIClient;
    use crate::server::server_api::ServerApiProvider;

    App::test((), |mut app| async move {
        app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        let parent_id = AIConversation::new(false, false).id();
        let child_id = AIConversation::new(false, false).id();
        let child_run_id = "550e8400-e29b-41d4-a716-446655440601".to_string();

        let mock = MockAIClient::new();
        let ai_client: Arc<dyn AIClient> = Arc::new(mock);
        let server_api = ServerApiProvider::new_for_test().get();

        let poller = app.add_singleton_model(|ctx| {
            OrchestrationEventStreamer::new_with_clients_for_test(ai_client, server_api, ctx)
        });

        poller.update(&mut app, |me, ctx| {
            me.streams
                .entry(parent_id)
                .or_default()
                .watched_run_ids
                .insert(child_run_id.clone());
            me.remember_killed_run_id(child_run_id.clone());

            me.on_conversation_removed(child_id, Some(child_run_id.clone()), ctx);
        });

        poller.read(&app, |me, _| {
            assert!(me.killed_run_ids.contains(&child_run_id));
            assert!(
                me.streams
                    .get(&parent_id)
                    .is_some_and(|s| !s.watched_run_ids.contains(&child_run_id)),
                "killed child run_id should be pruned from parent watchers"
            );
        });
    });
}

// ---- Viewer-mode bookkeeping surface ----
//
// The tests below cover the additive surface that supports the
// viewer-mode ancestor SSE path: `conversation_status_from_lifecycle_event_type`,
// `is_known_child`, the `register_viewer_mode_consumer` /
// `unregister_viewer_mode_consumer` refcount, and the
// `is_remote_run_view` flag-gated relaxation. These tests drive the
// bookkeeping directly via pure-function calls or short App fixtures.

#[test]
fn lifecycle_event_type_in_progress_maps_to_in_progress() {
    assert_eq!(
        conversation_status_from_lifecycle_event_type(api::LifecycleEventType::InProgress),
        ConversationStatus::InProgress
    );
}

#[test]
fn lifecycle_event_type_succeeded_maps_to_success() {
    assert_eq!(
        conversation_status_from_lifecycle_event_type(api::LifecycleEventType::Succeeded),
        ConversationStatus::Success
    );
}

#[test]
fn lifecycle_event_type_failed_maps_to_error() {
    assert_eq!(
        conversation_status_from_lifecycle_event_type(api::LifecycleEventType::Failed),
        ConversationStatus::Error
    );
}

#[test]
fn lifecycle_event_type_errored_maps_to_error() {
    assert_eq!(
        conversation_status_from_lifecycle_event_type(api::LifecycleEventType::Errored),
        ConversationStatus::Error
    );
}

#[test]
fn lifecycle_event_type_cancelled_maps_to_cancelled() {
    assert_eq!(
        conversation_status_from_lifecycle_event_type(api::LifecycleEventType::Cancelled),
        ConversationStatus::Cancelled
    );
}

#[test]
fn lifecycle_event_type_blocked_maps_to_blocked_with_empty_action() {
    // Matches the REST path on `AmbientAgentTaskState::Blocked`: empty
    // `blocked_action`. The wire event does not currently carry a
    // `blocked_action` payload.
    assert_eq!(
        conversation_status_from_lifecycle_event_type(api::LifecycleEventType::Blocked),
        ConversationStatus::Blocked {
            blocked_action: String::new(),
        }
    );
}

#[test]
#[allow(deprecated)]
fn lifecycle_event_type_legacy_started_maps_to_in_progress() {
    assert_eq!(
        conversation_status_from_lifecycle_event_type(api::LifecycleEventType::Started),
        ConversationStatus::InProgress
    );
}

#[test]
#[allow(deprecated)]
fn lifecycle_event_type_legacy_restarted_maps_to_in_progress() {
    assert_eq!(
        conversation_status_from_lifecycle_event_type(api::LifecycleEventType::Restarted),
        ConversationStatus::InProgress
    );
}

#[test]
#[allow(deprecated)]
fn lifecycle_event_type_legacy_idle_maps_to_success() {
    assert_eq!(
        conversation_status_from_lifecycle_event_type(api::LifecycleEventType::Idle),
        ConversationStatus::Success
    );
}

#[test]
fn unknown_lifecycle_maps_to_error() {
    // `Unspecified` is the proto's forward-compat catch-all. The viewer's
    // `AmbientAgentTaskState::Unknown` similarly collapses to `Error`;
    // matching that keeps the pill bar in a defined state for any future
    // wire-level variant the client doesn't recognize.
    assert_eq!(
        conversation_status_from_lifecycle_event_type(api::LifecycleEventType::Unspecified),
        ConversationStatus::Error
    );
}

fn make_parent_task_id_for_test(byte: u8) -> AmbientAgentTaskId {
    // Stable, distinct task IDs per byte; the UUID itself is not load-bearing.
    let mut bytes = [0u8; 16];
    bytes[0] = 0x55;
    bytes[1] = 0x0e;
    bytes[2] = 0x84;
    bytes[3] = 0x00;
    bytes[4] = 0xe2;
    bytes[5] = 0x9b;
    bytes[6] = 0x41;
    bytes[7] = 0xd4;
    bytes[8] = 0xa7;
    bytes[9] = 0x16;
    bytes[10] = 0x44;
    bytes[11] = 0x66;
    bytes[12] = 0x55;
    bytes[13] = 0x44;
    bytes[14] = 0x00;
    bytes[15] = byte;
    let uuid = uuid::Uuid::from_bytes(bytes);
    let s = uuid.to_string();
    s.parse().expect("valid task id")
}

#[test]
fn is_known_child_dedupes_per_parent_after_first_observation() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        let mock = MockAIClient::new();
        let ai_client: Arc<dyn AIClient> = Arc::new(mock);
        let server_api = ServerApiProvider::new_for_test().get();

        let streamer = app.add_singleton_model(|ctx| {
            OrchestrationEventStreamer::new_with_clients_for_test(ai_client, server_api, ctx)
        });

        let parent_task_id = make_parent_task_id_for_test(0xa1);
        let run_id = "child-run-1";

        // Before any registration, the entry doesn't exist and the run is unknown.
        streamer.read(&app, |me, _| {
            assert!(
                !me.is_known_child(parent_task_id, run_id),
                "unknown parent_task_id must report run as not-known"
            );
        });

        // Register a consumer to materialize the entry, then seed the known
        // set (simulating the emission path that populates this on the
        // first lifecycle event observed for a new run_id).
        let consumer_id = warpui::EntityId::new();
        let placeholder_conv_id =
            crate::ai::agent::conversation::AIConversation::new(true, false).id();
        streamer.update(&mut app, |me, ctx| {
            me.register_viewer_mode_consumer(parent_task_id, placeholder_conv_id, consumer_id, ctx);
        });

        streamer.read(&app, |me, _| {
            assert!(
                !me.is_known_child(parent_task_id, run_id),
                "newly-registered viewer-mode entry must report unseen run as not-known"
            );
        });

        // First observation: seed `known_children` (emission path).
        streamer.update(&mut app, |me, _| {
            me.viewer_mode_orchestrators
                .get_mut(&parent_task_id)
                .expect("entry exists")
                .known_children
                .insert(run_id.to_string());
        });

        streamer.read(&app, |me, _| {
            assert!(
                me.is_known_child(parent_task_id, run_id),
                "after first observation the run must be known"
            );
        });
    });
}

#[test]
fn is_known_child_isolated_per_parent() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        let mock = MockAIClient::new();
        let ai_client: Arc<dyn AIClient> = Arc::new(mock);
        let server_api = ServerApiProvider::new_for_test().get();

        let streamer = app.add_singleton_model(|ctx| {
            OrchestrationEventStreamer::new_with_clients_for_test(ai_client, server_api, ctx)
        });

        let parent_a = make_parent_task_id_for_test(0xb1);
        let parent_b = make_parent_task_id_for_test(0xb2);
        let shared_run_id = "child-run-shared";

        let consumer_id = warpui::EntityId::new();
        let placeholder_conv_id =
            crate::ai::agent::conversation::AIConversation::new(true, false).id();
        streamer.update(&mut app, |me, ctx| {
            me.register_viewer_mode_consumer(parent_a, placeholder_conv_id, consumer_id, ctx);
            me.register_viewer_mode_consumer(parent_b, placeholder_conv_id, consumer_id, ctx);
            me.viewer_mode_orchestrators
                .get_mut(&parent_a)
                .expect("entry A")
                .known_children
                .insert(shared_run_id.to_string());
        });

        streamer.read(&app, |me, _| {
            assert!(
                me.is_known_child(parent_a, shared_run_id),
                "run_id seeded under parent A must be known to parent A"
            );
            assert!(
                !me.is_known_child(parent_b, shared_run_id),
                "per-parent isolation: run_id seeded under parent A must NOT be known to parent B"
            );
        });
    });
}

#[test]
fn viewer_mode_consumer_refcount_handles_multiple_panes_and_double_unregister() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        let mock = MockAIClient::new();
        let ai_client: Arc<dyn AIClient> = Arc::new(mock);
        let server_api = ServerApiProvider::new_for_test().get();

        let streamer = app.add_singleton_model(|ctx| {
            OrchestrationEventStreamer::new_with_clients_for_test(ai_client, server_api, ctx)
        });

        let parent_task_id = make_parent_task_id_for_test(0xc1);
        let consumer_a = warpui::EntityId::new();
        let consumer_b = warpui::EntityId::new();
        // Each pane has its own orchestrator-placeholder conversation; the
        // recorded value is used to persist per-pane cursors.
        let placeholder_a = crate::ai::agent::conversation::AIConversation::new(true, false).id();
        let placeholder_b = crate::ai::agent::conversation::AIConversation::new(true, false).id();

        // Register two panes for the same orchestrator. Refcount should be 2
        // and both placeholders should be recorded.
        streamer.update(&mut app, |me, ctx| {
            me.register_viewer_mode_consumer(parent_task_id, placeholder_a, consumer_a, ctx);
            me.register_viewer_mode_consumer(parent_task_id, placeholder_b, consumer_b, ctx);
        });

        streamer.read(&app, |me, _| {
            let entry = me
                .viewer_mode_orchestrators
                .get(&parent_task_id)
                .expect("entry must exist after registration");
            assert_eq!(entry.consumers.len(), 2, "two viewer panes => refcount=2");
            assert_eq!(entry.consumers.get(&consumer_a), Some(&placeholder_a));
            assert_eq!(entry.consumers.get(&consumer_b), Some(&placeholder_b));
        });

        // Unregister pane A. Entry must stay alive (pane B still registered).
        streamer.update(&mut app, |me, _| {
            me.unregister_viewer_mode_consumer(parent_task_id, consumer_a);
        });
        streamer.read(&app, |me, _| {
            let entry = me
                .viewer_mode_orchestrators
                .get(&parent_task_id)
                .expect("entry must remain while at least one consumer is registered");
            assert_eq!(entry.consumers.len(), 1);
            assert!(entry.consumers.contains_key(&consumer_b));
        });

        // Unregister pane B. Entry should now be removed.
        streamer.update(&mut app, |me, _| {
            me.unregister_viewer_mode_consumer(parent_task_id, consumer_b);
        });
        streamer.read(&app, |me, _| {
            assert!(
                !me.viewer_mode_orchestrators.contains_key(&parent_task_id),
                "entry must be removed once the last consumer unregisters"
            );
        });

        // Double-unregister must be a no-op. Covers the `Drop` refcount race
        // where a late `Drop` impl unregisters after the last consumer has
        // already removed the entry.
        streamer.update(&mut app, |me, _| {
            me.unregister_viewer_mode_consumer(parent_task_id, consumer_a);
            me.unregister_viewer_mode_consumer(parent_task_id, consumer_b);
        });
        streamer.read(&app, |me, _| {
            assert!(
                !me.viewer_mode_orchestrators.contains_key(&parent_task_id),
                "entry must stay absent after double-unregister"
            );
        });
    });
}

#[test]
fn is_remote_run_view_excludes_shared_session_viewer() {
    use crate::ai::agent::conversation::AIConversation;

    App::test((), |mut app| async move {
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        // Build a shared-session viewer conversation by passing
        // `is_viewing_shared_session = true` to `AIConversation::new`.
        let conversation = AIConversation::new(true, false);
        let conversation_id = conversation.id();
        let terminal_view_id = warpui::EntityId::new();
        history_model.update(&mut app, |model, ctx| {
            model.restore_conversations(terminal_view_id, vec![conversation], ctx);
        });

        let mock = MockAIClient::new();
        let ai_client: Arc<dyn AIClient> = Arc::new(mock);
        let server_api = ServerApiProvider::new_for_test().get();

        let streamer = app.add_singleton_model(|ctx| {
            OrchestrationEventStreamer::new_with_clients_for_test(ai_client, server_api, ctx)
        });

        streamer.read(&app, |me, ctx| {
            assert!(
                me.is_remote_run_view(conversation_id, ctx),
                "shared-session viewer conversations are passive remote-run views"
            );
        });
    });
}

#[test]
fn is_remote_run_view_excludes_remote_child() {
    use crate::ai::agent::conversation::AIConversation;

    App::test((), |mut app| async move {
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        let mut conversation = AIConversation::new(false, false);
        conversation.mark_as_remote_child();
        let conversation_id = conversation.id();
        let terminal_view_id = warpui::EntityId::new();
        history_model.update(&mut app, |model, ctx| {
            model.restore_conversations(terminal_view_id, vec![conversation], ctx);
        });

        let mock = MockAIClient::new();
        let ai_client: Arc<dyn AIClient> = Arc::new(mock);
        let server_api = ServerApiProvider::new_for_test().get();

        let streamer = app.add_singleton_model(|ctx| {
            OrchestrationEventStreamer::new_with_clients_for_test(ai_client, server_api, ctx)
        });

        streamer.read(&app, |me, ctx| {
            assert!(
                me.is_remote_run_view(conversation_id, ctx),
                "remote-child conversations represent owner-side runs hosted elsewhere"
            );
        });
    });
}

#[test]
fn reevaluate_eligibility_does_not_reconnect_when_watched_run_ids_unchanged() {
    App::test((), |mut app| async move {
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        let own_run_id = "550e8400-e29b-41d4-a716-446655440401";
        let child_run_id = "550e8400-e29b-41d4-a716-446655440402";
        let mut conversation = AIConversation::new(false, false);
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

        // Open SSE (gen 0) with watched set == connected snapshot.
        let (_, rx) = futures::channel::mpsc::unbounded::<SseStreamItem>();
        let consumer_id = warpui::EntityId::new();
        poller.update(&mut app, |me, _| {
            let stream = me.streams.entry(conversation_id).or_default();
            stream.event_cursor = 0;
            stream.watched_run_ids.insert(own_run_id.to_string());
            stream.watched_run_ids.insert(child_run_id.to_string());
            stream.consumers.insert(consumer_id);
            let (abort_handle, _) = futures::future::AbortHandle::new_pair();
            let connected_filter =
                AgentEventFilter::RunIds(vec![own_run_id.to_string(), child_run_id.to_string()]);
            stream.sse_connection = Some(SseConnectionState {
                event_receiver: rx,
                generation: 0,
                abort_handle,
                connected_filter,
            });
            me.next_sse_generation = 1;
        });

        // Fire a status transition; should reach `reevaluate_eligibility`
        // (true, true) without changing the run_id set.
        history_model.update(&mut app, |model, ctx| {
            model.update_conversation_status(
                terminal_view_id,
                conversation_id,
                ConversationStatus::Success,
                ctx,
            );
        });

        poller.read(&app, |me, _| {
            let generation = me
                .streams
                .get(&conversation_id)
                .and_then(|s| s.sse_connection.as_ref())
                .map(|c| c.generation);
            assert_eq!(
                generation,
                Some(0),
                "SSE must not reconnect when watched_run_ids is unchanged; got generation={generation:?}"
            );
        });
    });
}

#[test]
fn finish_restore_fetch_reconnects_sse_when_children_added_to_open_connection() {
    // When a status transition races with the restore fetch and opens SSE
    // before children are known, finish_restore_fetch must reconnect SSE
    // with the updated run_id set rather than leaving children unwatched.
    use std::sync::Arc;

    use warpui::App;

    use crate::ai::agent::conversation::{AIConversation, ConversationStatus};
    use crate::server::server_api::ai::MockAIClient;
    use crate::server::server_api::ServerApiProvider;

    App::test((), |mut app| async move {
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        let own_run_id = "550e8400-e29b-41d4-a716-446655440400";
        let mut conversation = AIConversation::new(false, false);
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
            let (abort_handle, _) = futures::future::AbortHandle::new_pair();
            let connected_filter = AgentEventFilter::RunIds(vec![own_run_id.to_string()]);
            stream.sse_connection = Some(SseConnectionState {
                event_receiver: rx,
                generation: 0,
                abort_handle,
                connected_filter,
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
            let generation = me
                .streams
                .get(&conversation_id)
                .and_then(|s| s.sse_connection.as_ref())
                .map(|c| c.generation);
            assert!(
                generation.is_some_and(|g| g > 0),
                "SSE must be reconnected (new generation) after children are discovered; got generation={generation:?}"
            );
        });
    });
}

/// Captures `ChildSpawned` events emitted by the streamer so the regression
/// tests below can assert exactly which children were broadcast.
///
/// Subscribes from the app context (mirrors the pattern in
/// `notebooks/link_tests.rs`) so we don't need a real subscriber model.
fn capture_child_spawns(
    app: &mut App,
    streamer: &warpui::ModelHandle<OrchestrationEventStreamer>,
) -> std::sync::Arc<parking_lot::Mutex<Vec<(AmbientAgentTaskId, String)>>> {
    let captured: std::sync::Arc<parking_lot::Mutex<Vec<(AmbientAgentTaskId, String)>>> =
        std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
    let captured_for_closure = captured.clone();
    app.update(|ctx| {
        ctx.subscribe_to_model(streamer, move |_, event, _| {
            if let OrchestrationEventStreamerEvent::ChildSpawned {
                parent_task_id,
                run_id,
            } = event
            {
                captured_for_closure
                    .lock()
                    .push((*parent_task_id, run_id.clone()));
            }
        })
    });
    captured
}

#[test]
fn finish_ancestor_seed_fetch_emits_child_spawned_for_each_seeded_child() {
    // Regression test for the orchestration viewer pill bar in the
    // remote-remote case: the cold-start REST seed must broadcast
    // `ChildSpawned` for every seeded child so the viewer model materializes
    // a pill placeholder per child. Previously the seed populated
    // `known_children` and advanced the cursor but did not emit, so
    // the pill bar stayed empty until a new lifecycle event arrived for
    // a child the SSE had not yet replayed.
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        let mock = MockAIClient::new();
        let ai_client: Arc<dyn AIClient> = Arc::new(mock);
        let server_api = ServerApiProvider::new_for_test().get();

        let streamer = app.add_singleton_model(|ctx| {
            OrchestrationEventStreamer::new_with_clients_for_test(ai_client, server_api, ctx)
        });

        let parent_task_id = make_parent_task_id_for_test(0xd1);
        let child_a = make_parent_task_id_for_test(0xd2);
        let child_b = make_parent_task_id_for_test(0xd3);

        // Register a viewer-mode consumer so the entry exists. The seed
        // fetch is normally kicked off by registration; here we drive
        // `finish_ancestor_seed_fetch` synchronously to control the input.
        let consumer_id = warpui::EntityId::new();
        let placeholder_conv_id =
            crate::ai::agent::conversation::AIConversation::new(true, false).id();
        streamer.update(&mut app, |me, ctx| {
            me.register_viewer_mode_consumer(parent_task_id, placeholder_conv_id, consumer_id, ctx);
        });

        let captured_spawns = capture_child_spawns(&mut app, &streamer);

        // Drive the seed-apply path directly with a parent-and-two-children
        // payload (matching the REST endpoint shape that may include the
        // parent itself in the response).
        streamer.update(&mut app, |me, ctx| {
            me.finish_ancestor_seed_fetch(
                parent_task_id,
                Ok(vec![
                    make_ambient_task_with_task_id(parent_task_id, Some(5)),
                    make_ambient_task_with_task_id(child_a, Some(11)),
                    make_ambient_task_with_task_id(child_b, Some(7)),
                ]),
                ctx,
            );
        });

        let spawns = captured_spawns.lock().clone();
        let mut seen: Vec<String> = spawns
            .iter()
            .filter(|(parent, _)| *parent == parent_task_id)
            .map(|(_, run_id)| run_id.clone())
            .collect();
        seen.sort();
        let mut expected = vec![child_a.to_string(), child_b.to_string()];
        expected.sort();
        assert_eq!(
            seen, expected,
            "ChildSpawned must be emitted exactly once per seeded child \
             (parent excluded)"
        );

        streamer.read(&app, |me, _| {
            assert!(
                me.is_known_child(parent_task_id, &child_a.to_string()),
                "child_a should be in known_children after seed"
            );
            assert!(
                me.is_known_child(parent_task_id, &child_b.to_string()),
                "child_b should be in known_children after seed"
            );
            assert!(
                !me.is_known_child(parent_task_id, &parent_task_id.to_string()),
                "the parent's own task_id must NOT be tracked as a child"
            );
            let entry = me
                .viewer_mode_orchestrators
                .get(&parent_task_id)
                .expect("viewer-mode entry exists after seed");
            assert!(entry.seeded, "seeded flag must flip after seed apply");
            assert_eq!(
                entry.event_cursor, 11,
                "event_cursor must advance to max(child.last_event_sequence)"
            );
        });
    });
}

#[test]
fn register_viewer_mode_consumer_replays_known_children_for_later_panes() {
    // Regression for the late-arriving-consumer arm of the same bug: the
    // shared-session viewer model often registers its viewer-mode consumer
    // *after* the ancestor seed has already been applied (because the
    // active parent placeholder isn't set on the terminal view at
    // construction time). Without a replay, the new consumer never observes
    // `ChildSpawned` for known children and the pill bar stays empty.
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        let mock = MockAIClient::new();
        let ai_client: Arc<dyn AIClient> = Arc::new(mock);
        let server_api = ServerApiProvider::new_for_test().get();

        let streamer = app.add_singleton_model(|ctx| {
            OrchestrationEventStreamer::new_with_clients_for_test(ai_client, server_api, ctx)
        });

        let parent_task_id = make_parent_task_id_for_test(0xe1);
        let child_a = make_parent_task_id_for_test(0xe2);
        let consumer_a = warpui::EntityId::new();
        let consumer_b = warpui::EntityId::new();
        let placeholder_a = crate::ai::agent::conversation::AIConversation::new(true, false).id();
        let placeholder_b = crate::ai::agent::conversation::AIConversation::new(true, false).id();

        // Pane A registers first and the seed lands.
        streamer.update(&mut app, |me, ctx| {
            me.register_viewer_mode_consumer(parent_task_id, placeholder_a, consumer_a, ctx);
            me.finish_ancestor_seed_fetch(
                parent_task_id,
                Ok(vec![make_ambient_task_with_task_id(child_a, Some(3))]),
                ctx,
            );
        });

        // Subscribe AFTER the seed has been applied so only the replay
        // emissions are captured.
        let captured_spawns = capture_child_spawns(&mut app, &streamer);

        // Pane B registers later — the entry is already seeded. The streamer
        // must replay `ChildSpawned` for the already-known children so
        // pane B materializes pill placeholders identical to pane A.
        streamer.update(&mut app, |me, ctx| {
            me.register_viewer_mode_consumer(parent_task_id, placeholder_b, consumer_b, ctx);
        });

        let spawns = captured_spawns.lock().clone();
        let replayed: Vec<&(AmbientAgentTaskId, String)> = spawns
            .iter()
            .filter(|(parent, run_id)| *parent == parent_task_id && run_id == &child_a.to_string())
            .collect();
        assert_eq!(
            replayed.len(),
            1,
            "second viewer-mode registration must replay ChildSpawned exactly once \
             per known child (captured={spawns:?})"
        );
    });
}

// ---- Owner-side parent-family ancestor streaming -------------------------

/// Reads the connected filter for a conversation's owner-side SSE.
fn connected_filter(
    me: &OrchestrationEventStreamer,
    conversation_id: AIConversationId,
) -> Option<AgentEventFilter> {
    me.streams
        .get(&conversation_id)
        .and_then(|s| s.sse_connection.as_ref())
        .map(|c| c.connected_filter.clone())
}

/// Reads the generation for a conversation's owner-side SSE.
fn sse_generation(
    me: &OrchestrationEventStreamer,
    conversation_id: AIConversationId,
) -> Option<u64> {
    me.streams
        .get(&conversation_id)
        .and_then(|s| s.sse_connection.as_ref())
        .map(|c| c.generation)
}

#[test]
fn parent_with_many_children_opens_one_ancestor_include_self_stream() {
    // A parent with far more than the 100 explicit-run-id limit must open a
    // single parent-family ancestor stream rather than a run-id vector.
    App::test((), |mut app| async move {
        let _owner_guard = FeatureFlag::OwnerOrchestrationAncestorStreamer.override_enabled(true);

        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));
        let own_run_id = "550e8400-e29b-41d4-a716-446655440500";
        let mut conversation = AIConversation::new(false, false);
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

        let ai_client: Arc<dyn AIClient> = Arc::new(MockAIClient::new());
        let server_api = ServerApiProvider::new_for_test().get();
        let poller = app.add_singleton_model(|ctx| {
            OrchestrationEventStreamer::new_with_clients_for_test(ai_client, server_api, ctx)
        });

        let consumer_id = warpui::EntityId::new();
        poller.update(&mut app, |me, ctx| {
            let stream = me.streams.entry(conversation_id).or_default();
            stream.consumers.insert(consumer_id);
            stream.watched_run_ids.insert(own_run_id.to_string());
            for i in 0..190 {
                stream.watched_run_ids.insert(format!("child-{i}"));
            }
            me.start_sse_connection(conversation_id, ctx);
        });

        poller.read(&app, |me, _| match connected_filter(me, conversation_id) {
            Some(AgentEventFilter::AncestorRunId {
                ancestor_run_id,
                include_self,
            }) => {
                assert_eq!(ancestor_run_id, own_run_id);
                assert!(
                    include_self,
                    "owner-side parent-family stream must include self"
                );
            }
            other => panic!("expected AncestorRunId include_self filter, got {other:?}"),
        });
    });
}

#[test]
fn registering_additional_child_does_not_reconnect_parent_family_stream() {
    // Once a parent-family ancestor stream is connected, registering more
    // children must not reconnect: the filter shape is unchanged.
    App::test((), |mut app| async move {
        let _owner_guard = FeatureFlag::OwnerOrchestrationAncestorStreamer.override_enabled(true);

        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));
        let own_run_id = "550e8400-e29b-41d4-a716-446655440501";
        let mut conversation = AIConversation::new(false, false);
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

        let ai_client: Arc<dyn AIClient> = Arc::new(MockAIClient::new());
        let server_api = ServerApiProvider::new_for_test().get();
        let poller = app.add_singleton_model(|ctx| {
            OrchestrationEventStreamer::new_with_clients_for_test(ai_client, server_api, ctx)
        });

        let consumer_id = warpui::EntityId::new();
        poller.update(&mut app, |me, ctx| {
            let stream = me.streams.entry(conversation_id).or_default();
            stream.consumers.insert(consumer_id);
            stream.watched_run_ids.insert(own_run_id.to_string());
            stream.watched_run_ids.insert("child-1".to_string());
            me.start_sse_connection(conversation_id, ctx);
        });
        poller.read(&app, |me, _| {
            assert_eq!(sse_generation(me, conversation_id), Some(0));
            assert!(matches!(
                connected_filter(me, conversation_id),
                Some(AgentEventFilter::AncestorRunId {
                    include_self: true,
                    ..
                })
            ));
        });

        // Registering another child re-evaluates eligibility but must not
        // reconnect, since the parent-family filter is unchanged.
        poller.update(&mut app, |me, ctx| {
            me.register_watched_run_id(conversation_id, "child-2".to_string(), ctx);
        });
        poller.read(&app, |me, _| {
            assert_eq!(
                sse_generation(me, conversation_id),
                Some(0),
                "parent-family stream must not reconnect when a new child is registered"
            );
        });
    });
}

#[test]
fn child_only_conversation_opens_self_run_id_filter() {
    // A child-only conversation keeps the explicit run-id stream watching
    // only its own run_id, even when the owner-ancestor flag is enabled.
    App::test((), |mut app| async move {
        let _owner_guard = FeatureFlag::OwnerOrchestrationAncestorStreamer.override_enabled(true);

        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));
        let own_run_id = "550e8400-e29b-41d4-a716-446655440502";
        let mut conversation = AIConversation::new(false, false);
        conversation.set_run_id(own_run_id.to_string());
        conversation.set_parent_agent_id("550e8400-e29b-41d4-a716-4466554405ff".to_string());
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

        let ai_client: Arc<dyn AIClient> = Arc::new(MockAIClient::new());
        let server_api = ServerApiProvider::new_for_test().get();
        let poller = app.add_singleton_model(|ctx| {
            OrchestrationEventStreamer::new_with_clients_for_test(ai_client, server_api, ctx)
        });

        let consumer_id = warpui::EntityId::new();
        poller.update(&mut app, |me, ctx| {
            let stream = me.streams.entry(conversation_id).or_default();
            stream.consumers.insert(consumer_id);
            stream.watched_run_ids.insert(own_run_id.to_string());
            me.start_sse_connection(conversation_id, ctx);
        });

        poller.read(&app, |me, _| match connected_filter(me, conversation_id) {
            Some(AgentEventFilter::RunIds(run_ids)) => {
                assert_eq!(run_ids, vec![own_run_id.to_string()]);
            }
            other => panic!("expected RunIds([self]) filter, got {other:?}"),
        });
    });
}

#[test]
fn parent_over_run_id_limit_without_flag_does_not_open_stream() {
    // With owner-ancestor streaming disabled, a parent that exceeds the
    // explicit-run-id limit must not open a known-bad stream; instead it
    // avoids opening a stream the server would reject.
    App::test((), |mut app| async move {
        let _owner_guard = FeatureFlag::OwnerOrchestrationAncestorStreamer.override_enabled(false);

        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));
        let own_run_id = "550e8400-e29b-41d4-a716-446655440503";
        let mut conversation = AIConversation::new(false, false);
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

        let ai_client: Arc<dyn AIClient> = Arc::new(MockAIClient::new());
        let server_api = ServerApiProvider::new_for_test().get();
        let poller = app.add_singleton_model(|ctx| {
            OrchestrationEventStreamer::new_with_clients_for_test(ai_client, server_api, ctx)
        });

        let consumer_id = warpui::EntityId::new();
        poller.update(&mut app, |me, ctx| {
            let stream = me.streams.entry(conversation_id).or_default();
            stream.consumers.insert(consumer_id);
            stream.watched_run_ids.insert(own_run_id.to_string());
            for i in 0..150 {
                stream.watched_run_ids.insert(format!("child-{i}"));
            }
            me.start_sse_connection(conversation_id, ctx);
        });

        poller.read(&app, |me, _| {
            let stream = me.streams.get(&conversation_id).expect("stream present");
            assert!(
                stream.sse_connection.is_none(),
                "must not open an oversized run-id stream the server would reject"
            );
        });
    });
}

#[test]
fn parent_crossing_run_id_limit_without_flag_tears_down_partial_stream() {
    // If a parent starts below the explicit-run-id limit and later registers
    // enough children to exceed it, the old stream has become partial. It must
    // be torn down instead of silently missing newly-registered children.
    App::test((), |mut app| async move {
        let _owner_guard = FeatureFlag::OwnerOrchestrationAncestorStreamer.override_enabled(false);

        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));
        let own_run_id = "550e8400-e29b-41d4-a716-446655440507";
        let mut conversation = AIConversation::new(false, false);
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

        let ai_client: Arc<dyn AIClient> = Arc::new(MockAIClient::new());
        let server_api = ServerApiProvider::new_for_test().get();
        let poller = app.add_singleton_model(|ctx| {
            OrchestrationEventStreamer::new_with_clients_for_test(ai_client, server_api, ctx)
        });

        let consumer_id = warpui::EntityId::new();
        poller.update(&mut app, |me, ctx| {
            let stream = me.streams.entry(conversation_id).or_default();
            stream.consumers.insert(consumer_id);
            stream.watched_run_ids.insert(own_run_id.to_string());
            for i in 0..99 {
                stream.watched_run_ids.insert(format!("child-{i}"));
            }
            me.start_sse_connection(conversation_id, ctx);
        });

        poller.read(&app, |me, _| {
            let stream = me.streams.get(&conversation_id).expect("stream present");
            assert!(
                stream.sse_connection.is_some(),
                "100 watched run IDs is still within the explicit-run-id limit"
            );
        });

        poller.update(&mut app, |me, ctx| {
            me.register_watched_run_id(conversation_id, "child-over-limit".to_string(), ctx);
        });

        poller.read(&app, |me, _| {
            let stream = me.streams.get(&conversation_id).expect("stream present");
            assert!(
                stream.sse_connection.is_none(),
                "oversized parent must tear down the old partial run-id stream"
            );
        });
    });
}
#[test]
fn restored_parent_with_children_opens_ancestor_include_self_stream() {
    // A restored parent whose children come back from the server fetch opens
    // a parent-family ancestor stream using the merged cursor.
    App::test((), |mut app| async move {
        let _owner_guard = FeatureFlag::OwnerOrchestrationAncestorStreamer.override_enabled(true);

        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));
        let own_run_id = "550e8400-e29b-41d4-a716-446655440504";
        let mut conversation = AIConversation::new(false, false);
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

        let ai_client: Arc<dyn AIClient> = Arc::new(MockAIClient::new());
        let server_api = ServerApiProvider::new_for_test().get();
        let poller = app.add_singleton_model(|ctx| {
            OrchestrationEventStreamer::new_with_clients_for_test(ai_client, server_api, ctx)
        });

        let consumer_id = warpui::EntityId::new();
        poller.update(&mut app, |me, _| {
            let stream = me.streams.entry(conversation_id).or_default();
            stream.event_cursor = 5;
            stream.watched_run_ids.insert(own_run_id.to_string());
            stream.consumers.insert(consumer_id);
        });

        let task_id: crate::ai::ambient_agents::AmbientAgentTaskId =
            "550e8400-e29b-41d4-a716-446655440000".parse().unwrap();
        poller.update(&mut app, |me, ctx| {
            me.finish_restore_fetch(
                conversation_id,
                task_id,
                /* sqlite_cursor */ 5,
                Ok(make_ambient_task_with_children(vec![
                    "child-run-1".to_string()
                ])),
                ctx,
            );
        });

        poller.read(&app, |me, _| {
            match connected_filter(me, conversation_id) {
                Some(AgentEventFilter::AncestorRunId {
                    ancestor_run_id,
                    include_self,
                }) => {
                    assert_eq!(ancestor_run_id, own_run_id);
                    assert!(include_self);
                }
                other => panic!("expected AncestorRunId include_self filter, got {other:?}"),
            }
            assert_eq!(
                me.streams.get(&conversation_id).map(|s| s.event_cursor),
                Some(5),
                "restore must keep the merged cursor"
            );
        });
    });
}

#[test]
fn restored_child_without_children_opens_self_run_id_stream() {
    // A restored child conversation with no children of its own stays on the
    // explicit self run-id stream even when the owner-ancestor flag is on.
    App::test((), |mut app| async move {
        let _owner_guard = FeatureFlag::OwnerOrchestrationAncestorStreamer.override_enabled(true);

        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));
        let own_run_id = "550e8400-e29b-41d4-a716-446655440505";
        let mut conversation = AIConversation::new(false, false);
        conversation.set_run_id(own_run_id.to_string());
        conversation.set_parent_agent_id("550e8400-e29b-41d4-a716-4466554405fe".to_string());
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

        let ai_client: Arc<dyn AIClient> = Arc::new(MockAIClient::new());
        let server_api = ServerApiProvider::new_for_test().get();
        let poller = app.add_singleton_model(|ctx| {
            OrchestrationEventStreamer::new_with_clients_for_test(ai_client, server_api, ctx)
        });

        let consumer_id = warpui::EntityId::new();
        poller.update(&mut app, |me, _| {
            let stream = me.streams.entry(conversation_id).or_default();
            stream.watched_run_ids.insert(own_run_id.to_string());
            stream.consumers.insert(consumer_id);
        });

        let task_id: crate::ai::ambient_agents::AmbientAgentTaskId =
            "550e8400-e29b-41d4-a716-446655440000".parse().unwrap();
        poller.update(&mut app, |me, ctx| {
            me.finish_restore_fetch(
                conversation_id,
                task_id,
                /* sqlite_cursor */ 0,
                Ok(make_ambient_task_with_children(vec![])),
                ctx,
            );
        });

        poller.read(&app, |me, _| match connected_filter(me, conversation_id) {
            Some(AgentEventFilter::RunIds(run_ids)) => {
                assert_eq!(run_ids, vec![own_run_id.to_string()]);
            }
            other => panic!("expected RunIds([self]) filter, got {other:?}"),
        });
    });
}
