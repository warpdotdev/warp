use super::*;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;

fn id_token(account_id: &str) -> String {
    let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"none"}"#);
    let claims = serde_json::json!({
        "https://api.openai.com/auth": {
            "chatgpt_account_id": account_id
        }
    });
    let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).unwrap());
    format!("{header}.{payload}.signature")
}

#[test]
fn known_expiry_refreshes_five_minutes_early() {
    assert_eq!(
        refresh_delay(Some(60 * 60)),
        Duration::from_secs(55 * 60)
    );
}

#[test]
fn near_expiry_refreshes_immediately() {
    assert_eq!(refresh_delay(Some(60)), Duration::ZERO);
}

#[test]
fn missing_expiry_refreshes_after_twenty_four_hours() {
    assert_eq!(refresh_delay(None), Duration::from_secs(24 * 60 * 60));
}

#[test]
fn token_response_builds_codex_tokens_and_extracts_account_id() {
    let token = id_token("account-new");
    let stored = codex_tokens_from_response(
        TokenResponse {
            id_token: Some(token.clone()),
            access_token: "access-new".into(),
            refresh_token: Some("refresh-new".into()),
            expires_in: Some(3600),
        },
        None,
    )
    .unwrap();

    assert_eq!(stored.access_token, "access-new");
    assert_eq!(stored.refresh_token.as_deref(), Some("refresh-new"));
    assert_eq!(stored.id_token.as_deref(), Some(token.as_str()));
    assert_eq!(stored.chatgpt_account_id, "account-new");
    assert!(stored.expires_at.is_some());
    assert!(stored.connected_at.is_some());
}

#[test]
fn refresh_response_carries_forward_optional_identity_and_refresh_fields() {
    let connected_at = SystemTime::now() - Duration::from_secs(60);
    let previous = CodexTokens {
        access_token: "access-old".into(),
        refresh_token: Some("refresh-old".into()),
        id_token: Some(id_token("account-old")),
        chatgpt_account_id: "account-old".into(),
        expires_at: None,
        connected_at: Some(connected_at),
    };
    let stored = codex_tokens_from_response(
        TokenResponse {
            id_token: None,
            access_token: "access-new".into(),
            refresh_token: None,
            expires_in: None,
        },
        Some(&previous),
    )
    .unwrap();

    assert_eq!(stored.refresh_token.as_deref(), Some("refresh-old"));
    assert_eq!(stored.id_token, previous.id_token);
    assert_eq!(stored.chatgpt_account_id, "account-old");
    assert_eq!(stored.connected_at, Some(connected_at));
    assert_eq!(stored.expires_at, None);
}
