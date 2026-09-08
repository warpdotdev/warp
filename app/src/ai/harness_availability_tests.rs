use super::*;
use crate::server::ids::ServerId;
use crate::workspaces::user_workspaces::{TeamContextForOperation, TeamlessScopeForTest};

fn team_scope(team_uid: i64) -> RequestTeamScope {
    RequestTeamScope::from_scope(&TeamContextForOperation::new_for_test(ServerId::from(
        team_uid,
    )))
}

fn teamless_scope() -> RequestTeamScope {
    RequestTeamScope::from_scope(&TeamlessScopeForTest)
}

#[test]
fn auth_secret_cache_key_distinguishes_team_scope_and_harness() {
    assert_ne!(
        AuthSecretCacheKey::new(team_scope(7), Harness::Claude),
        AuthSecretCacheKey::new(team_scope(8), Harness::Claude)
    );
    assert_ne!(
        AuthSecretCacheKey::new(teamless_scope(), Harness::Claude),
        AuthSecretCacheKey::new(teamless_scope(), Harness::Codex)
    );
}

#[test]
fn invalidation_rejects_in_flight_auth_secret_fetch_generation() {
    let cache_key = AuthSecretCacheKey::new(team_scope(7), Harness::Claude);
    let mut model = HarnessAvailabilityModel {
        harnesses: default_harnesses(),
        auth_secrets: HashMap::from([(cache_key, AuthSecretFetchState::Loading)]),
        auth_secret_retry_after: HashMap::from([(cache_key, Instant::now())]),
        auth_secret_generation: 7,
    };
    let in_flight_generation = model.auth_secret_generation;

    model.invalidate_auth_secrets();

    assert!(!model.is_auth_secret_fetch_current(in_flight_generation));
    assert!(model.auth_secrets.is_empty());
    assert!(model.auth_secret_retry_after.is_empty());
}

#[test]
fn window_team_switch_reads_only_the_new_team_cache() {
    let window_a_initial_scope = team_scope(7);
    let window_b_scope = team_scope(8);
    let model = HarnessAvailabilityModel {
        harnesses: default_harnesses(),
        auth_secrets: HashMap::from([
            (
                AuthSecretCacheKey::new(window_a_initial_scope, Harness::Claude),
                AuthSecretFetchState::Loaded(vec![AuthSecretEntry {
                    name: "team-a".to_string(),
                    owner: SecretOwner::CurrentUser,
                }]),
            ),
            (
                AuthSecretCacheKey::new(window_b_scope, Harness::Claude),
                AuthSecretFetchState::Loaded(vec![AuthSecretEntry {
                    name: "team-b".to_string(),
                    owner: SecretOwner::CurrentUser,
                }]),
            ),
        ]),
        auth_secret_retry_after: HashMap::new(),
        auth_secret_generation: 0,
    };

    assert!(matches!(
        model.auth_secrets_for(window_a_initial_scope, Harness::Claude),
        AuthSecretFetchState::Loaded(entries) if entries[0].name == "team-a"
    ));

    let window_a_after_switch = window_b_scope;
    assert!(matches!(
        model.auth_secrets_for(window_a_after_switch, Harness::Claude),
        AuthSecretFetchState::Loaded(entries) if entries[0].name == "team-b"
    ));
}
