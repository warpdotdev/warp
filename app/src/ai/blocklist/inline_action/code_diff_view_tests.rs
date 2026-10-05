use super::*;

#[test]
fn v4a_string_restore_previews() {
    let hunk = V4AHunk {
        change_context: vec![],
        pre_context: "before\n\n".into(),
        old: "\n".into(),
        new: "replacement\n\n".into(),
        post_context: "after\n\n".into(),
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
