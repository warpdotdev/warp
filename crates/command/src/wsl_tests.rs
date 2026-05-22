use std::ffi::OsStr;
#[cfg(unix)]
use std::ffi::OsString;
#[cfg(unix)]
use std::fs;
#[cfg(unix)]
use std::path::PathBuf;

use super::{known_bare_name, resolve_binary_in_wsl_safe_path};

#[cfg(unix)]
fn make_executable(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt as _;
    let mut perms = fs::metadata(path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms).unwrap();
}

#[cfg(unix)]
fn write_exec(path: &std::path::Path) {
    fs::write(path, b"#!/bin/sh\nexit 0\n").unwrap();
    make_executable(path);
}

#[cfg(unix)]
fn join(parts: &[PathBuf]) -> OsString {
    std::env::join_paths(parts).unwrap()
}

#[cfg(unix)]
#[test]
fn picks_first_linux_path_when_wsl() {
    let linux_dir = tempfile::tempdir().unwrap();
    write_exec(&linux_dir.path().join("git"));

    // `/mnt/c/...` paths in the synthetic PATH that don't exist on
    // disk simulate the WSL-with-Windows-git layout: the resolver
    // should skip them on the prefix and never stat them.
    let path_env = join(&[
        PathBuf::from("/mnt/c/Program Files/Git/cmd"),
        linux_dir.path().to_path_buf(),
    ]);

    let resolved =
        resolve_binary_in_wsl_safe_path("git", Some(path_env.as_os_str()), true).unwrap();
    assert_eq!(resolved, linux_dir.path().join("git"));
    assert!(!resolved.starts_with("/mnt"));
}

#[cfg(unix)]
#[test]
fn passes_through_first_match_when_not_wsl() {
    // When not WSL, `/mnt/...` is just another directory; the resolver
    // should pick the first dir on PATH that contains an exec match.
    let mnt_dir = tempfile::tempdir().unwrap();
    write_exec(&mnt_dir.path().join("git"));
    let other_dir = tempfile::tempdir().unwrap();
    write_exec(&other_dir.path().join("git"));

    let path_env = join(&[mnt_dir.path().to_path_buf(), other_dir.path().to_path_buf()]);

    let resolved =
        resolve_binary_in_wsl_safe_path("git", Some(path_env.as_os_str()), false).unwrap();
    assert_eq!(resolved, mnt_dir.path().join("git"));
}

#[cfg(unix)]
#[test]
fn falls_back_to_none_when_only_mnt_has_git() {
    let path_env = join(&[
        PathBuf::from("/mnt/c/Program Files/Git/cmd"),
        PathBuf::from("/mnt/c/Windows/System32"),
    ]);
    assert!(resolve_binary_in_wsl_safe_path("git", Some(path_env.as_os_str()), true).is_none());
}

#[cfg(unix)]
#[test]
fn picks_user_local_bin() {
    let bin_dir = tempfile::tempdir().unwrap();
    let empty_dir = bin_dir.path().join("empty");
    fs::create_dir_all(&empty_dir).unwrap();
    let local_bin = bin_dir.path().join("home/.local/bin");
    fs::create_dir_all(&local_bin).unwrap();
    write_exec(&local_bin.join("git"));

    // PATH order: an empty dir, then a `/mnt/...` candidate that
    // should be skipped on WSL, then the directory with the real
    // exec. The resolver must walk past the first two and land on
    // the third.
    let path_env = join(&[
        empty_dir,
        PathBuf::from("/mnt/c/Program Files/Git/cmd"),
        local_bin.clone(),
    ]);
    let resolved =
        resolve_binary_in_wsl_safe_path("git", Some(path_env.as_os_str()), true).unwrap();
    assert_eq!(resolved, local_bin.join("git"));
}

#[cfg(unix)]
#[test]
fn follows_symlinks() {
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("git-wrapper");
    write_exec(&real);
    let link = dir.path().join("git");
    std::os::unix::fs::symlink(&real, &link).unwrap();

    let path_env = join(&[dir.path().to_path_buf()]);
    let resolved =
        resolve_binary_in_wsl_safe_path("git", Some(path_env.as_os_str()), true).unwrap();
    assert_eq!(resolved, link);
}

#[cfg(unix)]
#[test]
fn skips_non_executable() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("git"), b"not exec").unwrap();
    // Intentionally do NOT chmod +x.

    let path_env = join(&[dir.path().to_path_buf()]);
    assert!(resolve_binary_in_wsl_safe_path("git", Some(path_env.as_os_str()), true).is_none());
}

#[test]
fn handles_empty_path_env() {
    assert!(resolve_binary_in_wsl_safe_path("git", None, true).is_none());
    assert!(resolve_binary_in_wsl_safe_path("git", None, false).is_none());
}

#[cfg(unix)]
#[test]
fn handles_non_utf8_path_components() {
    use std::os::unix::ffi::OsStringExt as _;

    let dir = tempfile::tempdir().unwrap();
    write_exec(&dir.path().join("git"));

    // Build a PATH whose first component is a non-UTF-8 byte sequence,
    // followed by a real directory. The resolver must walk past the
    // garbage entry without panicking and find the valid one.
    let mut bytes = b"/\xff\xfe/bad:".to_vec();
    bytes.extend_from_slice(dir.path().as_os_str().as_encoded_bytes());
    let path_env = OsString::from_vec(bytes);

    let resolved =
        resolve_binary_in_wsl_safe_path("git", Some(path_env.as_os_str()), true).unwrap();
    assert_eq!(resolved, dir.path().join("git"));
}

#[test]
fn known_bare_name_recognizes_git_and_gh() {
    assert_eq!(known_bare_name(OsStr::new("git")), Some("git"));
    assert_eq!(known_bare_name(OsStr::new("gh")), Some("gh"));
}

#[test]
fn known_bare_name_skips_paths() {
    assert_eq!(known_bare_name(OsStr::new("/usr/bin/git")), None);
    assert_eq!(known_bare_name(OsStr::new("./git")), None);
    assert_eq!(known_bare_name(OsStr::new("bin/git")), None);
    #[cfg(windows)]
    assert_eq!(known_bare_name(OsStr::new("C:\\git\\git.exe")), None);
}

#[test]
fn known_bare_name_skips_unknowns() {
    assert_eq!(known_bare_name(OsStr::new("ls")), None);
    assert_eq!(known_bare_name(OsStr::new("python")), None);
    assert_eq!(known_bare_name(OsStr::new("")), None);
}
