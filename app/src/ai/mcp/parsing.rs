use std::collections::HashMap;

use handlebars::{get_arguments, render_template};
#[cfg(feature = "local_fs")]
use serde::Deserialize;

use crate::ai::mcp::templatable::{JsonTemplate, TemplatableMCPServer, TemplateVariable};
use crate::ai::mcp::templatable_installation::{
    TemplatableMCPServerInstallation, VariableType, VariableValue,
};
#[cfg(feature = "local_fs")]
use crate::ai::mcp::{JSONMCPServer, JSONTransportType};

/// Normalize MCP JSON input to ensure it has a server name wrapper.
///
/// If the JSON is a single server definition (has `command` or `url` at the top level),
/// wrap it with a generated name. Otherwise, return the JSON as-is.
///
/// Note: When returning the original JSON, we preserve the exact input string.
#[cfg(not(target_family = "wasm"))]
pub(crate) fn normalize_mcp_json(json_str: &str) -> serde_json::Result<String> {
    // Some docs don't show curly braces around the json object, so add them if necessary.
    let json = json_str.trim();
    let json_for_parsing = if json.starts_with('{') {
        json.to_owned()
    } else {
        format!("{{{json}}}")
    };

    let value: serde_json::Value = serde_json::from_str(&json_for_parsing)?;

    // Check if this is a single server definition (has command or url at top level)
    let is_single_server = value.get("command").is_some() || value.get("url").is_some();

    if is_single_server {
        let server_name = uuid::Uuid::new_v4().to_string();
        let mut map = serde_json::Map::new();
        map.insert(server_name, value);
        Ok(serde_json::Value::Object(map).to_string())
    } else {
        Ok(json_str.to_string())
    }
}

/// A single entry under `[mcp_servers.<name>]` in a Codex TOML file.
///
/// Codex servers are either STDIO (discriminated by a required `command` field)
/// or streamable HTTP (discriminated by a required `url` field).
/// See https://developers.openai.com/codex/mcp/ for more details on Codex's MCP configuration spec.
#[cfg(feature = "local_fs")]
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum CodexServerEntry {
    /// A local process server launched over stdin/stdout.
    Stdio {
        /// The command that starts the server. Acts as the discriminant.
        command: String,
        /// Arguments to pass to the server command.
        #[serde(default)]
        args: Vec<String>,
        /// Literal key=value environment variables set on the server process.
        #[serde(default)]
        env: HashMap<String, String>,
        /// Names of environment variables to forward from the calling shell.
        /// Each name `N` is lowered to a `"${N}"` placeholder in the merged `env` map.
        /// Explicit `env` values win over `env_vars` placeholders on collision.
        #[serde(default)]
        env_vars: Vec<String>,
        /// Working directory for the server process.
        /// Mapped to `working_directory` in Warp JSON.
        cwd: Option<String>,
    },
    /// A remote server reached over streamable HTTP.
    Http {
        /// The server URL. Acts as the discriminant.
        url: String,
        /// Name of an environment variable holding a bearer token.
        /// Lowered to `Authorization: "Bearer ${VAR}"` in the headers map.
        bearer_token_env_var: Option<String>,
        /// Static header values sent verbatim (e.g. `X-Client = "codex"`).
        /// Wins over `env_http_headers` on collision.
        #[serde(default)]
        http_headers: HashMap<String, String>,
        /// Map of header names to environment variable names.
        /// Each entry becomes `header = "${var_name}"` in the headers map.
        #[serde(default)]
        env_http_headers: HashMap<String, String>,
    },
}

#[cfg(feature = "local_fs")]
impl From<CodexServerEntry> for JSONTransportType {
    fn from(entry: CodexServerEntry) -> Self {
        match entry {
            CodexServerEntry::Stdio {
                command,
                args,
                env,
                env_vars,
                cwd,
            } => {
                // Build merged env: env_vars placeholders first.
                let mut merged_env: HashMap<String, String> = env_vars
                    .into_iter()
                    .map(|var_name| (var_name.clone(), format!("${{{var_name}}}")))
                    .collect();
                // Explicit env wins over env_vars placeholders on collision.
                for (k, v) in env {
                    merged_env.insert(k, v);
                }

                JSONTransportType::CLIServer {
                    command,
                    args,
                    env: merged_env,
                    working_directory: cwd,
                }
            }
            CodexServerEntry::Http {
                url,
                bearer_token_env_var,
                http_headers,
                env_http_headers,
            } => {
                // Build merged headers.
                // Merge order (later wins): bearer < env_http_headers < http_headers.
                let mut merged_headers: HashMap<String, String> = HashMap::new();

                if let Some(var_name) = bearer_token_env_var {
                    merged_headers.insert(
                        "Authorization".to_owned(),
                        format!("Bearer ${{{var_name}}}"),
                    );
                }
                for (header, var_name) in env_http_headers {
                    merged_headers.insert(header, format!("${{{var_name}}}"));
                }
                for (header, value) in http_headers {
                    merged_headers.insert(header, value);
                }

                JSONTransportType::SSEServer {
                    url,
                    headers: merged_headers,
                }
            }
        }
    }
}

