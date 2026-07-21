use std::collections::HashMap;

use ai::api_keys::ApiKeyManager;
use warp_core::features::FeatureFlag;
use warp_multi_agent_api as api;
use warpui::{App, SingletonEntity};

use super::{
    AIConversation, AIConversationAutoexecuteMode, AIConversationId, ConversationStatus,
    ConversationUsageTotals, RecordingSpanStatus, RestoreConversationError,
    artifact_from_fork_proto, footer_model_token_usage,
};
use crate::ai::artifacts::Artifact;
use crate::ai::llms::LLMPreferences;
use crate::auth::AuthStateProvider;
use crate::auth::auth_manager::AuthManager;
use crate::network::NetworkStatus;
use crate::persistence::model::AgentConversationData;
use crate::server::server_api::ServerApiProvider;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::workspaces::user_workspaces::UserWorkspaces;

fn restored_conversation(conversation_data: Option<AgentConversationData>) -> AIConversation {
    AIConversation::new_restored(
        AIConversationId::new(),
        vec![api::Task {
            id: "root-task".to_string(),
            messages: vec![],
            dependencies: None,
            description: String::new(),
            summary: String::new(),
            server_data: String::new(),
        }],
        conversation_data,
    )
    .unwrap()
}

fn restored_conversation_with_root_description(description: &str) -> AIConversation {
    AIConversation::new_restored(
        AIConversationId::new(),
        vec![api::Task {
            id: "root-task".to_string(),
            messages: vec![],
            dependencies: None,
            description: description.to_string(),
            summary: String::new(),
            server_data: String::new(),
        }],
        None,
    )
    .unwrap()
}

fn user_query_message(id: &str, request_id: &str, query: &str) -> api::Message {
    api::Message {
        fetched_memories: vec![],
        id: id.to_string(),
        task_id: "root-task".to_string(),
        server_message_data: String::new(),
        citations: vec![],
        message: Some(api::message::Message::UserQuery(api::message::UserQuery {
            query: query.to_string(),
            context: None,
            referenced_attachments: HashMap::new(),
            mode: None,
            intended_agent: Default::default(),
        })),
        request_id: request_id.to_string(),
        timestamp: None,
    }
}

fn tool_call_message(
    id: &str,
    request_id: &str,
    tool_call_id: &str,
    tool: api::message::tool_call::Tool,
) -> api::Message {
    api::Message {
        fetched_memories: vec![],
        id: id.to_string(),
        task_id: "root-task".to_string(),
        server_message_data: String::new(),
        citations: vec![],
        message: Some(api::message::Message::ToolCall(api::message::ToolCall {
            tool_call_id: tool_call_id.to_string(),
            tool: Some(tool),
        })),
        request_id: request_id.to_string(),
        timestamp: None,
    }
}

fn tool_call_result_message(
    id: &str,
    request_id: &str,
    tool_call_id: &str,
    result: api::message::tool_call_result::Result,
) -> api::Message {
    api::Message {
        fetched_memories: vec![],
        id: id.to_string(),
        task_id: "root-task".to_string(),
        server_message_data: String::new(),
        citations: vec![],
        message: Some(api::message::Message::ToolCallResult(
            api::message::ToolCallResult {
                tool_call_id: tool_call_id.to_string(),
                context: None,
                result: Some(result),
            },
        )),
        request_id: request_id.to_string(),
        timestamp: None,
    }
}

fn start_recording_tool_call() -> api::message::tool_call::Tool {
    api::message::tool_call::Tool::StartRecording(api::message::tool_call::StartRecording {
        frame_rate: 15,
        limits: None,
        summary: String::new(),
        playback_speed_multiplier: 0,
        target: None,
        description: String::new(),
    })
}

fn start_recording_success_result(recording_id: &str) -> api::message::tool_call_result::Result {
    api::message::tool_call_result::Result::StartRecording(api::StartRecordingResult {
        result: Some(api::start_recording_result::Result::Success(
            api::start_recording_result::Success {
                recording_id: recording_id.to_string(),
                started_at: None,
                settings: Some(api::start_recording_result::CaptureSettings {
                    width_px: 1280,
                    height_px: 720,
                }),
            },
        )),
    })
}

fn start_recording_error_result(message: &str) -> api::message::tool_call_result::Result {
    api::message::tool_call_result::Result::StartRecording(api::StartRecordingResult {
        result: Some(api::start_recording_result::Result::Error(
            api::start_recording_result::Error {
                message: message.to_string(),
            },
        )),
    })
}

fn use_computer_tool_call(summary: &str) -> api::message::tool_call::Tool {
    api::message::tool_call::Tool::UseComputer(api::message::tool_call::UseComputer {
        actions: vec![],
        post_actions_screenshot_params: None,
        action_summary: summary.to_string(),
    })
}

fn stop_recording_tool_call(recording_id: &str) -> api::message::tool_call::Tool {
    api::message::tool_call::Tool::StopRecording(api::message::tool_call::StopRecording {
        recording_id: recording_id.to_string(),
    })
}

