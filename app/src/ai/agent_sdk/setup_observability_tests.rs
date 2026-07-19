use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::time::Duration;

use warpui::r#async::executor::Background;

use super::{NamespaceCacheMountReport, SetupClientEventReporter};
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::server::server_api::ai::{
    AgentRunClientCacheInvocationPayload, AgentRunClientCacheModePayload,
    AgentRunClientEventRequest, MockAIClient,
};

fn run_id() -> AmbientAgentTaskId {
    "550e8400-e29b-41d4-a716-446655440000".parse().unwrap()
}

fn reporter(
    completed: Arc<AtomicBool>,
) -> (
    SetupClientEventReporter,
    Receiver<AgentRunClientEventRequest>,
) {
    let (sender, receiver) = mpsc::channel();
    let mut ai_client = MockAIClient::new();
    ai_client
        .expect_post_agent_run_client_event()
        .times(1)
        .returning(move |posted_run_id, request| {
            assert_eq!(*posted_run_id, run_id());
            assert!(completed.load(Ordering::SeqCst));
            sender.send(request).unwrap();
            Ok(())
        });

    (
        SetupClientEventReporter::new(
            run_id(),
            Arc::new(ai_client),
            Arc::new(Background::default()),
        ),
        receiver,
    )
}

#[tokio::test]
async fn namespace_cache_mount_posts_once_after_results_and_preserves_successes() {
    let completed = Arc::new(AtomicBool::new(false));
    let (reporter, receiver) = reporter(completed.clone());
    let successful_invocation = AgentRunClientCacheInvocationPayload::shared(
        false,
        vec![AgentRunClientCacheModePayload::new("go", 3, 1)],
    );
    let failed_invocation = AgentRunClientCacheInvocationPayload::repository(
        "b".repeat(64),
        true,
        vec![AgentRunClientCacheModePayload::new("rust", 0, 2)],
    );
    let expected_report = NamespaceCacheMountReport::new(
        false,
        vec![successful_invocation.clone(), failed_invocation.clone()],
    );

    let report = reporter
        .record_namespace_cache_mount({
            let expected_report = expected_report.clone();
            async move {
                completed.store(true, Ordering::SeqCst);
                expected_report
            }
        })
        .await;

    assert_eq!(report, expected_report);
    let request = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
    let value = serde_json::to_value(request).unwrap();
    assert_eq!(value["event_name"], "setup_namespace_cache_mount");
    assert_eq!(value["payload"]["is_error"], true);
    assert_eq!(
        value["payload"]["cache_invocations"],
        serde_json::json!([
            {
                "scope": "shared",
                "is_error": false,
                "modes": [
                    {"name": "go", "cache_hits": 3, "cache_misses": 1},
                ],
            },
            {
                "scope": "repository",
                "repo_key": "b".repeat(64),
                "is_error": true,
                "modes": [
                    {"name": "rust", "cache_hits": 0, "cache_misses": 2},
                ],
            },
        ])
    );
    assert_eq!(value["timestamp"], value["payload"]["finish_ts"]);
    assert!(value["payload"]["latency_ms"].as_i64().unwrap() >= 0);
}

#[tokio::test]
async fn namespace_cache_mount_setup_error_sets_overall_error() {
    let completed = Arc::new(AtomicBool::new(false));
    let (reporter, receiver) = reporter(completed.clone());

    reporter
        .record_namespace_cache_mount(async move {
            completed.store(true, Ordering::SeqCst);
            NamespaceCacheMountReport::new(
                true,
                vec![AgentRunClientCacheInvocationPayload::shared(
                    false,
                    vec![AgentRunClientCacheModePayload::new("go", 1, 0)],
                )],
            )
        })
        .await;

    let request = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
    let value = serde_json::to_value(request).unwrap();
    assert_eq!(value["payload"]["is_error"], true);
    assert_eq!(value["payload"]["cache_invocations"][0]["is_error"], false);
}

#[tokio::test]
async fn namespace_cache_mount_noop_reporter_does_not_post() {
    let mut ai_client = MockAIClient::new();
    ai_client.expect_post_agent_run_client_event().times(0);
    let reporter =
        SetupClientEventReporter::noop(Arc::new(ai_client), Arc::new(Background::default()));

    let report = reporter
        .record_namespace_cache_mount(async {
            NamespaceCacheMountReport::new(
                false,
                vec![AgentRunClientCacheInvocationPayload::shared(false, vec![])],
            )
        })
        .await;

    assert!(!report.setup_is_error);
}
