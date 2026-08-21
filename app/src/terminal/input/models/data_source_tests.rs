use std::sync::Arc;

use warpui::platform::WindowStyle;
use warpui::{App, Element, Entity, TypedActionView, View, ViewHandle, WindowId};

use super::*;
use crate::LaunchMode;
use crate::ai::execution_profiles::profiles::AIExecutionProfilesModel;
use crate::ai::mcp::TemplatableMCPServerManager;
use crate::auth::AuthStateProvider;
use crate::auth::auth_manager::AuthManager;
use crate::cloud_object::model::persistence::CloudModel;
use crate::network::NetworkStatus;
use crate::server::cloud_objects::update_manager::UpdateManager;
use crate::server::server_api::ServerApiProvider;
use crate::server::server_api::team::MockTeamClient;
use crate::server::server_api::workspace::MockWorkspaceClient;
use crate::server::sync_queue::SyncQueue;
use crate::settings::PrivacySettings;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::workspaces::team::Team;
use crate::workspaces::team_tester::TeamTesterStatus;
use crate::workspaces::workspace::{
    ByoFirstPartyKey, ManagedByokByoePolicy, TeamByoSettings, Workspace,
};

fn team_for_test(uid: i64, name: &str) -> Team {
    Team {
        uid: uid.into(),
        name: name.to_string(),
        color: None,
        invite_link: None,
        members: vec![],
        pending_email_invites: vec![],
        invite_link_domain_restrictions: vec![],
        billing_metadata: Default::default(),
        stripe_customer_id: None,
        settings: Default::default(),
        is_eligible_for_discovery: false,
        has_billing_history: false,
        visibility: crate::workspaces::team::TeamVisibility::Open,
    }
}

/// Two teams with opposing `team_byo` policy, mirroring the already-verified fixture in
/// `credential_source_for_model_differs_by_window_team_policy` (`ai/llms_tests.rs`): team A has
/// a team-provided Anthropic key; team B has no `team_byo` at all, so once the workspace's
/// managed-BYOK/BYOE plan entitlement is on, it has neither a team-provided key nor member keys.
fn workspace_with_two_teams_of_opposing_byo_policy() -> (Team, Team, Workspace) {
    let mut team_a = team_for_test(111, "team-a");
    team_a.settings.team_byo = Some(TeamByoSettings {
        first_party_enabled: true,
        endpoints_enabled: false,
        allow_user_keys: false,
        allow_user_endpoints: false,
        first_party_keys: vec![ByoFirstPartyKey {
            provider: LLMProvider::Anthropic,
            credential_uid: "cred-a".to_string(),
        }],
        endpoints: vec![],
    });
    let team_b = team_for_test(222, "team-b");

    let mut workspace = Workspace {
        uid: "workspace_uid123456789".to_string().into(),
        name: "test".to_string(),
        stripe_customer_id: None,
        teams: vec![team_a.clone(), team_b.clone()],
        billing_metadata: Default::default(),
        bonus_grants_purchased_this_month: Default::default(),
        billing_cycle_usage: None,
        has_billing_history: false,
        settings: Default::default(),
        invite_link_domain_restrictions: vec![],
        pending_email_invites: vec![],
        is_eligible_for_discovery: false,
        members: vec![],
        total_requests_used_since_last_refresh: 0,
    };
    workspace.billing_metadata.tier.managed_byok_byoe_policy =
        Some(ManagedByokByoePolicy { enabled: true });
    (team_a, team_b, workspace)
}

fn anthropic_llm() -> LLMInfo {
    LLMInfo {
        display_name: "Claude".to_string(),
        base_model_name: "Claude".to_string(),
        id: "claude-opus".into(),
        reasoning_level: None,
        usage_metadata: crate::ai::llms::LLMUsageMetadata {
            request_multiplier: 1,
            credit_multiplier: None,
        },
        description: None,
        disable_reason: None,
        vision_supported: false,
        spec: None,
        provider: LLMProvider::Anthropic,
        host_configs: Default::default(),
        discount_percentage: None,
        context_window: Default::default(),
    }
}

#[derive(Default)]
struct WindowTestView;

impl Entity for WindowTestView {
    type Event = ();
}

