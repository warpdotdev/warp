use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::Result;
use async_trait::async_trait;
use futures::executor::block_on;
use instant::Duration;
use serde::Deserialize;
use warp_core::channel::ChannelState;
use warp_graphql::client::RequestOptions;
use warp_server_auth::credentials::AuthToken;

use super::{HttpStatusError, get_authenticated_public_api};
use crate::base_client::BaseClient;

static CHANNEL_STATE_LOCK: Mutex<()> = Mutex::new(());

struct FakeBaseClient {
    client: Arc<http_client::Client>,
    failure_count: AtomicUsize,
}

impl FakeBaseClient {
    fn new() -> Self {
        Self {
            client: Arc::new(http_client::Client::new()),
            failure_count: AtomicUsize::new(0),
        }
    }
}

#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
impl BaseClient for FakeBaseClient {
    fn http_client(&self) -> Arc<http_client::Client> {
        self.client.clone()
    }

    fn anonymous_id(&self) -> String {
        String::new()
    }

    fn unauthenticated_graphql_request_options(&self) -> RequestOptions {
        RequestOptions::default()
    }

    async fn graphql_request_options(&self, _timeout: Option<Duration>) -> Result<RequestOptions> {
        Ok(RequestOptions::default())
    }

    async fn authenticated_public_api_request_headers(&self) -> Result<Vec<(String, String)>> {
        Ok(vec![(
            "X-Warp-Test-Context".to_string(),
            "ambient".to_string(),
        )])
    }

    fn on_authenticated_public_api_failure(&self, _response: &http_client::Response) {
        self.failure_count.fetch_add(1, Ordering::SeqCst);
    }

    async fn get_or_create_ambient_workload_token(&self) -> Result<Option<String>> {
        Ok(None)
    }

    fn is_auth_refresh_allowed(&self) -> bool {
        true
    }

    fn on_graphql_staging_access_blocked(&self) {}

    fn on_graphql_iap_challenge_received(&self) {}

    fn on_graphql_user_account_disabled(&self) {}
}

#[derive(Debug, Deserialize)]
struct IdentityResponse {
    agents: Vec<String>,
}

#[test]
fn authenticated_get_sends_token_and_app_headers_then_deserializes_response() {
    let _guard = CHANNEL_STATE_LOCK.lock().unwrap();
    let mut server = mockito::Server::new();
    ChannelState::override_server_root_url(server.url()).unwrap();
    let request = server
        .mock("GET", "/api/v1/agent/identities")
        .match_header("authorization", "Bearer daemon-token")
        .match_header("x-warp-test-context", "ambient")
        .with_status(200)
        .with_body(r#"{"agents":["oz"]}"#)
        .create();
    let base_client = FakeBaseClient::new();

    let response: IdentityResponse = block_on(get_authenticated_public_api(
        &base_client,
        AuthToken::Bearer("daemon-token".to_string()),
        "agent/identities",
    ))
    .unwrap();

    request.assert();
    assert_eq!(response.agents, vec!["oz"]);
    assert_eq!(base_client.failure_count.load(Ordering::SeqCst), 0);
}

#[test]
fn authenticated_get_notifies_app_and_retains_typed_status_error_on_failure() {
    let _guard = CHANNEL_STATE_LOCK.lock().unwrap();
    let mut server = mockito::Server::new();
    ChannelState::override_server_root_url(server.url()).unwrap();
    let request = server
        .mock("GET", "/api/v1/agent/identities")
        .with_status(403)
        .with_body(r#"{"error":"forbidden"}"#)
        .create();
    let base_client = FakeBaseClient::new();

    let error = block_on(get_authenticated_public_api::<IdentityResponse>(
        &base_client,
        AuthToken::NoAuth,
        "agent/identities",
    ))
    .unwrap_err();

    request.assert();
    assert_eq!(error.to_string(), "forbidden");
    assert_eq!(base_client.failure_count.load(Ordering::SeqCst), 1);
    assert!(error.chain().any(|cause| {
        cause
            .downcast_ref::<HttpStatusError>()
            .is_some_and(|error| error.status == 403)
    }));
}
