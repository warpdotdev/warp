use std::path::PathBuf;

use repo_metadata::{RepositoryUpdate, TargetFile};

use super::*;

#[test]
fn should_refresh_metadata_ignores_ignored_file_updates() {
    let mut ignored_update = RepositoryUpdate::default();
    ignored_update
        .modified
        .insert(TargetFile::new(PathBuf::from("/repo/ignored.log"), true));
    assert!(!LocalGitRepoStatusModel::should_refresh_metadata(
        &ignored_update
    ));

    let mut tracked_update = RepositoryUpdate::default();
    tracked_update
        .modified
        .insert(TargetFile::new(PathBuf::from("/repo/src/main.rs"), false));
    assert!(LocalGitRepoStatusModel::should_refresh_metadata(
        &tracked_update
    ));

    let remote_ref_update = RepositoryUpdate {
        remote_ref_updated: true,
        ..Default::default()
    };
    assert!(LocalGitRepoStatusModel::should_refresh_metadata(
        &remote_ref_update
    ));
}

#[cfg(feature = "local_fs")]
#[test]
fn file_statuses_roll_up_to_ancestor_directories() {
    let repo = PathBuf::from("/repo");
    let statuses = vec![
        ("src/lib.rs".to_string(), GitFileStatus::Modified),
        ("src/new.rs".to_string(), GitFileStatus::Untracked),
        ("README.md".to_string(), GitFileStatus::Deleted),
    ];
    let decorations = RepoGitFileStatuses::from_relative(&repo, statuses);

    let p = |rel: &str| StandardizedPath::try_from_local(&repo.join(rel)).unwrap();

    // Files report their own status.
    assert_eq!(
        decorations.file_status(&p("src/lib.rs")),
        Some(&GitFileStatus::Modified)
    );
    assert_eq!(
        decorations.file_status(&p("src/new.rs")),
        Some(&GitFileStatus::Untracked)
    );
    assert_eq!(
        decorations.file_status(&p("README.md")),
        Some(&GitFileStatus::Deleted)
    );

    // `src/` holds a Modified and an Untracked file → Modified wins (higher priority).
    assert_eq!(
        decorations.dir_status(&p("src")),
        Some(&GitFileStatus::Modified)
    );

    // The repo root rolls up the highest priority among all descendants:
    // Deleted (3) outranks Modified (2).
    let root = StandardizedPath::try_from_local(&repo).unwrap();
    assert_eq!(decorations.dir_status(&root), Some(&GitFileStatus::Deleted));

    // Nothing rolls up above the repo root.
    let above_root = StandardizedPath::try_from_local(&PathBuf::from("/")).unwrap();
    assert!(decorations.dir_status(&above_root).is_none());
}

#[cfg(feature = "local_fs")]
#[test]
fn directory_roll_up_prefers_conflicts() {
    let repo = PathBuf::from("/repo");
    let statuses = vec![
        ("a/x.rs".to_string(), GitFileStatus::Modified),
        ("a/y.rs".to_string(), GitFileStatus::Conflicted),
    ];
    let decorations = RepoGitFileStatuses::from_relative(&repo, statuses);

    let dir = StandardizedPath::try_from_local(&repo.join("a")).unwrap();
    assert_eq!(
        decorations.dir_status(&dir),
        Some(&GitFileStatus::Conflicted)
    );
}
