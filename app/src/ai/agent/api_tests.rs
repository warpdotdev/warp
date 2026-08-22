use std::sync::Arc;
use std::time::SystemTime;

use ai::api_keys::{
    ApiKeyManager, AwsCredentials, AwsCredentialsState, GeapCredentials, GeapCredentialsState,
};
use warp_core::channel::{Channel, ChannelState};
use warp_core::features::FeatureFlag;
use warpui::{App, SingletonEntity as _};

use super::{RequestParams, ServerConversationToken};
use crate::ai::agent::ServerOutputId;
use crate::ai::geap_credentials::{GeapPolicy, geap_policy_for_context};
use crate::ai::llms::{LLMModelHost, LLMProvider};
use crate::auth::AuthStateProvider;
use crate::server::server_api::team::MockTeamClient;
use crate::server::server_api::workspace::MockWorkspaceClient;
use crate::settings::AISettings;
use crate::workspaces::team::Team;
use crate::workspaces::user_workspaces::{TeamContextForOperation, UserWorkspaces};
use crate::workspaces::workspace::{
    BillingMetadata, HostEnablementSetting, LlmHostSettings, ManagedByokByoePolicy,
    TeamByoSettings, TeamSettings, Tier, Workspace,
};

/// A workload identity provider resource name shaped like a real one, used only to satisfy
/// [`crate::ai::geap_credentials::current_geap_policy`]'s non-empty-audience check.
const GEAP_TEST_AUDIENCE: &str = "//iam.googleapis.com/projects/123456/locations/global/workloadIdentityPools/warp-pool/providers/warp-provider";
const GEAP_TEST_SA_EMAIL: &str = "warp-geap@test-project.iam.gserviceaccount.com";

/// Enables AWS Bedrock and Gemini Enterprise (GEAP) at the team level, both enforced
/// (bypassing the per-user `AISettings` toggle), so `apply_team_byo_policy`'s preservation of
/// these org-level credentials can be exercised independently of `team_byo` member policy.
fn enable_aws_and_geap_hosts(team: &mut Team) {
    team.settings.llm_settings.enabled = true;
    team.settings.llm_settings.host_configs.insert(
        LLMModelHost::AwsBedrock,
        LlmHostSettings {
            enabled: true,
            enablement_setting: HostEnablementSetting::Enforce,
            gcp_audience: None,
            gcp_sa_email: None,
        },
    );
    team.settings.llm_settings.host_configs.insert(
        LLMModelHost::GeminiEnterprise,
        LlmHostSettings {
            enabled: true,
            enablement_setting: HostEnablementSetting::Enforce,
            gcp_audience: Some(GEAP_TEST_AUDIENCE.to_string()),
            gcp_sa_email: Some(GEAP_TEST_SA_EMAIL.to_string()),
        },
    );
}

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

fn team_byo_settings(allow_user_keys: bool, allow_user_endpoints: bool) -> TeamByoSettings {
    TeamByoSettings {
        first_party_enabled: true,
        endpoints_enabled: true,
        allow_user_keys,
        allow_user_endpoints,
        first_party_keys: vec![],
        endpoints: vec![],
    }
}

/// Two teams on one workspace with identical workspace-level BYO entitlement (derived from
/// team A's billing metadata by `Workspace::from_local_cache`) but opposite `team_byo`
/// policies, so only the team's own policy - not plan entitlement - can explain a difference
/// in behavior between them.
fn workspace_with_two_teams_of_opposing_byo_policy() -> (Team, Team, Workspace) {
    let team_a = Team::from_local_cache(
        111.into(),
        "team-a".to_string(),
        Some(TeamSettings {
            team_byo: Some(team_byo_settings(true, true)),
            ..Default::default()
        }),
        Some(BillingMetadata {
            tier: Tier {
                managed_byok_byoe_policy: Some(ManagedByokByoePolicy { enabled: true }),
                ..Default::default()
            },
            ..Default::default()
        }),
        None,
    );
    let team_b = Team::from_local_cache(
        222.into(),
        "team-b".to_string(),
        Some(TeamSettings {
            team_byo: Some(team_byo_settings(false, false)),
            ..Default::default()
        }),
        None,
        None,
    );
    let workspace = Workspace::from_local_cache(
        "workspace_uid123456789".to_string().into(),
        "test".to_string(),
        Some(vec![team_a.clone(), team_b.clone()]),
    );
    (team_a, team_b, workspace)
}

#[test]
fn apply_team_byo_policy_gates_member_credentials_by_team_policy() {
    let (team_a, team_b, mut workspace) = workspace_with_two_teams_of_opposing_byo_policy();
    for team in &mut workspace.teams {
        enable_aws_and_geap_hosts(team);
    }

    App::test((), |mut app| async move {
        let _geap_flag = FeatureFlag::GeminiEnterprise.override_enabled(true);
        app.update(|ctx| {
            warpui_extras::secure_storage::register_noop("test", ctx);
            warp_core::telemetry::testing::MockTelemetryContextProvider::register(ctx);
        });
        app.add_singleton_model(|_| AuthStateProvider::new_for_test());
        app.add_singleton_model(AISettings::new_with_defaults);
        app.add_singleton_model(|ctx| {
            UserWorkspaces::mock(
                Arc::new(MockTeamClient::new()),
                Arc::new(MockWorkspaceClient::new()),
                vec![workspace],
                ctx,
            )
        });
        let team_context_a = TeamContextForOperation::new_for_test(team_a.uid);
        let team_context_b = TeamContextForOperation::new_for_test(team_b.uid);
        let api_key_manager = app.add_singleton_model(ApiKeyManager::new);
        api_key_manager.update(&mut app, |manager, ctx| {
            manager
                .persist_provider_key(LLMProvider::Anthropic, Some("sk-ant-test".to_owned()), ctx)
                .expect("no-op secure storage should accept the provider key");
            // A member-provided custom endpoint, gated by `are_member_byo_endpoints_allowed_for_team`
            // separately from the provider-key gate above.
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
            // Org-level credentials: neither is gated by `team_byo` member policy, so both must
            // survive `apply_team_byo_policy` regardless of which team is requesting.
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
            let binding = match geap_policy_for_context(&team_context_a, ctx) {
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

        let mut request_params = RequestParams::new_for_test();
        app.read(|ctx| {
            let api_key_manager = ApiKeyManager::as_ref(ctx);
            let is_aws_bedrock_enabled = UserWorkspaces::as_ref(ctx)
                .is_aws_bedrock_credentials_enabled_for_context(Some(&team_context_a), ctx);
            let geap_binding = geap_policy_for_context(&team_context_a, ctx).mint_binding();
            request_params.api_keys = api_key_manager.api_keys_for_request(
                true,
                is_aws_bedrock_enabled,
                geap_binding.clone(),
            );
            request_params.geap_mint_binding = geap_binding;
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
            let mut allowed = request_params.clone();
            allowed.apply_team_byo_policy(&team_context_a, ctx);
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
            disallowed.apply_team_byo_policy(&team_context_b, ctx);
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
        });
    });
}
