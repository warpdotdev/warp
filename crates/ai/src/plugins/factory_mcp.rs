//! The Warp Factory `mcp.json` artifact.
//!
//! This is not an Agent Plugins file. A Factory MCP file lives at a fixed Factory entity
//! directory — outside every plugin root — declares the Warp Factory schema, and may carry
//! managed `warpId` entries alongside ordinary ones. Location is the primary discriminator and
//! `$schema` is the second, which is what stops a plugin author from smuggling a managed server
//! in by dropping a Factory-shaped file into a plugin package.
//!
//! Factoryfile sync owns the managed entries and has already projected them into the run's
//! managed MCP configuration by the time the client sees the file, so the client recognizes them
//! and moves on. The client owns the ordinary entries: it resolves their relative paths against
//! the entity directory that contains the file, and it never expands `${PLUGIN_ROOT}` or
//! `${PLUGIN_DATA}`, which are meaningless outside a plugin package.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use super::diagnostics::{PluginDiagnostic, PluginDiagnosticCode};
use super::launch::{PLUGIN_DATA_PLACEHOLDER, PLUGIN_ROOT_PLACEHOLDER};
use super::mcp::MCP_SCHEMA_1_0_0;

/// The canonical Warp Factory MCP 1.0.0 schema identifier.
pub const FACTORY_MCP_SCHEMA_1_0_0: &str = "https://warp.dev/schemas/factory-mcp/1.0.0/schema.json";

const PERMITTED_TOP_LEVEL_FIELDS: &[&str] = &["$schema", "mcpServers"];
const PERMITTED_MANAGED_FIELDS: &[&str] = &["type", "warpId"];
const PERMITTED_STDIO_FIELDS: &[&str] = &["type", "command", "args", "env", "cwd"];
const PERMITTED_HTTP_FIELDS: &[&str] = &["type", "url", "headers"];

/// One entry in a Factory MCP file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FactoryMcpEntry {
    /// A reference to a Warp-managed MCP server, already projected by Factoryfile sync.
    ///
    /// The client records it so that surfaces can explain where a managed server came from, and
    /// creates no installation for it.
    Managed { warp_id: String },
    Stdio {
        command: String,
        args: Vec<String>,
        env: BTreeMap<String, String>,
        cwd: Option<String>,
    },
    StreamableHttp {
        url: String,
        headers: Vec<(String, String)>,
    },
}

impl FactoryMcpEntry {
    /// Whether the client creates an installation for this entry.
    pub fn is_client_owned(&self) -> bool {
        !matches!(self, FactoryMcpEntry::Managed { .. })
    }
}

/// A named entry from a Factory MCP file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactoryMcpServer {
    pub name: String,
    pub entry: FactoryMcpEntry,
}

/// A parsed Factory MCP file.
#[derive(Debug, Clone)]
pub struct FactoryMcpFile {
    pub path: PathBuf,
    /// The entity directory that contains the file; relative paths resolve against it.
    pub entity_dir: PathBuf,
    /// Ordinary entries the client is responsible for loading, in declaration order.
    pub servers: Vec<FactoryMcpServer>,
    /// Names of the managed entries that were recognized and deliberately not installed.
    pub ignored_managed: Vec<String>,
    pub diagnostics: Vec<PluginDiagnostic>,
}

