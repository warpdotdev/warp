use std::collections::HashMap;

use warpui::App;

use super::{
    classify_agent_mode_base_model_id, parse_ambient_task_id, validate_agent_mode_base_model_id,
};
use crate::LaunchMode;
use crate::ai::execution_profiles::profiles::AIExecutionProfilesModel;
use crate::ai::llms::{
    AvailableLLMs, LLMContextWindow, LLMId, LLMInfo, LLMPreferences, LLMProvider, LLMUsageMetadata,
    ModelsByFeature,
};
use crate::ai::mcp::TemplatableMCPServerManager;
use crate::auth::AuthStateProvider;
use crate::auth::auth_manager::AuthManager;
use crate::cloud_object::model::persistence::CloudModel;
use crate::network::NetworkStatus;
use crate::server::cloud_objects::update_manager::UpdateManager;
use crate::server::server_api::ServerApiProvider;
use crate::server::sync_queue::SyncQueue;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::workspaces::team_tester::TeamTesterStatus;
use crate::workspaces::user_workspaces::UserWorkspaces;

#[test]
fn parse_ambient_task_id_accepts_valid_ids() {
    let task_id =
        parse_ambient_task_id("550e8400-e29b-41d4-a716-446655440000", "Invalid run ID").unwrap();

    assert_eq!(task_id.to_string(), "550e8400-e29b-41d4-a716-446655440000");
}

#[test]
fn parse_ambient_task_id_preserves_error_prefix() {
    let err = parse_ambient_task_id("not-a-run-id", "Invalid run ID").unwrap_err();

    assert!(err.to_string().contains("Invalid run ID 'not-a-run-id'"));
}

// -- validate_agent_mode_base_model_id unavailable-vs-invalid error heuristics --
//
// Regression tests for the scenario where the server is unhealthy and returns
// an empty/unavailable agent-mode model list. Previously both validators blamed
// the user's model id ("Unknown model id ..." / "is not a valid agent mode
// LLM"), hiding the real server-availability issue. The fix tracks the
// fetch-failure / list-unavailable state and surfaces a distinct error.

#[test]
fn classify_returns_server_unavailable_error_when_list_unavailable() {
    // Simulates an unhealthy server: the authed model-list fetch failed, so the
    // list is unavailable. The cached/default list is still non-empty (e.g.
    // "auto"), which is exactly the case that previously produced the
    // misleading "Unknown model id" error for any id not in the list.
    let valid_ids = vec![LLMId::from("auto")];
    let err = classify_agent_mode_base_model_id("claude-sonnet-4-5", &valid_ids, true)
        .expect_err("unavailable list should error");
    let msg = format!("{err:#}");
    assert!(
        !msg.contains("Unknown model id"),
        "should not blame the model id when the list is unavailable: {msg}"
    );
    assert!(
        msg.to_lowercase().contains("unavailable"),
        "should surface a server/model-list unavailability error: {msg}"
    );
}

#[test]
fn classify_returns_unknown_id_error_when_list_available_and_id_genuinely_invalid() {
    // A non-empty, available list that does not contain the id still produces
    // the existing "Unknown model id" error (with suggestions).
    let valid_ids = vec![LLMId::from("auto"), LLMId::from("gpt-x")];
    let err = classify_agent_mode_base_model_id("claude-sonnet-4-5", &valid_ids, false)
        .expect_err("genuinely invalid id should error");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("Unknown model id"),
        "should preserve the existing 'Unknown model id' error: {msg}"
    );
    assert!(
        msg.contains("auto") && msg.contains("gpt-x"),
        "should list the available model suggestions: {msg}"
    );
}

#[test]
fn classify_accepts_id_in_choices_even_when_list_unavailable() {
    // A custom-endpoint (local) id that is among the choices should still
    // validate even when the server list is unavailable, because custom
    // endpoints are independent of server health (the validator chains custom
    // choices alongside the server list).
    let valid_ids = vec![LLMId::from("custom-config-key")];
    let id = classify_agent_mode_base_model_id("custom-config-key", &valid_ids, true)
        .expect("an id present in the choices should validate");
    assert_eq!(id.as_str(), "custom-config-key");
}

