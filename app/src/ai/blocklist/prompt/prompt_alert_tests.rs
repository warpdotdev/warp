use std::sync::Arc;

use ai::LLMProvider;
use warpui::App;

use super::*;
use crate::ai::credit_availability::AICreditSource;
use crate::auth::UserUid;
use crate::server::server_api::ServerApiProvider;
use crate::server::server_api::team::MockTeamClient;
use crate::server::server_api::workspace::MockWorkspaceClient;
use crate::server::telemetry::context_provider::AppTelemetryContextProvider;
use crate::workspaces::team::{DiscoverableTeam, MembershipRole, Team, TeamMember};
use crate::workspaces::user_workspaces::TeamlessScopeForTest;
use crate::workspaces::workspace::{
    ByoApiKeyPolicy, MultiAdminPolicy, NativeWorkspacesPolicy, Workspace, WorkspaceMember,
    WorkspaceMemberUsageInfo, WorkspaceUid,
};

const TEST_EMAIL: &str = "member@example.com";
const TEST_TEAM_UID: i64 = 42;

fn team_with_role(role: MembershipRole) -> Team {
    let mut team = Team::from_local_cache(
        ServerId::from(TEST_TEAM_UID),
        "Test Team".to_string(),
        None,
        None,
        Some(vec![TeamMember {
            uid: UserUid::new("team-member"),
            email: TEST_EMAIL.to_string(),
            role,
            is_disabled: false,
        }]),
        None,
    );
    team.billing_metadata.tier.multi_admin_policy = Some(MultiAdminPolicy { enabled: true });
    team
}

fn workspace_with_role(role: MembershipRole) -> Workspace {
    let mut workspace = Workspace::from_local_cache(
        WorkspaceUid::from(ServerId::from(7_i64)),
        "Test Workspace".to_string(),
        None,
        None,
    );
    workspace.billing_metadata.tier.native_workspaces_policy =
        Some(NativeWorkspacesPolicy { enabled: true });
    workspace.members = vec![WorkspaceMember {
        uid: UserUid::new("workspace-member"),
        email: TEST_EMAIL.to_string(),
        role,
        is_disabled: false,
        usage_info: WorkspaceMemberUsageInfo {
            is_unlimited: false,
            request_limit: 0,
            requests_used_since_last_refresh: 0,
            is_request_limit_prorated: false,
        },
    }];
    workspace
}

fn initialize_app(app: &mut App) {
    initialize_app_with_workspaces(app, vec![]);
}

fn initialize_app_with_workspaces(app: &mut App, workspaces: Vec<Workspace>) {
    app.add_singleton_model(|_| NetworkStatus::new());
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(|_| ServerApiProvider::new_for_test());
    app.add_singleton_model(AppTelemetryContextProvider::new_context_provider);
    app.add_singleton_model(|ctx| {
        UserWorkspaces::mock(
            Arc::new(MockTeamClient::new()),
            Arc::new(MockWorkspaceClient::new()),
            workspaces,
            ctx,
        )
    });
    if app
        .models_of_type::<settings::PrivatePreferences>()
        .is_empty()
    {
        app.update(crate::settings::init_and_register_user_preferences);
    }
    app.update(|ctx| {
        warpui_extras::secure_storage::register_noop("test", ctx);
        ctx.add_singleton_model(ApiKeyManager::new);
    });
    app.add_singleton_model(|_| crate::pricing::PricingInfoModel::new());
    app.add_singleton_model(|ctx| {
        AIRequestUsageModel::new_for_test(ServerApiProvider::as_ref(ctx).get_ai_client(), ctx)
    });
}

fn apply_server_availability(app: &mut App, availability: AICreditAvailability) {
    AIRequestUsageModel::handle(app).update(app, |model, ctx| {
        model.apply_server_availability(Ok(availability), ctx);
    });
}

fn determine_state(app: &mut App) -> PromptAlertState {
    app.read(|ctx| PromptAlertView::determine_state(&TeamlessScopeForTest, ctx))
}

#[test]
fn test_server_available_maps_to_no_alert() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        apply_server_availability(
            &mut app,
            AICreditAvailability::available_with_source(Some(AICreditSource::BaseLimit)),
        );
        assert_eq!(determine_state(&mut app), PromptAlertState::NoAlert);
    });
}

#[test]
fn test_server_delinquent_maps_to_delinquency_alert() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        apply_server_availability(
            &mut app,
            AICreditAvailability::unavailable(AICreditDenialReason::Delinquent),
        );
        assert_eq!(
            determine_state(&mut app),
            PromptAlertState::DelinquentDueToPaymentIssue
        );
    });
}

