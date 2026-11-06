use super::*;

#[test]
fn test_admin_panel_link_generation() {
    let team_uid = ServerId::from(12345);
    let expected_link = format!("{}/admin/{}", ChannelState::server_root_url(), team_uid);
    let actual_link = AdminActions::admin_panel_link_for_team(team_uid);
    assert_eq!(actual_link, expected_link);
}

#[test]
fn test_workspace_admin_panel_link_generation() {
    let expected_link = format!("{}/admin", ChannelState::server_root_url());
    let actual_link = AdminActions::admin_panel_link_for_workspace();
    assert_eq!(actual_link, expected_link);
}

#[test]
fn test_workspace_teams_admin_panel_link_generation() {
    let expected_link = format!("{}/admin/workspace/teams", ChannelState::server_root_url());
    let actual_link = AdminActions::workspace_teams_admin_panel_link();
    assert_eq!(actual_link, expected_link);
}
