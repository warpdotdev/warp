use warpui::App;

use super::*;
use crate::ai::execution_profiles::profiles::AIExecutionProfilesModel;
use crate::ai::mcp::TemplatableMCPServerManager;
use crate::auth::auth_manager::AuthManager;
use crate::auth::AuthStateProvider;
use crate::cloud_object::model::persistence::CloudModel;
use crate::network::NetworkStatus;
use crate::server::cloud_objects::update_manager::UpdateManager;
use crate::server::server_api::ServerApiProvider;
use crate::server::sync_queue::SyncQueue;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::workspaces::team_tester::TeamTesterStatus;
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::LaunchMode;

// -- DisableReason::should_clear_preference tests --

#[test]
fn should_clear_preference_admin_disabled() {
    // AdminDisabled always clears, regardless of BYOK status.
    assert!(DisableReason::AdminDisabled.should_clear_preference(false));
    assert!(DisableReason::AdminDisabled.should_clear_preference(true));
}

#[test]
fn should_clear_preference_unavailable() {
    assert!(DisableReason::Unavailable.should_clear_preference(false));
    assert!(DisableReason::Unavailable.should_clear_preference(true));
}

#[test]
fn should_not_clear_preference_out_of_requests() {
    // Transient — never clears.
    assert!(!DisableReason::OutOfRequests.should_clear_preference(false));
    assert!(!DisableReason::OutOfRequests.should_clear_preference(true));
}

#[test]
fn should_not_clear_preference_provider_outage() {
    // Transient — never clears.
    assert!(!DisableReason::ProviderOutage.should_clear_preference(false));
    assert!(!DisableReason::ProviderOutage.should_clear_preference(true));
}

#[test]
fn should_clear_preference_requires_upgrade_without_byok() {
    // No BYOK key → server will reject → clear.
    assert!(DisableReason::RequiresUpgrade.should_clear_preference(false));
}

#[test]
fn should_not_clear_preference_requires_upgrade_with_byok() {
    // BYOK key present → server allows → keep.
    assert!(!DisableReason::RequiresUpgrade.should_clear_preference(true));
}

#[test]
fn llm_info_deserializes_without_base_model_name() {
    let raw = r#"{
            "display_name": "gpt-4o",
            "id": "gpt-4o",
            "usage_metadata": {
                "request_multiplier": 1,
                "credit_multiplier": null
            },
            "description": null,
            "disable_reason": null,
            "vision_supported": false,
            "spec": null,
            "provider": "Unknown"
        }"#;

    let info: LLMInfo = serde_json::from_str(raw).expect("should deserialize");
    assert_eq!(info.display_name, "gpt-4o");
    assert_eq!(info.base_model_name, "gpt-4o");
}

#[test]
fn llm_info_deserializes_host_configs_as_vec() {
    // Wire format from server: host_configs is a Vec
    let raw = r#"{
            "display_name": "gpt-4o",
            "id": "gpt-4o",
            "usage_metadata": { "request_multiplier": 1, "credit_multiplier": null },
            "provider": "OpenAI",
            "host_configs": [
                { "enabled": true, "model_routing_host": "DirectApi" },
                { "enabled": false, "model_routing_host": "AwsBedrock" }
            ]
        }"#;

    let info: LLMInfo = serde_json::from_str(raw).expect("should deserialize vec format");
    assert_eq!(info.display_name, "gpt-4o");
    assert_eq!(info.host_configs.len(), 2);
    assert!(
        info.host_configs
            .get(&LLMModelHost::DirectApi)
            .unwrap()
            .enabled
    );
    assert!(
        !info
            .host_configs
            .get(&LLMModelHost::AwsBedrock)
            .unwrap()
            .enabled
    );
}

#[test]
fn llm_info_round_trip_serializes_and_deserializes() {
    // Start with wire format (Vec)
    let wire_json = r#"{
            "display_name": "claude-3",
            "base_model_name": "claude-3",
            "id": "claude-3",
            "usage_metadata": { "request_multiplier": 2, "credit_multiplier": 1.5 },
            "description": "A powerful model",
            "vision_supported": true,
            "provider": "Anthropic",
            "host_configs": [
                { "enabled": true, "model_routing_host": "DirectApi" }
            ]
        }"#;

    // Deserialize from wire format
    let info: LLMInfo = serde_json::from_str(wire_json).expect("should deserialize");

    // Serialize (produces HashMap format)
    let serialized = serde_json::to_string(&info).expect("should serialize");

    // Deserialize again (from HashMap format)
    let round_tripped: LLMInfo =
        serde_json::from_str(&serialized).expect("should deserialize after round trip");

    assert_eq!(info, round_tripped);
}

// -- build_custom_llm_infos / display label tests --

