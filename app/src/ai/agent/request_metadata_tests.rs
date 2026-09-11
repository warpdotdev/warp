use std::collections::HashMap;

use ai::skills::SkillPathOrigin;
use chrono::Local;
use warp_multi_agent_api as api;
use warp_multi_agent_api::client_action::{
    Action, AddMessagesToTask, CreateTask, MoveMessagesToNewTask,
};
use warp_multi_agent_api::message::request_metadata;
use warpui::{App, EntityId};

use super::*;
use crate::ai::agent::conversation::{AIConversation, AIConversationId};
use crate::ai::agent::request_metadata::summarize_turn;
use crate::ai::agent::task::TaskId;
use crate::ai::agent::{
    AIAgentActionId, AIAgentActionResult, AIAgentActionResultType, AIAgentInput,
    CancellationReason, UserQueryMode,
};
use crate::ai::blocklist::history_model::BlocklistAIHistoryModel;
use crate::ai::blocklist::{RequestInput, ResponseStreamId};
use crate::ai::llms::LLMId;
use crate::test_util::ai_agent_tasks::create_api_task;
use crate::test_util::settings::initialize_history_persistence_for_tests;

fn timestamp(seconds: i64) -> prost_types::Timestamp {
    prost_types::Timestamp { seconds, nanos: 0 }
}

fn duration(seconds: i64, nanos: i32) -> prost_types::Duration {
    prost_types::Duration { seconds, nanos }
}

fn record_message(
    request_id: &str,
    outcome: request_metadata::Outcome,
    with_charges: bool,
) -> api::Message {
    let charges = with_charges.then(|| api::RequestCharges {
        usage_by_category: HashMap::from([(
            "primary_agent".to_string(),
            api::ChargedUsage {
                direct_api_inference_usage: HashMap::from([(
                    "Claude Sonnet 4.6".to_string(),
                    api::InferenceUsage {
                        token_count: Some(api::TokenCount {
                            input: 1000,
                            output: 200,
                            input_cache_read: 50,
                            input_cache_write: 0,
                        }),
                        token_cost: Some(api::TokenCost {
                            input_cost_in_cents: 0.30,
                            output_cost_in_cents: 0.30,
                            input_cache_read_cost_in_cents: 0.01,
                            input_cache_write_cost_in_cents: 0.0,
                            input_cost_in_credits: 0.10,
                            output_cost_in_credits: 0.20,
                            input_cache_read_cost_in_credits: 0.02,
                            input_cache_write_cost_in_credits: 0.0,
                        }),
                        web_search_count: 1,
                        web_search_cost_in_cents: 0.5,
                        web_search_cost_in_credits: 0.5,
                    },
                )]),
                byok_inference_usage: HashMap::new(),
                custom_endpoint_inference_usage: HashMap::new(),
                platform_usage_in_cents: 2.0,
                platform_usage_duration: Some(duration(90, 0)),
                platform_usage_in_credits: 1.0,
            },
        )]),
    });

    api::Message {
        id: format!("msg-{request_id}"),
        task_id: "root".to_string(),
        request_id: request_id.to_string(),
        timestamp: Some(timestamp(1_000_012)),
        message: Some(api::message::Message::RequestMetadata(
            api::message::RequestMetadata {
                timing: Some(api::RequestTiming {
                    request_timespan: Some(api::TimeSpan {
                        started_at: Some(timestamp(1_000_000)),
                        ended_at: Some(timestamp(1_000_012)),
                    }),
                    first_token_at: Some(timestamp(1_000_001)),
                    llm_generation_timespans: vec![api::TimeSpan {
                        started_at: Some(timestamp(1_000_004)),
                        ended_at: Some(timestamp(1_000_011)),
                    }],
                }),
                charges,
                incomplete: outcome != request_metadata::Outcome::Completed,
                outcome: outcome as i32,
                tool_call_summary: Some(request_metadata::ToolCallSummary {
                    tool_calls: 4,
                    commands_executed: 2,
                    files_changed: 3,
                    lines_added: 40,
                    lines_removed: 8,
                }),
                context_window: Some(request_metadata::ContextWindow { usage: 42.0 }),
            },
        )),
        ..Default::default()
    }
}

