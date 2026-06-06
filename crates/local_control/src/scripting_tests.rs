use chrono::Duration;

use super::*;

#[test]
fn verified_terminal_grant_carries_subject_and_action() {
    let grant = ScriptingGrant::verified_warp_terminal(
        "session-1",
        "user-1",
        vec![ActionKind::AppPing],
        Duration::minutes(5),
    );

    assert_eq!(grant.subject, "user-1");
    assert!(grant.has_action(ActionKind::AppPing));
    assert!(!grant.has_action(ActionKind::InputRun));
    grant
        .verify_action(ActionKind::AppPing)
        .expect("action is accepted");
}
