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
use futures::future::BoxFuture;
use futures::FutureExt;
use settings::Setting;
use warp_cli::agent::Harness;
use warp_core::execution_mode::AppExecutionMode;
use warpui::{Entity, EntityId, ModelContext, ModelHandle, SingletonEntity};

use super::start_agent::{StartAgentExecutor, StartAgentOutcome};
use super::{ActionExecution, AnyActionExecution, ExecuteActionInput, PreprocessActionInput};
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::{
    AIAgentAction, AIAgentActionId, AIAgentActionResultType, AIAgentActionType, AIAgentInput,
    StartAgentExecutionMode,
};
use crate::ai::auth_secret_types::auth_secret_types_for_harness;
use crate::ai::blocklist::inline_action::orchestration_controls::OrchestrationEditState;
use crate::ai::blocklist::{BlocklistAIHistoryModel, BlocklistAIPermissions};
use crate::ai::cloud_agent_settings::CloudAgentSettings;
use crate::ai::document::plan_publication::{
    prepare_plan_publications, wait_for_plan_publications,
};
use crate::ai::local_harness_setup::local_harness_product_disabled_message;

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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingRunAgents {
    Publishing,
    Spawning,
}
#[derive(Debug, Clone)]
struct ExistingLaunchedAgent {
    name: String,
    agent_id: String,
}

pub struct RunAgentsExecutor {
    pending: HashMap<AIAgentActionId, PendingRunAgents>,
    launched_agents: HashMap<AIConversationId, HashMap<String, ExistingLaunchedAgent>>,
    start_agent_executor: ModelHandle<StartAgentExecutor>,
    terminal_view_id: EntityId,
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
    pub fn new(
        start_agent_executor: ModelHandle<StartAgentExecutor>,
        terminal_view_id: EntityId,
    ) -> Self {
        Self {
            pending: HashMap::new(),
            launched_agents: HashMap::new(),
            start_agent_executor,
            terminal_view_id,
        }
    }

    pub fn is_pending(&self, action_id: &AIAgentActionId) -> bool {
        self.pending.contains_key(action_id)
    }

    /// Cancels a pending run so publication completion cannot fan out children.
    pub(super) fn cancel_execution(
        &mut self,
        action_id: &AIAgentActionId,
        ctx: &mut ModelContext<Self>,
    ) {
        if matches!(
            self.pending.get(action_id),
            Some(PendingRunAgents::Publishing)
        ) {
            self.pending.remove(action_id);
            ctx.emit(RunAgentsExecutorEvent::SpawningFinished {
                action_id: action_id.clone(),
            });
        }
    }

    fn record_launched_agents(
        &mut self,
        conversation_id: AIConversationId,
        agents: &[RunAgentsAgentOutcome],
    ) {
        for agent in agents {
            let RunAgentsAgentOutcomeKind::Launched { agent_id } = &agent.kind else {
                continue;
            };
            let Some(normalized_name) = normalize_agent_name(&agent.name) else {
                continue;
            };
            self.launched_agents
                .entry(conversation_id)
                .or_default()
                .insert(
                    normalized_name,
                    ExistingLaunchedAgent {
                        name: agent.name.clone(),
                        agent_id: agent_id.clone(),
                    },
                );
        }
    }

    fn duplicate_launched_agents_reason(
        &self,
        request: &RunAgentsRequest,
        parent_conversation_id: AIConversationId,
        ctx: &ModelContext<Self>,
    ) -> Option<String> {
        duplicate_launched_agents_reason(
            request,
            parent_conversation_id,
            &self.launched_agents,
            ctx,
        )
    }

    /// Publishes parent plans and dispatches children after a bounded best-effort wait.
    fn dispatch_prepared_run_agents(
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
        let pending_plan_publications = prepare_plan_publications(parent_conversation_id, ctx);

        let snapshot = RunAgentsSpawningSnapshot {
            agent_count: request.agent_run_configs.len(),
        };
        self.pending
            .insert(action_id.clone(), PendingRunAgents::Publishing);
        ctx.emit(RunAgentsExecutorEvent::SpawningStarted {
            action_id: action_id.clone(),
            snapshot,
        });

        let action_id_for_wait = action_id.clone();
        ctx.spawn(
            async move {
                // Wait briefly for each plan to become server-backed without blocking
                // launch on a failed or slow publication. Resolves immediately when
                // there is nothing to wait on.
                wait_for_plan_publications(pending_plan_publications).await;
                request
            },
            move |me, request, ctx| {
                if !me.is_pending(&action_id_for_wait) {
                    return;
                }
                me.dispatch_children_for_prepared_request(
                    action_id_for_wait.clone(),
                    request,
                    parent_conversation_id,
                    sender,
                    ctx,
                )
            },
        );

        receiver
    }

