use std::fmt;

use serde::{Deserialize, Serialize};

use super::diff_state::{DiffHunk, DiffLineType};

/**
 * Maximum diff size that we will attempt to render. Diffs larger than this
 * should not be rendered to avoid performance issues.
 *
 * Also reused as the per-file limit for base content in a remote session.
 * Files larger than this should not be sent over the wire and should not be rendered.
 */
pub const MAX_DIFF_SIZE: usize = 4_375_000; // 4.375MB in decimal

/**
 * Upper bound on the raw bytes read from a single-file `git diff` subprocess's
 * stdout before the subprocess is killed and the capture is abandoned (see
 * `run_git_command_capped` and APP-5462). This is a backstop above
 * `MAX_DIFF_SIZE`, not a replacement for it: `get_file_diff` still checks
 * completed output against `MAX_DIFF_SIZE`. A diff at or under `MAX_DIFF_SIZE`
 * must still arrive intact, and diff output carries some overhead over the
 * raw content it represents (headers, hunk markers, escaped paths), so this
 * budget is a small multiple of `MAX_DIFF_SIZE` rather than an equal cap —
 * enough margin to never clip a diff that would otherwise render, while still
 * bounding a single pathological file (e.g. a huge generated/minified or
 * binary-like text file) to a small, constant amount of memory instead of the
 * multi-GB growth an unbounded capture allowed.
 */
pub const MAX_DIFF_STDOUT_CAPTURE_BYTES: usize = MAX_DIFF_SIZE * 3; // ~12.5MB in decimal

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
