use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use futures::FutureExt as _;
use futures::channel::oneshot;
use futures::future::{AbortHandle, Abortable, Shared};
use instant::Instant;
use parking_lot::Mutex;
use warp_harness_usage::ExtractionOutcome;
use warp_harness_usage::api::HarnessUsageRequest;
use warpui::r#async::executor::Background;
use warpui::r#async::{FutureExt as _, Timer};
use warpui::duration_with_jitter;

use super::transcript_persistence::UploadedTranscriptUsage;
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::server::server_api::ServerApi;
use crate::server::server_api::harness_support::{
    HarnessUsageCapability, HarnessUsageError, HarnessUsageErrorKind, HarnessUsagePublicationStatus,
};

const MAX_PUBLICATION_ATTEMPTS: usize = 3;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone)]
struct ReportingContext {
    task_id: AmbientAgentTaskId,
    execution_id: i64,
    client: Arc<ServerApi>,
}

#[derive(Default)]
enum ReportingLifecycle {
    #[default]
    Uninitialized,
    Disabled,
    Active(ReportingContext),
}

#[derive(Clone)]
struct ActivePublisher {
    abort: AbortHandle,
    done: Shared<oneshot::Receiver<()>>,
}
struct PublicationDrain {
    state: Arc<Mutex<ReportingState>>,
    active: ActivePublisher,
}

impl Drop for PublicationDrain {
    fn drop(&mut self) {
        self.active.abort.abort();
        let mut state = self.state.lock();
        state.pending = None;
        state.active = None;
    }
}

#[derive(Default)]
struct ReportingState {
    lifecycle: ReportingLifecycle,
    sequence: i64,
    pending: Option<HarnessUsageRequest>,
    active: Option<ActivePublisher>,
    closing: bool,
}

#[derive(Clone, Copy)]
pub(super) struct CaptureIdentity {
    execution_id: i64,
    sequence: i64,
}

impl CaptureIdentity {
    pub(super) fn build_usage_request(
        self,
        captured_at: DateTime<Utc>,
        outcome: ExtractionOutcome,
    ) -> Option<HarnessUsageRequest> {
        match outcome {
            ExtractionOutcome::Usable(extracted) => {
                if !extracted.diagnostics.reasons.is_empty() {
                    log::debug!(
                        "Harness usage extracted with diagnostics: reasons={:?}",
                        extracted.diagnostics.reasons
                    );
                }
                Some(HarnessUsageRequest::new(
                    self.execution_id,
                    self.sequence,
                    captured_at,
                    extracted.snapshot,
                ))
            }
            ExtractionOutcome::Unavailable(diagnostics) => {
                log::debug!(
                    "Harness usage unavailable: reasons={:?}",
                    diagnostics.reasons
                );
                None
            }
        }
    }
}

#[derive(Default)]
pub(crate) struct UsageReporter {
    state: Arc<Mutex<ReportingState>>,
}

impl UsageReporter {
    pub(crate) fn initialize(
        &self,
        client: Arc<ServerApi>,
        task_id: Option<AmbientAgentTaskId>,
        capability: Option<HarnessUsageCapability>,
    ) {
        let lifecycle = task_id.zip(capability).map(|(task_id, capability)| {
            ReportingLifecycle::Active(ReportingContext {
                task_id,
                execution_id: capability.execution_id,
                client,
            })
        });
        if lifecycle.is_none() {
            log::debug!("Harness usage disabled: no supported execution-bound startup context");
        }
        let mut state = self.state.lock();
        if matches!(&state.lifecycle, ReportingLifecycle::Uninitialized) {
            state.lifecycle = lifecycle.unwrap_or(ReportingLifecycle::Disabled);
        }
    }

    pub(super) fn is_enabled(&self) -> bool {
        matches!(&self.state.lock().lifecycle, ReportingLifecycle::Active(_))
    }

    pub(super) fn begin_capture(&self) -> Option<CaptureIdentity> {
        let mut state = self.state.lock();
        let execution_id = match &state.lifecycle {
            ReportingLifecycle::Active(context) if !state.closing => context.execution_id,
            ReportingLifecycle::Uninitialized
            | ReportingLifecycle::Disabled
            | ReportingLifecycle::Active(_) => return None,
        };
        let Some(next) = state.sequence.checked_add(1) else {
            state.lifecycle = ReportingLifecycle::Disabled;
            state.pending = None;
            log::warn!("Harness usage disabled: capture sequence exhausted");
            return None;
        };
        state.sequence = next;
        Some(CaptureIdentity {
            execution_id,
            sequence: next,
        })
    }

