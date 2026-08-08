use std::sync::Arc;

use chrono::Utc;
use futures::executor::block_on;
use mockito::Matcher;
use warp_core::channel::ChannelState;
use warp_core::execution_mode::ExecutionMode;
use warp_server_auth::auth_state::AuthState;
use warp_server_auth::credentials::{AuthToken, Credentials, LoginToken};
use warp_server_auth::user::FirebaseAuthTokens;

use super::AuthSession;

fn session_with_state(
    auth_state: Arc<AuthState>,
) -> (AuthSession, async_channel::Receiver<super::AuthEvent>) {
    session_with_state_and_oauth_client_id(auth_state, None)
}

fn session_with_state_and_oauth_client_id(
    auth_state: Arc<AuthState>,
    oauth_client_id: Option<&str>,
) -> (AuthSession, async_channel::Receiver<super::AuthEvent>) {
    let (event_sender, event_receiver) = async_channel::unbounded();
    let session = AuthSession {
        client: Arc::new(http_client::Client::new()),
        auth_state,
        event_sender,
        oauth_client: AuthSession::create_oauth_client(oauth_client_id),
    };
    (session, event_receiver)
}

#[test]
fn tui_execution_mode_uses_warp_agent_cli_oauth_client() {
    let client = AuthSession::create_oauth_client(Some(ExecutionMode::Tui.client_id()));

    assert_eq!(client.client_id().as_str(), "warp-agent-cli");
}

#[test]
fn non_tui_execution_modes_use_warp_cli_oauth_client() {
    assert_eq!(
        AuthSession::create_oauth_client(Some(ExecutionMode::Sdk.client_id()))
            .client_id()
            .as_str(),
        "warp-cli"
    );
    assert_eq!(
        AuthSession::create_oauth_client(Some(ExecutionMode::App.client_id()))
            .client_id()
            .as_str(),
        "warp-cli"
    );
    assert_eq!(
        AuthSession::create_oauth_client(Some(ExecutionMode::RemoteServerDaemon.client_id()))
            .client_id()
            .as_str(),
        "warp-cli"
    );
    assert_eq!(
        AuthSession::create_oauth_client(None).client_id().as_str(),
        "warp-cli"
    );
}

#[test]
fn bearer_credentials_are_returned_without_session_refresh_events() {
    let auth_state = Arc::new(AuthState::new_logged_out_for_test());
    auth_state.set_credentials(Some(Credentials::Bearer("daemon-token".to_string())));
    let (session, event_receiver) = session_with_state(auth_state);

    assert!(!session.allowed_to_refresh_token());
    let token = block_on(session.get_or_refresh_access_token()).unwrap();

    assert!(matches!(token, AuthToken::Bearer(token) if token == "daemon-token"));
    assert!(event_receiver.try_recv().is_err());
}

#[test]
fn unexpired_firebase_credentials_return_cached_token_without_refresh_events() {
    let auth_state = Arc::new(AuthState::new_logged_out_for_test());
    auth_state.set_credentials(Some(Credentials::Firebase(FirebaseAuthTokens {
        id_token: "cached-token".to_string(),
        refresh_token: "refresh-token".to_string(),
        expiration_time: Utc::now().fixed_offset() + chrono::Duration::hours(1),
    })));
    let (session, event_receiver) = session_with_state(auth_state);

    let token = block_on(session.get_or_refresh_access_token()).unwrap();

    assert!(matches!(token, AuthToken::Firebase(token) if token == "cached-token"));
    assert!(event_receiver.try_recv().is_err());
}

#[test]
fn api_key_exchange_defers_owner_type_until_user_properties_are_fetched() {
    let auth_state = Arc::new(AuthState::new_logged_out_for_test());
    let (session, _) = session_with_state(auth_state);

    let credentials =
        block_on(session.exchange_credentials(LoginToken::ApiKey("api-key".to_string()))).unwrap();

    assert!(matches!(
        credentials,
        Credentials::ApiKey {
            key,
            owner_type: None
        } if key == "api-key"
    ));
}

#[test]
fn device_code_request_reports_warp_agent_cli_oauth_client() {
    let auth_state = Arc::new(AuthState::new_logged_out_for_test());
    let (session, _) =
        session_with_state_and_oauth_client_id(auth_state, Some(ExecutionMode::Tui.client_id()));
    let mut server = ChannelState::mock_server();
    let request = server
        .mock("POST", "/api/v1/oauth/device/auth")
        .match_body(Matcher::UrlEncoded(
            "client_id".to_string(),
            "warp-agent-cli".to_string(),
        ))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            r#"{
                "device_code": "device-code",
                "user_code": "ABCD-EFGH",
                "verification_uri": "https://app.warp.dev/device",
                "verification_uri_complete": "https://app.warp.dev/device?user_code=ABCD-EFGH",
                "expires_in": 600,
                "interval": 5
            }"#,
        )
        .create();

    block_on(session.request_device_code()).unwrap();

    request.assert();
}