fn stop_recording_success_result(artifact_uid: &str) -> api::message::tool_call_result::Result {
    api::message::tool_call_result::Result::StopRecording(api::StopRecordingResult {
        result: Some(api::stop_recording_result::Result::Success(
            api::stop_recording_result::Success {
                artifact_uid: artifact_uid.to_string(),
                duration: Some(prost_types::Duration {
                    seconds: 2,
                    nanos: 0,
                }),
                width_px: 1280,
                height_px: 720,
                size_bytes: 42,
                completion_status: api::stop_recording_result::CompletionStatus::Complete as i32,
                termination_reason: "Stopped by agent".to_string(),
            },
        )),
    })
}

fn stop_recording_error_result(message: &str) -> api::message::tool_call_result::Result {
    api::message::tool_call_result::Result::StopRecording(api::StopRecordingResult {
        result: Some(api::stop_recording_result::Result::Error(
            api::stop_recording_result::Error {
                message: message.to_string(),
            },
        )),
    })
}
fn restored_conversation_with_messages(messages: Vec<api::Message>) -> AIConversation {
    AIConversation::new_restored(
        AIConversationId::new(),
        vec![api::Task {
            id: "root-task".to_string(),
            messages,
            dependencies: None,
            description: String::new(),
            summary: String::new(),
            server_data: String::new(),
        }],
        None,
    )
    .unwrap()
}

fn agent_output_message(id: &str, request_id: &str) -> api::Message {
    api::Message {
        fetched_memories: vec![],
        id: id.to_string(),
        task_id: "root-task".to_string(),
        server_message_data: String::new(),
        citations: vec![],
        message: Some(api::message::Message::AgentOutput(
            api::message::AgentOutput {
                text: "Done".to_string(),
            },
        )),
        request_id: request_id.to_string(),
        timestamp: None,
    }
}

fn restored_conversation_with_queries(queries: &[&str]) -> AIConversation {
    let messages = queries
        .iter()
        .enumerate()
        .flat_map(|(index, query)| {
            let request_id = format!("request-{index}");
            [
                user_query_message(&format!("user-{index}"), &request_id, query),
                agent_output_message(&format!("agent-{index}"), &request_id),
            ]
        })
        .collect();

    AIConversation::new_restored(
        AIConversationId::new(),
        vec![api::Task {
            id: "root-task".to_string(),
            messages,
            dependencies: None,
            description: String::new(),
            summary: String::new(),
            server_data: String::new(),
        }],
        None,
    )
    .unwrap()
}

fn initialize_custom_endpoint_usage_test_app(app: &mut App) {
    initialize_settings_for_tests(app);
    app.add_singleton_model(|_| ServerApiProvider::new_for_test());
    app.add_singleton_model(|_| NetworkStatus::new());
    app.add_singleton_model(UserWorkspaces::default_mock);
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(AuthManager::new_for_test);
}

#[allow(deprecated)]
fn custom_endpoint_usage_metadata(
    config_key: &str,
    total_tokens: u32,
) -> api::response_event::stream_finished::ConversationUsageMetadata {
    let category = "primary_agent".to_string();
    api::response_event::stream_finished::ConversationUsageMetadata {
        context_window_usage: 0.0,
        credits_spent: 0.0,
        platform_credits_spent: 0.0,
        summarized: false,
        token_usage: vec![],
        tool_usage_metadata: None,
        total_input_tokens: 0,
        warp_token_usage: HashMap::new(),
        byok_token_usage: HashMap::new(),
        context_window_segments: Vec::new(),
        custom_endpoint_token_usage: HashMap::from([(
            config_key.to_string(),
            api::response_event::stream_finished::ModelTokenUsage {
                model_id: config_key.to_string(),
                total_tokens,
                token_usage_by_category: HashMap::from([(category, total_tokens)]),
            },
        )]),
    }
}

#[test]
fn latest_user_query_returns_latest_non_empty_user_query() {
    let conversation =
        restored_conversation_with_queries(&["write unit tests", "fix the failing test"]);

    assert_eq!(
        conversation.latest_user_query(),
        Some("fix the failing test".to_string())
    );
}

#[test]
fn latest_user_query_trims_and_skips_empty_queries() {
    let conversation = restored_conversation_with_queries(&["  write unit tests  ", "  "]);

    assert_eq!(
        conversation.latest_user_query(),
        Some("write unit tests".to_string())
    );
}

#[test]
fn title_uses_root_task_description() {
    let conversation = restored_conversation_with_root_description("Root task title");

    assert_eq!(conversation.title().as_deref(), Some("Root task title"));
}

#[test]
fn title_falls_back_to_initial_query_when_root_description_is_empty() {
    let conversation = restored_conversation_with_queries(&["Initial query"]);

    assert_eq!(conversation.title().as_deref(), Some("Initial query"));
}

