use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::Result;
use chrono::{DateTime, Utc};
use instant::Instant;
use parking_lot::Mutex;
use warp_harness_usage::{CaptureDiagnostics, ExtractionOutcome, JsonlReadStatus};
use warpui::duration_with_jitter;
use warpui::r#async::{FutureExt as _, Timer};

use crate::ai::agent::api::ServerConversationToken;
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::server::retry_strategies::is_transient_http_error;
use crate::server::server_api::ServerApi;
use crate::server::server_api::harness_support::{
    HARNESS_USAGE_METRICS_VERSION, HarnessSupportClient, HarnessUsageContext, HarnessUsageError,
    HarnessUsageErrorKind, HarnessUsagePublication, HarnessUsageReport, UsageHarness,
    upload_to_target,
};

const MAX_ATTEMPTS: usize = 3;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

struct ReportingContext {
    task_id: AmbientAgentTaskId,
    execution_id: i64,
    client: Arc<ServerApi>,
}

#[derive(Clone, Copy)]
pub(super) struct CaptureIdentity {
    execution_id: i64,
    sequence: i64,
}

impl CaptureIdentity {
    pub(super) fn report(
        self,
        harness: UsageHarness,
        captured_at: DateTime<Utc>,
        outcome: ExtractionOutcome,
    ) -> Option<HarnessUsageReport> {
        match outcome {
            ExtractionOutcome::Usable(snapshot) => Some(HarnessUsageReport {
                metrics_version: HARNESS_USAGE_METRICS_VERSION,
                harness,
                execution_id: self.execution_id,
                capture_sequence: self.sequence,
                captured_at,
                snapshot: *snapshot,
            }),
            ExtractionOutcome::Unavailable(reasons) => {
                log::debug!("Harness usage unavailable: reasons={reasons:?}");
                None
            }
        }
    }
}

/// Metrics state is owned by the same runner and ordered operation as transcript persistence.
#[derive(Default)]
pub(crate) struct UsageReporter {
    context: OnceLock<Option<ReportingContext>>,
    sequence: Mutex<i64>,
    pending: Mutex<Option<HarnessUsageReport>>,
    disabled: AtomicBool,
}

impl UsageReporter {
    pub(crate) fn initialize(
        &self,
        client: Arc<ServerApi>,
        task_id: Option<AmbientAgentTaskId>,
        context: Option<HarnessUsageContext>,
    ) {
        let context = task_id.zip(context).and_then(|(task_id, context)| {
            (context.metrics_version == HARNESS_USAGE_METRICS_VERSION && context.execution_id > 0)
                .then_some(ReportingContext {
                    task_id,
                    execution_id: context.execution_id,
                    client,
                })
        });
        if context.is_none() {
            log::debug!("Harness usage disabled: no supported execution-bound startup context");
        }
        let _ = self.context.set(context);
    }

    pub(super) fn is_enabled(&self) -> bool {
        self.context.get().is_some_and(Option::is_some) && !self.disabled.load(Ordering::Relaxed)
    }

    pub(super) fn begin_capture(&self) -> Option<CaptureIdentity> {
        *self.pending.lock() = None;
        if !self.is_enabled() {
            return None;
        }
        let context = self.context.get()?.as_ref()?;
        let mut sequence = self.sequence.lock();
        let Some(next) = sequence.checked_add(1) else {
            self.disabled.store(true, Ordering::Relaxed);
            log::warn!("Harness usage disabled: capture sequence exhausted");
            return None;
        };
        *sequence = next;
        Some(CaptureIdentity {
            execution_id: context.execution_id,
            sequence: next,
        })
    }

    pub(super) fn stage_uploaded(&self, report: Option<HarnessUsageReport>) {
        *self.pending.lock() = report;
    }

    pub(super) async fn publish(&self) {
        let report = self.pending.lock().take();
        let Some(report) = report.filter(|_| self.is_enabled()) else {
            return;
        };
        let Some(context) = self.context.get().and_then(Option::as_ref) else {
            return;
        };
        let started = Instant::now();
        let result = publish_with_retry(
            &report,
            |report| async move {
                context
                    .client
                    .report_harness_usage_for_task(&context.task_id, report)
                    .with_timeout(REQUEST_TIMEOUT)
                    .await
                    .unwrap_or_else(|_| Err(HarnessUsageError::new(HarnessUsageErrorKind::Retryable)))
            },
            wait_before_retry,
        )
        .await;
        match result {
            Ok(publication) => log::debug!(
                "Harness usage published: status={:?} elapsed_ms={}",
                publication.status,
                started.elapsed().as_millis()
            ),
            Err(error) => {
                if matches!(
                    error.kind,
                    HarnessUsageErrorKind::Disabled
                        | HarnessUsageErrorKind::Unauthorized
                        | HarnessUsageErrorKind::Conflict
                        | HarnessUsageErrorKind::InvalidResponse
                ) {
                    self.disabled.store(true, Ordering::Relaxed);
                }
                log::warn!("Harness usage not published: {error}");
            }
        }
    }
}

