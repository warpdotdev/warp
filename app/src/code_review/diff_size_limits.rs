use std::fmt;
use std::mem::size_of;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::diff_state::{DiffHunk, DiffLine, DiffLineType};

/**
 * Maximum diff size that we will attempt to render. Diffs larger than this
 * should not be rendered to avoid performance issues.
 *
 * Also reused as the per-file limit for base content in a remote session.
 * Files larger than this should not be sent over the wire and should not be rendered.
 */
pub const MAX_DIFF_SIZE: usize = 4_375_000; // 4.375MB in decimal

/// Maximum number of files whose parsed hunks and base content a local Head diff retains.
/// This bounds per-file editor overhead that is not represented by [`MAX_TOTAL_DIFF_BYTES`].
pub const MAX_TOTAL_DIFF_FILES: usize = 2_000;

/// Maximum cumulative diff-line text and base content retained by a local Head diff.
pub const MAX_TOTAL_DIFF_BYTES: usize = 256 * 1024 * 1024; // 256MB

const _: () = assert!(MAX_TOTAL_DIFF_BYTES > MAX_DIFF_SIZE);
const _: () = assert!(MAX_TOTAL_DIFF_FILES > 0);

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
    /// Diff that cannot be rendered
    Unrenderable(UnrenderableReason),
}

/// Why a [`DiffSize::Unrenderable`] file cannot be rendered.
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum UnrenderableReason {
    /// The diff/patch itself is too large to render performantly (computed
    /// locally from the patch via [`compute_diff_size`]).
    DiffTooLarge,
    /// Render data was withheld because the file or aggregate diff exceeded its budget.
    FileTooLarge,
}

impl fmt::Display for UnrenderableReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DiffTooLarge => write!(f, "Diff is too large to render"),
            Self::FileTooLarge => write!(f, "File is too large to render"),
        }
    }
}

/// Estimates the allocations retained for a parsed file diff and its base content.
pub fn approx_file_diff_bytes(
    hunks: &Arc<Vec<DiffHunk>>,
    content_at_head: Option<&String>,
) -> usize {
    let hunk_bytes = hunks.capacity().saturating_mul(size_of::<DiffHunk>());
    let line_bytes = hunks
        .iter()
        .map(|hunk| {
            let line_storage = hunk.lines.capacity().saturating_mul(size_of::<DiffLine>());
            let text_storage = hunk
                .lines
                .iter()
                .map(|line| line.text.capacity())
                .fold(0usize, usize::saturating_add);
            line_storage.saturating_add(text_storage)
        })
        .fold(0usize, usize::saturating_add);
    hunk_bytes
        .saturating_add(line_bytes)
        .saturating_add(content_at_head.map_or(0, String::capacity))
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
    if is_diff_unrenderable(diff_size) {
        return DiffSize::Unrenderable(UnrenderableReason::DiffTooLarge);
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
        return DiffSize::Unrenderable(UnrenderableReason::DiffTooLarge);
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

#[cfg(test)]
#[path = "diff_size_limits_tests.rs"]
mod tests;
