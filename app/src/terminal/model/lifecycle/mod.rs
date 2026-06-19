mod telemetry;
mod transition;

pub use telemetry::LifecycleRecoveryRecord;
pub(in crate::terminal) use telemetry::LifecycleTelemetryEvent;
use telemetry::LifecycleTelemetryLimiter;
pub(in crate::terminal) use transition::{
    CommandStartKind, IgnoreReason, LifecycleAction, LifecycleInput, LifecyclePhase,
    LifecycleSnapshot, LifecycleTransition, NextBlockIdDisposition, PreexecObservation,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StartCommandOutcome {
    Accepted,
    Coalesced,
    RejectedExecuting,
    IgnoredTerminated,
}

impl StartCommandOutcome {
    pub fn is_accepted(self) -> bool {
        matches!(self, StartCommandOutcome::Accepted)
    }
}

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
    pub(super) fn plan(
        &mut self,
        snapshot: &LifecycleSnapshot,
        input: LifecycleInput,
    ) -> LifecycleTransition {
        let previous_phase = transition::reconcile_phase(self.phase, snapshot);
        let (next_phase, action) = transition::plan(previous_phase, input, snapshot);
        let should_record = action.is_ignored()
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

    pub(super) fn commit(&mut self, transition: &LifecycleTransition) {
        if matches!(transition.action, LifecycleAction::BeginEpoch) {
            self.epoch = self.epoch.wrapping_add(1);
        }
        self.phase = transition.next_phase;
    }

    pub(super) fn reset_unknown(&mut self) {
        self.phase = LifecyclePhase::Unknown;
    }
}

#[cfg(test)]
#[path = "mod_test.rs"]
mod tests;
