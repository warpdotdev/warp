//! Native, cumulative usage observations, independent of transport and billing.

mod capture;
mod claude;
mod codex;
mod counters;
mod tools;

use std::collections::BTreeMap;

pub use capture::{
    CaptureDiagnostics, JsonlCapture, JsonlDiagnostics, JsonlReadStatus, parse_jsonl,
};
pub use claude::{CacheCreation, ClaudeUsage, extract_claude};
pub use codex::{CodexUsage, extract_codex};
use serde::Serialize;

pub const PARSER_VERSION: i32 = 2;
pub const MAX_SCOPE_LENGTH: usize = 256;
pub const MAX_SCOPE_ENTRIES: usize = 64;
const MAX_IDENTITIES: usize = 100_000;
const MAX_ATTRIBUTIONS: usize = 64;
const MAX_TOOL_NAMES: usize = 256;

pub type ReasonCounts = BTreeMap<ReasonCode, u32>;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasonCode {
    IncompleteInput,
    InvalidData,
    AmbiguousAccounting,
    ResourceLimit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverageStatus {
    Known,
    Partial,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Coverage {
    pub token_status: CoverageStatus,
    pub tool_status: CoverageStatus,
    pub captured_scope: &'static str,
    pub reason_codes: ReasonCounts,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct UsageSnapshot {
    pub payload: NativePayload,
    pub coverage: Coverage,
    pub session_ids: Vec<String>,
    pub root_scope: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub subagent_scope: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ExtractionOutcome {
    Usable(Box<UsageSnapshot>),
    Unavailable(ReasonCounts),
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum NativePayload {
    Claude(UsagePayload<ClaudeUsage>),
    Codex(UsagePayload<CodexUsage>),
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct UsagePayload<T> {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<T>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub attribution: Vec<AttributedUsage<T>>,
    #[serde(rename = "toolCalls", skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<ToolCalls>,
}

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

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AttributedUsage<T> {
    #[serde(flatten)]
    pub attribution: Attribution,
    pub usage: T,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ToolCalls {
    pub total: i64,
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
