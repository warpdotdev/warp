use super::RequestTeamScope;
use crate::server::ids::ServerId;
use crate::workspaces::user_workspaces::{TeamContextForOperation, TeamlessScopeForTest};

#[test]
fn unresolved_capture_matches_any_later_scope() {
    let captured = RequestTeamScope::from_scope(&TeamlessScopeForTest);

    assert!(captured.matches_scope(&TeamlessScopeForTest));
    assert!(captured.matches_scope(&TeamContextForOperation::new_for_test(ServerId::from(1))));
}

#[test]
fn resolved_capture_matches_the_same_team() {
    let captured =
        RequestTeamScope::from_scope(&TeamContextForOperation::new_for_test(ServerId::from(1)));

    assert!(captured.matches_scope(&TeamContextForOperation::new_for_test(ServerId::from(1))));
}

#[test]
fn resolved_capture_rejects_a_different_team() {
    let captured =
        RequestTeamScope::from_scope(&TeamContextForOperation::new_for_test(ServerId::from(1)));

    assert!(!captured.matches_scope(&TeamContextForOperation::new_for_test(ServerId::from(2))));
}

#[test]
fn resolved_capture_rejects_reverting_to_unknown() {
    let captured =
        RequestTeamScope::from_scope(&TeamContextForOperation::new_for_test(ServerId::from(1)));

    assert!(!captured.matches_scope(&TeamlessScopeForTest));
}
