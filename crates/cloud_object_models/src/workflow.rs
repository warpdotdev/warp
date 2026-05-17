use cloud_objects::{
    cloud_object::{GenericCloudObject, GenericServerObject, ObjectType, ServerObjectModel},
    ids::{GenericStringObjectId, ServerId, SyncId},
};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

/// Workflow model used by Warp and warp-internal.
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq, Hash)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum Workflow {
    AgentMode {
        name: String,
        /// The query to be inserted in the terminal input.
        query: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        #[serde(default)]
        arguments: Vec<Argument>,
    },
    #[serde(untagged)]
    Command {
        name: String,
        command: String,
        #[serde(default)]
        tags: Vec<String>,
        description: Option<String>,
        #[serde(default)]
        arguments: Vec<Argument>,
        source_url: Option<String>,
        author: Option<String>,
        author_url: Option<String>,
        #[serde(default)]
        shells: Vec<warp_workflows::Shell>,
        #[serde(default)]
        environment_variables: Option<SyncId>,
    },
}

impl Workflow {
    pub fn name(&self) -> &str {
        match self {
            Self::AgentMode { name, .. } => name.as_str(),
            Self::Command { name, .. } => name.as_str(),
        }
    }

    pub fn content(&self) -> &str {
        match self {
            Self::AgentMode { query, .. } => query,
            Self::Command { command, .. } => command,
        }
    }

    pub fn prompt(&self) -> Option<&str> {
        if let Self::AgentMode { query, .. } = self {
            Some(query.as_str())
        } else {
            None
        }
    }

    pub fn command(&self) -> Option<&str> {
        if let Self::Command { command, .. } = self {
            Some(command.as_str())
        } else {
            None
        }
    }

    pub fn description(&self) -> Option<&String> {
        match self {
            Self::AgentMode { description, .. } => description.as_ref(),
            Self::Command { description, .. } => description.as_ref(),
        }
    }

    pub fn arguments(&self) -> &Vec<Argument> {
        match self {
            Self::AgentMode { arguments, .. } => arguments,
            Self::Command { arguments, .. } => arguments,
        }
    }

    pub fn tags(&self) -> Option<&Vec<String>> {
        match self {
            Self::Command { tags, .. } => Some(tags),
            Self::AgentMode { .. } => None,
        }
    }

    pub fn source_url(&self) -> Option<&String> {
        match self {
            Self::Command { source_url, .. } => source_url.as_ref(),
            Self::AgentMode { .. } => None,
        }
    }

    pub fn author_name(&self) -> Option<&String> {
        match self {
            Self::Command { author, .. } => author.as_ref(),
            Self::AgentMode { .. } => None,
        }
    }

    pub fn shells(&self) -> Option<&Vec<warp_workflows::Shell>> {
        match self {
            Self::Command { shells, .. } => Some(shells),
            Self::AgentMode { .. } => None,
        }
    }

    pub fn is_command_workflow(&self) -> bool {
        matches!(self, Self::Command { .. })
    }

    pub fn is_agent_mode_workflow(&self) -> bool {
        matches!(self, Self::AgentMode { .. })
    }

    pub fn name_starts_with_char_ignore_case(&self, c: char) -> bool {
        self.name()
            .chars()
            .next()
            .is_some_and(|first| first.eq_ignore_ascii_case(&c))
    }

    pub fn get_enum_ids(&self) -> Vec<SyncId> {
        self.arguments()
            .iter()
            .filter_map(|arg| match arg.arg_type {
                ArgumentType::Enum { enum_id } => Some(enum_id),
                ArgumentType::Text => None,
            })
            .collect()
    }

    pub fn get_server_enum_ids(&self) -> Vec<GenericStringObjectId> {
        self.arguments()
            .iter()
            .filter_map(|arg| match arg.arg_type {
                ArgumentType::Enum { enum_id } => enum_id.into_server(),
                ArgumentType::Text => None,
            })
            .map(Into::into)
            .collect()
    }

    pub fn default_env_vars(&self) -> Option<SyncId> {
        match self {
            Workflow::Command {
                environment_variables,
                ..
            } => *environment_variables,
            Workflow::AgentMode { .. } => None,
        }
    }

    pub fn replace_object_id(&mut self, old_id: SyncId, new_id: SyncId) -> bool {
        let mut changed = false;
        let arguments = match self {
            Self::Command { arguments, .. } => arguments,
            Self::AgentMode { arguments, .. } => arguments,
        };
        for arg in arguments.iter_mut() {
            match &mut arg.arg_type {
                ArgumentType::Enum { enum_id } if *enum_id == old_id => {
                    *enum_id = new_id;
                    changed = true;
                }
                ArgumentType::Enum { .. } | ArgumentType::Text => {}
            }
        }
        if let Self::Command {
            environment_variables,
            ..
        } = self
        {
            if *environment_variables == Some(old_id) {
                *environment_variables = Some(new_id);
                changed = true;
            }
        }
        changed
    }

