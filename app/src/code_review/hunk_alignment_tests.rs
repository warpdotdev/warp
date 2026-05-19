use crate::code_review::{
    diff_state::{DiffHunk, DiffLine, DiffLineType},
    hunk_alignment::{HunkAlignment, PaneLine, PaneLineKind},
};

fn hunk(lines: Vec<DiffLine>) -> DiffHunk {
    DiffHunk {
        old_start_line: 1,
        old_line_count: 1,
        new_start_line: 1,
        new_line_count: 1,
        lines,
        unified_diff_start: 0,
        unified_diff_end: 0,
    }
}

fn context(old: usize, new: usize, text: &str) -> DiffLine {
    DiffLine {
        line_type: DiffLineType::Context,
        old_line_number: Some(old),
        new_line_number: Some(new),
        text: text.to_string(),
        no_trailing_newline: false,
    }
}

fn delete(old: usize, text: &str) -> DiffLine {
    DiffLine {
        line_type: DiffLineType::Delete,
        old_line_number: Some(old),
        new_line_number: None,
        text: text.to_string(),
        no_trailing_newline: false,
    }
}

fn add(new: usize, text: &str) -> DiffLine {
    DiffLine {
        line_type: DiffLineType::Add,
        old_line_number: None,
        new_line_number: Some(new),
        text: text.to_string(),
        no_trailing_newline: false,
    }
}

#[test]
fn aligns_paired_modifications_on_the_same_row() {
    let alignment = HunkAlignment::from_diff_hunks(&[hunk(vec![
        context(1, 1, "fn example() {"),
        delete(2, "    old_a();"),
        delete(3, "    old_b();"),
        add(2, "    new_a();"),
        add(3, "    new_b();"),
        context(4, 4, "}"),
    ])]);

    let rows = alignment.rows();
    assert_eq!(rows.len(), 4);
    assert_eq!(rows[1].baseline.text(), "    old_a();");
    assert_eq!(rows[1].modified.text(), "    new_a();");
    assert_eq!(rows[2].baseline.text(), "    old_b();");
    assert_eq!(rows[2].modified.text(), "    new_b();");
}

#[test]
fn renders_excess_additions_as_baseline_gaps() {
    let alignment = HunkAlignment::from_diff_hunks(&[hunk(vec![
        context(1, 1, "before"),
        delete(2, "old"),
        add(2, "new"),
        add(3, "extra"),
        context(3, 4, "after"),
    ])]);

    let rows = alignment.rows();
    assert_eq!(rows.len(), 4);
    assert!(matches!(rows[2].baseline, PaneLine::Gap { .. }));
    assert_eq!(rows[2].modified.text(), "extra");
    assert_eq!(rows[2].modified.kind(), Some(PaneLineKind::Add));
}

#[test]
fn renders_excess_deletions_as_modified_gaps() {
    let alignment = HunkAlignment::from_diff_hunks(&[hunk(vec![
        context(1, 1, "before"),
        delete(2, "old"),
        delete(3, "extra"),
        add(2, "new"),
        context(4, 3, "after"),
    ])]);

    let rows = alignment.rows();
    assert_eq!(rows.len(), 4);
    assert_eq!(rows[2].baseline.text(), "extra");
    assert_eq!(rows[2].baseline.kind(), Some(PaneLineKind::Delete));
    assert!(matches!(rows[2].modified, PaneLine::Gap { .. }));
}

#[test]
fn context_resets_delete_add_pairing() {
    let alignment = HunkAlignment::from_diff_hunks(&[hunk(vec![
        delete(1, "old"),
        context(2, 1, "shared"),
        add(2, "new"),
    ])]);

    let rows = alignment.rows();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].baseline.text(), "old");
    assert!(matches!(rows[0].modified, PaneLine::Gap { .. }));
    assert!(matches!(rows[2].baseline, PaneLine::Gap { .. }));
    assert_eq!(rows[2].modified.text(), "new");
}
