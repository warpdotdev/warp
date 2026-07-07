use chrono::{Duration, Utc};
use virtual_fs::{Stub, VirtualFS};

use super::*;
fn workspace_metadata(path: impl Into<PathBuf>) -> WorkspaceMetadata {
    WorkspaceMetadata {
        path: path.into(),
        navigated_ts: None,
        modified_ts: Some(Utc::now()),
        queried_ts: None,
    }
}

#[test]
#[cfg(feature = "local_fs")]
fn snapshot_storage_app_default_matches_snapshot_dir() {
    VirtualFS::test(
        "snapshot_storage_app_default_matches_snapshot_dir",
        |_dirs, _sandbox| {
            let storage = SnapshotStorage::app_default().unwrap();
            assert_eq!(storage.path(), snapshot_dir().unwrap());
            assert!(storage.is_app_default());
        },
    );
}

#[test]
fn split_snapshot_metadata_by_validity_uses_injected_snapshot_dir() {
    let snapshot_dir = tempfile::tempdir().unwrap();
    let storage = SnapshotStorage::from_dir(snapshot_dir.path().join("daemon")).unwrap();
    let repo_path = PathBuf::from("/remote/repo");
    std::fs::write(storage.snapshot_path(&repo_path), b"snapshot").unwrap();

    let (invalid_metadata, valid_metadata) =
        split_snapshot_metadata_by_validity(vec![workspace_metadata(&repo_path)], Some(&storage));

    assert!(invalid_metadata.is_empty());
    assert_eq!(valid_metadata.len(), 1);
    assert_eq!(valid_metadata[0].path, repo_path);
}

#[test]
fn split_snapshot_metadata_by_validity_rejects_missing_injected_snapshot() {
    let snapshot_dir = tempfile::tempdir().unwrap();
    let storage = SnapshotStorage::from_dir(snapshot_dir.path().join("daemon")).unwrap();
    let repo_path = PathBuf::from("/remote/repo");

    let (invalid_metadata, valid_metadata) =
        split_snapshot_metadata_by_validity(vec![workspace_metadata(&repo_path)], Some(&storage));

    assert_eq!(invalid_metadata.len(), 1);
    assert_eq!(invalid_metadata[0].path, repo_path);
    assert!(valid_metadata.is_empty());
}

#[test]
fn test_clean_up_snapshot_files() {
    VirtualFS::test("test_clean_up_snapshot_files", |dirs, mut sandbox| {
        // Create snapshot directory with test files
        sandbox.mkdir(REPO_SNAPSHOT_SUBDIR_NAME);

        // Create a valid snapshot file
        let test_path = PathBuf::from("/test/path");
        let mut hasher = DefaultHasher::new();
        test_path.hash(&mut hasher);
        let valid_snapshot_name = format!("snapshot_{}", hasher.finish());

        // Create entries in the virtual filesystem
        let mut snapshot_dir_relative_path = PathBuf::new();
        snapshot_dir_relative_path.push(REPO_SNAPSHOT_SUBDIR_NAME);
        sandbox.with_files(vec![
            // Valid snapshot file that matches metadata
            Stub::FileWithContent(
                snapshot_dir_relative_path
                    .join(&valid_snapshot_name)
                    .to_string_lossy()
                    .as_ref(),
                "valid content",
            ),
            // Expired snapshot file
            Stub::FileWithContent(
                snapshot_dir_relative_path
                    .join("snapshot_expired")
                    .to_string_lossy()
                    .as_ref(),
                "expired content",
            ),
            // Non-snapshot file that should be ignored
            Stub::FileWithContent(
                snapshot_dir_relative_path
                    .join("regular_file.txt")
                    .to_string_lossy()
                    .as_ref(),
                "regular content",
            ),
        ]);
        // Subdirectory with the 'snapshot_' prefix that should be ignored
        let invalid_subdir_path = snapshot_dir_relative_path.join("snapshot_prefixed_directory");
        sandbox.mkdir(invalid_subdir_path.to_string_lossy().to_string().as_str());

        let snapshot_dir_absolute_path = dirs.tests().join(REPO_SNAPSHOT_SUBDIR_NAME);

        let valid_snapshot_file = snapshot_dir_absolute_path.join(&valid_snapshot_name);
        let expired_file = snapshot_dir_absolute_path.join("snapshot_expired");
        let regular_file = snapshot_dir_absolute_path.join("regular_file.txt");
        let prefixed_subdir = snapshot_dir_absolute_path.join("snapshot_prefixed_directory");

        assert!(valid_snapshot_file.is_file());
        assert!(expired_file.is_file());
        assert!(regular_file.is_file());
        assert!(prefixed_subdir.is_dir());

        // Set the expired file to be older than shelf life
        let old_time = std::time::SystemTime::now()
            - REPO_SNAPSHOT_SHELF_LIFE_DURATION
            - Duration::days(1).to_std().unwrap();
        filetime::set_file_mtime(
            &expired_file,
            filetime::FileTime::from_system_time(old_time),
        )
        .unwrap();

        // Create test metadata that only includes the valid file
        let metadata = vec![workspace_metadata(test_path)];

        // Run cleanup
        clean_up_snapshot_files(&snapshot_dir_absolute_path, &metadata);

        // Valid snapshot should still exist
        assert!(valid_snapshot_file.exists());

        // Expired snapshot should be deleted
        assert!(!expired_file.exists());

        // Regular file should remain untouched
        assert!(regular_file.exists());

        // Subdirectory with 'snapshot_' prefix should remain untouched
        assert!(prefixed_subdir.exists());
    });
}

#[test]
fn test_clean_up_snapshot_files_no_snapshot_dir() {
    VirtualFS::test("test_clean_up_snapshot_files_no_dir", |dirs, _sandbox| {
        // Test with empty metadata when snapshot directory doesn't exist
        let snapshot_metadata = vec![];
        let snapshot_directory = dirs.tests().join(REPO_SNAPSHOT_SUBDIR_NAME);
        clean_up_snapshot_files(&snapshot_directory, &snapshot_metadata);
        assert!(snapshot_metadata.is_empty());
    });
}
