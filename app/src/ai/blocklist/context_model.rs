//! This module contains state management logic for pending context, where "pending context"
//! is defined as additional context to be attached to the next AI query.

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
};

use crate::ai::{
    agent::{AnyFileContent, FileContext},
    block_context::BlockContext,
};

use super::agent_view::{AgentViewController, AgentViewEntryOrigin, EnterAgentViewError};
use ai::project_context::model::ProjectContextModel;
use parking_lot::FairMutex;
use warp_core::features::FeatureFlag;
use warpui::{AppContext, Entity, EntityId, ModelContext, ModelHandle, SingletonEntity};

use crate::ai::agent::conversation::{AIConversationAutoexecuteMode, ConversationStatus};
use crate::{
    ai::{
        agent::todos::AIAgentTodoList,
        agent::{
            conversation::{AIConversation, AIConversationId},
            AIAgentAttachment, AIAgentContext, ImageContext,
        },
        document::ai_document_model::AIDocumentId,
        llms::{LLMPreferences, LLMPreferencesEvent},
        outline::RepoOutlines,
    },
    terminal::{
        event::{BlockCompletedEvent, BlockType},
        model::{block::BlockId, session::Sessions},
        model_events::{ModelEvent, ModelEventDispatcher},
        TerminalModel,
    },
    workspaces::user_workspaces::UserWorkspaces,
};

use super::{
    block::DirectoryContext, history_model::BlocklistAIHistoryModel, BlocklistAIHistoryEvent,
};

/// A non-image file picked via the "attach file" button, stored until query submission.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingFile {
    pub file_name: String,
    pub file_path: PathBuf,
    pub mime_type: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttachmentType {
    Image,
    File,
}

/// A pending attachment — either an image (base64 in memory) or a file (path reference).
#[derive(Clone, Debug)]
pub enum PendingAttachment {
    Image(ImageContext),
    File(PendingFile),
}

impl PendingAttachment {
    pub fn file_name(&self) -> &str {
        match self {
            PendingAttachment::Image(img) => &img.file_name,
            PendingAttachment::File(file) => &file.file_name,
        }
    }

    pub fn attachment_type(&self) -> AttachmentType {
        match self {
            PendingAttachment::Image(_) => AttachmentType::Image,
            PendingAttachment::File(_) => AttachmentType::File,
        }
    }
}

/// The state the pending query is in.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum PendingQueryState {
    /// The next query will continue an existing conversation.
    Existing { conversation_id: AIConversationId },
    New {
        /// Autoexecute override for the new conversation to be started.
        autoexecute_override: AIConversationAutoexecuteMode,
    },
}

impl Default for PendingQueryState {
    fn default() -> Self {
        Self::New {
            autoexecute_override: AIConversationAutoexecuteMode::default(),
        }
    }
}

impl PendingQueryState {
    pub fn targets_existing_conversation(&self) -> bool {
        matches!(self, PendingQueryState::Existing { .. })
    }
}

/// Model responsible for keeping track of session context to be attached to the next AI query.
pub struct BlocklistAIContextModel {
    terminal_model: Arc<FairMutex<TerminalModel>>,
    directory_context: DirectoryContext,

    /// `BlockId`s corresponding to blocks to be included as context with the next AI query.
    pending_context_block_ids: HashSet<BlockId>,

    /// Selected text to be included as context with the next AI query.
    pending_context_selected_text: Option<String>,

    /// Images and files to be included as attachments with the next AI query.
    pending_attachments: Vec<PendingAttachment>,

    /// Storage for diff hunk attachments that can be referenced in queries
    pending_inline_diff_hunk_attachments: HashMap<String, AIAgentAttachment>,

    /// The pending query could be new, which means it starts a new conversation, or follow-up, which means
    /// it continues the selected conversation.
    ///
    /// Note that this is intentionally decoupled from the active conversation in the HistoryModel.
    /// The active conversation (the one that agent outputs are being streamed to) can be different from the
    /// conversation we're following up in for the next query.
    pending_query_state: PendingQueryState,

    /// The ID of the terminal view this controller is associated with.
    terminal_view_id: EntityId,

    /// AI document ID to be included as context with the next AI query.
    /// When set, the document content will be attached as plain text context.
    pending_document_id: Option<AIDocumentId>,

    agent_view_controller: ModelHandle<AgentViewController>,