fn endpoint(
    name: &str,
    url: &str,
    api_key: &str,
    models: Vec<CustomEndpointModel>,
) -> CustomEndpoint {
    CustomEndpoint {
        name: name.into(),
        url: url.into(),
        api_key: api_key.into(),
        models,
    }
}

fn model(name: &str, alias: Option<&str>, config_key: &str) -> CustomEndpointModel {
    CustomEndpointModel {
        name: name.into(),
        alias: alias.map(|s| s.into()),
        config_key: config_key.into(),
    }
}

#[test]
fn custom_llm_infos_built_from_endpoints() {
    let keys = ai::api_keys::ApiKeys {
        custom_endpoints: vec![endpoint(
            "My Endpoint",
            "https://x.io",
            "k",
            vec![
                model("gpt-4", Some("fast"), "uuid-1"),
                model("llama", None, "uuid-2"),
            ],
        )],
        ..Default::default()
    };
    let infos = build_custom_llm_infos(&keys);
    assert_eq!(infos.len(), 2);
    assert_eq!(infos[0].display_name, "fast");
    assert_eq!(infos[0].id.as_str(), "uuid-1");
    assert_eq!(
        infos[0].description.as_deref(),
        Some("Custom · My Endpoint")
    );
    assert_eq!(infos[1].display_name, "llama");
    assert_eq!(infos[1].id.as_str(), "uuid-2");
}

#[test]
fn custom_llm_display_name_uses_alias_when_present() {
    let keys = ai::api_keys::ApiKeys {
        custom_endpoints: vec![endpoint(
            "ep",
            "https://a.io",
            "k",
            vec![model("raw-name", Some("My Alias"), "uuid-a")],
        )],
        ..Default::default()
    };
    let infos = build_custom_llm_infos(&keys);
    assert_eq!(infos[0].display_name, "My Alias");
}

#[test]
fn custom_llm_display_name_falls_back_to_name_when_alias_missing() {
    let keys = ai::api_keys::ApiKeys {
        custom_endpoints: vec![endpoint(
            "ep",
            "https://a.io",
            "k",
            vec![model("raw-name", None, "uuid-a")],
        )],
        ..Default::default()
    };
    let infos = build_custom_llm_infos(&keys);
    assert_eq!(infos[0].display_name, "raw-name");
}

#[test]
fn custom_endpoint_usage_display_label_resolves_alias_name_and_generic_fallback() {
    let keys = ai::api_keys::ApiKeys {
        custom_endpoints: vec![endpoint(
            "ep",
            "https://a.io",
            "k",
            vec![
                model("raw-alias", Some("Alias"), "uuid-alias"),
                model("raw-name", None, "uuid-name"),
                model("raw~name", None, "uuid-tilde-name"),
            ],
        )],
        ..Default::default()
    };
    let preferences = LLMPreferences {
        models_by_feature: ModelsByFeature::default(),
        last_update: None,
        base_llm_for_terminal_view: HashMap::new(),
        custom_llms: build_custom_llm_infos(&keys),
        custom_model_routers: Vec::new(),
        server_models_loaded: false,
    };

    assert_eq!(
        preferences.custom_endpoint_usage_display_label("uuid-alias"),
        "Alias"
    );
    assert_eq!(
        preferences.custom_endpoint_usage_display_label("uuid-name"),
        "raw-name"
    );
    assert_eq!(
        preferences.custom_endpoint_usage_display_label("uuid-tilde-name"),
        "raw~name"
    );
    assert_eq!(
        preferences.custom_endpoint_usage_display_label("unknown"),
        CUSTOM_ENDPOINT_USAGE_FALLBACK_LABEL
    );
}

#[test]
fn custom_llm_infos_skip_endpoints_with_empty_api_key() {
    let keys = ai::api_keys::ApiKeys {
        custom_endpoints: vec![
            endpoint("bad", "https://a.io", "", vec![model("m", None, "uuid-x")]),
            endpoint(
                "good",
                "https://b.io",
                "k",
                vec![model("m", None, "uuid-y")],
            ),
        ],
        ..Default::default()
    };
    let infos = build_custom_llm_infos(&keys);
    assert_eq!(infos.len(), 1);
    assert_eq!(infos[0].id.as_str(), "uuid-y");
}

#[test]
fn custom_llm_infos_skip_models_without_config_key() {
    let keys = ai::api_keys::ApiKeys {
        custom_endpoints: vec![endpoint(
            "ep",
            "https://a.io",
            "k",
            vec![
                model("unconfigured", None, ""),
                model("ready", None, "uuid-a"),
            ],
        )],
        ..Default::default()
    };
    let infos = build_custom_llm_infos(&keys);
    assert_eq!(infos.len(), 1);
    assert_eq!(infos[0].display_name, "ready");
}

