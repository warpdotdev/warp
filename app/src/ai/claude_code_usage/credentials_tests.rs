use chrono::TimeZone as _;

use super::*;

#[test]
fn test_parses_stored_credentials() {
    let token = parse_credentials(
        r#"{
            "claudeAiOauth": {
                "accessToken": "sk-ant-oat-example",
                "refreshToken": "sk-ant-ort-example",
                "expiresAt": 1784656270182,
                "subscriptionType": "max"
            },
            "mcpOAuth": {}
        }"#,
    )
    .expect("credentials should parse");

    assert_eq!("sk-ant-oat-example", token.token);
    assert_eq!(
        Some(1784656270182),
        token.expires_at.map(|at| at.timestamp_millis())
    );
}

#[test]
fn test_credentials_without_an_expiry_are_still_usable() {
    let token = parse_credentials(r#"{"claudeAiOauth": {"accessToken": "token"}}"#)
        .expect("credentials should parse");

    assert_eq!(None, token.expires_at);
    assert!(token.is_usable(Utc::now()));
}

#[test]
fn test_credentials_without_an_oauth_section_are_rejected() {
    assert!(parse_credentials(r#"{"mcpOAuth": {}}"#).is_err());
}

#[test]
fn test_empty_access_tokens_are_rejected() {
    assert!(parse_credentials(r#"{"claudeAiOauth": {"accessToken": ""}}"#).is_err());
}

#[test]
fn test_a_token_is_unusable_once_it_is_inside_the_expiry_skew() {
    let now = Utc.with_ymd_and_hms(2026, 7, 25, 12, 0, 0).unwrap();
    let token = |expires_at| ClaudeAccessToken {
        token: "token".to_string(),
        expires_at: Some(expires_at),
    };

    assert!(token(now + chrono::TimeDelta::hours(1)).is_usable(now));
    // Inside the skew window: technically valid, but not worth starting a
    // request with.
    assert!(!token(now + chrono::TimeDelta::seconds(30)).is_usable(now));
    assert!(!token(now - chrono::TimeDelta::hours(1)).is_usable(now));
}
