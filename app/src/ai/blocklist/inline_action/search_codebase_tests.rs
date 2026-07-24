use std::ops::Range;

use warp_core::features::FeatureFlag;

use super::*;
use crate::ai::agent::AnyFileContent;

fn native_path(path: &str) -> String {
    path.replace('/', std::path::MAIN_SEPARATOR_STR)
}

fn file_context(path: &std::path::Path, line_range: Option<Range<usize>>) -> FileContext {
    FileContext::new(
        path.to_string_lossy().to_string(),
        AnyFileContent::StringContent("fn main() {}\n".to_string()),
        line_range,
        None,
    )
}

#[cfg(feature = "local_fs")]
#[test]
fn search_codebase_render_and_detection_share_display_text_and_absolute_target() {
    let _relative_paths = FeatureFlag::RelativeBlocklistPaths.override_enabled(true);
    let root = tempfile::tempdir().unwrap();
    let cwd_path = root.path().join("repo").join("worktree");
    let file_path = cwd_path.join("src").join("lib.rs");
    std::fs::create_dir_all(file_path.parent().unwrap()).unwrap();
    std::fs::write(&file_path, "fn main() {}\n").unwrap();

    let cwd = cwd_path.to_string_lossy().to_string();
    let files = vec![file_context(&file_path, Some(7..9))];
    let display_files = search_codebase_display_files(&files, None, Some(&cwd));
    assert_eq!(
        display_files,
        vec![format!("{} (7-9)", native_path("src/lib.rs"))]
    );

    let location = TextLocation::Action {
        action_index: 3,
        line_index: 0,
    };
    let mut links = DetectedLinksState::default();
    detect_links(&mut links, &display_files[0], location, Some(&cwd), None);

    let detected = links
        .detected_links_by_location
        .get(&location)
        .expect("displayed search result should be detected");
    assert!(detected.detected_links.contains_key(&(0..10)));
    assert!(detected.detected_links.values().any(|detected| {
        matches!(
            &detected.link,
            DetectedLinkType::FilePath {
                absolute_path,
                line_and_column_num: Some(line_and_column_num),
            } if absolute_path == &file_path && line_and_column_num.line_num == 7
        )
    }));
}

#[test]
fn legacy_and_search_codebase_ui_paths_format_their_actual_result_shapes() {
    let _relative_paths = FeatureFlag::RelativeBlocklistPaths.override_enabled(true);
    let cwd = "/repo/worktree".to_string();
    let first = std::path::Path::new("/repo/worktree/src/lib.rs");
    let second = std::path::Path::new("/repo/worktree/src/other.rs");
    let files = vec![
        file_context(first, Some(40..45)),
        file_context(second, None),
        file_context(first, Some(10..20)),
    ];

    assert_eq!(
        search_codebase_display_files(&files, None, Some(&cwd)),
        vec![
            format!("{} (40-45)", native_path("src/lib.rs")),
            native_path("src/other.rs"),
            format!("{} (10-20)", native_path("src/lib.rs")),
        ]
    );
    assert_eq!(
        grouped_search_codebase_display_files(&files, None, Some(&cwd)),
        vec![
            format!("{} (10-20, 40-45)", native_path("src/lib.rs")),
            native_path("src/other.rs"),
        ]
    );
}
