use ai::agent::action::RunAgentsExecutionMode;
use ai::agent::orchestration_config::{OrchestrationConfig, OrchestrationExecutionMode};

use super::{
    should_show_auth_secret_picker, should_show_harness_picker, AuthSecretSelection,
    OrchestrationEditState,
};

fn remote_claude_state() -> OrchestrationEditState {
    OrchestrationEditState::from_run_agents_fields(
        "sonnet",
        "claude",
        &RunAgentsExecutionMode::Remote {
            environment_id: "env-1".to_string(),
            worker_host: "warp".to_string(),
            computer_use_enabled: false,
        },
    )
}

fn local_config(harness_type: &str, model_id: &str) -> OrchestrationConfig {
    OrchestrationConfig {
        model_id: model_id.to_string(),
        harness_type: harness_type.to_string(),
        execution_mode: OrchestrationExecutionMode::Local,
    }
}

#[test]
fn from_orchestration_config_preserves_local_claude() {
    let state =
        OrchestrationEditState::from_orchestration_config(&local_config("claude", "sonnet"));
    assert_eq!(state.harness_type, "claude");
    assert_eq!(state.model_id, "sonnet");
    assert!(matches!(
        state.execution_mode,
        RunAgentsExecutionMode::Local
    ));
}

#[test]
fn harness_picker_stays_visible_for_local_mode() {
    let state = OrchestrationEditState::from_run_agents_fields(
        "auto",
        "oz",
        &RunAgentsExecutionMode::Local,
    );
    assert!(should_show_harness_picker(&state));
}

#[test]
fn harness_picker_stays_visible_for_remote_mode() {
    let state = OrchestrationEditState::from_run_agents_fields(
        "auto",
        "oz",
        &RunAgentsExecutionMode::Remote {
            environment_id: "env-1".to_string(),
            worker_host: "warp".to_string(),
            computer_use_enabled: false,
        },
    );

    assert!(should_show_harness_picker(&state));
}

#[test]
fn from_orchestration_config_preserves_remote_claude() {
    let state = OrchestrationEditState::from_orchestration_config(&OrchestrationConfig {
        model_id: "sonnet".to_string(),
        harness_type: "claude".to_string(),
        execution_mode: OrchestrationExecutionMode::Remote {
            environment_id: "env-1".to_string(),
            worker_host: "warp".to_string(),
        },
    });

    assert_eq!(state.harness_type, "claude");
    assert_eq!(state.model_id, "sonnet");
    assert!(matches!(
        state.execution_mode,
        RunAgentsExecutionMode::Remote {
            ref environment_id,
            ref worker_host,
            computer_use_enabled: false,
        } if environment_id == "env-1" && worker_host == "warp"
    ));
}

#[test]
fn toggle_to_local_sanitizes_disabled_codex() {
    let mut state = OrchestrationEditState::from_run_agents_fields(
        "gpt-5",
        "codex",
        &RunAgentsExecutionMode::Remote {
            environment_id: "env-1".to_string(),
            worker_host: "warp".to_string(),
            computer_use_enabled: false,
        },
    );

    state.toggle_execution_mode_to_remote(false);

    assert_eq!(state.harness_type, "oz");
    assert_eq!(state.model_id, "");
    assert!(matches!(
        state.execution_mode,
        RunAgentsExecutionMode::Local
    ));
}

#[test]
fn toggle_to_local_preserves_claude() {
    let mut state = OrchestrationEditState::from_run_agents_fields(
        "sonnet",
        "claude",
        &RunAgentsExecutionMode::Remote {
            environment_id: "env-1".to_string(),
            worker_host: "warp".to_string(),
            computer_use_enabled: false,
        },
    );

    state.toggle_execution_mode_to_remote(false);

    assert_eq!(state.harness_type, "claude");
    assert_eq!(state.model_id, "sonnet");
    assert!(matches!(
        state.execution_mode,
        RunAgentsExecutionMode::Local
    ));
}

