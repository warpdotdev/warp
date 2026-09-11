use std::collections::{HashMap, HashSet};

use warp_multi_agent_api as api;
use warp_multi_agent_api::message::request_metadata;

use super::*;
use crate::ai::agent::conversation::{AIConversation, AIConversationId};
use crate::ai::agent::request_metadata::summarize_turn;

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

    let task = api::Task {
        id: "root".to_string(),
        messages: vec![
            message,
            record_message("req-1", request_metadata::Outcome::Completed, true),
            record_message("req-2", request_metadata::Outcome::Canceled, false),
        ],
        ..Default::default()
    };
    let records = RequestMetadataRecord::records_in_task(&task);
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
    assert_eq!(json["tool_call_summary"]["files_changed"], 3);
    assert_eq!(json["context_window"]["usage"], 42.0);
    // Must be a stable, pretty-printable document.
    assert!(
        serde_json::to_string_pretty(&json)
            .unwrap()
            .contains("\"tool_calls\": 4")
    );
}

#[test]
fn orphaned_summary_counts_only_server_records_missing_locally() {
    let server_records = vec![
        RequestMetadataRecord::from_message(&record_message(
            "req-1",
            request_metadata::Outcome::Completed,
            true,
        ))
        .unwrap(),
        RequestMetadataRecord::from_message(&record_message(
            "req-cancelled",
            request_metadata::Outcome::Canceled,
            true,
        ))
        .unwrap(),
        RequestMetadataRecord::from_message(&record_message(
            "req-errored",
            request_metadata::Outcome::Errored,
            false,
        ))
        .unwrap(),
    ];
    let local_request_ids: HashSet<String> = HashSet::from(["req-1".to_string()]);

    let summary = summarize_orphaned_records(&server_records, &local_request_ids);
    assert_eq!(summary.server_record_count, 3);
    assert_eq!(
        summary.count, 2,
        "the cancelled and errored turns never reached the client"
    );
    // Only the cancelled turn was charged anything; the errored one carries no charges.
    assert!((summary.total_cost_in_cents - 3.11).abs() < 1e-5);

    let none_missing = summarize_orphaned_records(
        &server_records,
        &HashSet::from([
            "req-1".to_string(),
            "req-cancelled".to_string(),
            "req-errored".to_string(),
        ]),
    );
    assert_eq!(
        none_missing,
        OrphanedRequestSummary {
            count: 0,
            total_cost_in_cents: 0.0,
            server_record_count: 3
        }
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

    assert_eq!(
        conversation.request_metadata_request_ids(),
        HashSet::from([
            "req-1".to_string(),
            "req-2".to_string(),
            "req-4".to_string()
        ])
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

    let summary = summarize_turn(&conversation.request_metadata_records_for_turn(third));
    assert_eq!(summary.request_count, 3);
    assert!((summary.total_cost_in_cents() - 3.0 * 3.11).abs() < 1e-4);
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
    assert_eq!(summary.llm_generation_ms(), Some(3_000));

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
    assert_eq!(summary.total_cost_in_cents(), record.total_cost_in_cents());
    assert_eq!(
        summary.time_to_first_token_ms(),
        record.time_to_first_token_ms()
    );
    assert_eq!(summary.request_duration_ms(), record.request_duration_ms());
    assert_eq!(summary.llm_generation_ms(), record.llm_generation_ms());
    assert_eq!(summary.tool_calls, record.tool_calls);
    assert_eq!(summary.context_window_usage, record.context_window_usage);
}
