pub mod util;

#[cfg_attr(not(target_family = "wasm"), path = "native.rs")]
#[cfg_attr(target_family = "wasm", path = "wasm.rs")]
mod imp;

use crate::tab_configs::{TabConfig, TabConfigError};
use crate::themes::theme::WarpThemeConfig;
use crate::{
    launch_configs::launch_config::LaunchConfig, themes::theme::ThemeKind,
    workflows::workflow::Workflow,
};
use lazy_static::lazy_static;
#[cfg(feature = "local_fs")]
use std::path::Path;
use std::path::PathBuf;
use warp_core::ui::theme::WarpTheme;
use warpui::{Entity, ModelContext, SingletonEntity};

pub(crate) use imp::load_tab_configs;
#[cfg(feature = "local_fs")]
pub use imp::load_workflows;
pub use imp::{load_launch_configs, load_theme_configs};

lazy_static! {
    pub static ref LAUNCH_CONFIG_COMMENT: String = format!(
        "# Warp Launch Configuration
#
#
# Use this to start a certain configuration of windows, tabs, and panes.
# Open the launch configuration palette to access and open any launch configuration.
#
# This file defines your launch configuration.
# More on how to do so here:
# https://docs.warp.dev/terminal/sessions/launch-configurations
#
# All launch configurations are stored under {}.
# Edit them anytime!
#
# You can also add commands that run on-start for your launch configurations like so:
# ---
# name: Example with Command
# windows:
#  - tabs:
#      - layout:
#          cwd: /Users/warp-user/project
#          commands:
#            - exec: code .
",
        warp_core::paths::home_relative_path(&crate::user_config::launch_configs_dir())
    );
}

#[derive(Clone)]
pub enum WarpConfigUpdateEvent {
    Themes,
    #[cfg_attr(not(feature = "local_fs"), expect(dead_code))]
    LocalUserWorkflows,
    LaunchConfigs,
    #[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
    TabConfigs,
    /// Emitted when one or more tab config files failed to parse.
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    TabConfigErrors(Vec<TabConfigError>),
    /// The settings file (`settings.toml`) was created, modified, or deleted.
    #[cfg_attr(not(feature = "local_fs"), expect(dead_code))]
    Settings,
    /// One or more settings in `settings.toml` could not be loaded.
    #[cfg_attr(not(feature = "local_fs"), expect(dead_code))]
    SettingsErrors(crate::settings::SettingsFileError),
    /// A previously-errored settings reload succeeded with no errors.
    #[cfg_attr(not(feature = "local_fs"), expect(dead_code))]
    SettingsErrorsCleared,
}

/// Singleton model containing user configurable file entities like themes, launch configs, and
/// workflows.
///
/// Emits events when entities are changed, which are detected via filesystem
/// watchers on the user's `data_dir()` (themes, workflows, launch configs,
/// tab configs, etc.) and, on platforms where it differs, `config_local_dir()`
/// (`settings.toml`, `keybindings.yaml`, `user_preferences.json`).
#[derive(Default)]
pub struct WarpConfig {
    launch_configs: Vec<LaunchConfig>,
    tab_configs: Vec<TabConfig>,
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    tab_config_errors: Vec<TabConfigError>,
    theme_config: WarpThemeConfig,
    local_user_workflows: Vec<Workflow>,
}

/// Platform-independent parts of WarpConfig.
///
/// Additional platform-dependent functionality can be found in impl blocks
/// in native.rs and wasm.rs.
impl WarpConfig {
    #[cfg(test)]
    pub fn mock(_ctx: &mut ModelContext<Self>) -> Self {
        Self {
            theme_config: WarpThemeConfig::new(),
            ..Default::default()
        }
    }

    pub fn launch_configs(&self) -> &Vec<LaunchConfig> {
        &self.launch_configs
    }

    pub fn tab_configs(&self) -> &Vec<TabConfig> {
        &self.tab_configs
    }

    pub fn theme_config(&self) -> &WarpThemeConfig {
        &self.theme_config
    }

    pub fn local_user_workflows(&self) -> &Vec<Workflow> {
        &self.local_user_workflows
    }

    /// Saving the newly created launch configuration to the WarpConfig that we currently
    /// have.
    pub fn append_launch_config(
        &mut self,
        launch_config: &LaunchConfig,
        ctx: &mut ModelContext<Self>,
    ) {
        if !self.launch_configs.contains(launch_config) {
            self.launch_configs.push(launch_config.to_owned());
            ctx.emit(WarpConfigUpdateEvent::LaunchConfigs);
        }
    }

