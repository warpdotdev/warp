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
fn resolves_workspace_admin_link_when_native_workspaces_enabled() {
    let team_uid = ServerId::from(12345);
    let expected_link = format!("{}/admin", ChannelState::server_root_url());
    assert_eq!(
        AdminActions::admin_panel_link(true, Some(team_uid)),
        Some(expected_link),
    );
}

#[test]
fn resolves_team_admin_link_when_native_workspaces_disabled() {
    let team_uid = ServerId::from(12345);
    let expected_link = format!("{}/admin/{}", ChannelState::server_root_url(), team_uid);
    assert_eq!(
        AdminActions::admin_panel_link(false, Some(team_uid)),
        Some(expected_link),
    );
}

#[test]
fn resolves_workspace_admin_link_without_a_team() {
    let expected_link = format!("{}/admin", ChannelState::server_root_url());
    assert_eq!(
        AdminActions::admin_panel_link(true, None),
        Some(expected_link),
    );
}

#[test]
fn resolves_no_admin_link_without_native_workspaces_or_a_team() {
    assert_eq!(AdminActions::admin_panel_link(false, None), None);
}
