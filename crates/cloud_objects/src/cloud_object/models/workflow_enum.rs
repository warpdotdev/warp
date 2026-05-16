use serde::{Deserialize, Serialize};

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
