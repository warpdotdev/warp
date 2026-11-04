use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use futures::executor::block_on;
use instant::Instant;
use warp_core::channel::{ChannelState, IapConfig};
use warpui_core::App;
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

/// Builds an `IapManager` directly (bypassing `IapManager::new`) so tests can
/// exercise `ensure_access` / `apply_refresh_result` without the
/// constructor's `start_refresh` side effect kicking off a real gcloud/WIF
/// fetch.
fn test_manager() -> IapManager {
    IapManager {
        state: Some(Arc::new(test_state())),
        path_resolver: Box::new(
            |_ctx: &mut AppContext| -> BoxFuture<'static, Option<String>> {
                Box::pin(async { None })
            },
        ),
        managed_mint: None,
        consecutive_failures: 0,
        access_gate: None,
        next_access_gate_id: 0,
    }
}

fn set_env_var(name: &str, value: &str) -> Option<std::ffi::OsString> {
    let previous = std::env::var_os(name);
    // Safety: tests that mutate process environment are marked `#[serial]` so
    // we do not race with other environment readers/writers in this crate.
    unsafe { std::env::set_var(name, value) };
    previous
}

fn restore_env_var(name: &str, previous: Option<std::ffi::OsString>) {
    match previous {
        // Safety: see `set_env_var`.
        Some(value) => unsafe { std::env::set_var(name, value) },
        // Safety: see `set_env_var`.
        None => unsafe { std::env::remove_var(name) },
    }
}

#[test]
fn non_retryable_failure_fails_access_gate_immediately_without_retry() {
    App::test((), |mut app| async move {
        let model = app.add_singleton_model(|_ctx| test_manager());

        let access_unavailable_messages: Rc<RefCell<Vec<String>>> = Rc::default();
        {
            let access_unavailable_messages = access_unavailable_messages.clone();
            app.update(|ctx| {
                ctx.subscribe_to_model(&model, move |_, event, _ctx| {
                    if let IapManagerEvent::AccessUnavailable { message } = event {
                        access_unavailable_messages
                            .borrow_mut()
                            .push(message.clone());
                    }
                });
            });
        }

        model.update(&mut app, |manager, ctx| {
            // Pre-mark the shared state as already refreshing so `ensure_access`'s
            // internal `start_refresh` call is a no-op instead of spawning a real
            // gcloud/WIF fetch; the test drives completion itself via
            // `apply_refresh_result` below, exactly as the real fetch would on
            // finishing.
            manager.state.as_ref().unwrap().set_refreshing();
            manager.ensure_access(ctx);
        });

        model.update(&mut app, |manager, ctx| {
            manager.apply_refresh_result(
                Err(anyhow::Error::new(NonRetryableIapError(
                    "WARP_STAGING_IAP_BOOTSTRAP_JWT is unset; cannot mint an IAP token via WIF"
                        .to_string(),
                ))),
                ctx,
            );
        });

        assert_eq!(
            access_unavailable_messages.borrow().as_slice(),
            ["WARP_STAGING_IAP_BOOTSTRAP_JWT is unset; cannot mint an IAP token via WIF"],
            "a non-retryable failure should fail the access gate immediately with its cause"
        );
        model.read(&app, |manager, _| {
            assert!(
                manager.access_gate.is_none(),
                "the access gate should be resolved, not left waiting for the timeout"
            );
            assert_eq!(
                manager.consecutive_failures, 0,
                "a non-retryable failure must not burn the failure-retry budget"
            );
        });
    });
}

#[test]
fn retryable_failure_schedules_retry_and_leaves_access_gate_pending() {
    App::test((), |mut app| async move {
        let model = app.add_singleton_model(|_ctx| test_manager());

        let access_unavailable_messages: Rc<RefCell<Vec<String>>> = Rc::default();
        {
            let access_unavailable_messages = access_unavailable_messages.clone();
            app.update(|ctx| {
                ctx.subscribe_to_model(&model, move |_, event, _ctx| {
                    if let IapManagerEvent::AccessUnavailable { message } = event {
                        access_unavailable_messages
                            .borrow_mut()
                            .push(message.clone());
                    }
                });
            });
        }

        model.update(&mut app, |manager, ctx| {
            manager.state.as_ref().unwrap().set_refreshing();
            manager.ensure_access(ctx);
        });

        model.update(&mut app, |manager, ctx| {
            manager.apply_refresh_result(Err(anyhow::anyhow!("gcloud: connection reset")), ctx);
        });

        assert!(
            access_unavailable_messages.borrow().is_empty(),
            "a retryable failure must not fail the access gate early"
        );
        model.read(&app, |manager, _| {
            assert_eq!(
                manager.consecutive_failures, 1,
                "a retryable failure should still schedule a failure retry"
            );
            assert!(
                manager.access_gate.is_some(),
                "the access gate should keep waiting out IAP_ACCESS_TIMEOUT for a retryable failure"
            );
        });
    });
}

