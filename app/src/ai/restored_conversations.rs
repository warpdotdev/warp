//! A singleton model for storing conversations by ID to enable restoration across terminal views.
//!
//! Conversations are stored in their raw persisted form (`AgentConversation`) and only converted
//! to the full `AIConversation` representation on demand — when a specific conversation is
//! actually needed for terminal view restoration. This lazy conversion avoids a multi-GiB
//! startup allocation that occurred when every persisted conversation was eagerly decoded and
//! materialized into `AIConversation` objects (see APP-4525).

use std::collections::HashMap;
use warpui::{Entity, SingletonEntity};

use crate::{
    ai::{
        agent::conversation::{AIConversation, AIConversationId},
        blocklist::history_model::convert_persisted_conversation_to_ai_conversation_with_metadata,
    },
    persistence::model::{AgentConversation, AgentConversationData},
};

/// Singleton model that holds restored agent conversations on app startup.
///
/// Loading restored conversations into this model is a means of propagating restored data from
/// sqlite (read at startup) to arbitrary consuming locations in the view/model hierarchy without
/// piping it all the way from the root view to the terminal view(s) that require it.
///
/// Conversations are stored in their raw persisted form and only converted to `AIConversation`
/// lazily when taken for restoration, avoiding a large upfront memory spike.
#[derive(Default)]
pub struct RestoredAgentConversations {
    /// Raw persisted conversations stored by their ID, awaiting lazy conversion.
    raw_conversations: HashMap<AIConversationId, AgentConversation>,
}

impl RestoredAgentConversations {
    pub fn new(conversations: Vec<AgentConversation>) -> Self {
        let mut raw_conversations = HashMap::new();
        for conversation in conversations.into_iter() {
            let conversation_id = conversation.conversation.conversation_id.clone();
            match AIConversationId::try_from(conversation_id.clone()) {
                Ok(id) => {
                    raw_conversations.insert(id, conversation);
                }
                Err(e) => {
                    log::warn!("Failed to parse conversation ID {conversation_id}: {e:?}");
                }
            }
        }

        Self { raw_conversations }
    }

    /// Checks whether a raw conversation exists and should be restored (has tasks, is not
    /// entirely passive). This avoids converting to `AIConversation` just for filtering.
    pub fn should_restore_conversation(&self, id: &AIConversationId) -> bool {
        let Some(conv) = self.raw_conversations.get(id) else {
            return false;
        };

        // Must have at least one task
        if conv.tasks.is_empty() {
            return false;
        }

        // Check if the conversation is entirely passive (has passive system queries
        // but no user queries). This mirrors AIConversation::is_entirely_passive()
        // but operates directly on the raw protobuf tasks.
        let mut has_passive_request = false;
        let mut has_user_query = false;
        for task in &conv.tasks {
            // Only check the root task (no parent) to mirror root_task_exchanges()
            let is_root = task
                .dependencies
                .as_ref()
                .map(|deps| deps.parent_task_id.is_empty())
                .unwrap_or(true);
            if !is_root {
                continue;
            }
            for message in &task.messages {
                match &message.message {
                    Some(warp_multi_agent_api::message::Message::UserQuery(_)) => {
                        has_user_query = true;
                    }
                    Some(warp_multi_agent_api::message::Message::SystemQuery(_)) => {
                        has_passive_request = true;
                    }
                    _ => {}
                }
            }
        }

        // If it has passive requests but no user queries, it's entirely passive
        if has_passive_request && !has_user_query {
            return false;
        }

        true
    }

    /// Returns the parent conversation ID from the raw persisted data, if any.
    /// This avoids converting to `AIConversation` just to look up the parent.
    pub fn get_parent_conversation_id(&self, id: &AIConversationId) -> Option<AIConversationId> {
        let conv = self.raw_conversations.get(id)?;
        let data =
            serde_json::from_str::<AgentConversationData>(&conv.conversation.conversation_data)
                .ok()?;
        let parent_id_str = data.parent_conversation_id?;
        AIConversationId::try_from(parent_id_str).ok()
    }

    /// Removes the restored conversation, converts it to `AIConversation`, and returns it.
    pub fn take_conversation(&mut self, id: &AIConversationId) -> Option<AIConversation> {
        let raw = self.raw_conversations.remove(id)?;
        let raw_id = raw.conversation.conversation_id.clone();
        match convert_persisted_conversation_to_ai_conversation_with_metadata(raw) {
            Some(conversation) => Some(conversation),
            None => {
                log::warn!("Failed to convert persisted conversation {raw_id} to AIConversation");
                None
            }
        }
    }

    /// Takes and returns AIConversations for the given IDs, sorted by first exchange start time.
    /// Conversations are converted from their raw persisted form on demand.
    pub fn take_conversations(
        &mut self,
        conversation_ids: &[AIConversationId],
    ) -> Vec<AIConversation> {
        let mut conversations = Vec::new();
        for &conversation_id in conversation_ids {
            if let Some(conversation) = self.take_conversation(&conversation_id) {
                conversations.push(conversation);
            }
        }

        // Sort by first exchange start time (oldest first)
        conversations.sort_by_key(|conversation| {
            conversation
                .first_exchange()
                .map(|exchange| exchange.start_time)
        });
        conversations
    }
}

impl Entity for RestoredAgentConversations {
    type Event = ();
}

impl SingletonEntity for RestoredAgentConversations {}
