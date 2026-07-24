//! Generic next-prompt conversation-selection behavior.

use warpui::{AppContext, Entity, ModelContext, ModelHandle, SingletonEntity};

use super::agent_view::{AgentViewEntryOrigin, EnterAgentViewError};
use super::{BlocklistAIHistoryEvent, BlocklistAIHistoryModel};
use crate::ai::agent::conversation::{
    AIConversation, AIConversationAutoexecuteMode, AIConversationId,
};
use crate::ai::agent_conversations_model::AgentConversationListPolicy;
#[cfg(any(test, feature = "test-util"))]
use crate::ai::agent_conversations_model::{
    AgentConversationEntry, AgentConversationListEntryState,
};

/// Handle to a terminal surface's conversation-selection implementation.
pub type ConversationSelectionHandle = ModelHandle<Box<dyn ConversationSelection>>;

/// The conversation targeted by the next query from a terminal surface.
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

/// Events emitted when a surface's conversation selection or presentation changes.
#[derive(Clone, Debug)]
pub enum ConversationSelectionEvent {
    /// The conversation targeted by the next query or its configuration changed.
    Changed,
    /// The surface began presenting a selected conversation.
    Activated {
        is_fullscreen: bool,
        origin: AgentViewEntryOrigin,
    },
    /// The surface stopped presenting a selected conversation.
    Deactivated {
        conversation_id: AIConversationId,
        final_exchange_count: usize,
        is_exit_before_new_entrance: bool,
    },
}
/// Coordinates the next-query target and conversation presentation for one terminal surface.
///
/// A selected conversation receives the surface's next query; without one, the next query starts a
/// new conversation. Implementations also describe whether that selection is actively presented
/// and whether it occupies the full surface, without exposing surface-specific presentation state.
pub trait ConversationSelection: AgentConversationListPolicy {
    /// Returns the conversation targeted by the next query.
    fn selected_conversation_id(&self, app: &AppContext) -> Option<AIConversationId>;

    /// Returns whether this surface presents a selected conversation as active.
    fn is_conversation_active(&self, app: &AppContext) -> bool;

    /// Returns whether an active conversation occupies the entire terminal surface.
    fn is_conversation_fullscreen(&self, app: &AppContext) -> bool;

    /// Selects an existing conversation for the next query.
    fn select_existing_conversation(
        &mut self,
        conversation_id: AIConversationId,
        origin: AgentViewEntryOrigin,
        ctx: &mut ModelContext<Box<dyn ConversationSelection>>,
    );

    /// Selects the new-conversation state for the next query.
    fn select_new_conversation(
        &mut self,
        origin: AgentViewEntryOrigin,
        ctx: &mut ModelContext<Box<dyn ConversationSelection>>,
    );

    /// Starts and selects a new conversation for this surface.
    fn try_start_new_conversation(
        &mut self,
        origin: AgentViewEntryOrigin,
        ctx: &mut ModelContext<Box<dyn ConversationSelection>>,
    ) -> Result<AIConversationId, EnterAgentViewError>;

    /// Returns the autoexecute override for the pending query.
    fn pending_query_autoexecute_override(&self, app: &AppContext)
    -> AIConversationAutoexecuteMode;

    /// Toggles the autoexecute override for the pending query.
    fn toggle_pending_query_autoexecute(
        &mut self,
        ctx: &mut ModelContext<Box<dyn ConversationSelection>>,
    );

    /// Reconciles selection after a terminal-surface-scoped history event.
    fn handle_history_event(
        &mut self,
        event: &BlocklistAIHistoryEvent,
        ctx: &mut ModelContext<Box<dyn ConversationSelection>>,
    );

    /// Returns the selected conversation, if it is loaded.
    fn selected_conversation<'a>(&self, app: &'a AppContext) -> Option<&'a AIConversation> {
        self.selected_conversation_id(app)
            .as_ref()
            .and_then(|conversation_id| {
                BlocklistAIHistoryModel::as_ref(app).conversation(conversation_id)
            })
    }
}

impl Entity for Box<dyn ConversationSelection> {
    type Event = ConversationSelectionEvent;
}

/// Inert [`ConversationSelection`] stub for tests: no selection, no-op writes.
#[cfg(any(test, feature = "test-util"))]
pub(crate) struct MockConversationSelection;

#[cfg(any(test, feature = "test-util"))]
impl AgentConversationListPolicy for MockConversationSelection {
    fn classify_entry(
        &self,
        _: &AgentConversationEntry,
        _: &AppContext,
    ) -> AgentConversationListEntryState {
        AgentConversationListEntryState::Unavailable
    }
}

#[cfg(any(test, feature = "test-util"))]
impl ConversationSelection for MockConversationSelection {
    fn selected_conversation_id(&self, _: &AppContext) -> Option<AIConversationId> {
        None
    }

    fn is_conversation_active(&self, _: &AppContext) -> bool {
        false
    }

    fn is_conversation_fullscreen(&self, _: &AppContext) -> bool {
        false
    }

    fn select_existing_conversation(
        &mut self,
        _: AIConversationId,
        _: AgentViewEntryOrigin,
        _: &mut ModelContext<Box<dyn ConversationSelection>>,
    ) {
    }

    fn select_new_conversation(
        &mut self,
        _: AgentViewEntryOrigin,
        _: &mut ModelContext<Box<dyn ConversationSelection>>,
    ) {
    }

    fn try_start_new_conversation(
        &mut self,
        _: AgentViewEntryOrigin,
        _: &mut ModelContext<Box<dyn ConversationSelection>>,
    ) -> Result<AIConversationId, EnterAgentViewError> {
        Ok(AIConversationId::new())
    }

    fn pending_query_autoexecute_override(&self, _: &AppContext) -> AIConversationAutoexecuteMode {
        AIConversationAutoexecuteMode::default()
    }

    fn toggle_pending_query_autoexecute(
        &mut self,
        _: &mut ModelContext<Box<dyn ConversationSelection>>,
    ) {
    }

    fn handle_history_event(
        &mut self,
        _: &BlocklistAIHistoryEvent,
        _: &mut ModelContext<Box<dyn ConversationSelection>>,
    ) {
    }
}
