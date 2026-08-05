use std::sync::Arc;

use warpui::App;

use super::*;
use crate::auth::user_uid::TEST_USER_EMAIL;
use crate::server::ids::ServerId;
use crate::server::server_api::ServerApiProvider;
use crate::server::server_api::team::MockTeamClient;
use crate::server::server_api::workspace::MockWorkspaceClient;
use crate::workspaces::team::{MembershipRole, Team, TeamMember};
use crate::workspaces::workspace::{
    CustomerType, DelinquencyStatus, UsageBasedPricingPolicy, Workspace, WorkspaceUid,
};

const ANOTHER_USER_EMAIL: &str = "someone_else@warp.dev";

fn initialize_app(app: &mut App, workspaces: Vec<Workspace>) {
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(|_| ServerApiProvider::new_for_test());
    app.add_singleton_model(|ctx| {
        UserWorkspaces::mock(
            Arc::new(MockTeamClient::new()),
            Arc::new(MockWorkspaceClient::new()),
            workspaces,
            ctx,
        )
    });
}

fn team_member(email: &str, role: MembershipRole) -> TeamMember {
    TeamMember {
        uid: Default::default(),
        email: email.to_owned(),
        role,
    }
}

/// A workspace whose sole team lists the test user with `role`.
fn workspace_with_team(role: MembershipRole) -> Workspace {
    let mut team = Team::from_local_cache(
        ServerId::from(7_i64),
        "Test Team".to_owned(),
        None,
        None,
        Some(vec![
            team_member(TEST_USER_EMAIL, role),
            team_member(ANOTHER_USER_EMAIL, MembershipRole::Owner),
        ]),
    );
    team.billing_metadata.customer_type = CustomerType::Free;
    Workspace::from_local_cache(
        WorkspaceUid::from(ServerId::from(1_i64)),
        "Test Workspace".to_owned(),
        Some(vec![team]),
    )
}

fn guidance(app: &mut App) -> BillingDenialGuidance {
    app.read(billing_denial_guidance)
}

#[test]
fn teamless_user_administers_their_own_billing_and_needs_no_extra_step() {
    App::test((), |mut app| async move {
        initialize_app(&mut app, vec![]);
        assert_eq!(
            guidance(&mut app),
            BillingDenialGuidance {
                kind: BillingDenialKind::RequestLimitReached,
                is_admin: true,
                next_step: None,
            }
        );
    });
}

#[test]
fn team_member_out_of_credits_is_pointed_at_an_admin() {
    App::test((), |mut app| async move {
        initialize_app(&mut app, vec![workspace_with_team(MembershipRole::User)]);
        let guidance = guidance(&mut app);
        assert_eq!(guidance.kind, BillingDenialKind::RequestLimitReached);
        assert!(!guidance.is_admin);
        assert_eq!(guidance.next_step, Some(MEMBER_UPGRADE_NEXT_STEP));
    });
}

#[test]
fn team_owner_out_of_credits_keeps_the_plain_upgrade_call_to_action() {
    App::test((), |mut app| async move {
        initialize_app(&mut app, vec![workspace_with_team(MembershipRole::Owner)]);
        let guidance = guidance(&mut app);
        assert!(guidance.is_admin);
        assert_eq!(guidance.next_step, None);
    });
}

#[test]
fn delinquent_billing_is_reported_separately_per_role() {
    App::test((), |mut app| async move {
        let mut member_workspace = workspace_with_team(MembershipRole::User);
        member_workspace.billing_metadata.delinquency_status = DelinquencyStatus::PastDue;
        initialize_app(&mut app, vec![member_workspace]);
        assert_eq!(
            guidance(&mut app),
            BillingDenialGuidance {
                kind: BillingDenialKind::DelinquentDueToPaymentIssue,
                is_admin: false,
                next_step: Some(MEMBER_RESOLVE_PAYMENT_ISSUE_NEXT_STEP),
            }
        );
    });

    App::test((), |mut app| async move {
        let mut admin_workspace = workspace_with_team(MembershipRole::Owner);
        admin_workspace.billing_metadata.delinquency_status = DelinquencyStatus::Unpaid;
        initialize_app(&mut app, vec![admin_workspace]);
        assert_eq!(
            guidance(&mut app),
            BillingDenialGuidance {
                kind: BillingDenialKind::DelinquentDueToPaymentIssue,
                is_admin: true,
                next_step: Some(ADMIN_RESOLVE_PAYMENT_ISSUE_NEXT_STEP),
            }
        );
    });
}

#[test]
fn toggleable_overages_that_are_off_ask_the_right_person_to_enable_them() {
    App::test((), |mut app| async move {
        let mut workspace = workspace_with_team(MembershipRole::User);
        workspace.billing_metadata.tier.usage_based_pricing_policy =
            Some(UsageBasedPricingPolicy { toggleable: true });
        initialize_app(&mut app, vec![workspace]);
        let guidance = guidance(&mut app);
        assert_eq!(
            guidance.kind,
            BillingDenialKind::OveragesToggleableButNotEnabled
        );
        assert_eq!(guidance.next_step, Some(MEMBER_ENABLE_OVERAGES_NEXT_STEP));
    });

    App::test((), |mut app| async move {
        let mut workspace = workspace_with_team(MembershipRole::Owner);
        workspace.billing_metadata.tier.usage_based_pricing_policy =
            Some(UsageBasedPricingPolicy { toggleable: true });
        initialize_app(&mut app, vec![workspace]);
        assert_eq!(
            guidance(&mut app).next_step,
            Some(ADMIN_ENABLE_OVERAGES_NEXT_STEP)
        );
    });
}

#[test]
fn an_exhausted_spend_limit_asks_the_right_person_to_raise_it() {
    App::test((), |mut app| async move {
        let mut workspace = workspace_with_team(MembershipRole::User);
        workspace.billing_metadata.tier.usage_based_pricing_policy =
            Some(UsageBasedPricingPolicy { toggleable: true });
        workspace.settings.usage_based_pricing_settings.enabled = true;
        initialize_app(&mut app, vec![workspace]);
        let guidance = guidance(&mut app);
        assert_eq!(
            guidance.kind,
            BillingDenialKind::MonthlyOveragesSpendLimitReached
        );
        assert_eq!(guidance.next_step, Some(MEMBER_RAISE_SPEND_LIMIT_NEXT_STEP));
    });

    App::test((), |mut app| async move {
        let mut workspace = workspace_with_team(MembershipRole::Owner);
        workspace.billing_metadata.tier.usage_based_pricing_policy =
            Some(UsageBasedPricingPolicy { toggleable: true });
        workspace.settings.usage_based_pricing_settings.enabled = true;
        initialize_app(&mut app, vec![workspace]);
        assert_eq!(
            guidance(&mut app).next_step,
            Some(ADMIN_RAISE_SPEND_LIMIT_NEXT_STEP)
        );
    });
}

#[test]
fn plans_with_no_upgrade_path_are_sent_to_support() {
    App::test((), |mut app| async move {
        let mut workspace = workspace_with_team(MembershipRole::Owner);
        workspace.billing_metadata.customer_type = CustomerType::Enterprise;
        initialize_app(&mut app, vec![workspace]);
        assert_eq!(
            guidance(&mut app).next_step,
            Some(CONTACT_SUPPORT_NEXT_STEP)
        );
    });
}
