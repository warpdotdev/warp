use std::path::PathBuf;

use warp_terminal::shell::{ShellLaunchData, ShellType};
use warpui_core::platform::OperatingSystem;

use super::*;

/// Regression tests for the web shared-session path rendering bug
/// (APP-5438): the viewer's local/browser operating system must not
/// override the shared session's actual path style.
mod use_unix_paths_impl_tests {
    use super::*;

    #[test]
    fn windows_viewer_with_unix_style_cwd_uses_forward_slashes() {
        // A web viewer on Windows looking at a Linux/Mac shared session: the
        // cwd's shape should win over the (viewer-derived) operating system.
        assert!(use_unix_paths_impl(
            OperatingSystem::Windows,
            None,
            ["/home/user/project"],
        ));
    }

    #[test]
    fn real_windows_host_uses_backslashes() {
        // A genuine Windows host (or a web viewer correctly on Windows,
        // viewing a Windows session) must still render backslashes.
        assert!(!use_unix_paths_impl(
            OperatingSystem::Windows,
            None,
            [r"C:\Users\username\project"],
        ));
    }

    #[test]
    fn ambiguous_cwd_falls_back_to_operating_system() {
        assert!(use_unix_paths_impl(
            OperatingSystem::Linux,
            None,
            ["relative/path"]
        ));
        assert!(!use_unix_paths_impl(
            OperatingSystem::Windows,
            None,
            ["relative/path"],
        ));
    }

    #[test]
    fn no_hints_falls_back_to_operating_system() {
        assert!(use_unix_paths_impl(OperatingSystem::Mac, None, []));
        assert!(!use_unix_paths_impl(OperatingSystem::Windows, None, []));
    }

    #[test]
    fn wsl_is_always_unix_regardless_of_operating_system_or_cwd() {
        let wsl = ShellLaunchData::WSL {
            distro: "Ubuntu".to_string(),
        };
        assert!(use_unix_paths_impl(
            OperatingSystem::Windows,
            Some(&wsl),
            [r"C:\Users\username"],
        ));
    }

    #[test]
    fn msys2_is_always_unix_regardless_of_operating_system_or_cwd() {
        let msys2 = ShellLaunchData::MSYS2 {
            executable_path: PathBuf::from(r"C:\Program Files\Git\usr\bin\bash.exe"),
            shell_type: ShellType::Bash,
        };
        assert!(use_unix_paths_impl(
            OperatingSystem::Windows,
            Some(&msys2),
            [r"C:\Users\username"],
        ));
    }

    #[test]
    fn docker_sandbox_is_always_unix_regardless_of_operating_system_or_cwd() {
        let docker_sandbox = ShellLaunchData::DockerSandbox {
            sbx_path: PathBuf::from("/usr/local/bin/sbx"),
            base_image: None,
        };
        assert!(use_unix_paths_impl(
            OperatingSystem::Windows,
            Some(&docker_sandbox),
            [r"C:\Users\username"],
        ));
    }
}

#[test]
fn windows_viewer_renders_read_files_style_path_with_forward_slashes_for_unix_session() {
    // End-to-end regression check through the public API used by read_files
    // display: a Unix-style cwd must produce forward slashes, independent of
    // the local/viewer operating system reported by `OperatingSystem::get()`
    // (which on web is the browser's OS, not the shared session's).
    let cwd = Some("/home/user/project".to_string());
    assert_eq!(
        shell_native_absolute_path("file.txt", None, cwd.as_ref()),
        "/home/user/project/file.txt"
    );
}

/// Regression tests for `to_shell_native_display_path`, the helper used only where a stored
/// `FileContext.file_name` is rendered (see `agent::file_locations::group_file_contexts_for_display`).
/// `shell_native_absolute_path` itself must NOT perform this reversal, since its result is also
/// used to build the path for command execution (grep, file_glob): shape isn't provenance, and a
/// legitimate Unix filename can look Windows-shaped.
mod to_shell_native_display_path_tests {
    use super::*;

