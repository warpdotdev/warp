use warp::tui_export::{
    AIConversationAutoexecuteMode, AIConversationId, AgentViewEntryOrigin, BlocklistAIHistoryEvent,
    BlocklistAIHistoryModel, ConversationSelection, ConversationSelectionEvent,
    EnterAgentViewError, PendingQueryState,
};
use warpui::{AppContext, EntityId, ModelContext, SingletonEntity};

/// TUI-owned next-prompt conversation selection.
pub(super) struct TuiConversationSelection {
    terminal_surface_id: EntityId,
    pending_query_state: PendingQueryState,
}

impl TuiConversationSelection {
    /// Creates TUI conversation selection for a terminal surface.
    pub(super) fn new(
        terminal_surface_id: EntityId,
        ctx: &mut ModelContext<Box<dyn ConversationSelection>>,
    ) -> Self {
        ctx.subscribe_to_model(
            &BlocklistAIHistoryModel::handle(ctx),
            |selection, _, event, ctx| selection.handle_history_event(event, ctx),
        );

        // TODO: Implement actual permissions once settings are in place and there is a UI for permissions requests.
        // For now, we just always set fast-forward to on.
        let pending_query_state = PendingQueryState::New {
            autoexecute_override: AIConversationAutoexecuteMode::RunToCompletion,
        };

        Self {
            terminal_surface_id,
            pending_query_state,
        }
    }

    /// Returns the selected existing conversation ID.
    fn selected_id(&self) -> Option<AIConversationId> {
        match self.pending_query_state {
            PendingQueryState::Existing { conversation_id } => Some(conversation_id),
            PendingQueryState::New { .. } => None,
        }
    }

    /// Updates pending state and emits only when the value changes.
    fn set_pending_query_state(
        &mut self,
        state: PendingQueryState,
        ctx: &mut ModelContext<Box<dyn ConversationSelection>>,
    ) {
        if self.pending_query_state != state {
            self.pending_query_state = state;
            ctx.emit(ConversationSelectionEvent::Changed);
        }
    }

    /// Emits activation for a selected TUI conversation.
    fn emit_activated(
        origin: AgentViewEntryOrigin,
        ctx: &mut ModelContext<Box<dyn ConversationSelection>>,
    ) {
        ctx.emit(ConversationSelectionEvent::Activated {
            is_fullscreen: true,
            origin,
        });
    }

    /// Emits deactivation for a previously selected TUI conversation.
    fn emit_deactivated(
        conversation_id: AIConversationId,
        is_exit_before_new_entrance: bool,
        ctx: &mut ModelContext<Box<dyn ConversationSelection>>,
    ) {
        let final_exchange_count = BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(&conversation_id)
            .map(|conversation| conversation.exchange_count())
            .unwrap_or(0);
        ctx.emit(ConversationSelectionEvent::Deactivated {
            conversation_id,
            final_exchange_count,
            is_exit_before_new_entrance,
        });
    }
}

impl ConversationSelection for TuiConversationSelection {
    fn selected_conversation_id(&self, _: &AppContext) -> Option<AIConversationId> {
        self.selected_id()
    }

    fn is_conversation_active(&self, _: &AppContext) -> bool {
        self.selected_id().is_some()
    }
    /// The TUI has no terminal/Agent View split, so every selected conversation is fullscreen.
    fn is_conversation_fullscreen(&self, _: &AppContext) -> bool {
        self.selected_id().is_some()
    }

    fn select_existing_conversation(
        &mut self,
        conversation_id: AIConversationId,
        origin: AgentViewEntryOrigin,
        ctx: &mut ModelContext<Box<dyn ConversationSelection>>,
    ) {
        let previous_conversation_id = self.selected_id();
        if previous_conversation_id == Some(conversation_id) {
            return;
        }
        if let Some(previous_conversation_id) = previous_conversation_id {
            Self::emit_deactivated(previous_conversation_id, true, ctx);
        }
        self.set_pending_query_state(PendingQueryState::Existing { conversation_id }, ctx);
        Self::emit_activated(origin, ctx);
    }

