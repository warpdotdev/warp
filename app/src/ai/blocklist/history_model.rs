use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
#[cfg(feature = "local_fs")]
use std::sync::Mutex;

use ai::skills::SkillPathOrigin;
use anyhow::anyhow;
use chrono::{DateTime, Local, NaiveDateTime};
#[cfg(feature = "local_fs")]
use diesel::SqliteConnection;
use itertools::Itertools as _;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use warp_cli::agent::Harness;
use warp_core::features::FeatureFlag;
use warp_multi_agent_api::client_action::{Action, StartNewConversation};
use warp_multi_agent_api::message::tool_call::Tool;
use warp_multi_agent_api::response_event::stream_finished::{
    ConversationUsageMetadata, TokenUsage,
};
use warpui::{AppContext, Entity, EntityId, ModelContext, SingletonEntity};

use super::controller::response_stream::ResponseStreamId;
use super::persistence::{PersistedAIInput, PersistedAIInputType};
use super::RequestInput;
use crate::ai::agent::api::ServerConversationToken;
use crate::ai::agent::conversation::{
    AIConversation, AIConversationId, ConversationStatus, ServerAIConversationMetadata, TodoStatus,
    UpdateConversationError,
};
use crate::ai::agent::task::helper::{MessageExt, ToolCallExt};
use crate::ai::agent::task::TaskId;
use crate::ai::agent::todos::AIAgentTodoList;
use crate::ai::agent::{
    AIAgentActionId, AIAgentExchange, AIAgentExchangeId, AIAgentInput, AIAgentOutputStatus,
    AIAgentTodoId, CancellationReason, FinishedAIAgentOutput, MessageId, RenderableAIError,
    RequestCost, Suggestions,
};
use crate::ai::artifacts::Artifact;
use crate::ai::document::ai_document_model::AIDocumentModel;
use crate::input_suggestions::HistoryOrder;
use crate::persistence::model::{AgentConversation, AgentConversationData};
use crate::persistence::ModelEvent;
#[cfg(feature = "local_fs")]
use crate::persistence::{database_file_path_for_current_scope, establish_ro_connection};
use crate::server::server_api::ServerApiProvider;
use crate::terminal::model::block::BlockId;
use crate::terminal::view::blocklist_filter;
use crate::ui_components::icons::Icon;
use crate::GlobalResourceHandlesProvider;

mod conversation_loader;
pub use conversation_loader::{
    convert_persisted_conversation_to_ai_conversation_with_metadata, load_conversation_from_server,
    CLIAgentConversation, CloudConversationData,
};
use warp_errors::report_error;

/// Mirrors [`crate::persistence::agent::MAX_PERSISTED_CONVERSATION_COUNT`].
/// Moot at steady state because the disk-side prune already keeps the
/// persisted set within this window; kept as defense-in-depth if rows ever
/// arrive from another source (cross-machine import, prune bypass).
pub(super) const MAX_HISTORICAL_CONVERSATIONS: usize = 200;

/// Metadata for conversations
/// When created from local DB, has_local_data=true and server_metadata=None.
/// When fetched from server, has_local_data=false and server_metadata=Some(...).
#[derive(Debug, Clone)]
pub struct AIConversationMetadata {
    pub id: AIConversationId,

    pub title: String,

    pub initial_query: String,

    pub last_modified_at: NaiveDateTime,

    pub initial_working_directory: Option<String>,

    pub credits_spent: Option<f32>,

    pub server_conversation_token: Option<ServerConversationToken>,

    /// Whether the full conversation data exists in the local database.
    /// false = must be fetched from server
    /// true = exists in local DB and can be fetched from there, even if it also exists in server
    pub has_local_data: bool,

    /// Whether this conversation exists in the cloud (has been synced).
    /// This is used to determine if the conversation can be shared.
    pub has_cloud_data: bool,

    /// Artifacts (plans, PRs) created during this conversation.
    pub artifacts: Vec<Artifact>,

    /// Full server metadata for cloud conversations, including permissions.
    /// Used by the sharing dialog to display permissions when the full conversation isn't loaded.
    pub server_conversation_metadata: Option<ServerAIConversationMetadata>,
}

impl From<&AIConversation> for AIConversationMetadata {
    fn from(conversation: &AIConversation) -> Self {
        let title = conversation.title().unwrap_or_default().to_string();
        let initial_query: String = conversation.initial_query().unwrap_or_default();
        let server_conversation_token = conversation.server_conversation_token().cloned();
        let has_cloud_data =
            conversation.server_metadata().is_some() || server_conversation_token.is_some();

        let last_modified_at = conversation
            .latest_exchange()
            .map(|exchange| exchange.start_time.naive_utc())
            .unwrap_or_else(|| chrono::Utc::now().naive_utc());

        Self {
            id: conversation.id(),
            title,
            initial_query,
            last_modified_at,
            initial_working_directory: conversation.initial_working_directory(),
            credits_spent: Some(conversation.credits_spent()),
            server_conversation_token,
            has_local_data: true,
            has_cloud_data,
            artifacts: conversation.artifacts().to_vec(),
            server_conversation_metadata: conversation.server_metadata().cloned(),
        }
    }
}

impl AIConversationMetadata {
    /// Create metadata from server-fetched GraphQL data.
    /// This is used when loading conversations from the cloud.
    pub fn from_server_metadata(
        conversation_id: AIConversationId,
        server_conversation_metadata: ServerAIConversationMetadata,
    ) -> Self {
        let title = server_conversation_metadata.title.clone();
        let last_modified_at = server_conversation_metadata
            .metadata
            .metadata_last_updated_ts
            .utc()
            .naive_utc();
        let credits_spent = Some(
            server_conversation_metadata.usage.credits_spent
                + server_conversation_metadata.usage.platform_credits_spent,
        );
        let server_conversation_token = Some(
            server_conversation_metadata
                .server_conversation_token
                .clone(),
        );
        let initial_working_directory = server_conversation_metadata.working_directory.clone();
        let artifacts = server_conversation_metadata.artifacts.clone();

        Self {
            id: conversation_id,
            title,
            // Server doesn't currently provide initial query in metadata
            // This is used to allow searching by initial query in command palette.
            initial_query: String::new(),
            last_modified_at,
            initial_working_directory,
            credits_spent,
            server_conversation_token,
            has_local_data: false,
            has_cloud_data: true, // Server metadata implies cloud data exists
            artifacts,
            server_conversation_metadata: Some(server_conversation_metadata),
        }
    }

