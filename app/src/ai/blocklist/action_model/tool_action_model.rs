use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use super::preprocess::PendingPreprocessedActions;
use super::{AIActionStatus, RunningActionPhase, RunningActions};
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::{
    AIAgentAction, AIAgentActionId, AIAgentActionResult, AIAgentActionResultType, AIAgentExchange,
    AIAgentInput, RequestCommandOutputResult,
};

/// Shared action queue/result state for Agent Mode tools.
pub(crate) struct AgentToolActionModel {
    pub(super) pending_preprocessed_actions: HashMap<AIConversationId, PendingPreprocessedActions>,
    pub(super) pending_actions: HashMap<AIConversationId, VecDeque<AIAgentAction>>,
    pub(super) running_actions: HashMap<AIConversationId, RunningActions>,
    pub(super) finished_action_results: HashMap<AIConversationId, Vec<Arc<AIAgentActionResult>>>,
    pub(super) action_order: HashMap<AIConversationId, HashMap<AIAgentActionId, usize>>,
    pub(super) past_action_results: HashMap<AIAgentActionId, Arc<AIAgentActionResult>>,
}

impl AgentToolActionModel {
    pub(crate) fn new() -> Self {
        Self {
            pending_preprocessed_actions: Default::default(),
            pending_actions: Default::default(),
            running_actions: Default::default(),
            finished_action_results: Default::default(),
            action_order: Default::default(),
            past_action_results: Default::default(),
        }
    }

    /// Records the dispatch order of a batch of actions so results can be sorted back
    /// into the original tool-call order when the batch drains.
    pub(crate) fn record_action_order(
        &mut self,
        conversation_id: AIConversationId,
        actions: &[AIAgentAction],
    ) {
        self.action_order.insert(
            conversation_id,
            actions
                .iter()
                .enumerate()
                .map(|(index, action)| (action.id.clone(), index))
                .collect(),
        );
    }

