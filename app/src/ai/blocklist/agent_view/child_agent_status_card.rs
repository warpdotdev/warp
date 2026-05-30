use std::collections::{HashMap, HashSet};

use warpui::elements::{Element, Empty, Flex, MouseStateHandle, ParentElement};
use warpui::platform::Cursor;
use warpui::prelude::Container;
use warpui::ui_components::components::UiComponent;
use warpui::{
    AppContext, Entity, ModelHandle, SingletonEntity, TypedActionView, View, ViewContext,
};

use crate::ai::agent::conversation::{AIConversationId, ConversationStatus, StatusColorStyle};
use crate::ai::blocklist::agent_view::orchestration_conversation_links::conversation_navigation_card_with_icon;
use crate::ai::blocklist::agent_view::{AgentViewController, AgentViewControllerEvent};
use crate::ai::blocklist::BlocklistAIHistoryEvent;
use crate::appearance::Appearance;
use crate::terminal::view::TerminalAction;
use crate::ui_components::buttons::close_button;
use crate::BlocklistAIHistoryModel;

#[derive(Debug, Clone)]
pub enum ChildAgentStatusCardAction {
    Dismiss(AIConversationId),
}

/// Renders a list of child agent statuses above the agent message bar.
///
/// Each row shows a status icon, agent name, and conversation title.
/// Clicking a row reveals the child agent's hidden pane.
/// Cards can be dismissed via an X button and automatically reappear
/// when the child agent starts or restarts (transitions to InProgress).
pub struct ChildAgentStatusCard {
    agent_view_controller: ModelHandle<AgentViewController>,
    mouse_states: HashMap<AIConversationId, MouseStateHandle>,
    dismiss_mouse_states: HashMap<AIConversationId, MouseStateHandle>,
    dismissed: HashSet<AIConversationId>,
    previous_statuses: HashMap<AIConversationId, ConversationStatus>,
}

impl Entity for ChildAgentStatusCard {
    type Event = ();
}

impl ChildAgentStatusCard {
    pub fn new(
        agent_view_controller: ModelHandle<AgentViewController>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        // Subscribe without terminal_view_id filtering so we receive status
        // updates for child conversations (which have a different terminal_view_id).
        let history_model = BlocklistAIHistoryModel::handle(ctx);
        ctx.subscribe_to_model(&history_model, |this, _, event, ctx| match event {
            BlocklistAIHistoryEvent::UpdatedConversationStatus {
                conversation_id, ..
            } => {
                this.on_conversation_status_updated(*conversation_id, ctx);
                this.ensure_mouse_states(ctx);
                ctx.notify();
            }
            BlocklistAIHistoryEvent::AppendedExchange { .. }
            | BlocklistAIHistoryEvent::SetActiveConversation { .. } => {
                this.ensure_mouse_states(ctx);
                ctx.notify();
            }
            BlocklistAIHistoryEvent::RemoveConversation {
                conversation_id, ..
            }
            | BlocklistAIHistoryEvent::DeletedConversation {
                conversation_id, ..
            } => {
                this.remove_state_for_conversation(*conversation_id);
                ctx.notify();
            }
            _ => {}
        });
        ctx.subscribe_to_model(&agent_view_controller, |this, _, event, ctx| {
            // Reset all per-child state when entering a conversation so stale
            // entries from a previous conversation's children don't accumulate.
            if matches!(event, AgentViewControllerEvent::EnteredAgentView { .. }) {
                this.dismissed.clear();
                this.previous_statuses.clear();
                this.mouse_states.clear();
                this.dismiss_mouse_states.clear();
            }
            this.ensure_mouse_states(ctx);
            ctx.notify();
        });

        Self {
            agent_view_controller,
            mouse_states: HashMap::new(),
            dismiss_mouse_states: HashMap::new(),
            dismissed: HashSet::new(),
            previous_statuses: HashMap::new(),
        }
    }

    fn ensure_mouse_states(&mut self, ctx: &AppContext) {
        let agent_view_controller = self.agent_view_controller.as_ref(ctx);
        let Some(active_conversation_id) = agent_view_controller
            .agent_view_state()
            .active_conversation_id()
        else {
            return;
        };
        let history_model = BlocklistAIHistoryModel::as_ref(ctx);
        for child in history_model.child_conversations_of(active_conversation_id) {
            let child_id = child.id();
            self.mouse_states.entry(child_id).or_default();
            self.dismiss_mouse_states.entry(child_id).or_default();
            self.previous_statuses
                .entry(child_id)
                .or_insert_with(|| child.status().clone());
        }
    }

