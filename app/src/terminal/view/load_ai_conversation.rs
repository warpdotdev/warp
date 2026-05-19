use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use vec1::Vec1;
use warp_core::features::FeatureFlag;
use warpui::{EntityId, ViewContext};

use super::blocklist_filter::exchanges_for_blocklist;
use crate::ai::blocklist::agent_view::{
    AgentViewEntryBlockParams, AgentViewEntryOrigin, DismissalStrategy, EphemeralMessage,
};
use crate::ai::blocklist::block::cli_controller::CLISubagentController;
use crate::ai::blocklist::history_model::{CLIAgentConversation, CloudConversationData};
use crate::ai::blocklist::BlocklistAIContextModel;
use crate::terminal::input::message_bar::Message as InputMessage;
use crate::terminal::input::message_bar::MessageItem;
use crate::terminal::model::block::SerializedBlock;
use crate::terminal::model::rich_content::RichContentType;
use crate::terminal::model_events::ModelEventDispatcher;
use crate::terminal::TerminalModel;
use crate::util::bindings::keybinding_name_to_keystroke;
use chrono::{DateTime, Local};
use itertools::Itertools;
use prost::Message;
use std::ops::Not;

use super::DEFAULT_AI_BLOCK_HEIGHT;

use crate::ai::agent::AIAgentActionResultType;
use crate::ai::agent::CreateDocumentsRequest;
use crate::ai::agent::{
    AIAgentAction, AIAgentActionType, AIAgentOutputMessage, AIAgentOutputMessageType,
    CreateDocumentsResult, EditDocumentsResult,
};
use crate::ai::ai_document_view::DEFAULT_PLANNING_DOCUMENT_TITLE;
use crate::ai::document::ai_document_model::AIDocumentModel;
use crate::{
    ai::{
        agent::{
            conversation::{AIConversation, AIConversationId},
            AIAgentExchange, AIAgentExchangeId, AIAgentOutput,
        },
        blocklist::{
            history_model::BlocklistAIHistoryModel, model::AIBlockModelImpl, AIBlock,
            BlocklistAIActionModel, BlocklistAIController, ClientIdentifiers,
        },
        get_relevant_files::controller::GetRelevantFilesController,
    },
    persistence::model::AgentConversationData,
    terminal::{
        find::TerminalFindModel,
        model::{
            blocks::RichContentItem, session::active_session::ActiveSession,
            terminal_model::BlockIndex,
        },
        view::{
            AIBlockMetadata, Event, RichContent, RichContentInsertionPosition, RichContentMetadata,
            TerminalView,
        },
    },
};
use warp_core::channel::ChannelState;
use warp_multi_agent_api as api;
use warpui::units::IntoPixels;
use warpui::{ModelHandle, SingletonEntity};

/// Describes restore-context setup state for directory reconciliation and hinting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RestorationDirState {
    /// No directory issue — terminal is already in the right place.
    Unchanged,
    /// The conversation's directory doesn't exist on this machine.
    MissingOriginalDir,
    /// The terminal needs to cd into the conversation's directory.
    NeedsCd { path: String },
}

/// Specifies how AI conversations should be restored when creating a TerminalView.
#[derive(Clone, Debug)]
pub enum ConversationRestorationInNewPaneType {
    /// Restore conversations from persistence during app startup.
    /// Contains the conversations to restore, already loaded from the database.
    /// Uses Vec1 to ensure at least one conversation is present.
    Startup {
        conversations: Vec1<AIConversation>,
        /// If set, the agent view was open in fullscreen mode for this conversation
        /// and should be restored after conversations are loaded.
        active_conversation_id: Option<AIConversationId>,
    },

    /// Load a conversation for the cloud conversation viewer or CLI.
    /// The conversation has already been converted from ConversationData.
    Historical {
        conversation: AIConversation,
        should_use_live_appearance: bool,
        /// The ambient agent task ID, if this is an ambient agent conversation.
        /// Used to display the session ended tombstone.
        ambient_agent_task_id: Option<crate::ai::ambient_agents::AmbientAgentTaskId>,
    },

    /// Fork an existing conversation into this new pane.
    /// This is like Historical but requires special persistence handling.
    Forked {
        conversation: AIConversation,
        /// True when the fork is paired with a follow-up prompt or summarize that
        /// will be sent immediately after restore.
        /// We skip the `couldn't find original conversation directory` ephemeral
        /// hint in that case so the warping indicator (gated on
        /// `ephemeral_message_model.current_message().is_none()` in
        /// `BlocklistAIStatusBar::render`) isn't suppressed by the hint.
        has_initial_query: bool,
    },

    /// Load a CLI agent conversation from its downloaded snapshot.
    HistoricalCLIAgent {
        conversation: CLIAgentConversation,
        should_use_live_appearance: bool,
    },
}

impl ConversationRestorationInNewPaneType {
    pub fn is_forked(&self) -> bool {
        matches!(self, Self::Forked { .. })
    }

    pub fn is_startup(&self) -> bool {
        matches!(self, Self::Startup { .. })
    }

    /// Whether restore-context hinting should run for this restoration mode.
    pub fn should_show_restore_context_hint(&self) -> bool {
        match self {
            Self::Startup { .. } => false,
            Self::Forked {
                has_initial_query, ..
            } => !has_initial_query,
            Self::Historical { .. } | Self::HistoricalCLIAgent { .. } => true,
        }
    }

