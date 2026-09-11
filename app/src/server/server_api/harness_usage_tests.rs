use std::collections::BTreeMap;
use std::time::Duration;

use chrono::{TimeZone, Utc};
use futures::executor::block_on;
use http_client::StatusCode;
use mockito::Matcher;
use rstest::rstest;
use serde_json::{Value, json};
use warp_core::channel::ChannelState;
use warp_server_client::base_client::{AMBIENT_WORKLOAD_TOKEN_HEADER, CLOUD_AGENT_ID_HEADER};

use super::{
    HarnessUsageContext, HarnessUsageCoverage, HarnessUsageCoverageStatus, HarnessUsageError,
    HarnessUsageErrorKind, HarnessUsagePublicationStatus, HarnessUsageReport, HarnessUsageSnapshot,
    ResolvedHarnessPrompt, ServerApi, UsageHarness, parse_harness_usage_retry_after,
};
use crate::ai::ambient_agents::AmbientAgentTaskId;
fn task_id() -> AmbientAgentTaskId {
    "550e8400-e29b-41d4-a716-446655440000".parse().unwrap()
}

fn report() -> HarnessUsageReport {
    HarnessUsageReport {
        schema_version: 1,
        parser_version: 1,
        harness: UsageHarness::ClaudeCode,
        execution_id: 41,
        capture_sequence: 7,
        last_updated: Utc.with_ymd_and_hms(2026, 9, 10, 12, 0, 0).unwrap(),
        snapshot: HarnessUsageSnapshot {
            coverage: HarnessUsageCoverage {
                token_status: HarnessUsageCoverageStatus::Known,
                tool_status: HarnessUsageCoverageStatus::Partial,
                captured_scope: Some("root_and_captured_subagents".into()),
                reason_codes: BTreeMap::from([("subagent_discovery_incomplete".into(), 1)]),
            },
            payload: json!({
                "usage": {"input_tokens": 9_007_199_254_740_993_i64},
                "toolCalls": {"total": 1, "byName": {"Read": 1}}
            })
            .as_object()
            .unwrap()
            .clone(),
            session_ids: vec!["root".into()],
            root_scope: Some("root".into()),
            subagent_scope: vec!["child".into()],
        },
    }
}

#[rstest]
#[case(json!({"prompt": "test"}))]
#[case(json!({"prompt": "test", "harness_usage": null}))]
#[case(json!({"prompt": "test", "harness_usage": {"schema_version": 2, "execution_id": 41}}))]
#[case(json!({"prompt": "test", "harness_usage": {"schema_version": 1, "execution_id": 0}}))]
#[case(json!({"prompt": "test", "harness_usage": {"schema_version": 1}}))]
#[case(json!({"prompt": "test", "harness_usage": "unknown"}))]
fn incompatible_startup_preserves_prompt_without_reporting(#[case] value: Value) {
    let prompt: ResolvedHarnessPrompt = serde_json::from_value(value).unwrap();

    assert_eq!(prompt.prompt, "test");
    assert_eq!(prompt.harness_usage, None);
}

#[test]
fn startup_accepts_only_the_advertised_execution() {
    let prompt: ResolvedHarnessPrompt = serde_json::from_value(json!({
        "prompt": "test",
        "resumption_prompt": "continue",
        "harness_usage": {"schema_version": 1, "execution_id": 41, "future_field": true}
    }))
    .unwrap();

    assert_eq!(
        prompt.harness_usage,
        Some(HarnessUsageContext {
            schema_version: 1,
            execution_id: 41
        })
    );
    assert_eq!(prompt.resumption_prompt.as_deref(), Some("continue"));
}

#[rstest]
#[case(UsageHarness::ClaudeCode, "CLAUDE_CODE")]
#[case(UsageHarness::Codex, "CODEX")]
fn publication_wire_preserves_native_integer_precision_and_capture_time(
    #[case] harness: UsageHarness,
    #[case] wire_harness: &str,
) {
    let mut report = report();
    report.harness = harness;
    let body = report.encode().unwrap();
    let value: Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(
        value,
        json!({
            "schema_version": 1,
            "parser_version": 1,
            "harness": wire_harness,
            "execution_id": 41,
            "capture_sequence": 7,
            "last_updated": "2026-09-10T12:00:00Z",
            "snapshot": {
                "coverage": {
                    "token_status": "known",
                    "tool_status": "partial",
                    "captured_scope": "root_and_captured_subagents",
                    "reason_codes": {"subagent_discovery_incomplete": 1}
                },
                "payload": {
                    "usage": {"input_tokens": 9_007_199_254_740_993_i64},
                    "toolCalls": {"total": 1, "byName": {"Read": 1}}
                },
                "session_ids": ["root"],
                "root_scope": "root",
                "subagent_scope": ["child"]
            }
        })
    );
}

