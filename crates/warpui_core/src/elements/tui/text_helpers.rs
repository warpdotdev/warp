//! Terminal-string measurement and truncation helpers shared by TUI elements.

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// Returns terminal display-cell width, saturating at `u16::MAX`.
pub fn text_width(text: &str) -> u16 {
    u16::try_from(UnicodeWidthStr::width(text)).unwrap_or(u16::MAX)
}

/// Truncates terminal text at grapheme boundaries, using as much of `...` as fits.
pub fn truncate_with_ellipsis(text: &str, maximum_columns: usize) -> String {
    if usize::from(text_width(text)) <= maximum_columns {
        return text.to_owned();
    }
    let ellipsis_columns = usize::from(text_width("...")).min(maximum_columns);
    let prefix_columns = maximum_columns.saturating_sub(ellipsis_columns);
    let mut prefix = String::new();
    let mut prefix_width = 0usize;
    for grapheme in UnicodeSegmentation::graphemes(text, true) {
        let grapheme_width = usize::from(text_width(grapheme));
        if prefix_width.saturating_add(grapheme_width) > prefix_columns {
            break;
        }
        prefix.push_str(grapheme);
        prefix_width = prefix_width.saturating_add(grapheme_width);
    }
    prefix.push_str(&".".repeat(ellipsis_columns));
    prefix
}

#[cfg(test)]
#[path = "text_helpers_tests.rs"]
mod tests;
