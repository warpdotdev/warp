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
///
/// Loading one conversation is expensive: it reads and protobuf-decodes every
/// `agent_tasks` row for that conversation, and the result is retained in
/// `conversations` until someone takes it. Callers that only need to *decide*
/// something about a conversation must therefore go through a predicate that
/// can answer from the write-time `summary` column
/// ([`Self::should_restore_into_pane`]) instead of loading a payload they are
/// about to discard.
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

    /// Seeds the store with a backing database instead of already-loaded
    /// conversations. Only used by tests.
    #[cfg(all(test, feature = "local_fs"))]
    pub fn new_with_db_connection(connection: SqliteConnection) -> Self {
        Self {
            conversations: HashMap::new(),
            taken: HashSet::new(),
            db_connection: Some(Arc::new(Mutex::new(connection))),
        }
    }

    /// The number of conversations currently held in memory. Only used by tests
    /// asserting that rejected conversations aren't retained.
    #[cfg(test)]
    pub fn cached_conversation_count(&self) -> usize {
        self.conversations.len()
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
    ///
    /// A conversation loaded this way stays cached until it is taken, so only
    /// call this when the caller actually needs the payload.
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

    /// Whether a restored terminal pane should restore this conversation:
    /// conversations with nothing to show, and passive suggestions the user
    /// never acted on, are skipped so the pane doesn't surface a "Previous
    /// session" banner or re-apply an ignored code diff.
    ///
    /// Answered from the conversation's write-time summary whenever that
    /// summary can answer it, so the common case never reads or decodes the
    /// conversation's `agent_tasks` blobs. When it can't, the conversation is
    /// loaded and evaluated directly — but a conversation that fails the filter
    /// is dropped again instead of being retained for the process lifetime.
    pub fn should_restore_into_pane(&mut self, id: &AIConversationId) -> bool {
        if self.taken.contains(id) {
            return false;
        }

        if let Some(conversation) = self.conversations.get(id) {
            return Self::passes_pane_restore_filter(conversation);
        }

        if let Some(is_entirely_passive) = self.summary_is_entirely_passive(id) {
            // Restore always produces at least a root task — a synthesized one
            // for a task-less row — so a conversation the summary can speak for
            // can never fail the has-tasks half of the filter.
            return !is_entirely_passive;
        }

        let conversation = self.load_from_db(id);
        let Some(conversation) = conversation.filter(Self::passes_pane_restore_filter) else {
            return false;
        };
        // Caching only the conversations that pass keeps the ones we just
        // rejected out of memory while still letting the imminent
        // `take_conversation` reuse this load.
        self.conversations.insert(*id, conversation);
        true
    }

    /// The pane-restore filter evaluated against a fully loaded conversation.
    fn passes_pane_restore_filter(conversation: &AIConversation) -> bool {
        if conversation.all_tasks().next().is_none() {
            return false;
        }
        !conversation.is_entirely_passive()
    }

    /// Reads `is_entirely_passive` out of the conversation's write-time
    /// summary, without touching its task payloads. `None` means the summary
    /// can't answer the question and the caller must fall back to a full load.
    fn summary_is_entirely_passive(&self, id: &AIConversationId) -> Option<bool> {
        #[cfg(feature = "local_fs")]
        {
            let conn = self.db_connection.clone()?;
            let mut conn = conn.lock().ok()?;
            match crate::persistence::agent::read_agent_conversation_summary_by_id(
                &mut conn,
                &id.to_string(),
            ) {
                Ok(summary) => summary?.is_entirely_passive,
                Err(e) => {
                    log::warn!("Failed to read AgentConversation summary {id}: {e:?}");
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