#[test]
fn recording_span_closes_on_matching_stop_result() {
    let conversation = restored_conversation_with_messages(vec![
        tool_call_message("start-call", "req-1", "start", start_recording_tool_call()),
        tool_call_message(
            "use-call",
            "req-1",
            "use",
            use_computer_tool_call("Click button"),
        ),
        tool_call_message(
            "stop-call",
            "req-1",
            "stop",
            stop_recording_tool_call("rec-1"),
        ),
        tool_call_result_message(
            "start-result",
            "req-2",
            "start",
            start_recording_success_result("rec-1"),
        ),
        tool_call_result_message(
            "stop-result",
            "req-2",
            "stop",
            stop_recording_success_result("artifact-1"),
        ),
    ]);

    let span = conversation
        .recording_span_for_action(&"use".to_string().into(), None)
        .expect("use action should be inside a recording span");

    assert_eq!(span.recording_id, "rec-1");
    assert_eq!(span.status, RecordingSpanStatus::Captured);
}

#[test]
fn recording_span_stays_open_without_stop_result() {
    let conversation = restored_conversation_with_messages(vec![
        tool_call_message("start-call", "req-1", "start", start_recording_tool_call()),
        tool_call_message(
            "use-call",
            "req-1",
            "use",
            use_computer_tool_call("Click button"),
        ),
        tool_call_result_message(
            "start-result",
            "req-2",
            "start",
            start_recording_success_result("rec-1"),
        ),
    ]);

    let span = conversation
        .recording_span_for_action(&"use".to_string().into(), None)
        .expect("use action should be inside an open recording span");

    assert_eq!(span.recording_id, "rec-1");
    assert_eq!(span.status, RecordingSpanStatus::Active);
}

#[test]
fn recording_span_ignores_failed_start() {
    let conversation = restored_conversation_with_messages(vec![
        tool_call_message("start-call", "req-1", "start", start_recording_tool_call()),
        tool_call_message(
            "use-call",
            "req-1",
            "use",
            use_computer_tool_call("Click button"),
        ),
        tool_call_result_message(
            "start-result",
            "req-2",
            "start",
            start_recording_error_result("unsupported"),
        ),
    ]);

    assert!(
        conversation
            .recording_span_for_action(&"use".to_string().into(), None)
            .is_none()
    );
}

#[test]
fn recording_span_ignores_mismatched_stop_id() {
    let conversation = restored_conversation_with_messages(vec![
        tool_call_message("start-call", "req-1", "start", start_recording_tool_call()),
        tool_call_message(
            "use-call",
            "req-1",
            "use",
            use_computer_tool_call("Click button"),
        ),
        tool_call_message(
            "stop-call",
            "req-1",
            "stop",
            stop_recording_tool_call("other"),
        ),
        tool_call_result_message(
            "start-result",
            "req-2",
            "start",
            start_recording_success_result("rec-1"),
        ),
        tool_call_result_message(
            "stop-result",
            "req-2",
            "stop",
            stop_recording_success_result("artifact-1"),
        ),
    ]);

    let span = conversation
        .recording_span_for_action(&"use".to_string().into(), None)
        .expect("mismatched stop should not close the span");

    assert_eq!(span.recording_id, "rec-1");
    assert_eq!(span.status, RecordingSpanStatus::Active);
}

#[test]
fn recording_span_clears_when_stop_errors() {
    let conversation = restored_conversation_with_messages(vec![
        tool_call_message("start-call", "req-1", "start", start_recording_tool_call()),
        tool_call_message(
            "use-call",
            "req-1",
            "use",
            use_computer_tool_call("Click button"),
        ),
        tool_call_message(
            "stop-call",
            "req-1",
            "stop",
            stop_recording_tool_call("rec-1"),
        ),
        tool_call_result_message(
            "start-result",
            "req-2",
            "start",
            start_recording_success_result("rec-1"),
        ),
        tool_call_result_message(
            "stop-result",
            "req-2",
            "stop",
            stop_recording_error_result("upload failed"),
        ),
    ]);

    assert!(
        conversation
            .recording_span_for_action(&"use".to_string().into(), None)
            .is_none()
    );
}

#[test]
fn reassign_exchange_ids_keeps_exchange_lookup_consistent() {
    let mut conversation = restored_conversation_with_queries(&["one", "two"]);

    let old_ids: Vec<_> = conversation.all_exchanges().iter().map(|e| e.id).collect();
    assert!(!old_ids.is_empty());

    // Pre-condition: every original id resolves via the exchange-id index.
    for id in &old_ids {
        assert!(conversation.exchange_with_id(*id).is_some());
    }

    conversation.reassign_exchange_ids();

    // Reassigning regenerates ids without changing the exchange count, so
    // `modify_task` does not rebuild the index; correctness relies on the
    // explicit `rebuild_exchange_index()` call. The stale ids must be gone.
    for id in &old_ids {
        assert!(conversation.exchange_with_id(*id).is_none());
    }

    // Every current id resolves via the rebuilt index.
    let new_ids: Vec<_> = conversation.all_exchanges().iter().map(|e| e.id).collect();
    assert_eq!(new_ids.len(), old_ids.len());
    for id in &new_ids {
        assert!(conversation.exchange_with_id(*id).is_some());
    }
}

