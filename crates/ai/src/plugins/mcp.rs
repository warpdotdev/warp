//! Root `mcp.json` parsing and validation for Agent Plugins 1.0.0.
//!
//! This is deliberately not Warp's native file-based MCP parser. Agent Plugins uses a different
//! closed schema, requires an explicit per-entry `type` instead of inferring the transport from
//! the presence of `command` or `url`, and defines `${PLUGIN_ROOT}`/`${PLUGIN_DATA}` expansion
//! that has nothing to do with Warp's `{{variable}}` templating. Conflating the two would let a
//! malformed plugin entry take on native semantics it never asked for.
//!
//! Failure boundaries follow §7.2.2: a bad top level disables MCP for the plugin, while a bad or
//! unsupported individual entry disables only that entry.
use std::collections::BTreeMap;

use serde_json::{Map, Value};
use url::{Host, Url};

use super::diagnostics::{PluginDiagnostic, PluginDiagnosticCode};
use super::factory_mcp::FACTORY_MCP_SCHEMA_1_0_0;
use super::launch::{
    PLUGIN_DATA_PLACEHOLDER, PLUGIN_DATA_VAR, PLUGIN_ROOT_PLACEHOLDER, PLUGIN_ROOT_VAR,
};
use super::manifest::{AGENT_PLUGINS_VERSION_1_0_0, schema_version_from_id};
use super::paths::is_plugin_relative;

/// The fixed MCP configuration location inside a plugin root.
pub const MCP_FILE_NAME: &str = "mcp.json";

/// The canonical Agent Plugins 1.0.0 MCP configuration schema identifier.
pub const MCP_SCHEMA_1_0_0: &str = "https://agent-plugins.org/schemas/1.0.0/mcp.schema.json";

const PERMITTED_TOP_LEVEL_FIELDS: &[&str] = &["$schema", "mcpServers"];
const PERMITTED_STDIO_FIELDS: &[&str] = &["type", "command", "args", "env", "cwd"];
const PERMITTED_HTTP_FIELDS: &[&str] = &["type", "url", "headers"];

/// A validated Agent Plugins MCP server entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginMcpServer {
    /// The member name under `mcpServers`, used as the component's local name.
    pub name: String,
    pub transport: PluginMcpTransport,
}

/// The transports Warp connects with.
///
/// The legacy `sse` transport has no variant here: it is recognized during parsing so that it
/// can be reported as an unsupported transport rather than as a malformed entry, then skipped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginMcpTransport {
    Stdio {
        /// A single executable token: a bare name or a `./`-relative path. Never expanded.
        command: String,
        args: Vec<String>,
        env: BTreeMap<String, String>,
        /// The literal configured working directory, before placeholder expansion.
        cwd: Option<String>,
    },
    StreamableHttp {
        url: String,
        /// Headers in declaration order. Names are unique case-insensitively.
        headers: Vec<(String, String)>,
    },
}

/// The outcome of parsing a plugin's `mcp.json`.
#[derive(Debug, Clone, Default)]
pub struct ParsedPluginMcp {
    /// Entries Warp can use, in `mcpServers` declaration order.
    pub servers: Vec<PluginMcpServer>,
    /// Per-entry problems. Each one disables exactly one entry.
    pub diagnostics: Vec<PluginDiagnostic>,
}

/// Parses a plugin's root `mcp.json`.
///
/// `manifest_version` is the Agent Plugins version declared by `plugin.json`; §10.1 requires the
/// two to match. `Err` disables MCP for the whole plugin and leaves its other component types
/// alone.
pub fn parse_plugin_mcp(
    content: &str,
    manifest_version: &str,
) -> Result<ParsedPluginMcp, PluginDiagnostic> {
    let value: Value = serde_json::from_str(content).map_err(|error| {
        PluginDiagnostic::new(
            PluginDiagnosticCode::McpInvalidJson,
            format!("{MCP_FILE_NAME} is not valid JSON: {error}"),
        )
    })?;
    let Value::Object(object) = value else {
        return Err(PluginDiagnostic::new(
            PluginDiagnosticCode::McpInvalidTopLevel,
            format!("{MCP_FILE_NAME} must contain a top-level JSON object"),
        ));
    };

    validate_mcp_schema(&object, manifest_version)?;

    for field in object.keys() {
        if !PERMITTED_TOP_LEVEL_FIELDS.contains(&field.as_str()) {
            return Err(PluginDiagnostic::new(
                PluginDiagnosticCode::McpInvalidTopLevel,
                format!("{MCP_FILE_NAME} does not permit the top-level field '{field}'"),
            ));
        }
    }

    let Some(servers_value) = object.get("mcpServers") else {
        return Err(PluginDiagnostic::new(
            PluginDiagnosticCode::McpInvalidTopLevel,
            format!("{MCP_FILE_NAME} is missing the required 'mcpServers' field"),
        ));
    };
    let Some(servers) = servers_value.as_object() else {
        return Err(PluginDiagnostic::new(
            PluginDiagnosticCode::McpInvalidTopLevel,
            format!("{MCP_FILE_NAME} field 'mcpServers' must be an object"),
        ));
    };

    let mut parsed = ParsedPluginMcp::default();
    for (name, entry) in servers {
        match parse_server_entry(name, entry) {
            Ok(server) => parsed.servers.push(server),
            Err(diagnostic) => parsed.diagnostics.push(diagnostic.with_component(name)),
        }
    }
    Ok(parsed)
}

