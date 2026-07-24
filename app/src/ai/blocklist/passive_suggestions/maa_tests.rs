use super::{
    contents_exceed_passive_code_diff_size_limits, PASSIVE_CODE_DIFF_LONG_FILE_BYTE_LIMIT,
    PASSIVE_CODE_DIFF_LONG_FILE_LINE_LIMIT, PASSIVE_CODE_DIFF_TOTAL_BYTE_LIMIT,
};

#[test]
fn empty_diffs_are_within_limits() {
    assert!(!contents_exceed_passive_code_diff_size_limits(
        std::iter::empty::<&str>()
    ));
}

#[test]
fn small_files_are_within_limits() {
    let contents = ["fn main() {}", "let x = 1;\nlet y = 2;\n"];
    assert!(!contents_exceed_passive_code_diff_size_limits(
        contents.iter().copied()
    ));
}

#[test]
fn single_file_over_byte_limit_exceeds() {
    let big = "a".repeat(PASSIVE_CODE_DIFF_LONG_FILE_BYTE_LIMIT);
    assert!(contents_exceed_passive_code_diff_size_limits([big.as_str()]));
}

#[test]
fn single_file_over_line_limit_exceeds() {
    // Many short lines: well under the per-file byte limit but over the line limit.
    let many_lines = "x\n".repeat(PASSIVE_CODE_DIFF_LONG_FILE_LINE_LIMIT);
    assert!(many_lines.len() < PASSIVE_CODE_DIFF_LONG_FILE_BYTE_LIMIT);
    assert!(contents_exceed_passive_code_diff_size_limits([
        many_lines.as_str()
    ]));
}

#[test]
fn many_small_files_over_total_byte_limit_exceeds() {
    // Each file is individually under the per-file byte limit, but together they
    // exceed the total byte limit.
    let per_file = PASSIVE_CODE_DIFF_LONG_FILE_BYTE_LIMIT - 1;
    let file = "a".repeat(per_file);
    let count = PASSIVE_CODE_DIFF_TOTAL_BYTE_LIMIT / per_file + 1;
    let contents: Vec<&str> = vec![file.as_str(); count];
    assert!(contents_exceed_passive_code_diff_size_limits(contents));
}
