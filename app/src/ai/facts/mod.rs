pub use cloud_object_models::{AIFact, AIMemory, CloudAIFact, CloudAIFactModel};
use warp_core::ui::appearance::Appearance;

use crate::cloud_object::model::generic_string_model::StringModel;
use crate::cloud_object::model::json_model::JsonModel;
use crate::cloud_object::{
    GenericStringObjectFormat, GenericStringObjectUniqueKey, JsonObjectType, Revision,
};
use crate::drive::CloudObjectTypeAndId;
use crate::drive::items::WarpDriveItem;
use crate::drive::items::ai_fact::WarpDriveAIFact;
use crate::server::ids::SyncId;
use crate::server::sync_queue::QueueItem;

pub mod manager;
pub mod view;
pub use manager::AIFactManager;
pub use view::{AIFactView, AIFactViewEvent};

impl StringModel for AIFact {
    type CloudObjectType = CloudAIFact;

    fn model_type_name(&self) -> &'static str {
        "Rule"
    }

    fn should_enforce_revisions() -> bool {
        true
    }

    fn model_format() -> GenericStringObjectFormat {
        GenericStringObjectFormat::Json(JsonObjectType::AIFact)
    }

    fn should_show_activity_toasts() -> bool {
        true
    }

    fn warn_if_unsaved_at_quit() -> bool {
        true
    }

    fn display_name(&self) -> String {
        match self {
            AIFact::Memory(memory) => memory.content.clone(),
        }
    }

    fn update_object_queue_item(
        &self,
        revision_ts: Option<Revision>,
        object: &Self::CloudObjectType,
    ) -> QueueItem {
        QueueItem::UpdateAIFact {
            model: object.model().clone().into(),
            id: object.id,
            revision: revision_ts.or_else(|| object.metadata.revision.clone()),
        }
    }

    fn uniqueness_key(&self) -> Option<GenericStringObjectUniqueKey> {
        None
    }

    fn renders_in_warp_drive(&self) -> bool {
        false
    }

    fn to_warp_drive_item(
        &self,
        id: SyncId,
        _appearance: &Appearance,
        ai_fact: &CloudAIFact,
    ) -> Option<Box<dyn WarpDriveItem>> {
        Some(Box::new(WarpDriveAIFact::new(
            CloudObjectTypeAndId::GenericStringObject {
                object_type: GenericStringObjectFormat::Json(JsonObjectType::AIFact),
                id,
            },
            ai_fact.clone(),
        )))
    }
}

impl JsonModel for AIFact {
    fn json_object_type() -> JsonObjectType {
        JsonObjectType::AIFact
    }
}
