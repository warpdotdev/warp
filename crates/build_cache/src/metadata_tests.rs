use std::error::Error as _;
use std::path::{Path, PathBuf};
use std::{fs, io};

use chrono::DateTime;
use serde_json::{Value, json};

use super::{CacheMetadata, CacheMetadataError, CacheUsage, write_cache_metadata};

fn usage(path: &str, mode: &str, targets: &[&str]) -> CacheUsage {
    CacheUsage {
        path: PathBuf::from(path),
        cache_framework: Some(mode.to_owned()),
        mount_target: targets.iter().map(|target| (*target).to_owned()).collect(),
    }
}

fn metadata_path(root: &Path) -> PathBuf {
    root.join(".ns").join("cache-metadata.json")
}

fn existing_document(root: &Path, contents: &[u8]) {
    fs::create_dir(root.join(".ns")).unwrap();
    fs::write(metadata_path(root), contents).unwrap();
}

fn read_document(root: &Path) -> Value {
    serde_json::from_slice(&fs::read(metadata_path(root)).unwrap()).unwrap()
}

fn assert_io_diagnostics(
    error: &CacheMetadataError,
    operation: &str,
    path: &Path,
    expected_source: &io::Error,
) {
    let source = error.source().unwrap().downcast_ref::<io::Error>().unwrap();
    assert_eq!(source.kind(), expected_source.kind());
    assert_eq!(source.raw_os_error(), expected_source.raw_os_error());
    let message = error.to_string();
    assert!(message.contains(operation), "{message}");
    assert!(message.contains(&format!("{path:?}")), "{message}");
    assert!(message.contains(&source.to_string()), "{message}");
}

#[test]
fn snapshot_records_mixed_usage_with_sorted_unique_targets() {
    let root = tempfile::tempdir().unwrap();
    let usages = [
        usage("repos/key/target", "rust", &["/work/z/target"]),
        usage(
            "repos/key/target",
            "rust",
            &["/work/a/target", "/work/z/target"],
        ),
        usage("git-mirrors", "git", &[]),
    ];

    write_cache_metadata(root.path(), usages).unwrap();

    let document: CacheMetadata =
        serde_json::from_slice(&fs::read(metadata_path(root.path())).unwrap()).unwrap();
    assert_eq!(document.version, 1);
    assert_eq!(document.user_request.len(), 2);
    let build = &document.user_request["repos/key/target"];
    assert_eq!(build.source, "warp");
    assert_eq!(build.cache_framework.as_deref(), Some("rust"));
    assert_eq!(build.mount_target, ["/work/a/target", "/work/z/target"]);
    let git = &document.user_request["git-mirrors"];
    assert_eq!(git.source, "warp");
    assert_eq!(git.cache_framework.as_deref(), Some("git"));
    assert!(git.mount_target.is_empty());
    assert!(document.updated_at.ends_with('Z'));
    assert_eq!(
        DateTime::parse_from_rfc3339(&document.updated_at)
            .unwrap()
            .offset()
            .local_minus_utc(),
        0
    );
    assert_eq!(fs::read_dir(root.path().join(".ns")).unwrap().count(), 1);
}

#[test]
fn snapshot_omits_inherited_usage() {
    let root = tempfile::tempdir().unwrap();
    existing_document(
        root.path(),
        br#"{"version":1,"future":true,"user_request":{"stale":{"source":"other"}}}"#,
    );

    write_cache_metadata(root.path(), [usage("git-mirrors", "git", &[])]).unwrap();

    let document = read_document(root.path());
    assert_eq!(
        document["userRequest"],
        json!({"git-mirrors": {"source": "warp", "cacheFramework": "git", "mountTarget": []}})
    );
    assert!(document.get("future").is_none());
    assert!(document.get("user_request").is_none());
}

#[test]
fn malformed_document_is_replaced() {
    let root = tempfile::tempdir().unwrap();
    existing_document(root.path(), b"{not json");

    write_cache_metadata(root.path(), [usage("git-mirrors", "git", &[])]).unwrap();

    assert_eq!(
        read_document(root.path())["userRequest"],
        json!({"git-mirrors": {"source": "warp", "cacheFramework": "git", "mountTarget": []}})
    );
}

