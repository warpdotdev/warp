use std::sync::Arc;

use futures::executor::block_on;
use warp_core::channel::ChannelState;
use warp_core::errors::AnyhowErrorExt as _;
use warp_server_auth::auth_state::AuthState;

use super::HttpStatusError;
use crate::auth::AuthEvent;
use crate::base_client::{
    AGENT_SOURCE_HEADER, AuthenticatedGraphqlConfig, BaseClient, CLOUD_AGENT_ID_HEADER,
    GraphqlRoutingConfig,
};
struct EmptyIapTokenProvider;

impl http_client::iap::IapTokenProvider for EmptyIapTokenProvider {
    fn cached_token(&self) -> Option<String> {
        None
    }
}

fn base_client(observe_iap_challenges: bool) -> (BaseClient, async_channel::Receiver<AuthEvent>) {
    base_client_with_auth(AuthState::new_for_test(), None, observe_iap_challenges)
}

fn base_client_with_auth(
    auth_state: AuthState,
    agent_source: Option<String>,
    observe_iap_challenges: bool,
) -> (BaseClient, async_channel::Receiver<AuthEvent>) {
    let (event_sender, event_receiver) = async_channel::unbounded();
    (
        BaseClient::new(
            Arc::new(http_client::Client::new()),
            Arc::new(auth_state),
            event_sender,
            agent_source,
            GraphqlRoutingConfig::default(),
            AuthenticatedGraphqlConfig::default(),
            observe_iap_challenges.then(|| {
                Arc::new(EmptyIapTokenProvider) as Arc<dyn http_client::iap::IapTokenProvider>
            }),
        ),
        event_receiver,
    )
}

#[test]
fn public_api_get_deserializes_successful_response() {
    let _request = {
        let mut server = ChannelState::mock_server();
        server
            .mock("GET", "/api/v1/test/success")
            .with_status(200)
            .with_body(r#"{"value":"success"}"#)
            .create()
    };
    let (base_client, _) = base_client(false);

    let response =
        block_on(base_client.get_public_api::<serde_json::Value>("test/success")).unwrap();

    assert_eq!(response, serde_json::json!({ "value": "success" }));
}

#[test]
fn public_api_get_sends_bearer_auth() {
    let _request = {
        let mut server = ChannelState::mock_server();
        server
            .mock("GET", "/api/v1/test/bearer-auth")
            .match_header("authorization", "Bearer bearer-token")
            .with_status(200)
            .with_body(r#"{"value":"success"}"#)
            .create()
    };
    let auth_state = AuthState::new_logged_out_for_test();
    auth_state.set_remote_server_bearer_token("bearer-token".to_string());
    let (base_client, _) = base_client_with_auth(auth_state, None, false);

    block_on(base_client.get_public_api::<serde_json::Value>("test/bearer-auth")).unwrap();
}

#[test]
fn public_api_get_inherits_ambient_headers() {
    let _request = {
        let mut server = ChannelState::mock_server();
        server
            .mock("GET", "/api/v1/test/ambient-headers")
            .match_header(CLOUD_AGENT_ID_HEADER, "ambient-task")
            .match_header(AGENT_SOURCE_HEADER, "cloud-mode")
            .with_status(200)
            .with_body(r#"{"value":"success"}"#)
            .create()
    };
    let (base_client, _) = base_client_with_auth(
        AuthState::new_for_test(),
        Some("cloud-mode".to_string()),
        false,
    );
    base_client.set_ambient_agent_task_id(Some("ambient-task".to_string()));

    block_on(base_client.get_public_api::<serde_json::Value>("test/ambient-headers")).unwrap();
}

#[test]
fn ordinary_public_api_failure_preserves_shared_status_error() {
    let _request = {
        let mut server = ChannelState::mock_server();
        server
            .mock("GET", "/api/v1/test/failure")
            .with_status(500)
            .with_body(r#"{"error":"request failed"}"#)
            .create()
    };
    let (base_client, event_receiver) = base_client(false);

    let error =
        block_on(base_client.get_public_api::<serde_json::Value>("test/failure")).unwrap_err();

    assert!(error.to_string().contains("request failed"));
    assert!(
        error
            .chain()
            .any(|cause| cause.downcast_ref::<HttpStatusError>().is_some())
    );
    assert!(event_receiver.try_recv().is_err());
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

    let error =
        block_on(base_client.get_public_api::<serde_json::Value>("agent/identities")).unwrap_err();

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

    block_on(base_client.get_public_api::<serde_json::Value>("agent/identities")).unwrap_err();

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
