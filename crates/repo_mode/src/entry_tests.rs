use std::fs;
use std::path::PathBuf;

use tempfile::tempdir;

use super::*;

#[test]
fn canonicalize_dedups_trailing_slash_variants() {
    let dir = tempdir().unwrap();
    let base = dir.path().to_path_buf();
    let with_slash = PathBuf::from(format!("{}/", base.display()));

    let a = canonicalize_repo_path(&base).unwrap();
    let b = canonicalize_repo_path(&with_slash).unwrap();
    assert_eq!(a, b);
}

#[test]
fn classify_repo_vs_folder() {
    let dir = tempdir().unwrap();
    let folder = dir.path().join("plain");
    fs::create_dir(&folder).unwrap();
    assert_eq!(classify_entry_kind(&folder), RepoEntryKind::Folder);

    let repo = dir.path().join("gitrepo");
    fs::create_dir(&repo).unwrap();
    fs::create_dir(repo.join(".git")).unwrap();
    assert_eq!(classify_entry_kind(&repo), RepoEntryKind::Repo);
}

#[test]
fn dead_path_check() {
    let dir = tempdir().unwrap();
    let existing = dir.path().join("exists");
    fs::create_dir(&existing).unwrap();
    assert!(!is_dead_path(&existing));

    let missing = dir.path().join("gone");
    assert!(is_dead_path(&missing));
}

#[test]
fn repo_entry_from_path_sets_display_name() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("my-project");
    fs::create_dir(&path).unwrap();
    let entry = RepoEntry::from_path(&path).unwrap();
    assert_eq!(entry.display_name, "my-project");
    assert_eq!(entry.kind, RepoEntryKind::Folder);
}
