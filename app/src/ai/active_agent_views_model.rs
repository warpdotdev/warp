use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use warpui::{
    AppContext, Entity, EntityId, ModelContext, ModelHandle, SingletonEntity, WeakModelHandle,
    WindowId,
};

use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent_conversations_model::{AgentConversationEntry, AgentConversationEntryId};
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::blocklist::agent_view::{AgentViewController, AgentViewControllerEvent};
use crate::ai::blocklist::orchestration_event_streamer::{
    register_agent_event_consumer, unregister_agent_event_consumer,
};
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::terminal::model::session::active_session::ActiveSession;

/// Contains the handles needed to track an active agent view.
struct ActiveAgentViewHandles {
    controller: WeakModelHandle<AgentViewController>,
    active_session: WeakModelHandle<ActiveSession>,
}

#[derive(Clone)]
pub enum ActiveAgentViewsEvent {
    /// A conversation was closed (exited from the agent view or its pane was removed).
    ConversationClosed { conversation_id: AIConversationId },
    /// A conversation was entered within a terminal view.
    TerminalViewFocused,
    /// An ambient agent session was opened in a tab.
    AmbientSessionOpened {
        #[allow(dead_code)]
        task_id: AmbientAgentTaskId,
    },
    /// An ambient agent session tab was closed.
    AmbientSessionClosed {
        #[allow(dead_code)]
        task_id: AmbientAgentTaskId,
    },
    /// A window was closed and its focused state was removed.
    WindowClosed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ConversationOrTaskId {
    ConversationId(AIConversationId),
    TaskId(AmbientAgentTaskId),
}

impl From<AgentConversationEntryId> for ConversationOrTaskId {
    fn from(id: AgentConversationEntryId) -> Self {
        match id {
            AgentConversationEntryId::Conversation(conversation_id) => {
                ConversationOrTaskId::ConversationId(conversation_id)
            }
            AgentConversationEntryId::AmbientRun(task_id) => ConversationOrTaskId::TaskId(task_id),
        }
    }
}

impl ConversationOrTaskId {
    pub fn conversation_id(&self) -> Option<AIConversationId> {
        match self {
            ConversationOrTaskId::ConversationId(conversation_id) => Some(*conversation_id),
            ConversationOrTaskId::TaskId(..) => None,
        }
    }
}

/// State of the focused terminal view and the active conversation in that terminal view.
#[derive(Clone)]
struct FocusedTerminalState {
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    focused_terminal_id: EntityId,
    active_conversation_id: Option<ConversationOrTaskId>,
}

/// ActiveAgentViewsModel tracks which agent conversations are currently "active" - meaning either:
/// - An interactive conversation whose agent view is expanded in a pane
/// - An ambient conversation that is open in a tab
/// This model also tracks which conversation is focused (i.e. active in the currently focused pane).
pub struct ActiveAgentViewsModel {
    /// Per-window focused terminal state, keyed by WindowId.
    focused_terminal_states: HashMap<WindowId, FocusedTerminalState>,
    last_focused_terminal_state: Option<FocusedTerminalState>,
    /// Map from terminal_view_id to agent view handles (for interactive conversations).
    agent_view_handles: HashMap<EntityId, ActiveAgentViewHandles>,
    /// Map from terminal_view_id to ambient task ID (for open ambient sessions).
    ambient_sessions: HashMap<EntityId, AmbientAgentTaskId>,
    /// Tracks when each conversation was last opened/focused for sorting purposes.
    last_opened_times: HashMap<ConversationOrTaskId, DateTime<Utc>>,
}

impl Entity for ActiveAgentViewsModel {
    type Event = ActiveAgentViewsEvent;
}

impl SingletonEntity for ActiveAgentViewsModel {}

impl ActiveAgentViewsModel {
    pub fn new() -> Self {
        Self {
            focused_terminal_states: HashMap::new(),
            last_focused_terminal_state: None,
            agent_view_handles: HashMap::new(),
            ambient_sessions: HashMap::new(),
            last_opened_times: HashMap::new(),
        }
    }

