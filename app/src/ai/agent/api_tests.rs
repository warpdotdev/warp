use std::sync::Arc;
use std::time::SystemTime;

use ai::api_keys::{
    ApiKeyManager, AwsCredentials, AwsCredentialsState, GeapCredentials, GeapCredentialsState,
};
use warp_core::channel::{Channel, ChannelState};
use warp_core::features::FeatureFlag;
use warpui::{App, SingletonEntity as _, WindowId};

use super::{RequestParams, ServerConversationToken};
use crate::ai::agent::ServerOutputId;
use crate::ai::geap_credentials::{GeapPolicy, current_geap_policy};
use crate::ai::llms::{LLMModelHost, LLMProvider};
use crate::auth::AuthStateProvider;
use crate::server::server_api::team::MockTeamClient;
use crate::server::server_api::workspace::MockWorkspaceClient;
use crate::settings::PrivacySettings;
use crate::workspaces::team::Team;
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::workspaces::workspace::{
    BillingMetadata, HostEnablementSetting, LlmHostSettings, ManagedByokByoePolicy,
    TeamByoSettings, TeamSettings, Tier, Workspace,
};

/// A workload identity provider resource name shaped like a real one, used only to satisfy
/// [`current_geap_policy`]'s non-empty-audience check.
const GEAP_TEST_AUDIENCE: &str = "//iam.googleapis.com/projects/123456/locations/global/workloadIdentityPools/warp-pool/providers/warp-provider";
const GEAP_TEST_SA_EMAIL: &str = "warp-geap@test-project.iam.gserviceaccount.com";
const TEST_WORKSPACE_UID: &str = "workspace_uid123456789";

#[test]
fn debugging_payload_is_link_on_dogfood_channels() {
    let token = ServerConversationToken::new("conversation-token".to_owned());
    let request_id = ServerOutputId::new("request-id".to_owned());
    let expected_link = format!(
        "{}/debug/maa/conversation-token",
        ChannelState::server_root_url()
    );

    for channel in [Channel::Dev, Channel::Local] {
        assert_eq!(
            token.debugging_payload_for_channel(None, channel),
            expected_link
        );
        assert_eq!(
            token.debugging_payload_for_channel(Some(&request_id), channel),
            format!("{expected_link}?request=request-id")
        );
    }
}

#[test]
fn debugging_payload_is_id_on_non_dogfood_channels() {
    let token = ServerConversationToken::new("conversation-token".to_owned());
    let request_id = ServerOutputId::new("request-id".to_owned());

    for channel in [
        Channel::Stable,
        Channel::Preview,
        Channel::Integration,
        Channel::Oss,
    ] {
        assert_eq!(
            token.debugging_payload_for_channel(None, channel),
            "{\"conversation_id\":\"conversation-token\"}"
        );
        assert_eq!(
            token.debugging_payload_for_channel(Some(&request_id), channel),
            "{\"request_id\":\"request-id\",\"conversation_id\":\"conversation-token\"}"
        );
    }
}

/// Enables AWS Bedrock and Gemini Enterprise (GEAP) at the workspace level, both enforced so
/// they bypass the per-user `AISettings` toggles. Lets a test show that
/// [`RequestParams::apply_team_byo_policy`] leaves org-level credentials alone while it strips
/// member-provided ones.
fn enable_aws_and_geap_hosts(workspace: &mut Workspace) {
    workspace.settings.llm_settings.enabled = true;
    workspace.settings.llm_settings.host_configs.insert(
        LLMModelHost::AwsBedrock,
        LlmHostSettings {
            enabled: true,
            enablement_setting: HostEnablementSetting::Enforce,
            gcp_audience: None,
            gcp_sa_email: None,
        },
    );
    workspace.settings.llm_settings.host_configs.insert(
        LLMModelHost::GeminiEnterprise,
        LlmHostSettings {
            enabled: true,
            enablement_setting: HostEnablementSetting::Enforce,
            gcp_audience: Some(GEAP_TEST_AUDIENCE.to_string()),
            gcp_sa_email: Some(GEAP_TEST_SA_EMAIL.to_string()),
        },
    );
}

