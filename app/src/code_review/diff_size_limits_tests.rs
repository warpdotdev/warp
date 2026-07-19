use super::*;
use crate::code_review::diff_state::{DiffHunk, DiffLine, DiffLineType};

fn line(text: &str, line_type: DiffLineType) -> DiffLine {
    DiffLine {
        line_type,
        old_line_number: None,
        new_line_number: None,
        text: text.to_string(),
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
    assert_eq!(approx_file_diff_bytes(&[], None), 0);
}

#[test]
fn approx_bytes_counts_only_content_when_no_hunks() {
    assert_eq!(approx_file_diff_bytes(&[], Some("hello")), 5);
}

#[test]
fn approx_bytes_sums_hunk_line_text_and_content() {
    let hunks = vec![
        hunk(vec![
            line("added line", DiffLineType::Add), // 10
            line("ctx", DiffLineType::Context),    // 3
        ]),
        hunk(vec![
            line("gone", DiffLineType::Delete), // 4
        ]),
    ];
    // 10 + 3 + 4 hunk bytes, plus 6 bytes of base content.
    assert_eq!(
        approx_file_diff_bytes(&hunks, Some("base!!")),
        10 + 3 + 4 + 6
    );
}
