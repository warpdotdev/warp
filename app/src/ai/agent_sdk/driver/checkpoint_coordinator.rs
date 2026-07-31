//! Periodic workspace-handoff checkpoint coordinator (REMOTE-2111 Phase 3).
//!
//! Drives the five-state machine described in `docs/remote-2111-checkpoint-spec.md`
//! (warp-server): `Idle -> Due -> InFlight -> Idle` on the periodic path, with
//! `Finalizing -> Stopped` reachable from `Idle`, `Due`, or `InFlight` via
//! [`CheckpointCoordinatorHandle::finalize`]. The timer only ever moves `Idle` to
//! `Due`; all gather/upload/commit work happens through
//! `super::snapshot::run_checkpoint_from_declarations_file`, reusing the same
//! declarations file and gather/upload pipeline as the legacy end-of-run snapshot.
//!
//! Safe-boundary gating ("only touch the filesystem/network when the conversation
//! isn't mid-turn") is implemented as a bounded poll of `AgentDriver`'s own state via
//! its `ModelSpawner`, rather than a push subscription: `AgentDriver` already reads
//! exactly the state needed (`run_conversation_id`, the terminal view's action model)
//! through this same read-only, spawner-based pattern used by `run_snapshot_upload`.
//! This trades a small amount of latency (up to [`SAFE_BOUNDARY_POLL_INTERVAL`]) for
//! avoiding new push-subscription wiring through the UI model graph.

// This module has no production caller yet -- `AgentDriver` wires
// `CheckpointCoordinatorHandle::new` into its spawn/finalize lifecycle in a
// follow-up, stacked PR. Until then, `CheckpointCoordinatorHandle::new` (and
// everything it reaches) is unreachable outside `#[cfg(test)]`, which reaches the
// same code through `new_for_test`. This module-level allow is temporary and should
// be removable once that PR merges on top of this one.
#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use futures::FutureExt as _;
use futures::future::BoxFuture;
use instant::Instant;
use rand::Rng as _;
use tokio::sync::{mpsc, oneshot};
use warpui::r#async::executor::Background;
use warpui::r#async::{FutureExt as _, Timer};
use warpui::{ModelSpawner, SingletonEntity};

use super::AgentDriver;
use super::snapshot::{self, CheckpointResult};
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::server::server_api::harness_support::{CheckpointGeneration, HarnessSupportClient};

/// A safe-boundary predicate, decoupled from `ModelSpawner<AgentDriver>` so
/// [`coordinator_loop`] can be exercised in isolation by tests. Production code builds
/// this from [`is_safe_boundary`]; tests supply a directly-controllable closure.
type BoundaryCheck = Arc<dyn Fn() -> BoxFuture<'static, bool> + Send + Sync>;

/// Default cadence between the start of one checkpoint attempt and the timer firing
/// again, absent an override on `AgentDriverOptions`. Deliberately not
/// `HARNESS_SAVE_INTERVAL` (30s) -- see the spec's "5 minutes plus jitter" cadence.
pub(super) const DEFAULT_CHECKPOINT_INTERVAL: Duration = Duration::from_secs(5 * 60);
/// Upper bound on additive jitter, so agents scheduled at the same time don't all
/// checkpoint in lockstep.
const CHECKPOINT_JITTER: Duration = Duration::from_secs(30);
/// How often the `Due` state re-checks whether the conversation is at a safe boundary.
const SAFE_BOUNDARY_POLL_INTERVAL: Duration = Duration::from_secs(2);

/// A request to finalize, carrying the deadline by which the coordinator must ack so
/// shutdown can proceed.
struct FinalizeRequest {
    deadline: Instant,
    ack: oneshot::Sender<()>,
}

/// Handle used by `AgentDriver` to request finalization of the periodic checkpoint
/// coordinator. Cloneable and fire-and-forget: dropping every handle without calling
/// [`finalize`](Self::finalize) simply leaves the coordinator running periodic
/// attempts until the process exits.
#[derive(Clone)]
pub(super) struct CheckpointCoordinatorHandle {
    finalize_tx: mpsc::UnboundedSender<FinalizeRequest>,
}

