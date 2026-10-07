use serde_json::json;

use super::*;
#[test]
fn native_captures_match_shared_wire_fixtures() {
    let fixtures: serde_json::Value =
        serde_json::from_str(include_str!("../testdata/cost_inputs_v3.json")).unwrap();
    for fixture in fixtures
        .as_array()
        .unwrap()
        .iter()
        .filter(|fixture| fixture["valid"] == true)
    {
        let entries = fixture["entries"].as_array().unwrap();
        let policy = ThresholdPolicy::parse(fixture["policy"].clone());
        let diagnostics = crate::CaptureDiagnostics {
            root: crate::JsonlDiagnostics {
                status: if fixture["complete"] == true {
                    crate::JsonlReadStatus::Readable
                } else {
                    crate::JsonlReadStatus::Missing
                },
                ..Default::default()
            },
            ..Default::default()
        };
        let outcome = if fixture["report"]["harness"] == "CODEX" {
            crate::extract_codex("root", entries, &diagnostics, policy.as_ref())
        } else {
            crate::extract_claude("root", entries, [], &diagnostics, policy.as_ref())
        };
        let crate::ExtractionOutcome::Usable(result) = outcome else {
            panic!("unavailable capture")
        };
        let request = HarnessUsageRequest::new(
            42,
            1,
            "2026-01-01T12:00:00Z".parse().unwrap(),
            result.snapshot,
        );
        assert_eq!(
            serde_json::to_value(request).unwrap(),
            fixture["report"],
            "{}",
            fixture["name"]
        );
    }
}

#[test]
fn policy_rejects_unsupported_or_malformed_rules_without_a_rule_identity() {
    for value in [
        json!({"schema_version":2,"models":{}}),
        json!({"schema_version":1,"models":{"model":{"kind":"input_gt","tokens":0}}}),
        json!({"schema_version":1,"models":{"model":{"kind":"none","tokens":1}}}),
        json!({"schema_version":1,"models":{"model":{"kind":"unknown"}}}),
        json!({"schema_version":1,"models":{"MODEL":{"kind":"none"}}}),
    ] {
        assert!(ThresholdPolicy::parse(value).is_none());
    }
    let policy =
        ThresholdPolicy::parse(json!({"schema_version":1,"models":{"model":{"kind":"none"}}}))
            .unwrap();
    assert_eq!(policy.models["model"], ThresholdRule::None);
    assert_eq!(
        normalize_model(" Claude-A-20260101[1m]-latest "),
        "claude-a"
    );
}

#[test]
fn ten_thousand_responses_in_one_key_do_not_consume_wire_capacity() {
    let policy = ThresholdPolicy::parse(
        json!({"schema_version":1,"models":{"model":{"kind":"input_gt","tokens":272000}}}),
    )
    .unwrap();
    let entries = (0..10000).map(|index| json!({
        "type":"assistant","message":{"id":format!("response-{index}"),"model":"model","content":[],
        "usage":{"input_tokens":100000,"cache_read_input_tokens":0,"cache_creation_input_tokens":0,"output_tokens":1}}
    })).collect::<Vec<_>>();
    let diagnostics = crate::CaptureDiagnostics {
        root: crate::JsonlDiagnostics {
            status: crate::JsonlReadStatus::Readable,
            ..Default::default()
        },
        ..Default::default()
    };
    let crate::ExtractionOutcome::Usable(result) =
        crate::extract_claude("root", &entries, [], &diagnostics, Some(&policy))
    else {
        panic!("unavailable")
    };
    let HarnessUsageSnapshot::ClaudeCode(snapshot) = result.snapshot else {
        panic!("wrong provider")
    };
    let group = &snapshot.payload.cost_estimation.unwrap().groups[0];
    assert_eq!(
        group.pre_threshold.as_ref().unwrap().input_tokens,
        Some(1_000_000_000)
    );
    assert_eq!(group.post_threshold, None);
    assert_eq!(snapshot.payload.output_tokens, Some(10000));
}

