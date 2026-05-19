//! Async executor for `AIAgentActionType::RunAgents`.
//!
//! Fans out per-child via [`super::start_agent::StartAgentExecutor::dispatch`]
//! and aggregates the outcomes into a single `RunAgentsResult`.
use std::collections::HashMap;
use std::time::Duration;

use ai::agent::action::{RunAgentsAgentRunConfig, RunAgentsExecutionMode, RunAgentsRequest};
use ai::agent::action_result::{
    RunAgentsAgentOutcome, RunAgentsAgentOutcomeKind, RunAgentsLaunchedExecutionMode,
    RunAgentsResult,
};
use ai::agent::orchestration_config::OrchestrationConfig;
use ai::skills::SkillReference;

use crate::ai::blocklist::inline_action::orchestration_controls::OrchestrationEditState;
use futures::{future::BoxFuture, FutureExt};
use warp_cli::agent::Harness;
use warp_core::execution_mode::AppExecutionMode;
use warpui::{Entity, ModelContext, ModelHandle};

use super::start_agent::{StartAgentExecutor, StartAgentOutcome};
use super::{ActionExecution, AnyActionExecution, ExecuteActionInput, PreprocessActionInput};
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::{
    AIAgentAction, AIAgentActionId, AIAgentActionResultType, AIAgentActionType,
    StartAgentExecutionMode,
};
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::ai::local_child_harnesses::local_child_harness_disabled_message;
use warpui::SingletonEntity;

/// Per-child spawn timeout. If a child agent doesn't report back within
/// this window (e.g. binary not found, server error), the slot is failed
/// rather than hanging the "Spawning agents" UI indefinitely.
const SPAWN_TIMEOUT: Duration = Duration::from_secs(30);

/// Snapshot of an in-flight dispatch, carried through
/// [`RunAgentsExecutorEvent::SpawningStarted`].
#[derive(Debug, Clone, Copy)]
pub struct RunAgentsSpawningSnapshot {
    pub agent_count: usize,
}

/// In-flight tracking per `RunAgents` action (idempotency guard).
struct PendingRunAgents;

