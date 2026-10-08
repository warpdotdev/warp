//! Rendering of right-to-left text (e.g. Arabic, Hebrew) within a terminal grid row.
//!
//! The grid stores text in logical order, one character per cell. Right-to-left runs are instead
//! shaped as a whole, so letters join and appear in visual order, and are painted within the same
//! columns they occupy in the grid. Left-to-right content around them keeps its grid alignment.

use std::ops::Range;

use unicode_bidi::{BidiClass, BidiInfo, Level, bidi_class};
use warp_core::features::FeatureFlag;
use warpui::PaintContext;
use warpui::fonts::FamilyId;
use warpui::geometry::vector::{Vector2F, vec2f};
use warpui::platform::LineStyle;
use warpui::text_layout::DEFAULT_TOP_BOTTOM_RATIO;

use super::AttributedStringBuilder;
use crate::terminal::model::char_or_str::CharOrStr;
use crate::terminal::model::cell::Cell;

/// The right-to-left runs of a single grid row, collecting their styled text while the row's cells
/// are rendered.
pub(super) struct RtlRuns {
    runs: Vec<RtlRun>,
}

struct RtlRun {
    columns: Range<usize>,
    text: AttributedStringBuilder,
}

impl RtlRuns {
    /// Returns the right-to-left runs of `cells`, or no runs if the row contains no
    /// right-to-left text.
    pub(super) fn for_row(cells: &[Cell], font_family: FamilyId) -> Self {
        let runs = if FeatureFlag::RtlTerminalText.is_enabled() {
            rtl_column_ranges(cells)
                .into_iter()
                .map(|columns| RtlRun {
                    text: AttributedStringBuilder::new(font_family, font_family, columns.len()),
                    columns,
                })
                .collect()
        } else {
            Vec::new()
        };
        Self { runs }
    }

    /// Returns the text of the run containing `column`, if any. Cells in a run must have their
    /// content appended here instead of having their glyph drawn at the cell.
    pub(super) fn run_containing(&mut self, column: usize) -> Option<&mut AttributedStringBuilder> {
        self.runs
            .iter_mut()
            .find(|run| run.columns.contains(&column))
            .map(|run| &mut run.text)
    }

    /// Shapes each run and paints it within its columns.
    ///
    /// `row_baseline` is the baseline origin of the row's first column.
    pub(super) fn paint(
        self,
        row_baseline: Vector2F,
        cell_width: f32,
        font_size: f32,
        line_height_ratio: f32,
        ctx: &mut PaintContext,
    ) {
        for run in self.runs {
            let run_width = cell_width * run.columns.len() as f32;
            let string_data = run.text.build();
            let line = ctx.text_layout_cache.layout_line(
                &string_data.line,
                LineStyle {
                    font_size,
                    line_height_ratio,
                    baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
                    fixed_width_tab_size: None,
                },
                &string_data.style_runs,
                run_width,
                Default::default(),
                &ctx.font_cache.text_layout_system(),
            );

            // Right-to-left text starts at the right edge of its columns. A run whose shaped text
            // is wider than its columns overflows to the right rather than over the preceding text.
            let run_x = cell_width * run.columns.start as f32 + (run_width - line.width).max(0.);
            let run_origin = row_baseline + vec2f(run_x, 0.);

            for glyph_run in &line.runs {
                let glyph_color = glyph_run.styles.foreground_color.unwrap_or_default();
                for glyph in &glyph_run.glyphs {
                    ctx.scene.draw_glyph(
                        run_origin + glyph.position_along_baseline,
                        glyph.id,
                        glyph_run.font_id,
                        line.font_size,
                        glyph_color,
                    );
                }
            }
        }
    }
}

/// Returns the column ranges of `cells` that the Unicode Bidirectional Algorithm, resolved with a
/// left-to-right paragraph direction, places above the paragraph's embedding level.
///
/// This includes neutrals and numbers embedded in right-to-left text, but not trailing whitespace.
fn rtl_column_ranges(cells: &[Cell]) -> Vec<Range<usize>> {
    if !cells.iter().any(|cell| is_strong_rtl(cell.c)) {
        return Vec::new();
    }

    let mut text = String::new();
    let mut column_byte_offsets = Vec::with_capacity(cells.len());
    for cell in cells {
        column_byte_offsets.push(text.len());
        match cell.content_for_display() {
            CharOrStr::Char(c) => text.push(c),
            CharOrStr::Str(s) => text.push_str(s),
        }
    }

    let bidi_info = BidiInfo::new(&text, Some(Level::ltr()));
    let mut levels = Vec::with_capacity(text.len());
    for paragraph in &bidi_info.paragraphs {
        let paragraph_levels = bidi_info.reordered_levels(paragraph, paragraph.range.clone());
        levels.extend_from_slice(&paragraph_levels[paragraph.range.clone()]);
    }

    let mut ranges: Vec<Range<usize>> = Vec::new();
    for (column, &byte_offset) in column_byte_offsets.iter().enumerate() {
        if levels[byte_offset].number() == 0 {
            continue;
        }
        match ranges.last_mut() {
            Some(range) if range.end == column => range.end = column + 1,
            _ => ranges.push(column..column + 1),
        }
    }
    ranges
}

fn is_strong_rtl(c: char) -> bool {
    // Every strong right-to-left character is at or above the Hebrew block, so this avoids the
    // bidi class lookup for ASCII and other common left-to-right text.
    c >= '\u{0590}' && matches!(bidi_class(c), BidiClass::R | BidiClass::AL)
}

#[cfg(test)]
#[path = "rtl_tests.rs"]
mod tests;
