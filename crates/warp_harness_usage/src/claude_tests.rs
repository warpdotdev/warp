use std::ops::Deref;

use serde_json::{Value, json};

use crate::api::{ClaudeUsage, CoverageStatus, HarnessUsageSnapshot, UsageSnapshot};
use crate::{
    CaptureDiagnostics, ExtractionDiagnostics, ExtractionOutcome, JsonlDiagnostics, JsonlLimits,
    JsonlReadStatus, ReasonCode, extract_claude, parse_jsonl,
};

const JSONL_LIMITS: JsonlLimits = JsonlLimits {
    max_line_bytes: 4 * 1024,
    max_total_bytes: 16 * 1024,
    max_records: 16,
};

struct TestCapture {
    snapshot: UsageSnapshot<ClaudeUsage>,
    diagnostics: ExtractionDiagnostics,
}

impl Deref for TestCapture {
    type Target = UsageSnapshot<ClaudeUsage>;

    fn deref(&self) -> &Self::Target {
        &self.snapshot
    }
}

fn capture(entries: &[Value]) -> TestCapture {
    let diagnostics = CaptureDiagnostics {
        root: JsonlDiagnostics {
            status: JsonlReadStatus::Readable,
            records_read: entries.len(),
            ..Default::default()
        },
        ..Default::default()
    };
    usable(extract_claude("root", entries, [], &diagnostics))
}

fn usable(outcome: ExtractionOutcome) -> TestCapture {
    let ExtractionOutcome::Usable(extracted) = outcome else {
        panic!("expected usable capture");
    };
    let extracted = *extracted;
    let HarnessUsageSnapshot::ClaudeCode(snapshot) = extracted.snapshot else {
        unreachable!()
    };
    TestCapture {
        snapshot,
        diagnostics: extracted.diagnostics,
    }
}

#[test]
fn a_category_missing_from_one_response_makes_observed_totals_partial() {
    let snapshot = capture(&[
        response("a", json!({"input_tokens":10,"output_tokens":1}), json!([])),
        response("b", json!({"output_tokens":2}), json!([])),
    ]);
    assert_eq!(
        serde_json::to_value(&snapshot.payload).unwrap()["usage"],
        json!({"input_tokens":10,"output_tokens":3})
    );
    assert_eq!(snapshot.coverage.token_status, CoverageStatus::Partial);
}

#[test]
fn conflicting_tool_names_leave_response_usage_unchanged() {
    let snapshot = capture(&[
        response(
            "a",
            json!({"input_tokens":4}),
            json!([{"type":"tool_use","id":"t","name":"Read"}]),
        ),
        response(
            "a",
            json!({"input_tokens":4}),
            json!([{"type":"tool_use","id":"t","name":"Write"},{"type":"tool_use","id":"u","name":"Read"}]),
        ),
    ]);
    let payload = serde_json::to_value(&snapshot.payload).unwrap();
    assert_eq!(payload["usage"], json!({"input_tokens":4}));
    assert_eq!(payload["toolCalls"], json!({"total":1,"byName":{"Read":1}}));
    assert_eq!(snapshot.coverage.token_status, CoverageStatus::Known);
    assert_eq!(snapshot.coverage.tool_status, CoverageStatus::Partial);
}

fn response(id: &str, usage: Value, content: Value) -> Value {
    json!({"type":"assistant","message":{"id":id,"model":"claude-a","usage":usage,"content":content}})
}

