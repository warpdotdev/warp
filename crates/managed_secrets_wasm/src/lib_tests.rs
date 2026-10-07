//! Unit tests for BYO context / payload construction.
//!
//! These cover the producer side of the pinned BYO seal format (the exact
//! `byo:1:TEAM:…` context string and the snake_case JSON payloads). The
//! seal/unseal round-trip across the real Tink/HPKE primitive lives in
//! `warp_managed_secrets`'s `envelope_tests.rs`, which has access to the private
//! decrypt primitive and fixture keysets.

use super::{
    ByoEndpointPayload, ByoFirstPartyPayload, byo_endpoint_context, byo_first_party_context,
};

#[test]
fn first_party_context_is_pinned() {
    assert_eq!(
        byo_first_party_context("t_abc", "openai"),
        "byo:1:TEAM:t_abc:first_party:provider=openai"
    );
    assert_eq!(
        byo_first_party_context("t_abc", "anthropic"),
        "byo:1:TEAM:t_abc:first_party:provider=anthropic"
    );
    assert_eq!(
        byo_first_party_context("t_abc", "google"),
        "byo:1:TEAM:t_abc:first_party:provider=google"
    );
}

#[test]
fn endpoint_context_is_pinned() {
    assert_eq!(
        byo_endpoint_context("t_abc", "550e8400-e29b-41d4-a716-446655440000"),
        "byo:1:TEAM:t_abc:endpoint:endpoint=550e8400-e29b-41d4-a716-446655440000"
    );
}

#[test]
fn first_party_payload_is_snake_case_json() {
    let json = serde_json::to_string(&ByoFirstPartyPayload { api_key: "sk-test" })
        .expect("serialize first-party payload");
    assert_eq!(json, r#"{"api_key":"sk-test"}"#);
}

#[test]
fn endpoint_payload_is_snake_case_json_in_order() {
    let json = serde_json::to_string(&ByoEndpointPayload {
        base_url: "https://example.test/v1",
        api_key: "sk-test",
    })
    .expect("serialize endpoint payload");
    assert_eq!(
        json,
        r#"{"base_url":"https://example.test/v1","api_key":"sk-test"}"#
    );
}

/// Special characters in secret values must be JSON-escaped (serde handles this),
/// proving why we serialize via serde rather than hand-building the JSON string.
#[test]
fn payload_escapes_special_characters() {
    let json = serde_json::to_string(&ByoFirstPartyPayload { api_key: "a\"b\\c" })
        .expect("serialize payload with special chars");
    assert_eq!(json, r#"{"api_key":"a\"b\\c"}"#);
}
