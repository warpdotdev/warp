use std::sync::Arc;

use futures::executor::block_on;
use warp_core::channel::ChannelState;
use warp_core::errors::AnyhowErrorExt as _;
use warp_server_auth::auth_state::AuthState;
use warp_server_auth::credentials::AuthToken;

use super::{HttpStatusError, get_authenticated_public_api};
use crate::auth::AuthEvent;
use crate::base_client::{AuthenticatedGraphqlConfig, BaseClient, GraphqlRoutingConfig};

fn base_client(observe_iap_challenges: bool) -> (BaseClient, async_channel::Receiver<AuthEvent>) {
    let (event_sender, event_receiver) = async_channel::unbounded();
    (
        BaseClient::new(
            Arc::new(http_client::Client::new()),
            Arc::new(AuthState::new_for_test()),
            event_sender,
            None,
            GraphqlRoutingConfig::default(),
            AuthenticatedGraphqlConfig::default(),
            observe_iap_challenges,
        ),
        event_receiver,
    )
}

#[test]
fn iap_challenge_failure_emits_event_when_observation_is_enabled() {
    let _request = {
        let mut server = ChannelState::mock_server();
        server
            .mock("GET", "/api/v1/agent/identities")
            .with_status(401)
            .with_header(http_client::iap::IAP_GENERATED_RESPONSE_HEADER, "true")
            .with_body(r#"{"error":"IAP challenge"}"#)
            .create()
    };
    let (base_client, event_receiver) = base_client(true);

    let error = block_on(get_authenticated_public_api::<serde_json::Value>(
        &base_client,
        AuthToken::Bearer("token".to_string()),
        "agent/identities",
    ))
    .unwrap_err();

    assert!(error.to_string().contains("IAP challenge"));
    assert!(
        error
            .chain()
            .any(|cause| cause.downcast_ref::<HttpStatusError>().is_some())
    );
    assert!(matches!(
        event_receiver.try_recv().unwrap(),
        AuthEvent::IapChallengeReceived
    ));
}

#[test]
fn iap_challenge_failure_emits_no_event_when_observation_is_disabled() {
    let _request = {
        let mut server = ChannelState::mock_server();
        server
            .mock("GET", "/api/v1/agent/identities")
            .with_status(401)
            .with_header(http_client::iap::IAP_GENERATED_RESPONSE_HEADER, "true")
            .with_body(r#"{"error":"IAP challenge"}"#)
            .create()
    };
    let (base_client, event_receiver) = base_client(false);

    block_on(get_authenticated_public_api::<serde_json::Value>(
        &base_client,
        AuthToken::Bearer("token".to_string()),
        "agent/identities",
    ))
    .unwrap_err();

    assert!(event_receiver.try_recv().is_err());
}

#[test]
fn shared_status_error_actionability_ignores_retryable_client_failures() {
    let error = anyhow::Error::new(HttpStatusError {
        status: 429,
        body: "retry later".to_string(),
    })
    .context("Public API request failed");

    assert!(!error.is_actionable());
}
