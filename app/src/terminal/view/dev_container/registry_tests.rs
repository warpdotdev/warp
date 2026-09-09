use std::path::PathBuf;

use super::DevContainerBuildKey;

fn key(workspace: &str, config: &str) -> DevContainerBuildKey {
    DevContainerBuildKey {
        workspace_folder: PathBuf::from(workspace),
        config_file: PathBuf::from(config),
    }
}

#[test]
fn keys_distinguish_workspace_and_config() {
    let a = key("/tmp/a", "/tmp/a/.devcontainer/devcontainer.json");
    let b = key("/tmp/b", "/tmp/b/.devcontainer/devcontainer.json");
    let a_other_config = key("/tmp/a", "/tmp/a/.devcontainer/web/devcontainer.json");
    assert_ne!(a, b);
    assert_ne!(a, a_other_config);
    assert_eq!(a, key("/tmp/a", "/tmp/a/.devcontainer/devcontainer.json"));
}