    fn update_focused_conversation_for_terminal(
        &mut self,
        terminal_view_id: EntityId,
        conversation_id: Option<ConversationOrTaskId>,
    ) {
        for focused_terminal_state in self.focused_terminal_states.values_mut() {
            if focused_terminal_state.focused_terminal_id == terminal_view_id
                && !matches!(
                    focused_terminal_state.active_conversation_id,
                    Some(ConversationOrTaskId::TaskId(_))
                )
            {
                focused_terminal_state.active_conversation_id = conversation_id;
            }
        }

        if let Some(last_focused_terminal_state) = &mut self.last_focused_terminal_state {
            if last_focused_terminal_state.focused_terminal_id == terminal_view_id
                && !matches!(
                    last_focused_terminal_state.active_conversation_id,
                    Some(ConversationOrTaskId::TaskId(_))
                )
            {
                last_focused_terminal_state.active_conversation_id = conversation_id;
            }
        }
    }

    /// Register an agent view controller to track when the agent view is entered/exited.
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    pub fn register_agent_view_controller(
        &mut self,
        controller: &ModelHandle<AgentViewController>,
        active_session: &ModelHandle<ActiveSession>,
        terminal_view_id: EntityId,
        ctx: &mut ModelContext<Self>,
    ) {
        // Skip registering this controller if it is already registered.
        if let Some(existing) = self.agent_view_handles.get(&terminal_view_id) {
            if existing
                .controller
                .upgrade(ctx)
                .is_some_and(|c| c.id() == controller.id())
            {
                return;
            }
        }

        self.agent_view_handles.insert(
            terminal_view_id,
            ActiveAgentViewHandles {
                controller: controller.downgrade(),
                active_session: active_session.downgrade(),
            },
        );

        // On pane re-attach the controller's `agent_view_state` is still
        // `Active` while the unregister path has already torn down the
        // streamer consumer. Re-register here; the `EnteredAgentView`
        // subscription only fires on subsequent state transitions.
        if let Some(conversation_id) = controller
            .as_ref(ctx)
            .agent_view_state()
            .active_conversation_id()
        {
            register_agent_event_consumer(conversation_id, terminal_view_id, ctx);
        }

        ctx.subscribe_to_model(controller, move |model, _, event, ctx| match event {
            AgentViewControllerEvent::EnteredAgentView {
                conversation_id, ..
            } => {
                let conv_id = ConversationOrTaskId::ConversationId(*conversation_id);
                model.last_opened_times.insert(conv_id, Utc::now());

                // Update the focused conversation in whichever window owns this terminal view.
                // We ignore agent view changes if we are focused on an ambient conversation,
                // as ambient conversation navigation operates at the task level instead of the conversation level.
                model.update_focused_conversation_for_terminal(terminal_view_id, Some(conv_id));
                // Bridge the controller's lifecycle into the streamer's
                // per-conversation consumer registry.
                register_agent_event_consumer(*conversation_id, terminal_view_id, ctx);
                // Emit so subscribers can move this conversation to the Active section.
                ctx.emit(ActiveAgentViewsEvent::TerminalViewFocused);
            }
            AgentViewControllerEvent::ExitedAgentView {
                conversation_id,
                is_exit_before_new_entrance,
                ..
            } => {
                // Skip if this exit is part of an in-place switch — the follow-up
                // entrance will register the new conversation's consumer.
                if *is_exit_before_new_entrance {
                    return;
                }

                model
                    .last_opened_times
                    .remove(&ConversationOrTaskId::ConversationId(*conversation_id));

                // Clear the focused conversation in whichever window owns this terminal view.
                model.update_focused_conversation_for_terminal(terminal_view_id, None);
                unregister_agent_event_consumer(*conversation_id, terminal_view_id, ctx);
                // Emit so subscribers can move this conversation to the Past section.
                ctx.emit(ActiveAgentViewsEvent::ConversationClosed {
                    conversation_id: *conversation_id,
                });
            }
            _ => {}
        });
    }

