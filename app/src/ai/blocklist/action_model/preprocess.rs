use std::collections::{HashSet, VecDeque};

use uuid::Uuid;

use crate::ai::agent::{AIAgentAction, AIAgentActionId};

/// A unique ID for a batch of preprocessed actions.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct PreprocessId(String);

impl PreprocessId {
    fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }
}

/// A list of pending preprocessed actions.
/// Each action goes through a preprocessing step where executors
/// can asynchronously do arbitrary work and store any state as needed.
/// Upon completing the preprocessing step for a batch of actions, consumers can
/// call `handle_process_actions_result` to get the actions that are ready to be queued.
#[derive(Default, Debug)]
pub(super) struct PendingPreprocessedActions(VecDeque<PreprocessActionBatch>);

impl PendingPreprocessedActions {
    pub fn contains(&self, action_id: &AIAgentActionId) -> bool {
        self.0.iter().any(|action| action.contains(action_id))
    }

    /// Returns the actions that are ready to be queued now that the group of actions identified by [`PreprocessId`] have completed.
    /// NOTE this may return actions that have been completed earlier to maintain the invariant that actions are returned in the
    /// order they are added.
    pub fn handle_preprocess_actions_result(
        &mut self,
        preprocess_id: PreprocessId,
        actions: Vec<AIAgentAction>,
    ) -> Vec<AIAgentAction> {
        let mut actions_to_queue = Vec::with_capacity(actions.len());

        // Find the index of the action with the given preprocess_id
        let Some(current_index) = self.0.iter().position(|batch| batch.id == preprocess_id) else {
            log::warn!("Action not found in list of preprocessed actions");
            return vec![];
        };

        // Check if there are any pending actions before the current one
        let has_pending_before = self
            .0
            .iter()
            .take(current_index)
            .any(|action| matches!(action.status, PreprocessActionStatus::Pending));

        if has_pending_before {
            // If there are pending actions before this one, just mark this one as done
            // and don't return any actions yet.
            self.0[current_index].status = PreprocessActionStatus::Done { actions };
            vec![]
        } else {
            // All actions before this one are done: process them all.

            // First, collect actions from all completed batches before this one
            for action in self.0.drain(..current_index) {
                match action.status {
                    PreprocessActionStatus::Pending => {
                        #[cfg(debug_assertions)]
                        panic!("Preprocess action batch should be completed but was pending")
                    }
                    PreprocessActionStatus::Done { actions } => {
                        actions_to_queue.extend(actions);
                    }
                }
            }

            // Then add the current batch's actions.
            actions_to_queue.extend(actions);

            // Remove the current batch.
            self.0.pop_front();

            // Process any subsequent completed batches.
            while let Some(action) = self.0.pop_front() {
                match action.status {
                    PreprocessActionStatus::Pending => {
                        self.0.push_front(action);
                        break;
                    }
                    PreprocessActionStatus::Done { actions } => {
                        actions_to_queue.extend(actions);
                    }
                }
            }

            actions_to_queue
        }
    }

    /// Inserts a batch of actions that need to be preprocessed. Returns a [`PreprocessId`] that
    /// uniquely identifies the batch.
    pub fn insert_preprocess_action_batch(
        &mut self,
        action_ids: HashSet<AIAgentActionId>,
    ) -> PreprocessId {
        let preprocess_id = PreprocessId::new();
        self.0.push_back(PreprocessActionBatch::new(
            preprocess_id.clone(),
            action_ids,
        ));
        preprocess_id
    }
}

#[derive(Clone, Debug, PartialEq)]
enum PreprocessActionStatus {
    Pending,
    Done { actions: Vec<AIAgentAction> },
}

/// A batch of actions that need to be preprocessed.
#[derive(Clone, Debug, PartialEq)]
struct PreprocessActionBatch {
    /// A unique identifier for this batch.
    id: PreprocessId,
    /// The current status of this batch.
    status: PreprocessActionStatus,
    /// Action IDs associated with this batch.
    action_ids: HashSet<AIAgentActionId>,
}

impl PreprocessActionBatch {
    fn contains(&self, action_id: &AIAgentActionId) -> bool {
        self.action_ids.contains(action_id)
    }

    fn new(preprocess_id: PreprocessId, action_ids: HashSet<AIAgentActionId>) -> Self {
        Self {
            id: preprocess_id,
            status: PreprocessActionStatus::Pending,
            action_ids,
        }
    }
}

#[cfg(test)]
#[path = "preprocess_tests.rs"]
mod tests;
