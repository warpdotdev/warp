use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::DateTime;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{CacheMetadataError, CachePathUsage, write_cache_metadata};
#[cfg(unix)]
use super::{MetadataDirectory, usage_entries};
use crate::spacectl::Mount;

fn mount(root: &Path, path: &str, mode: &str, target: &str) -> Mount {
    Mount {
        mode: mode.to_owned(),
        cache_path: root.join(path),
        mount_path: PathBuf::from(target),
        cache_hit: false,
    }
}

#[test]
fn nonnormalized_root_still_records_normalized_spacectl_paths() {
    let root = tempfile::tempdir().unwrap();
    let mounts = [mount(
        root.path(),
        "repos/key/target",
        "rust",
        "/work/target",
    )];

    write_cache_metadata(&root.path().join("child/.."), &mounts, []).unwrap();

    assert_eq!(
        read_document(root.path())["userRequest"],
        json!({"repos/key/target": {"source": "warp", "cacheFramework": "rust", "mountTarget": ["/work/target"]}})
    );
}

#[test]
fn redundant_relative_components_share_one_normalized_key() {
    let root = tempfile::tempdir().unwrap();
    let mounts = [mount(root.path(), "git-mirrors", "git", "/work")];

    write_cache_metadata(
        root.path(),
        &mounts,
        [(PathBuf::from("./git-mirrors//."), git_usage().1)],
    )
    .unwrap();

    assert_eq!(
        read_document(root.path())["userRequest"],
        json!({"git-mirrors": {"source": "warp", "cacheFramework": "git", "mountTarget": []}})
    );
}

#[test]
fn relative_parent_traversal_cannot_be_hidden_by_normalization() {
    let root = tempfile::tempdir().unwrap();

    assert!(matches!(
        write_cache_metadata(
            root.path(),
            &[],
            [(PathBuf::from("other/../git-mirrors"), git_usage().1)],
        ),
        Err(CacheMetadataError::InvalidPath)
    ));
    assert!(!metadata_path(root.path()).exists());
}

fn assert_malformed_unchanged(contents: &[u8]) {
    let root = tempfile::tempdir().unwrap();
    existing_document(root.path(), contents);

    assert!(matches!(
        write_cache_metadata(root.path(), &[], [git_usage()]),
        Err(CacheMetadataError::Malformed)
    ));
    assert_eq!(fs::read(metadata_path(root.path())).unwrap(), contents);
}