#[test]
fn decodes_a_completed_record() {
    let record = RequestMetadataRecord::from_message(&record_message(
        "req-1",
        request_metadata::Outcome::Completed,
        true,
    ))
    .expect("record");

    assert_eq!(record.request_id, "req-1");
    assert_eq!(record.outcome, RequestOutcome::Completed);
    assert!(!record.outcome.is_interrupted());
    assert_eq!(record.time_to_first_token_ms(), Some(1000));
    assert_eq!(record.request_duration_ms(), Some(12_000));
    assert_eq!(record.llm_generation_ms(), Some(7000));
    assert_eq!(record.total_tokens(), 1250);
    assert_eq!(record.model_charges.len(), 1);
    assert_eq!(record.model_charges[0].usage_type, "direct_api");
    assert!((record.inference_cost_in_cents() - 1.11).abs() < 1e-5);
    assert!((record.inference_cost_in_credits() - 0.82).abs() < 1e-5);
    assert!((record.platform_cost_in_cents() - 2.0).abs() < 1e-5);
    assert!((record.platform_cost_in_credits() - 1.0).abs() < 1e-5);
    assert!((record.total_cost_in_cents() - 3.11).abs() < 1e-5);
    assert!((record.total_cost_in_credits() - 1.82).abs() < 1e-5);
    assert_eq!(record.platform_charges[0].duration_seconds, 90.0);
    assert_eq!(record.tool_calls, Some(4));
    assert_eq!(record.commands_executed, Some(2));
    assert_eq!(record.files_changed, Some(3));
    assert_eq!(record.lines_added, Some(40));
    assert_eq!(record.lines_removed, Some(8));
    // The merged schema's context window is a 0-100 percentage.
    assert_eq!(record.context_window_usage, Some(42.0));
}

#[test]
fn errored_record_has_no_charges_but_keeps_timing() {
    let record = RequestMetadataRecord::from_message(&record_message(
        "req-err",
        request_metadata::Outcome::Errored,
        false,
    ))
    .expect("record");

    assert_eq!(record.outcome, RequestOutcome::Errored);
    assert!(record.outcome.is_interrupted());
    assert_eq!(record.outcome.label(), "Errored");
    assert!(record.model_charges.is_empty());
    assert!(record.platform_charges.is_empty());
    assert_eq!(record.total_cost_in_cents(), 0.0);
    assert_eq!(record.request_duration_ms(), Some(12_000));
}

#[test]
fn legacy_record_without_outcome_falls_back_to_incomplete_flag() {
    let mut message = record_message("req-legacy", request_metadata::Outcome::Completed, false);
    if let Some(api::message::Message::RequestMetadata(metadata)) = message.message.as_mut() {
        metadata.outcome = 0;
        metadata.incomplete = true;
    }
    let record = RequestMetadataRecord::from_message(&message).expect("record");
    assert_eq!(
        record.outcome,
        RequestOutcome::Unspecified { incomplete: true }
    );
    assert!(record.outcome.is_interrupted());
    assert_eq!(record.outcome.label(), "Incomplete");
}

#[test]
fn non_record_messages_are_ignored() {
    let message = api::Message {
        id: "m".to_string(),
        message: Some(api::message::Message::AgentOutput(
            api::message::AgentOutput {
                text: "hello".to_string(),
            },
        )),
        ..Default::default()
    };
    assert!(RequestMetadataRecord::from_message(&message).is_none());

    let messages = [
        message,
        record_message("req-1", request_metadata::Outcome::Completed, true),
        record_message("req-2", request_metadata::Outcome::Canceled, false),
    ];
    let records: Vec<_> = messages
        .iter()
        .filter_map(RequestMetadataRecord::from_message)
        .collect();
    assert_eq!(
        records
            .iter()
            .map(|r| r.request_id.as_str())
            .collect::<Vec<_>>(),
        ["req-1", "req-2"]
    );
}

