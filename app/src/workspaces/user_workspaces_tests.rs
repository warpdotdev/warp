use crate::ai::llms::LLMModelHost;
use crate::auth::AuthManager;
use crate::cloud_object::model::persistence::CloudModel;
use crate::features::FeatureFlag;
use crate::network::NetworkStatus;
use crate::server::cloud_objects::update_manager::UpdateManager;
use crate::server::ids::ClientId;
use crate::server::server_api::team::{MockTeamClient, TeamClient};
use crate::server::server_api::ServerApiProvider;
use crate::server::sync_queue::SyncQueue;
use crate::server::telemetry::context_provider::AppTelemetryContextProvider;
use crate::settings::{AISettings, CodeSettings, FocusedTerminalInfo};
use crate::system::SystemStats;
use crate::workflows::workflow::Workflow;
use crate::workflows::{CloudWorkflow, CloudWorkflowModel};
use crate::workspaces::team::Team;
use crate::workspaces::team_tester::TeamTesterStatus;
use crate::workspaces::update_manager::TeamUpdateManager;
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::workspaces::workspace::{
    AdminEnablementSetting, CodebaseContextSettings, CustomerType, HostEnablementSetting,
    LlmHostSettings, Workspace,
};

use mockall::Sequence;
use settings::{PrivatePreferences, PublicPreferences};
use std::time::Duration;
use warpui::{AddSingletonModel, App};
use warpui_extras::user_preferences;

use super::*;

#[derive(Default)]
struct CachedResources {
    workspaces: Vec<Workspace>,
}
#[derive(Clone, Copy)]
enum AuthTestState {
    LoggedIn,
    LoggedOut,
}

fn initialize_app(
    app: &mut App,
    resources: CachedResources,
    team_client: Arc<dyn TeamClient>,
    workspace_client: Arc<dyn WorkspaceClient>,
) {
    initialize_app_with_auth_state(
        app,
        resources,
        team_client,
        workspace_client,
        AuthTestState::LoggedIn,
    );
}

fn initialize_app_with_auth_state(
    app: &mut App,
    resources: CachedResources,
    team_client: Arc<dyn TeamClient>,
    workspace_client: Arc<dyn WorkspaceClient>,
    auth_state: AuthTestState,
) {
    // Add the necessary singleton models to the App
    app.add_singleton_model(|_| NetworkStatus::new());
    app.add_singleton_model(|_| SystemStats::new());
    app.add_singleton_model(TeamTesterStatus::new);
    app.add_singleton_model(SyncQueue::mock);
    app.add_singleton_model(CloudModel::mock);
    app.add_singleton_model(|ctx| {
        UserWorkspaces::mock(
            team_client.clone(),
            workspace_client.clone(),
            resources.workspaces,
            ctx,
        )
    });
    app.add_singleton_model(|ctx| TeamUpdateManager::new(team_client.clone(), None, ctx));
    app.add_singleton_model(UpdateManager::mock);
    app.add_singleton_model(PrivacySettings::mock);
    app.add_singleton_model(|_| ServerApiProvider::new_for_test());
    app.add_singleton_model(move |_| match auth_state {
        AuthTestState::LoggedIn => AuthStateProvider::new_for_test(),
        AuthTestState::LoggedOut => AuthStateProvider::new_logged_out_for_test(),
    });
    app.add_singleton_model(AuthManager::new_for_test);
    app.add_singleton_model(AppTelemetryContextProvider::new_context_provider);
    app.add_singleton_model(|_| {
        PublicPreferences::new(Box::<user_preferences::in_memory::InMemoryPreferences>::default())
    });
    app.add_singleton_model(|_| {
        PrivatePreferences::new(Box::<user_preferences::in_memory::InMemoryPreferences>::default())
    });

    app.add_singleton_model(CodeSettings::new_with_defaults);
    app.add_singleton_model(AISettings::new_with_defaults);
    app.add_singleton_model(FocusedTerminalInfo::new);

    // The start of polling is normally triggered by authentication completion, but
    // we need to do it manually for tests.
    TeamTesterStatus::handle(app).update(app, |team_tester, ctx| {
        team_tester.initiate_data_pollers(false, ctx);
    });
}