/// Parses a Factory MCP file supplied by a worker.
///
/// `Err` means the whole file is unusable. `Ok` may still carry per-entry diagnostics; one bad
/// entry never invalidates the rest.
pub fn parse_factory_mcp_file(
    path: &Path,
    content: &str,
) -> Result<FactoryMcpFile, PluginDiagnostic> {
    let entity_dir = path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    let value: Value = serde_json::from_str(content).map_err(|error| {
        PluginDiagnostic::new(
            PluginDiagnosticCode::FactoryMcpInvalid,
            format!("Factory MCP file is not valid JSON: {error}"),
        )
        .with_path(path)
    })?;
    let Value::Object(object) = value else {
        return Err(PluginDiagnostic::new(
            PluginDiagnosticCode::FactoryMcpInvalid,
            "Factory MCP file must contain a top-level JSON object".to_owned(),
        )
        .with_path(path));
    };

    validate_factory_schema(&object).map_err(|diagnostic| diagnostic.with_path(path))?;

    for field in object.keys() {
        if !PERMITTED_TOP_LEVEL_FIELDS.contains(&field.as_str()) {
            return Err(PluginDiagnostic::new(
                PluginDiagnosticCode::FactoryMcpInvalid,
                format!("Factory MCP file does not permit the top-level field '{field}'"),
            )
            .with_path(path));
        }
    }

    let Some(servers) = object.get("mcpServers").and_then(Value::as_object) else {
        return Err(PluginDiagnostic::new(
            PluginDiagnosticCode::FactoryMcpInvalid,
            "Factory MCP file requires an object 'mcpServers' field".to_owned(),
        )
        .with_path(path));
    };

    let mut file = FactoryMcpFile {
        path: path.to_path_buf(),
        entity_dir,
        servers: Vec::new(),
        ignored_managed: Vec::new(),
        diagnostics: Vec::new(),
    };
    for (name, entry) in servers {
        match parse_entry(entry) {
            Ok(FactoryMcpEntry::Managed { .. }) => file.ignored_managed.push(name.clone()),
            Ok(entry) => file.servers.push(FactoryMcpServer {
                name: name.clone(),
                entry,
            }),
            Err(diagnostic) => file
                .diagnostics
                .push(diagnostic.with_component(name).with_path(path)),
        }
    }
    Ok(file)
}

/// Requires the Warp Factory schema, and gives the Agent Plugins schema its own message.
///
/// An author who reaches for the Agent Plugins schema at a Factory entity path has the two
/// artifacts confused, and saying so is far more useful than a generic mismatch.
fn validate_factory_schema(object: &Map<String, Value>) -> Result<(), PluginDiagnostic> {
    let schema_id = object.get("$schema").and_then(Value::as_str);
    match schema_id {
        Some(FACTORY_MCP_SCHEMA_1_0_0) => Ok(()),
        Some(MCP_SCHEMA_1_0_0) => Err(PluginDiagnostic::new(
            PluginDiagnosticCode::FactoryMcpAgentPluginsSchema,
            format!(
                "an Agent Plugins MCP configuration is not valid at a Factory entity location; \
                 Agent Plugins MCP belongs in a plugin package's root mcp.json, and a Factory \
                 file must declare '{FACTORY_MCP_SCHEMA_1_0_0}'"
            ),
        )),
        _ => Err(PluginDiagnostic::new(
            PluginDiagnosticCode::FactoryMcpInvalid,
            format!("Factory MCP file must declare '$schema': '{FACTORY_MCP_SCHEMA_1_0_0}'"),
        )),
    }
}

