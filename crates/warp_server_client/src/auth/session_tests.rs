use std::sync::Arc;

use chrono::Utc;
use futures::executor::block_on;
use warp_server_auth::auth_state::AuthState;
use warp_server_auth::credentials::{AuthToken, Credentials, LoginToken};
use warp_server_auth::user::FirebaseAuthTokens;

use super::AuthSession;

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
