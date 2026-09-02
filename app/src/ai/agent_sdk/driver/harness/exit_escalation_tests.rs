use super::{
    ExitEscalation, ExitEscalationAction, ExitEscalationEvent, ExitEscalationPhase,
    driver_result_after_harness_run, may_synthesize_succeeded_on_flush,
};
use crate::ai::agent_sdk::driver::AgentDriverError;

fn actions_for(events: &[ExitEscalationEvent]) -> (ExitEscalation, Vec<ExitEscalationAction>) {
    let mut escalation = ExitEscalation::new();
    let actions = events
        .iter()
        .copied()
        .map(|event| escalation.on_event(event))
        .collect();
    (escalation, actions)
}

#[test]
fn clean_exit_after_exit_request_finishes_before_followup() {
    let (escalation, actions) = actions_for(&[
        ExitEscalationEvent::ShutdownRequested,
        ExitEscalationEvent::CommandExited,
    ]);

    assert_eq!(
        actions,
        vec![ExitEscalationAction::SendExit, ExitEscalationAction::Finish,]
    );
    assert_eq!(escalation.phase(), ExitEscalationPhase::Done);
}

#[test]
fn followup_enter_is_sent_after_the_first_deadline() {
    let (escalation, actions) = actions_for(&[
        ExitEscalationEvent::ShutdownRequested,
        ExitEscalationEvent::FollowupDeadlineElapsed,
        ExitEscalationEvent::CommandExited,
    ]);

    assert_eq!(
        actions,
        vec![
            ExitEscalationAction::SendExit,
            ExitEscalationAction::SendFollowup,
            ExitEscalationAction::Finish,
        ]
    );
    assert_eq!(escalation.phase(), ExitEscalationPhase::Done);
}

#[test]
fn force_kill_completes_the_ladder_after_the_second_deadline() {
    let (escalation, actions) = actions_for(&[
        ExitEscalationEvent::ShutdownRequested,
        ExitEscalationEvent::FollowupDeadlineElapsed,
        ExitEscalationEvent::ForceKillDeadlineElapsed,
    ]);

    assert_eq!(
        actions,
        vec![
            ExitEscalationAction::SendExit,
            ExitEscalationAction::SendFollowup,
            ExitEscalationAction::ForceKillAndFinish,
        ]
    );
    assert_eq!(escalation.phase(), ExitEscalationPhase::Done);
}

#[test]
fn scanner_detection_starts_the_same_ladder_as_an_exit_request() {
    let (from_scanner, scanner_actions) = actions_for(&[
        ExitEscalationEvent::ScannerDetected,
        ExitEscalationEvent::FollowupDeadlineElapsed,
        ExitEscalationEvent::ForceKillDeadlineElapsed,
    ]);
    let (from_shutdown, shutdown_actions) = actions_for(&[
        ExitEscalationEvent::ShutdownRequested,
        ExitEscalationEvent::FollowupDeadlineElapsed,
        ExitEscalationEvent::ForceKillDeadlineElapsed,
    ]);

    assert_eq!(scanner_actions, shutdown_actions);
    assert_eq!(from_scanner, from_shutdown);
}

#[test]
fn scanner_detection_during_an_in_flight_ladder_does_not_restart_it() {
    let mut escalation = ExitEscalation::new();
    assert_eq!(
        escalation.on_event(ExitEscalationEvent::ShutdownRequested),
        ExitEscalationAction::SendExit
    );
    assert_eq!(
        escalation.on_event(ExitEscalationEvent::ScannerDetected),
        ExitEscalationAction::Ignore
    );
    assert_eq!(
        escalation.phase(),
        ExitEscalationPhase::AwaitingGracefulExit
    );
}

#[test]
fn bounded_timeout_does_not_synthesize_succeeded_and_is_not_a_driver_error() {
    let timeout = Err(AgentDriverError::HarnessExitTimedOut {
        harness: "claude".into(),
    });

    assert!(!may_synthesize_succeeded_on_flush(&timeout));
    assert!(driver_result_after_harness_run(timeout).is_ok());
}

#[test]
fn successful_harness_completion_may_synthesize_succeeded() {
    assert!(may_synthesize_succeeded_on_flush(&Ok(())));
    assert!(driver_result_after_harness_run(Ok(())).is_ok());
}

#[test]
fn runtime_failure_stays_a_driver_error_and_does_not_synthesize_succeeded() {
    let failure = Err(AgentDriverError::HarnessRuntimeFailureDetected {
        harness: "claude".into(),
        pattern: "credit balance is too low".into(),
        excerpt: "Error: Your credit balance is too low".into(),
    });

    assert!(!may_synthesize_succeeded_on_flush(&failure));
    assert!(matches!(
        driver_result_after_harness_run(failure),
        Err(AgentDriverError::HarnessRuntimeFailureDetected { .. })
    ));
}
