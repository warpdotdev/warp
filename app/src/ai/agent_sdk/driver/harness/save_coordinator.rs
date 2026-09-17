use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result, anyhow};
use futures::FutureExt as _;
use futures::channel::oneshot;
use futures::future::{AbortHandle, Abortable, Shared};
use instant::Instant;
use parking_lot::Mutex;
use warp_errors::report_if_error;
use warpui::r#async::executor::Background;
use warpui::r#async::{BoxFuture, FutureExt as _};

use super::SavePoint;

const FINAL_SAVE_TIMEOUT: Duration = Duration::from_secs(30);

/// An active runner worker's operation, which may service initial and coalesced save points.
///
/// It must reread runner state rather than capture state from an individual request.
type SaveOperation = Arc<dyn Fn(SavePoint) -> BoxFuture<'static, Result<()>> + Send + Sync>;

#[derive(Default)]
struct SaveState {
    pending: Option<SavePoint>,
    active: Option<ActiveSave>,
    closing: bool,
    final_deadline: Option<Instant>,
    final_succeeded: Option<bool>,
}

#[derive(Clone)]
struct ActiveSave {
    abort: AbortHandle,
    done: Shared<oneshot::Receiver<()>>,
}

impl Drop for ActiveSave {
    fn drop(&mut self) {
        self.abort.abort();
    }
}

/// Runs one save at a time, retaining at most one pending request with `PostTurn` precedence.
///
/// Closing rejects new requests and retains the final deadline and outcome across calls.
#[derive(Default)]
pub(crate) struct SaveCoordinator {
    state: Arc<Mutex<SaveState>>,
}

impl SaveCoordinator {
    /// Starts a runner-scoped worker or coalesces this save point into its pending work.
    // TODO(vkodithala): Separate request coalescing from worker startup so this only accepts a
    // SavePoint.
    pub(super) fn request(
        &self,
        save_point: SavePoint,
        worker_operation: SaveOperation,
        background: &Background,
    ) {
        let mut state = self.state.lock();
        if state.closing {
            return;
        }
        state.pending = Some(match (state.pending, save_point) {
            (Some(SavePoint::PostTurn), _) | (_, SavePoint::PostTurn) => SavePoint::PostTurn,
            (Some(SavePoint::Final), _) | (_, SavePoint::Final) => SavePoint::Final,
            (Some(SavePoint::Periodic) | None, SavePoint::Periodic) => SavePoint::Periodic,
        });
        if state.active.is_some() {
            return;
        }

        let shared_state = self.state.clone();
        let (abort, registration) = AbortHandle::new_pair();
        let (done, receiver) = oneshot::channel();
        state.active = Some(ActiveSave {
            abort,
            done: receiver.shared(),
        });
        background
            .spawn(async move {
                let worker = async {
                    loop {
                        let save_point = {
                            let mut state = shared_state.lock();
                            match state.pending.take().filter(|_| !state.closing) {
                                Some(save_point) => save_point,
                                None => {
                                    state.active = None;
                                    return;
                                }
                            }
                        };
                        report_if_error!(
                            worker_operation(save_point)
                                .await
                                .context("Failed to save harness conversation")
                        );
                    }
                };
                let _ = Abortable::new(worker, registration).await;
                let _ = done.send(());
            })
            .detach();
    }

    /// Stops ordinary requests, then drains or cancels current work before a bounded final save.
    pub(super) async fn finish(
        &self,
        final_save: impl Future<Output = Result<()>>,
        budget: Duration,
    ) -> Result<()> {
        let (active, deadline) = {
            let mut state = self.state.lock();
            if let Some(succeeded) = state.final_succeeded {
                return if succeeded {
                    Ok(())
                } else {
                    Err(anyhow!("Harness final save failed"))
                };
            }
            state.closing = true;
            state.pending = None;
            let deadline = *state
                .final_deadline
                .get_or_insert_with(|| Instant::now() + budget);
            (state.active.clone(), deadline)
        };
        let budget = deadline.saturating_duration_since(Instant::now());
        if let Some(mut active) = active {
            // A stalled ordinary upload must leave time for the post-exit capture.
            if (&mut active.done).with_timeout(budget / 2).await.is_err() {
                active.abort.abort();
                (&mut active.done)
                    .with_timeout(deadline.saturating_duration_since(Instant::now()))
                    .await
                    .context("Timed out cancelling the harness save")?
                    .context("Harness save worker dropped")?;
            }
        }
        self.state.lock().active = None;
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(anyhow!("Harness final save deadline expired"));
        }
        let result = final_save
            .with_timeout(remaining)
            .await
            .context("Harness final save timed out")
            .and_then(|result| result);
        self.state.lock().final_succeeded = Some(result.is_ok());
        result
    }
}

pub(super) fn final_save_budget() -> Duration {
    let deadline = std::env::var("WARP_SANDBOX_DEADLINE")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|seconds| *seconds > 0)
        .and_then(|seconds| SystemTime::UNIX_EPOCH.checked_add(Duration::from_secs(seconds)));
    remaining_final_save_budget(SystemTime::now(), deadline)
}

fn remaining_final_save_budget(now: SystemTime, deadline: Option<SystemTime>) -> Duration {
    deadline.map_or(FINAL_SAVE_TIMEOUT, |deadline| {
        deadline
            .duration_since(now)
            .unwrap_or_default()
            .min(FINAL_SAVE_TIMEOUT)
    })
}

/// Waits for both saves so either can complete independently if the other fails.
pub(super) async fn save_transcript_and_block(
    transcript: impl Future<Output = Result<()>>,
    block: impl Future<Output = Result<()>>,
) -> Result<()> {
    let (transcript, block) = futures::join!(transcript, block);
    transcript.and(block)
}

#[cfg(test)]
#[path = "save_coordinator_tests.rs"]
mod tests;
