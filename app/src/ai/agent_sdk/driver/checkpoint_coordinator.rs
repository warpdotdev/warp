//! Periodic workspace-handoff checkpoint coordinator.
//!
//! Drives a five-state machine: `Idle -> Due -> InFlight -> Idle` on the periodic path,
//! with `Finalizing -> Stopped` reachable from `Idle`, `Due`, or `InFlight` via
//! [`CheckpointCoordinatorHandle::finalize`]. `Idle` waits out the jittered interval,
//! `Due` waits for a safe boundary, and `InFlight` runs exactly one attempt at a time.
//! The timer only ever moves `Idle` to `Due`; all gather/upload/commit work happens
//! through `super::snapshot::run_checkpoint_from_declarations_file`, reusing the same
//! declarations file and gather/upload pipeline as the legacy end-of-run snapshot.
//!
//! Safe-boundary gating ("only touch the filesystem/network when the conversation
//! isn't mid-turn") is implemented as a bounded poll of `AgentDriver`'s own state via
//! its `ModelSpawner`, rather than a push subscription: `AgentDriver` already reads
//! exactly the state needed (`run_conversation_id`, the terminal view's action model)
//! through this same read-only, spawner-based pattern used by `run_snapshot_upload`.
//! This trades a small amount of latency (up to [`SAFE_BOUNDARY_POLL_INTERVAL`]) for
//! avoiding new push-subscription wiring through the UI model graph.

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
use super::snapshot::{self, CheckpointResult, DeclarationsWriterHandle};
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::server::server_api::harness_support::{CheckpointGeneration, HarnessSupportClient};

/// Whether the conversation can tolerate a checkpoint attempt right now.
///
/// `DriverGone` is deliberately distinct from `Busy`: a dropped `AgentDriver` used to be
/// reported as "safe", which left the coordinator gathering and uploading the whole
/// workspace every interval forever, since nothing else ever stops the loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Boundary {
    /// Not mid-turn: safe to touch the filesystem and network.
    Safe,
    /// Mid-turn or actions in flight; retry after [`SAFE_BOUNDARY_POLL_INTERVAL`].
    Busy,
    /// The `AgentDriver` this coordinator serves no longer exists, so there is nothing
    /// left to checkpoint for and the loop must stop.
    DriverGone,
}

/// A safe-boundary predicate, decoupled from `ModelSpawner<AgentDriver>` so
/// [`coordinator_loop`] can be exercised in isolation by tests. Production code builds
/// this from [`is_safe_boundary`]; tests supply a directly-controllable closure.
type BoundaryCheck = Arc<dyn Fn() -> BoxFuture<'static, Boundary> + Send + Sync>;

/// Default cadence between the end of one checkpoint attempt and the timer firing again,
/// absent an override on `AgentDriverOptions`. Deliberately much coarser than
/// `HARNESS_SAVE_INTERVAL` (30s): each attempt gathers and uploads the whole workspace,
/// so it is priced as minutes-scale background work rather than a lightweight save.
///
/// Note this is measured from attempt *completion*, not attempt start: the timer is only
/// restarted once the `InFlight` state resolves, so back-to-back attempts can never
/// overlap.
pub(super) const DEFAULT_CHECKPOINT_INTERVAL: Duration = Duration::from_secs(5 * 60);
/// Upper bound on additive jitter, so agents scheduled at the same time don't all
/// checkpoint in lockstep.
const CHECKPOINT_JITTER: Duration = Duration::from_secs(30);
/// How often the `Due` state re-checks whether the conversation is at a safe boundary.
const SAFE_BOUNDARY_POLL_INTERVAL: Duration = Duration::from_secs(2);
/// How long `Due` will wait for a safe boundary before checkpointing anyway.
///
/// Without a cap, a single long turn (or a conversation parked in a state the predicate
/// never calls safe) starves the feature entirely -- exactly the long-running case periodic
/// checkpoints exist for. A slightly-inconsistent checkpoint is strictly better than none,
/// since the previous committed generation is only replaced on success.
const MAX_BOUNDARY_DEFERRAL: Duration = Duration::from_secs(10 * 60);
/// Slack added on top of the per-attempt floor by [`finalize_budget`], and to the
/// belt-and-braces bound in [`CheckpointCoordinatorHandle::finalize`], so neither the
/// coordinator's ack round trip nor the outer timeout eats into the attempt's own budget.
const FINALIZE_ACK_SLACK: Duration = Duration::from_secs(10);

