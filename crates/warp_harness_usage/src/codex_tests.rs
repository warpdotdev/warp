use serde_json::{Value, json};

use crate::{
    CaptureDiagnostics, CoverageStatus, ExtractionOutcome, JsonlDiagnostics, JsonlReadStatus,
    ReasonCode, UsageSnapshot, extract_codex, parse_jsonl,
};

fn capture(entries: &[Value]) -> UsageSnapshot {
    let diagnostics = CaptureDiagnostics {
        root: JsonlDiagnostics {
            status: JsonlReadStatus::Readable,
            records_read: entries.len(),
            ..Default::default()
        },
        ..Default::default()
    };
    let ExtractionOutcome::Usable(snapshot) = extract_codex("root", entries, &diagnostics) else {
        panic!("expected usable capture");
    };
    *snapshot
}

#[test]
fn optional_category_drift_preserves_latest_totals_without_inventing_a_baseline() {
    let snapshot = capture(&[
        json!({"type":"turn_context","payload":{"model":"codex-a"}}),
        checkpoint(10, 10),
        json!({"type":"event_msg","payload":{"type":"token_count","info":{
            "total_token_usage":{"input_tokens":15,"output_tokens":5,"total_tokens":20},
            "last_token_usage":{"input_tokens":7,"output_tokens":3,"total_tokens":10}
        }}}),
        checkpoint(30, 10),
        json!({"type":"event_msg","payload":{"type":"token_count","info":{
            "total_token_usage":{"input_tokens":30,"output_tokens":10,"total_tokens":40},
            "last_token_usage":{"input_tokens":8,"output_tokens":2,"total_tokens":10}
        }}}),
    ]);
    let payload = serde_json::to_value(&snapshot.payload).unwrap();
    assert_eq!(payload["usage"], json!({"input_tokens":30,"output_tokens":10,"total_tokens":40}));
    assert_eq!(
        payload["attribution"],
        json!([
            {"usage":{"input_tokens":30,"output_tokens":10}},
            {"model":"codex-a","usage":{"total_tokens":40}}
        ])
    );
    assert_eq!(snapshot.coverage.token_status, CoverageStatus::Partial);
    assert!(!snapshot.coverage.reason_codes.contains_key(&ReasonCode::AmbiguousAccounting));
}

fn checkpoint(total: i64, last: i64) -> Value {
    json!({"type":"event_msg","payload":{"type":"token_count","info":{
        "total_token_usage":{"total_tokens":total},
        "last_token_usage":{"total_tokens":last}
    }}})
}

#[test]
fn cumulative_checkpoints_preserve_distinct_equal_sized_requests() {
    let entries = parse_jsonl(include_bytes!("fixtures/codex.jsonl").as_slice());
    let snapshot = capture(&entries.entries);
    assert_eq!(snapshot.coverage.token_status, CoverageStatus::Known);
    assert_eq!(snapshot.coverage.tool_status, CoverageStatus::Known);
    assert_eq!(snapshot.coverage.captured_scope, "root_rollout_only");
    assert_eq!(
        serde_json::to_value(&snapshot.payload).unwrap(),
        serde_json::from_str::<Value>(include_str!("fixtures/codex_payload.json")).unwrap()
    );
}

#[test]
fn unexplained_decrease_retains_only_the_unambiguous_prefix() {
    let snapshot = capture(&[
        checkpoint(100, 10),
        checkpoint(20, 20),
        checkpoint(120, 100),
    ]);
    assert_eq!(
        serde_json::to_value(&snapshot.payload).unwrap()["usage"],
        json!({"total_tokens":100})
    );
    assert_eq!(snapshot.coverage.token_status, CoverageStatus::Partial);
    assert_eq!(
        snapshot.coverage.reason_codes[&ReasonCode::AmbiguousAccounting],
        1
    );
}

#[test]
fn native_session_transition_allows_an_independent_counter_segment() {
    let snapshot = capture(&[
        json!({"type":"session_meta","payload":{"id":"root"}}),
        checkpoint(100, 10),
        json!({"type":"response_item","payload":{"type":"function_call","call_id":"a","name":"shell"}}),
        json!({"type":"session_meta","payload":{"id":"next"}}),
        checkpoint(20, 20),
        json!({"type":"response_item","payload":{"type":"function_call","call_id":"a","name":"shell"}}),
        json!({"type":"session_meta","payload":{"id":"next"}}),
        checkpoint(20, 20),
    ]);
    let payload = serde_json::to_value(&snapshot.payload).unwrap();
    assert_eq!(payload["usage"], json!({"total_tokens":120}));
    assert_eq!(
        payload["toolCalls"],
        json!({"total":2,"byName":{"shell":2}})
    );
    assert_eq!(snapshot.coverage.token_status, CoverageStatus::Known);
}

