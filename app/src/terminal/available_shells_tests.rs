use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use warp_core::features::FeatureFlag;

use super::*;
use crate::terminal::shell::ShellType;
use crate::test_util::{Stub, VirtualFS};

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

            // We should expect there to be only one shell: canonical paths dedupe the symlink
            // aliases, but the stored path is the discovered one from the first search location.
            assert_eq!(
                fallback_shells,
                vec![AvailableShell {
                    id: Some(format!("local:{}", bin_bash.display())),
                    state: Arc::new(Config::KnownLocal(LocalConfig {
                        command: "bash".to_string(),
                        executable_path: bin_bash,
                        shell_type: ShellType::Bash,
                    }))
                }]
            )
        },
    );
}

#[test]
fn test_keeps_stable_symlink_path_for_homebrew_shells() {
    FeatureFlag::ShellSelector.set_enabled(true);
    VirtualFS::test(
        "test_keeps_stable_symlink_path_for_homebrew_shells",
        |dirs, mut sandbox| {
            let bin_fish = dirs.tests().join("bin").join("fish");
            let cellar_fish = dirs
                .tests()
                .join("Cellar")
                .join("fish")
                .join("1.0")
                .join("bin")
                .join("fish");

            sandbox.mkdir("Cellar/fish/1.0/bin");
            sandbox.mkdir("bin");
            sandbox.with_files(vec![Stub::MockExecutable("Cellar/fish/1.0/bin/fish")]);
            sandbox.ln("Cellar/fish/1.0/bin/fish", "bin/fish");

            let shells = AvailableShells::load_known_shells(&[dirs.tests().join("bin")], None);

            // The stored path must be the stable symlink (bin/fish), which survives a Homebrew
            // upgrade; the versioned Cellar path is removed when a new version is installed.
            assert_eq!(
                shells,
                vec![AvailableShell {
                    id: Some(format!("local:{}", bin_fish.display())),
                    state: Arc::new(Config::KnownLocal(LocalConfig {
                        command: "fish".to_string(),
                        executable_path: bin_fish,
                        shell_type: ShellType::Fish,
                    }))
                }],
                "expected the stable bin path, but shells contained {shells:?} (cellar path: {})",
                cellar_fish.display()
            );
        },
    );
}

#[test]
fn test_recovers_executable_preference_via_canonical_alias() {
    // A preference persisted by an older build carries the canonical Cellar path, while
    // detection now reports the stable bin symlink. Right after the Warp update both
    // still exist, and recovery must follow the canonical alias.
    VirtualFS::test(
        "test_recovers_executable_preference_via_canonical_alias",
        |dirs, mut sandbox| {
            let bin_fish = dirs.tests().join("bin").join("fish");
            let cellar_fish = dirs
                .tests()
                .join("Cellar")
                .join("fish")
                .join("1.0")
                .join("bin")
                .join("fish");

            sandbox.mkdir("Cellar/fish/1.0/bin");
            sandbox.mkdir("bin");
            sandbox.with_files(vec![Stub::MockExecutable("Cellar/fish/1.0/bin/fish")]);
            sandbox.ln("Cellar/fish/1.0/bin/fish", "bin/fish");

            let shells = make_available_shells(vec![AvailableShell::new_local_executable(
                "fish".to_string(),
                bin_fish.clone(),
                ShellType::Fish,
            )]);

            let recovered = shells
                .recover_unmatched_executable_preference(&NewSessionShell::Executable(
                    cellar_fish.display().to_string(),
                ))
                .expect("should recover to the detected fish");

            if let Config::KnownLocal(config) = recovered.state.as_ref() {
                assert_eq!(config.executable_path, bin_fish);
            } else {
                panic!("expected a KnownLocal shell, got {recovered:?}");
            }
        },
    );
}

#[test]
fn test_recovers_stale_executable_preference_by_shell_type() {
    // After the persisted Cellar path is removed by a formula upgrade, recovery
    // falls back to the detected shell of the same type.
    VirtualFS::test(
        "test_recovers_stale_executable_preference_by_shell_type",
        |dirs, mut sandbox| {
            let bin_fish = dirs.tests().join("bin").join("fish");
            let removed_cellar_fish = dirs
                .tests()
                .join("Cellar")
                .join("fish")
                .join("1.0")
                .join("bin")
                .join("fish");

            sandbox.mkdir("bin");
            sandbox.with_files(vec![Stub::MockExecutable("bin/fish")]);

            let shells = make_available_shells(vec![AvailableShell::new_local_executable(
                "fish".to_string(),
                bin_fish.clone(),
                ShellType::Fish,
            )]);

            let recovered = shells
                .recover_unmatched_executable_preference(&NewSessionShell::Executable(
                    removed_cellar_fish.display().to_string(),
                ))
                .expect("should recover to the detected fish");

            if let Config::KnownLocal(config) = recovered.state.as_ref() {
                assert_eq!(config.executable_path, bin_fish);
            } else {
                panic!("expected a KnownLocal shell, got {recovered:?}");
            }
        },
    );
}