/// The shutdown budget a caller must grant [`CheckpointCoordinatorHandle::finalize`] for a
/// final attempt to be possible.
///
/// Exposed so `AgentDriver` cannot drift out of sync with the floor enforced in
/// [`finalize_with_new_attempt`]. Passing anything at or below `script_timeout +
/// upload_timeout` silently skips the final attempt -- and because the coordinator owns the
/// whole end-of-run path, that means no end-of-run snapshot at all.
pub(super) fn finalize_budget(script_timeout: Duration, upload_timeout: Duration) -> Duration {
    script_timeout + upload_timeout + FINALIZE_ACK_SLACK
}

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
        declarations_writer: Option<DeclarationsWriterHandle>,
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
            declarations_writer,
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
            None,
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
        declarations_writer: Option<DeclarationsWriterHandle>,
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
                declarations_writer,
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
    ///
    /// Callers should derive `budget` from [`finalize_budget`] rather than passing a
    /// per-attempt timeout directly: the floor in [`finalize_with_new_attempt`] is
    /// `script_timeout + upload_timeout`, so a smaller budget silently skips the final
    /// attempt.
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
        // bound is defense-in-depth so a coordinator bug cannot wedge shutdown -- hence
        // the added slack: bounding on exactly `budget` would routinely preempt the
        // attempt the coordinator is legitimately still finishing, instead of only
        // catching a bug.
        let _ = tokio::time::timeout(budget + FINALIZE_ACK_SLACK, ack_rx).await;
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