    /// Whether this conversation is owned by an ambient agent run rather than
    /// being a direct user conversation.
    pub fn is_ambient_agent_conversation(&self) -> bool {
        self.server_conversation_metadata
            .as_ref()
            .is_some_and(|m| m.ambient_agent_task_id.is_some())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum UpdateHistoryError {
    #[error("Failed to update conversation: {0:?}")]
    Conversation(#[from] UpdateConversationError),
    #[error("Failed to find conversation with ID {0:?}")]
    ConversationNotFound(AIConversationId),
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub(crate) enum BeginConversationRenameError {
    #[error("conversation not found")]
    ConversationNotFound,
    #[error("conversation has no server token")]
    MissingServerConversationToken,
    #[error("conversation is not ready to rename")]
    ConversationNotReady,
    #[error("conversation rename already in progress")]
    RenameInProgress,
}

#[derive(Debug, Clone)]
struct InFlightConversationRename {
    attempted_title: String,
    previous_root_task_description: String,
    previous_server_metadata_title: Option<String>,
    previous_cached_metadata_title: Option<String>,
}

/// A single agent prompt-history candidate with prompt text and start_ts.
#[derive(Clone, Debug)]
pub(crate) struct PromptHistoryEntry {
    /// The user prompt text.
    pub(crate) text: Arc<str>,
    /// When the prompt was submitted.
    pub(crate) start_ts: DateTime<Local>,
}

/// Responsible for managing the history of user and AI exchanges.
#[derive(Default)]
pub struct BlocklistAIHistoryModel {
    /// Live conversations for each terminal surface.
    ///
    /// "Live" conversations are still visible/selectable for that terminal surface. Clearing
    /// a terminal surface moves those IDs out of this map, but closing a GUI view does not
    /// immediately remove its entry so the conversations can be restored.
    live_conversation_ids_for_terminal_surface: HashMap<EntityId, Vec<AIConversationId>>,

    /// Conversations that were once live for a terminal surface, but were cleared from the blocklist.
    ///
    /// This is used to preserve queries for up-arrow history after clearing the blocklist.
    cleared_conversation_ids_for_terminal_surface: HashMap<EntityId, Vec<AIConversationId>>,

    /// A [`HashMap`] mapping a [`AIConversationId`] to the [`AIConversation`] itself.
    /// Conversations may or may not be live in any open session. They will exist in this map if they
    /// have ever been loaded into memory.
    conversations_by_id: HashMap<AIConversationId, AIConversation>,

    /// The active conversation ID for a given terminal surface.
    ///
    /// The active conversation is the terminal surface's current or most recent progress
    /// target, such as a response stream or follow-up action flow. Selection
    /// lives in the surface's context/controller models.
    active_conversation_for_terminal_surface: HashMap<EntityId, AIConversationId>,

    /// The time at which each terminal surface was created. Note that this
    /// has no bearing on when any [`AIConversation`]s take place for that terminal surface.
    terminal_surface_created_at: HashMap<EntityId, DateTime<Local>>,

    /// A set of terminal surfaces that are shared ambient agent sessions.
    ambient_agent_terminal_surface_ids: HashSet<EntityId>,

    /// A set of terminal surfaces that are read-only conversation transcript viewers.
    /// This is view/UI state (not conversation state) and is used to filter transcript viewer
    /// conversations out of local history and navigation.
    conversation_transcript_viewer_terminal_surface_ids: HashSet<EntityId>,

    /// AI queries that were read from the SQLite DB. These exchanges do not contain as much
    /// information as the other exchanges we store because they are only used for display in
    /// history.
    persisted_queries: Vec<PersistedAIInput>,

    // TODO: When up-arrow prompt history supports pagination, consolidate
    // `persisted_queries` and `prompt_history` (both seeded from the same `ai_queries`
    // read) and share the in-memory conversation appending done by `all_ai_queries`.
    /// Prompt-history candidates for NLD input classification. Seeded once from `ai_queries`
    /// at startup, then extended with prompts submitted during the session
    /// (see [`Self::append_session_prompt`]). Ordered oldest-first.
    prompt_history: Vec<PromptHistoryEntry>,

    /// Metadata for both local and ambient agent conversations.
    /// Does not include the actual content of the conversations.
    all_conversations_metadata: HashMap<AIConversationId, AIConversationMetadata>,

    /// Reverse index from server-side agent identifier to local conversation ID.
    ///
    /// Keyed by `run_id` for current orchestration. Older conversation data may
    /// still contain `server_conversation_token`-backed identifiers, but new
    /// runtime lookups use run IDs.
    agent_id_to_conversation_id: HashMap<String, AIConversationId>,

    /// Reverse index from [`ServerConversationToken`] to local [`AIConversationId`].
    ///
    /// Maintained alongside every mutation of `conversations_by_id` and
    /// `all_conversations_metadata` that involves a token. Used to make
    /// `find_conversation_id_by_server_token` O(1); it is called once per
    /// ambient-agent task on every conversation-list refresh.
    server_token_to_conversation_id: HashMap<ServerConversationToken, AIConversationId>,

    /// Index from parent conversation ID to child conversation IDs.
    /// Populated at startup from the local DB and kept in sync at runtime
    /// via `set_parent_for_conversation` and `restore_conversations`.
    children_by_parent: HashMap<AIConversationId, Vec<AIConversationId>>,

    /// Conversations that have had at least one AIBlock receive imported review comments.
    conversations_with_imported_comments: HashSet<AIConversationId>,

    /// In-flight optimistic conversation rename state keyed by conversation.
    in_flight_conversation_renames: HashMap<AIConversationId, InFlightConversationRename>,

    #[cfg(feature = "local_fs")]
    db_connection: Option<Arc<Mutex<SqliteConnection>>>,
}

impl BlocklistAIHistoryModel {
    pub(crate) fn new(
        persisted_queries: Vec<PersistedAIInput>,
        prompt_history: Vec<(String, DateTime<Local>)>,
        multi_agent_conversations: &[AgentConversation],
    ) -> Self {
        #[cfg(feature = "local_fs")]
        let db_connection = database_file_path_for_current_scope()
            .to_str()
            .and_then(|db_url| {
                establish_ro_connection(db_url)
                    .ok()
                    .map(|conn| Arc::new(Mutex::new(conn)))
            });

        let prompt_history = prompt_history
            .into_iter()
            .filter(|(text, _)| !text.trim().is_empty())
            .map(|(text, start_ts)| PromptHistoryEntry {
                text: Arc::from(text),
                start_ts,
            })
            .collect();

        let mut model = Self {
            persisted_queries,
            prompt_history,
            #[cfg(feature = "local_fs")]
            db_connection,
            ..Self::default()
        };

        // Initialize historical conversations from local DB
        model.initialize_historical_conversations(multi_agent_conversations);

        model
    }

    #[cfg(test)]
    pub(crate) fn new_for_test() -> Self {
        Self::default()
    }

    /// Returns a flattened and ordered (oldest first) list of live conversations for a terminal surface.
    /// This works for terminal surfaces that have been closed.
    pub fn all_live_conversations_for_terminal_surface(
        &self,
        terminal_surface_id: EntityId,
    ) -> impl Iterator<Item = &AIConversation> {
        self.live_conversation_ids_for_terminal_surface
            .get(&terminal_surface_id)
            .into_iter()
            .flat_map(|conversation_ids| {
                conversation_ids
                    .iter()
                    .filter_map(|conversation_id| self.conversation(conversation_id))
            })
    }

    /// Returns a flattened and ordered (oldest first) list of exchanges from a terminal surface's live conversations.
    /// This works for terminal surfaces that have been closed.
    pub fn all_live_root_task_exchanges_for_terminal_surface(
        &self,
        terminal_surface_id: EntityId,
    ) -> impl Iterator<Item = &AIAgentExchange> {
        self.live_conversation_ids_for_terminal_surface
            .get(&terminal_surface_id)
            .into_iter()
            .flat_map(|conversation_ids| {
                conversation_ids.iter().flat_map(|conversation_id| {
                    self.conversations_by_id
                        .get(conversation_id)
                        .map(|conversation| conversation.root_task_exchanges())
                })
            })
            .flatten()
    }

    /// Returns a flattened and ordered (oldest first) list of exchanges from conversations
    /// that were cleared for a terminal surface, but are no longer live/visible.
    pub fn all_cleared_root_task_exchanges_for_terminal_surface(
        &self,
        terminal_surface_id: EntityId,
    ) -> impl Iterator<Item = &AIAgentExchange> {
        self.cleared_conversation_ids_for_terminal_surface
            .get(&terminal_surface_id)
            .into_iter()
            .flat_map(|conversation_ids| {
                conversation_ids.iter().flat_map(|conversation_id| {
                    self.conversations_by_id
                        .get(conversation_id)
                        .map(|conversation| conversation.root_task_exchanges())
                })
            })
            .flatten()
    }

    /// Returns a list of all conversations that have been cleared across all terminal surfaces.
    pub fn all_cleared_conversations(&self) -> Vec<(EntityId, &AIConversation)> {
        self.cleared_conversation_ids_for_terminal_surface
            .iter()
            .flat_map(|(terminal_surface_id, conversation_ids)| {
                conversation_ids.iter().filter_map(|conversation_id| {
                    self.conversations_by_id
                        .get(conversation_id)
                        .map(|conversation| (*terminal_surface_id, conversation))
                })
            })
            .collect::<Vec<_>>()
    }

    /// Returns all live conversations paired with their terminal surface IDs.
    /// This includes terminal surfaces that have been closed.
    pub fn all_live_conversations(&self) -> Vec<(EntityId, &AIConversation)> {
        self.live_conversation_ids_for_terminal_surface
            .iter()
            .flat_map(|(terminal_surface_id, conversation_ids)| {
                conversation_ids.iter().filter_map(|conversation_id| {
                    self.conversations_by_id
                        .get(conversation_id)
                        .map(|conversation| (*terminal_surface_id, conversation))
                })
            })
            .collect::<Vec<_>>()
    }

    /// Returns a conversation by ID by reading from memory. The conversation may not be available if:
    /// * The ID is invalid
    /// * The conversation has never been read into memory from db. Use load_conversation_from_db to handle reading from db.
    pub fn conversation(&self, conversation_id: &AIConversationId) -> Option<&AIConversation> {
        self.conversations_by_id.get(conversation_id)
    }

    pub fn mark_conversation_has_imported_comments(&mut self, id: AIConversationId) {
        self.conversations_with_imported_comments.insert(id);
    }

    pub fn conversation_has_imported_comments(&self, id: &AIConversationId) -> bool {
        self.conversations_with_imported_comments.contains(id)
    }

    pub fn conversation_mut(
        &mut self,
        conversation_id: &AIConversationId,
    ) -> Option<&mut AIConversation> {
        self.conversations_by_id.get_mut(conversation_id)
    }

    /// Returns all child conversations whose `parent_conversation_id` matches
    /// the given parent ID, using the `children_by_parent` index.
    pub fn child_conversations_of(&self, parent_id: AIConversationId) -> Vec<&AIConversation> {
        self.child_conversation_ids_of(&parent_id)
            .iter()
            .filter_map(|id| self.conversations_by_id.get(id))
            .collect()
    }

    fn resolved_parent_conversation_id_from_refs(
        &self,
        parent_conversation_id: Option<AIConversationId>,
        parent_agent_id: Option<&str>,
    ) -> Option<AIConversationId> {
        parent_conversation_id.or_else(|| {
            parent_agent_id.and_then(|agent_id| self.conversation_id_for_agent_id(agent_id))
        })
    }

    fn resolved_parent_conversation_id_from_persisted_data(
        &self,
        conversation_data: &AgentConversationData,
    ) -> Option<AIConversationId> {
        let parent_conversation_id = conversation_data
            .parent_conversation_id
            .as_deref()
            .and_then(|id| AIConversationId::try_from(id.to_owned()).ok());
        self.resolved_parent_conversation_id_from_refs(
            parent_conversation_id,
            conversation_data.parent_agent_id.as_deref(),
        )
    }

    pub fn resolved_parent_conversation_id_for_conversation(
        &self,
        conversation: &AIConversation,
    ) -> Option<AIConversationId> {
        self.resolved_parent_conversation_id_from_refs(
            conversation.parent_conversation_id(),
            conversation.parent_agent_id(),
        )
    }

    fn index_child_conversation(
        &mut self,
        child_id: AIConversationId,
        parent_id: AIConversationId,
    ) {
        let children = self.children_by_parent.entry(parent_id).or_default();
        if !children.contains(&child_id) {
            children.push(child_id);
        }
    }

    /// Creates a new child agent conversation.
    pub fn start_new_child_conversation(
        &mut self,
        terminal_surface_id: EntityId,
        name: String,
        parent_conversation_id: AIConversationId,
        orchestration_harness: Option<Harness>,
        ctx: &mut ModelContext<Self>,
    ) -> AIConversationId {
        let parent_agent_id = self
            .conversation(&parent_conversation_id)
            .and_then(|c| c.orchestration_agent_id());
        if parent_agent_id.is_none() {
            log::warn!(
                "No agent identifier for parent conversation {parent_conversation_id:?}; \
                 child agent will not be linked to parent on the server."
            );
        }

        let auto_execute = true; // Child auto-executes by default.
        let conversation_id =
            self.start_new_conversation(terminal_surface_id, auto_execute, false, false, ctx);
        {
            let conversation = self
                .conversation_mut(&conversation_id)
                .expect("Child conversation exists — was just created.");
            if let Some(id) = parent_agent_id {
                conversation.set_parent_agent_id(id);
            }
            conversation.set_agent_name(name);
            if let Some(harness) = orchestration_harness {
                conversation.set_orchestration_harness(harness);
            }
        }
        self.set_parent_for_conversation(conversation_id, parent_conversation_id);
        self.persist_conversation_state(conversation_id, ctx);
        conversation_id
    }

    /// Sets the parent conversation ID on a child conversation and updates
    /// the `children_by_parent` index.  All parent-child relationships should
    /// be established through this method so the index stays in sync.
    pub fn set_parent_for_conversation(
        &mut self,
        child_id: AIConversationId,
        parent_id: AIConversationId,
    ) {
        if let Some(conversation) = self.conversations_by_id.get_mut(&child_id) {
            conversation.set_parent_conversation_id(parent_id);
        }
        self.index_child_conversation(child_id, parent_id);
    }

    /// Returns the child conversation IDs for a parent from the startup index.
    /// Unlike `child_conversations_of`, this works before children are loaded
    /// into `conversations_by_id`.
    pub fn child_conversation_ids_of(&self, parent_id: &AIConversationId) -> &[AIConversationId] {
        self.children_by_parent
            .get(parent_id)
            .map(|v| v.as_slice())
            .unwrap_or_default()
    }

    fn persist_conversation_state(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(conversation) = self.conversations_by_id.get_mut(&conversation_id) else {
            return;
        };
        conversation.write_updated_conversation_state(ctx);
    }

    fn update_cached_metadata_for_conversation(&mut self, conversation_id: AIConversationId) {
        let Some(conversation) = self.conversations_by_id.get(&conversation_id) else {
            return;
        };
        let Some(metadata) = self.all_conversations_metadata.get_mut(&conversation_id) else {
            return;
        };

        metadata.server_conversation_token = conversation.server_conversation_token().cloned();
        if metadata.server_conversation_token.is_some() {
            metadata.has_cloud_data = true;
        }
        if let Some(server_metadata) = conversation.server_metadata() {
            metadata.server_conversation_metadata = Some(server_metadata.clone());
            metadata.has_cloud_data = true;
        }
    }

    /// Starts an optimistic local rename and records rollback state.
    pub(crate) fn begin_conversation_rename(
        &mut self,
        conversation_id: AIConversationId,
        title: String,
        ctx: &mut ModelContext<Self>,
    ) -> Result<String, BeginConversationRenameError> {
        if self
            .in_flight_conversation_renames
            .contains_key(&conversation_id)
        {
            return Err(BeginConversationRenameError::RenameInProgress);
        }

        let conversation = self
            .conversations_by_id
            .get(&conversation_id)
            .ok_or(BeginConversationRenameError::ConversationNotFound)?;
        let server_conversation_token = conversation
            .server_conversation_token()
            .ok_or(BeginConversationRenameError::MissingServerConversationToken)?
            .as_str()
            .to_owned();
        let root_task = conversation
            .get_root_task()
            .ok_or(BeginConversationRenameError::ConversationNotReady)?;
        if root_task.source().is_none() {
            return Err(BeginConversationRenameError::ConversationNotReady);
        }
        let previous_root_task_description = root_task.description().to_owned();
        let previous_server_metadata_title = conversation
            .server_metadata()
            .map(|metadata| metadata.title.clone());
        let previous_cached_metadata_title = self
            .all_conversations_metadata
            .get(&conversation_id)
            .map(|metadata| metadata.title.clone());

        self.in_flight_conversation_renames.insert(
            conversation_id,
            InFlightConversationRename {
                attempted_title: title.clone(),
                previous_root_task_description,
                previous_server_metadata_title,
                previous_cached_metadata_title,
            },
        );
        self.apply_conversation_title(conversation_id, title, ctx);
        Ok(server_conversation_token)
    }

    /// Completes an in-flight rename and applies any server-normalized title.
    pub(crate) fn complete_conversation_rename(
        &mut self,
        conversation_id: AIConversationId,
        title: String,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(rename) = self.in_flight_conversation_renames.remove(&conversation_id) else {
            log::warn!(
                "complete_conversation_rename called for conversation {conversation_id:?} with no in-flight rename"
            );
            return;
        };

        if rename.attempted_title != title {
            self.apply_conversation_title(conversation_id, title, ctx);
        }
    }

    /// Reverts an in-flight rename to the captured previous title snapshot.
    pub(crate) fn fail_conversation_rename(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(rename) = self.in_flight_conversation_renames.remove(&conversation_id) else {
            log::warn!(
                "fail_conversation_rename called for conversation {conversation_id:?} with no in-flight rename"
            );
            return;
        };

        let terminal_surface_id = self.terminal_surface_id_for_conversation(&conversation_id);

        let mut updated = false;
        if let Some(conversation) = self.conversations_by_id.get_mut(&conversation_id) {
            conversation.restore_conversation_title(
                rename.previous_root_task_description,
                rename.previous_server_metadata_title,
                ctx,
            );
            updated = true;
        } else {
            log::warn!(
                "fail_conversation_rename called for missing conversation {conversation_id:?}"
            );
        }

        let title = if let Some(previous_title) = rename.previous_cached_metadata_title {
            if let Some(metadata) = self.all_conversations_metadata.get_mut(&conversation_id) {
                metadata.title = previous_title.clone();
                if let Some(server_metadata) = metadata.server_conversation_metadata.as_mut() {
                    server_metadata.title = previous_title.clone();
                }
                updated = true;
            }
            previous_title
        } else {
            self.conversations_by_id
                .get(&conversation_id)
                .and_then(AIConversation::title)
                .unwrap_or_default()
        };

        if !updated {
            return;
        }

        ctx.emit(BlocklistAIHistoryEvent::UpdatedConversationTitle {
            terminal_surface_id,
            conversation_id,
            title,
        });
    }

    /// Applies a conversation title locally and notifies title observers.
    pub(crate) fn apply_conversation_title(
        &mut self,
        conversation_id: AIConversationId,
        title: String,
        ctx: &mut ModelContext<Self>,
    ) {
        let terminal_surface_id = self.terminal_surface_id_for_conversation(&conversation_id);

        let mut updated = false;
        if let Some(conversation) = self.conversations_by_id.get_mut(&conversation_id) {
            conversation.update_conversation_title(title.clone(), ctx);
            updated = true;
        } else {
            log::warn!(
                "apply_conversation_title called for missing conversation {conversation_id:?}"
            );
        }

        if let Some(metadata) = self.all_conversations_metadata.get_mut(&conversation_id) {
            metadata.title = title.clone();
            if let Some(server_metadata) = metadata.server_conversation_metadata.as_mut() {
                server_metadata.title = title.clone();
            }
            updated = true;
        }

        if !updated {
            return;
        }

        ctx.emit(BlocklistAIHistoryEvent::UpdatedConversationTitle {
            terminal_surface_id,
            conversation_id,
            title,
        });
    }
    pub fn mark_conversation_as_remote_child(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        {
            let Some(conversation) = self.conversations_by_id.get_mut(&conversation_id) else {
                return;
            };
            conversation.mark_as_remote_child();
        }
        self.persist_conversation_state(conversation_id, ctx);
    }

    /// Updates the persisted `last_event_sequence` for a conversation and
    /// writes the updated conversation state to SQLite. Used by the
    /// orchestration event poller after draining an event batch to keep the
    /// cursor durable across restarts.
    pub fn update_event_sequence(
        &mut self,
        conversation_id: AIConversationId,
        sequence: i64,
        ctx: &mut ModelContext<Self>,
    ) {
        {
            let Some(conversation) = self.conversations_by_id.get_mut(&conversation_id) else {
                return;
            };
            conversation.set_last_event_sequence(sequence);
        }
        self.persist_conversation_state(conversation_id, ctx);
    }

    /// Updates the persisted `pinned` state for a conversation and writes
    /// the change to SQLite. Used by the orchestration pin singleton to
    /// keep the per-conversation source of truth in sync with toggles.
    pub fn set_conversation_pinned(
        &mut self,
        conversation_id: AIConversationId,
        pinned: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(conversation) = self.conversations_by_id.get_mut(&conversation_id) else {
            log::warn!(
                "set_conversation_pinned called for conversation {conversation_id:?} that is \
                 not loaded; pin state change to {pinned} will not be persisted."
            );
            return;
        };
        if conversation.is_pinned() == pinned {
            return;
        }
        conversation.set_pinned(pinned);
        conversation.write_updated_conversation_state(ctx);
    }

    /// Sets a live conversation's server token, updates the reverse index, and
    /// synchronizes any cached metadata entry for the same conversation.
    ///
    /// Returns whether the token changed.
    pub fn set_server_conversation_token_for_conversation(
        &mut self,
        conversation_id: AIConversationId,
        token: String,
    ) -> bool {
        let new_token = ServerConversationToken::new(token.clone());
        {
            let Some(conversation) = self.conversations_by_id.get_mut(&conversation_id) else {
                return false;
            };

            let old_token = conversation.server_conversation_token().cloned();
            if old_token.as_ref() == Some(&new_token) {
                return false;
            }

            // Drop the old entry only if it still points at the given
            // conversation_id, so we don't wrongly remove an entry that's
            // been remapped.
            if let Some(old_token) = old_token {
                if let Entry::Occupied(entry) =
                    self.server_token_to_conversation_id.entry(old_token)
                {
                    if *entry.get() == conversation_id {
                        entry.remove();
                    }
                }
            }

            conversation.set_server_conversation_token(token);
        }

        self.server_token_to_conversation_id
            .insert(new_token, conversation_id);
        self.update_cached_metadata_for_conversation(conversation_id);
        true
    }

    /// Sets a live conversation's server token, updates the reverse index,
    /// synchronizes cached metadata, persists the rebound token to SQLite,
    /// and emits refresh events for live consumers.
    pub fn set_server_conversation_token_for_conversation_and_persist(
        &mut self,
        conversation_id: AIConversationId,
        token: String,
        ctx: &mut ModelContext<Self>,
    ) {
        if !self.set_server_conversation_token_for_conversation(conversation_id, token) {
            return;
        }
        self.persist_conversation_state(conversation_id, ctx);
        let terminal_surface_id = self.terminal_surface_id_for_conversation(&conversation_id);
        ctx.emit(BlocklistAIHistoryEvent::UpdatedConversationMetadata {
            terminal_surface_id,
            conversation_id,
        });
        if let Some(terminal_surface_id) = terminal_surface_id {
            ctx.emit(BlocklistAIHistoryEvent::ConversationServerTokenAssigned {
                conversation_id,
                terminal_surface_id,
            });
        }
    }

    /// Sets server metadata for a conversation and emits the ConversationMetadataUpdated event.
    /// This helper ensures we don't forget to emit the event when updating metadata.
    /// Updates in-memory conversations, or historical metadata if the conversation isn't loaded.
    pub fn set_server_metadata_for_conversation(
        &mut self,
        conversation_id: AIConversationId,
        metadata: ServerAIConversationMetadata,
        ctx: &mut ModelContext<Self>,
    ) {
        let terminal_surface_id;

        // Update in-memory conversation if it exists
        if let Some(conversation) = self.conversations_by_id.get_mut(&conversation_id) {
            conversation.set_server_metadata(metadata);
            terminal_surface_id = self.terminal_surface_id_for_conversation(&conversation_id);
        } else if let Some(conversation_metadata) =
            self.all_conversations_metadata.get_mut(&conversation_id)
        {
            // Conversation not in memory - update historical metadata instead
            // This is needed because we might update permissions from share dialog in
            // conversation list view when we only have metadata.
            conversation_metadata.server_conversation_metadata = Some(metadata);
            terminal_surface_id = None;
        } else {
            // Conversation not found anywhere
            return;
        }

        // Emit event so sharing dialog and other listeners can refresh.
        ctx.emit(BlocklistAIHistoryEvent::UpdatedConversationMetadata {
            terminal_surface_id,
            conversation_id,
        });
    }

    /// Returns the ID of the conversation that processed or is processing the response stream.
    ///
    /// A given response stream may only correspond to a single conversation at any given time,
    /// though the conversation to which it corresponds may change if a new conversation is started
    /// in the middle of the response, as is the case when the new conversation suggestion is
    /// accepted.
    pub fn conversation_for_response_stream(
        &self,
        response_stream_id: &ResponseStreamId,
    ) -> Option<AIConversationId> {
        self.conversations_by_id
            .iter()
            .find_map(|(conversation_id, conversation)| {
                if conversation.is_processing_response_stream(response_stream_id) {
                    Some(*conversation_id)
                } else {
                    None
                }
            })
    }

    pub fn conversation_status(
        &self,
        conversation_id: &AIConversationId,
    ) -> Option<&ConversationStatus> {
        self.conversation(conversation_id)
            .map(|conversation| conversation.status())
    }

    /// Returns the render status of one todo item in the conversation's todo
    /// history (see [`AIConversation::todo_status`]) — a narrow projection
    /// for consumers that don't need the whole `AIConversation`.
    pub fn todo_status(
        &self,
        conversation_id: &AIConversationId,
        todo_id: &AIAgentTodoId,
    ) -> Option<TodoStatus> {
        self.conversation(conversation_id)?.todo_status(todo_id)
    }

    /// Returns the conversation's active (most recent) todo list, if any — a
    /// narrow projection (see [`Self::todo_status`]).
    pub fn active_todo_list(&self, conversation_id: &AIConversationId) -> Option<&AIAgentTodoList> {
        self.conversation(conversation_id)?.active_todo_list()
    }

    /// Returns the terminal surface ID for the given conversation, if any.
    pub fn terminal_surface_id_for_conversation(
        &self,
        conversation_id: &AIConversationId,
    ) -> Option<EntityId> {
        self.live_conversation_ids_for_terminal_surface
            .iter()
            .find(|(_, conversation_ids)| conversation_ids.contains(conversation_id))
            .map(|(terminal_surface_id, _)| *terminal_surface_id)
    }

    /// Returns the conversation ID from the terminal surface's history corresponding to the action, if any.
    pub fn conversation_id_for_action(
        &self,
        action_id: &AIAgentActionId,
        terminal_surface_id: EntityId,
    ) -> Option<AIConversationId> {
        self.live_conversation_ids_for_terminal_surface
            .get(&terminal_surface_id)?
            .iter()
            .rev()
            .find(|conversation_id| {
                self.conversations_by_id
                    .get(conversation_id)
                    .is_some_and(|conversation| conversation.contains_action(action_id))
            })
            .copied()
    }

    pub fn existing_suggestions_for_conversation(
        &self,
        conversation_id: AIConversationId,
    ) -> Option<&Suggestions> {
        self.conversations_by_id
            .get(&conversation_id)
            .and_then(|c| c.existing_suggestions())
    }

    /// The active conversation is the one we're currently or have most recently streamed outputs for.
    /// If you want to get the conversation the next query will follow up in / what is selected in the input selector,
    /// use `context_model.selected_conversation` instead.
    pub fn active_conversation(&self, terminal_surface_id: EntityId) -> Option<&AIConversation> {
        self.active_conversation_id(terminal_surface_id)
            .and_then(|id| self.conversation(&id))
    }

    /// True if this conversation was started from a passive entrypoint, AND the user has made no follow ups.
    pub fn is_entirely_passive_conversation(&self, conversation_id: &AIConversationId) -> bool {
        self.conversation(conversation_id)
            .is_some_and(|conversation| conversation.is_entirely_passive())
    }

    pub fn is_exchange_hidden(
        &self,
        conversation_id: AIConversationId,
        exchange_id: AIAgentExchangeId,
    ) -> bool {
        self.conversations_by_id
            .get(&conversation_id)
            .is_some_and(|c| c.is_exchange_hidden(exchange_id))
    }

    /// Add a new [`AIAgentExchange`] to the [`AIConversation`] with the given [`AIConversationId`].
    /// Emits an event with the new exchange.
    pub(super) fn update_conversation_for_new_request_input(
        &mut self,
        request_input: RequestInput,
        stream_id: ResponseStreamId,
        terminal_surface_id: EntityId,
        ctx: &mut ModelContext<Self>,
    ) -> Result<(), UpdateHistoryError> {
        let conversation_id = request_input.conversation_id;
        let conversation = self
            .conversations_by_id
            .get_mut(&conversation_id)
            .ok_or(UpdateHistoryError::ConversationNotFound(conversation_id))?;

        // That first exchange is the synthetic orchestrator prompt (not user input)
        // which the NLD prompt history needs to exclude.
        let is_synthetic_orchestrator_prompt = conversation.is_child_agent_conversation()
            && conversation.root_task_exchanges().next().is_none();

        // Capture the new user query (text + submission time) before `request_input` is consumed.
        let new_prompt = request_input
            .all_inputs()
            .find_map(AIAgentInput::user_query)
            .map(|text| (text, request_input.request_start_ts));

        conversation.update_for_new_request_input(
            request_input,
            stream_id,
            terminal_surface_id,
            ctx,
        )?;

        // Append the new user query to the session NLD prompt history so input classification can
        // match it. Skip shared ambient agent sessions and the synthetic orchestrator prompt.
        if !is_synthetic_orchestrator_prompt
            && !self
                .ambient_agent_terminal_surface_ids
                .contains(&terminal_surface_id)
        {
            if let Some((text, start_ts)) = new_prompt {
                self.append_session_prompt(text, start_ts);
            }
        }
        Ok(())
    }

    pub fn restore_conversations(
        &mut self,
        terminal_surface_id: EntityId,
        conversations: Vec<AIConversation>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.terminal_surface_created_at
            .insert(terminal_surface_id, Local::now());

        let mut conversation_ids = Vec::new();
        for conversation in conversations.into_iter() {
            let conversation_id = conversation.id();
            conversation_ids.push(conversation_id);
            let live_conversation_ids = self
                .live_conversation_ids_for_terminal_surface
                .entry(terminal_surface_id)
                .or_default();
            if !live_conversation_ids.contains(&conversation_id) {
                live_conversation_ids.push(conversation_id);
            }

            if let Some(key) = agent_id_key(&conversation) {
                self.agent_id_to_conversation_id
                    .insert(key, conversation_id);
            }

            if let Some(token) = conversation.server_conversation_token() {
                self.server_token_to_conversation_id
                    .insert(token.clone(), conversation_id);
            }

            // Maintain the parent→child index for child agent conversations.
            if let Some(parent_id) =
                self.resolved_parent_conversation_id_for_conversation(&conversation)
            {
                self.index_child_conversation(conversation_id, parent_id);
            }

            let new_status = conversation.status().clone();
            self.conversations_by_id
                .insert(conversation_id, conversation);

            // Emit UpdatedConversationStatus for restored conversations so that
            // the workspace can set tab indicators appropriately
            ctx.emit(BlocklistAIHistoryEvent::UpdatedConversationStatus {
                conversation_id,
                terminal_surface_id,
                update: ConversationStatusUpdate::Restored,
                new_status,
            });
        }

        // Emit event so consumers can populate their associated view references.
        ctx.emit(BlocklistAIHistoryEvent::RestoredConversations {
            terminal_surface_id,
            conversation_ids,
        });
    }

    /// Sets the active conversation ID for a terminal surface and moves the conversation
    /// from any other terminal surface that currently contains it.
    ///
    /// For automatic follow-ups and request-stream bookkeeping, use
    /// [`Self::mark_active_conversation_id`] instead.
    pub fn set_active_conversation_id(
        &mut self,
        conversation_id: AIConversationId,
        terminal_surface_id: EntityId,
        ctx: &mut ModelContext<Self>,
    ) {
        if !self
            .live_conversation_ids_for_terminal_surface
            .get(&terminal_surface_id)
            .is_some_and(|conversation_ids| conversation_ids.contains(&conversation_id))
        {
            report_error!(
                "Attempted to set active conversation ID for a terminal surface that does not contain that conversation."
            );
            return;
        }

        // Track previous terminal surfaces we removed the conversation from so we can
        // emit terminal-surface-transfer events outside of the borrow of
        // `live_conversation_ids_for_terminal_surface`. The conversation rendering
        // model assumes a single canonical terminal surface per conversation, so each
        // previous terminal surface needs a chance to drop its now-stale rendered AI
        // blocks.
        let mut previous_terminal_surfaces: Vec<EntityId> = Vec::new();
        for (other_terminal_surface, other_terminal_surface_live_conversation_ids) in self
            .live_conversation_ids_for_terminal_surface
            .iter_mut()
            .filter(|(other_terminal_surface_id, _)| {
                **other_terminal_surface_id != terminal_surface_id
            })
        {
            let previous_len = other_terminal_surface_live_conversation_ids.len();
            other_terminal_surface_live_conversation_ids.retain(|id| *id != conversation_id);
            if other_terminal_surface_live_conversation_ids.len() != previous_len {
                previous_terminal_surfaces.push(*other_terminal_surface);
            }

            if self
                .active_conversation_for_terminal_surface
                .get(other_terminal_surface)
                .is_some_and(|id| *id == conversation_id)
            {
                self.active_conversation_for_terminal_surface
                    .remove(other_terminal_surface);
                ctx.emit(BlocklistAIHistoryEvent::ClearedActiveConversation {
                    conversation_id,
                    terminal_surface_id: *other_terminal_surface,
                });
            }
        }
        for previous_terminal_surface_id in previous_terminal_surfaces {
            ctx.emit(
                BlocklistAIHistoryEvent::ConversationTransferredBetweenTerminalSurfaces {
                    conversation_id,
                    previous_terminal_surface_id,
                    new_terminal_surface_id: terminal_surface_id,
                },
            );
        }

        self.active_conversation_for_terminal_surface
            .insert(terminal_surface_id, conversation_id);

        ctx.emit(BlocklistAIHistoryEvent::SetActiveConversation {
            conversation_id,
            terminal_surface_id,
        });
    }

    /// Marks a conversation as active for a terminal surface without removing it from other terminal surfaces.
    ///
    /// This is the non-transferring counterpart to [`Self::set_active_conversation_id`].
    /// Use this during automatic follow-ups and request sending where the
    /// conversation already belongs to this terminal surface and we only need to update
    /// the "most recently streamed" pointer.
    pub fn mark_active_conversation_id(
        &mut self,
        conversation_id: AIConversationId,
        terminal_surface_id: EntityId,
        ctx: &mut ModelContext<Self>,
    ) {
        if !self
            .live_conversation_ids_for_terminal_surface
            .get(&terminal_surface_id)
            .is_some_and(|conversation_ids| conversation_ids.contains(&conversation_id))
        {
            log::warn!(
                "mark_active_conversation_id: conversation {conversation_id:?} is not in \
                 terminal surface {terminal_surface_id:?} live list, skipping"
            );
            return;
        }

        self.active_conversation_for_terminal_surface
            .insert(terminal_surface_id, conversation_id);

        ctx.emit(BlocklistAIHistoryEvent::SetActiveConversation {
            conversation_id,
            terminal_surface_id,
        });
    }

    /// Starts a new conversation in the given terminal surface's history, effectively marking the
    /// existing conversation (if any) as completed.
    ///
    /// Returns the ID of the created conversation.
    ///
    /// Conversation completion is inferred if the conversation in question is _not_ the last
    /// element in the `conversations` vector.
    pub fn start_new_conversation(
        &mut self,
        terminal_surface_id: EntityId,
        is_autoexecute_override: bool,
        is_viewing_shared_session: bool,
        is_cli_agent_transcript: bool,
        ctx: &mut ModelContext<Self>,
    ) -> AIConversationId {
        let mut new_conversation =
            AIConversation::new(is_viewing_shared_session, is_cli_agent_transcript);
        if is_autoexecute_override {
            new_conversation.toggle_autoexecute_override();
        }
        let new_conversation_id = new_conversation.id();
        self.live_conversation_ids_for_terminal_surface
            .entry(terminal_surface_id)
            .or_default()
            .push(new_conversation_id);
        self.conversations_by_id
            .insert(new_conversation_id, new_conversation);

        ctx.emit(BlocklistAIHistoryEvent::StartedNewConversation {
            new_conversation_id,
            terminal_surface_id,
        });

        new_conversation_id
    }

    pub fn create_cli_subagent_task_for_conversation(
        &mut self,
        block_id: BlockId,
        conversation_id: AIConversationId,
        terminal_surface_id: EntityId,
        ctx: &mut ModelContext<Self>,
    ) -> Result<TaskId, UpdateHistoryError> {
        let conversation = self
            .conversations_by_id
            .get_mut(&conversation_id)
            .ok_or(UpdateHistoryError::ConversationNotFound(conversation_id))?;
        Ok(conversation.create_optimistic_cli_subagent_task(&block_id, terminal_surface_id, ctx))
    }

    pub fn update_conversation_status(
        &mut self,
        terminal_surface_id: EntityId,
        conversation_id: AIConversationId,
        status: ConversationStatus,
        ctx: &mut ModelContext<Self>,
    ) {
        self.update_conversation_status_with_error(
            terminal_surface_id,
            conversation_id,
            status,
            None,
            ctx,
        );
    }

    pub fn update_conversation_status_with_error(
        &mut self,
        terminal_surface_id: EntityId,
        conversation_id: AIConversationId,
        status: ConversationStatus,
        error: Option<RenderableAIError>,
        ctx: &mut ModelContext<Self>,
    ) {
        if let Some(conversation) = self.conversations_by_id.get_mut(&conversation_id) {
            conversation.update_status_with_error(status, error, terminal_surface_id, ctx);
        }
    }

    pub fn on_forked_conversation(
        &mut self,
        conversation_id: AIConversationId,
        terminal_surface_id: EntityId,
        ctx: &mut ModelContext<Self>,
    ) {
        // When a conversation is forked and restored for a new terminal surface,
        // we want to emit UpdatedStreamingExchange events for every exchange
        // to ensure that all of the existing exchanges are persisted correctly.
        if let Some(conversation) = self.conversations_by_id.get(&conversation_id) {
            for exchange in conversation.all_exchanges().into_iter() {
                let is_hidden = conversation.is_exchange_hidden(exchange.id);
                ctx.emit(BlocklistAIHistoryEvent::UpdatedStreamingExchange {
                    exchange_id: exchange.id,
                    terminal_surface_id,
                    conversation_id,
                    is_hidden,
                });
            }
        }
    }

    pub fn initialize_output_for_response_stream(
        &mut self,
        stream_id: &ResponseStreamId,
        conversation_id: AIConversationId,
        terminal_surface_id: EntityId,
        init_event: warp_multi_agent_api::response_event::StreamInit,
        ctx: &mut ModelContext<Self>,
    ) {
        let mut should_emit_server_token_assigned = false;
        let mut should_persist = false;
        if let Some(conversation) = self.conversations_by_id.get_mut(&conversation_id) {
            let had_token_before = conversation.server_conversation_token().is_some();

            if let Err(e) = conversation.initialize_output_for_response_stream(
                stream_id,
                init_event,
                terminal_surface_id,
                ctx,
            ) {
                log::warn!("Failed to update conversation with updated streamed output: {e}");
            }

            if let Some(key) = agent_id_key(conversation) {
                self.agent_id_to_conversation_id
                    .insert(key, conversation_id);
            }

            if let Some(token) = conversation.server_conversation_token() {
                self.server_token_to_conversation_id
                    .insert(token.clone(), conversation_id);
            }
            should_emit_server_token_assigned =
                !had_token_before && conversation.server_conversation_token().is_some();
            should_persist = true;
        }

        if should_persist {
            self.persist_conversation_state(conversation_id, ctx);
        }

        if should_emit_server_token_assigned {
            ctx.emit(BlocklistAIHistoryEvent::ConversationServerTokenAssigned {
                conversation_id,
                terminal_surface_id,
            });
        }
    }

    /// Assigns a `run_id` to a conversation that was spawned as a remote child
    /// agent. Updates the `agent_id_to_conversation_id` index and emits
    /// `ConversationServerTokenAssigned` so the `StartAgentExecutor` can
    /// complete the pending `start_agent` tool call.
    pub fn assign_run_id_for_conversation(
        &mut self,
        conversation_id: AIConversationId,
        run_id: String,
        task_id: Option<crate::ai::ambient_agents::AmbientAgentTaskId>,
        terminal_surface_id: EntityId,
        ctx: &mut ModelContext<Self>,
    ) {
        let (agent_key, server_token) = {
            let Some(conversation) = self.conversations_by_id.get_mut(&conversation_id) else {
                log::warn!(
                    "assign_run_id_for_conversation: conversation {conversation_id:?} not found"
                );
                return;
            };
            conversation.set_run_id(run_id);
            if let Some(task_id) = task_id {
                conversation.set_task_id(task_id);
            }
            (
                agent_id_key(conversation),
                conversation.server_conversation_token().cloned(),
            )
        };

        if let Some(key) = agent_key {
            self.agent_id_to_conversation_id
                .insert(key, conversation_id);
        }
        if let Some(token) = server_token {
            self.server_token_to_conversation_id
                .insert(token, conversation_id);
        }

        self.persist_conversation_state(conversation_id, ctx);
        ctx.emit(BlocklistAIHistoryEvent::ConversationServerTokenAssigned {
            conversation_id,
            terminal_surface_id,
        });
    }

    /// Resolves a server-side agent identifier to a local conversation ID.
    /// The identifier may be a server conversation token (v1) or a run_id (v2).
    pub fn conversation_id_for_agent_id(&self, agent_id: &str) -> Option<AIConversationId> {
        self.agent_id_to_conversation_id
            .get(agent_id)
            .copied()
            .or_else(|| {
                self.server_token_to_conversation_id
                    .get(&ServerConversationToken::new(agent_id.to_owned()))
                    .copied()
            })
    }

    /// Creates a new conversation and transfers relevant exchanges from
    /// the existing conversation to the new one. If successful, returns the new conversation id.
    fn handle_conversation_split(
        &mut self,
        old_conversation_id: AIConversationId,
        response_stream_id: &ResponseStreamId,
        start_from_message_id: MessageId,
        terminal_surface_id: EntityId,
        ctx: &mut ModelContext<Self>,
    ) -> Result<AIConversationId, UpdateHistoryError> {
        let exchange_ids_to_transfer: Vec<AIAgentExchangeId> = self
            .conversation(&old_conversation_id)
            .ok_or(UpdateHistoryError::ConversationNotFound(
                old_conversation_id,
            ))?
            .all_exchanges()
            .into_iter()
            .skip_while(|e| !e.added_message_ids.contains(&start_from_message_id))
            .map(|e| e.id)
            .collect();

        if exchange_ids_to_transfer.is_empty() {
            log::warn!("Starting a new conversation: message id not found");
            return Err(UpdateHistoryError::Conversation(
                UpdateConversationError::ExchangeNotFound,
            ));
        }
        log::info!(
            "Starting a new conversation: transferring {} exchanges to new conversation",
            exchange_ids_to_transfer.len()
        );

        let new_conversation_id =
            self.start_new_conversation(terminal_surface_id, false, false, false, ctx);
        for exchange_id in exchange_ids_to_transfer {
            let old_conversation = self
                .conversations_by_id
                .get_mut(&old_conversation_id)
                .ok_or(UpdateHistoryError::ConversationNotFound(
                    old_conversation_id,
                ))?;
            let exchange = old_conversation.remove_exchange(exchange_id)?;

            let new_conversation = self
                .conversations_by_id
                .get_mut(&new_conversation_id)
                .ok_or(UpdateHistoryError::ConversationNotFound(
                    new_conversation_id,
                ))?;
            new_conversation.append_reassigned_exchange(
                response_stream_id,
                exchange,
                terminal_surface_id,
                ctx,
            )?;
        }

        // Mark the old conversation as complete since we're starting a new one
        let old_conversation = self
            .conversations_by_id
            .get_mut(&old_conversation_id)
            .ok_or(UpdateHistoryError::ConversationNotFound(
                old_conversation_id,
            ))?;
        old_conversation.mark_completed_after_successful_split(
            response_stream_id,
            terminal_surface_id,
            ctx,
        )?;

        self.set_active_conversation_id(new_conversation_id, terminal_surface_id, ctx);

        ctx.emit(BlocklistAIHistoryEvent::SplitConversation {
            terminal_surface_id,
            old_conversation_id,
            new_conversation_id,
        });

        Ok(new_conversation_id)
    }

    /// Forks an existing conversation by creating a new conversation
    /// and copying the existing conversation's tasks into the new conversation.
    ///
    /// The `prefix` parameter specifies the prefix added to the root task description
    /// (e.g., `FORK_PREFIX` for forks, `PRE_REWIND_PREFIX` for pre-rewind backups).
    ///
    /// When `preserve_task_ids` is true, the forked conversation reuses the source's task ids
    /// instead of minting new ones. Used by local-to-cloud handoff so the local
    /// fork's task store matches the cloud-side fork. The cloud agent's
    /// `ClientAction`s reference those task ids; if we minted new ones locally
    /// they would fail to resolve.
    pub fn fork_conversation(
        &mut self,
        source_conversation: &AIConversation,
        prefix: &str,
        preserve_task_ids: bool,
        title_override: Option<&str>,
        app: &AppContext,
    ) -> Result<AIConversation, anyhow::Error> {
        let tasks: Vec<warp_multi_agent_api::Task> = source_conversation
            .all_tasks()
            .filter_map(|t| t.source().cloned())
            .collect();

        let updated_tasks_with_new_ids =
            update_forked_task_properties(tasks, prefix, preserve_task_ids, title_override);
        let Some(sqlite_sender) = GlobalResourceHandlesProvider::as_ref(app)
            .get()
            .model_event_sender
            .clone()
        else {
            return Err(anyhow!("No sqlite sender available."));
        };

        // We preserve reverted action IDs. Orphaned IDs (for actions not in fork) are harmless.
        // The reverted states are only copied to the new conversation if the revert happened before the user clicked fork,
        // but regardless of when the revert happened relative to the fork point.
        //
        // Example:
        // 1. Agent edit action
        // 2. Agent edit action
        // 3. User reverts edit from 1
        // 4. **User clicks fork**
        // 5. User reverts edit from 2
        //
        // In this example, the forked conversation will always show edit 1 as reverted and edit 2 as not reverted,
        // regardless of if the fork point is between 2 and 3 or 3 and 4. This is because we preserve all prior reverts,
        // either if they game before or after the fork point. However, once forked, we don't copy later reverts.
        let reverted_action_ids = if source_conversation.reverted_action_ids().is_empty() {
            None
        } else {
            Some(
                source_conversation
                    .reverted_action_ids()
                    .clone()
                    .into_iter()
                    .map_into()
                    .collect(),
            )
        };

        let conversation_data = AgentConversationData {
            server_conversation_token: None,
            conversation_usage_metadata: Some(source_conversation.usage_metadata()),
            reverted_action_ids,
            forked_from_server_conversation_token: source_conversation
                .server_conversation_token()
                .map(|t| t.as_str().to_string()),
            // We reset artifacts on fork
            artifacts_json: None,
            // Forked conversation loses its parentage
            parent_agent_id: None,
            agent_name: None,
            orchestration_harness_type: None,
            parent_conversation_id: None,
            is_remote_child: false,
            root_task_is_optimistic: None,
            run_id: None,
            autoexecute_override: Some(source_conversation.autoexecute_override().into()),
            last_event_sequence: None,
            pinned: false,
        };
        let forked_conversation_id = AIConversationId::new();
        if let Err(e) = sqlite_sender.send(ModelEvent::UpdateMultiAgentConversation {
            conversation_id: forked_conversation_id.to_string(),
            updated_tasks: updated_tasks_with_new_ids.clone(),
            conversation_data: conversation_data.clone(),
        }) {
            return Err(anyhow!("Failed to persist forked conversation: {e:?}."));
        }

        // Insert this conversation into the history model memory so we don't need to read from DB to restore this forked conversation
        // (otherwise, we can run into a race condition where the conversation is not found in the DB because we haven't finished writing to the db).
        let forked_conversation = self.insert_forked_conversation_from_tasks(
            forked_conversation_id,
            updated_tasks_with_new_ids.clone(),
            conversation_data.clone(),
        )?;

        Ok(forked_conversation)
    }

    /// Forks an existing conversation at a specific exchange boundary. When `exact_exchange`
    /// is true, the fork includes all messages up to and including the selected exchange.
    /// Otherwise, it extends through the full response (every message after the user's query
    /// until the next root-task user query).
    ///
    /// The `prefix` parameter specifies the prefix added to the root task description
    /// (e.g., `FORK_PREFIX` for forks, `PRE_REWIND_PREFIX` for pre-rewind backups).
    pub fn fork_conversation_at_exchange(
        &mut self,
        source_conversation: &AIConversation,
        from_exchange_id: AIAgentExchangeId,
        fork_from_exact_exchange: bool,
        prefix: &str,
        title_override: Option<&str>,
        app: &AppContext,
    ) -> Result<AIConversation, anyhow::Error> {
        let conversation = source_conversation;

        let exchanges_by_task: Vec<(TaskId, Vec<&AIAgentExchange>)> =
            conversation.all_exchanges_by_task();

        let root_task_id = conversation.get_root_task_id().clone();

        let mut message_ids_to_retain_by_task: HashMap<TaskId, HashSet<MessageId>> = HashMap::new();
        // Each task's last retained exchange. Retention keeps a prefix of each
        // task's exchanges, so only tool calls in this exchange can have been
        // severed from their results (which land in the next, dropped exchange).
        let mut fork_point_exchange_by_task: HashMap<TaskId, &AIAgentExchange> = HashMap::new();
        let mut found_from_exchange_id = false;
        'outer: for (task_id, task_exchanges) in exchanges_by_task.into_iter() {
            for exchange in task_exchanges {
                // In the non-exact case, we continue past the selected exchange until we reach
                // the next user query (effectively forking from the selected 'response').
                if found_from_exchange_id && task_id == root_task_id && exchange.has_user_query() {
                    break 'outer;
                }

                let message_ids_to_retain = message_ids_to_retain_by_task
                    .entry(task_id.clone())
                    .or_default();
                message_ids_to_retain.extend(exchange.added_message_ids.iter().cloned());
                fork_point_exchange_by_task.insert(task_id.clone(), exchange);
                if exchange.id == from_exchange_id {
                    if fork_from_exact_exchange {
                        break 'outer;
                    }
                    found_from_exchange_id = true;
                }
            }
        }

        if message_ids_to_retain_by_task.is_empty() {
            return Err(anyhow!(
                "No messages found for block in conversation {}.",
                conversation.id()
            ));
        }

        // Build truncated tasks by retaining only messages whose IDs are in
        // `allowed_message_ids`. Tasks whose message list becomes empty and
        // which are non-root tasks are dropped. Client `tool_call`s in the
        // fork-point exchange whose `tool_call_result` was truncated away are
        // reconciled so every `tool_use` stays paired (see
        // `reconcile_dangling_tool_calls_in_forked_task`).
        let truncated_tasks: Vec<warp_multi_agent_api::Task> = conversation
            .all_tasks()
            .filter_map(|t| {
                if let Some(message_ids_to_retain) = message_ids_to_retain_by_task.get(t.id()) {
                    let source_task = t.source()?;
                    let mut truncated_task = source_task.clone();
                    truncated_task
                        .messages
                        .retain(|m| message_ids_to_retain.contains(&MessageId::new(m.id.clone())));
                    if truncated_task.messages.is_empty() {
                        return None;
                    }
                    if let Some(fork_point_exchange) = fork_point_exchange_by_task.get(t.id()) {
                        reconcile_dangling_tool_calls_in_forked_task(
                            &mut truncated_task,
                            &source_task.messages,
                            &fork_point_exchange.added_message_ids,
                        );
                    }
                    Some(truncated_task)
                } else {
                    None
                }
            })
            .collect();

        if truncated_tasks.is_empty() {
            return Err(anyhow!(
                "Truncated tasks for forked conversation at block are empty for conversation {}.",
                conversation.id()
            ));
        }

        let updated_tasks_with_new_ids =
            update_forked_task_properties(truncated_tasks, prefix, false, title_override);

        let Some(sqlite_sender) = GlobalResourceHandlesProvider::as_ref(app)
            .get()
            .model_event_sender
            .clone()
        else {
            return Err(anyhow!("No sqlite sender available."));
        };

        // We preserve reverted action IDs. Orphaned IDs (for actions not in fork) are harmless.
        // The reverted states are only copied to the new conversation if the revert happened before the user clicked fork,
        // but regardless of when the revert happened relative to the fork point.
        //
        // Example:
        // 1. Agent edit action
        // 2. Agent edit action
        // 3. User reverts edit from 1
        // 4. **User clicks fork**
        // 5. User reverts edit from 2
        //
        // In this example, the forked conversation will always show edit 1 as reverted and edit 2 as not reverted,
        // regardless of if the fork point is between 2 and 3 or 3 and 4. This is because we preserve all prior reverts,
        // either if they game before or after the fork point. However, once forked, we don't copy later reverts.
        let reverted_action_ids = if conversation.reverted_action_ids().is_empty() {
            None
        } else {
            Some(
                conversation
                    .reverted_action_ids()
                    .clone()
                    .into_iter()
                    .map_into()
                    .collect(),
            )
        };

        // Start forked conversations without usage metadata for now; this can
        // be recomputed based on the retained exchanges in a follow-up.
        let conversation_data = AgentConversationData {
            server_conversation_token: None,
            conversation_usage_metadata: None,
            reverted_action_ids,
            forked_from_server_conversation_token: conversation
                .server_conversation_token()
                .map(|t| t.as_str().to_string()),
            // We reset artifacts on fork
            artifacts_json: None,
            // Forked conversation loses its parentage.
            parent_agent_id: None,
            agent_name: None,
            orchestration_harness_type: None,
            parent_conversation_id: None,
            is_remote_child: false,
            root_task_is_optimistic: None,
            run_id: None,
            autoexecute_override: Some(conversation.autoexecute_override().into()),
            last_event_sequence: None,
            pinned: false,
        };

        let forked_conversation_id = AIConversationId::new();
        if let Err(e) = sqlite_sender.send(ModelEvent::UpdateMultiAgentConversation {
            conversation_id: forked_conversation_id.to_string(),
            updated_tasks: updated_tasks_with_new_ids.clone(),
            conversation_data: conversation_data.clone(),
        }) {
            return Err(anyhow!(
                "Failed to persist forked conversation at block: {e:?}."
            ));
        }

        let forked_conversation = self.insert_forked_conversation_from_tasks(
            forked_conversation_id,
            updated_tasks_with_new_ids,
            conversation_data,
        )?;

        Ok(forked_conversation)
    }

    pub fn apply_client_actions(
        &mut self,
        response_stream_id: &ResponseStreamId,
        client_actions: Vec<warp_multi_agent_api::ClientAction>,
        conversation_id: AIConversationId,
        terminal_surface_id: EntityId,
        skill_path_origin: &SkillPathOrigin,
        ctx: &mut ModelContext<Self>,
    ) -> Result<(), UpdateHistoryError> {
        let mut current_conversation_id = conversation_id;
        for client_action in client_actions {
            match client_action.action {
                Some(Action::StartNewConversation(StartNewConversation {
                    start_from_message_id,
                })) => {
                    let new_conversation_id = self.handle_conversation_split(
                        current_conversation_id,
                        response_stream_id,
                        MessageId::new(start_from_message_id),
                        terminal_surface_id,
                        ctx,
                    )?;
                    current_conversation_id = new_conversation_id;
                }
                Some(action) => {
                    let conversation = self
                        .conversations_by_id
                        .get_mut(&current_conversation_id)
                        .ok_or(UpdateHistoryError::ConversationNotFound(
                        current_conversation_id,
                    ))?;
                    conversation.apply_client_action(
                        response_stream_id,
                        terminal_surface_id,
                        action,
                        skill_path_origin,
                        ctx,
                    )?;
                }
                None => {
                    log::warn!("Received empty client action");
                }
            }
        }
        Ok(())
    }

    pub fn update_conversation_cost_and_usage_for_request(
        &mut self,
        conversation_id: AIConversationId,
        request_cost: Option<RequestCost>,
        token_usage: Vec<TokenUsage>,
        usage_metadata: Option<ConversationUsageMetadata>,
        was_user_initiated_request: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        // Track whether this update changes any state derived by
        // `BlocklistAIHistoryEvent::ConversationUsageMetadataUpdated`
        // subscribers (e.g. the orchestration credit rollup or the TUI
        // footer's usage entry). We emit the event only when there's actual
        // data to react to.
        let emits_usage_event =
            request_cost.is_some() || usage_metadata.is_some() || !token_usage.is_empty();
        if let Some(conversation) = self.conversations_by_id.get_mut(&conversation_id) {
            if let Err(e) = conversation.update_cost_and_usage_for_request(
                request_cost,
                token_usage,
                usage_metadata,
                was_user_initiated_request,
                ctx,
            ) {
                log::warn!(
                    "Failed to update request cost for conversation {conversation_id}: {e:#}"
                );
            }
            if emits_usage_event {
                ctx.emit(BlocklistAIHistoryEvent::ConversationUsageMetadataUpdated {
                    conversation_id,
                });
            }
        } else {
            log::warn!(
                "Failed to update request cost because conversation {conversation_id} was not found"
            );
        }
    }

    pub fn mark_response_stream_completed_successfully(
        &mut self,
        stream_id: &ResponseStreamId,
        conversation_id: AIConversationId,
        terminal_surface_id: EntityId,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(conversation) = self.conversations_by_id.get_mut(&conversation_id) else {
            return;
        };
        if let Err(e) = conversation.mark_request_completed(stream_id, terminal_surface_id, ctx) {
            log::warn!("Failed to mark exchange as completed: {e}");
        }

        // If this conversation doesn't have server metadata yet, and it has a server conversation token,
        // fetch the metadata from the server.
        let should_fetch_metadata = FeatureFlag::CloudConversations.is_enabled()
            && conversation.server_metadata().is_none()
            && conversation.server_conversation_token().is_some();

        if should_fetch_metadata {
            let server_token = conversation
                .server_conversation_token()
                .unwrap()
                .as_str()
                .to_string();

            let server_api = ServerApiProvider::as_ref(ctx).get_ai_client();
            ctx.spawn(
                async move {
                    server_api
                        .list_ai_conversation_metadata(Some(vec![server_token]))
                        .await
                },
                move |model, result, ctx| match result {
                    Ok(mut metadata_list) if !metadata_list.is_empty() => {
                        if let Some(metadata) = metadata_list.pop() {
                            model.set_server_metadata_for_conversation(
                                conversation_id,
                                metadata,
                                ctx,
                            );
                        }
                    }
                    Ok(_) => {
                        log::warn!("No metadata returned for conversation {}", conversation_id);
                    }
                    Err(e) => {
                        log::warn!(
                            "Failed to fetch metadata for conversation {}: {e:#}",
                            conversation_id
                        );
                    }
                },
            );
        }
    }

    pub fn set_exchange_time_to_first_token(
        &mut self,
        conversation_id: AIConversationId,
        exchange_id: AIAgentExchangeId,
        time_to_first_token_ms: i64,
    ) {
        if let Some(conversation) = self.conversations_by_id.get_mut(&conversation_id) {
            if let Ok(exchange) = conversation.get_exchange_to_update(exchange_id) {
                exchange.time_to_first_token_ms = Some(time_to_first_token_ms);
            }
        }
    }

    pub fn mark_response_stream_cancelled(
        &mut self,
        stream_id: &ResponseStreamId,
        conversation_id: AIConversationId,
        terminal_surface_id: EntityId,
        reason: CancellationReason,
        ctx: &mut ModelContext<Self>,
    ) {
        if let Some(conversation) = self.conversations_by_id.get_mut(&conversation_id) {
            if reason.is_reverted() {
                if let Err(e) =
                    conversation.mark_request_cancelled_due_to_revert(terminal_surface_id, ctx)
                {
                    log::warn!("Failed to mark exchange as cancelled: {e}");
                }
            } else if let Err(e) =
                conversation.mark_request_cancelled(stream_id, terminal_surface_id, reason, ctx)
            {
                log::warn!("Failed to mark exchange as cancelled: {e}");
            }
        }
        AIDocumentModel::handle(ctx).update(ctx, |model, ctx| {
            model.clear_streaming_documents_for_conversation(&conversation_id, ctx);
        });
    }

    /// Marks the stream's exchanges as finished with `error`.
    pub fn mark_response_stream_completed_with_error(
        &mut self,
        error: RenderableAIError,
        recovery_pending: bool,
        stream_id: &ResponseStreamId,
        conversation_id: AIConversationId,
        terminal_surface_id: EntityId,
        ctx: &mut ModelContext<Self>,
    ) {
        if let Some(conversation) = self.conversations_by_id.get_mut(&conversation_id) {
            if let Err(e) = conversation.mark_request_completed_with_error(
                stream_id,
                error.clone(),
                recovery_pending,
                terminal_surface_id,
                ctx,
            ) {
                log::warn!("Failed to mark exchange as completed with error: {e}");
            }
        }
    }

    /// Handle clearing the blocklist for a terminal surface.
    /// The terminal surface will also cancel the active stream on processing the event emitted here.
    pub fn clear_conversations_for_terminal_surface(
        &mut self,
        terminal_surface_id: EntityId,
        ctx: &mut ModelContext<Self>,
    ) {
        // Cancel the active stream when we clear conversations for this terminal surface.
        let active_conversation_id = self
            .active_conversation_for_terminal_surface
            .remove(&terminal_surface_id);
        let mut cleared_conversation_ids: Vec<AIConversationId> = Vec::new();
        if let Some(ids) = self
            .live_conversation_ids_for_terminal_surface
            .remove(&terminal_surface_id)
        {
            cleared_conversation_ids.extend(ids.iter().copied());
            self.cleared_conversation_ids_for_terminal_surface
                .entry(terminal_surface_id)
                .and_modify(|existing| existing.extend(ids.clone()))
                .or_insert(ids);
        }
        ctx.emit(
            BlocklistAIHistoryEvent::ClearedConversationsForTerminalSurface {
                terminal_surface_id,
                active_conversation_id,
                cleared_conversation_ids,
            },
        );
    }

    /// Handle removing a conversation from the history model, blocklist and in-memory.
    pub fn remove_conversation(
        &mut self,
        conversation_id: AIConversationId,
        terminal_surface_id: EntityId,
        ctx: &mut ModelContext<Self>,
    ) {
        self.remove_conversation_from_memory(conversation_id, Some(terminal_surface_id), ctx);
    }

    /// Permanently delete a conversation.
    pub fn delete_conversation(
        &mut self,
        conversation_id: AIConversationId,
        terminal_surface_id: Option<EntityId>,
        ctx: &mut ModelContext<Self>,
    ) {
        let conversation_title = self
            .conversations_by_id
            .get(&conversation_id)
            .and_then(|c| c.title().map(|t| t.to_string()));
        // Capture the run_id BEFORE the in-memory record is dropped so it
        // can be forwarded on the DeletedConversation event.
        let run_id = self
            .conversations_by_id
            .get(&conversation_id)
            .and_then(|c| c.run_id());

        self.remove_conversation_from_memory(conversation_id, terminal_surface_id, ctx);

        // Delete persisted conversation from sqlite.
        let model_event_sender = GlobalResourceHandlesProvider::as_ref(ctx)
            .get()
            .model_event_sender
            .clone();
        let conversation_id_string = conversation_id.to_string();
        ctx.spawn(
            async move {
                if let Some(sender) = model_event_sender {
                    if let Err(e) = sender.send(ModelEvent::DeleteAIConversation {
                        conversation_id: conversation_id_string.clone(),
                    }) {
                        report_error!(anyhow::Error::new(e)
                            .context("Error sending DeleteAIConversation event"));
                    }
                    if let Err(e) = sender.send(ModelEvent::DeleteMultiAgentConversations {
                        conversation_ids: vec![conversation_id_string],
                    }) {
                        report_error!(anyhow::Error::new(e)
                            .context("Error sending DeleteMultiAgentConversations event"));
                    }
                }
            },
            |_, _, _| {},
        );

        // Only emit the event if we have a terminal_surface_id, since the event is
        // filtered by terminal_surface_id in handlers.
        if let Some(terminal_surface_id) = terminal_surface_id {
            ctx.emit(BlocklistAIHistoryEvent::DeletedConversation {
                terminal_surface_id,
                conversation_id,
                conversation_title,
                run_id,
            });
        }
    }

    /// Remove a conversation from all in-memory storage.
    fn remove_conversation_from_memory(
        &mut self,
        conversation_id: AIConversationId,
        terminal_surface_id: Option<EntityId>,
        ctx: &mut ModelContext<Self>,
    ) {
        // Capture the run_id BEFORE the in-memory record is dropped so the
        // RemoveConversation event can carry it (event subscribers can no
        // longer look it up via `conversation()` after this function returns).
        let run_id = self
            .conversations_by_id
            .get(&conversation_id)
            .and_then(|c| c.run_id());

        // Clean up reverse indices before removing the conversation. Guard
        // token-index removals with an equality check: the live conversation's
        // token and the metadata's token can diverge after a rebind, and we
        // must not clobber an entry already owned by another conversation.
        if let Some(conversation) = self.conversations_by_id.get(&conversation_id) {
            if let Some(key) = agent_id_key(conversation) {
                self.agent_id_to_conversation_id.remove(&key);
            }
            if let Some(token) = conversation.server_conversation_token() {
                if self.server_token_to_conversation_id.get(token) == Some(&conversation_id) {
                    self.server_token_to_conversation_id.remove(token);
                }
            }
        }
        // Also clean up the token index entry that might have been installed
        // via the metadata path (no live conversation present).
        if let Some(metadata) = self.all_conversations_metadata.get(&conversation_id) {
            if let Some(token) = &metadata.server_conversation_token {
                if self.server_token_to_conversation_id.get(token) == Some(&conversation_id) {
                    self.server_token_to_conversation_id.remove(token);
                }
            }
        }

        self.all_conversations_metadata.remove(&conversation_id);
        self.conversations_by_id.remove(&conversation_id);

        if let Some(terminal_surface_id) = terminal_surface_id {
            if self
                .active_conversation_for_terminal_surface
                .get(&terminal_surface_id)
                .is_some_and(|id| *id == conversation_id)
            {
                self.active_conversation_for_terminal_surface
                    .remove(&terminal_surface_id);
            }
            if let Some(vec) = self
                .live_conversation_ids_for_terminal_surface
                .get_mut(&terminal_surface_id)
            {
                vec.retain(|&id| id != conversation_id);
            }
            if let Some(vec) = self
                .cleared_conversation_ids_for_terminal_surface
                .get_mut(&terminal_surface_id)
            {
                vec.retain(|&id| id != conversation_id);
            }
            ctx.emit(BlocklistAIHistoryEvent::RemoveConversation {
                terminal_surface_id,
                conversation_id,
                run_id,
            });
        }
    }

    /// Returns true if the conversation is live for any terminal surface.
    pub fn is_conversation_live(&self, conversation_id: AIConversationId) -> bool {
        self.live_conversation_ids_for_terminal_surface
            .values()
            .any(|conversation_ids| conversation_ids.contains(&conversation_id))
    }

    pub fn mark_terminal_surface_as_ambient_agent_session_view(
        &mut self,
        terminal_surface_id: EntityId,
    ) {
        self.ambient_agent_terminal_surface_ids
            .insert(terminal_surface_id);
    }

    pub fn mark_terminal_surface_as_conversation_transcript_viewer(
        &mut self,
        terminal_surface_id: EntityId,
    ) {
        self.conversation_transcript_viewer_terminal_surface_ids
            .insert(terminal_surface_id);
    }

    pub fn is_terminal_surface_conversation_transcript_viewer(
        &self,
        terminal_surface_id: EntityId,
    ) -> bool {
        self.conversation_transcript_viewer_terminal_surface_ids
            .contains(&terminal_surface_id)
    }

    /// Returns [`AIQueryHistory`]s from all sources: live conversations, cleared conversations,
    /// and persisted queries from conversations not loaded in memory.
    ///
    /// When `terminal_surface_id` is provided, queries from that terminal surface are categorized as
    /// `CurrentSession` and all others as `DifferentSession`. When `None`, all queries are
    /// categorized as `DifferentSession`.
    ///
    /// Ambient agent sessions are always excluded.
    pub(crate) fn all_ai_queries(
        &self,
        terminal_surface_id: Option<EntityId>,
    ) -> impl Iterator<Item = AIQueryHistory> + '_ {
        // Collect all conversation IDs that are already in memory (live or cleared)
        // and build query vectors in the same loops
        let mut loaded_conversation_ids: HashSet<AIConversationId> = HashSet::new();

        let mut live_queries_vec = Vec::new();
        for (conversation_terminal_surface_id, conversation_ids) in
            self.live_conversation_ids_for_terminal_surface.iter()
        {
            loaded_conversation_ids.extend(conversation_ids);

            // Skip shared ambient agent sessions
            if self
                .ambient_agent_terminal_surface_ids
                .contains(conversation_terminal_surface_id)
            {
                continue;
            }

            let history_order =
                if terminal_surface_id.is_some_and(|id| id == *conversation_terminal_surface_id) {
                    HistoryOrder::CurrentSession
                } else {
                    HistoryOrder::DifferentSession
                };

            for conversation_id in conversation_ids {
                if let Some(conversation) = self.conversations_by_id.get(conversation_id) {
                    // For child agent conversations, skip the first exchange — it
                    // contains the synthetic orchestrator prompt, not user input.
                    // TODO(QUALITY-636): Replace positional skip with an
                    // `is_agent_initiated` field on the MAA UserQuery proto
                    // message so the flag survives server restoration.
                    let skip_count = if conversation.is_child_agent_conversation() {
                        1
                    } else {
                        0
                    };
                    for exchange in conversation.root_task_exchanges().skip(skip_count) {
                        if let Some(query) = ai_exchange_to_query_history(exchange, history_order) {
                            live_queries_vec.push(query);
                        }
                    }
                }
            }
        }

        let mut cleared_queries_vec = Vec::new();
        for (conversation_terminal_surface_id, conversation_ids) in
            self.cleared_conversation_ids_for_terminal_surface.iter()
        {
            loaded_conversation_ids.extend(conversation_ids);

            let history_order =
                if terminal_surface_id.is_some_and(|id| id == *conversation_terminal_surface_id) {
                    HistoryOrder::CurrentSession
                } else {
                    HistoryOrder::DifferentSession
                };

            for conversation_id in conversation_ids {
                if let Some(conversation) = self.conversations_by_id.get(conversation_id) {
                    let skip_count = if conversation.is_child_agent_conversation() {
                        1
                    } else {
                        0
                    };
                    for exchange in conversation.root_task_exchanges().skip(skip_count) {
                        if let Some(query) = ai_exchange_to_query_history(exchange, history_order) {
                            cleared_queries_vec.push(query);
                        }
                    }
                }
            }
        }

        // Add persisted queries from conversations not loaded in memory
        let persisted_queries_vec: Vec<_> = self
            .persisted_queries
            .iter()
            .filter(|persisted| !loaded_conversation_ids.contains(&persisted.conversation_id))
            .filter_map(|persisted| {
                persisted_ai_input_to_query_history(persisted, HistoryOrder::DifferentSession)
            })
            .collect();

        persisted_queries_vec
            .into_iter()
            .chain(cleared_queries_vec)
            .chain(live_queries_vec)
    }

    /// Appends a single user-query prompt to [`Self::prompt_history`], dropping whitespace-only
    /// prompts. Session prompts arrive in submission order, so pushing keeps the vec ascending.
    fn append_session_prompt(&mut self, text: String, start_ts: DateTime<Local>) {
        if text.trim().is_empty() {
            return;
        }
        self.prompt_history.push(PromptHistoryEntry {
            text: Arc::from(text),
            start_ts,
        });
    }

    /// Returns the prompt-history candidates for NLD input classification, oldest-first
    /// (ascending). The matcher reverses this to iterate newest-first.
    pub(crate) fn prompt_history_candidates(&self) -> Vec<PromptHistoryEntry> {
        self.prompt_history.clone()
    }

    /// Returns the active conversation ID for a terminal surface, if one exists.
    /// The active conversation is the one we're currently or have most recently streamed outputs for.
    /// If you want to check what conversation the next query will follow up in / what is selected in the input selector,
    /// use `context_model.selected_conversation_id` instead.
    pub(crate) fn active_conversation_id(
        &self,
        terminal_surface_id: EntityId,
    ) -> Option<AIConversationId> {
        let active_conversation_id = self
            .active_conversation_for_terminal_surface
            .get(&terminal_surface_id)
            .copied()?;

        let conversation_ids_for_terminal_surface = self
            .live_conversation_ids_for_terminal_surface
            .get(&terminal_surface_id)?;

        if !conversation_ids_for_terminal_surface.contains(&active_conversation_id) {
            log::warn!(
                "The active conversation ID {active_conversation_id:?} was not found in the list of conversation IDs for terminal surface {terminal_surface_id:?}. Conversation IDs: {conversation_ids_for_terminal_surface:?}"
            );
            return None;
        }

        Some(active_conversation_id)
    }

    /// Returns the last conversation ID created for a terminal surface, if one exists.
    #[cfg_attr(target_family = "wasm", allow(unused))]
    pub(crate) fn last_conversation_id(
        &self,
        terminal_surface_id: EntityId,
    ) -> Option<AIConversationId> {
        self.live_conversation_ids_for_terminal_surface
            .get(&terminal_surface_id)?
            .last()
            .copied()
    }

    /// Set the hidden status of the exchange with the given ID.
    pub fn set_exchange_hidden_status(
        &mut self,
        terminal_surface_id: EntityId,
        conversation_id: AIConversationId,
        exchange_id: AIAgentExchangeId,
        is_hidden: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(conversation) = self.conversations_by_id.get_mut(&conversation_id) else {
            return;
        };
        conversation.set_is_exchange_hidden(exchange_id, is_hidden, terminal_surface_id, ctx);
    }

    pub fn set_viewing_shared_session_for_conversation(
        &mut self,
        conversation_id: AIConversationId,
        is_viewing_shared_session: bool,
    ) {
        if let Some(conversation) = self.conversations_by_id.get_mut(&conversation_id) {
            conversation.set_is_viewing_shared_session(is_viewing_shared_session);
        }
    }

    pub fn set_has_code_review_opened_to_true(&mut self, conversation_id: AIConversationId) {
        if let Some(conversation) = self.conversations_by_id.get_mut(&conversation_id) {
            conversation.mark_code_review_as_opened();
        }
    }

    pub fn toggle_autoexecute_override(
        &mut self,
        conversation_id: &AIConversationId,
        terminal_surface_id: EntityId,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(conversation) = self.conversations_by_id.get_mut(conversation_id) else {
            return;
        };

        conversation.toggle_autoexecute_override();
        conversation.write_updated_conversation_state(ctx);
        ctx.emit(BlocklistAIHistoryEvent::UpdatedAutoexecuteOverride {
            terminal_surface_id,
        });
    }

    /// Truncates a conversation from the given exchange ID, removing all exchanges
    /// from that exchange onwards (inclusive). This is a lossy operation.
    ///
    /// Returns the set of exchange IDs that were removed.
    pub fn truncate_conversation_from_exchange(
        &mut self,
        conversation_id: AIConversationId,
        from_exchange_id: AIAgentExchangeId,
        ctx: &mut ModelContext<Self>,
    ) -> Result<HashSet<AIAgentExchangeId>, UpdateHistoryError> {
        let conversation = self
            .conversations_by_id
            .get_mut(&conversation_id)
            .ok_or(UpdateHistoryError::ConversationNotFound(conversation_id))?;

        let removed_exchange_ids = conversation.truncate_from_exchange(from_exchange_id, ctx)?;

        Ok(removed_exchange_ids)
    }

    /// Returns the latest exchange across all conversations for a terminal surface.
    /// This is useful for determining if a specific exchange is the most recent one.
    /// Excludes passive code generation exchanges from consideration.
    pub fn latest_exchange_across_all_conversations(
        &self,
        terminal_surface_id: EntityId,
    ) -> Option<&AIAgentExchange> {
        self.all_live_root_task_exchanges_for_terminal_surface(terminal_surface_id)
            .filter(|exchange| !exchange.has_passive_request())
            .max_by_key(|exchange| exchange.start_time)
    }

    /// Returns the conversation ID that contains the given exchange ID, if any.
    /// Searches through all conversations for a terminal surface.
    pub fn conversation_id_for_exchange(
        &self,
        exchange_id: AIAgentExchangeId,
        terminal_surface_id: EntityId,
    ) -> Option<AIConversationId> {
        self.live_conversation_ids_for_terminal_surface
            .get(&terminal_surface_id)?
            .iter()
            .find(|conversation_id| {
                self.conversations_by_id
                    .get(conversation_id)
                    .is_some_and(|conversation| {
                        conversation.exchange_with_id(exchange_id).is_some()
                    })
            })
            .copied()
    }

    /// Returns local conversation metadata,
    /// (excluding conversations from ambient agent runs).
    pub fn get_local_conversations_metadata(
        &self,
    ) -> impl Iterator<Item = &AIConversationMetadata> {
        self.all_conversations_metadata
            .values()
            .filter(|m| !m.is_ambient_agent_conversation())
    }

    /// Returns conversation metadata for a specific conversation ID.
    pub fn get_conversation_metadata(
        &self,
        conversation_id: &AIConversationId,
    ) -> Option<&AIConversationMetadata> {
        self.all_conversations_metadata.get(conversation_id)
    }

    /// Returns whether a conversation can be shared.
    ///
    /// A conversation can be shared if we have server metadata available
    /// (either from a loaded conversation or from conversation metadata).
    pub fn can_conversation_be_shared(&self, conversation_id: &AIConversationId) -> bool {
        self.get_server_conversation_metadata(conversation_id)
            .is_some()
    }

    /// Returns the server conversation metadata, used by the sharing dialog.
    ///
    /// This checks:
    /// 1. If the conversation is loaded in memory, returns from its server metadata
    /// 2. Otherwise, falls back to data stored in conversation metadata
    pub fn get_server_conversation_metadata(
        &self,
        conversation_id: &AIConversationId,
    ) -> Option<&ServerAIConversationMetadata> {
        // Check if conversation exists in memory and has server metadata
        if let Some(conversation) = self.conversation(conversation_id) {
            if let Some(m) = conversation.server_metadata() {
                return Some(m);
            }
        }

        // Fall back to conversation metadata
        if let Some(metadata) = self.get_conversation_metadata(conversation_id) {
            return metadata.server_conversation_metadata.as_ref();
        }

        None
    }

    pub fn get_server_conversation_metadata_by_server_token(
        &self,
        server_token: &ServerConversationToken,
    ) -> Option<&ServerAIConversationMetadata> {
        self.find_conversation_id_by_server_token(server_token)
            .and_then(|conversation_id| self.get_server_conversation_metadata(&conversation_id))
            .or_else(|| {
                self.all_conversations_metadata
                    .values()
                    .find(|metadata| {
                        metadata.server_conversation_token.as_ref() == Some(server_token)
                    })
                    .and_then(|metadata| metadata.server_conversation_metadata.as_ref())
            })
    }

    /// Finds an AIConversationId by its server conversation token.
    ///
    /// O(1) lookup via `server_token_to_conversation_id`, which is maintained
    /// alongside every mutation of `conversations_by_id` /
    /// `all_conversations_metadata` that involves a token. Used to look up
    /// conversations for ambient agent tasks, which store the server token
    /// but not the AIConversationId.
    pub fn find_conversation_id_by_server_token(
        &self,
        server_token: &ServerConversationToken,
    ) -> Option<AIConversationId> {
        if let Some(id) = self.server_token_to_conversation_id.get(server_token) {
            return Some(*id);
        }

        // A token miss is the expected outcome whenever a task references a
        // conversation this client hasn't loaded (shared-session tasks from
        // other users, pre-sync state).
        log::debug!(
            "No conversation found for server token: {}",
            server_token.as_str()
        );
        None
    }

    /// Returns the canonical local conversation ID for a server token, creating
    /// and caching one if this client has not seen the token before.
    pub fn get_or_set_canonical_conversation_id_for_server_token(
        &mut self,
        server_token: &ServerConversationToken,
    ) -> AIConversationId {
        if let Some(conversation_id) = self.server_token_to_conversation_id.get(server_token) {
            return *conversation_id;
        }

        let conversation_id = self
            .conversations_by_id
            .iter()
            .find_map(|(conversation_id, conversation)| {
                (conversation.server_conversation_token() == Some(server_token))
                    .then_some(*conversation_id)
            })
            .or_else(|| {
                self.all_conversations_metadata
                    .iter()
                    .find_map(|(conversation_id, metadata)| {
                        (metadata.server_conversation_token.as_ref() == Some(server_token))
                            .then_some(*conversation_id)
                    })
            })
            .unwrap_or_else(AIConversationId::new);

        self.server_token_to_conversation_id
            .insert(server_token.clone(), conversation_id);
        conversation_id
    }

    /// Mark conversations as historical
    /// Historical conversations consist of non-live conversations that were read from the disk or server on startup,
    /// and conversations (recorded here) that were live this session but have now been cleared.
    pub fn mark_conversations_historical_for_terminal_surface(
        &mut self,
        terminal_surface_id: EntityId,
    ) {
        if self.is_terminal_surface_conversation_transcript_viewer(terminal_surface_id) {
            // We don't mark conversation transcript viewer conversations as historical,
            // as they are stored separately and should not be persisted/displayed as regular user conversations.
            return;
        }

        // There's a slight concern here that the conversations we're preserving might not have persisted successfully
        // because of some unexpected error. Attempting to then restore these conversations would lead to unexpected behavior.
        // In the future it might be worthwhile to check that these conversations exist in the database before marking them as historical,
        // but for now this is an edge case that we don't need to worry about too much.
        let conversations_to_mark_historical: Vec<AIConversationMetadata> = self
            .all_live_conversations_for_terminal_surface(terminal_surface_id)
            .filter_map(|conversation| {
                let conversation_id = conversation.id();
                if !self.conversations_by_id.contains_key(&conversation_id)
                    || conversation.should_exclude_from_navigation()
                    || !blocklist_filter::conversation_would_render_in_blocklist(conversation)
                {
                    return None;
                }

                Some(conversation.into())
            })
            .collect();

        for metadata in conversations_to_mark_historical {
            if let Some(token) = &metadata.server_conversation_token {
                self.server_token_to_conversation_id
                    .insert(token.clone(), metadata.id);
            }
            self.all_conversations_metadata
                .insert(metadata.id, metadata);
        }
    }

    /// Inserts a conversation into memory by reconstructing exchanges from tasks.
    /// We use this when forking a conversation to ensure that the forked conversation
    /// is immediately available in memory before we try to restore it in a new tab.
    pub fn insert_forked_conversation_from_tasks(
        &mut self,
        conversation_id: AIConversationId,
        tasks: Vec<warp_multi_agent_api::Task>,
        conversation_data: AgentConversationData,
    ) -> anyhow::Result<AIConversation> {
        let mut conversation =
            AIConversation::new_restored(conversation_id, tasks, Some(conversation_data))?;

        // Assign fresh exchange IDs so persisted blocks do not collide.
        conversation.reassign_exchange_ids();

        if let Some(token) = conversation.server_conversation_token() {
            self.server_token_to_conversation_id
                .insert(token.clone(), conversation_id);
        }

        self.conversations_by_id
            .insert(conversation_id, conversation.clone());

        // This is harmless if we're opening the conversation immediately, but ensures it's in the conversation list right away if we fork in the background.
        let metadata = AIConversationMetadata::from(&conversation);
        if let Some(token) = &metadata.server_conversation_token {
            self.server_token_to_conversation_id
                .insert(token.clone(), conversation_id);
        }
        self.all_conversations_metadata
            .insert(conversation_id, metadata);

        Ok(conversation)
    }

    /// Rebuilds a remote-child placeholder conversation identified by
    /// `local_placeholder_id` from the cloud `tasks` + `cloud_conversation`,
    /// keeping the placeholder's local id and orchestration linkage
    /// (parent ids, agent_name, run_id, is_remote_child, pinned)
    /// authoritative. Cloud supplies the transcript and server-side metadata.
    ///
    /// Narrowly scoped to the remote-child placeholder hydration path
    /// (`pane_group::hydrate_remote_child_transcript_in_place`). Returns
    /// `Err` when the placeholder isn't loaded so the caller can fall back
    /// instead of silently producing a detached conversation.
    pub fn hydrate_remote_child_placeholder_with_cloud_transcript(
        &mut self,
        local_placeholder_id: AIConversationId,
        tasks: Vec<warp_multi_agent_api::Task>,
        cloud_conversation: AIConversation,
    ) -> anyhow::Result<AIConversation> {
        let placeholder = self
            .conversations_by_id
            .get(&local_placeholder_id)
            .cloned()
            .ok_or_else(|| {
                anyhow!(
                    "hydrate_remote_child_placeholder_with_cloud_transcript: \
                     local placeholder {local_placeholder_id} not found in conversations_by_id; \
                     refusing to construct a detached merged conversation"
                )
            })?;

        let merged_conversation_data =
            merged_remote_child_placeholder_conversation_data(&placeholder, &cloud_conversation);

        let mut merged = AIConversation::new_restored(
            local_placeholder_id,
            tasks,
            Some(merged_conversation_data),
        )?;
        merged.reassign_exchange_ids();

        if let Some(metadata) = cloud_conversation.server_metadata() {
            merged.set_server_metadata(metadata.clone());
        }

        if let Some(token) = merged.server_conversation_token() {
            self.server_token_to_conversation_id
                .insert(token.clone(), local_placeholder_id);
        }

        self.conversations_by_id
            .insert(local_placeholder_id, merged.clone());

        if let Some(parent_id) = self.resolved_parent_conversation_id_for_conversation(&merged) {
            self.index_child_conversation(local_placeholder_id, parent_id);
        }

        Ok(merged)
    }

    /// Clears all stored conversation-related data in memory.
    /// This is used when logging out to ensure no AI history persists across users.
    pub(crate) fn reset(&mut self) {
        self.live_conversation_ids_for_terminal_surface.clear();
        self.cleared_conversation_ids_for_terminal_surface.clear();
        self.conversations_by_id.clear();
        self.active_conversation_for_terminal_surface.clear();
        self.ambient_agent_terminal_surface_ids.clear();
        self.conversation_transcript_viewer_terminal_surface_ids
            .clear();
        self.persisted_queries.clear();
        self.prompt_history.clear();
        self.all_conversations_metadata.clear();
        self.agent_id_to_conversation_id.clear();
        self.server_token_to_conversation_id.clear();
        self.children_by_parent.clear();
    }
}

/// Builds the `AgentConversationData` for a remote-child placeholder
/// hydrated from a cloud transcript.
///
/// **Placeholder authoritative** (local orchestration linkage that the cloud
/// transcript cannot reconstruct):
/// - `parent_conversation_id`, `is_remote_child`, `pinned`
///
/// **Placeholder-preferred, cloud fallback** (local value wins when present,
/// cloud's value is used otherwise so we don't lose data on a stale
/// placeholder):
/// - `parent_agent_id`, `agent_name`, `orchestration_harness_type`, `run_id`
///
/// **Cloud authoritative** (server-side state the placeholder doesn't know):
/// - `server_conversation_token`, `conversation_usage_metadata`,
///   `forked_from_server_conversation_token`, `artifacts_json`,
///   `last_event_sequence`
///
/// **Reset on merge** (rebuild-from-cloud invariants):
/// - `reverted_action_ids = None`, `root_task_is_optimistic = None`,
///   `autoexecute_override = None`
fn merged_remote_child_placeholder_conversation_data(
    placeholder: &AIConversation,
    cloud_conversation: &AIConversation,
) -> AgentConversationData {
    AgentConversationData {
        // Cloud authoritative.
        server_conversation_token: cloud_conversation
            .server_conversation_token()
            .map(|t| t.as_str().to_string()),
        conversation_usage_metadata: Some(cloud_conversation.usage_metadata()),
        forked_from_server_conversation_token: cloud_conversation
            .forked_from_server_conversation_token()
            .map(|t| t.as_str().to_string()),
        artifacts_json: serde_json::to_string(cloud_conversation.artifacts()).ok(),
        last_event_sequence: cloud_conversation.last_event_sequence(),

        // Placeholder-preferred, cloud fallback.
        parent_agent_id: placeholder
            .parent_agent_id()
            .map(ToString::to_string)
            .or_else(|| {
                cloud_conversation
                    .parent_agent_id()
                    .map(ToString::to_string)
            }),
        agent_name: placeholder
            .agent_name()
            .map(ToString::to_string)
            .or_else(|| cloud_conversation.agent_name().map(ToString::to_string)),
        orchestration_harness_type: placeholder
            .orchestration_harness_type()
            .map(ToString::to_string)
            .or_else(|| {
                cloud_conversation
                    .orchestration_harness_type()
                    .map(ToString::to_string)
            }),
        run_id: placeholder.run_id().or_else(|| cloud_conversation.run_id()),

        // Placeholder authoritative.
        parent_conversation_id: placeholder
            .parent_conversation_id()
            .map(|id| id.to_string()),
        is_remote_child: placeholder.is_remote_child(),
        pinned: placeholder.is_pinned(),

        // Reset on merge.
        reverted_action_ids: None,
        root_task_is_optimistic: None,
        autoexecute_override: None,
    }
}

/// Returns the key to use in `agent_id_to_conversation_id` for the given
/// conversation.
fn agent_id_key(conversation: &AIConversation) -> Option<String> {
    conversation.orchestration_agent_id()
}

fn agent_id_key_from_persisted_data(conversation_data: &AgentConversationData) -> Option<&str> {
    conversation_data.run_id.as_deref()
}

/// Whether an `UpdatedConversationStatus` event represents a restoration
/// (the conversation was re-loaded for a terminal surface; the underlying
/// `ConversationStatus` did not change) or a real status set, in which case
/// the previous status is included.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConversationStatusUpdate {
    Restored,
    Changed { prev_status: ConversationStatus },
}

#[derive(Clone, Debug)]
pub enum BlocklistAIHistoryEvent {
    /// A new conversation was started.
    StartedNewConversation {
        new_conversation_id: AIConversationId,
        terminal_surface_id: EntityId,
    },