    /// Use live appearance background color, and don't add a session restoration banner.
    pub fn should_use_live_appearance(&self) -> bool {
        match self {
            Self::Forked { .. } => true,
            Self::Historical {
                should_use_live_appearance,
                ..
            }
            | Self::HistoricalCLIAgent {
                should_use_live_appearance,
                ..
            } => FeatureFlag::AgentView.is_enabled() || *should_use_live_appearance,
            Self::Startup { .. } => false,
        }
    }

    /// Returns the initial working directory from the conversation, if available.
    pub fn initial_working_directory(&self) -> Option<String> {
        match self {
            Self::Historical { conversation, .. } | Self::Forked { conversation, .. } => {
                conversation.initial_working_directory()
            }
            Self::HistoricalCLIAgent { conversation, .. } => {
                conversation.metadata.working_directory.clone()
            }
            Self::Startup { .. } => None,
        }
    }
}

/// Parameters needed for creating and inserting AI blocks
#[derive(Debug)]
pub struct AIBlockCreationParams {
    pub ai_controller: ModelHandle<BlocklistAIController>,
    pub get_relevant_files_controller: ModelHandle<GetRelevantFilesController>,
    pub ai_action_model: ModelHandle<BlocklistAIActionModel>,
    pub ai_context_model: ModelHandle<BlocklistAIContextModel>,
    pub cli_subagent_controller: ModelHandle<CLISubagentController>,
    pub find_model: ModelHandle<TerminalFindModel>,
    pub active_session: ModelHandle<ActiveSession>,
    pub model_events_handle: ModelHandle<ModelEventDispatcher>,
    pub terminal_view_id: EntityId,
    pub height: f32,
    pub conversation_id: AIConversationId,
    pub exchange_id: AIAgentExchangeId,
    pub working_directory: Option<String>,
    /// If this is populated, we have an actual block for a requested command, and
    /// this AI block should be inserted before command_block_index.
    /// Otherwise, we don't have an actual block and should create a dummy requested command block from the exchange output.
    pub command_block_index: Option<BlockIndex>,
    /// The exchange data used to process outputs for restoring code diffs, and dummy requested command blocks if command_block_index is None.
    pub exchange: AIAgentExchange,
    /// When true, uses the live (non-restored) appearance even though the block is restored.
    /// Used for forked conversations and cloud conversation viewer.
    pub use_live_appearance: bool,

    /// Whether this block is being restored as part of conversation restoration on app startup.
    pub is_restoring_on_startup: bool,
}

/// RestoredAIConversation stores a conversation to restore and any associated data we need for
/// restoration.
pub struct RestoredAIConversation {
    pub ai_conversation: AIConversation,
}

impl RestoredAIConversation {
    pub fn new(conversation: AIConversation) -> Self {
        RestoredAIConversation {
            ai_conversation: conversation,
        }
    }
}

impl TerminalView {
    /// Determine the directory state for restoring the conversation: whether it's missing, we're
    /// already in the right directory, or we need to cd.
    fn resolve_dir_restoration_state(
        &self,
        cloud_conversation: &CloudConversationData,
    ) -> RestorationDirState {
        let target_dir = match cloud_conversation {
            CloudConversationData::Oz(conversation) => {
                conversation.initial_working_directory().or_else(|| {
                    conversation
                        .server_metadata()
                        .and_then(|metadata| metadata.working_directory.clone())
                })
            }
            CloudConversationData::CLIAgent(cli_conversation) => {
                cli_conversation.metadata.working_directory.clone()
            }
        };

        let Some(target_dir) = target_dir else {
            // If we don't have a target dir, no need to cd
            return RestorationDirState::Unchanged;
        };

        if !Path::new(&target_dir).is_dir() {
            return RestorationDirState::MissingOriginalDir;
        }

        if self.pwd().as_deref() != Some(target_dir.as_str()) {
            return RestorationDirState::NeedsCd { path: target_dir };
        }

        RestorationDirState::Unchanged
    }

    pub(crate) fn restore_conversation_and_directory_context<F>(
        &mut self,
        cloud_conversation: CloudConversationData,
        use_live_appearance: bool,
        on_restored: F,
        ctx: &mut ViewContext<Self>,
    ) where
        F: FnOnce(&mut Self, &mut ViewContext<Self>) + 'static,
    {
        let restore_context_state = self.resolve_dir_restoration_state(&cloud_conversation);

        let restore_and_continue =
            move |me: &mut TerminalView,
                  restore_dir_state: RestorationDirState,
                  ctx: &mut ViewContext<TerminalView>| {
                me.maybe_show_restore_context_hint(restore_dir_state, ctx);

                match cloud_conversation {
                    CloudConversationData::Oz(conversation) => {
                        me.restore_conversation_after_view_creation(
                            RestoredAIConversation::new(*conversation),
                            use_live_appearance,
                            ctx,
                        );
                    }
                    CloudConversationData::CLIAgent(cli_conversation) => {
                        if FeatureFlag::AgentHarness.is_enabled() {
                            me.restore_cli_agent_block_snapshot(cli_conversation.block);
                        } else {
                            log::warn!(
                                "AgentHarness flag is disabled; ignoring CLI agent block snapshot"
                            );
                        }
                    }
                }

                on_restored(me, ctx);
            };

        match restore_context_state {
            RestorationDirState::NeedsCd { path } => {
                let path_for_hint = path.clone();
                let did_execute_cd = self.input.update(ctx, |input, ctx| {
                    input.try_execute_command(&format!("cd \"{path}\""), ctx)
                });
                if did_execute_cd {
                    self.on_next_block_completed(move |me, ctx| {
                        restore_and_continue(
                            me,
                            RestorationDirState::NeedsCd {
                                path: path_for_hint,
                            },
                            ctx,
                        );
                    });
                } else {
                    restore_and_continue(self, RestorationDirState::Unchanged, ctx);
                }
            }
            RestorationDirState::Unchanged | RestorationDirState::MissingOriginalDir => {
                restore_and_continue(self, restore_context_state, ctx);
            }
        }
    }

