use super::*;
use crate::workspaces::team::TeamMember;
use crate::workspaces::workspace::{
    MultiAdminPolicy, Tier, WorkspaceMember, WorkspaceMemberUsageInfo,
};

fn team_with_members(members: Vec<TeamMember>) -> Team {
    Team {
        uid: 1.into(),
        name: "Team".to_string(),
        color: None,
        invite_link: None,
        members,
        pending_email_invites: Vec::new(),
        invite_link_domain_restrictions: Vec::new(),
        billing_metadata: BillingMetadata {
            tier: Tier {
                multi_admin_policy: Some(MultiAdminPolicy { enabled: true }),
                ..Default::default()
            },
            ..Default::default()
        },
        stripe_customer_id: None,
        settings: Default::default(),
        is_eligible_for_discovery: false,
        has_billing_history: false,
    }
}

fn workspace_member(uid: UserUid, email: &str, role: MembershipRole) -> WorkspaceMember {
    WorkspaceMember {
        uid,
        email: email.to_string(),
        role,
        usage_info: WorkspaceMemberUsageInfo {
            is_unlimited: true,
            request_limit: 0,
            requests_used_since_last_refresh: 0,
            is_request_limit_prorated: false,
        },
    }
}

/// `role_detachment_enabled` mirrors the server-computed
/// `nativeWorkspacesRoleDetachmentEnabled` GraphQL field, which is already the
/// combination of native workspaces being enabled for the workspace AND the
/// server-wide workspace role sync detachment feature being on (see
/// `Workspace::is_workspace_role_detachment_enabled`). Passing `false` here
/// exercises the exact state prod is in today: native workspaces can be
/// enabled for a workspace while roles are still synced 1:1, so the derived
/// field is `false`.
fn workspace_with_members(
    members: Vec<WorkspaceMember>,
    role_detachment_enabled: bool,
) -> Workspace {
    Workspace {
        uid: "workspace_uid123456789".to_string().into(),
        name: "Workspace".to_string(),
        stripe_customer_id: None,
        teams: Vec::new(),
        billing_metadata: Default::default(),
        bonus_grants_purchased_this_month: Default::default(),
        billing_cycle_usage: None,
        has_billing_history: false,
        settings: Default::default(),
        invite_code: None,
        invite_link_domain_restrictions: Vec::new(),
        pending_email_invites: Vec::new(),
        is_eligible_for_discovery: false,
        members,
        total_requests_used_since_last_refresh: 0,
        native_workspaces_role_detachment_enabled: role_detachment_enabled,
    }
}

fn item_for<'a>(items: &'a [Item], email: &str) -> &'a Item {
    items
        .iter()
        .find(|item| item.text == email)
        .unwrap_or_else(|| panic!("expected an item for {email}"))
}

#[test]
fn workspace_admin_badge_supersedes_team_owner_chip_when_roles_are_detached() {
    let owner_uid = UserUid::new("owner");
    let team = team_with_members(vec![TeamMember {
        uid: owner_uid,
        email: "owner@example.com".to_string(),
        role: MembershipRole::Owner,
    }]);
    let workspace = workspace_with_members(
        vec![workspace_member(
            owner_uid,
            "owner@example.com",
            MembershipRole::Admin,
        )],
        true,
    );

    let items = TeamsPageView::team_to_item_list(&team, "viewer@example.com", Some(&workspace));

    let owner_item = item_for(&items, "owner@example.com");
    assert_eq!(owner_item.state, ItemState::WorkspaceAdmin);
}

#[test]
fn workspace_owner_badge_shown_for_workspace_owner_role() {
    let member_uid = UserUid::new("member");
    let team = team_with_members(vec![TeamMember {
        uid: member_uid,
        email: "member@example.com".to_string(),
        role: MembershipRole::User,
    }]);
    let workspace = workspace_with_members(
        vec![workspace_member(
            member_uid,
            "member@example.com",
            MembershipRole::Owner,
        )],
        true,
    );

    let items = TeamsPageView::team_to_item_list(&team, "viewer@example.com", Some(&workspace));

    let member_item = item_for(&items, "member@example.com");
    assert_eq!(member_item.state, ItemState::WorkspaceOwner);
}

#[test]
fn workspace_admin_viewer_gets_team_admin_powers_when_roles_are_detached() {
    let admin_uid = UserUid::new("admin");
    let target_uid = UserUid::new("target");
    let team = team_with_members(vec![
        TeamMember {
            uid: admin_uid,
            email: "admin@example.com".to_string(),
            role: MembershipRole::User,
        },
        TeamMember {
            uid: target_uid,
            email: "target@example.com".to_string(),
            role: MembershipRole::User,
        },
    ]);
    let workspace = workspace_with_members(
        vec![workspace_member(
            admin_uid,
            "admin@example.com",
            MembershipRole::Admin,
        )],
        true,
    );

    let items = TeamsPageView::team_to_item_list(&team, "admin@example.com", Some(&workspace));

    let target_item = item_for(&items, "target@example.com");
    assert!(
        target_item
            .actions
            .iter()
            .any(|action| action.label == "Remove from team"),
        "workspace admin should be able to remove a team member: {target_item:?}"
    );
    assert!(
        target_item
            .actions
            .iter()
            .any(|action| action.label == "Promote to admin"),
        "workspace admin should be able to promote a team member: {target_item:?}"
    );
}

/// This is prod's state today: native workspaces can be enabled for a
/// workspace, but the server-wide workspace role sync detachment feature is
/// off, so roles are still mirrored 1:1 and the server reports
/// `nativeWorkspacesRoleDetachmentEnabled: false`. A workspace admin who is
/// not a team admin must get no extra powers and no badge in this state.
#[test]
fn workspace_admin_has_no_effect_when_roles_are_still_synced() {
    let admin_uid = UserUid::new("admin");
    let target_uid = UserUid::new("target");
    let team = team_with_members(vec![
        TeamMember {
            uid: admin_uid,
            email: "admin@example.com".to_string(),
            role: MembershipRole::User,
        },
        TeamMember {
            uid: target_uid,
            email: "target@example.com".to_string(),
            role: MembershipRole::User,
        },
    ]);
    let workspace = workspace_with_members(
        vec![workspace_member(
            admin_uid,
            "admin@example.com",
            MembershipRole::Admin,
        )],
        false,
    );

    let items = TeamsPageView::team_to_item_list(&team, "admin@example.com", Some(&workspace));

    let target_item = item_for(&items, "target@example.com");
    assert!(
        target_item.actions.is_empty(),
        "a non-team-admin viewer should get no member actions when roles are synced: {target_item:?}"
    );
    assert_eq!(target_item.state, ItemState::Valid);
}

/// A pure team admin's badge and powers are unaffected by this feature,
/// whether or not the workspace / an independent workspace role exist.
#[test]
fn pure_team_admin_is_unaffected() {
    let admin_uid = UserUid::new("admin");
    let team = team_with_members(vec![TeamMember {
        uid: admin_uid,
        email: "admin@example.com".to_string(),
        role: MembershipRole::Admin,
    }]);

    let items_without_workspace =
        TeamsPageView::team_to_item_list(&team, "viewer@example.com", None);
    let admin_item = item_for(&items_without_workspace, "admin@example.com");
    assert_eq!(admin_item.state, ItemState::Admin);
}
