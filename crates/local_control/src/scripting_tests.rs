use chrono::{Duration, Utc};

use super::*;

#[test]
fn scripting_grant_scope_membership_works() {
    let grant = ScriptingGrant {
        source: ScriptingIdentitySource::ExternalApiKey {
            key_id: "key_test".to_owned(),
        },
        subject: "user@example.com".to_owned(),
        scopes: vec![ScriptingScope::LocalControlMutateUnderlyingData],
        issued_at: Utc::now(),
        expires_at: Utc::now() + Duration::minutes(5),
        revoked: false,
    };
    assert!(grant.has_scope(&ScriptingScope::LocalControlMutateUnderlyingData));
    assert!(!grant.has_scope(&ScriptingScope::LocalControlReadMetadata));
}

#[test]
fn scripting_grant_rejects_expired_grant() {
    let grant = ScriptingGrant {
        source: ScriptingIdentitySource::ExternalApiKey {
            key_id: "key_test".to_owned(),
        },
        subject: "user@example.com".to_owned(),
        scopes: vec![ScriptingScope::LocalControlReadMetadata],
        issued_at: Utc::now() - Duration::hours(1),
        expires_at: Utc::now() - Duration::seconds(1),
        revoked: false,
    };
    let err = grant
        .verify_for_action(ActionKind::InstanceList, None)
        .expect_err("expired grant is rejected");
    assert_eq!(err.code, ErrorCode::AuthenticatedUserUnavailable);
}

#[test]
fn scripting_grant_rejects_revoked_grant() {
    let grant = ScriptingGrant {
        source: ScriptingIdentitySource::ExternalApiKey {
            key_id: "key_test".to_owned(),
        },
        subject: "user@example.com".to_owned(),
        scopes: vec![ScriptingScope::LocalControlReadMetadata],
        issued_at: Utc::now(),
        expires_at: Utc::now() + Duration::minutes(5),
        revoked: true,
    };
    let err = grant
        .verify_for_action(ActionKind::InstanceList, None)
        .expect_err("revoked grant is rejected");
    assert_eq!(err.code, ErrorCode::UnauthorizedLocalClient);
}

#[test]
fn scripting_grant_rejects_missing_scope() {
    let grant = ScriptingGrant {
        source: ScriptingIdentitySource::ExternalApiKey {
            key_id: "key_test".to_owned(),
        },
        subject: "user@example.com".to_owned(),
        scopes: vec![ScriptingScope::LocalControlReadMetadata],
        issued_at: Utc::now(),
        expires_at: Utc::now() + Duration::minutes(5),
        revoked: false,
    };
    let err = grant
        .verify_for_action(ActionKind::TabCreate, None)
        .expect_err("scope mismatch is rejected");
    assert_eq!(err.code, ErrorCode::InsufficientPermissions);
}

#[test]
fn scripting_grant_rejects_subject_mismatch_for_authenticated_actions() {
    let grant = ScriptingGrant {
        source: ScriptingIdentitySource::ExternalApiKey {
            key_id: "key_test".to_owned(),
        },
        subject: "user@example.com".to_owned(),
        scopes: vec![ScriptingScope::LocalControlMutateUnderlyingData],
        issued_at: Utc::now(),
        expires_at: Utc::now() + Duration::minutes(5),
        revoked: false,
    };
    let err = grant
        .verify_for_action(ActionKind::InputInsert, Some("other@example.com"))
        .expect_err("subject mismatch is rejected");
    assert_eq!(err.code, ErrorCode::AuthenticatedUserUnavailable);
}

#[test]
fn api_key_storage_ref_serializes_without_raw_key() {
    let storage_ref = ApiKeyStorageRef {
        key_id: "kid_abc123".to_owned(),
        subject: "user@warp.dev".to_owned(),
        scopes: vec![ScriptingScope::LocalControlMutateUnderlyingData],
        expires_at: Utc::now() + Duration::minutes(5),
        revoked: false,
    };
    let json = serde_json::to_value(&storage_ref).expect("serializes");
    assert!(json["key_id"].as_str().is_some());
    assert!(json["subject"].as_str().is_some());
    assert!(json.get("raw_key").is_none());
    assert!(json.get("key_secret").is_none());
}

#[test]
fn api_key_secret_debug_redacts_raw_key() {
    let secret = ApiKeySecret::new("warp_sk_test_raw_secret".to_owned()).expect("valid");
    let debug = format!("{secret:?}");
    assert!(!debug.contains("warp_sk_test_raw_secret"));
    assert!(debug.contains("REDACTED"));
}

#[test]
fn exchange_stub_rejects_short_key() {
    let secret = ApiKeySecret::new("short".to_owned()).expect("nonempty");
    let err = exchange_api_key_stub(&secret).expect_err("short key is rejected");
    assert_eq!(err.code, ErrorCode::InvalidParams);
}
