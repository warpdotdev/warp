use itertools::Itertools;
use serde::{Deserialize, Serialize};
use view::command_dialog::EnvVarSecretCommand;
use warp_util::path::ShellFamily;

pub mod active_env_var_collection_data;
pub mod env_var_collection_block;
pub mod manager;
pub mod view;

use crate::{
    cloud_object::{
        model::{
            generic_string_model::{GenericStringModel, GenericStringObjectId, StringModel},
            json_model::{JsonModel, JsonSerializer},
        },
        GenericCloudObject, GenericStringObjectFormat, GenericStringObjectUniqueKey,
        JsonObjectType, Revision,
    },
    drive::items::{env_var_collection::WarpDriveEnvVarCollection, WarpDriveItem},
    external_secrets::ExternalSecret,
    server::{ids::SyncId, sync_queue::QueueItem},
    terminal::shell::ShellType,
    Appearance, CloudObjectTypeAndId,
};

#[derive(Clone, Debug, PartialEq)]
pub enum EnvVarCollectionType {
    /// Saved env vars, saved using cloud-sync. Today, we only support cloud
    Cloud(Box<CloudEnvVarCollection>),
}

impl EnvVarCollectionType {
    pub fn as_cloud_env_var_collection(&self) -> &CloudEnvVarCollection {
        match self {
            EnvVarCollectionType::Cloud(cloud_env_var) => cloud_env_var,
        }
    }
}

pub type CloudEnvVarCollection =
    GenericCloudObject<GenericStringObjectId, CloudEnvVarCollectionModel>;
pub type CloudEnvVarCollectionModel = GenericStringModel<EnvVarCollection, JsonSerializer>;

/// Defines the data model for a single environment variable
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct EnvVar {
    // Variable name
    pub name: String,
    // Variable value
    pub value: EnvVarValue,
    // Description of variable
    pub description: Option<String>,
}

/// Defines the various forms a value can take
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum EnvVarValue {
    // Represents a string variable, i.e. PORT=4000
    Constant(String),
    // Represents a computed secret, i.e. gcloud print auth token
    Command(EnvVarSecretCommand),
    // Represents a secret from an external secret manager
    Secret(ExternalSecret),
}

impl Default for EnvVarValue {
    fn default() -> Self {
        EnvVarValue::Constant(String::new())
    }
}

impl EnvVar {
    pub fn new(name: String, value: String, description: Option<String>) -> Self {
        Self {
            name,
            value: EnvVarValue::Constant(value),
            description,
        }
    }

    pub fn get_initialization_string(&self, shell_type: ShellType) -> String {
        let shell_family = ShellFamily::from(shell_type);
        let name = shell_family.escape(&self.name);
        let value = get_init_command_for_env_var(&self.value, shell_family);

        match shell_type {
            ShellType::Bash | ShellType::Zsh => {
                format!("export {name}={value};")
            }
            ShellType::Fish => {
                format!("set -x {name} {value};")
            }
            ShellType::PowerShell => {
                format!("$env:{name} = {value};")
            }
        }
    }
}

fn get_init_command_for_env_var(value: &EnvVarValue, shell_family: ShellFamily) -> String {
    match value {
        EnvVarValue::Constant(val) => match shell_family {
            ShellFamily::Posix => shell_family.escape(val).into_owned(),
            ShellFamily::PowerShell => format!("'{}'", val.replace("'", "''")),
        },
        EnvVarValue::Command(cmd) => format!("$({})", cmd.command),
        EnvVarValue::Secret(secret) => {
            format!("$({})", secret.get_secret_extraction_command(shell_family))
        }
    }
}

/// Defines the data model for a cloud synced collection of environment variables.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct EnvVarCollection {
    // Collection title
    pub title: Option<String>,
    // Description of collection
    pub description: Option<String>,
    // Environment variables associated with this collection
    pub vars: Vec<EnvVar>,
}

impl EnvVarCollection {
    #[allow(dead_code)]
    pub fn new(title: Option<String>, description: Option<String>, vars: Vec<EnvVar>) -> Self {
        Self {
            title,
            description,
            vars,
        }
    }

    fn key_value_iter(&self) -> impl Iterator<Item = (&str, &EnvVarValue)> {
        self.vars.iter().map(|var| (var.name.as_str(), &var.value))
    }