#[test]
fn json_carries_every_section() {
    let record = RequestMetadataRecord::from_message(&record_message(
        "req-1",
        request_metadata::Outcome::Canceled,
        true,
    ))
    .expect("record");
    let json = record.to_json();

    assert_eq!(json["request_id"], "req-1");
    assert_eq!(json["outcome"], "Canceled");
    assert_eq!(json["incomplete"], true);
    assert_eq!(
        json["timing"]["llm_generation_timespans"][0]["duration_ms"],
        7000
    );
    assert_eq!(
        json["charges"]["models"][0]["model_id"],
        "Claude Sonnet 4.6"
    );
    assert_eq!(json["charges"]["models"][0]["tokens"]["input"], 1000);
    assert_eq!(json["charges"]["platform"][0]["duration_seconds"], 90.0);
    assert_eq!(json["charges"]["platform"][0]["cost_in_credits"], 1.0);
    assert_eq!(json["tool_call_summary"]["files_changed"], 3);
    assert_eq!(json["context_window"]["usage"], 42.0);
    // Must be a stable, pretty-printable document.
    assert!(
        serde_json::to_string_pretty(&json)
            .unwrap()
            .contains("\"tool_calls\": 4")
    );
}

fn turn_messages(request_id: &str, seconds: i64) -> Vec<api::Message> {
    let user_query = api::Message {
        id: format!("query-{request_id}"),
        task_id: "root".to_string(),
        request_id: request_id.to_string(),
        timestamp: Some(timestamp(seconds)),
        message: Some(api::message::Message::UserQuery(api::message::UserQuery {
            query: format!("question for {request_id}"),
            ..Default::default()
        })),
        ..Default::default()
    };
    let agent_output = api::Message {
        id: format!("output-{request_id}"),
        task_id: "root".to_string(),
        request_id: request_id.to_string(),
        timestamp: Some(timestamp(seconds + 1)),
        message: Some(api::message::Message::AgentOutput(
            api::message::AgentOutput {
                text: format!("answer for {request_id}"),
            },
        )),
        ..Default::default()
    };
    let mut record = record_message(request_id, request_metadata::Outcome::Completed, true);
    record.timestamp = Some(timestamp(seconds + 2));
    vec![user_query, agent_output, record]
}

/// Every block reads its own turn's record, not only the newest one: after a restore, each
/// exchange must resolve to the record the server appended to *its* request.
#[test]
fn every_restored_exchange_resolves_its_own_record() {
    let mut messages = turn_messages("req-1", 1_000);
    messages.extend(turn_messages("req-2", 2_000));
    // The third turn was cancelled before its record reached this client.
    let mut cancelled = turn_messages("req-3", 3_000);
    cancelled.pop();
    messages.extend(cancelled);
    messages.extend(turn_messages("req-4", 4_000));

    let task = api::Task {
        id: "root".to_string(),
        messages,
        ..Default::default()
    };
    let conversation = AIConversation::new_restored(AIConversationId::new(), vec![task], None)
        .expect("restored conversation");

    let exchange_ids: Vec<_> = conversation
        .root_task_exchanges()
        .map(|exchange| exchange.id)
        .collect();
    assert_eq!(exchange_ids.len(), 4, "one exchange per request");

    let resolved: Vec<Vec<String>> = exchange_ids
        .iter()
        .map(|exchange_id| {
            conversation
                .request_metadata_records_for_exchange(*exchange_id)
                .into_iter()
                .map(|record| record.request_id)
                .collect()
        })
        .collect();
    assert_eq!(
        resolved,
        [
            vec!["req-1".to_string()],
            vec!["req-2".to_string()],
            vec![],
            vec!["req-4".to_string()],
        ]
    );
}

