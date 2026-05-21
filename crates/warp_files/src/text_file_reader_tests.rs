#![allow(clippy::single_range_in_vec_init)]

use std::io::Write as _;

use crate::FileModel;

use super::*;

fn make_accumulator(ranges: &[std::ops::Range<usize>], max_bytes: usize) -> TextFileAccumulator {
    TextFileAccumulator::new("test.txt".to_string(), None, ranges, max_bytes)
}

/// Helper: push a line that was terminated by a newline in the original file
/// (i.e. every line except possibly the very last one).
fn push(acc: &mut TextFileAccumulator, line: &str) {
    acc.push_line(line.to_string(), true);
}

/// Helper: push the final line of a file that had **no** trailing newline.
fn push_no_newline(acc: &mut TextFileAccumulator, line: &str) {
    acc.push_line(line.to_string(), false);
}

// ── Whole-file (no ranges) ──────────────────────────────────────────

#[test]
fn whole_file_reads_all_lines() {
    let mut acc = make_accumulator(&[], 1000);
    push(&mut acc, "hello");
    push(&mut acc, "world");
    let (segments, bytes_read) = acc.finalize();

    assert_eq!(segments.len(), 1);
    // File had trailing newline → preserved.
    assert_eq!(segments[0].content, "hello\nworld\n");
    assert_eq!(segments[0].line_range, None);
    assert_eq!(segments[0].line_count, 2);
    assert_eq!(bytes_read, 12); // "hello" + "\n" + "world" + "\n"
}

#[test]
fn whole_file_truncated_at_byte_limit() {
    let mut acc = make_accumulator(&[], 8);
    push(&mut acc, "hello"); // 5 bytes
    push(&mut acc, "world"); // +1 sep +5 = 11 > 8 → truncated
    push(&mut acc, "extra");
    let (segments, bytes_read) = acc.finalize();

    assert_eq!(segments.len(), 1);
    assert_eq!(segments[0].content, "hello");
    assert_eq!(segments[0].line_range, Some(1..1)); // truncated → range shown
    assert_eq!(segments[0].line_count, 3);
    assert_eq!(bytes_read, 5);
}

#[test]
fn empty_file_whole_file_produces_segment() {
    let acc = make_accumulator(&[], 1000);
    let (segments, bytes_read) = acc.finalize();

    assert_eq!(segments.len(), 1);
    assert_eq!(segments[0].content, "");
    assert_eq!(segments[0].line_range, None);
    assert_eq!(segments[0].line_count, 0);
    assert_eq!(bytes_read, 0);
}

// ── Line ranges ─────────────────────────────────────────────────────

#[test]
fn single_range_extracted() {
    let mut acc = make_accumulator(&[2..4], 1000);
    for i in 1..=5 {
        push(&mut acc, &format!("line{i}"));
    }
    let (segments, bytes_read) = acc.finalize();

    assert_eq!(segments.len(), 1);
    assert_eq!(segments[0].content, "line2\nline3");
    assert_eq!(segments[0].line_range, Some(2..4));
    assert_eq!(segments[0].line_count, 5);
    assert_eq!(bytes_read, 11); // "line2" + "\n" + "line3"
}

#[test]
fn multiple_ranges_produce_separate_segments() {
    let mut acc = make_accumulator(&[1..2, 4..6], 1000);
    for i in 1..=6 {
        push(&mut acc, &format!("L{i}"));
    }
    let (segments, bytes_read) = acc.finalize();

    assert_eq!(segments.len(), 2);
    assert_eq!(segments[0].content, "L1");
    assert_eq!(segments[0].line_range, Some(1..2));
    assert_eq!(segments[1].content, "L4\nL5");
    assert_eq!(segments[1].line_range, Some(4..6));
    for seg in &segments {
        assert_eq!(seg.line_count, 6);
    }
    assert_eq!(bytes_read, 7); // "L1" (2) + "L4\nL5" (5)
}

#[test]
fn unsorted_ranges_are_sorted() {
    let mut acc = make_accumulator(&[4..6, 1..3], 1000);
    for i in 1..=6 {
        push(&mut acc, &format!("L{i}"));
    }
    let (segments, _) = acc.finalize();

    assert_eq!(segments.len(), 2);
    // Should come out sorted by start line.
    assert_eq!(segments[0].line_range, Some(1..3));
    assert_eq!(segments[0].content, "L1\nL2");
    assert_eq!(segments[1].line_range, Some(4..6));
    assert_eq!(segments[1].content, "L4\nL5");
}

