//! Agent Plugins 1.0.0 package model.
//!
//! This module implements the portable parts of the [Agent Plugins
//! specification](https://agent-plugins.org/specification): manifest parsing, component
//! discovery within a package, the `mcp.json` configuration format, filesystem containment,
//! placeholder expansion, and the precedence rules Warp applies across plugin search roots.
//!
//! Nothing here touches the UI, filesystem watchers, or process spawning. Loading a package
//! only reads files; the launch plan produced by [`launch`] is inert until an MCP caller
//! chooses to spawn it.
mod data;
mod diagnostics;
mod discovery;
mod factory_mcp;
mod factory_runtime;
mod identity;
mod launch;
mod manifest;
mod mcp;
mod package;
mod paths;

pub use data::{
    FACTORY_UID_ENV, FactoryPluginDataLocator, LocalPluginDataLocator, PLUGIN_DATA_ROOT_ENV,
    PluginDataLocator, PluginFrontend, plugin_data_instance_key,
};
pub use diagnostics::{PluginDiagnostic, PluginDiagnosticCode, PluginDiagnosticSeverity};
pub use discovery::{
    ActivePluginSet, PluginCandidate, PluginSearchRoot, REPOSITORY_PLUGIN_PATHS, ShadowedPlugin,
    precedence_rank, repository_search_roots, resolve_active_packages, scan_search_root,
    user_search_roots,
};
pub use factory_mcp::{
    FACTORY_MCP_SCHEMA_1_0_0, FactoryMcpEntry, FactoryMcpFile, FactoryMcpServer,
    parse_factory_mcp_file,
};
pub use factory_runtime::{FACTORY_MCP_FILES_ENV, FactoryPluginRuntime, PLUGIN_DIRS_ENV};
pub use identity::{
    PluginComponentId, PluginComponentKind, PluginInstanceId, PluginScopeId, PluginSourceId,
    PluginSourceKind, filesystem_safe_segment, split_qualified_name,
};
pub use launch::{
    PLUGIN_DATA_VAR, PLUGIN_ROOT_VAR, PluginPlaceholders, ResolvedCommand, StdioLaunchPlan,
    expand_placeholders, plan_stdio_launch,
};
pub use manifest::{
    AGENT_PLUGINS_VERSION_1_0_0, MANIFEST_FILE_NAME, MANIFEST_SCHEMA_1_0_0, ParsedManifest,
    PluginAuthor, PluginManifest, parse_manifest, schema_version_from_id, validate_plugin_name,
};
pub use mcp::{
    MCP_FILE_NAME, MCP_SCHEMA_1_0_0, ParsedPluginMcp, PluginMcpServer, PluginMcpTransport,
    parse_plugin_mcp,
};
pub use package::{
    PluginPackage, PluginSkillComponent, SKILL_FILE_NAME, SKILLS_DIR_NAME, load_plugin_package,
};
pub use paths::{PluginPathError, is_plugin_relative, resolve_contained, resolve_partial};

#[cfg(test)]
#[path = "conformance_tests.rs"]
mod conformance_tests;
