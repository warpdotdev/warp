use std::collections::HashMap;

use ai::agent::action::RunAgentsExecutionMode;

use super::OrchestrationEditState;
use crate::ai::orchestration::config_state::{AuthSecretSelection, OrchestrationConfigState};

fn remote_mode() -> RunAgentsExecutionMode {
    RunAgentsExecutionMode::Remote {
        environment_id: "env-1".to_string(),
        worker_host: "warp".to_string(),
        computer_use_enabled: false,
    }
}

fn model_valid_among<'a>(valid: &'a [&'a str]) -> impl Fn(&str, &str, bool) -> bool + 'a {
    move |id, _harness, _is_local| valid.contains(&id)
}

#[test]
fn execution_mode_change_to_local_forces_oz_and_strips_cloud_fields() {
    let mut state = OrchestrationConfigState::from_run_agents_fields(
        Some("gpt-5"),
        Some("codex"),
        &remote_mode(),
    );

    state.apply_execution_mode_change_core(false, None, None, &model_valid_among(&[""]), &|_| {
        Some(String::new())
    });

    assert_eq!(state.harness_type, "oz");
    assert_eq!(state.model_id, "");
    assert!(matches!(
        state.execution_mode,
        RunAgentsExecutionMode::Local
    ));
}

#[test]
fn execution_mode_change_to_cloud_prefills_default_environment() {
    let mut state = OrchestrationConfigState::from_run_agents_fields(
        Some("auto"),
        Some("oz"),
        &RunAgentsExecutionMode::Local,
    );

    state.apply_execution_mode_change_core(
        true,
        None,
        Some("env-42".to_string()),
        &model_valid_among(&["auto"]),
        &|_| Some("auto".to_string()),
    );

    assert!(matches!(
        &state.execution_mode,
        RunAgentsExecutionMode::Remote { environment_id, .. } if environment_id == "env-42"
    ));
    assert_eq!(state.model_id, "auto");
}

#[test]
fn execution_mode_change_prefers_valid_fallback_over_default_model() {
    let mut state = OrchestrationConfigState::from_run_agents_fields(
        Some("stale"),
        Some("oz"),
        &RunAgentsExecutionMode::Local,
    );

    state.apply_execution_mode_change_core(
        true,
        Some("fallback".to_string()),
        None,
        &model_valid_among(&["fallback", "first"]),
        &|_| Some("first".to_string()),
    );

    assert_eq!(state.model_id, "fallback");
}

#[test]
fn forcing_oz_before_local_preserves_codex_model_memory() {
    let state = OrchestrationConfigState::from_run_agents_fields(
        Some("gpt-5"),
        Some("codex"),
        &remote_mode(),
    );
    let mut edit_state = OrchestrationEditState {
        orchestration_config_state: state,
        saved_model_per_harness: HashMap::from([("oz".to_string(), "auto".to_string())]),
    };
    let model_is_valid = |id: &str, harness: &str, _is_local: bool| match harness {
        "oz" => id == "auto",
        "codex" => id == "gpt-5",
        _ => false,
    };
    let default_model_id = |harness: &str| match harness {
        "oz" => Some("auto".to_string()),
        "codex" => Some(String::new()),
        _ => None,
    };

    edit_state.apply_harness_change_core(
        "oz",
        Some("auto".to_string()),
        AuthSecretSelection::Unset,
        &model_is_valid,
        &default_model_id,
    );
    edit_state
        .orchestration_config_state
        .apply_execution_mode_change_core(
            false,
            Some("auto".to_string()),
            None,
            &model_is_valid,
            &default_model_id,
        );

    assert_eq!(edit_state.orchestration_config_state.harness_type, "oz");
    assert_eq!(edit_state.orchestration_config_state.model_id, "auto");
    assert!(matches!(
        edit_state.orchestration_config_state.execution_mode,
        RunAgentsExecutionMode::Local
    ));
    assert_eq!(
        edit_state.saved_model_per_harness.get("codex"),
        Some(&"gpt-5".to_string())
    );
}
#[test]
fn harness_change_saves_and_restores_per_harness_model_memory() {
    let state = OrchestrationConfigState::from_run_agents_fields(
        Some("sonnet"),
        Some("claude"),
        &remote_mode(),
    );
    let mut edit_state = OrchestrationEditState {
        orchestration_config_state: state,
        saved_model_per_harness: HashMap::from([("codex".to_string(), "gpt-5".to_string())]),
    };

    edit_state.apply_harness_change_core(
        "codex",
        None,
        AuthSecretSelection::Unset,
        &model_valid_among(&["gpt-5", "sonnet", ""]),
        &|_| Some(String::new()),
    );

    // Restored the saved codex model and remembered the claude model.
    assert_eq!(edit_state.orchestration_config_state.harness_type, "codex");
    assert_eq!(edit_state.orchestration_config_state.model_id, "gpt-5");
    assert_eq!(
        edit_state.saved_model_per_harness.get("claude"),
        Some(&"sonnet".to_string())
    );
}

#[test]
fn harness_change_applies_resolved_auth_selection() {
    let mut state =
        OrchestrationConfigState::from_run_agents_fields(Some("auto"), Some("oz"), &remote_mode());
    state.auth_secret_selection = AuthSecretSelection::Named("old-key".to_string());
    let mut edit_state = OrchestrationEditState::new(state);

    edit_state.apply_harness_change_core(
        "claude",
        None,
        AuthSecretSelection::Named("anthropic-key".to_string()),
        &model_valid_among(&["auto"]),
        &|_| Some("auto".to_string()),
    );

    assert_eq!(
        edit_state.orchestration_config_state.auth_secret_selection,
        AuthSecretSelection::Named("anthropic-key".to_string())
    );
}

#[test]
fn revalidate_drops_deleted_named_secret_and_reseeds_from_resolved() {
    let mut state = OrchestrationConfigState::from_run_agents_fields(
        Some("sonnet"),
        Some("claude"),
        &remote_mode(),
    );
    state.auth_secret_selection = AuthSecretSelection::Named("deleted-key".to_string());

    state.revalidate_after_catalog_change_core(
        Some(&["other-key".to_string()]),
        AuthSecretSelection::Named("other-key".to_string()),
        &model_valid_among(&["sonnet"]),
        &|_| Some("sonnet".to_string()),
    );

    assert_eq!(
        state.auth_secret_selection,
        AuthSecretSelection::Named("other-key".to_string())
    );
}

#[test]
fn revalidate_keeps_named_secret_still_present() {
    let mut state = OrchestrationConfigState::from_run_agents_fields(
        Some("sonnet"),
        Some("claude"),
        &remote_mode(),
    );
    state.auth_secret_selection = AuthSecretSelection::Named("my-key".to_string());

    state.revalidate_after_catalog_change_core(
        Some(&["my-key".to_string()]),
        AuthSecretSelection::Unset,
        &model_valid_among(&["sonnet"]),
        &|_| Some("sonnet".to_string()),
    );

    assert_eq!(
        state.auth_secret_selection,
        AuthSecretSelection::Named("my-key".to_string())
    );
}

#[test]
fn revalidate_leaves_explicit_inherit_alone() {
    let mut state = OrchestrationConfigState::from_run_agents_fields(
        Some("sonnet"),
        Some("claude"),
        &remote_mode(),
    );
    state.auth_secret_selection = AuthSecretSelection::Inherit;

    state.revalidate_after_catalog_change_core(
        Some(&[]),
        AuthSecretSelection::Named("persisted".to_string()),
        &model_valid_among(&["sonnet"]),
        &|_| Some("sonnet".to_string()),
    );

    assert_eq!(state.auth_secret_selection, AuthSecretSelection::Inherit);
}

#[test]
fn revalidate_resets_vanished_model_to_default() {
    let mut state = OrchestrationConfigState::from_run_agents_fields(
        Some("gone"),
        Some("claude"),
        &remote_mode(),
    );

    state.revalidate_after_catalog_change_core(
        None,
        AuthSecretSelection::Unset,
        &model_valid_among(&[""]),
        &|_| Some(String::new()),
    );

    assert_eq!(state.model_id, "");
}