    CreatedSubtask {
        conversation_id: AIConversationId,
        terminal_surface_id: EntityId,
        task_id: TaskId,
    },

    /// Emitted when the optimistically created task is "upgraded" to a server-backed task upon
    /// receiving a CreateTask client action.
    UpgradedTask {
        optimistic_id: TaskId,
        server_id: TaskId,
        terminal_surface_id: EntityId,
    },

    AppendedExchange {
        exchange_id: AIAgentExchangeId,
        task_id: TaskId,
        terminal_surface_id: EntityId,
        conversation_id: AIConversationId,
        is_hidden: bool,

        // Populated if this exchange is appended as a result of an in-flight API request.
        response_stream_id: Option<ResponseStreamId>,
    },

    ReassignedExchange {
        exchange_id: AIAgentExchangeId,
        terminal_surface_id: EntityId,
        new_task_id: TaskId,
        new_conversation_id: AIConversationId,
    },

    /// Includes the terminal surface's [`EntityId`] so we can disambiguate the source of the event
    /// because this [`BlocklistAIHistoryModel`] is global.
    #[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
    UpdatedStreamingExchange {
        exchange_id: AIAgentExchangeId,
        terminal_surface_id: EntityId,
        conversation_id: AIConversationId,
        is_hidden: bool,
    },

