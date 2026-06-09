//! Dev-container-specific shell-starter types and helpers.
//!
//! This module owns the first local Dev Container integration surface: finding
//! devcontainer configs in a workspace, resolving the `devcontainer` CLI from
//! the user's shell PATH, and carrying the metadata needed to launch a shell
//! inside the configured container.

use futures::future::BoxFuture;
use futures::FutureExt as _;
use serde::{Deserialize, Serialize};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use warpui::{AppContext, SingletonEntity as _};

use super::shell::DirectShellStarter;
use crate::{
    terminal::shell::ShellType,
    util::path::{resolve_executable, resolve_executable_in_path},
};

#[cfg(feature = "local_tty")]
use crate::terminal::local_shell::LocalShellState;

const DEVCONTAINER_DIR: &str = ".devcontainer";
const DEVCONTAINER_FILE: &str = "devcontainer.json";
const ROOT_DEVCONTAINER_FILE: &str = ".devcontainer.json";

pub const DEVCONTAINER_CLI_NAME: &str = "devcontainer";

/// A devcontainer config selected for a workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevContainerConfig {
    pub workspace_folder: PathBuf,
    pub config_path: PathBuf,
    pub name: Option<String>,
}

impl DevContainerConfig {
    pub fn display_name(&self) -> String {
        if let Some(name) = self.name.as_deref().filter(|name| !name.is_empty()) {
            return name.to_owned();
        }

        let devcontainer_dir = self.workspace_folder.join(DEVCONTAINER_DIR);
        match self.config_path.parent() {
            Some(parent) if parent != devcontainer_dir => parent
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("Dev Container")
                .to_owned(),
            _ => "Dev Container".to_owned(),
        }
    }
}

/// Resolves the absolute path to the `devcontainer` CLI binary using Warp's
/// process PATH. Prefer [`resolve_devcontainer_cli_path_from_user_shell`] for
/// user-triggered launches.
pub fn resolve_devcontainer_cli_path() -> Option<PathBuf> {
    resolve_executable(DEVCONTAINER_CLI_NAME).map(|p| p.into_owned())
}

/// Resolves `devcontainer` using the PATH captured from the user's interactive
/// login shell, matching how MCP servers, LSP, and Docker Sandbox resolve
/// user-installed CLIs.
#[cfg(feature = "local_tty")]
pub fn resolve_devcontainer_cli_path_from_user_shell(
    ctx: &mut AppContext,
) -> BoxFuture<'static, Option<PathBuf>> {
    let path_future = LocalShellState::handle(ctx).update(ctx, |shell_state, ctx| {
        shell_state.get_interactive_path_env_var(ctx)
    });
    async move {
        let path_env_var = path_future.await;
        let resolved = match path_env_var.as_deref() {
            Some(path) => resolve_executable_in_path(DEVCONTAINER_CLI_NAME, OsStr::new(path)),
            None => resolve_executable(DEVCONTAINER_CLI_NAME),
        };
        resolved.map(|p| p.into_owned())
    }
    .boxed()
}

/// Finds devcontainer configs for the nearest ancestor workspace containing a
/// `.devcontainer` config.
pub fn find_nearest_devcontainer_configs(start_path: &Path) -> Vec<DevContainerConfig> {
    let start_path = if start_path.is_file() {
        start_path.parent().unwrap_or(start_path)
    } else {
        start_path
    };

    for ancestor in start_path.ancestors() {
        let configs = find_devcontainer_configs_for_workspace(ancestor);
        if !configs.is_empty() {
            return configs;
        }
    }
    Vec::new()
}

/// Finds devcontainer configs directly under a workspace, supporting both:
/// - `.devcontainer/devcontainer.json`
/// - `.devcontainer.json`
/// - `.devcontainer/<name>/devcontainer.json`
pub fn find_devcontainer_configs_for_workspace(workspace_folder: &Path) -> Vec<DevContainerConfig> {
    let devcontainer_dir = workspace_folder.join(DEVCONTAINER_DIR);
    let mut configs = Vec::new();

    let default_config = devcontainer_dir.join(DEVCONTAINER_FILE);
    if default_config.is_file() {
        configs.push(config_from_path(workspace_folder, default_config));
    }

    let root_config = workspace_folder.join(ROOT_DEVCONTAINER_FILE);
    if root_config.is_file() {
        configs.push(config_from_path(workspace_folder, root_config));
    }

    if let Ok(entries) = std::fs::read_dir(&devcontainer_dir) {
        let mut named_configs = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .map(|path| path.join(DEVCONTAINER_FILE))
            .filter(|path| path.is_file())
            .map(|path| config_from_path(workspace_folder, path))
            .collect::<Vec<_>>();

        named_configs.sort_by(|a, b| a.config_path.cmp(&b.config_path));
        configs.extend(named_configs);
    }

    configs
}

fn config_from_path(workspace_folder: &Path, config_path: PathBuf) -> DevContainerConfig {
    DevContainerConfig {
        workspace_folder: workspace_folder.to_path_buf(),
        name: read_devcontainer_name(&config_path),
        config_path,
    }
}

fn read_devcontainer_name(config_path: &Path) -> Option<String> {
    let config = std::fs::read_to_string(config_path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&config).ok()?;
    json.get("name")
        .and_then(|name| name.as_str())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
}

/// Wraps a [`DirectShellStarter`] and adds Dev-Container-specific parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DevContainerShellStarter {
    pub direct: DirectShellStarter,
    pub workspace_folder: PathBuf,
    pub config_path: PathBuf,
}

impl DevContainerShellStarter {
    pub fn new(
        direct: DirectShellStarter,
        workspace_folder: PathBuf,
        config_path: PathBuf,
    ) -> Self {
        Self {
            direct,
            workspace_folder,
            config_path,
        }
    }

    pub fn shell_type(&self) -> ShellType {
        self.direct.shell_type()
    }

    pub fn logical_shell_path(&self) -> &Path {
        self.direct.logical_shell_path()
    }

    pub fn display_name(&self) -> &str {
        "Dev Container"
    }

    pub fn workspace_folder(&self) -> &Path {
        &self.workspace_folder
    }

    pub fn config_path(&self) -> &Path {
        &self.config_path
    }
}

pub(crate) fn devcontainer_container_shell_script(init_script: &str) -> String {
    const INIT_DELIMITER: &str = "__WARP_DEVCONTAINER_INIT__";
    format!(
        "tmp=\"${{TMPDIR:-/tmp}}/warp-devcontainer-init-$$.sh\"\n\
         cat > \"$tmp\" <<'{INIT_DELIMITER}'\n\
         {init_script}\n\
         {INIT_DELIMITER}\n\
         exec bash --rcfile \"$tmp\" --noprofile\n"
    )
}

pub(crate) fn devcontainer_host_command_script(
    devcontainer_cli_path: &Path,
    workspace_folder: &Path,
    config_path: &Path,
    init_script: &str,
) -> String {
    let quote_path = |path: &Path| {
        let path = path.to_string_lossy();
        shell_words::quote(&path).into_owned()
    };
    let cli = quote_path(devcontainer_cli_path);
    let workspace_folder = quote_path(workspace_folder);
    let config_path = quote_path(config_path);
    let container_script =
        shell_words::quote(&devcontainer_container_shell_script(init_script)).into_owned();

    format!(
        "set -e\n\
         {cli} up --workspace-folder {workspace_folder} --config {config_path}\n\
         exec {cli} exec --workspace-folder {workspace_folder} --config {config_path} /bin/sh -lc {container_script}"
    )
}

#[cfg(test)]
#[path = "dev_container_tests.rs"]
mod tests;
