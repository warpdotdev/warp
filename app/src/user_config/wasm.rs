use std::path::Path;

use warpui::ModelContext;

use crate::launch_configs::launch_config::LaunchConfig;
use crate::tab_configs::{TabConfig, TabConfigError};
use crate::themes::theme::WarpThemeConfig;
use crate::workflows::workflow::Workflow;

impl super::WarpConfig {
    pub fn new(_ctx: &mut ModelContext<Self>) -> Self {
        Self {
            launch_configs: Default::default(),
            tab_configs: Default::default(),
            tab_config_errors: Default::default(),
            theme_config: WarpThemeConfig::new(),
            local_user_workflows: Default::default(),
        }
    }
}

/// Loads all themes relative to the `workflow_path`.
pub fn load_theme_configs(_theme_path: &Path) -> WarpThemeConfig {
    // There's no local filesystem for wasm, so we'll never be able to retrieve
    // themes from any path.
    Default::default()
}

/// Loads all workflows relative to the `workflow_path`.
#[cfg_attr(target_family = "wasm", allow(dead_code))]
pub fn load_workflows(_workflow_path: &Path) -> Vec<Workflow> {
    // There's no local filesystem for wasm, so we'll never be able to retrieve
    // workflows from any path.
    Default::default()
}

/// Loads all launch configs relative to the `launch_config_path`.
pub fn load_launch_configs(_launch_config_path: &Path) -> Vec<LaunchConfig> {
    // There's no local filesystem for wasm, so we'll never be able to retrieve
    // launch configs from any path.
    Default::default()
}

/// Loads all tab configs relative to the `tab_config_path`.
pub(crate) fn load_tab_configs(_tab_config_path: &Path) -> (Vec<TabConfig>, Vec<TabConfigError>) {
    // There's no local filesystem for wasm, so we'll never be able to retrieve
    // tab configs from any path.
    Default::default()
}
