use super::*;
use crate::terminal::shell::ShellType;
use crate::test_util::{Stub, VirtualFS};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use warp_core::features::FeatureFlag;

fn make_available_shells(shells: Vec<AvailableShell>) -> AvailableShells {
    AvailableShells {
        shells,
        shell_counts: HashMap::new(),
    }
}

#[test]
fn test_load_known_shells_with_empty_path_var() {
    FeatureFlag::ShellSelector.set_enabled(true);

    // First assert that if there is no fallback, and the env var is empty, we do not load ANY shells
    let paths_to_search = vec![];
    let fallback_shells = AvailableShells::load_known_shells(
        &paths_to_search,
        Some(Path::new("/some/nonexistent/path")),
    );
    assert!(
        fallback_shells.is_empty(),
        "expected there to be no shells, but shells contained {fallback_shells:?}"
    );

    VirtualFS::test(
        "test_load_known_shells_with_empty_path_var",
        |dirs, mut sandbox| {
            let bash = dirs.tests().join("bin").join("bash");
            let zsh = dirs.tests().join("bin").join("zsh");
            let fallback_shells_path = dirs.tests().join("etc").join("shells");
            // Now assert that if there is a fallback, and the env var is empty, we load the fallback
            sandbox.mkdir("etc");
            sandbox.mkdir("bin");
            sandbox.with_files(vec![
                Stub::FileWithContent(
                    "etc/shells",
                    format!("{}\n{}\n", bash.display(), zsh.display()).as_str(),
                ),
                Stub::MockExecutable("bin/bash"),
                Stub::MockExecutable("bin/zsh"),
            ]);

            let fallback_shells = AvailableShells::load_known_shells(
                &paths_to_search,
                Some(fallback_shells_path.as_path()),
            );

            assert_eq!(fallback_shells.len(), 2);
            // note about this test; current impl is that we add shells in the following order:
            //   zsh, bash, fish, pwsh, powershell
            // so it is important to assert that even though `/bin/bash` is located before `/bin/zsh`
            // in the fallback file, we still list `zsh` first.
            assert_eq!(
                fallback_shells,
                vec![
                    AvailableShell {
                        id: Some(format!("local:{}", zsh.display())),
                        state: Arc::new(Config::KnownLocal(LocalConfig {
                            command: "zsh".to_string(),
                            executable_path: zsh,
                            shell_type: ShellType::Zsh,
                        }))
                    },
                    AvailableShell {
                        id: Some(format!("local:{}", bash.display())),
                        state: Arc::new(Config::KnownLocal(LocalConfig {
                            command: "bash".to_string(),
                            executable_path: bash,
                            shell_type: ShellType::Bash,
                        }))
                    }
                ]
            )
        },
    );
}

#[test]
fn test_dedupe_symlinks_when_discovering_paths() {
    FeatureFlag::ShellSelector.set_enabled(true);
    VirtualFS::test(
        "test_dedupe_symlinks_when_discovering_paths",
        |dirs, mut sandbox| {
            let bin = dirs.tests().join("bin");
            let bin_bash = bin.join("bash");
            let usr_bin = dirs.tests().join("usr").join("bin");
            let usr_bin_bash = usr_bin.join("bash");
            let etc_shells = dirs.tests().join("etc").join("shells");

            let paths_to_search = vec![bin, usr_bin];
            let etc_shells_content =
                format!("{}\n{}\n", bin_bash.display(), usr_bin_bash.display());

            sandbox.mkdir("etc");
            sandbox.mkdir("usr/bin");
            sandbox.ln("usr/bin", "bin");
            sandbox.with_files(vec![
                Stub::FileWithContent("etc/shells", etc_shells_content.as_str()),
                Stub::MockExecutable("usr/bin/bash"),
            ]);

            let fallback_shells =
                AvailableShells::load_known_shells(&paths_to_search, Some(etc_shells.as_path()));

            // We should expect there to be only one shell, with the path and id for that shell being
            // the canonical path to the executable.
            assert_eq!(
                fallback_shells,
                vec![AvailableShell {
                    id: Some(format!("local:{}", usr_bin_bash.display())),
                    state: Arc::new(Config::KnownLocal(LocalConfig {
                        command: "bash".to_string(),
                        executable_path: usr_bin_bash,
                        shell_type: ShellType::Bash,
                    }))
                }]
            )
        },
    );
}

