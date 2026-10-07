use std::collections::HashMap;
use std::fmt;
use std::result::Result;
use std::str::FromStr;

use serde::de::{self, MapAccess, Visitor};
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_yaml::Value;
use strum::VariantNames as _;
use strum_macros::{Display, EnumString, VariantNames};
use warpui::{AppContext, SingletonEntity};

use super::workflow::{Argument, ArgumentType, Workflow};
use super::workflow_enum::{EnumVariants, WorkflowEnum};
use crate::cloud_object::model::persistence::CloudModel;
use crate::server::ids::{ClientId, SyncId};

/// Separate structure for exporting arguments. This new structure holds explicit enum information,
/// unlike the `Argument` struct which just holds the enum_id. It is also flatter than the normal `Argument`
/// struct, making it easier to use serde's built-in serialize and deserialize methods.
#[derive(Serialize, Deserialize, Debug)]
struct ExportArgument {
    pub name: String,
    #[serde(flatten, deserialize_with = "deserialize_arg_type")]
    pub arg_type: ExportArgumentType,
    pub description: Option<String>,
    pub default_value: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Default)]
#[serde(tag = "arg_type")]
enum ExportArgumentType {
    #[default]
    Text,
    Enum {
        enum_name: String,

        // This field is Some() for static enums
        #[serde(skip_serializing_if = "Option::is_none")]
        enum_variants: Option<Vec<String>>,

        // This field is Some() for dynamic enums
        #[serde(skip_serializing_if = "Option::is_none")]
        enum_command: Option<String>,
    },
}

impl ExportArgument {
    /// Create a new ExportArgument given an Argument
    fn new(argument: &Argument, app: &AppContext) -> Self {
        let arg_type = match argument.arg_type {
            ArgumentType::Text => ExportArgumentType::Text,
            ArgumentType::Enum { enum_id } => CloudModel::as_ref(app)
                .get_workflow_enum(&enum_id)
                .map(|workflow_enum| {
                    let model = &workflow_enum.model().string_model;
                    let enum_name = model.name.clone();

                    let mut enum_variants = None;
                    let mut enum_command = None;

                    match &model.variants {
                        EnumVariants::Static(variants) => enum_variants = Some(variants.clone()),
                        EnumVariants::Dynamic(command) => enum_command = Some(command.clone()),
                    };
                    ExportArgumentType::Enum {
                        enum_name,
                        enum_variants,
                        enum_command,
                    }
                })
                .unwrap_or(ExportArgumentType::Text),
        };

        ExportArgument {
            name: argument.name.clone(),
            arg_type,
            description: argument.description.clone(),
            default_value: argument.default_value.clone(),
        }
    }

    /// Convert an ExportArgument to an Argument and create a new WorkflowEnum, if possible
    fn to_argument(argument: ExportArgument) -> (Argument, Option<(ClientId, WorkflowEnum)>) {
        let mut new_enum_info = None;

        let name = argument.name;
        let description = argument.description;
        let default_value = argument.default_value;

        let arg_type = match argument.arg_type {
            ExportArgumentType::Text => ArgumentType::Text,
            ExportArgumentType::Enum {
                enum_name,
                enum_variants,
                enum_command,
            } => {
                let workflow_enum =
                    Self::try_into_workflow_enum(enum_name, enum_variants, enum_command);

                match workflow_enum {
                    Ok(enum_data) => {
                        let client_id = ClientId::default();
                        new_enum_info = Some((client_id, enum_data));
                        ArgumentType::Enum {
                            enum_id: SyncId::ClientId(client_id),
                        }
                    }
                    // If we are missing some enum info, use the default type instead
                    Err(_) => {
                        log::warn!(
                            "Tried to deserialize an enum argument without any static variants or dynamic command provided, defaulting to {:?} argument",
                            ArgumentType::default()
                        );
                        ArgumentType::default()
                    }
                }
            }
        };

        (
            Argument {
                name,
                arg_type,
                description,
                default_value,
            },
            new_enum_info,
        )
    }