/// Billing metadata for a plan that manages BYOK/BYOE centrally, which is the entitlement that
/// makes a team's `team_byo` policy enforceable at all.
fn managed_byok_billing_metadata() -> BillingMetadata {
    BillingMetadata {
        tier: Tier {
            managed_byok_byoe_policy: Some(ManagedByokByoePolicy { enabled: true }),
            ..Default::default()
        },
        ..Default::default()
    }
}

fn team_with_byo_policy(uid: i64, name: &str, allow_member_credentials: bool) -> Team {
    Team::from_local_cache(
        uid.into(),
        name.to_string(),
        Some(TeamSettings {
            team_byo: Some(TeamByoSettings {
                first_party_enabled: true,
                endpoints_enabled: true,
                allow_user_keys: allow_member_credentials,
                allow_user_endpoints: allow_member_credentials,
                first_party_keys: vec![],
                endpoints: vec![],
            }),
            ..Default::default()
        }),
        Some(managed_byok_billing_metadata()),
        None,
    )
}

fn workspace_with_teams(teams: Vec<Team>) -> Workspace {
    Workspace::from_local_cache(
        TEST_WORKSPACE_UID.to_string().into(),
        "test".to_string(),
        Some(teams),
    )
}

/// Two teams on one workspace with the same plan entitlement but opposite `team_byo` policies,
/// so only the team's own policy can explain a difference in behaviour between them.
fn two_teams_of_opposing_byo_policy() -> (Team, Team) {
    (
        team_with_byo_policy(111, "team-a", true),
        team_with_byo_policy(222, "team-b", false),
    )
}

fn register_workspace_and_api_key_manager(app: &mut App, workspace: Workspace) {
    app.update(|ctx| {
        warpui_extras::secure_storage::register_noop("test", ctx);
        warp_core::telemetry::testing::MockTelemetryContextProvider::register(ctx);
    });
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    // `UserWorkspaces::update_workspaces` pushes telemetry and secret-redaction settings into
    // `PrivacySettings`, which panics if the singleton was never registered.
    app.add_singleton_model(PrivacySettings::mock);
    app.add_singleton_model(|ctx| {
        UserWorkspaces::mock(
            Arc::new(MockTeamClient::new()),
            Arc::new(MockWorkspaceClient::new()),
            vec![workspace],
            ctx,
        )
    });
    app.add_singleton_model(ApiKeyManager::new);
}

