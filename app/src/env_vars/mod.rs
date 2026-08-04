pub use cloud_object_models::{
    CloudEnvVarCollection, CloudEnvVarCollectionModel, EnvVar, EnvVarCollection, EnvVarValue,
};
use itertools::Itertools;
use warp_util::path::ShellFamily;

pub mod active_env_var_collection_data;
pub mod env_var_collection_block;
pub mod manager;
pub mod view;

use crate::cloud_object::model::generic_string_model::StringModel;
use crate::cloud_object::model::json_model::JsonModel;
use crate::cloud_object::{
    GenericStringObjectFormat, GenericStringObjectUniqueKey, JsonObjectType, Revision,
};
use crate::drive::items::WarpDriveItem;
use crate::drive::items::env_var_collection::WarpDriveEnvVarCollection;
use crate::server::ids::SyncId;
use crate::server::sync_queue::QueueItem;
use crate::terminal::shell::ShellType;
use crate::{Appearance, CloudObjectTypeAndId};

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

pub trait EnvVarExt {
    fn get_initialization_string(&self, shell_type: ShellType) -> String;
}

impl EnvVarExt for EnvVar {
    fn get_initialization_string(&self, shell_type: ShellType) -> String {
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

pub trait EnvVarCollectionExt {
    fn export_variables_for_shell(&self, shell_type: ShellType) -> String;
}

impl EnvVarCollectionExt for EnvVarCollection {
    fn export_variables_for_shell(&self, shell_type: ShellType) -> String {
        serialize_variables_for_shell(self.key_value_iter(), shell_type)
    }
}

trait EnvVarCollectionKeyValueIter {
    fn key_value_iter(&self) -> impl Iterator<Item = (&str, &EnvVarValue)>;
}

impl EnvVarCollectionKeyValueIter for EnvVarCollection {
    fn key_value_iter(&self) -> impl Iterator<Item = (&str, &EnvVarValue)> {
        self.vars.iter().map(|var| (var.name.as_str(), &var.value))
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
