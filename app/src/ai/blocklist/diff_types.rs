//! Plain data types describing a resolved file diff, plus small pure helpers
//! over them.
//!
//! These live outside the GUI `code_diff_view` module so that the shared,
//! surface-agnostic executor and persistence models can name them without
//! depending on any GUI view.
use std::ops::Range;

use ai::diff_validation::{DiffDelta, DiffType};
use warp_core::HostId;

/// The base content and file path for a diff.
#[derive(Clone)]
#[cfg_attr(debug_assertions, derive(Debug))]
pub struct DiffBase {
    /// The original file content before the diff is applied.
    /// Empty for new file creation.
    pub content: String,
    /// The absolute file path.
    pub file_path: String,
}

/// User-visible file diff with the original contents of the file
/// and the changes to those contents.
#[derive(Clone)]
#[cfg_attr(debug_assertions, derive(Debug))]
pub struct FileDiff {
    pub base: DiffBase,
    pub diff_type: DiffType,
}

impl FileDiff {
    /// Creates a `FileDiff` from base content, an absolute path, and the diff to apply.
    pub fn new(content: String, file_path: String, diff_type: DiffType) -> FileDiff {
        FileDiff {
            base: DiffBase { content, file_path },
            diff_type,
        }
    }

    /// Returns the absolute path this diff targets.
    pub fn file_path(&self) -> String {
        self.base.file_path.clone()
    }

    /// Returns `(lines_added, lines_removed)` described by this diff's op.
    pub fn line_stats(&self) -> (usize, usize) {
        match &self.diff_type {
            DiffType::Create { delta } => (line_count(&delta.insertion), 0),
            DiffType::Delete { delta } => (
                0,
                delta
                    .replacement_line_range
                    .end
                    .saturating_sub(delta.replacement_line_range.start),
            ),
            DiffType::Update { deltas, .. } => deltas.iter().fold((0, 0), |(add, rem), delta| {
                let removed = delta
                    .replacement_line_range
                    .end
                    .saturating_sub(delta.replacement_line_range.start);
                (add + line_count(&delta.insertion), rem + removed)
            }),
        }
    }
}

/// Counts lines in `content`, treating non-empty trailing text as its own line.
fn line_count(content: &str) -> usize {
    content.lines().count()
}

/// Whether a code diff targets the local filesystem or a remote host.
#[derive(Clone, Debug)]
pub enum DiffSessionType {
    Local,
    Remote(HostId),
}

/// Derives the 1-indexed changed line ranges described by a diff's deltas.
pub fn changed_lines_from_op(diff_type: &DiffType) -> Vec<Range<usize>> {
    match diff_type {
        DiffType::Create { delta } => inserted_content_range(1, &delta.insertion)
            .into_iter()
            .collect(),
        DiffType::Update { deltas, .. } => deltas
            .iter()
            .filter_map(changed_line_range_for_delta)
            .collect(),
        DiffType::Delete { .. } => vec![],
    }
}

/// Maps a single delta to the line range it changed.
fn changed_line_range_for_delta(delta: &DiffDelta) -> Option<Range<usize>> {
    let replacement_range = &delta.replacement_line_range;
    if replacement_range.start == replacement_range.end {
        return inserted_content_range(replacement_range.start.max(1), &delta.insertion);
    }
    Some(replacement_range.clone())
}

/// Returns the line range covered by inserted content starting at `start`.
fn inserted_content_range(start: usize, content: &str) -> Option<Range<usize>> {
    let line_count = content.lines().count();
    (line_count > 0).then_some(start..start + line_count)
}
