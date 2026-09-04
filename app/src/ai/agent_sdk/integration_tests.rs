use warp_graphql::queries::get_oauth_connect_tx_status::OauthConnectTxStatus;

use super::IntegrationRetryState;
use crate::server::ids::ServerId;
use crate::server::team_scope::RequestTeamScope;
use crate::workspaces::user_workspaces::{TeamContextForOperation, TeamlessScopeForTest};

#[test]
fn completed_oauth_continuation_preserves_the_initiating_team_scope() {
    let scope = TeamContextForOperation::new_for_test(ServerId::from(7));
    assert_completed_oauth_continuation(RequestTeamScope::from_scope(&scope));
}

#[test]
fn completed_oauth_continuation_preserves_the_initiating_teamless_scope() {
    assert_completed_oauth_continuation(RequestTeamScope::from_scope(&TeamlessScopeForTest));
}

fn assert_completed_oauth_continuation(expected_scope: RequestTeamScope) {
    let retry_state = IntegrationRetryState::new(expected_scope)
        .continue_after_oauth(Ok(OauthConnectTxStatus::Completed))
        .unwrap();

    assert_eq!(retry_state.attempt, 2);
    assert_eq!(retry_state.request_team_scope, expected_scope);
}