    /// Unregister an agent view controller
    /// (called when the controller's terminal pane is hidden or closed).
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    pub fn unregister_agent_view_controller(
        &mut self,
        terminal_pane_id: EntityId,
        ctx: &mut ModelContext<Self>,
    ) {
        if let Some(handles) = self.agent_view_handles.remove(&terminal_pane_id) {
            let closed_conversation_id = handles
                .controller
                .upgrade(ctx)
                .and_then(|c| c.as_ref(ctx).agent_view_state().active_conversation_id());

            // If the focused terminal is the one being unregistered, clear the focused state.
            self.focused_terminal_states
                .retain(|_, state| state.focused_terminal_id != terminal_pane_id);
            if self
                .last_focused_terminal_state
                .as_ref()
                .is_some_and(|state| state.focused_terminal_id == terminal_pane_id)
            {
                self.last_focused_terminal_state = None;
            }

            if let Some(conversation_id) = closed_conversation_id {
                // The pane-close path bypasses exit_agent_view_internal, so
                // unregister the streamer consumer here.
                unregister_agent_event_consumer(conversation_id, terminal_pane_id, ctx);
                ctx.emit(ActiveAgentViewsEvent::ConversationClosed { conversation_id });
            }
        }
    }

    pub fn handle_pane_focus_change(
        &mut self,
        window_id: WindowId,
        focused_terminal_view_id: Option<EntityId>,
        focused_task_id: Option<AmbientAgentTaskId>,
        ctx: &mut ModelContext<Self>,
    ) {
        let old_focused = self.get_focused_conversation(window_id);

        if let Some(terminal_view_id) = focused_terminal_view_id {
            // Task ID takes precedence if viewing a shared ambient agent session.
            let active_conversation_id = if let Some(task_id) = focused_task_id {
                Some(ConversationOrTaskId::TaskId(task_id))
            } else {
                self.agent_view_handles
                    .get(&terminal_view_id)
                    .and_then(|handles| handles.controller.upgrade(ctx))
                    .and_then(|controller| {
                        controller
                            .as_ref(ctx)
                            .agent_view_state()
                            .active_conversation_id()
                    })
                    .map(ConversationOrTaskId::ConversationId)
            };

            let new_state = FocusedTerminalState {
                focused_terminal_id: terminal_view_id,
                active_conversation_id,
            };
            self.last_focused_terminal_state = Some(new_state.clone());
            self.focused_terminal_states.insert(window_id, new_state);
        } else {
            self.focused_terminal_states.remove(&window_id);
        }

        if old_focused != self.get_focused_conversation(window_id) {
            ctx.emit(ActiveAgentViewsEvent::TerminalViewFocused);
        }
    }

    /// Get the focused conversation for a specific window.
    /// Returns None if the window doesn't have an active agent view or ambient conversation.
    pub fn get_focused_conversation(&self, window_id: WindowId) -> Option<ConversationOrTaskId> {
        self.focused_terminal_states
            .get(&window_id)
            .and_then(|state| state.active_conversation_id)
    }

    pub fn get_focused_terminal_view_id(&self, window_id: WindowId) -> Option<EntityId> {
        self.focused_terminal_states
            .get(&window_id)
            .map(|state| state.focused_terminal_id)
    }

    /// Get the last focused terminal view id (persisted across non-terminal focus changes).
    pub fn get_last_focused_terminal_id(&self) -> Option<EntityId> {
        self.last_focused_terminal_state
            .as_ref()
            .map(|state| state.focused_terminal_id)
    }

    /// Get the most recent focused conversation or ambient task ID, persisted
    /// across non-terminal focus changes.
    pub fn get_last_focused_conversation(&self) -> Option<ConversationOrTaskId> {
        self.last_focused_terminal_state
            .as_ref()
            .and_then(|state| state.active_conversation_id)
    }

    /// Returns the focused conversation ID if it's a new/empty conversation view.
    /// Only returns Some if the focused agent view was just created to start a new
    /// conversation (i.e. has no exchanges yet).
    pub fn maybe_get_focused_new_conversation(
        &self,
        window_id: WindowId,
        ctx: &AppContext,
    ) -> Option<AIConversationId> {
        let state = self.focused_terminal_states.get(&window_id)?;
        let terminal_id = state.focused_terminal_id;

        let is_new = self
            .agent_view_handles
            .get(&terminal_id)
            .and_then(|handles| handles.controller.upgrade(ctx))
            .map(|c| c.as_ref(ctx).agent_view_state().is_new())
            .unwrap_or(false);

        if is_new {
            match state.active_conversation_id {
                Some(ConversationOrTaskId::ConversationId(id)) => Some(id),
                _ => None,
            }
        } else {
            None
        }
    }

