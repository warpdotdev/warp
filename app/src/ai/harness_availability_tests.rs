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
fn auth_secret_cache_key_distinguishes_harness() {
    let personal_scope = RequestTeamScope::from_scope(&TeamlessScopeForTest);

    assert_ne!(
        AuthSecretCacheKey::new(personal_scope, Harness::Claude),
        AuthSecretCacheKey::new(personal_scope, Harness::Codex)
    );
}
