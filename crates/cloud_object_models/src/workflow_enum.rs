use cloud_objects::{
    cloud_object::{GenericCloudObject, GenericServerObject, GenericStringModel, JsonObjectType},
    ids::GenericStringObjectId,
};
use serde::{Deserialize, Serialize};

use crate::{JsonModel, JsonSerializer};

/// Data model for a workflow enum.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq, Hash, PartialOrd)]
pub struct WorkflowEnum {
    pub name: String,
    pub is_shared: bool,
    pub variants: EnumVariants,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq, Hash, PartialOrd)]
pub enum EnumVariants {
    Static(Vec<String>),
    Dynamic(String),
}

impl JsonModel for WorkflowEnum {
    fn json_object_type() -> JsonObjectType {
        JsonObjectType::WorkflowEnum
    }
}

pub type CloudWorkflowEnum = GenericCloudObject<GenericStringObjectId, CloudWorkflowEnumModel>;
pub type CloudWorkflowEnumModel = GenericStringModel<WorkflowEnum, JsonSerializer>;
pub type ServerWorkflowEnum = GenericServerObject<GenericStringObjectId, CloudWorkflowEnumModel>;
