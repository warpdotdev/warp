use super::*;
use std::fs;
use std::path::Path;

#[test]
fn detects_default_devcontainer_config() {
    let temp_dir = tempfile::tempdir().unwrap();
    let workspace = temp_dir.path();
    let devcontainer_dir = workspace.join(".devcontainer");
    fs::create_dir_all(&devcontainer_dir).unwrap();
    fs::write(
        devcontainer_dir.join("devcontainer.json"),
        r#"{"name":"Default container"}"#,
    )
    .unwrap();

    let configs = find_devcontainer_configs_for_workspace(workspace);

    assert_eq!(configs.len(), 1);
    assert_eq!(configs[0].workspace_folder, workspace);
    assert_eq!(
        configs[0].config_path,
        workspace.join(".devcontainer/devcontainer.json")
    );
    assert_eq!(configs[0].display_name(), "Default container");
}

#[test]
fn detects_root_devcontainer_config() {
    let temp_dir = tempfile::tempdir().unwrap();
    let workspace = temp_dir.path();
    fs::write(
        workspace.join(".devcontainer.json"),
        r#"{"name":"Root container"}"#,
    )
    .unwrap();

    let configs = find_devcontainer_configs_for_workspace(workspace);

    assert_eq!(configs.len(), 1);
    assert_eq!(configs[0].workspace_folder, workspace);
    assert_eq!(configs[0].config_path, workspace.join(".devcontainer.json"));
    assert_eq!(configs[0].display_name(), "Root container");
}

#[test]
fn detects_named_devcontainer_configs_in_stable_order() {
    let temp_dir = tempfile::tempdir().unwrap();
    let workspace = temp_dir.path();
    fs::create_dir_all(workspace.join(".devcontainer/zeta")).unwrap();
    fs::create_dir_all(workspace.join(".devcontainer/alpha")).unwrap();
    fs::write(
        workspace.join(".devcontainer/zeta/devcontainer.json"),
        r#"{}"#,
    )
    .unwrap();
    fs::write(
        workspace.join(".devcontainer/alpha/devcontainer.json"),
        r#"{}"#,
    )
    .unwrap();

    let configs = find_devcontainer_configs_for_workspace(workspace);

    let names = configs
        .iter()
        .map(DevContainerConfig::display_name)
        .collect::<Vec<_>>();
    assert_eq!(names, vec!["alpha", "zeta"]);
}

#[test]
fn finds_nearest_devcontainer_config_from_nested_directory() {
    let temp_dir = tempfile::tempdir().unwrap();
    let workspace = temp_dir.path();
    fs::create_dir_all(workspace.join(".devcontainer")).unwrap();
    fs::create_dir_all(workspace.join("src/nested")).unwrap();
    fs::write(
        workspace.join(".devcontainer/devcontainer.json"),
        r#"{"name":"Project container"}"#,
    )
    .unwrap();

    let configs = find_nearest_devcontainer_configs(&workspace.join("src/nested"));

    assert_eq!(configs.len(), 1);
    assert_eq!(configs[0].workspace_folder, workspace);
    assert_eq!(configs[0].display_name(), "Project container");
}

#[test]
fn builds_devcontainer_host_command_with_quoted_paths() {
    let script = devcontainer_host_command_script(
        Path::new("/opt/dev container/bin/devcontainer"),
        Path::new("/tmp/work space"),
        Path::new("/tmp/work space/.devcontainer/devcontainer.json"),
        "echo warp-init",
    );

    assert!(script.contains("'/opt/dev container/bin/devcontainer'"));
    assert!(script.contains("up --workspace-folder '/tmp/work space' --config '/tmp/work space/.devcontainer/devcontainer.json'"));
    assert!(script.contains("exec '/opt/dev container/bin/devcontainer' exec"));
    assert!(script.contains("echo warp-init"));
    assert!(script.contains("exec bash --rcfile \"$tmp\" --noprofile"));
}
