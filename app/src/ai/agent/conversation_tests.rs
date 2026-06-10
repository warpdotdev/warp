use std::collections::HashMap;

use ai::api_keys::ApiKeyManager;
use warp_core::features::FeatureFlag;
use warp_multi_agent_api as api;
use warpui::{App, SingletonEntity};

use super::{
    artifact_from_fork_proto, footer_model_token_usage, AIConversation,
    AIConversationAutoexecuteMode, AIConversationId, ConversationStatus, RestoreConversationError,
};
use crate::ai::artifacts::Artifact;
use crate::ai::llms::LLMPreferences;
use crate::auth::auth_manager::AuthManager;
use crate::auth::AuthStateProvider;
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

fn user_query_message(id: &str, request_id: &str, query: &str) -> api::Message {
    api::Message {
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

fn agent_output_message(id: &str, request_id: &str) -> api::Message {
    api::Message {
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
        warp_token_usage: HashMap::new(),
        byok_token_usage: HashMap::new(),
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
    assert!(!ConversationStatus::Blocked {
        blocked_action: "approve".to_string()
    }
    .is_done());
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
    assert!(!ConversationStatus::Blocked {
        blocked_action: "approve".to_string()
    }
    .is_waiting_for_events());
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
