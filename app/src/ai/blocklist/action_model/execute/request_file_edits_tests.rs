use std::collections::HashMap;
use std::fs;

use ai::agent::action_result::{AnyFileContent, RequestFileEditsResult};
use ai::agent::FileLocations;
use ai::diff_validation::{DiffDelta, DiffType};

use super::{save_prepared_diffs, updated_file_contexts_from_editor_buffers};
use crate::ai::blocklist::inline_action::code_diff_view::FileDiff;

#[test]
fn updated_file_contexts_from_editor_buffers_returns_changed_lines_with_context() {
    let updated_files = vec![(
        FileLocations {
            name: "src/main.rs".to_string(),
            lines: std::iter::once(12..13).collect(),
        },
        true,
    )];
    let content = (1..=30)
        .map(|line| format!("line {line}"))
        .collect::<Vec<_>>()
        .join("\n");
    let content_map = HashMap::from([("src/main.rs".to_string(), content)]);

    let contexts = updated_file_contexts_from_editor_buffers(&updated_files, &content_map);

    assert_eq!(contexts.len(), 1);
    assert!(contexts[0].was_edited_by_user);
    assert_eq!(contexts[0].file_context.file_name, "src/main.rs");
    assert_eq!(contexts[0].file_context.line_range, Some(2..23));
    assert_eq!(contexts[0].file_context.line_count, 30);
    assert_eq!(
        contexts[0].file_context.content,
        AnyFileContent::StringContent(
            (2..=22)
                .map(|line| format!("line {line}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    );
}

#[test]
fn save_prepared_diffs_creates_file_without_code_diff_view() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("src").join("main.rs");
    let path = path.to_string_lossy().to_string();

    let result = save_prepared_diffs(vec![FileDiff::new(
        String::new(),
        path.clone(),
        DiffType::creation("fn main() {}\n".to_owned()),
    )]);

    let RequestFileEditsResult::Success {
        updated_files,
        deleted_files,
        lines_added,
        lines_removed,
        ..
    } = result
    else {
        panic!("expected create diff to save successfully");
    };

    assert_eq!(fs::read_to_string(&path).unwrap(), "fn main() {}\n");
    assert_eq!(deleted_files, Vec::<String>::new());
    assert_eq!(lines_added, 1);
    assert_eq!(lines_removed, 0);
    assert_eq!(updated_files.len(), 1);
    assert_eq!(updated_files[0].file_context.file_name, path);
}

#[test]
fn save_prepared_diffs_updates_file_without_code_diff_view() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("main.rs");
    let path = path.to_string_lossy().to_string();
    let original = "one\ntwo\nthree\n";
    fs::write(&path, original).unwrap();

    let result = save_prepared_diffs(vec![FileDiff::new(
        original.to_owned(),
        path.clone(),
        DiffType::update(
            vec![DiffDelta {
                replacement_line_range: 2..3,
                insertion: "TWO\n".to_owned(),
            }],
            None,
        ),
    )]);

    let RequestFileEditsResult::Success {
        updated_files,
        lines_added,
        lines_removed,
        ..
    } = result
    else {
        panic!("expected update diff to save successfully");
    };

    assert_eq!(fs::read_to_string(&path).unwrap(), "one\nTWO\nthree\n");
    assert_eq!(lines_added, 1);
    assert_eq!(lines_removed, 1);
    assert_eq!(updated_files.len(), 1);
    assert_eq!(updated_files[0].file_context.file_name, path);
}

#[test]
fn save_prepared_diffs_reports_save_failure_without_code_diff_view() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_string_lossy().to_string();

    let result = save_prepared_diffs(vec![FileDiff::new(
        String::new(),
        path,
        DiffType::creation("not a directory write\n".to_owned()),
    )]);

    let RequestFileEditsResult::DiffApplicationFailed { error } = result else {
        panic!("expected save failure");
    };
    assert!(
        error.contains("Failed to write"),
        "unexpected error message: {error}"
    );
}

#[test]
fn updated_file_contexts_from_editor_buffers_preserves_full_file_when_no_ranges() {
    let updated_files = vec![(
        FileLocations {
            name: "src/main.rs".to_string(),
            lines: vec![],
        },
        false,
    )];
    let content = "line 1\nline 2\n".to_string();
    let content_map = HashMap::from([("src/main.rs".to_string(), content.clone())]);

    let contexts = updated_file_contexts_from_editor_buffers(&updated_files, &content_map);

    assert_eq!(contexts.len(), 1);
    assert!(!contexts[0].was_edited_by_user);
    assert_eq!(contexts[0].file_context.line_range, None);
    assert_eq!(contexts[0].file_context.line_count, 2);
    assert_eq!(
        contexts[0].file_context.content,
        AnyFileContent::StringContent(content)
    );
}
