use chrono::{TimeZone, Utc};
use serde_json::{Value, json};

use super::*;
use crate::{
    CaptureDiagnostics, ExtractionOutcome, JsonlDiagnostics, JsonlReadStatus, MAX_REQUESTS,
    extract_claude,
};

fn request(snapshot: HarnessUsageSnapshot) -> HarnessUsageRequest {
    HarnessUsageRequest::new(
        7,
        3,
        Utc.with_ymd_and_hms(2026, 1, 1, 12, 0, 0).unwrap(),
        snapshot,
    )
}

#[test]
fn requests_match_contract_fixtures() {
    for fixture in [
        include_str!("fixtures/api/claude.json"),
        include_str!("fixtures/api/codex.json"),
    ] {
        let expected: Value = serde_json::from_str(fixture).unwrap();
        let usage = expected["snapshot"]["payload"]["requests"][0]["usage"].clone();
        let attribution = Attribution {
            model: expected["snapshot"]["payload"]["requests"][0]["model"]
                .as_str()
                .map(str::to_owned),
            service_tier: expected["snapshot"]["payload"]["requests"][0]["service_tier"]
                .as_str()
                .map(str::to_owned),
            inference_geo: expected["snapshot"]["payload"]["requests"][0]["inference_geo"]
                .as_str()
                .map(str::to_owned),
            speed: expected["snapshot"]["payload"]["requests"][0]["speed"]
                .as_str()
                .map(str::to_owned),
        };
        let tools = &expected["snapshot"]["payload"]["toolCalls"];
        let tool_calls = Some(ToolCalls {
            total: tools["total"].as_i64().unwrap(),
            by_name: serde_json::from_value(tools["byName"].clone()).unwrap(),
        });
        let mut findings = Findings::default();
        let coverage = Coverage {
            token_status: CoverageStatus::Known,
            tool_status: CoverageStatus::Known,
        };
        let snapshot = if expected["harness"] == "CLAUDE_CODE" {
            let counts = Counters::parse(
                &usage,
                [
                    "/input_tokens",
                    "/output_tokens",
                    "/cache_read_input_tokens",
                    "/cache_creation_input_tokens",
                    "/cache_creation/ephemeral_5m_input_tokens",
                    "/cache_creation/ephemeral_1h_input_tokens",
                ],
                &mut findings,
            )
            .unwrap();
            HarnessUsageSnapshot::ClaudeCode(UsageSnapshot {
                coverage,
                payload: UsagePayload::new(
                    vec![RequestUsage {
                        attribution,
                        usage: ClaudeUsage::from(counts),
                    }],
                    None,
                    tool_calls,
                ),
            })
        } else {
            let counts = Counters::parse(
                &usage,
                [
                    "/input_tokens",
                    "/cached_input_tokens",
                    "/output_tokens",
                    "/reasoning_output_tokens",
                    "/total_tokens",
                    "/cache_write_input_tokens",
                ],
                &mut findings,
            )
            .unwrap();
            HarnessUsageSnapshot::Codex(UsageSnapshot {
                coverage,
                payload: UsagePayload::new(
                    vec![RequestUsage {
                        attribution,
                        usage: CodexUsage::from(counts),
                    }],
                    None,
                    tool_calls,
                ),
            })
        };
        assert_eq!(serde_json::to_value(request(snapshot)).unwrap(), expected);
    }
}

#[test]
fn row_and_body_bounds_conserve_usage_and_keep_a_deterministic_prefix() {
    let entries = (0..MAX_REQUESTS + 3).rev().map(|index| json!({
        "type":"assistant",
        "message":{"id":format!("{index:05}"), "model":format!("{index:05}{}", "m".repeat(251)),
            "usage":{"input_tokens":1,"output_tokens":2,"service_tier":"s".repeat(256),
                "inference_geo":"g".repeat(256),"speed":"f".repeat(256)}, "content":[]}
    })).collect::<Vec<_>>();
    let diagnostics = CaptureDiagnostics {
        root: JsonlDiagnostics {
            status: JsonlReadStatus::Readable,
            ..Default::default()
        },
        ..Default::default()
    };
    let ExtractionOutcome::Usable(extracted) = extract_claude("root", &entries, [], &diagnostics)
    else {
        panic!("usable bounded capture");
    };
    let HarnessUsageSnapshot::ClaudeCode(snapshot) = &extracted.snapshot else {
        unreachable!()
    };
    assert_eq!(snapshot.payload.requests.len(), MAX_REQUESTS);
    assert_eq!(
        snapshot
            .payload
            .unattributed_usage
            .as_ref()
            .unwrap()
            .input_tokens,
        Some(3)
    );
    let mut report = request(extracted.snapshot);
    assert!(report.bound_to_body().unwrap());
    let encoded = serde_json::to_vec(&report).unwrap();
    assert!(encoded.len() <= MAX_BODY_BYTES);
    assert!(!report.bound_to_body().unwrap());
    assert_eq!(encoded, serde_json::to_vec(&report).unwrap());
    let HarnessUsageSnapshot::ClaudeCode(snapshot) = report.snapshot else {
        unreachable!()
    };
    let retained = snapshot.payload.requests.len();
    assert!(retained > 0 && retained < MAX_REQUESTS);
    assert!(
        snapshot.payload.requests[0]
            .attribution
            .model
            .as_ref()
            .unwrap()
            .starts_with("00000")
    );
    assert!(
        snapshot.payload.requests[retained - 1]
            .attribution
            .model
            .as_ref()
            .unwrap()
            .starts_with(&format!("{:05}", retained - 1))
    );
    assert_eq!(snapshot.coverage.token_status, CoverageStatus::Partial);
    let remainder = snapshot.payload.unattributed_usage.unwrap();
    assert_eq!(
        retained as i64 + remainder.input_tokens.unwrap(),
        entries.len() as i64
    );
    assert_eq!(
        2 * retained as i64 + remainder.output_tokens.unwrap(),
        2 * entries.len() as i64
    );
}

#[test]
fn compaction_does_not_revive_overflowed_remainder_components() {
    let usage = ClaudeUsage {
        input_tokens: Some(1),
        output_tokens: Some(1),
        cache_read_input_tokens: None,
        cache_creation_input_tokens: None,
        cache_creation: None,
    };
    let mut payload = UsagePayload::new(
        vec![RequestUsage {
            attribution: Attribution {
                model: Some("x".repeat(MAX_BODY_BYTES)),
                ..Default::default()
            },
            usage,
        }],
        Some(ClaudeUsage {
            input_tokens: None,
            output_tokens: Some(2),
            cache_read_input_tokens: None,
            cache_creation_input_tokens: None,
            cache_creation: None,
        }),
        None,
    );
    payload.unattributed_overflowed = vec![true, false, false, false, false, false];
    let mut report = request(HarnessUsageSnapshot::ClaudeCode(UsageSnapshot {
        coverage: Coverage {
            token_status: CoverageStatus::Partial,
            tool_status: CoverageStatus::Unavailable,
        },
        payload,
    }));
    assert!(report.bound_to_body().unwrap());
    let HarnessUsageSnapshot::ClaudeCode(snapshot) = report.snapshot else {
        unreachable!()
    };
    assert!(snapshot.payload.requests.is_empty());
    let remainder = snapshot.payload.unattributed_usage.unwrap();
    assert_eq!(remainder.input_tokens, None);
    assert_eq!(remainder.output_tokens, Some(3));
}