/// Applies the §7.2.1 and §10.1 rules for the MCP configuration's `$schema`.
///
/// A Warp Factory MCP file placed inside a plugin root is called out specifically: it is the one
/// wrong-schema case an author is likely to reach for deliberately, and the diagnostic has to say
/// that managed servers cannot enter through a plugin package.
fn validate_mcp_schema(
    object: &Map<String, Value>,
    manifest_version: &str,
) -> Result<(), PluginDiagnostic> {
    let Some(value) = object.get("$schema") else {
        return Err(PluginDiagnostic::new(
            PluginDiagnosticCode::McpUnsupportedSchema,
            format!("{MCP_FILE_NAME} is missing the required '$schema' field"),
        ));
    };
    let Some(schema_id) = value.as_str() else {
        return Err(PluginDiagnostic::new(
            PluginDiagnosticCode::McpUnsupportedSchema,
            format!("{MCP_FILE_NAME} field '$schema' must be a string"),
        ));
    };
    if schema_id == FACTORY_MCP_SCHEMA_1_0_0 {
        return Err(PluginDiagnostic::new(
            PluginDiagnosticCode::McpFactorySchemaInPluginRoot,
            format!(
                "{MCP_FILE_NAME} inside a plugin root declares the Warp Factory MCP schema; a \
                 plugin must use '{MCP_SCHEMA_1_0_0}' and cannot declare managed Warp MCP servers"
            ),
        ));
    }
    let Some(version) = schema_version_from_id(schema_id, "mcp.schema.json") else {
        return Err(PluginDiagnostic::new(
            PluginDiagnosticCode::McpUnsupportedSchema,
            format!("{MCP_FILE_NAME} field '$schema' must be '{MCP_SCHEMA_1_0_0}'"),
        ));
    };
    if version != manifest_version {
        return Err(PluginDiagnostic::new(
            PluginDiagnosticCode::McpVersionMismatch,
            format!(
                "{MCP_FILE_NAME} targets Agent Plugins {version} but plugin.json targets \
                 {manifest_version}"
            ),
        ));
    }
    if version != AGENT_PLUGINS_VERSION_1_0_0 {
        return Err(PluginDiagnostic::new(
            PluginDiagnosticCode::McpUnsupportedSchema,
            format!(
                "Agent Plugins {version} MCP configuration is not supported; Warp implements \
                 {AGENT_PLUGINS_VERSION_1_0_0}"
            ),
        ));
    }
    Ok(())
}

fn parse_server_entry(name: &str, entry: &Value) -> Result<PluginMcpServer, PluginDiagnostic> {
    let Some(object) = entry.as_object() else {
        return Err(invalid_server("server configuration must be an object"));
    };
    let Some(transport_type) = object.get("type") else {
        return Err(invalid_server("server configuration is missing 'type'"));
    };
    let Some(transport_type) = transport_type.as_str() else {
        return Err(invalid_server("server field 'type' must be a string"));
    };

    let transport = match transport_type {
        "stdio" => parse_stdio(object)?,
        "streamable-http" => parse_streamable_http(object)?,
        "sse" => {
            // Validate the entry before reporting it so an author does not fix the transport
            // only to discover the URL was also wrong.
            reject_unknown_fields(object, PERMITTED_HTTP_FIELDS)?;
            return Err(PluginDiagnostic::new(
                PluginDiagnosticCode::McpUnsupportedTransport,
                "the legacy 'sse' transport is not supported; use 'streamable-http'".to_owned(),
            ));
        }
        other => {
            return Err(invalid_server(format!(
                "server field 'type' must be 'stdio', 'streamable-http', or 'sse', found '{other}'"
            )));
        }
    };

    Ok(PluginMcpServer {
        name: name.to_owned(),
        transport,
    })
}