    pub(super) fn stage_uploaded(
        &self,
        uploaded: UploadedTranscriptUsage,
        background: &Background,
    ) {
        let Some(request) = uploaded.into_request() else {
            return;
        };
        self.stage_request(request, background);
    }

    fn stage_request(&self, request: HarnessUsageRequest, background: &Background) {
        let mut state = self.state.lock();
        if state.closing || !matches!(&state.lifecycle, ReportingLifecycle::Active(_)) {
            return;
        }
        state.pending = Some(request);
        if state.active.is_some() {
            return;
        }

        let shared_state = self.state.clone();
        let (abort, registration) = AbortHandle::new_pair();
        let (done, receiver) = oneshot::channel();
        state.active = Some(ActivePublisher {
            abort,
            done: receiver.shared(),
        });
        background
            .spawn(async move {
                let _ = Abortable::new(publish_pending(shared_state.clone()), registration).await;
                shared_state.lock().active = None;
                let _ = done.send(());
            })
            .detach();
    }

    pub(super) async fn close_and_drain(&self, budget: Duration) {
        let active = {
            let mut state = self.state.lock();
            state.closing = true;
            state.active.clone()
        };
        let Some(active) = active else {
            self.state.lock().pending = None;
            return;
        };
        let mut drain = PublicationDrain {
            state: self.state.clone(),
            active,
        };
        if (&mut drain.active.done).with_timeout(budget).await.is_err() {
            drain.active.abort.abort();
            let _ = (&mut drain.active.done).await;
        }
    }
}

async fn publish_pending(state: Arc<Mutex<ReportingState>>) {
    loop {
        let (request, context) = {
            let mut state = state.lock();
            let context = match &state.lifecycle {
                ReportingLifecycle::Active(context) => context.clone(),
                ReportingLifecycle::Uninitialized | ReportingLifecycle::Disabled => {
                    state.pending = None;
                    return;
                }
            };
            let Some(request) = state.pending.take() else {
                return;
            };
            (request, context)
        };
        let started = Instant::now();
        let result = publish_with_retry(
            &request,
            |request| {
                let context = context.clone();
                async move {
                    context
                        .client
                        .publish_harness_usage_for_task(&context.task_id, request)
                        .with_timeout(REQUEST_TIMEOUT)
                        .await
                        .unwrap_or_else(|_| {
                            Err(HarnessUsageError::new(HarnessUsageErrorKind::Retryable))
                        })
                }
            },
            wait_before_retry,
        )
        .await;
        match result {
            Ok(status) => log::debug!(
                "Harness usage published: status={status:?} elapsed_ms={}",
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
                    let mut state = state.lock();
                    state.lifecycle = ReportingLifecycle::Disabled;
                    state.pending = None;
                }
                log::warn!("Harness usage not published: {error}");
            }
        }
    }
}

async fn publish_with_retry<'a, F, Fut, S, Sleep>(
    request: &'a HarnessUsageRequest,
    mut send: F,
    mut sleep: S,
) -> Result<HarnessUsagePublicationStatus, HarnessUsageError>
where
    F: FnMut(&'a HarnessUsageRequest) -> Fut,
    Fut: Future<Output = Result<HarnessUsagePublicationStatus, HarnessUsageError>>,
    S: FnMut(Duration) -> Sleep,
    Sleep: Future<Output = ()>,
{
    for attempt in 1..=MAX_PUBLICATION_ATTEMPTS {
        match send(request).await {
            Err(error)
                if error.kind == HarnessUsageErrorKind::Retryable
                    && attempt < MAX_PUBLICATION_ATTEMPTS =>
            {
                let delay = error
                    .retry_after
                    .unwrap_or_default()
                    .max(publication_backoff(attempt));
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

fn publication_backoff(attempt: usize) -> Duration {
    duration_with_jitter(Duration::from_secs(if attempt == 1 { 1 } else { 2 }), 0.2)
}

async fn wait_before_retry(delay: Duration) {
    Timer::after(delay).await;
}

#[cfg(test)]
#[path = "usage_reporting_tests.rs"]
mod tests;
