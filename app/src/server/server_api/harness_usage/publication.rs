//! Task-authenticated publication of native harness usage.
use std::time::Duration;

use anyhow::{Result, ensure};
use chrono::{DateTime, Utc};
use http::header::{CONTENT_TYPE, RETRY_AFTER};
use http_client::StatusCode;
use serde::{Deserialize, Deserializer};
use serde_json::Value;
use warp_harness_usage::api::HarnessUsageRequest;

use super::super::ServerApi;
use crate::ai::ambient_agents::AmbientAgentTaskId;
mod wire;

const MAX_BODY_BYTES: usize = 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

pub(super) fn encode_request(request: &HarnessUsageRequest) -> Result<Vec<u8>> {
    ensure!(
        request.execution_id > 0 && request.capture_sequence > 0,
        "Invalid harness capture identity"
    );
    ensure!(
        request.has_usable_category(),
        "No usable harness usage category"
    );
    let body = serde_json::to_vec(request)?;
    ensure!(
        body.len() <= MAX_BODY_BYTES,
        "Harness usage body exceeds limit"
    );
    Ok(body)
}

/// Execution ownership supplied only by authenticated, reporting-enabled startup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HarnessUsageCapability {
    pub execution_id: i64,
}

pub(super) fn deserialize_harness_usage_capability<'de, D>(
    deserializer: D,
) -> Result<Option<HarnessUsageCapability>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    // Malformed capability data must not break raw transcript persistence or resume.
    Ok(serde_json::from_value::<wire::StartupCapability>(value)
        .ok()
        .filter(|capability| capability.execution_id > 0)
        .map(|capability| HarnessUsageCapability {
            execution_id: capability.execution_id,
        }))
}

/// Server disposition for one cumulative usage capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HarnessUsagePublicationStatus {
    /// The server retained this capture as the newest cumulative value.
    Accepted,
    /// The server had already retained a newer capture.
    IgnoredOlderCapture,
    /// The server had already retained this exact capture.
    Idempotent,
}

/// Determines whether publication may retry or should stop for this execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HarnessUsageErrorKind {
    /// A transient failure eligible for bounded retry.
    Retryable,
    /// The server does not support or has disabled publication.
    Disabled,
    /// The server rejected the task authentication.
    Unauthorized,
    /// The capture conflicts with the server's execution state.
    Conflict,
    /// The request failed local validation or server validation.
    InvalidReport,
    /// A successful response did not acknowledge a valid capture identity.
    InvalidResponse,
}

/// Safe publication diagnostics that never retain response bodies or credential errors.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("Harness usage publication failed: {kind:?} (HTTP {status:?})")]
pub struct HarnessUsageError {
    pub kind: HarnessUsageErrorKind,
    pub status: Option<StatusCode>,
    pub retry_after: Option<Duration>,
}

impl HarnessUsageError {
    pub fn new(kind: HarnessUsageErrorKind) -> Self {
        Self {
            kind,
            status: None,
            retry_after: None,
        }
    }

    pub(super) fn from_request_preparation_error(_: anyhow::Error) -> Self {
        Self::new(HarnessUsageErrorKind::Retryable)
    }

    async fn from_response(response: http_client::Response) -> Self {
        let status = response.status();
        let retry_after = response
            .headers()
            .get(RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| parse_harness_usage_retry_after(value, Utc::now()));
        let problem = response.json::<wire::Problem>().await.ok();
        let problem_type = problem.as_ref().map(|problem| problem.problem_type);
        let kind = if matches!(
            problem_type,
            Some(wire::ProblemType::Disabled | wire::ProblemType::Unsupported)
        ) || matches!(
            status,
            StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED | StatusCode::NOT_IMPLEMENTED
        ) {
            HarnessUsageErrorKind::Disabled
        } else if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
            HarnessUsageErrorKind::Unauthorized
        } else if matches!(
            status,
            StatusCode::CONFLICT | StatusCode::PRECONDITION_FAILED
        ) {
            HarnessUsageErrorKind::Conflict
        } else if matches!(
            status,
            StatusCode::REQUEST_TIMEOUT | StatusCode::TOO_MANY_REQUESTS
        ) || (status.is_server_error()
            && problem.and_then(|problem| problem.retryable) != Some(false))
        {
            HarnessUsageErrorKind::Retryable
        } else {
            HarnessUsageErrorKind::InvalidReport
        };
        Self {
            kind,
            status: Some(status),
            retry_after,
        }
    }
}

pub(super) fn parse_harness_usage_retry_after(value: &str, now: DateTime<Utc>) -> Option<Duration> {
    value
        .trim()
        .parse::<u64>()
        .map(Duration::from_secs)
        .ok()
        .or_else(|| {
            DateTime::parse_from_rfc2822(value).ok().map(|date| {
                (date.with_timezone(&Utc) - now)
                    .to_std()
                    .unwrap_or_default()
            })
        })
}

impl ServerApi {
    /// Makes one task-authenticated publication attempt and validates its acknowledgment.
    pub async fn publish_harness_usage_for_task(
        &self,
        task_id: &AmbientAgentTaskId,
        request: &HarnessUsageRequest,
    ) -> Result<HarnessUsagePublicationStatus, HarnessUsageError> {
        let body = encode_request(request)
            .map_err(|_| HarnessUsageError::new(HarnessUsageErrorKind::InvalidReport))?;
        let auth_token = self
            .get_or_refresh_access_token()
            .await
            .map_err(HarnessUsageError::from_request_preparation_error)?;
        let url = format!(
            "{}/api/v1/harness-support/usage",
            crate::ChannelState::server_root_url()
        );
        let mut http_request = self
            .base_client
            .http_client()
            .post(&url)
            .header(CONTENT_TYPE, "application/json")
            .body(body)
            .timeout(REQUEST_TIMEOUT);
        if let Some(token) = auth_token.as_bearer_token() {
            http_request = http_request.bearer_auth(token);
        }
        for (name, value) in self
            .ambient_agent_headers_for_task(task_id)
            .await
            .map_err(HarnessUsageError::from_request_preparation_error)?
        {
            http_request = http_request.header(name, value);
        }
        let response = http_request
            .send()
            .await
            .map_err(|_| HarnessUsageError::new(HarnessUsageErrorKind::Retryable))?;
        if !response.status().is_success() {
            self.observe_iap_challenge(&response);
            return Err(HarnessUsageError::from_response(response).await);
        }
        let acknowledgment = response
            .json::<wire::PublicationAcknowledgment>()
            .await
            .map_err(|error| {
                let kind = if error.is_decode() {
                    HarnessUsageErrorKind::InvalidResponse
                } else {
                    HarnessUsageErrorKind::Retryable
                };
                HarnessUsageError::new(kind)
            })?;
        if acknowledgment.execution_id <= 0
            || acknowledgment.capture_sequence <= 0
            || (acknowledgment.status != HarnessUsagePublicationStatus::IgnoredOlderCapture
                && (acknowledgment.execution_id != request.execution_id
                    || acknowledgment.capture_sequence != request.capture_sequence))
        {
            return Err(HarnessUsageError::new(
                HarnessUsageErrorKind::InvalidResponse,
            ));
        }
        Ok(acknowledgment.status)
    }
}
