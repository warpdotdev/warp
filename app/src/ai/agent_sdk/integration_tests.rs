use super::IntegrationRetryState;
use crate::server::ids::ServerId;
use crate::server::team_scope::RequestTeamScope;
use crate::workspaces::user_workspaces::{TeamContextForOperation, TeamlessScopeForTest};

#[test]
fn oauth_retry_retains_the_initiating_team_scope() {
    let team_uid = ServerId::from(7);
    let scope = TeamContextForOperation::new_for_test(team_uid);
    let retry_state = IntegrationRetryState::new(RequestTeamScope::from_scope(&scope));

    let retry_state = retry_state.next();

    assert_eq!(retry_state.attempt, 2);
    assert_eq!(retry_state.request_team_scope.team_uid(), Some(team_uid));
}

#[test]
fn oauth_retry_retains_a_teamless_initiating_scope() {
    let retry_state =
        IntegrationRetryState::new(RequestTeamScope::from_scope(&TeamlessScopeForTest));

    let retry_state = retry_state.next();

    assert_eq!(retry_state.attempt, 2);
    assert_eq!(retry_state.request_team_scope.team_uid(), None);
}