    UpdatedConversationStatus {
        conversation_id: AIConversationId,
        terminal_surface_id: EntityId,
        /// Distinguishes a restoration from a real status set.
        update: ConversationStatusUpdate,
        /// The conversation's status after this update.
        new_status: ConversationStatus,
    },

    /// The active conversation was set to another conversation in the history.
    SetActiveConversation {
        conversation_id: AIConversationId,
        terminal_surface_id: EntityId,
    },

    /// `conversation_id` is no longer marked as active for the given terminal surface.
    ClearedActiveConversation {
        conversation_id: AIConversationId,
        terminal_surface_id: EntityId,
    },

    ClearedConversationsForTerminalSurface {
        terminal_surface_id: EntityId,
        active_conversation_id: Option<AIConversationId>,
        /// All conversation ids that were live in `terminal_surface_id` before the clear.
        /// Subscribers (e.g. `QueuedQueryModel`) use this to drop per-conversation state.
        cleared_conversation_ids: Vec<AIConversationId>,
    },

    UpdatedTodoList {
        terminal_surface_id: EntityId,
    },

    UpdatedAutoexecuteOverride {
        terminal_surface_id: EntityId,
    },

    /// Emitted when a conversation is split into two (on suggest starting new conversation)
    SplitConversation {
        terminal_surface_id: EntityId,
        old_conversation_id: AIConversationId,
        new_conversation_id: AIConversationId,
    },

