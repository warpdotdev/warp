use std::collections::BTreeMap;

use chrono::{TimeZone, Utc};
use serde_json::{Value, json};
use warp_harness_usage::{
    AttributedUsage, Attribution, CacheCreation, ClaudeUsage, CodexUsage, Coverage, CoverageStatus,
    NativePayload, ReasonCode, ToolCalls, UsagePayload, UsageSnapshot,
};

use super::{HarnessUsageReport, MAX_BODY_BYTES, UsageHarness};

const CLAUDE_FIXTURE: &str = include_str!("testdata/harness_usage/claude.json");
const CODEX_FIXTURE: &str = include_str!("testdata/harness_usage/codex.json");

fn snapshot(harness: UsageHarness) -> UsageSnapshot {
    let payload = match harness {
        UsageHarness::ClaudeCode => NativePayload::Claude(UsagePayload {
            usage: Some(ClaudeUsage {
                input_tokens: Some(9_007_199_254_740_993),
                output_tokens: Some(0),
                cache_read_input_tokens: Some(2),
                cache_creation_input_tokens: Some(3),
                cache_creation: Some(CacheCreation {
                    ephemeral_5m_input_tokens: Some(3),
                    ephemeral_1h_input_tokens: None,
                }),
            }),
            attribution: vec![AttributedUsage {
                attribution: Attribution {
                    model: Some("claude-test".into()),
                    service_tier: Some("standard".into()),
                    inference_geo: Some("us".into()),
                    speed: Some("fast".into()),
                },
                usage: ClaudeUsage {
                    input_tokens: Some(9_007_199_254_740_993),
                    output_tokens: Some(0),
                    cache_read_input_tokens: None,
                    cache_creation_input_tokens: None,
                    cache_creation: None,
                },
            }],
            tool_calls: Some(ToolCalls {
                total: 2,
                by_name: BTreeMap::from([("Read".into(), 1), ("mcp__test__lookup".into(), 1)]),
            }),
        }),
        UsageHarness::Codex => NativePayload::Codex(UsagePayload {
            usage: Some(CodexUsage {
                input_tokens: Some(10),
                cached_input_tokens: Some(0),
                output_tokens: Some(4),
                reasoning_output_tokens: Some(2),
                total_tokens: Some(14),
            }),
            attribution: vec![AttributedUsage {
                attribution: Attribution {
                    model: Some("codex-test".into()),
                    service_tier: Some("priority".into()),
                    inference_geo: None,
                    speed: None,
                },
                usage: CodexUsage {
                    input_tokens: Some(10),
                    cached_input_tokens: None,
                    output_tokens: Some(4),
                    reasoning_output_tokens: None,
                    total_tokens: None,
                },
            }],
            tool_calls: Some(ToolCalls {
                total: 0,
                by_name: BTreeMap::new(),
            }),
        }),
    };
    UsageSnapshot {
        payload,
        coverage: Coverage {
            token_status: match harness {
                UsageHarness::ClaudeCode => CoverageStatus::Known,
                UsageHarness::Codex => CoverageStatus::Partial,
            },
            tool_status: CoverageStatus::Known,
            captured_scope: "producer-local-scope",
            reason_codes: BTreeMap::from([(ReasonCode::IncompleteInput, 1)]),
        },
        session_ids: vec!["producer-local-session".into()],
        root_scope: "producer-local-root".into(),
        subagent_scope: vec!["producer-local-child".into()],
    }
}

fn report(harness: UsageHarness, snapshot: &UsageSnapshot) -> HarnessUsageReport {
    HarnessUsageReport::new(
        harness,
        7,
        3,
        Utc.with_ymd_and_hms(2026, 1, 1, 12, 0, 0).unwrap(),
        snapshot,
    )
    .unwrap()
}

#[test]
fn wire_matches_shared_fixtures_without_producer_metadata() {
    for (harness, fixture) in [
        (UsageHarness::ClaudeCode, CLAUDE_FIXTURE),
        (UsageHarness::Codex, CODEX_FIXTURE),
    ] {
        let body = report(harness, &snapshot(harness)).encode().unwrap();
        let actual: Value = serde_json::from_slice(&body).unwrap();
        let expected: Value = serde_json::from_str(fixture).unwrap();

        assert_eq!(actual, expected, "{harness:?}");
    }
}