fn parse_stdio(object: &Map<String, Value>) -> Result<PluginMcpTransport, PluginDiagnostic> {
    reject_unknown_fields(object, PERMITTED_STDIO_FIELDS)?;

    let Some(command) = object.get("command").and_then(Value::as_str) else {
        return Err(invalid_server(
            "stdio server requires a string 'command' field",
        ));
    };
    validate_command_token(command)?;

    let args = match object.get("args") {
        None => Vec::new(),
        Some(value) => value
            .as_array()
            .ok_or_else(|| invalid_server("stdio server field 'args' must be an array"))?
            .iter()
            .map(|item| {
                item.as_str().map(str::to_owned).ok_or_else(|| {
                    invalid_server("stdio server field 'args' must contain only strings")
                })
            })
            .collect::<Result<Vec<_>, _>>()?,
    };

    let mut env = BTreeMap::new();
    if let Some(value) = object.get("env") {
        let entries = value
            .as_object()
            .ok_or_else(|| invalid_server("stdio server field 'env' must be an object"))?;
        for (key, item) in entries {
            if key == PLUGIN_ROOT_VAR || key == PLUGIN_DATA_VAR {
                return Err(invalid_server(format!(
                    "stdio server 'env' must not define the reserved variable '{key}'"
                )));
            }
            let item = item.as_str().ok_or_else(|| {
                invalid_server("stdio server field 'env' must contain only string values")
            })?;
            env.insert(key.clone(), item.to_owned());
        }
    }

    let cwd = match object.get("cwd") {
        None => None,
        Some(value) => {
            let cwd = value
                .as_str()
                .ok_or_else(|| invalid_server("stdio server field 'cwd' must be a string"))?;
            validate_cwd_form(cwd)?;
            Some(cwd.to_owned())
        }
    };

    Ok(PluginMcpTransport::Stdio {
        command: command.to_owned(),
        args,
        env,
        cwd,
    })
}

/// Enforces the §7.2.1 requirement that `command` is one executable token.
///
/// Placeholders are rejected outright rather than passed through literally. The standard forbids
/// expanding them here, so a `${PLUGIN_ROOT}`-prefixed command could only ever resolve to a file
/// literally named that — always an authoring mistake, and silently searching for it would make
/// the failure much harder to understand.
fn validate_command_token(command: &str) -> Result<(), PluginDiagnostic> {
    if command.is_empty() {
        return Err(invalid_server("stdio server 'command' must not be empty"));
    }
    if command.contains(PLUGIN_ROOT_PLACEHOLDER) || command.contains(PLUGIN_DATA_PLACEHOLDER) {
        return Err(invalid_server(
            "stdio server 'command' must not contain a placeholder; placeholders are not \
             expanded in 'command'",
        ));
    }
    if command.chars().any(char::is_whitespace) {
        return Err(invalid_server(
            "stdio server 'command' must be a single executable token, not a shell command string",
        ));
    }
    if is_plugin_relative(command) {
        return Ok(());
    }
    if command.contains('/') || command.contains('\\') {
        return Err(invalid_server(
            "stdio server 'command' must be a bare executable name or a plugin-relative path \
             beginning with './'",
        ));
    }
    Ok(())
}

/// Enforces the three `cwd` forms §7.2.1 permits, and rejects a parent-directory component.
///
/// Full containment still has to wait until `${PLUGIN_DATA}` is known, but a literal `..`
/// segment can never resolve to anything permitted whatever the roots turn out to be. Catching
/// it here reports the entry as invalid when the package is read, rather than letting it look
/// well-formed until someone tries to start the server.
fn validate_cwd_form(cwd: &str) -> Result<(), PluginDiagnostic> {
    let rooted_at = |placeholder: &str| {
        cwd == placeholder
            || cwd
                .strip_prefix(placeholder)
                .is_some_and(|rest| rest.starts_with('/'))
    };
    if !(is_plugin_relative(cwd)
        || rooted_at(PLUGIN_ROOT_PLACEHOLDER)
        || rooted_at(PLUGIN_DATA_PLACEHOLDER))
    {
        return Err(invalid_server(format!(
            "stdio server 'cwd' must begin with './', '{PLUGIN_ROOT_PLACEHOLDER}', or \
             '{PLUGIN_DATA_PLACEHOLDER}'"
        )));
    }
    // A whole segment, not a substring: `./a..b` is an ordinary directory name.
    if cwd.split('/').any(|segment| segment == "..") {
        return Err(invalid_server(
            "stdio server 'cwd' must not contain a parent-directory component",
        ));
    }
    Ok(())
}

