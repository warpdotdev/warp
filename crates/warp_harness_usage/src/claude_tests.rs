use serde_json::{Value, json};

use super::*;
use crate::api::{CostStatus, CoverageStatus};
use crate::{JsonlDiagnostics, JsonlReadStatus};

fn response(id: &str, input: i64, output: i64) -> Value {
    json!({"type":"assistant","message":{"id":id,"model":"claude-a","content":[],
        "usage":{"input_tokens":input,"cache_read_input_tokens":0,"cache_creation_input_tokens":0,"output_tokens":output}}})
}
fn diagnostics() -> CaptureDiagnostics {
    CaptureDiagnostics {
        root: JsonlDiagnostics {
            status: JsonlReadStatus::Readable,
            ..Default::default()
        },
        ..Default::default()
    }
}
fn policy() -> ThresholdPolicy {
    ThresholdPolicy::parse(
        json!({"schema_version":1,"models":{"claude-a":{"kind":"input_gt","tokens":100}}}),
    )
    .unwrap()
}
fn capture(entries: &[Value]) -> crate::api::UsageSnapshot<ClaudeUsage> {
    let ExtractionOutcome::Usable(result) =
        extract_claude("root", entries, [], &diagnostics(), Some(&policy()))
    else {
        panic!("unavailable")
    };
    let HarnessUsageSnapshot::ClaudeCode(snapshot) = result.snapshot else {
        panic!("wrong provider")
    };
    snapshot
}
#[test]
fn streaming_revisions_replace_usage_but_distinct_responses_add() {
    let snapshot = capture(&[
        response("a", 50, 0),
        response("a", 50, 5),
        response("b", 50, 5),
    ]);
    assert_eq!(snapshot.payload.output_tokens, Some(10));
    let group = &snapshot.payload.cost_estimation.unwrap().groups[0];
    assert_eq!(
        group.pre_threshold.as_ref().unwrap().input_tokens,
        Some(100)
    );
    assert_eq!(snapshot.coverage.cost_status, CostStatus::Known);
}
#[test]
fn threshold_is_strict_and_includes_cache_reads_and_creation() {
    let mut cached = response("c", 50, 3);
    cached["message"]["usage"]["cache_read_input_tokens"] = json!(30);
    cached["message"]["usage"]["cache_creation_input_tokens"] = json!(21);
    cached["message"]["usage"]["cache_creation"] =
        json!({"ephemeral_5m_input_tokens":1,"ephemeral_1h_input_tokens":20});
    let snapshot = capture(&[response("a", 99, 1), response("b", 100, 2), cached]);
    let group = &snapshot.payload.cost_estimation.unwrap().groups[0];
    assert_eq!(
        group.pre_threshold.as_ref().unwrap().input_tokens,
        Some(199)
    );
    assert_eq!(
        group.post_threshold.as_ref().unwrap().input_tokens,
        Some(50)
    );
    assert_eq!(
        group
            .post_threshold
            .as_ref()
            .unwrap()
            .cache_creation
            .as_ref()
            .unwrap()
            .ephemeral_1h_input_tokens,
        Some(20)
    );
}
#[test]
fn missing_ttl_split_disables_cost_not_known_output_or_tools() {
    let mut entry = response("a", 50, 5);
    entry["message"]["usage"]["cache_creation_input_tokens"] = json!(10);
    entry["message"]["content"] = json!([{"type":"tool_use","id":"tool","name":"Read"}]);
    let snapshot = capture(&[entry]);
    assert_eq!(snapshot.payload.cost_estimation, None);
    assert_eq!(snapshot.payload.output_tokens, Some(5));
    assert_eq!(snapshot.coverage.output_token_status, CoverageStatus::Known);
    assert_eq!(snapshot.payload.tool_calls.unwrap().total, 1);
}