#[test]
fn unsupported_version_is_replaced() {
    let root = tempfile::tempdir().unwrap();
    existing_document(root.path(), br#"{"version":2,"userRequest":{"old":{}}}"#);

    write_cache_metadata(root.path(), []).unwrap();

    let document = read_document(root.path());
    assert_eq!(document["version"], 1);
    assert_eq!(document["userRequest"], json!({}));
}

#[test]
fn redundant_relative_components_share_one_normalized_key() {
    let root = tempfile::tempdir().unwrap();
    let usages = [
        usage("./shared//cache/.", "rust", &["/work/a"]),
        usage("shared/cache", "rust", &["/work/b"]),
    ];

    write_cache_metadata(&root.path().join("child/.."), usages).unwrap();

    assert_eq!(
        read_document(root.path())["userRequest"],
        json!({"shared/cache": {"source": "warp", "cacheFramework": "rust", "mountTarget": ["/work/a", "/work/b"]}})
    );
}

#[test]
fn ambiguous_or_empty_modes_omit_framework() {
    let root = tempfile::tempdir().unwrap();
    let usages = [
        usage("shared/cache", "rust", &["/work/z"]),
        usage("shared/cache", "go", &["/work/a"]),
        usage("shared/manual", "", &["/work/manual"]),
        usage("shared/mixed", "rust", &["/work/known"]),
        usage("shared/mixed", "", &["/work/unknown"]),
    ];

    write_cache_metadata(root.path(), usages).unwrap();

    assert_eq!(
        read_document(root.path())["userRequest"],
        json!({
            "shared/cache": {"source": "warp", "mountTarget": ["/work/a", "/work/z"]},
            "shared/manual": {"source": "warp", "mountTarget": ["/work/manual"]},
            "shared/mixed": {"source": "warp", "mountTarget": ["/work/known", "/work/unknown"]}
        })
    );
}

#[test]
fn relative_parent_traversal_is_rejected() {
    let root = tempfile::tempdir().unwrap();

    assert!(matches!(
        write_cache_metadata(root.path(), [usage("other/../git-mirrors", "git", &[])]),
        Err(CacheMetadataError::InvalidPath)
    ));
    assert!(!metadata_path(root.path()).exists());
}

#[test]
fn absolute_usage_path_is_rejected() {
    let root = tempfile::tempdir().unwrap();
    let absolute = CacheUsage {
        path: root.path().join("target"),
        ..usage("unused", "rust", &[])
    };

    assert!(matches!(
        write_cache_metadata(root.path(), [absolute]),
        Err(CacheMetadataError::InvalidPath)
    ));
    assert!(!metadata_path(root.path()).exists());
}

#[test]
fn empty_relative_key_is_rejected() {
    let root = tempfile::tempdir().unwrap();

    assert!(matches!(
        write_cache_metadata(root.path(), [usage(".", "rust", &[])]),
        Err(CacheMetadataError::InvalidPath)
    ));
    assert!(!metadata_path(root.path()).exists());
}

#[test]
fn blocked_metadata_directory_returns_an_error() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join(".ns");
    fs::write(&directory, b"not a directory").unwrap();
    let expected_source = fs::create_dir_all(&directory).unwrap_err();

    let error = write_cache_metadata(root.path(), []).unwrap_err();

    assert_io_diagnostics(&error, "create directory", &directory, &expected_source);
    assert_eq!(
        fs::read(root.path().join(".ns")).unwrap(),
        b"not a directory"
    );
}

#[test]
fn blocked_cache_root_reports_failed_operation() {
    let root = tempfile::NamedTempFile::new().unwrap();
    let directory = root.path().join(".ns");
    #[cfg(not(windows))]
    let (operation, expected_source) = (
        "inspect directory",
        fs::symlink_metadata(&directory).unwrap_err(),
    );
    #[cfg(windows)]
    let (operation, expected_source) = (
        "create directory",
        fs::create_dir_all(&directory).unwrap_err(),
    );

    let error = write_cache_metadata(root.path(), []).unwrap_err();

    assert_io_diagnostics(&error, operation, &directory, &expected_source);
}