pub struct RunAgentsExecutor {
    pending: HashMap<AIAgentActionId, PendingRunAgents>,
    start_agent_executor: ModelHandle<StartAgentExecutor>,
}

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
    pub fn new(start_agent_executor: ModelHandle<StartAgentExecutor>) -> Self {
        Self {
            pending: HashMap::new(),
            start_agent_executor,
        }
    }

    pub fn is_pending(&self, action_id: &AIAgentActionId) -> bool {
        self.pending.contains_key(action_id)
    }

    /// Fans out per-child dispatches and returns a receiver for the
    /// aggregate `RunAgentsResult`. Validation failures short-circuit
    /// synchronously.
    pub fn dispatch_run_agents(
        &mut self,
        action_id: AIAgentActionId,
        request: RunAgentsRequest,
        parent_conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) -> async_channel::Receiver<RunAgentsResult> {
        let (sender, receiver) = async_channel::bounded(1);

        if self.pending.contains_key(&action_id) {
            log::warn!("RunAgentsExecutor: dispatch reentered for {action_id:?}; rejecting");
            let _ = sender.try_send(RunAgentsResult::Cancelled);
            return receiver;
        }

        if let Err(error) = validate_request(&request) {
            log::warn!("RunAgentsExecutor: validation failure: {error}");
            let _ = sender.try_send(RunAgentsResult::Failure { error });
            return receiver;
        }

        let snapshot = RunAgentsSpawningSnapshot {
            agent_count: request.agent_run_configs.len(),
        };
        self.pending.insert(action_id.clone(), PendingRunAgents);
        ctx.emit(RunAgentsExecutorEvent::SpawningStarted {
            action_id: action_id.clone(),
            snapshot,
        });

        let parent_run_id = BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(&parent_conversation_id)
            .and_then(|c| c.run_id());

        let RunAgentsRequest {
            execution_mode: run_execution_mode,
            harness_type,
            model_id,
            skills,
            agent_run_configs,
            base_prompt,
            harness_auth_secret_name,
            ..
        } = request;

        let mut slots: Vec<ChildSlot> = Vec::with_capacity(agent_run_configs.len());
        for cfg in &agent_run_configs {
            let prompt = compose_run_agents_child_prompt(&base_prompt, &cfg.prompt);
            let mode = match run_agents_to_start_agent_mode(
                &run_execution_mode,
                &harness_type,
                &model_id,
                &skills,
                harness_auth_secret_name.as_deref(),
                cfg,
            ) {
                Ok(mode) => mode,
                Err(err) => {
                    slots.push(ChildSlot::Failed(err));
                    continue;
                }
            };
            if matches!(run_execution_mode, RunAgentsExecutionMode::Remote { .. })
                && parent_run_id.is_none()
            {
                slots.push(ChildSlot::Failed(
                    "Remote child agents require the parent run_id to be available.".to_string(),
                ));
                continue;
            }
            let recv = self.start_agent_executor.update(ctx, |executor, exec_ctx| {
                executor.dispatch(
                    cfg.name.clone(),
                    prompt,
                    mode,
                    None, /* lifecycle_subscription */
                    parent_conversation_id,
                    parent_run_id.clone(),
                    exec_ctx,
                )
            });
            slots.push(ChildSlot::Pending(recv));
        }

        let agent_run_configs_for_result = agent_run_configs.clone();
        let action_id_for_aggr = action_id.clone();
        let run_model_id = model_id.clone();
        let run_harness_type = harness_type.clone();
        let run_execution_mode_for_aggr = run_execution_mode.clone();

        ctx.spawn(
            async move {
                let mut outcomes: Vec<RunAgentsAgentOutcomeKind> = Vec::with_capacity(slots.len());
                for slot in slots {
                    let kind = match slot {
                        ChildSlot::Failed(error) => RunAgentsAgentOutcomeKind::Failed { error },
                        ChildSlot::Pending(recv) => {
                            let timeout = warpui::r#async::Timer::after(SPAWN_TIMEOUT);
                            match futures::future::select(Box::pin(recv.recv()), Box::pin(timeout))
                                .await
                            {
                                futures::future::Either::Left((
                                    Ok(StartAgentOutcome::Started { agent_id }),
                                    _,
                                )) => RunAgentsAgentOutcomeKind::Launched { agent_id },
                                futures::future::Either::Left((
                                    Ok(StartAgentOutcome::Error(error)),
                                    _,
                                )) => RunAgentsAgentOutcomeKind::Failed { error },
                                futures::future::Either::Left((Err(_), _)) => {
                                    RunAgentsAgentOutcomeKind::Failed {
                                        error: "Cancelled before launch".to_string(),
                                    }
                                }
                                futures::future::Either::Right((_, _)) => {
                                    log::warn!(
                                        "Agent spawn timed out after {} seconds",
                                        SPAWN_TIMEOUT.as_secs()
                                    );
                                    RunAgentsAgentOutcomeKind::Failed {
                                        error: format!(
                                            "Agent failed to start within {} seconds. \
                                             The harness binary may not be installed.",
                                            SPAWN_TIMEOUT.as_secs()
                                        ),
                                    }
                                }
                            }
                        }
                    };
                    outcomes.push(kind);
                }
                outcomes
            },
            move |me, outcomes, ctx| {
                let agents: Vec<RunAgentsAgentOutcome> = agent_run_configs_for_result
                    .iter()
                    .zip(outcomes)
                    .map(|(cfg, kind)| RunAgentsAgentOutcome {
                        name: cfg.name.clone(),
                        kind,
                    })
                    .collect();
                let launched_mode = match &run_execution_mode_for_aggr {
                    RunAgentsExecutionMode::Local => RunAgentsLaunchedExecutionMode::Local,
                    RunAgentsExecutionMode::Remote {
                        environment_id,
                        worker_host,
                        computer_use_enabled,
                    } => RunAgentsLaunchedExecutionMode::Remote {
                        environment_id: environment_id.clone(),
                        worker_host: worker_host.clone(),
                        computer_use_enabled: *computer_use_enabled,
                    },
                };
                let result = RunAgentsResult::Launched {
                    model_id: run_model_id,
                    harness_type: run_harness_type,
                    execution_mode: launched_mode,
                    agents,
                };
                me.pending.remove(&action_id_for_aggr);
                ctx.emit(RunAgentsExecutorEvent::SpawningFinished {
                    action_id: action_id_for_aggr,
                });
                let _ = sender.try_send(result);
            },
        );

        receiver
    }

    pub(super) fn execute(
        &mut self,
        input: ExecuteActionInput,
        ctx: &mut ModelContext<Self>,
    ) -> impl Into<AnyActionExecution> {
        let AIAgentAction { action, id, .. } = input.action;
        let AIAgentActionType::RunAgents(request) = action else {
            return ActionExecution::InvalidAction;
        };
        let mut request = request.clone();
        let action_id = id.clone();
        let parent_conversation_id = input.conversation_id;

        // When auto-executing (autonomous/CLI-driver mode), the confirmation
        // card is bypassed. Replicate its policy/normalization here:
        // 1. Deny if the orchestration config is explicitly disapproved.
        // 2. Override model/harness/execution_mode from the approved config.
        if AppExecutionMode::as_ref(ctx).is_autonomous() {
            if let Some(conversation) =
                BlocklistAIHistoryModel::as_ref(ctx).conversation(&parent_conversation_id)
            {
                if let Some((config, status)) =
                    conversation.orchestration_config_for_plan(&request.plan_id)
                {
                    if status.is_disapproved() {
                        return ActionExecution::Sync(AIAgentActionResultType::RunAgents(
                            RunAgentsResult::Denied {
                                reason: "Orchestration config was disapproved".to_string(),
                            },
                        ));
                    }
                    if status.is_approved() {
                        resolve_request_from_config(&mut request, config);
                    }
                }
            }
        }

        let receiver = self.dispatch_run_agents(action_id, request, parent_conversation_id, ctx);

        ActionExecution::new_async(
            async move { receiver.recv().await },
            |result, _| match result {
                Ok(r) => AIAgentActionResultType::RunAgents(r),
                Err(_) => AIAgentActionResultType::RunAgents(RunAgentsResult::Cancelled),
            },
        )
    }

    pub(super) fn should_autoexecute(
        &self,
        _input: ExecuteActionInput,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        // Non-interactive (CLI driver) agents cannot present a
        // confirmation card, so they must auto-execute.
        AppExecutionMode::as_ref(ctx).is_autonomous()
    }

    pub(super) fn preprocess_action(
        &mut self,
        _action: PreprocessActionInput,
        _ctx: &mut ModelContext<Self>,
    ) -> BoxFuture<'static, ()> {
        futures::future::ready(()).boxed()
    }
}