#[test]
fn accept_disabled_reason_allows_local_claude_product() {
    let state = OrchestrationEditState::from_run_agents_fields(
        "auto",
        "claude",
        &RunAgentsExecutionMode::Local,
    );
    assert_eq!(state.accept_disabled_reason(), None);
}

#[test]
fn resolve_from_config_preserves_local_claude() {
    let mut state =
        OrchestrationEditState::from_run_agents_fields("", "", &RunAgentsExecutionMode::Local);

    state.resolve_from_config(&local_config("claude", "sonnet"));
    assert_eq!(state.harness_type, "claude");
    assert_eq!(state.model_id, "sonnet");
    assert!(matches!(
        state.execution_mode,
        RunAgentsExecutionMode::Local
    ));
}

#[test]
fn resolve_from_config_sanitizes_disabled_local_codex() {
    let mut state =
        OrchestrationEditState::from_run_agents_fields("", "", &RunAgentsExecutionMode::Local);

    state.resolve_from_config(&local_config("codex", "gpt-5"));

    assert_eq!(state.harness_type, "oz");
    assert_eq!(state.model_id, "");
    assert!(matches!(
        state.execution_mode,
        RunAgentsExecutionMode::Local
    ));
}

#[test]
fn select_create_new_auth_secret_marks_creating_new_from_named() {
    let mut state = remote_claude_state();
    state.auth_secret_selection = AuthSecretSelection::Named("my-key".to_string());
    assert_eq!(state.auth_secret_name(), Some("my-key"));

    state.select_create_new_auth_secret();

    // `CreatingNew` (distinct from `Unset`) blocks Accept and isn't re-seeded.
    assert!(matches!(
        state.auth_secret_selection,
        AuthSecretSelection::CreatingNew
    ));
    assert_eq!(state.auth_secret_name(), None);
    assert!(should_show_auth_secret_picker(&state));
}

#[test]
fn select_create_new_auth_secret_marks_creating_new_from_inherit() {
    let mut state = remote_claude_state();
    state.auth_secret_selection = AuthSecretSelection::Inherit;

    state.select_create_new_auth_secret();

    assert!(matches!(
        state.auth_secret_selection,
        AuthSecretSelection::CreatingNew
    ));
}

/// Tests for the run-wide model-availability accept gate and the auto-select
/// substitution behavior. These exercise `accept_disabled_reason_with_auth`
/// and `maybe_auto_select_valid_model` against a populated Oz model catalog.
mod model_availability_gate_tests {
    use ai::agent::action::RunAgentsExecutionMode;
    use settings::Setting;
    use warp_core::features::FeatureFlag;
    use warpui::{App, SingletonEntity};

    use super::super::{accept_disabled_reason_with_auth, maybe_auto_select_valid_model};
    use super::OrchestrationEditState;
    use crate::ai::execution_profiles::profiles::AIExecutionProfilesModel;
    use crate::ai::llms::{AvailableLLMs, LLMId, LLMInfo, LLMPreferences, ModelsByFeature};
    use crate::ai::mcp::TemplatableMCPServerManager;
    use crate::auth::auth_manager::AuthManager;
    use crate::auth::AuthStateProvider;
    use crate::cloud_object::model::persistence::CloudModel;
    use crate::network::NetworkStatus;
    use crate::server::cloud_objects::update_manager::UpdateManager;
    use crate::server::server_api::ServerApiProvider;
    use crate::server::sync_queue::SyncQueue;
    use crate::settings::{AISettings, CloudAgentInvalidModelBehavior};
    use crate::test_util::settings::initialize_settings_for_tests;
    use crate::workspaces::team_tester::TeamTesterStatus;
    use crate::workspaces::user_workspaces::UserWorkspaces;
    use crate::LaunchMode;

