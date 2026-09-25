use std::future::Future;
use std::io::BufReader;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use instant::Instant;
use warp_harness_usage::api::HarnessUsageRequest;
use warp_harness_usage::{
    CaptureDiagnostics, JsonlCapture, JsonlDiagnostics, JsonlLimits, JsonlReadStatus, parse_jsonl,
};
use warpui::r#async::Timer;
use warpui::duration_with_jitter;

use crate::ai::agent::api::ServerConversationToken;
use crate::server::server_api::harness_support::{HarnessSupportClient, upload_to_target};

const MAX_CAPTURE_ATTEMPTS: usize = 3;
const JSONL_LIMITS: JsonlLimits = JsonlLimits {
    max_line_bytes: 8 * 1024 * 1024,
    max_total_bytes: 64 * 1024 * 1024,
    max_records: 100_000,
};

pub(super) fn read_jsonl_capture(path: &Path) -> Result<JsonlCapture> {
    match std::fs::File::open(path) {
        Ok(file) => Ok(parse_jsonl(BufReader::new(file), JSONL_LIMITS)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(JsonlCapture {
            entries: Vec::new(),
            diagnostics: JsonlDiagnostics::default(),
        }),
        Err(error) => Err(error).context("Failed to open native transcript"),
    }
}

/// Raw transcript bytes and usage derived from the same native capture.
pub(super) struct CapturedTranscript {
    pub transcript_body: Vec<u8>,
    pub usage_request: Option<HarnessUsageRequest>,
    pub needs_retry: bool,
}

pub(super) struct UploadedTranscriptUsage(Option<HarnessUsageRequest>);

impl UploadedTranscriptUsage {
    pub(super) fn empty() -> Self {
        Self(None)
    }
    pub(super) fn into_request(self) -> Option<HarnessUsageRequest> {
        self.0
    }
}

pub(super) fn needs_capture_retry(diagnostics: &CaptureDiagnostics) -> bool {
    std::iter::once(&diagnostics.root)
        .chain(diagnostics.subagents.values())
        .any(|file| file.status != JsonlReadStatus::Readable || file.incomplete_trailing_record)
        || diagnostics.subagent_discovery_incomplete
}

pub(super) async fn capture_transcript_with_retry<F, Fut>(
    retry_incomplete: bool,
    mut capture: F,
) -> Result<Option<CapturedTranscript>>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<Option<CapturedTranscript>>>,
{
    let attempts = if retry_incomplete {
        MAX_CAPTURE_ATTEMPTS
    } else {
        1
    };
    let mut latest = None;
    for attempt in 1..=attempts {
        let started = Instant::now();
        let capture = capture().await;
        log::debug!(
            "Harness transcript capture: attempt={attempt} elapsed_ms={}",
            started.elapsed().as_millis()
        );
        match capture {
            Ok(Some(capture)) if !capture.needs_retry || attempt == attempts => {
                return Ok(Some(capture));
            }
            Ok(Some(capture)) => latest = Some(capture),
            Ok(None) if attempt == attempts => return Ok(latest),
            Ok(None) => {}
            Err(error) if attempt == attempts => {
                return latest.map(Some).map(Ok).unwrap_or(Err(error));
            }
            Err(_) => {}
        }
        Timer::after(capture_backoff(attempt)).await;
    }
    unreachable!()
}

/// Performs the legacy single raw-transcript upload attempt.
pub(super) async fn upload_captured_transcript(
    client: &dyn HarnessSupportClient,
    conversation_id: &ServerConversationToken,
    capture: CapturedTranscript,
) -> Result<UploadedTranscriptUsage> {
    let target = client.get_transcript_upload_target(conversation_id).await?;
    upload_to_target(
        client.http_client(),
        &target,
        capture.transcript_body.clone(),
    )
    .await
    .context("Harness transcript upload failed")?;
    log::debug!(
        "Harness transcript uploaded: bytes={}",
        capture.transcript_body.len()
    );
    Ok(UploadedTranscriptUsage(capture.usage_request))
}

fn capture_backoff(attempt: usize) -> Duration {
    duration_with_jitter(Duration::from_secs(if attempt == 1 { 1 } else { 2 }), 0.2)
}
