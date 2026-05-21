use ai::agent::action::{RunAgentsAgentRunConfig, RunAgentsExecutionMode, RunAgentsRequest};
use ai::agent::action_result::{
    RunAgentsAgentOutcome, RunAgentsAgentOutcomeKind, RunAgentsLaunchedExecutionMode,
    RunAgentsResult,
};
use ai::skills::SkillReference;
use std::path::PathBuf;

use super::RunAgentsEditState;
use crate::ai::blocklist::inline_action::orchestration_controls::OrchestrationEditState;

fn make_request(harness: &str, mode: RunAgentsExecutionMode) -> RunAgentsRequest {
    make_request_with_skills(harness, mode, Vec::new())
}

fn make_request_with_skills(
    harness: &str,
    mode: RunAgentsExecutionMode,
    skills: Vec<SkillReference>,
) -> RunAgentsRequest {
    RunAgentsRequest {
        summary: "summary".to_string(),
        base_prompt: "base".to_string(),
        skills,
        model_id: "auto".to_string(),
        harness_type: harness.to_string(),
        execution_mode: mode,
        agent_run_configs: vec![RunAgentsAgentRunConfig {
            name: "child".to_string(),
            prompt: "do work".to_string(),
            title: "Child agent".to_string(),
        }],
        plan_id: String::new(),
        harness_auth_secret_name: None,
    }
}

fn make_edit_state_with_orch_fields(
    harness: &str,
    mode: RunAgentsExecutionMode,
) -> RunAgentsEditState {
    let request = make_request(harness, mode);
    RunAgentsEditState {
        orch: OrchestrationEditState::from_run_agents_fields(
            &request.model_id,
            &request.harness_type,
            &request.execution_mode,
        ),
        agent_run_configs: request.agent_run_configs,
        base_prompt: request.base_prompt,
        summary: request.summary,
        skills: request.skills,
        plan_id: request.plan_id,
    }
}

#[test]
fn local_to_cloud_initializes_remote_with_empty_environment() {
    let mut state =
        RunAgentsEditState::from_request(&make_request("oz", RunAgentsExecutionMode::Local));
    assert!(matches!(
        state.orch.execution_mode,
        RunAgentsExecutionMode::Local
    ));

    state.orch.toggle_execution_mode_to_remote(true);
    let RunAgentsExecutionMode::Remote {
        environment_id,
        worker_host,
        computer_use_enabled,
    } = state.orch.execution_mode
    else {
        panic!("expected Remote after toggle");
    };
    assert_eq!(environment_id, "");
    assert_eq!(worker_host, "warp");
    assert!(!computer_use_enabled);
}

#[test]
fn cloud_to_local_drops_environment() {
    let mut state = RunAgentsEditState::from_request(&make_request(
        "oz",
        RunAgentsExecutionMode::Remote {
            environment_id: "env-1".to_string(),
            worker_host: "warp".to_string(),
            computer_use_enabled: false,
        },
    ));
    state.orch.toggle_execution_mode_to_remote(false);
    assert!(matches!(
        state.orch.execution_mode,
        RunAgentsExecutionMode::Local
    ));
}

#[test]
fn local_to_cloud_resets_opencode_to_oz() {
    let mut state =
        RunAgentsEditState::from_request(&make_request("opencode", RunAgentsExecutionMode::Local));
    state.orch.toggle_execution_mode_to_remote(true);
    assert_eq!(state.orch.harness_type, "oz");
}

#[test]
fn cloud_without_env_no_longer_disables_accept() {
    let state = RunAgentsEditState::from_request(&make_request(
        "oz",
        RunAgentsExecutionMode::Remote {
            environment_id: String::new(),
            worker_host: "warp".to_string(),
            computer_use_enabled: false,
        },
    ));
    assert!(
        state.orch.accept_disabled_reason().is_none(),
        "Cloud without env should NOT disable Accept (soft recommendation only)"
    );
}

#[test]
fn cloud_with_opencode_disables_accept() {
    // Bypass the toggle helper to test the validation gate directly.
    let state = RunAgentsEditState::from_request(&make_request(
        "opencode",
        RunAgentsExecutionMode::Remote {
            environment_id: "env-1".to_string(),
            worker_host: "warp".to_string(),
            computer_use_enabled: false,
        },
    ));
    let reason = state.orch.accept_disabled_reason();
    assert!(reason.is_some(), "Cloud + OpenCode should disable Accept");
    assert!(reason.unwrap().contains("OpenCode"));
}

#[test]
fn local_with_any_harness_does_not_disable_accept() {
    for harness in ["oz", "gemini", "opencode"] {
        let state =
            RunAgentsEditState::from_request(&make_request(harness, RunAgentsExecutionMode::Local));
        assert!(
            state.orch.accept_disabled_reason().is_none(),
            "Local + {harness} should allow Accept"
        );
    }
}

