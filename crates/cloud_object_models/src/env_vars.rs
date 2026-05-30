use cloud_objects::{
    cloud_object::{GenericCloudObject, GenericServerObject, GenericStringModel, JsonObjectType},
    ids::GenericStringObjectId,
};
use serde::{Deserialize, Serialize};
use warp_util::path::ShellFamily;

use crate::{JsonModel, JsonSerializer};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EnvVarSecretCommand {
    pub name: String,
    pub command: String,
}

/// Represents a completed external secret reference.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum ExternalSecret {
    OnePassword(OnePasswordSecret),
    LastPass(LastPassSecret),
}

impl ExternalSecret {
    pub fn get_secret_extraction_command(&self, shell_family: ShellFamily) -> String {
        let prefix = match shell_family {
            ShellFamily::Posix => "\\",
            ShellFamily::PowerShell => "",
        };
        match self {
            ExternalSecret::OnePassword(secret) => {
                format!(
                    "{}op item get --fields credential --reveal {}",
                    prefix, secret.reference
                )
            }
            ExternalSecret::LastPass(secret) => {
                format!("{}lpass show --password {}", prefix, secret.reference)
            }
        }
    }

    pub fn get_display_name(&self) -> String {
        match self {
            ExternalSecret::OnePassword(secret) => secret.name.clone(),
            ExternalSecret::LastPass(secret) => secret.name.clone(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct OnePasswordSecret {
    name: String,
    reference: String,
}

impl OnePasswordSecret {
    pub fn new(name: String, reference: String) -> Self {
        Self { name, reference }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct LastPassSecret {
    name: String,
    reference: String,
}

impl LastPassSecret {
    pub fn new(name: String, reference: String) -> Self {
        Self { name, reference }
    }
}

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
}

pub fn serialize_variables_internal<'s, I: IntoIterator<Item = (&'s str, &'s EnvVarValue)>>(
    pairs: I,
    prefix: &str,
    separator: &str,
    postfix: &str,
    delimiter: &str,
    shell_family: ShellFamily,
) -> String {
    // Prefix — what's prepended to each variable
    // Separator — what separates the variable name from the value
    // Postfix — what's appended to the end of each variable
    // Delimiter — what separates one variable from the next one
    // set -x var_name var_value;   set -x name2 value2;
    // ------     -             -   -
    //   ^        ^             ^   ^
    // prefix  separator   postfix  delimiter (in this case 4 spaces, usually one space or newline)
    pairs
        .into_iter()
        .map(|(name, value)| {
            format!(
                "{}{}{}{}{}",
                prefix,
                shell_family.escape(name),
                separator,
                get_init_command_for_env_var_value(value, shell_family),
                postfix
            )
        })
        .collect::<Vec<_>>()
        .join(delimiter)
}

pub fn get_init_command_for_env_var_value(
    value: &EnvVarValue,
    shell_family: ShellFamily,
) -> String {
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

impl JsonModel for EnvVarCollection {
    fn json_object_type() -> JsonObjectType {
        JsonObjectType::EnvVarCollection
    }
}

pub type CloudEnvVarCollection =
    GenericCloudObject<GenericStringObjectId, CloudEnvVarCollectionModel>;
pub type CloudEnvVarCollectionModel = GenericStringModel<EnvVarCollection, JsonSerializer>;
pub type ServerEnvVarCollection =
    GenericServerObject<GenericStringObjectId, CloudEnvVarCollectionModel>;
