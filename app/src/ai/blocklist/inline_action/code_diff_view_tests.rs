use ai::diff_validation::{V4ALineData, fuzzy_match_v4a_diffs};

use super::*;

#[test]
fn v4a_string_restore_previews() {
    let hunk = V4AHunk {
        line_data: Some(V4ALineData {
            pre_context: vec!["before".into(), "".into()],
            old: vec!["".into()],
            new: vec!["replacement".into(), "".into()],
            post_context: vec!["after".into(), "".into()],
        }),
        change_context: vec![],
        pre_context: "before\n".into(),
        old: "".into(),
        new: "replacement\n".into(),
        post_context: "after\n".into(),
    };
    let edits = vec![FileEdit::Edit(ParsedDiff::V4AEdit {
        file: Some("/tmp/preview.txt".into()),
        move_to: None,
        hunks: vec![hunk],
    })];
    let diffs = convert_file_edits_to_file_diffs(edits, &None, &None);
    assert_eq!(diffs.len(), 1);
    assert_eq!(diffs[0].base.content, "before\n\n\nafter\n\n");
    assert_eq!(diffs[0].line_stats(), (2, 1));
    let DiffType::Update { deltas, .. } = &diffs[0].diff_type else {
        panic!("expected restored update");
    };
    assert_eq!(deltas.len(), 1);
    assert_eq!(deltas[0].replacement_line_range, 3..4);
    assert_eq!(deltas[0].insertion, "replacement\n\n");
}

#[test]
fn v4a_restore_preserves_cr_ending_line() {
    let hunk: V4AHunk = serde_json::from_value(serde_json::json!({
        "change_context": [], "pre_context": "", "old": "a\r",
        "new": "b", "post_context": "",
        "line_data": {"old": ["a\r"], "new": ["b"]}
    }))
    .unwrap();
    let fresh = fuzzy_match_v4a_diffs("/tmp/preview.txt", std::slice::from_ref(&hunk), None, "a\r");
    assert_eq!(fresh.failures, None);
    let edits = vec![FileEdit::Edit(ParsedDiff::V4AEdit {
        file: Some("/tmp/preview.txt".into()),
        move_to: None,
        hunks: vec![hunk],
    })];

    let diffs = convert_file_edits_to_file_diffs(edits, &None, &None);

    assert_eq!(diffs.len(), 1);
    let DiffType::Update { deltas, .. } = &diffs[0].diff_type else {
        panic!("expected restored update");
    };
    assert_eq!(
        deltas,
        &[DiffDelta {
            replacement_line_range: 1..2,
            insertion: "b".into(),
        }]
    );
    assert_eq!(diffs[0].base.content, "a\r\n");
}
