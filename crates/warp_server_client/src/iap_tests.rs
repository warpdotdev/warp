use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use futures::executor::block_on;
use instant::Instant;
use warp_core::channel::{ChannelState, IapConfig};
use warpui_core::r#async::BoxFuture;

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
fn parse_aud_from_jwt_reads_string_aud() {
    let token = jwt_with_payload(r#"{"aud": "//iam.googleapis.com/projects/1/x", "sub": "y"}"#);
    assert_eq!(
        parse_aud_from_jwt(&token).as_deref(),
        Some("//iam.googleapis.com/projects/1/x")
    );
}

#[test]
fn parse_aud_from_jwt_reads_first_array_aud() {
    let token = jwt_with_payload(r#"{"aud": ["first-aud", "second-aud"]}"#);
    assert_eq!(parse_aud_from_jwt(&token).as_deref(), Some("first-aud"));
}

#[test]
fn parse_aud_from_jwt_missing_aud_is_none() {
    let token = jwt_with_payload(r#"{"sub": "y"}"#);
    assert_eq!(parse_aud_from_jwt(&token), None);
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

#[test]
fn generate_id_token_request_uses_camel_case_include_email() {
    let value = serde_json::to_value(GenerateIdTokenRequest {
        audience: "iap-client-id",
        include_email: true,
    })
    .unwrap();
    assert_eq!(value["audience"], "iap-client-id");
    assert_eq!(value["includeEmail"], true);
}

#[test]
fn generate_id_token_response_parses_token() {
    let parsed: GenerateIdTokenResponse =
        serde_json::from_str(r#"{"token": "an-id-token"}"#).unwrap();
    assert_eq!(parsed.token, "an-id-token");
}

#[test]
fn sts_response_parses_and_ignores_extra_fields() {
    let parsed: StsTokenExchangeResponse =
        serde_json::from_str(r#"{"access_token": "federated", "expires_in": 3600}"#).unwrap();
    assert_eq!(parsed.access_token, "federated");
}

/// Records how many times it was asked to mint, so tests can assert whether the
/// injected-JWT fast path or the minter fallback was taken.
struct FakeMinter {
    calls: Arc<AtomicUsize>,
    token: String,
}

impl IapIdentityTokenMinter for FakeMinter {
    fn mint_identity_token(
        &self,
        _audience: String,
        _requested_duration: Duration,
    ) -> BoxFuture<'static, anyhow::Result<String>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let token = self.token.clone();
        Box::pin(async move { Ok(token) })
    }
}

fn fake_minter(token: &str) -> (Arc<dyn IapIdentityTokenMinter>, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let minter: Arc<dyn IapIdentityTokenMinter> = Arc::new(FakeMinter {
        calls: calls.clone(),
        token: token.to_string(),
    });
    (minter, calls)
}

fn bootstrap_jwt(aud: &str, exp: u64) -> String {
    jwt_with_payload(&format!(r#"{{"aud":"{aud}","exp":{exp}}}"#))
}

fn wif_endpoints(base: &str) -> WifEndpoints {
    WifEndpoints {
        sts_token_url: format!("{base}/v1/token"),
        iam_generate_id_token_url_template: format!(
            "{base}/v1/projects/-/serviceAccounts/{{sa_email}}:generateIdToken"
        ),
    }
}

const TEST_SA_EMAIL: &str = "iap-access@example.iam.gserviceaccount.com";

#[test]
fn resolve_wif_identity_token_prefers_valid_injected_jwt() {
    let (minter, calls) = fake_minter("freshly-minted");
    let injected = bootstrap_jwt("//iam/providers/p", now_unix() + 3600);

    let token = block_on(resolve_wif_identity_token(
        injected.clone(),
        "//iam/providers/p",
        &minter,
    ))
    .unwrap();

    assert_eq!(token, injected);
    assert_eq!(calls.load(Ordering::SeqCst), 0, "minter must not be called");
}

#[test]
fn resolve_wif_identity_token_mints_when_injected_expired() {
    let (minter, calls) = fake_minter("freshly-minted");
    let injected = bootstrap_jwt("//iam/providers/p", 1);

    let token = block_on(resolve_wif_identity_token(
        injected,
        "//iam/providers/p",
        &minter,
    ))
    .unwrap();

    assert_eq!(token, "freshly-minted");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn fetch_iap_token_via_wif_returns_token_on_success() {
    let mut server = ChannelState::mock_server();
    let base = server.url();

    let sts = server
        .mock("POST", "/v1/token")
        .with_status(200)
        .with_body(r#"{"access_token":"federated-abc"}"#)
        .create();
    let id_token = jwt_with_payload(&format!(r#"{{"exp":{}}}"#, now_unix() + 3600));
    let iam = server
        .mock(
            "POST",
            mockito::Matcher::Regex(r"/serviceAccounts/.*:generateIdToken$".to_string()),
        )
        .match_header("authorization", "Bearer federated-abc")
        .with_status(200)
        .with_body(format!(r#"{{"token":"{id_token}"}}"#))
        .create();

    let (minter, calls) = fake_minter("unused");
    let injected = bootstrap_jwt("//iam/providers/oz-oidc-staging-iap", now_unix() + 3600);
    let endpoints = wif_endpoints(&base);

    let cached = block_on(fetch_iap_token_via_wif(
        minter,
        injected,
        "iap-client-id".to_string(),
        TEST_SA_EMAIL.to_string(),
        &endpoints,
    ))
    .unwrap();

    assert_eq!(cached.token, id_token);
    assert!(cached.expires_at > Instant::now());
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "a valid injected JWT should skip the minter"
    );
    sts.assert();
    iam.assert();
}

#[test]
fn fetch_iap_token_via_wif_errors_on_sts_failure() {
    let mut server = ChannelState::mock_server();
    let base = server.url();
    let _sts = server
        .mock("POST", "/v1/token")
        .with_status(400)
        .with_body("bad subject token")
        .create();

    let (minter, _) = fake_minter("unused");
    let injected = bootstrap_jwt("//iam/providers/p", now_unix() + 3600);
    let endpoints = wif_endpoints(&base);

    let err = block_on(fetch_iap_token_via_wif(
        minter,
        injected,
        "iap-client-id".to_string(),
        TEST_SA_EMAIL.to_string(),
        &endpoints,
    ))
    .unwrap_err();

    assert!(
        err.to_string().contains("STS token exchange failed"),
        "unexpected error: {err:#}"
    );
}

#[test]
fn fetch_iap_token_via_wif_errors_on_iam_failure() {
    let mut server = ChannelState::mock_server();
    let base = server.url();
    let _sts = server
        .mock("POST", "/v1/token")
        .with_status(200)
        .with_body(r#"{"access_token":"federated-abc"}"#)
        .create();
    let _iam = server
        .mock(
            "POST",
            mockito::Matcher::Regex(r"/serviceAccounts/.*:generateIdToken$".to_string()),
        )
        .with_status(403)
        .with_body("permission denied")
        .create();

    let (minter, _) = fake_minter("unused");
    let injected = bootstrap_jwt("//iam/providers/p", now_unix() + 3600);
    let endpoints = wif_endpoints(&base);

    let err = block_on(fetch_iap_token_via_wif(
        minter,
        injected,
        "iap-client-id".to_string(),
        TEST_SA_EMAIL.to_string(),
        &endpoints,
    ))
    .unwrap_err();

    assert!(
        err.to_string().contains("generateIdToken failed"),
        "unexpected error: {err:#}"
    );
}
