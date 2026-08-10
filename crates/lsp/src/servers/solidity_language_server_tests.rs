use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use super::{executable_exists_on_path, is_executable_file};

#[test]
fn executable_exists_on_path_finds_executable_file() {
    let dir = tempfile_dir("solidity-lsp-exec");
    let binary = dir.join("nomicfoundation-solidity-language-server");
    fs::write(&binary, "#!/bin/sh\n").unwrap();
    let mut perms = fs::metadata(&binary).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&binary, perms).unwrap();

    let path = dir.to_string_lossy().into_owned();
    assert!(executable_exists_on_path(
        Some(&path),
        "nomicfoundation-solidity-language-server"
    ));
}

#[test]
fn executable_exists_on_path_ignores_non_executable_file() {
    let dir = tempfile_dir("solidity-lsp-nonexec");
    let binary = dir.join("nomicfoundation-solidity-language-server");
    fs::write(&binary, "not executable").unwrap();
    let mut perms = fs::metadata(&binary).unwrap().permissions();
    perms.set_mode(0o644);
    fs::set_permissions(&binary, perms).unwrap();

    let path = dir.to_string_lossy().into_owned();
    assert!(!executable_exists_on_path(
        Some(&path),
        "nomicfoundation-solidity-language-server"
    ));
    assert!(!is_executable_file(&binary));
}

#[test]
fn executable_exists_on_path_returns_false_when_missing() {
    let dir = tempfile_dir("solidity-lsp-missing");
    let path = dir.to_string_lossy().into_owned();
    assert!(!executable_exists_on_path(
        Some(&path),
        "nomicfoundation-solidity-language-server"
    ));
    assert!(!executable_exists_on_path(
        None,
        "nomicfoundation-solidity-language-server"
    ));
}

fn tempfile_dir(prefix: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("{prefix}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}