    pub fn new(name: impl Into<String>, command: impl Into<String>) -> Self {
        Workflow::Command {
            name: name.into(),
            command: command.into(),
            tags: Vec::new(),
            arguments: Vec::new(),
            description: None,
            source_url: None,
            author: None,
            author_url: None,
            shells: Vec::new(),
            environment_variables: None,
        }
    }

    pub fn with_arguments(mut self, new_arguments: Vec<Argument>) -> Self {
        match self {
            Workflow::AgentMode {
                ref mut arguments, ..
            }
            | Workflow::Command {
                ref mut arguments, ..
            } => {
                *arguments = new_arguments;
            }
        }
        self
    }

    pub fn with_description(mut self, new_description: String) -> Self {
        match self {
            Workflow::AgentMode {
                ref mut description,
                ..
            }
            | Workflow::Command {
                ref mut description,
                ..
            } => {
                *description = Some(new_description);
            }
        }
        self
    }

    pub fn set_name(&mut self, new_name: &str) {
        match self {
            Workflow::AgentMode { name, .. } | Workflow::Command { name, .. } => {
                new_name.clone_into(name)
            }
        }
    }
}

impl From<warp_workflows::Workflow> for Workflow {
    fn from(workflow: warp_workflows::Workflow) -> Self {
        Workflow::Command {
            name: workflow.name,
            command: workflow.command,
            description: workflow.description,
            arguments: workflow.arguments.into_iter().map(Argument::from).collect(),
            tags: workflow.tags,
            source_url: workflow.source_url,
            author: workflow.author,
            author_url: workflow.author_url,
            shells: workflow.shells,
            environment_variables: None,
        }
    }
}

/// Argument model used by Warp and warp-internal.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq, Hash, Default)]
pub struct Argument {
    pub name: String,
    #[serde(flatten, deserialize_with = "deserialize_arg_type")]
    pub arg_type: ArgumentType,
    pub description: Option<String>,
    pub default_value: Option<String>,
}

impl From<warp_workflows::Argument> for Argument {
    fn from(arg: warp_workflows::Argument) -> Self {
        Argument {
            name: arg.name,
            arg_type: ArgumentType::Text,
            description: arg.description,
            default_value: arg.default_value,
        }
    }
}

impl Argument {
    pub fn new(name: impl Into<String>, arg_type: ArgumentType) -> Self {
        Argument {
            arg_type,
            name: name.into(),
            description: None,
            default_value: None,
        }
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn with_default(mut self, default: impl Into<String>) -> Self {
        self.default_value = Some(default.into());
        self
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn description(&self) -> &Option<String> {
        &self.description
    }

    pub fn arg_type(&self) -> &ArgumentType {
        &self.arg_type
    }

    pub fn default_value(&self) -> &Option<String> {
        &self.default_value
    }
}

/// The type of a workflow argument.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq, Hash)]
#[serde(tag = "arg_type")]
#[derive(Default)]
pub enum ArgumentType {
    #[default]
    Text,
    Enum {
        /// The ID of the associated workflow enum generic string object.
        enum_id: SyncId,
    },
}

fn deserialize_arg_type<'de, D>(deserializer: D) -> Result<ArgumentType, D::Error>
where
    D: Deserializer<'de>,
{
    let value: Value = Deserialize::deserialize(deserializer)?;

    let arg_type = match value.get("arg_type").and_then(|value| value.as_str()) {
        Some("Text") => ArgumentType::Text,
        Some("Enum") => {
            let enum_id = value
                .get("enum_id")
                .ok_or(serde::de::Error::missing_field("enum_id"))?;
            let deserialized_id = SyncId::deserialize(enum_id)
                .map_err(|_| serde::de::Error::custom("Unable to parse enum_id"))?;
            ArgumentType::Enum {
                enum_id: deserialized_id,
            }
        }
        _ => ArgumentType::default(),
    };

    Ok(arg_type)
}

#[derive(Clone, Debug, PartialEq)]
pub struct CloudWorkflowModel {
    pub data: Workflow,
}

impl CloudWorkflowModel {
    pub fn new(workflow: Workflow) -> Self {
        Self { data: workflow }
    }
}

impl ServerObjectModel for CloudWorkflowModel {
    fn object_type(&self) -> ObjectType {
        ObjectType::Workflow
    }
}

#[derive(Clone, Debug, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct WorkflowId(ServerId);
cloud_objects::server_id_traits! { WorkflowId, "Workflow" }

pub type CloudWorkflow = GenericCloudObject<WorkflowId, CloudWorkflowModel>;
pub type ServerWorkflow = GenericServerObject<WorkflowId, CloudWorkflowModel>;

impl From<CloudWorkflow> for Workflow {
    fn from(cloud_workflow: CloudWorkflow) -> Self {
        cloud_workflow.model().data.clone()
    }
}

impl From<&CloudWorkflow> for Workflow {
    fn from(cloud_workflow: &CloudWorkflow) -> Self {
        cloud_workflow.model().data.to_owned()
    }
}