#[test]
fn nullable_cache_fields_preserve_known_usage_attribution_and_tools() {
    for (field, expected) in [
        (
            "cache_creation",
            json!({"input_tokens":100,"output_tokens":0,"cache_read_input_tokens":4096,"cache_creation_input_tokens":0}),
        ),
        (
            "cache_read_input_tokens",
            json!({"input_tokens":100,"output_tokens":0,"cache_creation_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0}}),
        ),
        (
            "cache_creation_input_tokens",
            json!({"input_tokens":100,"output_tokens":0,"cache_read_input_tokens":4096,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0}}),
        ),
    ] {
        let mut usage = json!({
            "input_tokens":100,"output_tokens":0,"cache_read_input_tokens":4096,
            "cache_creation_input_tokens":0,
            "cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},
            "service_tier":"standard"
        });
        usage[field] = Value::Null;
        let snapshot = capture(&[response(
            "a",
            usage,
            json!([{"type":"tool_use","id":"t","name":"Read"}]),
        )]);
        assert_eq!(
            serde_json::to_value(&snapshot.payload).unwrap(),
            json!({
                "usage":expected,
                "attribution":[{"model":"claude-a","service_tier":"standard","usage":expected}],
                "toolCalls":{"total":1,"byName":{"Read":1}}
            }),
            "{field}"
        );
        assert_eq!(snapshot.coverage.token_status, CoverageStatus::Known);
        assert_eq!(snapshot.coverage.tool_status, CoverageStatus::Known);
        assert!(snapshot.diagnostics.reasons.is_empty(), "{field}");
    }
}

#[test]
fn null_cache_counter_in_one_response_makes_its_observed_total_partial() {
    let snapshot = capture(&[
        response(
            "a",
            json!({"input_tokens":10,"output_tokens":1,"cache_read_input_tokens":100}),
            json!([]),
        ),
        response(
            "b",
            json!({"input_tokens":20,"output_tokens":2,"cache_read_input_tokens":null}),
            json!([]),
        ),
    ]);
    assert_eq!(
        serde_json::to_value(&snapshot.payload).unwrap()["usage"],
        json!({"input_tokens":30,"output_tokens":3,"cache_read_input_tokens":100})
    );
    assert_eq!(snapshot.coverage.token_status, CoverageStatus::Partial);
    assert_eq!(snapshot.coverage.tool_status, CoverageStatus::Known);
    assert_eq!(
        snapshot.diagnostics.reasons,
        [(ReasonCode::IncompleteInput, 1)].into()
    );
}

#[test]
fn later_null_cache_fields_do_not_replace_a_complete_response() {
    let known = json!({
        "input_tokens":100,"output_tokens":2,"cache_read_input_tokens":4096,
        "cache_creation_input_tokens":15,
        "cache_creation":{"ephemeral_5m_input_tokens":10,"ephemeral_1h_input_tokens":5}
    });
    for field in [
        "cache_creation",
        "cache_read_input_tokens",
        "cache_creation_input_tokens",
    ] {
        let mut later = known.clone();
        later[field] = Value::Null;
        let snapshot = capture(&[
            response("a", known.clone(), json!([])),
            response("a", later, json!([])),
        ]);
        assert_eq!(
            serde_json::to_value(&snapshot.payload).unwrap()["usage"],
            json!({
                "input_tokens":100,"output_tokens":2,"cache_read_input_tokens":4096,
                "cache_creation_input_tokens":15,
                "cache_creation":{"ephemeral_5m_input_tokens":10,"ephemeral_1h_input_tokens":5}
            }),
            "{field}"
        );
        assert_eq!(snapshot.coverage.token_status, CoverageStatus::Known);
        assert!(snapshot.diagnostics.reasons.is_empty(), "{field}");
    }
}

#[test]
fn nullable_cache_fields_do_not_relax_other_counter_validation() {
    for (path, value) in [
        ("/cache_read_input_tokens", json!("0")),
        ("/cache_read_input_tokens", json!(-1)),
        ("/cache_creation_input_tokens", json!("0")),
        ("/cache_creation_input_tokens", json!(-1)),
        ("/cache_creation", json!("0")),
        ("/cache_creation", json!(-1)),
        ("/input_tokens", Value::Null),
        ("/output_tokens", Value::Null),
        ("/cache_creation/ephemeral_5m_input_tokens", Value::Null),
        ("/cache_creation/ephemeral_1h_input_tokens", Value::Null),
    ] {
        let mut usage = json!({
            "input_tokens":100,"output_tokens":0,"cache_read_input_tokens":4096,
            "cache_creation_input_tokens":0,
            "cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0}
        });
        *usage.pointer_mut(path).unwrap() = value;
        let snapshot = capture(&[response("a", usage, json!([]))]);
        assert_eq!(
            serde_json::to_value(&snapshot.payload).unwrap(),
            json!({"toolCalls":{"total":0,"byName":{}}}),
            "{path}"
        );
        assert_eq!(snapshot.coverage.token_status, CoverageStatus::Unavailable);
        assert_eq!(snapshot.coverage.tool_status, CoverageStatus::Known);
        assert_eq!(snapshot.diagnostics.reasons[&ReasonCode::InvalidData], 1);
    }
}

#[test]
fn evolving_responses_do_not_deduplicate_distinct_tool_blocks() {
    let entries = parse_jsonl(
        include_bytes!("fixtures/claude.jsonl").as_slice(),
        JSONL_LIMITS,
    );
    let snapshot = capture(&entries.entries);
    assert_eq!(snapshot.coverage.token_status, CoverageStatus::Partial);
    assert_eq!(snapshot.coverage.tool_status, CoverageStatus::Known);
    assert_eq!(
        serde_json::to_value(&snapshot.payload).unwrap(),
        serde_json::from_str::<Value>(include_str!("fixtures/claude_payload.json")).unwrap()
    );
}

#[test]
fn known_metadata_does_not_degrade_usage_coverage() {
    let snapshot = capture(&[
        json!({"type":"permission-mode","mode":"default"}),
        json!({"type":"attachment","attachment":{"type":"deferred_tools_delta"}}),
        json!({"type":"ai-title","title":"A title"}),
        response("a", json!({"input_tokens":10}), json!([])),
    ]);
    assert_eq!(
        serde_json::to_value(&snapshot.payload).unwrap()["usage"],
        json!({"input_tokens":10})
    );
    assert_eq!(snapshot.coverage.token_status, CoverageStatus::Known);
    assert_eq!(snapshot.coverage.tool_status, CoverageStatus::Known);
}

#[test]
fn empty_optional_classification_is_unknown() {
    let snapshot = capture(&[json!({
        "type":"assistant",
        "message": {
            "id":"a",
            "model":"claude-a",
            "usage":{"input_tokens":10,"inference_geo":""},
            "content":[]
        }
    })]);
    let payload = serde_json::to_value(&snapshot.payload).unwrap();
    assert_eq!(snapshot.coverage.token_status, CoverageStatus::Known);
    assert_eq!(payload["usage"], json!({"input_tokens":10}));
    assert_eq!(
        payload["attribution"],
        json!([{"model":"claude-a","usage":{"input_tokens":10}}])
    );
}

#[test]
fn conflicting_response_does_not_discard_independent_tool_data() {
    let snapshot = capture(&[
        response("a", json!({"input_tokens":10,"output_tokens":3}), json!([])),
        response("a", json!({"input_tokens":9,"output_tokens":4}), json!([])),
        response(
            "b",
            json!({"input_tokens":2}),
            json!([{"type":"tool_use","id":"t","name":"Read"}]),
        ),
        response("", json!({"input_tokens":900}), json!([])),
    ]);
    let payload = serde_json::to_value(&snapshot.payload).unwrap();
    assert_eq!(payload["usage"], json!({"input_tokens":2}));
    assert_eq!(payload["toolCalls"], json!({"total":1,"byName":{"Read":1}}));
    assert_eq!(snapshot.coverage.token_status, CoverageStatus::Partial);
    assert_eq!(snapshot.coverage.tool_status, CoverageStatus::Known);
    assert_eq!(
        snapshot.diagnostics.reasons[&ReasonCode::AmbiguousAccounting],
        1
    );
}

#[test]
fn overlapping_partial_vectors_are_not_reconstructed_from_maxima() {
    let snapshot = capture(&[
        response("a", json!({"input_tokens":10,"output_tokens":5}), json!([])),
        response("a", json!({"input_tokens":11}), json!([])),
        response("b", json!({"output_tokens":2}), json!([])),
    ]);
    assert_eq!(
        serde_json::to_value(&snapshot.payload).unwrap()["usage"],
        json!({"output_tokens":2})
    );
    assert_eq!(snapshot.coverage.token_status, CoverageStatus::Partial);
}

#[test]
fn overflow_omits_the_counter_without_rounding_large_integers() {
    let snapshot = capture(&[
        response(
            "a",
            json!({"input_tokens":9223372036854775807_i64,"output_tokens":9007199254740993_i64}),
            json!([]),
        ),
        response("b", json!({"input_tokens":1,"output_tokens":0}), json!([])),
        response("c", json!({"input_tokens":-1}), json!([])),
        response(
            "d",
            json!({"input_tokens":9223372036854775808_u64}),
            json!([]),
        ),
    ]);
    assert_eq!(
        serde_json::to_value(&snapshot.payload).unwrap()["usage"],
        json!({"output_tokens":9007199254740993_i64})
    );
    assert_eq!(snapshot.coverage.token_status, CoverageStatus::Partial);
    assert!(
        snapshot
            .diagnostics
            .reasons
            .contains_key(&ReasonCode::ResourceLimit)
    );
    assert!(
        snapshot
            .diagnostics
            .reasons
            .contains_key(&ReasonCode::InvalidData)
    );
}

#[test]
fn subagent_counts_are_included_without_claiming_unreadable_scope() {
    let root = vec![response("a", json!({"input_tokens":10}), json!([]))];
    let child = vec![response("b", json!({"input_tokens":20}), json!([]))];
    let diagnostics = CaptureDiagnostics {
        root: JsonlDiagnostics {
            status: JsonlReadStatus::Readable,
            ..Default::default()
        },
        subagents: [
            (
                "agent-a".to_owned(),
                JsonlDiagnostics {
                    status: JsonlReadStatus::Readable,
                    ..Default::default()
                },
            ),
            ("agent-b".to_owned(), JsonlDiagnostics::default()),
        ]
        .into(),
        subagent_discovery_incomplete: true,
    };
    let snapshot = usable(extract_claude(
        "root",
        &root,
        [("agent-a", child.as_slice())],
        &diagnostics,
    ));
    assert_eq!(
        serde_json::to_value(&snapshot.payload).unwrap()["usage"],
        json!({"input_tokens":30})
    );
    assert_eq!(snapshot.coverage.token_status, CoverageStatus::Partial);
    assert_eq!(snapshot.coverage.tool_status, CoverageStatus::Unavailable);
    assert!(
        snapshot
            .diagnostics
            .reasons
            .contains_key(&ReasonCode::IncompleteInput)
    );
}

#[test]
fn readable_empty_is_not_missing_or_an_oversized_scope() {
    let snapshot = capture(&[]);
    assert_eq!(snapshot.coverage.token_status, CoverageStatus::Unavailable);
    assert_eq!(
        serde_json::to_value(&snapshot.payload).unwrap(),
        json!({"toolCalls":{"total":0,"byName":{}}})
    );
    assert!(matches!(
        extract_claude("root", &[], [], &CaptureDiagnostics::default()),
        ExtractionOutcome::Unavailable(_)
    ));
    let scope = "x".repeat(257);
    assert!(matches!(
        extract_claude(&scope, &[], [], &CaptureDiagnostics::default()),
        ExtractionOutcome::Unavailable(_)
    ));
}

#[test]
fn synthetic_messages_and_tool_results_do_not_add_usage() {
    let snapshot = capture(&[
        json!({"type":"assistant","message":{"id":"fake","model":"<synthetic>","usage":{"input_tokens":100},"content":[{"type":"tool_use","id":"t","name":"Read"}]}}),
        json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t"}]}}),
    ]);
    assert_eq!(
        serde_json::to_value(&snapshot.payload).unwrap(),
        json!({"toolCalls":{"total":0,"byName":{}}})
    );
}
