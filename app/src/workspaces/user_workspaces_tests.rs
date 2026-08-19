use std::time::Duration;

use mockall::Sequence;
use settings::{PrivatePreferences, PublicPreferences};
use warp_graphql::billing::{
    BillingMetadata as GqlBillingMetadata, BonusGrantsInfo as GqlBonusGrantsInfo,
    CustomerType as GqlCustomerType, DelinquencyStatus as GqlDelinquencyStatus,
    PurchaseAddOnCreditsPolicy as GqlPurchaseAddOnCreditsPolicy, Tier as GqlTier,
};
use warp_graphql::queries::get_workspaces_metadata_for_user::{
    User as GqlUser, UserProfile as GqlUserProfile, UserPurchasePolicyBillingMetadata,
    UserPurchasePolicyTier,
};
use warp_graphql::workspace::{
    AddonCreditsSettings as GqlAddonCreditsSettings,
    AdminEnablementSetting as GqlAdminEnablementSetting,
    AdminEnablementSettingInfo as GqlAdminEnablementSettingInfo,
    AiAutonomySettingInfo as GqlAiAutonomySettingInfo, AiAutonomySettings as GqlAiAutonomySettings,
    AiAutonomySettingsInfo as GqlAiAutonomySettingsInfo, AiAutonomyValue as GqlAiAutonomyValue,
    AiPermissionsSettings as GqlAiPermissionsSettings,
    AiPermissionsSettingsInfo as GqlAiPermissionsSettingsInfo, AvailableLlms as GqlAvailableLlms,
    BooleanSettingInfo as GqlBooleanSettingInfo,
    CloudConversationStorageSettings as GqlCloudConversationStorageSettings,
    CodebaseContextSettings as GqlCodebaseContextSettings,
    ComputerUseAutonomyValue as GqlComputerUseAutonomyValue,
    ComputerUseSettingInfo as GqlComputerUseSettingInfo,
    FeatureModelChoice as GqlFeatureModelChoice, LinkSharingSettings as GqlLinkSharingSettings,
    LinkSharingSettingsInfo as GqlLinkSharingSettingsInfo, LlmSettings as GqlLlmSettings,
    MembershipRole as GqlMembershipRole,
    SandboxedAgentSettingsInfo as GqlSandboxedAgentSettingsInfo,
    SecretRedactionRegexListInfo as GqlSecretRedactionRegexListInfo,
    SecretRedactionSettings as GqlSecretRedactionSettings,
    SecretRedactionSettingsInfo as GqlSecretRedactionSettingsInfo,
    StringListSettingInfo as GqlStringListSettingInfo, Team as GqlTeam,
    TeamMember as GqlTeamMember, TeamSettings as GqlTeamSettings,
    TeamVisibility as GqlTeamVisibility, TelemetrySettings as GqlTelemetrySettings,
    UgcCollectionEnablementSetting as GqlUgcCollectionEnablementSetting,
    UgcCollectionSettingInfo as GqlUgcCollectionSettingInfo,
    UgcCollectionSettings as GqlUgcCollectionSettings,
    UsageBasedPricingSettings as GqlUsageBasedPricingSettings, Workspace as GqlWorkspace,
    WorkspaceSettings as GqlWorkspaceSettings,
    WriteToPtyAutonomyValue as GqlWriteToPtyAutonomyValue,
    WriteToPtySettingInfo as GqlWriteToPtySettingInfo,
};
use warpui::{AddSingletonModel, App, WindowId};
use warpui_extras::user_preferences;

use super::*;
use crate::ai::llms::LLMModelHost;
use crate::auth::AuthManager;
use crate::cloud_object::model::persistence::CloudModel;
use crate::cloud_object::{CloudObject, CloudObjectGuest};
use crate::drive::sharing::{SharingAccessLevel, Subject, UserKind};
use crate::features::FeatureFlag;
use crate::network::NetworkStatus;
use crate::server::cloud_objects::update_manager::UpdateManager;
use crate::server::ids::ClientId;
use crate::server::server_api::ServerApiProvider;
use crate::server::server_api::team::{MockTeamClient, TeamClient};
use crate::server::sync_queue::SyncQueue;
use crate::server::telemetry::context_provider::AppTelemetryContextProvider;
use crate::settings::{AISettings, CodeSettings, FocusedTerminalInfo};
use crate::system::SystemStats;
use crate::workflows::workflow::Workflow;
use crate::workflows::{CloudWorkflow, CloudWorkflowModel};
use crate::workspaces::gql_convert::PLACEHOLDER_WORKSPACE_UID;
use crate::workspaces::team::{Team, TeamMember, TeamVisibility};
use crate::workspaces::team_tester::TeamTesterStatus;
use crate::workspaces::update_manager::TeamUpdateManager;
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::workspaces::workspace::{
    AdminEnablementSetting, CodebaseContextSettings, HostEnablementSetting, LlmHostSettings,
    MultiAdminPolicy, PurchaseAddOnCreditsPolicy, Workspace,
};

#[derive(Default)]
struct CachedResources {
    workspaces: Vec<Workspace>,
}

fn initialize_app(
    app: &mut App,
    resources: CachedResources,
    team_client: Arc<dyn TeamClient>,
    workspace_client: Arc<dyn WorkspaceClient>,
) {
    initialize_app_with_auth(
        app,
        resources,
        team_client,
        workspace_client,
        AuthStateProvider::new_for_test(),
    );
}

fn initialize_app_with_auth(
    app: &mut App,
    resources: CachedResources,
    team_client: Arc<dyn TeamClient>,
    workspace_client: Arc<dyn WorkspaceClient>,
    auth_state_provider: AuthStateProvider,
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
    app.add_singleton_model(|_| auth_state_provider);
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

fn initialize_window_team_test_app(app: &mut App, workspaces: Vec<Workspace>) {
    app.add_singleton_model(PrivacySettings::mock);
    app.add_singleton_model(|ctx| {
        UserWorkspaces::mock(
            Arc::new(MockTeamClient::new()),
            Arc::new(MockWorkspaceClient::new()),
            workspaces,
            ctx,
        )
    });
}

fn register_ai_usage_model(app: &mut App) {
    app.add_singleton_model(|_| ServerApiProvider::new_for_test());
    if app.models_of_type::<PrivatePreferences>().is_empty() {
        app.update(crate::settings::init_and_register_user_preferences);
    }
    app.add_singleton_model(|ctx| {
        AIRequestUsageModel::new_for_test(ServerApiProvider::as_ref(ctx).get_ai_client(), ctx)
    });
}

#[test]
fn test_loading_all_spaces_after_switching_from_offline() {
    let _flag = FeatureFlag::KnowledgeSidebar.override_enabled(true);

    let team = Team {
        uid: 123.into(),
        name: "test".to_string(),
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
        visibility: TeamVisibility::Open,
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
                        ai_credit_availability: None,
                        user_purchase_policy: None,
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
                        ai_credit_availability: None,
                        user_purchase_policy: None,
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
        visibility: TeamVisibility::Open,
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
            ..Default::default()
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
            ..Default::default()
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
                ai_credit_availability: None,
                user_purchase_policy: None,
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
            ..Default::default()
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
                ai_credit_availability: None,
                user_purchase_policy: None,
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
            Arc::new(MockTeamClient::new()),
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

const TEST_GCP_AUDIENCE: &str = "//iam.googleapis.com/projects/123456/locations/global/workloadIdentityPools/warp-pool/providers/warp-provider";
const TEST_GCP_SA_EMAIL: &str = "warp-geap@test-project.iam.gserviceaccount.com";

fn workspace_with_gemini_enterprise_host(
    team: &Team,
    enabled: bool,
    enablement_setting: HostEnablementSetting,
) -> Workspace {
    let mut workspace = workspace_for_test(team);
    workspace.settings.llm_settings.enabled = true;
    workspace.settings.llm_settings.host_configs.insert(
        LLMModelHost::GeminiEnterprise,
        LlmHostSettings {
            enabled,
            enablement_setting,
            gcp_audience: Some(TEST_GCP_AUDIENCE.to_string()),
            gcp_sa_email: Some(TEST_GCP_SA_EMAIL.to_string()),
        },
    );
    workspace
}

#[test]
fn test_gemini_enterprise_credentials_default_off_when_admin_respects_user_setting() {
    let _flag = FeatureFlag::GeminiEnterprise.override_enabled(true);
    let team = team_for_test();
    let workspace = workspace_with_gemini_enterprise_host(
        &team,
        true,
        HostEnablementSetting::RespectUserSetting,
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
                !UserWorkspaces::as_ref(ctx).is_gemini_enterprise_credentials_enabled(ctx),
                "respect-user-setting should default the local Gemini Enterprise credentials toggle to off"
            );
            assert!(
                UserWorkspaces::as_ref(ctx).is_gemini_enterprise_credentials_toggleable(),
                "respect-user-setting should leave the local Gemini Enterprise credentials toggle editable"
            );
        });
    })
}

