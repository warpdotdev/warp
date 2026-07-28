use super::*;

#[test]
fn authorize_url_contains_required_params() {
    let pkce = PkceParams::generate();
    let url = authorize_url(&pkce);

    assert!(url.starts_with("https://auth.x.ai/oauth2/authorize?"));
    assert!(url.contains("response_type=code"));
    assert!(url.contains(&format!("client_id={CLIENT_ID}")));
    assert!(url.contains("code_challenge_method=S256"));
    assert!(url.contains("scope=openid"));
    assert!(url.contains("plan=generic"));
    assert!(url.contains("referrer=warp"));
    // The redirect URI must be percent-encoded and match the registered value.
    assert!(url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A56121%2Fcallback"));
    // The CSRF state and PKCE challenge are echoed into the URL verbatim
    // (both are URL-safe base64, so no percent-encoding is applied).
    assert!(url.contains(&format!("state={}", pkce.state)));
    assert!(url.contains(&format!("code_challenge={}", pkce.challenge)));
}

#[test]
fn token_response_parses_minimal_and_full() {
    let minimal: TokenResponse =
        serde_json::from_str(r#"{"access_token":"abc"}"#).expect("minimal response should parse");
    assert_eq!(minimal.access_token, "abc");
    assert!(minimal.refresh_token.is_none());
    assert!(minimal.expires_in.is_none());

    // Unconsumed response fields (token_type, scope) are ignored by serde.
    let full: TokenResponse = serde_json::from_str(
        r#"{"access_token":"a","refresh_token":"r","token_type":"Bearer","expires_in":3600,"scope":"api:access"}"#,
    )
    .expect("full response should parse");
    assert_eq!(full.access_token, "a");
    assert_eq!(full.refresh_token.as_deref(), Some("r"));
    assert_eq!(full.expires_in, Some(3600));
}

#[test]
fn manual_code_exchange_captures_attempt_verifier() {
    let pkce = PkceParams::generate();
    let exchange = ManualCodeExchange {
        verifier: pkce.verifier.clone(),
    };
    assert_eq!(exchange.verifier, pkce.verifier);
}

#[test]
fn manual_code_exchange_rejects_blank_code() {
    let exchange = ManualCodeExchange {
        verifier: "verifier".to_string(),
    };
    let result = warpui_core::r#async::block_on(exchange.exchange("   "));
    assert!(result.is_err());
}

#[test]
fn cancelling_loopback_wait_releases_listener() {
    let listener =
        TcpListener::bind((REDIRECT_HOST, 0)).expect("test callback listener should bind");
    listener
        .set_nonblocking(true)
        .expect("test callback listener should be non-blocking");
    let address = listener
        .local_addr()
        .expect("test callback listener should have an address");
    let cancellation = OauthCancellationHandle {
        cancelled: Arc::new(AtomicBool::new(false)),
    };
    cancellation.cancel();

    let result = warpui_core::r#async::block_on(run_oauth_flow(
        listener,
        PkceParams::generate(),
        cancellation,
    ));

    assert_eq!(
        result
            .expect_err("cancelled callback wait should fail")
            .to_string(),
        "Grok authorization was cancelled"
    );
    TcpListener::bind(address).expect("cancelled callback listener should release its port");
}