#[test]
fn apply_team_byo_policy_gates_member_credentials_by_the_requesting_windows_team() {
    let (team_a, team_b) = two_teams_of_opposing_byo_policy();
    let mut workspace = workspace_with_teams(vec![team_a.clone(), team_b.clone()]);
    enable_aws_and_geap_hosts(&mut workspace);

    App::test((), |mut app| async move {
        let _geap_flag = FeatureFlag::GeminiEnterprise.override_enabled(true);
        register_workspace_and_api_key_manager(&mut app, workspace);

        ApiKeyManager::handle(&app).update(&mut app, |manager, ctx| {
            manager
                .persist_provider_key(LLMProvider::Anthropic, Some("sk-ant-test".to_owned()), ctx)
                .expect("no-op secure storage should accept the provider key");
            manager.add_custom_endpoint(
                ai::api_keys::CustomEndpointParams {
                    name: "member-endpoint".to_string(),
                    url: "https://example.com/v1".to_string(),
                    api_key: "endpoint-key".to_string(),
                    models: vec![("member-model".to_string(), None, None)],
                    schema: Default::default(),
                },
                ctx,
            );
            manager.set_aws_credentials_state(
                AwsCredentialsState::Loaded {
                    credentials: AwsCredentials::new(
                        "access-key".to_string(),
                        "secret-key".to_string(),
                        None,
                        None,
                    ),
                    loaded_at: SystemTime::now(),
                },
                ctx,
            );
            let binding = match current_geap_policy(ctx) {
                GeapPolicy::Mintable(binding) => binding,
                other => panic!("expected a mintable GEAP policy, got {other:?}"),
            };
            manager.set_geap_credentials_state(
                GeapCredentialsState::Loaded {
                    credentials: GeapCredentials::new("geap-token".to_string(), None),
                    loaded_at: SystemTime::now(),
                    minted_for: binding,
                },
                ctx,
            );
        });

        let window_on_team_a = WindowId::new();
        let window_on_team_b = WindowId::new();
        UserWorkspaces::handle(&app).update(&mut app, |user_workspaces, ctx| {
            user_workspaces.set_team_for_window(window_on_team_a, team_a.uid, ctx);
            user_workspaces.set_team_for_window(window_on_team_b, team_b.uid, ctx);
        });

        let mut request_params = RequestParams::new_for_test();
        app.read(|ctx| {
            let api_key_manager = ApiKeyManager::as_ref(ctx);
            let is_aws_bedrock_enabled =
                UserWorkspaces::as_ref(ctx).is_aws_bedrock_credentials_enabled(ctx);
            let geap_binding = current_geap_policy(ctx).mint_binding();
            request_params.api_keys =
                api_key_manager.api_keys_for_request(true, is_aws_bedrock_enabled, geap_binding);
            request_params.custom_model_providers =
                api_key_manager.custom_model_providers_for_request(true);
        });
        let baseline_keys = request_params
            .api_keys
            .clone()
            .expect("test setup should attach BYO, Bedrock, and GEAP credentials");
        assert!(
            !baseline_keys.anthropic.is_empty(),
            "test setup should attach a BYO key for the policy check to gate"
        );
        assert!(baseline_keys.aws_credentials.is_some());
        assert!(baseline_keys.google_cloud_credentials.is_some());
        assert!(
            request_params.custom_model_providers.is_some(),
            "test setup should attach a member custom endpoint for the policy check to gate"
        );

        app.read(|ctx| {
            let user_workspaces = UserWorkspaces::as_ref(ctx);

            let mut allowed = request_params.clone();
            allowed.apply_team_byo_policy(
                &user_workspaces.team_context_for_window(window_on_team_a),
                ctx,
            );
            let allowed_keys = allowed
                .api_keys
                .expect("team A's policy allows members to use their own keys");
            assert!(
                !allowed_keys.anthropic.is_empty(),
                "team A's policy allows members to use their own keys"
            );
            assert!(
                allowed_keys.aws_credentials.is_some(),
                "Bedrock credentials are org-level and must survive either team's policy"
            );
            assert!(
                allowed_keys.google_cloud_credentials.is_some(),
                "GEAP credentials are org-level and must survive either team's policy"
            );
            assert!(
                allowed.custom_model_providers.is_some(),
                "team A's policy allows members to use their own custom endpoints"
            );

            let mut disallowed = request_params.clone();
            disallowed.apply_team_byo_policy(
                &user_workspaces.team_context_for_window(window_on_team_b),
                ctx,
            );
            let disallowed_keys = disallowed.api_keys.expect(
                "Bedrock/GEAP credentials must keep api_keys populated even once member keys are stripped",
            );
            assert!(
                disallowed_keys.anthropic.is_empty(),
                "team B's policy disallows members from using their own keys"
            );
            assert!(
                disallowed_keys.aws_credentials.is_some(),
                "a restrictive team_byo policy must not strip org-level Bedrock credentials"
            );
            assert!(
                disallowed_keys.google_cloud_credentials.is_some(),
                "a restrictive team_byo policy must not strip org-level GEAP credentials"
            );
            assert!(
                disallowed.custom_model_providers.is_none(),
                "team B's policy disallows members from using their own custom endpoints"
            );

            // The decision has to survive on the params, not be inferred from them. This is
            // the state the Grok refresh path re-injected into: `api_keys` is still `Some(..)`
            // because Bedrock and GEAP survived, so a caller reading only the stripped value
            // cannot tell that member credentials were disallowed.
            assert!(
                allowed.member_byo_credentials_allowed,
                "the permissive team's decision must be recorded on the params"
            );
            assert!(
                !disallowed.member_byo_credentials_allowed,
                "the restrictive team's decision must be recorded on the params"
            );
        });
    });
}

/// [`RequestParams::new`] has no window, so the team decision is unknown until
/// `apply_team_byo_policy` runs. For a credential gate that has to read as "not permitted":
/// the Grok refresh path ANDs this flag into its own BYO check, and a `true` default would let
/// any params that skipped the policy step put a member credential back on the request.
#[test]
fn request_params_do_not_allow_member_credentials_until_the_policy_has_been_applied() {
    assert!(!RequestParams::new_for_test().member_byo_credentials_allowed);
}