#[test]
fn test_gemini_enterprise_credentials_respect_user_setting_honors_member_toggle() {
    let _flag = FeatureFlag::GeminiEnterprise.override_enabled(true);
    let team = team_for_test();
    let workspace = workspace_with_gemini_enterprise_host(
        &team,
        true,
        HostEnablementSetting::RespectUserSetting,
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

        AISettings::handle(&app).update(&mut app, |settings, ctx| {
            let _ = settings
                .gemini_enterprise_credentials_enabled
                .set_value(true, ctx);
        });

        app.read(|ctx| {
            assert!(
                UserWorkspaces::as_ref(ctx).is_gemini_enterprise_credentials_enabled(ctx),
                "respect-user-setting should honor an opted-in Gemini Enterprise credentials toggle"
            );
        });
    })
}

#[test]
fn test_gemini_enterprise_credentials_enforced_by_admin() {
    let _flag = FeatureFlag::GeminiEnterprise.override_enabled(true);
    let team = team_for_test();
    let workspace =
        workspace_with_gemini_enterprise_host(&team, true, HostEnablementSetting::Enforce);

    App::test((), |mut app| async move {
        initialize_app(
            &mut app,
            CachedResources {
                workspaces: vec![workspace],
            },
            Arc::new(MockTeamClient::new()),
            Arc::new(MockWorkspaceClient::new()),
        );

        AISettings::handle(&app).update(&mut app, |settings, ctx| {
            let _ = settings
                .gemini_enterprise_credentials_enabled
                .set_value(false, ctx);
        });

        app.read(|ctx| {
            assert!(
                UserWorkspaces::as_ref(ctx).is_gemini_enterprise_credentials_enabled(ctx),
                "enforced Gemini Enterprise host policy should ignore the local credentials toggle"
            );
            assert!(
                !UserWorkspaces::as_ref(ctx).is_gemini_enterprise_credentials_toggleable(),
                "enforced Gemini Enterprise host policy should disable the local credentials toggle"
            );
        });
    })
}

#[test]
fn test_gemini_enterprise_credentials_disabled_when_host_disabled() {
    let _flag = FeatureFlag::GeminiEnterprise.override_enabled(true);
    let team = team_for_test();
    let workspace =
        workspace_with_gemini_enterprise_host(&team, false, HostEnablementSetting::Enforce);

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
                !UserWorkspaces::as_ref(ctx).is_gemini_enterprise_available_from_workspace(),
                "a disabled Gemini Enterprise host should not be available from the workspace"
            );
            assert!(
                !UserWorkspaces::as_ref(ctx).is_gemini_enterprise_credentials_enabled(ctx),
                "a disabled Gemini Enterprise host should gate credentials off even under ENFORCE"
            );
        });
    })
}

#[test]
fn test_gemini_enterprise_credentials_disabled_when_host_absent() {
    let _flag = FeatureFlag::GeminiEnterprise.override_enabled(true);
    let team = team_for_test();
    // Bedrock-only workspace: proves the GEAP gate reads its own host entry.
    let mut workspace = workspace_for_test(&team);
    workspace.settings.llm_settings.enabled = true;
    workspace.settings.llm_settings.host_configs.insert(
        LLMModelHost::AwsBedrock,
        LlmHostSettings {
            enabled: true,
            enablement_setting: HostEnablementSetting::Enforce,
            ..Default::default()
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
                UserWorkspaces::as_ref(ctx)
                    .gemini_enterprise_host_settings()
                    .is_none(),
                "a workspace without a Gemini Enterprise host entry should expose no settings"
            );
            assert!(
                !UserWorkspaces::as_ref(ctx).is_gemini_enterprise_credentials_enabled(ctx),
                "a workspace without a Gemini Enterprise host entry should gate credentials off"
            );
        });
    })
}

#[test]
fn test_gemini_enterprise_credentials_disabled_when_logged_out() {
    let _flag = FeatureFlag::GeminiEnterprise.override_enabled(true);
    let team = team_for_test();
    let workspace =
        workspace_with_gemini_enterprise_host(&team, true, HostEnablementSetting::Enforce);

    App::test((), |mut app| async move {
        initialize_app_with_auth(
            &mut app,
            CachedResources {
                workspaces: vec![workspace],
            },
            Arc::new(MockTeamClient::new()),
            Arc::new(MockWorkspaceClient::new()),
            AuthStateProvider::new_logged_out_for_test(),
        );

        app.read(|ctx| {
            assert!(
                !UserWorkspaces::as_ref(ctx).is_gemini_enterprise_credentials_enabled(ctx),
                "logged-out users should never mint or attach Gemini Enterprise credentials"
            );
        });
    })
}

#[test]
fn test_gemini_enterprise_host_settings_carries_federation_config() {
    let team = team_for_test();
    let workspace = workspace_with_gemini_enterprise_host(
        &team,
        true,
        HostEnablementSetting::RespectUserSetting,
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
            let user_workspaces = UserWorkspaces::as_ref(ctx);
            let settings = user_workspaces
                .gemini_enterprise_host_settings()
                .expect("workspace should expose the Gemini Enterprise host settings");
            assert_eq!(settings.gcp_audience.as_deref(), Some(TEST_GCP_AUDIENCE));
            assert_eq!(settings.gcp_sa_email.as_deref(), Some(TEST_GCP_SA_EMAIL));
        });
    })
}

fn workspace_for_test(team: &Team) -> Workspace {
    Workspace {
        uid: "workspace_uid123456789".to_string().into(),
        name: "test".to_string(),
        stripe_customer_id: None,
        teams: vec![team.clone()],
        billing_metadata: team.billing_metadata.clone(),
        bonus_grants_purchased_this_month: Default::default(),
        billing_cycle_usage: None,
        has_billing_history: false,
        settings: Default::default(),
        invite_link_domain_restrictions: vec![],
        pending_email_invites: vec![],
        is_eligible_for_discovery: false,
        members: vec![],
        total_requests_used_since_last_refresh: 0,
    }
}

