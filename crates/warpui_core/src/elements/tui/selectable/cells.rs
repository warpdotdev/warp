use std::ops::Range;

use ratatui::buffer::CellWidth;

use super::super::TuiBuffer;

/// An absolute cell position in selectable content.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct TuiContentPoint {
    pub row: usize,
    pub col: u16,
}

/// A half-open linear span in selectable content.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TuiSelectionSpan {
    pub start: TuiContentPoint,
    pub end: TuiContentPoint,
}

/// One rendered glyph and its cell/byte extents.
#[derive(Clone, Debug)]
pub struct TuiRowGlyph {
    pub start_col: u16,
    pub end_col: u16,
    pub byte_range: Range<usize>,
    pub text: String,
}

/// Builds rendered glyphs for one buffer row.
pub(crate) fn row_glyphs(buffer: &TuiBuffer, row: u16, width: u16) -> Vec<TuiRowGlyph> {
    let mut glyphs = Vec::new();
    let mut col = 0u16;
    let mut byte_offset = 0usize;
    while col < width {
        let cell = &buffer[(col, row)];
        let text = cell.symbol().to_owned();
        if text.is_empty() {
            col = col.saturating_add(1);
            continue;
        }
        let end_col = col.saturating_add(cell.cell_width().max(1)).min(width);
        let byte_end = byte_offset.saturating_add(text.len());
        glyphs.push(TuiRowGlyph {
            start_col: col,
            end_col,
            byte_range: byte_offset..byte_end,
            text,
        });
        byte_offset = byte_end;
        col = end_col;
    }
    glyphs
}

/// Returns the character cell span at `point`.
pub(crate) fn cell_span(point: TuiContentPoint, width: u16) -> TuiSelectionSpan {
    TuiSelectionSpan {
        start: point,
        end: point_after_col(point.row, point.col.saturating_add(1), width),
    }
}

/// Scrapes one selected buffer-row slice.
pub(crate) fn scrape_row(buffer: &TuiBuffer, row: u16, columns: Range<u16>) -> String {
    let width = buffer.area.width;
    let start = columns.start.min(width);
    let end = columns.end.min(width);
    let mut text = String::new();
    let mut col = 0u16;
    while col < end {
        let cell = &buffer[(col, row)];
        let next_col = col.saturating_add(cell.cell_width().max(1));
        if col >= start && !cell.symbol().is_empty() {
            text.push_str(cell.symbol());
        }
        col = next_col;
    }
    text.trim_end().to_owned()
}

/// Returns the point after `col`, wrapping at `width`.
pub(crate) fn point_after_col(row: usize, col: u16, width: u16) -> TuiContentPoint {
    if col >= width {
        TuiContentPoint {
            row: row.saturating_add(1),
            col: 0,
        }
    } else {
        TuiContentPoint { row, col }
    }
}