#[test]
fn test_server_spend_limit_reasons_preserve_scope() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        apply_server_availability(
            &mut app,
            AICreditAvailability::unavailable(AICreditDenialReason::EnterpriseTeamSpendLimitHit),
        );
        assert_eq!(
            determine_state(&mut app),
            PromptAlertState::EnterpriseTeamSpendLimitReached
        );

        apply_server_availability(
            &mut app,
            AICreditAvailability::unavailable(AICreditDenialReason::EnterprisePerUserSpendLimitHit),
        );
        assert_eq!(
            determine_state(&mut app),
            PromptAlertState::EnterpriseIndividualSpendLimitReached
        );
        apply_server_availability(
            &mut app,
            AICreditAvailability::unavailable(
                AICreditDenialReason::EnterprisePerUnassignedUserSpendLimitHit,
            ),
        );
        assert_eq!(
            determine_state(&mut app),
            PromptAlertState::EnterpriseUnassignedUserSpendLimitReached
        );

        apply_server_availability(
            &mut app,
            AICreditAvailability::unavailable(
                AICreditDenialReason::EnterpriseWorkspaceSpendLimitHit,
            ),
        );
        assert_eq!(
            determine_state(&mut app),
            PromptAlertState::EnterpriseWorkspaceSpendLimitReached
        );
    });
}

#[test]
fn test_spend_limit_presentation_identifies_scope() {
    assert_eq!(
        PromptAlertState::MonthlyOveragesSpendLimitReached.primary_text(),
        "You've reached your monthly spend limit"
    );
    assert_eq!(
        PromptAlertState::EnterpriseTeamSpendLimitReached.primary_text(),
        "You've reached your team's spend limit"
    );
    assert_eq!(
        PromptAlertState::EnterpriseIndividualSpendLimitReached.primary_text(),
        "You've reached the spend limit set for you"
    );
    assert_eq!(
        PromptAlertState::EnterpriseUnassignedUserSpendLimitReached.primary_text(),
        "Spend limit reached for members without a team"
    );
    assert_eq!(
        PromptAlertState::EnterpriseWorkspaceSpendLimitReached.primary_text(),
        "You've reached this workspace's spend limit"
    );
}

#[test]
fn test_spend_limit_tooltips_identify_scope() {
    assert_eq!(
        PromptAlertState::EnterpriseTeamSpendLimitReached.tooltip_text(),
        Some("You've reached your team's spend limit")
    );
    assert_eq!(
        PromptAlertState::EnterpriseIndividualSpendLimitReached.tooltip_text(),
        Some("You've reached the spend limit set for you")
    );
    assert_eq!(
        PromptAlertState::EnterpriseUnassignedUserSpendLimitReached.tooltip_text(),
        Some("Spend limit reached for members without a team")
    );
    assert_eq!(
        PromptAlertState::EnterpriseWorkspaceSpendLimitReached.tooltip_text(),
        Some("You've reached this workspace's spend limit")
    );
}

#[test]
fn test_team_spend_limit_cta_uses_selected_team_authority() {
    let workspace_member = workspace_with_role(MembershipRole::User);
    let workspace_admin = workspace_with_role(MembershipRole::Admin);
    let team_member = team_with_role(MembershipRole::User);
    let team_admin = team_with_role(MembershipRole::Admin);

    assert_eq!(
        enterprise_limit_cta(
            &PromptAlertState::EnterpriseTeamSpendLimitReached,
            Some(&workspace_member),
            Some(&team_member),
            Some(TEST_EMAIL),
        ),
        Some(vec![FormattedTextFragment::plain_text(
            ", contact a team admin"
        )])
    );
    assert_eq!(
        enterprise_limit_cta(
            &PromptAlertState::EnterpriseTeamSpendLimitReached,
            Some(&workspace_admin),
            Some(&team_member),
            Some(TEST_EMAIL),
        ),
        Some(vec![
            FormattedTextFragment::plain_text("  "),
            FormattedTextFragment::hyperlink(
                "Manage limit",
                AdminActions::admin_panel_link_for_team(ServerId::from(TEST_TEAM_UID)),
            ),
        ])
    );
    assert_eq!(
        enterprise_limit_cta(
            &PromptAlertState::EnterpriseTeamSpendLimitReached,
            Some(&workspace_member),
            Some(&team_admin),
            Some(TEST_EMAIL),
        ),
        Some(vec![
            FormattedTextFragment::plain_text("  "),
            FormattedTextFragment::hyperlink(
                "Manage limit",
                AdminActions::admin_panel_link_for_team(ServerId::from(TEST_TEAM_UID)),
            ),
        ])
    );
}