#[test]
fn removing_model_row_purges_from_custom_llms() {
    let before = ai::api_keys::ApiKeys {
        custom_endpoints: vec![endpoint(
            "ep",
            "https://a.io",
            "k",
            vec![model("a", None, "uuid-a"), model("b", None, "uuid-b")],
        )],
        ..Default::default()
    };
    assert_eq!(build_custom_llm_infos(&before).len(), 2);

    let after = ai::api_keys::ApiKeys {
        custom_endpoints: vec![endpoint(
            "ep",
            "https://a.io",
            "k",
            vec![model("b", None, "uuid-b")],
        )],
        ..Default::default()
    };
    let infos = build_custom_llm_infos(&after);
    assert_eq!(infos.len(), 1);
    assert_eq!(infos[0].id.as_str(), "uuid-b");
    assert!(infos.iter().all(|i| i.id.as_str() != "uuid-a"));
}

#[test]
fn removing_endpoint_purges_all_its_models_from_custom_llms() {
    let before = ai::api_keys::ApiKeys {
        custom_endpoints: vec![
            endpoint(
                "keep",
                "https://a.io",
                "k",
                vec![model("k1", None, "uuid-k1")],
            ),
            endpoint(
                "goner",
                "https://b.io",
                "k",
                vec![model("g1", None, "uuid-g1"), model("g2", None, "uuid-g2")],
            ),
        ],
        ..Default::default()
    };
    assert_eq!(build_custom_llm_infos(&before).len(), 3);

    let after = ai::api_keys::ApiKeys {
        custom_endpoints: vec![endpoint(
            "keep",
            "https://a.io",
            "k",
            vec![model("k1", None, "uuid-k1")],
        )],
        ..Default::default()
    };
    let infos = build_custom_llm_infos(&after);
    assert_eq!(infos.len(), 1);
    assert_eq!(infos[0].id.as_str(), "uuid-k1");
}

#[test]
fn reconcile_preserves_custom_models_saved_on_execution_profile() {
    App::test((), |mut app| async move {
        let _custom_inference_flag = FeatureFlag::CustomInferenceEndpoints.override_enabled(true);

        initialize_settings_for_tests(&mut app);
        app.add_singleton_model(|_| ServerApiProvider::new_for_test());
        app.add_singleton_model(|_| AuthStateProvider::new_for_test());
        app.add_singleton_model(AuthManager::new_for_test);
        app.add_singleton_model(|_| NetworkStatus::new());
        app.add_singleton_model(UserWorkspaces::default_mock);
        app.add_singleton_model(CloudModel::mock);
        app.add_singleton_model(TeamTesterStatus::mock);
        app.add_singleton_model(SyncQueue::mock);
        app.add_singleton_model(UpdateManager::mock);
        app.add_singleton_model(|_| TemplatableMCPServerManager::default());

        let profiles_model = app.add_singleton_model(|ctx| {
            AIExecutionProfilesModel::new(&LaunchMode::new_for_unit_test(), ctx)
        });
        let llm_preferences = app.add_singleton_model(LLMPreferences::new);

        let custom_model_id = LLMId::from("custom-model-config-key");
        ApiKeyManager::handle(&app).update(&mut app, |api_key_manager, ctx| {
            api_key_manager.add_custom_endpoint(
                "local".to_string(),
                "https://example.com/v1".to_string(),
                "test-key".to_string(),
                vec![(
                    "custom-model".to_string(),
                    Some("Custom Model".to_string()),
                    Some(custom_model_id.to_string()),
                )],
                ctx,
            );
        });

        let default_profile_id =
            profiles_model.read(&app, |profiles, _| profiles.default_profile_id());
        profiles_model.update(&mut app, |profiles, ctx| {
            profiles.set_base_model(default_profile_id, Some(custom_model_id.clone()), ctx);
            profiles.set_coding_model(default_profile_id, Some(custom_model_id.clone()), ctx);
            profiles.set_cli_agent_model(default_profile_id, Some(custom_model_id.clone()), ctx);
        });

        llm_preferences.update(&mut app, |preferences, ctx| {
            preferences.update_feature_model_choices(Ok(ModelsByFeature::default()), ctx);
        });

        profiles_model.read(&app, |profiles, ctx| {
            let profile = profiles.default_profile(ctx);
            assert_eq!(profile.data().base_model.as_ref(), Some(&custom_model_id));
            assert_eq!(profile.data().coding_model.as_ref(), Some(&custom_model_id));
            assert_eq!(
                profile.data().cli_agent_model.as_ref(),
                Some(&custom_model_id)
            );
        });
    });
}