    pub fn update_theme_config(
        &mut self,
        theme_config: WarpThemeConfig,
        ctx: &mut ModelContext<Self>,
    ) {
        self.theme_config = theme_config;
        ctx.emit(WarpConfigUpdateEvent::Themes);
    }

    pub fn add_new_theme_to_config(
        &mut self,
        theme_name: ThemeKind,
        theme: WarpTheme,
        ctx: &mut ModelContext<Self>,
    ) {
        self.theme_config.add_new_theme(theme_name, theme);
        ctx.emit(WarpConfigUpdateEvent::Themes);
    }

    /// Eagerly removes a tab config by its source path and emits a `TabConfigs` event.
    /// (Used after deleting the file on disk so the menu updates immediately
    /// rather than waiting for the filesystem watcher.)
    #[cfg(feature = "local_fs")]
    pub fn remove_tab_config_by_path(&mut self, path: &Path, ctx: &mut ModelContext<Self>) {
        let before = self.tab_configs.len();
        self.tab_configs
            .retain(|c| c.source_path.as_deref() != Some(path));
        if self.tab_configs.len() != before {
            ctx.emit(WarpConfigUpdateEvent::TabConfigs);
        }
    }
}

/// Returns the base directory in which all of the user's data is stored.
fn base_dir() -> PathBuf {
    warp_core::paths::data_dir()
}

/// Returns the path to the directory containing the user's custom themes.
pub fn themes_dir() -> PathBuf {
    warp_core::paths::themes_dir()
}

/// Returns the path to the directory containing the user's custom workflows.
#[cfg_attr(target_family = "wasm", expect(dead_code))]
pub fn workflows_dir() -> PathBuf {
    crate::workflows::local_workflows::workflows_dir(base_dir())
}

/// Returns the path to the directory containing the user's launch
/// configurations.
pub fn launch_configs_dir() -> PathBuf {
    base_dir().join("launch_configurations")
}

/// Returns the path to the directory containing the user's tab configs.
pub fn tab_configs_dir() -> PathBuf {
    base_dir().join("tab_configs")
}

/// Returns the path to the directory containing the built-in default tab configs.
/// These are shipped with Warp and user-editable (Warp does not overwrite modifications).
#[cfg_attr(target_family = "wasm", expect(dead_code))]
pub fn default_tab_configs_dir() -> PathBuf {
    base_dir().join("default_tab_configs")
}

/// Returns whether the path points to a tab config TOML file under one of Warp's
/// tab config directories.
#[cfg(feature = "local_fs")]
pub fn is_tab_config_toml(path: &Path) -> bool {
    let is_toml = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension == "toml");
    if !is_toml {
        return false;
    }

    [tab_configs_dir(), default_tab_configs_dir()]
        .into_iter()
        .any(|dir| path.starts_with(dir))
}

/// Ensures `~/.warp/default_tab_configs/worktree.toml` exists, creating it
/// from the embedded template if missing. Returns the path to the file.
#[cfg(feature = "local_fs")]
pub(crate) fn ensure_default_worktree_config() -> PathBuf {
    let dir = default_tab_configs_dir();
    let path = dir.join("worktree.toml");
    if !path.exists() {
        log::info!("Default worktree config missing; creating at {path:?}");
        if let Err(e) = std::fs::create_dir_all(&dir) {
            log::warn!("Failed to create default_tab_configs dir at {dir:?}: {e:?}");
            return path;
        }
        const TEMPLATE: &str = include_str!("../../resources/tab_configs/default_worktree.toml");
        if let Err(e) = std::fs::write(&path, TEMPLATE) {
            log::warn!("Failed to write default worktree config at {path:?}: {e:?}");
        } else {
            log::info!("Default worktree config created at {path:?}");
        }
    } else {
        log::info!("Default worktree config already exists at {path:?}");
    }
    path
}

#[cfg(feature = "local_fs")]
pub(crate) fn materialize_default_worktree_config(
    template_toml: &str,
    config_name: &str,
    repo_path: &str,
    pane_type: &str,
) -> Result<(String, TabConfig), String> {
    let worktree_path = crate::tab_configs::tab_config::generated_worktree_path_string(
        Path::new(repo_path),
        "{{autogenerated_branch_name}}",
    );
    let mut toml_value = toml::from_str::<toml::Value>(template_toml)
        .map_err(|e| format!("failed to parse default worktree template: {e:?}"))?;

    if let Some(doc) = toml_value.as_table_mut() {
        doc.insert(
            "name".to_string(),
            toml::Value::String(config_name.to_string()),
        );
    }

    replace_default_worktree_placeholders(&mut toml_value, repo_path, pane_type, &worktree_path);

    if let Some(doc) = toml_value.as_table_mut() {
        if let Some(params) = doc.get_mut("params").and_then(toml::Value::as_table_mut) {
            params.remove("repo");
            params.remove("pane_type");
            if params.is_empty() {
                doc.remove("params");
            }
        }
    }

    let toml_content = toml::to_string_pretty(&toml_value)
        .map_err(|e| format!("failed to serialize default worktree config: {e:?}"))?;
    let tab_config = toml::from_str::<TabConfig>(&toml_content)
        .map_err(|e| format!("failed to parse materialized worktree config: {e:?}"))?;

    Ok((toml_content, tab_config))
}

