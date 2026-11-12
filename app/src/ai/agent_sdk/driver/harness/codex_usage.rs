use std::path::Path;

use anyhow::{Context, Result, bail};
use chrono::Utc;
use uuid::Uuid;
use warp_harness_usage::{CaptureDiagnostics, JsonlReadStatus, extract_codex};

use super::codex_transcript::{CodexTranscriptEnvelope, parse_session_meta};
use super::transcript_persistence::{CapturedTranscript, needs_capture_retry, read_jsonl_capture};
use super::usage_reporting::CaptureIdentity;

pub(super) fn capture_transcript_with_usage(
    session_id: Uuid,
    transcript_path: &Path,
    is_final: bool,
    identity: Option<CaptureIdentity>,
) -> Result<CapturedTranscript> {
    let captured_at = Utc::now();
    let capture = read_jsonl_capture(transcript_path)?;
    if (is_final || identity.is_some()) && capture.diagnostics.status == JsonlReadStatus::Missing {
        bail!("Codex transcript disappeared before final save");
    }
    if capture.diagnostics.status == JsonlReadStatus::Unreadable && capture.entries.is_empty() {
        bail!("Codex transcript could not be read");
    }
    if identity.is_some() && capture.entries.is_empty() && !capture.diagnostics.is_complete() {
        bail!("Codex transcript has no complete readable records");
    }
    let metadata = parse_session_meta(capture.entries.first()).unwrap_or_default();
    let envelope = CodexTranscriptEnvelope::new(session_id, metadata, capture.entries);
    let transcript_body =
        serde_json::to_vec(&envelope).context("Failed to serialize codex transcript")?;
    let diagnostics = CaptureDiagnostics {
        root: capture.diagnostics,
        ..Default::default()
    };
    let usage_request = identity.and_then(|identity| {
        identity.build_usage_request(
            captured_at,
            extract_codex(&session_id.to_string(), &envelope.entries, &diagnostics),
        )
    });
    Ok(CapturedTranscript {
        transcript_body,
        usage_request,
        needs_retry: needs_capture_retry(&diagnostics),
    })
}
