use std::ffi::OsStr;

use clap::builder::PossibleValue;
use clap::error::ErrorKind;
use clap::{Arg, Command, Subcommand};

/// MCP-related subcommands.
#[derive(Debug, Clone, Subcommand)]
pub enum MCPCommand {
    /// List MCP servers.
    List,
}

impl MCPCommand {
    pub(crate) fn as_str_for_tracing(&self) -> &'static str {
        match self {
            MCPCommand::List => "mcp list",
        }
    }
}

/// Represents an MCP server specification from CLI input.
///
/// This is a lightweight representation - full parsing happens in the app layer
/// using `ParsedTemplatableMCPServerResult::from_user_json`.
#[derive(Debug, Clone)]
pub enum MCPSpec {
    /// Existing server by UUID.
    Uuid(uuid::Uuid),
    /// Well-known non-UUID managed MCP id (e.g. "linear"), resolved by the
    /// server. The server owns the set of recognized ids — ids it does not
    /// recognize fail resolution and are skipped at run setup, so new ids can
    /// be introduced server-side without a client change.
    WellKnown(String),
    /// JSON string (full config, server map, or single server).
    /// Parsing deferred to app layer.
    Json(String),
}

/// A bare identifier (letters, digits, `-`, `_`) that is not a UUID: treated
/// as a well-known managed MCP id and sent to the server for resolution.
pub fn is_well_known_mcp_id(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        && uuid::Uuid::parse_str(s).is_err()
}

impl clap::builder::ValueParserFactory for MCPSpec {
    type Parser = MCPSpecParser;

    fn value_parser() -> Self::Parser {
        MCPSpecParser
    }
}

#[derive(Copy, Clone)]
pub struct MCPSpecParser;

impl clap::builder::TypedValueParser for MCPSpecParser {
    type Value = MCPSpec;

    fn parse_ref(
        &self,
        _cmd: &Command,
        _arg: Option<&Arg>,
        value: &OsStr,
    ) -> Result<Self::Value, clap::Error> {
        let s = value
            .to_str()
            .ok_or_else(|| clap::Error::raw(ErrorKind::InvalidUtf8, "Invalid UTF-8 in MCP spec"))?;

        // Try UUID first
        if let Ok(uuid) = uuid::Uuid::parse_str(s) {
            return Ok(MCPSpec::Uuid(uuid));
        }

        // Check if it's a file path
        let path = std::path::Path::new(s);
        let json_content = if path.exists() && path.is_file() {
            std::fs::read_to_string(path).map_err(|e| {
                clap::Error::raw(
                    ErrorKind::Io,
                    format!("Failed to read MCP config file '{}': {e}", path.display()),
                )
            })?
        } else if is_well_known_mcp_id(s) {
            // Bare identifiers (e.g. "linear") are well-known managed MCP ids
            // resolved by the server at run setup.
            return Ok(MCPSpec::WellKnown(s.to_string()));
        } else {
            // Treat as inline JSON
            s.to_string()
        };

        Ok(MCPSpec::Json(json_content))
    }

    fn possible_values(&self) -> Option<Box<dyn Iterator<Item = PossibleValue> + '_>> {
        Some(Box::new(
            [
                PossibleValue::new("<path>").help("Path to a JSON file containing MCP config"),
                PossibleValue::new("<json>").help("Inline JSON MCP server configuration"),
            ]
            .into_iter(),
        ))
    }
}

#[cfg(test)]
#[path = "mcp_tests.rs"]
mod tests;
