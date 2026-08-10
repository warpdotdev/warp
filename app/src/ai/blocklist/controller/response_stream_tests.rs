#[cfg(not(target_family = "wasm"))]
use super::apply_geap_refresh_to_params;
use super::{RecoveryAction, recovery_action};
#[cfg(not(target_family = "wasm"))]
use crate::ai::agent::api::RequestParams;

// Argument order: has_received_client_actions, is_recoverable, has_retry_budget,
// can_attempt_resume_on_error, is_online.

#[test]
fn pre_action_failures_retry() {
    assert_eq!(
        recovery_action(false, true, true, true, true),
        RecoveryAction::RetryNow
    );
    // Resume eligibility is irrelevant pre-actions.
    assert_eq!(
        recovery_action(false, true, true, false, true),
        RecoveryAction::RetryNow
    );
}

#[test]
fn pre_action_failures_wait_for_connectivity_when_offline() {
    assert_eq!(
        recovery_action(false, true, true, true, false),
        RecoveryAction::RetryWhenOnline
    );
}

#[test]
fn pre_action_budget_exhaustion_is_terminal() {
    // The request has already been retried MAX_RETRIES times; stop.
    assert_eq!(
        recovery_action(false, true, false, true, true),
        RecoveryAction::Fail
    );
    assert_eq!(
        recovery_action(false, true, false, true, false),
        RecoveryAction::Fail
    );
}

#[test]
fn non_recoverable_pre_action_failure_is_terminal() {
    assert_eq!(
        recovery_action(false, false, true, true, true),
        RecoveryAction::Fail
    );
}

#[test]
fn post_action_recoverable_failures_resume() {
    assert_eq!(
        recovery_action(true, true, true, true, true),
        RecoveryAction::Resume
    );
    // Offline doesn't change the decision; the resume spawn waits for connectivity.
    assert_eq!(
        recovery_action(true, true, true, true, false),
        RecoveryAction::Resume
    );
    // The in-request retry budget is irrelevant once actions have executed.
    assert_eq!(
        recovery_action(true, true, false, true, true),
        RecoveryAction::Resume
    );
}

#[test]
fn post_action_failures_without_resume_eligibility_are_terminal() {
    // Resume requests themselves run with can_attempt_resume_on_error=false,
    // bounding recovery to a single resume.
    assert_eq!(
        recovery_action(true, true, true, false, true),
        RecoveryAction::Fail
    );
}

#[test]
fn non_recoverable_post_action_failure_is_terminal() {
    // A non-recoverable error (e.g. a client error) ends the conversation even
    // after actions have executed.
    assert_eq!(
        recovery_action(true, false, true, true, true),
        RecoveryAction::Fail
    );
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