/// A tool-result round trip: the request the client sends after executing a tool call. It has no
/// user query, so it belongs to the turn started by the last user query before it.
fn tool_round_trip_messages(request_id: &str, seconds: i64) -> Vec<api::Message> {
    let tool_result = api::Message {
        id: format!("result-{request_id}"),
        task_id: "root".to_string(),
        request_id: request_id.to_string(),
        timestamp: Some(timestamp(seconds)),
        message: Some(api::message::Message::ToolCallResult(
            api::message::ToolCallResult {
                tool_call_id: format!("call-{request_id}"),
                ..Default::default()
            },
        )),
        ..Default::default()
    };
    let agent_output = api::Message {
        id: format!("output-{request_id}"),
        task_id: "root".to_string(),
        request_id: request_id.to_string(),
        timestamp: Some(timestamp(seconds + 1)),
        message: Some(api::message::Message::AgentOutput(
            api::message::AgentOutput {
                text: format!("answer after tool for {request_id}"),
            },
        )),
        ..Default::default()
    };
    let mut record = record_message(request_id, request_metadata::Outcome::Completed, true);
    record.timestamp = Some(timestamp(seconds + 2));
    vec![tool_result, agent_output, record]
}

/// A user-visible turn spans every exchange from the user's query through its tool-result round
/// trips, so the icon belongs on the turn's last block only and the panel unions the records of
/// all of them.
#[test]
fn tool_round_trips_group_into_the_user_query_turn() {
    // Turn A: query (req-1) → tool round trip (req-2) → tool round trip (req-3).
    let mut messages = turn_messages("req-1", 1_000);
    messages.extend(tool_round_trip_messages("req-2", 2_000));
    messages.extend(tool_round_trip_messages("req-3", 3_000));
    // Turn B: a single-request query.
    messages.extend(turn_messages("req-4", 4_000));

    let task = api::Task {
        id: "root".to_string(),
        messages,
        ..Default::default()
    };
    let conversation = AIConversation::new_restored(AIConversationId::new(), vec![task], None)
        .expect("restored conversation");
    let exchange_ids: Vec<_> = conversation
        .root_task_exchanges()
        .map(|exchange| exchange.id)
        .collect();
    assert_eq!(exchange_ids.len(), 4, "one exchange per request");
    let [first, second, third, fourth] = exchange_ids[..] else {
        unreachable!()
    };

    for id in [first, second, third] {
        assert_eq!(
            conversation.turn_exchange_ids(id),
            vec![first, second, third]
        );
    }
    assert_eq!(conversation.turn_exchange_ids(fourth), vec![fourth]);

    assert!(!conversation.is_last_exchange_in_turn(first));
    assert!(!conversation.is_last_exchange_in_turn(second));
    assert!(conversation.is_last_exchange_in_turn(third));
    assert!(conversation.is_last_exchange_in_turn(fourth));

    let turn_a_request_ids: Vec<String> = conversation
        .request_metadata_records_for_turn(third)
        .into_iter()
        .map(|record| record.request_id)
        .collect();
    assert_eq!(turn_a_request_ids, ["req-1", "req-2", "req-3"]);
    // Asking via any exchange of the turn yields the same set.
    assert_eq!(
        conversation.request_metadata_records_for_turn(first),
        conversation.request_metadata_records_for_turn(third)
    );
    let turn_b_request_ids: Vec<String> = conversation
        .request_metadata_records_for_turn(fourth)
        .into_iter()
        .map(|record| record.request_id)
        .collect();
    assert_eq!(turn_b_request_ids, ["req-4"]);

    let turn_records = conversation.request_metadata_records_for_turn(third);
    let summary = summarize_turn(&turn_records);
    assert_eq!(summary.request_count, 3);
    let total_cost_in_cents: f32 = turn_records
        .iter()
        .map(|record| record.total_cost_in_cents())
        .sum();
    assert!((total_cost_in_cents - 3.0 * 3.11).abs() < 1e-4);
}

