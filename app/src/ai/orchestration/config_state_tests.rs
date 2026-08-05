use ai::agent::action::RunAgentsExecutionMode;
use ai::agent::orchestration_config::{OrchestrationConfig, OrchestrationExecutionMode};

use super::OrchestrationConfigState;

fn local_config(harness_type: &str, model_id: &str) -> OrchestrationConfig {
    OrchestrationConfig {
        model_id: model_id.to_string(),
        harness_type: harness_type.to_string(),
        execution_mode: OrchestrationExecutionMode::Local,
    }
}

#[test]
fn toggle_to_local_sanitizes_disabled_codex() {
    let mut state = OrchestrationConfigState::from_run_agents_fields(
        Some("gpt-5"),
        Some("codex"),
        &RunAgentsExecutionMode::Remote {
            environment_id: "env-1".to_string(),
            worker_host: "warp".to_string(),
            computer_use_enabled: None,
            runner_id: String::new(),
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
fn local_round_trip_preserves_remote_computer_use() {
    let mut state = OrchestrationConfigState::from_run_agents_fields(
        Some("auto"),
        Some("oz"),
        &RunAgentsExecutionMode::Remote {
            environment_id: "env-1".to_string(),
            worker_host: "warp".to_string(),
            computer_use_enabled: Some(true),
            runner_id: String::new(),
        },
    );

    state.toggle_execution_mode_to_remote(false);
    state.toggle_execution_mode_to_remote(true);

    assert!(matches!(
        state.execution_mode,
        RunAgentsExecutionMode::Remote {
            computer_use_enabled: Some(true),
            ..
        }
    ));
}

#[test]
fn toggle_to_local_preserves_claude() {
    let mut state = OrchestrationConfigState::from_run_agents_fields(
        Some("sonnet"),
        Some("claude"),
        &RunAgentsExecutionMode::Remote {
            environment_id: "env-1".to_string(),
            worker_host: "warp".to_string(),
            computer_use_enabled: None,
            runner_id: String::new(),
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
fn resolve_from_config_preserves_local_claude() {
    let mut state = OrchestrationConfigState::from_run_agents_fields(
        None,
        None,
        &RunAgentsExecutionMode::Local,
    );

    state.resolve_from_config(&local_config("claude", "sonnet"));
    assert_eq!(state.harness_type, "claude");
    assert_eq!(state.model_id, "sonnet");
    assert!(matches!(
        state.execution_mode,
        RunAgentsExecutionMode::Local
    ));
}

#[test]
fn runner_id_round_trips_through_config() {
    let config = OrchestrationConfig {
        model_id: "auto".to_string(),
        harness_type: "oz".to_string(),
        execution_mode: OrchestrationExecutionMode::Remote {
            environment_id: "env-1".to_string(),
            worker_host: "warp".to_string(),
            runner_id: "runner-7".to_string(),
        },
    };
    let state = OrchestrationConfigState::from_orchestration_config(&config);
    assert_eq!(state.to_orchestration_config(), config);
}

/// Regression for REMOTE-2444: the approved plan carries no computer-use
/// opinion, so a plan-driven launch must leave the flag unspecified instead
/// of seeding an explicit disable.
#[test]
fn from_orchestration_config_leaves_computer_use_unspecified() {
    let config = OrchestrationConfig {
        model_id: "auto".to_string(),
        harness_type: "oz".to_string(),
        execution_mode: OrchestrationExecutionMode::Remote {
            environment_id: "env-1".to_string(),
            worker_host: "warp".to_string(),
            runner_id: String::new(),
        },
    };
    let state = OrchestrationConfigState::from_orchestration_config(&config);
    assert!(matches!(
        state.execution_mode,
        RunAgentsExecutionMode::Remote {
            computer_use_enabled: None,
            ..
        }
    ));
}

/// An explicit per-call flag survives the approved-config override, which
/// only owns model / harness / execution mode.
#[test]
fn override_from_approved_config_preserves_explicit_computer_use() {
    let mut state = OrchestrationConfigState::from_run_agents_fields(
        Some("auto"),
        Some("oz"),
        &RunAgentsExecutionMode::Remote {
            environment_id: "env-1".to_string(),
            worker_host: "warp".to_string(),
            computer_use_enabled: Some(true),
            runner_id: String::new(),
        },
    );

    state.override_from_approved_config(&OrchestrationConfig {
        model_id: "auto".to_string(),
        harness_type: "oz".to_string(),
        execution_mode: OrchestrationExecutionMode::Remote {
            environment_id: "env-2".to_string(),
            worker_host: "warp".to_string(),
            runner_id: String::new(),
        },
    });

    assert!(matches!(
        state.execution_mode,
        RunAgentsExecutionMode::Remote {
            computer_use_enabled: Some(true),
            ..
        }
    ));
}

#[test]
fn resolve_from_config_sanitizes_disabled_local_codex() {
    let mut state = OrchestrationConfigState::from_run_agents_fields(
        None,
        None,
        &RunAgentsExecutionMode::Local,
    );

    state.resolve_from_config(&local_config("codex", "gpt-5"));

    assert_eq!(state.harness_type, "oz");
    assert_eq!(state.model_id, "");
    assert!(matches!(
        state.execution_mode,
        RunAgentsExecutionMode::Local
    ));
}
