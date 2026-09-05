use std::collections::HashMap;

use warp_multi_agent_api as api;

use super::{
    AGENT_CONVERSATION_SUMMARY_VERSION, AgentConversation, AgentConversationData,
    AgentConversationSummary, ChargedUsageTotals, ConversationUsageMetadata, ModelTokenUsage,
};

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

#[test]
fn conversation_usage_metadata_defaults_missing_provider_cost_to_unknown() {
    let metadata: ConversationUsageMetadata = serde_json::from_str(
        r#"{"was_summarized":false,"context_window_usage":0.0,"credits_spent":0.0}"#,
    )
    .unwrap();

    assert_eq!(metadata.total_provider_cost_in_cents, None);
    assert!(
        !serde_json::to_string(&metadata)
            .unwrap()
            .contains("total_provider_cost_in_cents")
    );
}

#[test]
fn conversation_usage_metadata_preserves_known_zero_provider_cost() {
    let metadata: ConversationUsageMetadata = serde_json::from_str(
        r#"{"was_summarized":false,"context_window_usage":0.0,"credits_spent":0.0,"total_provider_cost_in_cents":0.0}"#,
    )
    .unwrap();

    assert_eq!(metadata.total_provider_cost_in_cents, Some(0.0));
    assert!(
        serde_json::to_string(&metadata)
            .unwrap()
            .contains("\"total_provider_cost_in_cents\":0.0")
    );
}

fn inference_usage_with_web_search(
    input: u32,
    output: u32,
    input_cost_in_cents: f32,
    output_cost_in_cents: f32,
    web_search_count: u32,
    web_search_cost_in_cents: f32,
) -> api::response_event::stream_finished::InferenceUsage {
    api::response_event::stream_finished::InferenceUsage {
        token_count: Some(api::response_event::stream_finished::TokenCount {
            input,
            output,
            input_cache_read: 0,
            input_cache_write: 0,
        }),
        token_cost: Some(api::response_event::stream_finished::TokenCost {
            input_cost_in_cents,
            output_cost_in_cents,
            input_cache_read_cost_in_cents: 0.0,
            input_cache_write_cost_in_cents: 0.0,
        }),
        web_search_count,
        web_search_cost_in_cents,
    }
}

#[test]
fn charged_usage_totals_sums_web_search_fields_across_categories_and_models() {
    let mut usage_by_category = HashMap::new();
    usage_by_category.insert(
        "primary_agent".to_string(),
        api::response_event::stream_finished::ChargedUsage {
            direct_api_inference_usage: HashMap::from([(
                "claude-4.5".to_string(),
                inference_usage_with_web_search(1000, 200, 3.0, 6.0, 2, 5.0),
            )]),
            byok_inference_usage: HashMap::new(),
            custom_endpoint_inference_usage: HashMap::new(),
            platform_usage_in_cents: 1.0,
        },
    );
    usage_by_category.insert(
        "compaction".to_string(),
        api::response_event::stream_finished::ChargedUsage {
            direct_api_inference_usage: HashMap::from([(
                "claude-4.5".to_string(),
                inference_usage_with_web_search(500, 100, 1.5, 3.0, 1, 2.5),
            )]),
            byok_inference_usage: HashMap::new(),
            custom_endpoint_inference_usage: HashMap::new(),
            platform_usage_in_cents: 0.0,
        },
    );
    let charges = api::response_event::stream_finished::RequestCharges { usage_by_category };

    let totals = ChargedUsageTotals::from(&charges);

    assert_eq!(totals.web_search_count, 3);
    assert!((totals.web_search_cost_in_cents - 7.5).abs() < 1e-6);
    // Total cost must include web search cost alongside the token + platform costs.
    assert!((totals.total_cost_in_cents() - (3.0 + 6.0 + 1.5 + 3.0 + 1.0 + 7.5)).abs() < 1e-6);
}

#[test]
fn charged_usage_totals_add_assign_sums_web_search_fields() {
    let mut a = ChargedUsageTotals {
        web_search_count: 2,
        web_search_cost_in_cents: 4.0,
        ..Default::default()
    };
    let b = ChargedUsageTotals {
        web_search_count: 3,
        web_search_cost_in_cents: 6.0,
        ..Default::default()
    };

    a += b;

    assert_eq!(a.web_search_count, 5);
    assert!((a.web_search_cost_in_cents - 10.0).abs() < 1e-6);
}

