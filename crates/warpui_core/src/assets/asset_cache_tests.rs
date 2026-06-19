use super::{AssetSource, LocalFileContentVersion};

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
            assert!(
                content_version.is_some(),
                "expected a content version for an existing file"
            );
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