#[test]
fn local_with_disabled_claude_or_codex_disables_accept() {
    for (harness, expected) in [
        (
            "claude",
            "Local Claude Code child agents are temporarily disabled.",
        ),
        (
            "codex",
            "Local Codex child agents are temporarily disabled.",
        ),
    ] {
        let state = make_edit_state_with_orch_fields(harness, RunAgentsExecutionMode::Local);
        assert_eq!(state.orch.accept_disabled_reason(), Some(expected));
    }
}

#[test]
fn from_request_sanitizes_disabled_local_harness_to_oz() {
    let state =
        RunAgentsEditState::from_request(&make_request("codex", RunAgentsExecutionMode::Local));

    assert_eq!(state.orch.harness_type, "oz");
    assert_eq!(state.orch.model_id, "");
    assert!(state.orch.accept_disabled_reason().is_none());
}

#[test]
fn cloud_with_env_and_non_opencode_harness_allows_accept() {
    for harness in ["oz", "claude", "gemini"] {
        let state = RunAgentsEditState::from_request(&make_request(
            harness,
            RunAgentsExecutionMode::Remote {
                environment_id: "env-1".to_string(),
                worker_host: "warp".to_string(),
                computer_use_enabled: false,
            },
        ));
        assert!(
            state.orch.accept_disabled_reason().is_none(),
            "Cloud + env + {harness} should allow Accept"
        );
    }
}

#[test]
fn set_environment_id_no_op_in_local_mode() {
    let mut state =
        RunAgentsEditState::from_request(&make_request("oz", RunAgentsExecutionMode::Local));
    state.orch.set_environment_id("env-1".to_string());
    assert!(matches!(
        state.orch.execution_mode,
        RunAgentsExecutionMode::Local
    ));
}

#[test]
fn set_environment_id_updates_remote() {
    let mut state = RunAgentsEditState::from_request(&make_request(
        "oz",
        RunAgentsExecutionMode::Remote {
            environment_id: "old".to_string(),
            worker_host: "warp".to_string(),
            computer_use_enabled: false,
        },
    ));
    state.orch.set_environment_id("new-env".to_string());
    let RunAgentsExecutionMode::Remote { environment_id, .. } = state.orch.execution_mode else {
        panic!("expected Remote");
    };
    assert_eq!(environment_id, "new-env");
}

#[test]
fn to_request_round_trips_request_fields() {
    let mut req = make_request_with_skills(
        "claude",
        RunAgentsExecutionMode::Remote {
            environment_id: "env-2".to_string(),
            worker_host: "warp".to_string(),
            computer_use_enabled: true,
        },
        vec![
            SkillReference::BundledSkillId("writing-pr-descriptions".to_string()),
            SkillReference::Path(PathBuf::from("/tmp/skill/SKILL.md")),
        ],
    );
    req.plan_id = "plan-1".to_string();
    let state = RunAgentsEditState::from_request(&req);
    let round_tripped = state.to_request();
    assert_eq!(round_tripped.summary, req.summary);
    assert_eq!(round_tripped.base_prompt, req.base_prompt);
    assert_eq!(round_tripped.model_id, req.model_id);
    assert_eq!(round_tripped.harness_type, req.harness_type);
    assert_eq!(round_tripped.execution_mode, req.execution_mode);
    assert_eq!(round_tripped.agent_run_configs, req.agent_run_configs);
    assert_eq!(round_tripped.skills, req.skills);
    assert_eq!(round_tripped.plan_id, req.plan_id);
}

mod format_terminal_state_tests {
    use super::super::{format_terminal_state, StatusKind};
    use super::*;

    fn launched(name: &str, agent_id: &str) -> RunAgentsAgentOutcome {
        RunAgentsAgentOutcome {
            name: name.to_string(),
            kind: RunAgentsAgentOutcomeKind::Launched {
                agent_id: agent_id.to_string(),
            },
        }
    }

    fn failed(name: &str, error: &str) -> RunAgentsAgentOutcome {
        RunAgentsAgentOutcome {
            name: name.to_string(),
            kind: RunAgentsAgentOutcomeKind::Failed {
                error: error.to_string(),
            },
        }
    }

    fn launched_result(agents: Vec<RunAgentsAgentOutcome>) -> RunAgentsResult {
        RunAgentsResult::Launched {
            model_id: "auto".to_string(),
            harness_type: "oz".to_string(),
            execution_mode: RunAgentsLaunchedExecutionMode::Local,
            agents,
        }
    }

    #[test]
    fn launched_singular_uses_singular_label() {
        let result = launched_result(vec![launched("child", "a-1")]);
        let (label, kind) = format_terminal_state(&result);
        assert_eq!(label, "Spawned 1 agent");
        assert!(matches!(kind, StatusKind::Success));
    }

