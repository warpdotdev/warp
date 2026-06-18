use std::collections::{HashMap, HashSet};

use warp_graphql::ai::AgentTaskState;

use super::LocalTaskUpdate;
use crate::ai::ambient_agents::AmbientAgentTaskId;

/// Applies model-owned delivery state to outgoing updates and records the
/// results of requests sent by `LocalAgentTaskSyncModel`.
#[derive(Default)]
pub struct LocalTaskUpdateCache {
    in_progress_delivery_states: HashMap<AmbientAgentTaskId, InProgressDeliveryState>,
    next_in_progress_delivery_generation: u64,
    server_conversation_token_delivery_states:
        HashMap<AmbientAgentTaskId, ServerConversationTokenDeliveryState>,
    next_server_conversation_token_delivery_generation: u64,
    /// Different tokens cannot be safely ordered by the current fire-and-forget
    /// transport, so fail open after observing an overlap.
    server_conversation_token_dedupe_disabled: HashSet<AmbientAgentTaskId>,
}

/// Identifies the delivery attempts represented by one prepared update.
#[derive(Clone, Copy)]
pub struct PreparedLocalTaskUpdate {
    task_id: AmbientAgentTaskId,
    in_progress_delivery_generation: Option<u64>,
    server_conversation_token_delivery_generation: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InProgressDeliveryState {
    InFlight(u64),
    Delivered,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ServerConversationTokenDeliveryState {
    InFlight {
        generation: u64,
        token: String,
        previously_delivered: Option<String>,
    },
    Delivered(String),
}

impl LocalTaskUpdateCache {
    /// Strips successfully delivered fields from `update`, records any new
    /// in-flight deliveries, and returns a receipt for `record_result`.
    pub fn apply_to_update(
        &mut self,
        task_id: AmbientAgentTaskId,
        update: &mut LocalTaskUpdate,
    ) -> PreparedLocalTaskUpdate {
        let server_conversation_token_delivery_generation =
            self.apply_server_conversation_token_to_update(task_id, update);
        let in_progress_delivery_generation =
            self.apply_in_progress_state_to_update(task_id, update);
        PreparedLocalTaskUpdate {
            task_id,
            in_progress_delivery_generation,
            server_conversation_token_delivery_generation,
        }
    }

    fn apply_server_conversation_token_to_update(
        &mut self,
        task_id: AmbientAgentTaskId,
        update: &mut LocalTaskUpdate,
    ) -> Option<u64> {
        let token = update.server_conversation_token.clone()?;
        if self
            .server_conversation_token_dedupe_disabled
            .contains(&task_id)
        {
            return None;
        }

        let delivery_state = self
            .server_conversation_token_delivery_states
            .get(&task_id)
            .cloned();
        match delivery_state {
            Some(ServerConversationTokenDeliveryState::Delivered(delivered_token))
                if delivered_token == token =>
            {
                update.server_conversation_token = None;
                None
            }
            Some(ServerConversationTokenDeliveryState::InFlight {
                token: in_flight_token,
                ..
            }) if in_flight_token != token => {
                self.server_conversation_token_delivery_states
                    .remove(&task_id);
                self.server_conversation_token_dedupe_disabled
                    .insert(task_id);
                None
            }
            delivery_state => {
                let previously_delivered = match delivery_state {
                    Some(ServerConversationTokenDeliveryState::Delivered(token)) => Some(token),
                    Some(ServerConversationTokenDeliveryState::InFlight {
                        previously_delivered,
                        ..
                    }) => previously_delivered,
                    None => None,
                };
                self.next_server_conversation_token_delivery_generation = self
                    .next_server_conversation_token_delivery_generation
                    .wrapping_add(1);
                let generation = self.next_server_conversation_token_delivery_generation;
                self.server_conversation_token_delivery_states.insert(
                    task_id,
                    ServerConversationTokenDeliveryState::InFlight {
                        generation,
                        token,
                        previously_delivered,
                    },
                );
                Some(generation)
            }
        }
    }

    fn apply_in_progress_state_to_update(
        &mut self,
        task_id: AmbientAgentTaskId,
        update: &mut LocalTaskUpdate,
    ) -> Option<u64> {
        match update.task_state {
            // Status messages are only applied by the server together with a
            // task state, so an update carrying one must retain its state.
            Some(AgentTaskState::InProgress) if update.status_message.is_none() => {
                match self.in_progress_delivery_states.get(&task_id).copied() {
                    Some(InProgressDeliveryState::Delivered) => {
                        update.task_state = None;
                        None
                    }
                    Some(InProgressDeliveryState::InFlight(_))
                        if update.session_id.is_none()
                            && update.server_conversation_token.is_none() =>
                    {
                        update.task_state = None;
                        None
                    }
                    Some(InProgressDeliveryState::InFlight(_)) | None => {
                        self.next_in_progress_delivery_generation =
                            self.next_in_progress_delivery_generation.wrapping_add(1);
                        let generation = self.next_in_progress_delivery_generation;
                        self.in_progress_delivery_states
                            .insert(task_id, InProgressDeliveryState::InFlight(generation));
                        Some(generation)
                    }
                }
            }
            Some(_) => {
                self.in_progress_delivery_states.remove(&task_id);
                None
            }
            None => None,
        }
    }

    /// Records whether the prepared update was successfully applied by the
    /// server, retaining only successful delivery state.
    pub fn record_result(&mut self, prepared_update: PreparedLocalTaskUpdate, succeeded: bool) {
        let PreparedLocalTaskUpdate {
            task_id,
            in_progress_delivery_generation,
            server_conversation_token_delivery_generation,
        } = prepared_update;

        if let Some(generation) = in_progress_delivery_generation {
            self.record_in_progress_result(task_id, generation, succeeded);
        }
        if let Some(generation) = server_conversation_token_delivery_generation {
            self.record_server_conversation_token_result(task_id, generation, succeeded);
        }
    }

    fn record_in_progress_result(
        &mut self,
        task_id: AmbientAgentTaskId,
        generation: u64,
        succeeded: bool,
    ) {
        if self.in_progress_delivery_states.get(&task_id)
            != Some(&InProgressDeliveryState::InFlight(generation))
        {
            return;
        }

        if succeeded {
            self.in_progress_delivery_states
                .insert(task_id, InProgressDeliveryState::Delivered);
        } else {
            self.in_progress_delivery_states.remove(&task_id);
        }
    }

    fn record_server_conversation_token_result(
        &mut self,
        task_id: AmbientAgentTaskId,
        generation: u64,
        succeeded: bool,
    ) {
        if self
            .server_conversation_token_dedupe_disabled
            .contains(&task_id)
        {
            return;
        }
        let Some(ServerConversationTokenDeliveryState::InFlight {
            generation: current_generation,
            token,
            previously_delivered,
        }) = self
            .server_conversation_token_delivery_states
            .get(&task_id)
            .cloned()
        else {
            return;
        };
        if current_generation != generation {
            return;
        }

        if succeeded {
            self.server_conversation_token_delivery_states.insert(
                task_id,
                ServerConversationTokenDeliveryState::Delivered(token),
            );
        } else if let Some(previously_delivered) = previously_delivered {
            self.server_conversation_token_delivery_states.insert(
                task_id,
                ServerConversationTokenDeliveryState::Delivered(previously_delivered),
            );
        } else {
            self.server_conversation_token_delivery_states
                .remove(&task_id);
        }
    }

    pub fn remove_task(&mut self, task_id: &AmbientAgentTaskId) {
        self.in_progress_delivery_states.remove(task_id);
        self.server_conversation_token_delivery_states
            .remove(task_id);
        self.server_conversation_token_dedupe_disabled
            .remove(task_id);
    }
}