    fn remove_state_for_conversation(&mut self, conversation_id: AIConversationId) {
        self.dismissed.remove(&conversation_id);
        self.previous_statuses.remove(&conversation_id);
        self.mouse_states.remove(&conversation_id);
        self.dismiss_mouse_states.remove(&conversation_id);
    }

    /// Checks whether a child conversation transitioned to `InProgress` from a
    /// non-`InProgress` state, mirroring the Started/Restarted lifecycle event
    /// logic in `OrchestrationEventService::on_conversation_status_updated`.
    /// If so, restores any dismissed card for that conversation.
    fn on_conversation_status_updated(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &AppContext,
    ) {
        let Some(conversation) =
            BlocklistAIHistoryModel::as_ref(ctx).conversation(&conversation_id)
        else {
            return;
        };
        if !conversation.is_child_agent_conversation() {
            return;
        }
        let current_status = conversation.status().clone();
        if should_restore_dismissed_card(
            &current_status,
            self.previous_statuses.get(&conversation_id),
        ) {
            self.dismissed.remove(&conversation_id);
        }
        self.previous_statuses
            .insert(conversation_id, current_status);
    }
}

impl TypedActionView for ChildAgentStatusCard {
    type Action = ChildAgentStatusCardAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            ChildAgentStatusCardAction::Dismiss(conversation_id) => {
                self.dismissed.insert(*conversation_id);
                ctx.notify();
            }
        }
    }
}

impl View for ChildAgentStatusCard {
    fn ui_name() -> &'static str {
        "ChildAgentStatusCard"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let agent_view_controller = self.agent_view_controller.as_ref(app);
        let Some(active_conversation_id) = agent_view_controller
            .agent_view_state()
            .active_conversation_id()
        else {
            return Empty::new().finish();
        };

        let history_model = BlocklistAIHistoryModel::as_ref(app);
        let mut children = history_model.child_conversations_of(active_conversation_id);
        if children.is_empty() {
            return Empty::new().finish();
        }

        // Sort by creation time so rows have a stable visual order.
        children.sort_by_key(|c| c.first_exchange().map(|e| e.start_time));

        let appearance = Appearance::as_ref(app);
        let mut column = Flex::column();

        for child in &children {
            let conversation_id = child.id();
            if self.dismissed.contains(&conversation_id) {
                continue;
            }

            let agent_name = child.agent_name().unwrap_or("Agent").to_string();
            let title = child.title().unwrap_or_else(|| "Untitled".to_string());
            let status_icon = child
                .status()
                .status_icon_and_color(appearance.theme(), StatusColorStyle::Standard);

            let Some(mouse_state) = self.mouse_states.get(&conversation_id).cloned() else {
                log::error!(
                    "Missing mouse state handle for child agent card {:?}",
                    conversation_id
                );
                continue;
            };

            let Some(dismiss_mouse_state) =
                self.dismiss_mouse_states.get(&conversation_id).cloned()
            else {
                log::error!(
                    "Missing dismiss mouse state handle for child agent card {:?}",
                    conversation_id
                );
                continue;
            };

            let dismiss_button = close_button(appearance, dismiss_mouse_state)
                .build()
                .on_click(move |ctx: &mut warpui::EventContext<'_>, _, _| {
                    ctx.dispatch_typed_action(ChildAgentStatusCardAction::Dismiss(conversation_id));
                })
                .with_cursor(Cursor::PointingHand)
                .finish();

            let card = conversation_navigation_card_with_icon(
                Some(status_icon),
                agent_name,
                Some(title),
                move |ctx, _, _| {
                    ctx.dispatch_typed_action(TerminalAction::RevealChildAgent { conversation_id });
                },
                mouse_state,
                true,
                Some(dismiss_button),
                app,
            );
            column.add_child(Container::new(card).with_margin_top(4.).finish());
        }

        column.finish()
    }
}

/// Returns true when a dismissed card should be restored: the conversation
/// transitioned to `InProgress` from a non-`InProgress` state, matching the
/// Started/Restarted lifecycle event semantics.
fn should_restore_dismissed_card(
    current_status: &ConversationStatus,
    previous_status: Option<&ConversationStatus>,
) -> bool {
    let was_in_progress = previous_status.is_some_and(|s| s.is_in_progress());
    current_status.is_in_progress() && !was_in_progress
}