    /// Block IDs of user-executed commands to be auto-attached as context.
    /// When `AgentViewBlockContext` is enabled, completed user commands are tracked here
    /// and automatically included as context with the next user query.
    auto_attached_agent_view_user_block_ids: Vec<BlockId>,

    /// When true, submitting a prompt while the agent is responding will queue it
    /// instead of sending it immediately.
    /// Persists across exchanges in the same conversation (like fast-forward).
    queue_next_prompt_enabled: bool,
}

pub fn block_context_from_terminal_model(
    terminal_model: &TerminalModel,
    block_id: &BlockId,
    is_auto_attached: bool,
) -> Option<BlockContext> {
    let block = terminal_model
        .block_list()
        .block_index_for_id(block_id)
        .and_then(|block_id| terminal_model.block_list().block_at(block_id))?;

    // Note, if the user has explicitly asked Agent Mode to include a block as context, we do NOT
    // _force_ secrets to be obfuscated. It will respect the user's settings for secret redaction.
    let output = block.output_grid().content_summary(5000, 5000, false);

    Some(BlockContext {
        id: block_id.clone(),
        index: block.index(),
        command: block.command_to_string(),
        output,
        exit_code: block.exit_code(),
        is_auto_attached,
        started_ts: block.start_ts().cloned(),
        finished_ts: block.completed_ts().cloned(),
        pwd: None,
        shell: None,
        username: None,
        hostname: None,
        git_branch: None,
        os: None,
        session_id: None,
    })
}

impl BlocklistAIContextModel {
    pub fn new(
        sessions: ModelHandle<Sessions>,
        model_event_dispatcher: &ModelHandle<ModelEventDispatcher>,
        terminal_model: Arc<FairMutex<TerminalModel>>,
        terminal_view_id: EntityId,
        agent_view_controller: ModelHandle<AgentViewController>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        ctx.subscribe_to_model(model_event_dispatcher, move |me, event, ctx| match event {
            ModelEvent::BlockCompleted(BlockCompletedEvent {
                block_type: BlockType::User(user_block_completed),
                block_id,
                ..
            }) => {
                // If AgentViewBlockContext is enabled and we're in agent view, track user-executed
                // blocks for auto-attachment as context.
                if FeatureFlag::AgentViewBlockContext.is_enabled()
                    && me.agent_view_controller.as_ref(ctx).is_fullscreen()
                    && !user_block_completed.was_part_of_agent_interaction
                {
                    me.auto_attached_agent_view_user_block_ids
                        .push(block_id.clone());
                }

                // If the block that finished was part of an agent interaction (i.e. LRC finishing),
                // we should preserve input context.
                if !FeatureFlag::AgentViewBlockContext.is_enabled()
                    && !user_block_completed.was_part_of_agent_interaction
                {
                    me.reset_context_to_default(ctx);
                }
            }
            ModelEvent::BlockMetadataReceived(block_metadata_received) => {
                let pwd = block_metadata_received
                    .block_metadata
                    .current_working_directory()
                    .map(|s| PathBuf::from(s.to_owned()));
                let session_id = block_metadata_received.block_metadata.session_id();

                if let Some(session_id) = session_id {
                    let active_session = sessions.as_ref(ctx).get(session_id);
                    if let Some(active_session) = active_session {
                        me.update_directory_context(
                            pwd.map(|p| p.to_string_lossy().to_string()),
                            active_session.home_dir().map(|sq| sq.to_owned()),
                            ctx,
                        );
                    }
                }
            }
            _ => {}
        });

        ctx.subscribe_to_model(&BlocklistAIHistoryModel::handle(ctx), |me, event, ctx| {
            if event
                .terminal_view_id()
                .is_some_and(|id| id != me.terminal_view_id)
            {
                return;
            }

            match event {
                BlocklistAIHistoryEvent::ClearedConversationsInTerminalView { .. } => {
                    me.set_pending_query_state(PendingQueryState::default(), ctx);
                    if FeatureFlag::AgentView.is_enabled() {
                        me.agent_view_controller.update(ctx, |controller, ctx| {
                            controller.exit_agent_view(ctx);
                        });
                    }
                }
                BlocklistAIHistoryEvent::SplitConversation {
                    new_conversation_id,
                    ..
                } => {
                    me.set_pending_query_state_for_existing_conversation(
                        *new_conversation_id,
                        AgentViewEntryOrigin::AgentRequestedNewConversation,
                        ctx,
                    );
                }
                _ => {}
            }
        });

        ctx.subscribe_to_model(&LLMPreferences::handle(ctx), |me, event, ctx| {
            if let LLMPreferencesEvent::UpdatedActiveAgentModeLLM = event {
                let llm_prefs = LLMPreferences::as_ref(ctx);
                let vision_supported = llm_prefs.vision_supported(ctx, Some(me.terminal_view_id));
                if !vision_supported {
                    me.clear_pending_images(ctx);
                }
            }
        });

        // Clear auto-attached blocks when exiting agent view or switching conversations
        ctx.subscribe_to_model(&agent_view_controller, |me, event, _ctx| {
            use super::agent_view::AgentViewControllerEvent;
            match event {
                AgentViewControllerEvent::ExitedAgentView { .. }
                | AgentViewControllerEvent::EnteredAgentView { .. } => {
                    me.auto_attached_agent_view_user_block_ids.clear();
                }
                AgentViewControllerEvent::ExitConfirmed { .. } => {}
            }
        });

        // In sandboxed/autonomous mode (SDK mode with --sandboxed flag), automatically set
        // conversations to RunToCompletion mode so they don't wait for user confirmation.
        let pending_query_state =
            if warp_core::execution_mode::AppExecutionMode::as_ref(ctx).is_sandboxed() {
                PendingQueryState::New {
                    autoexecute_override: AIConversationAutoexecuteMode::RunToCompletion,
                }
            } else {
                Default::default()
            };

        Self {
            terminal_model,
            directory_context: Default::default(),
            pending_context_block_ids: HashSet::new(),
            pending_context_selected_text: None,
            pending_attachments: Default::default(),
            pending_query_state,
            terminal_view_id,
            agent_view_controller,
            pending_inline_diff_hunk_attachments: Default::default(),
            pending_document_id: None,
            auto_attached_agent_view_user_block_ids: Vec::new(),
            queue_next_prompt_enabled: false,
        }
    }

