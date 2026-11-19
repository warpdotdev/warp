use tempfile::tempdir;

use super::*;

#[test]
fn direct_replacement_refreshes_embedded_bootstrap_session() {
    let starter = ShellStarter::Direct(DirectShellStarter {
        shell_type: ShellType::Bash,
        shell_path: "/bin/bash".into(),
        args: arguments_for_session_spawning_command("/bin/bash", ShellType::Bash, 123.into()),
        session_id: 123.into(),
    });

    let replacement = starter.replacement().unwrap();

    assert_ne!(replacement.session_id(), starter.session_id());
    assert_eq!(replacement.launch_data(), starter.launch_data());
    let ShellStarter::Direct(replacement) = replacement else {
        panic!("replacement must remain a direct shell");
    };
    let init_script =
        init_shell_script_for_shell(ShellType::Bash, &crate::ASSETS, replacement.session_id());
    assert!(
        replacement.args()[1]
            .to_string_lossy()
            .contains(&init_script)
    );
}

#[test]
fn wsl_replacement_preserves_distribution_and_refreshes_bootstrap_session() {
    let starter = ShellStarter::Wsl(WslShellStarter {
        shell_type: ShellType::Fish,
        shell_path: "/usr/bin/fish".to_owned(),
        distribution: "Ubuntu".to_owned(),
        args: wsl_arguments_for_session_spawning_command(
            "Ubuntu",
            "/usr/bin/fish",
            ShellType::Fish,
            123.into(),
        ),
        session_id: 123.into(),
    });

    let replacement = starter.replacement().unwrap();

    assert_ne!(replacement.session_id(), starter.session_id());
    assert_eq!(replacement.launch_data(), starter.launch_data());
    let ShellStarter::Wsl(replacement) = replacement else {
        panic!("replacement must remain in WSL");
    };
    assert_eq!(replacement.args()[0], "--distribution");
    assert_eq!(replacement.args()[1], "Ubuntu");
    let init_script =
        init_shell_script_for_shell(ShellType::Fish, &crate::ASSETS, replacement.session_id());
    assert!(
        replacement
            .args()
            .last()
            .unwrap()
            .to_string_lossy()
            .contains(&init_script)
    );
}

#[test]
fn msys2_replacement_preserves_launch_mode_for_injected_bootstrap() {
    let starter = ShellStarter::MSYS2(DirectShellStarter {
        shell_type: ShellType::Bash,
        shell_path: r"C:\Program Files\Git\bin\bash.exe".into(),
        args: msys2_arguments_for_session_spawning_command(ShellType::Bash),
        session_id: 123.into(),
    });

    let replacement = starter.replacement().unwrap();

    assert_ne!(replacement.session_id(), starter.session_id());
    assert_eq!(replacement.launch_data(), starter.launch_data());
    let ShellStarter::MSYS2(replacement) = replacement else {
        panic!("replacement must retain MSYS2 bootstrap handling");
    };
    assert_eq!(replacement.args(), &["--noprofile", "--norc"]);
}

#[test]
fn recovery_uses_session_home_when_working_directory_is_missing() {
    let home = tempdir().unwrap();
    let missing_directory = home.path().join("missing");
    let starter = ShellStarter::Direct(DirectShellStarter::new_for_test(
        ShellType::Bash,
        "/bin/bash".into(),
        Vec::new(),
    ));

    let (restored, fallback) = starter
        .recovery_working_directory(missing_directory.to_str(), home.path().to_str())
        .unwrap();

    assert_eq!(restored, home.path().to_str().unwrap());
    assert!(fallback);
    assert!(
        starter
            .recovery_working_directory(missing_directory.to_str(), None,)
            .is_err()
    );
}

#[test]
fn test_program_invalid_bash() {
    // This test assumes there is no bash binary at /some/weird/path/bash.
    let shell_path = "/some/weird/path/bash".to_owned();
    assert!(supported_shell_path_and_type(&shell_path).is_none());
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

#[test]
fn bash_spawn_command_pins_bash_variable_to_resolved_path() {
    let args = arguments_for_session_spawning_command(
        "/opt/homebrew/bin/bash",
        ShellType::Bash,
        123.into(),
    );
    let command = args[1].to_string_lossy();

    // The assignment is single-quoted inside the rcfile, and the rcfile itself is
    // single-quoted for `echo`, so the quotes appear in their escaped form.
    assert!(
        command.contains(r#"BASH='"'"'/opt/homebrew/bin/bash'"'"';"#),
        "{command}"
    );
    // `$0` stays `bash` so we don't change what users see in `$0`.
    assert!(command.starts_with("exec -a bash '/opt/homebrew/bin/bash' "));
}

/// Runs the bash spawn command on a PTY (so the shell is interactive and reads the
/// rcfile, as in a real session), but with a launchd-like `PATH` in which `bash`
/// resolves to a different binary than the one being executed, and returns the
/// shell's `$BASH`.
#[cfg(unix)]
fn bash_variable_for_spawned_shell(shell_path: &Path) -> String {
    use std::fs::File;
    use std::io::{Read as _, Write as _};
    use std::os::unix::io::FromRawFd as _;
    use std::sync::mpsc;
    use std::time::Duration;

    use command::blocking::Command;

    let args = arguments_for_session_spawning_command(
        &shell_path.to_string_lossy(),
        ShellType::Bash,
        123.into(),
    );
    let pty = nix::pty::openpty(None, None).unwrap();
    // SAFETY: `openpty` returns freshly opened descriptors that nothing else owns.
    let (mut master, slave) =
        unsafe { (File::from_raw_fd(pty.master), File::from_raw_fd(pty.slave)) };
    let mut child = Command::new(shell_path)
        .args(args)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", shell_path.parent().unwrap())
        .stdin(slave.try_clone().unwrap())
        .stdout(slave.try_clone().unwrap())
        .stderr(slave)
        .spawn()
        .unwrap();
    // Octal escapes keep the markers out of the PTY's echo of the command itself.
    master
        .write_all(b"printf '\\074\\074%s\\076\\076' \"$BASH\"; exit\n")
        .unwrap();

    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut output = Vec::new();
        // Reading the master fails with EIO on Linux once the shell exits; the bytes
        // read before that are still in `output`.
        let _ = master.read_to_end(&mut output);
        let _ = tx.send(output);
    });
    let output = rx.recv_timeout(Duration::from_secs(30));
    let _ = child.kill();
    let _ = child.wait();
    let output = output.expect("timed out waiting for the shell to exit");

    let output = String::from_utf8_lossy(&output);
    let start = output.find("<<").expect("shell did not print $BASH") + 2;
    let end = output[start..].find(">>").unwrap() + start;
    output[start..end].to_owned()
}

#[cfg(unix)]
#[test]
fn spawned_bash_reports_the_executed_binary_as_bash_variable() {
    let Some(system_bash) = ["/bin/bash", "/usr/bin/bash"]
        .into_iter()
        .map(Path::new)
        .find(|path| path.exists())
    else {
        return;
    };

    let root = tempdir().unwrap();
    for dir_name in ["custom bash", "it's bash"] {
        let dir = root.path().join(dir_name);
        std::fs::create_dir(&dir).unwrap();
        let shell_path = dir.join("bash");
        std::os::unix::fs::symlink(system_bash, &shell_path).unwrap();

        assert_eq!(
            bash_variable_for_spawned_shell(&shell_path),
            shell_path.to_string_lossy()
        );
    }
}