#[test]
fn test_individual_spend_limit_cta_uses_selected_team_authority() {
    let workspace_member = workspace_with_role(MembershipRole::User);
    let workspace_admin = workspace_with_role(MembershipRole::Admin);
    let team_member = team_with_role(MembershipRole::User);
    let team_admin = team_with_role(MembershipRole::Admin);

    assert_eq!(
        enterprise_limit_cta(
            &PromptAlertState::EnterpriseIndividualSpendLimitReached,
            Some(&workspace_member),
            Some(&team_member),
            Some(TEST_EMAIL),
        ),
        Some(vec![FormattedTextFragment::plain_text(
            ", contact a team admin"
        )])
    );
    assert_eq!(
        enterprise_limit_cta(
            &PromptAlertState::EnterpriseIndividualSpendLimitReached,
            Some(&workspace_admin),
            Some(&team_member),
            Some(TEST_EMAIL),
        ),
        Some(vec![
            FormattedTextFragment::plain_text("  "),
            FormattedTextFragment::hyperlink(
                "Manage limit",
                AdminActions::admin_panel_link_for_team(ServerId::from(TEST_TEAM_UID)),
            ),
        ])
    );
    assert_eq!(
        enterprise_limit_cta(
            &PromptAlertState::EnterpriseIndividualSpendLimitReached,
            Some(&workspace_member),
            Some(&team_admin),
            Some(TEST_EMAIL),
        ),
        Some(vec![
            FormattedTextFragment::plain_text("  "),
            FormattedTextFragment::hyperlink(
                "Manage limit",
                AdminActions::admin_panel_link_for_team(ServerId::from(TEST_TEAM_UID)),
            ),
        ])
    );
}

#[test]
fn test_workspace_spend_limit_cta_uses_workspace_authority() {
    let workspace_member = workspace_with_role(MembershipRole::User);
    let team_admin = team_with_role(MembershipRole::Admin);
    assert_eq!(
        enterprise_limit_cta(
            &PromptAlertState::EnterpriseWorkspaceSpendLimitReached,
            Some(&workspace_member),
            Some(&team_admin),
            Some(TEST_EMAIL),
        ),
        Some(vec![FormattedTextFragment::plain_text(
            ", contact a workspace admin"
        )])
    );

    let workspace_admin = workspace_with_role(MembershipRole::Admin);
    assert_eq!(
        enterprise_limit_cta(
            &PromptAlertState::EnterpriseWorkspaceSpendLimitReached,
            Some(&workspace_admin),
            Some(&team_admin),
            Some(TEST_EMAIL),
        ),
        Some(vec![
            FormattedTextFragment::plain_text("  "),
            FormattedTextFragment::hyperlink(
                "Manage limit",
                AdminActions::admin_panel_link_for_workspace(),
            ),
        ])
    );
}

#[test]
fn test_unassigned_user_spend_limit_cta_uses_workspace_authority() {
    let workspace_admin = workspace_with_role(MembershipRole::Admin);
    assert_eq!(
        enterprise_limit_cta(
            &PromptAlertState::EnterpriseUnassignedUserSpendLimitReached,
            Some(&workspace_admin),
            None,
            Some(TEST_EMAIL),
        ),
        Some(vec![
            FormattedTextFragment::plain_text("  "),
            FormattedTextFragment::hyperlink(
                "Manage limit",
                AdminActions::admin_panel_link_for_workspace(),
            ),
        ])
    );
}

