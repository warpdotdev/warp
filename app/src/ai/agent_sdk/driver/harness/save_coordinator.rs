use std::future::Future;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result, anyhow};
use futures::FutureExt as _;
use futures::channel::oneshot;
use futures::future::{AbortHandle, Abortable, Shared};
use instant::Instant;
use parking_lot::Mutex;
use warp_errors::report_if_error;
use warpui::r#async::executor::Background;
use warpui::r#async::{BoxFuture, FutureExt as _, Timer};

use super::SavePoint;

const SAVE_THROTTLE_INTERVAL: Duration = Duration::from_secs(30);
// Shutdown must allow a full cooldown without consuming the final upload's timeout.
const FINAL_SAVE_TIMEOUT: Duration = Duration::from_secs(60);

/// An active runner worker's operation, which may service initial and coalesced save points.
///
/// It must reread runner state rather than capture state from an individual request.
pub(super) type SaveOperation =
    Arc<dyn Fn(SavePoint) -> BoxFuture<'static, Result<()>> + Send + Sync>;

#[derive(Default)]
struct SaveState {
    pending: Option<SavePoint>,
    active: Option<ActiveSave>,
    next_save_at: Option<Instant>,
    closing: bool,
    final_deadline: Option<Instant>,
    final_succeeded: Option<bool>,
}
impl SaveState {
    fn save_delay(&self) -> Duration {
        self.next_save_at.map_or(Duration::ZERO, |next| {
            next.saturating_duration_since(Instant::now())
        })
    }
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

/// Runs saves at least 30 seconds apart, retaining at most one pending request.
///
/// Closing rejects new requests and retains the final deadline and outcome across calls.
pub(crate) struct SaveCoordinator {
    state: Arc<Mutex<SaveState>>,
    worker_operation: OnceLock<SaveOperation>,
}

impl Default for SaveCoordinator {
    fn default() -> Self {
        Self {
            state: Arc::default(),
            worker_operation: OnceLock::new(),
        }
    }
}

impl SaveCoordinator {
    pub(super) fn set_worker_operation(&self, worker_operation: SaveOperation) {
        let _ = self.worker_operation.set(worker_operation);
    }
    /// Starts a runner-scoped worker or coalesces this save point into its pending work.
    pub(super) fn enqueue(&self, save_point: SavePoint, background: &Background) {
        let Some(worker_operation) = self.worker_operation.get().cloned() else {
            log::error!("Harness save coordinator was not initialized");
            return;
        };
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
                        let delay = {
                            let mut state = shared_state.lock();
                            if state.closing || state.pending.is_none() {
                                state.active = None;
                                return;
                            }
                            state.save_delay()
                        };
                        Timer::after(delay).await;
                        let save_point = {
                            let mut state = shared_state.lock();
                            match state.pending.take().filter(|_| !state.closing) {
                                Some(save_point) => {
                                    state.next_save_at =
                                        Some(Instant::now() + SAVE_THROTTLE_INTERVAL);
                                    save_point
                                }
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
    pub(super) async fn finalize(
        &self,
        final_save: impl Future<Output = Result<()>>,
        report_usage: impl Future<Output = ()>,
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
        let delay = self.state.lock().save_delay();
        if delay >= deadline.saturating_duration_since(Instant::now()) {
            self.state.lock().final_succeeded = Some(false);
            return Err(anyhow!(
                "Harness final save deadline cannot accommodate the throttle"
            ));
        }
        Timer::after(delay).await;
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            self.state.lock().final_succeeded = Some(false);
            return Err(anyhow!("Harness final save deadline expired"));
        }
        self.state.lock().next_save_at = Some(Instant::now() + SAVE_THROTTLE_INTERVAL);
        let result = final_save
            .with_timeout(remaining)
            .await
            .context("Harness final save timed out")
            .and_then(|result| result);
        self.state.lock().final_succeeded = Some(result.is_ok());
        let remaining = deadline.saturating_duration_since(Instant::now());
        if !remaining.is_zero() {
            let _ = report_usage.with_timeout(remaining).await;
        }
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

#[cfg(test)]
#[path = "save_coordinator_tests.rs"]
mod tests;
