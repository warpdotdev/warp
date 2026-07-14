//! The production-shaped TUI transcript over canonical terminal block-list order.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use parking_lot::FairMutex;
use warp::tui_export::{
    should_show_task_in_blocklist, AIAgentExchangeId, AIBlockModelImpl, AIConversationId,
    BlockIndex, BlockPadding, BlockSpacing, BlocklistAIActionModel, BlocklistAIHistoryEvent,
    BlocklistAIHistoryModel, ConversationBlockRestorationPlan, ModelEventDispatcher,
    RichContentItem, RichContentType, TerminalModel,
};
use warp_core::semantic_selection::SemanticSelection;
use warpui_core::elements::tui::{
    TuiElement, TuiRowResize, TuiScrollable, TuiScrollableElement, TuiSelectable,
    TuiSelectionHandle, TuiViewportVerticalAlignment, TuiViewportedList, TuiViewportedListState,
};
use warpui_core::{
    AppContext, Entity, EntityId, ModelHandle, SingletonEntity, TuiView, TypedActionView,
    ViewContext,
};

use super::agent_block::{TuiAIBlock, TuiAIBlockEvent};
use super::terminal_block::should_render_terminal_block;
use super::tui_block_list_viewport_source::{AgentBlockRegistry, TuiBlockListViewportSource};

/// Rows of blank space above every transcript block. Terminal blocks get it
/// via [`TRANSCRIPT_BLOCK_SPACING`]'s `padding_top`; agent blocks apply the
/// same top padding directly, so every adjacent block pair is separated by
/// exactly this many rows.
pub(crate) const BLOCK_TOP_PADDING_ROWS: u16 = 1;

/// Block spacing baked into the terminal model's block heights for this
/// transcript, passed in at session creation. The transcript renders whole
/// rows, so fractional pixel-derived padding would ceil into several blank
/// rows per block; instead every block gets exactly [`BLOCK_TOP_PADDING_ROWS`]
/// blank rows above it, no reserved Warp-prompt height, and no memory-stats
/// footer row (the transcript renders neither).
pub(crate) const TRANSCRIPT_BLOCK_SPACING: BlockSpacing = BlockSpacing {
    block_padding: BlockPadding {
        padding_top: BLOCK_TOP_PADDING_ROWS as f32,
        command_padding_top: 0.0,
        middle: 0.0,
        bottom: 0.0,
    },
    warp_prompt_height_lines: 0.0,
    show_memory_stats: false,
};

/// Events emitted by the transcript to its owning session view.
#[derive(Debug, Clone)]
pub(super) enum TuiTranscriptViewEvent {
    SelectionStarted,
    SelectionEnded(String),
}

/// Selection actions originating from the transcript's element tree.
#[derive(Debug, Clone)]
pub(super) enum TuiTranscriptAction {
    SelectionStarted,
    SelectionEnded(String),
}

/// TUI transcript view over one terminal surface's canonical block-list order.
pub(super) struct TuiTranscriptView {
    terminal_surface_id: EntityId,
    model: Arc<FairMutex<TerminalModel>>,
    action_model: ModelHandle<BlocklistAIActionModel>,
    model_events: ModelHandle<ModelEventDispatcher>,
    agent_blocks: AgentBlockRegistry,
    viewport: TuiViewportedListState,
    selection: TuiSelectionHandle,
}

impl TuiTranscriptView {
    /// Creates a transcript view for one terminal surface.
    pub(super) fn new(
        terminal_surface_id: EntityId,
        model: Arc<FairMutex<TerminalModel>>,
        action_model: ModelHandle<BlocklistAIActionModel>,
        model_events: &ModelHandle<ModelEventDispatcher>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        ctx.subscribe_to_model(
            &BlocklistAIHistoryModel::handle(ctx),
            |view, _, event, ctx| view.handle_history_event(event, ctx),
        );

        Self {
            terminal_surface_id,
            model,
            action_model,
            model_events: model_events.clone(),
            agent_blocks: Rc::new(RefCell::new(HashMap::new())),
            viewport: TuiViewportedListState::new_at_end(),
            selection: TuiSelectionHandle::default(),
        }
    }

    fn mark_agent_block_dirty(&self, view_id: EntityId, ctx: &mut ViewContext<Self>) {
        self.model
            .lock()
            .block_list_mut()
            .mark_rich_content_dirty(view_id);
        ctx.notify();
    }

