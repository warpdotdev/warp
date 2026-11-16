/// At the app level, we have structs for representing the UI framework level
/// notification structs, but with the data parsed to our liking.
/// The similar structs at the UI framework layer are lower-level (mostly strings).
use serde::{Deserialize, Serialize};
use warpui::{EntityId, WindowId};

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
