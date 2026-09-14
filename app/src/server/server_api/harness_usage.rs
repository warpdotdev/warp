use std::collections::BTreeMap;

use anyhow::{Result, bail, ensure};
use chrono::{DateTime, Utc};
use serde::Serialize;
use warp_harness_usage::{
    AttributedUsage, CacheCreation, ClaudeUsage, CodexUsage, CoverageStatus, NativePayload,
    ToolCalls, UsagePayload, UsageSnapshot,
};

const MAX_BODY_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageHarness {
    ClaudeCode,
    Codex,
}

/// One cumulative capture, retained unchanged across publication retries.
#[derive(Clone, Serialize)]
pub struct HarnessUsageReport {
    pub execution_id: i64,
    pub capture_sequence: i64,
    pub captured_at: DateTime<Utc>,
    #[serde(flatten)]
    snapshot: HarnessSnapshotWire,
}

impl HarnessUsageReport {
    pub fn new(
        harness: UsageHarness,
        execution_id: i64,
        capture_sequence: i64,
        captured_at: DateTime<Utc>,
        snapshot: &UsageSnapshot,
    ) -> Result<Self> {
        let coverage = UsageCoverageWire {
            token_status: snapshot.coverage.token_status.into(),
            tool_status: snapshot.coverage.tool_status.into(),
        };
        ensure!(
            snapshot.coverage.token_status != CoverageStatus::Unavailable
                || snapshot.coverage.tool_status != CoverageStatus::Unavailable,
            "No usable harness usage category"
        );
        let snapshot = match (harness, &snapshot.payload) {
            (UsageHarness::ClaudeCode, NativePayload::Claude(payload)) => {
                HarnessSnapshotWire::ClaudeCode(UsageSnapshotWire {
                    coverage,
                    payload: payload.into(),
                })
            }
            (UsageHarness::Codex, NativePayload::Codex(payload)) => {
                HarnessSnapshotWire::Codex(UsageSnapshotWire {
                    coverage,
                    payload: payload.into(),
                })
            }
            (UsageHarness::ClaudeCode, NativePayload::Codex(_))
            | (UsageHarness::Codex, NativePayload::Claude(_)) => {
                bail!("Harness does not match usage payload")
            }
        };
        Ok(Self {
            execution_id,
            capture_sequence,
            captured_at,
            snapshot,
        })
    }

    pub(super) fn encode(&self) -> Result<Vec<u8>> {
        ensure!(
            self.execution_id > 0 && self.capture_sequence > 0,
            "Invalid harness capture identity"
        );
        let body = serde_json::to_vec(self)?;
        ensure!(
            body.len() <= MAX_BODY_BYTES,
            "Harness usage body exceeds limit"
        );
        Ok(body)
    }
}

#[derive(Clone, Serialize)]
#[serde(
    tag = "harness",
    content = "snapshot",
    rename_all = "SCREAMING_SNAKE_CASE"
)]
enum HarnessSnapshotWire {
    ClaudeCode(UsageSnapshotWire<ClaudeTokenUsageWire>),
    Codex(UsageSnapshotWire<CodexTokenUsageWire>),
}

#[derive(Clone, Serialize)]
struct UsageSnapshotWire<T> {
    coverage: UsageCoverageWire,
    payload: UsagePayloadWire<T>,
}

#[derive(Clone, Serialize)]
struct UsageCoverageWire {
    token_status: CoverageStatusWire,
    tool_status: CoverageStatusWire,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum CoverageStatusWire {
    Known,
    Partial,
    Unavailable,
}

impl From<CoverageStatus> for CoverageStatusWire {
    fn from(status: CoverageStatus) -> Self {
        match status {
            CoverageStatus::Known => Self::Known,
            CoverageStatus::Partial => Self::Partial,
            CoverageStatus::Unavailable => Self::Unavailable,
        }
    }
}

#[derive(Clone, Serialize)]
struct UsagePayloadWire<T> {
    #[serde(skip_serializing_if = "Option::is_none")]
    usage: Option<T>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    attribution: Vec<AttributedUsageWire<T>>,
    #[serde(rename = "toolCalls", skip_serializing_if = "Option::is_none")]
    tool_calls: Option<ToolCallsWire>,
}

impl<T, U> From<&UsagePayload<T>> for UsagePayloadWire<U>
where
    for<'a> U: From<&'a T>,
{
    fn from(payload: &UsagePayload<T>) -> Self {
        Self {
            usage: payload.usage.as_ref().map(U::from),
            attribution: payload.attribution.iter().map(Into::into).collect(),
            tool_calls: payload.tool_calls.as_ref().map(Into::into),
        }
    }
}

#[derive(Clone, Serialize)]
struct AttributedUsageWire<T> {
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    service_tier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    inference_geo: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    speed: Option<String>,
    usage: T,
}

impl<T, U> From<&AttributedUsage<T>> for AttributedUsageWire<U>
where
    for<'a> U: From<&'a T>,
{
    fn from(usage: &AttributedUsage<T>) -> Self {
        Self {
            model: usage.attribution.model.clone(),
            service_tier: usage.attribution.service_tier.clone(),
            inference_geo: usage.attribution.inference_geo.clone(),
            speed: usage.attribution.speed.clone(),
            usage: (&usage.usage).into(),
        }
    }
}

#[derive(Clone, Serialize)]
struct ClaudeTokenUsageWire {
    #[serde(skip_serializing_if = "Option::is_none")]
    input_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_read_input_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_creation_input_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_creation: Option<CacheCreationWire>,
}

impl From<&ClaudeUsage> for ClaudeTokenUsageWire {
    fn from(usage: &ClaudeUsage) -> Self {
        Self {
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            cache_read_input_tokens: usage.cache_read_input_tokens,
            cache_creation_input_tokens: usage.cache_creation_input_tokens,
            cache_creation: usage.cache_creation.as_ref().map(Into::into),
        }
    }
}

#[derive(Clone, Serialize)]
struct CacheCreationWire {
    #[serde(skip_serializing_if = "Option::is_none")]
    ephemeral_5m_input_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ephemeral_1h_input_tokens: Option<i64>,
}

impl From<&CacheCreation> for CacheCreationWire {
    fn from(usage: &CacheCreation) -> Self {
        Self {
            ephemeral_5m_input_tokens: usage.ephemeral_5m_input_tokens,
            ephemeral_1h_input_tokens: usage.ephemeral_1h_input_tokens,
        }
    }
}

#[derive(Clone, Serialize)]
struct CodexTokenUsageWire {
    #[serde(skip_serializing_if = "Option::is_none")]
    input_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cached_input_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_output_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    total_tokens: Option<i64>,
}

impl From<&CodexUsage> for CodexTokenUsageWire {
    fn from(usage: &CodexUsage) -> Self {
        Self {
            input_tokens: usage.input_tokens,
            cached_input_tokens: usage.cached_input_tokens,
            output_tokens: usage.output_tokens,
            reasoning_output_tokens: usage.reasoning_output_tokens,
            total_tokens: usage.total_tokens,
        }
    }
}

#[derive(Clone, Serialize)]
struct ToolCallsWire {
    total: i64,
    #[serde(rename = "byName")]
    by_name: BTreeMap<String, i64>,
}

impl From<&ToolCalls> for ToolCallsWire {
    fn from(tool_calls: &ToolCalls) -> Self {
        Self {
            total: tool_calls.total,
            by_name: tool_calls.by_name.clone(),
        }
    }
}

#[cfg(test)]
#[path = "harness_usage_wire_tests.rs"]
mod tests;
