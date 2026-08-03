use super::should_show_open_admin_panel_link;

#[test]
fn test_hidden_when_neither_team_nor_workspace_admin() {
    assert!(!should_show_open_admin_panel_link(false, false, true));
    assert!(!should_show_open_admin_panel_link(false, false, false));
}

#[test]
fn test_visible_for_team_admin_only_on_enterprise_plan() {
    assert!(should_show_open_admin_panel_link(true, false, true));
}

#[test]
fn test_visible_for_workspace_admin_only_on_enterprise_plan() {
    assert!(should_show_open_admin_panel_link(false, true, true));
}

#[test]
fn test_visible_when_both_team_and_workspace_admin_on_enterprise_plan() {
    assert!(should_show_open_admin_panel_link(true, true, true));
}

#[test]
fn test_hidden_on_non_enterprise_plan_regardless_of_admin_status() {
    assert!(!should_show_open_admin_panel_link(true, false, false));
    assert!(!should_show_open_admin_panel_link(false, true, false));
    assert!(!should_show_open_admin_panel_link(true, true, false));
    assert!(!should_show_open_admin_panel_link(false, false, false));
}