#[test]
fn group_limit_drops_all_cost_but_continues_output() {
    let policy =
        ThresholdPolicy::parse(json!({"schema_version":1,"models":{"model":{"kind":"none"}}}))
            .unwrap();
    let mut entries = (0..128).map(|index| json!({
        "type":"assistant","message":{"id":format!("response-{index}"),"model":"model","content":[],
        "usage":{"input_tokens":1,"cache_read_input_tokens":0,"cache_creation_input_tokens":0,"output_tokens":1,"service_tier":format!("tier-{index}")}}
    })).collect::<Vec<_>>();
    let diagnostics = crate::CaptureDiagnostics {
        root: crate::JsonlDiagnostics {
            status: crate::JsonlReadStatus::Readable,
            ..Default::default()
        },
        ..Default::default()
    };
    let crate::ExtractionOutcome::Usable(result) =
        crate::extract_claude("root", &entries, [], &diagnostics, Some(&policy))
    else {
        panic!("unavailable")
    };
    let HarnessUsageSnapshot::ClaudeCode(snapshot) = result.snapshot else {
        panic!("wrong provider")
    };
    assert_eq!(snapshot.payload.cost_estimation.unwrap().groups.len(), 128);
    entries.push(json!({
        "type":"assistant","message":{"id":"overflow","model":"model","content":[{"type":"tool_use","id":"tool","name":"Read"}],
        "usage":{"input_tokens":1,"cache_read_input_tokens":0,"cache_creation_input_tokens":0,"output_tokens":1,"service_tier":"overflow"}}
    }));
    let crate::ExtractionOutcome::Usable(result) =
        crate::extract_claude("root", &entries, [], &diagnostics, Some(&policy))
    else {
        panic!("unavailable")
    };
    let HarnessUsageSnapshot::ClaudeCode(snapshot) = result.snapshot else {
        panic!("wrong provider")
    };
    assert_eq!(snapshot.payload.cost_estimation, None);
    assert_eq!(snapshot.payload.output_tokens, Some(129));
    assert_eq!(snapshot.coverage.output_token_status, CoverageStatus::Known);
    assert_eq!(snapshot.payload.tool_calls.unwrap().total, 1);
}

#[test]
fn oversized_cost_is_removed_before_retry_bytes_are_frozen() {
    let mut request = HarnessUsageRequest::new(
        1,
        1,
        Utc::now(),
        HarnessUsageSnapshot::Codex(UsageSnapshot {
            coverage: Coverage {
                cost_status: CostStatus::Known,
                output_token_status: CoverageStatus::Known,
                tool_status: CoverageStatus::Unavailable,
            },
            payload: UsagePayload::new(
                Some(CostEstimation {
                    groups: vec![UsageGroup {
                        attribution: Attribution {
                            model: Some("x".repeat(MAX_BODY_BYTES)),
                            ..Default::default()
                        },
                        long_context_threshold_tokens: None,
                        pre_threshold: Some(CodexUsage {
                            input_tokens: Some(0),
                            cached_input_tokens: Some(0),
                            cache_write_input_tokens: Some(0),
                            output_tokens: Some(0),
                            reasoning_output_tokens: None,
                            total_tokens: None,
                        }),
                        post_threshold: None,
                    }],
                }),
                Some(0),
                None,
            ),
        }),
    );
    assert!(request.bound_to_body().unwrap());
    let wire = serde_json::to_value(&request).unwrap();
    assert_eq!(wire["snapshot"]["coverage"]["cost_status"], "unavailable");
    assert_eq!(wire["snapshot"]["payload"]["output_tokens"], 0);
    assert!(wire["snapshot"]["payload"].get("cost_estimation").is_none());
    let bytes = serde_json::to_vec(&request).unwrap();
    assert!(!request.bound_to_body().unwrap());
    assert_eq!(serde_json::to_vec(&request).unwrap(), bytes);
}
