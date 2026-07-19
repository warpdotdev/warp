use super::*;

/// Helper: assert the parse produced a table and return it.
fn expect_table(content: &str) -> CsvTable {
    match parse_csv_for_render(content) {
        CsvRender::Table(table) => (*table).clone(),
        other => panic!("expected CsvRender::Table, got {other:?} for content {content:?}"),
    }
}

/// Helper: assert the parse fell back to raw with the given reason.
fn expect_fallback(content: &str, reason: CsvFallbackReason) {
    match parse_csv_for_render(content) {
        CsvRender::FallbackToRaw { reason: actual } => assert_eq!(
            actual, reason,
            "wrong fallback reason for content {content:?}"
        ),
        other => {
            panic!("expected FallbackToRaw({reason:?}), got {other:?} for content {content:?}")
        }
    }
}

/// Validation criterion #1: RFC-4180 quoted commas, doubled/escaped quotes, and
/// a quoted value spanning multiple physical lines render as single cell values.
#[test]
fn parse_csv_quoted_and_multiline() {
    // A quoted comma, a doubled-quote escape, and a multiline quoted value.
    let content = "header1,header2,header3\n\
        \"a,b\",\"x \"\"y\"\" z\",\"line1\nline2\"\n\
        plain,val,end\n";
    let table = expect_table(content);

    assert_eq!(table.header, ["header1", "header2", "header3"]);
    assert_eq!(table.rows.len(), 2);

    // Quoted comma stays in one cell.
    assert_eq!(table.rows[0][0], "a,b");
    // Doubled quotes decode to a single quote.
    assert_eq!(table.rows[0][1], "x \"y\" z");
    // Multiline quoted value spans two physical lines as one cell.
    assert_eq!(table.rows[0][2], "line1\nline2");

    assert_eq!(table.rows[1], ["plain", "val", "end"]);
}

/// Validation criterion #2: ragged rows (short and long) parse without panic,
/// short rows padded to the width and long rows preserved (not truncated).
#[test]
fn parse_csv_ragged_rows() {
    // Header has 3 columns; row 1 is short (2 cells), row 2 is long (4 cells).
    let content = "a,b,c\n1,2\n4,5,6,7\n";
    let table = expect_table(content);

    // The width grows to the widest row (4), and the header pads to match.
    assert_eq!(table.width(), 4);
    assert_eq!(table.header, ["a", "b", "c", ""]);
    // Short row padded with empty cells to width 4.
    assert_eq!(table.rows[0], ["1", "2", "", ""]);
    // Long row preserved at full width (overflow kept, not truncated).
    assert_eq!(table.rows[1], ["4", "5", "6", "7"]);
}

/// Validation criterion #3: a CSV exceeding the row cap and one exceeding the
/// byte cap both fall back to Raw (no panic, no OOM, no partial table).
#[test]
fn parse_csv_over_cap_falls_back_to_raw() {
    // Over the row cap: header + (MAX_CSV_ROWS + 1) data rows. Total records
    // exceed MAX_CSV_ROWS, and total bytes stay well under MAX_CSV_BYTES.
    let mut content = String::from("header\n");
    for _ in 0..(MAX_CSV_ROWS + 1) {
        content.push_str("row\n");
    }
    assert!(content.len() < MAX_CSV_BYTES);
    expect_fallback(&content, CsvFallbackReason::TooManyRows);

    // Over the byte cap: a single field larger than MAX_CSV_BYTES. The byte
    // cap is checked first, so this short-circuits before parsing.
    let content = format!("header\n{}", "x".repeat(MAX_CSV_BYTES + 1));
    assert!(content.len() > MAX_CSV_BYTES);
    expect_fallback(&content, CsvFallbackReason::TooManyBytes);
}

/// Validation criterion #4: malformed input that triggers a `csv` parse error
/// falls back to Raw with reason `ParseError`; the function never panics.
#[test]
fn parse_csv_malformed_falls_back_to_raw() {
    // An unterminated quoted field triggers a parse error.
    let content = "header\n\"unterminated";
    expect_fallback(content, CsvFallbackReason::ParseError);
}

/// An empty file falls back to Raw with reason `Empty`.
#[test]
fn parse_csv_empty_falls_back_to_raw() {
    expect_fallback("", CsvFallbackReason::Empty);
}

/// A header-only file is not empty: it renders a table with the header and no
/// data rows (the column names are still useful to show).
#[test]
fn parse_csv_header_only_renders_empty_table() {
    let table = expect_table("name,age,city\n");
    assert_eq!(table.header, ["name", "age", "city"]);
    assert!(table.rows.is_empty());
}
