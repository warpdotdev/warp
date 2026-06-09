pub use cloud_object_models::{
    CloudWorkflowEnum, CloudWorkflowEnumModel, EnumVariants, WorkflowEnum,
};

use crate::cloud_object::model::generic_string_model::StringModel;
use crate::cloud_object::model::json_model::JsonModel;
use crate::cloud_object::{
    GenericStringObjectFormat, GenericStringObjectUniqueKey, JsonObjectType, Revision,
};
use crate::server::sync_queue::QueueItem;

impl StringModel for WorkflowEnum {
    type CloudObjectType = CloudWorkflowEnum;

    fn model_type_name(&self) -> &'static str {
        "WorkflowEnum"
    }

    fn should_enforce_revisions() -> bool {
        true
    }

    fn model_format() -> GenericStringObjectFormat {
        GenericStringObjectFormat::Json(Self::json_object_type())
    }

    fn should_show_activity_toasts() -> bool {
        false
    }

    fn warn_if_unsaved_at_quit() -> bool {
        true
    }

    fn display_name(&self) -> String {
        self.model_type_name().to_owned()
    }

    fn update_object_queue_item(
        &self,
        revision_ts: Option<Revision>,
        object: &Self::CloudObjectType,
    ) -> QueueItem {
        QueueItem::UpdateWorkflowEnum {
            model: object.model().clone().into(),
            id: object.id,
            revision: revision_ts.or_else(|| object.metadata.revision.clone()),
        }
    }

    fn uniqueness_key(&self) -> Option<GenericStringObjectUniqueKey> {
        None
    }
}

impl JsonModel for WorkflowEnum {
    fn json_object_type() -> JsonObjectType {
        JsonObjectType::WorkflowEnum
    }
}
