use std::collections::HashMap;

use warp_multi_agent_api as api;

use super::{AgentConversation, AgentConversationData, AgentConversationSummary, ModelTokenUsage};

fn parentless_task(id: &str, message_count: usize) -> api::Task {
    api::Task {
        id: id.to_string(),
        description: String::new(),
        dependencies: None,
        messages: (0..message_count)
            .map(|i| api::Message {
                fetched_memories: vec![],
                id: format!("{id}-msg-{i}"),
                task_id: id.to_string(),
                server_message_data: String::new(),
                citations: vec![],
                message: None,
                request_id: String::new(),
                timestamp: None,
            })
            .collect(),
        summary: String::new(),
        server_data: String::new(),
    }
}

fn child_task(id: &str, parent_id: &str) -> api::Task {
    api::Task {
        id: id.to_string(),
        description: String::new(),
        dependencies: Some(api::task::Dependencies {
            parent_task_id: parent_id.to_string(),
        }),
        messages: vec![],
        summary: String::new(),
        server_data: String::new(),
    }
}

fn conversation_with_tasks(tasks: Vec<api::Task>) -> AgentConversation {
    AgentConversation {
        conversation: Default::default(),
        tasks,
    }
}

/// Legacy [stub + real] root shape produced by the pre-QUALITY-774
/// optimistic-root writer bug must be considered restorable so the
/// restore-side dedupe in `AIConversation::new_restored` can pick the
/// real root.
#[test]
fn is_restorable_accepts_legacy_stub_plus_real_root_shape() {
    let conversation = conversation_with_tasks(vec![
        parentless_task("optimistic-stub-uuid", 0),
        parentless_task("server-root-id", 2),
        child_task("child-1", "server-root-id"),
    ]);
    assert!(conversation.is_restorable());
}

/// Multi-root with multiple real roots (each non-empty) is genuinely
/// ambiguous and must remain rejected — the dedupe heuristic cannot
/// disambiguate between two real roots.
#[test]
fn is_restorable_rejects_multi_root_with_multiple_real_roots() {
    let conversation = conversation_with_tasks(vec![
        parentless_task("root-a", 1),
        parentless_task("root-b", 1),
    ]);
    assert!(!conversation.is_restorable());
}

/// Multi-root where every candidate is empty has nothing to anchor
/// restore on and must remain rejected.
#[test]
fn is_restorable_rejects_multi_root_with_no_real_root() {
    let conversation = conversation_with_tasks(vec![
        parentless_task("stub-1", 0),
        parentless_task("stub-2", 0),
    ]);
    assert!(!conversation.is_restorable());
}

/// Normal happy path: a single parentless root plus well-formed child
/// tasks remains restorable.
#[test]
fn is_restorable_accepts_single_root_plus_subtasks() {
    let conversation = conversation_with_tasks(vec![
        parentless_task("root", 1),
        child_task("child-1", "root"),
        child_task("child-2", "root"),
    ]);
    assert!(conversation.is_restorable());
}

/// Empty or single-task conversations are trivially restorable.
#[test]
fn is_restorable_accepts_empty_and_single_task_conversations() {
    assert!(conversation_with_tasks(vec![]).is_restorable());
    assert!(conversation_with_tasks(vec![parentless_task("root", 0)]).is_restorable());
}

fn user_query_message(task_id: &str, query: &str, pwd: Option<&str>) -> api::Message {
    let context = pwd.map(|pwd| api::InputContext {
        directory: Some(api::input_context::Directory {
            pwd: pwd.to_string(),
            ..Default::default()
        }),
        ..Default::default()
    });
    api::Message {
        id: format!("{task_id}-user-query"),
        task_id: task_id.to_string(),
        message: Some(api::message::Message::UserQuery(api::message::UserQuery {
            query: query.to_string(),
            context,
            ..Default::default()
        })),
        ..Default::default()
    }
}