#[test]
fn restored_conversation_defaults_autoexecute_override_when_not_persisted() {
    let _flag = FeatureFlag::RememberFastForwardState.override_enabled(true);
    let conversation_data: AgentConversationData =
        serde_json::from_str(r#"{"server_conversation_token":null}"#).unwrap();

    let conversation = restored_conversation(Some(conversation_data));

    assert_eq!(
        conversation.autoexecute_override(),
        AIConversationAutoexecuteMode::RespectUserSettings
    );
}

#[test]
fn restored_conversation_uses_persisted_last_event_sequence() {
    let conversation_data: AgentConversationData =
        serde_json::from_str(r#"{"server_conversation_token":null,"last_event_sequence":42}"#)
            .unwrap();

    let conversation = restored_conversation(Some(conversation_data));

    assert_eq!(conversation.last_event_sequence(), Some(42));
}

#[test]
fn restored_conversation_uses_persisted_remote_child_marker() {
    let conversation_data: AgentConversationData =
        serde_json::from_str(r#"{"server_conversation_token":null,"is_remote_child":true}"#)
            .unwrap();

    let conversation = restored_conversation(Some(conversation_data));

    assert!(conversation.is_remote_child());
}

#[test]
fn child_conversation_detection_uses_parent_agent_id() {
    let conversation_data: AgentConversationData = serde_json::from_str(
        r#"{"server_conversation_token":null,"parent_agent_id":"parent-run-id"}"#,
    )
    .unwrap();

    let conversation = restored_conversation(Some(conversation_data));

    assert!(conversation.is_child_agent_conversation());
    assert_eq!(conversation.parent_conversation_id(), None);
}

/// When the persisted task list is empty (e.g. a child conversation persisted
/// before any server response), restoring via `new_restored_synthesizing_on_empty`
/// must produce a fresh in-progress optimistic root, mirroring
/// `AIConversation::new()`.
#[test]
fn restored_conversation_with_empty_task_list_creates_in_progress_optimistic_root() {
    let conversation =
        AIConversation::new_restored_synthesizing_on_empty(AIConversationId::new(), vec![], None)
            .expect("empty task list must synthesize an optimistic root");

    let root_task = conversation
        .get_root_task()
        .expect("synthesized root task should exist");
    assert!(root_task.is_root_task());
    assert!(
        root_task.source().is_none(),
        "synthesized root is optimistic and has no api::Task source"
    );
    assert!(
        !root_task.id().to_string().is_empty(),
        "synthesized optimistic root must have a non-empty UUID id"
    );
    assert_eq!(conversation.status(), &ConversationStatus::InProgress);
    assert!(conversation.status_error_message().is_none());
}

#[test]
fn update_cost_and_usage_resolves_custom_endpoint_alias_for_footer_usage() {
    App::test((), |mut app| async move {
        initialize_custom_endpoint_usage_test_app(&mut app);
        ApiKeyManager::handle(&app).update(&mut app, |manager, ctx| {
            manager.add_custom_endpoint(
                "Endpoint".to_string(),
                "https://custom.example".to_string(),
                "key".to_string(),
                vec![(
                    "raw-model".to_string(),
                    Some("Friendly alias".to_string()),
                    Some("config-key".to_string()),
                )],
                ctx,
            );
        });
        app.add_singleton_model(LLMPreferences::new);

        let mut conversation = AIConversation::new(false, false);
        app.read(|ctx| {
            conversation
                .update_cost_and_usage_for_request(
                    None,
                    vec![],
                    Some(custom_endpoint_usage_metadata("config-key", 6)),
                    false,
                    ctx,
                )
                .expect("custom endpoint usage should update");
        });

        let usage = conversation
            .token_usage()
            .iter()
            .find(|usage| usage.model_id == "Friendly alias")
            .expect("custom endpoint alias should resolve into footer usage");
        assert_eq!(usage.custom_endpoint_tokens, 6);
        assert_eq!(usage.byok_tokens, 0);
        assert_eq!(
            usage
                .custom_endpoint_token_usage_by_category
                .get("primary_agent"),
            Some(&6)
        );
    });
}

#[test]
fn update_cost_and_usage_uses_fallback_label_for_unknown_custom_endpoint() {
    App::test((), |mut app| async move {
        initialize_custom_endpoint_usage_test_app(&mut app);
        app.add_singleton_model(LLMPreferences::new);

        let mut conversation = AIConversation::new(false, false);
        app.read(|ctx| {
            conversation
                .update_cost_and_usage_for_request(
                    None,
                    vec![],
                    Some(custom_endpoint_usage_metadata("missing-config-key", 9)),
                    false,
                    ctx,
                )
                .expect("fallback custom endpoint usage should update");
        });

        let usage = conversation
            .token_usage()
            .iter()
            .find(|usage| usage.model_id == "Custom endpoint")
            .expect("unknown custom endpoint usage should use the fallback label");
        assert_eq!(usage.custom_endpoint_tokens, 9);
        assert_eq!(usage.byok_tokens, 0);
        assert_eq!(
            usage
                .custom_endpoint_token_usage_by_category
                .get("primary_agent"),
            Some(&9)
        );
    });
}

fn stream_token_usage(
    model_id: &str,
    total_input: u32,
    output: u32,
    cost_in_cents: f32,
) -> api::response_event::stream_finished::TokenUsage {
    api::response_event::stream_finished::TokenUsage {
        model_id: model_id.to_string(),
        total_input,
        output,
        input_cache_read: 0,
        input_cache_write: 0,
        cost_in_cents,
    }
}

#[allow(deprecated)]
fn credits_usage_metadata(
    credits_spent: f32,
    platform_credits_spent: f32,
) -> api::response_event::stream_finished::ConversationUsageMetadata {
    api::response_event::stream_finished::ConversationUsageMetadata {
        context_window_usage: 0.0,
        credits_spent,
        platform_credits_spent,
        summarized: false,
        token_usage: vec![],
        tool_usage_metadata: None,
        total_input_tokens: 0,
        warp_token_usage: HashMap::new(),
        byok_token_usage: HashMap::new(),
        context_window_segments: Vec::new(),
        custom_endpoint_token_usage: HashMap::new(),
    }
}

#[test]
fn usage_totals_reads_gui_credits_and_accumulates_provider_cost() {
    App::test((), |mut app| async move {
        initialize_custom_endpoint_usage_test_app(&mut app);
        app.add_singleton_model(LLMPreferences::new);

        let mut conversation = AIConversation::new(false, false);
        assert_eq!(
            conversation.usage_totals(),
            ConversationUsageTotals::default()
        );

        app.read(|ctx| {
            conversation
                .update_cost_and_usage_for_request(
                    None,
                    vec![stream_token_usage("model-a", 100, 20, 1.5)],
                    Some(credits_usage_metadata(2.0, 0.5)),
                    false,
                    ctx,
                )
                .expect("usage should update");
            // The server's usage metadata is cumulative per conversation: the
            // newest snapshot replaces the previous credits rather than
            // summing, while provider cost accumulates per request.
            conversation
                .update_cost_and_usage_for_request(
                    None,
                    vec![stream_token_usage("model-a", 50, 10, 1.2)],
                    Some(credits_usage_metadata(3.0, 0.5)),
                    false,
                    ctx,
                )
                .expect("usage should update");
        });

        let totals = conversation.usage_totals();
        assert!((totals.credits_spent - 3.5).abs() < 1e-6);
        assert!((totals.cost_in_cents - 2.7).abs() < 1e-6);
    });
}

#[allow(deprecated)]
#[test]
fn footer_model_token_usage_keeps_custom_endpoint_usage_distinct_from_same_labeled_models() {
    App::test((), |mut app| async move {
        initialize_custom_endpoint_usage_test_app(&mut app);
        ApiKeyManager::handle(&app).update(&mut app, |manager, ctx| {
            manager.add_custom_endpoint(
                "Endpoint".to_string(),
                "https://custom.example".to_string(),
                "key".to_string(),
                vec![(
                    "raw-model".to_string(),
                    Some("Resolved custom".to_string()),
                    Some("config-key".to_string()),
                )],
                ctx,
            );
        });
        app.add_singleton_model(LLMPreferences::new);

        let category = "primary_agent".to_string();
        let usage_metadata = api::response_event::stream_finished::ConversationUsageMetadata {
            context_window_usage: 0.0,
            credits_spent: 0.0,
            platform_credits_spent: 0.0,
            summarized: false,
            #[allow(deprecated)]
            token_usage: vec![],
            tool_usage_metadata: None,
            total_input_tokens: 0,
            warp_token_usage: HashMap::new(),
            byok_token_usage: HashMap::from([(
                "Resolved custom".to_string(),
                api::response_event::stream_finished::ModelTokenUsage {
                    model_id: "Resolved custom".to_string(),
                    total_tokens: 4,
                    token_usage_by_category: HashMap::from([(category.clone(), 4)]),
                },
            )]),
            custom_endpoint_token_usage: HashMap::from([(
                "config-key".to_string(),
                api::response_event::stream_finished::ModelTokenUsage {
                    model_id: "config-key".to_string(),
                    total_tokens: 6,
                    token_usage_by_category: HashMap::from([(category.clone(), 6)]),
                },
            )]),
            context_window_segments: Vec::new(),
        };

        let model_usage =
            app.read(|ctx| footer_model_token_usage(&usage_metadata, LLMPreferences::as_ref(ctx)));
        let byok_usage = model_usage
            .iter()
            .find(|usage| usage.model_id == "Resolved custom" && usage.byok_tokens == 4)
            .expect("existing model usage should be present");
        let custom_usage = model_usage
            .iter()
            .find(|usage| usage.model_id == "Resolved custom" && usage.custom_endpoint_tokens == 6)
            .expect("custom endpoint usage should remain distinct");

        assert_eq!(model_usage.len(), 2);
        assert_eq!(
            byok_usage.byok_token_usage_by_category.get(&category),
            Some(&4)
        );
        assert_eq!(
            custom_usage
                .custom_endpoint_token_usage_by_category
                .get(&category),
            Some(&6)
        );
        assert_eq!(byok_usage.warp_tokens, 0);
        assert_eq!(custom_usage.warp_tokens, 0);
        assert_eq!(custom_usage.byok_tokens, 0);
    });
}

#[allow(deprecated)]
#[test]
fn footer_model_token_usage_preserves_unresolved_custom_endpoint_usage_with_fallback_label() {
    App::test((), |mut app| async move {
        initialize_custom_endpoint_usage_test_app(&mut app);
        app.add_singleton_model(LLMPreferences::new);

        let category = "primary_agent".to_string();
        let usage_metadata = api::response_event::stream_finished::ConversationUsageMetadata {
            context_window_usage: 0.0,
            credits_spent: 0.0,
            platform_credits_spent: 0.0,
            summarized: false,
            #[allow(deprecated)]
            token_usage: vec![],
            tool_usage_metadata: None,
            total_input_tokens: 0,
            warp_token_usage: HashMap::new(),
            byok_token_usage: HashMap::new(),
            custom_endpoint_token_usage: HashMap::from([(
                "missing-config-key".to_string(),
                api::response_event::stream_finished::ModelTokenUsage {
                    model_id: "missing-config-key".to_string(),
                    total_tokens: 9,
                    token_usage_by_category: HashMap::from([(category.clone(), 9)]),
                },
            )]),
            context_window_segments: Vec::new(),
        };

        let model_usage =
            app.read(|ctx| footer_model_token_usage(&usage_metadata, LLMPreferences::as_ref(ctx)));
        let custom_usage = model_usage
            .iter()
            .find(|usage| usage.model_id == "Custom endpoint")
            .expect("fallback custom endpoint usage should be present");

        assert_eq!(model_usage.len(), 1);
        assert_eq!(custom_usage.custom_endpoint_tokens, 9);
        assert_eq!(custom_usage.byok_tokens, 0);
        assert_eq!(
            custom_usage
                .custom_endpoint_token_usage_by_category
                .get(&category),
            Some(&9)
        );
        assert_eq!(custom_usage.warp_tokens, 0);
    });
}

/// The legacy `AgentConversationData.root_task_is_optimistic` flag must be
/// ignored on restore. A non-empty task list always produces a real
/// server-backed root regardless of whether the flag is set.
#[test]
fn restored_conversation_ignores_legacy_root_task_is_optimistic_flag_with_non_empty_tasks() {
    let conversation_data: AgentConversationData = serde_json::from_str(
        r#"{"server_conversation_token":null,"root_task_is_optimistic":true}"#,
    )
    .unwrap();

    let conversation = restored_conversation(Some(conversation_data));
    let root_task = conversation
        .get_root_task()
        .expect("root task should exist");

    assert_eq!(root_task.id().to_string(), "root-task");
    assert!(root_task.is_root_task());
    assert!(
        root_task.source().is_some(),
        "with a real task list, the legacy optimistic flag must be ignored",
    );
}

/// The legacy `root_task_is_optimistic` flag is ignored when restoring an
/// empty task list via `new_restored_synthesizing_on_empty`.
#[test]
fn restored_conversation_ignores_legacy_root_task_is_optimistic_flag_with_empty_tasks() {
    let conversation_data: AgentConversationData = serde_json::from_str(
        r#"{"server_conversation_token":null,"root_task_is_optimistic":true}"#,
    )
    .unwrap();

    let conversation = AIConversation::new_restored_synthesizing_on_empty(
        AIConversationId::new(),
        vec![],
        Some(conversation_data),
    )
    .expect("empty task list must synthesize an optimistic root regardless of legacy flag");

    let root_task = conversation
        .get_root_task()
        .expect("synthesized root task should exist");
    assert!(root_task.is_root_task());
    assert!(root_task.source().is_none());
    assert_eq!(conversation.status(), &ConversationStatus::InProgress);
}

/// Strict `new_restored` returns `NoRootTask` for an empty task list.
#[test]
fn new_restored_with_empty_task_list_returns_no_root_task_error() {
    let result = AIConversation::new_restored(AIConversationId::new(), vec![], None);
    assert!(
        matches!(result, Err(RestoreConversationError::NoRootTask)),
        "empty task list via strict new_restored must return NoRootTask; got {result:?}",
    );
}

/// When multiple parentless tasks exist (e.g. a legacy orphan optimistic
/// stub alongside the real server root), `new_restored` must prefer the
/// candidate whose `messages` is non-empty. Each ordering runs in a loop to
/// surface any nondeterminism in candidate selection.
#[test]
fn test_new_restored_prefers_parentless_task_with_messages_over_empty_stub() {
    let stub = api::Task {
        id: "optimistic-stub-uuid".to_string(),
        messages: vec![],
        dependencies: None,
        description: String::new(),
        summary: String::new(),
        server_data: String::new(),
    };
    let real = api::Task {
        id: "server-root-id".to_string(),
        messages: vec![user_query_message("user-msg", "request-1", "real query")],
        dependencies: None,
        description: String::new(),
        summary: String::new(),
        server_data: String::new(),
    };

    // Stub appears first in the vec.
    for _ in 0..50 {
        let conversation = AIConversation::new_restored(
            AIConversationId::new(),
            vec![stub.clone(), real.clone()],
            None,
        )
        .expect("restore with stub + real parentless tasks must succeed");
        let root_task = conversation
            .get_root_task()
            .expect("restored conversation must have a root task");
        assert_eq!(
            root_task.id().to_string(),
            "server-root-id",
            "expected the real (non-empty) parentless task to win when stub is first",
        );
        let source = root_task
            .source()
            .expect("chosen root must have api::Task source");
        assert!(
            !source.messages.is_empty(),
            "chosen root must have non-empty messages",
        );
    }

    // Real appears first in the vec.
    for _ in 0..50 {
        let conversation = AIConversation::new_restored(
            AIConversationId::new(),
            vec![real.clone(), stub.clone()],
            None,
        )
        .expect("restore with real + stub parentless tasks must succeed");
        let root_task = conversation
            .get_root_task()
            .expect("restored conversation must have a root task");
        assert_eq!(
            root_task.id().to_string(),
            "server-root-id",
            "expected the real (non-empty) parentless task to win when real is first",
        );
        let source = root_task
            .source()
            .expect("chosen root must have api::Task source");
        assert!(
            !source.messages.is_empty(),
            "chosen root must have non-empty messages",
        );
    }
}

#[test]
fn cli_agent_transcript_vehicle_is_excluded_from_navigation() {
    let conversation = AIConversation::new(false, true);

    assert!(conversation.should_exclude_from_navigation());
}

#[test]
fn restored_conversation_defaults_unknown_persisted_autoexecute_override() {
    let _flag = FeatureFlag::RememberFastForwardState.override_enabled(true);
    let conversation_data: AgentConversationData = serde_json::from_str(
        r#"{"server_conversation_token":null,"autoexecute_override":"UnexpectedValue"}"#,
    )
    .unwrap();

    let conversation = restored_conversation(Some(conversation_data));

    assert_eq!(
        conversation.autoexecute_override(),
        AIConversationAutoexecuteMode::RespectUserSettings
    );
}

#[test]
fn restored_conversation_uses_persisted_autoexecute_override_when_enabled() {
    let _flag = FeatureFlag::RememberFastForwardState.override_enabled(true);
    let conversation_data: AgentConversationData = serde_json::from_str(
        r#"{"server_conversation_token":null,"autoexecute_override":"RunToCompletion"}"#,
    )
    .unwrap();

    let conversation = restored_conversation(Some(conversation_data));

    assert_eq!(
        conversation.autoexecute_override(),
        AIConversationAutoexecuteMode::RunToCompletion
    );
}

#[test]
fn restored_conversation_ignores_persisted_autoexecute_override_when_disabled() {
    let _flag = FeatureFlag::RememberFastForwardState.override_enabled(false);
    let conversation_data: AgentConversationData = serde_json::from_str(
        r#"{"server_conversation_token":null,"autoexecute_override":"RunToCompletion"}"#,
    )
    .unwrap();

    let conversation = restored_conversation(Some(conversation_data));

    assert_eq!(
        conversation.autoexecute_override(),
        AIConversationAutoexecuteMode::RespectUserSettings
    );
}

#[test]
fn fork_artifacts_adds_file_artifacts_to_conversation() {
    let proto_artifact = api::message::artifact_event::ConversationArtifact {
        artifact: Some(
            api::message::artifact_event::conversation_artifact::Artifact::File(
                api::message::artifact_event::FileArtifact {
                    artifact_uid: "artifact-file-1".to_string(),
                    filepath: "outputs/report.txt".to_string(),
                    mime_type: "text/plain".to_string(),
                    size_bytes: 42,
                    description: "Daily summary".to_string(),
                },
            ),
        ),
    };

    assert_eq!(
        artifact_from_fork_proto(&proto_artifact),
        Some(Artifact::File {
            artifact_uid: "artifact-file-1".to_string(),
            filepath: "outputs/report.txt".to_string(),
            filename: "report.txt".to_string(),
            mime_type: "text/plain".to_string(),
            description: Some("Daily summary".to_string()),
            size_bytes: Some(42),
        })
    );
}

#[test]
fn waiting_for_events_display_label_is_waiting() {
    assert_eq!(
        format!("{}", ConversationStatus::WaitingForEvents),
        "Waiting"
    );
}

/// `is_done` returns true only for `Success | Error | Cancelled`;
/// `WaitingForEvents` and `Blocked` are not done because the run can still
/// resume on its own.
#[test]
fn is_done_only_includes_success_error_cancelled() {
    assert!(ConversationStatus::Success.is_done());
    assert!(ConversationStatus::Error.is_done());
    assert!(ConversationStatus::Cancelled.is_done());

    assert!(!ConversationStatus::InProgress.is_done());
    assert!(
        !ConversationStatus::Blocked {
            blocked_action: "approve".to_string()
        }
        .is_done()
    );
    assert!(!ConversationStatus::WaitingForEvents.is_done());
}

/// `is_waiting_for_events` is true only for the new variant.
#[test]
fn is_waiting_for_events_returns_true_only_for_waiting_for_events_variant() {
    assert!(ConversationStatus::WaitingForEvents.is_waiting_for_events());

    assert!(!ConversationStatus::InProgress.is_waiting_for_events());
    assert!(!ConversationStatus::Success.is_waiting_for_events());
    assert!(!ConversationStatus::Error.is_waiting_for_events());
    assert!(!ConversationStatus::Cancelled.is_waiting_for_events());
    assert!(
        !ConversationStatus::Blocked {
            blocked_action: "approve".to_string()
        }
        .is_waiting_for_events()
    );
}

/// A conversation that was yielded via `wait_for_events` at shutdown
/// restores as whatever `derive_status_from_root_task` returns (Success
/// for a cleanly-streamed last exchange). The unresolved tool call stays
/// in the transcript as an orphan; the next outbound request triggers
/// the server's existing supersede mechanism to synthesize the matching
/// `Cancel`. The waiting state itself is not durable across restart.
#[test]
fn restored_conversation_does_not_re_enter_waiting_for_events() {
    let conversation_data: AgentConversationData =
        serde_json::from_str(r#"{"server_conversation_token":null}"#).unwrap();

    let conversation = restored_conversation(Some(conversation_data));

    assert_eq!(conversation.status(), &ConversationStatus::Success);
}

fn fetched_memory(
    memory_id: &str,
    content: &str,
    memory_store_id: &str,
    source: Option<api::message::fetched_memory::Source>,
) -> api::message::FetchedMemory {
    api::message::FetchedMemory {
        memory_id: memory_id.to_string(),
        content: content.to_string(),
        memory_store_id: memory_store_id.to_string(),
        source,
    }
}

fn conversation_source(conversation_id: &str) -> Option<api::message::fetched_memory::Source> {
    Some(api::message::fetched_memory::Source::Conversation(
        api::message::fetched_memory::Conversation {
            conversation_id: conversation_id.to_string(),
        },
    ))
}

fn restored_conversation_with_memories_per_query(
    memories_per_query: Vec<Vec<api::message::FetchedMemory>>,
) -> AIConversation {
    let messages = memories_per_query
        .into_iter()
        .enumerate()
        .flat_map(|(index, memories)| {
            let request_id = format!("request-{index}");
            let query = api::Message {
                fetched_memories: memories,
                ..user_query_message(&format!("user-{index}"), &request_id, "query")
            };
            [
                query,
                agent_output_message(&format!("agent-{index}"), &request_id),
            ]
        })
        .collect();

    AIConversation::new_restored(
        AIConversationId::new(),
        vec![api::Task {
            id: "root-task".to_string(),
            messages,
            ..Default::default()
        }],
        None,
    )
    .unwrap()
}

#[test]
fn fetched_memories_is_empty_when_no_message_has_memories() {
    let conversation = restored_conversation_with_memories_per_query(vec![vec![]]);

    assert_eq!(conversation.fetched_memories(), vec![]);
}

#[test]
fn fetched_memories_preserves_order_across_and_within_messages() {
    let conversation = restored_conversation_with_memories_per_query(vec![
        vec![
            fetched_memory("m1", "first", "store-1", None),
            fetched_memory("m2", "second", "store-1", None),
        ],
        vec![fetched_memory("m3", "third", "store-2", None)],
    ]);

    let ids: Vec<String> = conversation
        .fetched_memories()
        .into_iter()
        .map(|memory| memory.memory_id)
        .collect();
    assert_eq!(ids, vec!["m1", "m2", "m3"]);
}

#[test]
fn fetched_memories_dedupes_keeping_first_position_and_latest_data() {
    let conversation = restored_conversation_with_memories_per_query(vec![
        vec![
            fetched_memory("m1", "old content", "store-1", None),
            fetched_memory("m2", "other", "store-1", None),
        ],
        vec![
            fetched_memory(
                "m1",
                "new content",
                "store-1",
                conversation_source("conversation-1"),
            ),
            fetched_memory("m1", "same memory id different store", "store-2", None),
        ],
    ]);

    let memories = conversation.fetched_memories();
    assert_eq!(
        memories,
        vec![
            fetched_memory(
                "m1",
                "new content",
                "store-1",
                conversation_source("conversation-1"),
            ),
            fetched_memory("m2", "other", "store-1", None),
            fetched_memory("m1", "same memory id different store", "store-2", None),
        ]
    );
}
