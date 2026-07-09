use warp_errors::report_error;
use warpui::{AppContext, EntityId, ModelContext, ModelHandle, SingletonEntity};

use super::{
    AgentViewController, AgentViewControllerEvent, AgentViewEntryOrigin, EnterAgentViewError,
};
use crate::ai::agent::conversation::{AIConversationAutoexecuteMode, AIConversationId};
use crate::ai::blocklist::conversation_selection::{
    ConversationSelection, ConversationSelectionEvent,
};
use crate::ai::blocklist::{BlocklistAIHistoryEvent, BlocklistAIHistoryModel};

/// GUI conversation selection backed unconditionally by Agent View.
pub(crate) struct AgentViewConversationSelection {
    terminal_surface_id: EntityId,
    agent_view_controller: ModelHandle<AgentViewController>,
}

impl AgentViewConversationSelection {
    /// Creates GUI conversation selection for a terminal view.
    pub(crate) fn new(
        terminal_surface_id: warpui::EntityId,
        agent_view_controller: ModelHandle<AgentViewController>,
        ctx: &mut ModelContext<Box<dyn ConversationSelection>>,
    ) -> Self {
        ctx.subscribe_to_model(&agent_view_controller, |_, _, event, ctx| match event {
            AgentViewControllerEvent::EnteredAgentView {
                display_mode,
                origin,
                ..
            } => {
                ctx.emit(ConversationSelectionEvent::Changed);
                ctx.emit(ConversationSelectionEvent::Activated {
                    is_fullscreen: display_mode.is_fullscreen(),
                    origin: origin.clone(),
                });
            }
            AgentViewControllerEvent::ExitedAgentView {
                conversation_id,
                final_exchange_count,
                is_exit_before_new_entrance,
                ..
            } => {
                ctx.emit(ConversationSelectionEvent::Changed);
                ctx.emit(ConversationSelectionEvent::Deactivated {
                    conversation_id: *conversation_id,
                    final_exchange_count: *final_exchange_count,
                    is_exit_before_new_entrance: *is_exit_before_new_entrance,
                });
            }
            AgentViewControllerEvent::ExitConfirmed { .. } => {}
        });
        ctx.subscribe_to_model(
            &BlocklistAIHistoryModel::handle(ctx),
            |selection, _, event, ctx| selection.handle_history_event(event, ctx),
        );
        Self {
            terminal_surface_id,
            agent_view_controller,
        }
    }
}

impl ConversationSelection for AgentViewConversationSelection {
    fn selected_conversation_id(&self, app: &AppContext) -> Option<AIConversationId> {
        self.agent_view_controller
            .as_ref(app)
            .agent_view_state()
            .active_conversation_id()
    }

    fn is_conversation_active(&self, app: &AppContext) -> bool {
        self.agent_view_controller.as_ref(app).is_active()
    }

    fn is_conversation_fullscreen(&self, app: &AppContext) -> bool {
        self.agent_view_controller.as_ref(app).is_fullscreen()
    }

    fn select_existing_conversation(
        &mut self,
        conversation_id: AIConversationId,
        origin: AgentViewEntryOrigin,
        ctx: &mut ModelContext<Box<dyn ConversationSelection>>,
    ) {
        if let Err(error) = self.agent_view_controller.update(ctx, |controller, ctx| {
            controller.try_enter_agent_view(Some(conversation_id), origin, ctx)
        }) {
            report_error!(anyhow::Error::new(error)
                .context("Failed to enter agent view for existing conversation"));
        }
    }

    fn select_new_conversation(
        &mut self,
        origin: AgentViewEntryOrigin,
        ctx: &mut ModelContext<Box<dyn ConversationSelection>>,
    ) {
        if let Err(error) = self.agent_view_controller.update(ctx, |controller, ctx| {
            controller.try_enter_agent_view(None, origin, ctx)
        }) {
            report_error!(anyhow::Error::new(error)
                .context("Failed to enter agent view for new conversation"));
        }
    }

    fn try_start_new_conversation(
        &mut self,
        origin: AgentViewEntryOrigin,
        ctx: &mut ModelContext<Box<dyn ConversationSelection>>,
    ) -> Result<AIConversationId, EnterAgentViewError> {
        self.agent_view_controller.update(ctx, |controller, ctx| {
            controller.try_enter_agent_view(None, origin, ctx)
        })
    }

    fn pending_query_autoexecute_override(
        &self,
        app: &AppContext,
    ) -> AIConversationAutoexecuteMode {
        self.selected_conversation_id(app)
            .as_ref()
            .and_then(|conversation_id| {
                BlocklistAIHistoryModel::as_ref(app).conversation(conversation_id)
            })
            .map(|conversation| conversation.autoexecute_override())
            .unwrap_or_default()
    }

    fn toggle_pending_query_autoexecute(
        &mut self,
        ctx: &mut ModelContext<Box<dyn ConversationSelection>>,
    ) {
        if let Some(conversation_id) = self.selected_conversation_id(ctx) {
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                history.toggle_autoexecute_override(
                    &conversation_id,
                    self.terminal_surface_id,
                    ctx,
                );
            });
        }
    }

    fn handle_history_event(
        &mut self,
        event: &BlocklistAIHistoryEvent,
        ctx: &mut ModelContext<Box<dyn ConversationSelection>>,
    ) {
        if event
            .terminal_surface_id()
            .is_some_and(|id| id != self.terminal_surface_id)
        {
            return;
        }
        match event {
            BlocklistAIHistoryEvent::ClearedConversationsForTerminalSurface { .. } => {
                self.agent_view_controller
                    .update(ctx, |controller, ctx| controller.exit_agent_view(ctx));
            }
            BlocklistAIHistoryEvent::SplitConversation {
                old_conversation_id,
                new_conversation_id,
                ..
            } if self.selected_conversation_id(ctx) == Some(*old_conversation_id) => {
                self.select_existing_conversation(
                    *new_conversation_id,
                    AgentViewEntryOrigin::AgentRequestedNewConversation,
                    ctx,
                );
            }
            _ => {}
        }
    }
}

#[cfg(test)]
#[path = "conversation_selection_tests.rs"]
mod tests;