fn auto_code_diff_message(task_id: &str) -> api::Message {
    api::Message {
        id: format!("{task_id}-auto-code-diff"),
        task_id: task_id.to_string(),
        message: Some(api::message::Message::SystemQuery(
            api::message::SystemQuery {
                context: None,
                r#type: Some(api::message::system_query::Type::AutoCodeDiff(
                    api::message::AutoCodeDiff {
                        query: "diff".to_string(),
                    },
                )),
            },
        )),
        ..Default::default()
    }
}

#[test]
fn summary_from_tasks_derives_query_title_and_working_directory() {
    let mut root = parentless_task("root", 0);
    root.description = "Root title".to_string();
    root.messages = vec![user_query_message(
        "root",
        "Initial query",
        Some("/tmp/repo"),
    )];

    let summary = AgentConversationSummary::from_tasks([&root]);

    assert_eq!(summary.initial_query, "Initial query");
    assert_eq!(summary.title, "Root title");
    assert_eq!(
        summary.initial_working_directory.as_deref(),
        Some("/tmp/repo")
    );
    assert!(summary.is_restorable);
    assert!(!summary.is_unlisted_auto_code_diff);
}

#[test]
fn summary_from_tasks_falls_back_to_initial_query_when_description_is_empty() {
    let mut root = parentless_task("root", 0);
    root.messages = vec![user_query_message("root", "Initial query", None)];

    let summary = AgentConversationSummary::from_tasks([&root]);

    assert_eq!(summary.title, "Initial query");
    assert_eq!(summary.initial_working_directory, None);
}

#[test]
fn summary_from_tasks_flags_auto_code_diff_only_conversations_as_unlisted() {
    let mut root = parentless_task("root", 0);
    root.messages = vec![auto_code_diff_message("root")];

    let summary = AgentConversationSummary::from_tasks([&root]);
    assert!(summary.is_unlisted_auto_code_diff);

    // A user query alongside the passive diff keeps the conversation listed.
    let mut interacted_root = parentless_task("root", 0);
    interacted_root.messages = vec![
        auto_code_diff_message("root"),
        user_query_message("root", "Follow-up", None),
    ];

    let summary = AgentConversationSummary::from_tasks([&interacted_root]);
    assert!(!summary.is_unlisted_auto_code_diff);
}

#[test]
fn summary_from_tasks_mirrors_restorability() {
    // Two real roots is the genuinely ambiguous, non-restorable shape.
    let summary = AgentConversationSummary::from_tasks([
        &parentless_task("root-a", 1),
        &parentless_task("root-b", 1),
    ]);
    assert!(!summary.is_restorable);

    let summary = AgentConversationSummary::from_tasks([&parentless_task("root", 1)]);
    assert!(summary.is_restorable);
}

#[test]
fn summary_roundtrips_through_json() {
    let mut root = parentless_task("root", 0);
    root.description = "Root title".to_string();
    root.messages = vec![user_query_message(
        "root",
        "Initial query",
        Some("/tmp/repo"),
    )];

    let summary = AgentConversationSummary::from_tasks([&root]);
    let json = serde_json::to_string(&summary).expect("summary should serialize");
    let roundtripped: AgentConversationSummary =
        serde_json::from_str(&json).expect("summary should deserialize");
    assert_eq!(roundtripped, summary);
}

#[test]
fn agent_conversation_data_roundtrips_last_event_sequence() {
    let data = AgentConversationData {
        server_conversation_token: None,
        conversation_usage_metadata: None,
        reverted_action_ids: None,
        forked_from_server_conversation_token: None,
        artifacts_json: None,
        parent_agent_id: None,
        agent_name: None,
        orchestration_harness_type: Some("claude".to_string()),
        parent_conversation_id: None,
        is_remote_child: false,
        root_task_is_optimistic: None,
        run_id: None,
        autoexecute_override: None,
        last_event_sequence: Some(42),
        pinned: false,
    };
    let json = serde_json::to_string(&data).expect("serialize");
    let roundtripped: AgentConversationData = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(roundtripped.last_event_sequence, Some(42));
    assert_eq!(
        roundtripped.orchestration_harness_type.as_deref(),
        Some("claude")
    );
}

