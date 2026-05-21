use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::anyhow;
use async_trait::async_trait;
use futures::stream::{self, BoxStream};
use futures::StreamExt;

use super::*;
use crate::server::server_api::ai::AgentRunEvent;

const ZERO_BACKOFF_STEPS: &[u64] = &[0];

struct FakeAgentEventSource {
    responses: Mutex<VecDeque<anyhow::Result<Vec<anyhow::Result<AgentEventSourceItem>>>>>,
}

impl FakeAgentEventSource {
    fn new(responses: Vec<anyhow::Result<Vec<anyhow::Result<AgentEventSourceItem>>>>) -> Self {
        Self {
            responses: Mutex::new(VecDeque::from(responses)),
        }
    }
}

#[async_trait]
impl AgentEventSource for FakeAgentEventSource {
    async fn open_stream(
        &self,
        _run_ids: &[String],
        _since_sequence: i64,
    ) -> anyhow::Result<BoxStream<'static, anyhow::Result<AgentEventSourceItem>>> {
        let response = self
            .responses
            .lock()
            .unwrap()
            .pop_front()
            .expect("fake response missing");
        let stream = response?;
        Ok(stream::iter(stream).boxed())
    }
}

#[derive(Default)]
struct RecordingConsumer {
    handled_sequences: Vec<i64>,
    persisted_sequences: Vec<i64>,
    driver_states: Vec<AgentEventDriverState>,
    stop_after: usize,
    fail_persist_cursor: bool,
    fail_driver_state: bool,
}

#[async_trait]
impl AgentEventConsumer for RecordingConsumer {
    async fn on_event(
        &mut self,
        event: AgentRunEvent,
    ) -> anyhow::Result<AgentEventConsumerControlFlow> {
        self.handled_sequences.push(event.sequence);
        if self.handled_sequences.len() >= self.stop_after {
            Ok(AgentEventConsumerControlFlow::Stop)
        } else {
            Ok(AgentEventConsumerControlFlow::Continue)
        }
    }

    async fn persist_cursor(&mut self, sequence: i64) -> anyhow::Result<()> {
        self.persisted_sequences.push(sequence);
        if self.fail_persist_cursor {
            Err(anyhow!("persist failed"))
        } else {
            Ok(())
        }
    }

    async fn on_driver_state(&mut self, state: AgentEventDriverState) -> anyhow::Result<()> {
        self.driver_states.push(state);
        if self.fail_driver_state {
            Err(anyhow!("state callback failed"))
        } else {
            Ok(())
        }
    }
}

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

fn ok_stream(
    items: Vec<anyhow::Result<AgentEventSourceItem>>,
) -> anyhow::Result<Vec<anyhow::Result<AgentEventSourceItem>>> {
    Ok(items)
}

#[tokio::test]
async fn driver_skips_duplicate_sequences_and_persists_new_cursor() {
    let source = FakeAgentEventSource::new(vec![ok_stream(vec![
        Ok(AgentEventSourceItem::Open),
        Ok(AgentEventSourceItem::Event(make_run_event(
            2,
            "new_message",
            "child-run",
            Some("msg-2"),
        ))),
        Ok(AgentEventSourceItem::Event(make_run_event(
            3,
            "new_message",
            "child-run",
            Some("msg-3"),
        ))),
        Ok(AgentEventSourceItem::Event(make_run_event(
            4,
            "new_message",
            "child-run",
            Some("msg-4"),
        ))),
    ])]);
    let mut consumer = RecordingConsumer {
        stop_after: 2,
        ..Default::default()
    };

    let config = AgentEventDriverConfig {
        run_ids: vec!["child-run".to_string()],
        since_sequence: 2,
        reconnect_backoff_steps: DEFAULT_AGENT_EVENT_RECONNECT_BACKOFF_STEPS,
        permanent_error_backoff_steps: DEFAULT_PERMANENT_ERROR_BACKOFF_STEPS,
        proactive_reconnect_after: None,
        failures_before_error_log: DEFAULT_AGENT_EVENT_FAILURES_BEFORE_ERROR_LOG,
    };

    run_agent_event_driver(source, config, &mut consumer)
        .await
        .unwrap();

    assert_eq!(consumer.handled_sequences, vec![3, 4]);
    assert_eq!(consumer.persisted_sequences, vec![3, 4]);
}