    fn dispatch_children_for_prepared_request(
        &mut self,
        action_id: AIAgentActionId,
        request: RunAgentsRequest,
        parent_conversation_id: AIConversationId,
        sender: async_channel::Sender<RunAgentsResult>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.pending
            .insert(action_id.clone(), PendingRunAgents::Spawning);
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
        let parent_conversation_id_for_result = parent_conversation_id;

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
                me.record_launched_agents(parent_conversation_id_for_result, &agents);
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
        if let Some(reason) = prepare_request_for_execution(
            &mut request,
            parent_conversation_id,
            self.terminal_view_id,
            &self.launched_agents,
            ctx,
        ) {
            return ActionExecution::Sync(AIAgentActionResultType::RunAgents(
                RunAgentsResult::Denied { reason },
            ));
        }

        let receiver =
            self.dispatch_prepared_run_agents(action_id, request, parent_conversation_id, ctx);

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
        input: ExecuteActionInput,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        let AIAgentActionType::RunAgents(request) = &input.action.action else {
            return false;
        };
        if AppExecutionMode::as_ref(ctx).is_autonomous() {
            return true;
        }
        let mut resolved_request = request.clone();
        resolve_request_from_approved_config(&mut resolved_request, input.conversation_id, ctx);
        populate_default_auth_secret_for_execution(&mut resolved_request, ctx);
        if self
            .duplicate_launched_agents_reason(&resolved_request, input.conversation_id, ctx)
            .is_some()
        {
            return true;
        }
        approved_orchestration_config_can_autoexecute(request, input.conversation_id, ctx)
            || BlocklistAIPermissions::as_ref(ctx)
                .get_run_agents_setting(ctx, Some(self.terminal_view_id))
                .is_always_allow()
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

enum ChildSlot {
    Failed(String),
    Pending(async_channel::Receiver<StartAgentOutcome>),
}

fn approved_orchestration_config_can_autoexecute(
    request: &RunAgentsRequest,
    parent_conversation_id: AIConversationId,
    ctx: &ModelContext<RunAgentsExecutor>,
) -> bool {
    let mut resolved_request = request.clone();
    resolve_request_from_approved_config(&mut resolved_request, parent_conversation_id, ctx)
        .is_some_and(|status| status.is_approved())
        && can_execute_with_auth_secret(&resolved_request, ctx)
}

fn resolve_request_from_approved_config(
    request: &mut RunAgentsRequest,
    parent_conversation_id: AIConversationId,
    ctx: &ModelContext<RunAgentsExecutor>,
) -> Option<ai::agent::orchestration_config::OrchestrationConfigStatus> {
    let conversation =
        BlocklistAIHistoryModel::as_ref(ctx).conversation(&parent_conversation_id)?;
    let (config, status) = conversation.orchestration_config_for_plan(&request.plan_id)?;
    if status.is_approved() {
        resolve_request_from_config(request, config);
    }
    Some(status)
}

/// Normalizes the request and returns a denial reason when launch is blocked.
///
/// Autonomous agents always run: their calls may still inherit approved plan
/// config fields and default auth secrets, but they bypass interactive policy
/// denials because they cannot present a confirmation card.
fn prepare_request_for_execution(
    request: &mut RunAgentsRequest,
    parent_conversation_id: AIConversationId,
    terminal_view_id: EntityId,
    launched_agents: &HashMap<AIConversationId, HashMap<String, ExistingLaunchedAgent>>,
    ctx: &ModelContext<RunAgentsExecutor>,
) -> Option<String> {
    let status = resolve_request_from_approved_config(request, parent_conversation_id, ctx);
    populate_default_auth_secret_for_execution(request, ctx);
    if let Some(reason) =
        duplicate_launched_agents_reason(request, parent_conversation_id, launched_agents, ctx)
    {
        return Some(reason);
    }

    if AppExecutionMode::as_ref(ctx).is_autonomous() {
        return None;
    }

    if status.is_some_and(|status| status.is_disapproved()) {
        return Some("Orchestration config was disapproved".to_string());
    }

    if BlocklistAIPermissions::as_ref(ctx)
        .get_run_agents_setting(ctx, Some(terminal_view_id))
        .is_never_allow()
    {
        return Some(
            "Running child agents is disabled by the active execution profile.".to_string(),
        );
    }

    if !can_execute_with_auth_secret(request, ctx) {
        return Some(
            "Cloud child agents using this harness require an API key before they can run."
                .to_string(),
        );
    }

    None
}

fn duplicate_launched_agents_reason(
    request: &RunAgentsRequest,
    parent_conversation_id: AIConversationId,
    launched_agents: &HashMap<AIConversationId, HashMap<String, ExistingLaunchedAgent>>,
    ctx: &ModelContext<RunAgentsExecutor>,
) -> Option<String> {
    let requested_agents = request
        .agent_run_configs
        .iter()
        .map(|cfg| normalize_agent_name(&cfg.name).map(|name| (name, cfg.name.clone())))
        .collect::<Option<Vec<_>>>()?;
    if requested_agents.is_empty() {
        return None;
    }

    let existing_agents =
        existing_launched_agents_for_conversation(parent_conversation_id, launched_agents, ctx);
    if existing_agents.is_empty() {
        return None;
    }

    let duplicates = requested_agents
        .iter()
        .map(|(normalized_name, _)| existing_agents.get(normalized_name))
        .collect::<Option<Vec<_>>>()?;
    let duplicate_list = duplicates
        .iter()
        .map(|agent| format!("{} ({})", agent.name, agent.agent_id))
        .collect::<Vec<_>>()
        .join(", ");
    let addresses = duplicates
        .iter()
        .map(|agent| agent.agent_id.as_str())
        .collect::<Vec<_>>()
        .join(", ");

    Some(format!(
        "Requested agent(s) have already been launched: {duplicate_list}. \
         Do not start duplicate child agents; send any follow-up with send_message_to_agent \
         using the existing agent id(s): {addresses}."
    ))
}

fn existing_launched_agents_for_conversation(
    parent_conversation_id: AIConversationId,
    launched_agents: &HashMap<AIConversationId, HashMap<String, ExistingLaunchedAgent>>,
    ctx: &ModelContext<RunAgentsExecutor>,
) -> HashMap<String, ExistingLaunchedAgent> {
    let mut existing_agents = launched_agents
        .get(&parent_conversation_id)
        .cloned()
        .unwrap_or_default();

    if let Some(conversation) =
        BlocklistAIHistoryModel::as_ref(ctx).conversation(&parent_conversation_id)
    {
        for exchange in conversation.all_exchanges() {
            for input in &exchange.input {
                let AIAgentInput::ActionResult { result, .. } = input else {
                    continue;
                };
                let AIAgentActionResultType::RunAgents(RunAgentsResult::Launched {
                    agents, ..
                }) = &result.result
                else {
                    continue;
                };
                for agent in agents {
                    let RunAgentsAgentOutcomeKind::Launched { agent_id } = &agent.kind else {
                        continue;
                    };
                    let Some(normalized_name) = normalize_agent_name(&agent.name) else {
                        continue;
                    };
                    existing_agents.entry(normalized_name).or_insert_with(|| {
                        ExistingLaunchedAgent {
                            name: agent.name.clone(),
                            agent_id: agent_id.clone(),
                        }
                    });
                }
            }
        }
    }

    existing_agents
}

fn normalize_agent_name(name: &str) -> Option<String> {
    let trimmed = name.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_ascii_lowercase())
}

fn requires_default_auth_secret_for_execution(request: &RunAgentsRequest) -> bool {
    if !request.execution_mode.is_remote() {
        return false;
    }
    let Some(harness) = Harness::parse_orchestration_harness(&request.harness_type) else {
        return false;
    };
    harness != Harness::Oz && !auth_secret_types_for_harness(harness).is_empty()
}

fn can_execute_with_auth_secret(
    request: &RunAgentsRequest,
    ctx: &ModelContext<RunAgentsExecutor>,
) -> bool {
    if !requires_default_auth_secret_for_execution(request) {
        return true;
    }
    if request
        .harness_auth_secret_name
        .as_deref()
        .is_some_and(|name| !name.trim().is_empty())
    {
        return true;
    }
    default_auth_secret_name_for_harness(&request.harness_type, ctx).is_some()
}

fn default_auth_secret_name_for_harness(
    harness_type: &str,
    ctx: &ModelContext<RunAgentsExecutor>,
) -> Option<String> {
    let harness = Harness::parse_orchestration_harness(harness_type)?;
    if harness == Harness::Oz {
        return None;
    }
    CloudAgentSettings::as_ref(ctx)
        .last_selected_auth_secret
        .value()
        .get(harness.config_name())
        .cloned()
        .filter(|name| !name.trim().is_empty())
}

fn populate_default_auth_secret_for_execution(
    request: &mut RunAgentsRequest,
    ctx: &ModelContext<RunAgentsExecutor>,
) {
    if !requires_default_auth_secret_for_execution(request)
        || request
            .harness_auth_secret_name
            .as_deref()
            .is_some_and(|name| !name.trim().is_empty())
    {
        return;
    }
    request.harness_auth_secret_name =
        default_auth_secret_name_for_harness(&request.harness_type, ctx);
}

/// Unconditionally overrides run-wide fields on a `RunAgentsRequest`
/// from the approved orchestration config, delegating to
/// `OrchestrationEditState::override_from_approved_config`.
fn resolve_request_from_config(request: &mut RunAgentsRequest, config: &OrchestrationConfig) {
    // The approved plan config is the source of truth for these run-wide fields,
    // so callers pass a mutable request and continue with the normalized value.
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
            if let Some(message) = local_harness_product_disabled_message(harness) {
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