#[test]
fn mismatching_last_usage_is_not_attributed_to_the_current_model() {
    let snapshot = capture(&[
        json!({"type":"turn_context","payload":{"model":"codex-a"}}),
        checkpoint(100, 10),
        checkpoint(120, 10),
    ]);
    let payload = serde_json::to_value(&snapshot.payload).unwrap();
    assert_eq!(payload["usage"], json!({"total_tokens":120}));
    assert_eq!(
        payload["attribution"],
        json!([{"usage":{"total_tokens":120}}])
    );
    assert_eq!(snapshot.coverage.token_status, CoverageStatus::Partial);
}

#[test]
fn conflicting_and_unidentified_invocations_do_not_fabricate_calls() {
    let snapshot = capture(&[
        json!({"type":"response_item","payload":{"type":"function_call","call_id":"a","name":"shell"}}),
        json!({"type":"response_item","payload":{"type":"custom_tool_call","call_id":"a","name":"apply_patch"}}),
        json!({"type":"response_item","payload":{"type":"custom_tool_call","call_id":"b","name":"apply_patch"}}),
        json!({"type":"response_item","payload":{"type":"function_call","name":"shell"}}),
        json!({"type":"response_item","payload":{"type":"function_call_output","call_id":"b"}}),
        json!({"type":"response_item","payload":{"type":"unsupported_call","call_id":"c","name":"search"}}),
    ]);
    assert_eq!(
        serde_json::to_value(&snapshot.payload).unwrap()["toolCalls"],
        json!({"total":1,"byName":{"apply_patch":1}})
    );
    assert_eq!(snapshot.coverage.tool_status, CoverageStatus::Partial);
    assert_eq!(snapshot.coverage.token_status, CoverageStatus::Unavailable);
}

#[test]
fn counter_bounds_and_absence_survive_serialization() {
    let snapshot = capture(&[checkpoint(9007199254740993, 9007199254740993)]);
    assert_eq!(
        serde_json::to_value(&snapshot.payload).unwrap()["usage"],
        json!({"total_tokens":9007199254740993_i64})
    );
    let overflow = capture(&[
        json!({"type":"session_meta","payload":{"id":"root"}}),
        checkpoint(i64::MAX, i64::MAX),
        json!({"type":"session_meta","payload":{"id":"next"}}),
        checkpoint(1, 1),
    ]);
    assert_eq!(overflow.coverage.token_status, CoverageStatus::Unavailable);
    assert!(
        overflow
            .coverage
            .reason_codes
            .contains_key(&ReasonCode::ResourceLimit)
    );
    assert!(
        serde_json::to_value(&overflow.payload)
            .unwrap()
            .get("usage")
            .is_none()
    );
}

#[test]
fn malformed_usage_cannot_establish_known_zero() {
    let snapshot = capture(&[
        json!({"type":"event_msg","payload":{"type":"token_count","info":{}}}),
        json!({"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1,"cached_input_tokens":2}}}}),
        checkpoint(0, 0),
    ]);
    assert_eq!(
        serde_json::to_value(&snapshot.payload).unwrap()["usage"],
        json!({"total_tokens":0})
    );
    assert_eq!(snapshot.coverage.token_status, CoverageStatus::Partial);
    assert!(matches!(
        extract_codex("root", &[], &CaptureDiagnostics::default()),
        ExtractionOutcome::Unavailable(_)
    ));
}

#[test]
fn status_only_token_events_do_not_degrade_valid_usage() {
    let snapshot = capture(&[
        json!({"type":"event_msg","payload":{"type":"token_count","info":null}}),
        checkpoint(10, 10),
    ]);
    assert_eq!(
        serde_json::to_value(&snapshot.payload).unwrap()["usage"],
        json!({"total_tokens":10})
    );
    assert_eq!(snapshot.coverage.token_status, CoverageStatus::Known);
}
