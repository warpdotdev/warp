use base64::prelude::{BASE64_URL_SAFE_NO_PAD, Engine as _};
use chrono::{TimeZone, Utc};

use super::parse_jwt_expiration;

/// Helper to create a JWT token string for testing.
fn make_jwt(payload_json: &str) -> String {
    let header = BASE64_URL_SAFE_NO_PAD.encode(r#"{"alg":"HS256","typ":"JWT"}"#);
    let payload = BASE64_URL_SAFE_NO_PAD.encode(payload_json);
    let signature = BASE64_URL_SAFE_NO_PAD.encode("fake_signature");
    format!("{header}.{payload}.{signature}")
}

#[test]
fn test_parse_jwt_expiration_valid() {
    // Unix timestamp for 2024-01-15 12:00:00 UTC.
    let exp_timestamp: i64 = 1705320000;
    let token = make_jwt(&format!(r#"{{"exp":{exp_timestamp},"sub":"user123"}}"#));

    let result = parse_jwt_expiration(&token).unwrap();
    let expected = Utc.timestamp_opt(exp_timestamp, 0).unwrap();

    assert_eq!(result, expected);
}

#[test]
fn test_parse_jwt_expiration_invalid_format_too_few_parts() {
    let token = "header.payload";
    assert!(parse_jwt_expiration(token).is_err());
}

#[test]
fn test_parse_jwt_expiration_invalid_format_too_many_parts() {
    let token = "a.b.c.d";
    assert!(parse_jwt_expiration(token).is_err());
}

#[test]
fn test_parse_jwt_expiration_invalid_json() {
    let header = BASE64_URL_SAFE_NO_PAD.encode(r#"{"alg":"HS256"}"#);
    let payload = BASE64_URL_SAFE_NO_PAD.encode("not valid json");
    let token = format!("{header}.{payload}.signature");

    assert!(parse_jwt_expiration(&token).is_err());
}

#[test]
fn test_parse_jwt_expiration_missing_exp_field() {
    let token = make_jwt(r#"{"sub":"user123","iat":1234567890}"#);
    assert!(parse_jwt_expiration(token.as_str()).is_err());
}