/// Run one checkpoint attempt to completion: drain pending declarations writes,
/// regenerate declarations, then gather, upload, and commit. Bounded by `upload_timeout`;
/// `script_timeout` separately bounds only the declarations-script sub-step (matching the
/// legacy pipeline).
///
/// `generation` carries the previous attempt's generation when this attempt is a retry, so
/// the re-gathered payload overwrites that attempt's staged objects instead of adding a new
/// set. See [`snapshot::run_checkpoint_from_declarations_file`].
#[allow(clippy::too_many_arguments)]
async fn run_one_attempt(
    client: Arc<dyn HarnessSupportClient>,
    task_id: AmbientAgentTaskId,
    working_dir: PathBuf,
    declarations_writer: Option<DeclarationsWriterHandle>,
    script_timeout: Duration,
    upload_timeout: Duration,
    generation: Option<CheckpointGeneration>,
) -> CheckpointResult {
    // Drain queued driver-side `file` appends before the bash script starts appending its
    // own `repo` entries to the same append-only JSONL. `AgentDriver::run_snapshot_upload`
    // does this on the legacy path for exactly this reason; without it a checkpoint can
    // both miss the agent's most recent edits and race the script on the shared file.
    if let Some(writer) = &declarations_writer {
        writer.flush().await;
    }
    snapshot::run_declarations_script(&working_dir, &task_id, script_timeout).await;
    let path = snapshot::resolve_declarations_path(Some(&task_id));
    // Kept so a timeout can still report the generation it was retrying: the attempt may have
    // staged objects under it, and the next retry should overwrite them rather than add more.
    let retried_generation = generation.clone();
    match snapshot::run_checkpoint_from_declarations_file(&path, client, generation)
        .with_timeout(upload_timeout)
        .await
    {
        Ok(result) => result,
        Err(_) => CheckpointResult::Failed {
            generation: retried_generation,
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
#[allow(clippy::too_many_arguments)]
fn start_attempt(
    client: Arc<dyn HarnessSupportClient>,
    task_id: AmbientAgentTaskId,
    working_dir: PathBuf,
    declarations_writer: Option<DeclarationsWriterHandle>,
    script_timeout: Duration,
    upload_timeout: Duration,
    generation: Option<CheckpointGeneration>,
    background: &Background,
) -> oneshot::Receiver<CheckpointResult> {
    let (tx, rx) = oneshot::channel();
    background
        .spawn(async move {
            let result = run_one_attempt(
                client,
                task_id,
                working_dir,
                declarations_writer,
                script_timeout,
                upload_timeout,
                generation,
            )
            .await;
            let _ = tx.send(result);
        })
        .detach();
    rx
}

/// Query `AgentDriver` (via its spawner) for whether the conversation is currently at
/// a safe boundary.
///
/// [`Boundary::Safe`] when the driver has no conversation yet, when its conversation can no
/// longer be found, or when the conversation is quiescent -- there is nothing to interrupt.
/// [`Boundary::DriverGone`] when the driver itself has been dropped, which stops the loop
/// rather than being conflated with "safe" and checkpointing forever.
///
/// `InProgress`/`TransientError` do not immediately imply `Busy`: for most of a turn the
/// agent is waiting on the model's response rather than touching the filesystem, and only
/// actually executing an action (a pending or running entry in the terminal's action model)
/// risks a concurrent mutation. Treating the whole status as `Busy` would make the safe-
/// boundary check nearly useless for a continuously active agent, since it would almost
/// never see anything but `InProgress` before `MAX_BOUNDARY_DEFERRAL` forces a checkpoint
/// anyway.
async fn is_safe_boundary(spawner: &ModelSpawner<AgentDriver>) -> Boundary {
    let result = spawner
        .spawn(|driver, ctx| {
            let Some(conversation_id) = driver.run_conversation_id else {
                return Boundary::Safe;
            };
            let Some(status) = BlocklistAIHistoryModel::as_ref(ctx)
                .conversation(&conversation_id)
                .map(|conversation| conversation.status().clone())
            else {
                return Boundary::Safe;
            };
            // Quiescent states, checked before the pending-action sweep below.
            //
            // `Blocked` in particular *is* backed by a pending action, so letting it fall
            // through would report `Busy` forever: a run parked on user approval (often for
            // hours) would poll every couple of seconds and never checkpoint, even though
            // nothing is mutating the workspace. That is precisely when a checkpoint is
            // most valuable.
            if status.is_waiting_for_events() || status.is_blocked() || status.is_done() {
                return Boundary::Safe;
            }
            // `InProgress` (running a turn) and `TransientError` (a failed turn about to be
            // retried) both cover time spent waiting on the model as well as time spent
            // executing actions, so fall through to the action model rather than treating
            // either as unconditionally busy.
            let terminal_view = driver
                .terminal_driver
                .as_ref(ctx)
                .terminal_view()
                .as_ref(ctx);
            if terminal_view
                .ai_action_model()
                .as_ref(ctx)
                .has_unfinished_actions_for_conversation(conversation_id)
            {
                Boundary::Busy
            } else {
                Boundary::Safe
            }
        })
        .await;
    result.unwrap_or(Boundary::DriverGone)
}

/// How the `Due` state resolved.
enum DueOutcome {
    /// Proceed to `InFlight`.
    Safe,
    /// A finalize request arrived while waiting; the caller owns it.
    Finalize(FinalizeRequest),
    /// The coordinator should stop: the driver is gone, or every handle was dropped.
    Stop,
}

/// Poll for a safe boundary, staying responsive to finalize throughout.
///
/// The boundary check is itself awaited inside `select!` rather than before it: it is a
/// round trip through `AgentDriver`'s model task queue, and a stalled queue must not be able
/// to wedge shutdown behind an unbounded await.
async fn wait_for_safe_boundary(
    boundary_check: &BoundaryCheck,
    finalize_rx: &mut mpsc::UnboundedReceiver<FinalizeRequest>,
) -> DueOutcome {
    let due_since = Instant::now();
    loop {
        // Checked immediately on entry (not only after the first poll interval elapses) so
        // an already-safe conversation doesn't pay needless latency.
        let boundary = futures::select! {
            boundary = boundary_check().fuse() => boundary,
            request = finalize_rx.recv().fuse() => {
                return request.map_or(DueOutcome::Stop, DueOutcome::Finalize);
            }
        };
        match boundary {
            Boundary::Safe => return DueOutcome::Safe,
            Boundary::DriverGone => {
                log::info!("AgentDriver is gone; stopping the periodic checkpoint coordinator");
                return DueOutcome::Stop;
            }
            Boundary::Busy => {}
        }
        if due_since.elapsed() >= MAX_BOUNDARY_DEFERRAL {
            log::warn!(
                "Conversation has not reached a safe boundary in {MAX_BOUNDARY_DEFERRAL:?}; \
                 checkpointing anyway rather than skipping this cycle entirely"
            );
            return DueOutcome::Safe;
        }
        futures::select! {
            _ = Timer::after(SAFE_BOUNDARY_POLL_INTERVAL).fuse() => {}
            request = finalize_rx.recv().fuse() => {
                return request.map_or(DueOutcome::Stop, DueOutcome::Finalize);
            }
        }
    }
}

/// Handle a finalize request received while no attempt is currently in flight: start
/// exactly one best-effort attempt only if `budget` exceeds the gather/upload floor,
/// bound it by the remaining budget, then ack.
#[allow(clippy::too_many_arguments)]
async fn finalize_with_new_attempt(
    request: FinalizeRequest,
    client: Arc<dyn HarnessSupportClient>,
    task_id: AmbientAgentTaskId,
    working_dir: PathBuf,
    declarations_writer: Option<DeclarationsWriterHandle>,
    script_timeout: Duration,
    upload_timeout: Duration,
    generation: Option<CheckpointGeneration>,
) {
    let floor = script_timeout + upload_timeout;
    let remaining = request.deadline.saturating_duration_since(Instant::now());
    if remaining > floor {
        log::info!(
            "Starting final checkpoint attempt at shutdown (remaining budget {remaining:?})"
        );
        let attempt = run_one_attempt(
            client,
            task_id,
            working_dir,
            declarations_writer,
            script_timeout,
            upload_timeout,
            generation,
        );
        if tokio::time::timeout(remaining, attempt).await.is_err() {
            log::warn!("Final checkpoint attempt did not complete within {remaining:?}");
        }
    } else {
        // Callers must size the budget with `finalize_budget`; anything at or below the
        // floor lands here and produces no end-of-run checkpoint at all.
        log::warn!(
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
    declarations_writer: Option<DeclarationsWriterHandle>,
    boundary_check: BoundaryCheck,
    interval: Duration,
    jitter: Duration,
    script_timeout: Duration,
    upload_timeout: Duration,
    background: Arc<Background>,
    mut finalize_rx: mpsc::UnboundedReceiver<FinalizeRequest>,
) {
    // Generation of the last attempt that failed, if any. Retrying under it overwrites that
    // attempt's staged objects; minting per attempt would instead pile up a new set each time
    // and can exhaust the server's per-execution staging budget. Cleared once an attempt
    // commits (its objects are the committed checkpoint) or skips (nothing was staged).
    let mut pending_generation: Option<CheckpointGeneration> = None;
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
                    declarations_writer.clone(),
                    script_timeout,
                    upload_timeout,
                    pending_generation,
                )
                .await;
                return;
            }
        }

        // --- Due: poll for a safe boundary, staying responsive to finalize. ---
        match wait_for_safe_boundary(&boundary_check, &mut finalize_rx).await {
            DueOutcome::Safe => {}
            DueOutcome::Finalize(request) => {
                finalize_with_new_attempt(
                    request,
                    client.clone(),
                    task_id,
                    working_dir.clone(),
                    declarations_writer.clone(),
                    script_timeout,
                    upload_timeout,
                    pending_generation,
                )
                .await;
                return;
            }
            DueOutcome::Stop => return,
        }

        // --- InFlight: run exactly one attempt, never overlapping another. ---
        let mut result_rx = start_attempt(
            client.clone(),
            task_id,
            working_dir.clone(),
            declarations_writer.clone(),
            script_timeout,
            upload_timeout,
            pending_generation.clone(),
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
                        pending_generation = None;
                    }
                    Ok(CheckpointResult::Skipped) => {
                        log::info!("Periodic checkpoint skipped: no usable declarations");
                        pending_generation = None;
                    }
                    Ok(CheckpointResult::Failed { generation, reason }) => {
                        log::warn!(
                            "Periodic checkpoint attempt failed (generation={:?}): {reason}",
                            generation.as_ref().map(CheckpointGeneration::as_str)
                        );
                        // Retry under the same generation so the next attempt overwrites
                        // whatever this one staged instead of staging a second set.
                        pending_generation = generation;
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
                // failures too: checkpoints are best-effort with no recovery-point or
                // recovery-time guarantee, so retrying sooner isn't worth the extra
                // whole-workspace gather and upload.
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