    fn wsl_shell() -> Option<ShellLaunchData> {
        Some(ShellLaunchData::WSL {
            distro: "Ubuntu".to_string(),
        })
    }

    fn msys2_shell() -> Option<ShellLaunchData> {
        Some(ShellLaunchData::MSYS2 {
            executable_path: PathBuf::from(r"C:\Program Files\Git\usr\bin\bash.exe"),
            shell_type: ShellType::Bash,
        })
    }

    #[test]
    fn wsl_unc_path_becomes_unix_path() {
        // A stored `\\WSL$\<distro>\...` UNC path (what `host_native_absolute_path` produces on
        // a Windows host by default) must reverse to forward slashes, matching the WSL shell's
        // own view of the file.
        assert_eq!(
            to_shell_native_display_path(r"\\WSL$\Ubuntu\home\user\file.txt", wsl_shell().as_ref()),
            "/home/user/file.txt"
        );
    }

    #[test]
    fn wsl_unc_path_from_a_different_distro_is_left_explicit() {
        // A UNC path naming a distro other than the session's own doesn't correspond to a path
        // within this session; it must not be rendered as if it were local.
        assert_eq!(
            to_shell_native_display_path(r"\\WSL$\Debian\home\user\file.txt", wsl_shell().as_ref()),
            "//WSL$/Debian/home/user/file.txt"
        );
        // The distro host is matched case-insensitively, like `parse_wsl_unc_path` itself.
        assert_eq!(
            to_shell_native_display_path(r"\\WSL$\ubuntu\home\user\file.txt", wsl_shell().as_ref()),
            "/home/user/file.txt"
        );
    }

    #[test]
    fn wsl_drive_letter_path_becomes_mnt_path() {
        // A stored drive-letter path (what `host_native_absolute_path` produces for a WSL path
        // under `/mnt/<drive>`) must reverse back to its `/mnt/<drive>` form.
        assert_eq!(
            to_shell_native_display_path(r"C:\Users\alice\file.txt", wsl_shell().as_ref()),
            "/mnt/c/Users/alice/file.txt"
        );
    }

    #[test]
    fn msys2_windows_path_becomes_unix_path() {
        // A stored Windows-native drive path (what `host_native_absolute_path` produces for an
        // MSYS2/Git Bash session) must reverse to forward slashes.
        assert_eq!(
            to_shell_native_display_path(
                r"C:\Users\username\project\file.txt",
                msys2_shell().as_ref()
            ),
            "/c/Users/username/project/file.txt"
        );
    }

    #[test]
    fn native_windows_session_is_unaffected() {
        // A genuine Windows session (no WSL/MSYS2 shell) must be unaffected.
        assert_eq!(
            to_shell_native_display_path(r"C:\home\user\file.txt", None),
            r"C:\home\user\file.txt"
        );
    }

    #[test]
    fn already_unix_path_is_unaffected() {
        assert_eq!(
            to_shell_native_display_path("/home/user/file.txt", wsl_shell().as_ref()),
            "/home/user/file.txt"
        );
        assert_eq!(
            to_shell_native_display_path("/c/Users/username", msys2_shell().as_ref()),
            "/c/Users/username"
        );
    }
}

/// The execution path (`shell_native_absolute_path`, used to build the literal string executed by
/// grep/file_glob) must never apply the host-native reversal: shape isn't provenance, so a
/// legitimate Unix filename that merely looks Windows-shaped must resolve to the exact same path
/// as before, not be silently rewritten into a different file.
#[test]
fn shell_native_absolute_path_leaves_windows_shaped_filenames_untouched_under_wsl() {
    let wsl_shell = Some(ShellLaunchData::WSL {
        distro: "Ubuntu".to_string(),
    });
    let cwd = Some("/home/user".to_string());
    // A relative filename that happens to look like a Windows drive-letter path must join under
    // the cwd unchanged, not be reinterpreted as a Windows path and reversed.
    assert_eq!(
        shell_native_absolute_path(r"C:\foo", wsl_shell.as_ref(), cwd.as_ref()),
        r"/home/user/C:\foo"
    );
}

