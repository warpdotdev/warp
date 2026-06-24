use std::io::Write;
use std::path::Path;
use std::{fs, io};

use anyhow::{anyhow, Result};
use itertools::Itertools;
use repo_metadata::RepositoryUpdate;
use warpui::{ModelContext, ModelHandle, SingletonEntity};

use super::util::{
    for_each_dir_entry, has_name, is_config_file, parse_model_config_dir_entry,
    parse_multi_launch_config_dir_entry, parse_multi_workflow_dir_entry,
    parse_single_theme_dir_entry, parse_tab_config_dir_entry,
};
use super::{
    custom_model_routers_dir, launch_configs_dir, tab_configs_dir, themes_dir, workflows_dir,
    WarpConfigUpdateEvent, LAUNCH_CONFIG_COMMENT,
};
use crate::ai::custom_model_routers::{CustomModelRouter, ModelConfigError};
use crate::features::FeatureFlag;
use crate::launch_configs::launch_config::LaunchConfig;
use crate::tab_configs::{TabConfig, TabConfigError};
use crate::themes::theme::WarpThemeConfig;
use crate::warp_managed_paths_watcher::{
    repository_update_touches_path, repository_update_touches_prefix, WarpManagedPathsWatcher,
    WarpManagedPathsWatcherEvent,
};
use crate::workflows::workflow::Workflow;

impl super::WarpConfig {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        // Load launch configs, and workflows from disk asynchronously on a background
        // thread.
        //
        // Themes are required during initialization by `Settings`, so we load this synchronously
        // on startup. We should investigate the possibility of offloading theme loading to a
        // background thread in the future.
        let _ = ctx.spawn(
            async move { load_launch_configs(&launch_configs_dir()) },
            |me, launch_configs, ctx| {
                me.launch_configs = launch_configs;
                ctx.emit(WarpConfigUpdateEvent::LaunchConfigs);
            },
        );
        if FeatureFlag::TabConfigs.is_enabled() {
            let _ = ctx.spawn(
                async move { load_tab_configs(&tab_configs_dir()) },
                |me, (tab_configs, tab_config_errors), ctx| {
                    me.tab_configs = tab_configs;
                    me.tab_config_errors = tab_config_errors;
                    ctx.emit(WarpConfigUpdateEvent::TabConfigs);
                    // Don't emit TabConfigErrors on startup — the error toast
                    // should only appear when the user saves a config file,
                    // not on app restart.
                },
            );
        }
        let _ = ctx.spawn(
            async move { load_workflows(&workflows_dir()) },
            |me, user_workflows, ctx| {
                me.local_user_workflows = user_workflows;
                ctx.emit(WarpConfigUpdateEvent::LocalUserWorkflows);
            },
        );
        if FeatureFlag::CustomModelRouters.is_enabled() {
            let _ = ctx.spawn(
                async move { load_model_configs(&custom_model_routers_dir()) },
                |me, (models, errors), ctx| {
                    me.custom_model_routers = models;
                    me.custom_model_router_errors = errors;
                    ctx.emit(WarpConfigUpdateEvent::ModelConfigs);
                    // Don't emit ModelConfigErrors on startup — like tab configs,
                    // the error toast should only appear when the user saves a
                    // file, not on app restart.
                },
            );
        }
        ctx.subscribe_to_model(
            &WarpManagedPathsWatcher::handle(ctx),
            Self::handle_warp_managed_paths_event,
        );