#[test]
fn test_loading_all_spaces_after_switching_from_offline() {
    let _flag = FeatureFlag::KnowledgeSidebar.override_enabled(true);

    let team = Team {
        uid: 123.into(),
        name: "test".to_string(),
        invite_code: None,
        members: vec![],
        pending_email_invites: vec![],
        invite_link_domain_restrictions: vec![],
        billing_metadata: Default::default(),
        stripe_customer_id: None,
        organization_settings: Default::default(),
        is_eligible_for_discovery: false,
        has_billing_history: false,
    };

    let workspace = Workspace {
        uid: "workspace_uid123456789".to_string().into(),
        name: "test".to_string(),
        stripe_customer_id: None,
        teams: vec![team.clone()],
        billing_metadata: Default::default(),
        bonus_grants_purchased_this_month: Default::default(),
        billing_cycle_usage: None,
        has_billing_history: false,
        settings: Default::default(),
        invite_code: None,
        invite_link_domain_restrictions: vec![],
        pending_email_invites: vec![],
        is_eligible_for_discovery: false,
        members: vec![],
        total_requests_used_since_last_refresh: 0,
    };

    App::test((), |mut app| async move {
        // Sequences used for ordering requests (so first call will return something different than
        // next etc.)
        let mut team_sequence = Sequence::new();

        // Lets start by initializing the server api mock
        let mut team_client = MockTeamClient::new();

        // On first call to workspaces_metadata we return no workspaces (and expect it to be called just once)
        team_client
            .expect_workspaces_metadata()
            .times(1)
            .in_sequence(&mut team_sequence)
            .returning(|| {
                Ok(WorkspacesMetadataWithPricing {
                    metadata: WorkspacesMetadataResponse {
                        workspaces: vec![],
                        joinable_teams: vec![],
                        experiments: None,
                        feature_model_choices: None,
                    },
                    pricing_info: None,
                })
            });

        // Second call will return list of teams (one team specifically) and we also expect only 1
        team_client
            .expect_workspaces_metadata()
            .times(1)
            .in_sequence(&mut team_sequence)
            .returning(move || {
                Ok(WorkspacesMetadataWithPricing {
                    metadata: WorkspacesMetadataResponse {
                        workspaces: vec![workspace.clone()],
                        joinable_teams: vec![],
                        experiments: None,
                        feature_model_choices: None,
                    },
                    pricing_info: None,
                })
            });

        initialize_app(
            &mut app,
            CachedResources { workspaces: vec![] },
            Arc::new(team_client),
            Arc::new(MockWorkspaceClient::new()),
        );

        // We also ensure that UserWorkspaces stores no teams.
        UserWorkspaces::handle(&app).read(&app, |teams, _| {
            assert!(!teams.has_teams());
        });

        // Spend time waiting for the initial load to finish etc.
        warpui::r#async::Timer::after(Duration::from_secs(1)).await;

        // Lets go offline
        NetworkStatus::handle(&app).update(&mut app, |network_status, ctx| {
            network_status.reachability_changed(false, ctx);
        });

        // Lets go back online
        NetworkStatus::handle(&app).update(&mut app, |network_status, ctx| {
            network_status.reachability_changed(true, ctx);
        });

        // Spend time waiting for the load to finish etc.
        warpui::r#async::Timer::after(Duration::from_secs(1)).await;

        // We also ensure that UserWorkspaces stores a team
        UserWorkspaces::handle(&app).read(&app, |teams, _| {
            assert!(teams.has_teams());
        });
    })
}

