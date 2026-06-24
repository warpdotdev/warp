use warpui::{Entity, EntityId, ModelContext, SingletonEntity};

use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent_management::notifications::{NotificationId, NotificationItems};

pub struct AgentNotificationsModel {
    notifications: NotificationItems,
}

impl Entity for AgentNotificationsModel {
    type Event = AgentManagementEvent;
}

impl SingletonEntity for AgentNotificationsModel {}

impl AgentNotificationsModel {
    pub(crate) fn new(_ctx: &mut ModelContext<Self>) -> Self {
        Self {
            notifications: NotificationItems::default(),
        }
    }

    pub(crate) fn notifications(&self) -> &NotificationItems {
        &self.notifications
    }

    pub(crate) fn mark_item_read(&mut self, _id: NotificationId, _ctx: &mut ModelContext<Self>) {}

    pub(crate) fn mark_all_items_read(&mut self, _ctx: &mut ModelContext<Self>) {}

    pub(crate) fn mark_items_from_terminal_view_read(
        &mut self,
        _terminal_view_id: EntityId,
        _ctx: &mut ModelContext<Self>,
    ) {
    }
}

#[derive(Clone, Debug)]
pub enum AgentManagementEvent {
    ConversationNeedsAttention {
        window_id: warpui::WindowId,
        tab_index: usize,
        terminal_view_id: EntityId,
        conversation_id: AIConversationId,
    },
    NotificationAdded {
        id: NotificationId,
    },
    NotificationUpdated,
    AllNotificationsMarkedRead,
}