#[tokio::test]
async fn driver_resets_failures_after_successful_event_delivery() {
    let source = FakeAgentEventSource::new(vec![
        ok_stream(vec![Ok(AgentEventSourceItem::Open), Err(anyhow!("boom-1"))]),
        ok_stream(vec![
            Ok(AgentEventSourceItem::Open),
            Ok(AgentEventSourceItem::Event(make_run_event(
                1,
                "new_message",
                "child-run",
                Some("msg-1"),
            ))),
            Err(anyhow!("boom-2")),
        ]),
        ok_stream(vec![
            Ok(AgentEventSourceItem::Open),
            Ok(AgentEventSourceItem::Event(make_run_event(
                2,
                "new_message",
                "child-run",
                Some("msg-2"),
            ))),
        ]),
    ]);
    let mut consumer = RecordingConsumer {
        stop_after: 2,
        ..Default::default()
    };

    let config = AgentEventDriverConfig {
        run_ids: vec!["child-run".to_string()],
        since_sequence: 0,
        reconnect_backoff_steps: ZERO_BACKOFF_STEPS,
        permanent_error_backoff_steps: DEFAULT_PERMANENT_ERROR_BACKOFF_STEPS,
        proactive_reconnect_after: None,
        failures_before_error_log: DEFAULT_AGENT_EVENT_FAILURES_BEFORE_ERROR_LOG,
    };

    run_agent_event_driver(source, config, &mut consumer)
        .await
        .unwrap();

    let retry_failures = consumer
        .driver_states
        .into_iter()
        .filter_map(|state| match state {
            AgentEventDriverState::RetryScheduled {
                consecutive_failures,
                ..
            } => Some(consecutive_failures),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(retry_failures, vec![1, 1]);
}

#[tokio::test]
async fn driver_ignores_persist_cursor_errors() {
    let source = FakeAgentEventSource::new(vec![ok_stream(vec![
        Ok(AgentEventSourceItem::Open),
        Ok(AgentEventSourceItem::Event(make_run_event(
            1,
            "new_message",
            "child-run",
            Some("msg-1"),
        ))),
    ])]);

    let mut consumer = RecordingConsumer {
        stop_after: 1,
        fail_persist_cursor: true,
        ..Default::default()
    };

    let config = AgentEventDriverConfig {
        run_ids: vec!["child-run".to_string()],
        since_sequence: 0,
        reconnect_backoff_steps: ZERO_BACKOFF_STEPS,
        permanent_error_backoff_steps: DEFAULT_PERMANENT_ERROR_BACKOFF_STEPS,
        proactive_reconnect_after: None,
        failures_before_error_log: DEFAULT_AGENT_EVENT_FAILURES_BEFORE_ERROR_LOG,
    };

    run_agent_event_driver(source, config, &mut consumer)
        .await
        .unwrap();

    assert_eq!(consumer.handled_sequences, vec![1]);
    assert_eq!(consumer.persisted_sequences, vec![1]);
}

#[tokio::test]
async fn driver_ignores_driver_state_errors() {
    let source = FakeAgentEventSource::new(vec![ok_stream(vec![
        Ok(AgentEventSourceItem::Open),
        Ok(AgentEventSourceItem::Event(make_run_event(
            1,
            "new_message",
            "child-run",
            Some("msg-1"),
        ))),
    ])]);
    let mut consumer = RecordingConsumer {
        stop_after: 1,
        fail_driver_state: true,
        ..Default::default()
    };

    let config = AgentEventDriverConfig {
        run_ids: vec!["child-run".to_string()],
        since_sequence: 0,
        reconnect_backoff_steps: ZERO_BACKOFF_STEPS,
        permanent_error_backoff_steps: DEFAULT_PERMANENT_ERROR_BACKOFF_STEPS,
        proactive_reconnect_after: None,
        failures_before_error_log: DEFAULT_AGENT_EVENT_FAILURES_BEFORE_ERROR_LOG,
    };

    run_agent_event_driver(source, config, &mut consumer)
        .await
        .unwrap();

    assert_eq!(consumer.handled_sequences, vec![1]);
}

#[tokio::test]
async fn driver_retries_initial_connection_until_stream_opens() {
    let source = FakeAgentEventSource::new(vec![
        Err(anyhow!("boom-1")),
        Err(anyhow!("boom-2")),
        ok_stream(vec![
            Ok(AgentEventSourceItem::Open),
            Ok(AgentEventSourceItem::Event(make_run_event(
                1,
                "new_message",
                "child-run",
                Some("msg-1"),
            ))),
        ]),
    ]);
    let mut consumer = RecordingConsumer {
        stop_after: 1,
        ..Default::default()
    };

    let config = AgentEventDriverConfig {
        run_ids: vec!["child-run".to_string()],
        since_sequence: 0,
        reconnect_backoff_steps: ZERO_BACKOFF_STEPS,
        permanent_error_backoff_steps: DEFAULT_PERMANENT_ERROR_BACKOFF_STEPS,
        proactive_reconnect_after: None,
        failures_before_error_log: DEFAULT_AGENT_EVENT_FAILURES_BEFORE_ERROR_LOG,
    };

    run_agent_event_driver(source, config, &mut consumer)
        .await
        .unwrap();

    assert_eq!(consumer.handled_sequences, vec![1]);
    let retry_failures = consumer
        .driver_states
        .into_iter()
        .filter_map(|state| match state {
            AgentEventDriverState::RetryScheduled {
                consecutive_failures,
                is_initial_connect,
                ..
            } if is_initial_connect => Some(consecutive_failures),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(retry_failures, vec![1, 2]);
}

#[test]
fn backoff_escalates_then_caps() {
    assert_eq!(
        agent_event_backoff(1, DEFAULT_AGENT_EVENT_RECONNECT_BACKOFF_STEPS),
        Duration::from_secs(1)
    );
    assert_eq!(
        agent_event_backoff(2, DEFAULT_AGENT_EVENT_RECONNECT_BACKOFF_STEPS),
        Duration::from_secs(2)
    );
    assert_eq!(
        agent_event_backoff(3, DEFAULT_AGENT_EVENT_RECONNECT_BACKOFF_STEPS),
        Duration::from_secs(5)
    );
    assert_eq!(
        agent_event_backoff(4, DEFAULT_AGENT_EVENT_RECONNECT_BACKOFF_STEPS),
        Duration::from_secs(10)
    );
    assert_eq!(
        agent_event_backoff(100, DEFAULT_AGENT_EVENT_RECONNECT_BACKOFF_STEPS),
        Duration::from_secs(10)
    );
}

#[test]
fn failure_threshold_is_reached_at_and_above_limit() {
    assert!(!agent_event_failures_exceeded_threshold(4, 5));
    assert!(agent_event_failures_exceeded_threshold(5, 5));
    assert!(agent_event_failures_exceeded_threshold(6, 5));
}

fn make_http_status_error(status: u16) -> anyhow::Error {
    use crate::server::server_api::presigned_upload::HttpStatusError;
    anyhow::Error::new(HttpStatusError {
        status,
        body: "not found".to_string(),
    })
    .context("SSE stream error")
}

#[tokio::test]
async fn driver_uses_slow_backoff_on_permanent_http_error() {
    let source = FakeAgentEventSource::new(vec![
        // First attempt: stream opens then returns a 404-enriched error.
        ok_stream(vec![
            Ok(AgentEventSourceItem::Open),
            Err(make_http_status_error(404)),
        ]),
        // Second attempt: succeeds with a stoppable event.
        ok_stream(vec![
            Ok(AgentEventSourceItem::Open),
            Ok(AgentEventSourceItem::Event(make_run_event(
                1,
                "new_message",
                "child-run",
                Some("msg-1"),
            ))),
        ]),
    ]);
    let mut consumer = RecordingConsumer {
        stop_after: 1,
        ..Default::default()
    };

    // Use asymmetric values: transient=9999s (would block if chosen),
    // permanent=0s (instant). Proving backoff is 0s confirms the
    // permanent schedule was selected.
    let config = AgentEventDriverConfig {
        run_ids: vec!["child-run".to_string()],
        since_sequence: 0,
        reconnect_backoff_steps: &[9999],
        permanent_error_backoff_steps: ZERO_BACKOFF_STEPS,
        proactive_reconnect_after: None,
        failures_before_error_log: DEFAULT_AGENT_EVENT_FAILURES_BEFORE_ERROR_LOG,
    };

    run_agent_event_driver(source, config, &mut consumer)
        .await
        .unwrap();

    assert_eq!(consumer.handled_sequences, vec![1]);
    // The backoff must be from the permanent schedule (0s), not transient (9999s).
    let retry_backoff = consumer
        .driver_states
        .iter()
        .find_map(|s| match s {
            AgentEventDriverState::RetryScheduled { backoff, .. } => Some(*backoff),
            _ => None,
        })
        .unwrap();
    assert_eq!(retry_backoff, Duration::from_secs(0));
}

#[tokio::test]
async fn driver_uses_fast_backoff_on_transient_http_error() {
    let source = FakeAgentEventSource::new(vec![
        // First attempt: stream opens then returns a 500-enriched error.
        ok_stream(vec![
            Ok(AgentEventSourceItem::Open),
            Err(make_http_status_error(500)),
        ]),
        // Second attempt: succeeds.
        ok_stream(vec![
            Ok(AgentEventSourceItem::Open),
            Ok(AgentEventSourceItem::Event(make_run_event(
                1,
                "new_message",
                "child-run",
                Some("msg-1"),
            ))),
        ]),
    ]);
    let mut consumer = RecordingConsumer {
        stop_after: 1,
        ..Default::default()
    };

    // Set permanent backoff to something large so we can verify it was NOT used.
    let config = AgentEventDriverConfig {
        run_ids: vec!["child-run".to_string()],
        since_sequence: 0,
        reconnect_backoff_steps: ZERO_BACKOFF_STEPS,
        permanent_error_backoff_steps: &[9999],
        proactive_reconnect_after: None,
        failures_before_error_log: DEFAULT_AGENT_EVENT_FAILURES_BEFORE_ERROR_LOG,
    };

    run_agent_event_driver(source, config, &mut consumer)
        .await
        .unwrap();

    // Should have retried with the fast backoff (0s) and succeeded.
    assert_eq!(consumer.handled_sequences, vec![1]);
    // The backoff used should be from ZERO_BACKOFF_STEPS (0s), not permanent (9999s).
    let retry_backoff = consumer
        .driver_states
        .iter()
        .find_map(|s| match s {
            AgentEventDriverState::RetryScheduled { backoff, .. } => Some(*backoff),
            _ => None,
        })
        .unwrap();
    assert_eq!(retry_backoff, Duration::from_secs(0));
}
