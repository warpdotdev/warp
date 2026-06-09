use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use instant::Instant;
use warp_core::channel::IapConfig;

use super::*;

/// Builds a syntactically-valid JWT (`header.payload.sig`) whose payload is the
/// provided JSON. The signature is a placeholder \u2014 `parse_exp_from_jwt` only
/// decodes the payload segment.
fn jwt_with_payload(payload_json: &str) -> String {
    let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let header = b64.encode(br#"{"alg":"none"}"#);
    let payload = b64.encode(payload_json.as_bytes());
    format!("{header}.{payload}.signature")
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn test_state() -> IapState {
    IapState::new(&IapConfig {
        audiences: "iap-client-id".into(),
        service_account_email: "iap-access@example.iam.gserviceaccount.com".into(),
    })
}

fn cached(token: &str, ttl: Option<Duration>) -> CachedToken {
    // `None` produces an already-at-boundary instant, which `valid_token` treats
    // as expired once the comparison reads a slightly later `Instant::now()`.
    let expires_at = ttl.map_or_else(Instant::now, |d| Instant::now() + d);
    CachedToken {
        token: token.to_string(),
        expires_at,
    }
}

#[test]
fn parse_exp_from_jwt_reads_exp_claim() {
    let token = jwt_with_payload(r#"{"exp": 1893456000, "sub": "x"}"#);
    assert_eq!(parse_exp_from_jwt(&token), Some(1893456000));
}

#[test]
fn parse_exp_from_jwt_missing_exp_is_none() {
    let token = jwt_with_payload(r#"{"sub": "x"}"#);
    assert_eq!(parse_exp_from_jwt(&token), None);
}

#[test]
fn parse_exp_from_jwt_not_a_jwt_is_none() {
    assert_eq!(parse_exp_from_jwt("not-a-jwt"), None);
}

#[test]
fn parse_exp_from_jwt_invalid_base64_is_none() {
    assert_eq!(parse_exp_from_jwt("aaa.!!!not-base64!!!.ccc"), None);
}

#[test]
fn get_expires_at_future_exp_is_ok() {
    let token = jwt_with_payload(&format!(r#"{{"exp": {}}}"#, now_unix() + 3600));
    let expires_at = get_expires_at(&token).expect("future exp should parse");
    assert!(expires_at > Instant::now());
}

#[test]
fn get_expires_at_past_exp_errs() {
    let token = jwt_with_payload(r#"{"exp": 1}"#);
    assert!(get_expires_at(&token).is_err());
}

#[test]
fn get_expires_at_missing_exp_errs() {
    let token = jwt_with_payload(r#"{"sub": "x"}"#);
    assert!(get_expires_at(&token).is_err());
}

#[test]
fn get_cached_loaded_valid_returns_token() {
    let state = test_state();
    state.set_loaded(cached("fresh-token", Some(Duration::from_secs(60))));
    assert_eq!(state.get_cached().as_deref(), Some("fresh-token"));
}

#[test]
fn get_cached_loaded_expired_is_none() {
    let state = test_state();
    state.set_loaded(cached("stale-token", None));
    assert_eq!(state.get_cached(), None);
}

#[test]
fn get_cached_refreshing_uses_valid_previous_token() {
    let state = test_state();
    state.set_loaded(cached("prev-token", Some(Duration::from_secs(60))));
    state.set_refreshing();
    assert_eq!(state.get_cached().as_deref(), Some("prev-token"));
}

#[test]
fn get_cached_refreshing_drops_expired_previous_token() {
    let state = test_state();
    state.set_loaded(cached("prev-token", None));
    state.set_refreshing();
    assert_eq!(state.get_cached(), None);
}

#[test]
fn get_cached_failed_uses_valid_previous_token() {
    let state = test_state();
    state.set_loaded(cached("prev-token", Some(Duration::from_secs(60))));
    state.set_failed("gcloud blew up".to_string());
    assert_eq!(state.get_cached().as_deref(), Some("prev-token"));
}
