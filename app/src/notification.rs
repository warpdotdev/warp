/// At the app level, we have structs for representing the UI framework level
/// notification structs, but with the data parsed to our liking.
/// The similar structs at the UI framework layer are lower-level (mostly strings).
use serde::{Deserialize, Serialize};
use warpui::{AppContext, EntityId, WindowId};

use crate::pane_group::PaneId;

/// This data is passed along to the MacOS notification delegate and returned
/// to us when the notification is interacted with.
#[derive(Debug, Deserialize, Serialize)]
pub enum NotificationContext {
    /// For block-specific notifications
    BlockOrigin {
        window_id: WindowId,
        pane_group_id: EntityId,
        pane_id: PaneId,
    },
}

pub fn should_emit_desktop_notification(ctx: &AppContext) -> bool {
    should_emit_desktop_notification_for_app_focus(ctx.windows().app_is_active())
}

fn should_emit_desktop_notification_for_app_focus(app_is_active: bool) -> bool {
    !app_is_active
}

#[cfg(test)]
mod tests {
    use super::should_emit_desktop_notification_for_app_focus;

    #[test]
    fn desktop_notifications_are_suppressed_when_app_is_active() {
        assert!(!should_emit_desktop_notification_for_app_focus(true));
    }

    #[test]
    fn desktop_notifications_are_allowed_when_app_is_inactive() {
        assert!(should_emit_desktop_notification_for_app_focus(false));
    }
}
