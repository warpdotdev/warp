//! This module contains functions for loading, fetching, and merging conversation data
//! from local database and server sources.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::Arc;

use futures::FutureExt;
use itertools::Itertools as _;
use persistence::model::AgentConversationRecord;
use warp_core::features::FeatureFlag;
use warpui::{AppContext, SingletonEntity};

use crate::ai::agent::api::convert_conversation::{
    convert_conversation_data_to_ai_conversation, RestorationMode,
};
use crate::ai::agent::api::ServerConversationToken;
use crate::ai::agent::conversation::{
    AIAgentHarness, AIConversation, AIConversationId, ServerAIConversationMetadata,
};
use crate::ai::agent::task::Task;
use crate::persistence::model::{AgentConversation, AgentConversationData};
use crate::server::server_api::ai::AIClient;
use crate::server::server_api::ServerApiProvider;
use crate::terminal::model::block::SerializedBlock;

#[cfg(feature = "local_fs")]
use crate::persistence::agent::read_agent_conversation_by_id;

use super::{AIConversationMetadata, BlocklistAIHistoryModel, MAX_HISTORICAL_CONVERSATIONS};

/// A conversation transcript from a CLI agent harness (e.g. Claude Code).
#[derive(Debug, Clone)]
pub struct CLIAgentConversation {
    /// Server metadata about this conversation.
    pub metadata: ServerAIConversationMetadata,
    /// A snapshot of the final agent TUI state.
    pub block: SerializedBlock,
}

/// Representation of the conversation data that can be fetched from cloud storage.
///
/// The exact format depends on the agent harness that produced the conversation.
pub enum CloudConversationData {
    /// A conversation produced by the Oz harness, which we can materialize into the
    /// [`AIConversation`] data model.
    Oz(Box<AIConversation>),
    /// A conversation produced by an external CLI agent harness.
    CLIAgent(Box<CLIAgentConversation>),
}

/// Converts an `AgentConversation` from the database to an `AIConversation`.
/// This utility function extracts the conversion logic that was originally embedded
/// in the terminal view restoration process.
#[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
pub fn convert_persisted_conversation_to_ai_conversation(
    persisted_conversation: AgentConversation,
) -> Option<AIConversation> {
    convert_persisted_conversation_to_ai_conversation_with_metadata(persisted_conversation)
}

/// Enhanced version of the conversion function with additional metadata.
/// This version supports the full feature set needed by terminal view restoration.
pub fn convert_persisted_conversation_to_ai_conversation_with_metadata(
    persisted_conversation: AgentConversation,
) -> Option<AIConversation> {
    let AgentConversation {
        tasks,
        conversation:
            AgentConversationRecord {
                conversation_id,
                conversation_data,
                ..
            },
    } = persisted_conversation;

    let conversation_id = match AIConversationId::try_from(conversation_id) {
        Ok(id) => id,
        Err(e) => {
            log::warn!("Failed to convert conversation ID: {e:?}");
            return None;
        }
    };

    let conversation_data = serde_json::from_str::<AgentConversationData>(&conversation_data).ok();

    match AIConversation::new_restored(conversation_id, tasks, conversation_data) {
        Ok(conversation) => Some(conversation),
        Err(e) => {
            log::warn!("Failed to convert persisted conversation to AIConversation: {e:?}");
            None
        }
    }
}

