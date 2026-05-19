//! Sidecar records emitted when a `SimpleLogger` rotates its active log file.
//!
//! Two record kinds, two layers of opt-in:
//!
//! - **Layer A — rotation events** (`[`RotationEvent`]) are written to a
//!   `<path>.rotations.jsonl` sidecar every time a rotation occurs, regardless
//!   of whether summarization is configured. No model dependency. This gives
//!   users an always-available timeline of *when* rotations happened and *what*
//!   was discarded, even if the raw bytes are gone.
//!
//! - **Layer B — summaries** (`[`RotationSummary`]) are written to a
//!   `<path>.summaries.jsonl` sidecar when a [`RotationSummarizer`] is
//!   configured and returns a non-empty record. The summarizer is invoked on
//!   the file that's about to be discarded, so its findings outlive the raw
//!   log content.
//!
//! Both sidecars use newline-delimited JSON so they can be tailed, grep'd,
//! and consumed by downstream tools without a parser.
//!
//! The summarizer interface is intentionally async and trait-shaped so a real
//! implementation can call a local model (Ollama, LM Studio) or a remote BYOK
//! endpoint without touching this crate. The `[`MockSummarizer`] shipped
//! alongside is a deterministic reference impl for tests and demos.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A timestamped record of a single rotation event, written to the
/// `<path>.rotations.jsonl` sidecar.
///
/// Emitted unconditionally when rotation fires (no model dep). The record
/// captures *when* the rotation happened, *which* file was being rotated, and
/// what was discarded — enough to reconstruct a coarse retention timeline even
/// after the bytes themselves are gone.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RotationEvent {
    /// Wall-clock time when the rotation completed. UTC for portability.
    pub timestamp: DateTime<Utc>,
    /// Absolute path of the active log file that was rotated.
    pub active_log: PathBuf,
    /// Number of bytes in the active file at the moment of rotation (i.e. the
    /// content that just got promoted to `.1`).
    pub bytes_rotated: u64,
    /// Absolute path of the file that was discarded as part of this rotation,
    /// if any. `None` for the first `max_rotation` rotations, before the cap
    /// is hit and any file actually ages out.
    pub discarded_path: Option<PathBuf>,
}

impl RotationEvent {
    /// Render this event as a single line of newline-delimited JSON, ready to
    /// be appended to the `.rotations.jsonl` sidecar.
    pub fn to_jsonl_line(&self) -> serde_json::Result<String> {
        let mut s = serde_json::to_string(self)?;
        s.push('\n');
        Ok(s)
    }
}

/// A summary record produced by a [`RotationSummarizer`] for a file that was
/// about to be discarded by rotation. Written to the `<path>.summaries.jsonl`
/// sidecar so the user retains a long-term view of what was happening even
/// after the raw bytes are gone.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RotationSummary {
    /// Wall-clock time the summary was produced. UTC.
    pub timestamp: DateTime<Utc>,
    /// Absolute path of the file that was summarized.
    pub source_path: PathBuf,
    /// Total bytes the summarizer was given as input.
    pub bytes_summarized: u64,
    /// Identifier of the model (or pipeline) that produced the summary.
    /// Format is implementation-defined; suggested shape:
    /// `"<model-name>:<size>@<runtime>"` (e.g. `"qwen2.5-coder:7b@ollama-local"`).
    pub model: String,
    /// Per-step traces. A single-call summarizer emits one entry; a multi-step
    /// pipeline (extract → classify → summarize) emits one per step.
    pub pipeline: Vec<PipelineStep>,
    /// Human-readable summary of the rotated content.
    pub summary: String,
    /// Structured findings extracted from the log. Free-form strings; callers
    /// can adopt their own convention (one per anomaly, one per error class,
    /// etc.). Empty if the summarizer found nothing worth surfacing.
    pub findings: Vec<String>,
}

impl RotationSummary {
    /// Render as newline-delimited JSON.
    pub fn to_jsonl_line(&self) -> serde_json::Result<String> {
        let mut s = serde_json::to_string(self)?;
        s.push('\n');
        Ok(s)
    }
}

/// A single step in a summarization pipeline. Useful for surfacing where time
/// went when the summarizer chains multiple model calls (extract → classify →
/// summarize), so latency regressions can be triaged per step rather than per
/// rotation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PipelineStep {
    /// Human-readable step name. Implementations choose the vocabulary;
    /// suggested values: `"extract_events"`, `"classify"`, `"summarize"`.
    pub step: String,
    /// Wall-clock duration of the step. Milliseconds for readability in JSON.
    pub duration_ms: u64,
}