    /// Test-only constructor that skips every subscription and singleton lookup performed by
    /// [`Self::new`], so unit tests can build a [`BlocklistAIContextModel`] without registering
    /// `BlocklistAIHistoryModel`, `LLMPreferences`, `ModelEventDispatcher`, `Sessions`, or
    /// `AppExecutionMode`. Callers still pass real [`TerminalModel`] and [`AgentViewController`]
    /// handles to populate the struct fields, but neither needs to be functional for the
    /// methods exercised by these tests.
    #[cfg(test)]
    pub(crate) fn new_for_test(
        terminal_model: Arc<FairMutex<TerminalModel>>,
        terminal_view_id: EntityId,
        agent_view_controller: ModelHandle<AgentViewController>,
    ) -> Self {
        Self {
            terminal_model,
            directory_context: Default::default(),
            pending_context_block_ids: HashSet::new(),
            pending_context_selected_text: None,
            pending_attachments: Default::default(),
            pending_query_state: PendingQueryState::default(),
            terminal_view_id,
            agent_view_controller,
            pending_inline_diff_hunk_attachments: Default::default(),
            pending_document_id: None,
            auto_attached_agent_view_user_block_ids: Vec::new(),
            queue_next_prompt_enabled: false,
        }
    }

    /// Resets the set of blocks to be included as context to an empty list.
    /// Also removes any selected text that was to be included as context.
    pub fn reset_context_to_default(&mut self, ctx: &mut ModelContext<Self>) {
        self.set_pending_context_block_ids(vec![], true, ctx);
        self.set_pending_context_selected_text(None, true, ctx);
        self.clear_pending_attachments(ctx);
        self.clear_diff_hunk_attachments();
        self.set_pending_document(None, ctx);
        self.auto_attached_agent_view_user_block_ids.clear();
    }

    /// Returns `true` if the next AI query has any context that should force the input to be
    /// locked in AI mode (skipping NLD): a pending image or file attachment.
    pub fn has_locking_attachment(&self) -> bool {
        !self.pending_attachments.is_empty()
    }

    /// Returns the set `BlockId`s corresponding to blocks to be included as context with the next
    /// query.
    pub fn pending_context_block_ids(&self) -> &HashSet<BlockId> {
        &self.pending_context_block_ids
    }

    /// Returns selected text to be included as context with the next query.
    pub fn pending_context_selected_text(&self) -> Option<&String> {
        self.pending_context_selected_text.as_ref()
    }

