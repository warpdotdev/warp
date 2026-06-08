use base64::Engine as _;
use chrono::TimeDelta;

use super::*;

fn jwt_with_payload(payload: serde_json::Value) -> String {
    let encoder = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let header = encoder.encode(br#"{"alg":"none"}"#);
    let payload = encoder.encode(serde_json::to_vec(&payload).unwrap());
    format!("{header}.{payload}.test-signature")
}

fn client_with_expiry(token: &str, expires_at: DateTime<Utc>) -> AuthenticatedHttpClient {
    let (refresh_hint_sender, _) = async_channel::bounded(1);
    AuthenticatedHttpClient {
        inner: reqwest::blocking::Client::new(),
        token_store: TokenStore::new(token.to_owned(), expires_at).unwrap(),
        refresh_hint_sender,
    }
}

#[test]
fn authorization_overwrites_supplied_header() {
    let client = client_with_expiry(
        "current-test-token",
        Utc::now() + TimeDelta::try_minutes(5).unwrap(),
    );
    let mut request = Request::builder()
        .header(AUTHORIZATION, "Bearer stale-test-token")
        .body(Bytes::new())
        .unwrap();

    client.authorize_request(&mut request).unwrap();

    assert_eq!(
        request.headers().get(AUTHORIZATION).unwrap(),
        "Bearer current-test-token"
    );
}

#[test]
fn expired_token_is_refused_and_supplied_header_is_removed() {
    let client = client_with_expiry(
        "expired-test-token",
        Utc::now() - TimeDelta::try_minutes(5).unwrap(),
    );
    let mut request = Request::builder()
        .header(AUTHORIZATION, "Bearer stale-test-token")
        .body(Bytes::new())
        .unwrap();

    assert!(matches!(
        client.authorize_request(&mut request),
        Err(AuthenticatedHttpError::NoValidToken)
    ));
    assert!(!request.headers().contains_key(AUTHORIZATION));
}

#[test]
fn debug_output_redacts_token() {
    let client = client_with_expiry(
        "secret-test-token",
        Utc::now() + TimeDelta::try_minutes(5).unwrap(),
    );

    let debug_output = format!("{client:?}");

    assert!(!debug_output.contains("secret-test-token"));
    assert!(debug_output.contains("expires_at"));
}

#[test]
fn authorized_request_debug_redacts_token() {
    let client = client_with_expiry(
        "secret-request-test-token",
        Utc::now() + TimeDelta::try_minutes(5).unwrap(),
    );
    let mut request = Request::builder().body(Bytes::new()).unwrap();

    client.authorize_request(&mut request).unwrap();
    let request_debug = format!("{request:?}");
    let headers_debug = format!("{:?}", request.headers());

    assert!(!request_debug.contains("secret-request-test-token"));
    assert!(!headers_debug.contains("secret-request-test-token"));
    assert!(request_debug.contains("Sensitive"));
    assert!(headers_debug.contains("Sensitive"));
}

#[test]
fn refreshed_token_run_id_exactly_matches() {
    let token = jwt_with_payload(serde_json::json!({ "run_id": "expected-run-id" }));

    validate_refreshed_token_run_id(&token, Some("expected-run-id")).unwrap();
}

#[test]
fn refreshed_token_run_id_is_required() {
    let token = jwt_with_payload(serde_json::json!({}));
    assert!(validate_refreshed_token_run_id(&token, Some("expected-run-id")).is_err());
}

#[test]
fn expected_run_id_is_required() {
    let token = jwt_with_payload(serde_json::json!({ "run_id": "expected-run-id" }));

    assert!(validate_refreshed_token_run_id(&token, None).is_err());
    assert!(validate_refreshed_token_run_id(&token, Some("")).is_err());
}

#[test]
fn refreshed_token_run_id_must_match() {
    let token = jwt_with_payload(serde_json::json!({ "run_id": "wrong-run-id" }));

    assert!(validate_refreshed_token_run_id(&token, Some("expected-run-id")).is_err());
}

#[test]
fn refreshed_token_run_id_must_be_a_string() {
    let token = jwt_with_payload(serde_json::json!({ "run_id": 123 }));
    assert!(validate_refreshed_token_run_id(&token, Some("expected-run-id")).is_err());
}

#[test]
fn malformed_refreshed_tokens_are_rejected() {
    let invalid_json = {
        let encoder = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let payload = encoder.encode(b"not-json");
        format!("header.{payload}.signature")
    };

    for token in ["not-a-jwt", "header.!!!.signature", &invalid_json] {
        assert!(validate_refreshed_token_run_id(token, Some("expected-run-id")).is_err());
    }
}

#[test]
fn rejected_refreshed_token_preserves_previous_token() {
    let token_store = TokenStore::new(
        "current-test-token".to_owned(),
        Utc::now() + TimeDelta::try_minutes(5).unwrap(),
    )
    .unwrap();
    let wrong_run_token = jwt_with_payload(serde_json::json!({ "run_id": "wrong-run-id" }));

    assert!(token_store
        .replace_refreshed(
            wrong_run_token,
            Utc::now() + TimeDelta::try_minutes(5).unwrap(),
            Some("expected-run-id"),
        )
        .is_err());
    assert_eq!(
        token_store.valid_authorization_header().unwrap(),
        "Bearer current-test-token"
    );
}
