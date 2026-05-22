use warpui::elements::MouseStateHandle;
use warpui::{AppContext, Element};

use super::cloud_object_styling::warp_drive_icon_color;
use super::index::{warp_drive_section_header_position_id, DriveIndexAction, DriveIndexSection};
use super::{CloudObjectTypeAndId, DriveObjectType};
use crate::appearance::Appearance;
use crate::cloud_object::{CloudObjectMetadata, Space};
use crate::themes::theme::Fill;
use crate::ui_components::icons::Icon;

pub mod ai_fact;
pub mod ai_fact_collection;
pub mod env_var_collection;
pub mod folder;
pub mod item;
pub mod mcp_server;
pub mod mcp_server_collection;
pub mod notebook;
pub mod space;
pub mod workflow;

pub trait WarpDriveItem {
    /// The display name of the item. If the item is unnamed, this may return `None` - implementations
    /// should prefer this over `Some("")`, as it lets the index view use alternate styling.
    fn display_name(&self) -> Option<String>;
    fn metadata(&self) -> Option<&CloudObjectMetadata>;
    fn object_type(&self) -> Option<DriveObjectType>;
    fn secondary_icon(&self, color: Option<Fill>) -> Option<Box<dyn Element>>; // The optional icon to the right of the name
    fn click_action(&self) -> Option<DriveIndexAction>;
    fn preview(&self, appearance: &Appearance) -> Option<Box<dyn Element>>;
    fn warp_drive_id(&self) -> WarpDriveItemId;
    fn sync_status_icon(
        &self,
        sync_queue_is_dequeueing: bool,
        hover_state: MouseStateHandle,
        appearance: &Appearance,
    ) -> Option<Box<dyn Element>>;

    fn icon(&self, appearance: &Appearance, color: Option<Fill>) -> Option<Box<dyn Element>> {
        let object_type = self.object_type()?;
        let icon_fill = color.unwrap_or(warp_drive_icon_color(appearance, object_type).into());
        Some(Icon::from(object_type).to_warpui_icon(icon_fill).finish())
    }

    /// If implemented, returns a string that summarizes the primary action history. For example, "Run 2 times in the last week"
    fn action_summary(&self, app: &AppContext) -> Option<String>;

    /// Returns Some(true) if this is an open folder, Some(false) if closed folder, None if not a folder
    fn is_folder_open(&self) -> Option<bool> {
        None
    }

    fn clone_box(&self) -> Box<dyn WarpDriveItem>;
}

impl WarpDriveItemId {
    pub fn drive_row_position_id(&self) -> String {
        match self {
            Self::AIFactCollection => "AI_fact_collection".to_string(),
            Self::MCPServerCollection => "MCP_server_collection".to_string(),
            Self::Object(object_id) => object_id.drive_row_position_id(),
            Self::Space(space) => {
                warp_drive_section_header_position_id(&DriveIndexSection::Space(*space))
            }
            Self::Trash => "Trash".to_string(),
        }
    }
}
/// This uniquely identifies an item in Warp Drive index
/// Includes spaces (which CloudObjectTypeAndId does not entail)
#[derive(Debug, Clone, PartialEq, Eq, Copy)]
pub enum WarpDriveItemId {
    AIFactCollection,
    MCPServerCollection,
    Object(CloudObjectTypeAndId),
    Space(Space),
    Trash,
}

impl Clone for Box<dyn WarpDriveItem> {
    fn clone(&self) -> Self {
        self.clone_box()
    }
}
