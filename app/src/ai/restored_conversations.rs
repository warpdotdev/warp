//! A singleton model for storing conversations by ID to enable restoration across terminal views.

use std::collections::HashMap;
use warpui::{Entity, SingletonEntity};

use crate::{
    ai::{
        agent::conversation::{AIConversation, AIConversationId},
        blocklist::history_model::convert_persisted_conversation_to_ai_conversation_with_metadata,
    },
    persistence::model::AgentConversation,
};

/// Singleton model that holds restored agent conversations on app startup.
///
/// Loading restored conversations into this model is a means of propagating restored data from
/// sqlite (read at startup) to arbitrary consuming locations in the view/model hierarchy without
/// piping it all the way from the root view to the terminal view(s) that require it.
#[derive(Default)]
pub struct RestoredAgentConversations {
    /// All conversations stored by their ID, available for restoration
    conversations: HashMap<AIConversationId, AIConversation>,
}

impl RestoredAgentConversations {
    pub fn new(conversations: Vec<AgentConversation>) -> Self {
        let mut conversations_by_id = HashMap::new();
        for conversation in conversations.into_iter() {
            let conversation_id = conversation.conversation.conversation_id.clone();
            let Some(conversation) =
                convert_persisted_conversation_to_ai_conversation_with_metadata(conversation)
            else {
                log::warn!(
                    "Failed to convert persisted conversation {conversation_id} to AIConversation"
                );
                continue;
            };
            conversations_by_id.insert(conversation.id(), conversation);
        }

        Self {
            conversations: conversations_by_id,
        }
    }

    /// Gets a reference to a restored conversation without removing it.
    pub fn get_conversation(&self, id: &AIConversationId) -> Option<&AIConversation> {
        self.conversations.get(id)
    }

    /// Removes the restored conversation and returns it, if any.
    pub fn take_conversation(&mut self, id: &AIConversationId) -> Option<AIConversation> {
        self.conversations.remove(id)
    }

    /// Takes and returns AIConversations for the given IDs, sorted by first exchange start time.
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