    fn llm_info(id: &str) -> LLMInfo {
        serde_json::from_str(&format!(
            r#"{{"display_name":"{id}","id":"{id}","usage_metadata":{{"request_multiplier":1,"credit_multiplier":null}},"provider":"Unknown"}}"#
        ))
        .expect("llm_info json should deserialize")
    }

    /// Builds a `ModelsByFeature` whose Oz `agent_mode` catalog contains exactly
    /// `ids`, with `default_id` as the auto-select fallback.
    fn catalog(default_id: &str, ids: &[&str]) -> ModelsByFeature {
        let choices: Vec<LLMInfo> = ids.iter().map(|id| llm_info(id)).collect();
        let agent_mode = AvailableLLMs::new(LLMId::from(default_id), choices, None)
            .expect("agent_mode catalog should build");
        ModelsByFeature {
            agent_mode,
            ..Default::default()
        }
    }

    fn setup(app: &mut App, models: ModelsByFeature) {
        initialize_settings_for_tests(app);
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
        llm_preferences.update(app, |prefs, ctx| {
            prefs.update_feature_model_choices(Ok(models), ctx);
        });
    }

    fn set_behavior(app: &mut App, behavior: CloudAgentInvalidModelBehavior) {
        AISettings::handle(app).update(app, |settings, ctx| {
            settings
                .cloud_agent_invalid_model_behavior
                .set_value(behavior, ctx)
                .expect("set invalid-model behavior");
        });
    }

    fn remote_oz_state(model: &str) -> OrchestrationEditState {
        OrchestrationEditState::from_run_agents_fields(
            model,
            "oz",
            &RunAgentsExecutionMode::Remote {
                environment_id: "env-1".to_string(),
                worker_host: "warp".to_string(),
                computer_use_enabled: false,
            },
        )
    }

    fn local_oz_state(model: &str) -> OrchestrationEditState {
        OrchestrationEditState::from_run_agents_fields(model, "oz", &RunAgentsExecutionMode::Local)
    }

    #[test]
    fn block_disables_accept_for_unavailable_cloud_model() {
        App::test((), |mut app| async move {
            setup(&mut app, catalog("valid-a", &["valid-a", "valid-b"]));
            set_behavior(&mut app, CloudAgentInvalidModelBehavior::Block);

            app.update(|ctx| {
                let mut state = remote_oz_state("bad-model");
                let reason = accept_disabled_reason_with_auth(&state, ctx);
                assert!(
                    reason
                        .as_deref()
                        .is_some_and(|r| r.contains("cloud agents")),
                    "Block should disable Accept for an unavailable cloud model, got {reason:?}"
                );
                // Auto-select is a no-op under Block, so the model stays unchanged.
                assert!(!maybe_auto_select_valid_model(&mut state, ctx));
                assert_eq!(state.model_id, "bad-model");
            });
        });
    }

    #[test]
    fn auto_select_allows_and_substitutes_unavailable_cloud_model() {
        App::test((), |mut app| async move {
            setup(&mut app, catalog("valid-a", &["valid-a", "valid-b"]));
            set_behavior(&mut app, CloudAgentInvalidModelBehavior::AutoSelect);

            app.update(|ctx| {
                let mut state = remote_oz_state("bad-model");
                // The gate does not block under auto-select.
                assert_eq!(accept_disabled_reason_with_auth(&state, ctx), None);
                // The substitution swaps in the Oz cloud default model.
                assert!(maybe_auto_select_valid_model(&mut state, ctx));
                assert_eq!(state.model_id, "valid-a");
            });
        });
    }

    #[test]
    fn valid_cloud_model_is_not_blocked_or_substituted() {
        App::test((), |mut app| async move {
            setup(&mut app, catalog("valid-a", &["valid-a", "valid-b"]));
            set_behavior(&mut app, CloudAgentInvalidModelBehavior::Block);

            app.update(|ctx| {
                let state = remote_oz_state("valid-b");
                assert_eq!(accept_disabled_reason_with_auth(&state, ctx), None);
            });

            set_behavior(&mut app, CloudAgentInvalidModelBehavior::AutoSelect);
            app.update(|ctx| {
                let mut state = remote_oz_state("valid-b");
                assert!(!maybe_auto_select_valid_model(&mut state, ctx));
                assert_eq!(state.model_id, "valid-b");
            });
        });
    }