#[test]
fn test_codebase_context_enabled_with_no_workspace() {
    App::test((), |mut app| async move {
        initialize_app(
            &mut app,
            CachedResources { workspaces: vec![] },
            Arc::new(MockTeamClient::new()),
            Arc::new(MockWorkspaceClient::new()),
        );

        app.read(|ctx| {
            let codebase_context_enabled =
                UserWorkspaces::as_ref(ctx).is_codebase_context_enabled(ctx);
            assert!(
                codebase_context_enabled,
                "codebase context should be on by default"
            );
        });
    })
}

fn team_for_test() -> Team {
    Team {
        uid: 123.into(),
        name: "test".to_string(),
        invite_code: None,
        members: vec![],
        pending_email_invites: vec![],
        invite_link_domain_restrictions: vec![],
        billing_metadata: Default::default(),
        stripe_customer_id: None,
        organization_settings: Default::default(),
        is_eligible_for_discovery: false,
        has_billing_history: false,
    }
}

#[test]
fn test_aws_bedrock_credentials_default_off_when_admin_respects_user_setting() {
    let team = team_for_test();
    let mut workspace = workspace_for_test(&team);
    workspace.settings.llm_settings.enabled = true;
    workspace.settings.llm_settings.host_configs.insert(
        LLMModelHost::AwsBedrock,
        LlmHostSettings {
            enabled: true,
            enablement_setting: HostEnablementSetting::RespectUserSetting,
        },
    );

    App::test((), |mut app| async move {
        initialize_app(
            &mut app,
            CachedResources {
                workspaces: vec![workspace],
            },
            Arc::new(MockTeamClient::new()),
            Arc::new(MockWorkspaceClient::new()),
        );

        app.read(|ctx| {
            assert!(
                !UserWorkspaces::as_ref(ctx).is_aws_bedrock_credentials_enabled(ctx),
                "respect-user-setting should default the local Bedrock credentials toggle to off"
            );
            assert!(
                UserWorkspaces::as_ref(ctx).is_aws_bedrock_credentials_toggleable(),
                "respect-user-setting should leave the local Bedrock credentials toggle editable"
            );
        });
    })
}

#[test]
fn test_aws_bedrock_credentials_respect_user_setting() {
    let team = team_for_test();
    let mut workspace = workspace_for_test(&team);
    workspace.settings.llm_settings.enabled = true;
    workspace.settings.llm_settings.host_configs.insert(
        LLMModelHost::AwsBedrock,
        LlmHostSettings {
            enabled: true,
            enablement_setting: HostEnablementSetting::RespectUserSetting,
        },
    );
    let mut team_client = MockTeamClient::new();
    let workspace_for_poll = workspace.clone();
    team_client.expect_workspaces_metadata().returning(move || {
        Ok(WorkspacesMetadataWithPricing {
            metadata: WorkspacesMetadataResponse {
                workspaces: vec![workspace_for_poll.clone()],
                joinable_teams: vec![],
                experiments: None,
                feature_model_choices: None,
            },
            pricing_info: None,
        })
    });

    App::test((), |mut app| async move {
        initialize_app(
            &mut app,
            CachedResources {
                workspaces: vec![workspace],
            },
            Arc::new(team_client),
            Arc::new(MockWorkspaceClient::new()),
        );

        AISettings::handle(&app).update(&mut app, |settings, ctx| {
            let _ = settings
                .aws_bedrock_credentials_enabled
                .set_value(false, ctx);
        });

        app.read(|ctx| {
            assert!(
                !UserWorkspaces::as_ref(ctx).is_aws_bedrock_credentials_enabled(ctx),
                "respect-user-setting should honor the local Bedrock credentials toggle"
            );
            assert!(
                UserWorkspaces::as_ref(ctx).is_aws_bedrock_credentials_toggleable(),
                "respect-user-setting should leave the local Bedrock credentials toggle editable"
            );
        });
    })
}

