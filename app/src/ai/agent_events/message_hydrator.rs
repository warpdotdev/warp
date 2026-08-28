use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
#[cfg(not(target_family = "wasm"))]
use futures::future::Either;
#[cfg(not(target_family = "wasm"))]
use instant::Instant;
#[cfg(not(target_family = "wasm"))]
use reqwest::Error as ReqwestError;
#[cfg(not(target_family = "wasm"))]
use warpui::r#async::Timer;

use crate::ai::agent::ReceivedMessageInput;
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::server::server_api::ServerApi;
use crate::server::server_api::ai::{AIClient, AgentRunEvent, ReadAgentMessageResponse};
#[cfg(not(target_family = "wasm"))]
use crate::server::server_api::presigned_upload::HttpStatusError;

pub(crate) const DEFAULT_AGENT_MESSAGE_FETCH_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_AGENT_MESSAGE_RETRY_DELAY: Duration = Duration::from_millis(50);

/// Hydrates `new_message` agent events into full message payloads and delivery
/// acknowledgements.
#[derive(Clone)]
pub(crate) struct MessageHydrator {
    ai_client: Arc<dyn AIClient>,
    task_scoped_server_api: Option<Arc<ServerApi>>,
    task_id: Option<AmbientAgentTaskId>,
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    fetch_timeout: Duration,
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    retry_delay: Duration,
}

impl MessageHydrator {
    pub(crate) fn new(ai_client: Arc<dyn AIClient>) -> Self {
        Self::with_fetch_timeout(ai_client, DEFAULT_AGENT_MESSAGE_FETCH_TIMEOUT)
    }

    pub(crate) fn for_task(server_api: Arc<ServerApi>, task_id: AmbientAgentTaskId) -> Self {
        let ai_client: Arc<dyn AIClient> = server_api.clone();
        Self {
            ai_client,
            task_scoped_server_api: Some(server_api),
            task_id: Some(task_id),
            fetch_timeout: DEFAULT_AGENT_MESSAGE_FETCH_TIMEOUT,
            retry_delay: DEFAULT_AGENT_MESSAGE_RETRY_DELAY,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_fetch_timing(
        ai_client: Arc<dyn AIClient>,
        fetch_timeout: Duration,
        retry_delay: Duration,
    ) -> Self {
        Self {
            ai_client,
            task_scoped_server_api: None,
            task_id: None,
            fetch_timeout,
            retry_delay,
        }
    }

    pub(crate) fn with_fetch_timeout(
        ai_client: Arc<dyn AIClient>,
        fetch_timeout: Duration,
    ) -> Self {
        Self {
            ai_client,
            task_scoped_server_api: None,
            task_id: None,
            fetch_timeout,
            retry_delay: DEFAULT_AGENT_MESSAGE_RETRY_DELAY,
        }
    }

    async fn read_message(&self, message_id: &str) -> Result<ReadAgentMessageResponse> {
        match (self.task_scoped_server_api.as_ref(), self.task_id) {
            (Some(server_api), Some(task_id)) => {
                server_api
                    .read_agent_message_for_task(&task_id, message_id)
                    .await
            }
            _ => self.ai_client.read_agent_message(message_id).await,
        }
        .with_context(|| format!("Failed to read agent message {message_id}"))
    }

    pub(crate) async fn hydrate_event_for_recipient(
        &self,
        event: &AgentRunEvent,
        recipient_run_id: &str,
    ) -> Option<ReceivedMessageInput> {
        if event.event_type != "new_message" || event.run_id != recipient_run_id {
            return None;
        }

        let message = match self.read_message_from_event_with_timeout(event).await {
            Ok(message) => message,
            Err(err) => {
                log::warn!(
                    "Failed to hydrate agent message for event ref_id={:?}: {err:#}",
                    event.ref_id
                );
                return None;
            }
        };
        if message.body.is_empty() {
            log::warn!(
                "Hydrated empty-body agent message: message_id={} event_sequence={} recipient_run_id={} sender_run_id={} subject={:?} task_id={:?}",
                message.message_id,
                event.sequence,
                recipient_run_id,
                message.sender_run_id,
                message.subject,
                self.task_id.map(|task_id| task_id.to_string())
            );
        }

        Some(ReceivedMessageInput {
            message_id: message.message_id,
            sender_agent_id: message.sender_run_id,
            addresses: vec![recipient_run_id.to_string()],
            subject: message.subject,
            message_body: message.body,
        })
    }

    #[cfg(not(target_family = "wasm"))]
    pub(crate) async fn read_message_with_timeout(
        &self,
        message_id: &str,
    ) -> Result<ReadAgentMessageResponse> {
        let deadline = Instant::now() + self.fetch_timeout;
        let mut last_error = None;

        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(message_read_timeout_error(message_id, last_error));
            }

            let read_message = self.read_message(message_id);
            let timeout = Timer::after(remaining);
            futures::pin_mut!(read_message);
            futures::pin_mut!(timeout);

            match futures::future::select(read_message, timeout).await {
                Either::Left((Ok(message), _)) => return Ok(message),
                Either::Left((Err(err), _)) if should_retry_message_read_error(&err) => {
                    last_error = Some(err);
                    let sleep_duration = self
                        .retry_delay
                        .min(deadline.saturating_duration_since(Instant::now()));
                    if sleep_duration.is_zero() {
                        return Err(message_read_timeout_error(message_id, last_error));
                    }
                    Timer::after(sleep_duration).await;
                }
                Either::Left((Err(err), _)) => return Err(err),
                Either::Right(_) => return Err(message_read_timeout_error(message_id, last_error)),
            }
        }
    }