#[test]
fn test_current_workspace_billing_metadata_uses_selected_teamless_workspace() {
    let first_team = team_for_test();
    let first_workspace = workspace_for_test(&first_team);
    let mut second_workspace = workspace_for_test(&first_team);
    second_workspace.uid = "workspace_uid987654321".to_string().into();
    second_workspace.teams.clear();
    second_workspace.billing_metadata.customer_type = CustomerType::Enterprise;
    let second_workspace_uid = second_workspace.uid;

    App::test((), |mut app| async move {
        initialize_window_team_test_app(&mut app, vec![first_workspace, second_workspace]);

        UserWorkspaces::handle(&app).update(&mut app, |user_workspaces, ctx| {
            user_workspaces.set_current_workspace_uid(second_workspace_uid, ctx);
        });

        app.read(|ctx| {
            assert_eq!(
                UserWorkspaces::as_ref(ctx)
                    .current_workspace_billing_metadata()
                    .map(|metadata| metadata.customer_type),
                Some(CustomerType::Enterprise)
            );
        });
    })
}
#[test]
fn test_window_team_assignment_is_immutable() {
    let first_team = team_for_test();
    let mut second_team = team_for_test();
    second_team.uid = 456.into();
    second_team.name = "second".to_string();
    let mut workspace = workspace_for_test(&first_team);
    workspace.teams.push(second_team.clone());

    App::test((), |mut app| async move {
        initialize_window_team_test_app(&mut app, vec![workspace]);

        let window_id = WindowId::new();
        UserWorkspaces::handle(&app).update(&mut app, |user_workspaces, ctx| {
            user_workspaces.set_team_for_window(window_id, second_team.uid, ctx);
            user_workspaces.set_team_for_window(window_id, first_team.uid, ctx);
        });

        app.read(|ctx| {
            let user_workspaces = UserWorkspaces::as_ref(ctx);
            assert_eq!(
                user_workspaces.team_uid_for_window(window_id),
                Some(second_team.uid)
            );
            assert_eq!(
                user_workspaces
                    .team_for_window(window_id)
                    .map(|team| team.uid),
                Some(second_team.uid)
            );
        });
    })
}

#[test]
fn test_window_team_assignment_inherits_from_source_or_default_team() {
    let first_team = team_for_test();
    let mut second_team = team_for_test();
    second_team.uid = 456.into();
    let mut workspace = workspace_for_test(&first_team);
    workspace.teams.push(second_team.clone());

    App::test((), |mut app| async move {
        initialize_window_team_test_app(&mut app, vec![workspace]);

        let source_window_id = WindowId::new();
        let inherited_window_id = WindowId::new();
        let fallback_window_id = WindowId::new();
        UserWorkspaces::handle(&app).update(&mut app, |user_workspaces, ctx| {
            user_workspaces.set_team_for_window(source_window_id, second_team.uid, ctx);
            let inherited_team_uid =
                user_workspaces.inherited_or_default_team_uid(Some(source_window_id));
            let fallback_team_uid = user_workspaces.inherited_or_default_team_uid(None);
            user_workspaces.register_window(inherited_window_id, inherited_team_uid, ctx);
            user_workspaces.register_window(fallback_window_id, fallback_team_uid, ctx);
        });

        app.read(|ctx| {
            let user_workspaces = UserWorkspaces::as_ref(ctx);
            assert_eq!(
                user_workspaces.team_uid_for_window(inherited_window_id),
                Some(second_team.uid)
            );
            assert_eq!(
                user_workspaces.team_uid_for_window(fallback_window_id),
                Some(first_team.uid)
            );
        });
    })
}

#[test]
fn warp_agent_cli_upgrade_link_is_channel_aware_and_user_bound() {
    let user_uid = UserUid::new("user-123");

    assert_eq!(
        UserWorkspaces::warp_agent_cli_upgrade_link(Some(user_uid)),
        format!(
            "{}{STRIPE_SUBSCRIPTION_INTERVAL_PAGE_PREFIX}/user/{user_uid}?source=warp-agent-cli",
            ChannelState::server_root_url(),
        )
    );
}

#[test]
fn warp_agent_cli_upgrade_link_uses_channel_aware_fallback_without_a_user() {
    assert_eq!(
        UserWorkspaces::warp_agent_cli_upgrade_link(None),
        format!(
            "{}{STRIPE_SUBSCRIPTION_INTERVAL_PAGE_PREFIX}?source=warp-agent-cli",
            ChannelState::server_root_url().trim_end_matches('/'),
        )
    );
}

#[test]
fn admin_billing_link_for_default_team_targets_the_first_admin_team() {
    let email = "admin@example.com";
    let user_uid = UserUid::new("admin");
    let mut first_team = team_for_test();
    first_team.members.push(TeamMember {
        uid: user_uid,
        email: email.to_owned(),
        role: MembershipRole::Owner,
    });
    let mut second_team = first_team.clone();
    second_team.uid = 456.into();
    let first_team_uid = first_team.uid;
    let mut workspace = workspace_for_test(&first_team);
    workspace.teams.push(second_team);

    App::test((), |mut app| async move {
        initialize_window_team_test_app(&mut app, vec![workspace]);

        app.read(|ctx| {
            assert_eq!(
                UserWorkspaces::as_ref(ctx).admin_billing_link_for_default_team(email),
                Some(format!(
                    "{}/admin/{first_team_uid}/billing",
                    ChannelState::server_root_url().trim_end_matches('/'),
                ))
            );
        });
    })
}

#[test]
fn admin_billing_link_for_default_team_accepts_admin_when_multi_admin_is_enabled() {
    let email = "admin@example.com";
    let user_uid = UserUid::new("admin");
    let mut team = team_for_test();
    team.billing_metadata.tier.multi_admin_policy = Some(MultiAdminPolicy { enabled: true });
    team.members.push(TeamMember {
        uid: user_uid,
        email: email.to_owned(),
        role: MembershipRole::Admin,
    });
    let team_uid = team.uid;
    let workspace = workspace_for_test(&team);

    App::test((), |mut app| async move {
        initialize_window_team_test_app(&mut app, vec![workspace]);

        app.read(|ctx| {
            assert_eq!(
                UserWorkspaces::as_ref(ctx).admin_billing_link_for_default_team(email),
                Some(format!(
                    "{}/admin/{team_uid}/billing",
                    ChannelState::server_root_url().trim_end_matches('/'),
                ))
            );
        });
    })
}

#[test]
fn admin_billing_link_for_default_team_rejects_admin_without_multi_admin_policy() {
    let email = "admin@example.com";
    let user_uid = UserUid::new("admin");
    let mut team = team_for_test();
    team.members.push(TeamMember {
        uid: user_uid,
        email: email.to_owned(),
        role: MembershipRole::Admin,
    });
    let workspace = workspace_for_test(&team);

    App::test((), |mut app| async move {
        initialize_window_team_test_app(&mut app, vec![workspace]);

        app.read(|ctx| {
            assert_eq!(
                UserWorkspaces::as_ref(ctx).admin_billing_link_for_default_team(email),
                None
            );
        });
    })
}

