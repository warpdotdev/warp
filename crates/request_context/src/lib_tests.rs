use super::*;

#[test]
fn unscoped_scope_has_no_team_uid() {
    let scope = RequestTeamScope::new(None);

    assert!(scope.is_unscoped());
    assert_eq!(scope.team_uid(), None);
}

#[test]
fn team_scope_exposes_its_team_uid() {
    let scope = RequestTeamScope::new(Some("team-uid".to_string()));

    assert!(!scope.is_unscoped());
    assert_eq!(scope.team_uid(), Some("team-uid"));
}

#[test]
fn scope_serializes_as_optional_team_uid() {
    assert_eq!(
        serde_json::to_value(RequestTeamScope::new(Some("team-uid".to_string()))).unwrap(),
        serde_json::json!("team-uid")
    );
    assert_eq!(
        serde_json::to_value(RequestTeamScope::new(None)).unwrap(),
        serde_json::Value::Null
    );
}