enum ChildSlot {
    Failed(String),
    Pending(async_channel::Receiver<StartAgentOutcome>),
}

/// Unconditionally overrides run-wide fields on a `RunAgentsRequest`
/// from the approved orchestration config, delegating to
/// `OrchestrationEditState::override_from_approved_config`.
fn resolve_request_from_config(request: &mut RunAgentsRequest, config: &OrchestrationConfig) {
    let mut edit_state = OrchestrationEditState::from_run_agents_fields(
        &request.model_id,
        &request.harness_type,
        &request.execution_mode,
    );
    edit_state.override_from_approved_config(config);
    request.model_id = edit_state.model_id;
    request.harness_type = edit_state.harness_type;
    request.execution_mode = edit_state.execution_mode;
}

/// Defence-in-depth validation; mirrors the card view's
/// `accept_disabled_reason` check.
fn validate_request(request: &RunAgentsRequest) -> Result<(), String> {
    if request.agent_run_configs.is_empty() {
        return Err("orchestrate: empty agent_run_configs".to_string());
    }
    if matches!(request.execution_mode, RunAgentsExecutionMode::Local) {
        if let Some(harness) = Harness::parse_local_child_harness(&request.harness_type) {
            if let Some(message) = local_child_harness_disabled_message(harness) {
                return Err(message.to_string());
            }
        }
    }
    if matches!(
        request.execution_mode,
        RunAgentsExecutionMode::Remote { .. }
    ) && request.harness_type.eq_ignore_ascii_case("opencode")
    {
        return Err("Remote child agents do not support the opencode harness yet.".to_string());
    }
    Ok(())
}

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
                    if let Some(message) = local_child_harness_disabled_message(harness) {
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
            computer_use_enabled,
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
                computer_use_enabled: *computer_use_enabled,
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
