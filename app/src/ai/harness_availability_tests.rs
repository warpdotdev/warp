use super::*;
use crate::workspaces::user_workspaces::{TeamContextForOperation, TeamlessScopeForTest};
fn team_scope(team_uid: i64) -> TeamContextForOperation {
    TeamContextForOperation::new_for_test(team_uid.into())
}

fn teamless_scope() -> TeamlessScopeForTest {
    TeamlessScopeForTest
}

#[test]
fn auth_secret_cache_key_distinguishes_team_scope_and_harness() {
    assert_ne!(
        AuthSecretCacheKey::new(&team_scope(7), Harness::Claude),
        AuthSecretCacheKey::new(&team_scope(8), Harness::Claude)
    );
    assert_ne!(
        AuthSecretCacheKey::new(&teamless_scope(), Harness::Claude),
        AuthSecretCacheKey::new(&teamless_scope(), Harness::Codex)
    );
}

#[test]
fn window_team_switch_reads_only_the_new_team_cache() {
    let window_a_initial_scope = team_scope(7);
    let window_b_scope = team_scope(8);
    let model = HarnessAvailabilityModel {
        harnesses: default_harnesses(),
        auth_secrets: HashMap::from([
            (
                AuthSecretCacheKey::new(&window_a_initial_scope, Harness::Claude),
                AuthSecretFetchState::Loaded(vec![AuthSecretEntry {
                    name: "team-a".to_string(),
                    owner: SecretOwner::CurrentUser,
                }]),
            ),
            (
                AuthSecretCacheKey::new(&window_b_scope, Harness::Claude),
                AuthSecretFetchState::Loaded(vec![AuthSecretEntry {
                    name: "team-b".to_string(),
                    owner: SecretOwner::CurrentUser,
                }]),
            ),
        ]),
        auth_secret_retry_after: HashMap::new(),
    };

    assert!(matches!(
        model.auth_secrets_for(&window_a_initial_scope, Harness::Claude),
        AuthSecretFetchState::Loaded(entries) if entries[0].name == "team-a"
    ));

    let window_a_after_switch = window_b_scope;
    assert!(matches!(
        model.auth_secrets_for(&window_a_after_switch, Harness::Claude),
        AuthSecretFetchState::Loaded(entries) if entries[0].name == "team-b"
    ));
}

#[test]
fn personal_secret_mutations_update_every_team_cache() {
    let team_a = team_scope(7);
    let team_b = team_scope(8);
    let key_a = AuthSecretCacheKey::new(&team_a, Harness::Claude);
    let key_b = AuthSecretCacheKey::new(&team_b, Harness::Claude);
    let mut model = HarnessAvailabilityModel {
        harnesses: default_harnesses(),
        auth_secrets: HashMap::from([
            (key_a, AuthSecretFetchState::Loaded(Vec::new())),
            (key_b, AuthSecretFetchState::Loaded(Vec::new())),
        ]),
        auth_secret_retry_after: HashMap::new(),
    };
    let entry = AuthSecretEntry {
        name: "personal".to_string(),
        owner: SecretOwner::CurrentUser,
    };

    model.insert_created_auth_secret_entry(key_a, entry);

    assert!(matches!(
        model.auth_secrets_for(&team_a, Harness::Claude),
        AuthSecretFetchState::Loaded(entries) if entries.len() == 1
    ));
    assert!(matches!(
        model.auth_secrets_for(&team_b, Harness::Claude),
        AuthSecretFetchState::Loaded(entries) if entries.len() == 1
    ));

    model.remove_deleted_auth_secret_entries(key_a, "personal", &SecretOwner::CurrentUser);

    assert!(matches!(
        model.auth_secrets_for(&team_a, Harness::Claude),
        AuthSecretFetchState::Loaded(entries) if entries.is_empty()
    ));
    assert!(matches!(
        model.auth_secrets_for(&team_b, Harness::Claude),
        AuthSecretFetchState::Loaded(entries) if entries.is_empty()
    ));
}

#[test]
fn team_secret_mutations_update_only_the_owner_team_cache() {
    let team_a = team_scope(7);
    let team_b = team_scope(8);
    let key_a = AuthSecretCacheKey::new(&team_a, Harness::Claude);
    let key_b = AuthSecretCacheKey::new(&team_b, Harness::Claude);
    let mut model = HarnessAvailabilityModel {
        harnesses: default_harnesses(),
        auth_secrets: HashMap::from([
            (key_a, AuthSecretFetchState::Loaded(Vec::new())),
            (key_b, AuthSecretFetchState::Loaded(Vec::new())),
        ]),
        auth_secret_retry_after: HashMap::new(),
    };
    let entry = AuthSecretEntry {
        name: "team".to_string(),
        owner: SecretOwner::Team {
            team_uid: team_a.team_uid().unwrap().uid(),
        },
    };

    model.insert_created_auth_secret_entry(key_a, entry);

    assert!(matches!(
        model.auth_secrets_for(&team_a, Harness::Claude),
        AuthSecretFetchState::Loaded(entries) if entries.len() == 1
    ));
    assert!(matches!(
        model.auth_secrets_for(&team_b, Harness::Claude),
        AuthSecretFetchState::Loaded(entries) if entries.is_empty()
    ));
}
