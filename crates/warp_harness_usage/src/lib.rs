//! Native, cumulative usage observations from captured third-party harness histories.
//!
//! The crate parses captured records, accounts for provider-specific usage semantics, and
//! produces typed snapshots with attribution, tool calls, and coverage diagnostics.

mod capture;
mod claude;
mod codex;
mod counters;
mod tools;

use std::collections::BTreeMap;

pub use capture::{
    CaptureDiagnostics, JsonlCapture, JsonlDiagnostics, JsonlLimits, JsonlReadStatus, parse_jsonl,
};
pub use claude::{CacheCreation, ClaudeUsage, extract_claude};
pub use codex::{CodexUsage, extract_codex};
use serde::{Serialize, Serializer};

/// Version of the native payload and parser semantics.
///
/// Bump this when a change affects how published payloads are interpreted, not merely when
/// implementation details change.
pub const PARSER_VERSION: i32 = 2;

/// Maximum length accepted for a session, scope, message, invocation, or classification ID.
pub const MAX_SCOPE_LENGTH: usize = 256;

/// Maximum number of sessions or captured subagent scopes included in one snapshot.
pub const MAX_SCOPE_ENTRIES: usize = 64;
const MAX_IDENTITIES: usize = 100_000;
const MAX_ATTRIBUTIONS: usize = 64;
const MAX_TOOL_NAMES: usize = 256;

/// Counts of diagnostic conditions observed while reading or accounting for a capture.
pub type ReasonCounts = BTreeMap<ReasonCode, u32>;

/// Diagnostic categories attached to incomplete, ambiguous, invalid, or bounded observations.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasonCode {
    /// The capture ended before all relevant input could be observed.
    IncompleteInput,
    /// A record or value did not match the expected data shape.
    InvalidData,
    /// Multiple valid observations could not be reconciled exactly.
    AmbiguousAccounting,
    /// A configured input or arithmetic bound was exceeded.
    ResourceLimit,
}
/// Confidence level for one extracted usage category.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverageStatus {
    /// The category was observed completely for the captured scope.
    Known,
    /// Some usable data was observed, but the capture or accounting was incomplete.
    Partial,
    /// No usable value for the category was produced.
    Unavailable,
}
/// Independent confidence information for token and tool-call extraction.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Coverage {
    /// Confidence level for token counters.
    pub token_status: CoverageStatus,
    /// Confidence level for tool-call counts.
    pub tool_status: CoverageStatus,
    /// Provider-specific description of which histories were inspected.
    pub captured_scope: &'static str,
    /// Bounded diagnostics explaining partial or unavailable coverage.
    pub reason_codes: ReasonCounts,
}
/// A usable observation and the scope and confidence information needed to interpret it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct UsageSnapshot {
    /// Native Claude or Codex usage and tool-call data.
    pub payload: NativePayload,
    /// Independent token and tool-call coverage for this snapshot.
    pub coverage: Coverage,
    /// Unique session identifiers represented in the captured records.
    pub session_ids: Vec<String>,
    /// Identifier of the root history supplied to the extractor.
    pub root_scope: String,
    /// Captured subagent scopes, when the provider supports them.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub subagent_scope: Vec<String>,
}
/// Whether extraction produced a usable snapshot or no publishable data.
#[derive(Clone, Debug, PartialEq)]
pub enum ExtractionOutcome {
    /// At least one usage category was extracted and can be interpreted with `coverage`.
    Usable(Box<UsageSnapshot>),
    /// No usable snapshot was produced; the reasons explain the failure.
    Unavailable(ReasonCounts),
}
/// Provider-specific native payload under the shared snapshot envelope.
#[derive(Clone, Debug, PartialEq)]
pub enum NativePayload {
    /// Claude response usage, attribution, and tool calls.
    Claude(UsagePayload<ClaudeUsage>),
    /// Codex rollout usage, attribution, and tool calls.
    Codex(UsagePayload<CodexUsage>),
}

