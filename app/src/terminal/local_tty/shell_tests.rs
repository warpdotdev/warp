use super::*;
use crate::terminal::available_shells::AvailableShell;
use std::path::PathBuf;

#[test]
fn test_program_invalid_bash() {
    // This test assumes there is no bash binary at /some/weird/path/bash.
    let shell_path = "/some/weird/path/bash".to_owned();
    assert!(supported_shell_path_and_type(&shell_path).is_none());
}

#[test]
fn test_dev_container_shell_starter_uses_bash_and_devcontainer_cli() {
    let devcontainer_cli_path = PathBuf::from("/usr/local/bin/devcontainer");
    let workspace_folder = PathBuf::from("/workspace/project");
    let config_path = workspace_folder.join(".devcontainer/devcontainer.json");
    let shell = AvailableShell::new_dev_container_shell(
        devcontainer_cli_path.clone(),
        workspace_folder.clone(),
        config_path.clone(),
    );

    let starter = ShellStarter::init(shell).expect("should create dev container shell starter");

    match starter {
        ShellStarterSourceOrWslName::Source(ShellStarterSource::Override(
            ShellStarter::DevContainer(starter),
        )) => {
            assert_eq!(starter.logical_shell_path(), devcontainer_cli_path);
            assert_eq!(starter.workspace_folder(), workspace_folder);
            assert_eq!(starter.config_path(), config_path);
            assert_eq!(starter.shell_type(), ShellType::Bash);
        }
        _ => panic!("expected Dev Container shell starter"),
    }
}

#[test]
fn test_program_invalid_zsh() {
    // This test assumes there is no bash zsh at /some/weird/path/bash.
    let shell_path = "/some/weird/path/zsh".to_owned();
    assert!(supported_shell_path_and_type(&shell_path).is_none());
}

#[test]
fn test_program_unknown_shell() {
    let shell_path = "/some/weird/path/wtfsh".to_owned();
    assert!(supported_shell_path_and_type(&shell_path).is_none());
}

#[test]
fn test_trim_wsl_err_from_output() {
    assert_eq!(
        take_until_utf16_crlf(b"/bin/bash\n".to_vec()),
        b"/bin/bash\n".to_vec()
    );
    assert_eq!(
        take_until_utf16_crlf(b"/bin/bash\n\r\0\n\0W\0A\0R\0N\0I\0N\0G\0".to_vec()),
        b"/bin/bash\n".to_vec()
    );
}
