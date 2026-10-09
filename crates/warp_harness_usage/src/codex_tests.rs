use serde_json::{Value, json};

use super::*;
use crate::api::{CostStatus, CoverageStatus};
use crate::{JsonlDiagnostics, JsonlReadStatus};

fn policy() -> ThresholdPolicy {
    ThresholdPolicy::parse(json!({"schema_version":1,"models":{"gpt-a":{"kind":"input_gt","tokens":10},"gpt-b":{"kind":"none"}}})).unwrap()
}

fn context(turn: &str, model: &str) -> Value {
    json!({"type":"turn_context","payload":{"turn_id":turn,"model":model}})
}

fn record(
    id: &str,
    turn: &str,
    input: i64,
    output: i64,
    thread_input: i64,
    thread_output: i64,
) -> Value {
    let usage = json!({"input_tokens":input,"cached_input_tokens":0,"cache_write_input_tokens":0,"output_tokens":output});
    json!({"type":"event_msg","payload":{"type":"token_usage_record","thread_id":"root","turn_id":turn,"session_id":"owner","root_turn_id":turn,
        "response_id":id,"usage":usage,"turn_token_usage":usage,
        "thread_token_usage":{"input_tokens":thread_input,"cached_input_tokens":0,"cache_write_input_tokens":0,"output_tokens":thread_output}}})
}

fn capture(entries: &[Value]) -> crate::api::UsageSnapshot<CodexUsage> {
    let diagnostics = CaptureDiagnostics {
        root: JsonlDiagnostics {
            status: JsonlReadStatus::Readable,
            ..Default::default()
        },
        ..Default::default()
    };
    let ExtractionOutcome::Usable(result) =
        extract_codex("root", entries, &diagnostics, Some(&policy()))
    else {
        panic!("unavailable")
    };
    let HarnessUsageSnapshot::Codex(snapshot) = result.snapshot else {
        panic!("wrong provider")
    };
    snapshot
}

#[test]
fn model_aliases_share_cost_groups() {
    let snapshot = capture(&[
        context("t1", "gpt-a"),
        record("a", "t1", 10, 2, 10, 2),
        context("t2", " GPT-A-20260101-latest "),
        record("b", "t2", 10, 2, 20, 4),
    ]);
    let groups = snapshot.payload.cost_metadata.unwrap().groups;

    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].attribution.model.as_deref(), Some("gpt-a"));
    assert_eq!(
        groups[0].pre_threshold.as_ref().unwrap().input_tokens,
        Some(20)
    );
    assert_eq!(
        groups[0].pre_threshold.as_ref().unwrap().output_tokens,
        Some(4)
    );
    assert_eq!(groups[0].post_threshold, None);
    assert_eq!(snapshot.payload.output_tokens, Some(4));
}

#[test]
fn native_responses_use_individual_input_and_deduplicate_checkpoint_copies() {
    let first = record("a", "t1", 10, 2, 10, 2);
    let second = record("b", "t2", 10, 2, 20, 4);
    let snapshot = capture(&[
        context("t1", "gpt-a"),
        first.clone(),
        json!({"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"output_tokens":2}}}}),
        context("t2", "gpt-a"),
        second,
        json!({"type":"compacted","payload":{"latest_token_usage_record":first["payload"]}}),
    ]);
    let group = &snapshot.payload.cost_metadata.unwrap().groups[0];
    assert_eq!(group.pre_threshold.as_ref().unwrap().input_tokens, Some(20));
    assert_eq!(group.post_threshold, None);
    assert_eq!(snapshot.payload.output_tokens, Some(4));
    assert_eq!(snapshot.coverage.cost_status, CostStatus::Known);
}

#[test]
fn configured_tier_and_model_are_joined_to_their_turn_not_the_last_model() {
    let snapshot = capture(&[
        json!({"type":"event_msg","payload":{"type":"thread_settings_applied","thread_settings":{"service_tier":"flex"}}}),
        context("t1", "gpt-a"),
        record("a", "t1", 11, 1, 11, 1),
        json!({"type":"event_msg","payload":{"type":"thread_settings_applied","thread_settings":{"service_tier":"priority"}}}),
        context("t2", "gpt-b"),
        record("b", "t2", 20, 2, 31, 3),
    ]);
    let groups = snapshot.payload.cost_metadata.unwrap().groups;
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0].attribution.service_tier.as_deref(), Some("flex"));
    assert!(groups[0].post_threshold.is_some());
    assert_eq!(
        groups[1].attribution.service_tier.as_deref(),
        Some("priority")
    );
    assert!(groups[1].pre_threshold.is_some());
    assert_eq!(groups[1].long_context_threshold_tokens, None);
}

#[test]
fn uncovered_cumulative_history_disables_cost_without_discarding_observed_output() {
    let snapshot = capture(&[context("t1", "gpt-a"), record("a", "t1", 10, 2, 110, 20)]);
    assert_eq!(snapshot.payload.cost_metadata, None);
    assert_eq!(snapshot.payload.output_tokens, Some(2));
    assert_eq!(
        snapshot.coverage.output_token_status,
        CoverageStatus::Partial
    );
}

#[test]
fn conflicting_response_identity_cannot_recover_from_a_later_duplicate() {
    let first = record("a", "t1", 10, 2, 10, 2);
    let mut conflict = first.clone();
    conflict["payload"]["session_id"] = json!("other-owner");
    let snapshot = capture(&[context("t1", "gpt-a"), first.clone(), conflict, first]);
    assert_eq!(snapshot.payload.cost_metadata, None);
    assert_eq!(snapshot.payload.output_tokens, None);
}

