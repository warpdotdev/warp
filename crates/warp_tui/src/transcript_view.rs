//! The production-shaped TUI transcript over canonical terminal block-list order.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use parking_lot::FairMutex;
use warp::tui_export::{
    should_show_task_in_blocklist, AIAgentExchangeId, AIBlockModelImpl, AIConversationId,
    BlocklistAIHistoryEvent, BlocklistAIHistoryModel, RichContentItem, RichContentType,
    TerminalModel,
};
use warpui_core::elements::tui::{
    TuiElement, TuiScrollable, TuiViewportVerticalAlignment, TuiViewportedList,
    TuiViewportedListState,
};
use warpui_core::{
    AppContext, Entity, EntityId, SingletonEntity, TuiView, TypedActionView, ViewContext,
};

use super::agent_block::TuiAgentBlockView;
use super::tui_block_list_viewport_source::{
    AgentBlockRegistration, AgentBlockRegistry, TuiBlockListViewportSource,
};

/// TUI transcript view over one terminal surface's canonical block-list order.
pub(super) struct TuiTranscriptView {
    terminal_surface_id: EntityId,
    model: Arc<FairMutex<TerminalModel>>,
    agent_blocks: AgentBlockRegistry,
    viewport: TuiViewportedListState,
}

impl TuiTranscriptView {
    /// Creates a transcript view for one terminal surface.
    pub(super) fn new(
        terminal_surface_id: EntityId,
        model: Arc<FairMutex<TerminalModel>>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        ctx.subscribe_to_model(
            &BlocklistAIHistoryModel::handle(ctx),
            |view, _, event, ctx| view.handle_history_event(event, ctx),
        );

        Self {
            terminal_surface_id,
            model,
            agent_blocks: Rc::new(RefCell::new(HashMap::new())),
            viewport: TuiViewportedListState::new_at_end(),
        }
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
                    self.insert_agent_block(*conversation_id, *exchange_id, ctx);
                }
            }
            BlocklistAIHistoryEvent::UpdatedStreamingExchange { exchange_id, .. } => {
                self.mark_exchange_dirty(*exchange_id, ctx);
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
            BlocklistAIHistoryEvent::ClearedConversationsForTerminalSurface { .. } => {
                self.clear_agent_blocks(ctx);
            }
            _ => {}
        }
    }

    fn insert_agent_block(
        &mut self,
        conversation_id: AIConversationId,
        exchange_id: AIAgentExchangeId,
        ctx: &mut ViewContext<Self>,
    ) {
        if self
            .agent_blocks
            .borrow()
            .values()
            .any(|registration| registration.exchange_id == exchange_id)
        {
            return;
        }

        let Ok(block_model) = AIBlockModelImpl::<TuiAgentBlockView>::new(
            exchange_id,
            conversation_id,
            false,
            false,
            ctx,
        ) else {
            log::warn!(
                "Failed to create TUI model for AI block on AppendedExchange: {exchange_id:?}"
            );
            return;
        };
        let block_model = Rc::new(block_model);
        let view = ctx.add_tui_view(|_| TuiAgentBlockView::new(block_model));
        let view_id = view.id();
        self.agent_blocks.borrow_mut().insert(
            view_id,
            AgentBlockRegistration {
                view,
                conversation_id,
                exchange_id,
            },
        );
        self.model.lock().block_list_mut().append_rich_content(
            RichContentItem::new(Some(RichContentType::AIBlock), view_id, None, false),
            false,
        );
        ctx.notify();
    }

    fn mark_exchange_dirty(&mut self, exchange_id: AIAgentExchangeId, ctx: &mut ViewContext<Self>) {
        let view_id = self
            .agent_blocks
            .borrow()
            .iter()
            .find_map(|(view_id, registration)| {
                (registration.exchange_id == exchange_id).then_some(*view_id)
            });
        if let Some(view_id) = view_id {
            self.model
                .lock()
                .block_list_mut()
                .mark_rich_content_dirty(view_id);
            ctx.notify();
        }
    }

    fn reassign_exchange(
        &mut self,
        exchange_id: AIAgentExchangeId,
        conversation_id: AIConversationId,
        ctx: &mut ViewContext<Self>,
    ) {
        let mut agent_blocks = self.agent_blocks.borrow_mut();
        let registration = agent_blocks.iter_mut().find_map(|(view_id, registration)| {
            (registration.exchange_id == exchange_id).then_some((*view_id, registration))
        });
        let Some((view_id, registration)) = registration else {
            return;
        };
        let Ok(block_model) = AIBlockModelImpl::<TuiAgentBlockView>::new(
            exchange_id,
            conversation_id,
            false,
            false,
            ctx,
        ) else {
            log::warn!(
                "Failed to create reassigned TUI model for AI block on ReassignedExchange: {exchange_id:?}"
            );
            return;
        };
        registration.conversation_id = conversation_id;
        registration
            .view
            .update(ctx, |view, _| view.set_model(Rc::new(block_model)));
        self.model
            .lock()
            .block_list_mut()
            .mark_rich_content_dirty(view_id);
        ctx.notify();
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
            .filter_map(|(view_id, registration)| {
                (registration.conversation_id == conversation_id).then_some(*view_id)
            })
            .collect::<Vec<_>>();
        for view_id in view_ids {
            self.agent_blocks.borrow_mut().remove(&view_id);
            self.model
                .lock()
                .block_list_mut()
                .remove_rich_content(view_id);
        }
        ctx.notify();
    }

    fn clear_agent_blocks(&mut self, ctx: &mut ViewContext<Self>) {
        let view_ids = self
            .agent_blocks
            .borrow()
            .keys()
            .copied()
            .collect::<Vec<_>>();
        self.agent_blocks.borrow_mut().clear();
        let mut model = self.model.lock();
        for view_id in view_ids {
            model.block_list_mut().remove_rich_content(view_id);
        }
        ctx.notify();
    }
}

impl Entity for TuiTranscriptView {
    type Event = ();
}

impl TuiView for TuiTranscriptView {
    fn ui_name() -> &'static str {
        "TuiTranscriptView"
    }

    fn child_view_ids(&self, _app: &AppContext) -> Vec<EntityId> {
        self.agent_blocks.borrow().keys().copied().collect()
    }

    fn render(&self, _app: &AppContext) -> Box<dyn TuiElement> {
        let source = TuiBlockListViewportSource::new(self.model.clone(), self.agent_blocks.clone());
        Box::new(TuiScrollable::new(
            TuiViewportedList::new(self.viewport.clone(), source)
                .with_vertical_alignment(TuiViewportVerticalAlignment::GrowFromBottom),
        ))
    }
}

impl TypedActionView for TuiTranscriptView {
    type Action = ();
}

#[cfg(test)]
#[path = "transcript_view_tests.rs"]
mod tests;
