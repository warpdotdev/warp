use super::*;

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

/// Regression test for https://github.com/warpdotdev/warp/issues/13308.
///
/// WSL sessions must NOT be launched with `--exec`. On the WSL 2.9.3 / WSLC
/// preview the `wsl --exec` path breaks the ConPTY stdin relay, so the bootstrap
/// bytes Warp writes back never reach the shell and the tab hangs at
/// "Starting bash". The interactive launch must instead use the same
/// `--shell-type standard --` shape as the working detection/home-dir calls.
#[test]
fn test_wsl_session_args_do_not_use_exec() {
    for shell_type in [ShellType::Bash, ShellType::Zsh, ShellType::Fish] {
        let shell_path = match shell_type {
            ShellType::Bash => BASH_SHELL_PATH,
            ShellType::Zsh => ZSH_SHELL_PATH,
            ShellType::Fish => FISH_SHELL_PATH,
            ShellType::PowerShell => unreachable!("only bash/zsh/fish are iterated"),
        };
        let args = wsl_arguments_for_session_spawning_command(
            "Ubuntu",
            shell_path,
            shell_type,
            generate_session_id(),
        );

        // `--exec` is the WSL flag that regressed on 2.9.3; it must be gone.
        assert!(
            !args.iter().any(|arg| arg == "--exec"),
            "WSL session args for {shell_type:?} must not contain --exec: {args:?}"
        );

        // The launch must use the known-good `--shell-type standard --` prefix
        // (matching the working detection/home-dir calls), followed by the shell.
        assert_eq!(
            args[..6],
            [
                OsString::from("--distribution"),
                OsString::from("Ubuntu"),
                OsString::from("--shell-type"),
                OsString::from("standard"),
                OsString::from("--"),
                OsString::from(shell_path),
            ],
            "unexpected WSL arg prefix for {shell_type:?}: {args:?}"
        );
    }
}
