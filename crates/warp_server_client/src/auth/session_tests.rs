use std::sync::Arc;

use chrono::Utc;
use futures::executor::block_on;
use warp_server_auth::auth_state::AuthState;
use warp_server_auth::credentials::{AuthToken, Credentials, LoginToken};
use warp_server_auth::user::FirebaseAuthTokens;

use super::{AuthSession, UserAuthenticationError};

fn session_with_state(
    auth_state: Arc<AuthState>,
) -> (AuthSession, async_channel::Receiver<super::AuthEvent>) {
    let (event_sender, event_receiver) = async_channel::unbounded();
    let session = AuthSession::new(
        Arc::new(http_client::Client::new()),
        auth_state,
        event_sender,
    );
    (session, event_receiver)
}

#[test]
fn device_authorization_uses_warp_agent_cli_client() {
    let client = AuthSession::create_oauth_client();

    assert_eq!(client.client_id().as_str(), "warp-agent-cli");
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

fn fetch_access_token_response(
    status: usize,
    body: &str,
) -> Result<FirebaseAuthTokens, UserAuthenticationError> {
    let mut server = mockito::Server::new();
    let response = server
        .mock("POST", "/token")
        .with_status(status)
        .with_body(body)
        .create();
    let client = http_client::Client::new();

    let result = block_on(AuthSession::fetch_access_token(
        client.post(format!("{}/token", server.url())),
    ));

    response.assert();
    result
}

#[test]
fn rejected_refresh_token_is_classified_from_error_body() {
    let result = fetch_access_token_response(
        400,
        r#"{"error":{"code":400,"message":"INVALID_REFRESH_TOKEN"}}"#,
    );

    assert!(matches!(
        result,
        Err(UserAuthenticationError::DeniedAccessToken(error))
            if error.message == "INVALID_REFRESH_TOKEN"
    ));
}

#[test]
fn disabled_account_is_classified_from_error_body() {
    let result =
        fetch_access_token_response(400, r#"{"error":{"code":400,"message":"USER_DISABLED"}}"#);

    assert!(matches!(
        result,
        Err(UserAuthenticationError::UserAccountDisabled(error))
            if error.message == "USER_DISABLED"
    ));
}

#[test]
fn unavailable_firebase_response_is_not_a_terminal_verdict() {
    let result =
        fetch_access_token_response(503, r#"{"error":{"code":503,"message":"UNAVAILABLE"}}"#);

    assert!(matches!(
        result,
        Err(UserAuthenticationError::Unexpected(_))
    ));
}

#[test]
fn firebase_quota_response_is_not_a_terminal_verdict() {
    let result = fetch_access_token_response(
        429,
        r#"{"error":{"code":429,"message":"TOO_MANY_ATTEMPTS_TRY_LATER"}}"#,
    );

    assert!(matches!(
        result,
        Err(UserAuthenticationError::Unexpected(_))
    ));
}

#[test]
fn non_firebase_response_is_not_a_terminal_verdict() {
    let result = fetch_access_token_response(429, "<html>Too many requests</html>");

    assert!(matches!(
        result,
        Err(UserAuthenticationError::Unexpected(_))
    ));
}

#[test]
fn successful_firebase_response_returns_tokens() {
    let tokens = fetch_access_token_response(
        200,
        r#"{"id_token":"new-id-token","refresh_token":"new-refresh-token","expires_in":"3600"}"#,
    )
    .unwrap();

    assert_eq!(tokens.id_token, "new-id-token");
    assert_eq!(tokens.refresh_token, "new-refresh-token");
}
