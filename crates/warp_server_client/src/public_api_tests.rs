use std::sync::{Arc, Mutex, MutexGuard};

use futures::executor::block_on;
use warp_core::channel::ChannelState;
use warp_server_auth::auth_state::AuthState;
use warp_server_auth::credentials::AuthToken;

use super::get_authenticated_public_api;
use crate::auth::AuthEvent;
use crate::base_client::{AuthenticatedGraphqlConfig, BaseClient, GraphqlRoutingConfig};

static CHANNEL_URL_LOCK: Mutex<()> = Mutex::new(());

struct ChannelUrlOverride {
    _lock: MutexGuard<'static, ()>,
    original_url: String,
}

impl ChannelUrlOverride {
    fn new(url: String) -> Self {
        let lock = CHANNEL_URL_LOCK.lock().unwrap();
        let original_url = ChannelState::server_root_url().into_owned();
        ChannelState::override_server_root_url(url).unwrap();
        Self {
            _lock: lock,
            original_url,
        }
    }
}

impl Drop for ChannelUrlOverride {
    fn drop(&mut self) {
        ChannelState::override_server_root_url(self.original_url.clone()).unwrap();
    }
}

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
    let mut server = mockito::Server::new();
    let _channel_url = ChannelUrlOverride::new(server.url());
    let _request = server
        .mock("GET", "/api/v1/agent/identities")
        .with_status(401)
        .with_header(http_client::iap::IAP_GENERATED_RESPONSE_HEADER, "true")
        .with_body(r#"{"error":"IAP challenge"}"#)
        .create();
    let (base_client, event_receiver) = base_client(true);

    let error = block_on(get_authenticated_public_api::<serde_json::Value>(
        &base_client,
        AuthToken::Bearer("token".to_string()),
        "agent/identities",
    ))
    .unwrap_err();

    assert!(error.to_string().contains("IAP challenge"));
    assert!(matches!(
        event_receiver.try_recv().unwrap(),
        AuthEvent::IapChallengeReceived
    ));
}

#[test]
fn iap_challenge_failure_emits_no_event_when_observation_is_disabled() {
    let mut server = mockito::Server::new();
    let _channel_url = ChannelUrlOverride::new(server.url());
    let _request = server
        .mock("GET", "/api/v1/agent/identities")
        .with_status(401)
        .with_header(http_client::iap::IAP_GENERATED_RESPONSE_HEADER, "true")
        .with_body(r#"{"error":"IAP challenge"}"#)
        .create();
    let (base_client, event_receiver) = base_client(false);

    block_on(get_authenticated_public_api::<serde_json::Value>(
        &base_client,
        AuthToken::Bearer("token".to_string()),
        "agent/identities",
    ))
    .unwrap_err();

    assert!(event_receiver.try_recv().is_err());
}
