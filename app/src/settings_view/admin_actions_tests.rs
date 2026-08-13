use super::*;
use crate::auth::UserUid;
use crate::workspaces::team::MembershipRole;
use crate::workspaces::workspace::{
    NativeWorkspacesPolicy, WorkspaceMember, WorkspaceMemberUsageInfo, WorkspaceUid,
};

const MEMBER_EMAIL: &str = "member@warp.dev";

fn workspace(role: MembershipRole, native_workspaces_enabled: bool) -> Workspace {
    let mut workspace = Workspace::from_local_cache(
        WorkspaceUid::from(ServerId::from(1)),
        "Workspace".to_string(),
        None,
    );
    workspace.members = vec![WorkspaceMember {
        uid: UserUid::new("member-uid"),
        email: MEMBER_EMAIL.to_string(),
        role,
        usage_info: WorkspaceMemberUsageInfo {
            is_unlimited: false,
            request_limit: 0,
            requests_used_since_last_refresh: 0,
            is_request_limit_prorated: false,
        },
    }];
    workspace.billing_metadata.tier.native_workspaces_policy = Some(NativeWorkspacesPolicy {
        enabled: native_workspaces_enabled,
    });
    workspace
}

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
fn test_workspace_admin_on_native_workspaces_plan_uses_workspace_panel() {
    assert!(AdminActions::should_use_workspace_admin_panel(
        Some(&workspace(MembershipRole::Admin, true)),
        MEMBER_EMAIL,
    ));
}

#[test]
fn test_workspace_admin_without_native_workspaces_keeps_team_panel() {
    assert!(!AdminActions::should_use_workspace_admin_panel(
        Some(&workspace(MembershipRole::Owner, false)),
        MEMBER_EMAIL,
    ));
}

#[test]
fn test_non_workspace_admin_keeps_team_panel() {
    assert!(!AdminActions::should_use_workspace_admin_panel(
        Some(&workspace(MembershipRole::User, true)),
        MEMBER_EMAIL,
    ));

    // A team admin who isn't a member of the workspace roster at all.
    assert!(!AdminActions::should_use_workspace_admin_panel(
        Some(&workspace(MembershipRole::Owner, true)),
        "someone-else@warp.dev",
    ));
}

#[test]
fn test_missing_workspace_keeps_team_panel() {
    assert!(!AdminActions::should_use_workspace_admin_panel(
        None,
        MEMBER_EMAIL
    ));
}
