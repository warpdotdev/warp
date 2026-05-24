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

/// Represents an MCP server specification from CLI input.
///
/// This is a lightweight representation - full parsing happens in the app layer
/// using `ParsedTemplatableMCPServerResult::from_user_json`.
#[derive(Debug, Clone)]
pub enum MCPSpec {
    /// Existing server by UUID.
    Uuid(uuid::Uuid),
    /// JSON string (full config, server map, or single server).
    /// Parsing deferred to app layer.
    Json(String),
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
