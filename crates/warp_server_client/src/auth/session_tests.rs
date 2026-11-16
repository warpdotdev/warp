use std::sync::{Arc, Barrier};

use chrono::Utc;
use futures::executor::block_on;
use futures::future::{AbortHandle, Abortable, join_all};
use mockito::Matcher;
use warp_core::channel::ChannelState;
use warp_errors::AnyhowErrorExt as _;
use warp_server_auth::auth_state::AuthState;
use warp_server_auth::credentials::{
    AuthToken, Credentials, FirebaseToken, LoginToken, RefreshToken,
};
use warp_server_auth::user::FirebaseAuthTokens;

use super::{AuthEvent, AuthSession, UserAuthenticationError, parse_retry_after};

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

fn expired_firebase_credentials(refresh_token: &str) -> Credentials {
    Credentials::Firebase(FirebaseAuthTokens {
        id_token: "expired-token".to_string(),
        refresh_token: refresh_token.to_string(),
        expiration_time: Utc::now().fixed_offset() - chrono::Duration::hours(1),
    })
}

fn session_with_refresh_urls(
    auth_state: Arc<AuthState>,
    direct_url: String,
    proxy_url: String,
) -> (AuthSession, async_channel::Receiver<AuthEvent>) {
    let (mut session, event_receiver) = session_with_state(auth_state);
    session.refresh_urls = Some((direct_url, proxy_url));
    (session, event_receiver)
}

