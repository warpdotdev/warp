use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::Serialize;

/// One cumulative harness usage capture.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct HarnessUsageRequest {
    pub execution_id: i64,
    pub capture_sequence: i64,
    pub captured_at: DateTime<Utc>,
    #[serde(flatten)]
    pub snapshot: HarnessUsageSnapshot,
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

    pub fn has_usable_category(&self) -> bool {
        self.snapshot.has_usable_category()
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

impl HarnessUsageSnapshot {
    fn has_usable_category(&self) -> bool {
        let coverage = match self {
            Self::ClaudeCode(snapshot) => &snapshot.coverage,
            Self::Codex(snapshot) => &snapshot.coverage,
        };
        coverage.token_status != CoverageStatus::Unavailable
            || coverage.tool_status != CoverageStatus::Unavailable
    }
}

/// Publishable usage and coverage for one provider.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct UsageSnapshot<T> {
    pub coverage: Coverage,
    pub payload: UsagePayload<T>,
}

/// Independent confidence information for token and tool-call extraction.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Coverage {
    pub token_status: CoverageStatus,
    pub tool_status: CoverageStatus,
}

/// Confidence level for one extracted usage category.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverageStatus {
    Known,
    Partial,
    Unavailable,
}

/// Usage, attribution breakdowns, and tool calls for one provider.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct UsagePayload<T> {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<T>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub attribution: Vec<AttributedUsage<T>>,
    #[serde(rename = "toolCalls", skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<ToolCalls>,
}

/// Usage associated with one set of observed classifications.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AttributedUsage<T> {
    #[serde(flatten)]
    pub attribution: Attribution,
    pub usage: T,
}

/// Classifications attached to an observed usage group.
#[derive(Clone, Debug, Default, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct Attribution {
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

/// Cumulative token usage reported by Codex rollout checkpoints.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CodexUsage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cached_input_tokens: Option<i64>,
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
