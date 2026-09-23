use std::collections::BTreeMap;
use std::time::Duration;

use chrono::{TimeZone, Utc};
use futures::executor::block_on;
use http_client::StatusCode;
use mockito::Matcher;
use serde_json::json;
use warp_core::channel::ChannelState;
use warp_harness_usage::api::{
    ClaudeUsage, Coverage, CoverageStatus, HarnessUsageRequest, HarnessUsageSnapshot, ToolCalls,
    UsagePayload, UsageSnapshot,
};
use warp_server_client::base_client::{AMBIENT_WORKLOAD_TOKEN_HEADER, CLOUD_AGENT_ID_HEADER};

use super::{
    HarnessUsageCapability, HarnessUsageError, HarnessUsageErrorKind,
    HarnessUsagePublicationStatus, ResolvedHarnessPrompt, ServerApi,
    parse_harness_usage_retry_after,
};
use crate::ai::ambient_agents::AmbientAgentTaskId;

fn task_id() -> AmbientAgentTaskId {
    "550e8400-e29b-41d4-a716-446655440000".parse().unwrap()
}

fn report() -> HarnessUsageRequest {
    HarnessUsageRequest::new(
        41,
        7,
        Utc.with_ymd_and_hms(2026, 9, 10, 12, 0, 0).unwrap(),
        HarnessUsageSnapshot::ClaudeCode(UsageSnapshot {
            coverage: Coverage {
                token_status: CoverageStatus::Known,
                tool_status: CoverageStatus::Partial,
            },
            payload: UsagePayload {
                usage: Some(ClaudeUsage {
                    input_tokens: Some(9_007_199_254_740_993),
                    output_tokens: None,
                    cache_read_input_tokens: None,
                    cache_creation_input_tokens: None,
                    cache_creation: None,
                }),
                attribution: Vec::new(),
                tool_calls: Some(ToolCalls {
                    total: 1,
                    by_name: BTreeMap::from([("Read".into(), 1)]),
                }),
            },
        }),
    )
}

#[test]
fn incompatible_startup_preserves_prompt_without_reporting() {
    for value in [
        json!({"prompt": "test"}),
        json!({"prompt": "test", "harness_usage": null}),
        json!({"prompt": "test", "harness_usage": {"execution_id": 0}}),
        json!({"prompt": "test", "harness_usage": {}}),
        json!({"prompt": "test", "harness_usage": "unknown"}),
    ] {
        let prompt: ResolvedHarnessPrompt = serde_json::from_value(value).unwrap();

        assert_eq!(prompt.prompt, "test");
        assert_eq!(prompt.harness_usage, None);
    }
}

#[test]
fn startup_accepts_only_the_advertised_execution() {
    let prompt: ResolvedHarnessPrompt = serde_json::from_value(json!({
        "prompt": "test",
        "resumption_prompt": "continue",
        "harness_usage": {"execution_id": 41, "future_field": true}
    }))
    .unwrap();

    assert_eq!(
        prompt.harness_usage,
        Some(HarnessUsageCapability { execution_id: 41 })
    );
    assert_eq!(prompt.resumption_prompt.as_deref(), Some("continue"));
}

#[test]
fn invalid_reports_are_rejected_before_auth_or_network() {
    for (execution_id, capture_sequence) in [(41, 0), (0, 7)] {
        let mut report = report();
        report.execution_id = execution_id;
        report.capture_sequence = capture_sequence;

        let error =
            block_on(ServerApi::new_for_test().publish_harness_usage_for_task(&task_id(), &report))
                .unwrap_err();

        assert_eq!(error.kind, HarnessUsageErrorKind::InvalidReport);
        assert_eq!(error.status, None);
    }
}

#[test]
fn publication_reuses_workload_auth_but_not_an_unrelated_ambient_task() {
    for (status, expected_status, execution_id, capture_sequence) in [
        ("accepted", HarnessUsagePublicationStatus::Accepted, 41, 7),
        (
            "idempotent",
            HarnessUsagePublicationStatus::Idempotent,
            41,
            7,
        ),
        (
            "ignored_older_capture",
            HarnessUsagePublicationStatus::IgnoredOlderCapture,
            42,
            1,
        ),
    ] {
        let task_id = task_id();
        let report = report();
        let request = {
            let mut server = ChannelState::mock_server();
            server
                .mock("POST", "/api/v1/harness-support/usage")
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
            .set_ambient_workload_token_for_test("synthetic-workload-token".into(), None);

        let publication =
            block_on(server.publish_harness_usage_for_task(&task_id, &report)).unwrap();

        assert_eq!(publication, expected_status);
        assert_eq!(report.execution_id, 41);
        request.assert();
        request.remove();
    }
}

#[test]
fn publication_preserves_http_failure_classification() {
    for (status, problem_type, expected_kind) in [
        (
            403,
            "feature_not_available",
            HarnessUsageErrorKind::Disabled,
        ),
        (
            401,
            "authentication_required",
            HarnessUsageErrorKind::Unauthorized,
        ),
        (403, "not_authorized", HarnessUsageErrorKind::Unauthorized),
        (409, "conflict", HarnessUsageErrorKind::Conflict),
        (412, "conflict", HarnessUsageErrorKind::Conflict),
        (400, "invalid_request", HarnessUsageErrorKind::InvalidReport),
        (422, "invalid_request", HarnessUsageErrorKind::InvalidReport),
        (
            422,
            "operation_not_supported",
            HarnessUsageErrorKind::Disabled,
        ),
        (404, "resource_not_found", HarnessUsageErrorKind::Disabled),
        (
            429,
            "resource_unavailable",
            HarnessUsageErrorKind::Retryable,
        ),
        (503, "internal_error", HarnessUsageErrorKind::Retryable),
    ] {
        let request = {
            let mut server = ChannelState::mock_server();
            server
                .mock("POST", "/api/v1/harness-support/usage")
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

        let error =
            block_on(server.publish_harness_usage_for_task(&task_id(), &report())).unwrap_err();

        assert_eq!(error.kind, expected_kind, "{status} {problem_type}");
        assert_eq!(
            error.status,
            Some(StatusCode::from_u16(status as u16).unwrap())
        );
        assert_eq!(error.retry_after, Some(Duration::from_secs(2)));
        assert!(!format!("{error:?}").contains("body content"));
        request.assert();
        request.remove();
    }
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
fn request_preparation_errors_are_retryable_without_retaining_diagnostics() {
    let error =
        HarnessUsageError::from_request_preparation_error(anyhow::anyhow!("private diagnostic"));

    assert_eq!(error.kind, HarnessUsageErrorKind::Retryable);
    assert!(!format!("{error:?}").contains("private diagnostic"));
}
