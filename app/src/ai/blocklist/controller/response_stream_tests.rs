#[cfg(not(target_family = "wasm"))]
use std::time::Duration;

#[cfg(not(target_family = "wasm"))]
use super::apply_geap_refresh_to_params;
use super::{FailReason, MAX_RECOVERY_ATTEMPTS, RecoveryAction, RecoveryBudget, recovery_action};
#[cfg(not(target_family = "wasm"))]
use super::{ResponseStream, ResponseStreamId};
#[cfg(not(target_family = "wasm"))]
use crate::ai::agent::api::RequestParams;
// `agent_sdk` (and so the driver's recovery deadline) is native-only.
#[cfg(not(target_family = "wasm"))]
use crate::ai::agent_sdk::driver::AUTO_RESUME_TIMEOUT;
#[cfg(not(target_family = "wasm"))]
use crate::server::retry_strategies::backoff_after_attempts;

// Argument order: has_received_client_actions, is_recoverable, recovery, is_online.

/// A budget with every recovery attempt spent.
fn exhausted() -> RecoveryBudget {
    let mut recovery = RecoveryBudget::fresh();
    for _ in 0..MAX_RECOVERY_ATTEMPTS {
        recovery = recovery.next_attempt();
    }
    recovery
}

#[test]
fn pre_action_failures_retry() {
    assert_eq!(
        recovery_action(false, true, RecoveryBudget::fresh(), true),
        RecoveryAction::Retry
    );
    // Resume eligibility is irrelevant pre-actions.
    assert_eq!(
        recovery_action(false, true, RecoveryBudget::fresh().without_resume(), true),
        RecoveryAction::Retry
    );
}

#[test]
fn pre_action_failures_wait_for_connectivity_when_offline() {
    assert_eq!(
        recovery_action(false, true, RecoveryBudget::fresh(), false),
        RecoveryAction::RetryWhenOnline
    );
}

#[test]
fn pre_action_budget_exhaustion_is_terminal() {
    // The turn has already spent MAX_RECOVERY_ATTEMPTS attempts; stop.
    assert_eq!(
        recovery_action(false, true, exhausted(), true),
        RecoveryAction::Fail(FailReason::BudgetExhausted)
    );
    assert_eq!(
        recovery_action(false, true, exhausted(), false),
        RecoveryAction::Fail(FailReason::BudgetExhausted)
    );
}

#[test]
fn non_recoverable_pre_action_failure_is_terminal() {
    assert_eq!(
        recovery_action(false, false, RecoveryBudget::fresh(), true),
        RecoveryAction::Fail(FailReason::NotRecoverable)
    );
}

#[test]
fn post_action_recoverable_failures_resume() {
    assert_eq!(
        recovery_action(true, true, RecoveryBudget::fresh(), true),
        RecoveryAction::Resume
    );
    // Offline doesn't change the decision; the resume spawn waits for connectivity.
    assert_eq!(
        recovery_action(true, true, RecoveryBudget::fresh(), false),
        RecoveryAction::Resume
    );
}

#[test]
fn post_action_failures_without_resume_eligibility_are_terminal() {
    // Passive background requests may not resume, and a post-action failure has no other
    // recovery available.
    assert_eq!(
        recovery_action(true, true, RecoveryBudget::fresh().without_resume(), true),
        RecoveryAction::Fail(FailReason::ResumeNotAllowed)
    );
}

