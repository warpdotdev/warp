use std::future::Future;
use std::sync::{Arc, Weak};

use anyhow::{Context, Error, Result, anyhow};
use warp_errors::report_if_error;
use warpui::ModelSpawner;
use warpui::r#async::executor::Background;

use super::save_coordinator::{SaveCoordinator, SaveOperation, final_save_budget};
use super::transcript_persistence::UploadedTranscriptUsage;
use super::usage_reporting::{CaptureIdentity, UsageReporter};
use super::{AgentDriver, HarnessRunner, SavePoint};
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::server::server_api::ServerApi;
use crate::server::server_api::harness_support::HarnessUsageCapability;

pub(crate) struct PersistenceOutcome {
    // Transcript and block persistence determine save success. Usage publication is best-effort
    // and therefore does not contribute to this result.
    result: Result<()>,
    // This is present only when the raw transcript upload succeeded, so derived usage is never
    // published without its corresponding transcript.
    uploaded_usage: Option<UploadedTranscriptUsage>,
}

impl PersistenceOutcome {
    pub(super) fn skipped() -> Self {
        // Intentional no-ops are successful without implying that either artifact was saved.
        Self {
            result: Ok(()),
            uploaded_usage: None,
        }
    }

    pub(super) fn failed(error: Error) -> Self {
        // Failures before persistence starts cannot make derived usage eligible for publication.
        Self {
            result: Err(error),
            uploaded_usage: None,
        }
    }

    pub(super) fn block_only(result: Result<()>) -> Self {
        // Harnesses without transcript persistence can still report their block-save result.
        Self {
            result,
            uploaded_usage: None,
        }
    }

    #[cfg(test)]
    pub(super) fn into_result(self) -> Result<()> {
        self.result
    }
}

#[derive(Default)]
pub(crate) struct HarnessPersistence {
    // Coalesces frequent save requests and ensures transcript/block saves run serially.
    saves: SaveCoordinator,
    // Publishes the newest eligible usage snapshot independently of durable artifact saves.
    usage: UsageReporter,
}

impl HarnessPersistence {
    pub(crate) fn initialize(
        &self,
        client: Arc<ServerApi>,
        task_id: Option<AmbientAgentTaskId>,
        capability: Option<HarnessUsageCapability>,
    ) {
        // Usage reporting is enabled only when startup supplied both a task and an
        // execution-bound server capability. Transcript and block persistence remain enabled.
        self.usage.initialize(client, task_id, capability);
    }

    pub(super) fn is_reporting_enabled(&self) -> bool {
        self.usage.is_enabled()
    }

    pub(super) fn begin_capture(&self) -> Option<CaptureIdentity> {
        // Reserve the execution and monotonic sequence identity before extraction. None tells the
        // caller to persist the transcript without producing a usage request.
        self.usage.begin_capture()
    }

    pub(super) fn enqueue<R>(
        &self,
        runner: Weak<R>,
        save_point: SavePoint,
        foreground: ModelSpawner<AgentDriver>,
        background: Arc<Background>,
    ) where
        R: HarnessRunner + ?Sized,
    {
        let publisher = background.clone();
        // The coordinator retains this runner-scoped operation and invokes it with the save point
        // that survives coalescing, which may differ from the request passed to this call.
        let operation: SaveOperation = Arc::new(move |save_point| {
            let runner = runner.clone();
            let foreground = foreground.clone();
            let publisher = publisher.clone();
            Box::pin(async move {
                // Background persistence must not keep a runner alive after driver cleanup.
                let Some(runner) = runner.upgrade() else {
                    return Ok(());
                };
                if matches!(save_point, SavePoint::PostTurn) {
                    // Hook-driven updates may expose new harness-specific session state needed by
                    // the capture, such as Codex's native session ID.
                    report_if_error!(
                        runner
                            .handle_session_update(&foreground)
                            .await
                            .context("Failed to handle harness session update before save")
                    );
                }
                let outcome = runner.save_conversation(save_point, &foreground).await;
                // Complete stages any usage made eligible by a successful transcript upload before
                // propagating an independent block-save failure.
                runner.persistence().complete(outcome, &publisher)
            })
        });
        // SaveCoordinator installs only the first operation; subsequent calls enqueue work against
        // the same runner-scoped closure.
        self.saves.set_worker_operation(operation);
        self.saves.enqueue(save_point, &background);
    }

    pub(super) async fn finalize<R>(
        &self,
        runner: &R,
        foreground: &ModelSpawner<AgentDriver>,
        background: &Background,
    ) -> Result<()>
    where
        R: HarnessRunner + ?Sized,
    {
        let budget = final_save_budget();
        // One deadline covers the entire shutdown sequence: drain or cancel an ordinary save, run
        // the final capture/upload, then give any remaining time to usage publication.
        self.saves
            .finalize(
                async {
                    // Refresh hook-derived state once more so the final capture includes the latest
                    // session metadata even when no PostTurn save observed it.
                    report_if_error!(
                        runner
                            .handle_session_update(foreground)
                            .await
                            .context("Failed to handle harness session update before final save")
                    );
                    let outcome = runner.save_conversation(SavePoint::Final, foreground).await;
                    self.complete(outcome, background)
                },
                self.usage.close_and_drain(budget),
                budget,
            )
            .await
    }

    fn complete(&self, outcome: PersistenceOutcome, background: &Background) -> Result<()> {
        // UploadedTranscriptUsage can only be constructed after the raw transcript upload
        // succeeds. Stage it even if the parallel block snapshot failed.
        if let Some(uploaded_usage) = outcome.uploaded_usage {
            self.usage.stage_uploaded(uploaded_usage, background);
        }
        // Publication continues on its own worker and never changes durable save success.
        outcome.result
    }
}

pub(super) async fn save_transcript_and_block(
    transcript: impl Future<Output = Result<UploadedTranscriptUsage>>,
    block: impl Future<Output = Result<()>>,
) -> PersistenceOutcome {
    // The transcript and terminal block are independent artifacts, so neither upload should delay
    // starting the other.
    let (transcript, block) = futures::join!(transcript, block);
    match (transcript, block) {
        (Ok(uploaded_usage), Ok(())) => PersistenceOutcome {
            result: Ok(()),
            uploaded_usage: Some(uploaded_usage),
        },
        // Preserve eligible usage from the successfully uploaded transcript while still surfacing
        // the block failure to the save coordinator.
        (Ok(uploaded_usage), Err(error)) => PersistenceOutcome {
            result: Err(error.context("Harness block snapshot save failed")),
            uploaded_usage: Some(uploaded_usage),
        },
        // A failed transcript upload makes its derived usage ineligible, regardless of whether the
        // independent block snapshot succeeded.
        (Err(error), Ok(())) => PersistenceOutcome {
            result: Err(error.context("Harness transcript save failed")),
            uploaded_usage: None,
        },
        // Concurrent uploads do not short-circuit, so retain both causes when neither artifact was
        // persisted.
        (Err(transcript), Err(block)) => PersistenceOutcome {
            result: Err(anyhow!(
                "Harness transcript and block snapshot saves failed: \
                 transcript={transcript:#}; block={block:#}"
            )),
            uploaded_usage: None,
        },
    }
}

#[cfg(test)]
#[path = "harness_persistence_tests.rs"]
mod tests;