impl View for WindowTestView {
    fn ui_name() -> &'static str {
        "WindowTestView"
    }

    fn render(&self, _: &AppContext) -> Box<dyn Element> {
        warpui::elements::Empty::new().finish()
    }
}

impl TypedActionView for WindowTestView {
    type Action = ();
}

fn create_test_window(app: &mut App) -> WindowId {
    let (window_id, _view): (WindowId, ViewHandle<WindowTestView>) =
        app.add_window(WindowStyle::NotStealFocus, |_| WindowTestView);
    window_id
}

/// Regression for a `ModelSelectorDataSource` left resolving the *source* window's team
/// after its owning `InlineModelSelectorView` is transferred to another window (cross-window
/// tab drag): `set_window_id` (called from `InlineModelSelectorView::on_window_transferred`)
/// must actually change which team's `team_byo` policy the credential-source computation
/// downstream of `window_id` (`ModelSearchItem::new`, via `byo_key_source_for_model`) uses.
#[test]
fn set_window_id_changes_the_team_used_for_credential_source_resolution() {
    let (team_a, team_b, workspace) = workspace_with_two_teams_of_opposing_byo_policy();

    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        app.add_singleton_model(|_| ServerApiProvider::new_for_test());
        app.add_singleton_model(|_| AuthStateProvider::new_for_test());
        app.add_singleton_model(AuthManager::new_for_test);
        app.add_singleton_model(|_| NetworkStatus::new());
        app.add_singleton_model(PrivacySettings::mock);
        app.add_singleton_model(|ctx| {
            UserWorkspaces::mock(
                Arc::new(MockTeamClient::new()),
                Arc::new(MockWorkspaceClient::new()),
                vec![workspace],
                ctx,
            )
        });
        app.add_singleton_model(CloudModel::mock);
        app.add_singleton_model(TeamTesterStatus::mock);
        app.add_singleton_model(SyncQueue::mock);
        app.add_singleton_model(UpdateManager::mock);
        app.add_singleton_model(|_| TemplatableMCPServerManager::default());
        app.add_singleton_model(|ctx| {
            AIExecutionProfilesModel::new(&LaunchMode::new_for_unit_test(), ctx)
        });
        app.add_singleton_model(LLMPreferences::new);

        let window_a = create_test_window(&mut app);
        let window_b = create_test_window(&mut app);
        UserWorkspaces::handle(&app).update(&mut app, |user_workspaces, ctx| {
            user_workspaces.set_team_for_window(window_a, team_a.uid, ctx);
            user_workspaces.set_team_for_window(window_b, team_b.uid, ctx);
        });

        let data_source =
            app.add_model(|_| ModelSelectorDataSource::new(EntityId::new(), window_a, None));

        let llm = anthropic_llm();
        let choice = || ModelPickerChoice {
            llm: llm.clone(),
            disable_reason: None,
            name_match_result: None,
            score: OrderedFloat(0.),
        };
        let active_llm_id = llm.id.clone();

        app.read(|ctx| {
            // Before any transfer: resolves team A's team-provided-key policy.
            let team_uid_before =
                UserWorkspaces::as_ref(ctx).team_uid_for_window(data_source.as_ref(ctx).window_id);
            assert_eq!(team_uid_before, Some(team_a.uid));
            let item_a =
                ModelSearchItem::new(choice(), &active_llm_id, window_a, team_uid_before, ctx);
            assert_eq!(
                item_a.byo_key_source,
                Some(ByoKeySource::TeamProvided),
                "team A's first-party key should surface as the credential source"
            );
        });

        // Simulate what `InlineModelSelectorView::on_window_transferred` does after a
        // cross-window tab drag lands this data source's owning view in window B.
        data_source.update(&mut app, |ds, _| ds.set_window_id(window_b));

        app.read(|ctx| {
            let team_uid_after =
                UserWorkspaces::as_ref(ctx).team_uid_for_window(data_source.as_ref(ctx).window_id);
            assert_eq!(
                team_uid_after,
                Some(team_b.uid),
                "set_window_id must update which window's team is resolved"
            );
            let item_b =
                ModelSearchItem::new(choice(), &active_llm_id, window_b, team_uid_after, ctx);
            assert_eq!(
                item_b.byo_key_source, None,
                "after the transfer, credential-source resolution must use team B's \
                 restrictive policy, not team A's stale one"
            );
        });
    });
}
