use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, UNIX_EPOCH};

use super::{AssetCache, AssetHandle, AssetSource, AssetStateInternal, LocalFileContentVersion};

#[cfg(not(target_arch = "wasm32"))]
fn unique_temp_path(name: &str) -> std::path::PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "warp_asset_cache_test_{}_{name}",
        std::process::id()
    ));
    path
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn local_file_content_version_changes_when_file_contents_change() {
    let path = unique_temp_path("contents_change.png");
    std::fs::write(&path, b"aaaa").expect("write temp file");
    let path_string = path.to_string_lossy().to_string();

    let source = AssetSource::LocalFile {
        path: path_string.clone(),
        content_version: None,
    }
    .with_local_file_content_version();

    match &source {
        AssetSource::LocalFile {
            path: resolved_path,
            content_version,
        } => {
            assert_eq!(resolved_path, &path_string);
            let content_version = content_version
                .as_ref()
                .expect("expected a content version for an existing file");
            assert_eq!(
                content_version.modified,
                std::fs::metadata(&path)
                    .expect("read temp file metadata")
                    .modified()
                    .ok()
            );
            assert_eq!(content_version.file_size, 4);
        }
        other => panic!("expected a local file source, got {other:?}"),
    }

    let unchanged = AssetSource::LocalFile {
        path: path_string.clone(),
        content_version: None,
    }
    .with_local_file_content_version();
    assert_eq!(
        source, unchanged,
        "an unmodified file should produce the same cache key"
    );

    std::fs::write(&path, b"bbbbbbbb").expect("rewrite temp file");
    let after_change = AssetSource::LocalFile {
        path: path_string,
        content_version: None,
    }
    .with_local_file_content_version();
    assert_ne!(
        source, after_change,
        "changing file contents should produce a different cache key"
    );

    let _ = std::fs::remove_file(&path);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn local_file_content_version_is_none_for_missing_file() {
    let path = unique_temp_path("definitely_missing.png");
    let _ = std::fs::remove_file(&path);
    assert!(
        LocalFileContentVersion::for_path(&path).is_none(),
        "a missing file should not produce a content version"
    );
}

#[test]
fn with_local_file_content_version_leaves_non_local_sources_unchanged() {
    let bundled = AssetSource::Bundled { path: "icon.svg" };
    assert_eq!(bundled.clone().with_local_file_content_version(), bundled);
}

#[test]
fn evicts_oldest_versioned_local_file_assets_over_limit() {
    let oldest_source = AssetSource::LocalFile {
        path: "oldest.png".to_string(),
        content_version: Some(LocalFileContentVersion {
            modified: Some(UNIX_EPOCH + Duration::from_secs(1)),
            file_size: 4,
        }),
    };
    let newest_source = AssetSource::LocalFile {
        path: "newest.png".to_string(),
        content_version: Some(LocalFileContentVersion {
            modified: Some(UNIX_EPOCH + Duration::from_secs(2)),
            file_size: 4,
        }),
    };
    let unversioned_source = AssetSource::LocalFile {
        path: "unversioned.png".to_string(),
        content_version: None,
    };
    let oldest_handle = AssetHandle {
        source: oldest_source.clone(),
        asset_type: TypeId::of::<String>(),
    };
    let newest_handle = AssetHandle {
        source: newest_source.clone(),
        asset_type: TypeId::of::<String>(),
    };
    let unversioned_handle = AssetHandle {
        source: unversioned_source,
        asset_type: TypeId::of::<String>(),
    };
    let mut assets = HashMap::from([
        (
            oldest_handle.clone(),
            AssetStateInternal::Loaded {
                data: Rc::new(String::new()) as Rc<dyn Any>,
                timestamp: 1,
                size_in_bytes: 4,
            },
        ),
        (
            newest_handle.clone(),
            AssetStateInternal::Loaded {
                data: Rc::new(String::new()) as Rc<dyn Any>,
                timestamp: 2,
                size_in_bytes: 4,
            },
        ),
        (
            unversioned_handle.clone(),
            AssetStateInternal::Loaded {
                data: Rc::new(String::new()) as Rc<dyn Any>,
                timestamp: 0,
                size_in_bytes: 10,
            },
        ),
    ]);

    let evicted_sources = AssetCache::evict_versioned_local_file_assets(&mut assets, 4);

    assert_eq!(evicted_sources, vec![oldest_source]);
    assert!(matches!(
        assets.get(&oldest_handle),
        Some(AssetStateInternal::Evicted)
    ));
    assert!(matches!(
        assets.get(&newest_handle),
        Some(AssetStateInternal::Loaded { .. })
    ));
    assert!(matches!(
        assets.get(&unversioned_handle),
        Some(AssetStateInternal::Loaded { .. })
    ));
}