    /// Returns all pending attachments (images and files) for the next query.
    pub fn pending_attachments(&self) -> &[PendingAttachment] {
        &self.pending_attachments
    }

    /// Returns only the pending images for the next query.
    pub fn pending_images(&self) -> Vec<&ImageContext> {
        self.pending_attachments
            .iter()
            .filter_map(|a| match a {
                PendingAttachment::Image(img) => Some(img),
                PendingAttachment::File(_) => None,
            })
            .collect()
    }

    /// Returns only the pending files for the next query.
    pub fn pending_files(&self) -> Vec<&PendingFile> {
        self.pending_attachments
            .iter()
            .filter_map(|a| match a {
                PendingAttachment::File(file) => Some(file),
                PendingAttachment::Image(_) => None,
            })
            .collect()
    }

    /// Given a block ID, transform it into an AIAgentContext::Block.
    pub fn transform_block_to_context(
        &self,
        block_id: &BlockId,
        is_auto_attached_in_agent_view: bool,
    ) -> Option<AIAgentContext> {
        let terminal_model = self.terminal_model.lock();
        block_context_from_terminal_model(&terminal_model, block_id, is_auto_attached_in_agent_view)
            .map(Box::new)
            .map(AIAgentContext::Block)
    }

    /// Returns `AIAgentContext` for the blocks to be included in the current AI query.
    /// If `is_user_query` is true, includes blocks, selected text, and images as context.
    /// If false, excludes these user-specific contexts but includes everything else.
    pub fn pending_context(&self, app: &AppContext, is_user_query: bool) -> Vec<AIAgentContext> {
        let pwd = self.current_pwd();
        let is_pwd_indexed = if cfg!(feature = "agent_mode_evals") {
            // In evals, we want to disable file outline based search. Full
            // source code embedding based context is still available.
            false
        } else {
            UserWorkspaces::as_ref(app).is_codebase_context_enabled(app)
                && pwd.as_ref().is_some_and(|pwd| {
                    RepoOutlines::as_ref(app).is_directory_indexed(Path::new(&pwd))
                })
        };

        let project_rules = if let Some(pwd) = pwd.clone().and_then(|path| {
            PathBuf::from_str(&path)
                .ok()
                .and_then(|s| s.canonicalize().ok())
        }) {
            ProjectContextModel::as_ref(app).find_applicable_rules(&pwd)
        } else {
            None
        };

        let mut context = Vec::new();

        // Always include directory context
        context.push(AIAgentContext::Directory {
            pwd,
            home_dir: self.home_directory(),
            are_file_symbols_indexed: is_pwd_indexed,
        });

        let (head, branch) = {
            let terminal_model = self.terminal_model.lock();
            let active_block = terminal_model.block_list().active_block();
            (
                active_block.git_branch().cloned(),
                active_block.git_branch_name().cloned(),
            )
        };
        if head.is_some() || branch.is_some() {
            context.push(AIAgentContext::Git {
                head: head.unwrap_or_default(),
                branch,
            });
        }

        // Always include project rules if available
        if let Some(rules) = project_rules {
            context.push(AIAgentContext::ProjectRules {
                root_path: rules.root_path.to_string_lossy().into(),
                active_rules: rules
                    .active_rules
                    .into_iter()
                    .map(|rule| {
                        let line_count = rule.content.lines().count();
                        FileContext {
                            file_name: rule.path.to_string_lossy().into(),
                            content: AnyFileContent::StringContent(rule.content.clone()),
                            line_range: None,
                            last_modified: None,
                            line_count,
                        }
                    })
                    .collect(),
                additional_rule_paths: rules.additional_rule_paths,
            });
        }

        // If this is a user query, add user-selected contexts
        if is_user_query {
            // Add selected blocks (manually attached)
            for block_id in &self.pending_context_block_ids {
                if let Some(block_context) = self.transform_block_to_context(block_id, false) {
                    context.push(block_context);
                }
            }

            // Add auto-attached user-executed blocks (when AgentViewBlockContext is enabled)
            if FeatureFlag::AgentViewBlockContext.is_enabled() {
                for block_id in &self.auto_attached_agent_view_user_block_ids {
                    // Skip if already in pending_context_block_ids to avoid duplicates
                    if !self.pending_context_block_ids.contains(block_id) {
                        if let Some(block_context) = self.transform_block_to_context(block_id, true)
                        {
                            context.push(block_context);
                        }
                    }
                }
            }

            // Add selected text
            if let Some(selected_text) = &self.pending_context_selected_text {
                context.push(AIAgentContext::SelectedText(selected_text.clone()));
            }

            // Add images from pending attachments
            for attachment in &self.pending_attachments {
                if let PendingAttachment::Image(image) = attachment {
                    context.push(AIAgentContext::Image(image.clone()));
                }
            }
        }

        context
    }