#[test]
fn legacy_checkpoints_do_not_create_requests_or_known_zero_output() {
    let snapshot = capture(&[
        context("t1", "gpt-a"),
        json!({"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"output_tokens":2}}}}),
    ]);
    assert_eq!(snapshot.coverage.cost_status, CostStatus::Unavailable);
    assert_eq!(
        snapshot.coverage.output_token_status,
        CoverageStatus::Unavailable
    );
    assert_eq!(snapshot.payload.output_tokens, None);
    let wire = serde_json::to_value(snapshot).unwrap();
    assert!(wire["payload"].get("requests").is_none());
    assert!(wire["payload"].get("unattributed_usage").is_none());
}

#[test]
fn real_summarization_is_counted_once_and_synthetic_compaction_is_free() {
    let summary = record("summary", "t2", 11, 3, 21, 5);
    let snapshot = capture(&[
        context("t1", "gpt-a"),
        record("a", "t1", 10, 2, 10, 2),
        context("t2", "gpt-a"),
        summary.clone(),
        json!({"type":"compacted","payload":{"latest_token_usage_record":summary["payload"]}}),
        json!({"type":"compacted","payload":{"message":"synthetic","replacement_history":[]}}),
        json!({"type":"event_msg","payload":{"type":"token_count","info":null}}),
    ]);
    assert_eq!(snapshot.payload.output_tokens, Some(5));
    let group = &snapshot.payload.cost_metadata.unwrap().groups[0];
    assert_eq!(
        group.post_threshold.as_ref().unwrap().output_tokens,
        Some(3)
    );
}

#[test]
fn missing_context_disables_cost_only() {
    let snapshot = capture(&[record("a", "t1", 10, 2, 10, 2)]);
    assert_eq!(snapshot.payload.cost_metadata, None);
    assert_eq!(snapshot.payload.output_tokens, Some(2));
}

#[test]
fn missing_required_native_counter_disables_cost_only() {
    let mut event = record("a", "t1", 10, 2, 10, 2);
    event["payload"]["usage"]
        .as_object_mut()
        .unwrap()
        .remove("cache_write_input_tokens");

    let snapshot = capture(&[context("t1", "gpt-a"), event]);
    assert_eq!(snapshot.payload.cost_metadata, None);
    assert_eq!(snapshot.payload.output_tokens, Some(2));
}

#[test]
fn optional_counters_are_omitted_when_any_response_lacks_them() {
    let mut first = record("a", "t1", 5, 1, 5, 1);
    first["payload"]["usage"]["reasoning_output_tokens"] = json!(1);
    first["payload"]["turn_token_usage"]["reasoning_output_tokens"] = json!(1);
    first["payload"]["thread_token_usage"]["reasoning_output_tokens"] = json!(1);
    let snapshot = capture(&[
        context("t1", "gpt-a"),
        first,
        context("t2", "gpt-a"),
        record("b", "t2", 5, 1, 10, 2),
    ]);
    let group = &snapshot.payload.cost_metadata.unwrap().groups[0];
    assert_eq!(
        group
            .pre_threshold
            .as_ref()
            .unwrap()
            .reasoning_output_tokens,
        None
    );
    assert_eq!(group.pre_threshold.as_ref().unwrap().input_tokens, Some(10));
}

#[test]
fn copied_foreign_settings_do_not_change_root_pricing_tier() {
    let snapshot = capture(&[
        json!({"type":"event_msg","payload":{"type":"thread_settings_applied","thread_id":"root","thread_settings":{"service_tier":"flex"}}}),
        json!({"type":"event_msg","payload":{"type":"thread_settings_applied","thread_id":"foreign","thread_settings":{"service_tier":"priority"}}}),
        context("t1", "gpt-a"),
        record("a", "t1", 10, 2, 10, 2),
    ]);
    assert_eq!(
        snapshot.payload.cost_metadata.unwrap().groups[0]
            .attribution
            .service_tier
            .as_deref(),
        Some("flex")
    );
}

#[test]
fn trailing_uncovered_checkpoint_disables_cost_but_keeps_native_output() {
    let snapshot = capture(&[
        context("t1", "gpt-a"),
        record("a", "t1", 10, 2, 10, 2),
        json!({"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":20,"output_tokens":4}}}}),
    ]);
    assert_eq!(snapshot.payload.cost_metadata, None);
    assert_eq!(snapshot.payload.output_tokens, Some(2));
    assert_eq!(
        snapshot.coverage.output_token_status,
        CoverageStatus::Partial
    );
}

#[test]
fn synthetic_full_context_checkpoint_does_not_reset_billable_history() {
    let snapshot = capture(&[
        context("t1", "gpt-a"),
        record("a", "t1", 10, 2, 10, 2),
        json!({"type":"event_msg","payload":{"type":"token_count","info":{
            "model_context_window":100,
            "total_token_usage":{"input_tokens":0,"cached_input_tokens":0,"cache_write_input_tokens":0,
                "output_tokens":0,"reasoning_output_tokens":0,"total_tokens":100},
            "last_token_usage":{"input_tokens":0,"cached_input_tokens":0,"cache_write_input_tokens":0,
                "output_tokens":0,"reasoning_output_tokens":0,"total_tokens":88}
        }}}),
        context("t2", "gpt-a"),
        record("b", "t2", 11, 3, 21, 5),
    ]);
    assert_eq!(snapshot.coverage.cost_status, CostStatus::Known);
    assert_eq!(snapshot.payload.output_tokens, Some(5));
    let group = &snapshot.payload.cost_metadata.unwrap().groups[0];
    assert_eq!(group.pre_threshold.as_ref().unwrap().input_tokens, Some(10));
    assert_eq!(
        group.post_threshold.as_ref().unwrap().input_tokens,
        Some(11)
    );
}
