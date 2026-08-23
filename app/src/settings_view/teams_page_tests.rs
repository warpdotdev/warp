use super::*;
use crate::workspaces::team::TeamMember;
use crate::workspaces::workspace::{
    EmailInvite, MultiAdminPolicy, NativeWorkspacesPolicy, Tier, WorkspaceMember,
    WorkspaceMemberUsageInfo,
};

fn member(email: &str, role: MembershipRole) -> TeamMember {
    TeamMember {
        uid: UserUid::new(email),
        email: email.to_string(),
        role,
    }
}

fn team_with_members(members: Vec<TeamMember>, multi_admin_enabled: bool) -> Team {
    Team {
        uid: 1.into(),
        name: "Test Team".to_string(),
        color: None,
        invite_link: None,
        members,
        pending_email_invites: vec![],
        invite_link_domain_restrictions: vec![],
        billing_metadata: BillingMetadata {
            tier: Tier {
                multi_admin_policy: Some(MultiAdminPolicy {
                    enabled: multi_admin_enabled,
                }),
                ..Default::default()
            },
            ..Default::default()
        },
        stripe_customer_id: None,
        settings: Default::default(),
        is_eligible_for_discovery: false,
        has_billing_history: false,
        visibility: Default::default(),
    }
}

fn workspace_with_member(
    email: &str,
    role: MembershipRole,
    native_workspaces_enabled: bool,
) -> Workspace {
    let mut workspace =
        Workspace::from_local_cache(ServerId::from(2).into(), "Test Workspace".to_string(), None);
    workspace.billing_metadata.tier.native_workspaces_policy = Some(NativeWorkspacesPolicy {
        enabled: native_workspaces_enabled,
    });
    workspace.members.push(WorkspaceMember {
        uid: UserUid::new(email),
        email: email.to_string(),
        role,
        usage_info: WorkspaceMemberUsageInfo {
            is_unlimited: true,
            request_limit: 0,
            requests_used_since_last_refresh: 0,
            is_request_limit_prorated: false,
        },
    });
    workspace
}

fn admin_workspace(email: &str) -> Workspace {
    workspace_with_member(email, MembershipRole::Admin, true)
}

/// Returns the action labels rendered for the item with the given `text` (a
/// member email or pending-invite email), in the order they were pushed.
fn action_labels(items: &[Item], text: &str) -> Vec<String> {
    items
        .iter()
        .find(|item| item.text == text)
        .map(|item| item.actions.iter().map(|a| a.label.clone()).collect())
        .unwrap_or_default()
}

const OWNER_EMAIL: &str = "owner@example.com";
const ADMIN_EMAIL: &str = "admin@example.com";
const MEMBER_EMAIL: &str = "member@example.com";

#[test]
fn owner_can_transfer_promote_and_remove_without_workspace_admin_role() {
    let team = team_with_members(
        vec![
            member(OWNER_EMAIL, MembershipRole::Owner),
            member(MEMBER_EMAIL, MembershipRole::User),
        ],
        true,
    );
    let workspace = workspace_with_member(OWNER_EMAIL, MembershipRole::User, true);

    let items = TeamsPageView::team_to_item_list(&team, OWNER_EMAIL, &workspace);

    assert_eq!(
        action_labels(&items, MEMBER_EMAIL),
        vec!["Transfer ownership", "Promote to admin", "Remove from team"]
    );
}

#[test]
fn team_admin_can_promote_and_remove_without_workspace_admin_role() {
    let team = team_with_members(
        vec![
            member(ADMIN_EMAIL, MembershipRole::Admin),
            member(MEMBER_EMAIL, MembershipRole::User),
        ],
        true,
    );
    let workspace = workspace_with_member(ADMIN_EMAIL, MembershipRole::User, true);

    let items = TeamsPageView::team_to_item_list(&team, ADMIN_EMAIL, &workspace);

    // No "Transfer ownership" -- that stays owner-only.
    assert_eq!(
        action_labels(&items, MEMBER_EMAIL),
        vec!["Promote to admin", "Remove from team"]
    );
}

#[test]
fn non_admin_workspace_member_gets_no_member_actions() {
    let team = team_with_members(
        vec![
            member(MEMBER_EMAIL, MembershipRole::User),
            member("other@example.com", MembershipRole::User),
        ],
        true,
    );
    let workspace = workspace_with_member(MEMBER_EMAIL, MembershipRole::User, true);

    let items = TeamsPageView::team_to_item_list(&team, MEMBER_EMAIL, &workspace);

    assert!(action_labels(&items, "other@example.com").is_empty());
}

#[test]
fn workspace_admin_without_team_role_can_promote_demote_and_remove() {
    let team = team_with_members(
        vec![
            member(MEMBER_EMAIL, MembershipRole::User),
            member("regular@example.com", MembershipRole::User),
            member(ADMIN_EMAIL, MembershipRole::Admin),
        ],
        true,
    );
    let workspace = admin_workspace(MEMBER_EMAIL);

    let items = TeamsPageView::team_to_item_list(&team, MEMBER_EMAIL, &workspace);

    assert_eq!(
        action_labels(&items, "regular@example.com"),
        vec!["Promote to admin", "Remove from team"]
    );
    assert_eq!(
        action_labels(&items, ADMIN_EMAIL),
        vec!["Demote from admin", "Remove from team"]
    );
}

