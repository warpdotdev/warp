//! Render-only CSV parser for Warp's file viewer.
//!
//! When the `CsvViewerRendering` feature flag is enabled and a `.csv` file is
//! opened, [`parse_csv_for_render`] turns the file's text into a row-oriented
//! [`CsvTable`] that the notebook viewer renders as a read-only GUI table. Any
//! failure mode — a parse error, an over-cap row/byte count, or an empty file —
//! yields [`CsvRender::FallbackToRaw`] so the caller can fall back to showing
//! the raw text in the code editor. This mirrors `ipynb_parser`'s
//! "render-only, never blank, fall back to raw on any failure" contract: the
//! parser never panics and never produces a blank view.
//!
//! Behavior is gated at runtime by `FeatureFlag::CsvViewerRendering` at the
//! call sites (routing + rendering); this module compiles unconditionally so
//! the feature is consistent across build configurations, following the
//! `JupyterNotebookRendering` precedent and the `add-feature-flag` skill's
//! "prefer runtime checks over cfg directives" guidance.

use std::sync::Arc;

use csv::ReaderBuilder;

/// Maximum number of records (header + data rows) rendered as a table.
/// Files with more records fall back to Raw rather than rendering a huge table.
/// Conservatively tuned and easy to adjust.
pub const MAX_CSV_ROWS: usize = 50_000;

/// Maximum input size, in bytes, rendered as a table. Larger inputs fall back
/// to Raw to avoid allocating a large parsed structure.
pub const MAX_CSV_BYTES: usize = 10 * 1024 * 1024;

/// The parsed, row-oriented shape of a CSV file, ready to render as a table.
///
/// `header` is the first record; `rows` are the remaining records. Both are
/// normalized to a uniform column [`CsvTable::width`] (see [`parse_csv_for_render`]):
/// short rows are padded with empty cells and long rows keep their overflow
/// cells so nothing is silently truncated. The struct is cheaply shareable via
/// an [`Arc`] so the table's `row_render_fn` closure can capture a clone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CsvTable {
    pub header: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

impl CsvTable {
    /// The uniform column count (header width, >= every row width after
    /// normalization).
    pub fn width(&self) -> usize {
        self.header.len()
    }
}

/// Why a CSV fell back to Raw instead of rendering as a table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CsvFallbackReason {
    /// The file exceeded [`MAX_CSV_ROWS`] records.
    TooManyRows,
    /// The file exceeded [`MAX_CSV_BYTES`] bytes.
    TooManyBytes,
    /// The `csv` parser rejected the input (e.g. an unterminated quoted field).
    ParseError,
    /// The file had no records at all.
    Empty,
}

/// The result of parsing a CSV file for rendering.
///
/// `Table` carries the parsed rows; `FallbackToRaw` tells the caller to show
/// the raw text in the code editor instead.
#[derive(Debug, Clone)]
pub enum CsvRender {
    Table(Arc<CsvTable>),
    FallbackToRaw { reason: CsvFallbackReason },
}

/// Parse `content` for rendered display.
///
/// Uses the `csv` crate with `flexible(true)` (ragged rows allowed) and
/// `has_headers(false)` (the first record is treated as the header here). The
/// first record becomes the header row; remaining records become data rows.
/// Rows are normalized to the maximum width across the header and all rows:
/// short rows are padded with empty cells, long rows keep their overflow cells.
///
/// Returns [`CsvRender::FallbackToRaw`] (never panics) when the input is empty,
/// exceeds the row or byte cap, or fails to parse.
pub fn parse_csv_for_render(content: &str) -> CsvRender {
    if content.len() > MAX_CSV_BYTES {
        return CsvRender::FallbackToRaw {
            reason: CsvFallbackReason::TooManyBytes,
        };
    }

    // The `csv` crate is intentionally lenient (with `flexible(true)` and valid
    // UTF-8 it accepts almost anything, including an unterminated quoted field),
    // so it never raises a parse error of its own. Detect the clearest RFC-4180
    // malformation — a quoted field that is never closed — ourselves and fall
    // back to Raw rather than rendering a silently-truncated table.
    if has_unterminated_quoted_field(content) {
        return CsvRender::FallbackToRaw {
            reason: CsvFallbackReason::ParseError,
        };
    }

    let mut reader = ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .from_reader(content.as_bytes());

    // Collect records one at a time so the row cap short-circuits before the
    // whole file is materialized.
    let mut records: Vec<csv::StringRecord> = Vec::new();
    for result in reader.records() {
        match result {
            Ok(record) => {
                records.push(record);
                if records.len() > MAX_CSV_ROWS {
                    return CsvRender::FallbackToRaw {
                        reason: CsvFallbackReason::TooManyRows,
                    };
                }
            }
            Err(_) => {
                return CsvRender::FallbackToRaw {
                    reason: CsvFallbackReason::ParseError,
                };
            }
        }
    }

    if records.is_empty() {
        return CsvRender::FallbackToRaw {
            reason: CsvFallbackReason::Empty,
        };
    }

    let header: Vec<String> = records[0].iter().map(|field| field.to_string()).collect();
    let rows: Vec<Vec<String>> = records[1..]
        .iter()
        .map(|record| record.iter().map(|field| field.to_string()).collect())
        .collect();

    // Normalize to the maximum width so ragged rows render without truncation:
    // short rows pad with empty cells, long rows keep their overflow cells.
    let width = header
        .len()
        .max(rows.iter().map(|row| row.len()).max().unwrap_or(0));
    let mut header = header;
    pad_to_width(&mut header, width);
    let mut rows = rows;
    for row in &mut rows {
        pad_to_width(row, width);
    }

    CsvRender::Table(Arc::new(CsvTable { header, rows }))
}

/// Extend `cells` with empty strings until it has `width` entries. Rows longer
/// than `width` are left untouched (overflow is preserved, not truncated).
fn pad_to_width(cells: &mut Vec<String>, width: usize) {
    if cells.len() < width {
        cells.resize(width, String::new());
    }
}

/// Detect an unterminated quoted field (a `"` that opens a quoted field but is
/// never closed before EOF), which is an RFC-4180 malformation the lenient `csv`
/// crate silently accepts. Tracks quote state across the whole file, treating
/// doubled quotes (`""`) as escaped quotes inside a field (or an empty quoted
/// field outside one). Scans bytes directly: `"` is ASCII and never appears as a
/// sub-byte of a multibyte UTF-8 sequence, so this is safe on `&str` input.
fn has_unterminated_quoted_field(content: &str) -> bool {
    let bytes = content.as_bytes();
    let mut in_quote = false;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            let doubled = i + 1 < bytes.len() && bytes[i + 1] == b'"';
            if in_quote {
                if doubled {
                    // Escaped quote inside a quoted field; stay inside.
                    i += 2;
                    continue;
                }
                // Closing quote.
                in_quote = false;
            } else if doubled {
                // Empty quoted field (""); stay outside.
                i += 2;
                continue;
            } else {
                // Opening quote.
                in_quote = true;
            }
        }
        i += 1;
    }
    // `in_quote` at EOF means a quoted field was opened but never closed.
    in_quote
}

#[cfg(test)]
#[path = "csv_viewer_tests.rs"]
mod tests;
