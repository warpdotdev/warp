use super::*;

fn key(name: &str, scope: &str, created_at: DateTime<Utc>) -> ApiKeyInfo {
    key_with_uid(name, name, scope, created_at)
}

fn key_with_uid(uid: &str, name: &str, scope: &str, created_at: DateTime<Utc>) -> ApiKeyInfo {
    ApiKeyInfo {
        uid: uid.to_string(),
        name: name.to_string(),
        key_suffix: "abcd".to_string(),
        scope: scope.to_string(),
        created_at,
        last_used_at: None,
        expires_at: None,
    }
}

#[test]
fn sort_api_keys_sorts_by_name_ascending() {
    let created_at = Utc::now();
    let mut keys = vec![
        key("beta", "Team", created_at),
        key("alpha", "Personal", created_at),
    ];

    sort_api_keys(
        &mut keys,
        Some(ApiKeySortByArg::Name),
        Some(SortOrderArg::Asc),
    );

    assert_eq!(keys[0].name, "alpha");
    assert_eq!(keys[1].name, "beta");
}

#[test]
fn sort_api_keys_sorts_by_created_at_descending() {
    let older = Utc::now() - chrono::Duration::days(1);
    let newer = Utc::now();
    let mut keys = vec![key("older", "Team", older), key("newer", "Personal", newer)];

    sort_api_keys(
        &mut keys,
        Some(ApiKeySortByArg::CreatedAt),
        Some(SortOrderArg::Desc),
    );

    assert_eq!(keys[0].name, "newer");
    assert_eq!(keys[1].name, "older");
}

#[test]
fn resolve_api_key_identifier_prefers_uid_match() {
    let created_at = Utc::now();
    let keys = vec![
        key_with_uid("target", "other-name", "Team", created_at),
        key_with_uid("other-uid", "target", "Team", created_at),
    ];

    assert_eq!(
        resolve_api_key_identifier(&keys, "target")
            .unwrap()
            .unwrap(),
        keys[0].clone()
    );
}

#[test]
fn resolve_api_key_identifier_falls_back_to_name_match() {
    let created_at = Utc::now();
    let keys = vec![key_with_uid("uid-1", "deploy-key", "Team", created_at)];

    assert_eq!(
        resolve_api_key_identifier(&keys, "deploy-key")
            .unwrap()
            .unwrap(),
        keys[0].clone()
    );
}

#[test]
fn resolve_api_key_identifier_errors_for_ambiguous_name_matches() {
    let created_at = Utc::now();
    let keys = vec![
        key_with_uid("uid-1", "deploy-key", "Team", created_at),
        key_with_uid("uid-2", "deploy-key", "Personal", created_at),
    ];
    let err = resolve_api_key_identifier(&keys, "deploy-key").unwrap_err();

    assert_eq!(
        err.to_string(),
        "Multiple API keys match 'deploy-key'; specify the key by UID"
    );
}

#[test]
fn resolve_api_key_identifier_errors_when_not_found() {
    let created_at = Utc::now();
    let keys = vec![key_with_uid("uid-1", "deploy-key", "Team", created_at)];
    let err = resolve_api_key_identifier(&keys, "missing-key").unwrap_err();

    assert_eq!(err.to_string(), "API key 'missing-key' not found");
}

#[test]
fn api_key_display_includes_creation_date() {
    let created_at = "2026-01-02T03:04:05Z".parse().unwrap();
    let key = key_with_uid("uid-1", "deploy-key", "Team", created_at);

    assert_eq!(
        key.to_string(),
        "deploy-key (uid-1, created 2026-01-02 03:04:05 UTC)"
    );
}