#[test]
fn test_recovery_prefers_the_closest_install_prefix() {
    // With several installs of the same shell on the machine, a stale preference
    // recovers to the detected shell closest to where the stale one lived.
    VirtualFS::test(
        "test_recovery_prefers_the_closest_install_prefix",
        |dirs, _sandbox| {
            let macports_fish = dirs.tests().join("opt/local/bin").join("fish");
            let homebrew_fish = dirs.tests().join("opt/homebrew/bin").join("fish");
            let removed_cellar_fish = dirs
                .tests()
                .join("opt/homebrew/Cellar")
                .join("fish")
                .join("1.0")
                .join("bin")
                .join("fish");

            let shells = make_available_shells(vec![
                AvailableShell::new_local_executable(
                    "fish".to_string(),
                    macports_fish,
                    ShellType::Fish,
                ),
                AvailableShell::new_local_executable(
                    "fish".to_string(),
                    homebrew_fish.clone(),
                    ShellType::Fish,
                ),
            ]);

            let recovered = shells
                .recover_unmatched_executable_preference(&NewSessionShell::Executable(
                    removed_cellar_fish.display().to_string(),
                ))
                .expect("should recover to a detected fish");

            if let Config::KnownLocal(config) = recovered.state.as_ref() {
                assert_eq!(config.executable_path, homebrew_fish);
            } else {
                panic!("expected a KnownLocal shell, got {recovered:?}");
            }
        },
    );
}

#[test]
fn test_does_not_recover_existing_unmatched_or_non_shell_preference() {
    VirtualFS::test(
        "test_does_not_recover_existing_unmatched_or_non_shell_preference",
        |dirs, mut sandbox| {
            sandbox.mkdir("usr/bin");
            sandbox.with_files(vec![Stub::MockExecutable("usr/bin/zsh")]);

            // The detected zsh lives at a different path with no file behind it,
            // so its canonical form cannot alias the existing preference path.
            let shells = make_available_shells(vec![AvailableShell::new_local_executable(
                "zsh".to_string(),
                dirs.tests().join("usr/local/bin").join("zsh"),
                ShellType::Zsh,
            )]);

            // An existing executable that matches no detected binary is a deliberate
            // out-of-catalog choice and is left to the launch-time fallback.
            assert!(
                shells
                    .recover_unmatched_executable_preference(&NewSessionShell::Executable(
                        dirs.tests().join("usr/bin").join("zsh").display().to_string(),
                    ))
                    .is_none()
            );

            // A stale path whose file name is not a supported shell does not recover either.
            assert!(
                shells
                    .recover_unmatched_executable_preference(&NewSessionShell::Executable(
                        dirs.tests().join("removed/bin").join("nu").display().to_string(),
                    ))
                    .is_none()
            );
        },
    );
}

#[test]
fn test_get_from_shell_launch_data_recovers_stale_snapshot_path() {
    VirtualFS::test(
        "test_get_from_shell_launch_data_recovers_stale_snapshot_path",
        |dirs, mut sandbox| {
            let bin_fish = dirs.tests().join("bin").join("fish");

            sandbox.mkdir("bin");
            sandbox.with_files(vec![Stub::MockExecutable("bin/fish")]);

            let shells = make_available_shells(vec![AvailableShell::new_local_executable(
                "fish".to_string(),
                bin_fish.clone(),
                ShellType::Fish,
            )]);

            // A snapshot carrying a Cellar path removed by a formula upgrade restores
            // to the detected shell instead of a dead custom path.
            let stale = ShellLaunchData::Executable {
                executable_path: dirs
                    .tests()
                    .join("Cellar")
                    .join("fish")
                    .join("1.0")
                    .join("bin")
                    .join("fish"),
                shell_type: ShellType::Fish,
            };
            let recovered = shells
                .get_from_shell_launch_data(&stale)
                .expect("should recover from the stale snapshot path");
            if let Config::KnownLocal(config) = recovered.state.as_ref() {
                assert_eq!(config.executable_path, bin_fish);
            } else {
                panic!("expected a KnownLocal shell, got {recovered:?}");
            }

            // An existing out-of-catalog path still restores as a custom shell.
            sandbox.mkdir("usr/bin");
            sandbox.with_files(vec![Stub::MockExecutable("usr/bin/zsh")]);
            let custom_path = dirs.tests().join("usr/bin").join("zsh");
            let existing = ShellLaunchData::Executable {
                executable_path: custom_path.clone(),
                shell_type: ShellType::Zsh,
            };
            assert_eq!(
                shells.get_from_shell_launch_data(&existing),
                Some(AvailableShell::new_custom_shell(
                    "zsh".to_string(),
                    custom_path,
                    ShellType::Zsh,
                ))
            );
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
