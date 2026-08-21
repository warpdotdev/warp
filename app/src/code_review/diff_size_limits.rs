use std::fmt;

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

/**
 * Maximum number of changed files whose diffs are fully fetched and parsed
 * (hunks plus base content) in a single load. `MAX_DIFF_SIZE` bounds any one
 * file, but nothing previously bounded how many under-cap files a single
 * load (e.g. `git diff` against a long-diverged branch, or a repo-wide
 * reformat) could materialize — with enough changed files that aggregate is
 * unbounded even though every individual file is small (see APP-5462).
 * Beyond this count, remaining files are presented the same way a single
 * oversized file already is (`DiffSize::Unrenderable(DiffTooLarge)`)
 * instead of being fetched and parsed.
 */
pub const MAX_TOTAL_DIFF_FILES: usize = 2_000;

/**
 * Maximum aggregate bytes retained across all materialized file diffs (hunks
 * plus base content) in a single load — the other half of the aggregate
 * bound described on `MAX_TOTAL_DIFF_FILES`. Measured by
 * [`approx_file_diff_bytes`], a *lower bound* on real retained memory (see
 * its own doc comment for what it omits), not a full accounting of it.
 */
pub const MAX_TOTAL_DIFF_BYTES: usize = 256 * 1024 * 1024; // 256MB in decimal

/**
 * A lower bound on the memory a single file's materialized diff retains:
 * every `DiffHunk` line contributes its own struct size
 * (`size_of::<DiffLine>()`, not a hardcoded guess, so this stays correct if
 * the struct's fields change) plus its text's byte length floored at 8 (a
 * typical minimum allocator size class, so a 1-byte line isn't counted as
 * costing only 1 byte of heap), and the loaded base content (if any)
 * contributes its own length.
 *
 * This deliberately still undercounts real retained memory: each line's
 * `String` has its own heap allocation and allocator size-class rounding
 * beyond the floor applied here, `Vec` growth leaves capacity slack, and
 * `DiffHunk` itself has header fields not counted per line. Those add a
 * roughly constant-factor fudge on top of this number, not an
 * order-of-magnitude one — unlike counting only `line.text.len()` (an
 * earlier version of this budget), which undercounts badly: `DiffLine`
 * itself is tens of bytes of fixed overhead per line regardless of text
 * length, and diffs are typically dense with short lines, so the struct
 * overhead — not the text — dominates real retained memory. Undercounting
 * that badly let a budget meant to cap memory in the hundreds of megabytes
 * actually admit many times that.
 */
pub fn approx_file_diff_bytes(hunks: &[DiffHunk], content_at_head: Option<&str>) -> usize {
    // A common minimum allocator size class; see the doc comment above.
    const MIN_LINE_TEXT_ALLOCATION: usize = 8;

    let line_struct_size = std::mem::size_of::<DiffLine>();
    let hunks_bytes: usize = hunks
        .iter()
        .flat_map(|hunk| &hunk.lines)
        .map(|line| line_struct_size + line.text.len().max(MIN_LINE_TEXT_ALLOCATION))
        .sum();
    hunks_bytes + content_at_head.map_or(0, str::len)
}

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
    /// The base file content was withheld because it exceeded the per-file wire
    /// budget ([`MAX_DIFF_SIZE`]). Only produced when serializing a diff for a
    /// remote subscriber.
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
