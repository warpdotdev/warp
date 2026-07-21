#![allow(clippy::single_range_in_vec_init)]

use super::*;
use crate::agent::action_result::{AnyFileContent, FileContext};

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
    let cwd = "/foo/bar/buzz".to_string();

    assert_eq!(
        FileLocations {
            name: "/foo/bar/buzz/src/lib.rs".to_string(),
            lines: vec![],
        }
        .to_user_message(None, Some(&cwd), None),
        "src/lib.rs"
    );
    assert_eq!(
        FileLocations {
            name: "/foo/bazz.rs".to_string(),
            lines: vec![10..20, 40..45],
        }
        .to_user_message(None, Some(&cwd), None),
        "../../bazz.rs (10-20, 40-45)"
    );
    assert_eq!(
        FileLocations {
            name: "/outside.rs".to_string(),
            lines: vec![2..5],
        }
        .to_user_message(None, Some(&cwd), None),
        "/outside.rs (2-5)"
    );
}

#[test]
fn to_user_message_preserves_range_clamping_and_whole_file_suppression() {
    let cwd = "/repo".to_string();
    let locations = FileLocations {
        name: "/repo/src/lib.rs".to_string(),
        lines: vec![1..100],
    };

    assert_eq!(
        locations.to_user_message(None, Some(&cwd), Some(20)),
        "src/lib.rs"
    );

    let locations = FileLocations {
        name: "/repo/src/lib.rs".to_string(),
        lines: vec![5..100],
    };
    assert_eq!(
        locations.to_user_message(None, Some(&cwd), Some(20)),
        "src/lib.rs (5-20)"
    );
}

#[test]
fn group_file_contexts_preserves_first_occurrence_order_and_sorts_ranges() {
    let cwd = "/repo".to_string();
    let contexts = vec![
        file_context("/repo/src/first.rs", Some(40..45)),
        file_context("/repo/src/second.rs", None),
        file_context("/repo/src/first.rs", Some(10..20)),
    ];

    assert_eq!(
        group_file_contexts_for_display(&contexts, None, Some(&cwd)),
        vec![
            "src/first.rs (10-20, 40-45)".to_string(),
            "src/second.rs".to_string(),
        ]
    );
}