    fn handle_history_event(
        &mut self,
        event: &BlocklistAIHistoryEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        if event
            .terminal_surface_id()
            .is_some_and(|id| id != self.terminal_surface_id)
        {
            return;
        }

        match event {
            BlocklistAIHistoryEvent::AppendedExchange {
                exchange_id,
                task_id,
                conversation_id,
                is_hidden,
                ..
            } => {
                if *is_hidden {
                    return;
                }
                let should_show = BlocklistAIHistoryModel::as_ref(ctx)
                    .conversation(conversation_id)
                    .and_then(|conversation| conversation.get_task(task_id))
                    .is_some_and(should_show_task_in_blocklist);
                if should_show {
                    self.insert_agent_block(*conversation_id, *exchange_id, None, ctx);
                }
            }
            BlocklistAIHistoryEvent::UpdatedStreamingExchange { exchange_id, .. } => {
                self.mark_exchange_dirty(*exchange_id, ctx);
            }
            // Todo statuses are projections of conversation-wide state. A new
            // list can cancel rows rendered by an older exchange, so dirty
            // every todo-rendering block on this surface rather than only the
            // exchange carrying the UpdateTodos message.
            BlocklistAIHistoryEvent::UpdatedTodoList { .. } => {
                self.mark_todo_blocks_dirty(ctx);
            }
            // The first pending item switches between InProgress and Stopped
            // when its conversation starts or stops, without changing the
            // output messages that own the rendered task list.
            BlocklistAIHistoryEvent::UpdatedConversationStatus {
                conversation_id, ..
            } => {
                self.mark_conversation_dirty(*conversation_id, ctx);
            }
            BlocklistAIHistoryEvent::ReassignedExchange {
                exchange_id,
                new_conversation_id,
                ..
            } => self.reassign_exchange(*exchange_id, *new_conversation_id, ctx),
            BlocklistAIHistoryEvent::RemoveConversation {
                conversation_id, ..
            }
            | BlocklistAIHistoryEvent::DeletedConversation {
                conversation_id, ..
            }
            | BlocklistAIHistoryEvent::ConversationTransferredBetweenTerminalSurfaces {
                conversation_id,
                ..
            } => self.remove_conversation(*conversation_id, ctx),
            BlocklistAIHistoryEvent::ClearedConversationsForTerminalSurface {
                active_conversation_id,
                cleared_conversation_ids,
                ..
            } => {
                let mut conversation_ids = cleared_conversation_ids.clone();
                if let Some(active_conversation_id) = active_conversation_id {
                    if !conversation_ids.contains(active_conversation_id) {
                        conversation_ids.push(*active_conversation_id);
                    }
                }
                for conversation_id in conversation_ids {
                    self.remove_conversation(conversation_id, ctx);
                }
            }
            BlocklistAIHistoryEvent::StartedNewConversation { .. }
            | BlocklistAIHistoryEvent::CreatedSubtask { .. }
            | BlocklistAIHistoryEvent::UpgradedTask { .. }
            | BlocklistAIHistoryEvent::SetActiveConversation { .. }
            | BlocklistAIHistoryEvent::ClearedActiveConversation { .. }
            | BlocklistAIHistoryEvent::UpdatedAutoexecuteOverride { .. }
            | BlocklistAIHistoryEvent::SplitConversation { .. }
            | BlocklistAIHistoryEvent::RestoredConversations { .. }
            | BlocklistAIHistoryEvent::UpdatedConversationMetadata { .. }
            | BlocklistAIHistoryEvent::UpdatedConversationTitle { .. }
            | BlocklistAIHistoryEvent::UpdatedConversationArtifacts { .. }
            | BlocklistAIHistoryEvent::ConversationServerTokenAssigned { .. }
            | BlocklistAIHistoryEvent::NewConversationRequestComplete { .. }
            | BlocklistAIHistoryEvent::OrchestrationConfigUpdated { .. }
            | BlocklistAIHistoryEvent::ConversationUsageMetadataUpdated { .. }
            | BlocklistAIHistoryEvent::LocalSharedSessionEstablished { .. } => {}
        }
    }

    /// Whether the transcript has no visible content: no agent block and no
    /// terminal block it would render (per [`should_render_terminal_block`];
    /// the idle prompt block awaiting the first command doesn't count). The
    /// session view fills the transcript slot with the zero state exactly
    /// while this holds.
    pub(super) fn is_empty(&self) -> bool {
        if !self.agent_blocks.borrow().is_empty() {
            return false;
        }
        let model = self.model.lock();
        let block_list = model.block_list();
        !block_list
            .blocks()
            .iter()
            .any(|block| should_render_terminal_block(block, block_list))
    }