    pub fn current_pwd(&self) -> Option<String> {
        self.directory_context.pwd.clone()
    }

    pub fn home_directory(&self) -> Option<String> {
        self.directory_context.home_dir.clone()
    }

    /// Updates the context model's stored directory context.
    pub fn update_directory_context(
        &mut self,
        pwd: Option<String>,
        home_dir: Option<String>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.directory_context = DirectoryContext { pwd, home_dir };
        ctx.emit(BlocklistAIContextEvent::UpdatedPendingContext {
            previous_block_ids: self.pending_context_block_ids.clone(),
            requires_block_resync: true,
            requires_text_resync: false,
        });
    }

    /// Set `requires_visual_resync` to `false` only if the pending context was modified as a result
    /// of manual user selections. In such cases, a visual resync won't be required because the
    /// pending context was synchronized to the manual selection.
    pub fn set_pending_context_block_ids(
        &mut self,
        ids: impl IntoIterator<Item = BlockId>,
        requires_visual_resync: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        // Filter out blocks that can't be used as AI context
        let filtered_ids: Vec<BlockId> = {
            let terminal_model = self.terminal_model.lock();
            ids.into_iter()
                .filter(|block_id| {
                    terminal_model
                        .block_list()
                        .block_with_id(block_id)
                        .map(|block| {
                            block.can_be_ai_context(terminal_model.block_list().agent_view_state())
                        })
                        .unwrap_or(false)
                })
                .collect()
        };

        let new_pending_context_block_ids = HashSet::from_iter(filtered_ids);

        // Maintain the invariant that we can't simultaneously use both blocks and selected text
        // as context for the next AI request.
        if !new_pending_context_block_ids.is_empty() {
            self.pending_context_selected_text = None;
        }

        if new_pending_context_block_ids != self.pending_context_block_ids {
            let previous_block_ids = self.pending_context_block_ids.clone();
            ctx.emit(BlocklistAIContextEvent::UpdatedPendingContext {
                previous_block_ids,
                requires_block_resync: requires_visual_resync,
                requires_text_resync: !new_pending_context_block_ids.is_empty(),
            });
        }
        self.pending_context_block_ids = new_pending_context_block_ids;
    }

    /// Set `requires_visual_resync` to `false` only if the pending context was modified as a result
    /// of manual user selections. In such cases, a visual resync won't be required because the
    /// pending context was synchronized to the manual selection.
    pub fn set_pending_context_selected_text(
        &mut self,
        text: Option<String>,
        requires_visual_resync: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        // It doesn't make sense to allow empty text as AI context.
        // Enforcing this assertion here ensures we don't run into weird behaviour with `Some("")` later.
        debug_assert!(!matches!(text.as_deref(), Some("")));

        let previous_block_ids = self.pending_context_block_ids.clone();
        // Maintain the invariant that we can't simultaneously use both blocks and selected text
        // as context for the next AI request.
        if text.is_some() {
            self.pending_context_block_ids = HashSet::new();
        }

        if text != self.pending_context_selected_text {
            ctx.emit(BlocklistAIContextEvent::UpdatedPendingContext {
                previous_block_ids,
                requires_block_resync: text.is_some(),
                requires_text_resync: requires_visual_resync,
            });
        }
        self.pending_context_selected_text = text;
    }

    /// Set the pending AI document to be included as context with the next AI query.
    pub fn set_pending_document(
        &mut self,
        document_id: Option<AIDocumentId>,
        ctx: &mut ModelContext<Self>,
    ) {
        if document_id != self.pending_document_id {
            self.pending_document_id = document_id;
            ctx.emit(BlocklistAIContextEvent::UpdatedPendingContext {
                previous_block_ids: self.pending_context_block_ids.clone(),
                requires_block_resync: false,
                requires_text_resync: false,
            });
        }
    }

    /// Get the pending AI document ID if one is set.
    pub fn pending_document_id(&self) -> Option<AIDocumentId> {
        self.pending_document_id
    }

