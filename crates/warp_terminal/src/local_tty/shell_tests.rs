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