#[test]
fn agent_conversation_data_accepts_legacy_orchestration_avatar_id() {
    let legacy_json = r#"{"orchestration_avatar_id":"orbit"}"#;
    let data: AgentConversationData =
        serde_json::from_str(legacy_json).expect("legacy rows must deserialize");

    assert_eq!(data.orchestration_harness_type.as_deref(), Some("orbit"));
}

#[test]
fn agent_conversation_data_roundtrips_remote_child_marker() {
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
        is_remote_child: true,
        root_task_is_optimistic: None,
        run_id: None,
        autoexecute_override: None,
        last_event_sequence: None,
        pinned: false,
    };
    let json = serde_json::to_string(&data).expect("serialize");
    let roundtripped: AgentConversationData = serde_json::from_str(&json).expect("deserialize");
    assert!(roundtripped.is_remote_child);
}

#[test]
fn agent_conversation_data_roundtrips_optimistic_root_marker() {
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
        root_task_is_optimistic: Some(true),
        run_id: None,
        autoexecute_override: None,
        last_event_sequence: None,
        pinned: false,
    };
    let json = serde_json::to_string(&data).expect("serialize");
    let roundtripped: AgentConversationData = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(roundtripped.root_task_is_optimistic, Some(true));
}

#[test]
fn agent_conversation_data_deserializes_legacy_payload_without_last_event_sequence() {
    // Legacy rows persisted before this feature landed omit the field
    // entirely. `#[serde(default)]` must accept them as `None`.
    let legacy_json = r#"{"server_conversation_token":null}"#;
    let data: AgentConversationData =
        serde_json::from_str(legacy_json).expect("legacy rows must deserialize");
    assert_eq!(data.last_event_sequence, None);
    assert_eq!(data.orchestration_harness_type, None);
    assert!(!data.is_remote_child);
}

#[test]
fn agent_conversation_data_skips_serializing_none_last_event_sequence() {
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
        last_event_sequence: None,
        pinned: false,
    };
    let json = serde_json::to_string(&data).expect("serialize");
    assert!(
        !json.contains("last_event_sequence"),
        "None should be skipped in serialized output: {json}"
    );
}

#[test]
fn agent_conversation_data_roundtrips_pinned() {
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
        last_event_sequence: None,
        pinned: true,
    };
    let json = serde_json::to_string(&data).expect("serialize");
    let roundtripped: AgentConversationData = serde_json::from_str(&json).expect("deserialize");
    assert!(roundtripped.pinned);
}

#[test]
fn agent_conversation_data_skips_serializing_unpinned() {
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
        last_event_sequence: None,
        pinned: false,
    };
    let json = serde_json::to_string(&data).expect("serialize");
    assert!(
        !json.contains("pinned"),
        "Unpinned default should be skipped: {json}"
    );
}

#[test]
fn agent_conversation_data_legacy_rows_default_to_unpinned() {
    let legacy_json = r#"{"server_conversation_token":null}"#;
    let data: AgentConversationData =
        serde_json::from_str(legacy_json).expect("legacy rows must deserialize");
    assert!(!data.pinned);
}

#[allow(deprecated)]
#[test]
fn model_token_usage_replays_custom_endpoint_usage_by_model_id() {
    let usage = ModelTokenUsage {
        model_id: "Friendly alias".to_string(),
        custom_endpoint_tokens: 6,
        custom_endpoint_token_usage_by_category: HashMap::from([("primary_agent".to_string(), 6)]),
        ..Default::default()
    };

    let (key, proto) = usage
        .to_proto_custom_endpoint_usage()
        .expect("custom endpoint usage should serialize for replay");

    assert_eq!(key, "Friendly alias");
    assert_eq!(proto.model_id, "Friendly alias");
    assert_eq!(proto.total_tokens, 6);
    assert_eq!(proto.token_usage_by_category.get("primary_agent"), Some(&6));
}

#[allow(deprecated)]
#[test]
fn model_token_usage_replay_skips_non_custom_endpoint_entries() {
    let warp_only = ModelTokenUsage {
        model_id: "warp-model".to_string(),
        warp_tokens: 4,
        ..Default::default()
    };
    assert!(warp_only.to_proto_custom_endpoint_usage().is_none());
}