    pub fn clear_pending_images(&mut self, ctx: &mut ModelContext<Self>) {
        let original_attachment_count = self.pending_attachments.len();
        self.pending_attachments
            .retain(|a| !matches!(a, PendingAttachment::Image(_)));
        if self.pending_attachments.len() < original_attachment_count {
            ctx.emit(BlocklistAIContextEvent::UpdatedPendingContext {
                previous_block_ids: self.pending_context_block_ids.clone(),
                requires_block_resync: false,
                requires_text_resync: false,
            });
        }
    }

    pub fn append_pending_images(
        &mut self,
        images: Vec<ImageContext>,
        ctx: &mut ModelContext<Self>,
    ) {
        if !images.is_empty() {
            let attachments: Vec<PendingAttachment> =
                images.into_iter().map(PendingAttachment::Image).collect();
            self.append_pending_attachments(attachments, ctx);
        }
    }

    pub fn remove_pending_image(&mut self, index: usize, ctx: &mut ModelContext<Self>) {
        // Find the nth image in the combined list and remove it.
        let position = self
            .pending_attachments
            .iter()
            .enumerate()
            .filter(|(_, a)| matches!(a, PendingAttachment::Image(_)))
            .nth(index)
            .map(|(i, _)| i);
        if let Some(pos) = position {
            self.remove_pending_attachment(pos, ctx);
        }
    }

    /// Returns the number of images removed
    pub fn remove_last_pending_images(
        &mut self,
        images_to_remove: usize,
        ctx: &mut ModelContext<Self>,
    ) -> usize {
        let image_indices: Vec<usize> = self
            .pending_attachments
            .iter()
            .enumerate()
            .filter(|(_, a)| matches!(a, PendingAttachment::Image(_)))
            .map(|(i, _)| i)
            .collect();
        let len = image_indices.len();

        if images_to_remove == 0 || len == 0 {
            return 0;
        }

        let to_remove = images_to_remove.min(len);
        // Remove from the end to avoid shifting indices.
        for &idx in image_indices.iter().rev().take(to_remove) {
            self.pending_attachments.remove(idx);
        }

        ctx.emit(BlocklistAIContextEvent::UpdatedPendingContext {
            previous_block_ids: self.pending_context_block_ids.clone(),
            requires_block_resync: false,
            requires_text_resync: false,
        });

        to_remove
    }

    pub fn pending_query_state(&self) -> &PendingQueryState {
        &self.pending_query_state
    }

    /// Convenience function to set pending query state to continue an existing conversation by ID.
    pub fn set_pending_query_state_for_existing_conversation(
        &mut self,
        conversation_id: AIConversationId,
        origin: AgentViewEntryOrigin,
        ctx: &mut ModelContext<Self>,
    ) {
        self.set_pending_query_state(PendingQueryState::Existing { conversation_id }, ctx);
        if FeatureFlag::AgentView.is_enabled() {
            if let Err(e) = self.agent_view_controller.update(ctx, |controller, ctx| {
                controller.try_enter_agent_view(Some(conversation_id), origin, ctx)
            }) {
                log::error!("Failed to enter agent view for existing conversation: {e}");
            }
        }
    }

    /// Sets the pending query state to the defaults for a *new* conversation (i.e. not a
    /// followup).
    pub fn set_pending_query_state_for_new_conversation(
        &mut self,
        origin: AgentViewEntryOrigin,
        ctx: &mut ModelContext<Self>,
    ) {
        self.set_pending_query_state(PendingQueryState::default(), ctx);

        if FeatureFlag::AgentView.is_enabled() {
            if let Err(e) = self.agent_view_controller.update(ctx, |controller, ctx| {
                controller.try_enter_agent_view(None, origin, ctx)
            }) {
                log::error!("Failed to enter agent view for new conversation: {e}");
            }
        }
    }

    /// Attempts to enter agent view for a new conversation and returns the conversation ID.
    /// This should be used when a slash command needs to create a new conversation
    /// and the AgentView feature flag is enabled.
    ///
    /// Returns `Ok(conversation_id)` on success, or `Err` if entry is blocked.
    pub fn try_enter_agent_view_for_new_conversation(
        &mut self,
        origin: AgentViewEntryOrigin,
        ctx: &mut ModelContext<Self>,
    ) -> Result<AIConversationId, EnterAgentViewError> {
        let conversation_id = self.agent_view_controller.update(ctx, |controller, ctx| {
            controller.try_enter_agent_view(None, origin, ctx)
        })?;
        self.set_pending_query_state(PendingQueryState::default(), ctx);
        Ok(conversation_id)
    }

