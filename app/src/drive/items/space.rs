use warpui::elements::MouseStateHandle;
use warpui::Element;

use super::{WarpDriveItem, WarpDriveItemId};
use crate::appearance::Appearance;
use crate::cloud_object::{CloudObjectMetadata, Space};
use crate::drive::index::DriveIndexAction;
use crate::drive::DriveObjectType;
use crate::themes::theme::Fill;

#[derive(Clone)]
pub struct WarpDriveSpace {
    space: Space,
}

impl WarpDriveSpace {
    #[allow(dead_code)]
    pub fn new(space: Space) -> Self {
        Self { space }
    }
}

impl WarpDriveItem for WarpDriveSpace {
    fn display_name(&self) -> Option<String> {
        None
    }

    fn metadata(&self) -> Option<&CloudObjectMetadata> {
        None
    }

    fn object_type(&self) -> Option<DriveObjectType> {
        None
    }

    fn secondary_icon(&self, _color: Option<Fill>) -> Option<Box<dyn Element>> {
        None
    }

    fn click_action(&self) -> Option<DriveIndexAction> {
        None
    }

    fn preview(&self, _appearance: &Appearance) -> Option<Box<dyn Element>> {
        None
    }

    fn warp_drive_id(&self) -> WarpDriveItemId {
        WarpDriveItemId::Space(self.space)
    }

    fn sync_status_icon(
        &self,
        _sync_queue_is_dequeueing: bool,
        _hover_state: MouseStateHandle,
        _appearance: &Appearance,
    ) -> Option<Box<dyn Element>> {
        None
    }

    fn clone_box(&self) -> Box<dyn WarpDriveItem> {
        Box::new(self.clone())
    }

    fn action_summary(&self, _app: &warpui::AppContext) -> Option<String> {
        None
    }
}
