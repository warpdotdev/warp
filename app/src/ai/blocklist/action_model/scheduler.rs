//! Shared agent-tool scheduling loop, parameterized over [`AgentToolScheduleHost`].
//!
//! [`AgentToolScheduler`] owns the preprocessing fan-out, pending queue, serial/parallel phase
//! admission, ordered result draining, and follow-up readiness logic. Both the GUI
//! [`BlocklistAIActionModel`] and the TUI [`TuiToolActionModel`] implement [`AgentToolScheduleHost`]
//! and delegate scheduling to the static methods on [`AgentToolScheduler`].

use std::collections::HashSet;
use std::sync::Arc;

use futures::future::BoxFuture;
use warpui::AppContext;

use super::execute::{RunningActionPhase, TryExecuteResult};
use super::preprocess::PreprocessId;
use super::tool_action_model::AgentToolActionModel;
use super::NotExecutedReason;
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::{
    AIAgentAction, AIAgentActionId, AIAgentActionResult, AIAgentActionResultType,
    AIAgentActionType, CancellationReason, RequestCommandOutputResult,
};

// ─── StartedAction ─────────────────────────────────────────────────────────────

/// Outcome of attempting to start one pending action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StartedAction {
    Sync,
    Async { phase: RunningActionPhase },
}

// ─── can_start_action_with_current_phase ───────────────────────────────────────

/// Returns whether another action may join the currently running phase.
///
/// Parallel phases only admit additional actions that classify into the same group and
/// can still be auto-executed. Serial phases always act as a barrier.
pub(super) fn can_start_action_with_current_phase(
    current_phase: RunningActionPhase,
    next_phase: RunningActionPhase,
    can_autoexecute: bool,
) -> bool {
    match current_phase {
        RunningActionPhase::Serial => false,
        RunningActionPhase::Parallel(group) => {
            next_phase == RunningActionPhase::Parallel(group) && can_autoexecute
        }
    }
}

// ─── AgentToolScheduleHost ─────────────────────────────────────────────────────

/// Implemented by surfaces (GUI, TUI) to plug surface-specific behavior into
/// the shared scheduling loop.
pub(crate) trait AgentToolScheduleHost: Sized {
    type Context<'a>;

    /// Returns the [`AppContext`] for the current call.
    fn app_context<'a, 'b>(ctx: &'a Self::Context<'b>) -> &'a AppContext;

    /// Mutable access to shared action-queue state.
    fn tools(&mut self) -> &mut AgentToolActionModel;

    /// Read-only access to shared action-queue state.
    fn tools_ref(&self) -> &AgentToolActionModel;

    /// Preprocessing step for a single action. Returns a future that resolves when done.
    fn preprocess(
        &mut self,
        action: &AIAgentAction,
        conversation_id: AIConversationId,
        ctx: &mut Self::Context<'_>,
    ) -> BoxFuture<'static, ()>;

    /// Attempts to execute the action. The host guarantees that
    /// [`AgentToolScheduler::finish_action`] is eventually called for async results.
    fn try_execute(
        &mut self,
        action: AIAgentAction,
        conversation_id: AIConversationId,
        is_user_initiated: bool,
        ctx: &mut Self::Context<'_>,
    ) -> TryExecuteResult;

    /// Returns whether the host would auto-execute this action without user confirmation.
    fn can_autoexecute(
        &mut self,
        action: &AIAgentAction,
        conversation_id: AIConversationId,
        ctx: &mut Self::Context<'_>,
    ) -> bool;

    /// Returns the execution phase for this action (Serial or Parallel(group)).
    fn action_phase(&self, action: &AIAgentAction, ctx: &AppContext) -> RunningActionPhase;