/// A turn whose final request was cancelled or disconnected mid-stream never receives that
/// request's record. The panel must not treat the earlier requests' records as the turn's
/// total: a non-empty record set is not the eligibility contract.
#[test]
fn turn_with_a_missing_final_record_is_not_eligible() {
    let mut messages = turn_messages("req-1", 1_000);
    // The tool-follow-up request req-2 was cancelled mid-stream: its partial output arrived,
    // but its record never did.
    let mut follow_up = tool_round_trip_messages("req-2", 2_000);
    follow_up.pop();
    messages.extend(follow_up);

    let task = api::Task {
        id: "root".to_string(),
        messages,
        ..Default::default()
    };
    let conversation = AIConversation::new_restored(AIConversationId::new(), vec![task], None)
        .expect("restored conversation");
    let exchange_ids: Vec<_> = conversation
        .root_task_exchanges()
        .map(|exchange| exchange.id)
        .collect();
    assert_eq!(exchange_ids.len(), 2, "one exchange per request");
    let [_, last] = exchange_ids[..] else {
        unreachable!()
    };

    // The lookup still resolves the one record that did arrive — which is exactly why the
    // old non-empty-records check landed the icon on the cancelled request's block, claiming
    // the completed charge as the turn's total.
    let resolved: Vec<String> = conversation
        .request_metadata_records_for_turn(last)
        .into_iter()
        .map(|record| record.request_id)
        .collect();
    assert_eq!(resolved, ["req-1".to_string()]);

    // …but the turn is not eligible for the panel while req-2's record is missing.
    assert!(conversation.turn_panel_records(last).is_none());

    // The complete version of the same turn is eligible and yields both records.
    let mut messages = turn_messages("req-1", 1_000);
    messages.extend(tool_round_trip_messages("req-2", 2_000));
    let task = api::Task {
        id: "root".to_string(),
        messages,
        ..Default::default()
    };
    let conversation = AIConversation::new_restored(AIConversationId::new(), vec![task], None)
        .expect("restored conversation");
    let last = conversation
        .root_task_exchanges()
        .last()
        .expect("exchange")
        .id;
    let records = conversation
        .turn_panel_records(last)
        .expect("complete turn is eligible");
    assert_eq!(
        records
            .iter()
            .map(|record| record.request_id.as_str())
            .collect::<Vec<_>>(),
        ["req-1", "req-2"]
    );
}

/// Builds a `RequestInput` the way the live send path does: one task's inputs plus the
/// request bookkeeping fields.
fn live_request_input(
    conversation_id: AIConversationId,
    task_id: TaskId,
    input: AIAgentInput,
) -> RequestInput {
    RequestInput {
        conversation_id,
        input_messages: HashMap::from([(task_id, vec![input])]),
        working_directory: None,
        model_id: LLMId::from("test-model"),
        coding_model_id: LLMId::from("test-coding-model"),
        cli_agent_model_id: LLMId::from("test-cli-agent-model"),
        computer_use_model_id: LLMId::from("test-computer-use-model"),
        shared_session_response_initiator: None,
        request_start_ts: Local::now(),
        supported_tools_override: None,
    }
}