    /// This is emitted when an ephemeral/abandoned conversation is cleaned up
    /// (e.g. empty conversations the user never used, rejected passive code suggestions).
    /// `run_id` carries the conversation's last known server run identifier
    /// (captured before the in-memory record was dropped) so subscribers can
    /// still act on it without a history-model lookup.
    RemoveConversation {
        terminal_surface_id: EntityId,
        conversation_id: AIConversationId,
        run_id: Option<String>,
    },

    /// This is emitted when a user explicitly deletes an existing conversation.
    /// `run_id` is captured before the in-memory record was dropped — see
    /// the note on [`Self::RemoveConversation`].
    DeletedConversation {
        terminal_surface_id: EntityId,
        conversation_id: AIConversationId,
        conversation_title: Option<String>,
        run_id: Option<String>,
    },

    /// Emitted when conversations are restored for a terminal surface.
    RestoredConversations {
        terminal_surface_id: EntityId,
        conversation_ids: Vec<AIConversationId>,
    },

    /// Emitted when conversation metadata is updated.
    /// `terminal_surface_id` is None when updating historical-only conversations.
    UpdatedConversationMetadata {
        terminal_surface_id: Option<EntityId>,
        conversation_id: AIConversationId,
    },

    /// Emitted when a conversation title changes.
    UpdatedConversationTitle {
        terminal_surface_id: Option<EntityId>,
        conversation_id: AIConversationId,
        title: String,
    },

