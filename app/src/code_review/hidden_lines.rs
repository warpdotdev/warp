use std::ops::Range;

use ai::diff_validation::DiffDelta;
use rangemap::RangeSet;
use warp_editor::content::text::LineCount;
use warp_editor::render::model::LineCount as RenderLineCount;

/// The number of context lines to show before and after each change
const CONTEXT_LINES: usize = 4;

/// Calculate which lines should be hidden in a file after applying diffs.
///
/// This function takes a list of diff deltas and comment line numbers, then calculates which lines should be hidden
/// (everything except for CONTEXT_LINES before and after each change and comment).
///
/// These are 0-indexed line numbers BEFORE the diffs are applied, so the first line is 0.
///
/// # Arguments
///
/// * `diffs` - The list of diff deltas that will be applied to the file
/// * `line_count` - The total number of lines in the file before any diffs are applied
/// * `comment_line_numbers` - Line numbers where comments exist (0-indexed)
///
/// # Returns
///
/// A `RangeSet<usize>` containing the line ranges that should be hidden (0-indexed).
///
/// Note that DiffDelta uses 1-indexed line ranges, so we convert them to 0-indexed
/// ```
pub fn calculate_hidden_lines(
    diffs: &[DiffDelta],
    line_count: usize,
    comment_line_numbers: &[RenderLineCount],
) -> RangeSet<LineCount> {
    // Calculate the visible line ranges (with context)
    let mut visible_ranges: RangeSet<LineCount> = RangeSet::new();

    // Add ranges for diffs
    for diff in diffs {
        // Convert 1-indexed line ranges to 0-indexed
        let start_line = diff.replacement_line_range.start.saturating_sub(1);
        let end_line = diff.replacement_line_range.end.saturating_sub(1);

        let context_start = start_line.saturating_sub(CONTEXT_LINES);
        let context_end = end_line + CONTEXT_LINES;

        if context_start < context_end {
            visible_ranges.insert(context_start.into()..context_end.into());
        }
    }

    // Add ranges for comments
    for &comment_line in comment_line_numbers {
        let line_number = comment_line.as_usize();
        let context_start = line_number.saturating_sub(CONTEXT_LINES);
        let context_end = (line_number + CONTEXT_LINES + 1).min(line_count); // +1 because we want to include the line itself, clamped to file bounds

        if context_start < context_end {
            visible_ranges.insert(LineCount::from(context_start)..LineCount::from(context_end));
        }
    }

    // Calculate hidden ranges as the complement of visible ranges
    let all_lines: Range<LineCount> = LineCount::from(0)..LineCount::from(line_count);

    // Find gaps in the visible ranges
    visible_ranges
        .gaps(&all_lines)
        .collect::<RangeSet<LineCount>>()
}

#[cfg(test)]
#[path = "hidden_lines_tests.rs"]
mod tests;