    /// Sets the value of `pending_query_state`, emitting an event if it changed.
    fn set_pending_query_state(&mut self, state: PendingQueryState, ctx: &mut ModelContext<Self>) {
        if self.pending_query_state != state {
            self.pending_query_state = state;
            ctx.emit(BlocklistAIContextEvent::PendingQueryStateUpdated);
        }
    }

    /// Returns `true` if a new conversation may be created.
    pub fn can_start_new_conversation(&self) -> bool {
        let terminal_model = self.terminal_model.lock();
        if FeatureFlag::AgentView.is_enabled() {
            !terminal_model
                .block_list()
                .active_block()
                .is_active_and_long_running()
        } else {
            !terminal_model
                .block_list()
                .active_block()
                .is_agent_in_control()
        }
    }

    /// Returns the conversation ID the pending query is following up for, if any.
    /// None if the pending query should start a new conversation.
    pub fn selected_conversation_id(&self, ctx: &AppContext) -> Option<AIConversationId> {
        if FeatureFlag::AgentView.is_enabled() {
            return self
                .agent_view_controller
                .as_ref(ctx)
                .agent_view_state()
                .active_conversation_id();
        }

        match self.pending_query_state {
            PendingQueryState::Existing {
                conversation_id, ..
            } => Some(conversation_id),
            PendingQueryState::New { .. } => None,
        }
    }

