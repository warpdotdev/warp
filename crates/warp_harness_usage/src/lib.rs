//! Native, cumulative usage observations from captured third-party harness histories.
//!
//! The crate parses captured records, accounts for provider-specific usage semantics, and
//! produces typed snapshots with attribution, tool calls, and coverage diagnostics.
pub mod api;

mod capture;
mod claude;
mod codex;
mod counters;
mod tools;

use std::collections::BTreeMap;

pub use capture::{
    CaptureDiagnostics, JsonlCapture, JsonlDiagnostics, JsonlLimits, JsonlReadStatus, parse_jsonl,
};
pub use claude::extract_claude;
pub use codex::extract_codex;

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
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
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

/// Bounded producer-local reasons observed during extraction.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExtractionDiagnostics {
    pub reasons: ReasonCounts,
}

/// A publishable snapshot and its producer-local diagnostics.
#[derive(Clone, Debug, PartialEq)]
pub struct ExtractedUsage {
    pub snapshot: api::HarnessUsageSnapshot,
    pub diagnostics: ExtractionDiagnostics,
}

/// Whether extraction produced publishable usage.
#[derive(Clone, Debug, PartialEq)]
pub enum ExtractionOutcome {
    Usable(Box<ExtractedUsage>),
    Unavailable(ExtractionDiagnostics),
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

    fn status(usable: bool, partial: bool) -> api::CoverageStatus {
        match (usable, partial) {
            (false, _) => api::CoverageStatus::Unavailable,
            (true, true) => api::CoverageStatus::Partial,
            (true, false) => api::CoverageStatus::Known,
        }
    }

    fn diagnostics(self) -> ExtractionDiagnostics {
        ExtractionDiagnostics {
            reasons: self.reasons,
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