    /// Inserts a CLI agent block snapshot into the terminal model.
    ///
    /// CLI agent conversations are represented by a harness-specific transcript and
    /// a snapshot of the block contents. When restoring a CLI agent conversation, we
    /// display the block snapshot as if it were restored session contents.
    fn restore_cli_agent_block_snapshot(&mut self, block: SerializedBlock) {
        self.model
            .lock()
            .block_list_mut()
            .insert_restored_block(&block);
    }

    /// Restore AI documents from exchanges by processing CreateDocuments and EditDocuments actions.
    /// This ensures documents are available before AI blocks are rendered.
    fn restore_ai_documents_from_exchanges(
        &self,
        exchanges: &[&AIAgentExchange],
        conversation_id: AIConversationId,
        ctx: &mut ViewContext<Self>,
    ) {
        let document_model = AIDocumentModel::handle(ctx);

        for exchange in exchanges {
            if let Some(output) = exchange.output_status.output() {
                for message in &output.get().messages {
                    if let AIAgentOutputMessage {
                        message: AIAgentOutputMessageType::Action(action),
                        ..
                    } = message
                    {
                        match &action.action {
                            AIAgentActionType::CreateDocuments(CreateDocumentsRequest {
                                documents,
                            }) => {
                                if let Some(result) =
                                    self.ai_action_model.read(ctx, |action_model, _| {
                                        action_model.get_action_result(&action.id).cloned()
                                    })
                                {
                                    if let AIAgentActionResultType::CreateDocuments(
                                        CreateDocumentsResult::Success { created_documents },
                                    ) = &result.result
                                    {
                                        // Create a mapping from document index to title
                                        let document_titles: Vec<String> =
                                            documents.iter().map(|doc| doc.title.clone()).collect();

                                        document_model.update(ctx, |doc_model, doc_ctx| {
                                            for (index, doc_context) in
                                                created_documents.iter().enumerate()
                                            {
                                                let title = document_titles
                                                    .get(index)
                                                    .cloned()
                                                    .unwrap_or_else(|| {
                                                        DEFAULT_PLANNING_DOCUMENT_TITLE.to_string()
                                                    });

                                                doc_model.restore_document(
                                                    doc_context.document_id,
                                                    conversation_id,
                                                    title,
                                                    doc_context.content.clone(),
                                                    exchange.start_time,
                                                    doc_ctx,
                                                );
                                            }
                                        });
                                    }
                                }
                            }
                            AIAgentActionType::EditDocuments { .. } => {
                                if let Some(result) =
                                    self.ai_action_model.read(ctx, |action_model, _| {
                                        action_model.get_action_result(&action.id).cloned()
                                    })
                                {
                                    if let AIAgentActionResultType::EditDocuments(
                                        EditDocumentsResult::Success { updated_documents },
                                    ) = &result.result
                                    {
                                        document_model.update(ctx, |doc_model, doc_ctx| {
                                            for doc_context in updated_documents {
                                                doc_model.restore_document_edit(
                                                    &doc_context.document_id,
                                                    doc_context.content.clone(),
                                                    exchange.start_time,
                                                    doc_ctx,
                                                );
                                            }
                                        });
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    /// Restore conversations from a list of AIBlockCreationParams. If the conversation has more exchanges
    /// than AIBlockCreationParams, (which can happen if we reach the max number of persisted ai blocks),
    /// we still only restore the blocks provided in ai_block_params.
    ///
    /// The active conversation id is set to the conversation id of the last AI block being restored.
    fn restore_conversations_from_block_params(
        &mut self,
        ai_block_params: Vec<AIBlockCreationParams>,
        restored_conversations: Vec<RestoredAIConversation>,
        ctx: &mut ViewContext<Self>,
    ) -> usize {
        let conversations: Vec<AIConversation> = restored_conversations
            .into_iter()
            .map(|r| r.ai_conversation)
            .collect();

        for conversation in &conversations {
            self.ai_action_model.update(ctx, |action_model, _ctx| {
                action_model
                    .restore_action_results_from_exchanges(exchanges_for_blocklist(conversation));
            });
        }

        // Restore AI documents for each conversation
        for conversation in &conversations {
            let conversation_id = conversation.id();
            let exchanges = exchanges_for_blocklist(conversation);
            self.restore_ai_documents_from_exchanges(&exchanges, conversation_id, ctx);
        }

        // Determine the active conversation id from the last AI block being restored
        let active_conversation_id = ai_block_params.last().map(|params| params.conversation_id);
        let is_restoring_on_startup = ai_block_params
            .last()
            .is_some_and(|params| params.is_restoring_on_startup);

        // Store conversations in the history model (with correct cancellation statuses)
        BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, ctx| {
            history_model.restore_conversations(self.view_id, conversations, ctx);
            if let Some(active_conversation_id) = active_conversation_id {
                // Use `mark_active_conversation_id` (non-transferring) so we
                // don't rip the conversation out of any other terminal view's
                // live list. This matters for the orchestration pill bar's
                // in-place navigation: the child agent's hidden pane must
                // retain ownership so its `conversation_id_for_action`
                // lookups for in-flight requested commands keep resolving.
                // `restore_conversation_after_view_creation` decides after
                // restoration whether the current view should become the
                // canonical owner (regular terminal panes) or remain a
                // non-owning mirror (conversation transcript viewers).
                history_model.mark_active_conversation_id(
                    active_conversation_id,
                    self.view_id,
                    ctx,
                );
            }
        });

        // If `AgentView` is enabled and we're restoring conversations on startup (as opposed to
        // loading a conversation due to selection from the command palette), then we don't eagerly
        // set the pending query state (which is equivalent to _entering_ the agent view when the
        // FeatureFlag is enabled).
        if !FeatureFlag::AgentView.is_enabled() || !is_restoring_on_startup {
            // Set agent pending state for follow-up if we have an active conversation
            if let Some(conversation_id) = active_conversation_id {
                let origin = AgentViewEntryOrigin::RestoreExistingConversation;
                self.ai_context_model.update(ctx, |context_model, ctx| {
                    context_model.set_pending_query_state_for_existing_conversation(
                        conversation_id,
                        origin,
                        ctx,
                    );
                });
            }
        }

        // Track which conversations have had their agent view blocks inserted
        let mut conversations_with_agent_view_block = std::collections::HashSet::new();

        // Create AI blocks. Note this must happen after restoring action results in the action model,
        // because AI block creation relies on the action result for an action existing in order to determine
        // what the state should be.
        let blocks_created = ai_block_params.len();
        for params in ai_block_params {
            let conversation_id = params.conversation_id;
            let command_block_index = params.command_block_index;

            if FeatureFlag::AgentView.is_enabled()
                && params.is_restoring_on_startup
                && !conversations_with_agent_view_block.contains(&conversation_id)
            {
                // Insert an agent view block before the first AI block of each conversation.
                // Use the same insertion position as the AI block (based on command_block_index)
                // so they stay together.
                conversations_with_agent_view_block.insert(conversation_id);

                let position = match command_block_index {
                    Some(idx) => RichContentInsertionPosition::BeforeBlockIndex(idx),
                    None => RichContentInsertionPosition::Append {
                        insert_below_long_running_block: false,
                    },
                };
                self.insert_agent_view_entry_block(
                    AgentViewEntryBlockParams {
                        conversation_id,
                        is_new: false,
                        is_restored: true,
                        origin: AgentViewEntryOrigin::RestoreExistingConversation,
                        agent_view_controller: self.agent_view_controller.clone(),
                    },
                    position,
                    ctx,
                );
            }

            self.create_and_insert_ai_block(params, ctx);
        }

        // Restored command blocks are inserted into the block list by
        // `create_and_insert_ai_block` → `process_restored_outputs` without going through the
        // normal `BlockCompleted` event path that initialises mouse states. Ensure every block
        // index in the block list has mouse states so the label hover tooltip, bookmark button,
        // and filter button work for these blocks.
        self.ensure_mouse_states_for_all_blocks();

        blocks_created
    }

    /// Restore a conversation using the stored exchanges for said conversation.
    /// This is used for opening a historical conversation from the agent mode homepage, and
    /// when loading from a debug link.
    pub fn restore_conversation_after_view_creation(
        &mut self,
        restored: RestoredAIConversation,
        use_live_appearance: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        let conversation_id = restored.ai_conversation.id();
        log::info!("Restoring conversation after view creation: {conversation_id}",);

        // Insert command blocks into the existing blocklist first, mirroring the
        // quit/restart path where blocks are passed to the TerminalModel constructor.
        // This ensures command_block_indices_for_exchanges finds them and AI blocks
        // get command_block_index = Some(...), so process_restored_outputs doesn't
        // need to create blocks itself.
        let serialized_items = restored.ai_conversation.to_serialized_blocklist_items();
        if !serialized_items.is_empty() {
            let mut model = self.model.lock();
            let block_list = model.block_list_mut();
            for item in &serialized_items {
                match item {
                    crate::ai::blocklist::SerializedBlockListItem::Command { block } => {
                        block_list.insert_restored_block(block);
                    }
                }
            }
        }

        // Calculate height for AI blocks
        let size_info = *self.size_info;
        let height = DEFAULT_AI_BLOCK_HEIGHT
            .into_pixels()
            .to_lines(size_info.cell_height_px());

        // Now compute block indices — the just-inserted command blocks will be found.
        let exchanges = exchanges_for_blocklist(&restored.ai_conversation);
        let command_block_indices = {
            let terminal_model = self.model.lock();
            command_block_indices_for_exchanges(
                &terminal_model,
                exchanges.iter().copied(),
                exchanges.len(),
            )
        };

        // Process all exchanges for this conversation
        let mut all_ai_block_params = Vec::new();
        for (exchange, command_block_index) in exchanges.into_iter().zip(command_block_indices) {
            let params = AIBlockCreationParams {
                ai_controller: self.ai_controller.clone(),
                get_relevant_files_controller: self.get_relevant_files_controller.clone(),
                ai_action_model: self.ai_action_model.clone(),
                ai_context_model: self.ai_context_model.clone(),
                cli_subagent_controller: self.cli_subagent_controller.clone(),
                model_events_handle: self.model_events_handle.clone(),
                find_model: self.find_model.clone(),
                active_session: self.active_session.clone(),
                terminal_view_id: self.view_id,
                height: height.as_f64() as f32,
                conversation_id,
                exchange_id: exchange.id,
                working_directory: exchange.working_directory.clone(),
                command_block_index,
                exchange: (*exchange).clone(),
                use_live_appearance,
                is_restoring_on_startup: false,
            };

            all_ai_block_params.push(params);
        }

        // Restore action results from all exchanges
        let blocks_created =
            self.restore_conversations_from_block_params(all_ai_block_params, vec![restored], ctx);

        if !BlocklistAIHistoryModel::as_ref(ctx)
            .is_terminal_view_conversation_transcript_viewer(self.view_id)
        {
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, ctx| {
                history_model.set_active_conversation_id(conversation_id, self.view_id, ctx);
            });
        }

        log::info!(
            "Successfully restored {blocks_created} AI blocks for conversation: {conversation_id}"
        );
    }

    /// Restore AI conversations and create AI blocks from exchanges.
    /// This is called when restoring conversations in a new terminal pane.
    /// In this case, we expect shell command blocks to already exist in the terminal model, since they must
    /// be restored before bootstrapping finishes.
    /// Then we need to order the AI blocks correctly relative to shell commands that exist in the model.
    pub(super) fn restore_conversations_on_view_creation(
        &mut self,
        conversation_restoration: ConversationRestorationInNewPaneType,
        ctx: &mut ViewContext<Self>,
    ) {
        // We don't want blocks to appear as restored for forked conversations
        // and conversations in the cloud conversation viewer.
        let use_live_appearance = conversation_restoration.should_use_live_appearance();
        let is_fork_conversation_in_new_pane = conversation_restoration.is_forked();
        let is_startup = conversation_restoration.is_startup();
        let should_show_restore_context_hint =
            conversation_restoration.should_show_restore_context_hint();

        // Save the target working directory so we can detect when the dir doesn't exist on this machine.
        let target_dir = conversation_restoration.initial_working_directory();

        // Extract the active conversation ID if agent view was open (only for startup restoration)
        let active_conversation_id_to_restore = match &conversation_restoration {
            ConversationRestorationInNewPaneType::Startup {
                active_conversation_id,
                ..
            } => *active_conversation_id,
            _ => None,
        };

        // Extract restored conversations from restoration type
        let restored_conversations: Vec<RestoredAIConversation> = match conversation_restoration {
            ConversationRestorationInNewPaneType::Startup { conversations, .. } => conversations
                .into_iter()
                .map(RestoredAIConversation::new)
                .collect(),
            ConversationRestorationInNewPaneType::Historical { conversation, .. } => {
                vec![RestoredAIConversation::new(conversation)]
            }
            ConversationRestorationInNewPaneType::Forked { conversation, .. } => {
                vec![RestoredAIConversation::new(conversation)]
            }
            ConversationRestorationInNewPaneType::HistoricalCLIAgent { conversation, .. } => {
                if FeatureFlag::AgentHarness.is_enabled() {
                    self.restore_cli_agent_block_snapshot(conversation.block);
                }
                return;
            }
        };
        if restored_conversations.is_empty() {
            return;
        }
        let conversation_ids = restored_conversations
            .iter()
            .map(|r| r.ai_conversation.id())
            .collect::<Vec<_>>();
        log::info!(
            "Restoring {} conversations on view creation: {:?}",
            restored_conversations.len(),
            conversation_ids
        );

        // Calculate height for AI blocks
        let size_info = *self.size_info;
        let height = DEFAULT_AI_BLOCK_HEIGHT
            .into_pixels()
            .to_lines(size_info.cell_height_px());

        // Construct list of AIBlockCreationParams before moving conversations into history model
        let mut all_exchanges_with_conversation_ids = Vec::new();

        // Collect all exchanges from all conversations
        for restored in &restored_conversations {
            let conversation_id = restored.ai_conversation.id();

            let exchanges = exchanges_for_blocklist(&restored.ai_conversation);

            for exchange in exchanges {
                all_exchanges_with_conversation_ids.push((exchange.clone(), conversation_id));
            }
        }

        // Sort by timestamp to prepare for batch block index lookup
        all_exchanges_with_conversation_ids.sort_by_key(|(exchange, _)| exchange.start_time);

        // Compute all block indices based on the restoration type
        let command_block_indices = {
            let terminal_model = self.model.lock();
            let exchange_count = all_exchanges_with_conversation_ids.len();
            command_block_indices_for_exchanges(
                &terminal_model,
                all_exchanges_with_conversation_ids
                    .iter()
                    .map(|(exchange, _)| exchange),
                exchange_count,
            )
        };

        // Create AIBlockCreationParams with the computed indices
        let all_ai_block_params: Vec<AIBlockCreationParams> = all_exchanges_with_conversation_ids
            .into_iter()
            .zip(command_block_indices)
            .map(
                |((exchange, conversation_id), command_block_index)| AIBlockCreationParams {
                    ai_controller: self.ai_controller.clone(),
                    get_relevant_files_controller: self.get_relevant_files_controller.clone(),
                    ai_action_model: self.ai_action_model.clone(),
                    ai_context_model: self.ai_context_model.clone(),
                    cli_subagent_controller: self.cli_subagent_controller.clone(),
                    model_events_handle: self.model_events_handle.clone(),
                    find_model: self.find_model.clone(),
                    active_session: self.active_session.clone(),
                    terminal_view_id: self.view_id,
                    height: height.as_f64() as f32,
                    conversation_id,
                    exchange_id: exchange.id,
                    working_directory: exchange.working_directory.clone(),
                    command_block_index,
                    exchange,
                    use_live_appearance,
                    is_restoring_on_startup: is_startup,
                },
            )
            .collect();

        let blocks_created = self.restore_conversations_from_block_params(
            all_ai_block_params,
            restored_conversations,
            ctx,
        );

        if is_fork_conversation_in_new_pane {
            for conversation_id in &conversation_ids {
                self.persist_blocks_for_forked_conversation(*conversation_id, ctx);
            }
        }

        // Show a contextual ephemeral hint when the restored conversation's
        // directory or branch doesn't match the current terminal state.
        if should_show_restore_context_hint {
            let restore_context_state =
                if target_dir.as_ref().is_some_and(|d| !Path::new(d).is_dir()) {
                    RestorationDirState::MissingOriginalDir
                } else {
                    RestorationDirState::Unchanged
                };

            self.maybe_show_restore_context_hint(restore_context_state, ctx);
        }

        log::info!(
            "Successfully restored {blocks_created} AI blocks on view creation for conversations: {conversation_ids:?}"
        );

        // If agent view was open before the session was saved, restore it
        if FeatureFlag::AgentView.is_enabled() {
            if let Some(conversation_id) = active_conversation_id_to_restore {
                // Check if the conversation was successfully restored
                let conversation_exists = BlocklistAIHistoryModel::handle(ctx)
                    .as_ref(ctx)
                    .conversation(&conversation_id)
                    .is_some();

                if conversation_exists {
                    log::info!("Restoring agent view for conversation: {conversation_id}");
                    self.enter_agent_view_for_conversation(
                        None,
                        AgentViewEntryOrigin::RestoreExistingConversation,
                        conversation_id,
                        ctx,
                    );
                } else {
                    log::warn!(
                        "Cannot restore agent view: conversation {conversation_id} not found"
                    );
                }
            }
        }
    }

    /// When we fork a conversation, we copy all of the ai and terminal blocks that were part of the original conversation.
    /// Because these new blocks were not created through the normal conversation flow, they were never persisted to the database.
    /// To fix this, we manually emit completed events for these ai and terminal blocks so that they are persisted correctly.
    fn persist_blocks_for_forked_conversation(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ViewContext<Self>,
    ) {
        // Persist newly created AI blocks for this forked conversation.
        BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, ctx| {
            history_model.on_forked_conversation(conversation_id, self.view_id, ctx);
        });

        let model = self.model.lock();
        model
            .block_list()
            .blocks()
            .iter()
            .filter(|b| b.is_restored() && !b.is_background())
            .map(|b| Arc::new(SerializedBlock::from(b)))
            .for_each(|block| {
                ctx.emit(Event::BlockCompleted {
                    is_local: self.is_block_considered_remote(block.session_id, None, ctx),
                    block,
                });
            });
    }

    /// Show a contextual ephemeral hint when the restored conversation's
    /// directory doesn't match the current terminal state.
    pub(crate) fn maybe_show_restore_context_hint(
        &mut self,
        restore_context_state: RestorationDirState,
        ctx: &mut ViewContext<Self>,
    ) {
        if !FeatureFlag::InlineRepoMenu.is_enabled() {
            return;
        }

        let open_repo_hint: MessageItem =
            if let Some(keystroke) = keybinding_name_to_keystroke("/open-repo", ctx) {
                MessageItem::keystroke(keystroke)
            } else {
                MessageItem::text("/open-repo")
            };

        // Build the message from dir state
        let mut items: Vec<MessageItem> = Vec::new();

        match &restore_context_state {
            RestorationDirState::MissingOriginalDir => {
                items.push(MessageItem::text(
                    "couldn't find original conversation directory ",
                ));
                items.push(open_repo_hint.clone());
                items.push(MessageItem::text(" change repos"));
            }
            RestorationDirState::NeedsCd { .. } => {
                items.push(MessageItem::text(
                    "changed directory to continue conversation ",
                ));
                items.push(open_repo_hint.clone());
                items.push(MessageItem::text(" change repos"));
            }
            RestorationDirState::Unchanged => {}
        }

        if !items.is_empty() {
            let message = InputMessage::new(items);
            self.ephemeral_message_model.update(ctx, |model, ctx| {
                model.show_ephemeral_message(
                    EphemeralMessage::new(message, DismissalStrategy::UntilExplicitlyDismissed),
                    ctx,
                );
            });
        }
    }

    /// Process code diffs from AI output messages and apply them to the AI block for rendering.
    /// Command blocks should already exist in the blocklist (extracted via
    /// `conversation.to_serialized_blocklist_items`) before AI blocks are created.
    fn process_restored_outputs(
        &mut self,
        ai_block: &mut AIBlock,
        output: &AIAgentOutput,
        ctx: &mut ViewContext<AIBlock>,
    ) {
        for message in &output.messages {
            if let AIAgentOutputMessage {
                message:
                    AIAgentOutputMessageType::Action(AIAgentAction {
                        action: AIAgentActionType::RequestFileEdits { file_edits, .. },
                        id,
                        ..
                    }),
                ..
            } = message
            {
                ai_block.set_restored_file_edits(id, file_edits.clone(), ctx);
            }
        }
    }

    /// Load a conversation from AI tasks, converting them to exchanges and creating
    /// the necessary AI blocks in the terminal view.
    pub fn load_conversation_from_tasks(
        &mut self,
        task_list: api::ConversationData,
        ctx: &mut ViewContext<Self>,
    ) {
        let tasks = task_list.tasks;
        if tasks.is_empty() {
            log::warn!("No tasks provided - conversation will be empty");
            return;
        }

        // Create a conversation - exchanges are computed internally from tasks
        let conversation_id = AIConversationId::new();

        let conversation_data = AgentConversationData {
            server_conversation_token: None,
            conversation_usage_metadata: None,
            reverted_action_ids: None,
            forked_from_server_conversation_token: None,
            artifacts_json: None,
            parent_agent_id: None,
            agent_name: None,
            orchestration_harness_type: None,
            parent_conversation_id: None,
            is_remote_child: false,
            run_id: None,
            autoexecute_override: None,
            last_event_sequence: None,
            pinned: false,
        };

        match AIConversation::new_restored(conversation_id, tasks, Some(conversation_data)) {
            Ok(conversation) => {
                // Use live appearance for cloud conversation viewer
                self.restore_conversation_after_view_creation(
                    RestoredAIConversation::new(conversation),
                    true,
                    ctx,
                );
            }
            Err(e) => {
                log::error!("Failed to load conversation from tasks: {e:?}");
            }
        }
    }

    /// The exchange is processed to create dummy shell command blocks and code diff views.
    pub fn create_and_insert_ai_block(
        &mut self,
        params: AIBlockCreationParams,
        ctx: &mut ViewContext<Self>,
    ) {
        // Extract values we need before fields get moved
        let conversation_id = params.conversation_id;
        let exchange_id = params.exchange_id;
        let command_block_index = params.command_block_index;
        let exchange = params.exchange;

        let ai_block_model = match AIBlockModelImpl::<AIBlock>::new(
            exchange_id,
            conversation_id,
            true,
            params.use_live_appearance,
            ctx,
        ) {
            Ok(ai_block_model) => ai_block_model,
            Err(err) => {
                log::warn!("AI block creation failed. {err}");
                return;
            }
        };

        let shell_launch_data = params.active_session.as_ref(ctx).shell_launch_data(ctx);

        let restored_block_view_handle = ctx.add_typed_action_view(|ctx| {
            AIBlock::new(
                Rc::new(ai_block_model),
                self.model.clone(),
                ClientIdentifiers {
                    conversation_id: params.conversation_id,
                    client_exchange_id: params.exchange_id,
                    response_stream_id: None,
                },
                params.ai_controller,
                params.get_relevant_files_controller,
                params.working_directory,
                shell_launch_data,
                params.ai_action_model,
                params.ai_context_model,
                params.find_model,
                params.active_session,
                &params.cli_subagent_controller,
                &params.model_events_handle,
                self.agent_view_controller.clone(),
                self.ambient_agent_view_model.clone(),
                self.view_handle.clone(),
                params.terminal_view_id,
                ctx,
            )
        });

        // Register the restored block with the find model so it can be searched with find.
        let restored_block_clone = restored_block_view_handle.clone();
        self.find_model.update(ctx, |find_model, _ctx| {
            find_model.register_findable_rich_content_view(restored_block_clone);
        });

        // Insert into block list if command_block_index is provided
        let item = RichContentItem::new(
            Some(RichContentType::AIBlock),
            restored_block_view_handle.id(),
            FeatureFlag::AgentView
                .is_enabled()
                .then_some(conversation_id),
            FeatureFlag::AgentView.is_enabled()
                && self
                    .agent_view_controller
                    .as_ref(ctx)
                    .agent_view_state()
                    .active_conversation_id()
                    .is_some_and(|id| id == conversation_id),
        );
        if let Some(cmd_block_index) = command_block_index {
            self.model
                .lock()
                .block_list_mut()
                .insert_rich_content_before_block_index(item, cmd_block_index);
        } else {
            self.model
                .lock()
                .block_list_mut()
                .append_rich_content(item, false);
        }

        let rich_content =
            RichContent::new(restored_block_view_handle.clone(), Some(conversation_id))
                .with_metadata(RichContentMetadata::AIBlock(AIBlockMetadata {
                    exchange_id,
                    conversation_id,
                    ai_block_handle: restored_block_view_handle.clone(),
                }));

        self.rich_content_views.push(rich_content);

        // Process restored outputs: handle code diffs and create shell command blocks
        // We need to call this after inserting the rich content in the blocklist so the requested commands are in the right position.
        if let Some(output) = exchange.output_status.output() {
            // Process code diffs for the AI block
            let ai_block_handle = restored_block_view_handle.clone();
            ai_block_handle.update(ctx, |ai_block, block_ctx| {
                self.process_restored_outputs(ai_block, &output.get(), block_ctx);
            });
        }

        ctx.subscribe_to_view(&restored_block_view_handle, |me, block, event, ctx| {
            me.handle_ai_block_event(
                block.clone(),
                true, // is_restored
                event,
                ctx,
            );
        });
    }

    /// Loads an agent mode conversation from a debug link in the clipboard.
    /// This is used for debugging purposes only when in dogfood channel state.
    pub fn load_agent_mode_conversation(&mut self, ctx: &mut ViewContext<Self>) {
        if !ChannelState::channel().is_dogfood() {
            return;
        }

        let content = ctx.clipboard().read();
        let Some(debug_link) = content
            .paths
            .and_then(|paths| paths.into_iter().exactly_one().ok())
            .or(content
                .plain_text
                .is_empty()
                .not()
                .then_some(content.plain_text))
        else {
            log::error!("Clipboard contents are not a conversation debug link");
            return;
        };

        // Parse the debug link and construct the protobuf URL
        let proto_url = if debug_link.contains("/debug/maa/") {
            // Split URL into base and query params
            let mut parts = debug_link.splitn(2, '?');
            let base_url = parts.next().unwrap_or(&debug_link);
            let query_params = parts.next();

            let clean_url = base_url.trim_end_matches('/');
            let mut url = format!("{clean_url}/raw/final_tasks.pb");

            // Preserve all query parameters if present
            if let Some(params) = query_params {
                url.push_str(&format!("?{params}"));
            }

            url
        } else {
            log::error!(
                "Invalid debug link format. Expected format: http://host/debug/maa/conversation-id"
            );
            return;
        };

        log::info!("Downloading conversation data from: {proto_url}");

        // Download the protobuf data
        ctx.spawn(
            async move {
                let client = http_client::Client::new();
                let response = client
                    .get(&proto_url)
                    .header("Accept", "application/protobuf")
                    .send()
                    .await?;

                if !response.status().is_success() {
                    return Err(anyhow::anyhow!("HTTP {}", response.status()));
                }

                let proto_bytes = response.bytes().await?;
                log::debug!("Downloaded {} bytes from debug link", proto_bytes.len());
                let task_list =
                    api::ConversationData::decode(proto_bytes.as_ref()).map_err(|e| {
                        anyhow::anyhow!(
                            "Failed to decode protobuf (size: {} bytes): {}",
                            proto_bytes.len(),
                            e
                        )
                    })?;

                Ok(task_list)
            },
            |terminal_view, task_list_result, ctx| match task_list_result {
                Ok(task_list) => {
                    log::info!(
                        "Successfully downloaded and parsed conversation data with {} tasks",
                        task_list.tasks.len()
                    );
                    terminal_view.load_conversation_from_tasks(task_list, ctx);
                }
                Err(err) => {
                    log::warn!("Failed to download conversation data from debug link: {err}");
                }
            },
        );
    }
}

/// Returns block indices where `AIBlock`s created for the given `exchanges` should be inserted.
///
/// For each exchange, finds the first command block whose timestamp is after the exchange's
/// start time. The AI block should be inserted before that command block.
///
/// Blocks in the blocklist may not be sorted by timestamp (e.g. when `insert_restored_block`
/// appends blocks at the end during `restore_conversation_after_view_creation`), so we scan
/// all command blocks for each exchange rather than relying on a forward-only cursor.
///
/// The returned vec is guaranteed to have `exchange_count` len.
fn command_block_indices_for_exchanges<'a>(
    terminal_model: &TerminalModel,
    exchanges: impl Iterator<Item = &'a AIAgentExchange>,
    _exchange_count: usize,
) -> Vec<Option<BlockIndex>> {
    let blocks = terminal_model.block_list().blocks();

    // Collect shell command blocks with their timestamps
    let command_blocks: Vec<(BlockIndex, DateTime<Local>)> = blocks
        .iter()
        .enumerate()
        .filter_map(|(index, block)| {
            // Only consider shell command blocks (not background/rich content), and only if they have a start timestamp
            if !block.is_background() {
                block.start_ts().map(|ts| (BlockIndex::from(index), *ts))
            } else {
                None
            }
        })
        .collect();

    let exchange_timestamps: Vec<DateTime<Local>> =
        exchanges.map(|exchange| exchange.start_time).collect();
    find_block_indices_for_exchange_timestamps(&command_blocks, &exchange_timestamps)
}

/// Pure implementation of the block-index search used by
/// [`command_block_indices_for_exchanges`].
///
/// For each exchange timestamp, finds the command block with the smallest timestamp
/// that is >= the exchange timestamp. Iterates the command block list backwards
/// because `insert_restored_block` appends blocks at the end of the blocklist, so
/// the tail is a sorted group of conversation blocks while the prefix contains
/// pre-existing terminal blocks we don't want to match against. The backwards scan
/// tracks the best candidate and stops as soon as it sees a block before the exchange
/// timestamp — within the sorted tail, everything earlier is also <=.
fn find_block_indices_for_exchange_timestamps(
    command_blocks: &[(BlockIndex, DateTime<Local>)],
    exchange_timestamps: &[DateTime<Local>],
) -> Vec<Option<BlockIndex>> {
    let mut result = Vec::with_capacity(exchange_timestamps.len());

    for &exchange_timestamp in exchange_timestamps {
        let mut best: Option<(BlockIndex, DateTime<Local>)> = None;
        for &(idx, ts) in command_blocks.iter().rev() {
            if ts >= exchange_timestamp {
                // This block is at or after the exchange; remember it if it's
                // the earliest such block we've seen. We use >= because a
                // command block's start_ts comes from the tool call message
                // timestamp, which is in the same exchange whose start_time
                // we're comparing against.
                // When timestamps tie, keep moving backwards so we pick the
                // earliest inserted block for that timestamp.
                if best.is_none_or(|(best_idx, best_ts)| {
                    ts < best_ts || (ts == best_ts && idx < best_idx)
                }) {
                    best = Some((idx, ts));
                }
            } else {
                // We've reached a block before the exchange timestamp.
                // The tail (conversation blocks) is sorted, so all earlier
                // blocks in the tail are also <= exchange_timestamp. Any
                // blocks further back belong to the pre-existing prefix
                // and should not be matched.
                break;
            }
        }

        result.push(best.map(|(idx, _)| idx));
    }

    result
}

#[cfg(test)]
#[path = "load_ai_conversation_tests.rs"]
mod tests;
