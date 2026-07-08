use std::collections::HashMap;

use chrono::Utc;
use cloud_objects::cloud_object::{
    GenericCloudObject, GenericServerObject, GenericStringModel, JsonObjectType,
};
use cloud_objects::ids::GenericStringObjectId;
use handlebars::get_arguments;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use warp_core::report_error;

use crate::{JsonModel, JsonSerializer};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JSONMCPServer {
    #[serde(flatten)]
    pub transport_type: JSONTransportType,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum JSONTransportType {
    CLIServer {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env: HashMap<String, String>,
        #[serde(default)]
        working_directory: Option<String>,
    },
    SSEServer {
        #[serde(alias = "serverUrl")]
        url: String,
        #[serde(default)]
        headers: HashMap<String, String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MCPServer {
    pub transport_type: TransportType,
    pub name: String,
    #[serde(default)]
    pub uuid: uuid::Uuid,
}

#[derive(Debug, Clone, Copy)]
pub enum MCPServerState {
    NotRunning,
    Starting,
    Authenticating,
    Running,
    ShuttingDown,
    FailedToStart,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransportType {
    CLIServer(CLIServer),
    ServerSentEvents(ServerSentEvents),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CLIServer {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    pub cwd_parameter: Option<String>,
    /// Static env vars added via editor inputs.
    pub static_env_vars: Vec<StaticEnvVar>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StaticEnvVar {
    pub name: String,
    /// To avoid leaking environment variables, we ensure that values are not
    /// serialized before being sent to our servers
    #[serde(skip_serializing, default)]
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StaticHeader {
    pub name: String,
    /// To avoid leaking header values (which may contain secrets), we ensure that values are not
    /// serialized before being sent to our servers
    #[serde(skip_serializing, default)]
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerSentEvents {
    pub url: String,
    /// Static headers added via editor inputs.
    #[serde(default)]
    pub headers: Vec<StaticHeader>,
}

impl JsonModel for MCPServer {
    fn json_object_type() -> JsonObjectType {
        JsonObjectType::MCPServer
    }
}

pub type CloudMCPServer = GenericCloudObject<GenericStringObjectId, CloudMCPServerModel>;
pub type CloudMCPServerModel = GenericStringModel<MCPServer, JsonSerializer>;
pub type ServerMCPServer = GenericServerObject<GenericStringObjectId, CloudMCPServerModel>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, Hash)]
pub struct JsonTemplate {
    pub json: String,
    pub variables: Vec<TemplateVariable>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub struct TemplateVariable {
    pub key: String,
    /// When present, the variable should be filled via a dropdown of these values
    /// instead of a freetext input.
    #[serde(default)]
    pub allowed_values: Option<Vec<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GalleryData {
    pub gallery_item_id: Uuid,
    pub version: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TemplatableMCPServer {
    pub uuid: uuid::Uuid,
    pub name: String,
    pub description: Option<String>,
    pub template: JsonTemplate,
    #[serde(default)]
    pub version: i64, // This will default to 0 if stored objects have no version
    pub gallery_data: Option<GalleryData>,
}

#[derive(Debug)]
pub enum FromStoredJsonError {
    NoServersFound,
    TooManyServersFound,
    ParseError(serde_json::Error),
}

impl TemplatableMCPServer {
    /// Looks for MCP servers under known wrapper keys (`mcpServers`, `servers`,
    /// `mcp.servers`, `mcp_servers`). Returns `None` if no known key is found.
    fn find_servers_under_known_keys(
        config: &serde_json::Value,
    ) -> Option<HashMap<String, serde_json::Value>> {
        const POINTERS: [&str; 4] = ["/mcp/servers", "/servers", "/mcpServers", "/mcp_servers"];
        for pointer in POINTERS {
            if let Some(value) = config.pointer(pointer)
                && let Ok(servers) =
                    serde_json::from_value::<HashMap<String, serde_json::Value>>(value.clone())
            {
                return Some(servers);
            }
        }
        None
    }

    /// Permissively parses MCP servers from JSON.
    ///
    /// Accepts servers under known wrapper keys (VSCode, Claude Desktop, etc.)
    /// and also falls back to treating the entire object as a bare server map.
    /// This is appropriate for user-pasted input.
    pub fn find_template_map(
        config: serde_json::Value,
    ) -> serde_json::Result<HashMap<String, serde_json::Value>> {
        if let Some(servers) = Self::find_servers_under_known_keys(&config) {
            return Ok(servers);
        }
        // Fallback: treat the entire object as a bare map of servers.
        serde_json::from_value::<HashMap<String, serde_json::Value>>(config)
    }
    /// Like [`find_template_map`], but without the bare-object fallback.
    ///
    /// Returns servers only when found under a known wrapper key. This prevents
    /// misinterpreting unrelated JSON files (e.g. Claude Code's `~/.claude.json`
    /// settings) as MCP config.
    pub fn find_template_map_strict(
        config: &serde_json::Value,
    ) -> HashMap<String, serde_json::Value> {
        Self::find_servers_under_known_keys(config).unwrap_or_default()
    }

    pub fn to_user_json(&self) -> String {
        let value: serde_json::Value = serde_json::from_str(&self.template.json)
            // All templates should be valid JSON - this should never fail
            // Ones that are not should not have been saved in the first place
            .unwrap_or_else(|err| {
                report_error!(
                    anyhow::Error::new(err).context("Could not parse MCP server template to json")
                );
                Default::default()
            });
        serde_json::to_string_pretty(&value)
            // serde_json::to_string_pretty should never fail on this value since we just parsed it as valid json
            .unwrap_or_else(|err| {
                report_error!(
                    anyhow::Error::new(err).context("Could not serialize MCP server to user json")
                );
                Default::default()
            })
    }

    // Uses from_user_json to parse the json and then returns the first TemplatableMCPServer
    // This is meant to be used for stored json from the database, which should only contain
    // a single server and already checked for json validity
    pub fn from_stored_json(
        json: &str,
        uuid: uuid::Uuid,
    ) -> Result<TemplatableMCPServer, FromStoredJsonError> {
        let templates = Self::from_user_json(json);
        match templates {
            Ok(templates) => {
                if templates.is_empty() {
                    // This should never happen for stored json from the database
                    report_error!(
                        "No templatable MCP servers found in stored json",
                        extra: { "uuid" => %uuid }
                    );
                    Err(FromStoredJsonError::NoServersFound)
                } else if templates.len() > 1 {
                    Err(FromStoredJsonError::TooManyServersFound)
                } else {
                    // templates should always contain exactly one server for stored json from the database
                    let mut templatable_mcp_server = templates[0].clone();
                    templatable_mcp_server.uuid = uuid;
                    Ok(templatable_mcp_server)
                }
            }
            Err(err) => Err(FromStoredJsonError::ParseError(err)),
        }
    }

    pub fn from_user_json(json: &str) -> serde_json::Result<Vec<TemplatableMCPServer>> {
        // Some docs don't show curly braces around the json object, so add them if necessary.
        let json = json.trim();
        let json = if json.starts_with("{") {
            json.to_owned()
        } else {
            format!("{{{json}}}")
        };

        let config: serde_json::Value = serde_json::from_str(&json)?;
        let template_jsons = Self::find_template_map(config)?;
        Ok(template_jsons
            .iter()
            .map(|(name, json)| {
                // Each template_json is the nested config for a single MCP server
                // We need to re-wrap it in a top level object so that we can
                // reuse from_user_json to read it later
                let normalized_map =
                    serde_json::Map::from_iter(vec![(name.to_owned(), json.clone())]);
                let normalized_json = serde_json::Value::Object(normalized_map).to_string();

                let description: Option<String> = json
                    .get("description")
                    .and_then(|value| value.as_str().map(|s| s.to_owned()));
                let arguments = get_arguments(&normalized_json);
                let variables = arguments
                    .iter()
                    .map(|argument| TemplateVariable {
                        key: argument.to_owned(),
                        allowed_values: None,
                    })
                    .collect::<Vec<TemplateVariable>>();

                TemplatableMCPServer {
                    uuid: uuid::Uuid::new_v4(),
                    name: name.to_owned(),
                    description,
                    template: JsonTemplate {
                        json: normalized_json,
                        variables,
                    },
                    version: Utc::now().timestamp(),
                    gallery_data: None,
                }
            })
            .collect())
    }
}

impl JsonModel for TemplatableMCPServer {
    fn json_object_type() -> JsonObjectType {
        JsonObjectType::TemplatableMCPServer
    }
}

pub type CloudTemplatableMCPServer =
    GenericCloudObject<GenericStringObjectId, CloudTemplatableMCPServerModel>;
pub type CloudTemplatableMCPServerModel = GenericStringModel<TemplatableMCPServer, JsonSerializer>;
pub type ServerTemplatableMCPServer =
    GenericServerObject<GenericStringObjectId, CloudTemplatableMCPServerModel>;

#[cfg(test)]
#[path = "mcp_tests.rs"]
mod tests;