#[test]
fn contradictory_cache_partitions_do_not_discard_observed_output() {
    let mut entry = response("a", 50, 5);
    entry["message"]["usage"]["cache_creation_input_tokens"] = json!(10);
    entry["message"]["usage"]["cache_creation"] =
        json!({"ephemeral_5m_input_tokens":1,"ephemeral_1h_input_tokens":2});
    let snapshot = capture(&[entry]);
    assert_eq!(snapshot.payload.cost_estimation, None);
    assert_eq!(snapshot.payload.output_tokens, Some(5));
    assert_eq!(
        snapshot.coverage.output_token_status,
        CoverageStatus::Partial
    );
}
#[test]
fn conflicting_identity_keeps_independent_tool_counts() {
    let mut entry = response("a", 50, 5);
    entry["message"]["content"] = json!([{"type":"tool_use","id":"tool","name":"Read"}]);
    let snapshot = capture(&[entry, response("a", 49, 6), response("b", 2, 0)]);
    assert_eq!(snapshot.payload.cost_estimation, None);
    assert_eq!(snapshot.payload.output_tokens, Some(0));
    assert_eq!(
        snapshot.coverage.output_token_status,
        CoverageStatus::Partial
    );
    assert_eq!(snapshot.payload.tool_calls.unwrap().total, 1);
}
#[test]
fn subagents_are_included_and_capture_holes_disable_cost() {
    let child = vec![response("b", 10, 2)];
    let mut diagnostics = diagnostics();
    diagnostics
        .subagents
        .insert("child".into(), diagnostics.root.clone());
    let root = vec![response("a", 10, 1)];
    let ExtractionOutcome::Usable(result) = extract_claude(
        "root",
        &root,
        [("child", child.as_slice())],
        &diagnostics,
        Some(&policy()),
    ) else {
        panic!("unavailable")
    };
    let HarnessUsageSnapshot::ClaudeCode(snapshot) = result.snapshot else {
        panic!("wrong provider")
    };
    assert_eq!(snapshot.payload.output_tokens, Some(3));
    assert_eq!(snapshot.coverage.cost_status, CostStatus::Known);
    diagnostics.subagent_discovery_incomplete = true;
    let ExtractionOutcome::Usable(result) = extract_claude(
        "root",
        &root,
        [("child", child.as_slice())],
        &diagnostics,
        Some(&policy()),
    ) else {
        panic!("unavailable")
    };
    let HarnessUsageSnapshot::ClaudeCode(snapshot) = result.snapshot else {
        panic!("wrong provider")
    };
    assert_eq!(snapshot.payload.cost_estimation, None);
    assert_eq!(snapshot.payload.output_tokens, Some(3));
}
#[test]
fn missing_policy_or_unknown_model_is_not_a_local_threshold_guess() {
    let entry = response("a", 200, 5);
    let ExtractionOutcome::Usable(result) = extract_claude(
        "root",
        std::slice::from_ref(&entry),
        [],
        &diagnostics(),
        None,
    ) else {
        panic!("unavailable")
    };
    let HarnessUsageSnapshot::ClaudeCode(snapshot) = result.snapshot else {
        panic!("wrong provider")
    };
    assert_eq!(snapshot.payload.cost_estimation, None);
    assert_eq!(snapshot.payload.output_tokens, Some(5));
    let mut unknown = entry;
    unknown["message"]["model"] = json!("unknown");
    assert_eq!(capture(&[unknown]).payload.cost_estimation, None);
}
#[test]
fn readable_empty_is_measured_zero_but_missing_is_unavailable() {
    let snapshot = capture(&[]);
    assert_eq!(snapshot.payload.output_tokens, Some(0));
    assert_eq!(snapshot.payload.cost_estimation.unwrap().groups.len(), 0);
    let ExtractionOutcome::Usable(result) = extract_claude(
        "root",
        &[],
        [],
        &CaptureDiagnostics::default(),
        Some(&policy()),
    ) else {
        panic!("unavailable")
    };
    let HarnessUsageSnapshot::ClaudeCode(snapshot) = result.snapshot else {
        panic!("wrong provider")
    };
    assert_eq!(snapshot.payload.output_tokens, None);
    assert_eq!(snapshot.payload.cost_estimation, None);
}
#[test]
fn synthetic_compaction_and_api_errors_do_not_add_usage() {
    let mut synthetic = response("synthetic", 999, 999);
    synthetic["isSynthetic"] = json!(true);
    let mut error = response("error", 999, 999);
    error["isApiErrorMessage"] = json!(true);
    let snapshot = capture(&[synthetic, error, response("summary", 10, 3)]);
    assert_eq!(snapshot.payload.output_tokens, Some(3));
}
#[test]
fn output_overflow_never_recovers_but_large_integer_is_exact() {
    assert_eq!(
        capture(&[response("a", 1, 9_007_199_254_740_993)])
            .payload
            .output_tokens,
        Some(9_007_199_254_740_993)
    );
    let snapshot = capture(&[
        response("a", 1, i64::MAX),
        response("b", 1, 1),
        response("c", 1, 0),
    ]);
    assert_eq!(snapshot.payload.output_tokens, None);
    assert_eq!(snapshot.payload.cost_estimation, None);
}
