use warpui::elements::{Container, Flex, MouseStateHandle, ParentElement};
use warpui::fonts::Weight;
use warpui::ui_components::components::{UiComponent, UiComponentStyles};
use warpui::{AppContext, Element};

use super::{WarpDriveItem, WarpDriveItemId};
use crate::ai::facts::{AIFact, AIMemory, CloudAIFact};
use crate::appearance::Appearance;
use crate::cloud_object::CloudObjectMetadata;
use crate::drive::index::DriveIndexAction;
use crate::drive::{CloudObjectTypeAndId, DriveObjectType};
use crate::themes::theme::Fill;

#[derive(Clone)]
pub struct WarpDriveAIFact {
    id: CloudObjectTypeAndId,
    ai_fact: CloudAIFact,
}

impl WarpDriveAIFact {
    pub fn new(id: CloudObjectTypeAndId, ai_fact: CloudAIFact) -> Self {
        Self { id, ai_fact }
    }
}

impl WarpDriveItem for WarpDriveAIFact {
    fn display_name(&self) -> Option<String> {
        match &self.ai_fact.model().string_model {
            AIFact::Memory(AIMemory { content, name, .. }) => {
                if let Some(name) = name {
                    if !name.is_empty() {
                        Some(name.clone())
                    } else {
                        Some(content.clone())
                    }
                } else {
                    Some(content.clone())
                }
            }
        }
    }
    fn metadata(&self) -> Option<&CloudObjectMetadata> {
        Some(&self.ai_fact.metadata)
    }

    fn object_type(&self) -> Option<DriveObjectType> {
        Some(DriveObjectType::AIFact)
    }

    fn secondary_icon(&self, _color: Option<Fill>) -> Option<Box<dyn Element>> {
        None
    }

    fn click_action(&self) -> Option<DriveIndexAction> {
        Some(DriveIndexAction::OpenAIFactCollection)
    }

    fn preview(&self, appearance: &Appearance) -> Option<Box<dyn Element>> {
        let title_to_render = match &self.ai_fact.model().string_model {
            AIFact::Memory(AIMemory { content, .. }) => content.clone(),
        };

        let title = appearance
            .ui_builder()
            .wrappable_text(title_to_render, true)
            .with_style(UiComponentStyles {
                font_color: Some(
                    appearance
                        .theme()
                        .main_text_color(appearance.theme().background())
                        .into(),
                ),
                font_size: Some(14.),
                font_weight: Some(Weight::Bold),
                ..Default::default()
            })
            .build()
            .finish();

        Some(
            Flex::column()
                .with_child(Container::new(title).finish())
                .finish(),
        )
    }

    fn warp_drive_id(&self) -> WarpDriveItemId {
        WarpDriveItemId::Object(self.id)
    }

    fn sync_status_icon(
        &self,
        sync_queue_is_dequeueing: bool,
        hover_state: MouseStateHandle,
        appearance: &Appearance,
    ) -> Option<Box<dyn Element>> {
        self.ai_fact.metadata.pending_changes_statuses.render_icon(
            sync_queue_is_dequeueing,
            hover_state,
            appearance,
        )
    }

    fn action_summary(&self, _app: &AppContext) -> Option<String> {
        None
    }

    fn clone_box(&self) -> Box<dyn WarpDriveItem> {
        Box::new(self.clone())
    }
}