#[test]
fn empty_file_with_ranges_produces_empty_segment() {
    let acc = make_accumulator(&[1..5], 1000);
    let (segments, bytes_read) = acc.finalize();

    assert_eq!(segments.len(), 1);
    assert_eq!(segments[0].content, "");
    assert_eq!(segments[0].line_range, Some(1..5));
    assert_eq!(segments[0].line_count, 0);
    assert_eq!(bytes_read, 0);
}

#[test]
fn range_past_eof_produces_empty_segment() {
    // File has 5 lines, but range requests lines 10..15 which are all past EOF.
    let mut acc = make_accumulator(&[10..15], 1000);
    for i in 1..=5 {
        push(&mut acc, &format!("line{i}"));
    }
    let (segments, bytes_read) = acc.finalize();

    assert_eq!(segments.len(), 1);
    assert_eq!(segments[0].content, "");
    assert_eq!(segments[0].line_range, Some(10..15));
    assert_eq!(segments[0].line_count, 5);
    assert_eq!(bytes_read, 0);
}

#[test]
fn multiple_ranges_some_past_eof() {
    // First range is valid, second is entirely past EOF.
    let mut acc = make_accumulator(&[1..3, 10..15], 1000);
    for i in 1..=5 {
        push(&mut acc, &format!("line{i}"));
    }
    let (segments, _bytes_read) = acc.finalize();

    assert_eq!(segments.len(), 2);
    assert_eq!(segments[0].content, "line1\nline2");
    assert_eq!(segments[0].line_range, Some(1..3));
    assert_eq!(segments[1].content, "");
    assert_eq!(segments[1].line_range, Some(10..15));
    for seg in &segments {
        assert_eq!(seg.line_count, 5);
    }
}

#[test]
fn line_count_reflects_total_file_lines() {
    let mut acc = make_accumulator(&[2..4], 1000);
    for i in 1..=10 {
        push(&mut acc, &format!("line{i}"));
    }
    let (segments, _) = acc.finalize();

    assert_eq!(segments[0].line_count, 10);
}

// ── Truncation with ranges ──────────────────────────────────────────

#[test]
fn range_truncated_at_byte_limit() {
    let mut acc = make_accumulator(&[2..6], 8);
    push(&mut acc, "skip"); // line 1, outside range
    push(&mut acc, "aaaa"); // line 2, 4 bytes
    push(&mut acc, "bbbb"); // line 3, +1+4 = 9 > 8 → truncated
    push(&mut acc, "cccc");
    push(&mut acc, "dddd");
    let (segments, bytes_read) = acc.finalize();

    assert_eq!(segments.len(), 1);
    assert_eq!(segments[0].content, "aaaa");
    assert_eq!(segments[0].line_range, Some(2..2));
    assert_eq!(segments[0].line_count, 5);
    assert_eq!(bytes_read, 4);
}

#[test]
fn global_budget_shared_across_ranges() {
    // Budget of 12 bytes. First range uses 5, leaving 7 for the second.
    let mut acc = make_accumulator(&[1..2, 3..5], 12);
    push(&mut acc, "hello"); // range 1: 5 bytes, total 5
    push(&mut acc, "gap"); // not in any range
    push(&mut acc, "world"); // range 2: 5 bytes, total 10 ≤ 12
    push(&mut acc, "extra"); // range 2: +1+5 = 16 > 12 → truncated
    let (segments, bytes_read) = acc.finalize();

    assert_eq!(segments.len(), 2);
    assert_eq!(segments[0].content, "hello");
    assert_eq!(segments[0].line_range, Some(1..2));
    assert_eq!(segments[1].content, "world");
    assert_eq!(segments[1].line_range, Some(3..3)); // truncated at line 3
    assert_eq!(bytes_read, 10);
}

// ── Trailing newline preservation ───────────────────────────────────

#[test]
fn whole_file_with_trailing_newline() {
    // Simulates a file "hello\nworld\n" — both lines terminated.
    let mut acc = make_accumulator(&[], 1000);
    push(&mut acc, "hello");
    push(&mut acc, "world");
    let (segments, _) = acc.finalize();

    assert_eq!(segments[0].content, "hello\nworld\n");
}

