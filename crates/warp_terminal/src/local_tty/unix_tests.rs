use nix::sys::termios::LocalFlags;

use super::*;

fn shell_starter(shell_type: ShellType, shell_path: &str) -> DirectShellStarter {
    DirectShellStarter::new_for_test(shell_type, PathBuf::from(shell_path), Vec::new())
}

fn dev_container_starter(remote_user: Option<&str>) -> DevContainerShellStarter {
    DevContainerShellStarter::new(
        shell_starter(ShellType::Bash, "docker"),
        PathBuf::from("/home/user/project"),
        "abc123".to_owned(),
        remote_user.map(str::to_owned),
        "/workspaces/project".to_owned(),
        "deadbeef".to_owned(),
    )
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

#[test]
fn dev_container_exec_args_attaches_with_it() {
    let starter = dev_container_starter(None);
    let args = dev_container_exec_args(&starter);

    // `-it`: Docker allocates its own pty on the host-exec side, which
    // forwards resizes and Ctrl-C natively.
    assert_eq!(args[0], "exec");
    assert_eq!(args[1], "-it");
}

#[test]
fn dev_container_exec_args_execs_bash_directly_with_an_unquoted_rcfile_path() {
    let starter = dev_container_starter(None);
    let args = dev_container_exec_args(&starter);

    // No `script` wrapper: `-it` allocates the pty on the relay itself, so
    // there's no need to allocate a second one inside the container.
    assert!(!args.iter().any(|arg| arg == "script"));

    let bash_pos = args
        .iter()
        .position(|arg| arg == "bash")
        .expect("args should exec `bash` directly");
    assert_eq!(args[bash_pos + 1], "--rcfile");
    // Unquoted: this is a literal argv element, not a string a shell
    // re-parses, so quoting it would hand bash a path that doesn't exist.
    assert_eq!(
        args[bash_pos + 2],
        std::ffi::OsString::from(starter.container_init_script_path())
    );
    assert_eq!(args[bash_pos + 3], "--noprofile");
}

#[test]
fn dev_container_exec_args_includes_remote_user_when_present() {
    let starter = dev_container_starter(Some("vscode"));
    let args = dev_container_exec_args(&starter);

    let user_pos = args
        .iter()
        .position(|arg| arg == "-u")
        .expect("args should include -u when a remote user is set");
    assert_eq!(args[user_pos + 1], "vscode");
}

#[test]
fn spawn_command_in_pty_leaves_the_pty_in_cooked_mode() {
    let size = SizeInfo::new_without_font_metrics(24, 80);

    let mut command = Command::new("/bin/sleep");
    command.arg("5");
    let mut spawned = spawn_command_in_pty(command, &size, true)
        .expect("spawn_command_in_pty should succeed for a trivial command");

    let termios = termios::tcgetattr(spawned.result.leader_fd)
        .expect("tcgetattr should succeed on the leader fd");
    assert!(
        termios.local_flags.contains(LocalFlags::ISIG)
            && termios.local_flags.contains(LocalFlags::ICANON)
            && termios.local_flags.contains(LocalFlags::ECHO),
        "the pty should stay in cooked mode; raw-mode forcing was only ever needed by the \
         retired `-i` Dev Container workaround"
    );

    let _ = spawned.child.kill();
    let _ = spawned.child.wait();
}

#[test]
fn dev_container_cp_args_targets_the_given_container_path() {
    let starter = dev_container_starter(None);
    let host_path =
        PathBuf::from("/home/user/.cache/warp-terminal-local/dev-container/init/deadbeef.sh");
    let args = dev_container_cp_args(
        &starter.container_id,
        &host_path,
        &starter.container_init_script_path(),
    );

    assert_eq!(
        args,
        vec![
            std::ffi::OsString::from("cp"),
            std::ffi::OsString::from(&host_path),
            std::ffi::OsString::from(format!("abc123:{}", starter.container_init_script_path())),
        ]
    );
}

#[test]
fn dev_container_chown_args_run_as_root_targeting_the_given_user() {
    let starter = dev_container_starter(Some("vscode"));
    let args = dev_container_chown_args(
        &starter.container_id,
        "vscode",
        &starter.container_init_script_path(),
    );

    assert_eq!(
        args,
        vec![
            std::ffi::OsString::from("exec"),
            std::ffi::OsString::from("-u"),
            std::ffi::OsString::from("0"),
            std::ffi::OsString::from("abc123"),
            std::ffi::OsString::from("chown"),
            std::ffi::OsString::from("vscode"),
            std::ffi::OsString::from(starter.container_init_script_path()),
        ]
    );
}

#[test]
fn dev_container_chmod_args_lock_the_init_script_to_owner_read_only() {
    let starter = dev_container_starter(None);
    let args =
        dev_container_chmod_args(&starter.container_id, &starter.container_init_script_path());

    assert_eq!(
        args,
        vec![
            std::ffi::OsString::from("exec"),
            std::ffi::OsString::from("-u"),
            std::ffi::OsString::from("0"),
            std::ffi::OsString::from("abc123"),
            std::ffi::OsString::from("chmod"),
            std::ffi::OsString::from("400"),
            std::ffi::OsString::from(starter.container_init_script_path()),
        ]
    );
}

#[test]
fn dev_container_cp_args_also_work_for_the_bootstrap_script_path() {
    let starter = dev_container_starter(None);
    let host_path = host_bootstrap_script_path_for_content_hash("abcdef0123456789");
    let container_path = container_bootstrap_script_path_for_content_hash("abcdef0123456789");
    let args = dev_container_cp_args(&starter.container_id, &host_path, &container_path);

    assert_eq!(
        args,
        vec![
            std::ffi::OsString::from("cp"),
            std::ffi::OsString::from(&host_path),
            std::ffi::OsString::from(format!("abc123:{container_path}")),
        ]
    );
}

#[test]
fn bootstrap_script_paths_are_keyed_by_content_hash_not_sandbox() {
    // Same hash in, same paths out, regardless of which sandbox asks -- this is the whole
    // point: every session on a build stages the same bootstrap bytes at the same path.
    assert_eq!(
        host_bootstrap_script_path_for_content_hash("deadbeef"),
        host_bootstrap_script_path_for_content_hash("deadbeef")
    );
    assert_ne!(
        host_bootstrap_script_path_for_content_hash("deadbeef"),
        host_bootstrap_script_path_for_content_hash("cafef00d")
    );
    assert!(
        host_bootstrap_script_path_for_content_hash("deadbeef")
            .to_string_lossy()
            .contains("deadbeef")
    );
    assert!(container_bootstrap_script_path_for_content_hash("deadbeef").contains("deadbeef"));
}

#[test]
fn dev_container_default_user_args_query_the_unqualified_exec_user() {
    let starter = dev_container_starter(None);
    let args = dev_container_default_user_args(&starter.container_id);

    // Deliberately no `-u` here: the point is to ask the container what user
    // an unqualified `docker exec` (the same as the real attach uses when
    // there's no `remoteUser`) actually runs as. Passing `-u 0` would always
    // answer "root" regardless of the image's real default user.
    assert_eq!(
        args,
        vec![
            std::ffi::OsString::from("exec"),
            std::ffi::OsString::from("abc123"),
            std::ffi::OsString::from("id"),
            std::ffi::OsString::from("-un"),
        ]
    );
}

#[test]
fn dev_container_default_user_args_do_not_force_root_unlike_chown_args() {
    let starter = dev_container_starter(None);
    let default_user_args = dev_container_default_user_args(&starter.container_id);
    let chown_args = dev_container_chown_args(
        &starter.container_id,
        "someuser",
        &starter.container_init_script_path(),
    );

    // The default-user probe and the chown step have opposite goals for
    // *which* user runs the command: the probe must run unqualified to
    // observe the real default user, while chown must run as root (uid 0)
    // to be able to change ownership at all. Guard against ever
    // "simplifying" the probe back to reusing the chown/chmod/rm helpers'
    // `-u 0`.
    assert!(!default_user_args.iter().any(|arg| arg == "-u"));
    assert!(chown_args.iter().any(|arg| arg == "-u"));
}

#[test]
fn dev_container_init_script_sources_the_staged_bootstrap_script() {
    let starter = dev_container_starter(None);
    let container_bootstrap_path = container_bootstrap_script_path_for_content_hash("deadbeef");
    let script = dev_container_init_script(starter.session_id(), &container_bootstrap_path);

    // The bootstrap script is `source`d directly from a file the container
    // already has, rather than typed into the pty by Warp later.
    assert!(script.contains(&format!("source '{container_bootstrap_path}'\n")));
}

#[test]
fn content_hash_is_stable_and_content_sensitive() {
    assert_eq!(content_hash(b"same bytes"), content_hash(b"same bytes"));
    assert_ne!(content_hash(b"these bytes"), content_hash(b"other bytes"));
}

#[test]
fn dev_container_bootstrap_exists_args_check_unqualified() {
    let starter = dev_container_starter(None);
    let container_path = container_bootstrap_script_path_for_content_hash("deadbeef");
    let args = dev_container_bootstrap_exists_args(&starter.container_id, &container_path);

    // Deliberately no `-u 0`: existence in `/tmp` doesn't require any special
    // privilege, unlike the chown/chmod/rm helpers.
    assert!(!args.iter().any(|arg| arg == "-u"));
    assert_eq!(
        args,
        vec![
            std::ffi::OsString::from("exec"),
            std::ffi::OsString::from("abc123"),
            std::ffi::OsString::from("test"),
            std::ffi::OsString::from("-e"),
            std::ffi::OsString::from(&container_path),
        ]
    );
}

#[test]
fn dev_container_rm_args_remove_the_given_container_path() {
    let starter = dev_container_starter(None);
    let args = dev_container_rm_args(&starter.container_id, &starter.container_init_script_path());

    assert_eq!(
        args,
        vec![
            std::ffi::OsString::from("exec"),
            std::ffi::OsString::from("-u"),
            std::ffi::OsString::from("0"),
            std::ffi::OsString::from("abc123"),
            std::ffi::OsString::from("rm"),
            std::ffi::OsString::from("-f"),
            std::ffi::OsString::from(starter.container_init_script_path()),
        ]
    );
}