    /// Remove the focused state for a window
    /// (called when said window is closed and cleaned up from the undo stack).
    pub fn remove_focused_state_for_window(
        &mut self,
        window_id: WindowId,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.focused_terminal_states.remove(&window_id).is_some() {
            ctx.emit(ActiveAgentViewsEvent::WindowClosed);
        }
    }

    /// Register an ambient session (open in a tab).
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    pub fn register_ambient_session(
        &mut self,
        terminal_view_id: EntityId,
        task_id: AmbientAgentTaskId,
        ctx: &mut ModelContext<Self>,
    ) {
        self.ambient_sessions
            .retain(|view_id, id| *view_id == terminal_view_id || *id != task_id);
        let existing = self.ambient_sessions.insert(terminal_view_id, task_id);
        if existing != Some(task_id) {
            self.last_opened_times
                .insert(ConversationOrTaskId::TaskId(task_id), Utc::now());
            ctx.emit(ActiveAgentViewsEvent::AmbientSessionOpened { task_id });
        }
    }

    /// Unregister an ambient session when the tab is closed.
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    pub fn unregister_ambient_session(
        &mut self,
        terminal_view_id: EntityId,
        ctx: &mut ModelContext<Self>,
    ) {
        if let Some(task_id) = self.ambient_sessions.remove(&terminal_view_id) {
            if !self.ambient_sessions.values().any(|id| *id == task_id) {
                self.last_opened_times
                    .remove(&ConversationOrTaskId::TaskId(task_id));
                ctx.emit(ActiveAgentViewsEvent::AmbientSessionClosed { task_id });
            }
        }
    }

    /// Returns the terminal view ID for a conversation if it's currently active
    /// (i.e., has an expanded agent view in some pane).
    pub fn terminal_view_id_for_conversation(
        &self,
        conversation_id: AIConversationId,
        ctx: &AppContext,
    ) -> Option<EntityId> {
        self.agent_view_handles
            .iter()
            .find_map(|(terminal_view_id, handles)| {
                let controller = handles.controller.upgrade(ctx)?;
                controller
                    .as_ref(ctx)
                    .agent_view_state()
                    .active_conversation_id()
                    .is_some_and(|id| id == conversation_id)
                    .then_some(*terminal_view_id)
            })
    }

    /// Returns true if the conversation is currently open
    /// (i.e., has an expanded agent view in some pane).
    pub fn is_conversation_open(
        &self,
        conversation_id: AIConversationId,
        ctx: &AppContext,
    ) -> bool {
        self.terminal_view_id_for_conversation(conversation_id, ctx)
            .is_some()
    }

    /// Returns the active session for a conversation if it's currently active
    /// (i.e., has an expanded agent view).
    pub fn get_active_session_for_conversation(
        &self,
        conversation_id: AIConversationId,
        ctx: &AppContext,
    ) -> Option<ModelHandle<ActiveSession>> {
        for handles in self.agent_view_handles.values() {
            let Some(controller) = handles.controller.upgrade(ctx) else {
                continue;
            };
            let is_active = controller
                .as_ref(ctx)
                .agent_view_state()
                .active_conversation_id()
                .is_some_and(|id| id == conversation_id);
            if is_active {
                return handles.active_session.upgrade(ctx);
            }
        }
        None
    }

    /// Returns the controller for a conversation if it's currently active
    /// (i.e., has an expanded agent view).
    pub fn get_controller_for_conversation(
        &self,
        conversation_id: AIConversationId,
        ctx: &AppContext,
    ) -> Option<ModelHandle<AgentViewController>> {
        for handles in self.agent_view_handles.values() {
            if let Some(controller) = handles.controller.upgrade(ctx) {
                let is_active = controller
                    .as_ref(ctx)
                    .agent_view_state()
                    .active_conversation_id()
                    .is_some_and(|id| id == conversation_id);
                if is_active {
                    return Some(controller);
                }
            }
        }
        None
    }

