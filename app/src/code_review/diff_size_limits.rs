use serde::{Deserialize, Serialize};

use super::diff_state::{DiffHunk, DiffLineType};

/**
 * Maximum diff size that we will attempt to render. Diffs larger than this
 * should not be rendered to avoid performance issues.
 */
pub(crate) const MAX_DIFF_SIZE: usize = 4_375_000; // 4.375MB in decimal

/**
 * Reasonable limit for diff size. Diffs bigger than this _could_ be displayed
 * but it might cause some slowness.
 */
const MAX_REASONABLE_DIFF_SIZE: usize = 2_187_500; // ~2.1875MB in decimal

/**
 * The longest line length we should try to display. If a diff has a line longer
 * than this, we don't attempt to render it.
 */
const MAX_CHARACTERS_PER_LINE: usize = 5000;

/**
 * Current line-based limit for auto-expansion in code review.
 * This exists separately from the new size-based limits.
 */
const DIFF_LINE_RENDER_LIMIT: usize = 10_000;

/**
 * We have a lower deletion line limit since rendering deleted chunks are more
 * performance intensive.
 */
const DELETION_LINE_RENDER_LIMIT: usize = 8000;

#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum DiffSize {
    /// Small diff that can be rendered normally
    Normal,
    /// Large diff that should be collapsed by default but can be expanded
    Large,
    /// Diff that's too large to render safely
    Unrenderable,
}

/// Determines if a diff size exceeds the maximum renderable limit
fn is_diff_unrenderable(buffer_length: usize) -> bool {
    buffer_length > MAX_DIFF_SIZE
}

/// Determines if a diff buffer is too large for reasonable rendering
fn is_buffer_too_large(buffer_length: usize) -> bool {
    buffer_length >= MAX_REASONABLE_DIFF_SIZE
}

/// Determines if a diff has any line that's too long
fn is_diff_too_large(diff: &[DiffHunk]) -> bool {
    diff.iter()
        .flat_map(|hunk| &hunk.lines)
        .any(|line| line.text.len() > MAX_CHARACTERS_PER_LINE)
}

/// Categorizes a diff based on multiple size heuristics
pub fn compute_diff_size(diffs: &[DiffHunk], diff_size: usize) -> DiffSize {
    if compute_diff_size_for_buffer_length(diff_size) == DiffSize::Unrenderable {
        return DiffSize::Unrenderable;
    }

    let buffer_size = compute_diff_size_for_buffer_length(diff_size);

    let additions = diffs
        .iter()
        .flat_map(|hunk| &hunk.lines)
        .filter(|line| line.line_type == DiffLineType::Add)
        .count();

    let deletions = diffs
        .iter()
        .flat_map(|hunk| &hunk.lines)
        .filter(|line| line.line_type == DiffLineType::Delete)
        .count();

    // To avoid performance issues, set a lower render limit for deletion lines.
    if deletions > DELETION_LINE_RENDER_LIMIT {
        return DiffSize::Unrenderable;
    }

    if buffer_size == DiffSize::Large
        || is_diff_too_large(diffs)
        || additions > DIFF_LINE_RENDER_LIMIT
        || deletions > DIFF_LINE_RENDER_LIMIT
    {
        return DiffSize::Large;
    }

    DiffSize::Normal
}

/// Categorizes a diff based only on the buffered git output length.
pub fn compute_diff_size_for_buffer_length(diff_size: usize) -> DiffSize {
    if is_diff_unrenderable(diff_size) {
        return DiffSize::Unrenderable;
    }

    if is_buffer_too_large(diff_size) {
        return DiffSize::Large;
    }

    DiffSize::Normal
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_diff_size_for_buffer_length_marks_unrenderable() {
        assert_eq!(
            compute_diff_size_for_buffer_length(MAX_DIFF_SIZE + 1),
            DiffSize::Unrenderable
        );
    }
}
