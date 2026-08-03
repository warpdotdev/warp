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
fn parse_branch_tracking_counts_accepts_git_rev_list_output() {
    assert_eq!(
        LocalGitRepoStatusModel::parse_branch_tracking_counts("2\t3\n"),
        Some((2, 3, 0))
    );
    assert_eq!(
        LocalGitRepoStatusModel::parse_branch_tracking_counts("10 0 4"),
        Some((10, 0, 4))
    );
    assert_eq!(
        LocalGitRepoStatusModel::parse_branch_tracking_counts("error"),
        None
    );
}