#[test]
fn preserves_remaining_optional_native_fields() {
    let mut claude = snapshot(UsageHarness::ClaudeCode);
    let NativePayload::Claude(payload) = &mut claude.payload else {
        unreachable!()
    };
    payload.usage.as_mut().unwrap().cache_creation = Some(CacheCreation {
        ephemeral_5m_input_tokens: None,
        ephemeral_1h_input_tokens: Some(i64::MAX),
    });
    let actual = serde_json::to_value(report(UsageHarness::ClaudeCode, &claude)).unwrap();
    assert_eq!(
        actual["snapshot"]["payload"]["usage"]["cache_creation"],
        json!({"ephemeral_1h_input_tokens": i64::MAX})
    );

    let mut codex = snapshot(UsageHarness::Codex);
    let NativePayload::Codex(payload) = &mut codex.payload else {
        unreachable!()
    };
    payload.attribution[0].attribution.inference_geo = Some("us".into());
    payload.attribution[0].attribution.speed = Some("fast".into());
    let actual = serde_json::to_value(report(UsageHarness::Codex, &codex)).unwrap();
    assert_eq!(
        actual["snapshot"]["payload"]["attribution"][0],
        json!({
            "model": "codex-test",
            "service_tier": "priority",
            "inference_geo": "us",
            "speed": "fast",
            "usage": {"input_tokens": 10, "output_tokens": 4}
        })
    );
}

#[test]
fn missing_categories_are_not_measured_zero() {
    let mut snapshot = snapshot(UsageHarness::Codex);
    let NativePayload::Codex(payload) = &mut snapshot.payload else {
        unreachable!()
    };
    payload.usage = None;
    payload.attribution.clear();
    snapshot.coverage.token_status = CoverageStatus::Unavailable;

    let actual = serde_json::to_value(report(UsageHarness::Codex, &snapshot)).unwrap();
    assert_eq!(
        actual["snapshot"],
        json!({
            "coverage": {"token_status": "unavailable", "tool_status": "known"},
            "payload": {"toolCalls": {"total": 0, "byName": {}}}
        })
    );

    let NativePayload::Codex(payload) = &mut snapshot.payload else {
        unreachable!()
    };
    payload.tool_calls = None;
    payload.usage = Some(CodexUsage {
        input_tokens: None,
        cached_input_tokens: None,
        output_tokens: Some(0),
        reasoning_output_tokens: None,
        total_tokens: None,
    });
    snapshot.coverage.token_status = CoverageStatus::Partial;
    snapshot.coverage.tool_status = CoverageStatus::Unavailable;
    let actual = serde_json::to_value(report(UsageHarness::Codex, &snapshot)).unwrap();
    assert_eq!(
        actual["snapshot"]["payload"],
        json!({"usage": {"output_tokens": 0}})
    );
}

#[test]
fn mismatched_harness_payloads_cannot_construct_a_report() {
    for (harness, other) in [
        (UsageHarness::ClaudeCode, UsageHarness::Codex),
        (UsageHarness::Codex, UsageHarness::ClaudeCode),
    ] {
        assert!(HarnessUsageReport::new(harness, 7, 3, Utc::now(), &snapshot(other),).is_err());
    }
}

#[test]
fn unusable_and_oversized_reports_are_rejected() {
    let mut snapshot = snapshot(UsageHarness::ClaudeCode);
    snapshot.coverage.token_status = CoverageStatus::Unavailable;
    snapshot.coverage.tool_status = CoverageStatus::Unavailable;
    assert!(
        HarnessUsageReport::new(UsageHarness::ClaudeCode, 7, 3, Utc::now(), &snapshot).is_err()
    );

    snapshot.coverage.tool_status = CoverageStatus::Known;
    let NativePayload::Claude(payload) = &mut snapshot.payload else {
        unreachable!()
    };
    payload.tool_calls.as_mut().unwrap().by_name =
        BTreeMap::from([("a".repeat(MAX_BODY_BYTES), 2)]);
    assert!(
        report(UsageHarness::ClaudeCode, &snapshot)
            .encode()
            .is_err()
    );
}