pub(super) struct CapturedTranscript {
    pub body: Vec<u8>,
    pub report: Option<HarnessUsageReport>,
    pub needs_retry: bool,
}

pub(super) fn needs_capture_retry(diagnostics: &CaptureDiagnostics) -> bool {
    std::iter::once(&diagnostics.root)
        .chain(diagnostics.subagents.values())
        .any(|file| file.status != JsonlReadStatus::Readable || file.incomplete_trailing_record)
        || diagnostics.subagent_discovery_incomplete
}

pub(super) async fn capture_with_retry<F, Fut>(
    enabled: bool,
    mut capture: F,
) -> Result<CapturedTranscript>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<CapturedTranscript>>,
{
    let attempts = if enabled { MAX_ATTEMPTS } else { 1 };
    let mut latest = None;
    for attempt in 1..=attempts {
        let started = Instant::now();
        let capture = capture().await;
        log::debug!(
            "Harness capture and extraction: attempt={attempt} elapsed_ms={}",
            started.elapsed().as_millis()
        );
        match capture {
            Ok(capture) if !capture.needs_retry || attempt == attempts => return Ok(capture),
            Ok(capture) => latest = Some(capture),
            Err(error) if attempt == attempts => return latest.ok_or(error),
            Err(_) => {}
        }
        wait_before_retry(backoff(attempt)).await;
    }
    unreachable!()
}

pub(super) async fn upload_capture(
    client: &dyn HarnessSupportClient,
    conversation_id: &ServerConversationToken,
    reporter: &UsageReporter,
    capture: CapturedTranscript,
) -> Result<()> {
    let attempts = if reporter.is_enabled() { MAX_ATTEMPTS } else { 1 };
    for attempt in 1..=attempts {
        let result = async {
            let target = client.get_transcript_upload_target(conversation_id).await?;
            upload_to_target(client.http_client(), &target, capture.body.clone()).await
        }
        .await;
        match result {
            Ok(()) => {
                log::debug!("Harness transcript captured: bytes={}", capture.body.len());
                reporter.stage_uploaded(capture.report);
                return Ok(());
            }
            Err(error) if attempt < attempts && is_transient_http_error(&error) => {
                wait_before_retry(backoff(attempt)).await;
            }
            Err(error) => return Err(error.context("Harness transcript upload failed")),
        }
    }
    unreachable!()
}

async fn publish_with_retry<'a, F, Fut, S, Sleep>(
    report: &'a HarnessUsageReport,
    mut send: F,
    mut sleep: S,
) -> Result<HarnessUsagePublication, HarnessUsageError>
where
    F: FnMut(&'a HarnessUsageReport) -> Fut,
    Fut: Future<Output = Result<HarnessUsagePublication, HarnessUsageError>>,
    S: FnMut(Duration) -> Sleep,
    Sleep: Future<Output = ()>,
{
    for attempt in 1..=MAX_ATTEMPTS {
        match send(report).await {
            Err(error) if error.kind == HarnessUsageErrorKind::Retryable && attempt < MAX_ATTEMPTS => {
                let delay = error.retry_after.unwrap_or_default().max(backoff(attempt));
                // Retry-After cannot extend idle lifetime indefinitely.
                if delay > REQUEST_TIMEOUT {
                    return Err(error);
                }
                sleep(delay).await;
            }
            result => return result,
        }
    }
    unreachable!()
}

pub(super) fn backoff(attempt: usize) -> Duration {
    duration_with_jitter(Duration::from_secs(if attempt == 1 { 1 } else { 2 }), 0.2)
}

pub(super) async fn wait_before_retry(delay: Duration) {
    Timer::after(delay).await;
}

#[cfg(test)]
#[path = "usage_reporting_tests.rs"]
mod tests;
