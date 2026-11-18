use chrono::{TimeZone, Utc};
use serde_json::{Value, json};

use super::*;
use crate::api::{CoverageStatus, HarnessUsageRequest};
use crate::{JsonlDiagnostics, JsonlReadStatus};

fn extract(entries: &[Value], incomplete: bool) -> PiUsageSnapshot {
    let outcome = extract_pi(
        "session",
        entries,
        &CaptureDiagnostics {
            root: JsonlDiagnostics {
                status: JsonlReadStatus::Readable,
                incomplete_trailing_record: incomplete,
                ..Default::default()
            },
            ..Default::default()
        },
    );
    let ExtractionOutcome::Usable(extracted) = outcome else {
        panic!("expected usable snapshot");
    };
    let HarnessUsageSnapshot::Pi(snapshot) = extracted.snapshot else {
        panic!("expected Pi snapshot");
    };
    snapshot
}

fn assistant(id: &str, cost: Value) -> Value {
    json!({"type":"message", "id":id, "message":{
        "role":"assistant", "provider":"openrouter", "model":"vendor/model",
        "content":[{"type":"toolCall","id":format!("tool-{id}"),"name":"bash"}],
        "usage":{"input":10,"output":2,"cacheRead":30,"cacheWrite":4,
            "totalTokens":46,"cost":cost}
    }})
}

#[test]
fn duplicate_entries_count_once_and_dollar_metadata_does_not_affect_tokens() {
    let billed = assistant("a", json!({"total":0.12,"totalFromProvider":true}));
    let entries = vec![
        billed.clone(),
        billed,
        assistant("b", json!({"total":0.09})),
        assistant("c", Value::Null),
        assistant("d", json!({"total":0})),
        assistant("e", json!({"total":0,"totalFromProvider":true})),
    ];
    let snapshot = extract(&entries, false);
    assert_eq!(snapshot.payload.attribution.len(), 1);
    assert_eq!(snapshot.payload.tool_calls.unwrap().total, 5);
    assert_eq!(snapshot.coverage.token_status, CoverageStatus::Known);
    let group = &snapshot.payload.attribution[0];
    assert_eq!(group.usage.input, Some(50));
    assert_eq!(group.max_input_tokens, Some(44));
}

#[test]
fn accounts_for_summaries_standalone_and_nested_usage_with_partial_capture() {
    let usage = json!({"input":1,"output":2,"cacheRead":3,"cacheWrite":4,
        "totalTokens":10,"cost":{"total":0.01}});
    let snapshot = extract(
        &[
            json!({"type":"compaction","id":"a","usage":usage}),
            json!({"type":"branch_summary","id":"b","usage":usage}),
            json!({"type":"usage","id":"c","provider":"google","model":"gemini","usage":usage}),
            json!({"type":"message","id":"d","message":{"role":"toolResult","usage":usage}}),
        ],
        true,
    );
    assert_eq!(snapshot.payload.attribution.len(), 2);
    assert_eq!(snapshot.coverage.token_status, CoverageStatus::Partial);
    let unattributed = snapshot
        .payload
        .attribution
        .iter()
        .find(|group| group.provider.is_none())
        .unwrap();
    assert_eq!(unattributed.usage.input, Some(3));
}

#[test]
fn conflicting_ids_leave_partial_usage() {
    let mut conflict = assistant("a", json!({"total":1}));
    conflict["message"]["usage"]["input"] = json!(50);
    let snapshot = extract(
        &[
            assistant("a", json!({"total":1})),
            conflict,
            assistant("b", json!({"total":-1})),
        ],
        false,
    );
    assert_eq!(snapshot.coverage.token_status, CoverageStatus::Partial);
    assert_eq!(snapshot.payload.attribution[0].usage.input, Some(10));
}

#[test]
fn token_usage_matches_contract() {
    let snapshot = extract(&[assistant("a", Value::Null)], false);
    let request = HarnessUsageRequest::new(
        7,
        3,
        Utc.with_ymd_and_hms(2026, 1, 1, 12, 0, 0).unwrap(),
        HarnessUsageSnapshot::Pi(snapshot),
    );
    assert!(request.has_usable_category());
    assert_eq!(
        serde_json::to_value(request).unwrap(),
        serde_json::from_str::<Value>(include_str!("fixtures/api/pi.json")).unwrap()
    );
}

#[test]
fn overflowing_disjoint_tokens_do_not_produce_a_publishable_snapshot() {
    let mut entry = assistant("a", json!({"total":0.1}));
    entry["message"]["usage"]["input"] = json!(i64::MAX);
    assert!(matches!(
        extract_pi(
            "session",
            &[entry],
            &CaptureDiagnostics {
                root: JsonlDiagnostics {
                    status: JsonlReadStatus::Readable,
                    ..Default::default()
                },
                ..Default::default()
            }
        ),
        ExtractionOutcome::Unavailable(_)
    ));
}
