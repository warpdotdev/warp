//! Bounded shutdown ladder for a third-party harness: `/exit`, a follow-up
//! Enter, then a best-effort force-kill. Completes without waiting to prove
//! the process exited. A harness waiting on user input skips straight to the
//! force-kill, since typed text would answer whatever prompt is open.

use super::super::AgentDriverError;

/// How far the bounded harness-exit ladder has progressed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ExitEscalationPhase {
    Running,
    AwaitingGracefulExit,
    AwaitingFollowup,
    Done,
}

/// Input that advances [`ExitEscalation`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ExitEscalationEvent {
    CommandExited,
    /// CLI session reached a terminal state and asked the driver to stop the harness.
    ShutdownRequested {
        /// The session is blocked on user input, so typed text would answer the open prompt.
        awaiting_input: bool,
    },
    /// Runtime-failure scanner confirmed a hang/auth failure and asked the driver to stop.
    ScannerDetected,
    FollowupDeadlineElapsed,
    ForceKillDeadlineElapsed,
}

/// Side effect the driver should perform in response to an event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ExitEscalationAction {
    SendExit,
    SendFollowup,
    ForceKillAndFinish,
    Finish,
    Ignore,
}

/// Decision table for the `/exit` → Enter → force-kill ladder.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ExitEscalation {
    phase: ExitEscalationPhase,
}

impl ExitEscalation {
    pub(crate) fn new() -> Self {
        Self {
            phase: ExitEscalationPhase::Running,
        }
    }

    #[cfg(test)]
    pub(crate) fn phase(&self) -> ExitEscalationPhase {
        self.phase
    }

    pub(crate) fn on_event(&mut self, event: ExitEscalationEvent) -> ExitEscalationAction {
        match (self.phase, event) {
            (
                ExitEscalationPhase::Running
                | ExitEscalationPhase::AwaitingGracefulExit
                | ExitEscalationPhase::AwaitingFollowup,
                ExitEscalationEvent::CommandExited,
            ) => {
                self.phase = ExitEscalationPhase::Done;
                ExitEscalationAction::Finish
            }
            (
                ExitEscalationPhase::Running,
                ExitEscalationEvent::ShutdownRequested {
                    awaiting_input: false,
                }
                | ExitEscalationEvent::ScannerDetected,
            ) => {
                self.phase = ExitEscalationPhase::AwaitingGracefulExit;
                ExitEscalationAction::SendExit
            }
            (
                ExitEscalationPhase::Running,
                ExitEscalationEvent::ShutdownRequested {
                    awaiting_input: true,
                },
            ) => {
                self.phase = ExitEscalationPhase::Done;
                ExitEscalationAction::ForceKillAndFinish
            }
            (
                ExitEscalationPhase::AwaitingGracefulExit,
                ExitEscalationEvent::FollowupDeadlineElapsed,
            ) => {
                self.phase = ExitEscalationPhase::AwaitingFollowup;
                ExitEscalationAction::SendFollowup
            }
            (
                ExitEscalationPhase::AwaitingFollowup,
                ExitEscalationEvent::ForceKillDeadlineElapsed,
            ) => {
                self.phase = ExitEscalationPhase::Done;
                ExitEscalationAction::ForceKillAndFinish
            }
            (
                ExitEscalationPhase::Done,
                ExitEscalationEvent::CommandExited
                | ExitEscalationEvent::ShutdownRequested { .. }
                | ExitEscalationEvent::ScannerDetected
                | ExitEscalationEvent::FollowupDeadlineElapsed
                | ExitEscalationEvent::ForceKillDeadlineElapsed,
            )
            | (
                ExitEscalationPhase::AwaitingGracefulExit | ExitEscalationPhase::AwaitingFollowup,
                ExitEscalationEvent::ShutdownRequested { .. }
                | ExitEscalationEvent::ScannerDetected,
            )
            | (
                ExitEscalationPhase::Running,
                ExitEscalationEvent::FollowupDeadlineElapsed
                | ExitEscalationEvent::ForceKillDeadlineElapsed,
            )
            | (
                ExitEscalationPhase::AwaitingGracefulExit,
                ExitEscalationEvent::ForceKillDeadlineElapsed,
            )
            | (
                ExitEscalationPhase::AwaitingFollowup,
                ExitEscalationEvent::FollowupDeadlineElapsed,
            ) => ExitEscalationAction::Ignore,
        }
    }
}

/// Maps a bounded harness-exit timeout to `Ok` so `report_driver_error` cannot
/// overwrite the CLI session's already-reported terminal state. Other errors
/// pass through.
pub(crate) fn driver_result_after_harness_run(
    result: Result<(), AgentDriverError>,
) -> Result<(), AgentDriverError> {
    match result {
        Err(AgentDriverError::HarnessExitTimedOut { .. }) => Ok(()),
        other => other,
    }
}

#[cfg(test)]
#[path = "exit_escalation_tests.rs"]
mod tests;
