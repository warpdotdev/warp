//! Stable diagnostic codes for plugin loading, component validation, and precedence.
//!
//! Codes are part of the contract with structured logs and with the messages returned for an
//! explicit invocation of an invalid or ambiguous component, so they must stay stable once
//! shipped. A diagnostic never carries a secret: it names the plugin, its scope, its source
//! path, and a reason, and it deliberately omits configured `env` and header values.
use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// How badly a diagnostic affects what the user can use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PluginDiagnosticSeverity {
    /// Something the user asked for is unavailable.
    Error,
    /// Something was ignored or superseded, but a usable result remains.
    Warning,
}

/// A stable identifier for a diagnostic condition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PluginDiagnosticCode {
    /// The candidate directory has no readable root `plugin.json`.
    ManifestMissing,
    /// The root `plugin.json` could not be read.
    ManifestUnreadable,
    /// The root `plugin.json` is not valid JSON, or not a JSON object.
    ManifestInvalidJson,
    /// The manifest `$schema` is absent or names an Agent Plugins version Warp does not support.
    ManifestUnsupportedSchema,
    /// A top-level manifest field outside the closed schema. Reported and ignored.
    ManifestUnknownField,
    /// The `extensions` field is not an object. Reported and ignored.
    ManifestInvalidExtensions,
    /// A permitted manifest field has the wrong type or violates its constraints.
    ManifestInvalidField,
    /// The manifest `name` violates the Agent Plugins name constraints.
    ManifestInvalidName,
    /// A package path resolved outside the filesystem-resolved plugin root.
    PathEscapesPluginRoot,
    /// A fixed component location exists but is the wrong filesystem kind.
    ComponentWrongFilesystemKind,
    /// One skill could not be loaded. Other skills and component types are unaffected.
    SkillInvalid,
    /// `mcp.json` is not valid JSON, so MCP is disabled for this plugin.
    McpInvalidJson,
    /// `mcp.json` declares a `$schema` Warp does not support, so MCP is disabled for this plugin.
    McpUnsupportedSchema,
    /// `mcp.json` targets a different Agent Plugins version than `plugin.json`.
    McpVersionMismatch,
    /// `mcp.json` violates a top-level requirement, so MCP is disabled for this plugin.
    McpInvalidTopLevel,
    /// A Warp Factory MCP file was found inside a plugin root. MCP is disabled for this plugin.
    McpFactorySchemaInPluginRoot,
    /// One MCP server entry is invalid. Other entries and component types are unaffected.
    McpServerInvalid,
    /// One MCP server entry declares a transport Warp does not support in v1.
    McpUnsupportedTransport,
    /// A same-name package at a lower precedence was shadowed as a complete package.
    PluginShadowed,
    /// Two equally ranked sources provide the same plugin name, so neither is selected.
    PluginAmbiguous,
    /// An unqualified component name matches more than one active component.
    ComponentAmbiguous,
    /// A plugin component was referenced while Agent Plugin discovery is disabled.
    DiscoveryDisabled,
    /// A Factory MCP file declares the Agent Plugins schema at a Factory entity location.
    FactoryMcpAgentPluginsSchema,
    /// A Factory MCP file is not usable: unreadable, invalid JSON, or an invalid top level.
    FactoryMcpInvalid,
    /// One Factory MCP entry is invalid. Other entries are unaffected.
    FactoryMcpEntryInvalid,
}