    /// Emitted when conversation artifacts are updated (plans, PRs, etc.)
    UpdatedConversationArtifacts {
        terminal_surface_id: EntityId,
        conversation_id: AIConversationId,
        artifact: Artifact,
    },

    /// Emitted when a conversation first receives its server-assigned conversation token
    /// (during StreamInit). Used by the StartAgentExecutor to resolve pending StartAgent
    /// actions for child agent conversations.
    ConversationServerTokenAssigned {
        conversation_id: AIConversationId,
        terminal_surface_id: EntityId,
    },

    /// Emitted when a conversation moves between terminal surfaces — i.e. when
    /// `set_active_conversation_id` removes the conversation from the live
    /// list of one or more `previous_terminal_surface_id`s. The previous terminal surfaces
    /// must drop any rendered AI blocks for this conversation so the new
    /// terminal surface is the sole renderer; otherwise we end up with a transcript
    /// split across panes (some blocks in the old view, new exchanges in the
    /// new view). The `terminal_surface_id()` accessor returns the previous
    /// terminal surface so existing per-view event filters do the right thing.
    ConversationTransferredBetweenTerminalSurfaces {
        conversation_id: AIConversationId,
        previous_terminal_surface_id: EntityId,
        new_terminal_surface_id: EntityId,
    },