/// Normalizes the contents of a Codex `config.toml` into a JSON string
/// compatible with `ParsedTemplatableMCPServerResult::from_user_json`.
#[cfg(feature = "local_fs")]
pub(crate) fn normalize_codex_toml_to_json(file_contents: &str) -> Result<String, anyhow::Error> {
    // Parse into a raw Value first so we can handle per-entry deserialization failures
    // gracefully. Using HashMap<String, CodexServerEntry> directly would cause the entire
    // parse to fail if any single entry matches neither Stdio nor Http.
    let raw: toml::Value = toml::from_str(file_contents)
        .map_err(|e| anyhow::anyhow!("Failed to parse Codex TOML: {e}"))?;

    let out_servers: HashMap<String, JSONMCPServer> = raw
        .get("mcp_servers")
        .and_then(|v| v.as_table())
        .map(|table| {
            table
                .iter()
                .filter_map(|(name, val)| {
                    val.clone()
                        .try_into::<CodexServerEntry>()
                        .ok()
                        .map(|entry| {
                            (
                                name.clone(),
                                JSONMCPServer {
                                    transport_type: JSONTransportType::from(entry),
                                },
                            )
                        })
                })
                .collect()
        })
        .unwrap_or_default();

    let wrapped = serde_json::json!({ "mcp_servers": out_servers });
    serde_json::to_string(&wrapped)
        .map_err(|e| anyhow::anyhow!("Failed to serialize normalized Codex TOML as JSON: {e}"))
}

#[derive(Debug, Clone)]
pub struct ParsedTemplatableMCPServerResult {
    pub templatable_mcp_server: TemplatableMCPServer,
    pub templatable_mcp_server_installation: Option<TemplatableMCPServerInstallation>,
    #[cfg_attr(target_family = "wasm", expect(dead_code))]
    pub variable_values: HashMap<String, VariableValue>,
}

/// Extracts a field from JSON as a HashMap<String, String>.
fn extract_string_map(
    json_value: &serde_json::Value,
    field_name: &str,
) -> Option<HashMap<String, String>> {
    json_value
        .get(field_name)
        .and_then(|value| value.as_object())
        .map(|obj| {
            obj.iter()
                .filter_map(|(key, value)| value.as_str().map(|s| (key.to_owned(), s.to_owned())))
                .collect()
        })
}

/// Converts a HashMap's keys to template placeholders (e.g., `{{key}}`) and
/// inserts the result back into the JSON value under the given field name.
fn templatize_field(
    json_value: &mut serde_json::Value,
    field_name: &str,
    map: &HashMap<String, String>,
) {
    let templated_value = serde_json::Value::Object(
        map.keys()
            .map(|key| {
                (
                    key.to_owned(),
                    serde_json::Value::String(format!("{{{{{key}}}}}")),
                )
            })
            .collect(),
    );
    if let Some(object) = json_value.as_object_mut() {
        object.insert(field_name.to_owned(), templated_value);
    }
}

impl ParsedTemplatableMCPServerResult {
    /// Parses MCP servers from a config file (e.g. `~/.claude.json`, `.mcp.json`).
    ///
    /// Unlike [`from_user_json`], this only recognises servers under a known
    /// wrapper key (`mcpServers`, `servers`, etc.) and will **not** fall back to
    /// treating every top-level key as a server name.
    #[cfg(not(target_family = "wasm"))]
    pub fn from_config_file_json(json: &str) -> serde_json::Result<Vec<Self>> {
        let json = json.trim();
        let json = if json.starts_with('{') {
            json.to_owned()
        } else {
            format!("{{{json}}}")
        };

        let config: serde_json::Value = serde_json::from_str(&json)?;
        let template_jsons = TemplatableMCPServer::find_template_map_strict(&config);

        Ok(template_jsons
            .iter()
            .map(|(name, json)| Self::parse_result(name, json))
            .collect())
    }