fn successful_refresh_response(id_token: &str, refresh_token: &str) -> String {
    format!(r#"{{"id_token":"{id_token}","refresh_token":"{refresh_token}","expires_in":"3600"}}"#)
}
#[test]
fn retry_after_http_date_uses_remaining_delay() {
    let now = chrono::DateTime::parse_from_rfc3339("2026-09-20T17:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);

    let delay = parse_retry_after("Sun, 20 Sep 2026 17:02:00 GMT", now);

    assert_eq!(delay, Some(instant::Duration::from_secs(120)));
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

#[test]
fn logged_out_session_does_not_request_refresh() {
    let (direct_url, proxy_url, direct_request, proxy_request) = {
        let mut server = ChannelState::mock_server();
        let direct_path = "/firebase/logged-out";
        let proxy_path = "/firebase/logged-out-proxy";
        let direct_request = server
            .mock("POST", direct_path)
            .with_status(200)
            .expect(0)
            .create();
        let proxy_request = server
            .mock("POST", proxy_path)
            .with_status(200)
            .expect(0)
            .create();
        (
            format!("{}{direct_path}", server.url()),
            format!("{}{proxy_path}", server.url()),
            direct_request,
            proxy_request,
        )
    };
    let auth_state = Arc::new(AuthState::new_logged_out_for_test());
    let (session, event_receiver) = session_with_refresh_urls(auth_state, direct_url, proxy_url);

    let error = block_on(session.get_or_refresh_access_token()).unwrap_err();

    assert_eq!(error.to_string(), "missing authentication credentials");
    assert!(event_receiver.try_recv().is_err());
    direct_request.assert();
    proxy_request.assert();
}

#[test]
fn direct_firebase_400_does_not_use_proxy() {
    let (direct_url, proxy_url, direct_request, proxy_request) = {
        let mut server = ChannelState::mock_server();
        let direct_path = "/firebase/direct-400";
        let proxy_path = "/firebase/direct-400-proxy";
        let direct_request = server
            .mock("POST", direct_path)
            .with_status(400)
            .with_body(r#"{"error":{"code":400,"message":"INVALID_REFRESH_TOKEN"}}"#)
            .expect(1)
            .create();
        let proxy_request = server
            .mock("POST", proxy_path)
            .with_status(200)
            .with_body(successful_refresh_response("unexpected", "unexpected"))
            .expect(0)
            .create();
        (
            format!("{}{direct_path}", server.url()),
            format!("{}{proxy_path}", server.url()),
            direct_request,
            proxy_request,
        )
    };
    let auth_state = Arc::new(AuthState::new_logged_out_for_test());
    auth_state.set_credentials(Some(expired_firebase_credentials("invalid-refresh")));
    let (session, _) = session_with_refresh_urls(auth_state, direct_url, proxy_url);

    let result = block_on(session.get_or_refresh_access_token());

    assert!(result.is_err());
    direct_request.assert();
    proxy_request.assert();
}

#[test]
fn terminal_firebase_verdict_takes_precedence_over_http_status() {
    let (direct_url, proxy_url, direct_request, proxy_request) = {
        let mut server = ChannelState::mock_server();
        let direct_path = "/firebase/direct-500";
        let proxy_path = "/firebase/direct-500-proxy";
        let direct_request = server
            .mock("POST", direct_path)
            .with_status(503)
            .with_body(r#"{"error":{"code":400,"message":"INVALID_REFRESH_TOKEN"}}"#)
            .expect(1)
            .create();
        let proxy_request = server
            .mock("POST", proxy_path)
            .with_status(200)
            .expect(0)
            .create();
        (
            format!("{}{direct_path}", server.url()),
            format!("{}{proxy_path}", server.url()),
            direct_request,
            proxy_request,
        )
    };
    let auth_state = Arc::new(AuthState::new_logged_out_for_test());
    auth_state.set_credentials(Some(expired_firebase_credentials("refresh-token")));
    let (session, event_receiver) = session_with_refresh_urls(auth_state, direct_url, proxy_url);

    let error = block_on(session.get_or_refresh_access_token()).unwrap_err();
    assert!(matches!(
        error.downcast_ref::<UserAuthenticationError>(),
        Some(UserAuthenticationError::DeniedAccessToken(_))
    ));
    assert!(matches!(
        event_receiver.try_recv().unwrap(),
        AuthEvent::NeedsReauth
    ));
    direct_request.assert();
    proxy_request.assert();
}

#[test]
fn middlebox_html_response_falls_back_to_proxy() {
    let (direct_url, proxy_url, direct_request, proxy_request) = {
        let mut server = ChannelState::mock_server();
        let direct_path = "/firebase/direct-invalid-json";
        let proxy_path = "/firebase/direct-invalid-json-proxy";
        let direct_request = server
            .mock("POST", direct_path)
            .with_status(403)
            .with_body("<html>Access blocked</html>")
            .expect(1)
            .create();
        let proxy_request = server
            .mock("POST", proxy_path)
            .with_status(200)
            .with_body(successful_refresh_response("proxy-id", "proxy-refresh"))
            .expect(1)
            .create();
        (
            format!("{}{direct_path}", server.url()),
            format!("{}{proxy_path}", server.url()),
            direct_request,
            proxy_request,
        )
    };
    let auth_state = Arc::new(AuthState::new_logged_out_for_test());
    auth_state.set_credentials(Some(expired_firebase_credentials("refresh-token")));
    let (session, event_receiver) = session_with_refresh_urls(auth_state, direct_url, proxy_url);

    let token = block_on(session.get_or_refresh_access_token()).unwrap();

    assert!(matches!(token, AuthToken::Firebase(token) if token == "proxy-id"));
    assert!(matches!(
        event_receiver.try_recv().unwrap(),
        AuthEvent::AccessTokenRefreshed { .. }
    ));
    direct_request.assert();
    proxy_request.assert();
}

#[test]
fn proxy_429_starts_shared_cooldown() {
    let (proxy_url, proxy_request) = {
        let mut server = ChannelState::mock_server();
        let proxy_path = "/firebase/proxy-429";
        let request = server
            .mock("POST", proxy_path)
            .with_status(429)
            .with_header("retry-after", "120")
            .expect(1)
            .create();
        (format!("{}{proxy_path}", server.url()), request)
    };
    let auth_state = Arc::new(AuthState::new_logged_out_for_test());
    auth_state.set_credentials(Some(expired_firebase_credentials("refresh-token")));
    let (session, _) = session_with_refresh_urls(auth_state, "http://".to_string(), proxy_url);

    let first = block_on(session.get_or_refresh_access_token());
    let second = block_on(session.get_or_refresh_access_token());

    assert!(first.is_err());
    assert!(second.is_err());
    proxy_request.assert();
}

#[test]
fn waiting_refresh_caller_observes_cooldown_after_failure() {
    let (proxy_url, proxy_request) = {
        let mut server = ChannelState::mock_server();
        let proxy_path = "/firebase/concurrent-proxy-429";
        let request = server
            .mock("POST", proxy_path)
            .with_status(429)
            .expect(1)
            .create();
        (format!("{}{proxy_path}", server.url()), request)
    };
    let auth_state = Arc::new(AuthState::new_logged_out_for_test());
    auth_state.set_credentials(Some(expired_firebase_credentials("refresh-token")));
    let (mut session, _) = session_with_refresh_urls(auth_state, "http://".to_string(), proxy_url);
    let result_ready = Arc::new(Barrier::new(2));
    let allow_finalization = Arc::new(Barrier::new(2));
    session.refresh_result_barriers = Some((result_ready.clone(), allow_finalization.clone()));
    let session = Arc::new(session);
    let leader = {
        let session = session.clone();
        std::thread::spawn(move || block_on(session.get_or_refresh_access_token()))
    };
    result_ready.wait();
    let late_caller = {
        let session = session.clone();
        std::thread::spawn(move || block_on(session.get_or_refresh_access_token()))
    };
    allow_finalization.wait();
    let leader_error = leader.join().unwrap().unwrap_err();
    let late_error = late_caller.join().unwrap().unwrap_err();

    assert_eq!(late_error.to_string(), leader_error.to_string());
    assert!(!late_error.is_actionable());
    assert!(!leader_error.is_actionable());
    proxy_request.assert();
}

#[test]
fn terminal_refresh_failure_suppresses_later_requests() {
    let (direct_url, proxy_url, direct_request) = {
        let mut server = ChannelState::mock_server();
        let direct_path = "/firebase/terminal";
        let proxy_path = "/firebase/terminal-proxy";
        let direct_request = server
            .mock("POST", direct_path)
            .with_status(400)
            .with_body(r#"{"error":{"code":400,"message":"TOKEN_EXPIRED"}}"#)
            .expect(1)
            .create();
        (
            format!("{}{direct_path}", server.url()),
            format!("{}{proxy_path}", server.url()),
            direct_request,
        )
    };
    let auth_state = Arc::new(AuthState::new_logged_out_for_test());
    auth_state.set_credentials(Some(expired_firebase_credentials("expired-refresh")));
    let (session, event_receiver) = session_with_refresh_urls(auth_state, direct_url, proxy_url);

    let first = block_on(session.get_or_refresh_access_token());
    let second = block_on(session.get_or_refresh_access_token());

    assert!(first.is_err());
    assert!(second.is_err());
    direct_request.assert();
    assert!(matches!(
        event_receiver.try_recv().unwrap(),
        AuthEvent::NeedsReauth
    ));
    assert!(event_receiver.try_recv().is_err());
}

#[test]
fn changed_refresh_credentials_clear_terminal_suppression() {
    let (direct_url, proxy_url, invalid_request, replacement_request) = {
        let mut server = ChannelState::mock_server();
        let direct_path = "/firebase/replaced-credentials";
        let proxy_path = "/firebase/replaced-credentials-proxy";
        let invalid_request = server
            .mock("POST", direct_path)
            .match_body(Matcher::UrlEncoded(
                "refresh_token".to_string(),
                "invalid-refresh".to_string(),
            ))
            .with_status(400)
            .with_body(r#"{"error":{"code":400,"message":"INVALID_REFRESH_TOKEN"}}"#)
            .expect(1)
            .create();
        let replacement_request = server
            .mock("POST", direct_path)
            .match_body(Matcher::UrlEncoded(
                "refresh_token".to_string(),
                "replacement-refresh".to_string(),
            ))
            .with_status(200)
            .with_body(successful_refresh_response(
                "replacement-id",
                "rotated-refresh",
            ))
            .expect(1)
            .create();
        (
            format!("{}{direct_path}", server.url()),
            format!("{}{proxy_path}", server.url()),
            invalid_request,
            replacement_request,
        )
    };
    let auth_state = Arc::new(AuthState::new_logged_out_for_test());
    auth_state.set_credentials(Some(expired_firebase_credentials("invalid-refresh")));
    let (session, _) = session_with_refresh_urls(auth_state.clone(), direct_url, proxy_url);

    assert!(block_on(session.get_or_refresh_access_token()).is_err());
    auth_state.set_credentials(Some(expired_firebase_credentials("replacement-refresh")));
    let token = block_on(session.get_or_refresh_access_token()).unwrap();

    assert!(matches!(token, AuthToken::Firebase(token) if token == "replacement-id"));
    invalid_request.assert();
    replacement_request.assert();
}

#[test]
fn successful_refresh_clears_transient_backoff() {
    let (direct_url, proxy_url, direct_request) = {
        let mut server = ChannelState::mock_server();
        let direct_path = "/firebase/success-reset";
        let proxy_path = "/firebase/success-reset-proxy";
        let direct_request = server
            .mock("POST", direct_path)
            .with_status(200)
            .with_body(successful_refresh_response("fresh-id", "fresh-refresh"))
            .expect(1)
            .create();
        (
            format!("{}{direct_path}", server.url()),
            format!("{}{proxy_path}", server.url()),
            direct_request,
        )
    };
    let auth_state = Arc::new(AuthState::new_logged_out_for_test());
    auth_state.set_credentials(Some(expired_firebase_credentials("refresh-token")));
    let (session, _) = session_with_refresh_urls(auth_state, direct_url, proxy_url);
    {
        let mut state = session.refresh_state.lock();
        state.update_credentials("refresh-token");
        state.record_transient_failure(
            instant::Instant::now() - instant::Duration::from_secs(60),
            Some(instant::Duration::from_secs(1)),
        );
    }

    let token = block_on(session.get_or_refresh_access_token()).unwrap();

    assert!(matches!(token, AuthToken::Firebase(token) if token == "fresh-id"));
    let state = session.refresh_state.lock();
    assert!(state.retry_at.is_none());
    assert_eq!(state.consecutive_transient_failures, 0);
    direct_request.assert();
}

#[test]
fn full_event_channel_eventually_delivers_needs_reauth() {
    let (direct_url, proxy_url, direct_request) = {
        let mut server = ChannelState::mock_server();
        let direct_path = "/firebase/full-event-channel";
        let proxy_path = "/firebase/full-event-channel-proxy";
        let direct_request = server
            .mock("POST", direct_path)
            .with_status(400)
            .with_body(r#"{"error":{"code":400,"message":"INVALID_REFRESH_TOKEN"}}"#)
            .expect(1)
            .create();
        (
            format!("{}{direct_path}", server.url()),
            format!("{}{proxy_path}", server.url()),
            direct_request,
        )
    };
    let auth_state = Arc::new(AuthState::new_logged_out_for_test());
    auth_state.set_credentials(Some(expired_firebase_credentials("refresh-token")));
    let (event_sender, event_receiver) = async_channel::bounded(1);
    event_sender
        .try_send(AuthEvent::IapChallengeReceived)
        .unwrap();
    let mut session = AuthSession::new(
        Arc::new(http_client::Client::new()),
        auth_state,
        event_sender,
    );
    session.refresh_urls = Some((direct_url, proxy_url));
    let event_ready = Arc::new(Barrier::new(2));
    session.refresh_event_barrier = Some(event_ready.clone());
    let session = Arc::new(session);
    let refresh = {
        let session = session.clone();
        std::thread::spawn(move || block_on(session.get_or_refresh_access_token()))
    };
    event_ready.wait();
    assert!(matches!(
        event_receiver.try_recv().unwrap(),
        AuthEvent::IapChallengeReceived
    ));
    assert!(refresh.join().unwrap().is_err());
    assert!(matches!(
        event_receiver.try_recv().unwrap(),
        AuthEvent::NeedsReauth
    ));
    direct_request.assert();
}

#[test]
fn cancelled_reauth_delivery_is_retried_by_later_caller() {
    let (direct_url, proxy_url, direct_request) = {
        let mut server = ChannelState::mock_server();
        let direct_path = "/firebase/cancelled-reauth-delivery";
        let proxy_path = "/firebase/cancelled-reauth-delivery-proxy";
        let direct_request = server
            .mock("POST", direct_path)
            .with_status(400)
            .with_body(r#"{"error":{"code":400,"message":"INVALID_REFRESH_TOKEN"}}"#)
            .expect(1)
            .create();
        (
            format!("{}{direct_path}", server.url()),
            format!("{}{proxy_path}", server.url()),
            direct_request,
        )
    };
    let auth_state = Arc::new(AuthState::new_logged_out_for_test());
    auth_state.set_credentials(Some(expired_firebase_credentials("refresh-token")));
    let (event_sender, event_receiver) = async_channel::bounded(1);
    event_sender
        .try_send(AuthEvent::IapChallengeReceived)
        .unwrap();
    let mut session = AuthSession::new(
        Arc::new(http_client::Client::new()),
        auth_state,
        event_sender,
    );
    session.refresh_urls = Some((direct_url, proxy_url));
    let event_ready = Arc::new(Barrier::new(2));
    session.refresh_event_barrier = Some(event_ready.clone());
    let session = Arc::new(session);
    let (abort_handle, abort_registration) = AbortHandle::new_pair();
    let cancelled_refresh = {
        let session = session.clone();
        std::thread::spawn(move || {
            block_on(Abortable::new(
                session.get_or_refresh_access_token(),
                abort_registration,
            ))
        })
    };
    event_ready.wait();
    abort_handle.abort();
    assert!(cancelled_refresh.join().unwrap().is_err());
    assert!(matches!(
        event_receiver.try_recv().unwrap(),
        AuthEvent::IapChallengeReceived
    ));

    let later_refresh = {
        let session = session.clone();
        std::thread::spawn(move || block_on(session.get_or_refresh_access_token()))
    };
    event_ready.wait();
    assert!(later_refresh.join().unwrap().is_err());
    assert!(matches!(
        event_receiver.try_recv().unwrap(),
        AuthEvent::NeedsReauth
    ));
    direct_request.assert();
}

#[test]
fn concurrent_refresh_demand_sends_one_request() {
    let (direct_url, proxy_url, direct_request) = {
        let mut server = ChannelState::mock_server();
        let direct_path = "/firebase/singleflight";
        let proxy_path = "/firebase/singleflight-proxy";
        let direct_request = server
            .mock("POST", direct_path)
            .with_status(200)
            .with_body(successful_refresh_response(
                "singleflight-id",
                "rotated-refresh",
            ))
            .expect(1)
            .create();
        (
            format!("{}{direct_path}", server.url()),
            format!("{}{proxy_path}", server.url()),
            direct_request,
        )
    };
    let auth_state = Arc::new(AuthState::new_logged_out_for_test());
    auth_state.set_credentials(Some(expired_firebase_credentials("refresh-token")));
    let (session, event_receiver) = session_with_refresh_urls(auth_state, direct_url, proxy_url);

    let results = block_on(join_all(
        (0..8).map(|_| session.get_or_refresh_access_token()),
    ));

    assert!(results.into_iter().all(
        |result| matches!(result, Ok(AuthToken::Firebase(token)) if token == "singleflight-id")
    ));
    direct_request.assert();
    assert!(matches!(
        event_receiver.try_recv().unwrap(),
        AuthEvent::AccessTokenRefreshed { .. }
    ));
    assert!(event_receiver.try_recv().is_err());
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
    ))
    .map_err(super::TokenRefreshError::into_user_authentication_error);

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

fn exchange_refresh_response(
    status: usize,
    body: &str,
    proxy_requests: usize,
) -> Result<Credentials, UserAuthenticationError> {
    let mut server = mockito::Server::new();
    let direct_request = server
        .mock("POST", "/direct")
        .with_status(status)
        .with_body(body)
        .expect(1)
        .create();
    let proxy_request = server
        .mock("POST", "/proxy")
        .with_status(200)
        .with_body(successful_refresh_response("proxy-id", "proxy-refresh"))
        .expect(proxy_requests)
        .create();
    let (session, _) = session_with_refresh_urls(
        Arc::new(AuthState::new_logged_out_for_test()),
        format!("{}/direct", server.url()),
        format!("{}/proxy", server.url()),
    );

    let result = block_on(session.exchange_credentials(LoginToken::Firebase(
        FirebaseToken::Refresh(RefreshToken::new("refresh-token".to_string())),
    )));

    direct_request.assert();
    proxy_request.assert();
    result
}

#[test]
fn unavailable_firebase_response_falls_back_to_proxy() {
    let result =
        exchange_refresh_response(503, r#"{"error":{"code":503,"message":"UNAVAILABLE"}}"#, 1);

    assert!(matches!(result, Ok(Credentials::Firebase(tokens)) if tokens.id_token == "proxy-id"));
}

#[test]
fn firebase_quota_response_falls_back_to_proxy() {
    let result = exchange_refresh_response(
        429,
        r#"{"error":{"code":429,"message":"TOO_MANY_ATTEMPTS_TRY_LATER"}}"#,
        1,
    );

    assert!(matches!(result, Ok(Credentials::Firebase(tokens)) if tokens.id_token == "proxy-id"));
}

#[test]
fn disabled_account_verdict_does_not_use_proxy() {
    let result = exchange_refresh_response(
        400,
        r#"{"error":{"code":400,"message":"USER_DISABLED"}}"#,
        0,
    );

    assert!(matches!(
        result,
        Err(UserAuthenticationError::UserAccountDisabled(_))
    ));
}

#[test]
fn successful_http_response_without_firebase_payload_falls_back_to_proxy() {
    let result = exchange_refresh_response(200, "not-json", 1);

    assert!(matches!(result, Ok(Credentials::Firebase(tokens)) if tokens.id_token == "proxy-id"));
}

#[test]
fn refresh_result_does_not_overwrite_replaced_credentials() {
    let mut server = mockito::Server::new();
    let direct_request = server
        .mock("POST", "/direct")
        .with_status(200)
        .with_body(successful_refresh_response("stale-id", "stale-refresh"))
        .expect(1)
        .create();
    let auth_state = Arc::new(AuthState::new_logged_out_for_test());
    auth_state.set_credentials(Some(expired_firebase_credentials("old-refresh")));
    let (mut session, event_receiver) = session_with_refresh_urls(
        auth_state.clone(),
        format!("{}/direct", server.url()),
        format!("{}/proxy", server.url()),
    );
    let result_ready = Arc::new(Barrier::new(2));
    let allow_finalization = Arc::new(Barrier::new(2));
    session.refresh_result_barriers = Some((result_ready.clone(), allow_finalization.clone()));
    let session = Arc::new(session);
    let refresh = std::thread::spawn(move || block_on(session.get_or_refresh_access_token()));
    result_ready.wait();

    auth_state.set_credentials(Some(expired_firebase_credentials("replacement-refresh")));
    allow_finalization.wait();

    assert!(refresh.join().unwrap().is_err());
    assert!(matches!(
        auth_state.credentials(),
        Some(Credentials::Firebase(tokens)) if tokens.refresh_token == "replacement-refresh"
    ));
    assert!(event_receiver.try_recv().is_err());
    direct_request.assert();
}