    /// Records an action as currently running in the given conversation.
    ///
    /// Asserts that any existing running phase for this conversation matches the new
    /// action's phase, since a phase must drain before actions from a different phase
    /// are admitted.
    pub(crate) fn record_running_action(
        &mut self,
        conversation_id: AIConversationId,
        action_id: AIAgentActionId,
        phase: RunningActionPhase,
    ) {
        match self.running_actions.entry(conversation_id) {
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                debug_assert_eq!(entry.get().phase, phase);
                entry.get_mut().add_action(action_id);
            }
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(RunningActions::new(phase, action_id));
            }
        }
    }

    /// Removes the given action from the running set; clears the conversation entry when empty.
    pub(crate) fn finish_running_action(
        &mut self,
        conversation_id: AIConversationId,
        action_id: &AIAgentActionId,
    ) {
        let should_remove = self
            .running_actions
            .get_mut(&conversation_id)
            .is_some_and(|running| {
                running.remove_action(action_id);
                running.is_empty()
            });
        if should_remove {
            self.running_actions.remove(&conversation_id);
        }
    }

    pub(crate) fn push_finished_result(
        &mut self,
        conversation_id: AIConversationId,
        result: Arc<AIAgentActionResult>,
    ) {
        self.finished_action_results
            .entry(conversation_id)
            .or_default()
            .push(result);
    }

    /// Returns the pending action with the given ID, if any.
    #[cfg_attr(not(feature = "tui"), allow(dead_code))]
    pub(crate) fn find_pending_action(
        &self,
        conversation_id: AIConversationId,
        action_id: &AIAgentActionId,
    ) -> Option<&AIAgentAction> {
        self.pending_actions
            .get(&conversation_id)
            .and_then(|q| q.iter().find(|a| &a.id == action_id))
    }

    /// Returns the next pending action for a conversation, or `None` if a phase is running
    /// (running phases block new actions until they drain).
    pub(crate) fn blocked_action_for_conversation(
        &self,
        conversation_id: &AIConversationId,
    ) -> Option<&AIAgentAction> {
        if self.running_actions.contains_key(conversation_id) {
            return None;
        }
        self.pending_actions
            .get(conversation_id)
            .and_then(|queue| queue.front())
    }

    /// Returns all pending actions across all conversations.
    pub(crate) fn get_pending_actions(&self) -> Vec<&AIAgentAction> {
        self.pending_actions
            .values()
            .flat_map(|queue| queue.iter())
            .collect()
    }

    /// Returns all pending actions for a specific conversation.
    pub(crate) fn get_pending_actions_for_conversation(
        &self,
        conversation_id: &AIConversationId,
    ) -> impl Iterator<Item = &AIAgentAction> {
        self.pending_actions
            .get(conversation_id)
            .into_iter()
            .flat_map(|queue| queue.iter())
    }

    /// Returns a pending action by its ID, searching across all conversations.
    pub(crate) fn get_pending_action_by_id(
        &self,
        action_id: &AIAgentActionId,
    ) -> Option<&AIAgentAction> {
        self.pending_actions
            .values()
            .flat_map(|queue| queue.iter())
            .find(|action| &action.id == action_id)
    }

    /// Returns whether a conversation has any pending or running actions.
    pub(crate) fn has_unfinished_actions_for_conversation(
        &self,
        conversation_id: AIConversationId,
    ) -> bool {
        let has_pending = self
            .pending_actions
            .get(&conversation_id)
            .is_some_and(|queue| !queue.is_empty());
        let has_running = self
            .running_actions
            .get(&conversation_id)
            .is_some_and(|running| !running.is_empty());
        has_pending || has_running
    }

    /// Returns finished action results received from the most recent AI output for a conversation.
    pub(crate) fn get_finished_action_results(
        &self,
        conversation_id: AIConversationId,
    ) -> Option<&Vec<Arc<AIAgentActionResult>>> {
        self.finished_action_results.get(&conversation_id)
    }

    /// Returns the result for a finished action, searching current finished results and past results.
    pub(crate) fn get_action_result(
        &self,
        id: &AIAgentActionId,
    ) -> Option<&Arc<AIAgentActionResult>> {
        self.finished_action_results
            .values()
            .flat_map(|results| results.iter())
            .find(|result| &result.id == id)
            .or_else(|| self.past_action_results.get(id))
    }

    /// Returns the status of an action by ID.
    ///
    /// `is_view_only` controls whether a front-of-queue action is reported as `Blocked`
    /// (interactive surfaces) or `Queued` (view-only surfaces that never block on user acceptance).
    pub(crate) fn get_action_status(
        &self,
        id: &AIAgentActionId,
        is_view_only: bool,
    ) -> Option<AIActionStatus> {
        for (conversation_id, pending_actions_for_conversation) in &self.pending_actions {
            for (index, action) in pending_actions_for_conversation.iter().enumerate() {
                if &action.id != id {
                    continue;
                }

                if index == 0
                    && !is_view_only
                    && !self.running_actions.contains_key(conversation_id)
                {
                    return Some(AIActionStatus::Blocked);
                }

                return Some(AIActionStatus::Queued);
            }
        }

        self.running_actions
            .values()
            .find(|running| running.contains(id))
            .map(|_| AIActionStatus::RunningAsync)
            .or_else(|| {
                self.get_action_result(id)
                    .map(|result| AIActionStatus::Finished(result.clone()))
            })
            .or_else(|| {
                self.pending_preprocessed_actions
                    .values()
                    .any(|preprocessing| preprocessing.contains(id))
                    .then_some(AIActionStatus::Preprocessing)
            })
    }

    /// Bulk restores action results from a list of exchanges (used when loading conversations from tasks).
    ///
    /// Long-running command snapshots are downgraded to `CancelledBeforeExecution` since the command
    /// was incomplete when the app was closed.
    pub(crate) fn restore_action_results_from_exchanges(
        &mut self,
        exchanges: Vec<&AIAgentExchange>,
    ) {
        for exchange in exchanges.iter() {
            for input in &exchange.input {
                if let AIAgentInput::ActionResult { result, .. } = input {
                    let result_id = result.id.clone();
                    let mut result_to_insert = result.clone();
                    if let AIAgentActionResultType::RequestCommandOutput(
                        RequestCommandOutputResult::LongRunningCommandSnapshot { .. },
                    ) = &result.result
                    {
                        result_to_insert.result = AIAgentActionResultType::RequestCommandOutput(
                            RequestCommandOutputResult::CancelledBeforeExecution,
                        );
                    }
                    self.past_action_results
                        .insert(result_id, Arc::new(result_to_insert));
                }
            }
        }
    }

    /// Returns the number of currently running actions (test helper).
    #[cfg(any(test, feature = "integration_tests"))]
    pub(crate) fn running_action_count(&self, conversation_id: AIConversationId) -> usize {
        self.running_actions
            .get(&conversation_id)
            .map(|r| r.action_ids.len())
            .unwrap_or(0)
    }

    /// Returns the number of pending (not-yet-started) actions (test helper).
    #[cfg(any(test, feature = "integration_tests"))]
    pub(crate) fn pending_action_count(&self, conversation_id: AIConversationId) -> usize {
        self.pending_actions
            .get(&conversation_id)
            .map(|q| q.len())
            .unwrap_or(0)
    }

    pub(crate) fn drain_finished_results(
        &mut self,
        conversation_id: AIConversationId,
    ) -> Vec<AIAgentActionResult> {
        let action_order = self.action_order.remove(&conversation_id);
        let mut finished_results = self
            .finished_action_results
            .remove(&conversation_id)
            .unwrap_or_default();
        if let Some(action_order) = action_order {
            finished_results
                .sort_by_key(|result| action_order.get(&result.id).copied().unwrap_or(usize::MAX));
        }
        for result in &finished_results {
            self.past_action_results
                .insert(result.id.clone(), result.clone());
        }
        finished_results
            .into_iter()
            .map(|result| (*result).clone())
            .collect()
    }
}