#[test]
fn test_find_by_command_name_matches_known_shell() {
    let zsh_path = PathBuf::from("/bin/zsh");
    let pwsh_path = PathBuf::from("/opt/homebrew/bin/pwsh");
    let shells = make_available_shells(vec![
        AvailableShell::new_local_executable("zsh".to_string(), zsh_path.clone(), ShellType::Zsh),
        AvailableShell::new_local_executable(
            "pwsh".to_string(),
            pwsh_path.clone(),
            ShellType::PowerShell,
        ),
    ]);

    let matched = shells
        .find_by_command_name("pwsh")
        .expect("should find pwsh by command name");
    assert_eq!(
        matched.id(),
        Some(format!("local:{}", pwsh_path.display()).as_str()),
    );

    let matched = shells
        .find_by_command_name("zsh")
        .expect("should find zsh by command name");
    assert_eq!(
        matched.id(),
        Some(format!("local:{}", zsh_path.display()).as_str()),
    );
}

#[test]
fn test_find_by_command_name_returns_none_for_unknown_name() {
    let shells = make_available_shells(vec![AvailableShell::new_local_executable(
        "zsh".to_string(),
        PathBuf::from("/bin/zsh"),
        ShellType::Zsh,
    )]);

    assert!(shells.find_by_command_name("pwsh").is_none());
    assert!(shells.find_by_command_name("").is_none());
}

#[test]
fn test_find_by_command_name_is_case_sensitive_on_unix() {
    // File names on Unix are case-sensitive, so an uppercase request should
    // not match a lowercase stored command.
    let shells = make_available_shells(vec![AvailableShell::new_local_executable(
        "pwsh".to_string(),
        PathBuf::from("/opt/homebrew/bin/pwsh"),
        ShellType::PowerShell,
    )]);

    assert!(shells.find_by_command_name("pwsh").is_some());
    assert!(shells.find_by_command_name("PWSH").is_none());
    assert!(shells.find_by_command_name("PowerShell").is_none());
}

#[test]
fn test_find_by_command_name_skips_system_default() {
    // A SystemDefault entry should never be matched: it has no command name.
    let shells = make_available_shells(vec![
        AvailableShell::default(),
        AvailableShell::new_local_executable(
            "zsh".to_string(),
            PathBuf::from("/bin/zsh"),
            ShellType::Zsh,
        ),
    ]);

    let matched = shells
        .find_by_command_name("zsh")
        .expect("should find zsh past the SystemDefault entry");
    assert_eq!(
        matched.id(),
        Some(format!("local:{}", PathBuf::from("/bin/zsh").display()).as_str()),
    );
}

#[test]
fn test_find_by_command_name_matches_msys2_shell() {
    // Construct an MSYS2 shell directly so the `Config::MSYS2` arm of
    // `find_by_command_name` is exercised from any platform — its
    // `AvailableShell::new_msys2` constructor is gated to Windows, but the
    // match arm is platform-independent.
    let path = PathBuf::from("/tmp/msys64/usr/bin/bash-msys2");
    let msys2_shell = AvailableShell {
        id: Some(format!("msys2:{}", path.display())),
        state: Arc::new(Config::MSYS2(LocalConfig {
            command: "bash-msys2".to_string(),
            executable_path: path.clone(),
            shell_type: ShellType::Bash,
        })),
    };
    let shells = make_available_shells(vec![msys2_shell]);

    let matched = shells
        .find_by_command_name("bash-msys2")
        .expect("should find MSYS2 shell by command name");
    assert_eq!(
        matched.id(),
        Some(format!("msys2:{}", path.display()).as_str()),
    );
}

#[test]
fn test_command_name_matches_unix() {
    // Unix matching is a plain case-sensitive equality check: no case
    // folding, no `.exe` suffix handling.
    assert!(command_name_matches("pwsh", "pwsh", false));
    assert!(command_name_matches("zsh", "zsh", false));

    assert!(!command_name_matches("pwsh", "PWSH", false));
    assert!(!command_name_matches("pwsh", "pwsh.exe", false));
    assert!(!command_name_matches("pwsh.exe", "pwsh", false));
    assert!(!command_name_matches("pwsh", "powershell", false));
    assert!(!command_name_matches("", "pwsh", false));
}

#[test]
fn test_command_name_matches_windows() {
    // Windows matching is case-insensitive and allows an optional trailing
    // `.exe` on either side.
    assert!(command_name_matches("pwsh", "pwsh", true));
    assert!(command_name_matches("pwsh", "PWSH", true));
    assert!(command_name_matches("PWSH", "pwsh", true));
    assert!(command_name_matches("PwSh", "pWsH", true));

    // `.exe` is optional on either side.
    assert!(command_name_matches("pwsh.exe", "pwsh", true));
    assert!(command_name_matches("pwsh", "pwsh.exe", true));
    assert!(command_name_matches("pwsh.exe", "PWSH.EXE", true));
    assert!(command_name_matches("powershell.exe", "PowerShell", true));

    // Distinct shells should not collide.
    assert!(!command_name_matches("pwsh", "powershell", true));
    assert!(!command_name_matches("pwsh.exe", "powershell.exe", true));
    assert!(!command_name_matches("bash.exe", "zsh", true));
}