    pub fn selected_conversation<'a>(&self, ctx: &'a AppContext) -> Option<&'a AIConversation> {
        self.selected_conversation_id(ctx)
            .as_ref()
            .and_then(|conversation_id| {
                BlocklistAIHistoryModel::as_ref(ctx).conversation(conversation_id)
            })
    }

    pub fn selected_conversation_todolist<'a>(
        &self,
        ctx: &'a AppContext,
    ) -> Option<&'a AIAgentTodoList> {
        self.selected_conversation(ctx)
            .and_then(|c| c.active_todo_list())
            .and_then(|todo_list| {
                // Don't show todo list if it's empty or finished
                if todo_list.is_empty() || todo_list.is_finished() {
                    None
                } else {
                    Some(todo_list)
                }
            })
    }

    pub fn pending_query_autoexecute_override(
        &self,
        ctx: &AppContext,
    ) -> AIConversationAutoexecuteMode {
        match &self.pending_query_state {
            PendingQueryState::New {
                autoexecute_override,
            } => *autoexecute_override,
            PendingQueryState::Existing {
                conversation_id, ..
            } => BlocklistAIHistoryModel::as_ref(ctx)
                .conversation(conversation_id)
                .map(|conversation| conversation.autoexecute_override())
                .unwrap_or_default(),
        }
    }

    pub fn is_queue_next_prompt_enabled(&self) -> bool {
        self.queue_next_prompt_enabled
    }

    pub fn toggle_queue_next_prompt(&mut self, ctx: &mut ModelContext<Self>) {
        self.queue_next_prompt_enabled = !self.queue_next_prompt_enabled;
        ctx.emit(BlocklistAIContextEvent::QueueNextPromptToggled);
    }

    pub fn toggle_pending_query_autoexecute(&mut self, ctx: &mut ModelContext<Self>) {
        // When AgentView is enabled, the autoexecution toggle should apply to the active agent view
        // conversation -- even when starting a new conversation, the agent view always has a conversation
        // ID.
        if FeatureFlag::AgentView.is_enabled() {
            if let Some(conversation_id) = self
                .agent_view_controller
                .as_ref(ctx)
                .agent_view_state()
                .active_conversation_id()
            {
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                    history.toggle_autoexecute_override(
                        &conversation_id,
                        self.terminal_view_id,
                        ctx,
                    );
                });
            }
            return;
        }

        match &mut self.pending_query_state {
            PendingQueryState::New {
                autoexecute_override,
            } => {
                *autoexecute_override = if *autoexecute_override
                    == AIConversationAutoexecuteMode::RespectUserSettings
                {
                    AIConversationAutoexecuteMode::RunToCompletion
                } else {
                    AIConversationAutoexecuteMode::RespectUserSettings
                };
                ctx.emit(BlocklistAIContextEvent::PendingQueryStateUpdated);
            }
            PendingQueryState::Existing {
                conversation_id, ..
            } => {
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                    history.toggle_autoexecute_override(
                        conversation_id,
                        self.terminal_view_id,
                        ctx,
                    );
                });
            }
        }
    }

    /// Returns true if the pending query targets an existing conversation
    /// (as opposed to starting a new one).
    pub fn is_targeting_existing_conversation(&self) -> bool {
        self.pending_query_state.targets_existing_conversation()
    }

    /// Returns the status of the selected conversation for purposes of rendering the input hint
    /// text, or `None` if there is no selected conversation to display (either because no
    /// conversation is selected, or because the selected conversation is empty/passive/untitled
    /// and should be treated as a "new" conversation). Mirrors the `agent_indicator` pattern in
    /// `app/src/tab.rs`.
    pub fn selected_conversation_status_for_hint(
        &self,
        app: &AppContext,
    ) -> Option<ConversationStatus> {
        let conversation = self.selected_conversation(app)?;
        if conversation.is_empty()
            || conversation.is_entirely_passive()
            || conversation.title().is_none()
        {
            return None;
        }
        Some(conversation.status().clone())
    }

    /// Returns true if there are any blocks that can be used as AI context.
    pub fn can_attach_blocks(&self) -> bool {
        let terminal_model = self.terminal_model.lock();
        terminal_model
            .block_list()
            .blocks()
            .iter()
            .any(|block| block.can_be_ai_context(terminal_model.block_list().agent_view_state()))
    }

    /// Register a diff hunk attachment that can be referenced in future queries
    pub fn register_diff_hunk_attachment(
        &mut self,
        diff_hunk_id: String,
        attachment: AIAgentAttachment,
    ) {
        self.pending_inline_diff_hunk_attachments
            .insert(diff_hunk_id, attachment);
    }

    /// Get a diff hunk attachment by its ID
    pub fn get_diff_hunk_attachment(&self, diff_hunk_id: &str) -> Option<&AIAgentAttachment> {
        self.pending_inline_diff_hunk_attachments.get(diff_hunk_id)
    }

    /// Clear all diff hunk attachments (should be called after each request)
    pub fn clear_diff_hunk_attachments(&mut self) {
        self.pending_inline_diff_hunk_attachments.clear();
    }

    /// Appends attachments to the pending list.
    pub fn append_pending_attachments(
        &mut self,
        attachments: Vec<PendingAttachment>,
        ctx: &mut ModelContext<Self>,
    ) {
        if !attachments.is_empty() {
            ctx.emit(BlocklistAIContextEvent::UpdatedPendingContext {
                previous_block_ids: self.pending_context_block_ids.clone(),
                requires_block_resync: false,
                requires_text_resync: false,
            });
        }
        self.pending_attachments.extend(attachments);
    }

    /// Removes an attachment by index.
    pub fn remove_pending_attachment(&mut self, index: usize, ctx: &mut ModelContext<Self>) {
        if index < self.pending_attachments.len() {
            self.pending_attachments.remove(index);
            ctx.emit(BlocklistAIContextEvent::UpdatedPendingContext {
                previous_block_ids: self.pending_context_block_ids.clone(),
                requires_block_resync: false,
                requires_text_resync: false,
            });
        }
    }

    /// Clears all pending attachments.
    pub fn clear_pending_attachments(&mut self, ctx: &mut ModelContext<Self>) {
        if !self.pending_attachments.is_empty() {
            ctx.emit(BlocklistAIContextEvent::UpdatedPendingContext {
                previous_block_ids: self.pending_context_block_ids.clone(),
                requires_block_resync: false,
                requires_text_resync: false,
            });
        }
        self.pending_attachments.clear();
    }
}

pub enum BlocklistAIContextEvent {
    /// The bool fields determine whether a visual resync is needed for each respective selection type.
    /// For example, if selected text is cleared via the `BlocklistAIContextModel` **only**, then
    /// the `TerminalView`'s current text selection should be visually cleared as well.
    UpdatedPendingContext {
        previous_block_ids: HashSet<BlockId>,
        requires_block_resync: bool,
        requires_text_resync: bool,
    },
    /// Emitted whenever the value changes.
    PendingQueryStateUpdated,
    QueueNextPromptToggled,
}

impl Entity for BlocklistAIContextModel {
    type Event = BlocklistAIContextEvent;
}

#[cfg(test)]
#[path = "context_model_tests.rs"]
mod tests;