/// A window `UserWorkspaces` has never been told about is on no team, so no team policy
/// restricts it. Every TUI window is in this state today: `register_window` is only called
/// from the GUI's `RootView::new`, so the TUI cannot resolve a team and an admin's `team_byo`
/// restriction does not reach its requests. This pins that gap rather than endorsing it --
/// registering TUI windows is what closes it, and this assertion should flip when it does.
#[test]
fn apply_team_byo_policy_is_inert_for_an_unregistered_window() {
    let (_team_a, team_b) = two_teams_of_opposing_byo_policy();
    let workspace = workspace_with_teams(vec![team_b]);

    App::test((), |mut app| async move {
        register_workspace_and_api_key_manager(&mut app, workspace);
        ApiKeyManager::handle(&app).update(&mut app, |manager, ctx| {
            manager
                .persist_provider_key(LLMProvider::Anthropic, Some("sk-ant-test".to_owned()), ctx)
                .expect("no-op secure storage should accept the provider key");
        });

        let mut request_params = RequestParams::new_for_test();
        app.read(|ctx| {
            request_params.api_keys =
                ApiKeyManager::as_ref(ctx).api_keys_for_request(true, false, None);
        });

        app.read(|ctx| {
            let user_workspaces = UserWorkspaces::as_ref(ctx);
            assert!(
                user_workspaces.is_managed_byok_byoe_enabled(),
                "the plan manages BYOK/BYOE centrally, so only the missing window explains the \
                 restriction not applying"
            );
            let unregistered_window = WindowId::new();
            let team_scope = user_workspaces.team_context_for_window(unregistered_window);
            request_params.apply_team_byo_policy(&team_scope, ctx);
            assert!(
                request_params
                    .api_keys
                    .as_ref()
                    .is_some_and(|keys| !keys.anthropic.is_empty()),
                "an unregistered window resolves to no team, so the restrictive team_byo policy \
                 does not apply to it"
            );
        });
    });
}

/// The policy is resolved per request rather than captured when the conversation started, so
/// an admin restriction binds on the requesting window's very next request.
#[test]
fn apply_team_byo_policy_rebinds_after_the_window_changes_team() {
    let (team_a, team_b) = two_teams_of_opposing_byo_policy();
    let workspace = workspace_with_teams(vec![team_a.clone(), team_b.clone()]);

    App::test((), |mut app| async move {
        register_workspace_and_api_key_manager(&mut app, workspace);
        ApiKeyManager::handle(&app).update(&mut app, |manager, ctx| {
            manager
                .persist_provider_key(LLMProvider::Anthropic, Some("sk-ant-test".to_owned()), ctx)
                .expect("no-op secure storage should accept the provider key");
        });

        let window_id = WindowId::new();
        UserWorkspaces::handle(&app).update(&mut app, |user_workspaces, ctx| {
            user_workspaces.set_team_for_window(window_id, team_a.uid, ctx);
        });

        let mut request_params = RequestParams::new_for_test();
        app.read(|ctx| {
            request_params.api_keys =
                ApiKeyManager::as_ref(ctx).api_keys_for_request(true, false, None);
        });
        assert!(request_params.api_keys.is_some());

        app.read(|ctx| {
            let user_workspaces = UserWorkspaces::as_ref(ctx);
            let mut on_team_a = request_params.clone();
            on_team_a
                .apply_team_byo_policy(&user_workspaces.team_context_for_window(window_id), ctx);
            assert!(
                on_team_a
                    .api_keys
                    .is_some_and(|keys| !keys.anthropic.is_empty()),
                "the window is on the permissive team, so its member key is sent"
            );
        });

        // A window only changes team by reconciling away from a team that has left the
        // workspace, so drop team A to move this window onto team B.
        UserWorkspaces::handle(&app).update(&mut app, |user_workspaces, ctx| {
            user_workspaces
                .update_workspaces(vec![workspace_with_teams(vec![team_b.clone()])], ctx);
        });

        app.read(|ctx| {
            let user_workspaces = UserWorkspaces::as_ref(ctx);
            let mut on_team_b = request_params.clone();
            on_team_b
                .apply_team_byo_policy(&user_workspaces.team_context_for_window(window_id), ctx);
            assert!(
                on_team_b
                    .api_keys
                    .is_none_or(|keys| keys.anthropic.is_empty()),
                "the same request params, re-evaluated after the window moved to the restrictive \
                 team, must no longer carry the member key"
            );
        });
    });
}
