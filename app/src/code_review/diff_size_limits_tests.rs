use std::mem::size_of;
use std::sync::Arc;

use super::*;
use crate::code_review::diff_state::{DiffHunk, DiffLine, DiffLineType};

fn line(text: String, line_type: DiffLineType) -> DiffLine {
    DiffLine {
        line_type,
        old_line_number: None,
        new_line_number: None,
        text,
        no_trailing_newline: false,
    }
}

fn hunk(lines: Vec<DiffLine>) -> DiffHunk {
    DiffHunk {
        old_start_line: 0,
        old_line_count: 0,
        new_start_line: 0,
        new_line_count: 0,
        lines,
        unified_diff_start: 0,
        unified_diff_end: 0,
    }
}

#[test]
fn approx_bytes_empty_diff_no_content_is_zero() {
    assert_eq!(approx_file_diff_bytes(&Arc::new(Vec::new()), None), 0);
}

#[test]
fn approx_bytes_counts_content_capacity_when_no_hunks() {
    let mut content = String::with_capacity(32);
    content.push_str("hello");

    assert_eq!(
        approx_file_diff_bytes(&Arc::new(Vec::new()), Some(&content)),
        content.capacity()
    );
}

#[test]
fn approx_bytes_counts_retained_structure_and_string_capacities() {
    let mut text = String::with_capacity(32);
    text.push('a');
    let text_capacity = text.capacity();

    let mut lines = Vec::with_capacity(4);
    lines.push(line(text, DiffLineType::Add));

    let mut hunks = Vec::with_capacity(3);
    hunks.push(hunk(lines));
    let hunks = Arc::new(hunks);

    let mut content = String::with_capacity(64);
    content.push_str("base");

    let expected = hunks
        .capacity()
        .saturating_mul(size_of::<DiffHunk>())
        .saturating_add(
            hunks[0]
                .lines
                .capacity()
                .saturating_mul(size_of::<DiffLine>()),
        )
        .saturating_add(text_capacity)
        .saturating_add(content.capacity());
    assert_eq!(approx_file_diff_bytes(&hunks, Some(&content)), expected);
}
