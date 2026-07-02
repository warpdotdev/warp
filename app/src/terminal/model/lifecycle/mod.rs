//! Coordinates terminal block lifecycle transitions.
//!
//! `TerminalModel` supplies lifecycle inputs together with a snapshot of live block state. The
//! coordinator reconciles its remembered phase against that snapshot, asks the pure transition
//! policy for an action, and attaches rate-limited diagnostics when the action is conservative or
//! corrective. Callers apply the planned action before committing its next phase.

mod telemetry;
mod transition;

pub use telemetry::LifecycleRecoveryRecord;
pub(in crate::terminal) use telemetry::LifecycleTelemetryEvent;
use telemetry::LifecycleTelemetryLimiter;
pub(in crate::terminal) use transition::{
    CommandStartKind, IgnoreReason, LifecycleAction, LifecycleInput, LifecyclePhase,
    LifecycleSnapshot, LifecycleTransition, NextBlockIdDisposition, PreexecObservation,
};
use warp_core::features::FeatureFlag;

use super::block::BlockState;

/// Describes whether a command-start intent was accepted or conservatively ignored.
///
/// Callers must perform start-dependent side effects, such as writing PTY bytes or attaching
/// metadata only when this outcome is [`StartCommandOutcome::Accepted`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StartCommandOutcome {
    /// The active block was started.
    Accepted,
    /// The active block was already submitted, so the repeated start was coalesced.
    Coalesced,
    /// The active block was executing, so starting another command was rejected.
    RejectedExecuting,
    /// The terminal had terminated, so the start intent was ignored.
    IgnoredTerminated,
}

impl StartCommandOutcome {
    /// Returns whether callers may perform side effects associated with starting the command.
    pub fn is_accepted(self) -> bool {
        matches!(self, StartCommandOutcome::Accepted)
    }
}

/// Remembers the lifecycle phase for one terminal and plans transitions without applying mutations.
///
/// The coordinator deliberately does not own `TerminalModel` or `BlockList`. This keeps transition
/// policy testable as pure data while requiring the model to apply an accepted action before it
/// commits the planned phase.
pub(super) struct BlockLifecycleCoordinator {
    phase: LifecyclePhase,
    epoch: u64,
    telemetry_limiter: LifecycleTelemetryLimiter,
}

impl Default for BlockLifecycleCoordinator {
    fn default() -> Self {
        Self {
            phase: LifecyclePhase::Unknown,
            epoch: 0,
            telemetry_limiter: LifecycleTelemetryLimiter::default(),
        }
    }
}

impl BlockLifecycleCoordinator {
    /// Plans the action and next phase for an observed lifecycle input.
    ///
    /// Planning first reconciles the remembered phase with live block evidence so stale internal
    /// state cannot authorize a mutation. It then evaluates the pure transition policy and may
    /// attach a rate-limited diagnostic record. This method does not advance the remembered phase;
    /// the caller must apply the returned action and then call [`Self::commit`].
    pub(super) fn plan(
        &mut self,
        snapshot: &LifecycleSnapshot,
        input: LifecycleInput,
    ) -> LifecycleTransition {
        let previous_phase = transition::reconcile_phase(self.phase, snapshot);
        let (planned_next_phase, planned_action) = transition::plan(previous_phase, input);
        let recovers_command_finished = matches!(
            (input, planned_action),
            (
                LifecycleInput::CommandFinished(NextBlockIdDisposition::Novel),
                LifecycleAction::AcceptCommandFinished,
            )
        ) && match previous_phase {
            LifecyclePhase::AwaitingPrecmd | LifecyclePhase::Unknown => true,
            LifecyclePhase::AtPrompt => snapshot.is_bootstrap_done,
            LifecyclePhase::Submitted | LifecyclePhase::Executing | LifecyclePhase::Terminated => {
                false
            }
        };
        let is_gated_recovery = recovers_command_finished
            || matches!(
                planned_action,
                LifecycleAction::ReconcileCompletionThenApplyPrecmd
            );
        let (next_phase, action) =
            if is_gated_recovery && !FeatureFlag::TerminalLifecycleRecovery.is_enabled() {
                (
                    previous_phase,
                    LifecycleAction::Ignore(IgnoreReason::RecoveryDisabled),
                )
            } else {
                (planned_next_phase, planned_action)
            };
        let reconciles_missing_execution = matches!(
            (input, planned_action),
            (
                LifecycleInput::CommandFinished(NextBlockIdDisposition::Novel),
                LifecycleAction::AcceptCommandFinished,
            )
        ) && !snapshot.finished
            && snapshot.block_state != BlockState::Executing;
        let should_record = action.is_ignored()
            || is_gated_recovery
            || reconciles_missing_execution
            || snapshot.completion_mismatch
            || matches!(
                (previous_phase, input),
                (
                    LifecyclePhase::AwaitingPrecmd | LifecyclePhase::Unknown,
                    LifecycleInput::StartCommand(_) | LifecycleInput::Preexec(_)
                )
            );
        let recovery_record = should_record
            .then(|| {
                LifecycleRecoveryRecord::new(
                    previous_phase,
                    next_phase,
                    input.kind(),
                    action,
                    snapshot,
                )
            })
            .and_then(|record| self.telemetry_limiter.record(record));

        LifecycleTransition {
            previous_phase,
            next_phase,
            action,
            recovery_record,
        }
    }

    /// Commits a transition after its planned action has been applied.
    ///
    /// Beginning a shell epoch advances the diagnostic epoch counter, and every committed
    /// transition replaces the coordinator's remembered phase with its planned next phase.
    pub(super) fn commit(&mut self, transition: &LifecycleTransition) {
        if matches!(transition.action, LifecycleAction::BeginEpoch) {
            self.epoch = self.epoch.wrapping_add(1);
        }
        self.phase = transition.next_phase;
    }

    /// Forgets the remembered phase after externally supplied block state replaces or extends it.
    pub(super) fn reset_unknown(&mut self) {
        self.phase = LifecyclePhase::Unknown;
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