impl PluginDiagnosticCode {
    /// The stable wire/log representation of this code.
    pub fn as_str(self) -> &'static str {
        match self {
            PluginDiagnosticCode::ManifestMissing => "agent_plugin_manifest_missing",
            PluginDiagnosticCode::ManifestUnreadable => "agent_plugin_manifest_unreadable",
            PluginDiagnosticCode::ManifestInvalidJson => "agent_plugin_manifest_invalid_json",
            PluginDiagnosticCode::ManifestUnsupportedSchema => {
                "agent_plugin_manifest_unsupported_schema"
            }
            PluginDiagnosticCode::ManifestUnknownField => "agent_plugin_manifest_unknown_field",
            PluginDiagnosticCode::ManifestInvalidExtensions => {
                "agent_plugin_manifest_invalid_extensions"
            }
            PluginDiagnosticCode::ManifestInvalidField => "agent_plugin_manifest_invalid_field",
            PluginDiagnosticCode::ManifestInvalidName => "agent_plugin_manifest_invalid_name",
            PluginDiagnosticCode::PathEscapesPluginRoot => "agent_plugin_path_escapes_plugin_root",
            PluginDiagnosticCode::ComponentWrongFilesystemKind => {
                "agent_plugin_component_wrong_filesystem_kind"
            }
            PluginDiagnosticCode::SkillInvalid => "agent_plugin_skill_invalid",
            PluginDiagnosticCode::McpInvalidJson => "agent_plugin_mcp_invalid_json",
            PluginDiagnosticCode::McpUnsupportedSchema => "agent_plugin_mcp_unsupported_schema",
            PluginDiagnosticCode::McpVersionMismatch => "agent_plugin_mcp_version_mismatch",
            PluginDiagnosticCode::McpInvalidTopLevel => "agent_plugin_mcp_invalid_top_level",
            PluginDiagnosticCode::McpFactorySchemaInPluginRoot => {
                "agent_plugin_mcp_factory_schema_in_plugin_root"
            }
            PluginDiagnosticCode::McpServerInvalid => "agent_plugin_mcp_server_invalid",
            PluginDiagnosticCode::McpUnsupportedTransport => {
                "agent_plugin_mcp_unsupported_transport"
            }
            PluginDiagnosticCode::PluginShadowed => "agent_plugin_shadowed",
            PluginDiagnosticCode::PluginAmbiguous => "agent_plugin_ambiguous",
            PluginDiagnosticCode::ComponentAmbiguous => "agent_plugin_component_ambiguous",
            PluginDiagnosticCode::DiscoveryDisabled => "agent_plugin_discovery_disabled",
            PluginDiagnosticCode::FactoryMcpAgentPluginsSchema => {
                "factory_mcp_agent_plugins_schema"
            }
            PluginDiagnosticCode::FactoryMcpInvalid => "factory_mcp_invalid",
            PluginDiagnosticCode::FactoryMcpEntryInvalid => "factory_mcp_entry_invalid",
        }
    }

    fn default_severity(self) -> PluginDiagnosticSeverity {
        match self {
            PluginDiagnosticCode::ManifestUnknownField
            | PluginDiagnosticCode::ManifestInvalidExtensions
            | PluginDiagnosticCode::McpUnsupportedTransport
            | PluginDiagnosticCode::PluginShadowed => PluginDiagnosticSeverity::Warning,
            _ => PluginDiagnosticSeverity::Error,
        }
    }
}

impl fmt::Display for PluginDiagnosticCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One reported plugin problem, with enough context to act on it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginDiagnostic {
    pub code: PluginDiagnosticCode,
    pub severity: PluginDiagnosticSeverity,
    /// The manifest name when known, otherwise the candidate directory name.
    pub plugin: Option<String>,
    /// The component this diagnostic is scoped to, when it is not package-level.
    pub component: Option<String>,
    /// The file or directory the diagnostic is about.
    pub path: Option<PathBuf>,
    /// An actionable, secret-free explanation.
    pub reason: String,
}

impl PluginDiagnostic {
    pub fn new(code: PluginDiagnosticCode, reason: impl Into<String>) -> Self {
        Self {
            code,
            severity: code.default_severity(),
            plugin: None,
            component: None,
            path: None,
            reason: reason.into(),
        }
    }

    pub fn with_plugin(mut self, plugin: impl Into<String>) -> Self {
        self.plugin = Some(plugin.into());
        self
    }

    pub fn with_component(mut self, component: impl Into<String>) -> Self {
        self.component = Some(component.into());
        self
    }

    pub fn with_path(mut self, path: impl AsRef<Path>) -> Self {
        self.path = Some(path.as_ref().to_path_buf());
        self
    }

    /// Fills in the plugin name for diagnostics collected before the manifest was parsed.
    pub fn or_plugin(mut self, plugin: &str) -> Self {
        if self.plugin.is_none() {
            self.plugin = Some(plugin.to_owned());
        }
        self
    }

    pub fn is_error(&self) -> bool {
        self.severity == PluginDiagnosticSeverity::Error
    }
}

impl fmt::Display for PluginDiagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}]", self.code)?;
        if let Some(plugin) = &self.plugin {
            write!(f, " plugin '{plugin}'")?;
        }
        if let Some(component) = &self.component {
            write!(f, " component '{component}'")?;
        }
        write!(f, ": {}", self.reason)?;
        if let Some(path) = &self.path {
            write!(f, " ({})", path.display())?;
        }
        Ok(())
    }
}