fn parse_entry(entry: &Value) -> Result<FactoryMcpEntry, PluginDiagnostic> {
    let Some(object) = entry.as_object() else {
        return Err(invalid_entry("entry must be an object"));
    };
    let Some(entry_type) = object.get("type").and_then(Value::as_str) else {
        return Err(invalid_entry("entry requires a string 'type' field"));
    };

    match entry_type {
        "managed" => {
            reject_unknown_fields(object, PERMITTED_MANAGED_FIELDS)?;
            let warp_id = object
                .get("warpId")
                .and_then(Value::as_str)
                .ok_or_else(|| invalid_entry("managed entry requires a string 'warpId' field"))?;
            Ok(FactoryMcpEntry::Managed {
                warp_id: warp_id.to_owned(),
            })
        }
        "stdio" => {
            reject_unknown_fields(object, PERMITTED_STDIO_FIELDS)?;
            let command = object
                .get("command")
                .and_then(Value::as_str)
                .ok_or_else(|| invalid_entry("stdio entry requires a string 'command' field"))?;
            reject_plugin_placeholders(command, "command")?;
            let args = string_array(object, "args")?;
            for arg in &args {
                reject_plugin_placeholders(arg, "args")?;
            }
            let env = string_map(object, "env")?;
            for value in env.values() {
                reject_plugin_placeholders(value, "env")?;
            }
            let cwd = match object.get("cwd") {
                None => None,
                Some(value) => {
                    let cwd = value
                        .as_str()
                        .ok_or_else(|| invalid_entry("stdio entry field 'cwd' must be a string"))?;
                    reject_plugin_placeholders(cwd, "cwd")?;
                    Some(cwd.to_owned())
                }
            };
            Ok(FactoryMcpEntry::Stdio {
                command: command.to_owned(),
                args,
                env,
                cwd,
            })
        }
        "streamable-http" => {
            reject_unknown_fields(object, PERMITTED_HTTP_FIELDS)?;
            let url = object.get("url").and_then(Value::as_str).ok_or_else(|| {
                invalid_entry("streamable-http entry requires a string 'url' field")
            })?;
            let headers = match object.get("headers") {
                None => Vec::new(),
                Some(value) => {
                    let entries = value
                        .as_object()
                        .ok_or_else(|| invalid_entry("entry field 'headers' must be an object"))?;
                    entries
                        .iter()
                        .map(|(name, item)| {
                            item.as_str()
                                .map(|item| (name.clone(), item.to_owned()))
                                .ok_or_else(|| {
                                    invalid_entry("entry field 'headers' must contain only strings")
                                })
                        })
                        .collect::<Result<Vec<_>, _>>()?
                }
            };
            Ok(FactoryMcpEntry::StreamableHttp {
                url: url.to_owned(),
                headers,
            })
        }
        other => Err(invalid_entry(format!(
            "entry field 'type' must be 'managed', 'stdio', or 'streamable-http', found '{other}'"
        ))),
    }
}

/// Factory files define no plugin placeholders, so an author using one has almost certainly
/// copied a plugin entry and expects an expansion that will never happen.
fn reject_plugin_placeholders(value: &str, field: &str) -> Result<(), PluginDiagnostic> {
    if value.contains(PLUGIN_ROOT_PLACEHOLDER) || value.contains(PLUGIN_DATA_PLACEHOLDER) {
        return Err(invalid_entry(format!(
            "entry field '{field}' must not use {PLUGIN_ROOT_PLACEHOLDER} or \
             {PLUGIN_DATA_PLACEHOLDER}; those are defined only inside a plugin package"
        )));
    }
    Ok(())
}

fn string_array(object: &Map<String, Value>, field: &str) -> Result<Vec<String>, PluginDiagnostic> {
    let Some(value) = object.get(field) else {
        return Ok(Vec::new());
    };
    value
        .as_array()
        .ok_or_else(|| invalid_entry(format!("entry field '{field}' must be an array")))?
        .iter()
        .map(|item| {
            item.as_str().map(str::to_owned).ok_or_else(|| {
                invalid_entry(format!("entry field '{field}' must contain only strings"))
            })
        })
        .collect()
}

fn string_map(
    object: &Map<String, Value>,
    field: &str,
) -> Result<BTreeMap<String, String>, PluginDiagnostic> {
    let Some(value) = object.get(field) else {
        return Ok(BTreeMap::new());
    };
    value
        .as_object()
        .ok_or_else(|| invalid_entry(format!("entry field '{field}' must be an object")))?
        .iter()
        .map(|(key, item)| {
            item.as_str()
                .map(|item| (key.clone(), item.to_owned()))
                .ok_or_else(|| {
                    invalid_entry(format!(
                        "entry field '{field}' must contain only string values"
                    ))
                })
        })
        .collect()
}

fn reject_unknown_fields(
    object: &Map<String, Value>,
    permitted: &[&str],
) -> Result<(), PluginDiagnostic> {
    for field in object.keys() {
        if !permitted.contains(&field.as_str()) {
            return Err(invalid_entry(format!(
                "entry does not permit the field '{field}' for this variant"
            )));
        }
    }
    Ok(())
}

fn invalid_entry(reason: impl Into<String>) -> PluginDiagnostic {
    PluginDiagnostic::new(PluginDiagnosticCode::FactoryMcpEntryInvalid, reason)
}

#[cfg(test)]
#[path = "factory_mcp_tests.rs"]
mod tests;
