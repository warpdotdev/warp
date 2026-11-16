//! A singleton model for restoring conversations by ID across terminal views.

use std::collections::{HashMap, HashSet};
#[cfg(feature = "local_fs")]
use std::sync::{Arc, Mutex};

#[cfg(feature = "local_fs")]
use diesel::SqliteConnection;
use warpui::{Entity, SingletonEntity};

use crate::ai::agent::conversation::{AIConversation, AIConversationId};
#[cfg_attr(not(any(feature = "local_fs", test)), allow(unused_imports))]
use crate::ai::blocklist::history_model::convert_persisted_conversation_to_ai_conversation_with_metadata;
#[cfg(test)]
use crate::persistence::model::AgentConversation;
#[cfg(feature = "local_fs")]
use crate::persistence::{database_file_path_for_current_scope, establish_ro_connection};

/// Singleton model that restores agent conversations on demand.
///
/// Startup only loads conversation metadata, so the full task payloads are
/// loaded lazily from the local database the first time a consumer (e.g. pane
/// restoration) asks for a conversation. Consuming restored data this way
/// avoids piping it from the root view down to the terminal view(s) that
/// require it.
pub struct RestoredAgentConversations {
    /// Conversations already loaded (or test-seeded) but not yet taken.
    conversations: HashMap<AIConversationId, AIConversation>,
    /// IDs already handed out via `take_conversation(s)`. Preserves the
    /// historical take-once semantics now that the backing database can
    /// otherwise serve the same conversation repeatedly.
    taken: HashSet<AIConversationId>,
    #[cfg(feature = "local_fs")]
    db_connection: Option<Arc<Mutex<SqliteConnection>>>,
}

impl RestoredAgentConversations {
    pub fn new() -> Self {
        #[cfg(feature = "local_fs")]
        let db_connection = database_file_path_for_current_scope()
            .to_str()
            .and_then(|db_url| {
                establish_ro_connection(db_url)
                    .ok()
                    .map(|conn| Arc::new(Mutex::new(conn)))
            });

        Self {
            conversations: HashMap::new(),
            taken: HashSet::new(),
            #[cfg(feature = "local_fs")]
            db_connection,
        }
    }

    /// Seeds the store with already-loaded conversations instead of a backing
    /// database. Only used by tests.
    #[cfg(test)]
    pub fn new_seeded(conversations: Vec<AgentConversation>) -> Self {
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
            taken: HashSet::new(),
            #[cfg(feature = "local_fs")]
            db_connection: None,
        }
    }

    /// Loads and converts a conversation from the local database.
    fn load_from_db(&self, id: &AIConversationId) -> Option<AIConversation> {
        #[cfg(feature = "local_fs")]
        {
            let conn = self.db_connection.clone()?;
            let mut conn = conn.lock().ok()?;
            match crate::persistence::agent::read_agent_conversation_by_id(
                &mut conn,
                &id.to_string(),
            ) {
                Ok(Some(conversation)) => {
                    convert_persisted_conversation_to_ai_conversation_with_metadata(conversation)
                }
                Ok(None) => None,
                Err(e) => {
                    log::warn!("Failed to read AgentConversation {id}: {e:?}");
                    None
                }
            }
        }
        #[cfg(not(feature = "local_fs"))]
        {
            let _ = id;
            None
        }
    }

    /// Gets a reference to a restored conversation without taking it, loading
    /// it from the local database when it isn't cached yet.
    pub fn get_conversation(&mut self, id: &AIConversationId) -> Option<&AIConversation> {
        if self.taken.contains(id) {
            return None;
        }
        if !self.conversations.contains_key(id) {
            let loaded = self.load_from_db(id)?;
            self.conversations.insert(*id, loaded);
        }
        self.conversations.get(id)
    }

    /// Takes the restored conversation and returns it, if any. Each
    /// conversation is handed out at most once.
    ///
    /// The ID is only marked as taken once a conversation was actually
    /// handed out, so a failed load (e.g. a transient read error) doesn't
    /// permanently consume the restore opportunity for this session.
    pub fn take_conversation(&mut self, id: &AIConversationId) -> Option<AIConversation> {
        if self.taken.contains(id) {
            return None;
        }
        let conversation = self
            .conversations
            .remove(id)
            .or_else(|| self.load_from_db(id))?;
        self.taken.insert(*id);
        Some(conversation)
    }

    /// Takes and returns AIConversations for the given IDs, sorted by first exchange start time.
    pub fn take_conversations(
        &mut self,
        conversation_ids: &[AIConversationId],
    ) -> Vec<AIConversation> {
        let mut conversations = Vec::new();
        for conversation_id in conversation_ids {
            if let Some(conversation) = self.take_conversation(conversation_id) {
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

#[cfg(test)]
#[path = "restored_conversations_tests.rs"]
mod tests;