#[test]
fn whole_file_without_trailing_newline() {
    // Simulates a file "hello\nworld" — last line has no terminator.
    let mut acc = make_accumulator(&[], 1000);
    push(&mut acc, "hello");
    push_no_newline(&mut acc, "world");
    let (segments, _) = acc.finalize();

    assert_eq!(segments[0].content, "hello\nworld");
}

#[test]
fn single_line_file_with_trailing_newline() {
    // Simulates a file "hello\n".
    let mut acc = make_accumulator(&[], 1000);
    push(&mut acc, "hello");
    let (segments, _) = acc.finalize();

    assert_eq!(segments[0].content, "hello\n");
}

#[test]
fn single_line_file_without_trailing_newline() {
    // Simulates a file "hello" (no terminator).
    let mut acc = make_accumulator(&[], 1000);
    push_no_newline(&mut acc, "hello");
    let (segments, _) = acc.finalize();

    assert_eq!(segments[0].content, "hello");
}

#[test]
fn newline_only_file_round_trips() {
    // A file consisting of exactly "\n" should round-trip correctly.
    // read_line() yields a single empty line with has_newline=true.
    let mut acc = make_accumulator(&[], 1000);
    push(&mut acc, "");
    let (segments, _) = acc.finalize();

    assert_eq!(segments[0].content, "\n");
}

#[test]
fn ranged_read_does_not_append_trailing_newline() {
    // Trailing newline preservation only applies to whole-file reads.
    // Ranged reads should not add a trailing newline.
    let mut acc = make_accumulator(&[1..3], 1000);
    push(&mut acc, "line1");
    push(&mut acc, "line2");
    push(&mut acc, "line3");
    let (segments, _) = acc.finalize();

    assert_eq!(segments[0].content, "line1\nline2");
}

// ── FileModel::read_text_file (async, real file) ───────────────

#[test]
fn non_utf8_file_returns_not_text() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("binary.bin");
    {
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(b"valid line\n").unwrap();
        f.write_all(&[0xFF, 0xFE, 0x80, 0x81]).unwrap();
        f.write_all(b"\nanother line\n").unwrap();
    }

    let result =
        futures::executor::block_on(FileModel::read_text_file(&path, 10_000, &[], None)).unwrap();
    assert!(matches!(result, TextFileReadResult::NotText));
}

#[test]
fn read_text_file_preserves_trailing_newline() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("trailing.txt");
    std::fs::write(&path, "hello\nworld\n").unwrap();

    let result =
        futures::executor::block_on(FileModel::read_text_file(&path, 10_000, &[], None)).unwrap();
    let TextFileReadResult::Segments { segments, .. } = result else {
        panic!("expected Segments");
    };
    assert_eq!(segments[0].content, "hello\nworld\n");
}

#[test]
fn read_text_file_no_trailing_newline() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("no_trailing.txt");
    std::fs::write(&path, "hello\nworld").unwrap();

    let result =
        futures::executor::block_on(FileModel::read_text_file(&path, 10_000, &[], None)).unwrap();
    let TextFileReadResult::Segments { segments, .. } = result else {
        panic!("expected Segments");
    };
    assert_eq!(segments[0].content, "hello\nworld");
}

#[test]
fn read_text_file_crlf_normalized_to_lf() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("crlf.txt");
    std::fs::write(&path, "hello\r\nworld\r\n").unwrap();

    let result =
        futures::executor::block_on(FileModel::read_text_file(&path, 10_000, &[], None)).unwrap();
    let TextFileReadResult::Segments { segments, .. } = result else {
        panic!("expected Segments");
    };
    // CRLF is normalized to LF; trailing newline preserved.
    assert_eq!(segments[0].content, "hello\nworld\n");
}

#[test]
fn read_text_file_round_trip_fidelity() {
    // Verifies that reading a file and writing the content back produces
    // an identical file — the original motivation for this fix.
    let dir = tempfile::tempdir().unwrap();
    let original = "fn main() {\n    println!(\"hello\");\n}\n";
    let path = dir.path().join("roundtrip.rs");
    std::fs::write(&path, original).unwrap();

    let result =
        futures::executor::block_on(FileModel::read_text_file(&path, 10_000, &[], None)).unwrap();
    let TextFileReadResult::Segments { segments, .. } = result else {
        panic!("expected Segments");
    };
    assert_eq!(segments[0].content, original);
}