    /// Returns the last opened time for a conversation, used for sorting active conversations.
    pub fn get_last_opened_time(&self, id: &ConversationOrTaskId) -> Option<DateTime<Utc>> {
        self.last_opened_times.get(id).copied()
    }

    /// Returns the terminal view ID that has an active ambient session with the given task ID.
    pub fn get_terminal_view_id_for_ambient_task(
        &self,
        task_id: AmbientAgentTaskId,
    ) -> Option<EntityId> {
        self.ambient_sessions
            .iter()
            .find_map(|(view_id, id)| (*id == task_id).then_some(*view_id))
    }

    /// Returns the terminal view ID that has an active conversation with the given ID.
    pub fn get_terminal_view_id_for_conversation(
        &self,
        conversation_id: AIConversationId,
        ctx: &AppContext,
    ) -> Option<EntityId> {
        for (terminal_view_id, handles) in &self.agent_view_handles {
            let Some(controller) = handles.controller.upgrade(ctx) else {
                continue;
            };
            let is_active = controller
                .as_ref(ctx)
                .agent_view_state()
                .active_conversation_id()
                .is_some_and(|id| id == conversation_id);
            if is_active {
                return Some(*terminal_view_id);
            }
        }

        None
    }

    pub fn get_terminal_view_id_for_entry(
        &self,
        entry: &AgentConversationEntry,
        ctx: &AppContext,
    ) -> Option<EntityId> {
        if let Some(task_id) = entry.identity.ambient_agent_task_id {
            if let Some(terminal_view_id) = self.get_terminal_view_id_for_ambient_task(task_id) {
                return Some(terminal_view_id);
            }
        }

        if let Some(conversation_id) = entry.identity.local_conversation_id {
            return self.get_terminal_view_id_for_conversation(conversation_id, ctx);
        }

        None
    }
    /// Get all currently active conversation IDs.
    /// A conversation is active if it is open and a query has been sent since it was last opened.
    /// New (empty) conversations and ambient sessions are always considered active when open.
    pub fn get_all_active_conversation_ids(
        &self,
        ctx: &AppContext,
    ) -> HashSet<ConversationOrTaskId> {
        let history_model = BlocklistAIHistoryModel::as_ref(ctx);
        let mut ids = HashSet::new();

        for handles in self.agent_view_handles.values() {
            if let Some(controller) = handles.controller.upgrade(ctx) {
                let state = controller.as_ref(ctx).agent_view_state();
                if let Some(conversation_id) = state.active_conversation_id() {
                    let Some(conversation) = history_model.conversation(&conversation_id) else {
                        continue;
                    };
                    if !conversation.is_entirely_passive()
                        && state.was_conversation_modified_since_opening(history_model)
                    {
                        ids.insert(ConversationOrTaskId::ConversationId(conversation_id));
                    }
                }
            }
        }

        // Ambient sessions are always considered active when open.
        for task_id in self.ambient_sessions.values() {
            ids.insert(ConversationOrTaskId::TaskId(*task_id));
        }

        ids
    }

    /// Get all currently open conversation IDs.
    /// A conversation is considered open if it is in an expanded agent view.
    pub fn get_all_open_conversation_ids(&self, ctx: &AppContext) -> HashSet<ConversationOrTaskId> {
        let mut ids = HashSet::new();

        // Collect from interactive agent views (expanded).
        for handles in self.agent_view_handles.values() {
            if let Some(controller) = handles.controller.upgrade(ctx) {
                if let Some(conversation_id) = controller
                    .as_ref(ctx)
                    .agent_view_state()
                    .active_conversation_id()
                {
                    ids.insert(ConversationOrTaskId::ConversationId(conversation_id));
                }
            }
        }

        // Collect from ambient sessions (open in tabs)
        for task_id in self.ambient_sessions.values() {
            ids.insert(ConversationOrTaskId::TaskId(*task_id));
        }

        ids
    }
}

#[cfg(test)]
#[path = "active_agent_views_model_tests.rs"]
mod tests;