#[test]
fn malformed_mount_target_is_not_rewritten() {
    assert_malformed_unchanged(br#"{"version":1,"userRequest":{"other":{"mountTarget":false}}}"#);
}

#[test]
fn malformed_source_is_not_rewritten() {
    assert_malformed_unchanged(br#"{"version":1,"userRequest":{"other":{"source":false}}}"#);
}

#[test]
fn malformed_framework_is_not_rewritten() {
    assert_malformed_unchanged(
        br#"{"version":1,"userRequest":{"other":{"cache_framework":false}}}"#,
    );
}

#[test]
fn repeated_null_target_is_not_rewritten() {
    assert_malformed_unchanged(br#"{"version":1,"userRequest":{"other":{"mount_target":[null]}}}"#);
}

#[test]
fn malformed_timestamp_is_not_rewritten() {
    assert_malformed_unchanged(br#"{"version":1,"updatedAt":"not a timestamp","userRequest":{}}"#);
}

#[test]
fn nonprotobuf_timestamp_is_not_rewritten() {
    assert_malformed_unchanged(br#"{"version":1,"updatedAt":"2026-10-03t00:00:00z"}"#);
    assert_malformed_unchanged(br#"{"version":1,"updatedAt":"2026-10-03T00:00:00.1234567891Z"}"#);
    assert_malformed_unchanged(br#"{"version":1,"updatedAt":"2026-10-03T00:00:60Z"}"#);
}

#[test]
fn duplicate_usage_aliases_are_not_rewritten() {
    assert_malformed_unchanged(
        br#"{"version":1,"userRequest":{"other":{"mountTarget":[],"mount_target":[]}}}"#,
    );
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NamespaceMetadata {
    version: u32,
    #[serde(alias = "user_request")]
    user_request: BTreeMap<String, NamespaceUsage>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NamespaceUsage {
    source: Option<String>,
    #[serde(alias = "cache_framework")]
    cache_framework: Option<String>,
    #[serde(alias = "mount_target")]
    mount_target: Option<Vec<String>>,
}

#[test]
fn protobuf_shaped_document_decodes_after_merge() {
    let root = tempfile::tempdir().unwrap();
    existing_document(
        root.path(),
        br#"{
            "version":"1",
            "updated_at":null,
            "future":true,
            "user_request":{"other":{"source":null,"cache_framework":null,"mount_target":null}}
        }"#,
    );

    write_cache_metadata(root.path(), &[], [git_usage()]).unwrap();

    let document: NamespaceMetadata =
        serde_json::from_slice(&fs::read(metadata_path(root.path())).unwrap()).unwrap();
    assert_eq!(document.version, 1);
    assert_eq!(document.user_request["other"].source, None);
    assert_eq!(document.user_request["other"].cache_framework, None);
    assert_eq!(document.user_request["other"].mount_target, None);
    assert_eq!(
        document.user_request["git-mirrors"].source.as_deref(),
        Some("warp")
    );
    assert_eq!(
        document.user_request["git-mirrors"]
            .cache_framework
            .as_deref(),
        Some("git")
    );
    assert_eq!(
        document.user_request["git-mirrors"].mount_target,
        Some(Vec::new())
    );
}

#[cfg(unix)]
#[test]
fn directory_swap_cannot_redirect_metadata_update() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let outside_contents = br#"{"version":1,"userRequest":{"outside":{"source":"other"}}}"#;
    existing_document(root.path(), br#"{"version":1,"userRequest":{}}"#);
    fs::write(outside.path().join("cache-metadata.json"), outside_contents).unwrap();
    let directory = MetadataDirectory::open(root.path()).unwrap();

    fs::rename(root.path().join(".ns"), root.path().join("original")).unwrap();
    std::os::unix::fs::symlink(outside.path(), root.path().join(".ns")).unwrap();
    directory
        .update(usage_entries(root.path(), &[], [git_usage()]).unwrap())
        .unwrap();

    assert_eq!(
        fs::read(outside.path().join("cache-metadata.json")).unwrap(),
        outside_contents
    );
    let updated: Value = serde_json::from_slice(
        &fs::read(root.path().join("original/cache-metadata.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        updated["userRequest"],
        json!({"git-mirrors": {"source": "warp", "cacheFramework": "git", "mountTarget": []}})
    );
    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 1);
}

#[cfg(unix)]
#[test]
fn fifo_metadata_returns_promptly_without_reading() {
    use std::os::unix::fs::FileTypeExt as _;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join(".ns")).unwrap();
    nix::unistd::mkfifo(&metadata_path(root.path()), nix::sys::stat::Mode::S_IRUSR).unwrap();
    let path = root.path().to_owned();
    let (send, receive) = mpsc::channel();

    let worker = thread::spawn(move || {
        send.send(write_cache_metadata(&path, &[], [git_usage()]))
            .unwrap();
    });
    let result = receive.recv_timeout(Duration::from_secs(2)).unwrap();
    worker.join().unwrap();

    assert!(matches!(result, Err(CacheMetadataError::NonRegular)));
    assert!(
        fs::symlink_metadata(metadata_path(root.path()))
            .unwrap()
            .file_type()
            .is_fifo()
    );
}

fn metadata_path(root: &Path) -> PathBuf {
    root.join(".ns/cache-metadata.json")
}

fn existing_document(root: &Path, contents: &[u8]) {
    fs::create_dir(root.join(".ns")).unwrap();
    fs::write(metadata_path(root), contents).unwrap();
}

fn read_document(root: &Path) -> Value {
    serde_json::from_slice(&fs::read(metadata_path(root)).unwrap()).unwrap()
}

fn git_usage() -> (PathBuf, CachePathUsage) {
    (
        PathBuf::from("git-mirrors"),
        CachePathUsage {
            source: "warp".to_owned(),
            cache_framework: Some("git".to_owned()),
            mount_target: Vec::new(),
        },
    )
}

#[test]
fn merges_relative_mount_usage_without_losing_unknown_fields() {
    let root = tempfile::tempdir().unwrap();
    existing_document(
        root.path(),
        br#"{
            "version": 1,
            "future": {"enabled": true},
            "userRequest": {
                "third-party": {"source": "other", "future": 7},
                "repos/key/target": {"future": "keep", "cacheFramework": "old"}
            }
        }"#,
    );
    let mounts = [
        mount(root.path(), "repos/key/target", "rust", "/work/z/target"),
        mount(root.path(), "repos/key/target", "rust", "/work/a/target"),
        mount(root.path(), "repos/key/target", "rust", "/work/z/target"),
    ];

    write_cache_metadata(root.path(), &mounts, [git_usage()]).unwrap();

    let document = read_document(root.path());
    assert_eq!(document["version"], 1);
    assert_eq!(document["future"], json!({"enabled": true}));
    assert_eq!(
        document["userRequest"]["third-party"],
        json!({"source": "other", "future": 7})
    );
    assert_eq!(
        document["userRequest"]["repos/key/target"],
        json!({
            "source": "warp",
            "cacheFramework": "rust",
            "mountTarget": ["/work/a/target", "/work/z/target"],
            "future": "keep"
        })
    );
    assert_eq!(
        document["userRequest"]["git-mirrors"],
        json!({"source": "warp", "cacheFramework": "git", "mountTarget": []})
    );
    let updated_at = document["updatedAt"].as_str().unwrap();
    assert!(updated_at.ends_with('Z'));
    assert_eq!(
        DateTime::parse_from_rfc3339(updated_at)
            .unwrap()
            .offset()
            .local_minus_utc(),
        0
    );
    assert_eq!(fs::read_dir(root.path().join(".ns")).unwrap().count(), 1);
}

#[test]
fn preserves_snake_case_map_without_creating_a_second_map() {
    let root = tempfile::tempdir().unwrap();
    existing_document(
        root.path(),
        br#"{"version":1,"updated_at":"2026-10-03T01:00:00.123456789+01:00","user_request":{"other":{"source":"third-party"}}}"#,
    );

    write_cache_metadata(root.path(), &[], [git_usage()]).unwrap();

    let document = read_document(root.path());
    assert!(document.get("userRequest").is_none());
    assert!(document.get("updated_at").is_none());
    assert!(document["updatedAt"].is_string());
    assert_eq!(
        document["user_request"]["other"],
        json!({"source": "third-party"})
    );
    assert_eq!(
        document["user_request"]["git-mirrors"],
        json!({"source": "warp", "cacheFramework": "git", "mountTarget": []})
    );
}

#[test]
fn ambiguous_or_empty_modes_omit_framework_and_sort_targets() {
    let root = tempfile::tempdir().unwrap();
    existing_document(
        root.path(),
        br#"{"version":1,"userRequest":{"shared/cache":{"cache_framework":"old","mount_target":[]}}}"#,
    );
    let mounts = [
        mount(root.path(), "shared/cache", "rust", "/work/z"),
        mount(root.path(), "shared/cache", "go", "/work/a"),
        mount(root.path(), "shared/manual", "", "/work/manual"),
    ];

    write_cache_metadata(root.path(), &mounts, []).unwrap();

    let document = read_document(root.path());
    assert_eq!(
        document["userRequest"]["shared/cache"],
        json!({"source": "warp", "mountTarget": ["/work/a", "/work/z"]})
    );
    assert_eq!(
        document["userRequest"]["shared/manual"],
        json!({"source": "warp", "mountTarget": ["/work/manual"]})
    );
}

#[test]
fn malformed_document_remains_unchanged() {
    let root = tempfile::tempdir().unwrap();
    existing_document(root.path(), b"{not json");

    assert!(matches!(
        write_cache_metadata(root.path(), &[], [git_usage()]),
        Err(CacheMetadataError::Malformed)
    ));
    assert_eq!(fs::read(metadata_path(root.path())).unwrap(), b"{not json");
}

#[test]
fn unsupported_version_remains_unchanged() {
    let root = tempfile::tempdir().unwrap();
    let contents = br#"{"version":2,"userRequest":{}}"#;
    existing_document(root.path(), contents);

    assert!(matches!(
        write_cache_metadata(root.path(), &[], [git_usage()]),
        Err(CacheMetadataError::UnsupportedVersion)
    ));
    assert_eq!(fs::read(metadata_path(root.path())).unwrap(), contents);
}

#[test]
fn invalid_map_remains_unchanged() {
    let root = tempfile::tempdir().unwrap();
    let contents = br#"{"version":1,"userRequest":[]}"#;
    existing_document(root.path(), contents);

    assert!(matches!(
        write_cache_metadata(root.path(), &[], [git_usage()]),
        Err(CacheMetadataError::Malformed)
    ));
    assert_eq!(fs::read(metadata_path(root.path())).unwrap(), contents);
}

#[test]
fn volume_escape_mount_is_rejected() {
    let root = tempfile::tempdir().unwrap();
    let mounts = [Mount {
        cache_path: root.path().join("../outside"),
        ..mount(root.path(), "unused", "rust", "/work/target")
    }];

    assert!(matches!(
        write_cache_metadata(root.path(), &mounts, []),
        Err(CacheMetadataError::InvalidPath)
    ));
    assert!(!metadata_path(root.path()).exists());
}
#[test]
fn absolute_mount_outside_volume_is_rejected() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let mounts = [mount(outside.path(), "target", "rust", "/work/target")];

    assert!(matches!(
        write_cache_metadata(root.path(), &mounts, []),
        Err(CacheMetadataError::InvalidPath)
    ));
    assert!(!metadata_path(root.path()).exists());
}

#[test]
fn blocked_metadata_directory_returns_a_nonfatal_error() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join(".ns"), b"not a directory").unwrap();

    assert!(matches!(
        write_cache_metadata(root.path(), &[], [git_usage()]),
        Err(CacheMetadataError::Io)
    ));
    assert_eq!(
        fs::read(root.path().join(".ns")).unwrap(),
        b"not a directory"
    );
}

#[cfg(unix)]
#[test]
fn symlinked_document_is_not_followed_or_replaced() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::NamedTempFile::new().unwrap();
    fs::write(outside.path(), br#"{"version":1}"#).unwrap();
    fs::create_dir(root.path().join(".ns")).unwrap();
    std::os::unix::fs::symlink(outside.path(), metadata_path(root.path())).unwrap();

    assert!(matches!(
        write_cache_metadata(root.path(), &[], [git_usage()]),
        Err(CacheMetadataError::Symlink)
    ));
    assert!(
        fs::symlink_metadata(metadata_path(root.path()))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(fs::read(outside.path()).unwrap(), br#"{"version":1}"#);
}

#[cfg(unix)]
#[test]
fn symlinked_metadata_directory_is_not_followed() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), root.path().join(".ns")).unwrap();

    assert!(matches!(
        write_cache_metadata(root.path(), &[], [git_usage()]),
        Err(CacheMetadataError::Symlink)
    ));
    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
}