/// A live request that is cancelled before the server's input echo lands leaves its exchange
/// with no messages at all (`update_for_new_request_input` starts `added_message_ids` empty,
/// and the cancel path never fills it). Such an exchange contributes no request ids, so the
/// turn must be treated as incomplete rather than rendering its earlier requests' records as
/// the total.
#[test]
fn exchange_without_any_request_messages_is_not_eligible() {
    App::test((), |mut app| async move {
        initialize_history_persistence_for_tests(&mut app);
        let history_model =
            app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], vec![], &[]));
        let terminal_surface_id = EntityId::new();

        let mut conversation = AIConversation::new(false, false);
        let optimistic_task_id = conversation.get_root_task_id().clone();

        // The turn's initiating request, sent the live way: its exchange starts with no
        // messages until the server echoes the input back.
        let stream_id = ResponseStreamId::new_for_test();
        let query_input = AIAgentInput::UserQuery {
            query: "plan the migration".to_string(),
            context: Default::default(),
            static_query_type: None,
            referenced_attachments: Default::default(),
            user_query_mode: UserQueryMode::Normal,
            running_command: None,
            intended_agent: None,
        };
        history_model.update(&mut app, |_, ctx| {
            conversation
                .update_for_new_request_input(
                    live_request_input(conversation.id(), optimistic_task_id, query_input),
                    stream_id.clone(),
                    terminal_surface_id,
                    ctx,
                )
                .expect("new request input should apply");
        });

        // The server upgrades the optimistic root, echoes the input, and streams the response
        // and its record; the request completes.
        history_model.update(&mut app, |_, ctx| {
            conversation
                .initialize_output_for_response_stream(
                    &stream_id,
                    api::response_event::StreamInit {
                        request_id: "req-1".to_string(),
                        conversation_id: "server-conversation".to_string(),
                        run_id: String::new(),
                    },
                    terminal_surface_id,
                    ctx,
                )
                .expect("stream init should apply");
            conversation
                .apply_client_action(
                    &stream_id,
                    terminal_surface_id,
                    Action::CreateTask(CreateTask {
                        task: Some(create_api_task("root", vec![])),
                    }),
                    &SkillPathOrigin::Unavailable,
                    ctx,
                )
                .expect("create task should apply");
            conversation
                .apply_client_action(
                    &stream_id,
                    terminal_surface_id,
                    Action::AddMessagesToTask(AddMessagesToTask {
                        task_id: "root".to_string(),
                        messages: turn_messages("req-1", 1_000),
                    }),
                    &SkillPathOrigin::Unavailable,
                    ctx,
                )
                .expect("add messages should apply");
            conversation
                .mark_request_completed(&stream_id, terminal_surface_id, ctx)
                .expect("request should complete");
        });

        let query_exchange_id = conversation
            .root_task_exchanges()
            .last()
            .expect("query exchange exists")
            .id;
        let resolved: Vec<String> = conversation
            .request_metadata_records_for_exchange(query_exchange_id)
            .into_iter()
            .map(|record| record.request_id)
            .collect();
        assert_eq!(resolved, ["req-1".to_string()]);

        // A tool-result round trip is requested, but the stream is cancelled before the
        // server's input echo lands: the new exchange exists and closes the turn, yet holds
        // no messages.
        let follow_up_stream_id = ResponseStreamId::new_for_test();
        let follow_up_input = AIAgentInput::ActionResult {
            result: AIAgentActionResult {
                id: AIAgentActionId::from("action-1".to_string()),
                task_id: TaskId::new("root".to_string()),
                result: AIAgentActionResultType::OpenCodeReview,
            },
            context: Default::default(),
        };
        history_model.update(&mut app, |_, ctx| {
            conversation
                .update_for_new_request_input(
                    live_request_input(
                        conversation.id(),
                        TaskId::new("root".to_string()),
                        follow_up_input,
                    ),
                    follow_up_stream_id.clone(),
                    terminal_surface_id,
                    ctx,
                )
                .expect("follow-up request input should apply");
            conversation
                .mark_request_cancelled(
                    &follow_up_stream_id,
                    terminal_surface_id,
                    CancellationReason::ManuallyCancelled,
                    ctx,
                )
                .expect("cancellation should apply");
        });

        let follow_up_exchange_id = conversation
            .root_task_exchanges()
            .last()
            .expect("follow-up exchange exists")
            .id;
        assert!(conversation.is_last_exchange_in_turn(follow_up_exchange_id));

        // The turn's records cover only the query request — which is exactly what the panel
        // must not present as the turn's total: the message-less follow-up exchange makes the
        // turn ineligible.
        assert!(
            conversation
                .turn_panel_records(follow_up_exchange_id)
                .is_none()
        );
    });
}

