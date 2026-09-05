use super::*;
use crate::code_review::diff_state::{DiffHunk, DiffLine, DiffLineType};

fn line(text: &str) -> DiffLine {
    DiffLine {
        line_type: DiffLineType::Add,
        old_line_number: None,
        new_line_number: Some(1),
        text: text.to_string(),
        no_trailing_newline: false,
    }
}

fn hunk(lines: Vec<DiffLine>) -> DiffHunk {
    DiffHunk {
        old_start_line: 1,
        old_line_count: 0,
        new_start_line: 1,
        new_line_count: lines.len(),
        lines,
        unified_diff_start: 0,
        unified_diff_end: 0,
    }
}

#[test]
fn approx_bytes_empty_diff_no_content_is_zero() {
    assert_eq!(approx_file_diff_bytes(&[], None), 0);
}

#[test]
fn approx_bytes_counts_only_content_when_no_hunks() {
    assert_eq!(approx_file_diff_bytes(&[], Some("hello")), 5);
}

/// This is the regression this suite exists to catch: a prior version of
/// this estimator summed only `line.text.len()`, which for realistic
/// short diff lines undercounts real retained memory by tens of times —
/// enough that a budget meant to cap aggregate memory in the hundreds of
/// megabytes could in practice admit many gigabytes (APP-5462). A single
/// one-byte line must therefore cost far more than one byte: at least the
/// fixed `DiffLine` struct size on top of the text.
#[test]
fn approx_bytes_counts_line_struct_overhead_not_just_text() {
    let hunks = [hunk(vec![line("x")])];

    let bytes = approx_file_diff_bytes(&hunks, None);

    let line_struct_size = std::mem::size_of::<DiffLine>();
    assert_eq!(bytes, line_struct_size + 8);
    // Guard against the specific regression: the broken formula returns
    // exactly `1` here (just `line.text.len()`), so assert we are not that.
    assert_ne!(
        bytes, 1,
        "must count DiffLine's own struct size, not just its text"
    );
}

/// The per-line text floor exists to keep the estimate an honest lower
/// bound even for the smallest possible line: real allocators round small
/// allocations up to a minimum size class, so an empty `String` never
/// actually costs 0 bytes of heap.
#[test]
fn approx_bytes_floors_empty_line_text_at_minimum_allocation() {
    let hunks = [hunk(vec![line("")])];
    let line_struct_size = std::mem::size_of::<DiffLine>();

    let bytes = approx_file_diff_bytes(&hunks, None);

    assert_eq!(
        bytes,
        line_struct_size + 8,
        "an empty line's text contribution must still be floored to the minimum allocation"
    );
}

#[test]
fn approx_bytes_sums_hunk_line_text_and_content() {
    let hunks = [
        hunk(vec![line("added line one"), line("added line two")]),
        hunk(vec![line("added line three")]),
    ];
    let line_struct_size = std::mem::size_of::<DiffLine>();
    let expected_text_bytes = "added line one".len()
        + "added line two".len()
        + "added line three".len()
        + line_struct_size * 3;

    let bytes = approx_file_diff_bytes(&hunks, Some("base!!"));

    assert_eq!(bytes, expected_text_bytes + "base!!".len());
}

/// A many-short-line diff (the dense/pathological case the struct overhead
/// matters most for) should scale with line count, not just total text
/// length, so a diff full of one-character lines is correctly counted as
/// much larger than its raw text size would suggest.
#[test]
fn approx_bytes_scales_with_line_count_for_short_lines() {
    let many_short_lines: Vec<DiffLine> = (0..1_000).map(|_| line("x")).collect();
    let hunks = [hunk(many_short_lines)];

    let bytes = approx_file_diff_bytes(&hunks, None);

    let line_struct_size = std::mem::size_of::<DiffLine>();
    assert_eq!(bytes, (line_struct_size + 8) * 1_000);
    // The raw text alone is only 1000 bytes; the real estimate must be
    // dominated by per-line struct overhead, not the text.
    assert!(
        bytes > 1_000 * 10,
        "struct overhead must dominate for short lines"
    );
}