#[test]
fn test_aws_bedrock_credentials_enforced_by_admin() {
    let team = team_for_test();
    let mut workspace = workspace_for_test(&team);
    workspace.settings.llm_settings.enabled = true;
    workspace.settings.llm_settings.host_configs.insert(
        LLMModelHost::AwsBedrock,
        LlmHostSettings {
            enabled: true,
            enablement_setting: HostEnablementSetting::Enforce,
        },
    );
    let mut team_client = MockTeamClient::new();
    let workspace_for_poll = workspace.clone();
    team_client.expect_workspaces_metadata().returning(move || {
        Ok(WorkspacesMetadataWithPricing {
            metadata: WorkspacesMetadataResponse {
                workspaces: vec![workspace_for_poll.clone()],
                joinable_teams: vec![],
                experiments: None,
                feature_model_choices: None,
            },
            pricing_info: None,
        })
    });

    App::test((), |mut app| async move {
        initialize_app(
            &mut app,
            CachedResources {
                workspaces: vec![workspace],
            },
            Arc::new(team_client),
            Arc::new(MockWorkspaceClient::new()),
        );

        AISettings::handle(&app).update(&mut app, |settings, ctx| {
            let _ = settings
                .aws_bedrock_credentials_enabled
                .set_value(false, ctx);
        });

        app.read(|ctx| {
            assert!(
                UserWorkspaces::as_ref(ctx).is_aws_bedrock_credentials_enabled(ctx),
                "enforced Bedrock host policy should ignore the local Bedrock credentials toggle"
            );
            assert!(
                !UserWorkspaces::as_ref(ctx).is_aws_bedrock_credentials_toggleable(),
                "enforced Bedrock host policy should disable the local Bedrock credentials toggle"
            );
        });
    })
}

fn workspace_for_test(team: &Team) -> Workspace {
    Workspace {
        uid: "workspace_uid123456789".to_string().into(),
        name: "test".to_string(),
        stripe_customer_id: None,
        teams: vec![team.clone()],
        billing_metadata: Default::default(),
        bonus_grants_purchased_this_month: Default::default(),
        billing_cycle_usage: None,
        has_billing_history: false,
        settings: Default::default(),
        invite_code: None,
        invite_link_domain_restrictions: vec![],
        pending_email_invites: vec![],
        is_eligible_for_discovery: false,
        members: vec![],
        total_requests_used_since_last_refresh: 0,
    }
}

fn mock_team_client_returning_workspaces(workspaces: Vec<Workspace>) -> MockTeamClient {
    let mut team_client = MockTeamClient::new();
    team_client.expect_workspaces_metadata().returning(move || {
        Ok(WorkspacesMetadataWithPricing {
            metadata: WorkspacesMetadataResponse {
                workspaces: workspaces.clone(),
                joinable_teams: vec![],
                experiments: None,
                feature_model_choices: None,
            },
            pricing_info: None,
        })
    });
    team_client
}

#[test]
fn test_custom_inference_enabled_for_non_enterprise_workspace() {
    let mut team = team_for_test();
    team.billing_metadata.customer_type = CustomerType::Free;
    let mut workspace = workspace_for_test(&team);
    workspace.billing_metadata.customer_type = CustomerType::Free;

    App::test((), |mut app| async move {
        let team_client = mock_team_client_returning_workspaces(vec![workspace.clone()]);
        initialize_app(
            &mut app,
            CachedResources {
                workspaces: vec![workspace],
            },
            Arc::new(team_client),
            Arc::new(MockWorkspaceClient::new()),
        );

        app.read(|ctx| {
            assert!(
                UserWorkspaces::as_ref(ctx).is_custom_inference_enabled(ctx),
                "custom inference should be enabled for non-enterprise workspaces"
            );
        });
    })
}