#[test]
fn stale_gate_timeout_does_not_clobber_a_newer_gate() {
    App::test((), |mut app| async move {
        let model = app.add_singleton_model(|_ctx| test_manager());

        let access_unavailable_messages: Rc<RefCell<Vec<String>>> = Rc::default();
        {
            let access_unavailable_messages = access_unavailable_messages.clone();
            app.update(|ctx| {
                ctx.subscribe_to_model(&model, move |_, event, _ctx| {
                    if let IapManagerEvent::AccessUnavailable { message } = event {
                        access_unavailable_messages
                            .borrow_mut()
                            .push(message.clone());
                    }
                });
            });
        }

        // Gate A: arm it via `ensure_access`, pre-marking the shared state as
        // already refreshing so the call doesn't spawn a real gcloud/WIF fetch.
        let gate_a = model.update(&mut app, |manager, ctx| {
            manager.state.as_ref().unwrap().set_refreshing();
            manager.ensure_access(ctx);
            manager
                .access_gate
                .expect("ensure_access should arm a gate")
        });

        // Gate A resolves early via a non-retryable failure, exactly as the
        // real WIF fetch would on completion.
        model.update(&mut app, |manager, ctx| {
            manager.apply_refresh_result(
                Err(anyhow::Error::new(NonRetryableIapError(
                    "WARP_STAGING_IAP_BOOTSTRAP_JWT is unset; cannot mint an IAP token via WIF"
                        .to_string(),
                ))),
                ctx,
            );
        });
        assert_eq!(access_unavailable_messages.borrow().len(), 1);

        // Gate B starts before A's original timer would have fired.
        let gate_b = model.update(&mut app, |manager, ctx| {
            manager.state.as_ref().unwrap().set_refreshing();
            manager.ensure_access(ctx);
            manager
                .access_gate
                .expect("ensure_access should arm a new gate")
        });
        assert_ne!(
            gate_a, gate_b,
            "each `ensure_access` call should be assigned a fresh gate id"
        );

        // A's stale timer finally fires: it must be a no-op, not clobber B.
        model.update(&mut app, |manager, ctx| {
            manager.resolve_access_gate_timeout(gate_a, ctx);
        });
        assert_eq!(
            access_unavailable_messages.borrow().len(),
            1,
            "a stale timer belonging to an already-resolved gate must not fire again"
        );
        model.read(&app, |manager, _| {
            assert_eq!(
                manager.access_gate,
                Some(gate_b),
                "a stale timer must not clear a newer, still-pending gate"
            );
        });

        // B's own timer fires: this is the real timeout for the still-pending gate.
        model.update(&mut app, |manager, ctx| {
            manager.resolve_access_gate_timeout(gate_b, ctx);
        });
        assert_eq!(access_unavailable_messages.borrow().len(), 2);
        model.read(&app, |manager, _| {
            assert!(manager.access_gate.is_none());
        });
    });
}

#[test]
#[serial_test::serial]
fn ensure_access_resolves_gate_immediately_on_cache_hit() {
    let home_dir = tempfile::TempDir::new().expect("failed to create temp HOME");
    let previous_home = set_env_var(
        "HOME",
        home_dir.path().to_str().expect("temp dir path is utf8"),
    );

    let cached_token = jwt_with_payload(&format!(r#"{{"exp":{}}}"#, now_unix() + 3600));
    cache::write(&cached_token);

    App::test((), |mut app| async move {
        let (minter, _) = fake_minter("unused");
        let model = app.add_singleton_model(|_ctx| IapManager {
            managed_mint: Some(ManagedIapMint::new(minter)),
            ..test_manager()
        });

        model.update(&mut app, |manager, ctx| {
            manager.ensure_access(ctx);
        });

        model.read(&app, |manager, _| {
            assert!(
                manager.access_gate.is_none(),
                "a cache hit should resolve the gate immediately instead of waiting out \
                 IAP_ACCESS_TIMEOUT"
            );
            assert!(
                manager.has_valid_token(),
                "the cached token should be loaded"
            );
        });
    });

    restore_env_var("HOME", previous_home);
}

#[test]
fn fetch_iap_token_via_wif_missing_aud_claim_is_non_retryable() {
    let (minter, _) = fake_minter("unused");
    // A syntactically valid JWT with no `aud` claim at all.
    let injected = jwt_with_payload(&format!(r#"{{"exp":{}}}"#, now_unix() + 3600));
    let endpoints = wif_endpoints("http://unused.invalid");

    let err = block_on(fetch_iap_token_via_wif(
        minter,
        injected,
        "iap-client-id".to_string(),
        TEST_SA_EMAIL.to_string(),
        &endpoints,
    ))
    .unwrap_err();

    assert!(
        err.downcast_ref::<NonRetryableIapError>().is_some(),
        "a JWT with no readable `aud` claim can never recover through retry: {err:#}"
    );
    assert!(err.to_string().contains("no readable `aud` claim"));
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
