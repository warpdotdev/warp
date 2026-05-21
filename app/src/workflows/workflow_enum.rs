use serde::{Deserialize, Serialize};

use crate::{
    cloud_object::{
        model::{
            generic_string_model::{GenericStringModel, GenericStringObjectId, StringModel},
            json_model::{JsonModel, JsonSerializer},
        },
        GenericCloudObject, GenericStringObjectFormat, GenericStringObjectUniqueKey,
        JsonObjectType, Revision,
    },
    server::sync_queue::QueueItem,
};

/// Data model for a workflow enum, one type of argument that can be inserted into a workflow
/// A workflow enum can either be static or dynamic, as determined by the type of `EnumVariants` it uses
///
/// A `Static` enum contains a finite set of user-specified string values
/// A `Dynamic` enum contains a single shell command, which is executed to determine suggested variants for that argument
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq, Hash, PartialOrd)]
pub struct WorkflowEnum {
    /// Enum name
    pub name: String,
    /// Whether or not the variable should be visible to other workflows
    pub is_shared: bool,
    /// The variants for this enum
    pub variants: EnumVariants,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq, Hash, PartialOrd)]
pub enum EnumVariants {
    Static(Vec<String>), // contains the explicit variants for a static enum
    Dynamic(String),     // contains the value of the shell command associated with the dynamic enum
}

pub type CloudWorkflowEnum = GenericCloudObject<GenericStringObjectId, CloudWorkflowEnumModel>;
pub type CloudWorkflowEnumModel = GenericStringModel<WorkflowEnum, JsonSerializer>;

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