#[test]
fn empty_cache_root_reports_normalization_failure() {
    let path = Path::new("");
    let expected_source = std::path::absolute(path).unwrap_err();

    let error = write_cache_metadata(path, []).unwrap_err();

    assert_io_diagnostics(&error, "normalize cache root", path, &expected_source);
}

#[test]
fn failed_replacement_removes_temporary_file() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir_all(metadata_path(root.path())).unwrap();
    let sentinel = metadata_path(root.path()).join("keep");
    fs::write(&sentinel, b"unchanged").unwrap();

    let probe = tempfile::NamedTempFile::new_in(root.path().join(".ns")).unwrap();
    let expected_source = probe.persist(metadata_path(root.path())).unwrap_err().error;

    let error = write_cache_metadata(root.path(), [usage("git-mirrors", "git", &[])]).unwrap_err();

    assert_io_diagnostics(
        &error,
        "persist metadata file",
        &metadata_path(root.path()),
        &expected_source,
    );

    assert_eq!(fs::read(sentinel).unwrap(), b"unchanged");
    assert_eq!(fs::read_dir(root.path().join(".ns")).unwrap().count(), 1);
}

#[cfg(unix)]
#[test]
fn final_symlink_is_replaced_without_changing_its_target() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::NamedTempFile::new().unwrap();
    fs::write(outside.path(), b"unchanged").unwrap();
    fs::create_dir(root.path().join(".ns")).unwrap();
    std::os::unix::fs::symlink(outside.path(), metadata_path(root.path())).unwrap();

    write_cache_metadata(root.path(), [usage("git-mirrors", "git", &[])]).unwrap();

    assert!(
        fs::symlink_metadata(metadata_path(root.path()))
            .unwrap()
            .is_file()
    );
    assert_eq!(fs::read(outside.path()).unwrap(), b"unchanged");
    assert_eq!(
        read_document(root.path())["userRequest"],
        json!({"git-mirrors": {"source": "warp", "cacheFramework": "git", "mountTarget": []}})
    );
}

#[test]
fn hardlinked_document_is_replaced_without_changing_other_links() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::NamedTempFile::new().unwrap();
    fs::write(outside.path(), b"unchanged").unwrap();
    fs::create_dir(root.path().join(".ns")).unwrap();
    fs::hard_link(outside.path(), metadata_path(root.path())).unwrap();

    write_cache_metadata(root.path(), []).unwrap();

    assert_eq!(fs::read(outside.path()).unwrap(), b"unchanged");
    assert_eq!(read_document(root.path())["userRequest"], json!({}));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(
            fs::metadata(metadata_path(root.path()))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}

#[cfg(unix)]
#[test]
fn fifo_document_is_replaced_without_opening_it() {
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join(".ns")).unwrap();
    nix::unistd::mkfifo(&metadata_path(root.path()), nix::sys::stat::Mode::S_IRUSR).unwrap();
    let path = root.path().to_owned();
    let (send, receive) = mpsc::channel();

    let worker = thread::spawn(move || {
        send.send(write_cache_metadata(
            &path,
            [usage("git-mirrors", "git", &[])],
        ))
        .unwrap();
    });
    receive
        .recv_timeout(Duration::from_secs(2))
        .unwrap()
        .unwrap();
    worker.join().unwrap();

    assert!(
        fs::symlink_metadata(metadata_path(root.path()))
            .unwrap()
            .is_file()
    );
    assert_eq!(
        read_document(root.path())["userRequest"],
        json!({"git-mirrors": {"source": "warp", "cacheFramework": "git", "mountTarget": []}})
    );
}

#[cfg(unix)]
#[test]
fn symlinked_metadata_directory_is_rejected() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), root.path().join(".ns")).unwrap();

    assert!(matches!(
        write_cache_metadata(root.path(), [usage("git-mirrors", "git", &[])]),
        Err(CacheMetadataError::Symlink)
    ));
    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
}
