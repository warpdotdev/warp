use std::sync::Arc;

use futures::executor::block_on;

use super::FactoryClient;
use crate::ChannelState;
use crate::auth::auth_state::AuthState;
use crate::server::server_api::ServerApi;
use crate::server::telemetry::TelemetryApi;

fn server_api_with_bearer_token() -> ServerApi {
    let (event_sender, _) = async_channel::unbounded();
    let auth_state = Arc::new(AuthState::new_logged_out_for_test());
    auth_state.set_remote_server_bearer_token("factory-access-token".to_string());
    ServerApi::new_with_parts(
        Arc::new(http_client::Client::new_for_test()),
        auth_state,
        event_sender,
        None,
        None,
        TelemetryApi::new(),
    )
}

fn factory_access_response(body: &str) -> anyhow::Result<bool> {
    let _request = {
        let mut server = ChannelState::mock_server();
        server
            .mock("GET", "/api/v1/factory/access")
            .match_header("authorization", "Bearer factory-access-token")
            .with_status(200)
            .with_body(body)
            .create()
    };

    block_on(server_api_with_bearer_token().has_factory_access())
}

#[test]
fn factory_access_request_uses_authenticated_endpoint_and_decodes_rollout_state() {
    assert!(factory_access_response(r#"{"allowed":true}"#).unwrap());
    assert!(!factory_access_response(r#"{"allowed":false}"#).unwrap());
}

#[test]
fn factory_access_request_rejects_malformed_response() {
    assert!(factory_access_response(r#"{"access":true}"#).is_err());
}
