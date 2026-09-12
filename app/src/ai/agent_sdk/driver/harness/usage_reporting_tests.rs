use std::cell::RefCell;
use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;
use chrono::{TimeZone, Utc};
use futures::{executor::block_on, future};
use serde_json::json;
use warp_core::channel::ChannelState;
use warp_harness_usage::{CaptureDiagnostics, JsonlDiagnostics, JsonlReadStatus, extract_claude};

use super::*;
use crate::ai::agent_sdk::driver::harness::save_coordinator::save_transcript_and_block;

fn task_id() -> AmbientAgentTaskId {
    "550e8400-e29b-41d4-a716-446655440000".parse().unwrap()
}

fn reporter(client: Arc<ServerApi>) -> UsageReporter {
    let reporter = UsageReporter::default();
    reporter.initialize(client, Some(task_id()), Some(HarnessUsageContext {
        metrics_version: 1,
        execution_id: 41,
    }));
    reporter
}

fn report() -> HarnessUsageReport {
    CaptureIdentity { execution_id: 41, sequence: 1 }.report(
        UsageHarness::ClaudeCode,
        Utc.with_ymd_and_hms(2026, 9, 10, 12, 0, 0).unwrap(),
        extract_claude("root", &[], [], &CaptureDiagnostics {
            root: JsonlDiagnostics { status: JsonlReadStatus::Readable, ..Default::default() },
            ..Default::default()
        }),
    ).unwrap()
}

#[test]
fn retries_keep_exact_report_and_exhaust_without_another_event() {
    let report = report();
    let bodies = RefCell::new(Vec::new());
    let delays = RefCell::new(Vec::new());
    let result = block_on(publish_with_retry(
        &report,
        |report| {
            bodies.borrow_mut().push(serde_json::to_vec(report).unwrap());
            future::ready(Err(HarnessUsageError::new(HarnessUsageErrorKind::Retryable)))
        },
        |delay| {
            delays.borrow_mut().push(delay);
            future::ready(())
        },
    ));
    assert_eq!(result.unwrap_err().kind, HarnessUsageErrorKind::Retryable);
    let bodies = bodies.into_inner();
    assert_eq!(bodies.len(), 3);
    assert!(bodies.windows(2).all(|pair| pair[0] == pair[1]));
    let delays = delays.into_inner();
    assert_eq!(delays.len(), 2);
    assert!((Duration::from_secs(1)..=Duration::from_millis(1200)).contains(&delays[0]));
    assert!((Duration::from_secs(2)..=Duration::from_millis(2400)).contains(&delays[1]));
}

#[test]
fn permanent_errors_and_long_retry_after_do_not_rearm_idle_work() {
    for error in [
        HarnessUsageError::new(HarnessUsageErrorKind::Unauthorized),
        HarnessUsageError::new(HarnessUsageErrorKind::Conflict),
        HarnessUsageError::new(HarnessUsageErrorKind::InvalidReport),
        HarnessUsageError {
            kind: HarnessUsageErrorKind::Retryable,
            status: None,
            retry_after: Some(Duration::from_secs(60)),
        },
    ] {
        let calls = RefCell::new(0);
        let result = block_on(publish_with_retry(
            &report(),
            |_| {
                *calls.borrow_mut() += 1;
                future::ready(Err(error.clone()))
            },
            |_| async { panic!("must not retry") },
        ));
        assert_eq!(result.unwrap_err(), error);
        assert_eq!(*calls.borrow(), 1);
    }
}

#[test]
fn reporting_context_cannot_adopt_another_execution() {
    let client = Arc::new(ServerApi::new_for_test());
    let reporter = reporter(client.clone());
    let first = reporter.begin_capture().unwrap();
    reporter.initialize(client, Some(task_id()), Some(HarnessUsageContext {
        metrics_version: 1,
        execution_id: 42,
    }));
    let next = reporter.begin_capture().unwrap();
    assert_eq!((first.execution_id, first.sequence), (41, 1));
    assert_eq!((next.execution_id, next.sequence), (41, 2));
    assert!(UsageReporter::default().begin_capture().is_none());
}

#[test]
fn raw_success_gates_reporting_independently_of_block_failure() {
    for (raw_status, usable) in [(200, true), (403, true), (200, false)] {
        let should_report = raw_status == 200 && usable;
        let (target, raw, metrics) = {
            let mut server = ChannelState::mock_server();
            let url = format!("{}/usage-test-raw", server.url());
            let target = server.mock("POST", "/api/v1/harness-support/transcript")
                .with_status(200)
                .with_body(json!({"url": url, "method": "PUT", "headers": {}}).to_string())
                .expect(1).create();
            let raw = server.mock("PUT", "/usage-test-raw")
                .match_body("{}").with_status(raw_status).expect(1).create();
            let metrics = server.mock("POST", "/api/v1/harness-support/harness-usage")
                .with_status(409).expect(usize::from(should_report)).create();
            (target, raw, metrics)
        };
        let client = Arc::new(ServerApi::new_for_test());
        let reporter = reporter(client.clone());
        let conversation = ServerConversationToken::new("synthetic-conversation".to_owned());
        let persistence = block_on(save_transcript_and_block(
            upload_capture(&*client, &conversation, &reporter, CapturedTranscript {
                body: b"{}".to_vec(),
                report: usable.then(report),
                needs_retry: false,
            }),
            future::ready(Err(anyhow!("block unavailable"))),
        ));
        assert!(persistence.is_err());
        block_on(reporter.publish());
        block_on(reporter.publish());
        assert_eq!(reporter.disabled.load(Ordering::Relaxed), should_report);
        target.assert();
        raw.assert();
        metrics.assert();
    }
}

#[tokio::test]
async fn a_later_read_error_preserves_the_last_usable_capture() {
    let mut original = Some(CapturedTranscript {
        body: b"captured before failure".to_vec(),
        report: Some(report()),
        needs_retry: true,
    });
    let captured = capture_with_retry(true, || {
        future::ready(original.take().ok_or_else(|| anyhow!("read failed")))
    }).await.unwrap();
    assert_eq!(captured.body, b"captured before failure");
    let retained = captured.report.unwrap();
    assert_eq!(retained.capture_sequence, 1);
    assert_eq!(retained.captured_at, report().captured_at);
}