    /// Spawns `futures` and calls `then` when they all complete. Implemented by each
    /// host with its concrete context's `ctx.spawn`.
    fn spawn_after_preprocess(
        &mut self,
        futures: Vec<BoxFuture<'static, ()>>,
        ctx: &mut Self::Context<'_>,
        then: impl FnOnce(&mut Self, &mut Self::Context<'_>) + 'static,
    );

    // ── Side-effect hooks (defaults = no-op) ──────────────────────────────

    /// Returns false to suppress enqueueing this action (e.g. view-only guard).
    fn should_enqueue(
        &self,
        _conversation_id: AIConversationId,
        _action_id: &AIAgentActionId,
        _ctx: &AppContext,
    ) -> bool {
        true
    }

    /// Called when an action is added to the pending queue.
    fn on_action_enqueued(
        &mut self,
        _conversation_id: AIConversationId,
        _action_id: &AIAgentActionId,
        _ctx: &mut Self::Context<'_>,
    ) {
    }

    /// Called when an action transitions to started/running.
    fn on_action_started(
        &mut self,
        _conversation_id: AIConversationId,
        _is_wait_for_events: bool,
        _ctx: &mut Self::Context<'_>,
    ) {
    }

    /// Called when an action could not be started (needs confirmation, not ready, etc.).
    fn on_action_not_executed(
        &mut self,
        _action: &AIAgentAction,
        _reason: NotExecutedReason,
        _conversation_id: AIConversationId,
        _ctx: &mut Self::Context<'_>,
    ) {
    }

    /// Called after an action's result is recorded, before checking phase drain.
    fn on_action_finished(
        &mut self,
        _conversation_id: AIConversationId,
        _result: &Arc<AIAgentActionResult>,
        _cancellation_reason: Option<CancellationReason>,
        _ctx: &mut Self::Context<'_>,
    ) {
    }

    /// Called when the current running phase has fully drained and there are no more
    /// pending actions for this conversation.
    fn on_phase_drained(
        &mut self,
        _conversation_id: AIConversationId,
        _cancellation_reason: Option<CancellationReason>,
        _ctx: &mut Self::Context<'_>,
    ) {
    }
}

// ─── AgentToolScheduler ────────────────────────────────────────────────────────

/// Unit struct whose generic static methods own the agent-tool scheduling loop.
pub(crate) struct AgentToolScheduler;

impl AgentToolScheduler {
    /// Queues `actions` for the given conversation: records order, runs preprocessing,
    /// and after all preprocessing completes schedules the first batch of actions.
    pub(crate) fn queue_actions<H: AgentToolScheduleHost>(
        host: &mut H,
        actions: Vec<AIAgentAction>,
        conversation_id: AIConversationId,
        ctx: &mut H::Context<'_>,
    ) {
        host.tools().record_action_order(conversation_id, &actions);
        let mut preprocess_futures = Vec::with_capacity(actions.len());
        let mut action_ids = HashSet::with_capacity(actions.len());

        for action in actions.iter() {
            action_ids.insert(action.id.clone());
            preprocess_futures.push(host.preprocess(action, conversation_id, ctx));
        }

        let preprocess_id = host
            .tools()
            .pending_preprocessed_actions
            .entry(conversation_id)
            .or_default()
            .insert_preprocess_action_batch(action_ids);

        host.spawn_after_preprocess(preprocess_futures, ctx, move |host, ctx| {
            AgentToolScheduler::handle_preprocess_actions_results(
                host,
                conversation_id,
                preprocess_id,
                actions,
                ctx,
            );
        });
    }

