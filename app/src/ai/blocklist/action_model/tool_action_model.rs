use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use super::preprocess::PendingPreprocessedActions;
use super::{RunningActionPhase, RunningActions};
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::{AIAgentAction, AIAgentActionId, AIAgentActionResult};

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
    pub(crate) fn record_running_action(
        &mut self,
        conversation_id: AIConversationId,
        action_id: AIAgentActionId,
        phase: RunningActionPhase,
    ) {
        match self.running_actions.entry(conversation_id) {
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                entry.get_mut().add_action(action_id);
            }
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(RunningActions::new(phase, action_id));
            }
        }
    }

    /// Convenience wrapper used by the TUI, which always runs actions serially.
    pub(crate) fn record_serial_running_action(
        &mut self,
        conversation_id: AIConversationId,
        action_id: AIAgentActionId,
    ) {
        self.record_running_action(conversation_id, action_id, RunningActionPhase::Serial);
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

    /// Returns whether the conversation has any actions still running.
    pub(crate) fn has_running_actions(&self, conversation_id: AIConversationId) -> bool {
        self.running_actions
            .get(&conversation_id)
            .is_some_and(|running| !running.is_empty())
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
