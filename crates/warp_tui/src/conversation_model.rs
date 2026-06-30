//! Reusable per-surface TUI conversation coordination.
use anyhow::Result;
use warp::tui_export::{
    AIConversationId, AgentViewEntryOrigin, BlocklistAIController, ConversationSelectionHandle,
};
use warpui::{AppContext, Entity, ModelContext, ModelHandle};

/// Per-surface conversation/composer model for a future interactive TUI.
///
/// This model deliberately contains no transcript widgets. It coordinates selected
/// conversation state and prompt submission for one TUI selection.
pub(super) struct TuiConversationModel {
    conversation_selection: ConversationSelectionHandle,
    ai_controller: ModelHandle<BlocklistAIController>,
}

impl TuiConversationModel {
    /// Creates a TUI conversation model around the shared production AI models.
    pub(super) fn new(
        _terminal_surface_id: warpui::EntityId,
        conversation_selection: ConversationSelectionHandle,
        ai_controller: ModelHandle<BlocklistAIController>,
        _ctx: &mut ModelContext<Self>,
    ) -> Self {
        Self {
            conversation_selection,
            ai_controller,
        }
    }

    /// Returns this surface's currently selected next-prompt target.
    fn selected_conversation_id(&self, ctx: &AppContext) -> Option<AIConversationId> {
        self.conversation_selection
            .as_ref(ctx)
            .selected_conversation_id(ctx)
    }

    /// Creates and selects an empty conversation for this TUI selection.
    fn start_new_conversation(&mut self, ctx: &mut ModelContext<Self>) -> Result<AIConversationId> {
        self.conversation_selection
            .update(ctx, |selection, ctx| {
                selection.try_start_new_conversation(AgentViewEntryOrigin::Cli, ctx)
            })
            .map_err(Into::into)
    }

    /// Sends a prompt to this surface's selected conversation, creating one if needed.
    pub(super) fn send_prompt(&mut self, prompt: String, ctx: &mut ModelContext<Self>) {
        let conversation_id = match self.selected_conversation_id(ctx) {
            Some(conversation_id) => conversation_id,
            None => match self.start_new_conversation(ctx) {
                Ok(conversation_id) => conversation_id,
                Err(error) => {
                    log::error!("Failed to create TUI conversation: {error:#}");
                    return;
                }
            },
        };
        self.ai_controller.update(ctx, |controller, ctx| {
            controller.send_user_query_in_conversation(prompt, conversation_id, None, ctx);
        });
    }
}

impl Entity for TuiConversationModel {
    type Event = ();
}