    #[cfg(target_family = "wasm")]
    pub(crate) async fn read_message_with_timeout(
        &self,
        message_id: &str,
    ) -> Result<ReadAgentMessageResponse> {
        self.read_message(message_id).await
    }

    pub(crate) async fn read_message_from_event_with_timeout(
        &self,
        event: &AgentRunEvent,
    ) -> Result<ReadAgentMessageResponse> {
        let Some(message_id) = event.ref_id.as_deref() else {
            return Err(anyhow!("Agent event is missing ref_id"));
        };
        self.read_message_with_timeout(message_id).await
    }

    pub(crate) async fn mark_message_delivered(&self, message_id: &str) -> Result<()> {
        match (self.task_scoped_server_api.as_ref(), self.task_id) {
            (Some(server_api), Some(task_id)) => {
                server_api
                    .mark_message_delivered_for_task(&task_id, message_id)
                    .await
            }
            _ => self.ai_client.mark_message_delivered(message_id).await,
        }
        .with_context(|| format!("Failed to mark agent message {message_id} as delivered"))
    }

    pub(crate) async fn mark_messages_delivered_best_effort<'a, I>(
        &self,
        message_ids: I,
    ) -> Vec<(String, anyhow::Error)>
    where
        I: IntoIterator<Item = &'a str>,
    {
        let mut failures = Vec::new();
        // TODO(REMOTE-1266): Parallelize delivery acknowledgements for bursty
        // batches once the parent-bridge restore path is hardened enough to
        // tolerate a concurrent FuturesUnordered/join_all flow here.
        for message_id in message_ids {
            if let Err(err) = self.mark_message_delivered(message_id).await {
                failures.push((message_id.to_string(), err));
            }
        }
        failures
    }
}

#[cfg(not(target_family = "wasm"))]
fn should_retry_message_read_error(err: &anyhow::Error) -> bool {
    // Immediate read-after-event lag can surface as a short-lived 404 before
    // the message row becomes readable. Restrict retries to status-preserving
    // eventual-consistency/server responses plus clearly transient transport
    // failures so permanent 4xxs fail fast instead of timing out.
    if let Some(status) = message_read_error_status(err) {
        return matches!(status, 404 | 408 | 429 | 500..=599);
    }

    err.chain().any(|cause| {
        cause
            .downcast_ref::<ReqwestError>()
            .is_some_and(is_transient_message_read_transport_error)
    })
}

#[cfg(not(target_family = "wasm"))]
fn message_read_error_status(err: &anyhow::Error) -> Option<u16> {
    err.chain().find_map(|cause| {
        cause
            .downcast_ref::<HttpStatusError>()
            .map(|http_err| http_err.status)
    })
}

#[cfg(not(target_family = "wasm"))]
fn is_transient_message_read_transport_error(err: &ReqwestError) -> bool {
    err.is_timeout() || err.is_connect()
}

#[cfg(not(target_family = "wasm"))]
fn message_read_timeout_error(
    message_id: &str,
    last_error: Option<anyhow::Error>,
) -> anyhow::Error {
    let timeout_error = anyhow!("Timed out reading agent message {message_id}");
    match last_error {
        Some(err) => timeout_error.context(err),
        None => timeout_error,
    }
}