#[cfg(feature = "local_fs")]
fn replace_default_worktree_placeholders(
    value: &mut toml::Value,
    repo_path: &str,
    pane_type: &str,
    worktree_path: &str,
) {
    match value {
        toml::Value::String(string) => {
            *string = string
                .replace(
                    "{{worktree_path_prefix}}{{autogenerated_branch_name}}",
                    worktree_path,
                )
                .replace("{{repo}}", repo_path)
                .replace("{{pane_type}}", pane_type)
                .replace("{{worktree_path_prefix}}", "");
        }
        toml::Value::Array(array) => {
            for value in array {
                replace_default_worktree_placeholders(value, repo_path, pane_type, worktree_path);
            }
        }
        toml::Value::Table(table) => {
            for (_, value) in table.iter_mut() {
                replace_default_worktree_placeholders(value, repo_path, pane_type, worktree_path);
            }
        }
        toml::Value::Boolean(_)
        | toml::Value::Datetime(_)
        | toml::Value::Float(_)
        | toml::Value::Integer(_) => {}
    }
}

/// Returns a path for a new tab config file that does not yet exist in `dir`.
/// Tries `my_tab_config.toml`, then `my_tab_config_1.toml`, `my_tab_config_2.toml`, etc.
#[cfg(feature = "local_fs")]
pub(crate) fn find_unused_tab_config_path(dir: &Path) -> PathBuf {
    find_unused_toml_path(dir, "my_tab_config")
}

/// Returns a `.toml` path in `dir` that does not yet exist.
///
/// Tries `{base_name}.toml`, then `{base_name}_1.toml`, `{base_name}_2.toml`, etc.
#[cfg(feature = "local_fs")]
pub(crate) fn find_unused_toml_path(dir: &Path, base_name: &str) -> PathBuf {
    let base = dir.join(format!("{base_name}.toml"));
    if !base.exists() {
        return base;
    }
    let mut n = 1u32;
    loop {
        let candidate = dir.join(format!("{base_name}_{n}.toml"));
        if !candidate.exists() {
            return candidate;
        }
        n = n.saturating_add(1);
    }
}

/// Sanitizes a suggested TOML filename base into a lowercase ASCII-ish stem.
///
/// Preserves ASCII letters, digits, hyphens, and underscores. All other
/// characters are replaced with underscores, repeated underscores are collapsed,
/// and leading/trailing underscores are removed.
#[cfg(feature = "local_fs")]
pub(crate) fn sanitize_toml_base_name(base_name: &str) -> String {
    let mut sanitized = String::with_capacity(base_name.len());
    let mut last_was_underscore = false;

    for c in base_name.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
            sanitized.push(c);
            last_was_underscore = c == '_';
        } else if !last_was_underscore && !sanitized.is_empty() {
            sanitized.push('_');
            last_was_underscore = true;
        }
    }

    sanitized = sanitized.trim_matches('_').to_string();
    if sanitized.is_empty() {
        "worktree".to_string()
    } else {
        sanitized
    }
}

/// Returns a path for a new worktree tab config that does not yet exist in `dir`.
/// Uses the branch name to create a descriptive filename like `worktree_my-branch.toml`.
///
/// The caller is expected to pass a branch name that has already been validated
/// (alphanumeric, hyphens, underscores only) so no sanitization is performed here.
#[cfg(feature = "local_fs")]
pub(crate) fn find_unused_worktree_config_path(dir: &Path, branch_name: &str) -> PathBuf {
    let base = dir.join(format!("worktree_{branch_name}.toml"));
    if !base.exists() {
        return base;
    }
    let mut n = 1u32;
    loop {
        let candidate = dir.join(format!("worktree_{branch_name}_{n}.toml"));
        if !candidate.exists() {
            return candidate;
        }
        n = n.saturating_add(1);
    }
}

impl Entity for WarpConfig {
    type Event = WarpConfigUpdateEvent;
}

impl SingletonEntity for WarpConfig {}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