    #[test]
    fn empty_and_auto_models_are_always_allowed() {
        App::test((), |mut app| async move {
            setup(&mut app, catalog("valid-a", &["valid-a", "valid-b"]));
            set_behavior(&mut app, CloudAgentInvalidModelBehavior::Block);

            app.update(|ctx| {
                assert_eq!(
                    accept_disabled_reason_with_auth(&remote_oz_state(""), ctx),
                    None
                );
                assert_eq!(
                    accept_disabled_reason_with_auth(&remote_oz_state("auto"), ctx),
                    None
                );
            });
        });
    }

    #[test]
    fn local_only_custom_model_allowed_locally_but_flagged_for_cloud() {
        App::test((), |mut app| async move {
            let _custom_inference = FeatureFlag::CustomInferenceEndpoints.override_enabled(true);
            setup(&mut app, catalog("valid-a", &["valid-a", "valid-b"]));
            set_behavior(&mut app, CloudAgentInvalidModelBehavior::Block);

            let custom_id = "custom-model-config-key";
            ai::api_keys::ApiKeyManager::handle(&app).update(&mut app, |api_key_manager, ctx| {
                api_key_manager.add_custom_endpoint(
                    "local".to_string(),
                    "https://example.com/v1".to_string(),
                    "test-key".to_string(),
                    vec![(
                        "custom-model".to_string(),
                        Some("Custom Model".to_string()),
                        Some(custom_id.to_string()),
                    )],
                    ctx,
                );
            });

            app.update(|ctx| {
                // Legitimately runnable locally: no block for a local run.
                assert_eq!(
                    accept_disabled_reason_with_auth(&local_oz_state(custom_id), ctx),
                    None,
                    "local custom model should be allowed for local runs"
                );
                // But not accepted by Oz for cloud runs.
                assert!(
                    accept_disabled_reason_with_auth(&remote_oz_state(custom_id), ctx).is_some(),
                    "local custom model should be flagged for cloud runs"
                );
            });
        });
    }

    fn set_fallback_model(app: &mut App, model_id: &str) {
        AISettings::handle(app).update(app, |settings, ctx| {
            settings
                .cloud_agent_fallback_model_id
                .set_value(model_id.to_string(), ctx)
                .expect("set fallback model");
        });
    }

    #[test]
    fn auto_select_uses_configured_fallback_model() {
        App::test((), |mut app| async move {
            setup(&mut app, catalog("valid-a", &["valid-a", "valid-b"]));
            set_behavior(&mut app, CloudAgentInvalidModelBehavior::AutoSelect);
            // Configure an explicit fallback distinct from the catalog default.
            set_fallback_model(&mut app, "valid-b");

            app.update(|ctx| {
                let mut state = remote_oz_state("bad-model");
                // Substitution uses the configured fallback, not the default.
                assert!(maybe_auto_select_valid_model(&mut state, ctx));
                assert_eq!(state.model_id, "valid-b");
            });
        });
    }

    #[test]
    fn auto_select_falls_back_to_default_when_configured_fallback_unavailable() {
        App::test((), |mut app| async move {
            setup(&mut app, catalog("valid-a", &["valid-a", "valid-b"]));
            set_behavior(&mut app, CloudAgentInvalidModelBehavior::AutoSelect);
            // A configured fallback that isn't a valid cloud model is ignored in
            // favor of the Oz default.
            set_fallback_model(&mut app, "also-bad");

            app.update(|ctx| {
                let mut state = remote_oz_state("bad-model");
                assert!(maybe_auto_select_valid_model(&mut state, ctx));
                assert_eq!(state.model_id, "valid-a");
            });
        });
    }
}