fn parse_streamable_http(
    object: &Map<String, Value>,
) -> Result<PluginMcpTransport, PluginDiagnostic> {
    reject_unknown_fields(object, PERMITTED_HTTP_FIELDS)?;

    let Some(url) = object.get("url").and_then(Value::as_str) else {
        return Err(invalid_server(
            "streamable-http server requires a string 'url' field",
        ));
    };
    validate_remote_url(url)?;

    let headers = match object.get("headers") {
        None => Vec::new(),
        Some(value) => parse_headers(value)?,
    };

    Ok(PluginMcpTransport::StreamableHttp {
        url: url.to_owned(),
        headers,
    })
}

/// Applies the §7.2.1 URL rules: absolute HTTP(S), no userinfo, no fragment, HTTPS unless the
/// host is loopback.
fn validate_remote_url(raw: &str) -> Result<(), PluginDiagnostic> {
    let url = Url::parse(raw)
        .map_err(|error| invalid_server(format!("server 'url' is not an absolute URL: {error}")))?;
    let scheme = url.scheme();
    if scheme != "http" && scheme != "https" {
        return Err(invalid_server(format!(
            "server 'url' must use the http or https scheme, found '{scheme}'"
        )));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(invalid_server(
            "server 'url' must not contain user information",
        ));
    }
    if url.fragment().is_some() {
        return Err(invalid_server("server 'url' must not contain a fragment"));
    }
    if scheme == "http" && !is_loopback_host(url.host()) {
        return Err(invalid_server(
            "server 'url' must use https for a non-loopback host",
        ));
    }
    Ok(())
}

fn is_loopback_host(host: Option<Host<&str>>) -> bool {
    match host {
        Some(Host::Domain(domain)) => domain.eq_ignore_ascii_case("localhost"),
        Some(Host::Ipv4(address)) => address.is_loopback(),
        Some(Host::Ipv6(address)) => address.is_loopback(),
        None => false,
    }
}

/// Parses `headers`, enforcing valid HTTP field syntax and case-insensitive name uniqueness.
fn parse_headers(value: &Value) -> Result<Vec<(String, String)>, PluginDiagnostic> {
    let entries = value
        .as_object()
        .ok_or_else(|| invalid_server("server field 'headers' must be an object"))?;

    let mut headers = Vec::with_capacity(entries.len());
    let mut seen: Vec<String> = Vec::with_capacity(entries.len());
    for (name, item) in entries {
        if !is_valid_header_name(name) {
            return Err(invalid_server(format!(
                "server header name '{name}' is not a valid HTTP header field name"
            )));
        }
        let lowercase = name.to_ascii_lowercase();
        if seen.contains(&lowercase) {
            return Err(invalid_server(format!(
                "server headers declare '{name}' more than once under different casing"
            )));
        }
        let item = item
            .as_str()
            .ok_or_else(|| invalid_server("server field 'headers' must contain only strings"))?;
        if !is_valid_header_value(item) {
            // The value is deliberately not echoed: header values are package data that an
            // author may still have mistakenly used for a credential.
            return Err(invalid_server(format!(
                "server header '{name}' has a value that is not a valid HTTP header field value"
            )));
        }
        seen.push(lowercase);
        headers.push((name.clone(), item.to_owned()));
    }
    Ok(headers)
}

/// RFC 9110 field-name: one or more `tchar`.
fn is_valid_header_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().all(|c| {
            c.is_ascii_alphanumeric()
                || matches!(
                    c,
                    '!' | '#'
                        | '$'
                        | '%'
                        | '&'
                        | '\''
                        | '*'
                        | '+'
                        | '-'
                        | '.'
                        | '^'
                        | '_'
                        | '`'
                        | '|'
                        | '~'
                )
        })
}

/// RFC 9110 field-value: visible ASCII, space, and horizontal tab, with no leading or trailing
/// whitespace.
fn is_valid_header_value(value: &str) -> bool {
    if value.starts_with([' ', '\t']) || value.ends_with([' ', '\t']) {
        return false;
    }
    value
        .chars()
        .all(|c| c == '\t' || (' '..='~').contains(&c) || ('\u{80}'..='\u{ff}').contains(&c))
}

fn reject_unknown_fields(
    object: &Map<String, Value>,
    permitted: &[&str],
) -> Result<(), PluginDiagnostic> {
    for field in object.keys() {
        if !permitted.contains(&field.as_str()) {
            return Err(invalid_server(format!(
                "server configuration does not permit the field '{field}' for this transport"
            )));
        }
    }
    Ok(())
}

fn invalid_server(reason: impl Into<String>) -> PluginDiagnostic {
    PluginDiagnostic::new(PluginDiagnosticCode::McpServerInvalid, reason)
}

#[cfg(test)]
#[path = "mcp_tests.rs"]
mod tests;
