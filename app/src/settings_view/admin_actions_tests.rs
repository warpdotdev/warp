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
fn test_workspace_admin_panel_routing() {
    // (workspace role, native workspaces enabled, viewer email, uses workspace panel)
    let cases = [
        (MembershipRole::Admin, true, MEMBER_EMAIL, true),
        (MembershipRole::Owner, true, MEMBER_EMAIL, true),
        // An admin on a plan without native workspaces, a non-admin member, and
        // a viewer absent from the roster all keep the team-scoped panel.
        (MembershipRole::Owner, false, MEMBER_EMAIL, false),
        (MembershipRole::User, true, MEMBER_EMAIL, false),
        (MembershipRole::Owner, true, "someone-else@warp.dev", false),
    ];

    for (role, native_workspaces_enabled, email, expected) in cases {
        assert_eq!(
            AdminActions::should_use_workspace_admin_panel(
                Some(&workspace(role, native_workspaces_enabled)),
                email,
            ),
            expected,
            "role={role:?}, native_workspaces_enabled={native_workspaces_enabled}, email={email}"
        );
    }

    // No workspace at all also keeps the team-scoped panel.
    assert!(!AdminActions::should_use_workspace_admin_panel(
        None,
        MEMBER_EMAIL
    ));
}
