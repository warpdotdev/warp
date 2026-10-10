use std::collections::HashMap;
use std::path::PathBuf;

use super::{
    LAUNCH_CONFIG_BINDING_PREFIX, TAB_CONFIG_BINDING_PREFIX, launch_config_bindings,
    tab_config_bindings,
};
use crate::launch_configs::launch_config::LaunchConfig;
use crate::tab_configs::TabConfig;

fn launch_config(name: &str) -> LaunchConfig {
    LaunchConfig {
        name: name.to_string(),
        active_window_index: None,
        windows: Vec::new(),
    }
}

fn tab_config(name: &str, source_path: Option<&str>) -> TabConfig {
    TabConfig {
        name: name.to_string(),
        title: None,
        color: None,
        panes: Vec::new(),
        params: HashMap::new(),
        source_path: source_path.map(PathBuf::from),
    }
}

#[test]
fn test_launch_config_binding_names_use_config_name() {
    let bindings = launch_config_bindings(&[launch_config("backend"), launch_config("My App")]);
    let names: Vec<String> = bindings
        .iter()
        .map(|binding| binding.name().to_string())
        .collect();
    assert_eq!(
        names,
        vec![
            format!("{LAUNCH_CONFIG_BINDING_PREFIX}backend"),
            format!("{LAUNCH_CONFIG_BINDING_PREFIX}My App"),
        ]
    );
}

#[test]
fn test_duplicate_launch_config_names_keep_first() {
    let bindings = launch_config_bindings(&[launch_config("Backend"), launch_config("backend")]);
    assert_eq!(bindings.len(), 1);
    assert_eq!(
        bindings[0].name(),
        format!("{LAUNCH_CONFIG_BINDING_PREFIX}Backend")
    );
}

#[test]
fn test_tab_config_binding_names_use_file_stem() {
    let bindings = tab_config_bindings(&[tab_config(
        "Frontend",
        Some("/tmp/tab_configs/frontend.toml"),
    )]);
    assert_eq!(bindings.len(), 1);
    assert_eq!(
        bindings[0].name(),
        format!("{TAB_CONFIG_BINDING_PREFIX}frontend")
    );
}

#[test]
fn test_tab_config_without_source_path_is_skipped() {
    let bindings = tab_config_bindings(&[
        tab_config("unsaved", None),
        tab_config("saved", Some("/x/saved.toml")),
    ]);
    assert_eq!(bindings.len(), 1);
    assert_eq!(
        bindings[0].name(),
        format!("{TAB_CONFIG_BINDING_PREFIX}saved")
    );
}

#[test]
fn test_duplicate_tab_config_stems_keep_first() {
    let bindings = tab_config_bindings(&[
        tab_config("first", Some("/a/Repo.toml")),
        tab_config("second", Some("/b/repo.toml")),
    ]);
    assert_eq!(bindings.len(), 1);
    assert_eq!(
        bindings[0].name(),
        format!("{TAB_CONFIG_BINDING_PREFIX}Repo")
    );
}
