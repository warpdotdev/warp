use super::*;
use crate::server::ids::ServerId;

fn team(uid: i64) -> Team {
    Team::from_local_cache(ServerId::from(uid), format!("team-{uid}"), None, None, None)
}

#[test]
fn no_teams_is_unavailable() {
    let err = select_integration_team(None, &[]).unwrap_err();
    assert!(err.to_string().contains("not currently a member"));
}

#[test]
fn sole_team_is_used_without_team_flag() {
    let teams = [team(1)];
    let selected = select_integration_team(None, &teams).unwrap();
    assert_eq!(selected, teams[0].uid);
}

#[test]
fn sole_team_matching_team_flag_is_used() {
    let teams = [team(1)];
    let uid_str = teams[0].uid.to_string();
    let selected = select_integration_team(Some(&uid_str), &teams).unwrap();
    assert_eq!(selected, teams[0].uid);
}

#[test]
fn sole_team_mismatched_team_flag_is_rejected() {
    let teams = [team(1)];
    let other_uid = team(2).uid.to_string();
    let err = select_integration_team(Some(&other_uid), &teams).unwrap_err();
    assert!(err.to_string().contains("not a member"));
}

#[test]
fn multiple_teams_without_team_flag_requires_selection() {
    let teams = [team(1), team(2)];
    let err = select_integration_team(None, &teams).unwrap_err();
    assert!(err.to_string().contains("--team"));
}

#[test]
fn multiple_teams_with_matching_team_flag_is_used() {
    let teams = [team(1), team(2)];
    let uid_str = teams[1].uid.to_string();
    let selected = select_integration_team(Some(&uid_str), &teams).unwrap();
    assert_eq!(selected, teams[1].uid);
}

#[test]
fn multiple_teams_with_non_member_team_flag_is_rejected() {
    let teams = [team(1), team(2)];
    let other_uid = team(3).uid.to_string();
    let err = select_integration_team(Some(&other_uid), &teams).unwrap_err();
    assert!(err.to_string().contains("not a member"));
}

#[test]
fn invalid_team_uid_is_rejected() {
    let teams = [team(1)];
    let err = select_integration_team(Some("not-a-valid-uid"), &teams).unwrap_err();
    assert!(err.to_string().contains("not a valid team UID"));
}
