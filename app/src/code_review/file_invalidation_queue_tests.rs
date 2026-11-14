use warp_core::sync_queue::SyncQueueTaskTrait;

use super::*;

/// Regression test for a bug where the failure path converted the
/// repo-relative path with `to_string_lossy()` before returning it, while
/// callers dedupe/remove entries by the exact `PathBuf` they inserted.
/// `retrieve_diff_state` rejects non-UTF-8 paths before ever producing a
/// success result, so a lossy string round-trip here would never match the
/// original `PathBuf`, stranding the entry in the caller's dedup set.
#[cfg(unix)]
#[tokio::test]
async fn run_reports_exact_path_for_non_utf8_file() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let repo_dir = tempfile::tempdir().expect("create temp repo dir");
    let repo_path = repo_dir.path().to_path_buf();

    // A filename containing an invalid UTF-8 byte. Regular filesystems on
    // Linux treat filenames as raw bytes, so this is a legal file to create.
    let non_utf8_name = OsStr::from_bytes(b"bad-\xffname.txt");
    let file = repo_path.join(non_utf8_name);
    std::fs::write(&file, b"content").expect("write file with non-UTF-8 name");

    let mut task = FileInvalidationTask {
        file: file.clone(),
        repo_path: repo_path.clone(),
        mode: DiffMode::Head,
        merge_base: None,
    };

    let result = task.run().await;
    let err = match result {
        Err(err) => err,
        Ok(_) => panic!("expected a non-UTF-8 path to fail to produce a diff state"),
    };

    // The error must carry the *exact* relative PathBuf, not a lossy,
    // string-converted approximation, so a caller can remove the matching
    // dedup entry by equality.
    let expected_relative = file
        .strip_prefix(&repo_path)
        .expect("file should be under repo_path")
        .to_path_buf();
    assert_eq!(err.path, expected_relative);
}
