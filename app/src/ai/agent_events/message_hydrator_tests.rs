use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use mockall::predicate::eq;

use super::*;
use crate::server::server_api::ai::{
    AIClient, AgentRunEvent, MockAIClient, ReadAgentMessageResponse,
};
use crate::server::server_api::presigned_upload::HttpStatusError;

fn make_run_event(
    sequence: i64,
    event_type: &str,
    run_id: &str,
    ref_id: Option<&str>,
) -> AgentRunEvent {
    AgentRunEvent {
        event_type: event_type.to_string(),
        run_id: run_id.to_string(),
        ref_id: ref_id.map(|value| value.to_string()),
        execution_id: None,
        occurred_at: "2026-01-01T00:00:00Z".to_string(),
        sequence,
    }
}

fn make_message_response(message_id: &str) -> ReadAgentMessageResponse {
    ReadAgentMessageResponse {
        message_id: message_id.to_string(),
        sender_run_id: "parent-run".to_string(),
        subject: "Need a redirect".to_string(),
        body: "Switch to the failing test first.".to_string(),
        sent_at: "2026-01-01T00:00:00Z".to_string(),
        delivered_at: None,
        read_at: Some("2026-01-01T00:00:01Z".to_string()),
    }
}

fn http_status_read_error(status: u16) -> anyhow::Error {
    anyhow::Error::new(HttpStatusError {
        status,
        body: format!("status {status} body"),
    })
}

#[tokio::test]
async fn hydrator_reads_new_message_for_matching_run() {
    let mut ai_client = MockAIClient::new();
    ai_client
        .expect_read_agent_message()
        .with(eq("msg-123"))
        .times(1)
        .returning(|_| Ok(make_message_response("msg-123")));

    let ai_client: Arc<dyn AIClient> = Arc::new(ai_client);
    let hydrator = MessageHydrator::new(ai_client);
    let event = make_run_event(7, "new_message", "child-run", Some("msg-123"));

    let hydrated = hydrator
        .hydrate_event_for_recipient(&event, "child-run")
        .await
        .expect("expected hydrated message");

    assert_eq!(hydrated.message_id, "msg-123");
    assert_eq!(hydrated.sender_agent_id, "parent-run");
    assert_eq!(hydrated.subject, "Need a redirect");
    assert_eq!(hydrated.message_body, "Switch to the failing test first.");
    assert_eq!(hydrated.addresses, vec!["child-run".to_string()]);
}

#[tokio::test]
async fn hydrator_ignores_events_for_other_runs() {
    let ai_client: Arc<dyn AIClient> = Arc::new(MockAIClient::new());
    let hydrator = MessageHydrator::new(ai_client);
    let event = make_run_event(7, "new_message", "other-run", Some("msg-123"));

    assert!(
        hydrator
            .hydrate_event_for_recipient(&event, "child-run")
            .await
            .is_none()
    );
}

#[tokio::test]
async fn read_message_with_timeout_retries_transient_failures_until_success() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let attempts_clone = attempts.clone();
    let mut ai_client = MockAIClient::new();
    ai_client
        .expect_read_agent_message()
        .with(eq("msg-123"))
        .times(2)
        .returning(move |_| {
            let attempt = attempts_clone.fetch_add(1, Ordering::SeqCst);
            if attempt == 0 {
                Err(http_status_read_error(404))
            } else {
                Ok(make_message_response("msg-123"))
            }
        });

    let ai_client: Arc<dyn AIClient> = Arc::new(ai_client);
    let hydrator = MessageHydrator::with_fetch_timing(
        ai_client,
        Duration::from_millis(100),
        Duration::from_millis(5),
    );

    let message = hydrator.read_message_with_timeout("msg-123").await.unwrap();

    assert_eq!(attempts.load(Ordering::SeqCst), 2);
    assert_eq!(message.message_id, "msg-123");
    assert_eq!(message.body, "Switch to the failing test first.");
}

#[tokio::test]
async fn read_message_with_timeout_times_out_after_retrying_transient_failures() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let attempts_clone = attempts.clone();
    let mut ai_client = MockAIClient::new();
    ai_client
        .expect_read_agent_message()
        .with(eq("msg-123"))
        .returning(move |_| {
            attempts_clone.fetch_add(1, Ordering::SeqCst);
            Err(http_status_read_error(404))
        });

    let ai_client: Arc<dyn AIClient> = Arc::new(ai_client);
    let hydrator = MessageHydrator::with_fetch_timing(
        ai_client,
        Duration::from_millis(120),
        Duration::from_millis(20),
    );

    let err = hydrator
        .read_message_with_timeout("msg-123")
        .await
        .expect_err("expected timeout after transient retries");
    let err_chain = format!("{err:#}");

    assert!(
        err_chain.contains("Timed out reading agent message msg-123"),
        "{err:#}"
    );
    assert!(
        attempts.load(Ordering::SeqCst) >= 2,
        "expected at least one retry before timeout"
    );
}

#[tokio::test]
async fn read_message_with_timeout_does_not_retry_permanent_http_failures() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let attempts_clone = attempts.clone();
    let mut ai_client = MockAIClient::new();
    ai_client
        .expect_read_agent_message()
        .with(eq("msg-123"))
        .times(1)
        .returning(move |_| {
            attempts_clone.fetch_add(1, Ordering::SeqCst);
            Err(http_status_read_error(403))
        });

    let ai_client: Arc<dyn AIClient> = Arc::new(ai_client);
    let hydrator = MessageHydrator::with_fetch_timing(
        ai_client,
        Duration::from_millis(100),
        Duration::from_millis(5),
    );

    let err = hydrator
        .read_message_with_timeout("msg-123")
        .await
        .expect_err("expected permanent failure to fail fast");
    let err_chain = format!("{err:#}");

    assert!(
        err_chain.contains("HTTP request failed with status 403"),
        "{err:#}"
    );
    assert_eq!(
        attempts.load(Ordering::SeqCst),
        1,
        "permanent 4xx errors should not retry"
    );
}
