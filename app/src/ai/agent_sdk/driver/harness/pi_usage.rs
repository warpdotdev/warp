use std::fs;
use std::path::Path;

use anyhow::Result;
use chrono::Utc;
use uuid::Uuid;
use warp_harness_usage::{CaptureDiagnostics, extract_pi, parse_jsonl};

use super::transcript_persistence::{CapturedTranscript, JSONL_LIMITS, needs_capture_retry};
use super::usage_reporting::CaptureIdentity;

pub(super) fn capture_transcript_with_usage(
    session_id: Uuid,
    path: &Path,
    is_final: bool,
    identity: Option<CaptureIdentity>,
) -> Result<CapturedTranscript> {
    let transcript_body = fs::read(path)?;
    let Some(identity) = identity else {
        return Ok(CapturedTranscript {
            transcript_body,
            usage_request: None,
            needs_retry: false,
        });
    };
    let capture = parse_jsonl(transcript_body.as_slice(), JSONL_LIMITS);
    let diagnostics = CaptureDiagnostics {
        root: capture.diagnostics,
        ..Default::default()
    };
    // TODO(vkodithala): Support ACP session/load with retained usage history so cost estimates remain
    // accurate across executions instead of replacing prior spend with a fresh session's usage.
    // https://agentclientprotocol.com/rfds/session-resume
    let usage_request = identity.build_usage_request(
        Utc::now(),
        extract_pi(&session_id.to_string(), &capture.entries, &diagnostics),
    );
    Ok(CapturedTranscript {
        transcript_body,
        usage_request,
        needs_retry: is_final && needs_capture_retry(&diagnostics),
    })
}