#[test]
fn admin_billing_link_for_default_team_rejects_regular_members() {
    let email = "member@example.com";
    let user_uid = UserUid::new("member");
    let mut team = team_for_test();
    team.members.push(TeamMember {
        uid: user_uid,
        email: email.to_owned(),
        role: MembershipRole::User,
    });
    let workspace = workspace_for_test(&team);

    App::test((), |mut app| async move {
        initialize_window_team_test_app(&mut app, vec![workspace]);

        app.read(|ctx| {
            assert_eq!(
                UserWorkspaces::as_ref(ctx).admin_billing_link_for_default_team(email),
                None
            );
        });
    })
}

#[test]
fn test_window_team_assignment_falls_back_when_team_is_removed() {
    let first_team = team_for_test();
    let mut removed_team = team_for_test();
    removed_team.uid = 456.into();
    let mut workspace = workspace_for_test(&first_team);
    workspace.teams.push(removed_team.clone());

    App::test((), |mut app| async move {
        initialize_window_team_test_app(&mut app, vec![workspace.clone()]);

        let window_id = WindowId::new();
        UserWorkspaces::handle(&app).update(&mut app, |user_workspaces, ctx| {
            user_workspaces.set_team_for_window(window_id, removed_team.uid, ctx);
            workspace.teams.retain(|team| team.uid != removed_team.uid);
            user_workspaces.update_workspaces(vec![workspace], ctx);
        });

        app.read(|ctx| {
            assert_eq!(
                UserWorkspaces::as_ref(ctx).team_uid_for_window(window_id),
                Some(first_team.uid)
            );
        });
    })
}

#[test]
fn test_window_team_assignment_reconciles_when_current_workspace_changes() {
    let first_team = team_for_test();
    let first_workspace = workspace_for_test(&first_team);
    let mut second_team = team_for_test();
    second_team.uid = 456.into();
    let mut second_workspace = workspace_for_test(&second_team);
    second_workspace.uid = "workspace_uid987654321".to_string().into();
    let second_workspace_uid = second_workspace.uid;

    App::test((), |mut app| async move {
        initialize_window_team_test_app(&mut app, vec![first_workspace, second_workspace]);

        let window_id = WindowId::new();
        UserWorkspaces::handle(&app).update(&mut app, |user_workspaces, ctx| {
            user_workspaces.register_window(window_id, None, ctx);
            user_workspaces.set_current_workspace_uid(second_workspace_uid, ctx);
        });

        app.read(|ctx| {
            assert_eq!(
                UserWorkspaces::as_ref(ctx).team_uid_for_window(window_id),
                Some(second_team.uid)
            );
        });
    })
}

#[test]
fn test_spaces_for_window_orders_selected_team_shared_and_personal() {
    let _flag = FeatureFlag::SharedWithMe.override_enabled(true);
    let first_team = team_for_test();
    let mut selected_team = team_for_test();
    selected_team.uid = 456.into();
    selected_team.name = "selected".to_string();
    let mut workspace = workspace_for_test(&first_team);
    workspace.teams.push(selected_team.clone());

    App::test((), |mut app| async move {
        initialize_window_team_test_app(&mut app, vec![workspace]);
        app.add_singleton_model(CloudModel::mock);
        app.add_singleton_model(|_| AuthStateProvider::new_for_test());

        let window_id = WindowId::new();
        UserWorkspaces::handle(&app).update(&mut app, |user_workspaces, ctx| {
            user_workspaces.set_team_for_window(window_id, selected_team.uid, ctx);
        });

        let current_user_uid = app.read(|ctx| {
            AuthStateProvider::as_ref(ctx)
                .get()
                .user_id()
                .expect("test user should be authenticated")
        });
        let mut shared_object = CloudWorkflow::new_local(
            CloudWorkflowModel {
                data: Workflow::new("shared workflow", "echo shared"),
            },
            Owner::User {
                user_uid: UserUid::new("other-user"),
            },
            None,
            ClientId::default(),
        );
        shared_object
            .permissions_mut()
            .guests
            .push(CloudObjectGuest {
                subject: Subject::User(UserKind::Account(current_user_uid)),
                access_level: SharingAccessLevel::View,
                source: None,
            });
        CloudModel::handle(&app).update(&mut app, |cloud_model, _| {
            cloud_model.add_object(shared_object.id, shared_object);
        });

        app.read(|ctx| {
            assert_eq!(
                UserWorkspaces::as_ref(ctx).spaces_for_window(window_id, ctx),
                vec![
                    Space::Team {
                        team_uid: selected_team.uid
                    },
                    Space::Shared,
                    Space::Personal,
                ]
            );
        });
    })
}
#[test]
fn test_unassigned_window_is_initialized_after_workspace_metadata_loads() {
    let team = team_for_test();
    let workspace = workspace_for_test(&team);
    let workspace_uid = workspace.uid;

    App::test((), |mut app| async move {
        initialize_window_team_test_app(&mut app, vec![]);

        let window_id = WindowId::new();
        UserWorkspaces::handle(&app).update(&mut app, |user_workspaces, ctx| {
            user_workspaces.register_window(window_id, None, ctx);
        });
        app.read(|ctx| {
            assert_eq!(
                UserWorkspaces::as_ref(ctx).team_uid_for_window(window_id),
                None
            );
        });

        UserWorkspaces::handle(&app).update(&mut app, |user_workspaces, ctx| {
            user_workspaces.update_workspaces(vec![workspace], ctx);
            user_workspaces.set_current_workspace_uid(workspace_uid, ctx);
        });

        app.read(|ctx| {
            assert_eq!(
                UserWorkspaces::as_ref(ctx).team_uid_for_window(window_id),
                Some(team.uid)
            );
        });
    })
}

#[test]
fn test_codebase_context_enabled_by_team_disabled_by_user() {
    let team = team_for_test();

    // Codebase context is governed by the workspace-level effective settings.
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
            assert!(codebase_context_enabled,
            "codebase context should be on when it's enabled by the team, regardless of user setting");
        });
    })
}

#[test]
fn test_codebase_context_enabled_by_team_and_user() {
    let team = team_for_test();

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
fn test_codebase_context_disabled_by_workspace() {
    let team = team_for_test();

    let mut workspace = workspace_for_test(&team);
    workspace.settings.codebase_context_settings = CodebaseContextSettings {
        setting: AdminEnablementSetting::Disable,
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
                !codebase_context_enabled,
                "codebase context should be off when it's disabled by the workspace"
            );
        });
    })
}

