use std::sync::atomic::{AtomicUsize, Ordering};

use chrono::Utc;
use cloud_object_models::CodeForge;
use serde_json::json;

use super::*;
use crate::server::server_api::ai::{MockAIClient, RepositoryRevisionRequest};
use crate::server::server_api::presigned_upload::HttpStatusError;

fn task_id() -> AmbientAgentTaskId {
    "550e8400-e29b-41d4-a716-446655440000".parse().unwrap()
}

fn request() -> RepositoryRevisionSnapshotRequest {
    RepositoryRevisionSnapshotRequest {
        snapshot_uuid: "ea954dd4-4d72-492d-bddd-f3336fc33575".to_string(),
        captured_at: Utc::now(),
        unresolved_repository_count: 0,
        repositories: Vec::new(),
    }
}

#[test]
fn request_serializes_shared_wire_contract() {
    let request = RepositoryRevisionSnapshotRequest {
        snapshot_uuid: "ea954dd4-4d72-492d-bddd-f3336fc33575".to_string(),
        captured_at: "2026-08-19T23:00:00Z".parse().unwrap(),
        unresolved_repository_count: 1,
        repositories: vec![RepositoryRevisionRequest {
            code_forge: CodeForge::GitHub,
            owner: "warpdotdev".to_string(),
            repo: "warp".to_string(),
            checkout_path: "warp".to_string(),
            checkout_ref: Some("main".to_string()),
            head_sha: "0123456789abcdef0123456789abcdef01234567".to_string(),
        }],
    };

    assert_eq!(
        serde_json::to_value(request).unwrap(),
        json!({
            "snapshot_uuid": "ea954dd4-4d72-492d-bddd-f3336fc33575",
            "captured_at": "2026-08-19T23:00:00Z",
            "unresolved_repository_count": 1,
            "repositories": [{
                "code_forge": "GITHUB",
                "owner": "warpdotdev",
                "repo": "warp",
                "checkout_path": "warp",
                "checkout_ref": "main",
                "head_sha": "0123456789abcdef0123456789abcdef01234567"
            }]
        })
    );
}

#[tokio::test]
async fn transient_failure_retries_with_identical_snapshot() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let attempts_for_mock = attempts.clone();
    let mut client = MockAIClient::new();
    client
        .expect_post_repository_revision_snapshot()
        .times(2)
        .withf(|run_id, request| {
            run_id.to_string() == "550e8400-e29b-41d4-a716-446655440000"
                && request.snapshot_uuid == "ea954dd4-4d72-492d-bddd-f3336fc33575"
        })
        .returning(move |_, _| {
            if attempts_for_mock.fetch_add(1, Ordering::SeqCst) == 0 {
                Err(HttpStatusError {
                    status: 503,
                    body: "unavailable".to_string(),
                }
                .into())
            } else {
                Ok(())
            }
        });

    post_snapshot_with_retry(task_id(), Arc::new(client), request())
        .await
        .unwrap();

    assert_eq!(attempts.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn permanent_failure_does_not_retry() {
    let mut client = MockAIClient::new();
    client
        .expect_post_repository_revision_snapshot()
        .times(1)
        .returning(|_, _| {
            Err(HttpStatusError {
                status: 404,
                body: "not found".to_string(),
            }
            .into())
        });

    let result = post_snapshot_with_retry(task_id(), Arc::new(client), request()).await;

    assert!(result.is_err());
}