// -- agent_mode_models_unavailable flag lifecycle (set on failed fetch,
// cleared via the shared on_server_update path) --
//
// Regression test for the stuck-flag scenario: a failed authed fetch sets the
// flag, and a later successful model-list update delivered through
// `update_feature_model_choices(Ok(..))` — the login / workspace-metadata
// path, which goes straight to `on_server_update` without touching
// `refresh_authed_models` — must clear it. Before the fix the clear lived only
// in `refresh_authed_models`'s Ok branch, so the flag stayed `true` and a
// genuinely-invalid id was misreported as "model list unavailable" instead of
// "Unknown model id".

fn server_llm(id: &str) -> LLMInfo {
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
        disable_reason: None,
        vision_supported: false,
        spec: None,
        provider: LLMProvider::Unknown,
        host_configs: HashMap::new(),
        discount_percentage: None,
        context_window: LLMContextWindow::default(),
    }
}

fn available(default_id: &str, choices: Vec<LLMInfo>) -> AvailableLLMs {
    AvailableLLMs::new(default_id.into(), choices, None).expect("choices are non-empty")
}

#[test]
fn update_feature_model_choices_clears_unavailable_flag_after_failed_fetch() {
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

        // Simulate a failed authed fetch: the server was unhealthy, so the
        // agent-mode model list is marked unavailable.
        llm_preferences.update(&mut app, |preferences, _| {
            preferences.set_agent_mode_models_unavailable(true);
        });
        assert!(
            llm_preferences.read(&app, |preferences, _| {
                preferences.agent_mode_models_unavailable()
            }),
            "flag should be set after a failed fetch"
        );

        // While the flag is set, a genuinely-invalid id is reported as a
        // server/model-list availability error rather than "Unknown model id".
        llm_preferences.read(&app, |_, app| {
            let err = validate_agent_mode_base_model_id("claude-sonnet-4-5", app)
                .expect_err("unavailable list should error");
            let msg = format!("{err:#}");
            assert!(
                !msg.contains("Unknown model id"),
                "should not blame the model id while the list is unavailable: {msg}"
            );
            assert!(
                msg.to_lowercase().contains("unavailable"),
                "should surface a server/model-list unavailability error: {msg}"
            );
        });

        // A later successful model-list update arrives through the login /
        // workspace-metadata path (`update_feature_model_choices(Ok(..))`),
        // which goes straight to `on_server_update` and previously bypassed
        // the flag clear.
        let models = ModelsByFeature {
            agent_mode: available("auto", vec![server_llm("auto"), server_llm("gpt-x")]),
            coding: available("auto", vec![server_llm("auto")]),
            cli_agent: Some(available(
                "cli-agent-auto",
                vec![server_llm("cli-agent-auto")],
            )),
            computer_use: None,
        };
        llm_preferences.update(&mut app, |preferences, ctx| {
            preferences.update_feature_model_choices(Ok(models), ctx);
        });

        // The successful update must have cleared the unavailable flag ...
        assert!(
            !llm_preferences.read(&app, |preferences, _| {
                preferences.agent_mode_models_unavailable()
            }),
            "a successful model-list update through update_feature_model_choices must clear the unavailable flag"
        );

        // ... so a genuinely-invalid id is now reported as "Unknown model id"
        // (with suggestions) rather than "model list unavailable".
        llm_preferences.read(&app, |_, app| {
            let err = validate_agent_mode_base_model_id("claude-sonnet-4-5", app)
                .expect_err("genuinely invalid id should still error");
            let msg = format!("{err:#}");
            assert!(
                msg.contains("Unknown model id"),
                "after the list is available again, a genuinely-invalid id should report 'Unknown model id': {msg}"
            );
        });
    });
}