#[test]
fn test_codebase_context_respect_user_setting() {
    let team = team_for_test();

    // Workspace defers codebase context to the user setting.
    let mut workspace = workspace_for_test(&team);
    workspace.settings.codebase_context_settings.setting =
        AdminEnablementSetting::RespectUserSetting;

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
        visibility: TeamVisibility::Open,
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
    let team = team_for_test();
    let mut workspace = workspace_for_test(&team);
    workspace.settings.enable_warp_attribution = AdminEnablementSetting::Enable;

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
    let team = team_for_test();
    let mut workspace = workspace_for_test(&team);
    workspace.settings.enable_warp_attribution = AdminEnablementSetting::Disable;

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
    let team = team_for_test();
    let mut workspace = workspace_for_test(&team);
    workspace.settings.enable_warp_attribution = AdminEnablementSetting::RespectUserSetting;

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
fn test_team_switcher_hidden_with_zero_teams() {
    // When the user is in no workspace / no teams, `can_switch_teams` must return
    // false so the pill does not render.
    App::test((), |mut app| async move {
        initialize_window_team_test_app(&mut app, vec![]);
        app.read(|ctx| {
            assert!(
                !UserWorkspaces::as_ref(ctx).can_switch_teams(),
                "0 teams: switcher should be hidden"
            );
        });
    })
}

#[test]
fn test_team_switcher_hidden_with_single_team() {
    // With exactly 1 team, `can_switch_teams` must return false.
    let team = team_for_test();
    let workspace = workspace_for_test(&team);
    App::test((), |mut app| async move {
        initialize_window_team_test_app(&mut app, vec![workspace]);
        app.read(|ctx| {
            assert!(
                !UserWorkspaces::as_ref(ctx).can_switch_teams(),
                "1 team: switcher should be hidden"
            );
        });
    })
}

#[test]
fn test_team_switcher_visible_with_multiple_teams() {
    // With 2+ teams, `can_switch_teams` must return true so the pill is shown.
    let team1 = team_for_test();
    let mut team2 = team_for_test();
    team2.uid = 456.into();
    team2.name = "Second Team".to_string();
    let mut workspace = workspace_for_test(&team1);
    workspace.teams.push(team2);

    App::test((), |mut app| async move {
        initialize_window_team_test_app(&mut app, vec![workspace]);
        app.read(|ctx| {
            assert!(
                UserWorkspaces::as_ref(ctx).can_switch_teams(),
                "2 teams: switcher should be visible"
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
        visibility: TeamVisibility::Open,
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

#[test]
fn test_team_billing_metadata_prefers_team_over_workspace() {
    let mut team = team_for_test();
    team.billing_metadata.customer_type = CustomerType::Build;
    let mut workspace = workspace_for_test(&team);
    workspace.billing_metadata.customer_type = CustomerType::Free;

    App::test((), |mut app| async move {
        initialize_window_team_test_app(&mut app, vec![workspace]);

        app.read(|ctx| {
            let user_workspaces = UserWorkspaces::as_ref(ctx);
            let team = user_workspaces.team_from_uid(123.into());
            assert!(team.is_some(), "test team should exist");
            assert_eq!(
                user_workspaces
                    .team_billing_metadata(team)
                    .map(|billing| billing.customer_type),
                Some(CustomerType::Build),
                "the team's billing metadata should win when a team exists"
            );
            assert_eq!(
                user_workspaces
                    .team_billing_metadata(None)
                    .map(|billing| billing.customer_type),
                Some(CustomerType::Free),
                "the workspace's billing metadata should be used without a team"
            );
        });
    })
}

#[test]
fn test_team_billing_metadata_enables_teamless_premium_purchases() {
    let team = team_for_test();
    let mut workspace = workspace_for_test(&team);
    workspace.teams.clear();
    workspace
        .billing_metadata
        .tier
        .purchase_add_on_credits_policy = Some(PurchaseAddOnCreditsPolicy {
        enabled: false,
        premium_enabled: true,
        price_premium_bps: 1000,
    });

    App::test((), |mut app| async move {
        initialize_window_team_test_app(&mut app, vec![workspace]);

        app.read(|ctx| {
            let user_workspaces = UserWorkspaces::as_ref(ctx);
            assert!(!user_workspaces.has_teams(), "user should be teamless");
            let billing = user_workspaces.team_billing_metadata(None);
            assert!(
                billing.is_some_and(|billing| billing.is_purchase_add_on_credits_policy_enabled()),
                "premiumEnabled on the workspace policy should enable purchases without a team"
            );
            assert_eq!(
                billing.map_or(0, |billing| billing.addon_credits_price_premium_bps()),
                1000
            );
        });
    })
}

#[test]
fn test_team_billing_metadata_disabled_policy_stays_disabled_without_team() {
    let team = team_for_test();
    let mut workspace = workspace_for_test(&team);
    workspace.teams.clear();
    workspace
        .billing_metadata
        .tier
        .purchase_add_on_credits_policy = Some(PurchaseAddOnCreditsPolicy {
        enabled: false,
        premium_enabled: false,
        price_premium_bps: 0,
    });

    App::test((), |mut app| async move {
        initialize_window_team_test_app(&mut app, vec![workspace]);

        app.read(|ctx| {
            let billing = UserWorkspaces::as_ref(ctx).team_billing_metadata(None);
            assert!(
                !billing.is_some_and(|billing| billing.is_purchase_add_on_credits_policy_enabled()),
                "a fully disabled policy should keep purchases disabled without a team"
            );
        });
    })
}

#[test]
fn test_purchase_addon_credits_forwards_teamless_team_uid() {
    App::test((), |mut app| async move {
        let mut workspace_client = MockWorkspaceClient::new();
        workspace_client
            .expect_purchase_addon_credits()
            .withf(|team_uid, credits| team_uid.is_none() && *credits == 1_000)
            .times(1)
            .returning(|_, _| {
                Ok(PurchaseAddonCreditsOutcome::CheckoutRequired {
                    checkout_url: "https://example.com/checkout".to_string(),
                })
            });

        app.add_singleton_model(|ctx| {
            UserWorkspaces::mock(
                Arc::new(MockTeamClient::new()),
                Arc::new(workspace_client),
                vec![],
                ctx,
            )
        });

        UserWorkspaces::handle(&app).update(&mut app, |user_workspaces, ctx| {
            user_workspaces.purchase_addon_credits(None, 1_000, ctx);
        });

        // Give the spawned client call time to run so the mock expectation is
        // exercised before the test ends.
        warpui::r#async::Timer::after(Duration::from_millis(100)).await;
    })
}

#[test]
fn test_purchase_addon_credits_forwards_team_uid_when_present() {
    App::test((), |mut app| async move {
        let mut workspace_client = MockWorkspaceClient::new();
        workspace_client
            .expect_purchase_addon_credits()
            .withf(|team_uid, credits| *team_uid == Some(123.into()) && *credits == 2_000)
            .times(1)
            .returning(|_, _| {
                Ok(PurchaseAddonCreditsOutcome::CheckoutRequired {
                    checkout_url: "https://example.com/checkout".to_string(),
                })
            });

        app.add_singleton_model(|ctx| {
            UserWorkspaces::mock(
                Arc::new(MockTeamClient::new()),
                Arc::new(workspace_client),
                vec![],
                ctx,
            )
        });

        UserWorkspaces::handle(&app).update(&mut app, |user_workspaces, ctx| {
            user_workspaces.purchase_addon_credits(Some(123.into()), 2_000, ctx);
        });

        // Give the spawned client call time to run so the mock expectation is
        // exercised before the test ends.
        warpui::r#async::Timer::after(Duration::from_millis(100)).await;
    })
}

#[test]
fn test_remove_user_from_team_rejected_emits_error_event_without_updating_workspaces() {
    let team = team_for_test();
    let team_uid = team.uid;
    let workspace = workspace_for_test(&team);

    App::test((), |mut app| async move {
        let mut team_client = MockTeamClient::new();
        team_client
            .expect_remove_user_from_team()
            .times(1)
            .returning(|_, _, _| {
                Err(anyhow::anyhow!(
                    "missing response data for RemoveUserFromTeam: Not found: no rows in result set"
                ))
            });

        app.add_singleton_model(|ctx| {
            UserWorkspaces::mock(
                Arc::new(team_client),
                Arc::new(MockWorkspaceClient::new()),
                vec![workspace],
                ctx,
            )
        });

        let user_workspaces_handle = UserWorkspaces::handle(&app);
        let (sender, receiver) = async_channel::unbounded();
        app.update(|ctx| {
            let sender = sender.clone();
            ctx.subscribe_to_model(
                &user_workspaces_handle,
                move |_, event: &UserWorkspacesEvent, _| {
                    if let UserWorkspacesEvent::RemoveUserFromTeamRejected(err) = event {
                        let _ = sender.try_send(err.to_string());
                    }
                },
            );
        });

        UserWorkspaces::handle(&app).update(&mut app, |user_workspaces, ctx| {
            user_workspaces.remove_user_from_team(
                UserUid::new("member-uid"),
                team_uid,
                CloudObjectEventEntrypoint::TeamSettings,
                ctx,
            );
        });

        warpui::r#async::Timer::after(Duration::from_millis(100)).await;

        let error_message = receiver
            .try_recv()
            .expect("expected RemoveUserFromTeamRejected to be emitted");
        assert!(
            error_message.contains("no rows in result set"),
            "the rejected event should carry the server's error message, got: {error_message}"
        );

        // A failed removal must not silently drop the team from local state.
        app.read(|ctx| {
            assert!(
                UserWorkspaces::as_ref(ctx).has_teams(),
                "a rejected removal should leave the existing team data untouched"
            );
        });
    })
}

#[test]
fn test_remove_user_from_team_success_emits_success_event_and_refreshes_members() {
    let user_uid = UserUid::new("member-uid");
    let mut team = team_for_test();
    team.members.push(TeamMember {
        uid: user_uid,
        email: "member@example.com".to_string(),
        role: MembershipRole::User,
    });
    let team_uid = team.uid;
    let workspace = workspace_for_test(&team);

    let mut updated_team = team.clone();
    updated_team.members.clear();
    let updated_workspace = workspace_for_test(&updated_team);

    App::test((), |mut app| async move {
        let mut team_client = MockTeamClient::new();
        team_client
            .expect_remove_user_from_team()
            .times(1)
            .returning(move |_, _, _| {
                Ok(WorkspacesMetadataWithPricing {
                    metadata: WorkspacesMetadataResponse {
                        workspaces: vec![updated_workspace.clone()],
                        joinable_teams: vec![],
                        experiments: None,
                        feature_model_choices: None,
                        ai_credit_availability: None,
                        user_purchase_policy: None,
                    },
                    pricing_info: None,
                })
            });

        app.add_singleton_model(PrivacySettings::mock);
        app.add_singleton_model(|ctx| {
            UserWorkspaces::mock(
                Arc::new(team_client),
                Arc::new(MockWorkspaceClient::new()),
                vec![workspace],
                ctx,
            )
        });

        let user_workspaces_handle = UserWorkspaces::handle(&app);
        let (sender, receiver) = async_channel::unbounded();
        app.update(|ctx| {
            let sender = sender.clone();
            ctx.subscribe_to_model(
                &user_workspaces_handle,
                move |_, event: &UserWorkspacesEvent, _| {
                    if matches!(event, UserWorkspacesEvent::RemoveUserFromTeamSuccess) {
                        let _ = sender.try_send(());
                    }
                },
            );
        });

        UserWorkspaces::handle(&app).update(&mut app, |user_workspaces, ctx| {
            user_workspaces.remove_user_from_team(
                user_uid,
                team_uid,
                CloudObjectEventEntrypoint::TeamSettings,
                ctx,
            );
        });

        warpui::r#async::Timer::after(Duration::from_millis(100)).await;

        receiver
            .try_recv()
            .expect("expected RemoveUserFromTeamSuccess to be emitted");

        // The acceptance criteria requires that a successful removal continues to
        // refresh the member list, exactly like before this fix.
        app.read(|ctx| {
            let team = UserWorkspaces::as_ref(ctx)
                .team_from_uid(team_uid)
                .expect("team should still exist after removal");
            assert!(
                team.members.is_empty(),
                "member list should refresh to reflect the removal"
            );
        });
    })
}

fn gql_tier(purchase_policy: Option<GqlPurchaseAddOnCreditsPolicy>) -> GqlTier {
    GqlTier {
        name: "Free".to_string(),
        description: "Free tier".to_string(),
        warp_ai_policy: None,
        team_size_policy: None,
        shared_notebooks_policy: None,
        shared_workflows_policy: None,
        session_sharing_policy: None,
        ai_autonomy_policy: None,
        telemetry_data_collection_policy: None,
        ugc_data_collection_policy: None,
        usage_based_pricing_policy: None,
        codebase_context_policy: None,
        byo_api_key_policy: None,
        byo_endpoint_policy: None,
        managed_byok_byoe_policy: None,
        purchase_add_on_credits_policy: purchase_policy,
        enterprise_pay_as_you_go_policy: None,
        enterprise_credits_auto_reload_policy: None,
        multi_admin_policy: None,
        native_workspaces_policy: None,
        ambient_agents_policy: None,
        usage_visibility_policy: None,
    }
}

fn gql_workspace(
    uid: &str,
    purchase_policy: Option<GqlPurchaseAddOnCreditsPolicy>,
) -> GqlWorkspace {
    let empty_llms = GqlAvailableLlms {
        default_id: String::new(),
        choices: vec![],
        preferred_codex_model_id: None,
    };
    GqlWorkspace {
        uid: uid.into(),
        name: "workspace".to_string(),
        stripe_customer_id: None,
        members: vec![],
        teams: vec![],
        billing_metadata: GqlBillingMetadata {
            customer_type: GqlCustomerType::Free,
            delinquency_status: GqlDelinquencyStatus::NoDelinquency,
            tier: gql_tier(purchase_policy),
            service_agreements: vec![],
            ai_overages: None,
        },
        bonus_grants_info: GqlBonusGrantsInfo {
            grants: vec![],
            spending_info: None,
        },
        billing_cycle_usage_history: None,
        settings: GqlWorkspaceSettings {
            is_discoverable: false,
            is_invite_link_enabled: false,
            llm_settings: GqlLlmSettings {
                enabled: false,
                host_configs: vec![],
            },
            team_byo: None,
            telemetry_settings: GqlTelemetrySettings {
                force_enabled: false,
            },
            ugc_collection_settings: GqlUgcCollectionSettings {
                setting: GqlUgcCollectionEnablementSetting::RespectUserSetting,
            },
            cloud_conversation_storage_settings: GqlCloudConversationStorageSettings {
                setting: GqlAdminEnablementSetting::RespectUserSetting,
            },
            ai_permissions_settings: GqlAiPermissionsSettings {
                allow_ai_in_remote_sessions: true,
                remote_session_regex_list: vec![],
            },
            link_sharing_settings: GqlLinkSharingSettings {
                anyone_with_link_sharing_enabled: true,
                direct_link_sharing_enabled: true,
            },
            secret_redaction_settings: GqlSecretRedactionSettings {
                enabled: false,
                regexes: vec![],
            },
            ai_autonomy_settings: GqlAiAutonomySettings {
                apply_code_diffs_setting: None,
                read_files_setting: None,
                read_files_allowlist: None,
                create_plans_setting: None,
                execute_commands_setting: None,
                execute_commands_allowlist: None,
                execute_commands_denylist: None,
                write_to_pty_setting: None,
                computer_use_setting: None,
            },
            usage_based_pricing_settings: GqlUsageBasedPricingSettings {
                enabled: false,
                max_monthly_spend_cents: None,
            },
            addon_credits_settings: GqlAddonCreditsSettings {
                auto_reload_enabled: false,
                max_monthly_spend_cents: None,
                selected_auto_reload_credit_denomination: None,
            },
            codebase_context_settings: GqlCodebaseContextSettings {
                enabled: true,
                setting: GqlAdminEnablementSetting::RespectUserSetting,
            },
            sandboxed_agent_settings: None,
            ambient_agent_settings: None,
        },
        has_billing_history: false,
        pending_email_invites: vec![],
        invite_link_domain_restrictions: vec![],
        is_eligible_for_discovery: false,
        feature_model_choice: GqlFeatureModelChoice {
            agent_mode: empty_llms.clone(),
            planning: empty_llms.clone(),
            coding: empty_llms.clone(),
            cli_agent: empty_llms.clone(),
            computer_use_agent: empty_llms,
        },
        total_requests_used_since_last_refresh: 0,
    }
}

/// Team settings with every group at its neutral value, so a fixture only has to
/// override the field the test cares about.
fn gql_team_settings() -> GqlTeamSettings {
    fn admin_info() -> GqlAdminEnablementSettingInfo {
        GqlAdminEnablementSettingInfo {
            value: GqlAdminEnablementSetting::RespectUserSetting,
            is_enforced_by_workspace: false,
        }
    }

    fn bool_info(value: bool) -> GqlBooleanSettingInfo {
        GqlBooleanSettingInfo {
            value,
            is_enforced_by_workspace: false,
        }
    }

    fn autonomy_info() -> GqlAiAutonomySettingInfo {
        GqlAiAutonomySettingInfo {
            value: GqlAiAutonomyValue::RespectUserSetting,
            is_enforced_by_workspace: false,
        }
    }

    fn str_list() -> GqlStringListSettingInfo {
        GqlStringListSettingInfo {
            values: vec![],
            workspace_entries: vec![],
            team_entries: vec![],
        }
    }

    GqlTeamSettings {
        ugc_collection: GqlUgcCollectionSettingInfo {
            value: GqlUgcCollectionEnablementSetting::RespectUserSetting,
            is_enforced_by_workspace: false,
        },
        cloud_conversation_storage: admin_info(),
        codebase_context: admin_info(),
        ai_permissions: GqlAiPermissionsSettingsInfo {
            allow_ai_in_remote_sessions: bool_info(true),
            remote_session_regex_list: str_list(),
        },
        secret_redaction: GqlSecretRedactionSettingsInfo {
            enabled: bool_info(false),
            regexes: GqlSecretRedactionRegexListInfo {
                values: vec![],
                workspace_entries: vec![],
                team_entries: vec![],
            },
        },
        ai_autonomy: GqlAiAutonomySettingsInfo {
            apply_code_diffs: autonomy_info(),
            read_files: autonomy_info(),
            create_plans: autonomy_info(),
            execute_commands: autonomy_info(),
            write_to_pty: GqlWriteToPtySettingInfo {
                value: GqlWriteToPtyAutonomyValue::RespectUserSetting,
                is_enforced_by_workspace: false,
            },
            computer_use: GqlComputerUseSettingInfo {
                value: GqlComputerUseAutonomyValue::RespectUserSetting,
                is_enforced_by_workspace: false,
            },
            read_files_allowlist: str_list(),
            execute_commands_allowlist: str_list(),
            execute_commands_denylist: str_list(),
        },
        link_sharing: GqlLinkSharingSettingsInfo {
            anyone_with_link_sharing_enabled: bool_info(true),
            direct_link_sharing_enabled: bool_info(true),
        },
        sandboxed_agent: GqlSandboxedAgentSettingsInfo {
            execute_commands_denylist: str_list(),
        },
        llm_settings: GqlLlmSettings {
            enabled: false,
            host_configs: vec![],
        },
        telemetry_settings: GqlTelemetrySettings {
            force_enabled: false,
        },
        usage_based_pricing_settings: GqlUsageBasedPricingSettings {
            enabled: false,
            max_monthly_spend_cents: None,
        },
        addon_credits_settings: GqlAddonCreditsSettings {
            auto_reload_enabled: false,
            max_monthly_spend_cents: None,
            selected_auto_reload_credit_denomination: None,
        },
        ambient_agent_settings: None,
        team_byo: None,
    }
}

fn gql_team(uid: &str, name: &str, member_uids: &[&str]) -> GqlTeam {
    GqlTeam {
        // `ServerId` rejects anything but a 22-character id.
        uid: format!("{uid:0>22}").into(),
        name: name.to_string(),
        color: None,
        members: member_uids
            .iter()
            .map(|member_uid| GqlTeamMember {
                uid: (*member_uid).into(),
                email: format!("{member_uid}@example.com"),
                role: GqlMembershipRole::User,
            })
            .collect(),
        settings: gql_team_settings(),
        invite_link: None,
        visibility: GqlTeamVisibility::Open,
    }
}

fn apply_workspaces_metadata(app: &mut App, metadata: WorkspacesMetadataResponse) {
    UserWorkspaces::handle(app).update(app, |user_workspaces, ctx| {
        user_workspaces.on_workspaces_updated(
            Ok(WorkspacesMetadataWithPricing {
                metadata,
                pricing_info: None,
            }),
            ctx,
        );
    });
}

fn current_team_names(user_workspaces: &UserWorkspaces) -> Vec<String> {
    user_workspaces
        .current_workspace()
        .map(|workspace| {
            workspace
                .teams
                .iter()
                .map(|team| team.name.clone())
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn test_team_switcher_drops_teams_the_admin_is_not_a_member_of() {
    App::test((), |mut app| async move {
        initialize_window_team_test_app(&mut app, vec![]);
        register_ai_usage_model(&mut app);

        // The server hands a workspace admin every team in the workspace, but only
        // the team they actually joined is one they can operate as in the client.
        let mut workspace = gql_workspace("workspace_uid123456789", None);
        workspace.teams = vec![
            gql_team("member-team", "Member Team", &["test-user"]),
            gql_team("other-team", "Other Team", &["someone-else"]),
        ];

        apply_workspaces_metadata(&mut app, gql_user(None, vec![workspace]).into());

        app.read(|ctx| {
            let user_workspaces = UserWorkspaces::as_ref(ctx);
            assert_eq!(current_team_names(user_workspaces), ["Member Team"]);
            assert!(
                !user_workspaces.can_switch_teams(),
                "a single membership should hide the switcher"
            );
        });
    })
}

#[test]
fn test_team_switcher_keeps_every_team_the_user_is_a_member_of() {
    App::test((), |mut app| async move {
        initialize_window_team_test_app(&mut app, vec![]);
        register_ai_usage_model(&mut app);

        let mut workspace = gql_workspace("workspace_uid123456789", None);
        workspace.teams = vec![
            gql_team("first-team", "First Team", &["test-user"]),
            gql_team("other-team", "Other Team", &["someone-else"]),
            gql_team("second-team", "Second Team", &["test-user"]),
        ];

        apply_workspaces_metadata(&mut app, gql_user(None, vec![workspace]).into());

        app.read(|ctx| {
            let user_workspaces = UserWorkspaces::as_ref(ctx);
            assert_eq!(
                current_team_names(user_workspaces),
                ["First Team", "Second Team"]
            );
            assert!(
                user_workspaces.can_switch_teams(),
                "multiple memberships should keep the switcher visible"
            );
        });
    })
}

#[test]
fn test_teamless_user_falls_back_to_workspace_settings() {
    App::test((), |mut app| async move {
        initialize_window_team_test_app(&mut app, vec![]);
        register_ai_usage_model(&mut app);

        // A workspace admin who joined none of the workspace's teams is teamless
        // in the client, so the workspace's own settings supply their defaults.
        let mut workspace = gql_workspace("workspace_uid123456789", None);
        workspace.settings.llm_settings.enabled = true;
        workspace.teams = vec![gql_team("other-team", "Other Team", &["someone-else"])];

        apply_workspaces_metadata(&mut app, gql_user(None, vec![workspace]).into());

        app.read(|ctx| {
            let user_workspaces = UserWorkspaces::as_ref(ctx);
            assert!(
                !user_workspaces.has_teams(),
                "an admin with no membership should end up teamless"
            );
            assert!(
                user_workspaces.is_custom_llm_enabled_for_team(None),
                "workspace settings should supply the teamless default"
            );
        });
    })
}

#[test]
fn test_member_team_settings_win_over_workspace_settings() {
    App::test((), |mut app| async move {
        initialize_window_team_test_app(&mut app, vec![]);
        register_ai_usage_model(&mut app);

        let mut workspace = gql_workspace("workspace_uid123456789", None);
        workspace.settings.llm_settings.enabled = true;
        let mut team = gql_team("member-team", "Member Team", &["test-user"]);
        team.settings.llm_settings.enabled = false;
        workspace.teams = vec![team];

        apply_workspaces_metadata(&mut app, gql_user(None, vec![workspace]).into());

        app.read(|ctx| {
            let user_workspaces = UserWorkspaces::as_ref(ctx);
            let team = user_workspaces.sole_team();
            assert!(team.is_some(), "the member team should survive filtering");
            assert!(
                !user_workspaces.is_custom_llm_enabled_for_team(team),
                "the team's own settings should win when the user has a team"
            );
        });
    })
}

fn gql_premium_purchase_policy() -> GqlPurchaseAddOnCreditsPolicy {
    GqlPurchaseAddOnCreditsPolicy {
        enabled: false,
        premium_enabled: true,
        price_premium_bps: 1000,
    }
}

fn gql_user(
    user_purchase_policy: Option<GqlPurchaseAddOnCreditsPolicy>,
    workspaces: Vec<GqlWorkspace>,
) -> GqlUser {
    GqlUser {
        profile: GqlUserProfile {
            uid: "test-user".to_string(),
        },
        ai_credit_availability: warp_graphql::ai::AICreditAvailability {
            available: true,
            denial_reason: warp_graphql::ai::AICreditAvailabilityDenialReason::None,
            credit_source: None,
        },
        billing_metadata: user_purchase_policy.map(|policy| UserPurchasePolicyBillingMetadata {
            tier: UserPurchasePolicyTier {
                purchase_add_on_credits_policy: Some(policy),
            },
        }),
        workspaces,
        experiments: None,
        discoverable_teams: vec![],
    }
}

#[test]
fn test_user_level_policy_survives_placeholder_filtering_for_teamless_users() {
    App::test((), |mut app| async move {
        initialize_window_team_test_app(&mut app, vec![]);
        register_ai_usage_model(&mut app);

        // The real conversion path: a teamless user's ONLY workspace is the
        // placeholder, which must stay filtered out of `workspaces`, while
        // the user-level purchase policy is captured separately.
        let response: WorkspacesMetadataResponse = gql_user(
            Some(gql_premium_purchase_policy()),
            vec![gql_workspace(PLACEHOLDER_WORKSPACE_UID, None)],
        )
        .into();
        assert!(
            response.workspaces.is_empty(),
            "the placeholder workspace must stay filtered out"
        );
        assert_eq!(
            response.user_purchase_policy,
            Some(PurchaseAddOnCreditsPolicy {
                enabled: false,
                premium_enabled: true,
                price_premium_bps: 1000,
            })
        );

        UserWorkspaces::handle(&app).update(&mut app, |user_workspaces, ctx| {
            user_workspaces.on_workspaces_updated(
                Ok(WorkspacesMetadataWithPricing {
                    metadata: response,
                    pricing_info: None,
                }),
                ctx,
            );
        });

        app.read(|ctx| {
            let user_workspaces = UserWorkspaces::as_ref(ctx);
            assert!(
                user_workspaces.current_workspace().is_none(),
                "teamless users keep having no workspace"
            );
            let policy = user_workspaces.purchase_policy();
            assert!(
                policy.is_some_and(|policy| policy.allows_purchases()),
                "the user-level policy should enable purchases without a team or workspace"
            );
            assert_eq!(
                policy.map_or(0, |policy| policy.effective_premium_bps()),
                1000
            );
        });
    })
}

#[test]
fn test_workspace_policy_wins_over_user_level_policy() {
    App::test((), |mut app| async move {
        initialize_window_team_test_app(&mut app, vec![]);
        register_ai_usage_model(&mut app);

        let standard_policy = GqlPurchaseAddOnCreditsPolicy {
            enabled: true,
            premium_enabled: false,
            price_premium_bps: 0,
        };
        let response: WorkspacesMetadataResponse = gql_user(
            Some(gql_premium_purchase_policy()),
            vec![
                gql_workspace(PLACEHOLDER_WORKSPACE_UID, None),
                gql_workspace("workspace_uid123456789", Some(standard_policy)),
            ],
        )
        .into();
        assert_eq!(response.workspaces.len(), 1);

        UserWorkspaces::handle(&app).update(&mut app, |user_workspaces, ctx| {
            user_workspaces.on_workspaces_updated(
                Ok(WorkspacesMetadataWithPricing {
                    metadata: response,
                    pricing_info: None,
                }),
                ctx,
            );
        });

        app.read(|ctx| {
            let policy = UserWorkspaces::as_ref(ctx).purchase_policy();
            assert_eq!(
                policy.map(|policy| policy.enabled),
                Some(true),
                "a real workspace's policy should win over the user-level fallback"
            );
            assert_eq!(
                policy.map_or(-1, |policy| policy.effective_premium_bps()),
                0
            );
        });
    })
}

#[test]
fn test_team_policy_wins_over_workspace_and_user_policy() {
    let mut team = team_for_test();
    team.billing_metadata.tier.purchase_add_on_credits_policy = Some(PurchaseAddOnCreditsPolicy {
        enabled: true,
        premium_enabled: false,
        price_premium_bps: 0,
    });
    let mut workspace = workspace_for_test(&team);
    workspace
        .billing_metadata
        .tier
        .purchase_add_on_credits_policy = Some(PurchaseAddOnCreditsPolicy {
        enabled: false,
        premium_enabled: true,
        price_premium_bps: 1000,
    });

    App::test((), |mut app| async move {
        initialize_window_team_test_app(&mut app, vec![workspace]);

        UserWorkspaces::handle(&app).update(&mut app, |user_workspaces, _| {
            user_workspaces.set_user_purchase_policy(Some(PurchaseAddOnCreditsPolicy {
                enabled: false,
                premium_enabled: true,
                price_premium_bps: 2000,
            }));
        });

        app.read(|ctx| {
            let user_workspaces = UserWorkspaces::as_ref(ctx);
            let team = user_workspaces.team_from_uid(123.into());
            assert_eq!(
                user_workspaces
                    .purchase_policy_for_team(team)
                    .map(|policy| policy.enabled),
                Some(true),
                "the team's policy should win over workspace and user legs"
            );
            // Without a team, the workspace's policy still beats the user leg.
            assert_eq!(
                user_workspaces
                    .purchase_policy()
                    .map_or(0, |policy| policy.effective_premium_bps()),
                1000
            );
        });
    })
}
