use super::{recovery_action, RecoveryAction};

// Argument order: has_received_client_actions, is_retryable, is_transient_failure,
// has_retry_budget, can_attempt_resume_on_error, is_online.

#[test]
fn pre_action_failures_retry() {
    assert_eq!(
        recovery_action(false, true, true, true, true, true),
        RecoveryAction::RetryNow
    );
    // Retryability alone is enough pre-actions; transience only gates resumes.
    assert_eq!(
        recovery_action(false, true, false, true, true, true),
        RecoveryAction::RetryNow
    );
    // Resume eligibility is irrelevant pre-actions.
    assert_eq!(
        recovery_action(false, true, true, true, false, true),
        RecoveryAction::RetryNow
    );
}

#[test]
fn pre_action_failures_wait_for_connectivity_when_offline() {
    assert_eq!(
        recovery_action(false, true, true, true, true, false),
        RecoveryAction::RetryWhenOnline
    );
}

#[test]
fn pre_action_budget_exhaustion_is_terminal() {
    // The request has already been retried MAX_RETRIES times; stop.
    assert_eq!(
        recovery_action(false, true, true, false, true, true),
        RecoveryAction::Fail
    );
    assert_eq!(
        recovery_action(false, true, true, false, true, false),
        RecoveryAction::Fail
    );
}

#[test]
fn non_retryable_pre_action_failure_is_terminal() {
    assert_eq!(
        recovery_action(false, false, false, true, true, true),
        RecoveryAction::Fail
    );
}

#[test]
fn post_action_transient_failures_resume() {
    assert_eq!(
        recovery_action(true, true, true, true, true, true),
        RecoveryAction::Resume
    );
    // Offline doesn't change the decision; the resume spawn waits for connectivity.
    assert_eq!(
        recovery_action(true, true, true, true, true, false),
        RecoveryAction::Resume
    );
    // The in-request retry budget is irrelevant once actions have executed.
    assert_eq!(
        recovery_action(true, true, true, false, true, true),
        RecoveryAction::Resume
    );
}

#[test]
fn post_action_failures_without_resume_eligibility_are_terminal() {
    // Resume requests themselves run with can_attempt_resume_on_error=false,
    // bounding recovery to a single resume.
    assert_eq!(
        recovery_action(true, true, true, true, false, true),
        RecoveryAction::Fail
    );
}

#[test]
fn post_action_application_level_failures_are_terminal() {
    // Retryable but not transient (e.g. quota, overload): a resume would fail
    // identically, so the conversation ends instead.
    assert_eq!(
        recovery_action(true, true, false, true, true, true),
        RecoveryAction::Fail
    );
}