    #[test]
    fn launched_plural_uses_plural_label() {
        let result = launched_result(vec![
            launched("a", "a-1"),
            launched("b", "a-2"),
            launched("c", "a-3"),
        ]);
        let (label, kind) = format_terminal_state(&result);
        assert_eq!(label, "Spawned 3 agents");
        assert!(matches!(kind, StatusKind::Success));
    }

    #[test]
    fn launched_partial_uses_x_of_y_label_and_mixed_status() {
        let result = launched_result(vec![
            launched("a", "a-1"),
            failed("b", "boom"),
            launched("c", "a-3"),
        ]);
        let (label, kind) = format_terminal_state(&result);
        assert_eq!(label, "Spawned 2 of 3 agents");
        assert!(matches!(kind, StatusKind::Mixed));
    }

    #[test]
    fn failure_with_error_includes_error_text() {
        let (label, kind) = format_terminal_state(&RunAgentsResult::Failure {
            error: "server rejected request".to_string(),
        });
        assert_eq!(
            label,
            "Failed to start orchestration: server rejected request"
        );
        assert!(matches!(kind, StatusKind::Failure));
    }

    #[test]
    fn failure_with_empty_error_uses_short_label() {
        let (label, kind) = format_terminal_state(&RunAgentsResult::Failure {
            error: String::new(),
        });
        assert_eq!(label, "Failed to start orchestration");
        assert!(matches!(kind, StatusKind::Failure));
    }

    #[test]
    fn denied_with_reason_appends_reason() {
        let (label, kind) = format_terminal_state(&RunAgentsResult::Denied {
            reason: "disapproved".to_string(),
        });
        assert!(label.contains("disapproved"));
        assert!(matches!(kind, StatusKind::Cancelled));
    }

    #[test]
    fn denied_without_reason_uses_short_label() {
        let (label, kind) = format_terminal_state(&RunAgentsResult::Denied {
            reason: String::new(),
        });
        assert!(!label.contains("()"));
        assert!(matches!(kind, StatusKind::Cancelled));
    }

    #[test]
    fn cancelled_uses_cancelled_status() {
        let (label, kind) = format_terminal_state(&RunAgentsResult::Cancelled);
        assert_eq!(label, "Spawn agents cancelled");
        assert!(matches!(kind, StatusKind::Cancelled));
    }
}

mod override_from_approved_config_tests {
    use super::super::RunAgentsEditState;
    use super::*;
    use ai::agent::orchestration_config::{OrchestrationConfig, OrchestrationExecutionMode};

    fn local_config(model: &str, harness: &str) -> OrchestrationConfig {
        OrchestrationConfig {
            model_id: model.to_string(),
            harness_type: harness.to_string(),
            execution_mode: OrchestrationExecutionMode::Local,
        }
    }

    fn remote_config(model: &str, harness: &str, env: &str) -> OrchestrationConfig {
        OrchestrationConfig {
            model_id: model.to_string(),
            harness_type: harness.to_string(),
            execution_mode: OrchestrationExecutionMode::Remote {
                environment_id: env.to_string(),
                worker_host: "warp".to_string(),
            },
        }
    }

    #[test]
    fn overrides_model_and_harness_unconditionally() {
        let mut state =
            RunAgentsEditState::from_request(&make_request("oz", RunAgentsExecutionMode::Local));
        assert_eq!(state.orch.model_id, "auto");
        assert_eq!(state.orch.harness_type, "oz");

        state
            .orch
            .override_from_approved_config(&local_config("claude-4-opus", "claude"));
        assert_eq!(state.orch.model_id, "claude-4-opus");
        assert_eq!(state.orch.harness_type, "claude");
    }

    #[test]
    fn overrides_even_when_request_has_values() {
        let mut state = RunAgentsEditState::from_request(&make_request(
            "claude",
            RunAgentsExecutionMode::Local,
        ));
        state
            .orch
            .override_from_approved_config(&local_config("gpt-5", "codex"));
        assert_eq!(state.orch.model_id, "gpt-5");
        assert_eq!(state.orch.harness_type, "codex");
    }

    #[test]
    fn overrides_local_to_remote() {
        let mut state =
            RunAgentsEditState::from_request(&make_request("oz", RunAgentsExecutionMode::Local));
        state
            .orch
            .override_from_approved_config(&remote_config("auto", "oz", "env-1"));
        let RunAgentsExecutionMode::Remote {
            environment_id,
            worker_host,
            ..
        } = &state.orch.execution_mode
        else {
            panic!("expected Remote after override");
        };
        assert_eq!(environment_id, "env-1");
        assert_eq!(worker_host, "warp");
    }

