#![allow(clippy::single_range_in_vec_init)]

use warp_core::features::FeatureFlag;

use super::*;
use crate::agent::action_result::{AnyFileContent, FileContext};

fn native_path(path: &str) -> String {
    path.replace('/', std::path::MAIN_SEPARATOR_STR)
}

fn file_context(name: &str, line_range: Option<Range<usize>>) -> FileContext {
    FileContext::new(
        name.to_string(),
        AnyFileContent::StringContent("one\ntwo\nthree".to_string()),
        line_range,
        None,
    )
}

#[test]
fn to_user_message_formats_relative_paths_and_ranges() {
    let _relative_paths = FeatureFlag::RelativeBlocklistPaths.override_enabled(true);
    let cwd = "/foo/bar/buzz".to_string();

    assert_eq!(
        FileLocations {
            name: "/foo/bar/buzz/src/lib.rs".to_string(),
            lines: vec![],
        }
        .to_user_message(None, Some(&cwd), None),
        native_path("src/lib.rs")
    );
    assert_eq!(
        FileLocations {
            name: "/foo/bazz.rs".to_string(),
            lines: vec![10..20, 40..45],
        }
        .to_user_message(None, Some(&cwd), None),
        format!("{} (10-20, 40-45)", native_path("../../bazz.rs"))
    );
    assert_eq!(
        FileLocations {
            name: "/outside.rs".to_string(),
            lines: vec![2..5],
        }
        .to_user_message(None, Some(&cwd), None),
        format!("{} (2-5)", native_path("/outside.rs"))
    );
}

#[test]
fn to_user_message_preserves_range_clamping_and_whole_file_suppression() {
    let _relative_paths = FeatureFlag::RelativeBlocklistPaths.override_enabled(true);
    let cwd = "/repo".to_string();
    let locations = FileLocations {
        name: "/repo/src/lib.rs".to_string(),
        lines: vec![1..100],
    };

    assert_eq!(
        locations.to_user_message(None, Some(&cwd), Some(20)),
        native_path("src/lib.rs")
    );

    let locations = FileLocations {
        name: "/repo/src/lib.rs".to_string(),
        lines: vec![5..100],
    };
    assert_eq!(
        locations.to_user_message(None, Some(&cwd), Some(20)),
        format!("{} (5-20)", native_path("src/lib.rs"))
    );
}

#[test]
fn group_file_contexts_preserves_first_occurrence_order_and_sorts_ranges() {
    let _relative_paths = FeatureFlag::RelativeBlocklistPaths.override_enabled(true);
    let cwd = "/repo".to_string();
    let contexts = vec![
        file_context("/repo/src/first.rs", Some(40..45)),
        file_context("/repo/src/second.rs", None),
        file_context("/repo/src/first.rs", Some(10..20)),
    ];

    assert_eq!(
        group_file_contexts_for_display(&contexts, None, Some(&cwd)),
        vec![
            format!("{} (10-20, 40-45)", native_path("src/first.rs")),
            native_path("src/second.rs"),
        ]
    );
}