    /// Try to create a new workflow enum from parsed data
    fn try_into_workflow_enum(
        enum_name: String,
        enum_variants: Option<Vec<String>>,
        enum_command: Option<String>,
    ) -> Result<WorkflowEnum, anyhow::Error> {
        // Always create unshared enums on import
        let is_shared = false;

        // Try to grab variants or command
        let variants = if let Some(variants) = enum_variants {
            EnumVariants::Static(variants)
        } else if let Some(command) = enum_command {
            EnumVariants::Dynamic(command)
        } else {
            return Err(anyhow::anyhow!("Missing valid enum variants"));
        };

        Ok(WorkflowEnum {
            name: enum_name,
            is_shared,
            variants,
        })
    }
}

/// Custom serialize function used to export a workflow to YAML
pub fn export_serialize<S>(
    workflow: &Workflow,
    serializer: S,
    app: &AppContext,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let mut state = serializer.serialize_struct("Workflow", 9)?;

    let export_args: Vec<ExportArgument> = workflow
        .arguments()
        .iter()
        .map(|arg| ExportArgument::new(arg, app))
        .collect();
    match workflow {
        Workflow::Command {
            name,
            description,
            command,
            tags,
            source_url,
            author,
            author_url,
            shells,
            ..
        } => {
            if let Some(description) = description {
                state.serialize_field("description", description)?;
            }
            state.serialize_field("name", name)?;
            state.serialize_field("command", command)?;
            state.serialize_field("description", description)?;
            state.serialize_field("arguments", &export_args)?;
            state.serialize_field("tags", tags)?;
            state.serialize_field("source_url", source_url)?;
            state.serialize_field("author", author)?;
            state.serialize_field("author_url", author_url)?;
            state.serialize_field("shells", shells)?;
        }
        Workflow::AgentMode {
            name,
            description,
            query,
            ..
        } => {
            state.serialize_field("type", "agent_mode")?;
            state.serialize_field("name", name)?;
            state.serialize_field("query", query)?;
            state.serialize_field("description", description)?;
            state.serialize_field("arguments", &export_args)?;
        }
    }

    state.end()
}

/// Macro for deserializing workflow fields, given an associated variant on Field, a string name, a variable name, an expected type, a
/// and an optional flag, which is true when the field is optional as we deserialize.
macro_rules! extract_fields {
    ($map:expr_2021; $(($field:ident, $name:literal, $var:ident, $type:ty, $optional:expr_2021)),* $(,)?) => {{
        $(let mut $var = None;)*

        while let Some(key) = $map.next_key()? {
            match key {
                $(Field::$field => {
                    if $var.is_some() {
                        return Err(de::Error::duplicate_field($name));
                    }
                    $var = Some($map.next_value()?);
                })*
            }
        }

        $(
            let $var: $type = if $optional {
                $var.unwrap_or_default()
            } else {
                $var.ok_or_else(|| de::Error::missing_field($name))?
            };
        )*

        ($($var),*)
    }};
}