    /// Parses the user json and returns a vector of ParsedTemplatableMCPServerResult
    /// If the json is invalid, returns an error
    /// It's up to the caller to handle cases where there are an unexpected number of servers
    pub fn from_user_json(json: &str) -> serde_json::Result<Vec<Self>> {
        // Some docs don't show curly braces around the json object, so add them if necessary.
        let json = json.trim();
        let json = if json.starts_with('{') {
            json.to_owned()
        } else {
            format!("{{{json}}}")
        };

        let config: serde_json::Value = serde_json::from_str(&json)?;
        let template_jsons = TemplatableMCPServer::find_template_map(config)?;

        Ok(template_jsons
            .iter()
            .map(|(name, json)| Self::parse_result(name, json))
            .collect())
    }

    /// Parses a single MCP config
    /// Returns a ParsedTemplatableMCPServerResult
    /// If the json is invalid, returns an error
    pub(crate) fn parse_result(name: &str, json_value: &serde_json::Value) -> Self {
        // We need to clone the json value to avoid modifying the original value
        // json_value needs to be mutable to redact the env/header values
        let mut json_value = json_value.clone();

        let description: Option<String> = json_value
            .get("description")
            .and_then(|value| value.as_str().map(|s| s.to_owned()));

        // Extract env and headers, then convert their values to template placeholders
        let env = extract_string_map(&json_value, "env");
        if let Some(ref env) = env {
            templatize_field(&mut json_value, "env", env);
        }

        let headers = extract_string_map(&json_value, "headers");
        if let Some(ref headers) = headers {
            templatize_field(&mut json_value, "headers", headers);
        }

        let raw_json = json_value.to_string();
        let arguments = get_arguments(&raw_json);
        let variables = arguments
            .iter()
            .map(|argument| TemplateVariable {
                key: argument.clone(),
                allowed_values: None,
            })
            .collect::<Vec<TemplateVariable>>();

        // Each template_json is the nested config for a single MCP server
        // We need to re-wrap it in a top level object so that we can
        // reuse from_user_json to read it later
        let normalized_map =
            serde_json::Map::from_iter(vec![(name.to_owned(), json_value.clone())]);
        let normalized_value = serde_json::Value::Object(normalized_map);
        let normalized_json =
            serde_json::to_string_pretty(&normalized_value).unwrap_or(normalized_value.to_string());

        let templatable_mcp_server = TemplatableMCPServer {
            uuid: uuid::Uuid::new_v4(),
            name: name.to_owned(),
            description,
            template: JsonTemplate {
                json: normalized_json,
                variables,
            },
            version: chrono::Local::now().timestamp(),
            gallery_data: None,
        };

        // Combine env and headers into a single map for variable lookup
        let combined_values: HashMap<String, String> = env
            .clone()
            .unwrap_or_default()
            .into_iter()
            .chain(headers.clone().unwrap_or_default())
            .collect();

        // determine if all variables are present in env or headers
        let all_variables_present = templatable_mcp_server
            .template
            .variables
            .iter()
            .all(|variable| combined_values.contains_key(&variable.key));

        let variable_values: HashMap<String, VariableValue> = combined_values
            .into_iter()
            .map(|(key, value)| {
                (
                    key,
                    VariableValue {
                        variable_type: VariableType::Text,
                        value,
                    },
                )
            })
            .collect();

        let templatable_mcp_server_installation = match all_variables_present {
            true => Some(TemplatableMCPServerInstallation::new(
                uuid::Uuid::new_v4(),
                templatable_mcp_server.clone(),
                variable_values.clone(),
            )),
            false => None,
        };

        ParsedTemplatableMCPServerResult {
            templatable_mcp_server,
            templatable_mcp_server_installation,
            variable_values,
        }
    }
}

pub fn resolve_json(installation: &TemplatableMCPServerInstallation) -> String {
    // Collapse the variable values into a flat hashmap for easy replacement
    fn variable_value_to_string(variable_value: &VariableValue) -> String {
        match variable_value.variable_type {
            VariableType::Text => variable_value.value.clone(),
        }
    }

    let variable_values_strings: HashMap<String, String> = installation
        .variable_values()
        .iter()
        .map(|(key, variable_value)| (key.clone(), variable_value_to_string(variable_value)))
        .collect();

    render_template(installation.template_json(), &variable_values_strings)
}

pub fn prettify_json(json: &str) -> String {
    let value: serde_json::Value = serde_json::from_str(json).unwrap_or_default();
    serde_json::to_string_pretty(&value).unwrap_or(json.to_string())
}

#[cfg(test)]
#[path = "parsing_tests.rs"]
mod tests;