/// Errors a [`RotationSummarizer`] can surface. The framework treats every
/// variant as recoverable: the rotation itself never fails because of a
/// summarizer error — the summary is simply skipped and the rotation event
/// log still records that a rotation occurred.
#[derive(Debug, Error)]
pub enum SummarizerError {
    #[error("summarizer model unavailable: {0}")]
    ModelUnavailable(String),
    #[error("summarizer call timed out")]
    Timeout,
    #[error("summarizer error: {0}")]
    Other(String),
}

/// Async trait for producing a [`RotationSummary`] from a soon-to-be-discarded
/// log file's content.
///
/// Implementations decide whether to call a model (local Ollama, remote BYOK,
/// hosted API), chain multiple calls, or skip work for some inputs. Returning
/// `Ok(None)` is a valid first-class response — useful for "the input was too
/// small to be worth summarizing" or "the model decided there was nothing of
/// note here." The framework writes summary records only when `Ok(Some(_))` is
/// returned.
///
/// The trait is named `RotationSummarizer` rather than `LogSummarizer` to
/// emphasize that it's invoked specifically at rotation boundaries, not on
/// every log line.
#[async_trait]
pub trait RotationSummarizer: Send + Sync {
    /// Generate a summary for the rotated log file content.
    ///
    /// `source_path` is the absolute path of the file being summarized (the
    /// `.N` rotated file about to be deleted, *not* the active log).
    /// `content` is its full contents as a UTF-8 string; binary or malformed
    /// content is the caller's responsibility to filter.
    async fn summarize(
        &self,
        source_path: &Path,
        content: &str,
    ) -> Result<Option<RotationSummary>, SummarizerError>;
}

/// Deterministic mock summarizer for tests and demos. Never makes a network
/// call; produces a fixed three-step pipeline trace and a summary scaled to
/// the input size.
///
/// This is the reference impl shipped with the crate so the trait wiring can
/// be exercised end-to-end without depending on an external model.
pub struct MockSummarizer {
    /// Value placed in `RotationSummary.model`. Override per-test to assert
    /// the model identifier propagated correctly.
    pub model_name: String,
}

impl Default for MockSummarizer {
    fn default() -> Self {
        Self {
            model_name: "mock-summarizer-v0".to_string(),
        }
    }
}

#[async_trait]
impl RotationSummarizer for MockSummarizer {
    async fn summarize(
        &self,
        source_path: &Path,
        content: &str,
    ) -> Result<Option<RotationSummary>, SummarizerError> {
        let bytes = content.len() as u64;
        // Refuse trivially-small inputs so the test fixtures can also exercise
        // the "no summary produced" code path.
        if bytes < 16 {
            return Ok(None);
        }

        let line_count = content.lines().count();
        let warn_count = content
            .lines()
            .filter(|l| l.contains("WARN") || l.contains("warning"))
            .count();
        let error_count = content
            .lines()
            .filter(|l| l.contains("ERROR") || l.contains("error"))
            .count();

        let summary = format!(
            "Captured {} lines ({} bytes). Detected {} warning(s) and {} error(s).",
            line_count, bytes, warn_count, error_count,
        );

        let mut findings = Vec::new();
        if error_count > 0 {
            findings.push(format!(
                "{} ERROR-level line(s) in this window",
                error_count
            ));
        }
        if warn_count > 0 {
            findings.push(format!("{} WARN-level line(s) in this window", warn_count));
        }
        if line_count >= 1000 {
            findings.push(format!(
                "High log volume: {} lines (>= 1000) — possible chatty subsystem",
                line_count,
            ));
        }

        Ok(Some(RotationSummary {
            timestamp: Utc::now(),
            source_path: source_path.to_path_buf(),
            bytes_summarized: bytes,
            model: self.model_name.clone(),
            pipeline: vec![
                PipelineStep {
                    step: "extract_events".to_string(),
                    // Deterministic fake latency; real impls would measure.
                    duration_ms: 42,
                },
                PipelineStep {
                    step: "classify".to_string(),
                    duration_ms: 28,
                },
                PipelineStep {
                    step: "summarize".to_string(),
                    duration_ms: 73,
                },
            ],
            summary,
            findings,
        }))
    }
}

#[cfg(test)]
#[path = "rotation_events_tests.rs"]
mod tests;