#[test]
fn test_unassigned_user_spend_limit_cta_offers_admin_both_actions() {
    let mut workspace_admin = workspace_with_role(MembershipRole::Admin);
    workspace_admin.open_teams.push(DiscoverableTeam {
        team_uid: "0000000000000000000002".to_string(),
        num_members: 2,
        name: "Open Team".to_string(),
        team_accepting_invites: true,
    });

    let cta = enterprise_limit_cta(
        &PromptAlertState::EnterpriseUnassignedUserSpendLimitReached,
        Some(&workspace_admin),
        None,
        Some(TEST_EMAIL),
    )
    .expect("workspace admin should receive CTAs");
    assert_eq!(cta.len(), 4);
    assert_eq!(cta[0], FormattedTextFragment::plain_text("  "));
    assert_eq!(
        cta[1],
        FormattedTextFragment::hyperlink(
            "Manage limit",
            AdminActions::admin_panel_link_for_workspace(),
        )
    );
    assert_eq!(cta[2], FormattedTextFragment::plain_text(" or "));
    assert_eq!(cta[3].text, "join a team");
    let Some(markdown_parser::Hyperlink::Action(action)) = &cta[3].styles.hyperlink else {
        panic!("join CTA should dispatch an action");
    };
    assert!(matches!(
        action.as_any().downcast_ref::<WorkspaceAction>(),
        Some(WorkspaceAction::BrowseTeams)
    ));
}
#[test]
fn test_unassigned_user_spend_limit_cta_offers_open_teams() {
    let mut workspace_member = workspace_with_role(MembershipRole::User);
    workspace_member.open_teams.push(DiscoverableTeam {
        team_uid: "0000000000000000000002".to_string(),
        num_members: 2,
        name: "Open Team".to_string(),
        team_accepting_invites: true,
    });

    let cta = enterprise_limit_cta(
        &PromptAlertState::EnterpriseUnassignedUserSpendLimitReached,
        Some(&workspace_member),
        None,
        Some(TEST_EMAIL),
    )
    .expect("unassigned user should receive a CTA");
    assert_eq!(cta.len(), 2);
    assert_eq!(
        cta[0],
        FormattedTextFragment::plain_text("  Ask a workspace admin to increase it, or ")
    );
    assert_eq!(cta[1].text, "join a team");
    let Some(markdown_parser::Hyperlink::Action(action)) = &cta[1].styles.hyperlink else {
        panic!("join CTA should dispatch an action");
    };
    assert!(matches!(
        action.as_any().downcast_ref::<WorkspaceAction>(),
        Some(WorkspaceAction::BrowseTeams)
    ));
}

#[test]
fn test_unassigned_user_spend_limit_cta_asks_workspace_admin_without_open_teams() {
    let workspace_member = workspace_with_role(MembershipRole::User);
    assert_eq!(
        enterprise_limit_cta(
            &PromptAlertState::EnterpriseUnassignedUserSpendLimitReached,
            Some(&workspace_member),
            None,
            Some(TEST_EMAIL),
        ),
        Some(vec![FormattedTextFragment::plain_text(
            "  Ask a workspace admin to increase it"
        )])
    );
}
#[test]
fn test_server_out_of_credits_maps_to_request_limit_reached() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        // With no workspace overage policy in play, an out-of-credits denial
        // falls through to the generic request limit alert.
        for reason in [
            AICreditDenialReason::OutOfCredits,
            AICreditDenialReason::Unknown,
        ] {
            apply_server_availability(&mut app, AICreditAvailability::unavailable(reason));
            assert_eq!(
                determine_state(&mut app),
                PromptAlertState::RequestLimitReached,
                "unexpected alert state for {reason:?}",
            );
        }
    });
}

#[test]
fn test_local_fallback_used_before_first_server_response() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        // No server availability applied: the default request limit info has
        // requests remaining, so the legacy derivation reports no alert.
        assert_eq!(determine_state(&mut app), PromptAlertState::NoAlert);
    });
}

#[test]
fn test_server_managed_availability_maps_to_no_alert() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        // `available` with no credit source means a server-managed BYO path
        // is configured — definite availability, no local key required.
        apply_server_availability(&mut app, AICreditAvailability::available_with_source(None));
        assert_eq!(determine_state(&mut app), PromptAlertState::NoAlert);
    });
}

#[test]
fn test_out_of_credits_with_local_key_maps_to_no_alert() {
    App::test((), |mut app| async move {
        let uid = WorkspaceUid::from(crate::server::ids::ServerId::from(1_i64));
        let mut workspace =
            Workspace::from_local_cache(uid, "Test Workspace".to_string(), None, None);
        workspace.billing_metadata.tier.byo_api_key_policy =
            Some(ByoApiKeyPolicy { enabled: true });
        initialize_app_with_workspaces(&mut app, vec![workspace]);

        ApiKeyManager::handle(&app).update(&mut app, |manager, ctx| {
            manager.set_provider_key(LLMProvider::OpenAI, Some("test-key".to_string()), ctx);
        });

        // The server cannot see the locally stored key; the client refines
        // its OUT_OF_CREDITS answer.
        apply_server_availability(
            &mut app,
            AICreditAvailability::unavailable(AICreditDenialReason::OutOfCredits),
        );
        assert_eq!(determine_state(&mut app), PromptAlertState::NoAlert);
    });
}