/// Loads a conversation from the server asynchronously.
/// This is a free-floating function that can be called without a model reference.
pub async fn load_conversation_from_server(
    conversation_id: AIConversationId,
    server_conversation_token: ServerConversationToken,
    server_api: Arc<dyn AIClient>,
) -> Option<CloudConversationData> {
    if !FeatureFlag::CloudConversations.is_enabled() {
        return None;
    }

    log::info!(
        "Loading full conversation data for {conversation_id} from server using token {}",
        server_conversation_token.as_str()
    );

    match server_api
        .get_ai_conversation(server_conversation_token.clone())
        .await
    {
        Ok((conversation_data, server_metadata)) => {
            match server_metadata.harness {
                AIAgentHarness::Oz => {
                    // Convert Oz conversations to an AIConversation.
                    match convert_conversation_data_to_ai_conversation(
                        conversation_id,
                        &conversation_data,
                        server_metadata,
                        RestorationMode::Continue,
                    ) {
                        Some(conversation) => {
                            log::info!("Loaded Oz conversation {conversation_id} from server");
                            Some(CloudConversationData::Oz(Box::new(conversation)))
                        }
                        None => {
                            log::warn!(
                                "Failed to convert Oz server conversation data for {conversation_id}"
                            );
                            None
                        }
                    }
                }
                AIAgentHarness::ClaudeCode | AIAgentHarness::Gemini | AIAgentHarness::Codex => {
                    if !FeatureFlag::AgentHarness.is_enabled() {
                        log::warn!("Ignoring non-Oz conversation {conversation_id}: AgentHarness flag is disabled");
                        return None;
                    }
                    // Fetch snapshot data for third-party harness conversations.
                    match server_api
                        .get_block_snapshot(server_conversation_token)
                        .await
                    {
                        Ok(block) => {
                            log::info!("Loaded CLI agent block snapshot for {conversation_id}");
                            Some(CloudConversationData::CLIAgent(Box::new(
                                CLIAgentConversation {
                                    metadata: server_metadata,
                                    block,
                                },
                            )))
                        }
                        Err(e) => {
                            log::warn!(
                                "Failed to fetch block snapshot for {conversation_id}: {e:#}"
                            );
                            None
                        }
                    }
                }
                AIAgentHarness::Unknown => {
                    log::warn!(
                        "Ignoring conversation {conversation_id}: server reported an unknown harness; this client may be out of date"
                    );
                    None
                }
            }
        }
        Err(e) => {
            log::warn!("Failed to load conversation {conversation_id} from server: {e:#}");
            None
        }
    }
}

/// Boxes a future with the right type for the platform.
/// On WASM, futures must not implement Send.
fn box_future<F>(f: F) -> warpui::r#async::BoxFuture<'static, Option<CloudConversationData>>
where
    F: Future<Output = Option<CloudConversationData>> + warpui::r#async::Spawnable,
{
    cfg_if::cfg_if! {
        if #[cfg(target_family = "wasm")] {
            f.boxed_local()
        } else {
            f.boxed()
        }
    }
}

impl AIConversationMetadata {
    fn merge(&mut self, other: AIConversationMetadata) -> &mut Self {
        self.has_local_data |= other.has_local_data;
        if self.initial_query.is_empty() {
            self.initial_query = other.initial_query;
        }
        if self.initial_working_directory.is_none() {
            self.initial_working_directory = other.initial_working_directory;
        }
        if self.artifacts.is_empty() {
            self.artifacts = other.artifacts;
        }
        self
    }
}

impl BlocklistAIHistoryModel {
    /// Loads conversation data from the appropriate source (DB or server).
    ///
    /// This method automatically determines whether to load from the local database or
    /// the server based on the conversation's metadata:
    /// - If the conversation is already in memory, returns it immediately
    /// - If has_local_data is true, loads from the local database synchronously
    /// - Otherwise, loads from the server asynchronously
    ///
    /// Note: This does NOT insert the conversation into memory. Callers are responsible
    /// for inserting the loaded conversation if needed.
    pub fn load_conversation_data(
        &self,
        conversation_id: AIConversationId,
        ctx: &AppContext,
    ) -> warpui::r#async::BoxFuture<'static, Option<CloudConversationData>> {
        // First check if the conversation is already in memory
        if let Some(conversation) = self.conversations_by_id.get(&conversation_id) {
            return box_future(futures::future::ready(Some(CloudConversationData::Oz(
                Box::new(conversation.clone()),
            ))));
        }

        // Check metadata to determine the source
        let Some(metadata) = self
            .all_conversations_metadata
            .get(&conversation_id)
            .cloned()
        else {
            log::warn!("No metadata found for conversation {conversation_id}");
            return box_future(futures::future::ready(None));
        };