/// Summarization (`Action::MoveMessagesToNewTask`) moves an exchange's messages into a subtask
/// while the exchange's client representation keeps naming them: the record lookup must
/// resolve message membership across the task store, or the icons disappear after a move.
#[test]
fn records_resolve_after_a_summarization_move() {
    App::test((), |mut app| async move {
        let history_model =
            app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], vec![], &[]));

        let mut messages = turn_messages("req-1", 1_000);
        messages.extend(turn_messages("req-2", 2_000));
        let task = api::Task {
            id: "root".to_string(),
            messages,
            ..Default::default()
        };
        let mut conversation =
            AIConversation::new_restored(AIConversationId::new(), vec![task], None)
                .expect("restored conversation");
        let exchange_ids: Vec<_> = conversation
            .root_task_exchanges()
            .map(|exchange| exchange.id)
            .collect();
        let [first, second] = exchange_ids[..] else {
            unreachable!()
        };

        // Move the whole first turn into a summarization subtask, the way the server's
        // summarization sub-agent relocates earlier conversation messages.
        let action = Action::MoveMessagesToNewTask(MoveMessagesToNewTask {
            source_task_id: "root".to_string(),
            new_task: Some(api::Task {
                id: "summary-sub".to_string(),
                dependencies: Some(api::task::Dependencies {
                    parent_task_id: "root".to_string(),
                }),
                ..Default::default()
            }),
            first_message_id: "query-req-1".to_string(),
            last_message_id: "msg-req-1".to_string(),
            expected_message_count: 3,
            replacement_messages: Vec::new(),
        });
        history_model.update(&mut app, |_, ctx| {
            conversation
                .apply_client_action(
                    &ResponseStreamId::new_for_test(),
                    EntityId::new(),
                    action,
                    &SkillPathOrigin::Unavailable,
                    ctx,
                )
                .expect("move should apply");
        });

        // Both exchanges still resolve their own records across the task store.
        for (exchange_id, request_id) in [(first, "req-1"), (second, "req-2")] {
            let resolved: Vec<String> = conversation
                .request_metadata_records_for_exchange(exchange_id)
                .into_iter()
                .map(|record| record.request_id)
                .collect();
            assert_eq!(resolved, [request_id.to_string()]);
        }
        // The unaffected turn is still panel-eligible after the move.
        assert!(conversation.turn_panel_records(second).is_some());
    });
}

/// After a restore (or fork), the moved messages live in the summary subtask and the exchange
/// is rebuilt there rather than in the root: the non-root singleton turn fallback plus the
/// cross-task lookup must still resolve the record.
#[test]
fn records_resolve_in_restored_summarized_history() {
    use crate::ai::agent::task::TaskId;
    use crate::test_util::ai_agent_tasks::create_subagent_tool_call_message;

    // The persisted shape after a summarization move: the root keeps the summarization
    // sub-agent call (the replacement message the move inserted), and the moved turn —
    // including its record — lives in the summary subtask.
    let mut summarization_call = create_subagent_tool_call_message(
        "call-sum",
        "root",
        "summary-sub",
        Some(warp_multi_agent_api::message::tool_call::subagent::Metadata::Summarization(())),
    );
    summarization_call.request_id = "req-sum".to_string();
    summarization_call.timestamp = Some(timestamp(900));

    let root_task = api::Task {
        id: "root".to_string(),
        messages: vec![summarization_call],
        ..Default::default()
    };
    let subtask = api::Task {
        id: "summary-sub".to_string(),
        messages: turn_messages("req-1", 1_000),
        dependencies: Some(api::task::Dependencies {
            parent_task_id: "root".to_string(),
        }),
        ..Default::default()
    };
    let conversation =
        AIConversation::new_restored(AIConversationId::new(), vec![root_task, subtask], None)
            .expect("restored conversation");

    // The moved turn's exchange was rebuilt in the subtask, not the root.
    let subtask = conversation
        .all_tasks()
        .find(|task| task.id() == &TaskId::new("summary-sub".to_string()))
        .expect("summary subtask in the task store");
    let exchanges: Vec<_> = subtask.exchanges().collect();
    assert_eq!(
        exchanges.len(),
        1,
        "the subtask rebuilds exactly one exchange"
    );
    let exchange = exchanges[0];

    // A non-root exchange is its own turn (the singleton fallback)...
    assert_eq!(
        conversation.turn_exchange_ids(exchange.id),
        vec![exchange.id]
    );
    // ...and its record still resolves across the task store.
    let resolved: Vec<String> = conversation
        .request_metadata_records_for_exchange(exchange.id)
        .into_iter()
        .map(|record| record.request_id)
        .collect();
    assert_eq!(resolved, ["req-1".to_string()]);
    assert!(conversation.turn_panel_records(exchange.id).is_some());
}

