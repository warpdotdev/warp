mod agent_management_model;
pub(crate) mod agent_type_selector;
pub(crate) mod details_action_buttons;
pub(crate) mod notifications;

pub(crate) mod cloud_setup_guide_view;
pub(crate) mod telemetry;
pub(crate) mod view;

pub(crate) use agent_management_model::{AgentManagementEvent, AgentNotificationsModel};

pub fn init(app: &mut warpui::AppContext) {
    view::init(app);
    agent_type_selector::init(app);
    notifications::view::NotificationMailboxView::init(app);
}
