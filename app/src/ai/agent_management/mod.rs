mod agent_management_model;
pub(crate) mod details_action_buttons;
pub(crate) mod notifications;

pub(crate) mod telemetry;
pub(crate) mod view;

pub(crate) use agent_management_model::{AgentManagementEvent, AgentNotificationsModel};

pub fn init(app: &mut warpui::AppContext) {
    view::init(app);
    notifications::view::NotificationMailboxView::init(app);
}