    /// Links an executor-minted request to a freshly-created
    /// conversation.
    NewConversationRequestComplete {
        request_id: crate::ai::blocklist::StartAgentRequestId,
        conversation_id: AIConversationId,
    },

    /// Emitted when a conversation's orchestration config is updated
    /// (live wire snapshot, user edit, or restore-hydration).
    /// Consumers that perform UI side effects should gate on `!from_restore`.
    OrchestrationConfigUpdated {
        conversation_id: AIConversationId,
        from_restore: bool,
    },

    /// Emitted when a conversation's `conversation_usage_metadata` is updated
    /// (for example after a `StreamFinished` event). Subscribers that derive
    /// data from cross-conversation usage — e.g. the orchestration credit
    /// rollup in the agent-mode footer — can listen for this to re-render
    /// when a descendant's credits change.
    ConversationUsageMetadataUpdated {
        conversation_id: AIConversationId,
    },

    /// Emitted when a sharer-owned conversation establishes a local
    /// shared session.
    LocalSharedSessionEstablished {
        conversation_id: AIConversationId,
        session_id: session_sharing_protocol::common::SessionId,
    },
}

impl BlocklistAIHistoryEvent {
    /// Returns the terminal surface ID associated with this event, if any.
    /// Returns `None` for events that apply globally (e.g., historical conversation metadata updates).
    pub fn terminal_surface_id(&self) -> Option<EntityId> {
        match self {
            BlocklistAIHistoryEvent::StartedNewConversation {
                terminal_surface_id,
                ..
            }
            | BlocklistAIHistoryEvent::AppendedExchange {
                terminal_surface_id,
                ..
            }
            | BlocklistAIHistoryEvent::UpdatedStreamingExchange {
                terminal_surface_id,
                ..
            }
            | BlocklistAIHistoryEvent::UpdatedConversationStatus {
                terminal_surface_id,
                ..
            }
            | BlocklistAIHistoryEvent::SetActiveConversation {
                terminal_surface_id,
                ..
            }
            | BlocklistAIHistoryEvent::ClearedActiveConversation {
                terminal_surface_id,
                ..
            }
            | BlocklistAIHistoryEvent::ClearedConversationsForTerminalSurface {
                terminal_surface_id,
                ..
            }
            | BlocklistAIHistoryEvent::ReassignedExchange {
                terminal_surface_id,
                ..
            }
            | BlocklistAIHistoryEvent::UpdatedTodoList {
                terminal_surface_id,
                ..
            }
            | BlocklistAIHistoryEvent::UpdatedAutoexecuteOverride {
                terminal_surface_id,
                ..
            }
            | BlocklistAIHistoryEvent::SplitConversation {
                terminal_surface_id,
                ..
            }
            | BlocklistAIHistoryEvent::RemoveConversation {
                terminal_surface_id,
                ..
            }
            | BlocklistAIHistoryEvent::DeletedConversation {
                terminal_surface_id,
                ..
            }
            | BlocklistAIHistoryEvent::CreatedSubtask {
                terminal_surface_id,
                ..
            }
            | BlocklistAIHistoryEvent::RestoredConversations {
                terminal_surface_id,
                ..
            }
            | BlocklistAIHistoryEvent::UpgradedTask {
                terminal_surface_id,
                ..
            }
            | BlocklistAIHistoryEvent::ConversationTransferredBetweenTerminalSurfaces {
                previous_terminal_surface_id: terminal_surface_id,
                ..
            }
            | BlocklistAIHistoryEvent::UpdatedConversationArtifacts {
                terminal_surface_id,
                ..
            }
            | BlocklistAIHistoryEvent::ConversationServerTokenAssigned {
                terminal_surface_id,
                ..
            } => Some(*terminal_surface_id),
            // UpdatedConversationMetadata can have None when updating historical-only conversations
            BlocklistAIHistoryEvent::UpdatedConversationMetadata {
                terminal_surface_id,
                ..
            }
            | BlocklistAIHistoryEvent::UpdatedConversationTitle {
                terminal_surface_id,
                ..
            } => *terminal_surface_id,
            // NewConversationRequestComplete is executor-scoped and has no
            // terminal_surface_id.
            BlocklistAIHistoryEvent::NewConversationRequestComplete { .. } => None,
            // OrchestrationConfigUpdated is conversation-scoped and has no
            // terminal_surface_id.
            BlocklistAIHistoryEvent::OrchestrationConfigUpdated { .. } => None,
            // ConversationUsageMetadataUpdated is conversation-scoped and
            // has no terminal_surface_id. Cross-pane consumers (e.g. the
            // orchestrator footer reading descendant credits) can't be
            // disambiguated by a single terminal surface pane.
            BlocklistAIHistoryEvent::ConversationUsageMetadataUpdated { .. } => None,
            // Conversation-scoped; subscribers resolve the owning view via conversation_id.
            BlocklistAIHistoryEvent::LocalSharedSessionEstablished { .. } => None,
        }
    }
}

impl BlocklistAIHistoryModel {
    /// Emits [`BlocklistAIHistoryEvent::NewConversationRequestComplete`].
    pub fn record_new_conversation_request_complete(
        &mut self,
        request_id: crate::ai::blocklist::StartAgentRequestId,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        ctx.emit(BlocklistAIHistoryEvent::NewConversationRequestComplete {
            request_id,
            conversation_id,
        });
    }
}

impl Entity for BlocklistAIHistoryModel {
    type Event = BlocklistAIHistoryEvent;
}

impl SingletonEntity for BlocklistAIHistoryModel {}

/// Helper struct for showing AI history to the user. Guarantees that there is a user query and
/// contains less data than [`AIAgentExchange`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AIQueryHistory {
    /// The input originating from the user.
    pub query_text: String,

