use std::sync::Arc;

use parking_lot::FairMutex;
use warp::tui_export::{
    AIConversationAutoexecuteMode, AIConversationId, AgentConversationEntry,
    AgentConversationListEntryState, AgentConversationListPolicy, AgentRunDisplayStatus,
    AgentViewEntryOrigin, BlocklistAIHistoryEvent, BlocklistAIHistoryModel, ConversationSelection,
    ConversationSelectionEvent, EnterAgentViewError, Harness, PendingQueryState, TerminalModel,
};
use warpui::{AppContext, EntityId, ModelContext, SingletonEntity};

/// TUI-owned next-prompt conversation selection.
pub(super) struct TuiConversationSelection {
    terminal_surface_id: EntityId,
    terminal_model: Arc<FairMutex<TerminalModel>>,
    pending_query_state: PendingQueryState,
}

impl TuiConversationSelection {
    /// Creates TUI conversation selection for a terminal surface.
    pub(super) fn new(
        terminal_surface_id: EntityId,
        terminal_model: Arc<FairMutex<TerminalModel>>,
        ctx: &mut ModelContext<Box<dyn ConversationSelection>>,
    ) -> Self {
        let conversation_id = Self::start_new_conversation(terminal_surface_id, true, ctx);
        terminal_model
            .lock()
            .block_list_mut()
            .set_active_conversation_context(conversation_id, false, false);
        ctx.subscribe_to_model(
            &BlocklistAIHistoryModel::handle(ctx),
            |selection, _, event, ctx| selection.handle_history_event(event, ctx),
        );
        Self {
            terminal_surface_id,
            terminal_model,
            pending_query_state: PendingQueryState::Existing { conversation_id },
        }
    }

    /// Creates an empty conversation for a TUI terminal surface.
    fn start_new_conversation(
        terminal_surface_id: EntityId,
        is_autoexecute_override: bool,
        ctx: &mut ModelContext<Box<dyn ConversationSelection>>,
    ) -> AIConversationId {
        BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
            let conversation_id = history.start_new_conversation(
                terminal_surface_id,
                is_autoexecute_override,
                false,
                false,
                ctx,
            );
            history.set_active_conversation_id(conversation_id, terminal_surface_id, ctx);
            conversation_id
        })
    }

    /// Clears a removed selection and creates a replacement on the next event-loop tick.
    fn defer_replacement_conversation(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Box<dyn ConversationSelection>>,
    ) {
        self.set_terminal_conversation_context(None);
        self.set_pending_query_state(
            PendingQueryState::New {
                autoexecute_override: AIConversationAutoexecuteMode::RunToCompletion,
            },
            ctx,
        );
        Self::emit_deactivated(conversation_id, false, ctx);
        ctx.spawn(async {}, |selection, _, ctx| {
            if selection.selected_conversation_id(ctx).is_none() {
                selection.select_new_conversation(AgentViewEntryOrigin::Tui, ctx);
            }
        });
    }

    /// Updates command provenance for the TUI's selected conversation.
    fn set_terminal_conversation_context(&self, conversation_id: Option<AIConversationId>) {
        let mut terminal_model = self.terminal_model.lock();
        match conversation_id {
            Some(conversation_id) => terminal_model
                .block_list_mut()
                .set_active_conversation_context(conversation_id, false, false),
            None => terminal_model
                .block_list_mut()
                .clear_active_conversation_context(),
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

impl AgentConversationListPolicy for TuiConversationSelection {
    fn classify_entry(
        &self,
        entry: &AgentConversationEntry,
        _: &AppContext,
    ) -> AgentConversationListEntryState {
        classify_conversation_list_entry(
            self.selected_id(),
            entry.identity.local_conversation_id,
            entry.identity.server_conversation_token.is_some(),
            entry.display.harness,
            &entry.display.status,
        )
    }
}

fn classify_conversation_list_entry(
    selected_id: Option<AIConversationId>,
    local_conversation_id: Option<AIConversationId>,
    has_server_token: bool,
    harness: Option<Harness>,
    status: &AgentRunDisplayStatus,
) -> AgentConversationListEntryState {
    if selected_id.is_some_and(|selected_id| local_conversation_id == Some(selected_id)) {
        return AgentConversationListEntryState::Selected;
    }
    if harness != Some(Harness::Oz) {
        return AgentConversationListEntryState::Unavailable;
    }

    let has_terminal_status = match status {
        AgentRunDisplayStatus::TaskQueued
        | AgentRunDisplayStatus::TaskPending
        | AgentRunDisplayStatus::TaskClaimed
        | AgentRunDisplayStatus::TaskInProgress
        | AgentRunDisplayStatus::TaskBlocked { .. }
        | AgentRunDisplayStatus::ConversationInProgress
        | AgentRunDisplayStatus::ConversationBlocked { .. } => false,
        AgentRunDisplayStatus::TaskSucceeded
        | AgentRunDisplayStatus::TaskFailed
        | AgentRunDisplayStatus::TaskError
        | AgentRunDisplayStatus::TaskCancelled
        | AgentRunDisplayStatus::TaskUnknown
        | AgentRunDisplayStatus::ConversationSucceeded
        | AgentRunDisplayStatus::ConversationError
        | AgentRunDisplayStatus::ConversationCancelled => true,
    };
    if has_terminal_status && (local_conversation_id.is_some() || has_server_token) {
        AgentConversationListEntryState::Available
    } else {
        AgentConversationListEntryState::Unavailable
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
        self.set_terminal_conversation_context(Some(conversation_id));
        self.set_pending_query_state(PendingQueryState::Existing { conversation_id }, ctx);
        Self::emit_activated(origin, ctx);
    }

    fn select_new_conversation(
        &mut self,
        origin: AgentViewEntryOrigin,
        ctx: &mut ModelContext<Box<dyn ConversationSelection>>,
    ) {
        let previous_conversation_id = self.selected_id();
        // TODO: Implement actual permissions once settings are in place and there is a UI for permissions requests.
        // For now, we just always set fast-forward to on.

        if let Some(previous_conversation_id) = previous_conversation_id {
            Self::emit_deactivated(previous_conversation_id, true, ctx);
        }
        let conversation_id = Self::start_new_conversation(self.terminal_surface_id, true, ctx);
        self.set_terminal_conversation_context(Some(conversation_id));
        self.set_pending_query_state(PendingQueryState::Existing { conversation_id }, ctx);
        Self::emit_activated(origin, ctx);
    }

    fn try_start_new_conversation(
        &mut self,
        origin: AgentViewEntryOrigin,
        ctx: &mut ModelContext<Box<dyn ConversationSelection>>,
    ) -> Result<AIConversationId, EnterAgentViewError> {
        self.select_new_conversation(origin, ctx);
        Ok(self
            .selected_id()
            .expect("TUI new-conversation selection should be eager"))
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
            BlocklistAIHistoryEvent::ClearedConversationsForTerminalSurface {
                active_conversation_id,
                cleared_conversation_ids,
                ..
            } => {
                let selected_conversation_id = self.selected_id();
                let selected_conversation_was_cleared =
                    selected_conversation_id.is_some_and(|conversation_id| {
                        active_conversation_id == &Some(conversation_id)
                            || cleared_conversation_ids.contains(&conversation_id)
                    });
                if selected_conversation_was_cleared {
                    self.defer_replacement_conversation(
                        selected_conversation_id
                            .expect("cleared selection should have a conversation ID"),
                        ctx,
                    );
                }
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
                self.defer_replacement_conversation(*conversation_id, ctx);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
#[path = "conversation_selection_tests.rs"]
mod tests;