    /// Returns the view id of the agent block rendering `exchange_id`, if any.
    fn view_id_for_exchange(
        &self,
        exchange_id: AIAgentExchangeId,
        ctx: &AppContext,
    ) -> Option<EntityId> {
        self.agent_blocks
            .borrow()
            .iter()
            .find_map(|(view_id, view)| {
                (view.as_ref(ctx).exchange_id() == exchange_id).then_some(*view_id)
            })
    }

    fn insert_agent_block(
        &mut self,
        conversation_id: AIConversationId,
        exchange_id: AIAgentExchangeId,
        command_block_index: Option<BlockIndex>,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.view_id_for_exchange(exchange_id, ctx).is_some() {
            return;
        }

        let Ok(block_model) =
            AIBlockModelImpl::<TuiAIBlock>::new(exchange_id, conversation_id, false, false, ctx)
        else {
            log::warn!(
                "Failed to create TUI model for AI block on AppendedExchange: {exchange_id:?}"
            );
            return;
        };

        let block_model = Rc::new(block_model);
        let action_model = self.action_model.clone();
        let model_events = self.model_events.clone();
        let terminal_model = self.model.clone();
        let view = ctx.add_typed_action_tui_view(|ctx| {
            TuiAIBlock::new(
                conversation_id,
                exchange_id,
                block_model,
                action_model,
                &model_events,
                terminal_model,
                ctx,
            )
        });
        let view_id = view.id();
        ctx.subscribe_to_view(&view, move |transcript, _, event, ctx| match event {
            TuiAIBlockEvent::LayoutInvalidated => {
                transcript
                    .model
                    .lock()
                    .block_list_mut()
                    .mark_rich_content_dirty(view_id);
                ctx.notify();
            }
        });
        self.agent_blocks.borrow_mut().insert(view_id, view);
        let item = RichContentItem::new(Some(RichContentType::AIBlock), view_id, None, false);
        let mut model = self.model.lock();
        match command_block_index {
            Some(command_block_index) => model
                .block_list_mut()
                .insert_rich_content_before_block_index(item, command_block_index),
            None => model.block_list_mut().append_rich_content(item, false),
        }
        ctx.notify();
    }

    /// Materializes a shared restoration plan as TUI agent-block views.
    pub(super) fn restore_conversation(
        &mut self,
        conversation_id: AIConversationId,
        restoration_plan: ConversationBlockRestorationPlan,
        ctx: &mut ViewContext<Self>,
    ) {
        for restored_exchange in restoration_plan.into_exchanges() {
            self.insert_agent_block(
                conversation_id,
                restored_exchange.exchange().id,
                restored_exchange.command_block_index(),
                ctx,
            );
        }
        self.viewport.scroll_to_end();
        ctx.notify();
    }

    /// Clears agent rich content before replacing the sole conversation.
    pub(super) fn clear_for_replacement(&mut self, ctx: &mut ViewContext<Self>) {
        self.clear_agent_blocks(ctx);
        self.viewport.scroll_to_end();
    }

    fn mark_exchange_dirty(&mut self, exchange_id: AIAgentExchangeId, ctx: &mut ViewContext<Self>) {
        if let Some(view_id) = self.view_id_for_exchange(exchange_id, ctx) {
            self.mark_agent_block_dirty(view_id, ctx);
        }
    }

    /// Marks every todo-rendering agent block on this terminal surface dirty.
    /// Todo-list updates are conversation-wide — a newly active list can
    /// restyle rows in any older exchange as cancelled — but blocks without a
    /// todo message never change appearance, so their cached heights and
    /// layout stay untouched.
    fn mark_todo_blocks_dirty(&mut self, ctx: &mut ViewContext<Self>) {
        let view_ids = self
            .agent_blocks
            .borrow()
            .iter()
            .filter_map(|(view_id, view)| view.as_ref(ctx).renders_todos().then_some(*view_id))
            .collect::<Vec<_>>();
        if view_ids.is_empty() {
            return;
        }
        let mut model = self.model.lock();
        for view_id in view_ids {
            model.block_list_mut().mark_rich_content_dirty(view_id);
        }
        drop(model);
        ctx.notify();
    }

    /// Marks `conversation_id`'s todo-rendering agent blocks dirty.
    /// Conversation status participates in the projected status of the first
    /// pending todo (InProgress vs Stopped); no other TUI block content reads
    /// it, so blocks without todo messages keep their cached layout.
    fn mark_conversation_dirty(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ViewContext<Self>,
    ) {
        let view_ids = self
            .agent_blocks
            .borrow()
            .iter()
            .filter_map(|(view_id, view)| {
                let view = view.as_ref(ctx);
                (view.conversation_id() == conversation_id && view.renders_todos())
                    .then_some(*view_id)
            })
            .collect::<Vec<_>>();
        if view_ids.is_empty() {
            return;
        }
        let mut model = self.model.lock();
        for view_id in view_ids {
            model.block_list_mut().mark_rich_content_dirty(view_id);
        }
        drop(model);
        ctx.notify();
    }