    /// The time the input was sent.
    pub start_time: DateTime<Local>,

    /// The status of the output streaming from the AI API.
    pub output_status: AIQueryHistoryOutputStatus,

    /// The working directory when the AI query was submitted.
    pub working_directory: Option<String>,

    /// The ordering category for this query in history.
    pub history_order: HistoryOrder,
}

impl AIQueryHistory {
    /// Creates a new [`AIQueryHistory`] for testing.
    #[cfg(test)]
    pub(crate) fn new_for_test(
        query_text: &str,
        start_time: DateTime<Local>,
        history_order: HistoryOrder,
    ) -> Self {
        Self {
            query_text: query_text.to_owned(),
            start_time,
            output_status: AIQueryHistoryOutputStatus::Pending,
            working_directory: None,
            history_order,
        }
    }
}

fn ai_exchange_to_query_history(
    value: &AIAgentExchange,
    history_order: HistoryOrder,
) -> Option<AIQueryHistory> {
    let query = value.input.iter().find_map(AIAgentInput::display_query)?;

    Some(AIQueryHistory {
        query_text: query,
        start_time: value.start_time,
        output_status: AIQueryHistoryOutputStatus::from(&value.output_status),
        working_directory: value.working_directory.clone(),
        history_order,
    })
}

fn persisted_ai_input_to_query_history(
    value: &PersistedAIInput,
    history_order: HistoryOrder,
) -> Option<AIQueryHistory> {
    // Extract the query text from the first Query input
    let query_text = value
        .inputs
        .iter()
        .map(|input| match input {
            PersistedAIInputType::Query { text, .. } => Some(text.clone()),
        })
        .next()
        .flatten()?;

    Some(AIQueryHistory {
        query_text,
        start_time: value.start_ts,
        output_status: value.output_status.clone(),
        working_directory: value.working_directory.clone(),
        history_order,
    })
}

/// Status of output streaming from the AI API.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum AIQueryHistoryOutputStatus {
    /// We are waiting to or are currently streaming output.
    Pending,
    /// The user manually cancelled output streaming.
    Cancelled,
    /// Output streaming failed.
    Failed,
    /// Output streaming completed successfully.
    Completed,
}

impl AIQueryHistoryOutputStatus {
    /// Returns a string representation of the output status.
    pub(crate) fn display_text(&self) -> &'static str {
        match self {
            AIQueryHistoryOutputStatus::Completed => "Completed successfully",
            AIQueryHistoryOutputStatus::Pending => "Pending",
            AIQueryHistoryOutputStatus::Cancelled => "Cancelled by user",
            AIQueryHistoryOutputStatus::Failed => "Failed",
        }
    }

    pub(crate) fn icon(&self) -> Icon {
        match self {
            AIQueryHistoryOutputStatus::Completed => Icon::Check,
            AIQueryHistoryOutputStatus::Pending => Icon::Loading,
            AIQueryHistoryOutputStatus::Cancelled => Icon::SlashCircle,
            AIQueryHistoryOutputStatus::Failed => Icon::AlertTriangle,
        }
    }
}

impl From<&AIAgentOutputStatus> for AIQueryHistoryOutputStatus {
    fn from(status: &AIAgentOutputStatus) -> Self {
        match status {
            AIAgentOutputStatus::Streaming { .. } => Self::Pending,
            AIAgentOutputStatus::Finished {
                finished_output, ..
            } => match finished_output {
                FinishedAIAgentOutput::Cancelled { .. } => Self::Cancelled,
                FinishedAIAgentOutput::Error { .. } => Self::Failed,
                FinishedAIAgentOutput::Success { .. } => Self::Completed,
            },
        }
    }
}

/// Mirrors the server's `isClientToolCall`: true for tool calls the client
/// executes. Server and subagent tool calls are managed by the server's
/// runtime and must not be reconciled — notably the `RunPrimaryAgent`
/// bootstrap call at the start of every root task is unresolved by design (it
/// anchors the primary agent on the server's run stack), and pairing it with a
/// synthesized `Cancel` would pop the primary agent on the fork's next
/// request, breaking the conversation.
fn is_client_tool_call(tool_call: &warp_multi_agent_api::message::ToolCall) -> bool {
    !matches!(
        tool_call.tool,
        None | Some(Tool::Server(_)) | Some(Tool::Subagent(_))
    )
}

/// Reconciles client `tool_call`s in the fork-point exchange
/// (`fork_point_message_ids`, the task's last retained exchange) whose
/// `tool_call_result` was dropped by the truncation: a call and its result
/// carry different `request_id`s, so they land in different exchanges and the
/// fork point separates them. An unpaired `tool_use` fails the fork's next
/// request with an Anthropic `400 invalid_request_error`.
///
/// Each severed call gets its real result pulled forward from
/// `source_task_messages` (the task's pre-truncation history), or a
/// synthesized `Cancel` when none exists (a genuinely in-flight call). The
/// result is inserted immediately after its `tool_call`. Tool calls outside
/// the fork-point exchange were dangling in the source too, and are left
/// untouched so the fork reproduces the source history faithfully.
fn reconcile_dangling_tool_calls_in_forked_task(
    task: &mut warp_multi_agent_api::Task,
    source_task_messages: &[warp_multi_agent_api::Message],
    fork_point_message_ids: &HashSet<MessageId>,
) {
    let fork_point_message_ids: HashSet<&str> =
        fork_point_message_ids.iter().map(|id| &**id).collect();
    let resolved_tool_call_ids: HashSet<&str> = task
        .messages
        .iter()
        .filter_map(|m| m.tool_call_result().map(|r| r.tool_call_id.as_str()))
        .collect();

    // Collect each dangling tool_call's position, id, and request_id up front so
    // we don't mutate the message list while iterating it.
    let dangling: Vec<(usize, String, String)> = task
        .messages
        .iter()
        .enumerate()
        .filter_map(|(idx, message)| {
            if !fork_point_message_ids.contains(message.id.as_str()) {
                return None;
            }
            let tool_call = message.tool_call()?;
            (is_client_tool_call(tool_call)
                && !resolved_tool_call_ids.contains(tool_call.tool_call_id.as_str()))
            .then(|| {
                (
                    idx,
                    tool_call.tool_call_id.clone(),
                    message.request_id.clone(),
                )
            })
        })
        .collect();

    // Common case: nothing dangling, so skip scanning the source history.
    if dangling.is_empty() {
        return;
    }

    // Real results from the source history, keyed by tool_call_id, so a dropped
    // result can be pulled forward.
    let source_results_by_tool_call_id: HashMap<&str, &warp_multi_agent_api::Message> =
        source_task_messages
            .iter()
            .filter_map(|m| m.tool_call_result().map(|r| (r.tool_call_id.as_str(), m)))
            .collect();

    // Insert from the back so earlier indices remain valid as we splice.
    for (idx, tool_call_id, request_id) in dangling.into_iter().rev() {
        let reconciled = source_results_by_tool_call_id
            .get(tool_call_id.as_str())
            .map(|real| (*real).clone())
            .unwrap_or_else(|| {
                // Synthesized `Cancel` carries the call's `request_id` so it
                // groups into the same exchange as its `tool_call` on restore.
                warp_multi_agent_api::Message {
                    id: Uuid::new_v4().to_string(),
                    task_id: task.id.clone(),
                    server_message_data: String::new(),
                    citations: vec![],
                    fetched_memories: vec![],
                    message: Some(warp_multi_agent_api::message::Message::ToolCallResult(
                        warp_multi_agent_api::message::ToolCallResult {
                            tool_call_id: tool_call_id.clone(),
                            context: None,
                            result: Some(
                                warp_multi_agent_api::message::tool_call_result::Result::Cancel(()),
                            ),
                        },
                    )),
                    request_id: request_id.clone(),
                    timestamp: None,
                }
            });
        task.messages.insert(idx + 1, reconciled);
    }
}

/// Updates the given tasks, which are presumed to be clones of tasks from a source conversation to be
/// used to back a fork or copy of the source conversation.
///
/// When `preserve_task_ids` is false, reassigns new task IDs to each forked task to ensure task IDs
/// remain globally unique. When true, leaves task IDs as-is so the local fork's task store matches
/// an externally-known set of task ids whose ClientActions must resolve in the local fork.
///
/// Always prepends the given prefix to the root task's description.
fn update_forked_task_properties(
    tasks: Vec<warp_multi_agent_api::Task>,
    prefix: &str,
    preserve_task_ids: bool,
    title_override: Option<&str>,
) -> Vec<warp_multi_agent_api::Task> {
    let root_description = |current: &str| match title_override {
        Some(title) => title.to_owned(),
        None => format!("{prefix}{current}"),
    };

    if preserve_task_ids {
        return tasks
            .into_iter()
            .map(|mut t| {
                let is_root = t
                    .dependencies
                    .as_ref()
                    .map(|deps| deps.parent_task_id.is_empty())
                    .unwrap_or(true);
                if is_root {
                    t.description = root_description(&t.description);
                }
                t
            })
            .collect();
    }

    let mut old_to_new_task_ids = HashMap::new();
    fn get_new_task_id(new_ids: &mut HashMap<String, String>, old_task_id: &str) -> String {
        new_ids
            .entry(old_task_id.to_owned())
            .or_insert_with(|| Uuid::new_v4().to_string())
            .clone()
    }

    tasks
        .into_iter()
        .map(|mut t| {
            let new_id = get_new_task_id(&mut old_to_new_task_ids, &t.id);
            // Update task id to avoid duplicate tasks across conversations and ensure
            // all messages reference the new task id.
            t.id = new_id.clone();
            for message in &mut t.messages {
                message.task_id = new_id.clone();
                if let Some(subagent) = message.tool_call_mut().and_then(|tc| tc.subagent_mut()) {
                    subagent.task_id =
                        get_new_task_id(&mut old_to_new_task_ids, &subagent.task_id).clone();
                }
            }
            if let Some(deps) = t
                .dependencies
                .as_mut()
                .filter(|deps| !deps.parent_task_id.is_empty())
            {
                deps.parent_task_id =
                    get_new_task_id(&mut old_to_new_task_ids, &deps.parent_task_id).clone();
            } else {
                t.description = root_description(&t.description);
            }
            t
        })
        .collect()
}

/// The default prefix used when forking a conversation.
pub const FORK_PREFIX: &str = "(Fork) ";

/// The prefix used when saving a conversation before a rewind operation.
pub const PRE_REWIND_PREFIX: &str = "(Pre-Rewind) ";

#[cfg(test)]
#[path = "history_model_tests.rs"]
mod tests;
