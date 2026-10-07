use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Maximum encoded metrics body size, independent of raw transcript uploads.
pub const MAX_BODY_BYTES: usize = 1024 * 1024;

/// One cumulative harness usage capture.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct HarnessUsageRequest {
    pub execution_id: i64,
    pub capture_sequence: i64,
    pub captured_at: DateTime<Utc>,
    #[serde(flatten)]
    pub snapshot: HarnessUsageSnapshot,
}
/// Frozen conversation rules supplied by the server, without dollar rates.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThresholdPolicy {
    pub schema_version: u32,
    pub models: BTreeMap<String, ThresholdRule>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ThresholdRule {
    None,
    InputGt { tokens: i64 },
}

impl ThresholdPolicy {
    pub fn parse(value: serde_json::Value) -> Option<Self> {
        if serde_json::to_vec(&value).ok()?.len() > 64 * 1024 {
            return None;
        }
        if value.get("models")?.as_object()?.values().any(|rule| {
            rule.get("kind").and_then(serde_json::Value::as_str) == Some("none")
                && rule.as_object().is_none_or(|fields| fields.len() != 1)
        }) {
            return None;
        }
        let policy: Self = serde_json::from_value(value).ok()?;
        (policy.schema_version == 1
            && policy.models.len() <= 128
            && policy.models.iter().all(|(model, rule)| {
                !model.is_empty()
                    && model.len() <= 256
                    && normalize_model(model) == *model
                    && match rule {
                        ThresholdRule::None => true,
                        ThresholdRule::InputGt { tokens } => *tokens > 0,
                    }
            }))
        .then_some(policy)
    }
}

pub(crate) fn normalize_model(model: &str) -> String {
    let mut model = model.trim().to_ascii_lowercase();
    loop {
        let length = model.len();
        for suffix in ["[1m]", "-latest"] {
            if let Some(prefix) = model.strip_suffix(suffix) {
                model = prefix.to_owned();
            }
        }
        if model.len() >= 9 {
            let date = &model.as_bytes()[model.len() - 9..];
            if matches!(date[0], b'-' | b'@') && date[1..].iter().all(u8::is_ascii_digit) {
                model.truncate(model.len() - 9);
            }
        }
        if length == model.len() {
            return model;
        }
    }
}

impl HarnessUsageRequest {
    pub fn new(
        execution_id: i64,
        capture_sequence: i64,
        captured_at: DateTime<Utc>,
        snapshot: HarnessUsageSnapshot,
    ) -> Self {
        Self {
            execution_id,
            capture_sequence,
            captured_at,
            snapshot,
        }
    }

    /// Remove cost inputs in full when the publication exceeds the body limit.
    pub fn bound_to_body(&mut self) -> Result<bool, serde_json::Error> {
        let body_size = serde_json::to_vec(&self)?.len();
        if body_size <= MAX_BODY_BYTES {
            return Ok(false);
        }
        match &mut self.snapshot {
            HarnessUsageSnapshot::ClaudeCode(snapshot) => {
                snapshot.payload.cost_estimation = None;
                snapshot.coverage.cost_status = CostStatus::Unavailable;
            }
            HarnessUsageSnapshot::Codex(snapshot) => {
                snapshot.payload.cost_estimation = None;
                snapshot.coverage.cost_status = CostStatus::Unavailable;
            }
        }
        if serde_json::to_vec(&self)?.len() > MAX_BODY_BYTES {
            return Err(serde::ser::Error::custom(
                "reporting metadata exceeds 1 MiB",
            ));
        }
        Ok(true)
    }
}

/// Provider-specific usage under the shared request envelope.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(
    tag = "harness",
    content = "snapshot",
    rename_all = "SCREAMING_SNAKE_CASE"
)]
pub enum HarnessUsageSnapshot {
    ClaudeCode(UsageSnapshot<ClaudeUsage>),
    Codex(UsageSnapshot<CodexUsage>),
}

/// Publishable usage and coverage for one provider.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct UsageSnapshot<T> {
    pub coverage: Coverage,
    pub payload: UsagePayload<T>,
}

/// Independent confidence information for pricing inputs, output, and tools.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Coverage {
    pub cost_status: CostStatus,
    pub output_token_status: CoverageStatus,
    pub tool_status: CoverageStatus,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CostStatus {
    Known,
    Unavailable,
}

/// Confidence level for one extracted usage category.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverageStatus {
    Known,
    Partial,
    Unavailable,
}

/// Bounded cost inputs and independently measured reporting counters.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct UsagePayload<T> {
    pub format: PayloadFormat,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_estimation: Option<CostEstimation<T>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<i64>,
    #[serde(rename = "toolCalls", skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<ToolCalls>,
}

impl<T> UsagePayload<T> {
    pub fn new(
        cost_estimation: Option<CostEstimation<T>>,
        output_tokens: Option<i64>,
        tool_calls: Option<ToolCalls>,
    ) -> Self {
        Self {
            format: PayloadFormat::CostInputsV3,
            cost_estimation,
            output_tokens,
            tool_calls,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PayloadFormat {
    CostInputsV3,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CostEstimation<T> {
    pub groups: Vec<UsageGroup<T>>,
}

/// Native counters within one homogeneous pricing key.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct UsageGroup<T> {
    #[serde(flatten)]
    pub attribution: Attribution,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub long_context_threshold_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pre_threshold: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub post_threshold: Option<T>,
}

/// Classifications attached to an observed request.
#[derive(Clone, Debug, Default, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct Attribution {
    /// Unknown models stay absent rather than being inferred from neighboring requests.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inference_geo: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speed: Option<String>,
}

/// Counts for the same deduplicated invocation set.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ToolCalls {
    pub total: i64,
    #[serde(rename = "byName")]
    pub by_name: BTreeMap<String, i64>,
}

/// Cumulative token usage reported by Claude response messages.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ClaudeUsage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_read_input_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_creation_input_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_creation: Option<CacheCreation>,
}

/// Cache-write usage split by the provider's expiry windows.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CacheCreation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ephemeral_5m_input_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ephemeral_1h_input_tokens: Option<i64>,
}

/// Native response token usage reported by Codex.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CodexUsage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cached_input_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_write_input_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_output_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<i64>,
}

#[cfg(test)]
#[path = "api_tests.rs"]
mod tests;