#[test]
fn shell_native_absolute_path_leaves_windows_shaped_filenames_untouched_under_msys2() {
    let msys2_shell = Some(ShellLaunchData::MSYS2 {
        executable_path: PathBuf::from(r"C:\Program Files\Git\usr\bin\bash.exe"),
        shell_type: ShellType::Bash,
    });
    let cwd = Some("/c/Users/username".to_string());
    assert_eq!(
        shell_native_absolute_path(r"C:\foo", msys2_shell.as_ref(), cwd.as_ref()),
        r"/c/Users/username/C:\foo"
    );
}

#[cfg(unix)]
#[test]
fn test_host_native_absolute_path() {
    // Test with absolute path
    assert_eq!(
        host_native_absolute_path(
            "/home/user/file.txt",
            &None,
            &Some("/current/dir".to_string())
        ),
        "/home/user/file.txt"
    );

    // Test with relative path
    assert_eq!(
        host_native_absolute_path("file.txt", &None, &Some("/current/dir".to_string())),
        "/current/dir/file.txt"
    );

    // Test with tilde expansion
    assert_eq!(
        host_native_absolute_path("~/file.txt", &None, &Some("/current/dir".to_string())),
        shellexpand::tilde("~/file.txt").into_owned()
    );

    // Test with ..
    assert_eq!(
        host_native_absolute_path("../user/file.txt", &None, &Some("/current/dir".to_string())),
        "/current/user/file.txt"
    );

    // Test with .
    assert_eq!(
        host_native_absolute_path("./user/file.txt", &None, &Some("/current/dir".to_string())),
        "/current/dir/user/file.txt"
    );

    // Test with no current working directory
    assert_eq!(
        host_native_absolute_path("file.txt", &None, &None),
        "file.txt"
    );

    // Test with empty current working directory
    assert_eq!(
        host_native_absolute_path("file.txt", &None, &Some("".to_string())),
        "file.txt"
    );
}

#[cfg(windows)]
#[test]
fn test_host_native_absolute_path() {
    // Test with absolute path
    assert_eq!(
        host_native_absolute_path(
            r"C:\home\user\file.txt",
            &None,
            &Some(r"C:\current\dir".to_string())
        ),
        r"C:\home\user\file.txt"
    );

    // Test with relative path
    assert_eq!(
        host_native_absolute_path("file.txt", &None, &Some(r"C:\current\dir".to_string())),
        r"C:\current\dir\file.txt"
    );

    // Test with tilde expansion
    assert_eq!(
        host_native_absolute_path(r"~\file.txt", &None, &Some(r"C:\current\dir".to_string())),
        shellexpand::tilde(r"~\file.txt").into_owned()
    );

    // Test with ..
    assert_eq!(
        host_native_absolute_path(
            r"..\user\file.txt",
            &None,
            &Some(r"C:\current\dir".to_string())
        ),
        r"C:\current\user\file.txt"
    );

    // Test with .
    assert_eq!(
        host_native_absolute_path(
            r".\user\file.txt",
            &None,
            &Some(r"C:\current\dir".to_string())
        ),
        r"C:\current\dir\user\file.txt"
    );

    // Test with no current working directory
    assert_eq!(
        host_native_absolute_path("file.txt", &None, &None),
        "file.txt"
    );

    // Test with empty current working directory
    assert_eq!(
        host_native_absolute_path("file.txt", &None, &Some("".to_string())),
        "file.txt"
    );
}

