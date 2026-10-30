use ai::agent::action_result::{
    RunAgentsAgentOutcome, RunAgentsAgentOutcomeKind, RunAgentsLaunchedExecutionMode,
    RunAgentsResult,
};
use ai::agent::orchestration_config::{
    OrchestrationAgentConfig, OrchestrationConfig, OrchestrationConfigStatus,
    OrchestrationExecutionMode,
};

use super::*;
use crate::ai::agent::task::TaskId;
use crate::ai::agent::{AIAgentActionId, AIAgentActionResult};

#[test]
#[serial_test::serial]
fn orchestration_inputs_redact_child_names_models_and_errors() {
    let previous = secret_redaction::SECRETS_REGEX.lock().clone();
    let pattern = regex::Regex::new("synthetic-sensitive-value").unwrap();
    secret_redaction::set_user_and_enterprise_secret_regexes([&pattern], []);
    let sensitive = "synthetic-sensitive-value".to_string();
    let mut inputs = [
        AIAgentInput::OrchestrationConfigUpdate {
            plan_id: "plan".to_string(),
            config: OrchestrationConfig {
                model_id: sensitive.clone(),
                harness_type: "oz".to_string(),
                execution_mode: OrchestrationExecutionMode::Local,
                agent_configs: vec![OrchestrationAgentConfig {
                    name: sensitive.clone(),
                    model_id: sensitive.clone(),
                }],
            },
            status: OrchestrationConfigStatus::Approved,
        },
        AIAgentInput::ActionResult {
            context: Arc::from([]),
            result: AIAgentActionResult {
                id: AIAgentActionId::from("action".to_string()),
                task_id: TaskId::new("task".to_string()),
                result: AIAgentActionResultType::RunAgents(RunAgentsResult::Launched {
                    model_id: sensitive.clone(),
                    harness_type: "oz".to_string(),
                    execution_mode: RunAgentsLaunchedExecutionMode::Local,
                    agents: vec![RunAgentsAgentOutcome {
                        name: sensitive.clone(),
                        resolved_model_id: sensitive.clone(),
                        kind: RunAgentsAgentOutcomeKind::Failed {
                            error: sensitive.clone(),
                        },
                    }],
                }),
            },
        },
    ];
    redact_inputs(&mut inputs);
    *secret_redaction::SECRETS_REGEX.lock() = previous;
    assert!(!format!("{inputs:?}").contains(&sensitive));
    let AIAgentInput::OrchestrationConfigUpdate { config, .. } = &inputs[0] else {
        panic!("expected config update");
    };
    assert_eq!(config.harness_type, "oz");
    assert_eq!(config.agent_configs[0].name, "*************************");
}