#[test]
fn test_custom_inference_disabled_for_enterprise_workspace_without_warp_plan_or_flag() {
    let _flag = FeatureFlag::CustomInferenceEndpointsEnterprise.override_enabled(false);

    let mut team = team_for_test();
    team.billing_metadata.customer_type = CustomerType::Enterprise;
    let mut workspace = workspace_for_test(&team);
    workspace.billing_metadata.customer_type = CustomerType::Enterprise;
    workspace.billing_metadata.tier.name = "Enterprise".to_string();

    App::test((), |mut app| async move {
        let team_client = mock_team_client_returning_workspaces(vec![workspace.clone()]);
        initialize_app(
            &mut app,
            CachedResources {
                workspaces: vec![workspace],
            },
            Arc::new(team_client),
            Arc::new(MockWorkspaceClient::new()),
        );

        app.read(|ctx| {
            assert!(
                !UserWorkspaces::as_ref(ctx).is_custom_inference_enabled(ctx),
                "custom inference should be disabled for enterprise workspaces when neither Warp Plan nor enterprise flag is enabled"
            );
        });
    })
}

#[test]
fn test_custom_inference_enabled_for_enterprise_warp_plan_workspace() {
    let _flag = FeatureFlag::CustomInferenceEndpointsEnterprise.override_enabled(false);

    let mut team = team_for_test();
    team.billing_metadata.customer_type = CustomerType::Enterprise;
    let mut workspace = workspace_for_test(&team);
    workspace.billing_metadata.customer_type = CustomerType::Enterprise;
    workspace.billing_metadata.tier.name = "Warp Plan".to_string();

    App::test((), |mut app| async move {
        initialize_app(
            &mut app,
            CachedResources {
                workspaces: vec![workspace],
            },
            Arc::new(MockTeamClient::new()),
            Arc::new(MockWorkspaceClient::new()),
        );

        app.read(|ctx| {
            assert!(
                UserWorkspaces::as_ref(ctx).is_custom_inference_enabled(ctx),
                "custom inference should be enabled for enterprise workspaces on Warp Plan"
            );
        });
    })
}

#[test]
fn test_custom_inference_enabled_for_enterprise_workspace_when_flag_on() {
    let _flag = FeatureFlag::CustomInferenceEndpointsEnterprise.override_enabled(true);

    let mut team = team_for_test();
    team.billing_metadata.customer_type = CustomerType::Enterprise;
    let mut workspace = workspace_for_test(&team);
    workspace.billing_metadata.customer_type = CustomerType::Enterprise;
    workspace.billing_metadata.tier.name = "Enterprise".to_string();

    App::test((), |mut app| async move {
        initialize_app(
            &mut app,
            CachedResources {
                workspaces: vec![workspace],
            },
            Arc::new(MockTeamClient::new()),
            Arc::new(MockWorkspaceClient::new()),
        );

        app.read(|ctx| {
            assert!(
                UserWorkspaces::as_ref(ctx).is_custom_inference_enabled(ctx),
                "custom inference should be enabled for enterprise workspaces when enterprise flag is on"
            );
        });
    })
}

#[test]
fn test_custom_inference_disabled_for_logged_out_user() {
    App::test((), |mut app| async move {
        let team_client = mock_team_client_returning_workspaces(vec![]);
        initialize_app_with_auth_state(
            &mut app,
            CachedResources { workspaces: vec![] },
            Arc::new(team_client),
            Arc::new(MockWorkspaceClient::new()),
            AuthTestState::LoggedOut,
        );

        app.read(|ctx| {
            assert!(
                !UserWorkspaces::as_ref(ctx).is_custom_inference_enabled(ctx),
                "custom inference should be disabled for logged-out users"
            );
        });
    })
}
#[test]
fn test_codebase_context_enabled_by_team_disabled_by_user() {
    // Enable codebase context on a team level
    let mut team = team_for_test();
    team.organization_settings.codebase_context_settings.setting = AdminEnablementSetting::Enable;

    // Disable codebase context on the user level
    let mut workspace = workspace_for_test(&team);
    workspace.settings.codebase_context_settings = CodebaseContextSettings {
        setting: AdminEnablementSetting::Enable, // This doesn't matter since team setting overrides
    };

    App::test((), |mut app| async move {
        initialize_app(
            &mut app,
            CachedResources {
                workspaces: vec![workspace],
            },
            Arc::new(MockTeamClient::new()),
            Arc::new(MockWorkspaceClient::new()),
        );

        app.read(|ctx| {
            let codebase_context_enabled = UserWorkspaces::as_ref(ctx)
                .is_codebase_context_enabled(ctx);
            assert!(codebase_context_enabled,
            "codebase context should be on when it's enabled by the team, regardless of user setting");
        });
    })
}

