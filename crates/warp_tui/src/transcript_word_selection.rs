use string_offset::ByteOffset;
use warp_core::semantic_selection::SemanticSelection;
use warpui::SingletonEntity;
use warpui_core::elements::tui::{TuiContentPoint, TuiRowGlyph, TuiSelectionSpan};
use warpui_core::AppContext;

/// Resolves transcript word selection using configured smart-selection rules.
pub(super) fn word_span(
    point: TuiContentPoint,
    width: u16,
    glyphs: &[TuiRowGlyph],
    app: &AppContext,
) -> Option<TuiSelectionSpan> {
    let clicked = glyphs
        .iter()
        .position(|glyph| point.col >= glyph.start_col && point.col < glyph.end_col)?;
    let line = glyphs
        .iter()
        .map(|glyph| glyph.text.as_str())
        .collect::<String>();
    let semantic_selection = SemanticSelection::as_ref(app);
    let clicked_byte = glyphs[clicked].byte_range.start;
    let byte_range = semantic_selection
        .smart_search(&line, ByteOffset::from(clicked_byte))
        .map(|range| range.start.as_usize()..range.end.as_usize());

    let (start_index, end_index) = if let Some(byte_range) = byte_range {
        let start = glyphs.partition_point(|glyph| glyph.byte_range.end <= byte_range.start);
        let end = glyphs.partition_point(|glyph| glyph.byte_range.start < byte_range.end);
        (start, end)
    } else {
        let is_boundary = |glyph: &TuiRowGlyph| {
            glyph
                .text
                .chars()
                .next()
                .is_none_or(|c| semantic_selection.is_word_boundary_char(c))
        };
        if is_boundary(&glyphs[clicked]) {
            (clicked, clicked + 1)
        } else {
            let mut start = clicked;
            while start > 0 && !is_boundary(&glyphs[start - 1]) {
                start -= 1;
            }
            let mut end = clicked + 1;
            while end < glyphs.len() && !is_boundary(&glyphs[end]) {
                end += 1;
            }
            (start, end)
        }
    };
    let start = glyphs.get(start_index)?;
    let end = glyphs.get(end_index.saturating_sub(1))?;
    Some(TuiSelectionSpan {
        start: TuiContentPoint {
            row: point.row,
            col: start.start_col,
        },
        end: point_after_col(point.row, end.end_col, width),
    })
}

/// Returns the point after `col`, wrapping at `width`.
fn point_after_col(row: usize, col: u16, width: u16) -> TuiContentPoint {
    if col >= width {
        TuiContentPoint {
            row: row.saturating_add(1),
            col: 0,
        }
    } else {
        TuiContentPoint { row, col }
    }
}