        Self {
            theme_config: load_theme_configs(&themes_dir()),
            ..Default::default()
        }
    }

    fn handle_warp_managed_paths_event(
        &mut self,
        _: ModelHandle<WarpManagedPathsWatcher>,
        event: &WarpManagedPathsWatcherEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        let WarpManagedPathsWatcherEvent::FilesChanged(update) = event;

        if update_touches_dir(update, &themes_dir()) {
            let theme_dir = themes_dir();
            let _ = ctx.spawn(
                async move { load_theme_configs(&theme_dir) },
                |me, theme_config, ctx| {
                    me.theme_config = theme_config;
                    ctx.emit(WarpConfigUpdateEvent::Themes);
                },
            );
        }

        if update_touches_dir(update, &workflows_dir()) {
            let workflow_dir = workflows_dir();
            let _ = ctx.spawn(
                async move { load_workflows(&workflow_dir) },
                |me, workflows, ctx| {
                    me.local_user_workflows = workflows;
                    ctx.emit(WarpConfigUpdateEvent::LocalUserWorkflows);
                },
            );
        }

        if update_touches_dir(update, &launch_configs_dir()) {
            let launch_config_dir = launch_configs_dir();
            let _ = ctx.spawn(
                async move { load_launch_configs(&launch_config_dir) },
                |me, launch_configs, ctx| {
                    me.launch_configs = launch_configs;
                    ctx.emit(WarpConfigUpdateEvent::LaunchConfigs);
                },
            );
        }

        if FeatureFlag::TabConfigs.is_enabled() && update_touches_dir(update, &tab_configs_dir()) {
            let tab_config_dir = tab_configs_dir();
            let _ = ctx.spawn(
                async move { load_tab_configs(&tab_config_dir) },
                |me, (configs, errors), ctx| {
                    me.tab_configs = configs;
                    me.tab_config_errors = errors.clone();
                    ctx.emit(WarpConfigUpdateEvent::TabConfigs);
                    if !errors.is_empty() {
                        ctx.emit(WarpConfigUpdateEvent::TabConfigErrors(errors));
                    }
                },
            );
        }

        if FeatureFlag::CustomModelRouters.is_enabled()
            && update_touches_dir(update, &custom_model_routers_dir())
        {
            let dir_path = custom_model_routers_dir();
            let _ = ctx.spawn(
                async move { load_model_configs(&dir_path) },
                |me, (models, errors), ctx| {
                    me.custom_model_routers = models;
                    me.custom_model_router_errors = errors.clone();
                    ctx.emit(WarpConfigUpdateEvent::ModelConfigs);
                    if !errors.is_empty() {
                        ctx.emit(WarpConfigUpdateEvent::ModelConfigErrors(errors));
                    }
                },
            );
        }

        if FeatureFlag::SettingsFile.is_enabled()
            && update_touches_path(update, &crate::settings::user_preferences_toml_file_path())
        {
            ctx.emit(WarpConfigUpdateEvent::Settings);
        }
    }

    /// This method takes a file name candidate (appends .yaml if missing) and a LaunchConfig as
    /// arguments. It saves the file and returns the filename used if successful.
    #[cfg(feature = "local_fs")]
    pub fn save_new_launch_config(
        file_name: String,
        launch_config: LaunchConfig,
    ) -> Result<String> {
        let file_name = if is_config_file(&file_name) {
            file_name.trim().into()
        } else {
            format!("{file_name}.yaml")
        };

        if !has_name(file_name.trim()) {
            return Err(anyhow!("File name is empty"));
        };

        let path = crate::user_config::launch_configs_dir().join(&file_name);
        if path.exists() {
            return Err(anyhow!("File already exists"));
        };

        let file = crate::util::file::create_file(path)?;
        let mut writer = io::BufWriter::new(file);
        writer.write_all(LAUNCH_CONFIG_COMMENT.as_bytes())?;
        serde_yaml::to_writer(writer, &launch_config)?;
        Ok(file_name)
    }
}

pub fn load_theme_configs(theme_path: &Path) -> WarpThemeConfig {
    let mut theme_configs = WarpThemeConfig::new();
    for_each_dir_entry(theme_path, parse_single_theme_dir_entry)
        .into_iter()
        .for_each(|(theme_name, theme)| theme_configs.add_new_theme(theme_name, theme));
    theme_configs
}

/// Loads all workflows relative to the `workflow_path`.  A YAML file might
/// contain multiple workflows.
pub fn load_workflows(workflow_path: &Path) -> Vec<Workflow> {
    for_each_dir_entry(workflow_path, parse_multi_workflow_dir_entry)
        .into_iter()
        .flatten()
        .collect_vec()
}

/// Loads all launch configs relative to the `launch_config_path`. Each workflow is assumed to be in an
/// individual YAML file.
pub fn load_launch_configs(launch_config_path: &Path) -> Vec<LaunchConfig> {
    for_each_dir_entry(launch_config_path, parse_multi_launch_config_dir_entry)
        .into_iter()
        .flatten()
        .collect_vec()
}

/// Loads custom model routers from the config directory at `dir_path`
/// (`~/.warp/custom_model_routers/`), where each file defines a single router.
/// Returns the parsed routers (sorted by display name) and any per-file
/// parse/validation errors. If the directory does not exist, returns empty vecs.
pub fn load_model_configs(dir_path: &Path) -> (Vec<CustomModelRouter>, Vec<ModelConfigError>) {
    let results = for_each_dir_entry(dir_path, parse_model_config_dir_entry);
    let mut models = Vec::new();
    let mut errors = Vec::new();
    for result in results {
        match result {
            Ok(model) => models.push(model),
            Err(error) => errors.push(error),
        }
    }
    models.sort_by(|a, b| {
        let a_name = a.info.display_name.to_lowercase();
        let b_name = b.info.display_name.to_lowercase();
        a_name
            .cmp(&b_name)
            .then_with(|| a.info.display_name.cmp(&b.info.display_name))
    });
    (models, errors)
}

/// Loads all tab configs from `tab_config_path`. Each tab config is an individual TOML file.
///
/// Returns successfully parsed configs and any errors for files that failed to parse.
pub fn load_tab_configs(tab_config_path: &Path) -> (Vec<TabConfig>, Vec<TabConfigError>) {
    let results = for_each_dir_entry(tab_config_path, parse_tab_config_dir_entry);
    let mut configs = Vec::new();
    let mut errors = Vec::new();
    for result in results {
        match result {
            Ok(config) => configs.push(config),
            Err(error) => errors.push(error),
        }
    }
    configs.sort_by(|a, b| {
        let a_name = a.name.to_lowercase();
        let b_name = b.name.to_lowercase();
        a_name.cmp(&b_name).then_with(|| a.name.cmp(&b.name))
    });
    (configs, errors)
}

fn update_touches_dir(update: &RepositoryUpdate, path: &Path) -> bool {
    let canonical_path = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    repository_update_touches_prefix(update, path)
        || repository_update_touches_prefix(update, &canonical_path)
}

fn update_touches_path(update: &RepositoryUpdate, path: &Path) -> bool {
    let canonical_path = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    repository_update_touches_path(update, path)
        || repository_update_touches_path(update, &canonical_path)
}