#[test]
fn test_codebase_context_enabled_by_team_and_user() {
    // Enable codebase context on a team level
    let mut team = team_for_test();
    team.organization_settings.codebase_context_settings.setting = AdminEnablementSetting::Enable;

    // Enable codebase context on the user level (this doesn't matter since team overrides)
    let mut workspace = workspace_for_test(&team);
    workspace.settings.codebase_context_settings = CodebaseContextSettings {
        setting: AdminEnablementSetting::Enable,
    };

    App::test((), |mut app| async move {
        initialize_app(
            &mut app,
            CachedResources {
                workspaces: vec![workspace],
            },
            Arc::new(MockTeamClient::new()),
            Arc::new(MockWorkspaceClient::new()),
        );

        app.read(|ctx| {
            let codebase_context_enabled =
                UserWorkspaces::as_ref(ctx).is_codebase_context_enabled(ctx);
            assert!(
                codebase_context_enabled,
                "codebase context should be on when it's enabled by the team"
            );
        });
    })
}

#[test]
fn test_codebase_context_disabled_by_team() {
    // Disable codebase context on a team level
    let mut team = team_for_test();
    team.organization_settings.codebase_context_settings.setting = AdminEnablementSetting::Disable;

    // Enable codebase context on the user level (this doesn't matter since team overrides)
    let mut workspace = workspace_for_test(&team);
    workspace.settings.codebase_context_settings = CodebaseContextSettings {
        setting: AdminEnablementSetting::Enable,
    };

    App::test((), |mut app| async move {
        initialize_app(
            &mut app,
            CachedResources {
                workspaces: vec![workspace],
            },
            Arc::new(MockTeamClient::new()),
            Arc::new(MockWorkspaceClient::new()),
        );

        app.read(|ctx| {
            let codebase_context_enabled = UserWorkspaces::as_ref(ctx)
                .is_codebase_context_enabled(ctx);
            assert!(
                !codebase_context_enabled,
                "codebase context should be off when it's disabled by the team, regardless of the user's settings"
            );
        });
    })
}

#[test]
fn test_codebase_context_respect_user_setting() {
    // Set team to respect user setting
    let mut team = team_for_test();
    team.organization_settings.codebase_context_settings.setting =
        AdminEnablementSetting::RespectUserSetting;

    let workspace = workspace_for_test(&team);

    App::test((), |mut app| async move {
        initialize_app(
            &mut app,
            CachedResources {
                workspaces: vec![workspace],
            },
            Arc::new(MockTeamClient::new()),
            Arc::new(MockWorkspaceClient::new()),
        );

        app.read(|ctx| {
            let codebase_context_enabled = UserWorkspaces::as_ref(ctx)
                .is_codebase_context_enabled(ctx);
            // Should respect user setting, which defaults to true when AI is enabled
            assert!(
                codebase_context_enabled,
                "codebase context should respect user setting when team setting is RespectUserSetting"
            );

            // Test that team_allows_codebase_context returns the correct setting
            let team_setting = UserWorkspaces::as_ref(ctx)
                .team_allows_codebase_context();
            assert_eq!(
                team_setting,
                AdminEnablementSetting::RespectUserSetting,
                "team_allows_codebase_context should return RespectUserSetting"
            );
        });
    })
}

