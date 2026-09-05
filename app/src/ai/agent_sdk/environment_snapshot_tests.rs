use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cloud_object_models::CodeForge;
use instant::Instant;
use warp_server_client::HttpStatusError;
use warpui::r#async::executor::Background;

use super::*;
use crate::server::server_api::ai::MockAIClient;

fn task_id() -> AmbientAgentTaskId {
    "550e8400-e29b-41d4-a716-446655440000".parse().unwrap()
}

fn request() -> AgentRunEnvironmentSnapshotRequest {
    EnvironmentSnapshot {
        captured_at: Utc::now(),
        repositories: vec![RepositoryRevision {
            code_forge: CodeForge::GitHub,
            repo_owner: "warpdotdev".to_string(),
            repo_name: "warp".to_string(),
            checkout_path: "warp".to_string(),
            requested_checkout_ref: Some("main".to_string()),
            resolved_head_sha: "0123456789abcdef0123456789abcdef01234567".to_string(),
        }],
    }
    .into()
}

fn background() -> Arc<Background> {
    Arc::new(Background::new(1, |_| {
        "environment-snapshot-test".to_string()
    }))
}

#[test]
fn request_conversion_uses_expected_wire_shape() {
    let request = request();
    let json = serde_json::to_value(&request).unwrap();

    assert!(json.get("snapshot_uuid").is_none());
    assert!(json.get("unresolved_repository_count").is_none());
    assert_eq!(json["repositories"][0]["code_forge"], "GITHUB");
    assert_eq!(json["repositories"][0]["repo_owner"], "warpdotdev");
    assert_eq!(json["repositories"][0]["checkout_path"], "warp");
    assert_eq!(json["repositories"][0]["requested_checkout_ref"], "main");
    assert_eq!(
        json["repositories"][0]["resolved_head_sha"],
        "0123456789abcdef0123456789abcdef01234567"
    );
}

#[test]
fn empty_snapshot_serializes_as_an_explicit_empty_report() {
    let request = AgentRunEnvironmentSnapshotRequest::from(EnvironmentSnapshot::empty());
    let json = serde_json::to_value(request).unwrap();

    assert!(json.get("unresolved_repository_count").is_none());
    assert_eq!(json["repositories"], serde_json::json!([]));
}

#[tokio::test]
async fn transient_failures_retry_with_stable_request() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let requests_for_mock = requests.clone();
    let mut mock = MockAIClient::new();
    mock.expect_post_agent_run_environment_snapshot()
        .times(2)
        .returning(move |_, request| {
            let mut requests = requests_for_mock.lock().unwrap();
            requests.push(request);
            if requests.len() == 1 {
                Err(anyhow::anyhow!("transient transport failure"))
            } else {
                Ok(())
            }
        });

    publish_with_retry(task_id(), Arc::new(mock), request())
        .await
        .unwrap();

    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0], requests[1]);
}

#[tokio::test]
async fn transient_failures_stop_at_retry_bound() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let attempts_for_mock = attempts.clone();
    let mut mock = MockAIClient::new();
    mock.expect_post_agent_run_environment_snapshot()
        .times(REPORT_MAX_ATTEMPTS)
        .returning(move |_, _| {
            attempts_for_mock.fetch_add(1, Ordering::SeqCst);
            Err(anyhow::anyhow!("transient transport failure"))
        });

    assert!(
        publish_with_retry(task_id(), Arc::new(mock), request())
            .await
            .is_err()
    );
    assert_eq!(attempts.load(Ordering::SeqCst), REPORT_MAX_ATTEMPTS);
}

#[tokio::test]
async fn permanent_http_failure_does_not_retry() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let attempts_for_mock = attempts.clone();
    let mut mock = MockAIClient::new();
    mock.expect_post_agent_run_environment_snapshot()
        .times(1)
        .returning(move |_, _| {
            attempts_for_mock.fetch_add(1, Ordering::SeqCst);
            Err(anyhow::Error::new(HttpStatusError {
                status: 400,
                body: "missing active execution".to_string(),
            }))
        });

    assert!(
        publish_with_retry(task_id(), Arc::new(mock), request())
            .await
            .is_err()
    );
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
}

#[test]
fn no_task_reporter_does_not_publish() {
    let mut mock = MockAIClient::new();
    mock.expect_post_agent_run_environment_snapshot().times(0);
    let reporter = EnvironmentSnapshotReporter::noop(Arc::new(mock), background());

    reporter.report(EnvironmentSnapshot::empty());
}

#[test]
fn report_returns_before_blocking_client_call_completes() {
    let mut mock = MockAIClient::new();
    mock.expect_post_agent_run_environment_snapshot()
        .times(1)
        .returning(|_, _| {
            std::thread::sleep(Duration::from_millis(250));
            Ok(())
        });
    let reporter = EnvironmentSnapshotReporter::new(task_id(), Arc::new(mock), background());

    let start = Instant::now();
    reporter.report(EnvironmentSnapshot::empty());

    assert!(start.elapsed() < Duration::from_millis(50));
    std::thread::sleep(Duration::from_millis(300));
}
