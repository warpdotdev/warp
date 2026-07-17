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
use crate::terminal::input::models::query_model_picker_choices;
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

#[test]
fn host_icon_visibility_requires_enabled_credentials_and_model_host() {
    let mut info = server_llm("gemini-test", None);
    info.host_configs.insert(
        LLMModelHost::GeminiEnterprise,
        RoutingHostConfig {
            enabled: true,
            model_routing_host: LLMModelHost::GeminiEnterprise,
        },
    );

    assert!(should_show_host_icon_for_model(
        &info,
        &LLMModelHost::GeminiEnterprise,
        true,
    ));
    assert!(!should_show_host_icon_for_model(
        &info,
        &LLMModelHost::GeminiEnterprise,
        false,
    ));
    assert!(!should_show_host_icon_for_model(
        &info,
        &LLMModelHost::AwsBedrock,
        true,
    ));

    info.host_configs
        .get_mut(&LLMModelHost::GeminiEnterprise)
        .expect("Gemini Enterprise host should exist")
        .enabled = false;
    assert!(!should_show_host_icon_for_model(
        &info,
        &LLMModelHost::GeminiEnterprise,
        true,
    ));
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

fn disabled_agent_llm(id: &str, display_name: &str) -> LLMInfo {
    LLMInfo {
        disable_reason: Some(DisableReason::Unavailable),
        ..agent_llm(id, display_name)
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

// -- is_cloud_runnable_oz_model_id tests --

#[test]
fn is_cloud_runnable_oz_model_id_classifies_ids() {
    // A custom-endpoint (BYOK) model whose id is a bare `config_key` UUID —
    // this is the id that triggered the reported handoff failure.
    let keys = ai::api_keys::ApiKeys {
        custom_endpoints: vec![endpoint(
            "ep",
            "https://a.io",
            "k",
            vec![model("gpt", None, "52941f14-1b74-4afa-8f02-cdd5243b5aa9")],
        )],
        ..Default::default()
    };
    let preferences = LLMPreferences {
        models_by_feature: ModelsByFeature::default(),
        last_update: None,
        base_llm_for_terminal_view: HashMap::new(),
        custom_llms: build_custom_llm_infos(&keys),
        custom_model_routers: Vec::new(),
    };

    // Custom-endpoint (BYOK) UUID id — not cloud-runnable.
    assert!(!preferences
        .is_cloud_runnable_oz_model_id(&LLMId::from("52941f14-1b74-4afa-8f02-cdd5243b5aa9")));
    // Local custom router — not cloud-runnable.
    assert!(
        !preferences.is_cloud_runnable_oz_model_id(&LLMId::from("custom-router:local:my-router"))
    );
    // Cloud/team custom router — cloud-runnable: the server accepts the
    // `custom-router:cloud:` prefix at spawn and resolves it server-side.
    assert!(
        preferences.is_cloud_runnable_oz_model_id(&LLMId::from("custom-router:cloud:team-router"))
    );
    // Warp Oz slugs — cloud-runnable.
    assert!(preferences.is_cloud_runnable_oz_model_id(&LLMId::from("auto")));
    assert!(preferences.is_cloud_runnable_oz_model_id(&LLMId::from("auto-genius")));
    // A server-provided (non-custom, non-local-router) id is treated as
    // runnable; only definitively non-runnable ids are downgraded.
    assert!(preferences.is_cloud_runnable_oz_model_id(&LLMId::from("claude-4-opus")));
}

// -- Disable-aware default fallback tests --

fn server_llm(id: &str, disable_reason: Option<DisableReason>) -> LLMInfo {
    LLMInfo {
        display_name: id.to_string(),
        base_model_name: id.to_string(),
        id: id.into(),
        reasoning_level: None,
        usage_metadata: LLMUsageMetadata {
            request_multiplier: 1,
            credit_multiplier: None,
        },
        description: None,
        disable_reason,
        vision_supported: false,
        spec: None,
        provider: LLMProvider::Unknown,
        host_configs: HashMap::new(),
        discount_percentage: None,
        context_window: LLMContextWindow::default(),
    }
}

fn available(default_id: &str, choices: Vec<LLMInfo>) -> AvailableLLMs {
    AvailableLLMs {
        default_id: default_id.into(),
        choices,
        preferred_codex_model_id: None,
    }
}

#[test]
fn deserialized_available_llms_with_missing_default_does_not_panic() {
    // `AvailableLLMs::new()` guarantees `default_id` is one of `choices`, but
    // deserialization (e.g. a stale persisted cache or a server payload)
    // bypasses `new()`. Build such a struct, round-trip it through serde, and
    // confirm `default_llm_info()` falls back to the first choice instead of
    // panicking (Sentry: "Default LLM ID must be present in choices").
    let original = available(
        "missing-default",
        vec![server_llm("gpt-x", None), server_llm("gpt-y", None)],
    );
    let json = serde_json::to_string(&original).expect("should serialize");
    let deserialized: AvailableLLMs = serde_json::from_str(&json).expect("should deserialize");

    assert_eq!(deserialized.default_id.as_str(), "missing-default");
    assert_eq!(deserialized.default_llm_info().id.as_str(), "gpt-x");
}

#[test]
fn active_models_fall_back_to_usable_choice_or_custom_endpoint_when_default_disabled() {
    App::test((), |mut app| async move {
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

        app.add_singleton_model(|ctx| {
            AIExecutionProfilesModel::new(&LaunchMode::new_for_unit_test(), ctx)
        });
        let llm_preferences = app.add_singleton_model(LLMPreferences::new);

        let custom_model_id = LLMId::from("custom-config-key");
        ApiKeyManager::handle(&app).update(&mut app, |api_key_manager, ctx| {
            api_key_manager.add_custom_endpoint(
                "local".to_string(),
                "https://example.com/v1".to_string(),
                "test-key".to_string(),
                vec![(
                    "custom-model".to_string(),
                    None,
                    Some(custom_model_id.to_string()),
                )],
                ctx,
            );
        });

        // The base/coding default is admin-disabled but another hosted choice
        // is usable; every hosted CLI agent choice is admin-disabled.
        let models = ModelsByFeature {
            agent_mode: available(
                "auto",
                vec![
                    server_llm("auto", Some(DisableReason::AdminDisabled)),
                    server_llm("gpt-x", None),
                ],
            ),
            coding: available(
                "auto",
                vec![
                    server_llm("auto", Some(DisableReason::AdminDisabled)),
                    server_llm("gpt-x", None),
                ],
            ),
            cli_agent: Some(available(
                "cli-agent-auto",
                vec![server_llm(
                    "cli-agent-auto",
                    Some(DisableReason::AdminDisabled),
                )],
            )),
            computer_use: None,
        };
        llm_preferences.update(&mut app, |preferences, ctx| {
            preferences.update_feature_model_choices(Ok(models), ctx);
        });

        llm_preferences.read(&app, |preferences, app| {
            // Falls back to the first usable hosted choice.
            assert_eq!(
                preferences.get_active_base_model(app, None).id.as_str(),
                "gpt-x"
            );
            assert_eq!(
                preferences.get_active_coding_model(app, None).id.as_str(),
                "gpt-x"
            );
            // No usable hosted CLI choice → falls back to the custom endpoint.
            assert_eq!(
                preferences.get_active_cli_agent_model(app, None).id,
                custom_model_id
            );
        });
    });
}

/// Runs picker-query assertions with searchable, selectable, and disabled model fixtures plus
/// the app singletons consulted by model eligibility logic.
fn with_model_picker_query_test_context(f: impl FnOnce(&LLMPreferences, &AppContext) + 'static) {
    App::test((), |app| async move {
        app.add_singleton_model(|_| AuthStateProvider::new_for_test());
        app.add_singleton_model(UserWorkspaces::default_mock);
        app.read(|app_ctx| {
            let agent_mode = AvailableLLMs::new(
                "auto".into(),
                vec![
                    agent_llm("auto", "auto (cost-efficient)"),
                    agent_llm("gpt-5", "GPT 5"),
                    disabled_agent_llm("disabled-gpt", "GPT Disabled"),
                ],
                None,
            )
            .expect("choices are non-empty");
            let preferences = LLMPreferences {
                models_by_feature: ModelsByFeature {
                    agent_mode,
                    ..Default::default()
                },
                last_update: None,
                base_llm_for_terminal_view: HashMap::new(),
                custom_llms: Vec::new(),
                custom_model_routers: Vec::new(),
            };
            f(&preferences, app_ctx);
        });
    });
}

#[test]
fn active_models_use_default_when_usable() {
    App::test((), |mut app| async move {
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

        app.add_singleton_model(|ctx| {
            AIExecutionProfilesModel::new(&LaunchMode::new_for_unit_test(), ctx)
        });
        let llm_preferences = app.add_singleton_model(LLMPreferences::new);

        let models = ModelsByFeature {
            agent_mode: available(
                "auto",
                vec![server_llm("auto", None), server_llm("gpt-x", None)],
            ),
            coding: available("auto", vec![server_llm("auto", None)]),
            cli_agent: Some(available(
                "cli-agent-auto",
                vec![server_llm("cli-agent-auto", None)],
            )),
            computer_use: None,
        };
        llm_preferences.update(&mut app, |preferences, ctx| {
            preferences.update_feature_model_choices(Ok(models), ctx);
        });

        llm_preferences.read(&app, |preferences, app| {
            assert_eq!(
                preferences.get_active_base_model(app, None).id.as_str(),
                "auto"
            );
            assert_eq!(
                preferences
                    .get_active_cli_agent_model(app, None)
                    .id
                    .as_str(),
                "cli-agent-auto"
            );
        });
    });
}

#[test]
fn reconcile_preserves_custom_models_saved_on_execution_profile() {
    App::test((), |mut app| async move {
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

#[test]
fn reconcile_preserves_custom_endpoint_models_not_configured_locally() {
    // Regression test for QUALITY-866: a profile whose model was set to a custom
    // endpoint on device A should NOT be reset when device B syncs that profile
    // but does not have the corresponding custom endpoint configured.
    //
    // Before the fix, `reconcile_disabled_model_preferences` would clear any model
    // ID that couldn't be resolved locally, causing the profile to revert to Auto
    // and syncing that change back to cloud — erasing the user's setting on device A.
    //
    // The `context_window_limit` clear is a separately-guarded branch in
    // `reconcile_disabled_model_preferences` (gated on
    // `preferred_base_model_is_recognized`), so this test also sets a limit and
    // asserts it is preserved for the unrecognized custom endpoint ID.
    App::test((), |mut app| async move {
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

        // Simulate a model ID from a custom endpoint on another device.
        // This device (device B) does NOT have the endpoint configured locally.
        let remote_custom_model_id = LLMId::from("a1b2c3d4-5e6f-7890-abcd-ef1234567890");
        // Intentionally skip adding the endpoint to ApiKeyManager.

        let default_profile_id =
            profiles_model.read(&app, |profiles, _| profiles.default_profile_id());
        // Also set a context window limit so the separately-guarded
        // `context_window_limit` clear branch in `reconcile_disabled_model_preferences`
        // is exercised: it must NOT clear the limit for an unrecognized model ID.
        let preserved_context_window_limit: u32 = 200_000;
        profiles_model.update(&mut app, |profiles, ctx| {
            profiles.set_base_model(
                default_profile_id,
                Some(remote_custom_model_id.clone()),
                ctx,
            );
            profiles.set_coding_model(
                default_profile_id,
                Some(remote_custom_model_id.clone()),
                ctx,
            );
            profiles.set_cli_agent_model(
                default_profile_id,
                Some(remote_custom_model_id.clone()),
                ctx,
            );
            profiles.set_context_window_limit(
                default_profile_id,
                Some(preserved_context_window_limit),
                ctx,
            );
        });

        // Trigger a model list refresh (as happens on login, network reconnect, etc.).
        llm_preferences.update(&mut app, |preferences, ctx| {
            preferences.update_feature_model_choices(Ok(ModelsByFeature::default()), ctx);
        });

        // The model IDs should be PRESERVED even though no matching custom endpoint
        // is configured on this device.
        profiles_model.read(&app, |profiles, ctx| {
            let profile = profiles.default_profile(ctx);
            assert_eq!(
                profile.data().base_model.as_ref(),
                Some(&remote_custom_model_id),
                "base_model must be preserved for unknown custom endpoint IDs (cross-device sync)"
            );
            assert_eq!(
                profile.data().coding_model.as_ref(),
                Some(&remote_custom_model_id),
                "coding_model must be preserved for unknown custom endpoint IDs (cross-device sync)"
            );
            assert_eq!(
                profile.data().cli_agent_model.as_ref(),
                Some(&remote_custom_model_id),
                "cli_agent_model must be preserved for unknown custom endpoint IDs (cross-device sync)"
            );
            assert_eq!(
                profile.data().context_window_limit,
                Some(preserved_context_window_limit),
                "context_window_limit must be preserved for unknown custom endpoint IDs (cross-device sync)"
            );
        });
    });
}

// -- tui_agent_model_info tests --

fn agent_llm(id: &str, display_name: &str) -> LLMInfo {
    LLMInfo {
        display_name: display_name.to_owned(),
        base_model_name: display_name.to_owned(),
        id: id.into(),
        reasoning_level: None,
        usage_metadata: LLMUsageMetadata {
            request_multiplier: 1,
            credit_multiplier: None,
        },
        description: None,
        disable_reason: None,
        vision_supported: false,
        spec: None,
        provider: LLMProvider::Unknown,
        host_configs: HashMap::new(),
        discount_percentage: None,
        context_window: LLMContextWindow::default(),
    }
}

/// Preferences whose agent-mode models are a server-style list with an
/// `"auto"` default plus one concrete model.
fn preferences_for_tui_tests() -> LLMPreferences {
    let agent_mode = AvailableLLMs::new(
        "auto".into(),
        vec![
            agent_llm("auto", "auto (cost-efficient)"),
            agent_llm("claude-opus", "Opus"),
        ],
        None,
    )
    .expect("choices are non-empty");
    LLMPreferences {
        models_by_feature: ModelsByFeature {
            agent_mode,
            ..Default::default()
        },
        last_update: None,
        base_llm_for_terminal_view: HashMap::new(),
        custom_llms: Vec::new(),
        custom_model_routers: Vec::new(),
    }
}

/// Runs `f` against a test app with the singletons the shared model
/// resolution path (`model_info_for_id`) consults for custom-endpoint gating.
fn tui_agent_model_test(f: impl FnOnce(&LLMPreferences, &AppContext) + 'static) {
    App::test((), |app| async move {
        app.add_singleton_model(|_| AuthStateProvider::new_for_test());
        app.add_singleton_model(UserWorkspaces::default_mock);
        app.read(|app_ctx| f(&preferences_for_tui_tests(), app_ctx));
    });
}

#[test]
fn tui_agent_model_auto_resolves_to_the_default_model() {
    tui_agent_model_test(|preferences, app| {
        assert_eq!(
            preferences.tui_agent_model_info("auto", app).id.as_str(),
            "auto"
        );
    });
}

#[test]
fn shared_model_picker_query_orders_filters_and_marks_disabled_choices() {
    with_model_picker_query_test_context(|preferences, app| {
        let all = query_model_picker_choices(
            preferences,
            preferences.get_base_llm_choices_for_agent_mode(app),
            "",
            app,
        );
        assert_eq!(
            all.first().map(|choice| choice.llm.id.as_str()),
            Some("auto")
        );
        assert_eq!(
            all.last().map(|choice| choice.llm.id.as_str()),
            Some("disabled-gpt")
        );
        assert!(!all.last().expect("disabled choice").is_selectable());

        let filtered = query_model_picker_choices(
            preferences,
            preferences.get_base_llm_choices_for_agent_mode(app),
            "gpt 5",
            app,
        );
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].llm.id.as_str(), "gpt-5");
        assert!(filtered[0].name_match_result.is_some());
        assert!(filtered[0].is_selectable());
    });
}

#[test]
fn tui_agent_model_known_id_resolves_to_that_model() {
    tui_agent_model_test(|preferences, app| {
        let info = preferences.tui_agent_model_info("claude-opus", app);
        assert_eq!(info.id.as_str(), "claude-opus");
        assert_eq!(info.display_name, "Opus");
    });
}

#[test]
fn tui_agent_model_unknown_id_falls_back_to_the_default_model() {
    tui_agent_model_test(|preferences, app| {
        assert_eq!(
            preferences
                .tui_agent_model_info("not-a-model", app)
                .id
                .as_str(),
            "auto"
        );
    });
}