#[test]
fn test_joining_team_moves_objects() {
    let _flag = FeatureFlag::SharedWithMe.override_enabled(true);

    let team = Team {
        uid: 123.into(),
        name: "test".to_string(),
        invite_code: None,
        members: vec![],
        pending_email_invites: vec![],
        invite_link_domain_restrictions: vec![],
        billing_metadata: Default::default(),
        stripe_customer_id: None,
        organization_settings: Default::default(),
        is_eligible_for_discovery: false,
        has_billing_history: false,
    };
    let team_uid = team.uid;
    let workspace = Workspace {
        uid: "workspace_uid123456789".to_string().into(),
        name: "test".to_string(),
        stripe_customer_id: None,
        teams: vec![team.clone()],
        billing_metadata: Default::default(),
        bonus_grants_purchased_this_month: Default::default(),
        billing_cycle_usage: None,
        has_billing_history: false,
        settings: Default::default(),
        invite_code: None,
        invite_link_domain_restrictions: vec![],
        pending_email_invites: vec![],
        is_eligible_for_discovery: false,
        members: vec![],
        total_requests_used_since_last_refresh: 0,
    };

    let shared_object = CloudWorkflow::new_local(
        CloudWorkflowModel {
            data: Workflow::new("shared workflow", "echo shared"),
        },
        Owner::Team { team_uid },
        None,
        ClientId::default(),
    );
    let object_id = shared_object.id;

    App::test((), |mut app| async move {
        initialize_app(
            &mut app,
            CachedResources { workspaces: vec![] },
            Arc::new(MockTeamClient::new()),
            Arc::new(MockWorkspaceClient::new()),
        );
        CloudModel::handle(&app).update(&mut app, |cloud_model, _| {
            cloud_model.add_object(object_id, shared_object);
        });

        // At first, the object is shared.
        app.read(|ctx| {
            assert!(!UserWorkspaces::as_ref(ctx).has_teams());

            let space = CloudModel::as_ref(ctx)
                .get_by_uid(&object_id.uid())
                .unwrap()
                .space(ctx);
            assert_eq!(space, Space::Shared);
        });

        // Now, the user joins the owning team.
        UserWorkspaces::handle(&app).update(&mut app, |user_workspaces, ctx| {
            user_workspaces.update_workspaces(vec![workspace], ctx);
        });

        // This migrates the object into the team drive.
        app.read(|ctx: &AppContext| {
            let space = CloudModel::as_ref(ctx)
                .get_by_uid(&object_id.uid())
                .unwrap()
                .space(ctx);
            assert_eq!(space, Space::Team { team_uid });
        });
    })
}

#[test]
fn test_agent_attribution_default_with_no_workspace() {
    App::test((), |mut app| async move {
        initialize_app(
            &mut app,
            CachedResources { workspaces: vec![] },
            Arc::new(MockTeamClient::new()),
            Arc::new(MockWorkspaceClient::new()),
        );

        app.read(|ctx| {
            let setting = UserWorkspaces::as_ref(ctx).get_agent_attribution_setting();
            assert_eq!(
                setting,
                AdminEnablementSetting::RespectUserSetting,
                "attribution should default to RespectUserSetting when there is no workspace"
            );
        });
    })
}

#[test]
fn test_agent_attribution_forced_on_by_team() {
    let mut team = team_for_test();
    team.organization_settings.enable_warp_attribution = AdminEnablementSetting::Enable;
    let workspace = workspace_for_test(&team);

    App::test((), |mut app| async move {
        initialize_app(
            &mut app,
            CachedResources {
                workspaces: vec![workspace],
            },
            Arc::new(MockTeamClient::new()),
            Arc::new(MockWorkspaceClient::new()),
        );

        app.read(|ctx| {
            let setting = UserWorkspaces::as_ref(ctx).get_agent_attribution_setting();
            assert_eq!(
                setting,
                AdminEnablementSetting::Enable,
                "attribution should be Enable when forced on by the team"
            );
        });
    })
}