    /// Records a completed action result, updates running state, and schedules follow-up work.
    pub(crate) fn finish_action<H: AgentToolScheduleHost>(
        host: &mut H,
        conversation_id: AIConversationId,
        action_result: Arc<AIAgentActionResult>,
        cancellation_reason: Option<CancellationReason>,
        ctx: &mut H::Context<'_>,
    ) {
        host.tools()
            .finish_running_action(conversation_id, &action_result.id);

        // If a command entered long-running mode, cancel all other pending RequestCommandOutput
        // actions. Only one command can be active at a time.
        if matches!(
            &action_result.result,
            AIAgentActionResultType::RequestCommandOutput(
                RequestCommandOutputResult::LongRunningCommandSnapshot { .. }
            )
        ) {
            for action in AgentToolScheduler::drain_pending_request_command_actions(
                host.tools(),
                conversation_id,
            ) {
                let result = Arc::new(AIAgentActionResult {
                    id: action.id,
                    task_id: action.task_id,
                    result: action.action.cancelled_result(),
                });
                AgentToolScheduler::finish_action(
                    host,
                    conversation_id,
                    result,
                    cancellation_reason,
                    ctx,
                );
            }
        }

        host.tools()
            .push_finished_result(conversation_id, action_result.clone());
        host.on_action_finished(conversation_id, &action_result, cancellation_reason, ctx);

        if host
            .tools_ref()
            .running_actions
            .get(&conversation_id)
            .is_some_and(|running| !running.is_empty())
        {
            // Wait until the entire phase drains before scheduling subsequent actions.
            return;
        }

        // Phase fully drained — sort results back into original tool-call order.
        let action_order = host.tools_ref().action_order.get(&conversation_id).cloned();
        if let Some(action_order) = action_order {
            if let Some(finished_results) = host
                .tools()
                .finished_action_results
                .get_mut(&conversation_id)
            {
                finished_results.sort_by_key(|result| {
                    action_order.get(&result.id).copied().unwrap_or(usize::MAX)
                });
            }
        }

        if host
            .tools_ref()
            .pending_actions
            .get(&conversation_id)
            .is_none_or(|actions| actions.is_empty())
        {
            host.on_phase_drained(conversation_id, cancellation_reason, ctx);
        } else {
            AgentToolScheduler::try_to_execute_available_actions(host, conversation_id, ctx);
        }
    }

    /// Advances the scheduling loop: starts as many pending actions as the current phase allows.
    pub(super) fn try_to_execute_available_actions<H: AgentToolScheduleHost>(
        host: &mut H,
        conversation_id: AIConversationId,
        ctx: &mut H::Context<'_>,
    ) {
        loop {
            let Some(front_action) = host
                .tools_ref()
                .pending_actions
                .get(&conversation_id)
                .and_then(|queue| queue.front())
                .cloned()
            else {
                return;
            };

            if let Some(current_phase) = host
                .tools_ref()
                .running_actions
                .get(&conversation_id)
                .map(|r| r.phase)
            {
                if !AgentToolScheduler::can_start_action_in_current_phase(
                    host,
                    &front_action,
                    conversation_id,
                    current_phase,
                    ctx,
                ) {
                    return;
                }
            }

            let Some(result) = AgentToolScheduler::start_pending_action_by_id(
                host,
                &front_action.id,
                conversation_id,
                false,
                ctx,
            ) else {
                return;
            };

            if matches!(
                result,
                StartedAction::Async {
                    phase: RunningActionPhase::Serial
                }
            ) {
                return;
            }
        }
    }

    /// Removes the pending action with `action_id` from the queue and starts it.
    ///
    /// Returns `None` if the action could not be started.
    pub(super) fn start_pending_action_by_id<H: AgentToolScheduleHost>(
        host: &mut H,
        action_id: &AIAgentActionId,
        conversation_id: AIConversationId,
        is_user_initiated: bool,
        ctx: &mut H::Context<'_>,
    ) -> Option<StartedAction> {
        if is_user_initiated
            && host
                .tools_ref()
                .running_actions
                .contains_key(&conversation_id)
        {
            // User-driven approvals execute one action at a time so interactive
            // confirmations do not overlap.
            return None;
        }

        let idx = host
            .tools_ref()
            .pending_actions
            .get(&conversation_id)
            .and_then(|queue| queue.iter().position(|action| &action.id == action_id))?;

        let action = host
            .tools()
            .pending_actions
            .get_mut(&conversation_id)?
            .remove(idx)?;

        let action_id_clone = action.id.clone();
        let phase = host.action_phase(&action, H::app_context(ctx));
        // WaitForEvents owns its own status transition; skip the default in-progress update.
        let is_wait_for_events = matches!(action.action, AIAgentActionType::WaitForEvents { .. });
        let execute_result = host.try_execute(action, conversation_id, is_user_initiated, ctx);

        match execute_result {
            TryExecuteResult::ExecutedAsync => {
                host.on_action_started(conversation_id, is_wait_for_events, ctx);
                host.tools()
                    .record_running_action(conversation_id, action_id_clone, phase);
                Some(StartedAction::Async { phase })
            }
            TryExecuteResult::ExecutedSync => {
                host.on_action_started(conversation_id, is_wait_for_events, ctx);
                Some(StartedAction::Sync)
            }
            TryExecuteResult::NotExecuted { reason, action } => {
                host.tools()
                    .pending_actions
                    .entry(conversation_id)
                    .or_default()
                    .insert(idx, (*action).clone());
                host.on_action_not_executed(action.as_ref(), reason, conversation_id, ctx);
                None
            }
        }
    }