    #[test]
    fn overrides_remote_to_local() {
        let mut state = RunAgentsEditState::from_request(&make_request(
            "oz",
            RunAgentsExecutionMode::Remote {
                environment_id: "env-1".to_string(),
                worker_host: "warp".to_string(),
                computer_use_enabled: true,
            },
        ));
        state
            .orch
            .override_from_approved_config(&local_config("auto", "oz"));
        assert!(
            matches!(state.orch.execution_mode, RunAgentsExecutionMode::Local),
            "should be Local after override"
        );
    }

    #[test]
    fn preserves_computer_use_when_both_remote() {
        let mut state = RunAgentsEditState::from_request(&make_request(
            "oz",
            RunAgentsExecutionMode::Remote {
                environment_id: "old-env".to_string(),
                worker_host: "warp".to_string(),
                computer_use_enabled: true,
            },
        ));
        state
            .orch
            .override_from_approved_config(&remote_config("auto", "oz", "new-env"));
        let RunAgentsExecutionMode::Remote {
            environment_id,
            computer_use_enabled,
            ..
        } = &state.orch.execution_mode
        else {
            panic!("expected Remote");
        };
        assert_eq!(environment_id, "new-env", "env should come from config");
        assert!(
            *computer_use_enabled,
            "computer_use_enabled should be preserved from original request"
        );
    }

    #[test]
    fn does_not_carry_computer_use_from_local_to_remote() {
        let mut state =
            RunAgentsEditState::from_request(&make_request("oz", RunAgentsExecutionMode::Local));
        state
            .orch
            .override_from_approved_config(&remote_config("auto", "oz", "env-1"));
        let RunAgentsExecutionMode::Remote {
            computer_use_enabled,
            ..
        } = &state.orch.execution_mode
        else {
            panic!("expected Remote");
        };
        assert!(
            !*computer_use_enabled,
            "computer_use_enabled should default to false when original was Local"
        );
    }

    #[test]
    fn approved_local_disabled_harness_reports_disabled_reason_after_override() {
        let mut state =
            RunAgentsEditState::from_request(&make_request("oz", RunAgentsExecutionMode::Local));
        state
            .orch
            .override_from_approved_config(&local_config("auto", "claude"));
        assert_eq!(
            state.orch.accept_disabled_reason(),
            Some("Local Claude Code child agents are temporarily disabled.")
        );
    }
}

mod compute_is_denied_tests {
    use super::super::compute_is_denied;
    use ai::agent::orchestration_config::{
        OrchestrationConfig, OrchestrationConfigStatus, OrchestrationExecutionMode,
    };

    fn some_config(
        status: OrchestrationConfigStatus,
    ) -> Option<(OrchestrationConfig, OrchestrationConfigStatus)> {
        Some((
            OrchestrationConfig {
                model_id: "auto".to_string(),
                harness_type: "oz".to_string(),
                execution_mode: OrchestrationExecutionMode::Local,
            },
            status,
        ))
    }

    #[test]
    fn false_when_no_denied_result_and_no_config() {
        assert!(!compute_is_denied(false, &None));
    }

    #[test]
    fn true_when_has_denied_result_from_history() {
        assert!(compute_is_denied(true, &None));
    }

    #[test]
    fn true_when_config_is_disapproved() {
        let config = some_config(OrchestrationConfigStatus::Disapproved);
        assert!(compute_is_denied(false, &config));
    }

    #[test]
    fn true_when_both_denied_and_disapproved() {
        let config = some_config(OrchestrationConfigStatus::Disapproved);
        assert!(compute_is_denied(true, &config));
    }

    #[test]
    fn false_when_config_is_approved() {
        let config = some_config(OrchestrationConfigStatus::Approved);
        assert!(!compute_is_denied(false, &config));
    }

    #[test]
    fn false_when_config_status_is_none() {
        let config = some_config(OrchestrationConfigStatus::None);
        assert!(!compute_is_denied(false, &config));
    }

    #[test]
    fn denied_result_overrides_approved_config() {
        let config = some_config(OrchestrationConfigStatus::Approved);
        assert!(
            compute_is_denied(true, &config),
            "History denied result should take precedence over approved config"
        );
    }
}

#[test]
fn local_to_cloud_idempotent_when_already_remote() {
    let mut state = RunAgentsEditState::from_request(&make_request(
        "oz",
        RunAgentsExecutionMode::Remote {
            environment_id: "env-1".to_string(),
            worker_host: "warp".to_string(),
            computer_use_enabled: true,
        },
    ));
    state.orch.toggle_execution_mode_to_remote(true);
    let RunAgentsExecutionMode::Remote {
        environment_id,
        computer_use_enabled,
        ..
    } = state.orch.execution_mode
    else {
        panic!("expected Remote");
    };
    assert_eq!(
        environment_id, "env-1",
        "toggle to Remote when already Remote should not clobber env"
    );
    assert!(
        computer_use_enabled,
        "toggle to Remote when already Remote should not clobber computer_use"
    );
}