    fn reassign_exchange(
        &mut self,
        exchange_id: AIAgentExchangeId,
        conversation_id: AIConversationId,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(view_id) = self.view_id_for_exchange(exchange_id, ctx) else {
            return;
        };
        let Some(agent_block) = self.agent_blocks.borrow().get(&view_id).cloned() else {
            return;
        };
        let Ok(block_model) =
            AIBlockModelImpl::<TuiAIBlock>::new(exchange_id, conversation_id, false, false, ctx)
        else {
            log::warn!(
                "Failed to create reassigned TUI model for AI block on ReassignedExchange: {exchange_id:?}"
            );
            return;
        };
        agent_block.update(ctx, |view, ctx| {
            view.replace_model(conversation_id, Rc::new(block_model));
            ctx.notify();
        });
        self.mark_agent_block_dirty(view_id, ctx);
    }

    fn remove_conversation(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ViewContext<Self>,
    ) {
        let view_ids = self
            .agent_blocks
            .borrow()
            .iter()
            .filter_map(|(view_id, view)| {
                (view.as_ref(ctx).conversation_id() == conversation_id).then_some(*view_id)
            })
            .collect::<Vec<_>>();
        for view_id in view_ids {
            let rows = {
                let model = self.model.lock();
                model.block_list().rich_content_row_range(view_id)
            };
            if let Some(rows) = rows {
                self.selection.rebase_for_row_resize(TuiRowResize {
                    old_rows: rows,
                    new_height: 0,
                });
            }
            self.agent_blocks.borrow_mut().remove(&view_id);
            self.model
                .lock()
                .block_list_mut()
                .remove_rich_content(view_id);
        }
        ctx.notify();
    }

    /// Clears persistent selection owned by the transcript.
    pub(super) fn clear_selection(&mut self, ctx: &mut ViewContext<Self>) {
        if self.selection.clear() {
            ctx.notify();
        }
    }

    fn clear_agent_blocks(&mut self, ctx: &mut ViewContext<Self>) {
        let view_ids = self
            .agent_blocks
            .borrow()
            .keys()
            .copied()
            .collect::<Vec<_>>();
        self.agent_blocks.borrow_mut().clear();
        self.selection.clear();
        let mut model = self.model.lock();
        for view_id in view_ids {
            model.block_list_mut().remove_rich_content(view_id);
        }
        ctx.notify();
    }
}

impl Entity for TuiTranscriptView {
    type Event = TuiTranscriptViewEvent;
}

impl TuiView for TuiTranscriptView {
    fn ui_name() -> &'static str {
        "TuiTranscriptView"
    }

    fn child_view_ids(&self, _app: &AppContext) -> Vec<EntityId> {
        self.agent_blocks.borrow().keys().copied().collect()
    }

    fn render(&self, app: &AppContext) -> Box<dyn TuiElement> {
        let source = TuiBlockListViewportSource::new(self.model.clone(), self.agent_blocks.clone());
        let viewport = TuiViewportedList::new(self.viewport.clone(), source)
            .with_vertical_alignment(TuiViewportVerticalAlignment::GrowFromBottom);
        let semantic_selection = SemanticSelection::as_ref(app);
        let selectable = TuiSelectable::new(self.selection.clone(), viewport)
            .with_word_boundaries_policy(semantic_selection.word_boundary_policy())
            .with_smart_select_fn(semantic_selection.smart_select_fn())
            .on_selection_start(|event_ctx, _| {
                event_ctx.dispatch_typed_action(TuiTranscriptAction::SelectionStarted);
            })
            .on_copy(|text, event_ctx, _| {
                event_ctx.dispatch_typed_action(TuiTranscriptAction::SelectionEnded(text));
            });
        TuiScrollable::new(selectable.finish_scrollable()).finish()
    }
}

impl TypedActionView for TuiTranscriptView {
    type Action = TuiTranscriptAction;

    fn handle_action(&mut self, action: &TuiTranscriptAction, ctx: &mut ViewContext<Self>) {
        match action {
            TuiTranscriptAction::SelectionStarted => {
                ctx.emit(TuiTranscriptViewEvent::SelectionStarted);
            }
            TuiTranscriptAction::SelectionEnded(text) => {
                ctx.emit(TuiTranscriptViewEvent::SelectionEnded(text.clone()));
            }
        }
    }
}

#[cfg(test)]
#[path = "transcript_view_tests.rs"]
mod tests;
