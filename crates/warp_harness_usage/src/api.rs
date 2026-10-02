use std::collections::BTreeMap;
use std::mem;

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::Findings;
use crate::counters::Counters;

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

fn compact_snapshot<T>(
    snapshot: &mut UsageSnapshot<T>,
    budget: usize,
) -> Result<(), serde_json::Error>
where
    T: Serialize + From<Counters<6>>,
    for<'a> Counters<6>: From<&'a T>,
{
    let mut rows = mem::take(&mut snapshot.payload.requests);
    let sizes = rows
        .iter()
        .map(|row| serde_json::to_vec(row).map(|bytes| bytes.len()))
        .collect::<Result<Vec<_>, _>>()?;
    let mut row_bytes = sizes.iter().sum::<usize>();
    let mut remainder = snapshot
        .payload
        .unattributed_usage
        .as_ref()
        .map(Counters::from)
        .unwrap_or_default();
    remainder.restore_overflow(&snapshot.payload.unattributed_overflowed);
    snapshot.payload.unattributed_usage = None;
    snapshot.coverage.token_status = CoverageStatus::Partial;
    let base_size = serde_json::to_vec(&snapshot)?.len();
    let mut findings = Findings::default();
    loop {
        let remainder_size = if remainder.any() {
            let usage = T::from(remainder.clone());
            b",\"unattributed_usage\":".len() + serde_json::to_vec(&usage)?.len()
        } else {
            0
        };
        if base_size + row_bytes + rows.len().saturating_sub(1) + remainder_size <= budget {
            break;
        }
        let Some(row) = rows.pop() else {
            break;
        };
        row_bytes -= sizes[rows.len()];
        remainder.add(&Counters::from(&row.usage), &mut findings);
    }
    snapshot.payload.requests = rows;
    snapshot.payload.unattributed_usage = remainder.any().then(|| T::from(remainder.clone()));
    snapshot.payload.unattributed_overflowed = remainder.overflowed.to_vec();
    if snapshot.payload.requests.is_empty() && !remainder.any() {
        snapshot.coverage.token_status = CoverageStatus::Unavailable;
    }
    Ok(())
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

    /// Fold oversized request detail into unpriced usage before freezing publication.
    ///
    /// Retained rows preserve request boundaries for threshold-aware pricing; grouped usage cannot.
    pub fn bound_to_body(&mut self) -> Result<bool, serde_json::Error> {
        let body_size = serde_json::to_vec(&self)?.len();
        if body_size <= MAX_BODY_BYTES {
            return Ok(false);
        }
        match &mut self.snapshot {
            HarnessUsageSnapshot::ClaudeCode(snapshot) => {
                let overhead = body_size - serde_json::to_vec(&snapshot)?.len();
                compact_snapshot(snapshot, MAX_BODY_BYTES.saturating_sub(overhead))?;
            }
            HarnessUsageSnapshot::Codex(snapshot) => {
                let overhead = body_size - serde_json::to_vec(&snapshot)?.len();
                compact_snapshot(snapshot, MAX_BODY_BYTES.saturating_sub(overhead))?;
            }
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

/// Native requests, unassigned usage, and tool calls for one provider.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct UsagePayload<T> {
    pub requests: Vec<RequestUsage<T>>,
    /// Unpriced usage excluded from requests: cumulative history, unconfirmed deltas, newly observed
    /// counter baselines, or request detail omitted by row/body limits.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unattributed_usage: Option<T>,
    #[serde(rename = "toolCalls", skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<ToolCalls>,
    // Keep arithmetic uncertainty sticky when publication applies a second detail bound.
    #[serde(skip)]
    pub(crate) unattributed_overflowed: Vec<bool>,
}

impl<T> UsagePayload<T> {
    pub fn new(
        requests: Vec<RequestUsage<T>>,
        unattributed_usage: Option<T>,
        tool_calls: Option<ToolCalls>,
    ) -> Self {
        Self {
            requests,
            unattributed_usage,
            tool_calls,
            unattributed_overflowed: Vec::new(),
        }
    }
}

/// Usage and observed classifications of one inference request.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RequestUsage<T> {
    #[serde(flatten)]
    pub attribution: Attribution,
    pub usage: T,
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

/// Cumulative token usage reported by Codex rollout checkpoints.
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