    pub fn export_variables(&self, delimiter: &str, shell_family: ShellFamily) -> String {
        serialize_variables_internal(self.key_value_iter(), "", "=", "", delimiter, shell_family)
    }

    pub fn export_variables_for_shell(&self, shell_type: ShellType) -> String {
        serialize_variables_for_shell(self.key_value_iter(), shell_type)
    }
}

impl StringModel for EnvVarCollection {
    type CloudObjectType = CloudEnvVarCollection;

    fn model_type_name(&self) -> &'static str {
        "Environment variables"
    }

    fn should_enforce_revisions() -> bool {
        true
    }

    fn model_format() -> GenericStringObjectFormat {
        GenericStringObjectFormat::Json(Self::json_object_type())
    }

    fn display_name(&self) -> String {
        self.title.clone().unwrap_or_default()
    }

    fn set_display_name(&mut self, name: &str) {
        self.title = if name.is_empty() {
            None
        } else {
            Some(name.to_owned())
        }
    }

    fn update_object_queue_item(
        &self,
        revision_ts: Option<Revision>,
        object: &CloudEnvVarCollection,
    ) -> QueueItem {
        QueueItem::UpdateEnvVarCollection {
            model: object.model().clone().into(),
            id: object.id,
            revision: revision_ts.or_else(|| object.metadata.revision.clone()),
        }
    }

    fn uniqueness_key(&self) -> Option<GenericStringObjectUniqueKey> {
        None
    }

    fn should_show_activity_toasts() -> bool {
        true
    }

    fn warn_if_unsaved_at_quit() -> bool {
        true
    }

    fn renders_in_warp_drive(&self) -> bool {
        true
    }

    fn can_export(&self) -> bool {
        true
    }

    fn supports_linking(&self) -> bool {
        true
    }

    fn to_warp_drive_item(
        &self,
        id: SyncId,
        _appearance: &Appearance,
        env_var_collection: &CloudEnvVarCollection,
    ) -> Option<Box<dyn WarpDriveItem>> {
        Some(Box::new(WarpDriveEnvVarCollection::new(
            CloudObjectTypeAndId::GenericStringObject {
                object_type: GenericStringObjectFormat::Json(JsonObjectType::EnvVarCollection),
                id,
            },
            env_var_collection.clone(),
        )))
    }
}

impl JsonModel for EnvVarCollection {
    fn json_object_type() -> JsonObjectType {
        JsonObjectType::EnvVarCollection
    }
}

impl PartialEq<CloudEnvVarCollection> for CloudEnvVarCollection {
    fn eq(&self, other: &CloudEnvVarCollection) -> bool {
        self.model().string_model == other.model().string_model && self.id == other.id
    }
}

pub fn serialize_variables_for_shell<'s, I: IntoIterator<Item = (&'s str, &'s EnvVarValue)>>(
    pairs: I,
    shell_type: ShellType,
) -> String {
    match shell_type {
        // Warp doesn't support newlines in fish so we can't use env syntax
        ShellType::Fish => {
            serialize_variables_internal(pairs, "set -x ", " ", ";", " ", shell_type.into())
        }
        ShellType::Bash | ShellType::Zsh => {
            serialize_variables_internal(pairs, "", "=", "", " ", shell_type.into())
        }
        ShellType::PowerShell => {
            serialize_variables_internal(pairs, "$env:", " = ", ";", " ", shell_type.into())
        }
    }
}

// Prefix — what's prepended to each variable
// Separator — what separates the variable name from the value
// Postfix — what's appended to the end of each variable
// Delimiter — what separates one variable from the next one
// set -x var_name var_value;   set -x name2 value2;
// ------     -             -   -
//   ^        ^             ^   ^
// prefix  separator   postfix  delimiter (in this case 4 spaces, usually one space or newline)
fn serialize_variables_internal<'s, I: IntoIterator<Item = (&'s str, &'s EnvVarValue)>>(
    pairs: I,
    prefix: &str,
    separator: &str,
    postfix: &str,
    delimiter: &str,
    shell_family: ShellFamily,
) -> String {
    pairs
        .into_iter()
        .map(|(name, value)| {
            format!(
                "{}{}{}{}{}",
                prefix,
                shell_family.escape(name),
                separator,
                get_init_command_for_env_var(value, shell_family),
                postfix
            )
        })
        .collect_vec()
        .join(delimiter)
}
