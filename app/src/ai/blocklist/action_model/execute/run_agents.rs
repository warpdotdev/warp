use ai::agent::action::{RunAgentsAgentRunConfig, RunAgentsExecutionMode, RunAgentsRequest};
use ai::agent::action_result::RunAgentsResult;
use ai::skills::SkillReference;
use futures::future::BoxFuture;
use futures::FutureExt;
use warp_cli::agent::Harness;
use warpui::{Entity, EntityId, ModelContext, ModelHandle};

use super::start_agent::StartAgentExecutor;
use super::{ActionExecution, AnyActionExecution, ExecuteActionInput, PreprocessActionInput};
use crate::ai::agent::{
    AIAgentAction, AIAgentActionId, AIAgentActionResultType, AIAgentActionType,
    StartAgentExecutionMode,
};
use crate::ai::local_harness_setup::local_harness_product_disabled_message;

const RUN_AGENTS_DISABLED_REASON: &str =
    "Child agent orchestration is not available in this build.";

/// Snapshot of an in-flight dispatch, carried through
/// [`RunAgentsExecutorEvent::SpawningStarted`].
#[derive(Debug, Clone, Copy)]
pub struct RunAgentsSpawningSnapshot {
    pub agent_count: usize,
}

pub struct RunAgentsExecutor;

/// Lifecycle events for in-flight dispatches.
pub enum RunAgentsExecutorEvent {
    SpawningStarted {
        action_id: AIAgentActionId,
        snapshot: RunAgentsSpawningSnapshot,
    },
    SpawningFinished {
        action_id: AIAgentActionId,
    },
}

impl Entity for RunAgentsExecutor {
    type Event = RunAgentsExecutorEvent;
}

impl RunAgentsExecutor {
    pub fn new(
        _start_agent_executor: ModelHandle<StartAgentExecutor>,
        _terminal_view_id: EntityId,
    ) -> Self {
        Self
    }

    pub fn is_pending(&self, _action_id: &AIAgentActionId) -> bool {
        false
    }

    pub(super) fn cancel_execution(
        &mut self,
        _action_id: &AIAgentActionId,
        _ctx: &mut ModelContext<Self>,
    ) {
    }

    pub(super) fn execute(
        &mut self,
        input: ExecuteActionInput,
        _ctx: &mut ModelContext<Self>,
    ) -> impl Into<AnyActionExecution> {
        let AIAgentAction { action, .. } = input.action;
        let AIAgentActionType::RunAgents(request) = action else {
            return ActionExecution::<AIAgentActionResultType>::InvalidAction;
        };
        let _ = request;
        ActionExecution::Sync(AIAgentActionResultType::RunAgents(
            RunAgentsResult::Denied {
                reason: RUN_AGENTS_DISABLED_REASON.to_string(),
            },
        ))
    }

    pub(super) fn should_autoexecute(
        &self,
        input: ExecuteActionInput,
        _ctx: &mut ModelContext<Self>,
    ) -> bool {
        let AIAgentActionType::RunAgents(_) = &input.action.action else {
            return false;
        };
        true
    }

    pub(super) fn preprocess_action(
        &mut self,
        _action: PreprocessActionInput,
        _ctx: &mut ModelContext<Self>,
    ) -> BoxFuture<'static, ()> {
        futures::future::ready(()).boxed()
    }
}

#[cfg(test)]
#[path = "run_agents_tests.rs"]
mod tests;

/// Joins `base_prompt` and a per-agent prompt with `"\n\n"`,
/// falling back to whichever is non-empty.
pub fn compose_run_agents_child_prompt(base_prompt: &str, per_agent_prompt: &str) -> String {
    let base_trimmed = base_prompt.trim();
    let per_agent_trimmed = per_agent_prompt.trim();
    match (base_trimmed.is_empty(), per_agent_trimmed.is_empty()) {
        (false, false) => format!("{base_prompt}\n\n{per_agent_prompt}"),
        (false, true) => base_prompt.to_string(),
        (true, false) => per_agent_prompt.to_string(),
        (true, true) => String::new(),
    }
}

/// Translates run-wide config into a per-child
/// [`StartAgentExecutionMode`]. Returns `Err` for rejected
/// combinations (e.g. OpenCode+Remote).
///
/// `run_auth_secret_name` is the managed-secret name the orchestration UI
/// resolved for the run-wide harness; only Remote mode currently consumes
/// it (Local children inherit auth from the user's shell environment).
pub fn run_agents_to_start_agent_mode(
    run_execution_mode: &RunAgentsExecutionMode,
    run_harness_type: &str,
    run_model_id: &str,
    run_skills: &[SkillReference],
    run_auth_secret_name: Option<&str>,
    cfg: &RunAgentsAgentRunConfig,
) -> Result<StartAgentExecutionMode, String> {
    match run_execution_mode {
        RunAgentsExecutionMode::Local => {
            let trimmed = run_harness_type.trim();
            // Propagate run-wide model selection for local launches.
            let trimmed_model_id = run_model_id.trim();
            let model_id = (!trimmed_model_id.is_empty()).then(|| trimmed_model_id.to_string());
            if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("oz") {
                Ok(StartAgentExecutionMode::Local {
                    harness_type: None,
                    model_id,
                })
            } else {
                if let Some(harness) = Harness::parse_local_child_harness(trimmed) {
                    if let Some(message) = local_harness_product_disabled_message(harness) {
                        return Err(message.to_string());
                    }
                }
                Ok(StartAgentExecutionMode::Local {
                    harness_type: Some(trimmed.to_string()),
                    model_id,
                })
            }
        }
        RunAgentsExecutionMode::Remote {
            environment_id,
            worker_host,
        } => {
            // OpenCode is unsupported on Remote.
            if run_harness_type.eq_ignore_ascii_case("opencode") {
                return Err(
                    "Remote child agents do not support the opencode harness yet.".to_string(),
                );
            }
            Ok(StartAgentExecutionMode::Remote {
                environment_id: environment_id.clone(),
                skill_references: run_skills.to_vec(),
                model_id: run_model_id.to_string(),
                worker_host: worker_host.clone(),
                harness_type: run_harness_type.to_string(),
                title: cfg.title.clone(),
                auth_secret_name: run_auth_secret_name
                    .map(str::to_string)
                    .filter(|s| !s.trim().is_empty()),
            })
        }
    }
}
