use std::collections::HashMap;

use ai::agent::action_result::AnyFileContent;
use ai::agent::FileLocations;
use ai::diff_validation::{DiffDelta, DiffType};

use super::{build_resolved_edits, updated_file_contexts_from_editor_buffers};
use crate::ai::blocklist::diff_types::FileDiff;

#[test]
fn build_resolved_edits_applies_deltas_without_reviewed_content() {
    // Headless/TUI path: no reviewed content, so final content is derived from deltas.
    let base = "one\ntwo\nthree\n";
    let diff = FileDiff::new(
        base.to_owned(),
        "/tmp/main.rs".to_owned(),
        DiffType::update(
            vec![DiffDelta {
                replacement_line_range: 2..3,
                insertion: "TWO\n".to_owned(),
            }],
            None,
        ),
    );

    let resolved = build_resolved_edits(vec![diff], &HashMap::new()).unwrap();

    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0].path, "/tmp/main.rs");
    assert_eq!(resolved[0].final_content, "one\nTWO\nthree\n");
}

#[test]
fn build_resolved_edits_prefers_reviewed_content() {
    // GUI path: reviewed content for a path overrides the delta-applied content.
    let base = "one\ntwo\nthree\n";
    let diff = FileDiff::new(
        base.to_owned(),
        "/tmp/main.rs".to_owned(),
        DiffType::update(
            vec![DiffDelta {
                replacement_line_range: 2..3,
                insertion: "TWO\n".to_owned(),
            }],
            None,
        ),
    );
    let reviewed = HashMap::from([("/tmp/main.rs".to_owned(), "user edited\n".to_owned())]);

    let resolved = build_resolved_edits(vec![diff], &reviewed).unwrap();

    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0].final_content, "user edited\n");
}

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