    fn select_new_conversation(
        &mut self,
        _: AgentViewEntryOrigin,
        ctx: &mut ModelContext<Box<dyn ConversationSelection>>,
    ) {
        let previous_conversation_id = self.selected_id();
        // TODO: Implement actual permissions once settings are in place and there is a UI for permissions requests.
        // For now, we just always set fast-forward to on.
        self.set_pending_query_state(
            PendingQueryState::New {
                autoexecute_override: AIConversationAutoexecuteMode::RunToCompletion,
            },
            ctx,
        );

        if let Some(previous_conversation_id) = previous_conversation_id {
            Self::emit_deactivated(previous_conversation_id, false, ctx);
        }
    }

    fn try_start_new_conversation(
        &mut self,
        origin: AgentViewEntryOrigin,
        ctx: &mut ModelContext<Box<dyn ConversationSelection>>,
    ) -> Result<AIConversationId, EnterAgentViewError> {
        if let Some(previous_conversation_id) = self.selected_id() {
            Self::emit_deactivated(previous_conversation_id, true, ctx);
        }
        let is_autoexecute_override = matches!(
            self.pending_query_state,
            PendingQueryState::New {
                autoexecute_override: AIConversationAutoexecuteMode::RunToCompletion,
            }
        );
        let conversation_id = BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
            history.start_new_conversation(
                self.terminal_surface_id,
                is_autoexecute_override,
                false,
                false,
                ctx,
            )
        });
        self.set_pending_query_state(PendingQueryState::Existing { conversation_id }, ctx);
        Self::emit_activated(origin, ctx);
        Ok(conversation_id)
    }

    fn pending_query_autoexecute_override(
        &self,
        app: &AppContext,
    ) -> AIConversationAutoexecuteMode {
        match &self.pending_query_state {
            PendingQueryState::New {
                autoexecute_override,
            } => *autoexecute_override,
            PendingQueryState::Existing { conversation_id } => BlocklistAIHistoryModel::as_ref(app)
                .conversation(conversation_id)
                .map(|conversation| conversation.autoexecute_override())
                .unwrap_or_default(),
        }
    }

    fn toggle_pending_query_autoexecute(
        &mut self,
        ctx: &mut ModelContext<Box<dyn ConversationSelection>>,
    ) {
        match self.pending_query_state.clone() {
            PendingQueryState::New {
                autoexecute_override,
            } => {
                let autoexecute_override =
                    if autoexecute_override == AIConversationAutoexecuteMode::RespectUserSettings {
                        AIConversationAutoexecuteMode::RunToCompletion
                    } else {
                        AIConversationAutoexecuteMode::RespectUserSettings
                    };
                self.set_pending_query_state(
                    PendingQueryState::New {
                        autoexecute_override,
                    },
                    ctx,
                );
            }
            PendingQueryState::Existing { conversation_id } => {
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                    history.toggle_autoexecute_override(
                        &conversation_id,
                        self.terminal_surface_id,
                        ctx,
                    );
                });
            }
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
                self.select_new_conversation(AgentViewEntryOrigin::Cli, ctx);
            }
            BlocklistAIHistoryEvent::SplitConversation {
                old_conversation_id,
                new_conversation_id,
                ..
            } if self.selected_id() == Some(*old_conversation_id) => {
                self.select_existing_conversation(
                    *new_conversation_id,
                    AgentViewEntryOrigin::AgentRequestedNewConversation,
                    ctx,
                );
            }
            BlocklistAIHistoryEvent::RemoveConversation {
                conversation_id, ..
            }
            | BlocklistAIHistoryEvent::DeletedConversation {
                conversation_id, ..
            }
            | BlocklistAIHistoryEvent::ConversationTransferredBetweenTerminalSurfaces {
                conversation_id,
                ..
            } if self.selected_id() == Some(*conversation_id) => {
                self.select_new_conversation(AgentViewEntryOrigin::Cli, ctx);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
#[path = "conversation_selection_tests.rs"]
mod tests;