#[test]
fn workspace_admin_without_native_workspaces_policy_can_manage_members() {
    let team = team_with_members(
        vec![
            member(MEMBER_EMAIL, MembershipRole::User),
            member("other@example.com", MembershipRole::User),
        ],
        true,
    );
    let workspace = workspace_with_member(MEMBER_EMAIL, MembershipRole::Admin, false);

    let items = TeamsPageView::team_to_item_list(&team, MEMBER_EMAIL, &workspace);

    assert_eq!(
        action_labels(&items, "other@example.com"),
        vec!["Promote to admin", "Remove from team"]
    );
}

#[test]
fn workspace_admin_cannot_transfer_ownership() {
    let team = team_with_members(
        vec![
            member(MEMBER_EMAIL, MembershipRole::User),
            member(OWNER_EMAIL, MembershipRole::Owner),
        ],
        true,
    );
    let workspace = admin_workspace(MEMBER_EMAIL);

    let items = TeamsPageView::team_to_item_list(&team, MEMBER_EMAIL, &workspace);

    // Ownership transfer stays gated on team-owner permissions only.
    assert!(action_labels(&items, OWNER_EMAIL).is_empty());
}

#[test]
fn workspace_admin_without_multi_admin_plan_can_remove_but_not_promote() {
    let team = team_with_members(
        vec![
            member(MEMBER_EMAIL, MembershipRole::User),
            member("regular@example.com", MembershipRole::User),
        ],
        false,
    );
    let workspace = admin_workspace(MEMBER_EMAIL);

    let items = TeamsPageView::team_to_item_list(&team, MEMBER_EMAIL, &workspace);

    // The multi-admin plan gate on promote/demote is unaffected by the
    // workspace-admin override.
    assert_eq!(
        action_labels(&items, "regular@example.com"),
        vec!["Remove from team"]
    );
}

#[test]
fn current_user_gets_no_actions_against_their_own_row_as_workspace_admin() {
    let team = team_with_members(
        vec![
            member(MEMBER_EMAIL, MembershipRole::User),
            member("other@example.com", MembershipRole::User),
        ],
        true,
    );
    let workspace = admin_workspace(MEMBER_EMAIL);

    let items = TeamsPageView::team_to_item_list(&team, MEMBER_EMAIL, &workspace);

    assert!(action_labels(&items, MEMBER_EMAIL).is_empty());
}

#[test]
fn non_native_workspace_keeps_create_team_ui() {
    let workspace = workspace_with_member(ADMIN_EMAIL, MembershipRole::Admin, false);

    assert_eq!(
        TeamsWidget::page_sections_for(Some(&workspace), Some(ADMIN_EMAIL), true),
        vec![
            TeamsPageSection::CreateTeam,
            TeamsPageSection::JoinTeams {
                header: OR_JOIN_TEAM_HEADER
            }
        ]
    );
    assert_eq!(
        TeamsWidget::page_sections_for(Some(&workspace), Some(ADMIN_EMAIL), false),
        vec![TeamsPageSection::CreateTeam]
    );
}

#[test]
fn unresolved_workspace_keeps_create_team_ui() {
    assert_eq!(
        TeamsWidget::page_sections_for(None, Some(MEMBER_EMAIL), false),
        vec![TeamsPageSection::CreateTeam]
    );
}

#[test]
fn native_workspace_admin_gets_admin_panel_cta() {
    let workspace = admin_workspace(ADMIN_EMAIL);

    assert_eq!(
        TeamsWidget::page_sections_for(Some(&workspace), Some(ADMIN_EMAIL), true),
        vec![
            TeamsPageSection::JoinTeams {
                header: JOIN_TEAM_HEADER
            },
            TeamsPageSection::AdminPanelCta
        ]
    );
    assert_eq!(
        TeamsWidget::page_sections_for(Some(&workspace), Some(ADMIN_EMAIL), false),
        vec![TeamsPageSection::AdminPanelCta]
    );
}

#[test]
fn native_workspace_member_gets_join_or_empty_state() {
    let workspace = workspace_with_member(MEMBER_EMAIL, MembershipRole::User, true);

    assert_eq!(
        TeamsWidget::page_sections_for(Some(&workspace), Some(MEMBER_EMAIL), true),
        vec![TeamsPageSection::JoinTeams {
            header: JOIN_TEAM_HEADER
        }]
    );
    assert_eq!(
        TeamsWidget::page_sections_for(Some(&workspace), Some(MEMBER_EMAIL), false),
        vec![TeamsPageSection::NoTeamsToJoin]
    );
}

#[test]
fn viewer_missing_from_the_workspace_roster_is_not_an_admin() {
    let workspace = admin_workspace(ADMIN_EMAIL);

    assert_eq!(
        TeamsWidget::page_sections_for(Some(&workspace), None, false),
        vec![TeamsPageSection::NoTeamsToJoin]
    );
}

#[test]
fn workspace_admin_can_cancel_pending_invite() {
    let mut team = team_with_members(vec![member(MEMBER_EMAIL, MembershipRole::User)], true);
    team.pending_email_invites.push(EmailInvite {
        invitee_email: "invitee@example.com".to_string(),
        expired: false,
        team_uid: Some(1.into()),
    });
    let workspace = admin_workspace(MEMBER_EMAIL);

    let items = TeamsPageView::team_to_item_list(&team, MEMBER_EMAIL, &workspace);

    assert_eq!(
        action_labels(&items, "invitee@example.com"),
        vec!["Cancel invite"]
    );
}