#[rstest]
#[case(|report| report.schema_version = 2)]
#[case(|report| report.capture_sequence = 0)]
#[case(|report| report.snapshot.root_scope = Some("a".repeat(257)))]
#[case(|report| report.snapshot.session_ids = vec!["root".into(); 65])]
#[case(|report| { report.snapshot.payload.insert("large".into(), json!("a".repeat(1_048_576))); })]
fn invalid_reports_are_rejected_before_auth_or_network(
    #[case] invalidate: fn(&mut HarnessUsageReport),
) {
    let mut report = report();
    invalidate(&mut report);

    let error =
        block_on(ServerApi::new_for_test().report_harness_usage_for_task(&task_id(), &report))
            .unwrap_err();

    assert_eq!(error.kind, HarnessUsageErrorKind::InvalidReport);
    assert_eq!(error.status, None);
}

#[rstest]
#[case("accepted", HarnessUsagePublicationStatus::Accepted, 41, 7)]
#[case("idempotent", HarnessUsagePublicationStatus::Idempotent, 41, 7)]
#[case("stale", HarnessUsagePublicationStatus::Stale, 42, 1)]
fn publication_reuses_workload_auth_but_not_an_unrelated_ambient_task(
    #[case] status: &str,
    #[case] expected_status: HarnessUsagePublicationStatus,
    #[case] execution_id: i64,
    #[case] capture_sequence: i64,
) {
    let task_id = task_id();
    let report = report();
    let request = {
        let mut server = ChannelState::mock_server();
        server
            .mock("POST", "/api/v1/harness-support/harness-usage")
            .match_header(CLOUD_AGENT_ID_HEADER, task_id.to_string().as_str())
            .match_header(AMBIENT_WORKLOAD_TOKEN_HEADER, "synthetic-workload-token")
            .match_body(Matcher::Json(serde_json::to_value(&report).unwrap()))
            .with_status(200)
            .with_body(json!({ "status": status, "execution_id": execution_id, "capture_sequence": capture_sequence }).to_string())
            .expect(1)
            .create()
    };
    let server = ServerApi::new_for_test();
    server.set_ambient_agent_task_id(Some(
        "123e4567-e89b-12d3-a456-426614174000".parse().unwrap(),
    ));
    server
        .base_client
        .set_ambient_workload_token_for_test("synthetic-workload-token".into());

    let publication = block_on(server.report_harness_usage_for_task(&task_id, &report)).unwrap();

    assert_eq!(publication.status, expected_status);
    assert_eq!(report.execution_id, 41);
    request.assert();
}

#[rstest]
#[case(403, "feature_not_available", HarnessUsageErrorKind::Disabled)]
#[case(401, "authentication_required", HarnessUsageErrorKind::Unauthorized)]
#[case(403, "not_authorized", HarnessUsageErrorKind::Unauthorized)]
#[case(409, "conflict", HarnessUsageErrorKind::Conflict)]
#[case(412, "conflict", HarnessUsageErrorKind::Conflict)]
#[case(400, "invalid_request", HarnessUsageErrorKind::InvalidReport)]
#[case(422, "invalid_request", HarnessUsageErrorKind::InvalidReport)]
#[case(422, "operation_not_supported", HarnessUsageErrorKind::Disabled)]
#[case(404, "resource_not_found", HarnessUsageErrorKind::Disabled)]
#[case(429, "resource_unavailable", HarnessUsageErrorKind::Retryable)]
#[case(503, "internal_error", HarnessUsageErrorKind::Retryable)]
fn publication_preserves_http_failure_classification(
    #[case] status: usize,
    #[case] problem_type: &str,
    #[case] expected_kind: HarnessUsageErrorKind,
) {
    let request = {
        let mut server = ChannelState::mock_server();
        server
            .mock("POST", "/api/v1/harness-support/harness-usage")
            .with_status(status)
            .with_header("Retry-After", "2")
            .with_body(
                json!({
                    "type": format!("https://docs.warp.dev/errors/{problem_type}"),
                    "detail": "body content must not enter diagnostics"
                })
                .to_string(),
            )
            .expect(1)
            .create()
    };
    let server = ServerApi::new_for_test();

    let error = block_on(server.report_harness_usage_for_task(&task_id(), &report())).unwrap_err();

    assert_eq!(error.kind, expected_kind);
    assert_eq!(
        error.status,
        Some(StatusCode::from_u16(status as u16).unwrap())
    );
    assert_eq!(error.retry_after, Some(Duration::from_secs(2)));
    assert!(!format!("{error:?}").contains("body content"));
    request.assert();
}

#[test]
fn retry_after_http_date_uses_response_time_not_capture_time() {
    let now = Utc.with_ymd_and_hms(2026, 9, 10, 12, 0, 0).unwrap();

    assert_eq!(
        parse_harness_usage_retry_after("Thu, 10 Sep 2026 12:00:02 GMT", now),
        Some(Duration::from_secs(2))
    );
    assert_eq!(
        parse_harness_usage_retry_after("Thu, 10 Sep 2026 11:59:00 GMT", now),
        Some(Duration::ZERO)
    );
    assert_eq!(parse_harness_usage_retry_after("invalid", now), None);
}

#[test]
fn authentication_errors_never_retain_credential_diagnostics() {
    let error = HarnessUsageError::from_auth_error(anyhow::anyhow!("private diagnostic"));

    assert_eq!(error.kind, HarnessUsageErrorKind::Unauthorized);
    assert!(!format!("{error:?}").contains("private diagnostic"));
}
