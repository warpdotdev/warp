use serde::{Deserialize, Serialize};

use super::diff_state::{DiffHunk, DiffLineType};

/**
 * Maximum diff size that we will attempt to render. Diffs larger than this
 * should not be rendered to avoid performance issues.
 */
const MAX_DIFF_SIZE: usize = 4_375_000; // 4.375MB in decimal

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

/**
 * Combined (additions + deletions) threshold from `git diff --numstat` per file.
 * Files exceeding this skip `git diff --patch` entirely to avoid buffering
 * multi-GB diff output. Set well above the per-side render limits so normal
 * large diffs are unaffected.
 */
const MAX_NUMSTAT_LINES_EARLY_BAIL: usize = 50_000;

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

/// Returns `true` when the raw `git diff` output is too large to parse.
/// Called *before* `parse_diff_hunks` to avoid allocating memory for
/// diffs that would be marked [`DiffSize::Unrenderable`] anyway.
pub fn is_raw_diff_unrenderable(buffer_length: usize) -> bool {
    buffer_length > MAX_DIFF_SIZE
}

/// Returns `true` when `git diff --numstat` line counts predict the diff will
/// be unrenderable. Used to skip running `git diff --patch` entirely for
/// files with extreme change counts.
pub fn is_numstat_unrenderable(lines_added: usize, lines_removed: usize) -> bool {
    // Matches the deletion-only check in `compute_diff_size`.
    lines_removed > DELETION_LINE_RENDER_LIMIT
        // Catch pathologically large files that would produce multi-GB output.
        || lines_added + lines_removed > MAX_NUMSTAT_LINES_EARLY_BAIL
}

/// Categorizes a diff based on multiple size heuristics
pub fn compute_diff_size(diffs: &[DiffHunk], diff_size: usize) -> DiffSize {
    if is_diff_unrenderable(diff_size) {
        return DiffSize::Unrenderable;
    }

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

    if is_buffer_too_large(diff_size)
        || is_diff_too_large(diffs)
        || additions > DIFF_LINE_RENDER_LIMIT
        || deletions > DIFF_LINE_RENDER_LIMIT
    {
        return DiffSize::Large;
    }

    DiffSize::Normal
}