impl CheckpointCoordinatorHandle {
    /// Spawn the coordinator task on `background` and return a handle to it.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        client: Arc<dyn HarnessSupportClient>,
        task_id: AmbientAgentTaskId,
        working_dir: PathBuf,
        spawner: ModelSpawner<AgentDriver>,
        interval: Duration,
        script_timeout: Duration,
        upload_timeout: Duration,
        background: Arc<Background>,
    ) -> Self {
        let boundary_check: BoundaryCheck = Arc::new(move || {
            let spawner = spawner.clone();
            Box::pin(async move { is_safe_boundary(&spawner).await })
        });
        Self::spawn_with_boundary_check(
            client,
            task_id,
            working_dir,
            boundary_check,
            interval,
            CHECKPOINT_JITTER,
            script_timeout,
            upload_timeout,
            background,
        )
    }

    /// Test-facing constructor that bypasses `ModelSpawner<AgentDriver>` (and so the full
    /// UI framework) by taking the safe-boundary predicate directly, and disables jitter
    /// (production jitter is bounded by [`CHECKPOINT_JITTER`], up to 30s, which would
    /// otherwise make tests using a short `interval` flaky/slow).
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    fn new_for_test(
        client: Arc<dyn HarnessSupportClient>,
        task_id: AmbientAgentTaskId,
        working_dir: PathBuf,
        boundary_check: BoundaryCheck,
        interval: Duration,
        script_timeout: Duration,
        upload_timeout: Duration,
        background: Arc<Background>,
    ) -> Self {
        Self::spawn_with_boundary_check(
            client,
            task_id,
            working_dir,
            boundary_check,
            interval,
            Duration::ZERO,
            script_timeout,
            upload_timeout,
            background,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn spawn_with_boundary_check(
        client: Arc<dyn HarnessSupportClient>,
        task_id: AmbientAgentTaskId,
        working_dir: PathBuf,
        boundary_check: BoundaryCheck,
        interval: Duration,
        jitter: Duration,
        script_timeout: Duration,
        upload_timeout: Duration,
        background: Arc<Background>,
    ) -> Self {
        let (finalize_tx, finalize_rx) = mpsc::unbounded_channel();
        let loop_background = background.clone();
        background
            .spawn(coordinator_loop(
                client,
                task_id,
                working_dir,
                boundary_check,
                interval,
                jitter,
                script_timeout,
                upload_timeout,
                loop_background,
                finalize_rx,
            ))
            .detach();
        Self { finalize_tx }
    }

    /// Request finalization: run at most one more checkpoint attempt if none is
    /// already in flight (skipped if `budget` doesn't exceed the gather/upload
    /// floor), or await an already-in-flight attempt instead -- never both -- then
    /// stop the coordinator. Bounded by `budget` end to end. Safe to call at most
    /// once; safe to never call.
    pub(super) async fn finalize(&self, budget: Duration) {
        let (ack_tx, ack_rx) = oneshot::channel();
        let request = FinalizeRequest {
            deadline: Instant::now() + budget,
            ack: ack_tx,
        };
        if self.finalize_tx.send(request).is_err() {
            // Coordinator task already exited; nothing to wait for.
            return;
        }
        // The coordinator always acks well within `budget` (either immediately, when
        // below the floor, or after its own internally bounded attempt). This extra
        // bound is defense-in-depth so a coordinator bug cannot wedge shutdown.
        let _ = tokio::time::timeout(budget, ack_rx).await;
    }
}

/// Add up to `jitter` of additive random delay to `interval` so many agents scheduled at once
/// don't checkpoint in lockstep. Production always passes [`CHECKPOINT_JITTER`]; tests pass
/// `Duration::ZERO` for determinism.
fn jittered_interval(interval: Duration, jitter: Duration) -> Duration {
    let jitter_ms = u64::try_from(jitter.as_millis()).unwrap_or(u64::MAX);
    let extra = if jitter_ms == 0 {
        0
    } else {
        rand::thread_rng().gen_range(0..=jitter_ms)
    };
    interval + Duration::from_millis(extra)
}

/// Run one checkpoint attempt to completion: regenerate declarations, then gather,
/// upload, and commit. Bounded by `upload_timeout`; `script_timeout` separately
/// bounds only the declarations-script sub-step (matching the legacy pipeline).
async fn run_one_attempt(
    client: Arc<dyn HarnessSupportClient>,
    task_id: AmbientAgentTaskId,
    working_dir: PathBuf,
    script_timeout: Duration,
    upload_timeout: Duration,
) -> CheckpointResult {
    snapshot::run_declarations_script(&working_dir, &task_id, script_timeout).await;
    let path = snapshot::resolve_declarations_path(Some(&task_id));
    match snapshot::run_checkpoint_from_declarations_file(&path, client)
        .with_timeout(upload_timeout)
        .await
    {
        Ok(result) => result,
        Err(_) => CheckpointResult::Failed {
            generation: None,
            reason: format!("checkpoint attempt exceeded {upload_timeout:?} upload timeout"),
        },
    }
}

/// Spawn one attempt on `background` and return a receiver that resolves with its
/// result once the attempt completes. The spawned task runs to completion
/// independently of whether anything ever reads from the receiver, so a caller that
/// stops waiting (e.g. because a shutdown budget elapsed) cannot strand the attempt
/// or cause it to be silently abandoned mid-upload -- it simply keeps running in the
/// background and, if it succeeds, still commits.
fn start_attempt(
    client: Arc<dyn HarnessSupportClient>,
    task_id: AmbientAgentTaskId,
    working_dir: PathBuf,
    script_timeout: Duration,
    upload_timeout: Duration,
    background: &Background,
) -> oneshot::Receiver<CheckpointResult> {
    let (tx, rx) = oneshot::channel();
    background
        .spawn(async move {
            let result =
                run_one_attempt(client, task_id, working_dir, script_timeout, upload_timeout).await;
            let _ = tx.send(result);
        })
        .detach();
    rx
}

/// Query `AgentDriver` (via its spawner) for whether the conversation is currently at
/// a safe boundary: either yielded via `wait_for_events`, or simply not mid-turn with
/// no pending/running actions. Returns `true` (safe) if the driver has no
/// conversation yet, if its conversation can no longer be found, or if the driver
/// itself has been dropped -- in each case there is nothing left to interrupt.
async fn is_safe_boundary(spawner: &ModelSpawner<AgentDriver>) -> bool {
    spawner
        .spawn(|driver, ctx| {
            let Some(conversation_id) = driver.run_conversation_id else {
                return true;
            };
            let Some(status) = BlocklistAIHistoryModel::as_ref(ctx)
                .conversation(&conversation_id)
                .map(|conversation| conversation.status().clone())
            else {
                return true;
            };
            if status.is_waiting_for_events() {
                return true;
            }
            if status.is_in_progress() || status.is_transient_error() {
                return false;
            }
            let terminal_view = driver
                .terminal_driver
                .as_ref(ctx)
                .terminal_view()
                .as_ref(ctx);
            !terminal_view
                .ai_action_model()
                .as_ref(ctx)
                .has_unfinished_actions_for_conversation(conversation_id)
        })
        .await
        .unwrap_or(true)
}

/// Handle a finalize request received while no attempt is currently in flight: start
/// exactly one best-effort attempt only if `budget` exceeds the gather/upload floor,
/// bound it by the remaining budget, then ack.
async fn finalize_with_new_attempt(
    request: FinalizeRequest,
    client: Arc<dyn HarnessSupportClient>,
    task_id: AmbientAgentTaskId,
    working_dir: PathBuf,
    script_timeout: Duration,
    upload_timeout: Duration,
) {
    let floor = script_timeout + upload_timeout;
    let remaining = request.deadline.saturating_duration_since(Instant::now());
    if remaining > floor {
        log::info!(
            "Starting final checkpoint attempt at shutdown (remaining budget {remaining:?})"
        );
        let attempt = run_one_attempt(client, task_id, working_dir, script_timeout, upload_timeout);
        if tokio::time::timeout(remaining, attempt).await.is_err() {
            log::warn!("Final checkpoint attempt did not complete within {remaining:?}");
        }
    } else {
        log::info!(
            "Skipping final checkpoint attempt: remaining shutdown budget {remaining:?} \
             is below the {floor:?} floor"
        );
    }
    let _ = request.ack.send(());
}

/// Handle a finalize request received while an attempt started by the periodic
/// timer is already in flight: never start a second attempt -- just await the
/// existing one, bounded by the remaining budget, then ack.
async fn finalize_with_in_flight_attempt(
    request: FinalizeRequest,
    result_rx: oneshot::Receiver<CheckpointResult>,
) {
    let remaining = request.deadline.saturating_duration_since(Instant::now());
    match tokio::time::timeout(remaining, result_rx).await {
        Ok(Ok(result)) => {
            log::info!("In-flight checkpoint attempt resolved during finalization: {result:?}");
        }
        Ok(Err(_)) => {
            log::warn!("In-flight checkpoint attempt's result channel dropped without a result");
        }
        Err(_) => {
            // The spawned attempt keeps running in the background regardless; we
            // just stop waiting for it so shutdown can proceed within budget.
            log::warn!(
                "In-flight checkpoint attempt did not resolve within the remaining \
                 {remaining:?} shutdown budget; continuing shutdown without it"
            );
        }
    }
    let _ = request.ack.send(());
}

/// The coordinator's main loop. `Idle` and `Due` are collapsed into the top of the
/// loop body: the timer is the only thing that ever moves `Idle` to `Due`, and `Due`
/// then polls the safe-boundary predicate. `InFlight` runs the attempt on a
/// background task (via [`start_attempt`]) specifically so a finalize request racing
/// in can bound how long it waits without ever stranding the attempt itself.
#[allow(clippy::too_many_arguments)]
async fn coordinator_loop(
    client: Arc<dyn HarnessSupportClient>,
    task_id: AmbientAgentTaskId,
    working_dir: PathBuf,
    boundary_check: BoundaryCheck,
    interval: Duration,
    jitter: Duration,
    script_timeout: Duration,
    upload_timeout: Duration,
    background: Arc<Background>,
    mut finalize_rx: mpsc::UnboundedReceiver<FinalizeRequest>,
) {
    loop {
        // --- Idle: wait for the next (jittered) tick or a finalize request. ---
        futures::select! {
            _ = Timer::after(jittered_interval(interval, jitter)).fuse() => {}
            request = finalize_rx.recv().fuse() => {
                let Some(request) = request else { return };
                finalize_with_new_attempt(
                    request,
                    client.clone(),
                    task_id,
                    working_dir.clone(),
                    script_timeout,
                    upload_timeout,
                )
                .await;
                return;
            }
        }

        // --- Due: poll for a safe boundary, staying responsive to finalize. Checked
        // immediately on entry (not only after the first poll interval elapses) so an
        // already-safe conversation doesn't pay needless latency.
        let mut at_boundary = boundary_check().await;
        while !at_boundary {
            futures::select! {
                _ = Timer::after(SAFE_BOUNDARY_POLL_INTERVAL).fuse() => {
                    at_boundary = boundary_check().await;
                }
                request = finalize_rx.recv().fuse() => {
                    let Some(request) = request else { return };
                    finalize_with_new_attempt(
                        request,
                        client.clone(),
                        task_id,
                        working_dir.clone(),
                        script_timeout,
                        upload_timeout,
                    )
                    .await;
                    return;
                }
            }
        }

        // --- InFlight: run exactly one attempt, never overlapping another. ---
        let mut result_rx = start_attempt(
            client.clone(),
            task_id,
            working_dir.clone(),
            script_timeout,
            upload_timeout,
            &background,
        );
        futures::select! {
            result = (&mut result_rx).fuse() => {
                match result {
                    Ok(CheckpointResult::Committed { generation }) => {
                        log::info!(
                            "Periodic checkpoint committed: generation={}",
                            generation.as_str()
                        );
                    }
                    Ok(CheckpointResult::Skipped) => {
                        log::info!("Periodic checkpoint skipped: no usable declarations");
                    }
                    Ok(CheckpointResult::Failed { generation, reason }) => {
                        log::warn!(
                            "Periodic checkpoint attempt failed (generation={:?}): {reason}",
                            generation.as_ref().map(CheckpointGeneration::as_str)
                        );
                    }
                    Err(_) => {
                        log::warn!(
                            "Periodic checkpoint attempt's result channel dropped without a result"
                        );
                    }
                }
                // Success, skip, or failure: return to Idle and wait a full interval
                // before the next attempt either way. The periodic timer itself
                // (rather than a distinct short backoff) is the retry mechanism for
                // failures too, matching the spec's "no RPO/RTO SLO" framing.
            }
            request = finalize_rx.recv().fuse() => {
                let Some(request) = request else { return };
                finalize_with_in_flight_attempt(request, result_rx).await;
                return;
            }
        }
    }
}

#[cfg(test)]
#[path = "checkpoint_coordinator_tests.rs"]
mod tests;
