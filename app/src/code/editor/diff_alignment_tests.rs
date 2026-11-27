use super::{AlignedRow, ChangeSpan, DiffAlignment, GapRun};

fn span(base: std::ops::Range<usize>, modified: std::ops::Range<usize>) -> ChangeSpan {
    ChangeSpan { base, modified }
}

fn context(base: usize, modified: usize) -> AlignedRow {
    AlignedRow {
        base: Some(base),
        modified: Some(modified),
        is_change: false,
    }
}

fn change(base: Option<usize>, modified: Option<usize>) -> AlignedRow {
    AlignedRow {
        base,
        modified,
        is_change: true,
    }
}

#[test]
fn no_changes_aligns_every_line_as_context() {
    let alignment = DiffAlignment::new(vec![], 3, 3);
    assert_eq!(
        alignment.rows(),
        &[context(0, 0), context(1, 1), context(2, 2)]
    );
    assert!(alignment.base_gaps().is_empty());
    assert!(alignment.modified_gaps().is_empty());
}

#[test]
fn pure_addition_faces_gaps_on_the_base_side() {
    // Lines 1..4 of the modified text are new.
    let alignment = DiffAlignment::new(vec![span(1..1, 1..4)], 2, 5);
    assert_eq!(
        alignment.rows(),
        &[
            context(0, 0),
            change(None, Some(1)),
            change(None, Some(2)),
            change(None, Some(3)),
            context(1, 4),
        ]
    );
    assert_eq!(
        alignment.base_gaps(),
        vec![GapRun {
            insert_before: 1,
            count: 3
        }]
    );
    assert!(alignment.modified_gaps().is_empty());
}

#[test]
fn pure_deletion_faces_gaps_on_the_modified_side() {
    let alignment = DiffAlignment::new(vec![span(1..3, 1..1)], 4, 2);
    assert_eq!(
        alignment.rows(),
        &[
            context(0, 0),
            change(Some(1), None),
            change(Some(2), None),
            context(3, 1),
        ]
    );
    assert_eq!(
        alignment.modified_gaps(),
        vec![GapRun {
            insert_before: 1,
            count: 2
        }]
    );
    assert_eq!(alignment.changed_base_lines(), vec![1..3]);
}

#[test]
fn equal_replacement_pairs_rows_in_order() {
    // Context, Delete, Delete, Add, Add, Context.
    let alignment = DiffAlignment::new(vec![span(1..3, 1..3)], 4, 4);
    assert_eq!(
        alignment.rows(),
        &[
            context(0, 0),
            change(Some(1), Some(1)),
            change(Some(2), Some(2)),
            context(3, 3),
        ]
    );
    assert!(alignment.base_gaps().is_empty());
    assert!(alignment.modified_gaps().is_empty());
}

#[test]
fn longer_removal_pairs_first_then_leaves_gaps() {
    // Three removed lines replaced by one added line.
    let alignment = DiffAlignment::new(vec![span(0..3, 0..1)], 3, 1);
    assert_eq!(
        alignment.rows(),
        &[
            change(Some(0), Some(0)),
            change(Some(1), None),
            change(Some(2), None),
        ]
    );
    assert_eq!(
        alignment.modified_gaps(),
        vec![GapRun {
            insert_before: 1,
            count: 2
        }]
    );
}

#[test]
fn context_between_runs_resets_pairing() {
    // A deletion, one unchanged line, then an addition: the added line must not pair with the
    // deleted line across the context line.
    let alignment = DiffAlignment::new(vec![span(1..2, 1..1), span(3..3, 2..3)], 3, 3);
    assert_eq!(
        alignment.rows(),
        &[
            context(0, 0),
            change(Some(1), None),
            context(2, 1),
            change(None, Some(2)),
        ]
    );
    assert_eq!(
        alignment.modified_gaps(),
        vec![GapRun {
            insert_before: 1,
            count: 1
        }]
    );
    assert_eq!(
        alignment.base_gaps(),
        vec![GapRun {
            insert_before: 3,
            count: 1
        }]
    );
}

#[test]
fn interleaved_runs_place_each_gap_at_its_own_row() {
    // Removed 1, added 1, removed 1, added 2 as separate spans with context in between. Gaps
    // land next to their own change, not at the end of the region.
    let alignment = DiffAlignment::new(
        vec![span(1..2, 1..1), span(3..3, 2..4), span(4..5, 5..5)],
        6,
        6,
    );
    assert_eq!(
        alignment.modified_gaps(),
        vec![
            GapRun {
                insert_before: 1,
                count: 1
            },
            GapRun {
                insert_before: 5,
                count: 1
            },
        ]
    );
    assert_eq!(
        alignment.base_gaps(),
        vec![GapRun {
            insert_before: 3,
            count: 2
        }]
    );
    for row in alignment.rows() {
        if let (Some(base), Some(modified)) = (row.base, row.modified) {
            assert_eq!(alignment.modified_line_for_base_line(base), Some(modified));
            assert_eq!(alignment.base_line_for_modified_line(modified), Some(base));
        }
    }
}

#[test]
fn multi_hunk_numbering_resumes_after_each_hunk() {
    let alignment = DiffAlignment::new(vec![span(1..2, 1..3), span(5..7, 6..6)], 8, 7);
    let rows = alignment.rows();
    let base_lines = rows.iter().filter_map(|row| row.base).collect::<Vec<_>>();
    let modified_lines = rows
        .iter()
        .filter_map(|row| row.modified)
        .collect::<Vec<_>>();
    assert_eq!(base_lines, (0..8).collect::<Vec<_>>());
    assert_eq!(modified_lines, (0..7).collect::<Vec<_>>());
    // The unchanged line after the second hunk keeps its pairing.
    assert_eq!(alignment.modified_line_for_base_line(7), Some(6));
}

#[test]
fn deleted_file_yields_only_removed_rows() {
    let alignment = DiffAlignment::new(vec![span(0..3, 0..0)], 3, 0);
    assert_eq!(
        alignment.rows(),
        &[
            change(Some(0), None),
            change(Some(1), None),
            change(Some(2), None),
        ]
    );
    assert_eq!(
        alignment.modified_gaps(),
        vec![GapRun {
            insert_before: 0,
            count: 3
        }]
    );
}

#[test]
fn aligns_a_large_multi_hunk_diff() {
    let spans = (0..200)
        .map(|hunk| {
            // Each earlier hunk removed one more line than it added.
            let base_start = hunk * 25 + 5;
            let modified_start = base_start - hunk;
            span(
                base_start..base_start + 3,
                modified_start..modified_start + 2,
            )
        })
        .collect::<Vec<_>>();
    let alignment = DiffAlignment::new(spans, 5_000, 4_800);
    assert_eq!(alignment.modified_gaps().len(), 200);
    assert_eq!(alignment.rows().len(), 5_000);
}
