use super::*;
use crate::server::ids::ServerId;
use crate::workspaces::user_workspaces::{TeamContextForOperation, TeamlessScopeForTest};

#[test]
fn auth_secret_cache_key_distinguishes_team_scope() {
    let personal_scope = RequestTeamScope::from_scope(&TeamlessScopeForTest);
    let team_scope =
        RequestTeamScope::from_scope(&TeamContextForOperation::new_for_test(ServerId::from(7)));

    assert_ne!(
        AuthSecretCacheKey::new(personal_scope, Harness::Claude),
        AuthSecretCacheKey::new(team_scope, Harness::Claude)
    );
}

#[test]
fn invalidation_rejects_in_flight_auth_secret_fetch_generation() {
    let personal_scope = RequestTeamScope::from_scope(&TeamlessScopeForTest);
    let cache_key = AuthSecretCacheKey::new(personal_scope, Harness::Claude);
    let mut model = HarnessAvailabilityModel {
        harnesses: default_harnesses(),
        auth_secrets: HashMap::from([(cache_key, AuthSecretFetchState::Loading)]),
        auth_secret_retry_after: HashMap::new(),
        auth_secret_generation: 7,
    };
    let in_flight_generation = model.auth_secret_generation;

    model.invalidate_auth_secrets();

    assert!(!model.is_auth_secret_fetch_current(in_flight_generation));
    assert!(model.auth_secrets.is_empty());
    assert!(model.auth_secret_retry_after.is_empty());
}

#[test]
fn auth_secret_cache_key_distinguishes_harness() {
    let personal_scope = RequestTeamScope::from_scope(&TeamlessScopeForTest);

    assert_ne!(
        AuthSecretCacheKey::new(personal_scope, Harness::Claude),
        AuthSecretCacheKey::new(personal_scope, Harness::Codex)
    );
}
