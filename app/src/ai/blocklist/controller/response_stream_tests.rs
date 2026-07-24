#[cfg(not(target_family = "wasm"))]
use super::{
    GeapRefreshDispatch, ResponseStream, geap_refresh_dispatch_sends, replace_geap_credentials,
};
use super::{RecoveryAction, recovery_action};
use crate::ai::agent::RenderableAIError;
use crate::ai::agent::api::RequestParams;
#[cfg(not(target_family = "wasm"))]
use crate::network::NetworkStatus;
use crate::server::server_api::AIApiError;
#[cfg(not(target_family = "wasm"))]
use crate::server::server_api::ServerApiProvider;
use std::sync::Arc;
#[cfg(not(target_family = "wasm"))]
use uuid::Uuid;

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

#[cfg(not(target_family = "wasm"))]
#[test]
fn geap_request_refresh_dispatch_requires_success_and_fresh_credentials() {
    assert!(geap_refresh_dispatch_sends(
        GeapRefreshDispatch::Refreshed,
        true
    ));
    // A successful waiter without a newly servable credential must not send
    // the request with the expired snapshot.
    assert!(!geap_refresh_dispatch_sends(
        GeapRefreshDispatch::Refreshed,
        false
    ));
    // Failed, timed-out, and cancelled waits all map to the terminal fallback.
    assert!(!geap_refresh_dispatch_sends(
        GeapRefreshDispatch::Failed,
        true
    ));
    assert!(!geap_refresh_dispatch_sends(
        GeapRefreshDispatch::Failed,
        false
    ));
}

#[test]
fn geap_refresh_failure_is_terminal_and_uses_manual_recovery_renderable() {
    let error = Arc::new(AIApiError::GeminiEnterpriseCredentialsRefreshFailed);
    assert!(!error.is_recoverable());
    assert!(!error.is_actionable());
    assert!(matches!(
        RenderableAIError::from(&error),
        RenderableAIError::GeminiEnterpriseCredentialsExpiredOrInvalid
    ));
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
#[test]
fn geap_request_refresh_injects_the_fresh_token_before_send() {
    let mut params = RequestParams::new_for_test();
    params.api_keys = Some(Default::default());
    params.api_keys.as_mut().unwrap().google_cloud_credentials = Some(
        warp_multi_agent_api::request::settings::api_keys::GoogleCloudCredentials {
            access_token: "expired-token".to_string(),
        },
    );

    replace_geap_credentials(
        &mut params,
        Some(
            warp_multi_agent_api::request::settings::api_keys::GoogleCloudCredentials {
                access_token: "fresh-token".to_string(),
            },
        ),
    );

    assert_eq!(
        params
            .api_keys
            .unwrap()
            .google_cloud_credentials
            .unwrap()
            .access_token,
        "fresh-token"
    );
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn geap_spawn_request_dispatches_fresh_token_after_refresh_success() {
    use warpui::{AddSingletonModel, App};

    App::test((), |mut app| async move {
        app.add_singleton_model(|_| NetworkStatus::new());
        app.add_singleton_model(|_| ServerApiProvider::new_for_test());
        let stream = app
            .add_model(|_| ResponseStream::new_for_test(super::ResponseStreamId::new_for_test()));
        let request_id = Uuid::new_v4();

        stream.update(&mut app, |stream, ctx| {
            stream.current_request_id = Some(request_id);
            stream.params.api_keys = Some(Default::default());
            stream
                .params
                .api_keys
                .as_mut()
                .unwrap()
                .google_cloud_credentials = Some(
                warp_multi_agent_api::request::settings::api_keys::GoogleCloudCredentials {
                    access_token: "expired-token".to_string(),
                },
            );
            stream.dispatch_geap_refresh_result(
                request_id,
                GeapRefreshDispatch::Refreshed,
                Some(
                    warp_multi_agent_api::request::settings::api_keys::GoogleCloudCredentials {
                        access_token: "fresh-token".to_string(),
                    },
                ),
                futures::channel::oneshot::channel().1,
                ctx,
            );
            assert_eq!(
                stream
                    .params
                    .api_keys
                    .as_ref()
                    .unwrap()
                    .google_cloud_credentials
                    .as_ref()
                    .unwrap()
                    .access_token,
                "fresh-token"
            );
        });
    });
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn geap_spawn_request_dispatch_failure_does_not_send_expired_token() {
    use warpui::{AddSingletonModel, App};

    App::test((), |mut app| async move {
        app.add_singleton_model(|_| NetworkStatus::new());
        let stream = app
            .add_model(|_| ResponseStream::new_for_test(super::ResponseStreamId::new_for_test()));
        let request_id = Uuid::new_v4();

        stream.update(&mut app, |stream, ctx| {
            stream.current_request_id = Some(request_id);
            stream.params.api_keys = Some(Default::default());
            stream
                .params
                .api_keys
                .as_mut()
                .unwrap()
                .google_cloud_credentials = Some(
                warp_multi_agent_api::request::settings::api_keys::GoogleCloudCredentials {
                    access_token: "expired-token".to_string(),
                },
            );
            stream.dispatch_geap_refresh_result(
                request_id,
                GeapRefreshDispatch::Failed,
                None,
                futures::channel::oneshot::channel().1,
                ctx,
            );
            assert_eq!(
                stream
                    .params
                    .api_keys
                    .as_ref()
                    .unwrap()
                    .google_cloud_credentials
                    .as_ref()
                    .unwrap()
                    .access_token,
                "expired-token"
            );
            assert!(stream.error_event_emitted);
        });
    });
}
