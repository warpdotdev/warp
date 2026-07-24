use super::*;

fn shell_starter(shell_type: ShellType, shell_path: &str) -> DirectShellStarter {
    DirectShellStarter::new_for_test(shell_type, PathBuf::from(shell_path), Vec::new())
}

fn env_value(command: &Command, key: &str) -> Option<Option<String>> {
    command
        .get_envs()
        .find(|(env_key, _)| *env_key == std::ffi::OsStr::new(key))
        .map(|(_, value)| value.map(|value| value.to_string_lossy().into_owned()))
}

#[test]
fn host_bash_command_sets_history_size_sentinels() {
    let command = build_host_shell_command(
        shell_starter(ShellType::Bash, "/bin/bash"),
        None,
        HashMap::new(),
        None,
        false,
        false,
        false,
        false,
        true,
    );

    assert_eq!(
        env_value(&command, "HISTFILESIZE"),
        Some(Some(BASH_HISTORY_SIZE_SENTINEL.to_owned()))
    );
    assert_eq!(
        env_value(&command, "HISTSIZE"),
        Some(Some(BASH_HISTORY_SIZE_SENTINEL.to_owned()))
    );
    assert_eq!(
        env_value(&command, "WARP_INITIAL_HISTFILESIZE"),
        Some(Some(BASH_HISTORY_SIZE_SENTINEL.to_owned()))
    );
    assert_eq!(
        env_value(&command, "WARP_INITIAL_HISTSIZE"),
        Some(Some(BASH_HISTORY_SIZE_SENTINEL.to_owned()))
    );
}

#[test]
#[serial_test::serial]
fn host_command_strips_leaked_warp_loader_env() {
    // Simulate the Linux launcher's leaked `LD_LIBRARY_PATH` (Warp's bundled
    // libs prepended) plus the backup of the user's original value, then
    // confirm the shell we build restores the user's value and drops both
    // Warp's entry and the backup var — i.e. a spawned shell no longer inherits
    // Warp's bundled-library path (warp#12228).
    let prev_ldlp = std::env::var_os("LD_LIBRARY_PATH");
    let prev_backup = std::env::var_os("WARP_ORIGINAL_LD_LIBRARY_PATH");

    std::env::set_var(
        "LD_LIBRARY_PATH",
        "/opt/warpdotdev/warp-terminal/lib:/home/demo/userlib",
    );
    std::env::set_var("WARP_ORIGINAL_LD_LIBRARY_PATH", "/home/demo/userlib");

    let command = build_host_shell_command(
        shell_starter(ShellType::Bash, "/bin/bash"),
        None,
        HashMap::new(),
        None,
        false,
        false,
        false,
        false,
        true,
    );

    // The spawned shell gets the user's original value, not Warp's bundled dir.
    assert_eq!(
        env_value(&command, "LD_LIBRARY_PATH"),
        Some(Some("/home/demo/userlib".to_owned()))
    );
    // And the launcher backup var is not leaked into the shell.
    assert_eq!(
        env_value(&command, "WARP_ORIGINAL_LD_LIBRARY_PATH"),
        Some(None)
    );

    // Restore the ambient environment for other tests.
    match prev_ldlp {
        Some(value) => std::env::set_var("LD_LIBRARY_PATH", value),
        None => std::env::remove_var("LD_LIBRARY_PATH"),
    }
    match prev_backup {
        Some(value) => std::env::set_var("WARP_ORIGINAL_LD_LIBRARY_PATH", value),
        None => std::env::remove_var("WARP_ORIGINAL_LD_LIBRARY_PATH"),
    }
}

#[test]
fn host_non_bash_command_does_not_set_history_size_sentinels() {
    let command = build_host_shell_command(
        shell_starter(ShellType::Zsh, "/bin/zsh"),
        None,
        HashMap::new(),
        None,
        false,
        false,
        false,
        false,
        true,
    );

    assert_eq!(env_value(&command, "HISTFILESIZE"), None);
    assert_eq!(env_value(&command, "HISTSIZE"), None);
    assert_eq!(env_value(&command, "WARP_INITIAL_HISTFILESIZE"), None);
    assert_eq!(env_value(&command, "WARP_INITIAL_HISTSIZE"), None);
}

#[test]
fn docker_sandbox_command_sets_history_size_sentinels() {
    let docker_starter =
        DockerSandboxShellStarter::new(shell_starter(ShellType::Bash, "sbx"), None);
    let command = build_docker_sandbox_command(
        &docker_starter,
        None,
        HashMap::new(),
        false,
        false,
        false,
        false,
        true,
    );

    assert_eq!(
        env_value(&command, "HISTFILESIZE"),
        Some(Some(BASH_HISTORY_SIZE_SENTINEL.to_owned()))
    );
    assert_eq!(
        env_value(&command, "HISTSIZE"),
        Some(Some(BASH_HISTORY_SIZE_SENTINEL.to_owned()))
    );
    assert_eq!(
        env_value(&command, "WARP_INITIAL_HISTFILESIZE"),
        Some(Some(BASH_HISTORY_SIZE_SENTINEL.to_owned()))
    );
    assert_eq!(
        env_value(&command, "WARP_INITIAL_HISTSIZE"),
        Some(Some(BASH_HISTORY_SIZE_SENTINEL.to_owned()))
    );
}
