//! Reusable per-surface TUI conversation coordination.

use anyhow::anyhow;
use warp::tui_export::{
    AIConversationId, AgentViewEntryOrigin, BlocklistAIController, BlocklistAIHistoryEvent,
    BlocklistAIHistoryModel, CloudConversationData, ConversationSelectionEvent,
    ConversationSelectionHandle, ConversationStatus, ConversationStatusUpdate,
    ServerConversationToken,
};
use warpui::{AppContext, Entity, EntityId, ModelContext, ModelHandle, SingletonEntity};

/// Events emitted by a TUI conversation model for presentation layers.
#[derive(Clone, Debug)]
pub(super) enum TuiConversationModelEvent {
    SelectedConversationChanged {
        conversation_id: Option<AIConversationId>,
    },
    ConversationStarted {
        conversation_id: AIConversationId,
    },
    ConversationUpdated {
        conversation_id: AIConversationId,
    },
    ConversationStatusChanged {
        conversation_id: AIConversationId,
        status: ConversationStatus,
        update: ConversationStatusUpdate,
    },
    Error {
        message: String,
    },
}

/// Per-surface conversation/composer model for a future interactive TUI.
///
/// This model deliberately contains no transcript widgets. It coordinates selected
/// conversation state, conversation restore/create operations, prompt
/// submission, and history-backed stream events for one TUI selection.
pub(super) struct TuiConversationModel {
    terminal_surface_id: EntityId,
    conversation_selection: ConversationSelectionHandle,
    ai_controller: ModelHandle<BlocklistAIController>,
}

impl TuiConversationModel {
    /// Creates a TUI conversation model around the shared production AI models.
    pub(super) fn new(
        terminal_surface_id: EntityId,
        conversation_selection: ConversationSelectionHandle,
        ai_controller: ModelHandle<BlocklistAIController>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        ctx.subscribe_to_model(&conversation_selection, |model, _, event, ctx| {
            if matches!(event, ConversationSelectionEvent::Changed) {
                ctx.emit(TuiConversationModelEvent::SelectedConversationChanged {
                    conversation_id: model.selected_conversation_id(ctx),
                });
            }
        });
        ctx.subscribe_to_model(
            &BlocklistAIHistoryModel::handle(ctx),
            |model, _, event, ctx| model.handle_history_event(event, ctx),
        );
        Self {
            terminal_surface_id,
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

    /// Selects a live conversation as this surface's next-prompt target.
    fn select_conversation(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) -> anyhow::Result<()> {
        let is_live = BlocklistAIHistoryModel::as_ref(ctx)
            .all_live_conversations_for_terminal_surface(self.terminal_surface_id)
            .any(|conversation| conversation.id() == conversation_id);
        if !is_live {
            return Err(anyhow!(
                "Conversation {conversation_id} is not live for TUI surface {}",
                self.terminal_surface_id
            ));
        }
        self.conversation_selection.update(ctx, |selection, ctx| {
            selection.select_existing_conversation(conversation_id, AgentViewEntryOrigin::Cli, ctx);
        });
        Ok(())
    }

    /// Creates and selects an empty conversation for this TUI selection.
    fn start_new_conversation(
        &mut self,
        ctx: &mut ModelContext<Self>,
    ) -> anyhow::Result<AIConversationId> {
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
                    ctx.emit(TuiConversationModelEvent::Error {
                        message: format!("{error:#}"),
                    });
                    return;
                }
            },
        };
        self.ai_controller.update(ctx, |controller, ctx| {
            controller.send_user_query_in_conversation(prompt, conversation_id, None, ctx);
        });
    }

    /// Restores, selects, and sends a prompt to a conversation identified by its server token.
    pub(super) fn restore_conversation_by_server_token_and_send_prompt(
        &mut self,
        prompt: String,
        server_conversation_token: ServerConversationToken,
        ctx: &mut ModelContext<Self>,
    ) {
        let history = BlocklistAIHistoryModel::handle(ctx);
        if let Some(conversation_id) = history
            .as_ref(ctx)
            .find_conversation_id_by_server_token(&server_conversation_token)
        {
            let is_live = history
                .as_ref(ctx)
                .all_live_conversations_for_terminal_surface(self.terminal_surface_id)
                .any(|conversation| conversation.id() == conversation_id);
            if is_live {
                if let Err(error) = self.select_conversation(conversation_id, ctx) {
                    ctx.emit(TuiConversationModelEvent::Error {
                        message: format!("{error:#}"),
                    });
                    return;
                }
                self.send_prompt(prompt, ctx);
                return;
            }
            if let Some(conversation) = history.as_ref(ctx).conversation(&conversation_id).cloned()
            {
                history.update(ctx, |history, ctx| {
                    history.restore_conversations(
                        self.terminal_surface_id,
                        vec![conversation],
                        ctx,
                    );
                });
                if let Err(error) = self.select_conversation(conversation_id, ctx) {
                    ctx.emit(TuiConversationModelEvent::Error {
                        message: format!("{error:#}"),
                    });
                    return;
                }
                self.send_prompt(prompt, ctx);
                return;
            }
        }

        let token_for_error = server_conversation_token.as_str().to_owned();
        let future = history.update(ctx, |history, ctx| {
            history.load_conversation_by_server_token(&server_conversation_token, ctx)
        });
        ctx.spawn(future, move |model, conversation, ctx| {
            let Some(CloudConversationData::Oz(conversation)) = conversation else {
                ctx.emit(TuiConversationModelEvent::Error {
                    message: format!(
                        "Failed to load conversation with server token {token_for_error}"
                    ),
                });
                return;
            };
            let conversation_id = conversation.id();
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                history.restore_conversations(model.terminal_surface_id, vec![*conversation], ctx);
            });
            if let Err(error) = model.select_conversation(conversation_id, ctx) {
                ctx.emit(TuiConversationModelEvent::Error {
                    message: format!("{error:#}"),
                });
                return;
            }
            model.send_prompt(prompt, ctx);
        });
    }

    /// Converts terminal-surface-scoped history events into TUI presentation events.
    fn handle_history_event(
        &mut self,
        event: &BlocklistAIHistoryEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        if event
            .terminal_surface_id()
            .is_some_and(|terminal_surface_id| terminal_surface_id != self.terminal_surface_id)
        {
            return;
        }
        match event {
            BlocklistAIHistoryEvent::StartedNewConversation {
                new_conversation_id,
                ..
            } => ctx.emit(TuiConversationModelEvent::ConversationStarted {
                conversation_id: *new_conversation_id,
            }),
            BlocklistAIHistoryEvent::UpdatedStreamingExchange {
                conversation_id, ..
            } => ctx.emit(TuiConversationModelEvent::ConversationUpdated {
                conversation_id: *conversation_id,
            }),
            BlocklistAIHistoryEvent::UpdatedConversationStatus {
                conversation_id,
                new_status,
                update,
                ..
            } => ctx.emit(TuiConversationModelEvent::ConversationStatusChanged {
                conversation_id: *conversation_id,
                status: new_status.clone(),
                update: update.clone(),
            }),
            _ => {}
        }
    }
}

impl Entity for TuiConversationModel {
    type Event = TuiConversationModelEvent;
}