#[test]
fn test_agent_attribution_forced_off_by_team() {
    let mut team = team_for_test();
    team.organization_settings.enable_warp_attribution = AdminEnablementSetting::Disable;
    let workspace = workspace_for_test(&team);

    App::test((), |mut app| async move {
        initialize_app(
            &mut app,
            CachedResources {
                workspaces: vec![workspace],
            },
            Arc::new(MockTeamClient::new()),
            Arc::new(MockWorkspaceClient::new()),
        );

        app.read(|ctx| {
            let setting = UserWorkspaces::as_ref(ctx).get_agent_attribution_setting();
            assert_eq!(
                setting,
                AdminEnablementSetting::Disable,
                "attribution should be Disable when forced off by the team"
            );
        });
    })
}

#[test]
fn test_agent_attribution_respects_user_setting() {
    let mut team = team_for_test();
    team.organization_settings.enable_warp_attribution = AdminEnablementSetting::RespectUserSetting;
    let workspace = workspace_for_test(&team);

    App::test((), |mut app| async move {
        initialize_app(
            &mut app,
            CachedResources {
                workspaces: vec![workspace],
            },
            Arc::new(MockTeamClient::new()),
            Arc::new(MockWorkspaceClient::new()),
        );

        app.read(|ctx| {
            let setting = UserWorkspaces::as_ref(ctx).get_agent_attribution_setting();
            assert_eq!(
                setting,
                AdminEnablementSetting::RespectUserSetting,
                "attribution should be RespectUserSetting when the team defers to user preference"
            );
        });
    })
}

#[test]
fn test_leaving_team_moves_objects() {
    let _flag = FeatureFlag::SharedWithMe.override_enabled(true);

    let team = Team {
        uid: 123.into(),
        name: "test".to_string(),
        invite_code: None,
        members: vec![],
        pending_email_invites: vec![],
        invite_link_domain_restrictions: vec![],
        billing_metadata: Default::default(),
        stripe_customer_id: None,
        organization_settings: Default::default(),
        is_eligible_for_discovery: false,
        has_billing_history: false,
    };
    let team_uid = team.uid;
    let workspace = Workspace {
        uid: "workspace_uid123456789".to_string().into(),
        name: "test".to_string(),
        stripe_customer_id: None,
        teams: vec![team.clone()],
        billing_metadata: Default::default(),
        bonus_grants_purchased_this_month: Default::default(),
        billing_cycle_usage: None,
        has_billing_history: false,
        settings: Default::default(),
        invite_code: None,
        invite_link_domain_restrictions: vec![],
        pending_email_invites: vec![],
        is_eligible_for_discovery: false,
        members: vec![],
        total_requests_used_since_last_refresh: 0,
    };

    let shared_object = CloudWorkflow::new_local(
        CloudWorkflowModel {
            data: Workflow::new("shared workflow", "echo shared"),
        },
        Owner::Team { team_uid },
        None,
        ClientId::default(),
    );
    let object_id = shared_object.id;

    App::test((), |mut app| async move {
        initialize_app(
            &mut app,
            CachedResources {
                workspaces: vec![workspace],
            },
            Arc::new(MockTeamClient::new()),
            Arc::new(MockWorkspaceClient::new()),
        );
        CloudModel::handle(&app).update(&mut app, |cloud_model, _| {
            cloud_model.add_object(object_id, shared_object);
        });

        // At first, the object is in the team drive.
        app.read(|ctx| {
            let space = CloudModel::as_ref(ctx)
                .get_by_uid(&object_id.uid())
                .unwrap()
                .space(ctx);
            assert_eq!(space, Space::Team { team_uid });
        });

        // Now, the user leaves the owning team. However, the object is still shared with them.
        UserWorkspaces::handle(&app).update(&mut app, |user_workspaces, ctx| {
            user_workspaces.update_workspaces(vec![], ctx);
        });

        // This migrates the object into the shared space.
        app.read(|ctx| {
            let space = CloudModel::as_ref(ctx)
                .get_by_uid(&object_id.uid())
                .unwrap()
                .space(ctx);
            assert_eq!(space, Space::Shared);
        });
    })
}