#[test]
fn ineligibility_is_reported_ahead_of_an_exhausted_budget() {
    // A passive request that spent its budget on pre-action retries and then fails after
    // actions is blocked by both constraints. The reason logged should be the one that would
    // still block it if the budget were full, since that is what a reader needs to know.
    let exhausted_passive = exhausted().without_resume();
    assert_eq!(
        recovery_action(true, true, exhausted_passive, true),
        RecoveryAction::Fail(FailReason::ResumeNotAllowed)
    );
    // Pre-action, the budget is the only thing standing in the way, so it is the reason.
    assert_eq!(
        recovery_action(false, true, exhausted_passive, true),
        RecoveryAction::Fail(FailReason::BudgetExhausted)
    );
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn a_scheduled_resume_inherits_a_charged_budget() {
    // The boundary the controller consumes: whatever budget a failed request was running
    // with, the resume it schedules must run with that budget charged one attempt, and with
    // resume eligibility carried over unchanged. Getting this wrong in either direction is
    // the bug REMOTE-2269 was about — a fresh budget would restart recovery from scratch,
    // and a lost `resume_allowed` would silently re-enable resumes for a passive request.
    let stream = ResponseStream::new_for_test(ResponseStreamId::new_for_test());
    let before = RecoveryBudget::fresh().without_resume();
    let after = stream.recovery.next_attempt();

    assert_eq!(stream.recovery, before, "test fixture drifted");
    assert_eq!(after.attempts_used(), before.attempts_used() + 1);
    // Eligibility is preserved, so the resumed request is still bound by the same rule.
    assert_eq!(
        recovery_action(true, true, after, true),
        RecoveryAction::Fail(FailReason::ResumeNotAllowed)
    );
}

#[test]
fn non_recoverable_post_action_failure_is_terminal() {
    // A non-recoverable error (e.g. a client error) ends the conversation even
    // after actions have executed.
    assert_eq!(
        recovery_action(true, false, RecoveryBudget::fresh(), true),
        RecoveryAction::Fail(FailReason::NotRecoverable)
    );
}

#[test]
fn resume_failures_consume_the_shared_budget() {
    // Every resume is charged against the budget, so a conversation that keeps hitting
    // transport resets after client actions gets MAX_RECOVERY_ATTEMPTS resumes and then
    // fails — where it used to get exactly one, because the resumed request ran with
    // resumes disabled rather than with the remaining budget.
    let mut recovery = RecoveryBudget::fresh();
    let mut actions = Vec::new();
    for _ in 0..MAX_RECOVERY_ATTEMPTS + 1 {
        let action = recovery_action(
            /*has_received_client_actions*/ true, true, recovery, true,
        );
        actions.push(action);
        if action == RecoveryAction::Resume {
            recovery = recovery.next_attempt();
        }
    }

    let (resumes, terminal) = actions.split_at(MAX_RECOVERY_ATTEMPTS);
    assert!(
        resumes
            .iter()
            .all(|action| *action == RecoveryAction::Resume)
    );
    assert_eq!(
        terminal,
        [RecoveryAction::Fail(FailReason::BudgetExhausted)]
    );
    assert_eq!(recovery.attempts_used(), MAX_RECOVERY_ATTEMPTS);
}

#[test]
fn retries_and_resumes_share_one_budget() {
    // The failure mode from REMOTE-2269: a pre-action failure retries, the retry then
    // fails after client actions have streamed (which is when the pre-action path is no
    // longer available), and recovery switches to resumes. Both kinds draw from the same
    // counter, so the chain is bounded by MAX_RECOVERY_ATTEMPTS sends in total rather than
    // by a per-kind allowance.
    let mut recovery = RecoveryBudget::fresh();

    assert_eq!(
        recovery_action(false, true, recovery, true),
        RecoveryAction::Retry
    );
    recovery = recovery.next_attempt();

    assert_eq!(
        recovery_action(true, true, recovery, true),
        RecoveryAction::Resume
    );
    recovery = recovery.next_attempt();

    assert_eq!(
        recovery_action(true, true, recovery, true),
        RecoveryAction::Resume
    );
    recovery = recovery.next_attempt();

    // Three attempts spent: the next failure is terminal whichever kind of recovery it
    // would have used, because the counter is shared and not per-kind.
    assert_eq!(
        recovery_action(true, true, recovery, true),
        RecoveryAction::Fail(FailReason::BudgetExhausted)
    );
    assert_eq!(
        recovery_action(false, true, recovery, true),
        RecoveryAction::Fail(FailReason::BudgetExhausted)
    );
}

#[test]
fn spending_an_attempt_preserves_resume_eligibility() {
    // Charging an attempt must not quietly re-enable resumes for a passive request, nor
    // disable them for a normal one.
    let passive = RecoveryBudget::fresh().without_resume().next_attempt();
    assert_eq!(
        recovery_action(true, true, passive, true),
        RecoveryAction::Fail(FailReason::ResumeNotAllowed)
    );

    let normal = RecoveryBudget::fresh().next_attempt();
    assert_eq!(
        recovery_action(true, true, normal, true),
        RecoveryAction::Resume
    );
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn the_recovery_backoff_fits_inside_the_cloud_run_recovery_window() {
    // The driver re-arms AUTO_RESUME_TIMEOUT per recovery attempt, so what has to fit in
    // that window is a single attempt's wait, not the whole chain. Assert both anyway: if
    // the budget or the backoff schedule ever grows enough to approach the deadline, a run
    // would start dying on the deadline instead of on the failure it was recovering from.
    let total: Duration = (1..=MAX_RECOVERY_ATTEMPTS)
        .map(backoff_after_attempts)
        .sum();
    assert!(
        total * 2 < AUTO_RESUME_TIMEOUT,
        "total recovery backoff {total:?} is too close to AUTO_RESUME_TIMEOUT {AUTO_RESUME_TIMEOUT:?}"
    );
    for attempt in 1..=MAX_RECOVERY_ATTEMPTS {
        assert!(backoff_after_attempts(attempt) * 4 < AUTO_RESUME_TIMEOUT);
    }
}

#[cfg(not(target_family = "wasm"))]
fn params_with_geap_token(access_token: &str) -> RequestParams {
    let mut params = RequestParams::new_for_test();
    params.api_keys = Some(Default::default());
    params.api_keys.as_mut().unwrap().google_cloud_credentials = Some(
        warp_multi_agent_api::request::settings::api_keys::GoogleCloudCredentials {
            access_token: access_token.to_string(),
        },
    );
    params
}

#[cfg(not(target_family = "wasm"))]
fn geap_token(params: &RequestParams) -> Option<&str> {
    params
        .api_keys
        .as_ref()?
        .google_cloud_credentials
        .as_ref()
        .map(|credentials| credentials.access_token.as_str())
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn geap_refresh_success_swaps_in_the_fresh_credential() {
    let mut params = params_with_geap_token("expired-token");

    apply_geap_refresh_to_params(
        &mut params,
        Some(
            warp_multi_agent_api::request::settings::api_keys::GoogleCloudCredentials {
                access_token: "fresh-token".to_string(),
            },
        ),
    );

    assert_eq!(geap_token(&params), Some("fresh-token"));
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn geap_refresh_failure_leaves_the_request_snapshot_untouched() {
    // A failed mint, a timeout, and a dropped sender all arrive here as
    // `None`. The request must go out byte-for-byte as it would have without
    // the refresh attempt: the credential stays attached (so the server keeps
    // Gemini Enterprise in its fallback chain and reports a real credentials
    // error) rather than being stripped (which would silently reroute the
    // request to another host).
    let mut params = params_with_geap_token("expired-token");

    apply_geap_refresh_to_params(&mut params, None);

    assert_eq!(geap_token(&params), Some("expired-token"));
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn geap_refresh_does_not_add_credentials_to_a_keyless_request() {
    // A request that carries no API keys at all is left alone, so the refresh
    // path can never introduce credentials the snapshot did not already gate.
    let mut params = RequestParams::new_for_test();
    params.api_keys = None;

    apply_geap_refresh_to_params(
        &mut params,
        Some(
            warp_multi_agent_api::request::settings::api_keys::GoogleCloudCredentials {
                access_token: "fresh-token".to_string(),
            },
        ),
    );

    assert!(params.api_keys.is_none());
}