        if metadata.has_local_data {
            // Load from local database synchronously
            let result = self
                .load_conversation_from_db(&conversation_id)
                .map(|c| CloudConversationData::Oz(Box::new(c)));
            box_future(futures::future::ready(result))
        } else {
            // Load from server asynchronously
            if let Some(server_token) = metadata.server_conversation_token {
                // Extract the server API before creating the async future
                let server_api = ServerApiProvider::as_ref(ctx).get_ai_client();
                box_future(load_conversation_from_server(
                    conversation_id,
                    server_token,
                    server_api,
                ))
            } else {
                log::warn!(
                    "Cannot load conversation {conversation_id}: no local data and no server token"
                );
                box_future(futures::future::ready(None))
            }
        }
    }

    /// Loads a conversation by its server token, with a server fallback.
    ///
    /// First attempts to find the conversation in local metadata and load it
    /// via `load_conversation_data`. If the token is not present locally
    /// (e.g. cloud metadata hasn't been merged yet), falls back to loading
    /// the conversation directly from the server.
    ///
    /// Note: This does NOT insert the conversation into memory. Callers are responsible
    /// for inserting the loaded conversation if needed.
    pub fn load_conversation_by_server_token(
        &mut self,
        server_token: &ServerConversationToken,
        ctx: &AppContext,
    ) -> warpui::r#async::BoxFuture<'static, Option<CloudConversationData>> {
        let conversation_id =
            self.get_or_set_canonical_conversation_id_for_server_token(server_token);
        if self.conversations_by_id.contains_key(&conversation_id)
            || self
                .all_conversations_metadata
                .contains_key(&conversation_id)
        {
            return self.load_conversation_data(conversation_id, ctx);
        }

        // Fallback: load directly from the server. This handles cases where
        // cloud metadata hasn't been merged into the local history model yet
        // (e.g. timing on startup, or conversations only surfaced via
        // AgentConversationsModel).
        log::warn!(
            "No local metadata for server token {}, falling back to server fetch",
            server_token.as_str()
        );
        let server_api = ServerApiProvider::as_ref(ctx).get_ai_client();
        box_future(load_conversation_from_server(
            conversation_id,
            server_token.clone(),
            server_api,
        ))
    }

    /// Loads a conversation from local DB and returns it.
    /// This is a private helper method. Use `get_load_conversation_data_future` instead.
    ///
    /// Note: This does NOT insert the conversation into memory. Callers are responsible
    /// for inserting the loaded conversation if needed.
    pub(super) fn load_conversation_from_db(
        &self,
        conversation_id: &AIConversationId,
    ) -> Option<AIConversation> {
        // First check if the conversation is in memory
        if let Some(conversation) = self.conversations_by_id.get(conversation_id) {
            return Some(conversation.clone());
        }

        // If not in memory, try to load from the database
        #[cfg(feature = "local_fs")]
        {
            let persisted_ai_conversation = self.db_connection.clone().and_then(|conn| {
                let mut conn = conn.lock().ok()?;

                let id_str = conversation_id.to_string();
                log::info!("Loading conversation {id_str} from db");
                match read_agent_conversation_by_id(&mut conn, &id_str) {
                    Ok(Some(conv)) => Some(conv),
                    Ok(None) => {
                        log::warn!("No AgentConversation found with id {id_str}");
                        None
                    }
                    Err(e) => {
                        log::warn!("Failed to read AgentConversation {id_str}: {e:?}");
                        None
                    }
                }
            });

            // Convert the persisted conversation to an AIConversation
            if let Some(persisted_conversation) = persisted_ai_conversation {
                if let Some(conversation) =
                    convert_persisted_conversation_to_ai_conversation(persisted_conversation)
                {
                    return Some(conversation);
                }
            }
        }

        None
    }

    /// Merges cloud conversation metadata with existing local metadata.
    /// Deduplicates by conversation_id and server_conversation_token.
    /// Also updates server_metadata on any already-restored conversations that match by token.
    pub fn merge_cloud_conversation_metadata(
        &mut self,
        cloud_metadata_list: Vec<ServerAIConversationMetadata>,
    ) {
        let local_count = self.all_conversations_metadata.len();
        let mut local_matched_with_server_count = 0;
        let mut new_cloud_count = 0;
        let mut restored_conversations_updated = 0;

        // Collect tokens belonging to child agent conversations so we can skip them.
        let mut child_conversation_tokens: HashSet<String> = HashSet::new();
        for conv in self.conversations_by_id.values() {
            if let Some(token) = conv.server_conversation_token() {
                if conv.is_child_agent_conversation() {
                    child_conversation_tokens.insert(token.as_str().to_string());
                }
            }
        }

        for server_meta in cloud_metadata_list {
            let server_token = server_meta.server_conversation_token.clone();
            let server_token_str = server_token.as_str();

            // Child agent conversations are managed by their parent's status card
            // and should not appear in navigation/history.
            if child_conversation_tokens.contains(server_token_str) {
                continue;
            }
            let had_metadata_entry = self
                .all_conversations_metadata
                .values()
                .any(|metadata| metadata.server_conversation_token.as_ref() == Some(&server_token));
            let canonical_conversation_id =
                self.get_or_set_canonical_conversation_id_for_server_token(&server_token);

            if let Some(conversation) = self.conversations_by_id.get_mut(&canonical_conversation_id)
            {
                conversation.set_server_metadata(server_meta.clone());
                restored_conversations_updated += 1;
                log::debug!(
                    "Updated server metadata for restored conversation {canonical_conversation_id} with token {server_token_str}"
                );
            }

            let stale_metadata: Vec<(AIConversationId, AIConversationMetadata)> = self
                .all_conversations_metadata
                .iter()
                .filter_map(|(conversation_id, metadata)| {
                    (*conversation_id != canonical_conversation_id
                        && metadata.server_conversation_token.as_ref() == Some(&server_token))
                    .then_some((*conversation_id, metadata.clone()))
                })
                .collect();
            for (stale_metadata_id, _) in &stale_metadata {
                self.all_conversations_metadata.remove(stale_metadata_id);
            }

            let existing_metadata = self
                .all_conversations_metadata
                .get(&canonical_conversation_id)
                .cloned();
            let local_metadata = self
                .conversations_by_id
                .get(&canonical_conversation_id)
                .map(AIConversationMetadata::from);
            let mut metadata = AIConversationMetadata::from_server_metadata(
                canonical_conversation_id,
                server_meta,
            );
            for local_metadata in existing_metadata
                .into_iter()
                .chain(stale_metadata.into_iter().map(|(_, metadata)| metadata))
                .chain(local_metadata)
            {
                metadata.merge(local_metadata);
            }

            self.server_token_to_conversation_id
                .insert(server_token.clone(), canonical_conversation_id);
            self.all_conversations_metadata
                .insert(canonical_conversation_id, metadata);

            if had_metadata_entry {
                local_matched_with_server_count += 1;
                log::debug!(
                    "Matched local conversation {canonical_conversation_id} with server token {server_token_str}"
                );
            } else {
                new_cloud_count += 1;
                log::debug!(
                    "Added new cloud-only conversation with local ID {canonical_conversation_id} and server token {server_token_str}"
                );
            }
        }

        log::info!(
            "Merged cloud conversations: {} local, {} found matched cloud metadata, {} new cloud-only added, {} restored conversations updated. Total: {}",
            local_count,
            local_matched_with_server_count,
            new_cloud_count,
            restored_conversations_updated,
            self.all_conversations_metadata.len()
        );
    }

    /// Initializes historical conversations from restored agent conversations.
    pub(super) fn initialize_historical_conversations(
        &mut self,
        conversations: &[AgentConversation],
    ) {
        let conversations = conversations
            .iter()
            .sorted_by_key(|c| c.conversation.last_modified_at)
            .rev();

        let collected: HashMap<AIConversationId, AIConversationMetadata> = conversations
            .take(MAX_HISTORICAL_CONVERSATIONS)
            .filter_map(|agent_conv| {
                // Try to convert the conversation ID
                let conversation_id = match AIConversationId::try_from(
                    agent_conv.conversation.conversation_id.clone(),
                ) {
                    Ok(id) => id,
                    Err(e) => {
                        log::warn!("Failed to convert conversation ID: {e:?}");
                        return None;
                    }
                };

                if !agent_conv.is_restorable() {
                    return None;
                }

                // Child agent conversations are managed by their parent's
                // status card and should not appear in navigation/history.
                // Record the parent→child mapping before filtering so that
                // create_missing_child_agent_panes can discover children
                // before they are loaded into conversations_by_id.
                let conversation_data = serde_json::from_str::<AgentConversationData>(
                    &agent_conv.conversation.conversation_data,
                )
                .ok();
                if let Some(parent_id_str) = conversation_data
                    .as_ref()
                    .and_then(|data| data.parent_conversation_id.as_deref())
                {
                    if let Ok(parent_id) = AIConversationId::try_from(parent_id_str.to_string()) {
                        self.children_by_parent
                            .entry(parent_id)
                            .or_default()
                            .push(conversation_id);
                    }
                    return None;
                }

                // Skip conversations that contain AutoCodeDiff system queries but do not contain any UserQuery messages.
                // These are passive, auto-initiated diffs that were never interacted with (past accepting or rejecting the diff),
                // so we don't want to list them as historical conversations.
                let mut has_user_query = false;
                let mut has_autocodediff = false;
                for task in &agent_conv.tasks {
                    for message in &task.messages {
                        match &message.message {
                            Some(warp_multi_agent_api::message::Message::UserQuery(_)) => {
                                has_user_query = true;
                            }
                            Some(warp_multi_agent_api::message::Message::SystemQuery(sys)) => {
                                if let Some(warp_multi_agent_api::message::system_query::Type::AutoCodeDiff(_)) = &sys.r#type {
                                    has_autocodediff = true;
                                }
                            }
                            _ => {}
                        }
                    }
                }
                if has_autocodediff && !has_user_query {
                    return None;
                }

                let root_task = agent_conv.tasks.iter().find(|task| {
                    task.dependencies.is_none()
                });

                let initial_query = root_task.map(|task| {
                    // find the first task with a user query
                    // (or in the case of a passive code diff, the summary of the diff)
                    task.messages.iter().find_map(|msg| {
                        match &msg.message {
                            Some(warp_multi_agent_api::message::Message::UserQuery(user_query)) => {
                                Some(user_query.query.clone())
                            }
                            Some(warp_multi_agent_api::message::Message::ToolCall(tool_call))  => {
                                let Some(tool) = &tool_call.tool else {
                                    return None;
                                };

                                if let warp_multi_agent_api::message::tool_call::Tool::ApplyFileDiffs(diff_suggestion) = tool {
                                    Some(diff_suggestion.summary.clone())
                                } else {
                                    None
                                }
                            }
                            _ => None,
                        }
                    }).unwrap_or_default()
                }).unwrap_or_default();

                if initial_query.is_empty() {
                    log::warn!(
                        "Failed to record conversation with ID {conversation_id} because it was missing an initial query"
                    );
                    return None;
                }

                // We derive the title from the description of the root task,
                // falling back to initial_query if the description is empty
                let title = root_task
                    .map(|task| task.description.clone())
                    .filter(|desc| !desc.is_empty())
                    .unwrap_or_else(|| initial_query.clone());

                // Extract working directory from the first UserQuery message in the tasks
                // TODO: search tasks in correct order once we've implemented task ordering.
                let initial_working_directory = agent_conv.tasks.iter().find_map(Task::api_task_initial_working_directory);
                let credits_spent = conversation_data
                    .as_ref()
                    .and_then(|data| data.conversation_usage_metadata.as_ref())
                    .map(|m| m.credits_spent);
                let artifacts = conversation_data
                    .as_ref()
                    .and_then(|data| data.artifacts_json.as_ref())
                    .and_then(|json| serde_json::from_str(json).ok())
                    .unwrap_or_default();
                let server_conversation_token = conversation_data
                    .and_then(|data| data.server_conversation_token)
                    .map(ServerConversationToken::new);

                Some((conversation_id, AIConversationMetadata {
                    id: conversation_id,
                    title,
                    initial_query,
                    last_modified_at: agent_conv.conversation.last_modified_at,
                    initial_working_directory,
                    credits_spent,
                    // If we have a server token, the conversation was synced to cloud
                    has_cloud_data: server_conversation_token.is_some(),
                    server_conversation_token,
                    has_local_data: true,
                    artifacts,
                    // Only populated when loading from server, not from local DB
                    server_conversation_metadata: None,
                }))
            })
            .collect();

        // Populate the token → conversation reverse index alongside the
        // forward metadata map.
        for (conversation_id, metadata) in &collected {
            if let Some(token) = &metadata.server_conversation_token {
                self.server_token_to_conversation_id
                    .insert(token.clone(), *conversation_id);
            }
        }
        self.all_conversations_metadata = collected;
    }
}