impl Serialize for NativePayload {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Claude(payload) => payload.serialize(serializer),
            Self::Codex(payload) => payload.serialize(serializer),
        }
    }
}
/// Usage, attribution breakdowns, and tool calls for one provider.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct UsagePayload<T> {
    /// Cumulative total usage, with fields absent when the provider did not report them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<T>,
    /// Usage grouped by the classifications observed alongside it.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub attribution: Vec<AttributedUsage<T>>,
    /// Deduplicated tool calls, when tool observations support a count.
    #[serde(rename = "toolCalls", skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<ToolCalls>,
}
/// Classifications that can be attached to an observed usage group.
///
/// When every classification is absent, the group contains usage whose attribution could not be
/// established from the native history.
#[derive(Clone, Debug, Default, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct Attribution {
    /// Model identifier reported by the provider.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Provider-reported service tier.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<String>,
    /// Provider-reported inference geography.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inference_geo: Option<String>,
    /// Provider-reported response speed classification.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speed: Option<String>,
}
/// Usage associated with one set of observed classifications.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AttributedUsage<T> {
    /// Classifications associated with `usage`.
    #[serde(flatten)]
    pub attribution: Attribution,
    /// Usage counters for this attribution group.
    pub usage: T,
}
/// Counts for the same deduplicated invocation set, grouped by tool name.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ToolCalls {
    /// Total number of deduplicated invocations.
    pub total: i64,
    /// Per-name counts whose values sum to `total`.
    #[serde(rename = "byName")]
    pub by_name: BTreeMap<String, i64>,
}

#[derive(Default)]
struct Findings {
    reasons: ReasonCounts,
    tokens_partial: bool,
    tools_partial: bool,
    limit_exceeded: bool,
}

impl Findings {
    fn reason(&mut self, reason: ReasonCode) {
        let count = self.reasons.entry(reason).or_default();
        *count = count.saturating_add(1);
    }

    fn token(&mut self, reason: ReasonCode) {
        self.tokens_partial = true;
        self.reason(reason);
    }

    fn tool(&mut self, reason: ReasonCode) {
        self.tools_partial = true;
        self.reason(reason);
    }

    fn limit(&mut self, reason: ReasonCode) {
        self.limit_exceeded = true;
        self.reason(reason);
    }

    fn capture(&mut self, diagnostics: &CaptureDiagnostics) {
        for file in std::iter::once(&diagnostics.root).chain(diagnostics.subagents.values()) {
            match file.status {
                JsonlReadStatus::Missing | JsonlReadStatus::Unreadable => {
                    self.reason(ReasonCode::IncompleteInput)
                }
                JsonlReadStatus::Readable => {}
                JsonlReadStatus::ResourceLimited => self.limit(ReasonCode::ResourceLimit),
            }
            if file.malformed_records > 0 {
                let count = self.reasons.entry(ReasonCode::InvalidData).or_default();
                *count = count.saturating_add(file.malformed_records);
            }
            if file.incomplete_trailing_record {
                self.reason(ReasonCode::IncompleteInput);
            }
            if !file.is_complete() {
                self.tokens_partial = true;
                self.tools_partial = true;
            }
        }
        if diagnostics.subagent_discovery_incomplete {
            self.tokens_partial = true;
            self.tools_partial = true;
            self.reason(ReasonCode::IncompleteInput);
        }
    }

    fn status(usable: bool, partial: bool) -> CoverageStatus {
        match (usable, partial) {
            (false, _) => CoverageStatus::Unavailable,
            (true, true) => CoverageStatus::Partial,
            (true, false) => CoverageStatus::Known,
        }
    }
}

fn identifier(value: &str, findings: &mut Findings) -> bool {
    if value.is_empty() || value.len() > MAX_SCOPE_LENGTH {
        findings.limit(ReasonCode::ResourceLimit);
        false
    } else {
        true
    }
}