    /// Called after preprocessing completes for a batch; enqueues actions in order and
    /// kicks off the scheduling loop.
    fn handle_preprocess_actions_results<H: AgentToolScheduleHost>(
        host: &mut H,
        conversation_id: AIConversationId,
        preprocess_id: PreprocessId,
        actions: Vec<AIAgentAction>,
        ctx: &mut H::Context<'_>,
    ) {
        let actions_to_enqueue = host
            .tools()
            .pending_preprocessed_actions
            .entry(conversation_id)
            .or_default()
            .handle_preprocess_actions_result(preprocess_id, actions);

        for action in actions_to_enqueue {
            let action_id = action.id.clone();
            // Skip actions that already have results (can happen in session sharing when the
            // sharer finishes and sends a result while preprocessing is still running).
            if host
                .tools_ref()
                .finished_action_results
                .get(&conversation_id)
                .is_some_and(|results| results.iter().any(|r| r.id == action_id))
            {
                continue;
            }

            if !host.should_enqueue(conversation_id, &action_id, H::app_context(ctx)) {
                continue;
            }

            host.tools()
                .pending_actions
                .entry(conversation_id)
                .or_default()
                .push_back(action);
            host.on_action_enqueued(conversation_id, &action_id, ctx);
        }
        AgentToolScheduler::try_to_execute_available_actions(host, conversation_id, ctx);
    }

    /// Returns whether `action` may start alongside the currently running `current_phase`.
    fn can_start_action_in_current_phase<H: AgentToolScheduleHost>(
        host: &mut H,
        action: &AIAgentAction,
        conversation_id: AIConversationId,
        current_phase: RunningActionPhase,
        ctx: &mut H::Context<'_>,
    ) -> bool {
        // Recompute phase on demand so executor-side capability checks use latest runtime state.
        let next_phase = host.action_phase(action, H::app_context(ctx));
        let can_autoexecute = host.can_autoexecute(action, conversation_id, ctx);
        can_start_action_with_current_phase(current_phase, next_phase, can_autoexecute)
    }

    /// Removes and returns all pending `RequestCommandOutput` actions for a conversation.
    fn drain_pending_request_command_actions(
        tools: &mut AgentToolActionModel,
        conversation_id: AIConversationId,
    ) -> Vec<AIAgentAction> {
        let Some(pending_actions) = tools.pending_actions.get_mut(&conversation_id) else {
            return Vec::new();
        };

        let mut to_drain = Vec::new();
        let mut i = 0;
        while i < pending_actions.len() {
            if matches!(
                pending_actions[i].action,
                AIAgentActionType::RequestCommandOutput { .. }
            ) {
                to_drain.push(
                    pending_actions
                        .remove(i)
                        .expect("index is valid because i < pending_actions.len()"),
                );
            } else {
                i += 1;
            }
        }
        to_drain
    }
}

#[cfg(test)]
#[path = "scheduler_tests.rs"]
mod tests;
