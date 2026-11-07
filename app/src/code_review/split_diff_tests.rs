use warp_editor::content::text::LineCount;
use warp_editor::render::model::LineCount as RenderLineCount;

use super::{map_hidden_range, modified_range_for_base_line, unchanged_line_location};
use crate::code::editor::diff_alignment::{ChangeSpan, DiffAlignment};
use crate::code::editor::line::EditorLineLocation;

/// Base: a b c d e f. Modified: a B c d X e f. Line b is replaced by B, X is added before e.
fn spans() -> Vec<ChangeSpan> {
    vec![
        ChangeSpan {
            base: 1..2,
            modified: 1..2,
        },
        ChangeSpan {
            base: 4..4,
            modified: 4..5,
        },
    ]
}

fn current(line: usize) -> EditorLineLocation {
    EditorLineLocation::Current {
        line_number: RenderLineCount::from(line),
        line_range: RenderLineCount::from(line)..RenderLineCount::from(line + 1),
    }
}

#[test]
fn removed_base_line_maps_to_its_hunk_in_the_modified_pane() {
    assert_eq!(
        modified_range_for_base_line(&spans(), 1),
        Some(RenderLineCount::from(1)..RenderLineCount::from(2))
    );
}

#[test]
fn unchanged_base_line_has_no_hunk() {
    assert_eq!(modified_range_for_base_line(&spans(), 3), None);
}

#[test]
fn deletion_maps_to_an_empty_range_at_the_deletion_point() {
    // Base: a b c. Modified: a c. Line b was deleted.
    let spans = vec![ChangeSpan {
        base: 1..2,
        modified: 1..1,
    }];
    assert_eq!(
        modified_range_for_base_line(&spans, 1),
        Some(RenderLineCount::from(1)..RenderLineCount::from(1))
    );
}

#[test]
fn unchanged_base_line_comments_land_on_the_same_row_after_an_addition() {
    let alignment = DiffAlignment::new(spans(), 6, 7);
    // Base line e (4) sits beside modified e (5), one line down because of the added X.
    assert_eq!(unchanged_line_location(&alignment, 4), Some(current(5)));
    assert_eq!(unchanged_line_location(&alignment, 0), Some(current(0)));
}

#[test]
fn changed_base_lines_are_not_treated_as_unchanged() {
    let alignment = DiffAlignment::new(spans(), 6, 7);
    assert_eq!(unchanged_line_location(&alignment, 1), None);
}

#[test]
fn hidden_range_maps_through_the_alignment() {
    let alignment = DiffAlignment::new(spans(), 6, 7);
    // Modified lines 5..7 (e f) sit on the same rows as base lines 4..6.
    let mapped = map_hidden_range(
        5..7,
        alignment.modified_line_count(),
        alignment.base_line_count(),
        |line| alignment.base_line_for_modified_line(line),
    );
    assert_eq!(mapped, Some(LineCount::from(4)..LineCount::from(6)));
}

#[test]
fn hidden_range_covering_the_trailing_empty_line_still_maps() {
    let alignment = DiffAlignment::new(spans(), 6, 7);
    // Modified line 7 is the empty line after the trailing newline, past the aligned lines.
    let mapped = map_hidden_range(
        5..8,
        alignment.modified_line_count(),
        alignment.base_line_count(),
        |line| alignment.base_line_for_modified_line(line),
    );
    assert_eq!(mapped, Some(LineCount::from(4)..LineCount::from(7)));
}
