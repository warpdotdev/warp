#[cfg(windows)]
use std::path::PathBuf;

#[cfg(windows)]
use warp_terminal::shell::{ShellLaunchData, ShellType};

use super::*;

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
