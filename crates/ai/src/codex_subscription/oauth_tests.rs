use super::*;

#[test]
fn authorize_url_contains_required_params() {
    let pkce = PkceParams::generate();
    let url = authorize_url(&pkce);

    assert!(url.starts_with("https://auth.openai.com/oauth/authorize?"));
    assert!(url.contains("response_type=code"));
    assert!(url.contains(&format!("client_id={CLIENT_ID}")));
    assert!(url.contains("code_challenge_method=S256"));
    assert!(url.contains("scope=openid"));
    // OpenAI-specific params
    assert!(url.contains("id_token_add_organizations=true"));
    assert!(url.contains("codex_cli_simplified_flow=true"));
    assert!(url.contains("originator=warp"));
    // The redirect URI must be percent-encoded and match the registered value.
    assert!(url.contains("redirect_uri=http%3A%2F%2Flocalhost%3A1455%2Fauth%2Fcallback"));
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
        r#"{"access_token":"a","refresh_token":"r","token_type":"Bearer","expires_in":3600000,"scope":"openid"}"#,
    )
    .expect("full response should parse");
    assert_eq!(full.access_token, "a");
    assert_eq!(full.refresh_token.as_deref(), Some("r"));
    // OpenAI returns `expires_in` in milliseconds.
    assert_eq!(full.expires_in, Some(3_600_000));
}
