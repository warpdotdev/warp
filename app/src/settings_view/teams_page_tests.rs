use super::*;
use crate::workspaces::team::TeamMember;
use crate::workspaces::workspace::{EmailInvite, MultiAdminPolicy, Tier};

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
        invite_code: None,
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
    }
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
fn owner_can_transfer_promote_and_remove_regardless_of_workspace_admin_flag() {
    let team = team_with_members(
        vec![
            member(OWNER_EMAIL, MembershipRole::Owner),
            member(MEMBER_EMAIL, MembershipRole::User),
        ],
        true,
    );

    let items = TeamsPageView::team_to_item_list(&team, OWNER_EMAIL, false);

    assert_eq!(
        action_labels(&items, MEMBER_EMAIL),
        vec!["Transfer ownership", "Promote to admin", "Remove from team"]
    );
}

#[test]
fn team_admin_can_promote_and_remove_without_workspace_admin_flag() {
    let team = team_with_members(
        vec![
            member(ADMIN_EMAIL, MembershipRole::Admin),
            member(MEMBER_EMAIL, MembershipRole::User),
        ],
        true,
    );

    let items = TeamsPageView::team_to_item_list(&team, ADMIN_EMAIL, false);

    // No "Transfer ownership" -- that stays owner-only.
    assert_eq!(
        action_labels(&items, MEMBER_EMAIL),
        vec!["Promote to admin", "Remove from team"]
    );
}

#[test]
fn plain_member_without_workspace_admin_gets_no_member_actions() {
    let team = team_with_members(
        vec![
            member(MEMBER_EMAIL, MembershipRole::User),
            member("other@example.com", MembershipRole::User),
        ],
        true,
    );

    let items = TeamsPageView::team_to_item_list(&team, MEMBER_EMAIL, false);

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

    let items = TeamsPageView::team_to_item_list(&team, MEMBER_EMAIL, true);

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
fn workspace_admin_cannot_transfer_ownership() {
    let team = team_with_members(
        vec![
            member(MEMBER_EMAIL, MembershipRole::User),
            member(OWNER_EMAIL, MembershipRole::Owner),
        ],
        true,
    );

    let items = TeamsPageView::team_to_item_list(&team, MEMBER_EMAIL, true);

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

    let items = TeamsPageView::team_to_item_list(&team, MEMBER_EMAIL, true);

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

    let items = TeamsPageView::team_to_item_list(&team, MEMBER_EMAIL, true);

    assert!(action_labels(&items, MEMBER_EMAIL).is_empty());
}

#[test]
fn workspace_admin_flag_does_not_grant_pending_invite_cancellation() {
    let mut team = team_with_members(vec![member(MEMBER_EMAIL, MembershipRole::User)], true);
    team.pending_email_invites.push(EmailInvite {
        invitee_email: "invitee@example.com".to_string(),
        expired: false,
    });

    let items = TeamsPageView::team_to_item_list(&team, MEMBER_EMAIL, true);

    // Cancelling a pending invite remains gated on team-admin permissions;
    // it's out of scope for the workspace-admin override.
    assert!(action_labels(&items, "invitee@example.com").is_empty());
}