#[cfg(windows)]
#[test]
fn test_git_bash_paths() {
    let executable_path = PathBuf::from(r"C:\Program Files\Git\usr\bin\bash.exe");
    let git_bash_shell = Some(ShellLaunchData::MSYS2 {
        executable_path,
        shell_type: ShellType::Bash,
    });

    assert_eq!(
        host_native_absolute_path(
            "/c/Users/username/project/file.txt",
            &git_bash_shell,
            &Some("/c/Users/username".to_string())
        ),
        r"c:\Users\username\project\file.txt"
    );

    assert_eq!(
        host_native_absolute_path(
            "project/file.txt",
            &git_bash_shell,
            &Some("/c/Users/username".to_string())
        ),
        r"c:\Users\username\project\file.txt"
    );

    assert_eq!(
        host_native_absolute_path(
            "../project/file.txt",
            &git_bash_shell,
            &Some("/c/Users/username/docs".to_string())
        ),
        r"c:\Users\username\project\file.txt"
    );
}

#[cfg(windows)]
#[test]
fn test_wsl_paths() {
    let wsl_shell = Some(ShellLaunchData::WSL {
        distro: "Ubuntu".to_string(),
    });

    assert_eq!(
        host_native_absolute_path(
            "/mnt/c/Users/username/project/file.txt",
            &wsl_shell,
            &Some("/mnt/c/Users/username".to_string())
        ),
        r"c:\Users\username\project\file.txt"
    );

    assert_eq!(
        host_native_absolute_path(
            "project/file.txt",
            &wsl_shell,
            &Some("/mnt/c/Users/username".to_string())
        ),
        r"c:\Users\username\project\file.txt"
    );

    assert_eq!(
        host_native_absolute_path(
            "../project/file.txt",
            &wsl_shell,
            &Some("/mnt/c/Users/username/docs".to_string())
        ),
        r"c:\Users\username\project\file.txt"
    );

    assert_eq!(
        host_native_absolute_path(
            "/home/user/file.txt",
            &wsl_shell,
            &Some("/mnt/c/Users/username".to_string())
        ),
        r"\\WSL$\Ubuntu\home\user\file.txt"
    );
}

#[cfg(unix)]
#[test]
fn test_shell_native_absolute_path() {
    // Test with absolute path
    let cwd = Some("/current/dir".to_string());
    assert_eq!(
        shell_native_absolute_path("/home/user/file.txt", None, cwd.as_ref()),
        "/home/user/file.txt"
    );

    // Test with relative path
    let cwd = Some("/current/dir".to_string());
    assert_eq!(
        shell_native_absolute_path("file.txt", None, cwd.as_ref()),
        "/current/dir/file.txt"
    );

    // Test with tilde expansion
    let cwd = Some("/current/dir".to_string());
    assert_eq!(
        shell_native_absolute_path("~/file.txt", None, cwd.as_ref()),
        shellexpand::tilde("~/file.txt").into_owned()
    );

    // Test with ..
    let cwd = Some("/current/dir".to_string());
    assert_eq!(
        shell_native_absolute_path("../user/file.txt", None, cwd.as_ref()),
        "/current/user/file.txt"
    );

    // Test with .
    let cwd = Some("/current/dir".to_string());
    assert_eq!(
        shell_native_absolute_path("./user/file.txt", None, cwd.as_ref()),
        "/current/dir/user/file.txt"
    );

    // Test with no current working directory
    assert_eq!(
        shell_native_absolute_path("file.txt", None, None),
        "file.txt"
    );

    // Test with empty current working directory
    let cwd = Some("".to_string());
    assert_eq!(
        shell_native_absolute_path("file.txt", None, cwd.as_ref()),
        "file.txt"
    );
}

