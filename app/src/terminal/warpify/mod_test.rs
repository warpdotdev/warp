use channel_versions::overrides::TargetOS;

use crate::terminal::model::terminal_model::SubshellInitializationInfo;
use crate::terminal::shell::ShellType;

use super::subshell_bootstrap_success_block_bytes;

fn subshell_info() -> SubshellInitializationInfo {
    SubshellInitializationInfo {
        spawning_command: "bash".to_owned(),
        was_triggered_by_rc_file_snippet: false,
        env_var_collection_name: None,
        ssh_connection_info: None,
    }
}

#[test]
fn bash_auto_warpify_command_resolves_rc_symlinks_before_appending() {
    let (command, is_executable) = subshell_bootstrap_success_block_bytes(
        &subshell_info(),
        ShellType::Bash,
        TargetOS::Linux,
        false,
    );
    let command = String::from_utf8(command).expect("command should be valid UTF-8");

    assert!(is_executable);
    assert!(command.contains("rcfile=~/.bashrc"));
    assert!(command.contains("readlink -f \"$rcfile\""));
    assert!(command.contains("realpath \"$rcfile\""));
    assert!(command.contains(">> \"$target\""));
    assert!(command.contains("Auto-Warpify failed to update $target"));
    assert!(command.contains("\"shell\": \"bash\""));
    assert!(!command.contains("~\\.bashrc"));
    assert!(!command.contains(">> ~/.bashrc"));
}