/// Custom deserialize function used to import a workflow from YAML
pub fn export_deserialize<'de, D>(
    deserializer: D,
) -> Result<(Workflow, HashMap<ClientId, WorkflowEnum>), D::Error>
where
    D: Deserializer<'de>,
{
    /// We use `strum` here to derive `from_str` and `VARIANTS`, which allows us to convert between
    /// field variants and strings a lot more easily.
    #[derive(Display, EnumString, VariantNames)]
    #[strum(serialize_all = "snake_case")]
    enum Field {
        Name,
        Command,
        Tags,
        Description,
        Arguments,
        SourceUrl,
        Author,
        AuthorUrl,
        Shells,
        EnvironmentVariables,
    }

    impl<'de> Deserialize<'de> for Field {
        fn deserialize<D>(deserializer: D) -> Result<Field, D::Error>
        where
            D: Deserializer<'de>,
        {
            struct FieldVisitor;

            impl Visitor<'_> for FieldVisitor {
                type Value = Field;

                fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                    formatter.write_str("workflow identifier")
                }

                fn visit_str<E>(self, value: &str) -> Result<Field, E>
                where
                    E: de::Error,
                {
                    match Field::from_str(value) {
                        Ok(field) => Ok(field),
                        Err(_) => Err(de::Error::unknown_field(value, FIELDS)),
                    }
                }
            }

            deserializer.deserialize_identifier(FieldVisitor)
        }
    }

    struct WorkflowVisitor;

    impl<'de> Visitor<'de> for WorkflowVisitor {
        type Value = (Workflow, HashMap<ClientId, WorkflowEnum>);

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("struct Workflow")
        }

        fn visit_map<V>(
            self,
            mut map: V,
        ) -> Result<(Workflow, HashMap<ClientId, WorkflowEnum>), V::Error>
        where
            V: MapAccess<'de>,
        {
            // Use the macro to extract values for each field
            let (
                name,
                command,
                description,
                export_arguments,
                tags,
                source_url,
                author,
                author_url,
                shells,
                environment_variables,
            ) = extract_fields!(
                map; (Name, "name", name, String, false),
                (Command, "command", command, String, false),
                (Description, "description", description, Option<String>, true),
                (Arguments, "arguments", arguments, Vec<ExportArgument>, true),
                (Tags, "tags", tags, Vec<String>, true),
                (SourceUrl, "source_url", source_url, Option<String>, true),
                (Author, "author", author, Option<String>, true),
                (AuthorUrl, "author_url", author_url, Option<String>, true),
                (Shells, "shells", shells, Vec<warp_workflows::Shell>, true),
                (EnvironmentVariables, "environment_variables", environment_variables, Option<SyncId>, true),
            );

            // Convert the ExportArguments to Arguments, and get a list of workflow enums that need to be created
            let (arguments, potential_enums): (
                Vec<Argument>,
                Vec<Option<(ClientId, WorkflowEnum)>>,
            ) = export_arguments
                .into_iter()
                .map(ExportArgument::to_argument)
                .unzip();
            let workflow_enums = potential_enums.into_iter().flatten().collect();

            Ok((
                Workflow::Command {
                    name,
                    command,
                    description,
                    arguments,
                    tags,
                    source_url,
                    author,
                    author_url,
                    shells,
                    environment_variables,
                },
                workflow_enums,
            ))
        }
    }

    const FIELDS: &[&str] = Field::VARIANTS;
    deserializer.deserialize_struct("Workflow", FIELDS, WorkflowVisitor)
}

/// Custom deserialization for argument types, used to both `flatten` the argument type
/// and allow for the specification of `default` behavior.
///
/// We need to specify default behavior to remain compatible with old workflow formats.
///
/// Necessary because serde currently does not support the use of `flatten` with a `default`,
/// related GitHub issue here: https://github.com/serde-rs/serde/issues/1626
fn deserialize_arg_type<'de, D>(deserializer: D) -> Result<ExportArgumentType, D::Error>
where
    D: Deserializer<'de>,
{
    let value: Value = Deserialize::deserialize(deserializer)?;

    let arg_type = match value.get("arg_type").and_then(|value| value.as_str()) {
        Some("Text") => ExportArgumentType::Text,
        Some("Enum") => {
            let enum_name = value
                .get("enum_name")
                .and_then(|s| s.as_str().map(|s| s.to_string()));

            let enum_variants = value
                .get("enum_variants")
                .and_then(|v| v.as_sequence())
                .and_then(|seq| {
                    seq.iter()
                        .map(|s| s.as_str().map(|s| s.to_string()))
                        .collect()
                });
            let enum_command = value
                .get("enum_command")
                .and_then(|v| v.as_str().map(|s| s.to_string()));

            // If we don't have an enum name, default to a text argument
            match enum_name {
                Some(enum_name) => ExportArgumentType::Enum {
                    enum_name,
                    enum_variants,
                    enum_command,
                },
                None => ExportArgumentType::default(),
            }
        }
        _ => ExportArgumentType::default(),
    };

    Ok(arg_type)
}