#[cfg(windows)]
#[test]
fn test_shell_native_absolute_path() {
    // Test with absolute path
    let cwd = Some(r"C:\current\dir".to_string());
    assert_eq!(
        shell_native_absolute_path(r"C:\home\user\file.txt", None, cwd.as_ref()),
        r"C:\home\user\file.txt"
    );

    // Test with relative path
    let cwd = Some(r"C:\current\dir".to_string());
    assert_eq!(
        shell_native_absolute_path("file.txt", None, cwd.as_ref()),
        r"C:\current\dir\file.txt"
    );

    // Test with tilde expansion
    let cwd = Some(r"C:\current\dir".to_string());
    assert_eq!(
        shell_native_absolute_path(r"~\file.txt", None, cwd.as_ref()),
        shellexpand::tilde(r"~\file.txt").into_owned()
    );

    // Test with ..
    let cwd = Some(r"C:\current\dir".to_string());
    assert_eq!(
        shell_native_absolute_path(r"..\user\file.txt", None, cwd.as_ref()),
        r"C:\current\user\file.txt"
    );

    // Test with .
    let cwd = Some(r"C:\current\dir".to_string());
    assert_eq!(
        shell_native_absolute_path(r".\user\file.txt", None, cwd.as_ref()),
        r"C:\current\dir\user\file.txt"
    );

    // Test with no current working directory
    assert_eq!(
        shell_native_absolute_path("file.txt", None, None),
        "file.txt"
    );

    // Test with empty current working directory
    let cwd = Some("".to_string());
    assert_eq!(
        shell_native_absolute_path("file.txt", None, cwd.as_ref()),
        "file.txt"
    );
}

#[cfg(windows)]
#[test]
fn test_shell_native_git_bash_paths() {
    let executable_path = PathBuf::from(r"C:\Program Files\Git\usr\bin\bash.exe");
    let git_bash_shell = Some(ShellLaunchData::MSYS2 {
        executable_path,
        shell_type: ShellType::Bash,
    });

    // In shell_native_absolute_path, MSYS2 paths should remain in Unix format
    let cwd = Some("/c/Users/username".to_string());
    assert_eq!(
        shell_native_absolute_path(
            "/c/Users/username/project/file.txt",
            git_bash_shell.as_ref(),
            cwd.as_ref()
        ),
        "/c/Users/username/project/file.txt"
    );

    let cwd = Some("/c/Users/username".to_string());
    assert_eq!(
        shell_native_absolute_path("project/file.txt", git_bash_shell.as_ref(), cwd.as_ref()),
        "/c/Users/username/project/file.txt"
    );

    let cwd = Some("/c/Users/username/docs".to_string());
    assert_eq!(
        shell_native_absolute_path("../project/file.txt", git_bash_shell.as_ref(), cwd.as_ref()),
        "/c/Users/username/project/file.txt"
    );
}

#[cfg(windows)]
#[test]
fn test_shell_native_wsl_paths() {
    let wsl_shell = Some(ShellLaunchData::WSL {
        distro: "Ubuntu".to_string(),
    });

    // In shell_native_absolute_path, WSL paths should remain in Unix format
    let cwd = Some("/mnt/c/Users/username".to_string());
    assert_eq!(
        shell_native_absolute_path(
            "/mnt/c/Users/username/project/file.txt",
            wsl_shell.as_ref(),
            cwd.as_ref()
        ),
        "/mnt/c/Users/username/project/file.txt"
    );

    let cwd = Some("/mnt/c/Users/username".to_string());
    assert_eq!(
        shell_native_absolute_path("project/file.txt", wsl_shell.as_ref(), cwd.as_ref()),
        "/mnt/c/Users/username/project/file.txt"
    );

    let cwd = Some("/mnt/c/Users/username/docs".to_string());
    assert_eq!(
        shell_native_absolute_path("../project/file.txt", wsl_shell.as_ref(), cwd.as_ref()),
        "/mnt/c/Users/username/project/file.txt"
    );

    let cwd = Some("/mnt/c/Users/username".to_string());
    assert_eq!(
        shell_native_absolute_path("/home/user/file.txt", wsl_shell.as_ref(), cwd.as_ref()),
        "/home/user/file.txt"
    );
}
