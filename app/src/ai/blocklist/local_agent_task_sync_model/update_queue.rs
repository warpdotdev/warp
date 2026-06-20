use std::collections::{HashMap, VecDeque};

use warp_graphql::ai::AgentTaskState;

use super::LocalTaskUpdate;
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::server::server_api::ai::TaskStatusUpdate;

/// Serializes and coalesces model-owned task updates independently per task.
///
/// Callers enqueue updates synchronously. The queue returns an update only when
/// that task has no request in flight, and the caller reports its result before
/// asking the queue for the next update.
#[derive(Default)]
pub struct LocalTaskUpdateQueue {
    task_queues: HashMap<AmbientAgentTaskId, TaskQueue>,
}

#[derive(Default)]
struct TaskQueue {
    pending_updates: VecDeque<LocalTaskUpdate>,
    in_flight_update: Option<InFlightUpdate>,
    delivered_state: DeliveredTaskState,
    remove_when_idle: bool,
}

/// Tracks confirmed server field values used to deduplicate future updates.
///
/// The queue currently deduplicates only bare `InProgress` states and repeated
/// server conversation tokens.
#[derive(Default)]
struct DeliveredTaskState {
    task_state: Option<AgentTaskState>,
    server_conversation_token: Option<String>,
}

struct InFlightUpdate {
    task_state: Option<AgentTaskState>,
    server_conversation_token: Option<String>,
}

impl InFlightUpdate {
    fn from_update(update: &LocalTaskUpdate) -> Self {
        Self {
            task_state: update.task_state,
            server_conversation_token: update.server_conversation_token.clone(),
        }
    }
}

impl LocalTaskUpdateQueue {
    /// Enqueues an update and returns it for immediate delivery when the task is
    /// idle. Otherwise, the update remains queued until the active request
    /// completes.
    pub fn enqueue(
        &mut self,
        task_id: AmbientAgentTaskId,
        update: LocalTaskUpdate,
    ) -> Option<LocalTaskUpdate> {
        if update.is_empty() {
            return None;
        }

        let queue = self.task_queues.entry(task_id).or_default();
        debug_assert!(
            !queue.remove_when_idle,
            "updates must not be enqueued while final task cleanup is pending"
        );
        queue.enqueue(update);
        self.take_next_update(task_id)
    }

    /// Records the active request's result and returns the next non-redundant
    /// update for the task, if one is ready.
    pub fn record_result(
        &mut self,
        task_id: AmbientAgentTaskId,
        succeeded: bool,
    ) -> Option<LocalTaskUpdate> {
        let queue = self.task_queues.get_mut(&task_id)?;
        let in_flight_update = queue.in_flight_update.take()?;
        queue
            .delivered_state
            .record_result(in_flight_update, succeeded);

        self.take_next_update(task_id)
    }

    /// Marks a task for final cleanup and removes its queue after updates
    /// already accepted by the queue finish.
    ///
    /// Callers must not enqueue another update for the same task after cleanup.
    pub fn remove_task(&mut self, task_id: &AmbientAgentTaskId) {
        let should_remove = if let Some(queue) = self.task_queues.get_mut(task_id) {
            queue.remove_when_idle = true;
            queue.in_flight_update.is_none() && queue.pending_updates.is_empty()
        } else {
            false
        };

        if should_remove {
            self.task_queues.remove(task_id);
        }
    }

    fn take_next_update(&mut self, task_id: AmbientAgentTaskId) -> Option<LocalTaskUpdate> {
        let should_remove = {
            let queue = self.task_queues.get_mut(&task_id)?;
            if queue.in_flight_update.is_some() {
                return None;
            }

            while let Some(mut update) = queue.pending_updates.pop_front() {
                queue.delivered_state.apply_to(&mut update);
                if update.is_empty() {
                    continue;
                }
                queue.in_flight_update = Some(InFlightUpdate::from_update(&update));
                return Some(update);
            }

            queue.remove_when_idle || !queue.delivered_state.has_dedupe_state()
        };

        if should_remove {
            self.task_queues.remove(&task_id);
        }
        None
    }
}

impl TaskQueue {
    fn enqueue(&mut self, update: LocalTaskUpdate) {
        let update = if let Some(tail) = self.pending_updates.back_mut() {
            match tail.try_coalesce(update) {
                Ok(()) => return,
                Err(update) => update,
            }
        } else {
            update
        };
        self.pending_updates.push_back(update);
    }
}

impl DeliveredTaskState {
    fn apply_to(&self, update: &mut LocalTaskUpdate) {
        if update.status_message.is_none()
            && update.task_state == Some(AgentTaskState::InProgress)
            && self.task_state == update.task_state
        {
            update.task_state = None;
        }

        if update
            .server_conversation_token
            .as_ref()
            .is_some_and(|token| self.server_conversation_token.as_ref() == Some(token))
        {
            update.server_conversation_token = None;
        }
    }

    fn record_result(&mut self, update: InFlightUpdate, succeeded: bool) {
        record_field_result(&mut self.task_state, update.task_state, succeeded);
        record_field_result(
            &mut self.server_conversation_token,
            update.server_conversation_token,
            succeeded,
        );
    }

    fn has_dedupe_state(&self) -> bool {
        self.task_state == Some(AgentTaskState::InProgress)
            || self.server_conversation_token.is_some()
    }
}

/// Updates a confirmed field value or invalidates it when a failed request may
/// have changed the server to a different value.
fn record_field_result<T: PartialEq>(
    delivered: &mut Option<T>,
    attempted: Option<T>,
    succeeded: bool,
) {
    let Some(attempted) = attempted else {
        return;
    };
    if succeeded {
        *delivered = Some(attempted);
    } else if delivered.as_ref() != Some(&attempted) {
        *delivered = None;
    }
}

impl LocalTaskUpdate {
    /// Merges `newer` into this queued update when doing so preserves every
    /// meaningful transition. Conflicting field values remain separate FIFO
    /// entries.
    fn try_coalesce(&mut self, newer: Self) -> Result<(), Self> {
        if !options_compatible(&self.task_state, &newer.task_state)
            || !options_compatible(&self.session_id, &newer.session_id)
            || !options_compatible(
                &self.server_conversation_token,
                &newer.server_conversation_token,
            )
            || !status_messages_compatible(&self.status_message, &newer.status_message)
        {
            return Err(newer);
        }

        let Self {
            task_state,
            session_id,
            server_conversation_token,
            status_message,
        } = newer;
        if task_state.is_some() {
            self.task_state = task_state;
        }
        if session_id.is_some() {
            self.session_id = session_id;
        }
        if server_conversation_token.is_some() {
            self.server_conversation_token = server_conversation_token;
        }
        if status_message.is_some() {
            self.status_message = status_message;
        }
        Ok(())
    }
}

fn options_compatible<T: PartialEq>(left: &Option<T>, right: &Option<T>) -> bool {
    left.is_none() || right.is_none() || left == right
}

fn status_messages_compatible(
    left: &Option<TaskStatusUpdate>,
    right: &Option<TaskStatusUpdate>,
) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => {
            left.message == right.message && left.error_code == right.error_code
        }
        (Some(_), None) | (None, Some(_)) | (None, None) => true,
    }
}
