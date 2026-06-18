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