fn with_timing(
    mut message: api::Message,
    started: i64,
    first_token: i64,
    ended: i64,
) -> api::Message {
    if let Some(api::message::Message::RequestMetadata(metadata)) = message.message.as_mut() {
        metadata.timing = Some(api::RequestTiming {
            request_timespan: Some(api::TimeSpan {
                started_at: Some(timestamp(started)),
                ended_at: Some(timestamp(ended)),
            }),
            first_token_at: Some(timestamp(first_token)),
            llm_generation_timespans: vec![api::TimeSpan {
                started_at: Some(timestamp(started)),
                ended_at: Some(timestamp(started + 1)),
            }],
        });
    }
    message
}

/// The Turn panel groups an exchange's records (one per underlying API request) into one summed
/// view: charges merge per model, timing spans the whole turn, tool counts add up, the context
/// window comes from the latest record, and the outcome surfaces the worst case.
#[test]
fn summarize_turn_sums_charges_timing_and_tools_across_records() {
    let records = vec![
        RequestMetadataRecord::from_message(&with_timing(
            record_message("req-1", request_metadata::Outcome::Completed, true),
            1_000,
            1_001,
            1_010,
        ))
        .unwrap(),
        RequestMetadataRecord::from_message(&with_timing(
            record_message("req-2", request_metadata::Outcome::Canceled, true),
            2_000,
            2_002,
            2_030,
        ))
        .unwrap(),
        RequestMetadataRecord::from_message(&with_timing(
            record_message("req-3", request_metadata::Outcome::Errored, false),
            3_000,
            3_004,
            3_060,
        ))
        .unwrap(),
    ];

    let summary = summarize_turn(&records);
    assert_eq!(summary.request_count, 3);
    assert_eq!(summary.interrupted_count, 2);
    assert_eq!(summary.outcome, RequestOutcome::Errored);

    // Both charged records use the same model, so they merge into one row with summed
    // tokens and costs; the errored record carried no charges.
    assert_eq!(summary.model_charges.len(), 1);
    assert_eq!(summary.total_tokens(), 2 * 1250);
    assert!((summary.inference_cost_in_cents() - 2.22).abs() < 1e-5);
    assert!((summary.platform_cost_in_cents() - 4.0).abs() < 1e-5);
    assert_eq!(summary.model_charges[0].web_search_count, 2);

    // Wall-clock span covers the whole turn; generation spans concatenate; the first token is
    // the earliest one. (Test timestamps are epoch seconds, so a 2_060 s span is 2_060_000 ms.)
    assert_eq!(summary.request_duration_ms(), Some(2_060_000));
    assert_eq!(summary.time_to_first_token_ms(), Some(1_000));
    // Each record contributes one 1-second LLM span.
    assert_eq!(summary.llm_generation_spans.len(), 3);

    // Tool counts add up (the helper stamps 4/2/3/40/8 on every record, including the errored
    // one, which keeps its tool summary); the context window is the latest record's reading.
    assert_eq!(summary.tool_calls, Some(12));
    assert_eq!(summary.commands_executed, Some(6));
    assert_eq!(summary.files_changed, Some(9));
    assert_eq!(summary.lines_added, Some(120));
    assert_eq!(summary.lines_removed, Some(24));
    assert_eq!(summary.context_window_usage, Some(42.0));
}

#[test]
fn summarize_turn_of_one_record_matches_the_record() {
    let record = RequestMetadataRecord::from_message(&record_message(
        "req-1",
        request_metadata::Outcome::Completed,
        true,
    ))
    .unwrap();
    let summary = summarize_turn(std::slice::from_ref(&record));
    assert_eq!(summary.request_count, 1);
    assert_eq!(summary.interrupted_count, 0);
    assert_eq!(summary.outcome, RequestOutcome::Completed);
    assert_eq!(summary.model_charges, record.model_charges);
    assert_eq!(summary.platform_charges, record.platform_charges);
    assert_eq!(
        summary.time_to_first_token_ms(),
        record.time_to_first_token_ms()
    );
    assert_eq!(summary.request_duration_ms(), record.request_duration_ms());
    assert_eq!(summary.tool_calls, record.tool_calls);
    assert_eq!(summary.context_window_usage, record.context_window_usage);
}