#[test]
fn charged_usage_totals_deserializes_legacy_payload_without_web_search_fields() {
    let totals: ChargedUsageTotals = serde_json::from_str(
        r#"{"input_cost_in_cents":1.0,"output_cost_in_cents":2.0,"input_cache_read_cost_in_cents":0.0,"input_cache_write_cost_in_cents":0.0,"platform_cost_in_cents":0.0,"input_tokens":10,"output_tokens":5,"input_cache_read_tokens":0,"input_cache_write_tokens":0}"#,
    )
    .unwrap();

    assert_eq!(totals.web_search_count, 0);
    assert_eq!(totals.web_search_cost_in_cents, 0.0);
    assert!((totals.total_cost_in_cents() - 3.0).abs() < 1e-6);
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

fn auto_code_diff_task(id: &str) -> api::Task {
    let mut task = parentless_task(id, 0);
    task.messages = vec![auto_code_diff_message(id)];
    task
}

fn tool_call_result_message(
    task_id: &str,
    result: api::message::tool_call_result::Result,
) -> api::Message {
    api::Message {
        id: format!("{task_id}-tool-call-result"),
        task_id: task_id.to_string(),
        message: Some(api::message::Message::ToolCallResult(
            api::message::ToolCallResult {
                tool_call_id: format!("{task_id}-tool-call"),
                result: Some(result),
                ..Default::default()
            },
        )),
        ..Default::default()
    }
}

/// The current-version marker is what tells a newer client that a stored
/// summary carries every derived field it expects.
#[test]
fn summary_from_tasks_is_stamped_with_the_current_version() {
    let summary = AgentConversationSummary::from_tasks([&parentless_task("root", 1)]);
    assert_eq!(summary.version, AGENT_CONVERSATION_SUMMARY_VERSION);
    assert!(summary.is_current_version());
}

/// A summary written before the version field existed must not be trusted:
/// its derived fields are absent, and defaulting them would silently change
/// restore decisions.
#[test]
fn summary_without_a_version_is_not_current_and_reports_unknown_passiveness() {
    let legacy_json =
        r#"{"initial_query":"Initial query","title":"Root title","is_restorable":true}"#;
    let summary: AgentConversationSummary =
        serde_json::from_str(legacy_json).expect("legacy summaries must still deserialize");

    assert_eq!(summary.version, 0);
    assert!(!summary.is_current_version());
    assert_eq!(
        summary.is_entirely_passive, None,
        "a missing derived field must read as unknown, never as a default answer"
    );
}

/// A conversation whose root task holds only a passive `AutoCodeDiff` query is
/// what `AIConversation::is_entirely_passive()` reports `true` for.
#[test]
fn is_entirely_passive_is_true_for_an_untouched_auto_code_diff() {
    let summary = AgentConversationSummary::from_tasks([&auto_code_diff_task("root")]);
    assert_eq!(summary.is_entirely_passive, Some(true));
}

/// A user query anywhere in the root task means the user engaged with the
/// conversation, so it is no longer entirely passive.
#[test]
fn is_entirely_passive_is_false_once_the_root_task_has_a_user_query() {
    let mut root = auto_code_diff_task("root");
    root.messages
        .push(user_query_message("root", "Follow-up", None));
    assert_eq!(
        AgentConversationSummary::from_tasks([&root]).is_entirely_passive,
        Some(false)
    );

    // Order must not matter: the predicate is existential over exchanges.
    let mut root = parentless_task("root", 0);
    root.messages = vec![
        user_query_message("root", "Follow-up", None),
        auto_code_diff_message("root"),
    ];
    assert_eq!(
        AgentConversationSummary::from_tasks([&root]).is_entirely_passive,
        Some(false)
    );
}

/// No passive request at all means the conversation cannot be entirely
/// passive, whatever else it contains.
#[test]
fn is_entirely_passive_is_false_without_any_passive_request() {
    let mut root = parentless_task("root", 0);
    root.messages = vec![user_query_message("root", "Initial query", None)];
    assert_eq!(
        AgentConversationSummary::from_tasks([&root]).is_entirely_passive,
        Some(false)
    );

    // A root task with no messages restores to a conversation with a single
    // exchange-less root task, which is not entirely passive either.
    assert_eq!(
        AgentConversationSummary::from_tasks([&parentless_task("root", 0)]).is_entirely_passive,
        Some(false)
    );
}

/// Restore synthesizes a fresh root task for a row with no persisted tasks, so
/// there is no passive exchange to find.
#[test]
fn is_entirely_passive_is_false_for_a_task_less_row() {
    let summary = AgentConversationSummary::from_tasks(std::iter::empty());
    assert_eq!(summary.is_entirely_passive, Some(false));
}

/// Only the *root* task's exchanges feed the predicate: a user query in a
/// subtask does not make an otherwise passive conversation active.
#[test]
fn is_entirely_passive_only_considers_the_root_task() {
    let mut subtask = child_task("child-1", "root");
    subtask.messages = vec![user_query_message("child-1", "Subagent prompt", None)];

    let summary = AgentConversationSummary::from_tasks([&auto_code_diff_task("root"), &subtask]);
    assert_eq!(summary.is_entirely_passive, Some(true));
}

/// An accepted prompt suggestion restores to an input that renders a display
/// query, so it counts as a user query; a rejected one does not.
#[test]
fn is_entirely_passive_accounts_for_accepted_prompt_suggestions() {
    let accepted = tool_call_result_message(
        "root",
        api::message::tool_call_result::Result::SuggestPrompt(api::SuggestPromptResult {
            result: Some(api::suggest_prompt_result::Result::Accepted(())),
        }),
    );
    let mut root = auto_code_diff_task("root");
    root.messages.push(accepted);
    assert_eq!(
        AgentConversationSummary::from_tasks([&root]).is_entirely_passive,
        Some(false)
    );

    let rejected = tool_call_result_message(
        "root",
        api::message::tool_call_result::Result::SuggestPrompt(api::SuggestPromptResult {
            result: Some(api::suggest_prompt_result::Result::Rejected(())),
        }),
    );
    let mut root = auto_code_diff_task("root");
    root.messages.push(rejected);
    assert_eq!(
        AgentConversationSummary::from_tasks([&root]).is_entirely_passive,
        Some(true)
    );
}

/// Two message-bearing parentless tasks leave restore's own root pick up to
/// hash iteration order, so the summary must decline to answer rather than
/// pick a side.
#[test]
fn is_entirely_passive_is_unknown_for_an_ambiguous_root() {
    let mut other_root = parentless_task("root-b", 0);
    other_root.messages = vec![user_query_message("root-b", "Initial query", None)];

    let summary =
        AgentConversationSummary::from_tasks([&auto_code_diff_task("root-a"), &other_root]);
    assert_eq!(summary.is_entirely_passive, None);
}

/// Every persisted task having a parent is the shape restore rejects with
/// `NoRootTask`; there is no root to evaluate, so the answer is unknown.
#[test]
fn is_entirely_passive_is_unknown_without_a_parentless_task() {
    let summary = AgentConversationSummary::from_tasks([&child_task("child-1", "missing-root")]);
    assert_eq!(summary.is_entirely_passive, None);
}

fn invoke_skill_message(task_id: &str) -> api::Message {
    api::Message {
        id: format!("{task_id}-invoke-skill"),
        task_id: task_id.to_string(),
        message: Some(api::message::Message::InvokeSkill(
            api::message::InvokeSkill {
                skill: Some(Default::default()),
                ..Default::default()
            },
        )),
        ..Default::default()
    }
}

/// Whether an `InvokeSkill` message renders a display query depends on client
/// state this crate cannot see, so a root that pairs one with a passive request
/// degrades to unknown rather than guessing.
#[test]
fn is_entirely_passive_is_unknown_when_the_root_invokes_a_skill() {
    let mut root = auto_code_diff_task("root");
    root.messages.push(invoke_skill_message("root"));

    assert_eq!(
        AgentConversationSummary::from_tasks([&root]).is_entirely_passive,
        None
    );
}

/// An unclassifiable message can only resolve to a user query or to nothing,
/// never to a passive request. Without a passive request in the root the answer
/// is settled either way, so a slash-command conversation must not be pushed
/// onto the slow path.
#[test]
fn is_entirely_passive_is_false_for_a_skill_invocation_without_a_passive_request() {
    let mut root = parentless_task("root", 0);
    root.messages = vec![invoke_skill_message("root")];

    assert_eq!(
        AgentConversationSummary::from_tasks([&root]).is_entirely_passive,
        Some(false)
    );
}

/// Restore treats `Some(Dependencies { parent_task_id: "" })` as parentless (via
/// `TaskExt::parent_id`), so the summary must too. In the legacy QUALITY-774
/// `[stub + real root]` shape, answering from the empty stub instead of the real
/// root would call a passive-only conversation active and restore it with a
/// spurious "Previous session" banner.
#[test]
fn is_entirely_passive_treats_an_empty_parent_task_id_as_parentless() {
    let mut real_root = child_task("server-root-id", "");
    real_root.messages = vec![auto_code_diff_message("server-root-id")];

    let summary = AgentConversationSummary::from_tasks([
        &parentless_task("optimistic-stub-uuid", 0),
        &real_root,
    ]);
    assert_eq!(summary.is_entirely_passive, Some(true));

    // The same shape with a real user query in the real root.
    let mut real_root = child_task("server-root-id", "");
    real_root.messages = vec![user_query_message("server-root-id", "Initial query", None)];

    let summary = AgentConversationSummary::from_tasks([
        &parentless_task("optimistic-stub-uuid", 0),
        &real_root,
    ]);
    assert_eq!(summary.is_entirely_passive, Some(false));

    // Only `restored_root_task` prefers the message-bearing candidate; the
    // title/query finder shares the parentless rule but still takes the first
    // candidate in order, so a leading stub still wins there. That asymmetry is
    // pre-existing and out of scope here — pinned so a future change to the
    // finder is a deliberate one.
    let mut real_root_with_title = real_root.clone();
    real_root_with_title.description = "Root title".to_string();
    let summary = AgentConversationSummary::from_tasks([
        &parentless_task("optimistic-stub-uuid", 0),
        &real_root_with_title,
    ]);
    assert_eq!(summary.title, "");
    let summary = AgentConversationSummary::from_tasks([&real_root_with_title]);
    assert_eq!(summary.title, "Root title");
    assert_eq!(summary.initial_query, "Initial query");
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
