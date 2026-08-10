//! The Factory worker contract for plugins and entity-level MCP.
//!
//! A worker hands the client three environment variables rather than command-line arguments,
//! because the dispatch seam that already carries Factory skills is shared by three workers.
//! All three are comma-separated and ordered most specific first: automation, then the bound
//! agent, then the factory.
//!
//! The plugin-data rule is enforced here rather than at dispatch. Durable per-instance storage
//! only exists on some worker backends, so a worker that cannot provide it simply omits
//! [`PLUGIN_DATA_ROOT_ENV`]. Agent Plugins §9.1 requires a writable, persistent `PLUGIN_DATA`
//! for every plugin subprocess and admits no ephemeral fallback, so the client must refuse to
//! start plugin stdio servers in that case. Skills and remote-transport servers need no plugin
//! data and keep working.
use std::path::{Path, PathBuf};

use super::data::{FACTORY_UID_ENV, FactoryPluginDataLocator, PLUGIN_DATA_ROOT_ENV};
use super::diagnostics::{PluginDiagnostic, PluginDiagnosticCode};
use super::mcp::{PluginMcpServer, PluginMcpTransport};

/// Plugin *collection* directories, whose immediate children are plugin packages.
pub const PLUGIN_DIRS_ENV: &str = "WARP_PLUGIN_DIRS";

/// Warp Factory MCP file paths.
pub const FACTORY_MCP_FILES_ENV: &str = "WARP_FACTORY_MCP_FILES";

/// What a Factory worker asked this run to load.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FactoryPluginRuntime {
    /// Plugin collection directories, most specific first.
    pub plugin_collection_dirs: Vec<PathBuf>,
    /// Factory MCP file paths, most specific first.
    pub factory_mcp_files: Vec<PathBuf>,
    /// The durable root for plugin data, when the worker backend can provide one.
    ///
    /// Already contains the Factory UID. The client appends only `<scope>/<plugin-key>`.
    pub plugin_data_root: Option<PathBuf>,
    /// The Factory UID, for identity and diagnostics only. Never composed into a path.
    pub factory_uid: Option<String>,
}

impl FactoryPluginRuntime {
    /// Reads the contract from the process environment.
    ///
    /// `base` is the environment working directory; relative entries resolve against it so their
    /// meaning does not depend on the process's current directory, which setup may have changed.
    pub fn from_env(base: &Path) -> Self {
        Self {
            plugin_collection_dirs: resolve_all(&split_paths(&read_env(PLUGIN_DIRS_ENV)), base),
            factory_mcp_files: resolve_all(&split_paths(&read_env(FACTORY_MCP_FILES_ENV)), base),
            // An absolute root only: a relative durable root would be meaningless across the
            // worker's own directory changes.
            plugin_data_root: read_env(PLUGIN_DATA_ROOT_ENV)
                .map(PathBuf::from)
                .filter(|path| path.is_absolute()),
            factory_uid: read_env(FACTORY_UID_ENV),
        }
    }

    /// The locator for this run's plugin data, when the worker provided a durable root.
    ///
    /// `None` is the same condition as [`allows_stdio_plugin_servers`](Self::allows_stdio_plugin_servers)
    /// being false: with nowhere durable to put `PLUGIN_DATA`, no plugin stdio server may start.
    pub fn data_locator(&self) -> Option<FactoryPluginDataLocator> {
        self.plugin_data_root
            .as_ref()
            .map(|root| FactoryPluginDataLocator::new(root, self.factory_uid.clone()))
    }

    /// Whether this run may start plugin stdio MCP servers at all.
    pub fn allows_stdio_plugin_servers(&self) -> bool {
        self.plugin_data_root.is_some()
    }

    /// Partitions a plugin's servers into the ones this run may start and the ones it must
    /// refuse, with a diagnostic for each refusal.
    ///
    /// Refusing is the conformant outcome: starting a stdio server against a directory that does
    /// not survive the run would silently break the persistence guarantee the standard makes to
    /// the plugin.
    pub fn partition_startable<'a>(
        &self,
        plugin_name: &str,
        servers: impl IntoIterator<Item = &'a PluginMcpServer>,
    ) -> (Vec<&'a PluginMcpServer>, Vec<PluginDiagnostic>) {
        let mut startable = Vec::new();
        let mut refused = Vec::new();
        for server in servers {
            let is_stdio = matches!(server.transport, PluginMcpTransport::Stdio { .. });
            if is_stdio && !self.allows_stdio_plugin_servers() {
                refused.push(
                    PluginDiagnostic::new(
                        PluginDiagnosticCode::McpServerInvalid,
                        format!(
                            "this worker provides no durable plugin data root, so the stdio \
                             server cannot be started; Agent Plugins requires a persistent \
                             PLUGIN_DATA directory and permits no ephemeral fallback. Set \
                             {PLUGIN_DATA_ROOT_ENV} on a worker backend with durable storage."
                        ),
                    )
                    .with_plugin(plugin_name)
                    .with_component(&server.name),
                );
                continue;
            }
            startable.push(server);
        }
        (startable, refused)
    }
}

fn read_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

/// Splits a comma-separated list, preserving order and dropping blank entries.
fn split_paths(value: &Option<String>) -> Vec<PathBuf> {
    let Some(value) = value else {
        return Vec::new();
    };
    value
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(PathBuf::from)
        .collect()
}

fn resolve_all(paths: &[PathBuf], base: &Path) -> Vec<PathBuf> {
    paths
        .iter()
        .map(|path| {
            if path.is_absolute() {
                path.clone()
            } else {
                base.join(path)
            }
        })
        .collect()
}

#[cfg(test)]
#[path = "factory_runtime_tests.rs"]
mod tests;
