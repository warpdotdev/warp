use std::path::Path;

use anyhow::{Context, Result};
use chrono::Utc;
use uuid::Uuid;
use warp_harness_usage::extract_claude;

use super::claude_transcript::read_envelope_with_diagnostics;
use super::transcript_persistence::{CapturedTranscript, needs_capture_retry};
use super::usage_reporting::CaptureIdentity;

pub(super) fn capture_transcript_with_usage(
    session_id: Uuid,
    harness_working_dir: &Path,
    config_dir: &Path,
    claude_version: Option<String>,
    require_main_transcript: bool,
    identity: Option<CaptureIdentity>,
) -> Result<CapturedTranscript> {
    let captured_at = Utc::now();
    let (mut envelope, diagnostics) = read_envelope_with_diagnostics(
        session_id,
        harness_working_dir,
        config_dir,
        require_main_transcript || identity.is_some(),
    )?;
    envelope.claude_version = claude_version;
    let transcript_body =
        serde_json::to_vec(&envelope).context("Failed to serialize transcript envelope")?;
    let usage_request = identity.and_then(|identity| {
        identity.build_usage_request(
            captured_at,
            extract_claude(
                &session_id.to_string(),
                &envelope.entries,
                envelope
                    .subagents
                    .iter()
                    .map(|(id, entries)| (id.as_str(), entries.as_slice())),
                &diagnostics,
            ),
        )
    });
    Ok(CapturedTranscript {
        transcript_body,
        usage_request,
        needs_retry: needs_capture_retry(&diagnostics),
    })
}
